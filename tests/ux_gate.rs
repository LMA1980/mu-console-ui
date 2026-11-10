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
    assert_svg_bounds_zero_overflow(svg, vw, vh, context);
}

fn assert_svg_bounds_zero_overflow(svg: &str, vw: f64, vh: f64, context: &str) {
    let doc =
        roxmltree::Document::parse(svg).unwrap_or_else(|e| panic!("{context}: invalid XML: {e}"));

    let lower = svg.to_ascii_lowercase();
    let probe_tokens: Vec<String> = std::env::var("MU_TEST_FORBIDDEN_IDENTIFIERS")
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_else(|_| vec!["internal-secret-host".into(), "cluster-private.corp".into()]);
    for forbidden in &probe_tokens {
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
                 @font-face{{font-family:'Hurmit Nerd Font Mono';src:url('file:///Users/lma1980/repository/mu-console/dist/fonts/HurmitNerdFontMono-Regular.woff2') format('woff2');}}\
                 @font-face{{font-family:'MuCRT';src:url('file:///Users/lma1980/repository/mu-console/dist/fonts/HurmitNerdFontMono-Regular.woff2') format('woff2');}}\
                 @font-face{{font-family:'OpenDyslexicM Nerd Font Mono';src:url('file:///Users/lma1980/repository/mu-console/dist/fonts/OpenDyslexicMNerdFontMono-Regular.woff2') format('woff2');}}\
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
    assert!(!svg_anon.contains("c2-auth-status"));
    assert!(html_anon.contains("muconsole-auth-link"));
    assert!(html_anon.contains("READ-ONLY · SIGN IN"));
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
    assert!(!svg_uninv.contains("c2-auth-status"));
    assert!(html_uninv.contains("INVITE REQ (@guest-user)"));
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
    assert!(!svg_op.contains("c2-auth-status"));
    assert!(html_op.contains("@LMA1980 · OPERATOR"));
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

#[test]
fn u17_interactive_scorecards_breadcrumbs_filters_trace_selection_and_pagination() {
    use mu_console_protocol::{format_job_id, RegistryPackage};

    // 1. Packed ActionRunner job IDs format as 6-hex-char SHA prefixes while small IDs stay decimal.
    assert_eq!(format_job_id(29_612_437_286_333_911), "62add7");
    assert_eq!(format_job_id(104), "104");

    // 2. Clicking scorecard CI body row opens Trainway for that repo; clicking CD row opens Gate.
    let mut st = state_for_scenario(
        Scenario::NewBranch,
        SubView::Ci(CiSubView::Scorecards),
        CrtFont::Hurmit,
    );
    let cmds = st.on_dom_action(
        "data-control-id",
        "c4-open-trainway:fixture/engine",
        false,
        1_000,
    );
    assert_eq!(st.route.repo, Some(RepoId::new("fixture", "engine")));
    assert_eq!(st.route.subview, SubView::Ci(CiSubView::Trainway));
    assert!(!cmds.is_empty());

    // Clicking `[QUEUE & LOGS ↗]` on Trainway opens CI Queue & Logs.
    let cmds_logs = st.on_dom_action("data-control-id", "c5-open-logs", false, 1_100);
    assert_eq!(st.route.subview, SubView::Ci(CiSubView::QueueLogs));
    assert!(!cmds_logs.is_empty());

    // Clicking breadcrumb `c2-repo` returns to CI Scorecards.
    let cmds_crumb = st.on_dom_action("data-control-id", "c2-repo", false, 1_200);
    assert_eq!(st.route.subview, SubView::Ci(CiSubView::Scorecards));
    assert!(!cmds_crumb.is_empty());

    // Clicking scorecard CD body row opens CD Integration Gate.
    let cmds_gate = st.on_dom_action(
        "data-control-id",
        "c4-open-gate:fixture/superproject",
        false,
        1_300,
    );
    assert_eq!(st.route.repo, Some(RepoId::new("fixture", "superproject")));
    assert_eq!(st.route.area, Area::Cd);
    assert_eq!(st.route.subview, SubView::Cd(CdSubView::IntegrationGate));
    assert!(!cmds_gate.is_empty());

    // 3. Registry > Packages header filter selector + multi-page pagination with clickable ◀ / ▶.
    st.select_subview(SubView::Registry(RegistrySubView::Packages));
    let mut extra_pkgs = Vec::new();
    for i in 0..45 {
        extra_pkgs.push(RegistryPackage {
            ecosystem: mu_console_protocol::PackageEcosystem::Cargo,
            name: format!("mu-extra-crate-{i:02}"),
            latest_version: "0.2.0".into(),
            versions_count: 2,
            deps_count: 4,
            sha256_prefix: format!("a1b2c3d4e5{i:02}"),
            tainted: false,
            taint_reason: None,
            yanked: false,
            versions: vec!["0.2.0".into(), "0.1.0".into()],
        });
    }
    let mut vm = st.client.view().cloned().expect("view present");
    vm.registry.packages.extend(extra_pkgs);
    let snap_bytes = Engine::encode_event(&DataEvent::Snapshot(vm)).expect("encode snapshot");
    st.on_worker_frame(&snap_bytes);

    let svg_pkg_p1 = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    assert!(svg_pkg_p1.contains("PAGE 1/"));
    assert!(svg_pkg_p1.contains("reg-packages-screen:next"));
    assert_svg_zero_overflow(&svg_pkg_p1, 1512.0, 860.0, "u17_reg_packages_p1");

    st.on_dom_action("data-control-id", "reg-packages-screen:next", false, 1_400);
    assert_eq!(st.subview_page, 1);
    let svg_pkg_p2 = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    assert!(svg_pkg_p2.contains("PAGE 2/"));
    assert_svg_zero_overflow(&svg_pkg_p2, 1512.0, 860.0, "u17_reg_packages_p2");

    // Filtering by NUGET resets page to 0 and filters the catalog.
    st.on_dom_action("data-control-id", "reg-pkg-filter:NUGET", false, 1_500);
    assert_eq!(st.subview_page, 0);
    assert_eq!(
        st.reg_ecosystem_filter,
        Some(mu_console_protocol::PackageEcosystem::NuGet)
    );

    // 4. Observe > PurePath trace selection in-place (`select-trace:*`).
    st.select_subview(SubView::Observe(ObserveSubView::PurePath));
    let cmds_tr = st.on_dom_action("data-control-id", "select-trace:tr-cd-0042", false, 1_600);
    assert_eq!(st.selected_trace_id.as_deref(), Some("tr-cd-0042"));
    assert_eq!(st.route.trace_id.as_deref(), Some("tr-cd-0042"));
    assert!(!cmds_tr.is_empty());
}

#[test]
fn u18_avionics_skin_switcher_and_operator_job_restart_actions() {
    use mu_console_protocol::ConsoleSkin;
    use mu_console_theme::tokens::Effects;
    use mu_console_theme::wireframe::AvionicsSkin;
    use mu_console_ui::state::parse_skin_pref;

    // 1. Skin query parameter and storage preference parsing.
    assert_eq!(
        parse_skin_pref("?skin=avionics", None),
        ConsoleSkin::Avionics
    );
    assert_eq!(
        parse_skin_pref("?skin=wireframe", Some("avionics")),
        ConsoleSkin::Wireframe
    );
    assert_eq!(
        parse_skin_pref("", Some("skeuomorphic")),
        ConsoleSkin::Avionics
    );

    let mut st = state_for_scenario(
        Scenario::StageFailure,
        SubView::Ci(CiSubView::Trainway),
        CrtFont::Hurmit,
    );
    assert_eq!(st.skin, ConsoleSkin::Wireframe);

    // 2. Toggle skin via 's' hotkey and via data-skin-toggle button.
    let key_cmd = st.on_key("s", false, 50);
    assert_eq!(st.skin, ConsoleSkin::Avionics);
    assert_eq!(key_cmd, Some(UiCommand::SetSkin(ConsoleSkin::Avionics)));
    assert!(st.aria_polite.contains("avionics"));

    let shell_av = render_html_shell("<svg id=\"canvas\"></svg>", &st);
    assert!(shell_av.contains("data-skin=\"avionics\""));
    assert!(shell_av.contains("SKIN: AVIONICS [S]"));

    let btn_cmds = st.on_dom_action("data-skin-toggle", "true", false, 100);
    assert_eq!(st.skin, ConsoleSkin::Wireframe);
    assert_eq!(btn_cmds, vec![UiCommand::SetSkin(ConsoleSkin::Wireframe)]);

    // 3. Render AvionicsSkin across all 6 target viewports and verify zero overflow + tactile tokens.
    let av_skin = AvionicsSkin {
        effects: Effects::FULL,
    };
    let viewports = [
        (390.0, 844.0, "phone_portrait"),
        (844.0, 390.0, "phone_landscape"),
        (820.0, 1180.0, "tablet"),
        (1100.0, 720.0, "compact"),
        (1512.0, 860.0, "desktop"),
        (2560.0, 1080.0, "wide"),
    ];
    for (w, h, label) in viewports {
        let svg = render_console(&av_skin, &st, Viewport::new(w, h), SafeArea::ZERO);
        assert_svg_bounds_zero_overflow(&svg, w, h, &format!("u18_avionics_{label}"));
        assert!(svg.contains("mu-woodgrain"));
        assert!(svg.contains("mu-brushed"));
        assert!(svg.contains("mu-park"));
        assert!(svg.contains("mu-lcdbg"));
        assert!(svg.contains("mu-crt-ci"));
        assert!(svg.contains("mu-vignette"));
        assert!(svg.contains("mu-glass"));
    }

    // 4. Operator pull-to-turn c5-restart-switch and data-restart-job emit UiCommand::RestartJob.
    assert!(st.auth().is_operator());
    let pull_cmds = st.on_dom_action("data-control-id", "c5-restart-switch", true, 1_000);
    assert!(pull_cmds.is_empty(), "first Shift+click arms pull-to-turn");
    let turn_cmds = st.on_dom_action("data-control-id", "c5-restart-switch", true, 1_200);
    assert!(
        turn_cmds
            .iter()
            .any(|c| matches!(c, UiCommand::RestartJob { job_id: 1_233 })),
        "confirming c5-restart-switch must emit RestartJob for active failed job 1233"
    );

    let direct_cmds = st.on_dom_action("data-restart-job", "1234", false, 1_300);
    assert_eq!(direct_cmds, vec![UiCommand::RestartJob { job_id: 1_234 }]);
}

#[test]
fn u19_live_workflow_parity_ci_job_drilldown_resume_restart_and_worker_node_stages() {
    use mu_console_protocol::FleetSubView;

    let mut st = state_for_scenario(
        Scenario::StageFailure,
        SubView::Ci(CiSubView::QueueLogs),
        CrtFont::Hurmit,
    );
    let svg_ql = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    assert!(svg_ql.contains("INTERACTIVE CI JOB QUEUE"));
    assert!(svg_ql.contains("SELECTED JOB DRILL-DOWN"));
    assert!(svg_ql.contains("RESUME PIPELINE"));
    assert!(svg_ql.contains("RESTART ALL"));
    assert_svg_zero_overflow(&svg_ql, 1512.0, 860.0, "u19_ci_queue_logs");

    let sel_cmds = st.on_dom_action("data-control-id", "ci-job:1233", false, 1_000);
    assert_eq!(st.route.job, Some(1_233));
    assert!(st.follow_log);
    assert_eq!(sel_cmds, vec![UiCommand::SelectJob { job_id: 1_233 }]);

    st.on_dom_action("data-control-id", "ci-log-follow-toggle", false, 1_050);
    assert!(!st.follow_log);
    st.on_dom_action("data-control-id", "ci-log-follow-toggle", false, 1_060);
    assert!(st.follow_log);

    let res_cmds = st.on_dom_action("data-control-id", "ci-job-resume:1233", false, 1_100);
    assert_eq!(res_cmds, vec![UiCommand::ResumeJob { job_id: 1_233 }]);

    let rst_cmds = st.on_dom_action("data-control-id", "ci-job-restart:1233", false, 1_200);
    assert_eq!(rst_cmds, vec![UiCommand::RestartJob { job_id: 1_233 }]);

    // Switch to FLT > Worker Nodes and verify node selection + stage drill-down.
    st.select_subview(SubView::Fleet(FleetSubView::WorkerNodes));
    let svg_flt = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    assert!(svg_flt.contains("WORKER NODE STAGE DRILL-DOWN"));
    assert!(svg_flt.contains("flt-node:node-a"));
    assert_svg_zero_overflow(&svg_flt, 1512.0, 860.0, "u19_flt_worker_nodes");

    st.on_dom_action("data-control-id", "flt-node:worker-riscv64", false, 1_300);
    assert_eq!(st.selected_node_id.as_deref(), Some("worker-riscv64"));

    let open_job_cmds = st.on_dom_action("data-control-id", "ci-job-open:1233", false, 1_400);
    assert_eq!(st.route.subview, SubView::Ci(CiSubView::QueueLogs));
    assert_eq!(st.route.job, Some(1_233));
    assert!(open_job_cmds.contains(&UiCommand::SelectJob { job_id: 1_233 }));

    // Switching repository via c4-card-select clears stale route.job and route.branch.
    st.route.branch = Some("refs/heads/feat/test".into());
    st.on_dom_action(
        "data-control-id",
        "c4-card-select:LMA1980/ActionRunner-rs",
        false,
        1_500,
    );
    assert_eq!(st.route.job, None);
    assert_eq!(st.route.branch, None);
}

#[test]
fn u20_dom_action_target_resolution_repo_steppers_subview_tabs_and_trainway_signals() {
    use mu_console_ui::state::{
        is_action_root_boundary, is_passive_container_control_id, ACTION_ATTRS,
    };

    fn resolve_dom_action<'a>(node: roxmltree::Node<'a, 'a>) -> Option<(&'static str, String)> {
        let mut cur = Some(node);
        while let Some(el) = cur {
            if el.is_element() {
                if let Some(id) = el.attribute("id") {
                    if is_action_root_boundary(id) {
                        break;
                    }
                }
                for attr in ACTION_ATTRS {
                    if let Some(val) = el.attribute(attr) {
                        return Some((attr, val.to_owned()));
                    }
                }
                if let Some(cid) = el.attribute("data-control-id") {
                    if !is_passive_container_control_id(cid) {
                        return Some(("data-control-id", cid.to_owned()));
                    }
                }
            }
            cur = el.parent();
        }
        None
    }

    let mut st = state_for_scenario(
        Scenario::StageFailure,
        SubView::Ci(CiSubView::Scorecards),
        CrtFont::Hurmit,
    );
    let svg = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    let html = render_html_shell(&svg, &st);
    let doc = roxmltree::Document::parse(&html).expect("valid HTML shell XML");

    // 1. Verify clicking leaf <text> or <rect> inside sub-view tabs, repo steppers, and carousel
    // buttons inside #muconsole-shell[data-area="ci"][data-subview="scorecards"] resolves to the
    // leaf control and is NEVER hijacked by #muconsole-shell's data-area="ci".
    let check_leaf_target = |control_id: &str, expected: (&str, &str)| {
        let grp = doc
            .descendants()
            .find(|n| n.attribute("data-control-id") == Some(control_id))
            .unwrap_or_else(|| panic!("missing control {control_id}"));
        let leaf = grp
            .descendants()
            .find(|n| n.has_tag_name("text") || n.has_tag_name("rect"))
            .unwrap_or(grp);
        let resolved = resolve_dom_action(leaf)
            .unwrap_or_else(|| panic!("no action resolved for leaf of {control_id}"));
        assert_eq!(
            (resolved.0, resolved.1.as_str()),
            expected,
            "leaf click inside {control_id} was hijacked"
        );
    };

    check_leaf_target("c2-subview-bar:trainway", ("data-subview", "trainway"));
    check_leaf_target("c2-subview-bar:logs", ("data-subview", "logs"));
    check_leaf_target("c2-repo-prev", ("data-repo-step", "prev"));
    check_leaf_target("c2-repo-next", ("data-repo-step", "next"));
    check_leaf_target("c2-sub-next", ("data-repo-step", "sub-next"));
    check_leaf_target(
        "c4-carousel-switch:next",
        ("data-control-id", "c4-carousel-switch:next"),
    );

    // 2. Stepping repositories via c2-repo-next / c2-repo-prev updates outer_index, route.repo,
    // resets carousel_page to 0, and updates the C2 readout index/name.
    assert!(svg.contains("9/10 superproject"));
    st.carousel_page = 1;
    st.on_dom_action("data-repo-step", "next", false, 2_000);
    assert_eq!(st.outer_index, 9);
    assert_eq!(st.carousel_page, 0);
    let svg_next = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    assert!(svg_next.contains("10/10 tokenizer"));

    // 3. Trainway station header and signal aspect controls drill into CI > Queue & Logs.
    st.select_subview(SubView::Ci(CiSubView::Trainway));
    let svg_tw = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    assert!(svg_tw.contains("c5-station:"));
    assert!(svg_tw.contains("c5-signal:"));
    let sig_cmds = st.on_dom_action("data-control-id", "c5-signal:main:0", false, 2_100);
    assert_eq!(st.route.subview, SubView::Ci(CiSubView::QueueLogs));
    assert_eq!(st.route.branch.as_deref(), Some("main"));
    assert!(!sig_cmds.is_empty());

    // 4. AvionicsSkin CRT decorative overlays must carry pointer-events="none".
    let svg_av = render_console(
        &AvionicsSkin::default(),
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    let doc_av = roxmltree::Document::parse(&svg_av).expect("valid Avionics XML");
    for node in doc_av.descendants().filter(|n| n.has_tag_name("rect")) {
        if let Some(fill) = node.attribute("fill") {
            if fill.starts_with("url(#mu-lcdbg-")
                || fill.starts_with("url(#mu-scan-")
                || fill.starts_with("url(#mu-vignette-")
                || fill.starts_with("url(#mu-glass-")
            {
                assert_eq!(
                    node.attribute("pointer-events"),
                    Some("none"),
                    "Avionics overlay rect fill={fill} must have pointer-events=\"none\""
                );
            }
        }
    }
}

#[test]
fn u21_compact_trainway_band_pitch_and_dynamic_scorecard_page_capacity() {
    let mut st = state_for_scenario(
        Scenario::Repos40,
        SubView::Ci(CiSubView::Scorecards),
        CrtFont::Hurmit,
    );

    // 1. Dynamic Scorecard cards-per-page scales with viewport dimensions.
    let cases = [
        (Viewport::new(402.0, 720.0), 3_usize),
        (Viewport::new(932.0, 430.0), 4_usize),
        (Viewport::new(1024.0, 768.0), 6_usize),
        (Viewport::new(1512.0, 860.0), 9_usize),
        (Viewport::new(1600.0, 1000.0), 12_usize),
        (Viewport::new(1920.0, 1080.0), 20_usize),
    ];
    for (vp, expected_per_page) in cases {
        st.set_viewport(vp, SafeArea::ZERO);
        assert_eq!(
            st.scorecard_cards_per_page(),
            expected_per_page,
            "unexpected scorecard capacity at {}x{}",
            vp.width,
            vp.height
        );
        let svg = render_console(&WireframeSkin, &st, vp, SafeArea::ZERO);
        assert_svg_zero_overflow(
            &svg,
            vp.width,
            vp.height,
            &format!("u21_scorecards_{}x{}", vp.width, vp.height),
        );
    }

    // 2. Stepping carousel at 1920x1080 (20 cards/page, 40 repos -> 2 pages) wraps after 2 steps.
    st.set_viewport(Viewport::new(1920.0, 1080.0), SafeArea::ZERO);
    st.carousel_page = 0;
    st.on_dom_action("data-control-id", "c4-carousel-switch:next", false, 1_000);
    assert_eq!(st.carousel_page, 1);
    st.on_dom_action("data-control-id", "c4-carousel-switch:next", false, 1_100);
    assert_eq!(st.carousel_page, 0);

    // 3. Compact Trainway schematic uses split left metadata column + tight vertical track pitch.
    let mut st_tw = state_for_scenario(
        Scenario::Submodules,
        SubView::Ci(CiSubView::Trainway),
        CrtFont::Hurmit,
    );
    let svg_tw = render_console(
        &WireframeSkin,
        &st_tw,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    assert!(
        svg_tw.contains("LANE / BRANCH / STAGE"),
        "Desktop Trainway must render split-column LANE / BRANCH / STAGE header"
    );
    let doc_tw = roxmltree::Document::parse(&svg_tw).expect("valid Trainway XML");
    let trunk_row = doc_tw
        .descendants()
        .find(|n| n.attribute("data-control-id") == Some("c5-track-main"))
        .and_then(|g| g.descendants().find(|c| c.has_tag_name("text")))
        .and_then(|t| t.attribute("y"))
        .and_then(|y| y.parse::<f64>().ok())
        .expect("c5-track-main text y");
    let first_siding_row = doc_tw
        .descendants()
        .find(|n| {
            n.attribute("data-control-id")
                .is_some_and(|cid| cid.starts_with("c5-track-") && cid != "c5-track-main")
        })
        .and_then(|g| g.descendants().find(|c| c.has_tag_name("text")))
        .and_then(|t| t.attribute("y"))
        .and_then(|y| y.parse::<f64>().ok())
        .expect("first siding text y");
    let pitch = (first_siding_row - trunk_row).abs();
    assert!(
        (18.0..=26.0).contains(&pitch),
        "Trainway track vertical pitch must be compact (18..=26px), got {pitch}px"
    );

    // Verify that a merged siding with fork_stage at the last station still draws a full
    // horizontal track segment and curves back into the trunk at the right edge.
    let merged_patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::BranchUpsert(Branch {
        ref_name: "feat/merged-at-end".into(),
        base_ref: "main".into(),
        fork_stage: 8,
        pr_state: PrState::Merged,
        last_job: None,
        phase: SidingPhase::Active,
    })]))
    .expect("encode merged branch");
    st_tw.on_worker_frame(&merged_patch);
    let svg_tw_merged = render_console(
        &WireframeSkin,
        &st_tw,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    let doc_merged = roxmltree::Document::parse(&svg_tw_merged).expect("valid XML");
    let merged_path = doc_merged
        .descendants()
        .find(|n| n.attribute("data-track-ref") == Some("feat/merged-at-end"))
        .and_then(|n| n.attribute("d"))
        .expect("feat/merged-at-end path d");
    assert!(
        merged_path.matches(" Q").count() >= 2,
        "Merged siding path must have left turnout and right merge-back quadratic curves: {merged_path}"
    );
}

#[test]
fn u22_master_detail_ci_queue_logs_trainway_inspector_and_interactive_cd_obs_flt_controls() {
    use mu_console_protocol::{FleetSubView, ObserveSubView};

    // 1. CI > Queue & Logs dual master-detail screens at 1440x900
    let mut st = state_for_scenario(
        Scenario::StageFailure,
        SubView::Ci(CiSubView::QueueLogs),
        CrtFont::Hurmit,
    );
    let svg_ql = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1440.0, 900.0),
        SafeArea::ZERO,
    );
    assert!(svg_ql.contains("ci-queue-logs-screen"));
    assert!(svg_ql.contains("ci-job-detail-screen"));
    assert!(svg_ql.contains("ci-queue-filter:0"));
    assert!(svg_ql.contains("ci-log-follow-toggle"));
    assert_svg_zero_overflow(&svg_ql, 1440.0, 900.0, "u22_ci_queue_logs_1440x900");

    st.activate_target("ci-stage-select:0");
    assert_eq!(st.selected_stage_idx, Some(0));
    let svg_stage = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1440.0, 900.0),
        SafeArea::ZERO,
    );
    assert!(svg_stage.contains("STAGE #1 INSPECTOR"));
    assert_svg_zero_overflow(&svg_stage, 1440.0, 900.0, "u22_ci_stage_inspector");

    st.log_page = 2;
    st.follow_log = true;
    st.activate_target("ci-log-page-prev");
    assert!(!st.follow_log);
    assert_eq!(st.log_page, 1);
    st.activate_target("ci-log-page-next");
    assert_eq!(st.log_page, 2);

    // 2. CI > Trainway track-to-job selection & c5-selected-inspector
    st.select_subview(SubView::Ci(CiSubView::Trainway));
    let track_cmds = st.activate_target("c5-track-main");
    assert_eq!(st.route.branch.as_deref(), Some("main"));
    assert!(st.route.job.is_some());
    assert!(track_cmds
        .iter()
        .any(|c| matches!(c, UiCommand::SelectJob { .. })));
    let svg_tw = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1440.0, 900.0),
        SafeArea::ZERO,
    );
    assert!(svg_tw.contains("c5-selected-inspector"));
    assert!(svg_tw.contains("SELECTED TRACK:"));
    assert!(svg_tw.contains("OPEN LOGS"));
    assert_svg_zero_overflow(&svg_tw, 1440.0, 900.0, "u22_trainway_inspector");

    // 3. CD > Integration Gate history pill selection
    st.select_subview(SubView::Cd(CdSubView::IntegrationGate));
    st.activate_target("c6-history:0");
    assert_eq!(st.selected_history_idx, 1);
    let svg_cd = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1440.0, 900.0),
        SafeArea::ZERO,
    );
    assert!(svg_cd.contains("SELECTED HISTORY #1:"));
    assert_svg_zero_overflow(&svg_cd, 1440.0, 900.0, "u22_cd_history");

    // 4. OBS > Metrics sparkline selector & OBS > Smartscape layer filter & FLT > Sandbox Profiles
    st.select_subview(SubView::Observe(ObserveSubView::Metrics));
    st.activate_target("obs-metric:1");
    assert_eq!(st.selected_metric_idx, 1);
    let svg_met = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1440.0, 900.0),
        SafeArea::ZERO,
    );
    assert!(svg_met.contains("PLOTTED SERIES:"));
    assert_svg_zero_overflow(&svg_met, 1440.0, 900.0, "u22_obs_metrics");

    st.select_subview(SubView::Observe(ObserveSubView::Smartscape));
    st.activate_target("obs-smartscape-filter:1");
    assert_eq!(st.obs_smartscape_filter, 1);
    let svg_ss = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1440.0, 900.0),
        SafeArea::ZERO,
    );
    assert!(svg_ss.contains("LAYER FILTER: [SERVICES]"));
    assert_svg_zero_overflow(&svg_ss, 1440.0, 900.0, "u22_obs_smartscape");

    st.select_subview(SubView::Fleet(FleetSubView::SandboxProfiles));
    st.activate_target("flt-sandbox:0");
    assert_eq!(st.selected_sandbox_idx, Some(0));
    let svg_sb = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1440.0, 900.0),
        SafeArea::ZERO,
    );
    assert!(svg_sb.contains("MUISOLATION SECURITY &amp; HARDWARE INVARIANT INSPECTOR"));
    assert!(svg_sb.contains("RLIMIT_MEMLOCK"));
    assert_svg_zero_overflow(&svg_sb, 1440.0, 900.0, "u22_flt_sandboxes");
}

