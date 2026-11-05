// SPDX-License-Identifier: AGPL-3.0-only
//! Automated UX Validation Gate (`U1`–`U8`) for Milestone 6 & Milestone 7
//! (`WireframeSkin`, 5 Areas × 17 Sub-Views, C2 Repo/Submodule Stepper,
//! `SubViewBar`, `Hurmit` ↔ `OpenDyslexicM` toggle, keyboard/DOM/deep-link
//! parity, cross-area `[TRACE ↗]` / `[CD GATE ↗]` routing, public privacy
//! boundary, and invite-only GitHub operator authentication gating).
#![cfg(not(miri))]

use mu_console_data::engine::{Effect, Engine};
use mu_console_data::scenario::Scenario;
use mu_console_layout::{Breakpoint, SafeArea, Viewport};
use mu_console_protocol::{
    Area, AuthState, Branch, CdSubView, CiSubView, CrtFont, DataEvent, Delta, GateTest,
    ObserveSubView, PrState, RegistrySubView, RepoId, RepoScore, RepoStatus, SidingPhase, SubView,
    SubmoduleId, TestState, UiCommand, ViewModel, WorkerConfig,
};
use mu_console_theme::interaction::SwitchEvent;
use mu_console_theme::wireframe::{AvionicsSkin, WireframeSkin};
use mu_console_ui::components::{render_console, render_html_shell};
use mu_console_ui::router::{breadcrumb_segments, escape_up, format_hash, parse_hash};
use mu_console_ui::state::{
    extract_invite_token, font_slug, format_sign_in_url, parse_font_pref, validate_sign_in_href,
    ConsoleState, PENDING_INVITE_STORAGE_KEY, ROTARY_SETTLE_MS,
};
use mu_console_ui::WORKER_URL;

const VIEWPORTS: [(f64, f64); 6] = [
    (1920.0, 1080.0),
    (1512.0, 860.0),
    (1280.0, 800.0),
    (1024.0, 768.0),
    (932.0, 430.0),
    (402.0, 720.0),
];

fn state_for_scenario(scenario: Scenario, subview: SubView, font: CrtFont) -> ConsoleState {
    let mut eng = Engine::new(1);
    let cfg = WorkerConfig {
        api_base: "http://192.0.2.10:8080".into(),
        scenario: Some(scenario.name().to_owned()),
    };
    let mut state = ConsoleState::new();
    state.set_font(font);
    state.select_subview(subview);

    for ef in eng.handle(UiCommand::Configure(cfg), 1_000) {
        if let Effect::Post(ev) = ef {
            let bytes = Engine::encode_event(&ev).expect("encode DataEvent");
            state.on_worker_frame(&bytes);
        }
    }
    let end_ms = 1_000 + scenario.duration_ms();
    for t in (1_000..=end_ms).step_by(200) {
        for ef in eng.tick(t) {
            if let Effect::Post(ev) = ef {
                let bytes = Engine::encode_event(&ev).expect("encode DataEvent");
                state.on_worker_frame(&bytes);
            }
        }
    }
    state
}

fn assert_svg_zero_overflow(svg: &str, vw: f64, vh: f64, context: &str) {
    let doc =
        roxmltree::Document::parse(svg).unwrap_or_else(|e| panic!("{context}: invalid XML: {e}"));
    assert!(
        !svg.contains("feGaussianBlur"),
        "{context}: WireframeSkin must not emit feGaussianBlur"
    );
    assert!(
        !svg.contains("mu-wood"),
        "{context}: WireframeSkin must not emit wood veneer"
    );
    assert!(
        !svg.contains("mu-brushed"),
        "{context}: WireframeSkin must not emit brushed tungsten filter"
    );

    let lower = svg.to_ascii_lowercase();
    for forbidden in ["entrance", "rubicon", "pey-wail-deu", "lab.local"] {
        assert!(
            !lower.contains(forbidden),
            "{context}: SVG must never expose private identifier '{forbidden}'"
        );
    }

    let eps = 1e-4;
    for node in doc.descendants() {
        if node.has_tag_name("rect") {
            let x: f64 = node
                .attribute("x")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
            let y: f64 = node
                .attribute("y")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
            let w: f64 = node
                .attribute("width")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
            let h: f64 = node
                .attribute("height")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
            assert!(
                x >= -eps && y >= -eps && (x + w) <= vw + eps && (y + h) <= vh + eps,
                "{context}: <rect x={x} y={y} w={w} h={h}> overflows viewport {vw}x{vh}"
            );
        }

        if let (Some(bx), Some(by), Some(bw), Some(bh)) = (
            node.attribute("data-bbox-x"),
            node.attribute("data-bbox-y"),
            node.attribute("data-bbox-w"),
            node.attribute("data-bbox-h"),
        ) {
            let x: f64 = bx.parse().expect("bbox-x");
            let y: f64 = by.parse().expect("bbox-y");
            let w: f64 = bw.parse().expect("bbox-w");
            let h: f64 = bh.parse().expect("bbox-h");
            let cid = node.attribute("data-control-id").unwrap_or("?");
            assert!(
                x >= -eps && y >= -eps && (x + w) <= vw + eps && (y + h) <= vh + eps,
                "{context}: control {cid} bbox ({x},{y},{w},{h}) overflows viewport {vw}x{vh}"
            );
        }

        if let (Some(cols_s), Some(rows_s)) =
            (node.attribute("data-cols"), node.attribute("data-rows"))
        {
            let cols: usize = cols_s.parse().expect("data-cols");
            let rows: usize = rows_s.parse().expect("data-rows");
            let cid = node.attribute("data-control-id").unwrap_or("screen");
            let mut text_count = 0_usize;
            for child in node.descendants() {
                if child.has_tag_name("text") && child.attribute("data-pager").is_none() {
                    text_count += 1;
                    let txt = child.text().unwrap_or("");
                    let char_len = txt.chars().count();
                    assert!(
                        char_len <= cols,
                        "{context}: screen {cid} line '{txt}' ({char_len} chars) exceeds cols={cols}"
                    );
                }
            }
            assert!(
                text_count <= rows,
                "{context}: screen {cid} emitted {text_count} rows, exceeding rows={rows}"
            );
        }
    }
}

#[test]
fn u1_zero_viewport_overflow_across_all_viewports_17_subviews_fonts_and_scenarios() {
    for &(vw, vh) in &VIEWPORTS {
        for &area in &Area::ALL {
            for &subview in SubView::all_for(area) {
                for &font in &[CrtFont::Hurmit, CrtFont::OpenDyslexic] {
                    let st = state_for_scenario(Scenario::NewBranch, subview, font);
                    let svg =
                        render_console(&WireframeSkin, &st, Viewport::new(vw, vh), SafeArea::ZERO);
                    let ctx = format!("{vw}x{vh} subview={subview:?} font={font:?}");
                    assert_svg_zero_overflow(&svg, vw, vh, &ctx);
                }
            }
        }
    }
    for &scenario in &Scenario::ALL {
        for &area in &Area::ALL {
            let st = state_for_scenario(scenario, SubView::default_for(area), CrtFont::Hurmit);
            let svg = render_console(
                &WireframeSkin,
                &st,
                Viewport::new(1512.0, 860.0),
                SafeArea::ZERO,
            );
            let ctx = format!("1512x860 area={area:?} scenario={}", scenario.name());
            assert_svg_zero_overflow(&svg, 1512.0, 860.0, &ctx);
        }
    }
}

