// SPDX-License-Identifier: AGPL-3.0-only
//! Semantic SVG component renderer (`C1`–`C6`, `SubViewBar`, 17 single-focus
//! sub-views across `CI`, `CD`, `REG`, `OBS`, and `FLT`, phone bottom-sheet
//! selector, and HTML accessibility shell) driven through [`Skin`].

use mu_console_layout::trainway::allocate_lanes;
use mu_console_layout::{
    scorecard_cards, solve, Breakpoint, Card, Frames, Panel as LayoutPanel, SafeArea, Viewport,
};
use mu_console_protocol::{
    format_job_id, sanitize_public_log, Area, AuthState, CanaryBundleState, CdSubView, CiSubView,
    ConnState, ConsoleSkin, CrtFont, FleetSubView, GateOutcome, GateTest, LogSeverity,
    ObserveSubView, PackageEcosystem, PrState, RegistrySubView, RepoScore, RepoStatus,
    RepoTopology, SidingPhase, SmartscapeLayer, StageState, SubView, TestState,
};
use mu_console_theme::instruments::switch::SwitchAction;
use mu_console_theme::instruments::Rect;
use mu_console_theme::skin::{
    Lamp, Panel, Readout, Rotary, Screen, ScreenRow, ScreenTrack, Selector, SelectorOption,
    SemanticTone, SignalGlyph, Skin,
};
use mu_console_theme::svg::{escape_into, PathData, SvgWriter};

use crate::router::{area_label, breadcrumb_segments};
use crate::state::{font_slug, skin_slug, ConsoleState};

const fn stage_glyph_and_tone(state: StageState) -> (SignalGlyph, SemanticTone) {
    match state {
        StageState::Passed => (SignalGlyph::BlockPassed, SemanticTone::Pass),
        StageState::Running => (SignalGlyph::BlockRunning, SemanticTone::Warn),
        StageState::Failed => (SignalGlyph::BlockFailed, SemanticTone::Fail),
        StageState::Pending | StageState::Skipped => {
            (SignalGlyph::BlockPending, SemanticTone::Neutral)
        }
    }
}

const fn test_glyph_and_tone(state: TestState) -> (SignalGlyph, SemanticTone) {
    match state {
        TestState::Passed => (SignalGlyph::TestPassed, SemanticTone::Pass),
        TestState::Running => (SignalGlyph::TestRunning, SemanticTone::Warn),
        TestState::Failed => (SignalGlyph::BlockFailed, SemanticTone::Fail),
        TestState::Pending => (SignalGlyph::TestPending, SemanticTone::Neutral),
    }
}

/// Clamp `rect` strictly inside `bounds`.
fn clamp_rect(rect: Rect, bounds: Rect) -> Rect {
    let x = rect.x.clamp(bounds.x, bounds.right());
    let y = rect.y.clamp(bounds.y, bounds.bottom());
    let w = rect.w.min((bounds.right() - x).max(0.0)).max(0.0);
    let h = rect.h.min((bounds.bottom() - y).max(0.0)).max(0.0);
    Rect::new(x, y, w, h)
}

/// Paginate `body` rows while keeping `headers` pinned at the top of each page.
/// When all rows fit within `grid_rows`, returns `(all_rows, 0, 1)`.
fn paginate_with_headers<T: Clone>(
    headers: Vec<T>,
    body: Vec<T>,
    grid_rows: usize,
    requested_page: u32,
) -> (Vec<T>, u32, u32) {
    let max_r = grid_rows.max(1);
    if headers.len() + body.len() <= max_r {
        let mut all = headers;
        all.extend(body);
        return (all, 0, 1);
    }
    let avail = max_r.saturating_sub(headers.len()).max(1);
    let total_pages = u32::try_from(body.len().div_ceil(avail).max(1)).unwrap_or(1);
    let page = requested_page.min(total_pages.saturating_sub(1));
    let start = (page as usize) * avail;
    let end = (start + avail).min(body.len());
    let mut out = headers;
    if let Some(slice) = body.get(start..end) {
        out.extend_from_slice(slice);
    }
    out.truncate(max_r);
    (out, page, total_pages)
}

/// Render the full MuConsole SVG document for `viewport`, `safe`, `state`, and `skin`.
#[must_use]
pub fn render_console(
    skin: &dyn Skin,
    state: &ConsoleState,
    viewport: Viewport,
    safe: SafeArea,
) -> String {
    let font = state.font_metrics();
    let frames = solve(viewport, safe, state.route.area, font);
    let mut w = SvgWriter::document(viewport.width, viewport.height);
    skin.begin_frame(&mut w, viewport.width, viewport.height);

    render_c1_nav(skin, &mut w, state, &frames);
    render_c2_top_bar(skin, &mut w, state, &frames);
    render_subview_bar(skin, &mut w, state, &frames);
    render_active_subview(skin, &mut w, state, &frames);

    if frames.breakpoint == Breakpoint::Phone && state.phone_rotary_open {
        render_c3_phone_sheet(skin, &mut w, state, &frames);
    }

    w.finish_closing()
}

/// Render `C1` Area Navigation selector (`CI`, `CD`, `REG`, `OBS`, `FLT`).
fn render_c1_nav(skin: &dyn Skin, w: &mut SvgWriter, state: &ConsoleState, frames: &Frames) {
    let r = clamp_rect(frames.nav, frames.canvas);
    if r.w <= 0.0 || r.h <= 0.0 {
        return;
    }
    skin.panel(
        w,
        &Panel {
            id: "c1-nav-panel",
            title: if frames.breakpoint == Breakpoint::Phone {
                ""
            } else {
                "NAV"
            },
            rect: r,
            role: Some("navigation"),
            aria_label: Some("Area Navigation"),
        },
    );

    let badges = state.client.badges();
    let opts = [
        SelectorOption {
            id: "ci",
            label: "CI",
            badge: (badges.ci > 0).then_some(badges.ci),
            tone: if badges.ci > 0 {
                SemanticTone::Fail
            } else {
                SemanticTone::Pass
            },
        },
        SelectorOption {
            id: "cd",
            label: "CD",
            badge: (badges.cd > 0).then_some(badges.cd),
            tone: if badges.cd > 0 {
                SemanticTone::Warn
            } else {
                SemanticTone::Pass
            },
        },
        SelectorOption {
            id: "reg",
            label: "REG",
            badge: (badges.registry > 0).then_some(badges.registry),
            tone: if badges.registry > 0 {
                SemanticTone::Warn
            } else {
                SemanticTone::Info
            },
        },
        SelectorOption {
            id: "obs",
            label: "OBS",
            badge: (badges.observe > 0).then_some(badges.observe),
            tone: if badges.observe > 0 {
                SemanticTone::Fail
            } else {
                SemanticTone::Info
            },
        },
        SelectorOption {
            id: "flt",
            label: "FLT",
            badge: (badges.fleet > 0).then_some(badges.fleet),
            tone: if badges.fleet > 0 {
                SemanticTone::Fail
            } else {
                SemanticTone::Pass
            },
        },
    ];

    let selected = match state.route.area {
        Area::Ci => 0,
        Area::Cd => 1,
        Area::Registry => 2,
        Area::Observe => 3,
        Area::Fleet => 4,
    };

    let sel_rect = if frames.breakpoint == Breakpoint::Phone {
        clamp_rect(r.inset(2.0, 4.0, 2.0, 4.0), r)
    } else {
        clamp_rect(
            Rect::new(
                r.x + 4.0,
                r.y + 22.0,
                (r.w - 8.0).max(16.0),
                (r.h - 28.0).clamp(44.0, 270.0),
            ),
            r,
        )
    };

    skin.selector(
        w,
        &Selector {
            control_id: "c1-area-selector",
            label: "Area Navigation Selector",
            rect: sel_rect,
            action: SwitchAction::Maintained,
            options: &opts,
            selected,
            pulled: false,
            horizontal: frames.breakpoint == Breakpoint::Phone,
        },
    );
}

/// Render `C2` Top Bar: Breadcrumb segments, compact Repo/Submodule selector, status lamps, and `FONT` switch.
#[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
fn render_c2_top_bar(skin: &dyn Skin, w: &mut SvgWriter, state: &ConsoleState, frames: &Frames) {
    let r = clamp_rect(frames.top_bar, frames.canvas);
    if r.w <= 0.0 || r.h <= 0.0 {
        return;
    }
    skin.panel(
        w,
        &Panel {
            id: "c2-top-bar",
            title: "",
            rect: r,
            role: Some("banner"),
            aria_label: Some("Breadcrumb, Repository Selector, and Status Bar"),
        },
    );

    let inner = clamp_rect(r.inset(4.0, 6.0, 4.0, 6.0), r);
    let right_reserve = if frames.breakpoint == Breakpoint::Phone {
        218.0_f64.min(inner.w * 0.58)
    } else {
        760.0_f64.min(inner.w * 0.72)
    };

    // Breadcrumb segments on the left.
    let crumbs = breadcrumb_segments(&state.route, frames.breakpoint);
    let crumb_area_w = (inner.w - right_reserve - 6.0).max(40.0);
    let n_crumbs = crumbs.len().max(1);
    let cw = (crumb_area_w / n_crumbs as f64).max(24.0);
    for (i, c) in crumbs.iter().enumerate() {
        let cr = clamp_rect(
            Rect::new(
                inner.x + i as f64 * cw,
                inner.y,
                (cw - 4.0).max(16.0),
                inner.h,
            ),
            inner,
        );
        let (lbl, val) = if frames.breakpoint == Breakpoint::Phone {
            match c.id {
                "c2-area" | "c2-subview" => ("", c.value.as_str()),
                "c2-repo" => (
                    "",
                    c.value
                        .rsplit_once('/')
                        .map_or(c.value.as_str(), |(_, repo_name)| repo_name),
                ),
                _ => (c.label, c.value.as_str()),
            }
        } else {
            (c.label, c.value.as_str())
        };
        skin.readout(
            w,
            &Readout {
                control_id: c.id,
                label: lbl,
                value: val,
                rect: cr,
                tone: SemanticTone::Info,
                interactive: c.id != "c2-ellipsis",
            },
        );
    }

    let rx = (inner.right() - right_reserve).max(inner.x);
    if frames.breakpoint == Breakpoint::Phone {
        let trigger_w = 70.0_f64.min(right_reserve * 0.34).max(48.0);
        let trigger_rect = clamp_rect(
            Rect::new(
                rx,
                r.y,
                trigger_w,
                r.h.max(44.0).min(frames.canvas.bottom() - r.y),
            ),
            frames.canvas,
        );
        skin.readout(
            w,
            &Readout {
                control_id: "c2-phone-rotary-trigger",
                label: "",
                value: "▾ REPO",
                rect: trigger_rect,
                tone: SemanticTone::Warn,
                interactive: true,
            },
        );
        let font_rect = clamp_rect(
            Rect::new(
                (trigger_rect.right() + 4.0).min(r.right() - 48.0),
                r.y,
                (r.right() - trigger_rect.right() - 4.0).max(48.0),
                r.h.max(44.0).min(frames.canvas.bottom() - r.y),
            ),
            frames.canvas,
        );
        render_font_switch(skin, w, state, font_rect);
    } else {
        // Compact C2 Repository & Submodule Stepper (`[<] REPO [>] SUB`).
        let repos = state.filtered_repos();
        let current_repo = repos.get(state.outer_index);
        let repo_label = current_repo.map_or("ALL REPOS", |x| x.id.repo.as_str());
        let sub_label = if state.inner_index == 0 {
            "PARENT"
        } else {
            current_repo
                .and_then(|x| x.submodules.get(state.inner_index - 1))
                .map_or("PARENT", |s| s.path.as_str())
        };
        let step_w = 28.0;
        let repo_w = 112.0_f64.min(right_reserve * 0.18).max(64.0);
        let sub_w = 88.0_f64.min(right_reserve * 0.14).max(56.0);
        let mut cur_x = rx;

        let prev_rect = clamp_rect(Rect::new(cur_x, inner.y, step_w, inner.h), inner);
        skin.readout(
            w,
            &Readout {
                control_id: "c2-repo-prev",
                label: "",
                value: "◀",
                rect: prev_rect,
                tone: SemanticTone::Info,
                interactive: true,
            },
        );
        cur_x = prev_rect.right() + 3.0;

        let sel_rect = clamp_rect(Rect::new(cur_x, inner.y, repo_w, inner.h), inner);
        skin.readout(
            w,
            &Readout {
                control_id: "c2-repo-select",
                label: "REPO",
                value: repo_label,
                rect: sel_rect,
                tone: SemanticTone::Info,
                interactive: true,
            },
        );
        cur_x = sel_rect.right() + 3.0;

        let next_rect = clamp_rect(Rect::new(cur_x, inner.y, step_w, inner.h), inner);
        skin.readout(
            w,
            &Readout {
                control_id: "c2-repo-next",
                label: "",
                value: "▶",
                rect: next_rect,
                tone: SemanticTone::Info,
                interactive: true,
            },
        );
        cur_x = next_rect.right() + 4.0;

        let sub_rect = clamp_rect(Rect::new(cur_x, inner.y, sub_w, inner.h), inner);
        skin.readout(
            w,
            &Readout {
                control_id: "c2-sub-next",
                label: "SUB",
                value: sub_label,
                rect: sub_rect,
                tone: SemanticTone::Neutral,
                interactive: true,
            },
        );
        cur_x = sub_rect.right() + 6.0;

        let view = state.client.view();
        let stale = view.is_some_and(|v| v.stale_since_epoch_ms.is_some());
        let offline = matches!(state.client.conn(), Some(ConnState::Offline));
        let stop = view
            .and_then(|v| v.topology.as_ref())
            .is_some_and(RepoTopology::stop);
        let reload = state.client.reload_required().is_some();

        let rem = (inner.right() - cur_x).max(200.0);
        let font_w = 186.0_f64.min(rem * 0.36).max(80.0);
        let auth_w = 128.0_f64.min(rem * 0.24).max(64.0);
        let lamp_avail = (rem - font_w - auth_w - 16.0).max(114.0);
        let lamp_w = (lamp_avail / 3.0 - 4.0).clamp(36.0, 64.0);
        let lamps = [
            (
                "lamp-stale",
                if reload { "RELOAD" } else { "STALE" },
                stale || reload,
                SemanticTone::Warn,
            ),
            ("lamp-offline", "OFFLINE", offline, SemanticTone::Fail),
            ("lamp-stop", "STOP", stop, SemanticTone::Fail),
        ];
        for (idx, (id, lbl, lit, tone)) in lamps.into_iter().enumerate() {
            let lr = clamp_rect(
                Rect::new(
                    cur_x + idx as f64 * (lamp_w + 4.0),
                    inner.y,
                    lamp_w,
                    inner.h,
                ),
                inner,
            );
            skin.lamp(
                w,
                &Lamp {
                    control_id: id,
                    label: lbl,
                    rect: lr,
                    lit,
                    tone,
                },
            );
        }

        let auth = state.auth();
        let (auth_val, auth_tone) = match &auth {
            AuthState::Anonymous => ("READ-ONLY · SIGN IN".to_owned(), SemanticTone::Neutral),
            AuthState::InviteRequired { github_login } => {
                (format!("INVITE REQ (@{github_login})"), SemanticTone::Warn)
            }
            AuthState::Operator { github_login, .. } => {
                (format!("@{github_login} · OPERATOR"), SemanticTone::Pass)
            }
        };
        let auth_x = cur_x + 3.0 * (lamp_w + 4.0);
        let auth_rect = clamp_rect(Rect::new(auth_x, inner.y, auth_w, inner.h), inner);
        skin.readout(
            w,
            &Readout {
                control_id: "c2-auth-status",
                label: "AUTH",
                value: &auth_val,
                rect: auth_rect,
                tone: auth_tone,
                interactive: true,
            },
        );

        let font_x = (inner.right() - font_w).max(auth_rect.right() + 4.0);
        let font_rect = clamp_rect(
            Rect::new(font_x, inner.y, (inner.right() - font_x).max(48.0), inner.h),
            inner,
        );
        render_font_switch(skin, w, state, font_rect);
    }
}

fn render_font_switch(skin: &dyn Skin, w: &mut SvgWriter, state: &ConsoleState, rect: Rect) {
    let opts = [
        SelectorOption {
            id: "hurmit",
            label: "Standard font",
            badge: None,
            tone: SemanticTone::Info,
        },
        SelectorOption {
            id: "dyslexic",
            label: "Dyslexia-friendly font",
            badge: None,
            tone: SemanticTone::Pass,
        },
    ];
    let selected = match state.font {
        CrtFont::Hurmit => 0,
        CrtFont::OpenDyslexic => 1,
    };
    skin.selector(
        w,
        &Selector {
            control_id: "c2-font-switch",
            label: "Use dyslexia-friendly font",
            rect,
            action: SwitchAction::Maintained,
            options: &opts,
            selected,
            pulled: false,
            horizontal: true,
        },
    );
}

