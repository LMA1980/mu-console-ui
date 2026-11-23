// SPDX-License-Identifier: AGPL-3.0-only
//! Client UI state machine, fail-closed auth/policy gating, skin/font persistence,
//! explicit-activity `Timed` lease timers, and modal/DOM action routing.

use std::cell::Cell;

use mu_console_layout::{scorecard_page_capacity, SafeArea, Viewport};
use mu_console_protocol::{
    AbDeploySubView, Area, AuthState, AuthStatus, CiJobSummary, CiSubView, ConsoleSkin, CrtFont,
    DialogModel, LogSeverity, MaintenanceMode, PackageEcosystem, PasskeyEnrollmentGateModel,
    RepoId, RepoScore, RepoStatus, RiskConfirmAction, RiskConfirmDialogModel, Route,
    SessionRoleDescriptor, SessionRolesDialogModel, SubView, SubmoduleId, SubsystemId, UiCommand,
    UserProfileDialogModel, UserProfileTab, ViewModel, Visibility,
};
use mu_console_theme::metrics::{CrtFontMetrics, HURMIT, OPEN_DYSLEXIC};

use crate::client::{ClientUpdate, DataClient};
use crate::copy;
use crate::roles::{
    expire_timed_leases, format_active_roles, format_active_roles_html, redirect_if_lacking_access,
    touch_active_leases,
};
use crate::router::{
    can_access_subview, escape_up, format_hash, has_explicit_route, parse_hash,
    percent_decode_lossy,
};

/// Browser `localStorage` key storing the `Hurmit` / `OpenDyslexicM` preference.
pub const FONT_STORAGE_KEY: &str = "muconsole.dyslexia_font";
/// Browser `localStorage` key storing the signed-in `Html` / `Wireframe` skin preference.
pub const SKIN_STORAGE_KEY: &str = "muconsole.skin";
/// Browser `localStorage` key storing the active selected repository/submodule.
pub const REPO_STORAGE_KEY: &str = "muconsole.active_repo";
/// Browser `sessionStorage` key holding an onboarding `?invite=<token>` stripped before OAuth.
pub const PENDING_INVITE_STORAGE_KEY: &str = "muconsole.pending_invite";
/// Browser `sessionStorage` key holding a protected deep-link hash across OAuth login.
pub const PENDING_HASH_STORAGE_KEY: &str = "muconsole.pending_hash";
/// Repo selector settle debounce delay in milliseconds.
pub const ROTARY_SETTLE_MS: u64 = 350;
/// Filter query settle debounce delay in milliseconds.
pub const FILTER_SETTLE_MS: u64 = 250;

/// Canonical 8 console areas rendered in top navigation.
pub const AREAS: [Area; 8] = Area::ALL;

/// Semantic DOM attributes recognized by delegated event listeners in `crate::boot`.
pub const ACTION_ATTRS: [&str; 22] = [
    "data-control-id",
    "data-action",
    "data-area",
    "data-subview",
    "data-page-step",
    "data-repo-step",
    "data-dyslexia-toggle",
    "data-skin-toggle",
    "data-restart-job",
    "data-trace-id",
    "data-cd-gate",
    "data-ecosystem",
    "data-package",
    "data-entity-id",
    "data-purepath-filter",
    "data-select-trace",
    "data-span-id",
    "data-log-severity",
    "data-role-id",
    "data-credential-id",
    "data-subsystem",
    "data-external-href",
];

/// Browser WebAuthn ceremony queued for execution by `crate::boot`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebAuthnRequest {
    RegisterPasskey {
        label: Option<String>,
        requires_step_up: bool,
    },
    ElevateTimedRole {
        role_id: String,
    },
    RenewTimedRole {
        role_id: String,
    },
    RemovePasskey {
        credential_id: String,
    },
}

/// Editable draft inputs maintained by the UI for `@user` profile, A|B rollback note,
/// and `Area::Platform` administration controls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformFormState {
    pub selected_subsystem: SubsystemId,
    pub maintenance_mode: MaintenanceMode,
    pub maintenance_reason: String,
    pub maintenance_auto_clear_secs: Option<u64>,
    pub invite_note: String,
    pub invite_ttl_secs: u64,
    pub selected_user_login: Option<String>,
    pub user_roles_draft: String,
    pub role_id_draft: String,
    pub role_display_draft: String,
    pub folio_id_draft: String,
    pub folio_policies_draft: String,
    pub anonymous_lockdown_draft: bool,
    pub ab_rollback_note: String,
}

impl Default for PlatformFormState {
    fn default() -> Self {
        Self {
            selected_subsystem: SubsystemId::Ci,
            maintenance_mode: MaintenanceMode::Auto,
            maintenance_reason: String::new(),
            maintenance_auto_clear_secs: None,
            invite_note: String::new(),
            invite_ttl_secs: 86_400,
            selected_user_login: None,
            user_roles_draft: String::new(),
            role_id_draft: String::new(),
            role_display_draft: String::new(),
            folio_id_draft: String::new(),
            folio_policies_draft: String::new(),
            anonymous_lockdown_draft: false,
            ab_rollback_note: String::new(),
        }
    }
}

const fn subview_uses_server_filter(subview: SubView) -> bool {
    matches!(
        subview,
        SubView::Observability(mu_console_protocol::ObservabilitySubView::Logs)
            | SubView::Ci(CiSubView::QueueLogs)
    )
}

fn canonical_service_group(compact: &str) -> Option<&'static str> {
    match compact {
        "muregist" | "muregistry" | "muregistdaemon" | "muregistrydaemon" => Some("mu-registry"),
        "muobserve" | "muobservecollector" | "muobservehub" | "muobservable"
        | "muobservablehub" | "muobservablecli" | "muobservablemcp" => Some("mu-observable"),
        "actionrunnercontroller" | "mucicontroller" => Some("mu-ci-controller"),
        "actionrunnerrs" | "actionrunnerworker" | "muciworker" | "actionrunner" => {
            Some("mu-ci-worker")
        }
        "actionrunnerweb" | "muconsole" => Some("mu-console"),
        "mucondecontroller" | "mucdcontroller" => Some("mu-cd-controller"),
        "mucondeagent" | "mucdagent" => Some("mu-cd-agent"),
        "muisolation" => Some("mu-isolation"),
        _ => None,
    }
}

fn compact_alnum(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Case-insensitive multi-term substring match with canonical service alias and
/// hyphen/underscore normalization used by `#muconsole-filter`.
#[must_use]
pub fn service_filter_matches(filter: &str, candidate: &str) -> bool {
    let q = filter.trim();
    if q.is_empty() {
        return true;
    }
    let hay_lower = candidate.to_ascii_lowercase();
    let candidate_tokens: Vec<(String, Option<&'static str>)> = candidate
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .filter(|part| !part.is_empty())
        .map(|part| {
            let word_compact = compact_alnum(part);
            let group = canonical_service_group(&word_compact);
            (word_compact, group)
        })
        .filter(|(word_compact, _)| !word_compact.is_empty())
        .collect();

    q.split_whitespace().all(|tok| {
        let tok_lower = tok.to_ascii_lowercase();
        if hay_lower.contains(&tok_lower) {
            return true;
        }
        let tok_compact = compact_alnum(tok);
        if tok_compact.is_empty() {
            return false;
        }
        let tok_group = canonical_service_group(&tok_compact);
        candidate_tokens.iter().any(|(word_compact, word_group)| {
            *word_compact == tok_compact || (tok_group.is_some() && tok_group == *word_group)
        })
    })
}

/// Match full ref name (`refs/heads/foo`) against bare branch names (`foo`).
#[must_use]
pub fn branch_matches_ref(branch: &str, branch_ref: &str) -> bool {
    let a = branch.trim_start_matches("refs/heads/");
    let b = branch_ref.trim_start_matches("refs/heads/");
    !a.is_empty() && a == b
}

/// Return `true` when clicking inside an interactive action boundary (`ActionControl`).
#[must_use]
pub fn is_action_root_boundary(element_id: &str) -> bool {
    !element_id.is_empty() && !is_passive_container_control_id(element_id)
}

/// Passive layout wrapper IDs ignored when bubbling click events.
#[must_use]
pub fn is_passive_container_control_id(control_id: &str) -> bool {
    matches!(
        control_id,
        "c2-top-bar"
            | "muconsole-shell"
            | "muconsole-header"
            | "muconsole-dashboard"
            | "muconsole-main"
    )
}

/// Extract `?invite=<token>` from a URL query string (`?a=1&invite=tok&b=2`) and return
/// `(Some(token), cleaned_query)` with `invite` stripped out cleanly.
#[must_use]
pub fn extract_invite_token(query: &str) -> (Option<String>, String) {
    let trimmed = query.trim();
    let raw = trimmed.strip_prefix('?').unwrap_or(trimmed);
    if raw.is_empty() {
        return (None, String::new());
    }
    let mut found = None;
    let mut kept: Vec<&str> = Vec::new();
    for pair in raw.split('&').filter(|p| !p.is_empty()) {
        if let Some((k, v)) = pair.split_once('=') {
            if k == "invite" {
                if !v.is_empty() && found.is_none() {
                    found = Some(v.to_owned());
                }
                continue;
            }
        }
        kept.push(pair);
    }
    let clean = if kept.is_empty() {
        String::new()
    } else {
        format!("?{}", kept.join("&"))
    };
    (found, clean)
}

/// RFC 3986 percent-encode arbitrary strings for query parameters (`rd=`).
#[must_use]
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(*b as char);
        } else {
            use std::fmt::Write;
            let _ = write!(out, "%{:02X}", b);
        }
    }
    out
}

fn is_non_oauth_root_relative_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && path != "/oauth2"
        && !path.starts_with("/oauth2/")
        && !path.starts_with("/oauth2?")
}

fn normalize_oauth_return_path(rd: &str) -> String {
    if rd.contains('\r') || rd.contains('\n') {
        return "/".to_owned();
    }
    let trimmed = rd.trim();
    if trimmed.is_empty() {
        return "/".to_owned();
    }
    if trimmed.starts_with('#') || trimmed.starts_with('?') {
        return format!("/{trimmed}");
    }
    if !is_non_oauth_root_relative_path(trimmed) {
        return "/".to_owned();
    }
    trimmed.to_owned()
}

/// Construct a validated `/oauth2/start?rd=...` redirect URL.
#[must_use]
pub fn format_sign_in_url(rd: &str) -> String {
    let normalized = normalize_oauth_return_path(rd);
    format!("/oauth2/start?rd={}", percent_encode(&normalized))
}

/// Construct a validated `/oauth2/sign_out?rd=...` sign-out URL.
#[must_use]
pub fn format_sign_out_url(rd: &str) -> String {
    let normalized = normalize_oauth_return_path(rd);
    format!("/oauth2/sign_out?rd={}", percent_encode(&normalized))
}

/// Reject malformed sign-in URLs (missing `rd=`, unencoded parameters, empty or non-root-relative `rd`).
pub fn validate_sign_in_href(href: &str) -> Result<(), &'static str> {
    let (path, query) = href.split_once('?').ok_or("missing query string")?;
    if path != "/oauth2/start" {
        return Err("invalid path, expected /oauth2/start");
    }
    if !query.starts_with("rd=") {
        if query.starts_with("rd") {
            return Err("malformed rd parameter missing '='");
        }
        return Err("missing rd parameter");
    }
    let rd_val = &query["rd=".len()..];
    if rd_val.is_empty() {
        return Err("empty rd parameter");
    }
    if rd_val.contains('&') || rd_val.contains('=') {
        return Err("unencoded nested delimiter in rd parameter");
    }
    let decoded = percent_decode_lossy(rd_val);
    if decoded.contains('\r')
        || decoded.contains('\n')
        || !is_non_oauth_root_relative_path(&decoded)
    {
        return Err("rd parameter must be a root-relative non-oauth2 path");
    }
    Ok(())
}

fn find_query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    let raw = query.trim().strip_prefix('?').unwrap_or(query.trim());
    for pair in raw.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == key {
                return Some(v);
            }
        }
    }
    None
}

/// Resolve `CrtFont` from URL query (`?font=` / `?dyslexic=`) with `localStorage` fallback.
#[must_use]
pub fn parse_font_pref(query: &str, stored: Option<&str>) -> CrtFont {
    if let Some(val) = find_query_param(query, "font") {
        let lower = val.to_ascii_lowercase();
        if lower == "dyslexic" || lower == "opendyslexic" || lower == "opendyslexicm" {
            return CrtFont::OpenDyslexic;
        }
        if lower == "hurmit" || lower == "mono" {
            return CrtFont::Hurmit;
        }
    }
    if let Some(val) = find_query_param(query, "dyslexic") {
        if val == "1" || val.eq_ignore_ascii_case("true") {
            return CrtFont::OpenDyslexic;
        }
        if val == "0" || val.eq_ignore_ascii_case("false") {
            return CrtFont::Hurmit;
        }
    }
    if let Some(s) = stored {
        let lower = s.trim().to_ascii_lowercase();
        if lower == "1" || lower == "true" || lower.contains("dyslexic") {
            return CrtFont::OpenDyslexic;
        }
    }
    CrtFont::Hurmit
}

/// Canonical storage/DOM slug for [`CrtFont`].
#[must_use]
pub const fn font_slug(font: CrtFont) -> &'static str {
    match font {
        CrtFont::Hurmit => "hurmit",
        CrtFont::OpenDyslexic => "opendyslexic",
    }
}

/// Resolve `ConsoleSkin` preference from `?skin=html|wireframe` or `localStorage`
/// (defaulting to [`ConsoleSkin::Html`]; anonymous sessions still enforce `Html` via
/// [`ConsoleState::effective_skin`]).
#[must_use]
pub fn parse_skin_pref(query: &str, stored: Option<&str>) -> ConsoleSkin {
    if let Some(val) = find_query_param(query, "skin") {
        if val.eq_ignore_ascii_case("wireframe") {
            return ConsoleSkin::Wireframe;
        }
        if val.eq_ignore_ascii_case("html") {
            return ConsoleSkin::Html;
        }
    }
    if let Some(s) = stored {
        if s.trim().eq_ignore_ascii_case("wireframe") {
            return ConsoleSkin::Wireframe;
        }
    }
    ConsoleSkin::Html
}

/// Canonical storage/DOM slug for [`ConsoleSkin`].
#[must_use]
pub const fn skin_slug(skin: ConsoleSkin) -> &'static str {
    skin.as_str()
}