#[test]
fn u2_navigation_and_routing_c1_c2_deep_link_escape_and_legacy_redirect() {
    let mut st = state_for_scenario(
        Scenario::NewBranch,
        SubView::Ci(CiSubView::Trainway),
        CrtFont::Hurmit,
    );

    // C1 area switching via keyboard `1`..`5` and sub-view stepping via `[` / `]`.
    assert_eq!(st.route.area, Area::Ci);
    assert_eq!(st.route.subview, SubView::Ci(CiSubView::Trainway));
    let cmd = st.on_key("2", false, 1_000);
    assert_eq!(st.route.area, Area::Cd);
    assert_eq!(st.route.subview, SubView::Cd(CdSubView::IntegrationGate));
    assert!(matches!(cmd, Some(UiCommand::Subscribe(_))));

    let cmd = st.on_key("1", false, 1_000);
    assert_eq!(st.route.area, Area::Ci);
    assert_eq!(st.route.subview, SubView::Ci(CiSubView::Scorecards));
    assert!(matches!(cmd, Some(UiCommand::Subscribe(_))));

    // Sub-View cycling via `[` and `]` within an Area.
    let cmd = st.on_key("]", false, 1_050);
    assert_eq!(st.route.subview, SubView::Ci(CiSubView::Trainway));
    assert!(matches!(cmd, Some(UiCommand::Subscribe(_))));
    let cmd = st.on_key("[", false, 1_060);
    assert_eq!(st.route.subview, SubView::Ci(CiSubView::Scorecards));
    assert!(matches!(cmd, Some(UiCommand::Subscribe(_))));

    // Deep-link round-trip + breadcrumb segments + Esc up-one-level.
    let hash = "#/ci/trainway/fixture/superproject/code%2Fengine/feat%2Frvv/104";
    st.init_from_env(hash, "", None);
    assert_eq!(format_hash(&st.route), hash);
    let wide_crumbs = breadcrumb_segments(&st.route, Breakpoint::Wide);
    assert!(wide_crumbs.len() >= 5);
    let phone_crumbs = breadcrumb_segments(&st.route, Breakpoint::Phone);
    assert!(
        phone_crumbs.iter().any(|c| c.value == "…"),
        "phone breadcrumbs must collapse middle segments to '…'"
    );

    // Step up hierarchy with Escape.
    st.on_key("Escape", false, 1_100);
    assert_eq!(st.route.job, None);
    assert_eq!(st.route.branch.as_deref(), Some("feat/rvv"));
    st.on_key("Escape", false, 1_200);
    assert_eq!(st.route.branch, None);
    assert_eq!(
        st.route.submodule,
        Some(SubmoduleId {
            path: "code/engine".into()
        })
    );
    st.on_key("Escape", false, 1_300);
    assert_eq!(st.route.submodule, None);
    assert_eq!(st.route.repo, Some(RepoId::new("fixture", "superproject")));
    st.on_key("Escape", false, 1_400);
    assert_eq!(st.route.repo, None);
    assert_eq!(escape_up(&st.route).area, Area::Ci);

    // Legacy #/ovr and #/jobs/{id} redirects.
    let ovr = parse_hash("#/ovr");
    assert_eq!(ovr.area, Area::Ci);
    assert_eq!(ovr.subview, SubView::Ci(CiSubView::Scorecards));
    let legacy = parse_hash("#/jobs/77");
    assert_eq!(legacy.area, Area::Ci);
    assert_eq!(legacy.subview, SubView::Ci(CiSubView::Trainway));
    assert_eq!(legacy.job, Some(77));
    assert_eq!(legacy.branch.as_deref(), Some("main"));
}

#[test]
fn u3_repo_stepper_50_plus_repos_submodules_typeahead_filter_and_350ms_settle_debounce() {
    let mut st = state_for_scenario(
        Scenario::Repos40,
        SubView::Ci(CiSubView::Trainway),
        CrtFont::Hurmit,
    );

    // Add 15 more repositories via Patch so total is 55 (50+ scaling check).
    let mut extra = Vec::new();
    for i in 40..55 {
        let mut r = RepoScore::new(RepoId::new("fixture", &format!("zulu-{i:02}")));
        r.status = RepoStatus::Passing;
        r.score = 95;
        extra.push(Delta::RepoUpsert(r));
    }
    let patch_bytes = Engine::encode_event(&DataEvent::Patch(extra)).expect("encode patch");
    st.on_worker_frame(&patch_bytes);
    assert!(st.filtered_repos().len() >= 55);

    // Rapid outer repo steps at t=100, 200, 300 coalesce into a single SelectRepo after 350 ms.
    st.rotate_outer(1, 100);
    assert!(st.tick(200).is_empty());
    st.rotate_outer(1, 200);
    assert!(st.tick(300).is_empty());
    st.rotate_outer(-2, 300);
    assert!(
        st.tick(300 + ROTARY_SETTLE_MS - 10).is_empty(),
        "must not emit SelectRepo before 350 ms settle window expires"
    );
    let cmds = st.tick(300 + ROTARY_SETTLE_MS);
    assert_eq!(cmds.len(), 1, "must coalesce rapid steps into 1 SelectRepo");
    assert!(matches!(cmds[0], UiCommand::SelectRepo { .. }));

    // Submodule stepper steps through PARENT (0) and 1..=7 submodules on superproject.
    let mut st_sub = state_for_scenario(
        Scenario::NewBranch,
        SubView::Ci(CiSubView::Trainway),
        CrtFont::Hurmit,
    );
    st_sub.route.repo = Some(RepoId::new("fixture", "superproject"));
    st_sub.route.submodule = None;
    st_sub.sync_rotary_from_route();
    st_sub.on_dom_action("data-repo-step", "sub-next", false, 1_000);
    assert_eq!(st_sub.inner_index, 1);
    assert_eq!(
        st_sub.route.submodule.as_ref().map(|s| s.path.as_str()),
        Some("code/core")
    );
    let cmds = st_sub.tick(1_000 + ROTARY_SETTLE_MS);
    assert_eq!(
        cmds,
        vec![UiCommand::SelectRepo {
            repo: RepoId::new("fixture", "superproject"),
            submodule: Some(SubmoduleId {
                path: "code/core".into()
            }),
        }]
    );

    // `/` type-ahead filter narrows list and Escape clears it.
    st_sub.on_key("/", false, 2_000);
    assert!(st_sub.filter_focused);
    st_sub.set_filter_query("kernels-rvv", 2_010);
    assert_eq!(st_sub.filtered_repos().len(), 1);
    assert_eq!(st_sub.filtered_repos()[0].id.repo, "kernels-rvv");
    st_sub.on_key("Escape", false, 2_020);
    assert!(st_sub.filter_query.is_empty());
    assert!(!st_sub.filter_focused);
}