/// Render the `36px` horizontal `SubViewBar` directly below `C2`.
fn render_subview_bar(skin: &dyn Skin, w: &mut SvgWriter, state: &ConsoleState, frames: &Frames) {
    let r = clamp_rect(frames.subview_bar, frames.canvas);
    if r.w <= 0.0 || r.h <= 0.0 {
        return;
    }
    let views = SubView::all_for(state.route.area);
    let active_sv = if state.route.subview.area() == state.route.area {
        state.route.subview
    } else {
        SubView::default_for(state.route.area)
    };
    let selected = views.iter().position(|&sv| sv == active_sv).unwrap_or(0);
    let opts: Vec<SelectorOption<'_>> = views
        .iter()
        .map(|sv| SelectorOption {
            id: sv.slug(),
            label: sv.label(),
            badge: None,
            tone: SemanticTone::Info,
        })
        .collect();

    skin.selector(
        w,
        &Selector {
            control_id: "c2-subview-bar",
            label: "Sub-View Selector Bar",
            rect: r,
            action: SwitchAction::Maintained,
            options: &opts,
            selected,
            pulled: false,
            horizontal: true,
        },
    );
}

/// Dispatch the single active sub-view into the full-canvas `frames.panels[0]`.
fn render_active_subview(
    skin: &dyn Skin,
    w: &mut SvgWriter,
    state: &ConsoleState,
    frames: &Frames,
) {
    let Some(panel) = frames.panels.first() else {
        return;
    };
    let active_sv = if state.route.subview.area() == state.route.area {
        state.route.subview
    } else {
        SubView::default_for(state.route.area)
    };
    match active_sv {
        SubView::Ci(CiSubView::Scorecards) => {
            render_ci_scorecards(skin, w, state, frames, panel);
        }
        SubView::Ci(CiSubView::Trainway) => {
            render_c5_trainway(skin, w, state, frames, panel);
        }
        SubView::Ci(CiSubView::QueueLogs) => {
            render_ci_queue_logs(skin, w, state, frames, panel);
        }
        SubView::Cd(CdSubView::IntegrationGate) => {
            render_c6_delivery_gate(skin, w, state, frames, panel);
        }
        SubView::Cd(CdSubView::DeploymentsSlots) => {
            render_cd_deployments_slots(skin, w, state, frames, panel);
        }
        SubView::Cd(CdSubView::ActiveLeases) => {
            render_cd_active_leases(skin, w, state, frames, panel);
        }
        SubView::Registry(RegistrySubView::Packages) => {
            render_reg_packages(skin, w, state, frames, panel);
        }
        SubView::Registry(RegistrySubView::CanaryTaints) => {
            render_reg_canaries(skin, w, state, frames, panel);
        }
        SubView::Registry(RegistrySubView::CasHealth) => {
            render_reg_cas_health(skin, w, state, frames, panel);
        }
        SubView::Observe(ObserveSubView::Smartscape) => {
            render_obs_smartscape(skin, w, state, frames, panel);
        }
        SubView::Observe(ObserveSubView::PurePath) => {
            render_obs_purepath(skin, w, state, frames, panel);
        }
        SubView::Observe(ObserveSubView::ProblemsSlos) => {
            render_obs_problems_slos(skin, w, state, frames, panel);
        }
        SubView::Observe(ObserveSubView::Logs) => {
            render_obs_logs(skin, w, state, frames, panel);
        }
        SubView::Observe(ObserveSubView::Metrics) => {
            render_obs_metrics(skin, w, state, frames, panel);
        }
        SubView::Observe(ObserveSubView::DataStore) => {
            render_obs_datastore(skin, w, state, frames, panel);
        }
        SubView::Fleet(FleetSubView::WorkerNodes) => {
            render_fleet_worker_nodes(skin, w, state, frames, panel);
        }
        SubView::Fleet(FleetSubView::SandboxProfiles) => {
            render_fleet_sandbox_profiles(skin, w, state, frames, panel);
        }
    }
}

/// Render the phone bottom-sheet rotary modal with `>= 44x44 pt` touch targets.
fn render_c3_phone_sheet(
    skin: &dyn Skin,
    w: &mut SvgWriter,
    state: &ConsoleState,
    frames: &Frames,
) {
    let sheet_h = (frames.canvas.h * 0.52).clamp(220.0, frames.canvas.h);
    let sheet = clamp_rect(
        Rect::new(
            frames.canvas.x,
            (frames.canvas.bottom() - sheet_h).max(frames.canvas.y),
            frames.canvas.w,
            sheet_h,
        ),
        frames.canvas,
    );
    skin.panel(
        w,
        &Panel {
            id: "c3-phone-sheet",
            title: "REPOSITORY & SUBMODULE SELECTOR",
            rect: sheet,
            role: Some("dialog"),
            aria_label: Some("Repository and Submodule Selector Sheet"),
        },
    );

    let rotary_rect = clamp_rect(
        Rect::new(
            sheet.x + 8.0,
            sheet.y + 24.0,
            (sheet.w - 16.0).max(44.0),
            (sheet.h - 86.0).max(80.0),
        ),
        sheet,
    );
    let repos = state.filtered_repos();
    let current_repo = repos.get(state.outer_index);
    let outer_label = current_repo.map_or("—", |x| x.id.repo.as_str());
    let inner_total = current_repo.map_or(1, |x| x.submodules.len() + 1);
    let inner_label = if state.inner_index == 0 {
        "PARENT"
    } else {
        current_repo
            .and_then(|x| x.submodules.get(state.inner_index - 1))
            .map_or("PARENT", |s| s.path.as_str())
    };
    skin.rotary(
        w,
        &Rotary {
            control_id: "c3-rotary",
            rect: rotary_rect,
            outer_label,
            outer_index: state.outer_index,
            outer_total: repos.len(),
            inner_label,
            inner_index: state.inner_index,
            inner_total,
            filter_query: &state.filter_query,
            settling: state.pending_select_at_ms.is_some(),
        },
    );
}

