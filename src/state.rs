// SPDX-License-Identifier: AGPL-3.0-only
//! Interactive console state machine for `C1`–`C6`, `SubViewBar`, Registry &
//! Observe sub-views, keyboard shortcuts, rotary 350 ms settle debounce,
//! `PullToTurn` restart guard, and `Hurmit` <-> `OpenDyslexicM` persistence.

use std::collections::BTreeMap;

use mu_console_protocol::{
    Area, AuthState, CdSubView, CiSubView, CrtFont, LogSeverity, ObserveSubView, PackageEcosystem,
    PanelId, RegistrySubView, RepoId, RepoScore, Route, SubView, SubmoduleId, UiCommand,
};
use mu_console_theme::instruments::switch::SwitchAction;
use mu_console_theme::interaction::{SwitchEvent, SwitchOutcome, SwitchState};
use mu_console_theme::metrics::{CrtFontMetrics, HURMIT, OPEN_DYSLEXIC};

use crate::client::{ClientUpdate, DataClient};
use crate::router::{area_label, escape_up, parse_area, parse_hash, parse_subview};

/// `localStorage` key persisting the user's selected CRT reading font.
pub const FONT_STORAGE_KEY: &str = "muconsole.dyslexia_font";
/// `localStorage` key persisting the user's active repository/submodule selection.
pub const REPO_STORAGE_KEY: &str = "muconsole.active_repo";
/// `sessionStorage` key holding a captured one-time `?invite=<token>` across OAuth redirects.
pub const PENDING_INVITE_STORAGE_KEY: &str = "muconsole.pending_invite";
/// Rotary settle window before dispatching [`UiCommand::SelectRepo`].
pub const ROTARY_SETTLE_MS: u64 = 350;
/// Armed confirmation window (ms) for the guarded Trainway `RESTART` control.
pub const RESTART_ARM_WINDOW_MS: u64 = 3_000;

/// Ordered areas on the C1 selector rail (`CI`, `CD`, `REG`, `OBS`, `FLT`).
pub const AREAS: [Area; 5] = Area::ALL;

/// Extract `?invite=<token>` from a URL query string and return `(Some(token), sanitized_query)`
/// with the `invite` parameter stripped so one-time invite tokens are removed from browser history.
#[must_use]
pub fn extract_invite_token(query: &str) -> (Option<String>, String) {
    let raw = query.trim_start_matches('?');
    if raw.is_empty() {
        return (None, String::new());
    }
    let mut token = None;
    let mut kept = Vec::new();
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        if let Some(("invite", val)) = pair.split_once('=') {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                token = Some(trimmed.to_owned());
            }
        } else {
            kept.push(pair);
        }
    }
    let clean = if kept.is_empty() {
        String::new()
    } else {
        format!("?{}", kept.join("&"))
    };
    (token, clean)
}

/// Percent-encode a string for use in query parameter values (RFC 3986).
#[must_use]
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{:02X}", b);
            }
        }
    }
    out
}

/// Format a sign-in URL with a strictly formatted and percent-encoded `?rd=` return parameter.
#[must_use]
pub fn format_sign_in_url(rd: &str) -> String {
    if rd == "/" {
        "/oauth2/start?rd=/".to_string()
    } else {
        format!("/oauth2/start?rd={}", percent_encode(rd))
    }
}

/// Format a sign-out URL with a strictly formatted and percent-encoded `?rd=` return parameter.
#[must_use]
pub fn format_sign_out_url(rd: &str) -> String {
    if rd == "/" {
        "/oauth2/sign_out?rd=/".to_string()
    } else {
        format!("/oauth2/sign_out?rd={}", percent_encode(rd))
    }
}

/// Validate that a sign-in `href` has a valid query format with `=`.
/// Rejects malformed patterns such as `?rdhttps:` missing `=`.
pub fn validate_sign_in_href(href: &str) -> Result<(), &'static str> {
    let Some((path, query)) = href.split_once('?') else {
        return Err("missing query string");
    };
    if path != "/oauth2/start" {
        return Err("invalid path, expected /oauth2/start");
    }
    let mut has_rd = false;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == "rd" {
                if v.is_empty() {
                    return Err("empty rd parameter");
                }
                has_rd = true;
            }
        } else if pair.starts_with("rd") {
            return Err("malformed rd parameter missing '='");
        }
    }
    if has_rd {
        Ok(())
    } else {
        Err("missing rd parameter")
    }
}