/// Stateful client model bridging worker `ViewModel` snapshots/deltas, local modal workflows,
/// skin/font persistence, `Timed` lease timers, and DOM input dispatch.
#[derive(Debug)]
pub struct ConsoleState {
    pub client: DataClient,
    pub route: Route,
    pub font: CrtFont,
    pub skin: ConsoleSkin,
    pub url_skin_override: Option<ConsoleSkin>,
    pub viewport: Viewport,
    pub safe_area: SafeArea,
    pub lazy_dyslexic_loaded: bool,
    pub reduced_motion: bool,
    pub outer_index: usize,
    pub inner_index: usize,
    pub filter_query: String,
    pub filter_focused: bool,
    pub pending_select_at_ms: Option<u64>,
    pub pending_filter_at_ms: Option<u64>,
    pub phone_rotary_open: bool,
    pub carousel_page: u32,
    pub yard_page: u32,
    pub subview_page: u32,
    pub log_page: u32,
    pub follow_log: bool,
    pub ci_queue_filter: u8,
    pub selected_stage_idx: Option<usize>,
    pub selected_history_idx: usize,
    pub selected_test_idx: usize,
    pub selected_metric_idx: usize,
    pub obs_smartscape_filter: u8,
    pub selected_sandbox_idx: Option<usize>,
    pub reg_ecosystem_filter: Option<PackageEcosystem>,
    pub selected_package: Option<String>,
    pub selected_entity_id: Option<String>,
    pub purepath_filter: String,
    pub selected_trace_id: Option<String>,
    pub selected_span_id: Option<String>,
    pub selected_node_id: Option<String>,
    pub log_severity_filter: Option<LogSeverity>,
    pub obs_service_filter: Option<String>,
    pub rendered_ci_log_page: Cell<u32>,
    pub rendered_obs_log_page: Cell<u32>,
    pub aria_polite: String,
    pub aria_assertive: String,
    pub pending_invite: Option<String>,
    pub pending_navigation: Option<String>,
    pub pending_external_url: Option<String>,
    pub pending_webauthn: Option<WebAuthnRequest>,
    pub webauthn_error: Option<String>,
    pub stashed_deep_link: Option<String>,
    pub has_explicit_hash: bool,
    pub route_sync_dirty: bool,
    pub prefs_dirty: bool,
    pub local_dialog: Option<DialogModel>,
    session_roles_worker_synced: bool,
    pub passkey_gate_dismissed: bool,
    pub profile_tab: UserProfileTab,
    pub profile_display_name_draft: String,
    pub profile_passkey_label_draft: String,
    pub platform_form: PlatformFormState,
    prev_auth_status: AuthStatus,
    prev_github_login: Option<String>,
    prev_lease_ids: Vec<String>,
    last_lease_tick_ms: u64,
    last_activity_at_ms: u64,
}