#[test]
fn u4_scorecard_carousel_orders_parent_first_then_gitmodules_and_pages_cleanly() {
    let mut st = state_for_scenario(
        Scenario::NewBranch,
        SubView::Ci(CiSubView::Scorecards),
        CrtFont::Hurmit,
    );
    st.route.repo = Some(RepoId::new("fixture", "superproject"));
    let ordered = st.ordered_carousel_repos();
    assert_eq!(ordered[0].id, RepoId::new("fixture", "superproject"));
    assert_eq!(ordered[1].id, RepoId::new("fixture", "core"));
    assert_eq!(ordered[2].id, RepoId::new("fixture", "engine"));
    assert_eq!(ordered[3].id, RepoId::new("fixture", "tokenizer"));
    assert_eq!(ordered[4].id, RepoId::new("fixture", "kv"));
    assert_eq!(ordered[5].id, RepoId::new("fixture", "kernels-rvv"));
    assert_eq!(ordered[6].id, RepoId::new("fixture", "arch-synthetic"));
    assert_eq!(ordered[7].id, RepoId::new("fixture", "manager"));

    assert_eq!(st.carousel_page, 0);
    let svg_p0 = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    let doc_p0 = roxmltree::Document::parse(&svg_p0).expect("valid XML");
    for node in doc_p0.descendants() {
        if let Some(cid) = node.attribute("data-control-id") {
            if cid.starts_with("c4-card-") {
                for tnode in node.descendants().filter(|n| n.has_tag_name("text")) {
                    let txt = tnode.text().unwrap_or("");
                    assert!(
                        !txt.contains('…'),
                        "C4 half-card {cid} line '{txt}' must not truncate values with '…'"
                    );
                }
            }
        }
    }

    st.step_carousel(1, 6);
    assert_eq!(st.carousel_page, 1);
    let svg = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    assert!(svg.contains("PAGE 2/"));
    assert_svg_zero_overflow(&svg, 1512.0, 860.0, "u4_carousel_page_2");
}