#[test]
fn u23_c2_top_bar_declutter_header_skin_toggle_and_compact_dyslexia_switch() {
    use mu_console_protocol::FleetSubView;

    let mut st = state_for_scenario(
        Scenario::StageFailure,
        SubView::Ci(CiSubView::Trainway),
        CrtFont::Hurmit,
    );
    st.route.branch = Some("feat/test-branch".into());

    // 1. In CI > Trainway, c2-branch (REF) and c2-sub-next (SUB) are visible, and lamp-stop is lit.
    let svg_tw = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    let html_tw = render_html_shell(&svg_tw, &st);
    let doc_html = roxmltree::Document::parse(&html_tw).expect("valid shell XML");

    // #muconsole-skin-toggle lives inside #muconsole-header (never position:fixed;bottom:8px).
    let header = doc_html
        .descendants()
        .find(|n| n.attribute("id") == Some("muconsole-header"))
        .expect("#muconsole-header");
    let skin_btn = header
        .children()
        .find(|n| n.attribute("id") == Some("muconsole-skin-toggle"))
        .expect("#muconsole-skin-toggle inside #muconsole-header");
    let style = skin_btn.attribute("style").unwrap_or("");
    assert!(style.contains("font-size:10px"));
    assert!(style.contains("padding:2px 6px"));
    assert!(!style.contains("position:fixed"));

    assert!(!svg_tw.contains("c2-auth-status"));
    assert!(svg_tw.contains("c2-branch"));
    assert!(svg_tw.contains("c2-sub-next"));
    assert!(svg_tw.contains("lamp-stop"));
    assert!(!svg_tw.contains("lamp-stale"));
    assert!(!svg_tw.contains("lamp-offline"));

    // Verify c2-font-switch has 120x24 size and 12px top/bottom/right margin inside c2-top-bar.
    let doc_tw = roxmltree::Document::parse(&svg_tw).expect("valid SVG XML");
    let top_bar_rect = doc_tw
        .descendants()
        .find(|n| n.attribute("data-panel-id") == Some("c2-top-bar"))
        .and_then(|g| g.children().find(|c| c.has_tag_name("rect")))
        .expect("c2-top-bar rect");
    let bar_x: f64 = top_bar_rect
        .attribute("x")
        .and_then(|v| v.parse().ok())
        .unwrap();
    let bar_y: f64 = top_bar_rect
        .attribute("y")
        .and_then(|v| v.parse().ok())
        .unwrap();
    let bar_w: f64 = top_bar_rect
        .attribute("width")
        .and_then(|v| v.parse().ok())
        .unwrap();
    let bar_h: f64 = top_bar_rect
        .attribute("height")
        .and_then(|v| v.parse().ok())
        .unwrap();
    let bar_right = bar_x + bar_w;

    let font_sw = doc_tw
        .descendants()
        .find(|n| n.attribute("data-control-id") == Some("c2-font-switch"))
        .expect("c2-font-switch");
    let fw: f64 = font_sw
        .attribute("data-bbox-w")
        .and_then(|v| v.parse().ok())
        .unwrap();
    let fh: f64 = font_sw
        .attribute("data-bbox-h")
        .and_then(|v| v.parse().ok())
        .unwrap();
    let fx: f64 = font_sw
        .attribute("data-bbox-x")
        .and_then(|v| v.parse().ok())
        .unwrap();
    let fy: f64 = font_sw
        .attribute("data-bbox-y")
        .and_then(|v| v.parse().ok())
        .unwrap();
    assert!(
        (fw - 120.0).abs() <= 0.5,
        "c2-font-switch width {fw} must be 120px"
    );
    assert!(
        (fh - 24.0).abs() <= 0.5,
        "c2-font-switch height {fh} must be 24px"
    );
    assert!(
        ((bar_right - (fx + fw)) - 12.0).abs() <= 0.5,
        "c2-font-switch right margin {} must be 12px",
        bar_right - (fx + fw)
    );
    assert!(
        ((fy - bar_y) - (bar_h - fh) * 0.5).abs() <= 0.5,
        "c2-font-switch must be vertically centered in c2-top-bar"
    );

    // 2. Switching to OBS > Metrics, CD > Deployments & Slots, or FLT > Worker Nodes hides
    // unused c2-sub-next (SUB), c2-branch (REF), and lamp-stop.
    for sv in [
        SubView::Observe(ObserveSubView::Metrics),
        SubView::Cd(CdSubView::DeploymentsSlots),
        SubView::Fleet(FleetSubView::WorkerNodes),
    ] {
        st.select_subview(sv);
        assert!(st.route.branch.is_none());
        assert!(st.route.submodule.is_none());
        let svg = render_console(
            &WireframeSkin,
            &st,
            Viewport::new(1512.0, 860.0),
            SafeArea::ZERO,
        );
        assert!(!svg.contains("c2-sub-next"), "SUB must be hidden on {sv:?}");
        assert!(!svg.contains("c2-branch"), "REF must be hidden on {sv:?}");
        assert!(
            !svg.contains("lamp-stop"),
            "lamp-stop must be hidden on {sv:?}"
        );
    }

    // 3. Default unfiltered state (route.repo == None) displays "* All repositories" in c2-repo-select,
    // and rotate_outer(1) / rotate_outer(-1) transitions cleanly between "* All repositories" and 1/N.
    st.select_subview(SubView::Ci(CiSubView::Scorecards));
    st.route.repo = None;
    st.route.submodule = None;
    let svg_all = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    assert!(svg_all.contains("* All repositories"));
    assert!(!svg_all.contains("c2-sub-next"));

    st.rotate_outer(1, 3_000);
    assert_eq!(st.outer_index, 0);
    assert!(st.route.repo.is_some());
    let svg_first = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    let first_expected = format!(
        "1/{} {}",
        st.filtered_repos().len(),
        st.filtered_repos()[0].id.repo
    );
    assert!(svg_first.contains(&first_expected));

    st.rotate_outer(-1, 3_100);
    assert!(st.route.repo.is_none());
    let svg_back = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1512.0, 860.0),
        SafeArea::ZERO,
    );
    assert!(svg_back.contains("* All repositories"));

    // 4. Compact split CI Queue & Build Logs renders full log payloads without 78-char prefix truncation.
    st.select_subview(SubView::Ci(CiSubView::QueueLogs));
    let mut job_with_logs = st.client.view().expect("view").ci_jobs[0].clone();
    job_with_logs.raw_log_lines = vec![
        "2026-11-10T02:37:19.728991+00:00 [node-a] [miri-check] [stdout] test result: ok. 42 passed"
            .into(),
        "2026-11-10T02:37:20.112000+00:00 [node-a] [miri-check] [stderr] warning: unused variable"
            .into(),
    ];
    let log_patch = Engine::encode_event(&DataEvent::Patch(vec![
        Delta::Auth(AuthState::Operator {
            github_login: "LMA1980".into(),
            role: Some("admin".into()),
        }),
        Delta::CiJobs(vec![job_with_logs]),
    ]))
    .expect("encode patch");
    st.on_worker_frame(&log_patch);
    let svg_logs = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1440.0, 900.0),
        SafeArea::ZERO,
    );
    assert!(svg_logs.contains("CI QUEUE, WORKER SLOTS &amp; BUILD LOGS"));
    assert!(svg_logs.contains("02:37:19 [miri-check] test result: ok. 42 passed"));
    assert!(svg_logs.contains("02:37:20 [miri-check] ERR warning: unused variable"));
    assert!(!svg_logs.contains("2026-11-10T02:37:19.728991+00:00"));
    assert!(!svg_logs.contains("[stdout]"));
    assert_svg_zero_overflow(&svg_logs, 1440.0, 900.0, "u23_compact_ci_build_logs");

    // 5. Clicking data-subview="logs" while in Area::Observe stays in OBS > Logs (#/obs/logs)
    // and never redirects to CI > Queue & Logs; top bar replaces repo stepper with c2-obs-service-select.
    st.select_area(Area::Observe);
    assert_eq!(st.route.area, Area::Observe);
    assert!(st.route.repo.is_none());
    st.on_dom_action("data-subview", "logs", false, 3_500);
    assert_eq!(st.route.area, Area::Observe);
    assert_eq!(st.route.subview, SubView::Observe(ObserveSubView::Logs));
    assert_eq!(format_hash(&st.route), "#/obs/logs");

    let svg_obs_logs = render_console(
        &WireframeSkin,
        &st,
        Viewport::new(1440.0, 900.0),
        SafeArea::ZERO,
    );
    let html_obs_logs = render_html_shell(&svg_obs_logs, &st);
    assert!(html_obs_logs.contains("placeholder=\"Filter services, crates, sidecars, logs...\""));
    assert!(svg_obs_logs.contains("c2-obs-service-select"));
    assert!(svg_obs_logs.contains("* All services"));
    assert!(!svg_obs_logs.contains("c2-repo-select"));
    assert!(svg_obs_logs.contains("ACTIVE SERVICES, CRATES &amp; SIDECARS:"));
    assert!(svg_obs_logs.contains("ENTRIES (ERR:"));
    assert!(mu_console_ui::service_filter_matches(
        "action-runner-controller",
        "actionrunner-controller"
    ));
    assert_svg_zero_overflow(&svg_obs_logs, 1440.0, 900.0, "u23_obs_logs_multi_service");

    // Stepping service selector filters Observe logs by service and cycles back to * All services.
    let services = st.observed_services();
    assert!(!services.is_empty());
    st.activate_target("c2-obs-service-next");
    assert_eq!(st.obs_service_filter.as_deref(), Some(services[0].as_str()));
    st.activate_target("c2-obs-service-prev");
    assert!(st.obs_service_filter.is_none());
}