impl Default for ConsoleState {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsoleState {
    /// Construct initial fail-closed state (`ConsoleSkin::Html` default, unresolved auth).
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: DataClient::new(),
            route: Route::default(),
            font: CrtFont::Hurmit,
            skin: ConsoleSkin::Html,
            url_skin_override: None,
            viewport: Viewport::new(1280.0, 800.0),
            safe_area: SafeArea::ZERO,
            lazy_dyslexic_loaded: false,
            reduced_motion: false,
            outer_index: 0,
            inner_index: 0,
            filter_query: String::new(),
            filter_focused: false,
            pending_select_at_ms: None,
            pending_filter_at_ms: None,
            phone_rotary_open: false,
            carousel_page: 0,
            yard_page: 0,
            subview_page: 0,
            log_page: 0,
            follow_log: true,
            ci_queue_filter: 0,
            selected_stage_idx: None,
            selected_history_idx: 0,
            selected_test_idx: 0,
            selected_metric_idx: 0,
            obs_smartscape_filter: 0,
            selected_sandbox_idx: None,
            reg_ecosystem_filter: None,
            selected_package: None,
            selected_entity_id: None,
            purepath_filter: String::new(),
            selected_trace_id: None,
            selected_span_id: None,
            selected_node_id: None,
            log_severity_filter: None,
            obs_service_filter: None,
            rendered_ci_log_page: Cell::new(0),
            rendered_obs_log_page: Cell::new(0),
            aria_polite: String::new(),
            aria_assertive: String::new(),
            pending_invite: None,
            pending_navigation: None,
            pending_external_url: None,
            pending_webauthn: None,
            webauthn_error: None,
            stashed_deep_link: None,
            has_explicit_hash: false,
            route_sync_dirty: false,
            prefs_dirty: false,
            local_dialog: None,
            session_roles_worker_synced: false,
            passkey_gate_dismissed: false,
            profile_tab: UserProfileTab::UserDetails,
            profile_display_name_draft: String::new(),
            profile_passkey_label_draft: "Security Key".into(),
            platform_form: PlatformFormState::default(),
            prev_auth_status: AuthStatus::Unresolved,
            prev_github_login: None,
            prev_lease_ids: Vec::new(),
            last_lease_tick_ms: 0,
            last_activity_at_ms: 0,
        }
    }

    /// Initialize route, font preference, and `?invite=` token capture from initial URL/storage.
    pub fn init_from_env(&mut self, hash: &str, search: &str, stored_font: Option<&str>) {
        self.apply_hash(hash);
        self.font = parse_font_pref(search, stored_font);
        if find_query_param(search, "font").is_some()
            || find_query_param(search, "dyslexic").is_some()
            || stored_font.is_some_and(|s| !s.trim().is_empty())
        {
            self.prefs_dirty = true;
        }
        if self.font == CrtFont::OpenDyslexic {
            self.lazy_dyslexic_loaded = true;
        }
        let (invite, _clean) = extract_invite_token(search);
        if invite.is_some() {
            self.pending_invite = invite;
        }
        if let Some(q) = self.route.filter.as_deref() {
            self.filter_query = q.to_owned();
        }
    }

    /// Restore skin override (`?skin=html|wireframe` or `localStorage`), remaining locked to `Html`
    /// while unauthenticated.
    pub fn restore_stored_skin(&mut self, search: &str, stored_skin: Option<&str>) {
        if let Some(val) = find_query_param(search, "skin") {
            if val.eq_ignore_ascii_case("wireframe") {
                self.url_skin_override = Some(ConsoleSkin::Wireframe);
            } else if val.eq_ignore_ascii_case("html") {
                self.url_skin_override = Some(ConsoleSkin::Html);
            }
            self.prefs_dirty = true;
        } else if stored_skin.is_some_and(|s| !s.trim().is_empty()) {
            self.prefs_dirty = true;
        }
        self.skin = parse_skin_pref(search, stored_skin);
    }

    /// Parse and store URL hash route.
    pub fn apply_hash(&mut self, hash: &str) {
        self.has_explicit_hash = has_explicit_route(hash);
        let parsed = parse_hash(hash);
        self.route = parsed;
        if let Some(q) = self.route.filter.as_deref() {
            self.filter_query = q.to_owned();
        }
        if let Some(t) = self.route.trace_id.as_deref() {
            self.selected_trace_id = Some(t.to_owned());
        }
        self.sync_rotary_from_route();
    }

    /// Restore previously persisted repo selection (`owner/repo` or `owner/repo@submodule`).
    pub fn restore_stored_repo(&mut self, stored_repo: Option<&str>) {
        let Some(raw) = stored_repo.map(str::trim).filter(|s| !s.is_empty()) else {
            return;
        };
        let (repo_part, sub_part) = match raw.split_once('@') {
            Some((r, s)) if !s.is_empty() => (r, Some(SubmoduleId { path: s.to_owned() })),
            _ => (raw, None),
        };
        if let Some((owner, repo)) = repo_part.split_once('/') {
            if !owner.is_empty() && !repo.is_empty() {
                self.route.repo = Some(RepoId::new(owner, repo));
                self.route.submodule = sub_part;
                self.sync_rotary_from_route();
            }
        }
    }

    /// Format active repository selection for `localStorage` persistence.
    #[must_use]
    pub fn stored_repo_value(&self) -> Option<String> {
        let repo = self.route.repo.as_ref()?;
        match &self.route.submodule {
            Some(sub) => Some(format!("{}/{}@{}", repo.owner, repo.repo, sub.path)),
            None => Some(format!("{}/{}", repo.owner, repo.repo)),
        }
    }

    /// Return current [`AuthState`] (defaults to fail-closed [`AuthState::unresolved`] until hydrated).
    #[must_use]
    pub fn auth(&self) -> AuthState {
        self.client
            .view()
            .map(|v| v.auth.clone())
            .unwrap_or_else(AuthState::unresolved)
    }

    /// Return the latest worker error when cold-boot session verification has failed.
    #[must_use]
    pub fn session_probe_error(&self) -> Option<&mu_console_protocol::UiError> {
        if self.auth().is_resolved() {
            None
        } else {
            self.client.last_error()
        }
    }

    /// Return active skin enforced for the session (`Html` unconditionally unless `Active`).
    #[must_use]
    pub fn effective_skin(&self) -> ConsoleSkin {
        if self.auth().status != AuthStatus::Active {
            return ConsoleSkin::Html;
        }
        self.url_skin_override.unwrap_or(self.skin)
    }

    /// Return `true` when session is an active bound operator.
    #[must_use]
    pub fn is_operator(&self) -> bool {
        self.auth().is_operator()
    }

    /// Check whether `area` is enabled for the active session.
    #[must_use]
    pub fn can_access_area(&self, area: Area) -> bool {
        crate::router::effective_can_access_area(&self.auth(), area)
    }

    /// Check whether `subview` satisfies minimal active-role access requirements.
    #[must_use]
    pub fn can_access_subview(&self, subview: SubView) -> bool {
        can_access_subview(&self.auth(), subview)
    }

    /// Return `true` when `#muconsole-filter` applies to the currently visible view.
    #[must_use]
    pub fn is_filter_applicable(&self) -> bool {
        let auth = self.auth();
        auth.is_resolved()
            && !auth.anonymous_lockdown
            && can_access_subview(&auth, self.route.subview)
    }

    /// Context-sensitive `#muconsole-filter` placeholder for the current auth state and sub-view.
    #[must_use]
    pub fn filter_placeholder_text(&self) -> &'static str {
        copy::filter_placeholder_for_view(
            &self.auth(),
            self.route.subview,
            self.is_filter_applicable(),
        )
    }

    /// Return `true` when any caller-bound role ID is absent from `active_roles`.
    #[must_use]
    pub fn has_dormant_bound_roles(&self) -> bool {
        let auth = self.auth();
        let is_dormant = |role_id: &str| {
            !auth
                .active_roles
                .iter()
                .any(|active| active.role_id == role_id)
        };
        auth.bound_roles.iter().any(|id| is_dormant(id))
            || auth
                .bound_role_specs
                .iter()
                .any(|spec| is_dormant(&spec.id))
    }

    /// Format active header role pill string bounded safely on UTF-8 char boundaries.
    #[must_use]
    pub fn format_header_roles(&self) -> String {
        let auth = self.auth();
        let max_chars = self
            .client
            .view()
            .map(|v| v.max_active_roles_header_chars)
            .unwrap_or(mu_console_protocol::DEFAULT_MAX_ACTIVE_ROLES_HEADER_CHARS);
        format_active_roles(&auth.active_roles, max_chars)
    }

    /// Format active header role pill as HTML with `[MM:SS][MM:SS]` wrapped in `[data-lease-timer]`.
    #[must_use]
    pub fn format_header_roles_html(&self) -> String {
        let auth = self.auth();
        let max_chars = self
            .client
            .view()
            .map(|v| v.max_active_roles_header_chars)
            .unwrap_or(mu_console_protocol::DEFAULT_MAX_ACTIVE_ROLES_HEADER_CHARS);
        format_active_roles_html(&auth.active_roles, max_chars)
    }

    /// Return sign-in URL preserving the current deep-link path/hash cleanly across OAuth.
    #[must_use]
    pub fn sign_in_href(&self) -> String {
        let default_return = if self.has_explicit_hash { "" } else { "/" };
        let return_to = self.stashed_deep_link.as_deref().unwrap_or(default_return);
        if return_to.is_empty() {
            let hash = format_hash(&self.route);
            format_sign_in_url(&format!("/{hash}"))
        } else {
            format_sign_in_url(return_to)
        }
    }

    /// Return canonical `/oauth2/sign_out?rd=/` URL.
    #[must_use]
    pub fn sign_out_href(&self) -> String {
        format_sign_out_url("/")
    }

    /// Return active monospace font metrics.
    #[must_use]
    pub const fn font_metrics(&self) -> &'static CrtFontMetrics {
        match self.font {
            CrtFont::Hurmit => &HURMIT,
            CrtFont::OpenDyslexic => &OPEN_DYSLEXIC,
        }
    }

    /// Update active font preference locally and emit `UiCommand::SetCrtFont`.
    pub fn set_font(&mut self, font: CrtFont) -> UiCommand {
        self.font = font;
        if font == CrtFont::OpenDyslexic {
            self.lazy_dyslexic_loaded = true;
        }
        self.prefs_dirty = true;
        UiCommand::SetCrtFont(font)
    }

    /// Toggle `Hurmit` <-> `OpenDyslexicM` and emit `UiCommand::SetCrtFont`.
    pub fn toggle_font(&mut self) -> UiCommand {
        let next = match self.font {
            CrtFont::Hurmit => CrtFont::OpenDyslexic,
            CrtFont::OpenDyslexic => CrtFont::Hurmit,
        };
        self.set_font(next)
    }

    /// Update skin preference (effective only when `Active`; unauthenticated remains `Html`).
    pub fn set_skin(&mut self, skin: ConsoleSkin) -> UiCommand {
        if self.auth().status == AuthStatus::Active {
            self.url_skin_override = Some(skin);
            self.skin = skin;
            self.prefs_dirty = true;
            UiCommand::SetSkin(skin)
        } else {
            self.skin = ConsoleSkin::Html;
            UiCommand::SetSkin(ConsoleSkin::Html)
        }
    }

    /// Toggle `Html` <-> `Wireframe` when `Active` (no-op returning `Html` otherwise).
    pub fn toggle_skin(&mut self) -> UiCommand {
        if self.auth().status != AuthStatus::Active {
            return self.set_skin(ConsoleSkin::Html);
        }
        let next = match self.effective_skin() {
            ConsoleSkin::Html => ConsoleSkin::Wireframe,
            ConsoleSkin::Wireframe => ConsoleSkin::Html,
        };
        self.set_skin(next)
    }

    /// Resolve active dialog (`UserProfile`, `RiskConfirm`, and `SessionRoles` reconcile cleanly
    /// with worker `ViewModel::active_dialog`; `PasskeyEnrollmentGate` auto-opens when
    /// `passkey_count == 0` unless dismissed via `Not Now` / `Escape`).
    #[must_use]
    pub fn active_dialog(&self) -> Option<DialogModel> {
        let worker_dlg = self.client.view().and_then(|v| v.active_dialog.as_ref());
        if let Some(ref local) = self.local_dialog {
            match local {
                DialogModel::SessionRoles(local_sr) => {
                    if let Some(DialogModel::InviteCreatedOnce(_)) = worker_dlg {
                        return worker_dlg.cloned();
                    }
                    let mut merged = match worker_dlg {
                        Some(DialogModel::SessionRoles(worker_sr)) => {
                            let mut sr = worker_sr.clone();
                            if sr.roles.is_empty() {
                                sr.roles.clone_from(&local_sr.roles);
                            }
                            if sr.error_message.is_none() {
                                sr.error_message.clone_from(&local_sr.error_message);
                            }
                            sr
                        }
                        _ => local_sr.clone(),
                    };
                    if merged.error_message.is_none() {
                        merged.error_message.clone_from(&self.webauthn_error);
                    }
                    return Some(DialogModel::SessionRoles(merged));
                }
                other => return Some(other.clone()),
            }
        }
        if let Some(worker_dlg) = worker_dlg {
            if let DialogModel::SessionRoles(worker_sr) = worker_dlg {
                let mut sr = worker_sr.clone();
                if sr.roles.is_empty() {
                    let auth = self.auth();
                    sr.roles = resolve_descriptors_from_view(&auth, self.client.view());
                }
                if sr.error_message.is_none() {
                    sr.error_message.clone_from(&self.webauthn_error);
                }
                return Some(DialogModel::SessionRoles(sr));
            }
            if !matches!(worker_dlg, DialogModel::PasskeyEnrollmentGate(_))
                || !self.passkey_gate_dismissed
            {
                return Some(worker_dlg.clone());
            }
        }
        let auth = self.auth();
        if !self.passkey_gate_dismissed
            && (auth.passkey_enrollment_required
                || auth.status == AuthStatus::PasskeyEnrollmentRequired)
        {
            return Some(DialogModel::PasskeyEnrollmentGate(
                PasskeyEnrollmentGateModel {
                    github_login: auth.github_login.unwrap_or_else(|| "operator".into()),
                    passkey_count: auth.passkey_count,
                    reason_code: auth.reason_code,
                },
            ));
        }
        None
    }

    /// Currently active CI job ID selected by route, train, or queue summary.
    #[must_use]
    pub fn active_job_id(&self) -> Option<u64> {
        if let Some(id) = self.route.job {
            return Some(id);
        }
        let view = self.client.view()?;
        if let Some(branch) = &self.route.branch {
            if let Some(topo) = &view.topology {
                if let Some(tr) = topo
                    .trains
                    .iter()
                    .find(|t| branch_matches_ref(branch, &t.ref_name))
                {
                    return Some(tr.job_id);
                }
            }
        }
        if let Some(first) = view.ci_jobs.first() {
            return Some(first.job_id);
        }
        view.repos.first().and_then(|r| r.latest_job_id)
    }

    /// Trigger CI job restart confirmation (`DialogModel::RiskConfirm`) if `ci:jobs.restart` held.
    pub fn restart_job(&mut self, job_id: u64) -> Option<UiCommand> {
        let auth = self.auth();
        if !auth.has_policy("ci:jobs.restart") {
            self.aria_polite = copy::AUTH_DENIED_STATUS.into();
            return None;
        }
        let repo = self
            .route
            .repo
            .clone()
            .or_else(|| {
                self.client.view().and_then(|v| {
                    v.ci_jobs
                        .iter()
                        .find(|j| j.job_id == job_id)
                        .map(|j| j.repo.clone())
                })
            })
            .unwrap_or_else(|| RepoId::new("LMA1980", "mu-console"));
        let job_id_str = format!("{job_id:06}");
        let dlg = DialogModel::RiskConfirm(RiskConfirmDialogModel {
            action: RiskConfirmAction::RestartCiJob {
                job_id,
                job_id_str: job_id_str.clone(),
                repo,
            },
            confirmation_phrase: Some(format!("RESTART {job_id_str}")),
            typed_confirmation: String::new(),
        });
        self.local_dialog = Some(dlg.clone());
        Some(UiCommand::OpenDialog(dlg))
    }

    /// Execute risk-free CI resume immediately in 1 click (`no risk -> no dialog`) when permitted.
    pub fn resume_job(&mut self, job_id: u64) -> Option<UiCommand> {
        if !self.auth().has_policy("ci:jobs.resume") {
            self.aria_polite = copy::AUTH_DENIED_STATUS.into();
            return None;
        }
        Some(UiCommand::ResumeJob { job_id })
    }

    /// Record a WebAuthn browser/endpoint ceremony error for accessible status & banner feedback.
    pub fn on_webauthn_error(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        self.aria_assertive = format!("Passkey ceremony failed: {msg}");
        self.webauthn_error = Some(msg.clone());
        if let Some(DialogModel::SessionRoles(ref mut sr)) = self.local_dialog {
            sr.error_message = Some(msg.clone());
        }
        if let Some(DialogModel::SessionRoles(ref mut sr)) = self
            .client
            .view_mut()
            .and_then(|v| v.active_dialog.as_mut())
        {
            sr.error_message = Some(msg);
        }
    }

    fn clear_session_roles_error(&mut self) {
        self.webauthn_error = None;
        if let Some(DialogModel::SessionRoles(ref mut sr)) = self.local_dialog {
            sr.error_message = None;
        }
        if let Some(DialogModel::SessionRoles(ref mut sr)) = self
            .client
            .view_mut()
            .and_then(|v| v.active_dialog.as_mut())
        {
            sr.error_message = None;
        }
    }

    /// Apply decoded postcard frame from the data worker and handle auth/expiry route transitions.
    pub fn on_worker_frame(&mut self, frame: &[u8]) -> ClientUpdate {
        let update = self.client.on_frame(frame);
        match update {
            ClientUpdate::View | ClientUpdate::ReloadRequired { .. } => {
                self.sync_from_view_model();
            }
            ClientUpdate::LeaseTimers => {
                self.sync_lease_timers_from_view_model();
            }
            _ => {}
        }
        update
    }

    fn sync_lease_timers_from_view_model(&mut self) {
        let Some(view) = self.client.view() else {
            return;
        };
        if let Some(DialogModel::SessionRoles(ref mut sr)) = self.local_dialog {
            sync_session_roles_model(sr, &view.auth, Some(view));
        }
    }

    fn sync_from_view_model(&mut self) {
        let Some(view) = self.client.view() else {
            return;
        };
        let next_status = view.auth.status;
        let next_lease_ids: Vec<String> =
            view.auth.leases.iter().map(|l| l.role_id.clone()).collect();
        let lost_timed = self
            .prev_lease_ids
            .iter()
            .any(|id| !next_lease_ids.iter().any(|cur| cur == id));

        match &view.active_dialog {
            Some(DialogModel::SessionRoles(_)) => {
                self.session_roles_worker_synced = true;
                if matches!(self.local_dialog, Some(DialogModel::SessionRoles(_))) {
                    self.local_dialog = None;
                }
            }
            Some(DialogModel::InviteCreatedOnce(_)) => {
                self.session_roles_worker_synced = false;
                self.local_dialog = None;
            }
            Some(_) => {
                self.session_roles_worker_synced = false;
            }
            None => {
                if self.client.last_frame_closed_dialog() || self.session_roles_worker_synced {
                    self.session_roles_worker_synced = false;
                    if matches!(self.local_dialog, Some(DialogModel::SessionRoles(_))) {
                        self.local_dialog = None;
                    }
                    self.webauthn_error = None;
                } else if let Some(DialogModel::SessionRoles(ref mut sr)) = self.local_dialog {
                    sync_session_roles_model(sr, &view.auth, Some(view));
                }
            }
        }

        if let Some(profile) = &view.user_profile {
            if let Some(dn) = &profile.display_name {
                if self.profile_display_name_draft.is_empty() {
                    self.profile_display_name_draft = dn.clone();
                }
            } else if self.profile_display_name_draft.is_empty() {
                self.profile_display_name_draft = profile.github_login.clone();
            }
            if !self.prefs_dirty && self.url_skin_override.is_none() {
                self.skin = profile.preferences.skin;
                self.font = profile.preferences.font;
                if self.font == CrtFont::OpenDyslexic {
                    self.lazy_dyslexic_loaded = true;
                }
            }
        }

        // Anonymous cold boot on `/` -> auto-route to CI > Queue & Job Steps (#50).
        if self.prev_auth_status == AuthStatus::Unresolved
            && matches!(
                next_status,
                AuthStatus::Anonymous | AuthStatus::AuthorizationDenied
            )
            && !view.auth.anonymous_lockdown
            && !self.has_explicit_hash
        {
            self.route = Route::for_subview(SubView::Ci(CiSubView::QueueLogs));
            self.route_sync_dirty = true;
        }

        // Post-login landing (#50): keep route if permitted, or fall back to Dashboard if
        // stashed deep link requires a dormant role.
        if next_status != AuthStatus::PasskeyEnrollmentRequired
            || (self.prev_auth_status != AuthStatus::Unresolved
                && self.prev_github_login != view.auth.github_login)
        {
            self.passkey_gate_dismissed = false;
        }
        let transitioned_to_active =
            self.prev_auth_status != AuthStatus::Active && next_status == AuthStatus::Active;
        if transitioned_to_active
            || !matches!(
                next_status,
                AuthStatus::PasskeyEnrollmentRequired | AuthStatus::Active
            )
        {
            self.webauthn_error = None;
        }
        if transitioned_to_active {
            if let Some(stashed) = self.stashed_deep_link.take() {
                let parsed = parse_hash(&stashed);
                self.route = parsed;
            }
            if redirect_if_lacking_access(&view.auth, &mut self.route) {
                self.route_sync_dirty = true;
            }
        } else if lost_timed && redirect_if_lacking_access(&view.auth, &mut self.route) {
            // Timed role 00:00 expiry redirect ONLY if remaining active roles lack access (#46/#53).
            self.route_sync_dirty = true;
        }

        self.prev_auth_status = next_status;
        self.prev_github_login.clone_from(&view.auth.github_login);
        self.prev_lease_ids = next_lease_ids;
        self.sync_rotary_from_route();
    }

    /// Return filtered repositories obeying public-only boundary unless private read policy held.
    #[must_use]
    pub fn filtered_repos(&self) -> Vec<RepoScore> {
        let auth = self.auth();
        if !auth.is_resolved() || auth.anonymous_lockdown {
            return Vec::new();
        }
        let allow_private =
            auth.status == AuthStatus::Active && auth.has_policy("ci:repos.private.read");
        let Some(view) = self.client.view() else {
            return Vec::new();
        };
        view.repos
            .iter()
            .filter(|r| allow_private || r.visibility == Visibility::Public)
            .filter(|r| {
                let hay = format!(
                    "{}/{} {} {:?}",
                    r.id.owner, r.id.repo, r.default_branch, r.status
                );
                service_filter_matches(&self.filter_query, &hay)
            })
            .cloned()
            .collect()
    }

    /// Order scorecards parent-first followed by submodules cleanly.
    #[must_use]
    pub fn ordered_carousel_repos(&self) -> Vec<RepoScore> {
        let mut repos = self.filtered_repos();
        let selected_repo = self.route.repo.as_ref().or_else(|| {
            self.client
                .view()
                .and_then(|v| v.selection.as_ref().map(|s| &s.repo))
        });
        let anchor_parent: Option<RepoId> = selected_repo.and_then(|sel| {
            let entry = repos.iter().find(|r| &r.id == sel)?;
            if !entry.submodules.is_empty() {
                Some(entry.id.clone())
            } else {
                entry.parents.first().map(|p| p.repo.clone())
            }
        });
        let sub_order: Vec<RepoId> = anchor_parent
            .as_ref()
            .and_then(|pid| repos.iter().find(|r| &r.id == pid))
            .map(|parent| parent.submodules.iter().map(|s| s.repo.clone()).collect())
            .unwrap_or_default();

        repos.sort_by_key(|r| {
            let (tier, order) = if anchor_parent.as_ref() == Some(&r.id) {
                (0_u8, 0_usize)
            } else if let Some(pos) = sub_order.iter().position(|id| id == &r.id) {
                (1_u8, pos)
            } else {
                let sev = match r.status {
                    RepoStatus::Failing => 0_usize,
                    RepoStatus::Running => 1,
                    RepoStatus::Unknown => 2,
                    RepoStatus::Passing => 3,
                };
                (2_u8, sev)
            };
            (tier, order, r.id.owner.clone(), r.id.repo.clone())
        });
        repos
    }

    /// Visible CI jobs obeying public-only filtering when unauthenticated or lacking private read.
    #[must_use]
    pub fn visible_ci_jobs(&self) -> Vec<CiJobSummary> {
        let auth = self.auth();
        if !auth.is_resolved() || auth.anonymous_lockdown {
            return Vec::new();
        }
        let allow_private =
            auth.status == AuthStatus::Active && auth.has_policy("ci:repos.private.read");
        let can_view_logs = auth.status == AuthStatus::Active
            && (auth.has_policy("ci:jobs.logs.read") || auth.has_policy("obs:logs.read"));
        let Some(view) = self.client.view() else {
            return Vec::new();
        };
        let public_repos: Vec<&RepoId> = view
            .repos
            .iter()
            .filter(|r| r.visibility == Visibility::Public)
            .map(|r| &r.id)
            .collect();
        view.ci_jobs
            .iter()
            .filter(|job| {
                if allow_private {
                    return true;
                }
                let matched_repo = view.repos.iter().find(|r| r.id == job.repo);
                matched_repo.is_none_or(|r| r.visibility == Visibility::Public)
                    && (public_repos.is_empty()
                        || public_repos.contains(&&job.repo)
                        || view.repos.iter().all(|r| r.id != job.repo))
            })
            .filter(|job| {
                let hay = format!(
                    "{}/{} #{} {} {} {}",
                    job.repo.owner,
                    job.repo.repo,
                    job.job_id,
                    job.ref_name,
                    job.job_id_str,
                    job.queue_state
                );
                service_filter_matches(&self.filter_query, &hay)
                    || (can_view_logs
                        && job
                            .raw_log_lines
                            .iter()
                            .any(|line| service_filter_matches(&self.filter_query, line)))
            })
            .cloned()
            .collect()
    }

    /// Align rotary outer/inner indices with `self.route`.
    pub fn sync_rotary_from_route(&mut self) {
        let repos = self.filtered_repos();
        if repos.is_empty() {
            self.outer_index = 0;
            self.inner_index = 0;
            return;
        }
        if let Some(target) = &self.route.repo {
            if let Some(idx) = repos.iter().position(|r| &r.id == target) {
                self.outer_index = idx;
            }
        } else {
            self.outer_index = self.outer_index.min(repos.len().saturating_sub(1));
        }
        let subs_len = repos
            .get(self.outer_index)
            .map_or(0, |r| r.submodules.len());
        if let Some(sub) = &self.route.submodule {
            if let Some(idx) = repos
                .get(self.outer_index)
                .and_then(|r| r.submodules.iter().position(|s| s.path == sub.path))
            {
                self.inner_index = idx + 1;
            } else {
                self.inner_index = 0;
            }
        } else if subs_len == 0 {
            self.inner_index = 0;
        }
    }

    /// Cycle selected parent repository.
    pub fn rotate_outer(&mut self, delta: isize, now_ms: u64) {
        let repos = self.filtered_repos();
        if repos.is_empty() {
            return;
        }
        let len = isize::try_from(repos.len()).unwrap_or(1);
        let cur = isize::try_from(self.outer_index).unwrap_or(0);
        let next = usize::try_from((cur + delta).rem_euclid(len)).unwrap_or(0);
        self.outer_index = next;
        self.inner_index = 0;
        self.route.repo = Some(repos[next].id.clone());
        self.route.submodule = None;
        self.route.branch = None;
        self.route.job = None;
        self.pending_select_at_ms = Some(now_ms.saturating_add(ROTARY_SETTLE_MS));
    }

    /// Cycle selected submodule within the current parent repo (`0` = root).
    pub fn rotate_inner(&mut self, delta: isize, now_ms: u64) {
        let repos = self.filtered_repos();
        let Some(repo) = repos.get(self.outer_index) else {
            return;
        };
        let total = repo.submodules.len() + 1;
        let len = isize::try_from(total).unwrap_or(1);
        let cur = isize::try_from(self.inner_index).unwrap_or(0);
        let next = usize::try_from((cur + delta).rem_euclid(len)).unwrap_or(0);
        self.inner_index = next;
        self.route.submodule = if next == 0 {
            None
        } else {
            repo.submodules.get(next - 1).map(|s| SubmoduleId {
                path: s.path.clone(),
            })
        };
        self.pending_select_at_ms = Some(now_ms.saturating_add(ROTARY_SETTLE_MS));
    }

    /// Distinct service names visible in `Observability`.
    #[must_use]
    pub fn observed_services(&self) -> Vec<String> {
        let Some(view) = self.client.view() else {
            return Vec::new();
        };
        let mut names: Vec<String> = view
            .observe
            .smartscape_nodes
            .iter()
            .map(|n| n.label.clone())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Cycle service filter inside `Observability`.
    pub fn rotate_obs_service(&mut self, delta: isize) {
        let services = self.observed_services();
        if services.is_empty() {
            self.obs_service_filter = None;
            return;
        }
        let total = services.len() + 1;
        let cur_idx = self
            .obs_service_filter
            .as_deref()
            .and_then(|cur| services.iter().position(|s| s == cur).map(|i| i + 1))
            .unwrap_or(0);
        let next = usize::try_from(
            (isize::try_from(cur_idx).unwrap_or(0) + delta)
                .rem_euclid(isize::try_from(total).unwrap_or(1)),
        )
        .unwrap_or(0);
        self.obs_service_filter = if next == 0 {
            None
        } else {
            services.get(next - 1).cloned()
        };
    }

    fn subview_nav_command(&self, subview: SubView) -> UiCommand {
        if self.route.filter.is_some() && subview_uses_server_filter(subview) {
            UiCommand::Subscribe(self.route.clone())
        } else {
            UiCommand::SelectSubView(subview)
        }
    }

    /// Update `#muconsole-filter` query string cleanly.
    pub fn set_filter_query(&mut self, query: &str, now_ms: u64) {
        self.filter_query = query.to_owned();
        self.route.filter = if query.trim().is_empty() {
            None
        } else {
            Some(query.to_owned())
        };
        self.carousel_page = 0;
        self.subview_page = 0;
        self.pending_filter_at_ms = Some(now_ms.saturating_add(FILTER_SETTLE_MS));
    }

    /// Switch to an area only if active session holds minimal access (`disabled` rule).
    pub fn select_area(&mut self, area: Area) -> UiCommand {
        if !self.can_access_area(area) {
            self.aria_polite = copy::announce_disabled_area(area);
            return UiCommand::Subscribe(self.route.clone());
        }
        let default_sub = SubView::default_for(area);
        let sub = if self.can_access_subview(default_sub) {
            default_sub
        } else {
            SubView::all_for(area)
                .iter()
                .copied()
                .find(|s| self.can_access_subview(*s))
                .unwrap_or(default_sub)
        };
        self.route.area = area;
        self.route.subview = sub;
        self.has_explicit_hash = true;
        self.route_sync_dirty = true;
        self.subview_page = 0;
        self.aria_polite = copy::announce_route(area, sub.label());
        self.subview_nav_command(sub)
    }

    /// Switch directly to `subview` only when permitted for the active session.
    pub fn select_subview(&mut self, subview: SubView) -> Vec<UiCommand> {
        if !self.can_access_subview(subview) {
            self.aria_polite = copy::announce_disabled_area(subview.area());
            return Vec::new();
        }
        self.route.area = subview.area();
        self.route.subview = subview;
        self.has_explicit_hash = true;
        self.route_sync_dirty = true;
        self.subview_page = 0;
        self.aria_polite = copy::announce_route(subview.area(), subview.label());
        vec![self.subview_nav_command(subview)]
    }

    /// Cross-navigate to a PurePath trace when Observability read is permitted.
    pub fn open_trace(&mut self, trace_id: &str) -> Vec<UiCommand> {
        let target = SubView::Observability(mu_console_protocol::ObservabilitySubView::PurePath);
        if !self.can_access_subview(target) {
            self.aria_polite = copy::announce_disabled_area(Area::Observability);
            return Vec::new();
        }
        self.route.trace_id = Some(trace_id.to_owned());
        self.selected_trace_id = Some(trace_id.to_owned());
        self.select_subview(target)
    }

    /// Cross-navigate to `A|B Deployment > Integration Gate` when permitted.
    pub fn open_cd_gate(&mut self, deployment_id: Option<&str>) -> Vec<UiCommand> {
        let target = SubView::AbDeploy(AbDeploySubView::IntegrationGate);
        if !self.can_access_subview(target) {
            self.aria_polite = copy::announce_disabled_area(Area::AbDeploy);
            return Vec::new();
        }
        if let Some(id) = deployment_id {
            self.selected_entity_id = Some(id.to_owned());
        }
        self.select_subview(target)
    }

    /// Update viewport & safe area geometry.
    pub fn set_viewport(&mut self, viewport: Viewport, safe_area: SafeArea) {
        self.viewport = viewport;
        self.safe_area = safe_area;
    }

    /// Scorecard cards fitted per carousel page on the current viewport.
    #[must_use]
    pub fn scorecard_cards_per_page(&self) -> usize {
        let solved = mu_console_layout::solve(
            self.viewport,
            self.safe_area,
            self.route.area,
            self.font_metrics(),
        );
        scorecard_page_capacity(solved.active_canvas, solved.breakpoint, self.font_metrics())
    }

    /// Step scorecard carousel pagination cleanly.
    pub fn step_carousel(&mut self, delta: i32, cards_per_page: usize) -> UiCommand {
        let total = self.ordered_carousel_repos().len().max(1);
        let per_page = cards_per_page.max(1);
        let total_pages = total.div_ceil(per_page).max(1);
        let cur = i32::try_from(self.carousel_page).unwrap_or(0);
        let next = (cur + delta).rem_euclid(i32::try_from(total_pages).unwrap_or(1));
        self.carousel_page = u32::try_from(next).unwrap_or(0);
        UiCommand::Page {
            panel: mu_console_protocol::PanelId::Carousel,
            cursor: self.carousel_page,
        }
    }

    /// Step active view pagination.
    pub fn step_active_page(&mut self, delta: isize) {
        if delta < 0 {
            self.subview_page = self
                .subview_page
                .saturating_sub(delta.unsigned_abs() as u32);
        } else {
            self.subview_page = self.subview_page.saturating_add(delta as u32);
        }
    }

    /// Explicit user pointer/keyboard activity resets idle timers locally and notifies the worker.
    ///
    /// Background SSE updates and ticks never call this.
    pub fn on_explicit_user_activity(&mut self, now_ms: u64) -> Vec<UiCommand> {
        self.last_activity_at_ms = now_ms;
        let auth = self.client.view_mut();
        if let Some(view) = auth {
            if touch_active_leases(&mut view.auth, now_ms) {
                return vec![UiCommand::RecordUserActivity];
            }
        }
        Vec::new()
    }

    fn select_profile_font(&mut self, font: CrtFont) -> Vec<UiCommand> {
        let status = self.auth().status;
        if !matches!(
            status,
            AuthStatus::Active | AuthStatus::PasskeyEnrollmentRequired
        ) {
            return Vec::new();
        }
        let cmd = self.set_font(font);
        if status == AuthStatus::Active {
            vec![
                cmd,
                UiCommand::UpdateUserProfile {
                    display_name: None,
                    preferred_skin: None,
                    preferred_font: Some(font),
                },
            ]
        } else {
            vec![cmd]
        }
    }

    /// Open `@user` Profile modal (`User Details | Preferences | Passkeys`).
    pub fn open_user_profile(&mut self, tab: UserProfileTab) -> Vec<UiCommand> {
        let auth = self.auth();
        if !matches!(
            auth.status,
            AuthStatus::Active | AuthStatus::PasskeyEnrollmentRequired
        ) {
            return Vec::new();
        }
        if auth.status == AuthStatus::PasskeyEnrollmentRequired {
            self.passkey_gate_dismissed = true;
        }
        self.profile_tab = tab;
        let profile = if let Some(p) = self.client.view().and_then(|v| v.user_profile.as_ref()) {
            if self.profile_display_name_draft.is_empty() {
                self.profile_display_name_draft = p
                    .display_name
                    .clone()
                    .unwrap_or_else(|| p.github_login.clone());
            }
            p.clone()
        } else {
            let login = auth.github_login.unwrap_or_else(|| "operator".into());
            if self.profile_display_name_draft.is_empty() {
                self.profile_display_name_draft.clone_from(&login);
            }
            mu_console_protocol::UserProfileDto {
                github_login: login,
                display_name: (!self.profile_display_name_draft.is_empty())
                    .then(|| self.profile_display_name_draft.clone()),
                ..Default::default()
            }
        };
        let dlg = DialogModel::UserProfile(UserProfileDialogModel {
            profile,
            active_tab: tab,
            max_session_expires_epoch_ms: None,
        });
        self.local_dialog = Some(dlg.clone());
        vec![UiCommand::OpenDialog(dlg)]
    }

    /// Open `+role` Session Roles modal.
    pub fn open_session_roles(&mut self) -> Vec<UiCommand> {
        let auth = self.auth();
        if auth.status != AuthStatus::Active {
            return Vec::new();
        }
        self.webauthn_error = None;
        let mut roles = resolve_descriptors_from_view(&auth, self.client.view());
        if roles.is_empty() {
            if let Some(DialogModel::SessionRoles(sr)) =
                self.client.view().and_then(|v| v.active_dialog.as_ref())
            {
                roles.clone_from(&sr.roles);
            }
        }
        let dlg = DialogModel::SessionRoles(SessionRolesDialogModel {
            github_login: auth.github_login.unwrap_or_else(|| "operator".into()),
            roles,
            lock_all_enabled: !auth.leases.is_empty(),
            error_message: None,
        });
        self.session_roles_worker_synced = false;
        self.local_dialog = Some(dlg.clone());
        vec![UiCommand::OpenDialog(dlg)]
    }

    /// Complete a Passkey-confirmed timed role renewal with the WebAuthn assertion JSON envelope.
    pub fn complete_role_renewal(
        &mut self,
        role_id: impl Into<String>,
        assertion_json: impl Into<String>,
    ) -> Vec<UiCommand> {
        if self.auth().status != AuthStatus::Active {
            return Vec::new();
        }
        vec![UiCommand::RenewRoleWithPasskey {
            role_id: role_id.into(),
            assertion_json: assertion_json.into(),
        }]
    }

    /// Activate an interactive control target by ID (`data-control-id` / `data-action`).
    pub fn activate_target(&mut self, target: &str) -> Vec<UiCommand> {
        self.on_dom_action("data-control-id", target, false, 0)
    }

    /// Handle a DOM click on an interactive control or semantic `data-*` attribute.
    #[allow(clippy::too_many_lines)]
    pub fn on_dom_action(
        &mut self,
        attr_name: &str,
        attr_val: &str,
        _shift: bool,
        now_ms: u64,
    ) -> Vec<UiCommand> {
        match attr_name {
            "data-area" => {
                if let Some(area) = Area::from_slug(attr_val) {
                    if !self.can_access_area(area) {
                        self.aria_polite = copy::announce_disabled_area(area);
                        return Vec::new();
                    }
                    return vec![self.select_area(area)];
                }
                Vec::new()
            }
            "data-subview" => {
                if let Some((a_slug, sv_slug)) = attr_val.split_once(':') {
                    if let Some(area) = Area::from_slug(a_slug) {
                        if let Some(sv) = SubView::from_area_and_slug(area, sv_slug) {
                            return self.select_subview(sv);
                        }
                    }
                } else if let Some(sv) = SubView::from_area_and_slug(self.route.area, attr_val) {
                    return self.select_subview(sv);
                }
                Vec::new()
            }
            "data-repo-step" => match attr_val {
                "prev-outer" => {
                    self.rotate_outer(-1, now_ms);
                    Vec::new()
                }
                "next-outer" => {
                    self.rotate_outer(1, now_ms);
                    Vec::new()
                }
                "prev-inner" => {
                    self.rotate_inner(-1, now_ms);
                    Vec::new()
                }
                "next-inner" => {
                    self.rotate_inner(1, now_ms);
                    Vec::new()
                }
                _ => Vec::new(),
            },
            "data-dyslexia-toggle" => vec![self.toggle_font()],
            "data-skin-toggle" => vec![self.toggle_skin()],
            "data-external-href" => {
                if attr_val.starts_with("https://") || attr_val.starts_with("http://") {
                    self.pending_external_url = Some(attr_val.to_owned());
                }
                Vec::new()
            }
            "data-page-step" => match attr_val {
                "prev" => {
                    self.step_active_page(-1);
                    Vec::new()
                }
                "next" => {
                    self.step_active_page(1);
                    Vec::new()
                }
                _ => Vec::new(),
            },
            "data-trace-id" => self.open_trace(attr_val),
            "data-cd-gate" => self.open_cd_gate(Some(attr_val)),
            "data-ecosystem" => {
                self.reg_ecosystem_filter = match attr_val {
                    "cargo" => Some(PackageEcosystem::Cargo),
                    "nuget" => Some(PackageEcosystem::NuGet),
                    "maven" => Some(PackageEcosystem::Maven),
                    _ => None,
                };
                Vec::new()
            }
            "data-package" => {
                self.selected_package = Some(attr_val.to_owned());
                Vec::new()
            }
            "data-entity-id" => {
                self.selected_entity_id = Some(attr_val.to_owned());
                Vec::new()
            }
            "data-purepath-filter" => {
                self.purepath_filter = attr_val.to_owned();
                Vec::new()
            }
            "data-select-trace" => {
                self.selected_trace_id = Some(attr_val.to_owned());
                Vec::new()
            }
            "data-span-id" => {
                self.selected_span_id = Some(attr_val.to_owned());
                Vec::new()
            }
            "data-log-severity" => {
                self.log_severity_filter = match attr_val.to_ascii_lowercase().as_str() {
                    "info" => Some(LogSeverity::Info),
                    "warn" => Some(LogSeverity::Warn),
                    "error" => Some(LogSeverity::Error),
                    _ => None,
                };
                Vec::new()
            }
            "data-action" | "data-control-id" => self.handle_control_id(attr_val, now_ms),
            _ => Vec::new(),
        }
    }

    fn handle_control_id(&mut self, id: &str, now_ms: u64) -> Vec<UiCommand> {
        if let Some(area_slug) = id
            .strip_prefix("nav-area:")
            .or_else(|| id.strip_prefix("dash-card:"))
        {
            return self.on_dom_action("data-area", area_slug, false, now_ms);
        }
        if let Some(sv_target) = id.strip_prefix("nav-subview:") {
            return self.on_dom_action("data-subview", sv_target, false, now_ms);
        }
        if let Some(job_str) = id.strip_prefix("ci-job:") {
            if let Ok(job_id) = job_str.parse::<u64>() {
                self.route.job = Some(job_id);
                self.selected_stage_idx = None;
                self.log_page = 0;
                return vec![UiCommand::SelectJob { job_id }];
            }
        }
        if let Some(stage_str) = id.strip_prefix("ci-stage-select:") {
            if let Ok(idx) = stage_str.parse::<usize>() {
                self.selected_stage_idx = if self.selected_stage_idx == Some(idx) {
                    None
                } else {
                    Some(idx)
                };
                return Vec::new();
            }
        }
        if let Some(job_str) = id.strip_prefix("ci-job-resume:") {
            if let Ok(job_id) = job_str.parse::<u64>() {
                return self.resume_job(job_id).into_iter().collect();
            }
        }
        if let Some(job_str) = id.strip_prefix("ci-job-restart:") {
            if let Ok(job_id) = job_str.parse::<u64>() {
                return self.restart_job(job_id).into_iter().collect();
            }
        }
        if let Some(dep_id) = id.strip_prefix("ab-rollback:") {
            return self.open_ab_rollback_confirm(dep_id);
        }
        if let Some(role_id) = id.strip_prefix("role-enable:") {
            if self.auth().status != AuthStatus::Active {
                return Vec::new();
            }
            self.clear_session_roles_error();
            return vec![UiCommand::EnableRole {
                role_id: role_id.into(),
            }];
        }
        if let Some(role_id) = id.strip_prefix("role-disable:") {
            if self.auth().status != AuthStatus::Active {
                return Vec::new();
            }
            self.clear_session_roles_error();
            return vec![UiCommand::DisableRole {
                role_id: role_id.into(),
            }];
        }
        if let Some(role_id) = id.strip_prefix("role-elevate:") {
            if self.auth().status != AuthStatus::Active {
                return Vec::new();
            }
            self.clear_session_roles_error();
            self.pending_webauthn = Some(WebAuthnRequest::ElevateTimedRole {
                role_id: role_id.into(),
            });
            return Vec::new();
        }
        if let Some(role_id) = id.strip_prefix("role-renew:") {
            if self.auth().status != AuthStatus::Active {
                return Vec::new();
            }
            self.clear_session_roles_error();
            self.pending_webauthn = Some(WebAuthnRequest::RenewTimedRole {
                role_id: role_id.into(),
            });
            return Vec::new();
        }
        if let Some(cred_id) = id.strip_prefix("profile-passkey-remove:") {
            if self.auth().status != AuthStatus::Active {
                return Vec::new();
            }
            self.webauthn_error = None;
            self.pending_webauthn = Some(WebAuthnRequest::RemovePasskey {
                credential_id: cred_id.into(),
            });
            return Vec::new();
        }
        if let Some(login) = id.strip_prefix("platform-user-reset-passkeys:") {
            return self.open_platform_risk_confirm(
                RiskConfirmAction::ResetUserPasskeys {
                    github_login: login.into(),
                },
                Some(format!("RESET {login}")),
            );
        }
        if let Some(login) = id.strip_prefix("platform-user-revoke:") {
            let is_last_admin = self.client.view().is_some_and(|v| {
                let admins: Vec<_> = v
                    .platform
                    .users
                    .iter()
                    .filter(|u| u.is_platform_admin)
                    .collect();
                admins.len() <= 1 && admins.iter().any(|u| u.github_login == login)
            });
            if is_last_admin {
                self.aria_assertive = "Last platform administrator cannot be revoked.".into();
                return Vec::new();
            }
            return self.open_platform_risk_confirm(
                RiskConfirmAction::RevokeUser {
                    github_login: login.into(),
                },
                Some(format!("REVOKE {login}")),
            );
        }
        if let Some(role_id) = id.strip_prefix("platform-role-delete:") {
            return self.open_platform_risk_confirm(
                RiskConfirmAction::DeleteRoleSpec {
                    role_id: role_id.into(),
                },
                Some(format!("DELETE {role_id}")),
            );
        }
        if let Some(folio_id) = id.strip_prefix("platform-folio-delete:") {
            return self.open_platform_risk_confirm(
                RiskConfirmAction::DeletePolicyFolio {
                    folio_id: folio_id.into(),
                },
                Some(format!("DELETE {folio_id}")),
            );
        }
        if let Some(inv_id) = id.strip_prefix("platform-invite-revoke:") {
            if !self.auth().has_policy("platform:invites.write") {
                return Vec::new();
            }
            return vec![UiCommand::RevokeInvite {
                invite_id: inv_id.into(),
            }];
        }
        if let Some(track_ref) = id.strip_prefix("c5-track-") {
            self.route.branch = Some(track_ref.into());
            if let Some(job_id) = self.active_job_id() {
                self.route.job = Some(job_id);
                return vec![UiCommand::SelectJob { job_id }];
            }
            return Vec::new();
        }
        if let Some(hist) = id.strip_prefix("c6-history:") {
            if let Ok(idx) = hist.parse::<usize>() {
                self.selected_history_idx = idx + 1;
                return Vec::new();
            }
        }
        if let Some(m) = id.strip_prefix("obs-metric:") {
            if let Ok(idx) = m.parse::<usize>() {
                self.selected_metric_idx = idx;
                return Vec::new();
            }
        }
        if let Some(sb) = id.strip_prefix("flt-sandbox:") {
            if let Ok(idx) = sb.parse::<usize>() {
                self.selected_sandbox_idx = Some(idx);
                return Vec::new();
            }
        }
        if let Some(sub) = id.strip_prefix("platform-maint-select:") {
            self.platform_form.selected_subsystem =
                SubsystemId::from_policy_prefix(sub).unwrap_or(SubsystemId::Ci);
            return Vec::new();
        }
        if let Some(repo_idx_str) = id.strip_prefix("repo-select:") {
            if let Ok(idx) = repo_idx_str.parse::<usize>() {
                if let Some(r) = self.filtered_repos().get(idx).cloned() {
                    self.outer_index = idx;
                    self.inner_index = 0;
                    self.route.repo = Some(r.id.clone());
                    self.route.submodule = None;
                    return vec![UiCommand::SelectRepo {
                        repo: r.id,
                        submodule: None,
                    }];
                }
            }
        }

        match id {
            "c2-auth-status" | "hdr-auth-action" | "login-gate-signin" | "auth-gate-signin" => {
                let auth = self.auth();
                if !auth.is_resolved() {
                    return Vec::new();
                }
                let href = match auth.status {
                    AuthStatus::Anonymous => self.sign_in_href(),
                    AuthStatus::AuthorizationDenied
                    | AuthStatus::PasskeyEnrollmentRequired
                    | AuthStatus::Active => self.sign_out_href(),
                    AuthStatus::Unresolved => return Vec::new(),
                };
                self.pending_navigation = Some(href.clone());
                self.aria_polite = format!("Navigating to {href}");
                vec![UiCommand::Navigate { href }]
            }
            "dlg-passkey-signout"
            | "profile-signout"
            | "session-error-signout"
            | "auth-gate-signout" => {
                let href = self.sign_out_href();
                self.pending_navigation = Some(href.clone());
                vec![UiCommand::Navigate { href }]
            }
            "auth-gate-dashboard" => vec![self.select_area(Area::Dashboard)],
            "session-probe-retry" => {
                self.client.clear_last_error();
                self.aria_polite = copy::LOADING_SESSION_TITLE.into();
                vec![UiCommand::Retry]
            }
            "dlg-passkey-register"
            | "register-passkey"
            | "auth-gate-register-passkey"
            | "hold-passkey-register"
            | "profile-passkey-add" => {
                let auth = self.auth();
                if !matches!(
                    auth.status,
                    AuthStatus::PasskeyEnrollmentRequired | AuthStatus::Active
                ) {
                    return Vec::new();
                }
                if matches!(id, "register-passkey" | "auth-gate-register-passkey") {
                    self.passkey_gate_dismissed = false;
                    self.local_dialog = None;
                }
                self.webauthn_error = None;
                self.aria_polite = "Starting Passkey enrollment…".into();
                let requires_step_up = auth.status == AuthStatus::Active && auth.passkey_count > 0;
                let label = if self.profile_passkey_label_draft.trim().is_empty() {
                    None
                } else {
                    Some(self.profile_passkey_label_draft.trim().to_owned())
                };
                self.pending_webauthn = Some(WebAuthnRequest::RegisterPasskey {
                    label,
                    requires_step_up,
                });
                Vec::new()
            }
            "hdr-user" | "open-profile" => {
                let default_tab = if self.auth().status == AuthStatus::PasskeyEnrollmentRequired {
                    UserProfileTab::Passkeys
                } else {
                    UserProfileTab::UserDetails
                };
                self.open_user_profile(default_tab)
            }
            "profile-tab:details" => self.open_user_profile(UserProfileTab::UserDetails),
            "profile-tab:preferences" => self.open_user_profile(UserProfileTab::Preferences),
            "profile-tab:passkeys" => self.open_user_profile(UserProfileTab::Passkeys),
            "profile-save-details" => self.save_user_profile_details(),
            "profile-skin:html" => {
                if self.auth().status != AuthStatus::Active {
                    return Vec::new();
                }
                let cmd = self.set_skin(ConsoleSkin::Html);
                vec![
                    cmd,
                    UiCommand::UpdateUserProfile {
                        display_name: None,
                        preferred_skin: Some(ConsoleSkin::Html),
                        preferred_font: None,
                    },
                ]
            }
            "profile-skin:wireframe" => {
                if self.auth().status != AuthStatus::Active {
                    return Vec::new();
                }
                let cmd = self.set_skin(ConsoleSkin::Wireframe);
                vec![
                    cmd,
                    UiCommand::UpdateUserProfile {
                        display_name: None,
                        preferred_skin: Some(ConsoleSkin::Wireframe),
                        preferred_font: None,
                    },
                ]
            }
            "profile-font:hurmit" => self.select_profile_font(CrtFont::Hurmit),
            "profile-font:dyslexic" => self.select_profile_font(CrtFont::OpenDyslexic),
            "hdr-role" | "open-session-roles" | "profile-manage-roles" => self.open_session_roles(),
            "roles-lock-all" | "profile-lock-timed" => {
                if self.auth().status != AuthStatus::Active {
                    return Vec::new();
                }
                self.clear_session_roles_error();
                vec![UiCommand::LockTimedRoles]
            }
            "btn-cancel"
            | "btn-close"
            | "roles-close"
            | "btn-close-inv"
            | "dismiss-dialog"
            | "dlg-passkey-dismiss" => self.dismiss_dialog(),
            "btn-confirm" | "confirm-risk" => self.confirm_risk_dialog(),
            "font-toggle" | "c2-font-switch" => vec![self.toggle_font()],
            "repo-prev" => {
                self.rotate_outer(-1, now_ms);
                Vec::new()
            }
            "repo-next" => {
                self.rotate_outer(1, now_ms);
                Vec::new()
            }
            "sub-prev" => {
                self.rotate_inner(-1, now_ms);
                Vec::new()
            }
            "sub-next" => {
                self.rotate_inner(1, now_ms);
                Vec::new()
            }
            "ci-queue-filter" => {
                self.ci_queue_filter = (self.ci_queue_filter + 1) % 3;
                self.subview_page = 0;
                Vec::new()
            }
            "ci-queue-filter:all" => {
                self.ci_queue_filter = 0;
                self.subview_page = 0;
                Vec::new()
            }
            "ci-queue-filter:running" => {
                self.ci_queue_filter = 1;
                self.subview_page = 0;
                Vec::new()
            }
            "ci-queue-filter:failed" => {
                self.ci_queue_filter = 2;
                self.subview_page = 0;
                Vec::new()
            }
            "ci-log-follow-toggle" | "obs-log-follow-toggle" => {
                self.follow_log = !self.follow_log;
                if self.follow_log {
                    self.log_page = u32::MAX;
                    self.subview_page = u32::MAX;
                }
                Vec::new()
            }
            "ci-log-page-prev" => {
                self.follow_log = false;
                self.log_page = self.log_page.saturating_sub(1);
                Vec::new()
            }
            "ci-log-page-next" => {
                self.follow_log = false;
                self.log_page = self.log_page.saturating_add(1);
                Vec::new()
            }
            "ci-job-detail-screen:prev" => {
                self.follow_log = false;
                self.log_page = self.rendered_ci_log_page.get().saturating_sub(1);
                Vec::new()
            }
            "obs-logs-screen:prev" => {
                self.follow_log = false;
                self.subview_page = self.rendered_obs_log_page.get().saturating_sub(1);
                Vec::new()
            }
            "c6-history-strip" => {
                self.selected_history_idx = 0;
                Vec::new()
            }
            "obs-smartscape-filter:hosts" => {
                self.obs_smartscape_filter = 3;
                Vec::new()
            }
            "obs-smartscape-filter:processes" => {
                self.obs_smartscape_filter = 2;
                Vec::new()
            }
            "obs-smartscape-filter:services" => {
                self.obs_smartscape_filter = 1;
                Vec::new()
            }
            "obs-smartscape-filter:all" | "obs-smartscape-filter" => {
                self.obs_smartscape_filter = 0;
                Vec::new()
            }
            "obs-service-prev" => {
                self.rotate_obs_service(-1);
                Vec::new()
            }
            "obs-service-next" => {
                self.rotate_obs_service(1);
                Vec::new()
            }
            "obs-service-clear" => {
                self.obs_service_filter = None;
                Vec::new()
            }
            "platform-maint-apply" => self.open_platform_risk_confirm(
                RiskConfirmAction::SetSubsystemMaintenance {
                    subsystem: self.platform_form.selected_subsystem,
                    mode: self.platform_form.maintenance_mode,
                    reason: (!self.platform_form.maintenance_reason.trim().is_empty())
                        .then(|| self.platform_form.maintenance_reason.trim().to_owned()),
                    auto_clear_secs: self.platform_form.maintenance_auto_clear_secs,
                },
                None,
            ),
            "platform-invite-create" => {
                if !self.auth().has_policy("platform:invites.write") {
                    return Vec::new();
                }
                vec![UiCommand::CreateInvite {
                    note: (!self.platform_form.invite_note.trim().is_empty())
                        .then(|| self.platform_form.invite_note.trim().to_owned()),
                    ttl_secs: Some(self.platform_form.invite_ttl_secs),
                }]
            }
            "platform-rfi-toggle-anon" => {
                self.platform_form.anonymous_lockdown_draft =
                    !self.platform_form.anonymous_lockdown_draft;
                self.open_platform_risk_confirm(RiskConfirmAction::UpdateRoleFolioIndex, None)
            }
            _ => Vec::new(),
        }
    }

    fn save_user_profile_details(&self) -> Vec<UiCommand> {
        if self.auth().status != AuthStatus::Active {
            return Vec::new();
        }
        let name = self.profile_display_name_draft.trim();
        vec![UiCommand::UpdateUserProfile {
            display_name: (!name.is_empty()).then(|| name.to_owned()),
            preferred_skin: None,
            preferred_font: None,
        }]
    }

    fn open_ab_rollback_confirm(&mut self, deployment_id: &str) -> Vec<UiCommand> {
        if !self.auth().has_policy("ab:rollback") {
            self.aria_polite = copy::AUTH_DENIED_STATUS.into();
            return Vec::new();
        }
        let gate = self.client.view().and_then(|v| {
            v.gate
                .as_ref()
                .filter(|g| deployment_id.is_empty() || g.deployment_id == deployment_id)
        });
        let (dep_id, repo, target_node, current_slot, restore_slot) = match gate {
            Some(g) => (
                g.deployment_id.clone(),
                g.repo.clone(),
                g.target_node.clone(),
                g.staged_slot,
                g.restore_slot,
            ),
            None => (
                deployment_id.to_owned(),
                self.route
                    .repo
                    .clone()
                    .unwrap_or_else(|| RepoId::new("LMA1980", "mu-console")),
                "node-a".to_owned(),
                mu_console_protocol::Slot::B,
                mu_console_protocol::Slot::A,
            ),
        };
        let note = (!self.platform_form.ab_rollback_note.trim().is_empty())
            .then(|| self.platform_form.ab_rollback_note.trim().to_owned());
        // A|B emergency rollback uses fast RiskConfirm (no heavy confirmation phrase, optional note #56).
        let dlg = DialogModel::RiskConfirm(RiskConfirmDialogModel {
            action: RiskConfirmAction::TriggerAbRollback {
                deployment_id: dep_id,
                repo,
                target_node,
                current_slot,
                restore_slot,
                note,
            },
            confirmation_phrase: None,
            typed_confirmation: String::new(),
        });
        self.local_dialog = Some(dlg.clone());
        vec![UiCommand::OpenDialog(dlg)]
    }

    fn open_platform_risk_confirm(
        &mut self,
        action: RiskConfirmAction,
        confirmation_phrase: Option<String>,
    ) -> Vec<UiCommand> {
        let required = match &action {
            RiskConfirmAction::SetSubsystemMaintenance { .. } => "platform:maintenance.write",
            RiskConfirmAction::UpdateUserRoles { .. }
            | RiskConfirmAction::ResetUserPasskeys { .. }
            | RiskConfirmAction::RevokeUser { .. } => "platform:users.write",
            RiskConfirmAction::UpsertRoleSpec { .. } | RiskConfirmAction::DeleteRoleSpec { .. } => {
                "platform:roles.write"
            }
            RiskConfirmAction::UpsertPolicyFolio { .. }
            | RiskConfirmAction::DeletePolicyFolio { .. }
            | RiskConfirmAction::UpdateRoleFolioIndex => "platform:folios.write",
            RiskConfirmAction::RestartCiJob { .. } => "ci:jobs.restart",
            RiskConfirmAction::TriggerAbRollback { .. } => "ab:rollback",
        };
        if !self.auth().has_policy(required) {
            self.aria_polite = copy::AUTH_DENIED_STATUS.into();
            return Vec::new();
        }
        let dlg = DialogModel::RiskConfirm(RiskConfirmDialogModel {
            action,
            confirmation_phrase,
            typed_confirmation: String::new(),
        });
        self.local_dialog = Some(dlg.clone());
        vec![UiCommand::OpenDialog(dlg)]
    }

    /// Update the typed phrase inside an open `DialogModel::RiskConfirm`.
    pub fn set_risk_confirm_input(&mut self, typed: &str) {
        if let Some(DialogModel::RiskConfirm(rc)) = &mut self.local_dialog {
            rc.typed_confirmation = typed.to_owned();
        }
    }

    /// Confirm active `RiskConfirm` dialog if phrase guard (when present) matches.
    pub fn confirm_risk_dialog(&mut self) -> Vec<UiCommand> {
        let Some(DialogModel::RiskConfirm(rc)) = self.active_dialog() else {
            return Vec::new();
        };
        if let Some(expected) = &rc.confirmation_phrase {
            if rc.typed_confirmation.trim() != expected {
                self.aria_assertive = "Confirmation phrase does not match.".into();
                return Vec::new();
            }
        }
        let cmd = match rc.action {
            RiskConfirmAction::RestartCiJob { job_id, .. } => UiCommand::RestartJob { job_id },
            RiskConfirmAction::TriggerAbRollback {
                deployment_id,
                note,
                ..
            } => UiCommand::TriggerAbRollback {
                deployment_id,
                note: note.or_else(|| {
                    (!self.platform_form.ab_rollback_note.trim().is_empty())
                        .then(|| self.platform_form.ab_rollback_note.trim().to_owned())
                }),
            },
            RiskConfirmAction::SetSubsystemMaintenance {
                subsystem,
                mode,
                reason,
                auto_clear_secs,
            } => UiCommand::SetSubsystemMaintenance {
                subsystem,
                mode,
                reason,
                auto_clear_secs,
            },
            RiskConfirmAction::UpdateUserRoles {
                github_login,
                bound_roles,
            } => UiCommand::UpdateUserRoles {
                github_login,
                bound_roles,
            },
            RiskConfirmAction::ResetUserPasskeys { github_login } => {
                UiCommand::ResetUserPasskeys { github_login }
            }
            RiskConfirmAction::RevokeUser { github_login } => {
                UiCommand::RevokeUser { github_login }
            }
            RiskConfirmAction::DeleteRoleSpec { role_id } => UiCommand::DeleteRoleSpec { role_id },
            RiskConfirmAction::DeletePolicyFolio { folio_id } => {
                UiCommand::DeletePolicyFolio { folio_id }
            }
            RiskConfirmAction::UpsertRoleSpec { .. }
            | RiskConfirmAction::UpsertPolicyFolio { .. }
            | RiskConfirmAction::UpdateRoleFolioIndex => {
                let mut idx = self
                    .client
                    .view()
                    .map(|v| v.platform.role_folio_index.clone())
                    .unwrap_or_default();
                if self.platform_form.anonymous_lockdown_draft {
                    idx.anonymous.clear();
                } else if idx.anonymous.is_empty() {
                    idx.anonymous = vec![
                        "folio:system:dashboard".into(),
                        "folio:ci:anon-queue".into(),
                    ];
                }
                UiCommand::UpdateRoleFolioIndex(idx)
            }
        };
        self.local_dialog = None;
        vec![cmd, UiCommand::CloseDialog]
    }

    /// Dismiss active dialog (including `PasskeyEnrollmentGate` so 0-passkey users can browse public views).
    pub fn dismiss_dialog(&mut self) -> Vec<UiCommand> {
        let Some(dlg) = self.active_dialog() else {
            return Vec::new();
        };
        if matches!(dlg, DialogModel::PasskeyEnrollmentGate(_)) {
            self.passkey_gate_dismissed = true;
            self.local_dialog = None;
            self.session_roles_worker_synced = false;
            return vec![UiCommand::CloseDialog];
        }
        if !dlg.is_dismissible() {
            return Vec::new();
        }
        if matches!(dlg, DialogModel::SessionRoles(_)) {
            self.webauthn_error = None;
        }
        self.local_dialog = None;
        self.session_roles_worker_synced = false;
        if let Some(view) = self.client.view_mut() {
            if !matches!(
                view.active_dialog,
                Some(DialogModel::PasskeyEnrollmentGate(_))
            ) {
                view.active_dialog = None;
            }
        }
        vec![UiCommand::CloseDialog]
    }

    /// Handle keyboard navigation and shortcuts.
    pub fn on_key(&mut self, key: &str, shift: bool, now_ms: u64) -> Option<UiCommand> {
        if key == "Escape" {
            if self.active_dialog().is_some() {
                return self.dismiss_dialog().into_iter().next();
            }
            if self.filter_focused || !self.filter_query.is_empty() {
                let had_filter =
                    !self.filter_query.trim().is_empty() || self.route.filter.is_some();
                self.filter_focused = false;
                self.filter_query.clear();
                self.route.filter = None;
                self.pending_filter_at_ms = None;
                return if had_filter && subview_uses_server_filter(self.route.subview) {
                    Some(UiCommand::Subscribe(self.route.clone()))
                } else {
                    None
                };
            }
            let next = escape_up(&self.route);
            if self.can_access_subview(next.subview) {
                self.route = next;
                self.route_sync_dirty = true;
            }
            return None;
        }
        if self.filter_focused || self.active_dialog().is_some() {
            return None;
        }
        match key {
            "/" if !shift => {
                if self.is_filter_applicable() {
                    self.filter_focused = true;
                }
                None
            }
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" => {
                let idx = (key.as_bytes()[0] - b'1') as usize;
                let area = AREAS[idx];
                self.can_access_area(area).then(|| self.select_area(area))
            }
            "s" | "S" => {
                if self.auth().status == AuthStatus::Active {
                    Some(self.toggle_skin())
                } else {
                    None
                }
            }
            "d" | "D" | "f" | "F" => Some(self.toggle_font()),
            "[" if !shift => {
                self.rotate_outer(-1, now_ms);
                None
            }
            "]" if !shift => {
                self.rotate_outer(1, now_ms);
                None
            }
            _ => None,
        }
    }

    /// Periodic client tick handling rotary debounce settle and `Timed` lease expiry (`00:00`).
    pub fn tick(&mut self, now_ms: u64) -> Vec<UiCommand> {
        self.last_lease_tick_ms = now_ms;
        let mut cmds = Vec::new();

        if let Some(due) = self.pending_select_at_ms {
            if now_ms >= due {
                self.pending_select_at_ms = None;
                if let Some(repo) = self.route.repo.clone() {
                    cmds.push(UiCommand::SelectRepo {
                        repo,
                        submodule: self.route.submodule.clone(),
                    });
                }
            }
        }

        if let Some(due) = self.pending_filter_at_ms {
            if now_ms >= due {
                self.pending_filter_at_ms = None;
                if subview_uses_server_filter(self.route.subview) {
                    cmds.push(UiCommand::Subscribe(self.route.clone()));
                }
            }
        }

        if let Some(view) = self.client.view_mut() {
            let expired = expire_timed_leases(&mut view.auth, now_ms);
            if !expired.is_empty() {
                if redirect_if_lacking_access(&view.auth, &mut self.route) {
                    self.route_sync_dirty = true;
                }
                cmds.push(UiCommand::RefreshSession);
            }
            if let Some(DialogModel::SessionRoles(ref mut sr)) = view.active_dialog {
                let auth_snap = view.auth.clone();
                let platform_roles = view.platform.roles.clone();
                sync_session_roles_model_parts(sr, &auth_snap, &platform_roles);
            }
            if let Some(DialogModel::SessionRoles(ref mut sr)) = self.local_dialog {
                let auth_snap = view.auth.clone();
                let platform_roles = view.platform.roles.clone();
                sync_session_roles_model_parts(sr, &auth_snap, &platform_roles);
            }
        }
        cmds
    }
}