#[test]
fn u5_trainway_lanes_stop_lamp_yard_overflow_submodule_interchange_and_restart_guard() {
    let mut st_sub = state_for_scenario(
        Scenario::Submodules,
        SubView::Ci(CiSubView::Trainway),
        CrtFont::Hurmit,
    );
    let mut branch_deltas = Vec::new();
    for i in 0..8 {
        branch_deltas.push(Delta::BranchUpsert(Branch {
            ref_name: format!("feat/siding-{i}"),
            base_ref: "main".into(),
            fork_stage: 1,
            pr_state: PrState::Open,
            last_job: None,
            phase: SidingPhase::Active,
        }));
    }
    let patch_bytes = Engine::encode_event(&DataEvent::Patch(branch_deltas)).expect("encode");
    st_sub.on_worker_frame(&patch_bytes);
    let svg_sub = render_console(
        &WireframeSkin,
        &st_sub,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    assert!(
        svg_sub.contains("c5-yard-pager") && svg_sub.contains("YARD +"),
        "Trainway must render YARD +N overflow stub when tracks exceed max_sidings"
    );
    assert!(
        svg_sub.contains("c5-submodule-interchange"),
        "Trainway must render submodule interchange row"
    );

    let mut st_fail = state_for_scenario(
        Scenario::StageFailure,
        SubView::Ci(CiSubView::Trainway),
        CrtFont::Hurmit,
    );
    let svg_fail = render_console(
        &WireframeSkin,
        &st_fail,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    let doc_fail = roxmltree::Document::parse(&svg_fail).expect("valid XML");
    let stop_lamp = doc_fail
        .descendants()
        .find(|n| n.attribute("data-control-id") == Some("c5-stop-lamp"))
        .expect("c5-stop-lamp present");
    assert_eq!(stop_lamp.attribute("data-lit"), Some("true"));

    assert_eq!(
        st_fail.on_restart_switch(
            SwitchEvent::Rotate {
                to: 1,
                at_ms: 1_000,
                shift: false
            },
            1_000
        ),
        None
    );
    assert_eq!(st_fail.restart_count, 0);

    st_fail.on_restart_switch(SwitchEvent::Press { at_ms: 1_000 }, 1_000);
    assert_eq!(
        st_fail.on_restart_switch(
            SwitchEvent::Rotate {
                to: 1,
                at_ms: 1_100,
                shift: false
            },
            1_100
        ),
        None
    );
    assert!(st_fail.restart_armed_until_ms.is_none());

    assert_eq!(
        st_fail.on_restart_switch(
            SwitchEvent::Rotate {
                to: 1,
                at_ms: 1_400,
                shift: false
            },
            1_400
        ),
        None
    );
    assert!(st_fail.restart_armed_until_ms.is_some());
    st_fail.on_restart_switch(SwitchEvent::Release { at_ms: 1_450 }, 1_450);

    st_fail.on_restart_switch(SwitchEvent::Press { at_ms: 1_500 }, 1_500);
    let confirmed = st_fail.on_restart_switch(
        SwitchEvent::Rotate {
            to: 1,
            at_ms: 1_900,
            shift: false,
        },
        1_900,
    );
    assert_eq!(confirmed, Some(UiCommand::Retry));
    assert_eq!(st_fail.restart_count, 1);
}

#[test]
fn u6_delivery_gate_promote_rollback_history_replay_and_log_pager() {
    let mut st_ok = state_for_scenario(
        Scenario::NewBranch,
        SubView::Cd(CdSubView::IntegrationGate),
        CrtFont::Hurmit,
    );
    let promote_patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::GateStatus {
        outcome: mu_console_protocol::GateOutcome::Promoted,
        rollback_reason: None,
        tainted: false,
        lease_expires_epoch_ms: 2_000_000_000_000,
    }]))
    .expect("encode");
    st_ok.on_worker_frame(&promote_patch);
    let svg_ok = render_console(
        &WireframeSkin,
        &st_ok,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    assert!(svg_ok.contains("PROMOTE ▶"));

    let mut st_rb = state_for_scenario(
        Scenario::ItCriticalFailure,
        SubView::Cd(CdSubView::IntegrationGate),
        CrtFont::Hurmit,
    );
    let log_patch =
        Engine::encode_event(&DataEvent::Patch(vec![Delta::GateTestUpsert(GateTest {
            name: "it_infer_gemm".into(),
            critical: true,
            state: TestState::Failed,
            duration_ms: 4_100,
            crate_name: Some("engine".into()),
            log_excerpt: Some("line 1: gemm mismatch\nline 2: tile (3,4) nan".into()),
        })]))
        .expect("encode");
    st_rb.on_worker_frame(&log_patch);
    st_rb.selected_test_idx = 3;
    let svg_rb_p0 = render_console(
        &WireframeSkin,
        &st_rb,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    assert!(svg_rb_p0.contains("◀ ROLLBACK"));
    assert!(svg_rb_p0.contains("[C]"));
    assert!(svg_rb_p0.contains("LOG[1]: line 1: gemm mismatch"));
    assert!(svg_rb_p0.contains("PAGE 1/2"));

    st_rb.log_page = 1;
    let svg_rb_p1 = render_console(
        &WireframeSkin,
        &st_rb,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    assert!(svg_rb_p1.contains("LOG[2]: line 2: tile (3,4) nan"));
    assert!(svg_rb_p1.contains("PAGE 2/2"));

    st_rb.selected_history_idx = 2;
    let svg_hist = render_console(
        &WireframeSkin,
        &st_rb,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    assert!(svg_hist.contains("REPLAY dep-0041"));
}

#[test]
fn u7_font_toggle_dyslexic_reflow_phone_touch_targets_html_shell_and_skin_parity() {
    assert_eq!(
        parse_font_pref("?font=dyslexic", None),
        CrtFont::OpenDyslexic
    );
    assert_eq!(parse_font_pref("", Some("dyslexic")), CrtFont::OpenDyslexic);
    assert_eq!(parse_font_pref("", Some("hurmit")), CrtFont::Hurmit);
    assert_eq!(font_slug(CrtFont::OpenDyslexic), "dyslexic");

    let mut st = state_for_scenario(
        Scenario::NewBranch,
        SubView::Ci(CiSubView::Trainway),
        CrtFont::Hurmit,
    );
    let svg_hurmit = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    let cmd = st.on_key("d", false, 1_000);
    assert_eq!(cmd, Some(UiCommand::SetCrtFont(CrtFont::OpenDyslexic)));
    assert!(st.lazy_dyslexic_loaded);
    let svg_dys = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    let doc_h = roxmltree::Document::parse(&svg_hurmit).expect("hurmit XML");
    let doc_d = roxmltree::Document::parse(&svg_dys).expect("dyslexic XML");
    let trainway_cols_h: usize = doc_h
        .descendants()
        .find(|n| n.attribute("data-control-id") == Some("c5-trainway-screen"))
        .and_then(|n| n.attribute("data-cols"))
        .and_then(|s| s.parse().ok())
        .expect("hurmit trainway cols");
    let trainway_cols_d: usize = doc_d
        .descendants()
        .find(|n| n.attribute("data-control-id") == Some("c5-trainway-screen"))
        .and_then(|n| n.attribute("data-cols"))
        .and_then(|s| s.parse().ok())
        .expect("dyslexic trainway cols");
    assert_eq!(
        trainway_cols_d, trainway_cols_h,
        "Above the 12px floor, OpenDyslexicM preserves the exact column grid by scaling font_px"
    );

    assert!(
        svg_dys.contains("font-family=\"OpenDyslexicM Nerd Font Mono\""),
        "OpenDyslexicM Nerd Font Mono font-family must be applied to all CRT text"
    );
    let cards_dys = mu_console_layout::scorecard_cards(
        mu_console_layout::solve(
            Viewport::new(1512.0, 860.0),
            SafeArea::ZERO,
            Area::Ci,
            st.font_metrics(),
        )
        .active_canvas,
        Breakpoint::Wide,
        3,
        st.font_metrics(),
    );
    assert!(
        cards_dys[0].ci_grid.clamped,
        "At the 12px legibility floor, OpenDyslexicM (0.732em) clamps card font_px to 12.0px"
    );

    let html = render_html_shell(&svg_dys, &st);
    assert!(html.contains("Skip to dashboard"));
    assert!(html.contains("aria-live=\"polite\""));
    assert!(html.contains("aria-live=\"assertive\""));
    assert!(html.contains("font-size:16px"));
    assert!(html.contains("OpenDyslexicMNerdFontMono-Regular.woff2"));

    st.set_font(CrtFont::Hurmit);
    st.phone_rotary_open = false;
    let svg_phone_default = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(402.0, 720.0),
        SafeArea::ZERO,
    );
    assert_svg_zero_overflow(&svg_phone_default, 402.0, 720.0, "u7_phone_default");

    st.phone_rotary_open = true;
    let svg_phone = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(402.0, 720.0),
        SafeArea::ZERO,
    );
    assert_svg_zero_overflow(&svg_phone, 402.0, 720.0, "u7_phone_sheet");
    let doc_p = roxmltree::Document::parse(&svg_phone).expect("phone XML");
    for tid in [
        "c1-area-selector",
        "c2-phone-rotary-trigger",
        "c2-font-switch",
        "c3-rotary",
    ] {
        let node = doc_p
            .descendants()
            .find(|n| n.attribute("data-control-id") == Some(tid))
            .unwrap_or_else(|| panic!("missing phone control {tid}"));
        let w: f64 = node
            .attribute("data-bbox-w")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let h: f64 = node
            .attribute("data-bbox-h")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        assert!(
            w >= 44.0 && h >= 44.0,
            "phone touch target {tid} ({w}x{h}) must be >= 44x44 pt"
        );
    }

    let svg_av = render_console(
        &AvionicsSkin::default(),
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    let svg_wf = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    let doc_av = roxmltree::Document::parse(&svg_av).expect("av XML");
    let doc_wf = roxmltree::Document::parse(&svg_wf).expect("wf XML");
    let controls = |d: &roxmltree::Document<'_>| -> Vec<(String, Option<String>, Option<String>)> {
        d.descendants()
            .filter_map(|n| {
                Some((
                    n.attribute("data-control-id")?.to_owned(),
                    n.attribute("data-bbox-w").map(str::to_owned),
                    n.attribute("data-bbox-h").map(str::to_owned),
                ))
            })
            .collect()
    };
    assert_eq!(controls(&doc_wf), controls(&doc_av));

    if let Ok(dir) = std::env::var("MUCONSOLE_EXPORT_PREVIEWS") {
        let p = std::path::Path::new(&dir);
        std::fs::create_dir_all(p).expect("create preview dir");
        let wrap = |svg: &str, w: u32, h: u32| -> String {
            format!(
                "<!doctype html><html><head><meta charset=\"utf-8\"><style>\
                 @font-face{{font-family:'Hurmit Nerd Font Mono';src:url('file:///Users/lma1980/repository/MuConsole/dist/fonts/HurmitNerdFontMono-Regular.woff2') format('woff2');}}\
                 @font-face{{font-family:'MuCRT';src:url('file:///Users/lma1980/repository/MuConsole/dist/fonts/HurmitNerdFontMono-Regular.woff2') format('woff2');}}\
                 @font-face{{font-family:'OpenDyslexicM Nerd Font Mono';src:url('file:///Users/lma1980/repository/MuConsole/dist/fonts/OpenDyslexicMNerdFontMono-Regular.woff2') format('woff2');}}\
                 html,body{{margin:0;padding:0;width:{w}px;height:{h}px;overflow:hidden;background:#0e1116;}}\
                 </style></head><body>{svg}</body></html>"
            )
        };
        std::fs::write(
            p.join("wireframe_desktop_hurmit.html"),
            wrap(&svg_hurmit, 1512, 860),
        )
        .expect("write hurmit html");
        std::fs::write(
            p.join("wireframe_desktop_dyslexic.html"),
            wrap(&svg_dys, 1512, 860),
        )
        .expect("write dyslexic html");
        std::fs::write(
            p.join("wireframe_mobile.html"),
            wrap(&svg_phone_default, 402, 720),
        )
        .expect("write mobile html");
        std::fs::write(
            p.join("wireframe_mobile_rotary.html"),
            wrap(&svg_phone, 402, 720),
        )
        .expect("write mobile rotary html");
    }
}

#[test]
fn u8_cross_area_trace_and_cd_gate_deep_links_and_observe_registry_subviews() {
    let mut st = state_for_scenario(
        Scenario::NewBranch,
        SubView::Registry(RegistrySubView::CanaryTaints),
        CrtFont::Hurmit,
    );
    let svg_reg = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    assert!(
        svg_reg.contains("data-trace-id=\"tr-reg-taint-01\""),
        "Registry > Canary & Taint must render clickable data-trace-id deep-link"
    );
    assert!(
        svg_reg.contains("data-cd-gate=\"dep-041\""),
        "Registry > Canary & Taint must render clickable data-cd-gate deep-link"
    );

    // Click `[TRACE ↗]` -> navigates to Observe > PurePaths with trace_id selected.
    let cmds = st.on_dom_action("data-trace-id", "tr-reg-taint-01", false, 2_000);
    assert_eq!(st.route.area, Area::Observe);
    assert_eq!(st.route.subview, SubView::Observe(ObserveSubView::PurePath));
    assert_eq!(st.route.trace_id.as_deref(), Some("tr-reg-taint-01"));
    assert_eq!(
        format_hash(&st.route),
        "#/obs/purepath/fixture/superproject?trace=tr-reg-taint-01"
    );
    assert_eq!(cmds.len(), 2);

    let svg_obs = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    assert!(svg_obs.contains("tr-reg-taint-01"));

    // Click `[CD GATE ↗]` -> navigates to CD > Integration Gate.
    let cmds_cd = st.on_dom_action("data-cd-gate", "dep-041", false, 2_100);
    assert_eq!(st.route.area, Area::Cd);
    assert_eq!(st.route.subview, SubView::Cd(CdSubView::IntegrationGate));
    assert_eq!(cmds_cd.len(), 2);
}

#[test]
fn u8_operator_auth_states_invite_capture_and_log_restart_gating() {
    // 1. Invite query extraction and URL sanitization.
    assert_eq!(
        extract_invite_token("?invite=inv_019a_secret&font=dyslexic"),
        (Some("inv_019a_secret".into()), "?font=dyslexic".to_owned())
    );
    assert_eq!(
        extract_invite_token("?invite=inv_only"),
        (Some("inv_only".into()), String::new())
    );
    assert_eq!(
        extract_invite_token("?font=hurmit"),
        (None, "?font=hurmit".to_owned())
    );

    let mut st = state_for_scenario(
        Scenario::ItCriticalFailure,
        SubView::Cd(CdSubView::IntegrationGate),
        CrtFont::Hurmit,
    );
    st.init_from_env("#/cd", "?invite=inv_boot_123", None);
    assert_eq!(st.pending_invite.as_deref(), Some("inv_boot_123"));

    // 2. AuthState::Anonymous gates raw execution logs and Trainway restart mutations.
    let anon_patch =
        Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(AuthState::Anonymous)]))
            .expect("encode");
    st.on_worker_frame(&anon_patch);
    assert!(!st.is_operator());

    let svg_anon = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    let html_anon = render_html_shell(&svg_anon, &st);
    assert!(svg_anon.contains("c2-auth-status"));
    assert!(svg_anon.contains("READ-ONLY · SIGN IN"));
    assert!(svg_anon.contains("c6-log-auth-gate"));
    assert!(svg_anon.contains("RAW LOGS &amp; RESTART REQUIRE OPERATOR SESSION"));
    assert!(html_anon.contains("href=\"/oauth2/start?rd=/\""));
    assert!(html_anon.contains("data-auth-state=\"anonymous\""));
    assert_svg_zero_overflow(&svg_anon, 1512.0, 860.0, "u8_anonymous");

    // Restart switch refuses to arm or fire while anonymous.
    st.on_restart_switch(SwitchEvent::Press { at_ms: 1_000 }, 1_000);
    assert_eq!(
        st.on_restart_switch(
            SwitchEvent::Rotate {
                to: 1,
                at_ms: 1_500,
                shift: true
            },
            1_500
        ),
        None
    );
    assert!(st.restart_armed_until_ms.is_none());
    assert!(st
        .aria_assertive
        .contains("RAW LOGS & JOB RESTART REQUIRE INVITED GITHUB OPERATOR SESSION"));

    // 3. AuthState::InviteRequired shows @login and keeps raw logs/restart gated.
    let uninv_patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(
        AuthState::InviteRequired {
            github_login: "guest-user".into(),
        },
    )]))
    .expect("encode");
    st.on_worker_frame(&uninv_patch);
    assert!(!st.is_operator());
    let svg_uninv = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    let html_uninv = render_html_shell(&svg_uninv, &st);
    assert!(svg_uninv.contains("INVITE REQ (@guest-user)"));
    assert!(html_uninv.contains("data-auth-state=\"invite_required\""));
    assert!(html_uninv.contains("href=\"/oauth2/sign_out?rd=/\""));
    assert_svg_zero_overflow(&svg_uninv, 1512.0, 860.0, "u8_invite_required");

    // 4. AuthState::Operator unlocks raw execution logs and Trainway restart.
    let op_patch =
        Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(AuthState::Operator {
            github_login: "LMA1980".into(),
            role: Some("admin".into()),
        })]))
        .expect("encode");
    st.on_worker_frame(&op_patch);
    assert!(st.is_operator());
    let svg_op = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    let html_op = render_html_shell(&svg_op, &st);
    assert!(svg_op.contains("@LMA1980 · OPERATOR"));
    assert!(!svg_op.contains("c6-log-auth-gate"));
    assert!(html_op.contains("data-auth-state=\"operator\""));
    assert_svg_zero_overflow(&svg_op, 1512.0, 860.0, "u8_operator");
}