/// Parse `?font=dyslexic|hurmit` or stored `localStorage` value.
#[must_use]
pub fn parse_font_pref(query: &str, stored: Option<&str>) -> CrtFont {
    for pair in query.trim_start_matches('?').split('&') {
        if let Some(("font", val)) = pair.split_once('=') {
            if val.eq_ignore_ascii_case("dyslexic")
                || val.eq_ignore_ascii_case("opendyslexic")
                || val.eq_ignore_ascii_case("true")
                || val == "1"
            {
                return CrtFont::OpenDyslexic;
            }
            if val.eq_ignore_ascii_case("hurmit") || val.eq_ignore_ascii_case("false") || val == "0"
            {
                return CrtFont::Hurmit;
            }
        }
    }
    if let Some(s) = stored {
        let s = s.trim();
        if s.eq_ignore_ascii_case("dyslexic")
            || s.eq_ignore_ascii_case("opendyslexic")
            || s.eq_ignore_ascii_case("true")
            || s == "1"
        {
            return CrtFont::OpenDyslexic;
        }
    }
    CrtFont::Hurmit
}

/// Serialize [`CrtFont`] for `localStorage` / query parameters.
#[must_use]
pub const fn font_slug(font: CrtFont) -> &'static str {
    match font {
        CrtFont::Hurmit => "hurmit",
        CrtFont::OpenDyslexic => "dyslexic",
    }
}

/// Complete UI state for the console.
#[derive(Debug)]
pub struct ConsoleState {
    pub client: DataClient,
    pub route: Route,
    pub font: CrtFont,
    pub lazy_dyslexic_loaded: bool,
    pub reduced_motion: bool,
    /// Outer rotary ring index into [`Self::filtered_repos`].
    pub outer_index: usize,
    /// Inner rotary ring index (`0` = `PARENT`, `1..=M` = submodules).
    pub inner_index: usize,
    /// `/` type-ahead filter string.
    pub filter_query: String,
    pub filter_focused: bool,
    /// Pending rotary selection waiting for the 350 ms settle window (`due_ms`).
    pub pending_select_at_ms: Option<u64>,
    /// Phone bottom-sheet repo selector visibility.
    pub phone_rotary_open: bool,
    /// C4 Scorecard page index.
    pub carousel_page: u32,
    /// Persistent Trainway siding lane assignments (`ref_name -> lane`).
    pub trainway_lanes: BTreeMap<String, u32>,
    /// C5 Trainway `YARD +N` overflow page index.
    pub yard_page: u32,
    /// C5 Trainway `LIVE` follow toggle.
    pub trainway_live: bool,
    /// C5 guarded `PullToTurn` `RESTART` switch (`NORM` = 0, `RST` = 1).
    pub restart_switch: SwitchState,
    /// Timestamp until which the `RESTART` switch remains armed (`Some(until_ms)`).
    pub restart_armed_until_ms: Option<u64>,
    /// Count of confirmed restarts triggered through the `PullToTurn` guard.
    pub restart_count: u32,
    /// C6 Delivery history strip selection (`0` = live / latest gate).
    pub selected_history_idx: usize,
    /// C6 Delivery selected test row for log excerpt paging.
    pub selected_test_idx: usize,
    /// C6 Delivery log excerpt page cursor.
    pub log_page: u32,
    /// Fleet telemetry page cursor.
    pub fleet_page: u32,
    /// `REG > Packages` ecosystem filter (`None` = `ALL`).
    pub reg_ecosystem_filter: Option<PackageEcosystem>,
    /// `REG > Packages` selected package name for the version/dependency drawer.
    pub selected_package: Option<String>,
    /// `OBS > Smartscape` selected entity ID for the side inspection drawer.
    pub selected_entity_id: Option<String>,
    /// `OBS > PurePath` quick-filter pill (`ALL`, `ERRORS`, `SLOW P99`, `CI`, `CD`, `REGISTRY`).
    pub purepath_filter: String,
    /// `OBS > PurePath` selected trace ID.
    pub selected_trace_id: Option<String>,
    /// `OBS > PurePath` selected span ID.
    pub selected_span_id: Option<String>,
    /// `OBS > Logs` severity filter (`None` = `ALL`).
    pub log_severity_filter: Option<LogSeverity>,
    /// Polite `aria-live` announcement text.
    pub aria_polite: String,
    /// Assertive `aria-live` alert text.
    pub aria_assertive: String,
    /// One-time invite token captured from `?invite=<token>` or `sessionStorage`.
    pub pending_invite: Option<String>,
    /// Target URL for pending client-side or browser navigation.
    pub pending_navigation: Option<String>,
}