fn resolve_descriptors_from_view(
    auth: &AuthState,
    view: Option<&ViewModel>,
) -> Vec<SessionRoleDescriptor> {
    let platform_roles = view.map_or(&[][..], |v| v.platform.roles.as_slice());
    resolve_descriptors_from_parts(auth, platform_roles)
}

fn resolve_descriptors_from_parts(
    auth: &AuthState,
    platform_roles: &[mu_console_protocol::dto::RoleSpecDto],
) -> Vec<SessionRoleDescriptor> {
    let mut descs = auth.session_role_descriptors();
    for d in &mut descs {
        if let Some(spec) = platform_roles.iter().find(|r| r.id == d.role_id) {
            if d.description.is_empty() && !spec.description.is_empty() {
                d.description.clone_from(&spec.description);
            }
            if d.display_name.is_none() {
                d.display_name.clone_from(&spec.display_name);
            }
        }
    }
    if auth.catalog_role_specs.is_empty() {
        for spec in platform_roles {
            if !descs.iter().any(|d| d.role_id == spec.id) {
                descs.push(SessionRoleDescriptor {
                    role_id: spec.id.clone(),
                    display_name: spec.display_name.clone(),
                    description: spec.description.clone(),
                    activation: spec.activation,
                    step_up: spec.step_up,
                    active: false,
                    lease: None,
                    step_up_verified: false,
                    unassigned: true,
                });
            }
        }
    }
    descs
}