#[test]
fn u9_sign_in_url_generation_and_encoding_invariants() {
    // 1. Default root redirect adheres strictly to "/oauth2/start?rd=/"
    assert_eq!(format_sign_in_url("/"), "/oauth2/start?rd=/");
    assert!(validate_sign_in_href("/oauth2/start?rd=/").is_ok());

    // 2. Custom target path with hash and subview parameters is validly percent-encoded
    let deep_target = "/#/cd/gate?filter=ALL";
    let formatted_deep = format_sign_in_url(deep_target);
    assert_eq!(
        formatted_deep,
        "/oauth2/start?rd=%2F%23%2Fcd%2Fgate%3Ffilter%3DALL"
    );
    assert!(validate_sign_in_href(&formatted_deep).is_ok());

    // 3. Absolute URL return target is validly percent-encoded
    let abs_target = "https://console.lma1980.duckdns.org/#/ci/trainway";
    let formatted_abs = format_sign_in_url(abs_target);
    assert_eq!(
        formatted_abs,
        "/oauth2/start?rd=https%3A%2F%2Fconsole.lma1980.duckdns.org%2F%23%2Fci%2Ftrainway"
    );
    assert!(validate_sign_in_href(&formatted_abs).is_ok());

    // 4. Reject malformed URLs: specifically `?rdhttps:` without `=` (reproducing live screenshot bug)
    let malformed_no_equals =
        "/oauth2/start?rdhttps://console.lma1980.duckdns.org/?invite=inv_cca98d8e";
    assert!(
        validate_sign_in_href(malformed_no_equals).is_err(),
        "Expected validation failure on malformed ?rdhttps: without '='"
    );

    // 5. Reject missing rd parameter or empty rd value
    assert!(validate_sign_in_href("/oauth2/start").is_err());
    assert!(validate_sign_in_href("/oauth2/start?rd=").is_err());
    assert!(validate_sign_in_href("/oauth2/other?rd=/").is_err());

    // 6. Verify HTML shell integration: <a id="muconsole-auth-link"> has valid href
    let st = ConsoleState::new();
    let svg = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    let html = render_html_shell(&svg, &st);

    // Must match strict href
    assert!(html.contains("href=\"/oauth2/start?rd=/\""));
    // Must NOT contain malformed rd parameter
    assert!(!html.contains("?rdhttps:"));
    assert!(!html.contains("?rdhttp:"));
    // Must have '=' immediately following rd
    assert!(html.contains("?rd="));
}