impl Default for ConsoleState {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsoleState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: DataClient::new(),
            route: Route {
                area: Area::Ci,
                subview: SubView::Ci(CiSubView::Scorecards),
                repo: None,
                submodule: None,
                branch: None,
                job: None,
                trace_id: None,
                filter: None,
            },
            font: CrtFont::Hurmit,
            lazy_dyslexic_loaded: false,
            reduced_motion: false,
            outer_index: 0,
            inner_index: 0,
            filter_query: String::new(),
            filter_focused: false,
            pending_select_at_ms: None,
            phone_rotary_open: false,
            carousel_page: 0,
            trainway_lanes: BTreeMap::new(),
            yard_page: 0,
            trainway_live: true,
            restart_switch: SwitchState::new(SwitchAction::PullToTurn, 2, 0),
            restart_armed_until_ms: None,
            restart_count: 0,
            selected_history_idx: 0,
            selected_test_idx: 0,
            log_page: 0,
            fleet_page: 0,
            reg_ecosystem_filter: None,
            selected_package: None,
            selected_entity_id: None,
            purepath_filter: "ALL".into(),
            selected_trace_id: None,
            selected_span_id: None,
            log_severity_filter: None,
            aria_polite: String::new(),
            aria_assertive: String::new(),
            pending_invite: None,
            pending_navigation: None,
        }
    }

    /// Initialize route, font preference, and any `?invite=<token>` from browser location + storage.
    pub fn init_from_env(&mut self, hash: &str, search: &str, stored_font: Option<&str>) {
        self.apply_hash(hash);
        let (invite, _) = extract_invite_token(search);
        if invite.is_some() {
            self.pending_invite = invite;
        }
        let f = parse_font_pref(search, stored_font);
        self.set_font(f);
    }

    /// Apply a location hash and synchronize sub-view filters / trace selection.
    pub fn apply_hash(&mut self, hash: &str) {
        self.route = parse_hash(hash);
        if let Some(tr) = self.route.trace_id.clone() {
            self.selected_trace_id = Some(tr);
        }
        if let Some(flt) = self.route.filter.clone() {
            match self.route.subview {
                SubView::Observe(ObserveSubView::PurePath) => {
                    self.purepath_filter = flt.to_ascii_uppercase();
                }
                SubView::Observe(ObserveSubView::Logs) => {
                    self.log_severity_filter = match flt.to_ascii_uppercase().as_str() {
                        "ERROR" | "ERRORS" => Some(LogSeverity::Error),
                        "WARN" => Some(LogSeverity::Warn),
                        "INFO" => Some(LogSeverity::Info),
                        _ => None,
                    };
                }
                SubView::Registry(RegistrySubView::Packages) => {
                    self.reg_ecosystem_filter = match flt.to_ascii_uppercase().as_str() {
                        "CARGO" => Some(PackageEcosystem::Cargo),
                        "NUGET" => Some(PackageEcosystem::NuGet),
                        "MAVEN" => Some(PackageEcosystem::Maven),
                        _ => None,
                    };
                }
                _ => {}
            }
        }
        self.sync_rotary_from_route();
    }

    /// Restore active repository from `localStorage` (`muconsole.active_repo`) when URL has none.
    pub fn restore_stored_repo(&mut self, stored_repo: Option<&str>) {
        if self.route.repo.is_some() {
            return;
        }
        let Some(raw) = stored_repo.map(str::trim).filter(|s| !s.is_empty()) else {
            return;
        };
        let (repo_part, sub_part) = raw.split_once("::").unwrap_or((raw, ""));
        if let Some((owner, repo)) = repo_part.split_once('/') {
            if !owner.is_empty() && !repo.is_empty() {
                self.route.repo = Some(RepoId::new(owner, repo));
                if !sub_part.is_empty() {
                    self.route.submodule = Some(SubmoduleId {
                        path: sub_part.to_owned(),
                    });
                }
            }
        }
    }

    /// Serialize active repo + submodule for `localStorage` (`muconsole.active_repo`).
    #[must_use]
    pub fn stored_repo_value(&self) -> Option<String> {
        let repo = self.route.repo.as_ref()?;
        match self.route.submodule.as_ref() {
            Some(sub) => Some(format!("{}/{}::{}", repo.owner, repo.repo, sub.path)),
            None => Some(format!("{}/{}", repo.owner, repo.repo)),
        }
    }

    /// Current operator authentication state from the view model (defaults to [`AuthState::Anonymous`]).
    #[must_use]
    pub fn auth(&self) -> AuthState {
        self.client
            .view()
            .map(|v| v.auth.clone())
            .unwrap_or(AuthState::Anonymous)
    }

    /// Whether the active session is an authenticated, invite-bound operator.
    #[must_use]
    pub fn is_operator(&self) -> bool {
        self.auth().is_operator()
    }

    /// Return the formatted sign-in URL.
    #[must_use]
    pub fn sign_in_href(&self) -> String {
        format_sign_in_url("/")
    }

    /// Return the formatted sign-out URL.
    #[must_use]
    pub fn sign_out_href(&self) -> String {
        format_sign_out_url("/")
    }

    /// Active font metrics (`HURMIT` or `OPEN_DYSLEXIC`).
    #[must_use]
    pub const fn font_metrics(&self) -> &'static CrtFontMetrics {
        match self.font {
            CrtFont::Hurmit => &HURMIT,
            CrtFont::OpenDyslexic => &OPEN_DYSLEXIC,
        }
    }

    /// Set the active reading font, lazy-loading `OpenDyslexicM` on first use.
    pub fn set_font(&mut self, font: CrtFont) -> UiCommand {
        self.font = font;
        if font == CrtFont::OpenDyslexic {
            self.lazy_dyslexic_loaded = true;
        }
        self.aria_polite = format!("Font set to {}", self.font_metrics().family);
        UiCommand::SetCrtFont(font)
    }

    /// Toggle between `Hurmit` and `OpenDyslexicM`.
    pub fn toggle_font(&mut self) -> UiCommand {
        let next = match self.font {
            CrtFont::Hurmit => CrtFont::OpenDyslexic,
            CrtFont::OpenDyslexic => CrtFont::Hurmit,
        };
        self.set_font(next)
    }

    /// Feed a worker frame into [`DataClient`] and synchronize selection/route.
    pub fn on_worker_frame(&mut self, frame: &[u8]) -> ClientUpdate {
        let upd = self.client.on_frame(frame);
        match &upd {
            ClientUpdate::View => {
                if let Some(v) = self.client.view() {
                    if let Some(sel) = v.selection.as_ref() {
                        if self.route.repo.is_none() {
                            self.route.repo = Some(sel.repo.clone());
                            self.route.submodule.clone_from(&sel.submodule);
                        }
                    }
                    if self.selected_trace_id.is_none() {
                        self.selected_trace_id =
                            v.observe.traces.first().map(|t| t.trace_id.clone());
                    }
                    if v.topology
                        .as_ref()
                        .is_some_and(mu_console_protocol::RepoTopology::stop)
                    {
                        self.aria_assertive = "STOP: CI stage failure detected".into();
                    }
                }
                self.sync_rotary_from_route();
            }
            ClientUpdate::ReloadRequired { expected, found } => {
                self.aria_assertive = crate::client::reload_banner_text(*expected, *found);
            }
            _ => {}
        }
        upd
    }

    /// Repositories matching the current `/` type-ahead filter.
    #[must_use]
    pub fn filtered_repos(&self) -> Vec<RepoScore> {
        let Some(v) = self.client.view() else {
            return Vec::new();
        };
        let q = self.filter_query.trim().to_ascii_lowercase();
        if q.is_empty() {
            return v.repos.clone();
        }
        v.repos
            .iter()
            .filter(|r| {
                format!("{}/{}", r.id.owner, r.id.repo)
                    .to_ascii_lowercase()
                    .contains(&q)
            })
            .cloned()
            .collect()
    }

    /// Scorecards ordered for `CI > Scorecards`: selected parent repo first, then
    /// its submodules in `.gitmodules` order, then remaining repositories.
    #[must_use]
    pub fn ordered_carousel_repos(&self) -> Vec<RepoScore> {
        let Some(v) = self.client.view() else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(v.repos.len());
        let parent_id = self
            .route
            .repo
            .as_ref()
            .or_else(|| v.selection.as_ref().map(|s| &s.repo));
        if let Some(pid) = parent_id {
            if let Some(parent) = v.repos.iter().find(|r| &r.id == pid) {
                out.push(parent.clone());
                for sub in &parent.submodules {
                    if let Some(sr) = v.repos.iter().find(|r| r.id == sub.repo) {
                        if !out.iter().any(|x| x.id == sr.id) {
                            out.push(sr.clone());
                        }
                    }
                }
            }
        }
        for r in &v.repos {
            if !out.iter().any(|x| x.id == r.id) {
                out.push(r.clone());
            }
        }
        out
    }

    /// Synchronize `outer_index` and `inner_index` with `self.route`.
    pub fn sync_rotary_from_route(&mut self) {
        let repos = self.filtered_repos();
        if repos.is_empty() {
            self.outer_index = 0;
            self.inner_index = 0;
            return;
        }
        if let Some(target) = self.route.repo.as_ref() {
            if let Some(pos) = repos.iter().position(|r| &r.id == target) {
                self.outer_index = pos;
            }
        }
        self.outer_index = self.outer_index.min(repos.len() - 1);
        let parent = &repos[self.outer_index];
        self.inner_index = match self.route.submodule.as_ref() {
            None => 0,
            Some(sub) => parent
                .submodules
                .iter()
                .position(|s| s.path == sub.path)
                .map_or(0, |i| i + 1),
        };
    }

    /// Rotate the outer (repository) ring by `delta` detents and start the
    /// 350 ms settle timer.
    pub fn rotate_outer(&mut self, delta: isize, now_ms: u64) {
        let repos = self.filtered_repos();
        if repos.is_empty() {
            return;
        }
        let len = isize::try_from(repos.len()).unwrap_or(1);
        let cur = isize::try_from(self.outer_index).unwrap_or(0);
        let next = (cur + delta).rem_euclid(len);
        self.outer_index = usize::try_from(next).unwrap_or(0);
        self.inner_index = 0;
        let chosen = &repos[self.outer_index];
        self.route.repo = Some(chosen.id.clone());
        self.route.submodule = None;
        self.pending_select_at_ms = Some(now_ms.saturating_add(ROTARY_SETTLE_MS));
        self.aria_polite = format!("Selected repository {}/{}", chosen.id.owner, chosen.id.repo);
    }

    /// Rotate the inner (`PARENT` + submodules) ring by `delta` detents and
    /// start the 350 ms settle timer.
    pub fn rotate_inner(&mut self, delta: isize, now_ms: u64) {
        let repos = self.filtered_repos();
        let Some(parent) = repos.get(self.outer_index) else {
            return;
        };
        let total = parent.submodules.len() + 1;
        let len = isize::try_from(total).unwrap_or(1);
        let cur = isize::try_from(self.inner_index).unwrap_or(0);
        let next = (cur + delta).rem_euclid(len);
        self.inner_index = usize::try_from(next).unwrap_or(0);
        self.route.repo = Some(parent.id.clone());
        self.route.submodule = if self.inner_index == 0 {
            None
        } else {
            parent
                .submodules
                .get(self.inner_index - 1)
                .map(|s| SubmoduleId {
                    path: s.path.clone(),
                })
        };
        self.pending_select_at_ms = Some(now_ms.saturating_add(ROTARY_SETTLE_MS));
        let label = self
            .route
            .submodule
            .as_ref()
            .map_or("PARENT", |s| s.path.as_str());
        self.aria_polite = format!("Selected submodule {label}");
    }

    /// Update the `/` type-ahead filter and snap `outer_index` to the first
    /// matching repository.
    pub fn set_filter_query(&mut self, query: &str, now_ms: u64) {
        self.filter_query = query.to_owned();
        self.outer_index = 0;
        self.inner_index = 0;
        let repos = self.filtered_repos();
        if let Some(first) = repos.first() {
            self.route.repo = Some(first.id.clone());
            self.route.submodule = None;
            self.pending_select_at_ms = Some(now_ms.saturating_add(ROTARY_SETTLE_MS));
        }
    }

    /// Select an area on C1 (`CI`, `CD`, `REG`, `OBS`, `FLT`).
    pub fn select_area(&mut self, area: Area) -> UiCommand {
        self.route.area = area;
        if self.route.subview.area() != area {
            self.route.subview = SubView::default_for(area);
        }
        self.aria_polite = format!(
            "Area {} :: {}",
            area_label(area),
            self.route.subview.label()
        );
        UiCommand::Subscribe(self.route.clone())
    }

    /// Select a specific [`SubView`] tab in `SubViewBar`.
    pub fn select_subview(&mut self, subview: SubView) -> Vec<UiCommand> {
        self.route.area = subview.area();
        self.route.subview = subview;
        self.aria_polite = format!(
            "Sub-view {} :: {}",
            area_label(self.route.area),
            subview.label()
        );
        vec![
            UiCommand::Subscribe(self.route.clone()),
            UiCommand::SelectSubView(subview),
        ]
    }

    /// Navigate via `[TRACE ↗]` pill to `OBS > PurePath` pre-filtered to `trace_id`.
    pub fn open_trace(&mut self, trace_id: &str) -> Vec<UiCommand> {
        self.route.area = Area::Observe;
        self.route.subview = SubView::Observe(ObserveSubView::PurePath);
        self.route.trace_id = Some(trace_id.to_owned());
        self.selected_trace_id = Some(trace_id.to_owned());
        self.selected_span_id = None;
        self.aria_polite = format!("Opened PurePath trace {trace_id}");
        vec![
            UiCommand::Subscribe(self.route.clone()),
            UiCommand::SelectSubView(self.route.subview),
        ]
    }

    /// Navigate via `[CD GATE ↗]` pill to `CD > Integration Gate`.
    pub fn open_cd_gate(&mut self, deployment_id: Option<&str>) -> Vec<UiCommand> {
        self.route.area = Area::Cd;
        self.route.subview = SubView::Cd(CdSubView::IntegrationGate);
        if let Some(dep) = deployment_id {
            self.aria_polite = format!("Opened CD Integration Gate for {dep}");
        } else {
            self.aria_polite = "Opened CD Integration Gate".into();
        }
        vec![
            UiCommand::Subscribe(self.route.clone()),
            UiCommand::SelectSubView(self.route.subview),
        ]
    }

    /// Step the `CI > Scorecards` page by `delta` pages.
    pub fn step_carousel(&mut self, delta: i32, cards_per_page: usize) -> UiCommand {
        let total_repos = self.ordered_carousel_repos().len();
        let per = cards_per_page.max(1);
        let pages = i32::try_from(total_repos.div_ceil(per).max(1)).unwrap_or(1);
        let cur = i32::try_from(self.carousel_page).unwrap_or(0);
        let next = (cur + delta).rem_euclid(pages);
        self.carousel_page = u32::try_from(next).unwrap_or(0);
        UiCommand::Page {
            panel: PanelId::Carousel,
            cursor: self.carousel_page,
        }
    }

    /// Feed an event into the guarded `PullToTurn` Trainway `RESTART` switch.
    ///
    /// Requires an authenticated operator session (`self.is_operator()`).
    /// Rotating to position `1` (`RST`) requires either a >= 300 ms pull hold
    /// or `shift: true`. The first valid rotation arms the control for
    /// [`RESTART_ARM_WINDOW_MS`]; confirming within that window dispatches
    /// [`UiCommand::Retry`].
    pub fn on_restart_switch(&mut self, ev: SwitchEvent, now_ms: u64) -> Option<UiCommand> {
        if !self.is_operator() {
            self.restart_armed_until_ms = None;
            self.aria_assertive =
                "RAW LOGS & JOB RESTART REQUIRE INVITED GITHUB OPERATOR SESSION".into();
            return None;
        }
        if let Some(until) = self.restart_armed_until_ms {
            if now_ms > until {
                self.restart_armed_until_ms = None;
            }
        }
        match self.restart_switch.apply(ev) {
            SwitchOutcome::Fired(1) => {
                if self
                    .restart_armed_until_ms
                    .is_some_and(|until| now_ms <= until)
                {
                    self.restart_armed_until_ms = None;
                    self.restart_count = self.restart_count.saturating_add(1);
                    self.aria_assertive = "Trainway restart confirmed".into();
                    Some(UiCommand::Retry)
                } else {
                    self.restart_armed_until_ms =
                        Some(now_ms.saturating_add(RESTART_ARM_WINDOW_MS));
                    self.aria_polite = "Restart armed: confirm within 3 seconds to execute".into();
                    None
                }
            }
            SwitchOutcome::Refused => {
                self.aria_polite = "Pull and hold 300ms (or Shift) before turning RESTART".into();
                None
            }
            _ => None,
        }
    }

    /// Handle a DOM click on an interactive control or semantic `data-*` attribute.
    #[allow(clippy::too_many_lines)]
    pub fn on_dom_action(
        &mut self,
        attr_name: &str,
        attr_val: &str,
        shift: bool,
        now_ms: u64,
    ) -> Vec<UiCommand> {
        match attr_name {
            "data-area" => {
                if let Some(area) = parse_area(attr_val) {
                    let cmd = self.select_area(area);
                    return vec![cmd, UiCommand::SelectSubView(self.route.subview)];
                }
            }
            "data-subview" => {
                for area in Area::ALL {
                    if let Some(sv) = parse_subview(area, attr_val) {
                        return self.select_subview(sv);
                    }
                }
            }
            "data-repo-step" => match attr_val {
                "prev" => self.rotate_outer(-1, now_ms),
                "next" => self.rotate_outer(1, now_ms),
                "sub-prev" => self.rotate_inner(-1, now_ms),
                "sub-next" => self.rotate_inner(1, now_ms),
                _ => {}
            },
            "data-dyslexia-toggle" => {
                return vec![self.toggle_font()];
            }
            "data-trainway-restart" => {
                if let Some(cmd) = self.on_restart_switch(
                    SwitchEvent::Rotate {
                        to: 1,
                        at_ms: now_ms,
                        shift: true,
                    },
                    now_ms,
                ) {
                    return vec![cmd];
                }
            }
            "data-trace-id" => {
                return self.open_trace(attr_val);
            }
            "data-cd-gate" => {
                return self.open_cd_gate(Some(attr_val));
            }
            "data-ecosystem" => {
                self.reg_ecosystem_filter = match attr_val.to_ascii_uppercase().as_str() {
                    "CARGO" => Some(PackageEcosystem::Cargo),
                    "NUGET" => Some(PackageEcosystem::NuGet),
                    "MAVEN" => Some(PackageEcosystem::Maven),
                    _ => None,
                };
                self.route.filter = self.reg_ecosystem_filter.map(|e| e.label().to_owned());
                self.aria_polite = format!("Filtered packages by {attr_val}");
            }
            "data-package" => {
                if self.selected_package.as_deref() == Some(attr_val) {
                    self.selected_package = None;
                } else {
                    self.selected_package = Some(attr_val.to_owned());
                }
            }
            "data-entity-id" => {
                if self.selected_entity_id.as_deref() == Some(attr_val) {
                    self.selected_entity_id = None;
                } else {
                    self.selected_entity_id = Some(attr_val.to_owned());
                }
            }
            "data-purepath-filter" => {
                self.purepath_filter = attr_val.to_ascii_uppercase();
                self.route.filter = if self.purepath_filter == "ALL" {
                    None
                } else {
                    Some(self.purepath_filter.clone())
                };
                self.aria_polite = format!("PurePath filter {}", self.purepath_filter);
            }
            "data-select-trace" => {
                self.selected_trace_id = Some(attr_val.to_owned());
                self.route.trace_id = Some(attr_val.to_owned());
                self.selected_span_id = None;
            }
            "data-span-id" => {
                self.selected_span_id = Some(attr_val.to_owned());
            }
            "data-log-severity" => {
                self.log_severity_filter = match attr_val.to_ascii_uppercase().as_str() {
                    "ERROR" | "ERRORS" => Some(LogSeverity::Error),
                    "WARN" => Some(LogSeverity::Warn),
                    "INFO" => Some(LogSeverity::Info),
                    _ => None,
                };
                self.route.filter = self.log_severity_filter.map(|s| s.label().to_owned());
            }
            "data-control-id" => return self.on_control_id(attr_val, shift, now_ms),
            _ => {}
        }
        Vec::new()
    }

    fn on_control_id(&mut self, id: &str, shift: bool, now_ms: u64) -> Vec<UiCommand> {
        if let Some(area_slug) = id
            .strip_prefix("c1-area-rail:")
            .or_else(|| id.strip_prefix("c1-area-selector:"))
        {
            if let Some(area) = parse_area(area_slug) {
                let cmd = self.select_area(area);
                return vec![cmd, UiCommand::SelectSubView(self.route.subview)];
            }
        }
        if let Some(sv_slug) = id.strip_prefix("c2-subview-bar:") {
            if let Some(sv) = parse_subview(self.route.area, sv_slug) {
                return self.select_subview(sv);
            }
        }
        if let Some(repo_str) = id.strip_prefix("c4-card-select:") {
            if let Some((owner, repo)) = repo_str.split_once('/') {
                let rid = RepoId::new(owner, repo);
                self.route.repo = Some(rid.clone());
                self.route.submodule = None;
                self.sync_rotary_from_route();
                return vec![UiCommand::SelectRepo {
                    repo: rid,
                    submodule: None,
                }];
            }
        }
        match id {
            "c2-font-switch" | "c2-font-switch:hurmit" | "c2-font-switch:dyslexic" => {
                vec![self.toggle_font()]
            }
            "c2-repo-prev" | "c3-rotary:outer-prev" => {
                self.rotate_outer(-1, now_ms);
                Vec::new()
            }
            "c2-repo-next" | "c3-rotary:outer-next" | "c2-repo-select" => {
                self.rotate_outer(1, now_ms);
                Vec::new()
            }
            "c2-sub-prev" | "c3-rotary:inner-prev" => {
                self.rotate_inner(-1, now_ms);
                Vec::new()
            }
            "c2-sub-next" | "c3-rotary:inner-next" => {
                self.rotate_inner(1, now_ms);
                Vec::new()
            }
            "c2-phone-rotary-trigger" => {
                self.phone_rotary_open = !self.phone_rotary_open;
                Vec::new()
            }
            "c4-carousel-switch:prev" => vec![self.step_carousel(-1, 3)],
            "c4-carousel-switch:next" => vec![self.step_carousel(1, 3)],
            "c5-live-toggle" => {
                self.trainway_live = !self.trainway_live;
                Vec::new()
            }
            "c5-yard-pager" => {
                self.yard_page = self.yard_page.wrapping_add(1);
                Vec::new()
            }
            "c5-restart-switch" | "c5-restart-switch:rst" => self
                .on_restart_switch(
                    SwitchEvent::Rotate {
                        to: 1,
                        at_ms: now_ms,
                        shift,
                    },
                    now_ms,
                )
                .into_iter()
                .collect(),
            "c6-log-pager" => {
                self.log_page = self.log_page.wrapping_add(1);
                Vec::new()
            }
            "c2-auth-status" => {
                let href = match self.auth() {
                    AuthState::Anonymous => self.sign_in_href(),
                    _ => self.sign_out_href(),
                };
                self.pending_navigation = Some(href.clone());
                self.aria_polite = format!("Navigating to {href}");
                vec![UiCommand::Navigate { href }]
            }
            _ => Vec::new(),
        }
    }

    /// Handle a keyboard shortcut and return any resulting [`UiCommand`].
    pub fn on_key(&mut self, key: &str, shift: bool, now_ms: u64) -> Option<UiCommand> {
        if key == "Escape" {
            if self.phone_rotary_open {
                self.phone_rotary_open = false;
                self.aria_polite = "Closed repository selector sheet".into();
                return None;
            }
            if self.selected_package.is_some() {
                self.selected_package = None;
                self.aria_polite = "Closed package detail drawer".into();
                return None;
            }
            if self.selected_entity_id.is_some() {
                self.selected_entity_id = None;
                self.aria_polite = "Closed Smartscape entity drawer".into();
                return None;
            }
            if !self.filter_query.is_empty() || self.filter_focused {
                self.filter_query.clear();
                self.filter_focused = false;
                self.aria_polite = "Cleared repository filter".into();
                return None;
            }
            self.route = escape_up(&self.route);
            self.aria_polite = "Navigated up one level".into();
            return Some(UiCommand::Subscribe(self.route.clone()));
        }

        if self.filter_focused {
            return None;
        }

        match key {
            "1" => Some(self.select_area(Area::Ci)),
            "2" => Some(self.select_area(Area::Cd)),
            "3" => Some(self.select_area(Area::Registry)),
            "4" => Some(self.select_area(Area::Observe)),
            "5" => Some(self.select_area(Area::Fleet)),
            "[" => {
                let prev = self.route.subview.step(-1);
                self.select_subview(prev).into_iter().next()
            }
            "]" => {
                let next = self.route.subview.step(1);
                self.select_subview(next).into_iter().next()
            }
            "/" => {
                self.filter_focused = true;
                self.aria_polite = "Type-ahead repository filter focused".into();
                None
            }
            "ArrowUp" | "<" | "," => {
                if shift {
                    self.rotate_inner(-1, now_ms);
                } else {
                    self.rotate_outer(-1, now_ms);
                }
                None
            }
            "ArrowDown" | ">" | "." => {
                if shift {
                    self.rotate_inner(1, now_ms);
                } else {
                    self.rotate_outer(1, now_ms);
                }
                None
            }
            "ArrowLeft" => Some(self.step_carousel(-1, 3)),
            "ArrowRight" => Some(self.step_carousel(1, 3)),
            "d" | "D" | "f" | "F" => Some(self.toggle_font()),
            "r" | "R" => {
                self.aria_polite = "Stream reset and resynchronized".into();
                Some(UiCommand::ResetStream)
            }
            _ => None,
        }
    }

    /// Advance timers: emit [`UiCommand::SelectRepo`] once the 350 ms rotary
    /// settle window elapses, and expire the 3 s restart arm window.
    pub fn tick(&mut self, now_ms: u64) -> Vec<UiCommand> {
        let mut cmds = Vec::new();
        if let Some(until) = self.restart_armed_until_ms {
            if now_ms > until {
                self.restart_armed_until_ms = None;
            }
        }
        if let Some(due) = self.pending_select_at_ms {
            if now_ms >= due {
                self.pending_select_at_ms = None;
                let repo = self
                    .route
                    .repo
                    .clone()
                    .unwrap_or_else(|| RepoId::new("fixture", "superproject"));
                cmds.push(UiCommand::SelectRepo {
                    repo,
                    submodule: self.route.submodule.clone(),
                });
            }
        }
        cmds
    }
}