fn sync_session_roles_model(
    sr: &mut SessionRolesDialogModel,
    auth: &AuthState,
    view: Option<&ViewModel>,
) {
    let platform_roles = view.map_or(&[][..], |v| v.platform.roles.as_slice());
    sync_session_roles_model_parts(sr, auth, platform_roles);
}

fn sync_session_roles_model_parts(
    sr: &mut SessionRolesDialogModel,
    auth: &AuthState,
    platform_roles: &[mu_console_protocol::dto::RoleSpecDto],
) {
    sr.github_login = auth
        .github_login
        .clone()
        .unwrap_or_else(|| "operator".into());
    let resolved = resolve_descriptors_from_parts(auth, platform_roles);
    if !resolved.is_empty() || sr.roles.is_empty() {
        sr.roles = resolved;
    }
    sr.lock_all_enabled = !auth.leases.is_empty();
}

#[cfg(test)]
mod tests {
    use super::*;
    use mu_console_protocol::{encode, DataEvent, ViewModel};

    #[test]
    fn fail_closed_boot_and_anonymous_default_to_public_ci_steps() {
        let mut st = ConsoleState::new();
        assert!(!st.auth().is_resolved());
        for area in Area::ALL {
            assert!(!st.can_access_area(area));
        }

        // Anonymous without explicit hash -> defaults cleanly to CI > QueueLogs (#50).
        let vm = ViewModel {
            auth: AuthState {
                status: AuthStatus::Anonymous,
                active_policies: vec![
                    "system:dashboard.read".into(),
                    "ci:queue.read".into(),
                    "ci:repos.public.read".into(),
                ],
                anonymous_lockdown: false,
                ..AuthState::default()
            },
            ..ViewModel::default()
        };
        let frame = encode(&DataEvent::Snapshot(vm)).expect("encode");
        st.on_worker_frame(&frame);

        assert_eq!(st.route.subview, SubView::Ci(CiSubView::QueueLogs));
        assert_eq!(st.effective_skin(), ConsoleSkin::Html);

        // Signed-out user attempting to switch to Wireframe stays locked to Html (#49).
        st.set_skin(ConsoleSkin::Wireframe);
        assert_eq!(st.effective_skin(), ConsoleSkin::Html);
        assert_eq!(st.on_key("s", false, 100), None);
    }