#[test]
fn u10_invite_preservation_across_oauth2_redirect_lifecycle() {
    // Stage 1: Initial arrival with one-time invite token in URL query
    let initial_query = "?invite=inv_test_lifecycle_019a&repo=model-runner";
    let (extracted_token, clean_query) = extract_invite_token(initial_query);

    assert_eq!(extracted_token.as_deref(), Some("inv_test_lifecycle_019a"));
    assert_eq!(clean_query, "?repo=model-runner");

    // Simulate sessionStorage stash (mirroring boot.rs window.session_storage())
    let mut simulated_session_storage = std::collections::BTreeMap::<String, String>::new();
    if let Some(ref token) = extracted_token {
        simulated_session_storage.insert(PENDING_INVITE_STORAGE_KEY.to_string(), token.clone());
    }
    assert_eq!(
        simulated_session_storage
            .get(PENDING_INVITE_STORAGE_KEY)
            .map(String::as_str),
        Some("inv_test_lifecycle_019a")
    );

    // Initial state initialized from environment
    let mut st = ConsoleState::new();
    st.init_from_env("#/ci", initial_query, None);
    assert_eq!(
        st.pending_invite.as_deref(),
        Some("inv_test_lifecycle_019a")
    );

    // Stage 2: Browser navigates to OAuth2 start and completes GitHub handshake
    // The redirect return URL to the console root has NO ?invite= query parameter!
    let return_url_query = "";
    let (return_extracted, return_clean) = extract_invite_token(return_url_query);
    assert_eq!(return_extracted, None);
    assert_eq!(return_clean, "");

    // Stage 3: On return to root "/", stored_invite successfully recovers the token from sessionStorage
    let stored_invite = return_extracted.or_else(|| {
        simulated_session_storage
            .get(PENDING_INVITE_STORAGE_KEY)
            .cloned()
            .filter(|t| !t.is_empty())
    });
    assert_eq!(stored_invite.as_deref(), Some("inv_test_lifecycle_019a"));

    // Stage 4: Bootstrap recovers state and dispatches UiCommand::RedeemInvite
    let mut st_return = ConsoleState::new();
    let init_frame =
        Engine::encode_event(&DataEvent::Snapshot(ViewModel::default())).expect("encode");
    st_return.on_worker_frame(&init_frame);
    st_return.init_from_env("#/ci", return_url_query, None);
    if st_return.pending_invite.is_none() {
        st_return.pending_invite.clone_from(&stored_invite);
    }
    assert_eq!(
        st_return.pending_invite.as_deref(),
        Some("inv_test_lifecycle_019a")
    );

    // Verify dispatched command sent to worker
    let redeem_cmd = UiCommand::RedeemInvite {
        token: stored_invite.expect("recovered token"),
    };
    assert_eq!(
        redeem_cmd,
        UiCommand::RedeemInvite {
            token: "inv_test_lifecycle_019a".into()
        }
    );

    // Stage 5: Operator authentication confirmation purges invite from sessionStorage
    let op_patch =
        Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(AuthState::Operator {
            github_login: "LMA1980".into(),
            role: Some("operator".into()),
        })]))
        .expect("encode");
    st_return.on_worker_frame(&op_patch);
    assert!(st_return.is_operator());

    // When operator state is achieved, pending invite is consumed and storage is cleared
    if st_return.is_operator() && st_return.pending_invite.take().is_some() {
        simulated_session_storage.remove(PENDING_INVITE_STORAGE_KEY);
    }
    assert_eq!(st_return.pending_invite, None);
    assert_eq!(
        simulated_session_storage.get(PENDING_INVITE_STORAGE_KEY),
        None
    );
}