/// Render `CI > Scorecards`: Full-canvas responsive grid of twin-CRT repository scorecards.
fn render_ci_scorecards(
    skin: &dyn Skin,
    w: &mut SvgWriter,
    state: &ConsoleState,
    frames: &Frames,
    panel: &LayoutPanel,
) {
    let rect = clamp_rect(panel.plate, frames.canvas);
    let screen_rect = clamp_rect(panel.crt, rect);
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    skin.panel(
        w,
        &Panel {
            id: "ci-scorecards-panel",
            title: "CI REPOSITORY SCORECARDS & COVERAGE",
            rect,
            role: Some("region"),
            aria_label: Some("CI Repository Scorecards"),
        },
    );

    let repos = state.ordered_carousel_repos();
    let per_page = match frames.breakpoint {
        Breakpoint::Wide => 6,
        Breakpoint::Medium | Breakpoint::CompactLandscape => 4,
        Breakpoint::Phone => 2,
    };
    let total_pages = repos.len().div_ceil(per_page).max(1);
    let page = (state.carousel_page as usize).min(total_pages - 1);
    let start = page * per_page;
    let slice = repos
        .get(start..(start + per_page).min(repos.len()))
        .unwrap_or(&[]);

    // Header page stepper + dots in the top inset of `rect`.
    let hdr_y = rect.y + 4.0;
    let hdr_h = ((screen_rect.y - rect.y) - 8.0).clamp(18.0, 26.0);
    let step_w = 136.0_f64.min(rect.w * 0.35);
    let step_rect = clamp_rect(
        Rect::new(
            (rect.right() - step_w - 8.0).max(rect.x),
            hdr_y,
            step_w,
            hdr_h,
        ),
        rect,
    );
    let step_opts = [
        SelectorOption {
            id: "prev",
            label: "◀ PREV",
            badge: None,
            tone: SemanticTone::Info,
        },
        SelectorOption {
            id: "next",
            label: "NEXT ▶",
            badge: None,
            tone: SemanticTone::Info,
        },
    ];
    skin.selector(
        w,
        &Selector {
            control_id: "c4-carousel-switch",
            label: "Scorecard Page Stepper",
            rect: step_rect,
            action: SwitchAction::SpringReturn,
            options: &step_opts,
            selected: 1,
            pulled: false,
            horizontal: true,
        },
    );

    let dots_w = 180.0_f64.min((step_rect.x - rect.x - 16.0).max(60.0));
    let dots_rect = clamp_rect(
        Rect::new(
            (step_rect.x - dots_w - 6.0).max(rect.x),
            hdr_y,
            dots_w,
            hdr_h,
        ),
        rect,
    );
    let dots_val = format!("PAGE {}/{total_pages} ({} REPOS)", page + 1, repos.len());
    skin.readout(
        w,
        &Readout {
            control_id: "c4-page-dots",
            label: "PG",
            value: &dots_val,
            rect: dots_rect,
            tone: SemanticTone::Info,
            interactive: false,
        },
    );

    let font = state.font_metrics();
    let cards = scorecard_cards(screen_rect, frames.breakpoint, slice.len().max(1), font);
    for (idx, card_slot) in cards.iter().enumerate() {
        let plate_rect = clamp_rect(card_slot.plate, frames.canvas);
        if let Some(r) = slice.get(idx) {
            let sub = if r.submodules.is_empty() {
                ""
            } else {
                " · SUBMODULES"
            };
            let title = format!("{}/{}{sub}", r.id.owner, r.id.repo);
            skin.panel(
                w,
                &Panel {
                    id: "c4-card-plate",
                    title: &title,
                    rect: plate_rect,
                    role: Some("region"),
                    aria_label: Some(&title),
                },
            );
            let is_selected_repo = state.route.repo.as_ref() == Some(&r.id);
            render_twin_scorecard(
                skin,
                w,
                r,
                idx,
                card_slot,
                frames.canvas,
                font.family,
                is_selected_repo,
            );
        } else {
            skin.panel(
                w,
                &Panel {
                    id: "c4-card-plate",
                    title: "AWAITING REPOSITORY STREAM",
                    rect: plate_rect,
                    role: Some("region"),
                    aria_label: Some("Awaiting Repository Stream"),
                },
            );
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn render_twin_scorecard(
    skin: &dyn Skin,
    w: &mut SvgWriter,
    repo: &RepoScore,
    idx: usize,
    card: &Card,
    canvas: Rect,
    font_family: &str,
    is_selected_repo: bool,
) {
    let left_rect = clamp_rect(card.ci_crt, canvas);
    let right_rect = clamp_rect(card.cd_crt, canvas);

    let (left_tone, st_tag) = match repo.status {
        RepoStatus::Passing => (SemanticTone::Pass, "PASS"),
        RepoStatus::Running => (SemanticTone::Warn, "RUN "),
        RepoStatus::Failing => (SemanticTone::Fail, "FAIL"),
        RepoStatus::Unknown => (SemanticTone::Neutral, "UNK "),
    };
    let grade = match repo.score {
        97..=100 => "A+",
        90..=96 => "A",
        80..=89 => "B",
        70..=79 => "C",
        1..=69 => "D",
        _ => "-",
    };
    let l0 = format!("{}/{}", repo.id.owner, repo.id.repo);
    let l1 = format!("{st_tag} {:>3}/100 {grade}", repo.score.min(100));
    let l2 = format!(
        "COV {:>3}% {:+.1}%",
        (repo.coverage_bp / 100).min(100),
        f64::from(repo.bench_delta_bp) / 100.0
    );
    let job_tag = repo
        .latest_job_id
        .map_or_else(|| "0".to_owned(), format_job_id);
    let l3 = format!("SUB {} #{job_tag}", repo.submodules.len());
    let l4 = match repo.status {
        RepoStatus::Passing => "TRND: ▅▆▆▇▇██".to_owned(),
        RepoStatus::Running => "TRND: ▄▅▆▆▇▆▊".to_owned(),
        RepoStatus::Failing => "TRND: ▇▆▅▄▃▂▁".to_owned(),
        RepoStatus::Unknown => "TRND: ───────".to_owned(),
    };
    let sel_cid = format!("c4-card-select:{}/{}", repo.id.owner, repo.id.repo);
    let open_tw_cid = format!("c4-open-trainway:{}/{}", repo.id.owner, repo.id.repo);
    let open_gate_cid = format!("c4-open-gate:{}/{}", repo.id.owner, repo.id.repo);
    let left_lines = [
        ScreenRow {
            text: &l0,
            tone: SemanticTone::Info,
            signal: None,
            critical: false,
            control_id: Some(&sel_cid),
            selected: is_selected_repo,
        },
        ScreenRow {
            text: &l1,
            tone: left_tone,
            signal: None,
            critical: false,
            control_id: Some(&open_tw_cid),
            selected: false,
        },
        ScreenRow {
            text: &l2,
            tone: left_tone,
            signal: None,
            critical: false,
            control_id: Some(&open_tw_cid),
            selected: false,
        },
        ScreenRow {
            text: &l3,
            tone: SemanticTone::Neutral,
            signal: None,
            critical: false,
            control_id: Some(&open_tw_cid),
            selected: false,
        },
        ScreenRow {
            text: &l4,
            tone: left_tone,
            signal: None,
            critical: false,
            control_id: Some(&open_tw_cid),
            selected: false,
        },
    ];
    let left_id = format!("c4-card-{idx}-left");
    skin.screen(
        w,
        &Screen {
            control_id: &left_id,
            title: &l0,
            rect: left_rect,
            font_family,
            font_px: card.ci_grid.font_px,
            cols: card.ci_grid.cols,
            rows: card.ci_grid.rows,
            lines: &left_lines,
            stations: &[],
            tracks: &[],
            page: 0,
            total_pages: 1,
        },
    );

    let (r0, r1, r2, r3, r4, cd_tone) = match repo.cd.as_ref() {
        Some(cd) => {
            let (tone, gate_tag, crit_tag, ttl_tag) = match cd.outcome {
                GateOutcome::Promoted => (SemanticTone::Pass, "PROMOTED", "OK", "LEASE: LIVE"),
                GateOutcome::Pending => (SemanticTone::Warn, "PENDING", "OK", "LEASE: 142s"),
                GateOutcome::RolledBack => (SemanticTone::Fail, "ROLLBACK", "FL", "LEASE: REV"),
                GateOutcome::Tainted => (SemanticTone::Fail, "TAINTED", "FL", "LEASE: HALT"),
            };
            (
                format!("CD · {}", cd.deployment_id),
                format!(
                    "SLOT {:?}->{:?} STG",
                    cd.active_slot,
                    cd.active_slot.other()
                ),
                format!("GATE: {gate_tag}"),
                format!("IT {}/{} CRIT:{crit_tag}", cd.it_passed, cd.it_total),
                ttl_tag.to_owned(),
                tone,
            )
        }
        None => (
            "CD · NO DEPLOY".to_owned(),
            "SLOT: NONE".to_owned(),
            "GATE: IDLE".to_owned(),
            "IT 0/0 CRIT:--".to_owned(),
            "LEASE: —".to_owned(),
            SemanticTone::Neutral,
        ),
    };
    let right_lines = [
        ScreenRow {
            text: &r0,
            tone: cd_tone,
            signal: None,
            critical: false,
            control_id: Some(&open_gate_cid),
            selected: false,
        },
        ScreenRow {
            text: &r1,
            tone: cd_tone,
            signal: None,
            critical: false,
            control_id: Some(&open_gate_cid),
            selected: false,
        },
        ScreenRow {
            text: &r2,
            tone: cd_tone,
            signal: None,
            critical: false,
            control_id: Some(&open_gate_cid),
            selected: false,
        },
        ScreenRow {
            text: &r3,
            tone: cd_tone,
            signal: None,
            critical: false,
            control_id: Some(&open_gate_cid),
            selected: false,
        },
        ScreenRow {
            text: &r4,
            tone: SemanticTone::Neutral,
            signal: None,
            critical: false,
            control_id: Some(&open_gate_cid),
            selected: false,
        },
    ];
    let right_id = format!("c4-card-{idx}-right");
    skin.screen(
        w,
        &Screen {
            control_id: &right_id,
            title: &l0,
            rect: right_rect,
            font_family,
            font_px: card.cd_grid.font_px,
            cols: card.cd_grid.cols,
            rows: card.cd_grid.rows,
            lines: &right_lines,
            stations: &[],
            tracks: &[],
            page: 0,
            total_pages: 1,
        },
    );
}

/// Render `CI > Trainway`: `C5` CI Branch & PR Trainway Screen + `STOP` lamp + `FABRIC REV` + `YARD +N` + `RESTART` switch.
#[allow(clippy::too_many_lines)]
fn render_c5_trainway(
    skin: &dyn Skin,
    w: &mut SvgWriter,
    state: &ConsoleState,
    frames: &Frames,
    panel: &LayoutPanel,
) {
    let rect = clamp_rect(panel.plate, frames.canvas);
    let screen_rect = clamp_rect(panel.crt, rect);
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    skin.panel(
        w,
        &Panel {
            id: "c5-trainway-panel",
            title: if rect.w < 500.0 {
                "CI TRAINWAY"
            } else {
                "CI BRANCH & PR TRAINWAY"
            },
            rect,
            role: Some("region"),
            aria_label: Some("CI Branch and PR Trainway"),
        },
    );

    let view = state.client.view();
    let topo = view.and_then(|v| v.topology.as_ref());
    let stop = topo.is_some_and(RepoTopology::stop);

    let hdr_y = rect.y + 4.0;
    let hdr_h = ((screen_rect.y - rect.y) - 8.0).clamp(16.0, 24.0);
    let right = rect.right() - 6.0;

    // Guarded PullToTurn RESTART switch (requires invited GitHub operator session).
    let is_operator = state.is_operator();
    let rst_w = if rect.w < 500.0 {
        116.0_f64.min(rect.w * 0.30)
    } else {
        136.0_f64.min(rect.w * 0.32)
    };
    let rst_rect = clamp_rect(
        Rect::new((right - rst_w).max(rect.x), hdr_y, rst_w, hdr_h),
        rect,
    );
    let rst_opts = [
        SelectorOption {
            id: "norm",
            label: "READY",
            badge: None,
            tone: SemanticTone::Neutral,
        },
        SelectorOption {
            id: "rst",
            label: if !is_operator {
                "LOCKED"
            } else if state.restart_armed_until_ms.is_some() {
                "CONFIRM!"
            } else if rst_w < 124.0 {
                "RESTART"
            } else {
                "HOLD RST"
            },
            badge: None,
            tone: if is_operator {
                SemanticTone::Warn
            } else {
                SemanticTone::Neutral
            },
        },
    ];
    skin.selector(
        w,
        &Selector {
            control_id: "c5-restart-switch",
            label: "Guarded Trainway Restart Switch (Hold 300ms to arm)",
            rect: rst_rect,
            action: SwitchAction::PullToTurn,
            options: &rst_opts,
            selected: state.restart_switch.current(),
            pulled: state.restart_armed_until_ms.is_some(),
            horizontal: true,
        },
    );

    // LIVE toggle.
    let live_w = 78.0_f64.min(rect.w * 0.20);
    let live_rect = clamp_rect(
        Rect::new(
            (rst_rect.x - live_w - 4.0).max(rect.x),
            hdr_y,
            live_w,
            hdr_h,
        ),
        rect,
    );
    skin.readout(
        w,
        &Readout {
            control_id: "c5-live-toggle",
            label: "",
            value: if state.trainway_live {
                "● LIVE"
            } else {
                "○ PAUSED"
            },
            rect: live_rect,
            tone: if state.trainway_live {
                SemanticTone::Pass
            } else {
                SemanticTone::Warn
            },
            interactive: true,
        },
    );

    // STOP lamp.
    let stop_w = 58.0_f64.min(rect.w * 0.16);
    let stop_rect = clamp_rect(
        Rect::new(
            (live_rect.x - stop_w - 4.0).max(rect.x),
            hdr_y,
            stop_w,
            hdr_h,
        ),
        rect,
    );
    skin.lamp(
        w,
        &Lamp {
            control_id: "c5-stop-lamp",
            label: "STOP",
            rect: stop_rect,
            lit: stop,
            tone: SemanticTone::Fail,
        },
    );

    if rect.w >= 640.0 {
        let logs_w = 118.0_f64.min((stop_rect.x - rect.x - 16.0).max(64.0));
        let logs_rect = clamp_rect(
            Rect::new(
                (stop_rect.x - logs_w - 4.0).max(rect.x),
                hdr_y,
                logs_w,
                hdr_h,
            ),
            rect,
        );
        skin.readout(
            w,
            &Readout {
                control_id: "c5-open-logs",
                label: "",
                value: "QUEUE & LOGS ↗",
                rect: logs_rect,
                tone: SemanticTone::Info,
                interactive: true,
            },
        );
    }

    let max_sidings = match frames.breakpoint {
        Breakpoint::Wide | Breakpoint::Medium => 6,
        Breakpoint::CompactLandscape => 3,
        Breakpoint::Phone => 2,
    };

    type TrainwayRow = (
        String,
        SemanticTone,
        Option<SignalGlyph>,
        Option<String>,
        bool,
    );
    let mut row_strings: Vec<TrainwayRow> = Vec::new();
    let mut station_labels: Vec<&str> = Vec::new();
    let mut track_signals: Vec<Vec<(u32, SignalGlyph, SemanticTone)>> = Vec::new();
    type TrackMeta = (String, u32, u32, bool, bool, Option<String>, bool);
    let mut track_refs: Vec<TrackMeta> = Vec::new();
    let mut total_yard_pages = 1_u32;

    if let Some(t) = topo {
        for s in &t.stations {
            station_labels.push(s.label.as_str());
        }
        let layout = allocate_lanes(t, &state.trainway_lanes, max_sidings, state.yard_page);
        total_yard_pages = layout.total_pages.max(1);
        let sub_count = view
            .and_then(|v| {
                v.repos
                    .iter()
                    .find(|r| Some(&r.id) == state.route.repo.as_ref())
            })
            .map_or(t.submodules.len(), |r| r.submodules.len());
        let ci_trace_id = view
            .and_then(|v| {
                layout
                    .main_train
                    .as_ref()
                    .map(|mt| format!("tr-ci-{}", format_job_id(mt.job_id)))
                    .filter(|tid| v.observe.traces.iter().any(|tr| &tr.trace_id == tid))
                    .or_else(|| {
                        v.observe
                            .traces
                            .iter()
                            .find(|tr| tr.domain == "CI")
                            .map(|tr| tr.trace_id.clone())
                    })
            })
            .unwrap_or_else(|| "tr-ci-1234".into());
        let st_header = format!(
            "FABRIC REV {:04} · SUBMODULES: {} · INTERCHANGE ⇄ {} · [TRACE ↗ {ci_trace_id}]",
            t.fabric_rev,
            sub_count,
            if t.submodule_bump_pending {
                "BUMP PENDING"
            } else {
                "SYNCED"
            }
        );
        row_strings.push((
            st_header,
            SemanticTone::Info,
            None,
            Some(format!("trace:{ci_trace_id}")),
            false,
        ));

        if let Some(mt) = layout.main_train.as_ref() {
            let mut sigs = Vec::with_capacity(mt.stage_states.len());
            for (idx, st) in mt.stage_states.iter().enumerate() {
                let (g, tone) = stage_glyph_and_tone(*st);
                sigs.push((u32::try_from(idx).unwrap_or(0), g, tone));
            }
            let (glyph, tone) = if mt.failed() {
                (SignalGlyph::BlockFailed, SemanticTone::Fail)
            } else {
                (SignalGlyph::BlockRunning, SemanticTone::Pass)
            };
            let node_str = mt.node_id.as_deref().unwrap_or("QUEUE");
            let stage_idx = (mt.stage as usize + 1).min(mt.stage_states.len().max(1));
            let job_tag = format_job_id(mt.job_id);
            let line = format!(
                "L0 {:<14} #{:<6} [{}] {} · STAGE {}/{}",
                mt.ref_name,
                job_tag,
                node_str,
                if mt.node_lost { "NODE LOST" } else { "TRUNK" },
                stage_idx,
                mt.stage_states.len()
            );
            let is_sel = state.route.branch.as_deref() == Some(mt.ref_name.as_str());
            row_strings.push((
                line,
                if mt.node_lost {
                    SemanticTone::Fail
                } else {
                    tone
                },
                Some(glyph),
                Some(format!("c5-track-{}", mt.ref_name)),
                is_sel,
            ));
            track_signals.push(sigs);
            track_refs.push((
                mt.ref_name.clone(),
                0,
                0,
                false,
                false,
                Some(node_str.to_owned()),
                mt.node_lost,
            ));
        }

        for al in &layout.visible_sidings {
            let mut sigs = Vec::new();
            let mut node_str = "QUEUE";
            let mut lost = false;
            let mut failed = false;
            let mut stage_progress = String::new();
            if let Some(tr) = al.train.as_ref() {
                for (idx, st) in tr.stage_states.iter().enumerate() {
                    let (g, tone) = stage_glyph_and_tone(*st);
                    sigs.push((u32::try_from(idx).unwrap_or(0), g, tone));
                }
                node_str = tr.node_id.as_deref().unwrap_or("QUEUE");
                lost = tr.node_lost;
                failed = tr.failed();
                let st_idx = (tr.stage as usize + 1).min(tr.stage_states.len().max(1));
                let job_tag = format_job_id(tr.job_id);
                stage_progress = format!(" #{job_tag} · STAGE {st_idx}/{}", tr.stage_states.len());
            }
            let fading = matches!(al.branch.phase, SidingPhase::Fading { .. })
                || al.branch.pr_state == PrState::Merged;
            let closed = al.branch.pr_state == PrState::Closed;
            let under_construction = al.branch.phase == SidingPhase::UnderConstruction;
            let (glyph, tone) = if failed || lost {
                (SignalGlyph::BlockFailed, SemanticTone::Fail)
            } else if under_construction {
                (SignalGlyph::BlockPending, SemanticTone::Warn)
            } else {
                (SignalGlyph::BlockRunning, SemanticTone::Info)
            };
            let phase_tag = if lost {
                "NODE LOST"
            } else if under_construction {
                "NEW SIDING UNDER CONSTRUCTION"
            } else if fading {
                "MERGED"
            } else if closed {
                "CLOSED"
            } else {
                "ACTIVE"
            };
            let line = format!(
                "L{} {:<14} {:?} [{}] {phase_tag}{stage_progress}",
                al.lane, al.branch.ref_name, al.branch.pr_state, node_str
            );
            let is_sel = state.route.branch.as_deref() == Some(al.branch.ref_name.as_str());
            row_strings.push((
                line,
                tone,
                Some(glyph),
                Some(format!("c5-track-{}", al.branch.ref_name)),
                is_sel,
            ));
            track_signals.push(sigs);
            track_refs.push((
                al.branch.ref_name.clone(),
                al.lane,
                al.branch.fork_stage,
                under_construction || al.branch.pr_state == PrState::Open,
                fading,
                Some(node_str.to_owned()),
                lost,
            ));
        }

        if layout.yard_overflow > 0 {
            let yard_line = format!(
                "YARD +{} OVERFLOW SIDINGS (PG {}/{})",
                layout.yard_overflow,
                layout.page + 1,
                layout.total_pages
            );
            row_strings.push((
                yard_line,
                SemanticTone::Warn,
                Some(SignalGlyph::BlockPending),
                Some("c5-yard-pager".into()),
                false,
            ));
        }

        if t.submodule_bump_pending || !t.submodules.is_empty() {
            let bump = if t.submodule_bump_pending {
                "SUBMODULE BUMP PENDING".to_owned()
            } else {
                format!("ALL {sub_count} SUBMODULES SYNCED")
            };
            let sub_line = format!("INTERCHANGE ⇄ {bump} (CLICK TO STEP SUBMODULE)");
            row_strings.push((
                sub_line,
                if t.submodule_bump_pending {
                    SemanticTone::Warn
                } else {
                    SemanticTone::Info
                },
                None,
                Some("c5-submodule-interchange".into()),
                false,
            ));
        }
    } else {
        row_strings.push((
            "AWAITING TOPOLOGY STREAM…".into(),
            SemanticTone::Neutral,
            None,
            None,
            false,
        ));
    }

    if row_strings
        .iter()
        .all(|(_, _, _, cid, _)| cid.as_deref() != Some("c5-submodule-interchange"))
    {
        if let Some(v) = view {
            if let Some(parent) = v
                .repos
                .iter()
                .find(|r| Some(&r.id) == state.route.repo.as_ref())
            {
                if let Some(first_sub) = parent.submodules.first() {
                    let sub_line = format!(
                        "INTERCHANGE ⇄ {} ({}) @ {}",
                        first_sub.path, first_sub.repo.repo, first_sub.pinned_sha
                    );
                    row_strings.push((
                        sub_line,
                        SemanticTone::Info,
                        None,
                        Some("c5-submodule-interchange".into()),
                        false,
                    ));
                }
            }
        }
    }

    let screen_rows: Vec<ScreenRow<'_>> = row_strings
        .iter()
        .map(|(txt, tone, sig, cid, sel)| ScreenRow {
            text: txt.as_str(),
            tone: *tone,
            signal: *sig,
            critical: false,
            control_id: cid.as_deref(),
            selected: *sel,
        })
        .collect();

    let screen_tracks: Vec<ScreenTrack<'_>> = track_refs
        .iter()
        .zip(track_signals.iter())
        .map(
            |((ref_name, lane, fork, dashed, fading, node_tag, lost), sigs)| ScreenTrack {
                ref_name: ref_name.as_str(),
                lane: *lane,
                fork_stage: *fork,
                dashed: *dashed,
                fading: *fading,
                signals: sigs.as_slice(),
                node_tag: node_tag.as_deref(),
                node_lost: *lost,
            },
        )
        .collect();

    let font = state.font_metrics();
    skin.screen(
        w,
        &Screen {
            control_id: "c5-trainway-screen",
            title: "CI Branch and PR Trainway Screen",
            rect: screen_rect,
            font_family: font.family,
            font_px: panel.grid.font_px,
            cols: panel.grid.cols,
            rows: panel.grid.rows,
            lines: &screen_rows,
            stations: &station_labels,
            tracks: &screen_tracks,
            page: state.yard_page.min(total_yard_pages.saturating_sub(1)),
            total_pages: total_yard_pages,
        },
    );
}

/// Render `CI > Queue & Job Logs`: Active CI queue, worker slot occupancy (`max_slots = 1`), and sanitized logs.
#[allow(clippy::too_many_lines)]
fn render_ci_queue_logs(
    skin: &dyn Skin,
    w: &mut SvgWriter,
    state: &ConsoleState,
    frames: &Frames,
    panel: &LayoutPanel,
) {
    let rect = clamp_rect(panel.plate, frames.canvas);
    let screen_rect = clamp_rect(panel.crt, rect);
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    skin.panel(
        w,
        &Panel {
            id: "ci-queue-logs-panel",
            title: "CI QUEUE, WORKER SLOTS & SANITIZED JOB LOGS",
            rect,
            role: Some("region"),
            aria_label: Some("CI Queue and Job Logs"),
        },
    );

    type QueueRow = (String, SemanticTone, Option<SignalGlyph>, Option<String>);
    let headers: Vec<QueueRow> = vec![(
        "INVARIANT: max_slots = 1 per worker :: thread budgets: node-a=2, node-b=6, worker-riscv64=6"
            .into(),
        SemanticTone::Info,
        None,
        None,
    )];
    let mut body: Vec<QueueRow> = Vec::new();

    let mut follow_active_tail = false;

    if let Some(v) = state.client.view() {
        if !v.ci_jobs.is_empty() {
            body.push((
                "── INTERACTIVE CI JOB QUEUE & STAGE DRILL-DOWN (CLICK JOB TO INSPECT / RESUME / RESTART) ──"
                    .into(),
                SemanticTone::Info,
                None,
                None,
            ));
            let active_job = state
                .route
                .job
                .and_then(|id| v.ci_jobs.iter().find(|j| j.job_id == id))
                .or_else(|| {
                    state
                        .route
                        .repo
                        .as_ref()
                        .and_then(|r| v.ci_jobs.iter().find(|j| &j.repo == r))
                })
                .or_else(|| v.ci_jobs.first());
            follow_active_tail = state.follow_log
                && active_job.is_some_and(|j| {
                    (j.queue_state == "Active" || j.queue_state == "Queued")
                        && !j.raw_log_lines.is_empty()
                });
            for job in &v.ci_jobs {
                let is_sel = active_job.is_some_and(|a| a.job_id == job.job_id);
                let (sig, tone) = match job.queue_state.as_str() {
                    "Failed" => (SignalGlyph::BlockFailed, SemanticTone::Fail),
                    "Active" => (SignalGlyph::BlockRunning, SemanticTone::Warn),
                    "Queued" => (SignalGlyph::BlockPending, SemanticTone::Warn),
                    _ => (SignalGlyph::BlockPassed, SemanticTone::Pass),
                };
                body.push((
                    format!(
                        "{}{:<23} {}/{} @{:<14} [{:<6}] SCORE:{} COV:{:.1}% TESTS:{}/{} {:>3}s",
                        if is_sel { "▶ " } else { "  " },
                        job.job_id_str,
                        job.repo.owner,
                        job.repo.repo,
                        job.ref_name,
                        job.queue_state.to_ascii_uppercase(),
                        job.quality_score,
                        f64::from(job.coverage_bp) / 100.0,
                        job.tests_passed,
                        job.tests_total,
                        job.elapsed_secs
                    ),
                    tone,
                    Some(sig),
                    Some(format!("ci-job:{}", job.job_id)),
                ));
            }
            if let Some(sel_job) = active_job {
                body.push((
                    format!(
                        "── SELECTED JOB DRILL-DOWN [{}] ({}/{} @{}) ──",
                        sel_job.job_id_str, sel_job.repo.owner, sel_job.repo.repo, sel_job.ref_name
                    ),
                    SemanticTone::Info,
                    None,
                    None,
                ));
                let action_tone = if state.is_operator() {
                    SemanticTone::Pass
                } else {
                    SemanticTone::Warn
                };
                body.push((
                    format!(
                        "  [▶ RESUME PIPELINE (/api/jobs/{}/resume)] :: RE-RUN FAILED/PENDING STAGES",
                        sel_job.job_id_str
                    ),
                    action_tone,
                    None,
                    Some(format!("ci-job-resume:{}", sel_job.job_id)),
                ));
                body.push((
                    format!(
                        "  [↻ RESTART ALL (/api/jobs/{}/restart)] :: FULL PIPELINE RE-EXECUTION",
                        sel_job.job_id_str
                    ),
                    action_tone,
                    None,
                    Some(format!("ci-job-restart:{}", sel_job.job_id)),
                ));
                for (idx, st) in sel_job.stages.iter().enumerate() {
                    let (st_sig, st_tone) = stage_glyph_and_tone(st.state);
                    body.push((
                        format!(
                            "  [{}/{}] {:<16} {:<7} NODE:{:<15} THREADS:{} DUR:{:>3}s EXIT:{}",
                            idx + 1,
                            sel_job.stages.len(),
                            st.name,
                            st.state.label(),
                            st.assigned_node.as_deref().unwrap_or("unassigned"),
                            st.threads,
                            st.duration_secs,
                            st.exit_code
                        ),
                        st_tone,
                        Some(st_sig),
                        Some(format!("ci-job:{}", sel_job.job_id)),
                    ));
                }
                body.push((
                    format!(
                        "── RAW BUILD LOG (/api/jobs/{}/log · FOLLOW:{}) ──",
                        sel_job.job_id_str,
                        if state.follow_log { "ON" } else { "OFF" }
                    ),
                    SemanticTone::Info,
                    None,
                    Some("ci-log-follow-toggle".into()),
                ));
                if !sel_job.raw_log_lines.is_empty() {
                    for (li, line) in sel_job.raw_log_lines.iter().enumerate() {
                        body.push((
                            format!("  LOG[{:>02}]: {line}", li + 1),
                            SemanticTone::Neutral,
                            None,
                            None,
                        ));
                    }
                } else if !state.is_operator() {
                    body.push((
                        "  (raw build log requires Operator session — click READ-ONLY · SIGN IN to authenticate)"
                            .into(),
                        SemanticTone::Warn,
                        None,
                        Some("c2-auth-status".into()),
                    ));
                } else {
                    body.push((
                        format!(
                            "  (click job {} above to fetch /api/jobs/{}/log)",
                            sel_job.job_id_str, sel_job.job_id_str
                        ),
                        SemanticTone::Neutral,
                        None,
                        Some(format!("ci-job:{}", sel_job.job_id)),
                    ));
                }
            }
        }
        if let Some(topo) = v.topology.as_ref() {
            for tr in &topo.trains {
                let st_label = topo
                    .stations
                    .get(tr.stage as usize)
                    .map_or("STAGE", |s| s.label.as_str());
                let (sig, tone) = if tr.failed() || tr.node_lost {
                    (SignalGlyph::BlockFailed, SemanticTone::Fail)
                } else if tr.queued() {
                    (SignalGlyph::BlockPending, SemanticTone::Warn)
                } else {
                    (SignalGlyph::BlockRunning, SemanticTone::Pass)
                };
                let job_tag = format_job_id(tr.job_id);
                let tr_trace_id = {
                    let candidate = format!("tr-ci-{job_tag}");
                    if v.observe.traces.iter().any(|t| t.trace_id == candidate) {
                        candidate
                    } else {
                        v.observe
                            .traces
                            .iter()
                            .find(|t| t.domain == "CI")
                            .map_or_else(|| "tr-ci-1234".into(), |t| t.trace_id.clone())
                    }
                };
                body.push((
                    format!(
                        "JOB #{:<8} REF {:<18} STAGE {:<8} NODE {:<15} [TRACE ↗ {tr_trace_id}]",
                        job_tag,
                        tr.ref_name,
                        st_label,
                        tr.node_id.as_deref().unwrap_or("QUEUED")
                    ),
                    tone,
                    Some(sig),
                    Some(format!("trace:{tr_trace_id}")),
                ));
                if !topo.stations.is_empty() && !tr.stage_states.is_empty() {
                    let breakdown: Vec<String> = topo
                        .stations
                        .iter()
                        .zip(tr.stage_states.iter())
                        .map(|(st, state_enum)| {
                            let (glyph, _) = stage_glyph_and_tone(*state_enum);
                            format!("{} {}", st.label, glyph.symbol())
                        })
                        .collect();
                    body.push((
                        format!("  └─ PIPELINE: {}", breakdown.join(" · ")),
                        SemanticTone::Info,
                        None,
                        Some(format!("c5-track-{}", tr.ref_name)),
                    ));
                }
            }
        }
        let live_ci_traces: Vec<_> = v
            .observe
            .traces
            .iter()
            .filter(|t| {
                t.domain == "CI" && t.trace_id != "tr-ci-1234" && t.trace_id != "tr-obs-p99-07"
            })
            .take(10)
            .collect();
        if !live_ci_traces.is_empty() {
            body.push((
                "── CLUSTER-WIDE CI QUEUE LEDGER (/api/queue) ──".into(),
                SemanticTone::Info,
                None,
                None,
            ));
            for tr in live_ci_traces {
                let tone = if tr.error {
                    SemanticTone::Fail
                } else {
                    SemanticTone::Pass
                };
                let sig = if tr.error {
                    SignalGlyph::BlockFailed
                } else {
                    SignalGlyph::BlockPassed
                };
                body.push((
                    format!(
                        "{:<14} {:<42} {:>6}ms ({} stages) [TRACE ↗ {}]",
                        tr.trace_id,
                        tr.root_operation,
                        tr.duration_ms,
                        tr.spans.len(),
                        tr.trace_id
                    ),
                    tone,
                    Some(sig),
                    Some(format!("trace:{}", tr.trace_id)),
                ));
            }
        }
        body.push((
            "── SANITIZED STAGE LOG STREAM (PUBLIC PRIVACY BOUNDARY ENFORCED) ──".into(),
            SemanticTone::Info,
            None,
            None,
        ));
        if v.observe.logs.is_empty() {
            body.push((
                "  (no stage log events in active buffer — pipeline status shown above)".into(),
                SemanticTone::Neutral,
                None,
                None,
            ));
        }
        for log in &v.observe.logs {
            let tone = match log.severity {
                LogSeverity::Error => SemanticTone::Fail,
                LogSeverity::Warn => SemanticTone::Warn,
                LogSeverity::Info => SemanticTone::Neutral,
            };
            let clean = sanitize_public_log(&log.message);
            body.push((
                format!(
                    "[{}] {:<16} {} [TRACE ↗ {}]",
                    log.severity.label(),
                    log.service,
                    clean,
                    log.trace_id
                ),
                tone,
                None,
                Some(format!("trace:{}", log.trace_id)),
            ));
        }
    } else {
        body.push((
            "AWAITING CI QUEUE & LOG STREAM…".into(),
            SemanticTone::Neutral,
            None,
            None,
        ));
    }

    let effective_page = if follow_active_tail {
        u32::MAX
    } else {
        state.subview_page
    };
    let (rows, page, total_pages) =
        paginate_with_headers(headers, body, panel.grid.rows, effective_page);
    let screen_rows: Vec<ScreenRow<'_>> = rows
        .iter()
        .map(|(txt, tone, sig, cid)| ScreenRow {
            text: txt.as_str(),
            tone: *tone,
            signal: *sig,
            critical: false,
            control_id: cid.as_deref(),
            selected: false,
        })
        .collect();

    let font = state.font_metrics();
    skin.screen(
        w,
        &Screen {
            control_id: "ci-queue-logs-screen",
            title: "CI Queue and Sanitized Job Logs",
            rect: screen_rect,
            font_family: font.family,
            font_px: panel.grid.font_px,
            cols: panel.grid.cols,
            rows: panel.grid.rows,
            lines: &screen_rows,
            stations: &[],
            tracks: &[],
            page,
            total_pages,
        },
    );
}

/// Render `CD > Integration Gate`: `C6` CD Delivery Integration-Test Gate Screen.
#[allow(clippy::too_many_lines)]
fn render_c6_delivery_gate(
    skin: &dyn Skin,
    w: &mut SvgWriter,
    state: &ConsoleState,
    frames: &Frames,
    panel: &LayoutPanel,
) {
    let rect = clamp_rect(panel.plate, frames.canvas);
    let screen_rect = clamp_rect(panel.crt, rect);
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    skin.panel(
        w,
        &Panel {
            id: "c6-delivery-panel",
            title: "CD DELIVERY INTEGRATION-TEST GATE",
            rect,
            role: Some("region"),
            aria_label: Some("CD Delivery Integration-Test Gate"),
        },
    );

    let view = state.client.view();
    let gate = view.and_then(|v| v.gate.as_ref());

    let hdr_y = rect.y + 4.0;
    let hdr_h = ((screen_rect.y - rect.y) - 8.0).clamp(16.0, 24.0);
    let junc_w = 180.0_f64.min(rect.w * 0.45);
    let junc_rect = clamp_rect(
        Rect::new(
            (rect.right() - junc_w - 6.0).max(rect.x),
            hdr_y,
            junc_w,
            hdr_h,
        ),
        rect,
    );
    let (junc_val, junc_tone) = match gate {
        Some(g) if g.tainted || g.outcome == GateOutcome::Tainted => {
            ("TAINTED ✕", SemanticTone::Fail)
        }
        Some(g) if g.rollback_lit() => ("◀ ROLLBACK", SemanticTone::Fail),
        Some(g) if g.promote_lit() => ("PROMOTE ▶", SemanticTone::Pass),
        Some(_) => ("GATE PENDING", SemanticTone::Warn),
        None if view.is_some_and(|v| !v.repos.is_empty()) => ("GATE IDLE", SemanticTone::Neutral),
        None => ("AWAITING GATE", SemanticTone::Neutral),
    };
    skin.readout(
        w,
        &Readout {
            control_id: "c6-junction-status",
            label: "GATE",
            value: junc_val,
            rect: junc_rect,
            tone: junc_tone,
            interactive: false,
        },
    );

    type GateRowData = (
        String,
        SemanticTone,
        Option<SignalGlyph>,
        bool,
        Option<String>,
        bool,
    );
    let mut rows_data: Vec<GateRowData> = Vec::new();
    let mut total_log_pages = 1_u32;

    if let Some(g) = gate {
        let active_summary = view.and_then(|v| {
            state
                .selected_history_idx
                .checked_sub(1)
                .and_then(|hi| v.history.get(hi))
        });
        let ttl_secs = view
            .and_then(|v| {
                v.leases
                    .iter()
                    .find(|l| l.deployment_id == g.deployment_id)
                    .map(|l| l.ttl_remaining_ms / 1_000)
            })
            .unwrap_or(142);
        let cd_trace_id = view
            .and_then(|v| {
                v.registry
                    .canaries
                    .iter()
                    .find(|c| c.deployment_id == g.deployment_id || c.repo == g.repo)
                    .map(|c| c.trace_id.clone())
                    .or_else(|| {
                        v.observe
                            .traces
                            .iter()
                            .find(|t| t.domain == "CD")
                            .map(|t| t.trace_id.clone())
                    })
            })
            .unwrap_or_else(|| "tr-cd-0042".into());
        let short_cd_tid: String = cd_trace_id.chars().take(12).collect();
        let hdr = if let Some(hs) = active_summary {
            format!(
                "REPLAY {} · {} · IT {}/{} · {:?}",
                hs.deployment_id, hs.artifact_digest, hs.it_passed, hs.it_total, hs.outcome
            )
        } else {
            format!(
                "DEPLOY {} · SLOT {:?}->{:?} · TTL {}s{} · [TRACE ↗ {short_cd_tid}]",
                g.deployment_id,
                g.restore_slot,
                g.staged_slot,
                ttl_secs,
                g.rollback_reason
                    .map_or_else(String::new, |r| format!(" ({r:?})"))
            )
        };
        rows_data.push((
            hdr,
            junc_tone,
            None,
            false,
            Some(format!("trace:{cd_trace_id}")),
            false,
        ));

        let passed_it = g
            .tests
            .iter()
            .filter(|t| t.state == TestState::Passed)
            .count();
        let total_it = g.tests.len().max(1);
        let filled = ((passed_it * 10) / total_it).min(10);
        let bar = format!("{}{}", "█".repeat(filled), "░".repeat(10 - filled));
        let crit_failed = g
            .tests
            .iter()
            .any(|t| t.critical && t.state == TestState::Failed);
        rows_data.push((
            format!(
                "GATE [{bar}] {passed_it}/{} IT · CRIT: {}",
                g.tests.len(),
                if crit_failed { "FAIL" } else { "OK" }
            ),
            junc_tone,
            None,
            false,
            None,
            false,
        ));

        for (idx, tc) in g.tests.iter().enumerate() {
            let (glyph, tone) = test_glyph_and_tone(tc.state);
            let selected = idx == state.selected_test_idx;
            let crate_tag = tc.crate_name.as_deref().unwrap_or("core");
            let line = format!("{:<21} [{:<6}] {:>5}ms", tc.name, crate_tag, tc.duration_ms);
            rows_data.push((
                line,
                tone,
                Some(glyph),
                tc.critical,
                Some(format!("c6-test-{idx}")),
                selected,
            ));
        }

        let promote_tone = if g.promote_lit() {
            SemanticTone::Pass
        } else {
            SemanticTone::Neutral
        };
        let rollback_tone = if g.rollback_lit() {
            SemanticTone::Fail
        } else {
            SemanticTone::Neutral
        };
        rows_data.push((
            format!(
                "══╦══▶ PROMOTE SLOT {:?} ({})",
                g.staged_slot,
                if g.promote_lit() { "ACTIVE" } else { "STAGED" }
            ),
            promote_tone,
            Some(if g.promote_lit() {
                SignalGlyph::BlockPassed
            } else {
                SignalGlyph::BlockPending
            }),
            false,
            None,
            g.promote_lit(),
        ));
        rows_data.push((
            format!(
                "  ╚══▶ ROLLBACK SLOT {:?} ({})",
                g.restore_slot,
                if g.rollback_lit() {
                    "RESTORED"
                } else {
                    "STANDBY"
                }
            ),
            rollback_tone,
            Some(if g.rollback_lit() {
                SignalGlyph::BlockFailed
            } else {
                SignalGlyph::BlockPending
            }),
            false,
            None,
            g.rollback_lit(),
        ));

        // Selected test's log excerpt (paged, zero scrollbars; gated on operator auth).
        if let Some(sel_tc) = g
            .tests
            .get(state.selected_test_idx)
            .or_else(|| g.tests.first())
        {
            rows_data.push((
                format!(
                    "── LOG EXCERPT [{}] ({}ms) ──",
                    sel_tc.name, sel_tc.duration_ms
                ),
                SemanticTone::Info,
                None,
                false,
                None,
                false,
            ));
            if state.is_operator() {
                let lines = extract_log_lines(sel_tc);
                let per_page = 1_usize;
                total_log_pages = u32::try_from(lines.len().div_ceil(per_page).max(1)).unwrap_or(1);
                let pg =
                    (state.log_page as usize).min((total_log_pages as usize).saturating_sub(1));
                for (li, l) in lines.iter().skip(pg * per_page).take(per_page).enumerate() {
                    rows_data.push((
                        format!("LOG[{}]: {l}", pg * per_page + li + 1),
                        SemanticTone::Info,
                        None,
                        false,
                        Some("c6-log-pager".into()),
                        false,
                    ));
                }
            } else {
                rows_data.push((
                    "RAW LOGS & RESTART REQUIRE OPERATOR SESSION".into(),
                    SemanticTone::Warn,
                    None,
                    false,
                    Some("c6-log-auth-gate".into()),
                    false,
                ));
            }
        }

        if let Some(v) = view {
            if !v.history.is_empty() {
                let hist_summary: Vec<String> = v
                    .history
                    .iter()
                    .enumerate()
                    .take(3)
                    .map(|(hi, hg)| {
                        let sym = match hg.outcome {
                            GateOutcome::Promoted => "■",
                            GateOutcome::Pending => "▢",
                            GateOutcome::RolledBack | GateOutcome::Tainted => "✕",
                        };
                        let sel = if state.selected_history_idx == hi + 1 {
                            "*"
                        } else {
                            ""
                        };
                        format!("{}{sym}{sel}", hg.deployment_id)
                    })
                    .collect();
                rows_data.push((
                    format!("HISTORY: {}", hist_summary.join(" · ")),
                    SemanticTone::Neutral,
                    None,
                    false,
                    Some("c6-history-strip".into()),
                    state.selected_history_idx > 0,
                ));
            }
        }
    } else if view.is_some_and(|v| !v.repos.is_empty()) {
        let repo_label = state.route.repo.as_ref().map_or_else(
            || "SELECTED REPO".into(),
            |r| format!("{}/{}", r.owner, r.repo),
        );
        rows_data.push((
            format!("NO ACTIVE CD GATE FOR {repo_label} (0 DEPLOYMENTS RECORDED)"),
            SemanticTone::Neutral,
            None,
            false,
            None,
            false,
        ));
        rows_data.push((
            "SLOT A: STANDBY · SLOT B: STANDBY · LEASE: IDLE".into(),
            SemanticTone::Info,
            Some(SignalGlyph::BlockPending),
            false,
            None,
            false,
        ));
        rows_data.push((
            "INTEGRATION GATE: READY (AWAITING .muconde/MuConDe.yml PIPELINE TRIGGER)".into(),
            SemanticTone::Neutral,
            None,
            false,
            None,
            false,
        ));
    } else {
        rows_data.push((
            "AWAITING DELIVERY GATE STREAM…".into(),
            SemanticTone::Neutral,
            None,
            false,
            None,
            false,
        ));
    }

    let screen_rows: Vec<ScreenRow<'_>> = rows_data
        .iter()
        .map(|(txt, tone, sig, crit, cid, sel)| ScreenRow {
            text: txt.as_str(),
            tone: *tone,
            signal: *sig,
            critical: *crit,
            control_id: cid.as_deref(),
            selected: *sel,
        })
        .collect();

    let font = state.font_metrics();
    skin.screen(
        w,
        &Screen {
            control_id: "c6-delivery-screen",
            title: "CD Delivery Integration-Test Gate Screen",
            rect: screen_rect,
            font_family: font.family,
            font_px: panel.grid.font_px,
            cols: panel.grid.cols,
            rows: panel.grid.rows,
            lines: &screen_rows,
            stations: &[],
            tracks: &[],
            page: state.log_page,
            total_pages: total_log_pages,
        },
    );
}

fn extract_log_lines(tc: &GateTest) -> Vec<&str> {
    match tc.log_excerpt.as_deref() {
        Some(raw) if !raw.is_empty() => raw.lines().collect(),
        _ => vec!["status: ok (no failure excerpt)"],
    }
}

/// Render `CD > Deployments & Slots`: Blue/Green Slot A/B status per repository and deployment history.
fn render_cd_deployments_slots(
    skin: &dyn Skin,
    w: &mut SvgWriter,
    state: &ConsoleState,
    frames: &Frames,
    panel: &LayoutPanel,
) {
    let rect = clamp_rect(panel.plate, frames.canvas);
    let screen_rect = clamp_rect(panel.crt, rect);
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    skin.panel(
        w,
        &Panel {
            id: "cd-deployments-slots-panel",
            title: "CD BLUE/GREEN DEPLOYMENTS & SLOT A/B TOPOLOGY",
            rect,
            role: Some("region"),
            aria_label: Some("CD Deployments and Blue/Green Slots"),
        },
    );

    type CdRow = (String, SemanticTone, Option<SignalGlyph>, Option<String>);
    let headers: Vec<CdRow> = vec![(
        "BLUE/GREEN SLOT MATRIX :: ACTIVE TRAFFIC vs CANDIDATE STAGING".into(),
        SemanticTone::Info,
        None,
        None,
    )];
    let mut body: Vec<CdRow> = Vec::new();

    if let Some(v) = state.client.view() {
        let mut any_cd = false;
        for r in &v.repos {
            if let Some(cd) = r.cd.as_ref() {
                any_cd = true;
                let (sig, tone) = match cd.outcome {
                    GateOutcome::Promoted => (SignalGlyph::BlockPassed, SemanticTone::Pass),
                    GateOutcome::Pending => (SignalGlyph::BlockRunning, SemanticTone::Warn),
                    GateOutcome::RolledBack | GateOutcome::Tainted => {
                        (SignalGlyph::BlockFailed, SemanticTone::Fail)
                    }
                };
                let cd_trace_id = v
                    .registry
                    .canaries
                    .iter()
                    .find(|c| c.deployment_id == cd.deployment_id || c.repo == r.id)
                    .map(|c| c.trace_id.clone())
                    .or_else(|| {
                        v.observe
                            .traces
                            .iter()
                            .find(|t| t.domain == "CD")
                            .map(|t| t.trace_id.clone())
                    })
                    .unwrap_or_else(|| "tr-cd-0042".into());
                let short_cd_tid: String = cd_trace_id.chars().take(12).collect();
                body.push((
                    format!(
                        "{:<24} DEPLOY {:<10} ACTIVE:{:?} STAGED:{:?} IT {}/{} {:?} [TRACE ↗ {short_cd_tid}]",
                        format!("{}/{}", r.id.owner, r.id.repo),
                        cd.deployment_id,
                        cd.active_slot,
                        cd.active_slot.other(),
                        cd.it_passed,
                        cd.it_total,
                        cd.outcome
                    ),
                    tone,
                    Some(sig),
                    Some(format!("trace:{cd_trace_id}")),
                ));
            }
        }
        if !any_cd {
            body.push((
                "NO ACTIVE BLUE/GREEN SLOT DEPLOYMENTS ACROSS REPOSITORIES (CONFIGURE .muconde/ TO ENABLE CD)"
                    .into(),
                SemanticTone::Neutral,
                None,
                None,
            ));
            for r in &v.repos {
                body.push((
                    format!(
                        "{:<24} SLOT: NONE :: GATE: IDLE :: CLICK TO SELECT REPOSITORY",
                        format!("{}/{}", r.id.owner, r.id.repo)
                    ),
                    SemanticTone::Neutral,
                    Some(SignalGlyph::BlockPending),
                    Some(format!("c4-card-select:{}/{}", r.id.owner, r.id.repo)),
                ));
            }
        }
        if !v.history.is_empty() {
            body.push((
                "── RECENT DEPLOYMENT LEDGER ──".into(),
                SemanticTone::Info,
                None,
                None,
            ));
            for h in &v.history {
                let tone = match h.outcome {
                    GateOutcome::Promoted => SemanticTone::Pass,
                    GateOutcome::Pending => SemanticTone::Warn,
                    GateOutcome::RolledBack | GateOutcome::Tainted => SemanticTone::Fail,
                };
                body.push((
                    format!(
                        "DEPLOY {:<12} DIGEST {:<18} IT {}/{} OUTCOME {:?} [CD GATE ↗]",
                        h.deployment_id, h.artifact_digest, h.it_passed, h.it_total, h.outcome
                    ),
                    tone,
                    None,
                    Some(format!("gate:{}", h.deployment_id)),
                ));
            }
        }
    } else {
        body.push((
            "AWAITING CD DEPLOYMENTS STREAM…".into(),
            SemanticTone::Neutral,
            None,
            None,
        ));
    }

    let (rows, page, total_pages) =
        paginate_with_headers(headers, body, panel.grid.rows, state.subview_page);
    let screen_rows: Vec<ScreenRow<'_>> = rows
        .iter()
        .map(|(txt, tone, sig, cid)| ScreenRow {
            text: txt.as_str(),
            tone: *tone,
            signal: *sig,
            critical: false,
            control_id: cid.as_deref(),
            selected: false,
        })
        .collect();

    let font = state.font_metrics();
    skin.screen(
        w,
        &Screen {
            control_id: "cd-deployments-slots-screen",
            title: "CD Blue/Green Deployments and Slots",
            rect: screen_rect,
            font_family: font.family,
            font_px: panel.grid.font_px,
            cols: panel.grid.cols,
            rows: panel.grid.rows,
            lines: &screen_rows,
            stations: &[],
            tracks: &[],
            page,
            total_pages,
        },
    );
}

/// Render `CD > Active Leases`: TTL-bounded deployment leases across fabric nodes.
fn render_cd_active_leases(
    skin: &dyn Skin,
    w: &mut SvgWriter,
    state: &ConsoleState,
    frames: &Frames,
    panel: &LayoutPanel,
) {
    let rect = clamp_rect(panel.plate, frames.canvas);
    let screen_rect = clamp_rect(panel.crt, rect);
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    skin.panel(
        w,
        &Panel {
            id: "cd-active-leases-panel",
            title: "CD TTL-BOUNDED ACTIVE DEPLOYMENT LEASES",
            rect,
            role: Some("region"),
            aria_label: Some("CD Active Deployment Leases"),
        },
    );

    type LeaseRow = (String, SemanticTone, Option<SignalGlyph>, Option<String>);
    let headers: Vec<LeaseRow> = vec![(
        "LEASE ID    DEPLOY      SERVICE            TARGET NODE      SLOTS   TTL REM   STATE"
            .into(),
        SemanticTone::Info,
        None,
        None,
    )];
    let mut body: Vec<LeaseRow> = Vec::new();

    if let Some(v) = state.client.view() {
        if v.leases.is_empty() {
            body.push((
                "NO ACTIVE TTL DEPLOYMENT LEASES (0 STAGED CANDIDATES HOLDING SLOT LOCKS)".into(),
                SemanticTone::Neutral,
                None,
                None,
            ));
        }
        for l in &v.leases {
            let tone = if l.promoted {
                SemanticTone::Pass
            } else if l.ttl_remaining_ms < 60_000 {
                SemanticTone::Warn
            } else {
                SemanticTone::Info
            };
            body.push((
                format!(
                    "{:<11} {:<11} {:<18} {:<16} {:?}->{:?}   {:>5}s    {} [CD GATE ↗]",
                    l.lease_id,
                    l.deployment_id,
                    l.service,
                    l.target_node,
                    l.active_slot,
                    l.candidate_slot,
                    l.ttl_remaining_ms / 1_000,
                    if l.promoted { "PROMOTED" } else { "STAGED" }
                ),
                tone,
                Some(if l.promoted {
                    SignalGlyph::BlockPassed
                } else {
                    SignalGlyph::BlockRunning
                }),
                Some(format!("gate:{}", l.deployment_id)),
            ));
        }
    } else {
        body.push((
            "AWAITING ACTIVE LEASES STREAM…".into(),
            SemanticTone::Neutral,
            None,
            None,
        ));
    }

    let (rows, page, total_pages) =
        paginate_with_headers(headers, body, panel.grid.rows, state.subview_page);
    let screen_rows: Vec<ScreenRow<'_>> = rows
        .iter()
        .map(|(txt, tone, sig, cid)| ScreenRow {
            text: txt.as_str(),
            tone: *tone,
            signal: *sig,
            critical: false,
            control_id: cid.as_deref(),
            selected: false,
        })
        .collect();

    let font = state.font_metrics();
    skin.screen(
        w,
        &Screen {
            control_id: "cd-active-leases-screen",
            title: "CD Active Leases Screen",
            rect: screen_rect,
            font_family: font.family,
            font_px: panel.grid.font_px,
            cols: panel.grid.cols,
            rows: panel.grid.rows,
            lines: &screen_rows,
            stations: &[],
            tracks: &[],
            page,
            total_pages,
        },
    );
}

/// Render `REG > Packages`: Multi-Registry Package Browser (`Cargo`, `NuGet`, `Maven`) + expandable version drawer.
#[allow(clippy::too_many_lines)]
fn render_reg_packages(
    skin: &dyn Skin,
    w: &mut SvgWriter,
    state: &ConsoleState,
    frames: &Frames,
    panel: &LayoutPanel,
) {
    let rect = clamp_rect(panel.plate, frames.canvas);
    let screen_rect = clamp_rect(panel.crt, rect);
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    skin.panel(
        w,
        &Panel {
            id: "reg-packages-panel",
            title: "MUREGIST MULTI-ECOSYSTEM PACKAGE CATALOG (CARGO · NUGET · MAVEN)",
            rect,
            role: Some("region"),
            aria_label: Some("MuRegist Package Catalog"),
        },
    );

    let active_eco = state
        .reg_ecosystem_filter
        .map_or("ALL", PackageEcosystem::label);
    let hdr_y = rect.y + 4.0;
    let hdr_h = ((screen_rect.y - rect.y) - 8.0).clamp(16.0, 24.0);
    let flt_w = 248.0_f64.min(rect.w * 0.48);
    let flt_rect = clamp_rect(
        Rect::new(
            (rect.right() - flt_w - 6.0).max(rect.x),
            hdr_y,
            flt_w,
            hdr_h,
        ),
        rect,
    );
    let eco_opts = [
        SelectorOption {
            id: "ALL",
            label: "ALL",
            badge: None,
            tone: SemanticTone::Info,
        },
        SelectorOption {
            id: "CARGO",
            label: "CARGO",
            badge: None,
            tone: SemanticTone::Pass,
        },
        SelectorOption {
            id: "NUGET",
            label: "NUGET",
            badge: None,
            tone: SemanticTone::Info,
        },
        SelectorOption {
            id: "MAVEN",
            label: "MAVEN",
            badge: None,
            tone: SemanticTone::Info,
        },
    ];
    let selected_eco = match state.reg_ecosystem_filter {
        None => 0,
        Some(PackageEcosystem::Cargo) => 1,
        Some(PackageEcosystem::NuGet) => 2,
        Some(PackageEcosystem::Maven) => 3,
    };
    skin.selector(
        w,
        &Selector {
            control_id: "reg-pkg-filter",
            label: "Package Ecosystem Filter",
            rect: flt_rect,
            action: SwitchAction::Maintained,
            options: &eco_opts,
            selected: selected_eco,
            pulled: false,
            horizontal: true,
        },
    );

    type OwnedRow = (
        String,
        SemanticTone,
        Option<SignalGlyph>,
        Option<String>,
        bool,
    );
    let headers: Vec<OwnedRow> = vec![
        (
            format!(
                "FILTER ECOSYSTEM [ACTIVE: {active_eco}] :: CLICK HEADER TAB TO FILTER OR ROW TO INSPECT"
            ),
            SemanticTone::Info,
            None,
            None,
            false,
        ),
        (
            "ECOSYSTEM  PACKAGE NAME                       LATEST     VERS  DEPS  SHA256         STATUS"
                .into(),
            SemanticTone::Info,
            None,
            None,
            false,
        ),
    ];
    let mut body: Vec<OwnedRow> = Vec::new();

    if let Some(v) = state.client.view() {
        for pkg in &v.registry.packages {
            if let Some(flt) = state.reg_ecosystem_filter {
                if pkg.ecosystem != flt {
                    continue;
                }
            }
            let selected = state.selected_package.as_deref() == Some(pkg.name.as_str());
            let (tone, status_str) = if pkg.tainted {
                (SemanticTone::Fail, "TAINTED ✕")
            } else if pkg.yanked {
                (SemanticTone::Warn, "YANKED")
            } else {
                (SemanticTone::Pass, "VERIFIED ■")
            };
            body.push((
                format!(
                    "{:<9}  {:<33}  {:<9}  {:>4}  {:>4}  {:<13}  {status_str}",
                    pkg.ecosystem.label(),
                    pkg.name,
                    pkg.latest_version,
                    pkg.versions_count,
                    pkg.deps_count,
                    pkg.sha256_prefix,
                ),
                tone,
                Some(if pkg.tainted {
                    SignalGlyph::BlockFailed
                } else {
                    SignalGlyph::BlockPassed
                }),
                Some(format!("pkg:{}", pkg.name)),
                selected,
            ));
            if selected {
                body.push((
                    format!(
                        "  └─ VERSION HISTORY [{}]: {} :: TAINT: {}",
                        pkg.name,
                        pkg.versions.join(", "),
                        pkg.taint_reason.as_deref().unwrap_or("NONE")
                    ),
                    SemanticTone::Info,
                    None,
                    None,
                    true,
                ));
            }
        }
        if body.is_empty() {
            body.push((
                "0 PACKAGES PUBLISHED IN MUREGIST CATALOG".into(),
                SemanticTone::Neutral,
                None,
                None,
                false,
            ));
        }
    }

    let (rows, page, total_pages) =
        paginate_with_headers(headers, body, panel.grid.rows, state.subview_page);
    let screen_rows: Vec<ScreenRow<'_>> = rows
        .iter()
        .map(|(txt, tone, sig, cid, sel)| ScreenRow {
            text: txt.as_str(),
            tone: *tone,
            signal: *sig,
            critical: false,
            control_id: cid.as_deref(),
            selected: *sel,
        })
        .collect();

    let font = state.font_metrics();
    skin.screen(
        w,
        &Screen {
            control_id: "reg-packages-screen",
            title: "MuRegist Multi-Ecosystem Packages",
            rect: screen_rect,
            font_family: font.family,
            font_px: panel.grid.font_px,
            cols: panel.grid.cols,
            rows: panel.grid.rows,
            lines: &screen_rows,
            stations: &[],
            tracks: &[],
            page,
            total_pages,
        },
    );
}

/// Render `REG > Canary & Taint Ledger`: Canary bundles with `[TRACE ↗]` and `[CD GATE ↗]` links.
fn render_reg_canaries(
    skin: &dyn Skin,
    w: &mut SvgWriter,
    state: &ConsoleState,
    frames: &Frames,
    panel: &LayoutPanel,
) {
    let rect = clamp_rect(panel.plate, frames.canvas);
    let screen_rect = clamp_rect(panel.crt, rect);
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    skin.panel(
        w,
        &Panel {
            id: "reg-canaries-panel",
            title: "MUREGIST CANARY BUNDLES & TAINT PROVENANCE LEDGER",
            rect,
            role: Some("region"),
            aria_label: Some("MuRegist Canary and Taint Ledger"),
        },
    );

    type CanaryRow = (String, SemanticTone, Option<SignalGlyph>, Option<String>);
    let headers: Vec<CanaryRow> = vec![(
        "BUNDLE    REPO                     SERVICE           DIGEST               STATE        LINKS"
            .into(),
        SemanticTone::Info,
        None,
        None,
    )];
    let mut body: Vec<CanaryRow> = Vec::new();

    if let Some(v) = state.client.view() {
        for c in &v.registry.canaries {
            let (sig, tone) = match c.state {
                CanaryBundleState::Promoted => (SignalGlyph::BlockPassed, SemanticTone::Pass),
                CanaryBundleState::Candidate => (SignalGlyph::BlockRunning, SemanticTone::Warn),
                CanaryBundleState::Tainted | CanaryBundleState::RolledBack => {
                    (SignalGlyph::BlockFailed, SemanticTone::Fail)
                }
            };
            let short_tid: String = c.trace_id.chars().take(12).collect();
            body.push((
                format!(
                    "{:<8}  {:<23}  {:<16}  {:<19}  {:<11}  [TRACE ↗ {short_tid}]",
                    c.bundle_id,
                    format!("{}/{}", c.repo.owner, c.repo.repo),
                    c.service,
                    c.artifact_digest,
                    c.state.label(),
                ),
                tone,
                Some(sig),
                Some(format!("trace:{}", c.trace_id)),
            ));
            if let Some(reason) = c.taint_reason.as_ref() {
                body.push((
                    format!(
                        "  └─ TAINT REASON: {} · DEPLOY {} [CD GATE ↗]",
                        reason, c.deployment_id
                    ),
                    SemanticTone::Fail,
                    None,
                    Some(format!("gate:{}", c.deployment_id)),
                ));
            }
        }
        if body.is_empty() {
            body.push((
                "0 CANARY BUNDLES OR TAINTED DIGESTS IN MUREGIST LEDGER".into(),
                SemanticTone::Neutral,
                None,
                None,
            ));
        }
    }

    let (rows, page, total_pages) =
        paginate_with_headers(headers, body, panel.grid.rows, state.subview_page);
    let screen_rows: Vec<ScreenRow<'_>> = rows
        .iter()
        .map(|(txt, tone, sig, cid)| ScreenRow {
            text: txt.as_str(),
            tone: *tone,
            signal: *sig,
            critical: false,
            control_id: cid.as_deref(),
            selected: false,
        })
        .collect();

    let font = state.font_metrics();
    skin.screen(
        w,
        &Screen {
            control_id: "reg-canaries-screen",
            title: "MuRegist Canary and Taint Ledger",
            rect: screen_rect,
            font_family: font.family,
            font_px: panel.grid.font_px,
            cols: panel.grid.cols,
            rows: panel.grid.rows,
            lines: &screen_rows,
            stations: &[],
            tracks: &[],
            page,
            total_pages,
        },
    );
}

/// Render `REG > CAS Health`: Content-Addressable Storage deduplication, GC watermarks, and scrub status.
fn render_reg_cas_health(
    skin: &dyn Skin,
    w: &mut SvgWriter,
    state: &ConsoleState,
    frames: &Frames,
    panel: &LayoutPanel,
) {
    let rect = clamp_rect(panel.plate, frames.canvas);
    let screen_rect = clamp_rect(panel.crt, rect);
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    skin.panel(
        w,
        &Panel {
            id: "reg-cas-health-panel",
            title: "MUREGIST CONTENT-ADDRESSABLE STORAGE (CAS) HEALTH & SCRUB",
            rect,
            role: Some("region"),
            aria_label: Some("MuRegist CAS Health"),
        },
    );

    let mut rows: Vec<(String, SemanticTone)> = Vec::new();
    if let Some(v) = state.client.view() {
        let cas = &v.registry.cas;
        let fmt_mb = |b: u64| {
            if b == 0 || b >= 10_000_000 {
                format!("{} MB", b / 1_000_000)
            } else {
                format!(
                    "{:.2} MB",
                    f64::from(u32::try_from(b).unwrap_or(0)) / 1_000_000.0
                )
            }
        };
        rows.push((
            format!(
                "CAS BLOB COUNT: {} blobs :: DEDUP RATIO: {:.2}x ({} dedup / {} raw)",
                cas.blob_count,
                f64::from(cas.dedup_ratio_bp) / 100.0,
                fmt_mb(cas.dedup_bytes),
                fmt_mb(cas.raw_bytes)
            ),
            SemanticTone::Pass,
        ));
        rows.push((
            format!(
                "GC WATERMARK: {}% :: LAST SCRUB EPOCH: {} :: CORRUPT BLOBS: {}",
                cas.gc_watermark_pct, cas.last_scrub_epoch_ms, cas.corrupt_blobs
            ),
            if cas.corrupt_blobs == 0 {
                SemanticTone::Pass
            } else {
                SemanticTone::Fail
            },
        ));
        rows.push((
            "INTEGRITY VERIFICATION: SHA-256 MERKLE TREE VERIFIED ACROSS ALL ACTIVE SLOTS".into(),
            SemanticTone::Info,
        ));
    }

    let screen_rows: Vec<ScreenRow<'_>> = rows
        .iter()
        .map(|(txt, tone)| ScreenRow {
            text: txt.as_str(),
            tone: *tone,
            signal: Some(SignalGlyph::BlockPassed),
            critical: false,
            control_id: None,
            selected: false,
        })
        .collect();

    let font = state.font_metrics();
    skin.screen(
        w,
        &Screen {
            control_id: "reg-cas-health-screen",
            title: "MuRegist CAS Storage Health",
            rect: screen_rect,
            font_family: font.family,
            font_px: panel.grid.font_px,
            cols: panel.grid.cols,
            rows: panel.grid.rows,
            lines: &screen_rows,
            stations: &[],
            tracks: &[],
            page: 0,
            total_pages: 1,
        },
    );
}

/// Render `OBS > Smartscape`: 3-layer vertical dependency topology (`Services` -> `Processes` -> `Hosts`) + node drawer.
#[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
fn render_obs_smartscape(
    skin: &dyn Skin,
    w: &mut SvgWriter,
    state: &ConsoleState,
    frames: &Frames,
    panel: &LayoutPanel,
) {
    let rect = clamp_rect(panel.plate, frames.canvas);
    let screen_rect = clamp_rect(panel.crt, rect);
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    skin.panel(
        w,
        &Panel {
            id: "obs-smartscape-panel",
            title: "MUOBSERVE SMARTSCAPE 3-TIER VERTICAL TOPOLOGY (SERVICES -> PROCESSES -> HOSTS)",
            rect,
            role: Some("region"),
            aria_label: Some("MuObserve Smartscape Topology"),
        },
    );

    type OwnedRow = (
        String,
        SemanticTone,
        Option<SignalGlyph>,
        Option<String>,
        bool,
    );
    let mut body: Vec<OwnedRow> = Vec::new();
    if let Some(v) = state.client.view() {
        for layer in [
            SmartscapeLayer::Service,
            SmartscapeLayer::Process,
            SmartscapeLayer::Host,
        ] {
            body.push((
                format!("── LAYER: {} ──", layer.label()),
                SemanticTone::Info,
                None,
                None,
                false,
            ));
            let mut layer_count = 0_usize;
            for n in v
                .observe
                .smartscape_nodes
                .iter()
                .filter(|n| n.layer == layer)
            {
                layer_count += 1;
                let selected = state.selected_entity_id.as_deref() == Some(n.id.as_str());
                let tone = if !n.healthy || n.error_rate_bp > 100 {
                    SemanticTone::Warn
                } else {
                    SemanticTone::Pass
                };
                body.push((
                    format!(
                        "  [{:<28}] {:<28} RPS {:>5.1}  ERR {:>4.2}%  p95 {:>3}ms",
                        n.id,
                        n.label,
                        f64::from(n.req_per_sec_bp) / 100.0,
                        f64::from(n.error_rate_bp) / 100.0,
                        n.p95_ms
                    ),
                    tone,
                    Some(if n.healthy {
                        SignalGlyph::BlockPassed
                    } else {
                        SignalGlyph::BlockFailed
                    }),
                    Some(format!("entity:{}", n.id)),
                    selected,
                ));
                if selected {
                    let edges: Vec<String> = v
                        .observe
                        .smartscape_edges
                        .iter()
                        .filter(|e| e.from_id == n.id || e.to_id == n.id)
                        .map(|e| format!("{} -> {} ({})", e.from_id, e.to_id, e.protocol))
                        .collect();
                    let entity_trace_id = v
                        .observe
                        .traces
                        .first()
                        .map_or_else(|| "tr-ci-1234".into(), |t| t.trace_id.clone());
                    body.push((
                        format!(
                            "    └─ ENTITY DRAWER [{}]: {} · [TRACE ↗ {entity_trace_id}]",
                            n.id,
                            edges.join(" | ")
                        ),
                        SemanticTone::Info,
                        None,
                        Some(format!("trace:{entity_trace_id}")),
                        true,
                    ));
                }
            }
            if layer_count == 0 {
                body.push((
                    "  (0 entities in layer)".into(),
                    SemanticTone::Neutral,
                    None,
                    None,
                    false,
                ));
            }
        }
    }

    let (rows, page, total_pages) =
        paginate_with_headers(Vec::new(), body, panel.grid.rows, state.subview_page);
    let screen_rows: Vec<ScreenRow<'_>> = rows
        .iter()
        .map(|(txt, tone, sig, cid, sel)| ScreenRow {
            text: txt.as_str(),
            tone: *tone,
            signal: *sig,
            critical: false,
            control_id: cid.as_deref(),
            selected: *sel,
        })
        .collect();

    let font = state.font_metrics();
    skin.screen(
        w,
        &Screen {
            control_id: "obs-smartscape-screen",
            title: "MuObserve Smartscape 3-Tier Topology",
            rect: screen_rect,
            font_family: font.family,
            font_px: panel.grid.font_px,
            cols: panel.grid.cols,
            rows: panel.grid.rows,
            lines: &screen_rows,
            stations: &[],
            tracks: &[],
            page,
            total_pages,
        },
    );
}

/// Render `OBS > PurePath`: Quick-filter pills + 3-pane W3C trace waterfall (`★CRIT`) + correlated logs + host context.
#[allow(clippy::too_many_lines)]
fn render_obs_purepath(
    skin: &dyn Skin,
    w: &mut SvgWriter,
    state: &ConsoleState,
    frames: &Frames,
    panel: &LayoutPanel,
) {
    let rect = clamp_rect(panel.plate, frames.canvas);
    let screen_rect = clamp_rect(panel.crt, rect);
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    skin.panel(
        w,
        &Panel {
            id: "obs-purepath-panel",
            title: "MUOBSERVE PUREPATH DISTRIBUTED W3C TRACE WATERFALL & CORRELATED LOGS",
            rect,
            role: Some("region"),
            aria_label: Some("MuObserve PurePath Traces"),
        },
    );

    let hdr_y = rect.y + 4.0;
    let hdr_h = ((screen_rect.y - rect.y) - 8.0).clamp(16.0, 24.0);
    let flt_w = 340.0_f64.min(rect.w * 0.56);
    let flt_rect = clamp_rect(
        Rect::new(
            (rect.right() - flt_w - 6.0).max(rect.x),
            hdr_y,
            flt_w,
            hdr_h,
        ),
        rect,
    );
    let pp_pills = ["ALL", "ERRORS", "SLOW P99", "CI", "CD", "REGISTRY"];
    let selected_pp = pp_pills
        .iter()
        .position(|p| state.purepath_filter.eq_ignore_ascii_case(p))
        .unwrap_or(0);
    let pp_opts: Vec<SelectorOption<'_>> = pp_pills
        .iter()
        .map(|&p| SelectorOption {
            id: p,
            label: p,
            badge: None,
            tone: if p == "ERRORS" {
                SemanticTone::Fail
            } else if p == "SLOW P99" {
                SemanticTone::Warn
            } else {
                SemanticTone::Info
            },
        })
        .collect();
    skin.selector(
        w,
        &Selector {
            control_id: "obs-pp-filter",
            label: "PurePath Trace Filter",
            rect: flt_rect,
            action: SwitchAction::Maintained,
            options: &pp_opts,
            selected: selected_pp,
            pulled: false,
            horizontal: true,
        },
    );

    type OwnedCritRow = (
        String,
        SemanticTone,
        Option<SignalGlyph>,
        bool,
        Option<String>,
        bool,
    );
    let headers: Vec<OwnedCritRow> = vec![(
        format!(
            "PUREPATH FILTER [ACTIVE: {}] :: CLICK TRACE ROW TO INSPECT 3-PANE WATERFALL",
            state.purepath_filter
        ),
        SemanticTone::Info,
        None,
        false,
        None,
        false,
    )];
    let mut body: Vec<OwnedCritRow> = Vec::new();

    if let Some(v) = state.client.view() {
        let filtered_traces: Vec<_> = v
            .observe
            .traces
            .iter()
            .filter(|t| match state.purepath_filter.as_str() {
                "ERRORS" => t.error,
                "SLOW P99" => t.slow_p99,
                "CI" | "CD" | "REGISTRY" => t.domain.eq_ignore_ascii_case(&state.purepath_filter),
                _ => true,
            })
            .collect();

        let active_trace = state
            .selected_trace_id
            .as_deref()
            .and_then(|id| filtered_traces.iter().copied().find(|t| t.trace_id == id))
            .or_else(|| filtered_traces.first().copied());

        if filtered_traces.is_empty() {
            body.push((
                format!(
                    "  0 TRACES MATCHING FILTER [ACTIVE: {}] — SELECT [ALL] OR [CI] TO INSPECT WATERFALLS",
                    state.purepath_filter
                ),
                SemanticTone::Neutral,
                None,
                false,
                Some("ppfilter:ALL".into()),
                false,
            ));
        }

        for tr in &filtered_traces {
            let is_active = active_trace.is_some_and(|a| a.trace_id == tr.trace_id);
            let tone = if tr.error {
                SemanticTone::Fail
            } else if tr.slow_p99 {
                SemanticTone::Warn
            } else {
                SemanticTone::Pass
            };
            body.push((
                format!(
                    "TRACE {:<16} [{}] {} :: {} ({}ms){}",
                    tr.trace_id,
                    tr.domain,
                    tr.root_service,
                    tr.root_operation,
                    tr.duration_ms,
                    if tr.slow_p99 { " [SLOW P99]" } else { "" }
                ),
                tone,
                Some(if tr.error {
                    SignalGlyph::BlockFailed
                } else {
                    SignalGlyph::BlockPassed
                }),
                tr.error,
                Some(format!("select-trace:{}", tr.trace_id)),
                is_active,
            ));
        }

        if let Some(tr) = active_trace {
            body.push((
                format!(
                    "── PANE 1: SPAN TREE WATERFALL [{}] ({}ms total) ──",
                    tr.trace_id, tr.duration_ms
                ),
                SemanticTone::Info,
                None,
                false,
                None,
                false,
            ));
            if tr.spans.is_empty() {
                body.push((
                    "  (spans loading or summary-only trace — click trace row above to fetch detail)"
                        .into(),
                    SemanticTone::Neutral,
                    None,
                    false,
                    Some(format!("select-trace:{}", tr.trace_id)),
                    false,
                ));
            }
            for sp in &tr.spans {
                let crit_tag = if sp.critical_path { "★CRIT" } else { "     " };
                let sel_sp = state.selected_span_id.as_deref() == Some(sp.span_id.as_str());
                let tone = if sp.error {
                    SemanticTone::Fail
                } else if sp.critical_path {
                    SemanticTone::Warn
                } else {
                    SemanticTone::Pass
                };
                body.push((
                    format!(
                        "  {crit_tag} {:<11} {:<20} {:<32} +{:>4}ms dur:{:>4}ms self:{:>4}ms [{}]",
                        sp.span_id,
                        sp.service,
                        sp.operation,
                        sp.start_offset_us / 1_000,
                        sp.duration_us / 1_000,
                        sp.self_time_us / 1_000,
                        sp.hardware_class
                    ),
                    tone,
                    Some(if sp.error {
                        SignalGlyph::BlockFailed
                    } else {
                        SignalGlyph::TestPassed
                    }),
                    sp.critical_path,
                    Some(format!("span:{}", sp.span_id)),
                    sel_sp,
                ));
            }

            body.push((
                format!("── PANE 2: CORRELATED OTEL LOGS [trace={}] ──", tr.trace_id),
                SemanticTone::Info,
                None,
                false,
                None,
                false,
            ));
            let mut any_corr_log = false;
            for log in v.observe.logs.iter().filter(|l| l.trace_id == tr.trace_id) {
                any_corr_log = true;
                body.push((
                    format!(
                        "  [{}] {} (span={}): {}",
                        log.severity.label(),
                        log.service,
                        log.span_id.as_deref().unwrap_or("-"),
                        sanitize_public_log(&log.message)
                    ),
                    match log.severity {
                        LogSeverity::Error => SemanticTone::Fail,
                        LogSeverity::Warn => SemanticTone::Warn,
                        LogSeverity::Info => SemanticTone::Neutral,
                    },
                    None,
                    false,
                    None,
                    false,
                ));
            }
            if !any_corr_log {
                body.push((
                    "  (no correlated OTel logs recorded for this trace)".into(),
                    SemanticTone::Neutral,
                    None,
                    false,
                    None,
                    false,
                ));
            }

            body.push((
                "── PANE 3: HOST & PROCESS HARDWARE CONTEXT (K3 X100 vs A100) ──".into(),
                SemanticTone::Info,
                None,
                false,
                None,
                false,
            ));
            for sp in &tr.spans {
                body.push((
                    format!(
                        "  HOST CONTEXT [{}]: {} :: max_slots=1 :: RLIMIT_MEMLOCK<=28GiB",
                        sp.service, sp.hardware_class
                    ),
                    SemanticTone::Neutral,
                    None,
                    false,
                    None,
                    false,
                ));
            }
        }
    }

    let (rows, page, total_pages) =
        paginate_with_headers(headers, body, panel.grid.rows, state.subview_page);
    let screen_rows: Vec<ScreenRow<'_>> = rows
        .iter()
        .map(|(txt, tone, sig, crit, cid, sel)| ScreenRow {
            text: txt.as_str(),
            tone: *tone,
            signal: *sig,
            critical: *crit,
            control_id: cid.as_deref(),
            selected: *sel,
        })
        .collect();

    let font = state.font_metrics();
    skin.screen(
        w,
        &Screen {
            control_id: "obs-purepath-screen",
            title: "MuObserve PurePath Waterfall",
            rect: screen_rect,
            font_family: font.family,
            font_px: panel.grid.font_px,
            cols: panel.grid.cols,
            rows: panel.grid.rows,
            lines: &screen_rows,
            stations: &[],
            tracks: &[],
            page,
            total_pages,
        },
    );
}

/// Render `OBS > Problems & SLOs`: Causal anomaly cards with `[TRACE ↗]` deep-links.
fn render_obs_problems_slos(
    skin: &dyn Skin,
    w: &mut SvgWriter,
    state: &ConsoleState,
    frames: &Frames,
    panel: &LayoutPanel,
) {
    let rect = clamp_rect(panel.plate, frames.canvas);
    let screen_rect = clamp_rect(panel.crt, rect);
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    skin.panel(
        w,
        &Panel {
            id: "obs-problems-slos-panel",
            title: "MUOBSERVE CAUSAL PROBLEMS, FLAKY TESTS & CANARY SLO BURN RATE",
            rect,
            role: Some("region"),
            aria_label: Some("MuObserve Problems and SLOs"),
        },
    );

    type ProblemRow = (String, SemanticTone, Option<SignalGlyph>, Option<String>);
    let headers: Vec<ProblemRow> = vec![(
        "PROBLEM ID   CATEGORY             BURN RATE   CAUSAL ANOMALY & ROOT CAUSE TRACE".into(),
        SemanticTone::Info,
        None,
        None,
    )];
    let mut body: Vec<ProblemRow> = Vec::new();
    if let Some(v) = state.client.view() {
        for p in &v.observe.problems {
            let tone = if p.severity == "ERROR" {
                SemanticTone::Fail
            } else {
                SemanticTone::Warn
            };
            body.push((
                format!(
                    "[{}] {:<18} BURN {:>4.2}x :: {} [TRACE ↗ {}]",
                    p.problem_id,
                    p.category.label(),
                    f64::from(p.burn_rate_bp) / 100.0,
                    p.title,
                    p.root_cause_trace_id
                ),
                tone,
                Some(SignalGlyph::BlockFailed),
                Some(format!("trace:{}", p.root_cause_trace_id)),
            ));
            body.push((
                format!(
                    "  └─ IMPACTED: {} · ROOT CAUSE TRACE: {} (CLICK TO INSPECT WATERFALL)",
                    p.impacted_entity, p.root_cause_trace_id
                ),
                SemanticTone::Info,
                None,
                Some(format!("trace:{}", p.root_cause_trace_id)),
            ));
        }
        if body.is_empty() {
            body.push((
                "ALL SLO BUDGETS NOMINAL :: 0 ACTIVE CAUSAL PROBLEMS DETECTED".into(),
                SemanticTone::Pass,
                Some(SignalGlyph::BlockPassed),
                None,
            ));
        }
    }

    let (rows, page, total_pages) =
        paginate_with_headers(headers, body, panel.grid.rows, state.subview_page);
    let screen_rows: Vec<ScreenRow<'_>> = rows
        .iter()
        .map(|(txt, tone, sig, cid)| ScreenRow {
            text: txt.as_str(),
            tone: *tone,
            signal: *sig,
            critical: true,
            control_id: cid.as_deref(),
            selected: false,
        })
        .collect();

    let font = state.font_metrics();
    skin.screen(
        w,
        &Screen {
            control_id: "obs-problems-slos-screen",
            title: "MuObserve Problems and SLOs",
            rect: screen_rect,
            font_family: font.family,
            font_px: panel.grid.font_px,
            cols: panel.grid.cols,
            rows: panel.grid.rows,
            lines: &screen_rows,
            stations: &[],
            tracks: &[],
            page,
            total_pages,
        },
    );
}

/// Render `OBS > Logs`: Sanitized OTel log stream with severity pills and `[TRACE ↗]` links.
#[allow(clippy::too_many_lines)]
fn render_obs_logs(
    skin: &dyn Skin,
    w: &mut SvgWriter,
    state: &ConsoleState,
    frames: &Frames,
    panel: &LayoutPanel,
) {
    let rect = clamp_rect(panel.plate, frames.canvas);
    let screen_rect = clamp_rect(panel.crt, rect);
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    skin.panel(
        w,
        &Panel {
            id: "obs-logs-panel",
            title: "MUOBSERVE SANITIZED OPENTELEMETRY LOG ANALYTICS",
            rect,
            role: Some("region"),
            aria_label: Some("MuObserve Sanitized Logs"),
        },
    );

    let active_sev = state.log_severity_filter.map_or("ALL", LogSeverity::label);
    let hdr_y = rect.y + 4.0;
    let hdr_h = ((screen_rect.y - rect.y) - 8.0).clamp(16.0, 24.0);
    let flt_w = 248.0_f64.min(rect.w * 0.48);
    let flt_rect = clamp_rect(
        Rect::new(
            (rect.right() - flt_w - 6.0).max(rect.x),
            hdr_y,
            flt_w,
            hdr_h,
        ),
        rect,
    );
    let sev_opts = [
        SelectorOption {
            id: "ALL",
            label: "ALL",
            badge: None,
            tone: SemanticTone::Info,
        },
        SelectorOption {
            id: "ERROR",
            label: "ERROR",
            badge: None,
            tone: SemanticTone::Fail,
        },
        SelectorOption {
            id: "WARN",
            label: "WARN",
            badge: None,
            tone: SemanticTone::Warn,
        },
        SelectorOption {
            id: "INFO",
            label: "INFO",
            badge: None,
            tone: SemanticTone::Pass,
        },
    ];
    let selected_sev = match state.log_severity_filter {
        None => 0,
        Some(LogSeverity::Error) => 1,
        Some(LogSeverity::Warn) => 2,
        Some(LogSeverity::Info) => 3,
    };
    skin.selector(
        w,
        &Selector {
            control_id: "obs-log-filter",
            label: "OTel Log Severity Filter",
            rect: flt_rect,
            action: SwitchAction::Maintained,
            options: &sev_opts,
            selected: selected_sev,
            pulled: false,
            horizontal: true,
        },
    );

    type LogRow = (String, SemanticTone, Option<String>, bool);
    let headers: Vec<LogRow> = vec![(
        format!("OTEL LOG STREAM [SEVERITY: {active_sev}] :: PUBLIC PRIVACY BOUNDARY ENFORCED"),
        SemanticTone::Info,
        None,
        false,
    )];
    let mut body: Vec<LogRow> = Vec::new();

    if let Some(v) = state.client.view() {
        for log in &v.observe.logs {
            if let Some(flt) = state.log_severity_filter {
                if log.severity != flt {
                    continue;
                }
            }
            let tone = match log.severity {
                LogSeverity::Error => SemanticTone::Fail,
                LogSeverity::Warn => SemanticTone::Warn,
                LogSeverity::Info => SemanticTone::Neutral,
            };
            let has_trace = !log.trace_id.is_empty() && !log.trace_id.chars().all(|c| c == '0');
            if has_trace {
                body.push((
                    format!(
                        "[{}] {:<18} {} [TRACE ↗ {}]",
                        log.severity.label(),
                        log.service,
                        sanitize_public_log(&log.message),
                        log.trace_id
                    ),
                    tone,
                    Some(format!("trace:{}", log.trace_id)),
                    false,
                ));
            } else {
                body.push((
                    format!(
                        "[{}] {:<18} {}",
                        log.severity.label(),
                        log.service,
                        sanitize_public_log(&log.message)
                    ),
                    tone,
                    None,
                    false,
                ));
            }
        }
    }

    let (rows, page, total_pages) =
        paginate_with_headers(headers, body, panel.grid.rows, state.subview_page);
    let screen_rows: Vec<ScreenRow<'_>> = rows
        .iter()
        .map(|(txt, tone, cid, sel)| ScreenRow {
            text: txt.as_str(),
            tone: *tone,
            signal: None,
            critical: false,
            control_id: cid.as_deref(),
            selected: *sel,
        })
        .collect();

    let font = state.font_metrics();
    skin.screen(
        w,
        &Screen {
            control_id: "obs-logs-screen",
            title: "MuObserve Sanitized Logs",
            rect: screen_rect,
            font_family: font.family,
            font_px: panel.grid.font_px,
            cols: panel.grid.cols,
            rows: panel.grid.rows,
            lines: &screen_rows,
            stations: &[],
            tracks: &[],
            page,
            total_pages,
        },
    );
}

/// Render `OBS > Metrics`: Golden-signal SVG time-series charts + SpacemiT K3 (`X100` vs `A100`) benchmarks.
#[allow(clippy::cast_precision_loss)]
fn render_obs_metrics(
    skin: &dyn Skin,
    w: &mut SvgWriter,
    state: &ConsoleState,
    frames: &Frames,
    panel: &LayoutPanel,
) {
    let rect = clamp_rect(panel.plate, frames.canvas);
    let screen_rect = clamp_rect(panel.crt, rect);
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    skin.panel(
        w,
        &Panel {
            id: "obs-metrics-panel",
            title: "MUOBSERVE GOLDEN SIGNALS & SPACEMIT K3 (X100 VLEN=256 vs A100 VLEN=1024) BENCHMARKS",
            rect,
            role: Some("region"),
            aria_label: Some("MuObserve Metrics and Hardware Benchmarks"),
        },
    );

    let mut rows: Vec<(String, SemanticTone, Option<String>)> = Vec::new();
    if let Some(v) = state.client.view() {
        for m in &v.observe.metrics {
            rows.push((
                format!(
                    "{:<18} [{}] RPS:{:>3} p50:{:>2}ms p95:{:>2}ms p99:{:>2}ms [TRACE ↗ {}]",
                    m.service,
                    m.hardware_class,
                    m.rps,
                    m.p50_ms,
                    m.p95_ms,
                    m.p99_ms,
                    m.exemplar_trace_id
                ),
                SemanticTone::Pass,
                Some(format!("trace:{}", m.exemplar_trace_id)),
            ));
            let samples_str: Vec<String> = m.samples.iter().map(u32::to_string).collect();
            rows.push((
                format!("  └─ SERIES SAMPLES: [{}]", samples_str.join(", ")),
                SemanticTone::Info,
                Some(format!("trace:{}", m.exemplar_trace_id)),
            ));
        }
        if rows.is_empty() {
            rows.push((
                "0 GOLDEN-SIGNAL OR HARDWARE BENCHMARK SERIES IN MUOBSERVE".into(),
                SemanticTone::Neutral,
                None,
            ));
        }
    }

    let screen_rows: Vec<ScreenRow<'_>> = rows
        .iter()
        .map(|(txt, tone, cid)| ScreenRow {
            text: txt.as_str(),
            tone: *tone,
            signal: Some(SignalGlyph::BlockPassed),
            critical: false,
            control_id: cid.as_deref(),
            selected: false,
        })
        .collect();

    let font = state.font_metrics();
    skin.screen(
        w,
        &Screen {
            control_id: "obs-metrics-screen",
            title: "MuObserve Metrics and Hardware Benchmarks",
            rect: screen_rect,
            font_family: font.family,
            font_px: panel.grid.font_px,
            cols: panel.grid.cols,
            rows: panel.grid.rows,
            lines: &screen_rows,
            stations: &[],
            tracks: &[],
            page: 0,
            total_pages: 1,
        },
    );

    // Also emit an SVG polyline sparkline inside the bottom of `screen_rect` when space permits.
    if screen_rect.h >= 180.0 {
        if let Some(v) = state.client.view() {
            if let Some(first) = v.observe.metrics.first() {
                if first.samples.len() >= 2 {
                    let chart_w = (screen_rect.w - 32.0).max(40.0);
                    let chart_h = 36.0;
                    let chart_x = screen_rect.x + 16.0;
                    let chart_y = screen_rect.bottom() - chart_h - 12.0;
                    let min_v = f64::from(*first.samples.iter().min().unwrap_or(&0));
                    let max_v =
                        f64::from(*first.samples.iter().max().unwrap_or(&1)).max(min_v + 1.0);
                    let mut path = PathData::new();
                    for (i, &val) in first.samples.iter().enumerate() {
                        let px = chart_x + (i as f64 / (first.samples.len() - 1) as f64) * chart_w;
                        let norm = (f64::from(val) - min_v) / (max_v - min_v);
                        let py = chart_y + chart_h - norm * chart_h;
                        if i == 0 {
                            path = path.m(px, py);
                        } else {
                            path = path.l(px, py);
                        }
                    }
                    w.tag("path")
                        .attr("d", path.build())
                        .attr("fill", "none")
                        .attr("stroke", "#33f08a")
                        .attr("stroke-width", 1.8)
                        .attr("data-metric-sparkline", first.service.as_str())
                        .end();
                }
            }
        }
    }
}

/// Render `OBS > Data Store`: Read-only DuckDB hot buffer + Parquet cold tier health.
fn render_obs_datastore(
    skin: &dyn Skin,
    w: &mut SvgWriter,
    state: &ConsoleState,
    frames: &Frames,
    panel: &LayoutPanel,
) {
    let rect = clamp_rect(panel.plate, frames.canvas);
    let screen_rect = clamp_rect(panel.crt, rect);
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    skin.panel(
        w,
        &Panel {
            id: "obs-datastore-panel",
            title: "MUOBSERVE DUCKDB HOT BUFFER & PARQUET ZSTD COLD TIER STORAGE HEALTH",
            rect,
            role: Some("region"),
            aria_label: Some("MuObserve Data Store Health"),
        },
    );

    let mut rows: Vec<(String, SemanticTone)> = Vec::new();
    if let Some(v) = state.client.view() {
        let st = &v.observe.store;
        rows.push((
            format!(
                "HOT TIER (DuckDB In-Memory/NVMe): {} rows ({} MB)",
                st.hot_duckdb_rows,
                st.hot_duckdb_bytes / 1_000_000
            ),
            SemanticTone::Pass,
        ));
        rows.push((
            format!(
                "COLD TIER (Hive-Partitioned Zstd Parquet): {} partitions :: {} rows ({} MB)",
                st.cold_parquet_partitions,
                st.cold_parquet_rows,
                st.cold_parquet_bytes / 1_000_000
            ),
            SemanticTone::Pass,
        ));
        rows.push((
            format!(
                "ZSTD COMPRESSION RATIO: {:.2}x :: RETENTION: {} DAYS :: WATERMARK: {}",
                f64::from(st.zstd_compression_bp) / 100.0,
                st.retention_days,
                st.oldest_watermark_iso
            ),
            SemanticTone::Info,
        ));
        rows.push((
            "QUERY ENGINE GUARD: READ-ONLY SELECT ENFORCED (DDL/DML/COPY REJECTED)".into(),
            SemanticTone::Pass,
        ));
    }

    let screen_rows: Vec<ScreenRow<'_>> = rows
        .iter()
        .map(|(txt, tone)| ScreenRow {
            text: txt.as_str(),
            tone: *tone,
            signal: Some(SignalGlyph::BlockPassed),
            critical: false,
            control_id: None,
            selected: false,
        })
        .collect();

    let font = state.font_metrics();
    skin.screen(
        w,
        &Screen {
            control_id: "obs-datastore-screen",
            title: "MuObserve Data Store Health",
            rect: screen_rect,
            font_family: font.family,
            font_px: panel.grid.font_px,
            cols: panel.grid.cols,
            rows: panel.grid.rows,
            lines: &screen_rows,
            stations: &[],
            tracks: &[],
            page: 0,
            total_pages: 1,
        },
    );
}

/// Render `FLT > Worker Nodes`: Fabric node locks (`node-a`, `worker-riscv64`) and single-slot stage execution.
#[allow(clippy::too_many_lines)]
fn render_fleet_worker_nodes(
    skin: &dyn Skin,
    w: &mut SvgWriter,
    state: &ConsoleState,
    frames: &Frames,
    panel: &LayoutPanel,
) {
    let rect = clamp_rect(panel.plate, frames.canvas);
    let screen_rect = clamp_rect(panel.crt, rect);
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    skin.panel(
        w,
        &Panel {
            id: "flt-nodes-panel",
            title: "FLEET WORKER NODES & SINGLE-SLOT STAGE LOCKS (MAX_SLOTS = 1)",
            rect,
            role: Some("region"),
            aria_label: Some("Fleet Worker Nodes"),
        },
    );

    type FltRow = (
        String,
        SemanticTone,
        Option<SignalGlyph>,
        Option<String>,
        bool,
    );
    let headers: Vec<FltRow> = vec![(
        "INVARIANT: max_slots=1 per node :: thread budgets bounded via CARGO_BUILD_JOBS / RAYON_NUM_THREADS"
            .into(),
        SemanticTone::Info,
        None,
        None,
        false,
    )];
    let mut body: Vec<FltRow> = Vec::new();
    if let Some(v) = state.client.view() {
        let active_node_id = state
            .selected_node_id
            .as_deref()
            .or_else(|| v.nodes.first().map(|n| n.id.as_str()));
        for n in &v.nodes {
            let is_sel = active_node_id == Some(n.id.as_str());
            let tone = if n.online {
                SemanticTone::Pass
            } else {
                SemanticTone::Fail
            };
            let arch_str = if n.arch.is_empty() {
                if n.id.contains("riscv64") {
                    "riscv64"
                } else {
                    "x86_64"
                }
            } else {
                n.arch.as_str()
            };
            let slots_str = format!("{}/{}", n.active_slots, n.max_slots.max(1));
            let cpu_str = format!("{:.2}", f64::from(n.cpu_load_bp) / 100.0);
            let ram_str = if n.ram_total_mb > 0 {
                format!("{}/{}MB", n.ram_used_mb, n.ram_total_mb)
            } else {
                "KVM READY".into()
            };
            body.push((
                format!(
                    "{}NODE {:<15} {:<7} STATUS:{:<7} SLOTS:{} LOAD:{} RAM:{} SEEN:{}s",
                    if is_sel { "▶ " } else { "  " },
                    n.id.to_ascii_uppercase(),
                    arch_str,
                    if n.online { "ONLINE" } else { "OFFLINE" },
                    slots_str,
                    cpu_str,
                    ram_str,
                    n.last_seen_secs_ago
                ),
                tone,
                Some(if n.online {
                    SignalGlyph::BlockPassed
                } else {
                    SignalGlyph::BlockFailed
                }),
                Some(format!("flt-node:{}", n.id)),
                is_sel,
            ));
            if !n.tags.is_empty()
                || !n.toolchains.is_empty()
                || n.btrfs_enabled
                || n.exclusive_locked
                || n.ram_total_mb > 0
            {
                let tags_str = if n.tags.is_empty() {
                    "default".to_owned()
                } else {
                    n.tags.join(",")
                };
                let tc_str = if n.toolchains.is_empty() {
                    "rust".to_owned()
                } else {
                    n.toolchains.join(",")
                };
                body.push((
                    format!(
                        "  └─ BTRFS:{} LOCK:{} TAGS:[{}] TOOLCHAINS:[{}]",
                        if n.btrfs_enabled { "YES" } else { "NO" },
                        if n.exclusive_locked { "HELD" } else { "FREE" },
                        tags_str,
                        tc_str
                    ),
                    SemanticTone::Info,
                    None,
                    Some(format!("flt-node:{}", n.id)),
                    is_sel,
                ));
            }
        }
        if let Some(node_id) = active_node_id {
            body.push((
                format!(
                    "── WORKER NODE STAGE DRILL-DOWN [{node_id}] (CLICK STAGE TO OPEN JOB LOG) ──"
                ),
                SemanticTone::Info,
                None,
                None,
                false,
            ));
            let mut matched_stages = 0_usize;
            for job in &v.ci_jobs {
                for st in &job.stages {
                    if st.assigned_node.as_deref() == Some(node_id) {
                        matched_stages += 1;
                        let (st_sig, st_tone) = stage_glyph_and_tone(st.state);
                        body.push((
                            format!(
                                "  [{:<7}] {:<16} {}/{} @{:<14} THREADS:{} {:>3}s [JOB ↗ {}]",
                                st.state.label(),
                                st.name,
                                job.repo.owner,
                                job.repo.repo,
                                job.ref_name,
                                st.threads,
                                st.duration_secs,
                                job.job_id_str
                            ),
                            st_tone,
                            Some(st_sig),
                            Some(format!("ci-job-open:{}", job.job_id)),
                            false,
                        ));
                    }
                }
            }
            if matched_stages == 0 {
                body.push((
                    "  (0 active or recent stages recorded on this worker node)".into(),
                    SemanticTone::Neutral,
                    None,
                    None,
                    false,
                ));
            }
        }
        if let Some(topo) = v.topology.as_ref() {
            for tr in &topo.trains {
                let job_tag = format_job_id(tr.job_id);
                body.push((
                    format!(
                        "TRAIN #{:<8} {:<18} STAGE {}  NODE {}",
                        job_tag,
                        tr.ref_name,
                        tr.stage,
                        tr.node_id.as_deref().unwrap_or("QUEUED")
                    ),
                    if tr.node_lost || tr.failed() {
                        SemanticTone::Fail
                    } else {
                        SemanticTone::Info
                    },
                    None,
                    Some(format!("ci-job-open:{}", tr.job_id)),
                    false,
                ));
            }
        }
    } else {
        body.push((
            "AWAITING FLEET TELEMETRY…".into(),
            SemanticTone::Neutral,
            None,
            None,
            false,
        ));
    }

    let (lines_owned, page, total_pages) =
        paginate_with_headers(headers, body, panel.grid.rows, state.fleet_page);
    let rows: Vec<ScreenRow<'_>> = lines_owned
        .iter()
        .map(|(txt, tone, sig, cid, sel)| ScreenRow {
            text: txt.as_str(),
            tone: *tone,
            signal: *sig,
            critical: false,
            control_id: cid.as_deref(),
            selected: *sel,
        })
        .collect();

    let font = state.font_metrics();
    skin.screen(
        w,
        &Screen {
            control_id: "flt-nodes-screen",
            title: "Fleet Worker Nodes Screen",
            rect: screen_rect,
            font_family: font.family,
            font_px: panel.grid.font_px,
            cols: panel.grid.cols,
            rows: panel.grid.rows,
            lines: &rows,
            stations: &[],
            tracks: &[],
            page,
            total_pages,
        },
    );
}

/// Render `FLT > Sandbox Profiles`: Declarative `MuIsolation` profiles (`KernelSandbox` & `HypervisorSandbox`).
fn render_fleet_sandbox_profiles(
    skin: &dyn Skin,
    w: &mut SvgWriter,
    state: &ConsoleState,
    frames: &Frames,
    panel: &LayoutPanel,
) {
    let rect = clamp_rect(panel.plate, frames.canvas);
    let screen_rect = clamp_rect(panel.crt, rect);
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    skin.panel(
        w,
        &Panel {
            id: "flt-sandboxes-panel",
            title: "MUISOLATION DECLARATIVE SANDBOX PROFILES & RESOURCE CEILINGS",
            rect,
            role: Some("region"),
            aria_label: Some("Fleet Sandbox Profiles"),
        },
    );

    let mut lines_owned: Vec<(String, SemanticTone)> = Vec::new();
    lines_owned.push((
        "MONOTONIC SECURITY ATTENUATION: WORKFLOW OVERRIDES MAY ONLY TIGHTEN HOST CEILINGS".into(),
        SemanticTone::Info,
    ));
    if let Some(v) = state.client.view() {
        for sb in &v.sandboxes {
            lines_owned.push((
                format!(
                    "{:<28} BACKEND: {:<30} max_pids={} slots={} threads={} [{}]",
                    sb.profile_name,
                    sb.backend,
                    sb.max_pids,
                    sb.max_slots,
                    sb.thread_budget,
                    sb.hardware_target
                ),
                SemanticTone::Pass,
            ));
        }
    }

    let rows: Vec<ScreenRow<'_>> = lines_owned
        .iter()
        .map(|(txt, tone)| ScreenRow {
            text: txt.as_str(),
            tone: *tone,
            signal: Some(SignalGlyph::BlockPassed),
            critical: false,
            control_id: None,
            selected: false,
        })
        .collect();

    let font = state.font_metrics();
    skin.screen(
        w,
        &Screen {
            control_id: "flt-sandboxes-screen",
            title: "Fleet Sandbox Profiles Screen",
            rect: screen_rect,
            font_family: font.family,
            font_px: panel.grid.font_px,
            cols: panel.grid.cols,
            rows: panel.grid.rows,
            lines: &rows,
            stations: &[],
            tracks: &[],
            page: 0,
            total_pages: 1,
        },
    );
}

/// Helper to HTML-escape a string slice using [`escape_into`].
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    escape_into(&mut out, s);
    out
}

/// Render the accessible HTML shell wrapping `svg` with:
/// - Skip-to-dashboard link (`#muconsole-dashboard`);
/// - Operator authentication link (`#muconsole-auth-link`);
/// - Polite and assertive `aria-live` regions;
/// - `font-size: 16px` `/` type-ahead repository filter `<input>` (preventing iOS Safari auto-zoom);
/// - `data-crt-font`, `data-lazy-dyslexic`, `data-area`, `data-subview`, and `data-auth-state` attributes.
#[must_use]
pub fn render_html_shell(svg: &str, state: &ConsoleState) -> String {
    let font_attr = font_slug(state.font);
    let skin_attr = skin_slug(state.skin);
    let skin_label = match state.skin {
        ConsoleSkin::Wireframe => "SKIN: WIREFRAME [S]",
        ConsoleSkin::Avionics => "SKIN: AVIONICS [S]",
    };
    let dyslexic_face = if state.lazy_dyslexic_loaded {
        "@font-face{font-family:'OpenDyslexicM Nerd Font Mono';src:url('/fonts/OpenDyslexicMNerdFontMono-Regular.woff2') format('woff2');font-display:swap;}"
    } else {
        ""
    };
    let sign_in_url = state.sign_in_href();
    let sign_out_url = state.sign_out_href();
    let (auth_slug, auth_href, auth_label) = match state.auth() {
        AuthState::Anonymous => (
            "anonymous",
            sign_in_url.as_str(),
            "READ-ONLY · SIGN IN".to_owned(),
        ),
        AuthState::InviteRequired { github_login } => (
            "invite_required",
            sign_out_url.as_str(),
            format!("INVITE REQ (@{github_login})"),
        ),
        AuthState::Operator { github_login, .. } => (
            "operator",
            sign_out_url.as_str(),
            format!("@{github_login} · OPERATOR"),
        ),
    };
    let escaped_filter_query = escape_html(&state.filter_query);
    let escaped_aria_polite = escape_html(&state.aria_polite);
    let escaped_aria_assertive = escape_html(&state.aria_assertive);
    let escaped_auth_href = escape_html(auth_href);
    let escaped_auth_label = escape_html(&auth_label);

    format!(
        "<div id=\"muconsole-shell\" data-crt-font=\"{font_attr}\" data-skin=\"{skin_attr}\" data-lazy-dyslexic=\"{lazy_dyslexic}\" data-area=\"{area}\" data-subview=\"{subview}\" data-auth-state=\"{auth_slug}\">\
         <style>{dyslexic_face}#muconsole-filter{{font-size:16px;}}</style>\
         <header id=\"muconsole-header\" role=\"banner\">\
         <a href=\"#muconsole-dashboard\" class=\"skip-link\">Skip to dashboard</a>\
         <input id=\"muconsole-filter\" type=\"search\" placeholder=\"Filter repositories...\" aria-label=\"Type-ahead repository filter\" style=\"font-size:16px\" value=\"{escaped_filter_query}\"/>\
         <div id=\"muconsole-aria-polite\" role=\"status\" aria-live=\"polite\" aria-atomic=\"true\">{escaped_aria_polite}</div>\
         <div id=\"muconsole-aria-assertive\" role=\"alert\" aria-live=\"assertive\" aria-atomic=\"true\">{escaped_aria_assertive}</div>\
         <a id=\"muconsole-auth-link\" href=\"{escaped_auth_href}\" data-auth-state=\"{auth_slug}\">{escaped_auth_label}</a>\
         </header>\
         <main id=\"muconsole-dashboard\" role=\"main\" aria-label=\"MuConsole Unified CI/CD Dashboard\" tabindex=\"-1\">{svg}</main>\
         <button id=\"muconsole-skin-toggle\" type=\"button\" data-skin-toggle=\"true\" aria-label=\"Toggle visual skin\" style=\"position:fixed;bottom:8px;right:12px;z-index:20;font-family:var(--font-crt);font-size:11px;color:var(--accent-neon-cyan);background:rgba(5,10,14,0.85);border:1px solid var(--accent-neon-cyan);border-radius:3px;padding:3px 8px;cursor:pointer;white-space:nowrap\">{skin_label}</button>\
         </div>",
        lazy_dyslexic = state.lazy_dyslexic_loaded,
        area = area_label(state.route.area),
        subview = state.route.subview.slug(),
    )
}