    #[test]
    fn passkey_gate_auto_opens_dismisses_to_public_views_and_keeps_operator_nav_inert() {
        let mut st = ConsoleState::new();
        let vm = ViewModel {
            auth: AuthState {
                status: AuthStatus::PasskeyEnrollmentRequired,
                github_login: Some("alice".into()),
                passkey_count: 0,
                passkey_enrollment_required: true,
                ..AuthState::default()
            },
            ..ViewModel::default()
        };
        let frame = encode(&DataEvent::Snapshot(vm)).expect("encode");
        st.on_worker_frame(&frame);

        assert!(matches!(
            st.active_dialog(),
            Some(DialogModel::PasskeyEnrollmentGate(_))
        ));
        assert_eq!(st.dismiss_dialog(), vec![UiCommand::CloseDialog]);
        assert!(st.active_dialog().is_none());

        // Re-opening banner/trigger brings back PasskeyEnrollmentGate cleanly; Escape dismisses it.
        let _ = st.activate_target("register-passkey");
        let _ = st.pending_webauthn.take();
        assert!(matches!(
            st.active_dialog(),
            Some(DialogModel::PasskeyEnrollmentGate(_))
        ));
        assert_eq!(
            st.on_key("Escape", false, 100),
            Some(UiCommand::CloseDialog)
        );
        assert!(st.active_dialog().is_none());

        // Public Dashboard & CI views are accessible; operator-only Platform stays inert.
        assert!(st.can_access_area(Area::Dashboard));
        assert!(st.can_access_area(Area::Ci));
        assert!(!st.can_access_area(Area::Platform));
        assert!(st
            .on_dom_action("data-area", "platform", false, 200)
            .is_empty());
        assert_eq!(st.pending_navigation, None);
    }