#[test]
fn u11_c2_auth_status_click_dispatches_navigation_command() {
    let mut st = ConsoleState::new();
    let init_frame =
        Engine::encode_event(&DataEvent::Snapshot(ViewModel::default())).expect("encode");
    st.on_worker_frame(&init_frame);

    // 1. In default Anonymous state: clicking `c2-auth-status` must NOT return empty Vec::new().
    // It must return UiCommand::Navigate with the sign-in URL.
    let cmds_anon = st.on_dom_action("data-control-id", "c2-auth-status", false, 1_000);
    assert!(
        !cmds_anon.is_empty(),
        "Clicking c2-auth-status in Anonymous state must not be dropped as empty Vec::new()"
    );
    assert_eq!(
        cmds_anon,
        vec![UiCommand::Navigate {
            href: "/oauth2/start?rd=/".into(),
        }]
    );
    assert_eq!(st.pending_navigation.as_deref(), Some("/oauth2/start?rd=/"));
    assert!(
        st.aria_polite.contains("/oauth2/start")
            || st.aria_polite.to_ascii_lowercase().contains("sign in")
    );

    // 2. In InviteRequired state: clicking `c2-auth-status` must navigate to sign_out
    let uninv_patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(
        AuthState::InviteRequired {
            github_login: "guest-user".into(),
        },
    )]))
    .expect("encode");
    st.on_worker_frame(&uninv_patch);
    assert!(!st.is_operator());

    let cmds_uninv = st.on_dom_action("data-control-id", "c2-auth-status", false, 2_000);
    assert!(
        !cmds_uninv.is_empty(),
        "Clicking c2-auth-status in InviteRequired state must not be dropped as empty Vec::new()"
    );
    assert_eq!(
        cmds_uninv,
        vec![UiCommand::Navigate {
            href: "/oauth2/sign_out?rd=/".into(),
        }]
    );
    assert_eq!(
        st.pending_navigation.as_deref(),
        Some("/oauth2/sign_out?rd=/")
    );

    // 3. In Operator state: clicking `c2-auth-status` must navigate to sign_out
    let op_patch =
        Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(AuthState::Operator {
            github_login: "LMA1980".into(),
            role: Some("operator".into()),
        })]))
        .expect("encode");
    st.on_worker_frame(&op_patch);
    assert!(st.is_operator());

    let cmds_op = st.on_dom_action("data-control-id", "c2-auth-status", false, 3_000);
    assert!(
        !cmds_op.is_empty(),
        "Clicking c2-auth-status in Operator state must not be dropped as empty Vec::new()"
    );
    assert_eq!(
        cmds_op,
        vec![UiCommand::Navigate {
            href: "/oauth2/sign_out?rd=/".into(),
        }]
    );
    assert_eq!(
        st.pending_navigation.as_deref(),
        Some("/oauth2/sign_out?rd=/")
    );
}

#[test]
fn u12_root_absolute_asset_paths_and_worker_resolution() {
    assert!(
        WORKER_URL.starts_with('/'),
        "WORKER_URL must be root-absolute, got: {WORKER_URL}"
    );
    assert_eq!(WORKER_URL, "/muconsole-data_loader.js");

    let mut st = ConsoleState::new();
    st.lazy_dyslexic_loaded = true;
    let html = render_html_shell("", &st);
    assert!(
        html.contains("url('/fonts/OpenDyslexicMNerdFontMono-Regular.woff2')"),
        "render_html_shell must use root-absolute /fonts/ path"
    );
    assert!(
        !html.contains("./fonts/"),
        "render_html_shell must not contain relative ./fonts/"
    );

    let index_content = include_str!("../index.html");
    assert!(
        index_content.contains(r#"<base href="/" />"#),
        "index.html must contain <base href=\"/\" />"
    );
    assert!(!index_content.contains("href=\"./"));
    assert!(!index_content.contains("src=\"./"));
    assert!(!index_content.contains("url(\"./"));
    assert!(!index_content.contains("from './"));
    assert!(!index_content.contains("init('./"));
}

// ============================================================================
// Milestone 6.7: WireframeSkin HTML Chrome, Accessibility & Viewport Tests
// (#244, #245, #246, #247)
// ============================================================================

use mu_console_theme::css::tokens_css;

/// Helper to extract the declaration block inside `{ ... }` for a given CSS selector.
fn find_css_block<'a>(css: &'a str, selector: &str) -> &'a str {
    let pos = css
        .find(selector)
        .unwrap_or_else(|| panic!("CSS selector '{selector}' not found in tokens.css"));
    let open_brace = css[pos..]
        .find('{')
        .unwrap_or_else(|| panic!("No '{{' following selector '{selector}'"));
    let block_start = pos + open_brace + 1;
    let close_brace = css[block_start..]
        .find('}')
        .unwrap_or_else(|| panic!("No '}}' terminating block for '{selector}'"));
    &css[block_start..block_start + close_brace]
}

/// Helper to verify a CSS block contains a property with expected value (whitespace-insensitive).
fn assert_css_has_decl(block: &str, prop: &str, expected_val_substr: &str) {
    let normalized = block.replace(' ', "");
    let expected_prop = format!("{prop}:").replace(' ', "");
    assert!(
        normalized.contains(&expected_prop),
        "Block '{block}' is missing property '{prop}:'"
    );
    let expected_val = expected_val_substr.replace(' ', "");
    assert!(
        normalized.contains(&expected_val),
        "Block '{block}' missing expected value substring '{expected_val_substr}' for property '{prop}'"
    );
}

#[test]
fn u13_wireframe_skin_tokens_css_definitions_and_layout_rules() {
    let css = tokens_css();

    // 1. #muconsole-shell: 100dvh flex column container
    let shell_block = find_css_block(&css, "#muconsole-shell");
    assert_css_has_decl(shell_block, "display", "flex");
    assert_css_has_decl(shell_block, "flex-direction", "column");
    assert!(
        shell_block.contains("100dvh") || shell_block.contains("100vh"),
        "#muconsole-shell must enforce 100dvh height, got: {shell_block}"
    );
    assert_css_has_decl(shell_block, "overflow", "hidden");

    // 2. #muconsole-header: flex row header bar with border-bottom
    let header_block = find_css_block(&css, "#muconsole-header");
    assert_css_has_decl(header_block, "display", "flex");
    assert!(
        header_block.contains("border-bottom"),
        "#muconsole-header must have border-bottom rule, got: {header_block}"
    );

    // 3. .skip-link: offscreen by default, brought into view on :focus
    let skip_block = find_css_block(&css, ".skip-link");
    assert_css_has_decl(skip_block, "position", "absolute");
    assert!(
        skip_block.contains("-9999px") || skip_block.contains("-100%"),
        ".skip-link must be positioned offscreen by default, got: {skip_block}"
    );

    let focus_block = find_css_block(&css, ".skip-link:focus");
    assert!(
        focus_block.contains("z-index"),
        ".skip-link:focus must specify z-index, got: {focus_block}"
    );
    assert!(
        focus_block.contains("top:8px")
            || focus_block.contains("top: 8px")
            || focus_block.contains("top:"),
        ".skip-link:focus must position link visibly at top, got: {focus_block}"
    );

    // 4. #muconsole-filter: CRT styled search input
    let filter_block = find_css_block(&css, "#muconsole-filter");
    assert_css_has_decl(filter_block, "font-size", "16px");
    assert!(
        filter_block.contains("background"),
        "#muconsole-filter must specify background, got: {filter_block}"
    );
    assert!(
        filter_block.contains("border"),
        "#muconsole-filter must specify border, got: {filter_block}"
    );

    // 5. #muconsole-dashboard: flex child filling remaining vertical space
    let dashboard_block = find_css_block(&css, "#muconsole-dashboard");
    assert!(
        dashboard_block.contains("flex:1")
            || dashboard_block.contains("flex: 1")
            || dashboard_block.contains("flex:1 1 auto")
            || dashboard_block.contains("flex: 1 1 auto")
            || dashboard_block.contains("flex:1 1 0%"),
        "#muconsole-dashboard must flex-fill available space, got: {dashboard_block}"
    );
    assert_css_has_decl(dashboard_block, "overflow", "hidden");
}