    #[test]
    fn no_risk_resume_executes_once_while_restart_and_rollback_open_risk_confirm() {
        let mut st = ConsoleState::new();
        let vm = ViewModel {
            auth: AuthState {
                status: AuthStatus::Active,
                github_login: Some("ops".into()),
                passkey_count: 1,
                active_policies: vec![
                    "ci:jobs.read".into(),
                    "ci:jobs.resume".into(),
                    "ci:jobs.restart".into(),
                    "ab:slots.read".into(),
                    "ab:rollback".into(),
                ],
                ..AuthState::default()
            },
            ..ViewModel::default()
        };
        let frame = encode(&DataEvent::Snapshot(vm)).expect("encode");
        st.on_worker_frame(&frame);

        // Resume executes in 1 click without dialog.
        assert_eq!(
            st.activate_target("ci-job-resume:42"),
            vec![UiCommand::ResumeJob { job_id: 42 }]
        );
        assert!(st.active_dialog().is_none());

        // Restart opens RiskConfirm requiring phrase confirmation.
        let cmds = st.activate_target("ci-job-restart:42");
        assert!(matches!(
            cmds.first(),
            Some(UiCommand::OpenDialog(DialogModel::RiskConfirm(_)))
        ));
        assert!(st.confirm_risk_dialog().is_empty());
        st.set_risk_confirm_input("RESTART 000042");
        assert_eq!(
            st.confirm_risk_dialog(),
            vec![UiCommand::RestartJob { job_id: 42 }, UiCommand::CloseDialog]
        );

        // A|B Rollback uses fast RiskConfirm (no phrase required).
        let ab_cmds = st.activate_target("ab-rollback:dep-9");
        assert!(matches!(
            ab_cmds.first(),
            Some(UiCommand::OpenDialog(DialogModel::RiskConfirm(_)))
        ));
        assert!(matches!(
            st.confirm_risk_dialog().first(),
            Some(UiCommand::TriggerAbRollback { deployment_id, .. }) if deployment_id == "dep-9"
        ));
    }

    #[test]
    fn session_probe_error_retry_and_signout_transition_cleanly() {
        let mut st = ConsoleState::new();
        let err_frame = encode(&DataEvent::Error(mu_console_protocol::UiError::Http {
            status: 503,
            message: "GET /api/auth/session".into(),
        }))
        .expect("encode");
        st.on_worker_frame(&err_frame);
        assert!(st.session_probe_error().is_some());

        assert_eq!(
            st.activate_target("session-error-signout"),
            vec![UiCommand::Navigate {
                href: "/oauth2/sign_out?rd=%2F".into()
            }]
        );
        assert_eq!(
            st.activate_target("session-probe-retry"),
            vec![UiCommand::Retry]
        );
        assert!(st.session_probe_error().is_none());

        st.on_worker_frame(&err_frame);
        assert!(st.session_probe_error().is_some());

        let snap = encode(&DataEvent::Snapshot(ViewModel {
            auth: AuthState::anonymous(vec!["ci:queue.read".into()]),
            ..ViewModel::default()
        }))
        .expect("encode");
        st.on_worker_frame(&snap);
        assert!(st.session_probe_error().is_none());
    }

    #[test]
    fn session_roles_worker_sync_auto_close_error_propagation_and_profile_action() {
        use mu_console_protocol::dto::{
            AuthSessionDto, RoleActivationDto, RoleSpecDto, UserProfileDto,
        };
        use mu_console_protocol::{Delta, SessionRolesDialogModel, StepUpRequirement};

        let mut st = ConsoleState::new();
        let operator_spec = RoleSpecDto {
            id: "operator".into(),
            display_name: Some("Operator".into()),
            description: "Baseline read role".into(),
            activation: RoleActivationDto::pinned_baseline(),
            step_up: StepUpRequirement::None,
        };
        let sec_ops_spec = RoleSpecDto {
            id: "sec-ops".into(),
            display_name: Some("Sec Ops".into()),
            description: "Security triage role".into(),
            activation: RoleActivationDto::elective(),
            step_up: StepUpRequirement::None,
        };
        let super_spec = RoleSpecDto {
            id: "super-operator".into(),
            display_name: Some("Super Operator".into()),
            description: "All-access Passkey role".into(),
            activation: RoleActivationDto::super_operator(),
            step_up: StepUpRequirement::Passkey,
        };
        let auth = AuthState::from_session(&AuthSessionDto {
            authenticated: true,
            github_login: Some("ops".into()),
            invite_bound: true,
            passkey_count: 1,
            baseline_role: Some("operator".into()),
            bound_roles: vec!["operator".into(), "sec-ops".into()],
            bound_role_specs: vec![operator_spec.clone(), sec_ops_spec.clone()],
            catalog_role_specs: vec![operator_spec, sec_ops_spec, super_spec],
            active_roles: vec![],
            active_policies: vec!["system:dashboard.read".into()],
            ..AuthSessionDto::default()
        });
        let vm = ViewModel {
            auth: auth.clone(),
            user_profile: Some(UserProfileDto {
                github_login: "ops".into(),
                display_name: Some("ops".into()),
                preferences: Default::default(),
                bound_roles: vec!["operator".into(), "sec-ops".into()],
                passkey_count: 1,
                passkeys: vec![],
            }),
            ..ViewModel::default()
        };
        let snap = encode(&DataEvent::Snapshot(vm)).expect("encode");
        st.on_worker_frame(&snap);

        // open_session_roles opens immediately with local hydration including unassigned role.
        assert!(matches!(
            st.open_session_roles().as_slice(),
            [UiCommand::OpenDialog(DialogModel::SessionRoles(_))]
        ));
        match st.active_dialog() {
            Some(DialogModel::SessionRoles(sr)) => {
                assert!(sr
                    .roles
                    .iter()
                    .any(|r| r.role_id == "sec-ops" && !r.unassigned));
                assert!(sr
                    .roles
                    .iter()
                    .any(|r| r.role_id == "super-operator" && r.unassigned));
            }
            other => panic!("expected SessionRoles dialog, got {other:?}"),
        }

        // Worker SessionRoles delta overrides local_dialog and delivers error_message.
        let worker_sr = DialogModel::SessionRoles(SessionRolesDialogModel {
            github_login: "ops".into(),
            roles: auth.session_role_descriptors(),
            lock_all_enabled: false,
            error_message: Some("Worker validation failure".into()),
        });
        let sr_patch = encode(&DataEvent::Patch(vec![Delta::ActiveDialog(Some(
            worker_sr,
        ))]))
        .expect("encode");
        st.on_worker_frame(&sr_patch);
        match st.active_dialog() {
            Some(DialogModel::SessionRoles(sr)) => {
                assert_eq!(
                    sr.error_message.as_deref(),
                    Some("Worker validation failure")
                );
            }
            other => panic!("expected worker SessionRoles dialog, got {other:?}"),
        }

        // Routine SSE updates do NOT wipe webauthn_error while SessionRoles remains open.
        st.on_webauthn_error("Passkey verification was cancelled or timed out.");
        let ping_patch = encode(&DataEvent::Patch(vec![Delta::NodeUpsert(
            mu_console_protocol::FabricNode::new("node-a", true),
        )]))
        .expect("encode");
        st.on_worker_frame(&ping_patch);
        assert_eq!(
            st.webauthn_error.as_deref(),
            Some("Passkey verification was cancelled or timed out.")
        );
        // Verify ActiveDialog(None) auto-closes SessionRoles and clears webauthn_error.
        let close_patch =
            encode(&DataEvent::Patch(vec![Delta::ActiveDialog(None)])).expect("encode");
        st.on_worker_frame(&close_patch);
        assert!(st.active_dialog().is_none());
        assert!(st.webauthn_error.is_none());

        // Clicking profile-manage-roles from UserProfile switches to SessionRoles.
        let _ = st.activate_target("hdr-user");
        assert!(matches!(
            st.active_dialog(),
            Some(DialogModel::UserProfile(_))
        ));
        assert!(matches!(
            st.activate_target("profile-manage-roles").as_slice(),
            [UiCommand::OpenDialog(DialogModel::SessionRoles(_))]
        ));
        assert!(matches!(
            st.active_dialog(),
            Some(DialogModel::SessionRoles(_))
        ));
    }