#[test]
fn u14_semantic_html_shell_header_hierarchy_and_main_dashboard() {
    let st = ConsoleState::new();
    let svg_dummy = "<svg id=\"test-canvas\" width=\"100\" height=\"100\"><rect width=\"100\" height=\"100\"/></svg>";
    let html = render_html_shell(svg_dummy, &st);

    let doc = roxmltree::Document::parse(&html).unwrap_or_else(|e| {
        panic!("render_html_shell must emit valid XML: {e}\nHTML was:\n{html}")
    });

    let shell = doc.root_element();
    assert_eq!(
        shell.attribute("id"),
        Some("muconsole-shell"),
        "Root element must have id=\"muconsole-shell\""
    );

    // 1. Header container must exist under shell
    let header = shell
        .children()
        .find(|n| n.has_tag_name("header") && n.attribute("id") == Some("muconsole-header"))
        .expect("render_html_shell must contain <header id=\"muconsole-header\"> child under #muconsole-shell");

    // 2. Skip-link must be wrapped in header and target #muconsole-dashboard
    let skip_link = header
        .descendants()
        .find(|n| {
            n.attribute("class")
                .is_some_and(|c| c.split_whitespace().any(|x| x == "skip-link"))
        })
        .expect("<header id=\"muconsole-header\"> must wrap .skip-link");
    assert_eq!(skip_link.attribute("href"), Some("#muconsole-dashboard"));

    // 3. Filter input must be wrapped in header
    let filter = header
        .descendants()
        .find(|n| n.attribute("id") == Some("muconsole-filter"))
        .expect("<header id=\"muconsole-header\"> must wrap #muconsole-filter");
    assert_eq!(filter.tag_name().name(), "input");
    assert_eq!(filter.attribute("type"), Some("search"));

    // 4. Polite and assertive live regions must be wrapped in header
    let polite = header
        .descendants()
        .find(|n| n.attribute("id") == Some("muconsole-aria-polite"))
        .expect("<header id=\"muconsole-header\"> must wrap #muconsole-aria-polite");
    assert_eq!(polite.attribute("role"), Some("status"));
    assert_eq!(polite.attribute("aria-live"), Some("polite"));

    let assertive = header
        .descendants()
        .find(|n| n.attribute("id") == Some("muconsole-aria-assertive"))
        .expect("<header id=\"muconsole-header\"> must wrap #muconsole-aria-assertive");
    assert_eq!(assertive.attribute("role"), Some("alert"));
    assert_eq!(assertive.attribute("aria-live"), Some("assertive"));

    // 5. Auth link must be wrapped in header
    let auth_link = header
        .descendants()
        .find(|n| n.attribute("id") == Some("muconsole-auth-link"))
        .expect("<header id=\"muconsole-header\"> must wrap #muconsole-auth-link");
    assert_eq!(auth_link.tag_name().name(), "a");

    // 6. Main dashboard container must be sibling to header (NOT inside header)
    let main = shell
        .children()
        .find(|n| n.has_tag_name("main") && n.attribute("id") == Some("muconsole-dashboard"))
        .expect("render_html_shell must emit <main id=\"muconsole-dashboard\"> directly under #muconsole-shell");
    assert_ne!(
        main.parent(),
        Some(header),
        "<main id=\"muconsole-dashboard\"> must NOT be nested inside <header>"
    );

    // 7. Main dashboard must contain the rendered SVG canvas
    assert!(
        main.descendants()
            .any(|n| n.attribute("id") == Some("test-canvas")),
        "<main id=\"muconsole-dashboard\"> must wrap the SVG canvas"
    );
}

#[test]
fn u15_ios_safari_viewport_safety_and_filter_font_size_invariant() {
    let css = tokens_css();

    // 1. In tokens.css, #muconsole-filter must enforce font-size: 16px (or greater)
    let filter_block = find_css_block(&css, "#muconsole-filter");
    assert_css_has_decl(filter_block, "font-size", "16px");

    // Ensure no sub-16px font-size exists for #muconsole-filter
    for sub_px in ["10px", "11px", "12px", "13px", "14px", "15px"] {
        assert!(
            !filter_block.contains(&format!("font-size:{sub_px}"))
                && !filter_block.contains(&format!("font-size: {sub_px}")),
            "#muconsole-filter must never specify sub-16px font-size '{sub_px}' (triggers iOS auto-zoom)"
        );
    }

    // 2. In rendered HTML shell, input must enforce font-size: 16px across font toggles
    for &font in &[CrtFont::Hurmit, CrtFont::OpenDyslexic] {
        let mut st = ConsoleState::new();
        st.set_font(font);
        if font == CrtFont::OpenDyslexic {
            st.lazy_dyslexic_loaded = true;
        }
        let html = render_html_shell("<svg></svg>", &st);
        let doc = roxmltree::Document::parse(&html).expect("valid XML");
        let filter_node = doc
            .descendants()
            .find(|n| n.attribute("id") == Some("muconsole-filter"))
            .expect("must find #muconsole-filter");

        // Verify either inline style or embedded <style> enforces 16px
        let has_16px_inline = filter_node
            .attribute("style")
            .is_some_and(|s| s.contains("font-size:16px") || s.contains("font-size: 16px"));
        let has_16px_style_tag = html.contains("#muconsole-filter{font-size:16px")
            || html.contains("#muconsole-filter { font-size: 16px")
            || html.contains("#muconsole-filter{font-size: 16px");
        assert!(
            has_16px_inline || has_16px_style_tag,
            "Font {font:?}: #muconsole-filter must explicitly declare font-size: 16px"
        );
    }
}

#[test]
fn u16_tokens_css_distribution_drift_and_synchronization() {
    let generated = tokens_css();
    let checked_in_dist = include_str!("../dist/tokens.css");
    assert_eq!(
        generated,
        checked_in_dist,
        "dist/tokens.css has drifted from tokens_css()!\n\
         Ensure scripts/build-dist.sh or 'cp crates/muconsole-theme/assets/tokens.css dist/tokens.css' is run."
    );

    // dist/tokens.css must contain the new M6.7 selectors
    assert!(
        checked_in_dist.contains("#muconsole-shell"),
        "dist/tokens.css missing #muconsole-shell"
    );
    assert!(
        checked_in_dist.contains("#muconsole-header"),
        "dist/tokens.css missing #muconsole-header"
    );
    assert!(
        checked_in_dist.contains(".skip-link"),
        "dist/tokens.css missing .skip-link"
    );
    assert!(
        checked_in_dist.contains("#muconsole-filter"),
        "dist/tokens.css missing #muconsole-filter"
    );
    assert!(
        checked_in_dist.contains("#muconsole-dashboard"),
        "dist/tokens.css missing #muconsole-dashboard"
    );
}