    #[test]
    fn filter_applicability_placeholders_dormant_roles_and_subview_controls() {
        use mu_console_protocol::dto::RoleSpecDto;
        use mu_console_protocol::{
            DashboardSubView, ObservabilitySubView, ObserveView, PlatformSubView, RoleBadgeView,
            SmartscapeLayer, SmartscapeNode,
        };

        // Unresolved session disables filter with resolving placeholder.
        let mut st = ConsoleState::new();
        assert!(!st.is_filter_applicable());
        assert_eq!(st.filter_placeholder_text(), "Resolving session...");
        assert_eq!(st.on_key("/", false, 10), None);
        assert!(!st.filter_focused);

        // Anonymous on accessible QueueLogs vs inaccessible Platform.
        let anon_vm = ViewModel {
            auth: AuthState::anonymous(vec![
                "system:dashboard.read".into(),
                "ci:queue.read".into(),
                "ci:repos.public.read".into(),
            ]),
            ..ViewModel::default()
        };
        st.on_worker_frame(&encode(&DataEvent::Snapshot(anon_vm)).expect("encode"));
        assert_eq!(st.route.subview, SubView::Ci(CiSubView::QueueLogs));
        assert!(st.is_filter_applicable());
        assert_eq!(
            st.filter_placeholder_text(),
            "Filter public repos, branches, or jobs..."
        );

        st.apply_hash("#/dashboard/overview");
        assert!(st.is_filter_applicable());
        assert_eq!(st.filter_placeholder_text(), "Filter applications...");

        st.apply_hash("#/platform/users");
        assert!(!st.is_filter_applicable());
        assert_eq!(st.filter_placeholder_text(), "Sign in to filter...");

        // PasskeyEnrollmentRequired: public QueueLogs applicable, protected Scorecards disabled.
        let passkey_vm = ViewModel {
            auth: AuthState {
                status: AuthStatus::PasskeyEnrollmentRequired,
                github_login: Some("newop".into()),
                passkey_count: 0,
                passkey_enrollment_required: true,
                ..AuthState::default()
            },
            ..ViewModel::default()
        };
        st.on_worker_frame(&encode(&DataEvent::Snapshot(passkey_vm)).expect("encode"));
        st.apply_hash("#/ci/logs");
        assert!(st.is_filter_applicable());
        assert_eq!(
            st.filter_placeholder_text(),
            "Filter public repos, branches, or jobs..."
        );
        st.apply_hash("#/ci/scorecards");
        assert!(!st.is_filter_applicable());
        assert_eq!(st.filter_placeholder_text(), "Enroll passkey to unlock...");

        // Active with dormant bound role, expanded repo filter, and new filter target controls.
        let mut repo_pass = RepoScore::new(RepoId::new("LMA1980", "mu-console"));
        repo_pass.default_branch = "feat/cool-branch".into();
        repo_pass.status = RepoStatus::Passing;
        let mut repo_fail = RepoScore::new(RepoId::new("LMA1980", "k3-core"));
        repo_fail.default_branch = "main".into();
        repo_fail.status = RepoStatus::Failing;

        let active_vm = ViewModel {
            auth: AuthState {
                status: AuthStatus::Active,
                github_login: Some("ops".into()),
                passkey_count: 1,
                bound_roles: vec!["operator".into(), "sec-ops".into()],
                bound_role_specs: vec![RoleSpecDto {
                    id: "super-operator".into(),
                    ..RoleSpecDto::default()
                }],
                active_roles: vec![RoleBadgeView {
                    role_id: "operator".into(),
                    ..RoleBadgeView::default()
                }],
                active_policies: vec![
                    "system:dashboard.read".into(),
                    "ci:queue.read".into(),
                    "ci:nodes.read".into(),
                    "obs:smartscape.read".into(),
                ],
                ..AuthState::default()
            },
            repos: vec![repo_pass, repo_fail],
            observe: ObserveView {
                smartscape_nodes: vec![
                    SmartscapeNode {
                        id: "svc-b".into(),
                        label: "beta-svc".into(),
                        layer: SmartscapeLayer::Service,
                        req_per_sec_bp: 100,
                        error_rate_bp: 0,
                        p95_ms: 10,
                        healthy: true,
                    },
                    SmartscapeNode {
                        id: "svc-a".into(),
                        label: "alpha-svc".into(),
                        layer: SmartscapeLayer::Service,
                        req_per_sec_bp: 100,
                        error_rate_bp: 0,
                        p95_ms: 10,
                        healthy: true,
                    },
                ],
                ..ObserveView::default()
            },
            ..ViewModel::default()
        };
        st.on_worker_frame(&encode(&DataEvent::Snapshot(active_vm)).expect("encode"));
        assert!(st.has_dormant_bound_roles());

        st.apply_hash("#/dashboard/overview");
        assert_eq!(
            st.filter_placeholder_text(),
            "Filter applications, nodes, or alerts..."
        );

        st.apply_hash("#/ci/logs");
        assert_eq!(
            st.filter_placeholder_text(),
            "Filter repos, branches, or jobs..."
        );

        st.apply_hash("#/platform/maintenance");
        assert!(!st.is_filter_applicable());
        assert_eq!(
            st.filter_placeholder_text(),
            "Access restricted for active roles"
        );
        assert_eq!(
            st.route.subview,
            SubView::Platform(PlatformSubView::Maintenance)
        );

        // auth-gate-dashboard & auth-gate-signout actions.
        assert_eq!(
            st.activate_target("auth-gate-dashboard"),
            vec![UiCommand::SelectSubView(SubView::Dashboard(
                DashboardSubView::Overview
            ))]
        );
        assert_eq!(
            st.activate_target("auth-gate-signout"),
            vec![UiCommand::Navigate {
                href: "/oauth2/sign_out?rd=%2F".into()
            }]
        );

        // filtered_repos matches branch and status.
        st.set_filter_query("feat/cool-branch", 50);
        let by_branch = st.filtered_repos();
        assert_eq!(by_branch.len(), 1);
        assert_eq!(by_branch[0].id.repo, "mu-console");
        st.set_filter_query("Failing", 60);
        let by_status = st.filtered_repos();
        assert_eq!(by_status.len(), 1);
        assert_eq!(by_status[0].id.repo, "k3-core");
        st.set_filter_query("", 70);

        // CI queue & Observability filter target controls.
        st.subview_page = 3;
        assert!(st.activate_target("ci-queue-filter:running").is_empty());
        assert_eq!(st.ci_queue_filter, 1);
        assert_eq!(st.subview_page, 0);
        assert!(st.activate_target("ci-queue-filter:all").is_empty());
        assert_eq!(st.ci_queue_filter, 0);
        assert!(st.activate_target("ci-queue-filter:failed").is_empty());
        assert_eq!(st.ci_queue_filter, 2);

        st.apply_hash("#/observe/smartscape");
        assert_eq!(
            st.filter_placeholder_text(),
            "Filter services, hosts, processes, or layers..."
        );
        assert!(st
            .activate_target("obs-smartscape-filter:services")
            .is_empty());
        assert_eq!(st.obs_smartscape_filter, 1);
        assert!(st
            .activate_target("obs-smartscape-filter:processes")
            .is_empty());
        assert_eq!(st.obs_smartscape_filter, 2);
        assert!(st.activate_target("obs-smartscape-filter:hosts").is_empty());
        assert_eq!(st.obs_smartscape_filter, 3);
        assert!(st.activate_target("obs-smartscape-filter:all").is_empty());
        assert_eq!(st.obs_smartscape_filter, 0);

        assert!(st.activate_target("obs-service-next").is_empty());
        assert_eq!(st.obs_service_filter.as_deref(), Some("alpha-svc"));
        assert!(st.activate_target("obs-service-prev").is_empty());
        assert_eq!(st.obs_service_filter, None);
        assert!(st.activate_target("obs-service-prev").is_empty());
        assert_eq!(st.obs_service_filter.as_deref(), Some("beta-svc"));
        assert!(st.activate_target("obs-service-clear").is_empty());
        assert_eq!(st.obs_service_filter, None);

        let _ = ObservabilitySubView::ALL;
    }

    #[test]
    fn visible_ci_jobs_gates_raw_log_search_on_log_read_policy() {
        let mut job = CiJobSummary {
            job_id_str: "job-77".into(),
            job_id: 77,
            repo: RepoId::new("LMA1980", "mu-console"),
            ref_name: "main".into(),
            commit_sha: "abc1234".into(),
            queue_state: "running".into(),
            elapsed_secs: 10,
            wait_secs: 0,
            quality_score: 99,
            coverage_bp: 9000,
            tests_passed: 10,
            tests_total: 10,
            benchmark_summary: String::new(),
            stages: vec![],
            raw_log_lines: vec!["compiling crate\n".into(), "LOG_NEEDLE failed\n".into()],
        };
        job.raw_log_lines.push("finished".into());

        let mut st = ConsoleState::new();
        let vm_without_logs = ViewModel {
            auth: AuthState {
                status: AuthStatus::Active,
                github_login: Some("ops".into()),
                passkey_count: 1,
                active_policies: vec!["ci:queue.read".into(), "ci:repos.public.read".into()],
                ..AuthState::default()
            },
            ci_jobs: vec![job.clone()],
            ..ViewModel::default()
        };
        st.on_worker_frame(&encode(&DataEvent::Snapshot(vm_without_logs)).expect("encode"));
        st.set_filter_query("LOG_NEEDLE", 10);
        assert!(st.visible_ci_jobs().is_empty());
        st.set_filter_query("#77", 20);
        assert_eq!(st.visible_ci_jobs().len(), 1);

        let vm_with_logs = ViewModel {
            auth: AuthState {
                status: AuthStatus::Active,
                github_login: Some("ops".into()),
                passkey_count: 1,
                active_policies: vec![
                    "ci:queue.read".into(),
                    "ci:repos.public.read".into(),
                    "ci:jobs.logs.read".into(),
                ],
                ..AuthState::default()
            },
            ci_jobs: vec![job],
            ..ViewModel::default()
        };
        st.on_worker_frame(&encode(&DataEvent::Snapshot(vm_with_logs)).expect("encode"));
        st.set_filter_query("LOG_NEEDLE", 30);
        assert_eq!(st.visible_ci_jobs().len(), 1);
    }

    #[test]
    fn service_filter_matches_normalizes_service_aliases_and_separators() {
        // Canonical service alias normalization across hyphenated and compact forms.
        assert!(service_filter_matches("muregist", "mu-registry"));
        assert!(service_filter_matches("mu-registry", "muregist"));
        assert!(service_filter_matches("muregist-daemon", "mu-registry"));
        assert!(service_filter_matches("muregist", "svc:muregist"));
        assert!(service_filter_matches("mu_registry", "mu-registry"));
        assert!(service_filter_matches(
            "muregist error",
            "[mu-registry] HTTP 500 error"
        ));
        assert!(!service_filter_matches(
            "muregist",
            "[mu-console] HTTP 500 error"
        ));
        assert!(!service_filter_matches(
            "muregist error",
            "[mu-registry] HTTP 200 ok"
        ));

        // Additional canonical platform service aliases.
        assert!(service_filter_matches("mu-ci-worker", "actionrunner"));
        assert!(service_filter_matches(
            "mu-observable",
            "muobserve-collector"
        ));
        assert!(!service_filter_matches(
            "mu-ci-worker",
            "actionrunner-controller"
        ));

        // Preserves existing multi-term case-insensitive substring matching.
        assert!(service_filter_matches(
            "runner controller",
            "actionrunner-controller"
        ));
        assert!(service_filter_matches("", "anything"));
        assert!(service_filter_matches("   ", "anything"));
        assert!(!service_filter_matches("---", "muregist"));
    }

    #[test]
    fn filter_debounce_escape_and_subview_selection_commands() {
        use mu_console_protocol::dto::AuthSessionDto;
        use mu_console_protocol::ObservabilitySubView;

        let mut st = ConsoleState::new();
        let vm = ViewModel {
            auth: AuthState::from_session(&AuthSessionDto {
                authenticated: true,
                github_login: Some("ops".into()),
                invite_bound: true,
                passkey_count: 1,
                active_policies: vec![
                    "system:dashboard.read".into(),
                    "obs:logs.read".into(),
                    "obs:metrics.read".into(),
                    "ci:queue.read".into(),
                ],
                ..AuthSessionDto::default()
            }),
            ..ViewModel::default()
        };
        st.on_worker_frame(&encode(&DataEvent::Snapshot(vm)).expect("encode"));

        // Empty filter -> SelectSubView.
        assert_eq!(
            st.select_subview(SubView::Observability(ObservabilitySubView::Logs)),
            vec![UiCommand::SelectSubView(SubView::Observability(
                ObservabilitySubView::Logs
            ))]
        );

        // Typing schedules debounced Subscribe at now + FILTER_SETTLE_MS.
        st.set_filter_query("muregist", 1_000);
        assert_eq!(st.pending_filter_at_ms, Some(1_000 + FILTER_SETTLE_MS));
        assert_eq!(st.route.filter.as_deref(), Some("muregist"));
        assert!(st.tick(1_000 + FILTER_SETTLE_MS - 1).is_empty());

        let settled = st.tick(1_000 + FILTER_SETTLE_MS);
        assert_eq!(st.pending_filter_at_ms, None);
        assert_eq!(settled.len(), 1);
        match &settled[0] {
            UiCommand::Subscribe(route) => {
                assert_eq!(
                    route.subview,
                    SubView::Observability(ObservabilitySubView::Logs)
                );
                assert_eq!(route.filter.as_deref(), Some("muregist"));
            }
            other => panic!("expected Subscribe, got {other:?}"),
        }

        // Rapid typing resets debounce deadline.
        st.set_filter_query("mu", 2_000);
        st.set_filter_query("muregist", 2_100);
        assert!(st.tick(2_000 + FILTER_SETTLE_MS).is_empty());
        assert_eq!(st.tick(2_100 + FILTER_SETTLE_MS).len(), 1);

        // Navigating to Logs or QueueLogs while filter is non-empty uses Subscribe.
        let cmds = st.select_subview(SubView::Observability(ObservabilitySubView::Logs));
        assert!(matches!(
            cmds.as_slice(),
            [UiCommand::Subscribe(r)] if r.filter.as_deref() == Some("muregist")
        ));
        let ci_cmds = st.select_subview(SubView::Ci(CiSubView::QueueLogs));
        assert!(matches!(
            ci_cmds.as_slice(),
            [UiCommand::Subscribe(r)] if r.filter.as_deref() == Some("muregist")
        ));

        // Navigating to a server-filtered area fallback (Logs) preserves filter via Subscribe,
        // while a non-server-filtered area/subview emits SelectSubView.
        assert!(matches!(
            st.select_area(Area::Observability),
            UiCommand::Subscribe(ref r)
                if r.subview == SubView::Observability(ObservabilitySubView::Logs)
                    && r.filter.as_deref() == Some("muregist")
        ));
        assert_eq!(
            st.select_area(Area::Dashboard),
            UiCommand::SelectSubView(SubView::Dashboard(
                mu_console_protocol::DashboardSubView::Overview
            ))
        );
        assert_eq!(
            st.select_subview(SubView::Observability(ObservabilitySubView::Metrics)),
            vec![UiCommand::SelectSubView(SubView::Observability(
                ObservabilitySubView::Metrics
            ))]
        );
        let _ = st.select_subview(SubView::Observability(ObservabilitySubView::Logs));

        // Escape clears filter_query & pending_filter_at_ms and emits Subscribe with filter: None.
        st.set_filter_query("muregist", 3_000);
        let esc_cmd = st.on_key("Escape", false, 3_050);
        assert!(st.filter_query.is_empty());
        assert_eq!(st.route.filter, None);
        assert_eq!(st.pending_filter_at_ms, None);
        match esc_cmd {
            Some(UiCommand::Subscribe(route)) => {
                assert_eq!(route.filter, None);
                assert_eq!(
                    route.subview,
                    SubView::Observability(ObservabilitySubView::Logs)
                );
            }
            other => panic!("expected Some(Subscribe), got {other:?}"),
        }

        // Clearing filter via set_filter_query("") settles to Subscribe with filter: None.
        st.set_filter_query("muregist", 4_000);
        let _ = st.tick(4_000 + FILTER_SETTLE_MS);
        st.set_filter_query("", 5_000);
        let cleared = st.tick(5_000 + FILTER_SETTLE_MS);
        assert!(matches!(
            cleared.as_slice(),
            [UiCommand::Subscribe(r)] if r.filter.is_none()
        ));

        // Escape on non-server-filtered subview clears filter and returns None.
        let _ = st.select_subview(SubView::Ci(CiSubView::Trainway));
        st.set_filter_query("foo", 6_000);
        assert_eq!(st.on_key("Escape", false, 6_050), None);
        assert!(st.filter_query.is_empty());
        assert_eq!(st.pending_filter_at_ms, None);
    }
}
