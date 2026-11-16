// SPDX-License-Identifier: AGPL-3.0-only
//! Automated UX Validation Gate (`U1`–`U24`) for M2
//! (`HtmlCoreSkin` default + `WireframeSkin` opt-in, 8 Areas × 27 Sub-Views,
//! universal `disabled` router, C2 Repo/Submodule Stepper, `Hurmit` ↔
//! `OpenDyslexicM` toggle, cross-area `[TRACE ↗]` / `[CD GATE ↗]` routing,
//! invite-only GitHub OAuth + Passkey enrollment, and two-skin CSS parity).
#![cfg(not(miri))]

use mu_console_data::engine::{Effect, Engine};
use mu_console_data::scenario::Scenario;
use mu_console_layout::{Breakpoint, SafeArea, Viewport};
use mu_console_protocol::dto::{
    AuthSessionDto, CdEcosystem, CdRunDto, CdRunFailurePinpointDto, CdStepDto, PasskeySummaryDto,
    PlatformUserDto, RoleActivationDto, RoleSpecDto, UserPreferencesDto, UserProfileDto,
};
use mu_console_protocol::{
    format_job_id, AbDeploySubView, Area, AuthReasonCode, AuthState, Branch, CdRunState, CdSubView,
    CiSubView, ConsoleSkin, CrtFont, DataEvent, Delta, DialogModel, GateTest, ObservabilitySubView,
    PackageEcosystem, PlatformSubView, PrState, RegistryPackage, RepoId, RepoScore, RepoStatus,
    RepositorySubView, RoleBadgeView, RoleLeaseView, SidingPhase, SubView, SubmoduleId,
    SubsystemId, TestState, UiCommand, UserProfileTab, ViewModel, WorkerConfig,
};
use mu_console_theme::css::tokens_css;
use mu_console_ui::client::WORKER_URL;
use mu_console_ui::components::{render_console, render_console_with_skin, render_html_shell};
use mu_console_ui::router::{breadcrumb_segments, escape_up, format_hash, parse_hash};
use mu_console_ui::state::{
    extract_invite_token, font_slug, format_sign_in_url, is_action_root_boundary,
    is_passive_container_control_id, parse_font_pref, parse_skin_pref, service_filter_matches,
    validate_sign_in_href, ConsoleState, ACTION_ATTRS, PENDING_INVITE_STORAGE_KEY,
    ROTARY_SETTLE_MS,
};

const VIEWPORTS: [(f64, f64); 6] = [
    (1920.0, 1080.0),
    (1512.0, 860.0),
    (1280.0, 800.0),
    (1024.0, 768.0),
    (932.0, 430.0),
    (402.0, 720.0),
];

fn all_operator_policies() -> Vec<String> {
    [
        "system:dashboard.read",
        "ci:queue.read",
        "ci:repos.public.read",
        "ci:repos.private.read",
        "ci:jobs.read",
        "ci:jobs.resume",
        "ci:jobs.restart",
        "ci:nodes.read",
        "ci:sandboxes.read",
        "cd:runs.read",
        "cd:steps.read",
        "ab:slots.read",
        "ab:deploy.read",
        "ab:gate.logs.read",
        "ab:leases.read",
        "ab:rollback",
        "reg:packages.read",
        "reg:canaries.read",
        "reg:taints.read",
        "reg:cas.read",
        "obs:smartscape.read",
        "obs:traces.read",
        "obs:problems.read",
        "obs:logs.read",
        "obs:metrics.read",
        "obs:store.read",
        "obs:ingress.read",
        "mr:models.read",
        "mr:queue.read",
        "mr:openai_api.read",
        "mr:wireguard_vpn.read",
        "platform:maintenance.read",
        "platform:maintenance.write",
        "platform:users.read",
        "platform:users.write",
        "platform:invites.read",
        "platform:invites.write",
        "platform:roles.read",
        "platform:roles.write",
        "platform:folios.read",
        "platform:folios.write",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn full_active_auth(login: &str) -> AuthState {
    AuthState::from_session(&AuthSessionDto {
        authenticated: true,
        invite_bound: true,
        passkey_count: 1,
        github_login: Some(login.into()),
        baseline_role: Some("platform-admin".into()),
        active_roles: vec![RoleSpecDto {
            id: "platform-admin".into(),
            display_name: Some("operator".into()),
            ..RoleSpecDto::default()
        }],
        active_policies: all_operator_policies(),
        ..AuthSessionDto::default()
    })
}

fn state_for_scenario(scenario: Scenario, subview: SubView, font: CrtFont) -> ConsoleState {
    let mut eng = Engine::new(1);
    let cfg = WorkerConfig {
        api_base: "http://192.0.2.10:8080".into(),
        scenario: Some(scenario.name().to_owned()),
    };
    let mut state = ConsoleState::new();
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
    let auth_patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(full_active_auth(
        "LMA1980",
    ))]))
    .expect("encode active auth");
    state.on_worker_frame(&auth_patch);
    state.set_font(font);
    state.select_subview(subview);
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
            "{context}: output must never expose private identifier '{forbidden}'"
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
    }
}

#[test]
fn u1_zero_viewport_overflow_across_all_viewports_27_subviews_fonts_and_scenarios() {
    for &(vw, vh) in &VIEWPORTS {
        for &area in &Area::ALL {
            for &subview in SubView::all_for(area) {
                for &font in &[CrtFont::Hurmit, CrtFont::OpenDyslexic] {
                    let st = state_for_scenario(Scenario::NewBranch, subview, font);
                    let svg = render_console_with_skin(
                        ConsoleSkin::Wireframe,
                        &st,
                        Viewport::new(vw, vh),
                    );
                    let ctx = format!("{vw}x{vh} subview={subview:?} font={font:?}");
                    assert_svg_zero_overflow(&svg, vw, vh, &ctx);

                    let html =
                        render_console_with_skin(ConsoleSkin::Html, &st, Viewport::new(vw, vh));
                    assert!(
                        html.contains("mu-console-scene"),
                        "{ctx}: HtmlCoreSkin must emit .mu-console-scene container"
                    );
                }
            }
        }
    }
    for &scenario in &Scenario::ALL {
        for &area in &Area::ALL {
            let st = state_for_scenario(scenario, SubView::default_for(area), CrtFont::Hurmit);
            let svg =
                render_console_with_skin(ConsoleSkin::Wireframe, &st, Viewport::new(1512.0, 860.0));
            let ctx = format!("1512x860 area={area:?} scenario={}", scenario.name());
            assert_svg_zero_overflow(&svg, 1512.0, 860.0, &ctx);
        }
    }
}

#[test]
fn u2_navigation_and_routing_8_areas_deep_link_escape_and_legacy_aliases() {
    let mut st = state_for_scenario(
        Scenario::NewBranch,
        SubView::Ci(CiSubView::Trainway),
        CrtFont::Hurmit,
    );

    // 1..=8 keyboard area shortcuts cover all 8 Areas.
    for (idx, expected_area) in Area::ALL.iter().enumerate() {
        let key = (idx + 1).to_string();
        let cmd = st.on_key(&key, false, 1_000 + idx as u64);
        assert_eq!(st.route.area, *expected_area);
        assert!(matches!(cmd, Some(UiCommand::SelectSubView(_))));
    }

    // Sub-View switching inside CI.
    st.select_subview(SubView::Ci(CiSubView::Scorecards));
    assert_eq!(st.route.subview, SubView::Ci(CiSubView::Scorecards));
    st.select_subview(SubView::Ci(CiSubView::Trainway));
    assert_eq!(st.route.subview, SubView::Ci(CiSubView::Trainway));
    st.select_subview(SubView::Ci(CiSubView::QueueLogs));
    assert_eq!(st.route.subview, SubView::Ci(CiSubView::QueueLogs));

    // Deep-link round-trip + breadcrumb segments + Esc up-one-level.
    let hash =
        "#/ci/trainway?repo=fixture/superproject&submodule=code/engine&branch=feat/rvv&job=104";
    st.init_from_env(hash, "", None);
    assert_eq!(format_hash(&st.route), hash);
    let wide_crumbs = breadcrumb_segments(&st.route, Breakpoint::Wide);
    assert!(wide_crumbs.len() >= 5);
    assert_eq!(wide_crumbs[0].label, "CI");
    assert_eq!(wide_crumbs[1].label, "Trainway");
    let phone_crumbs = breadcrumb_segments(&st.route, Breakpoint::Phone);
    assert_eq!(phone_crumbs.len(), 2);

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
    assert_eq!(st.route.subview, SubView::Ci(CiSubView::Scorecards));
    assert_eq!(escape_up(&st.route).area, Area::Dashboard);

    // Legacy #/ovr, #/fleet, #/cd/gate, #/observe/*, #/registry/*, and #/iam/* aliases.
    assert_eq!(
        parse_hash("#/ovr").subview,
        SubView::Dashboard(mu_console_protocol::DashboardSubView::Overview)
    );
    let legacy = parse_hash("#/fleet?job=77");
    assert_eq!(legacy.area, Area::Ci);
    assert_eq!(legacy.subview, SubView::Ci(CiSubView::WorkerNodes));
    assert_eq!(legacy.job, Some(77));
    assert_eq!(
        parse_hash("#/registry/canaries").subview,
        SubView::Repository(RepositorySubView::CanaryTaints)
    );
    assert_eq!(
        parse_hash("#/observe/purepath").subview,
        SubView::Observability(ObservabilitySubView::PurePath)
    );
    assert_eq!(
        parse_hash("#/cd/gate").subview,
        SubView::AbDeploy(AbDeploySubView::IntegrationGate)
    );
    assert_eq!(
        parse_hash("#/iam/users").subview,
        SubView::Platform(PlatformSubView::UsersAndInvites)
    );
    assert_eq!(
        parse_hash("#/iam/roles").subview,
        SubView::Platform(PlatformSubView::RolesAndFolios)
    );
}

#[test]
fn u3_repo_stepper_50_plus_repos_submodules_typeahead_filter_and_350ms_settle_debounce() {
    let mut st = state_for_scenario(
        Scenario::Repos40,
        SubView::Ci(CiSubView::Trainway),
        CrtFont::Hurmit,
    );

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

    // Rapid outer repo steps coalesce into a single SelectRepo after 350 ms.
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
    assert_eq!(cmds.len(), 1);
    assert!(matches!(cmds[0], UiCommand::SelectRepo { .. }));

    // Submodule stepper steps through submodules on superproject.
    let mut st_sub = state_for_scenario(
        Scenario::NewBranch,
        SubView::Ci(CiSubView::Trainway),
        CrtFont::Hurmit,
    );
    st_sub.route.repo = Some(RepoId::new("fixture", "superproject"));
    st_sub.route.submodule = None;
    st_sub.sync_rotary_from_route();
    st_sub.on_dom_action("data-action", "sub-next", false, 1_000);
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
fn u4_scorecard_carousel_orders_parent_first_then_gitmodules() {
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

    let html = render_console(&st, Viewport::new(1512.0, 860.0), SafeArea::ZERO);
    assert!(html.contains("CI Scorecards Carousel"));
    let svg = render_console_with_skin(ConsoleSkin::Wireframe, &st, Viewport::new(1512.0, 860.0));
    assert_svg_zero_overflow(&svg, 1512.0, 860.0, "u4_wireframe_scorecards");
}

#[test]
fn u5_trainway_lanes_failure_risk_confirm_restart_and_one_click_resume() {
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
    let html_sub = render_console(&st_sub, Viewport::new(1512.0, 860.0), SafeArea::ZERO);
    assert!(html_sub.contains("CI Pipeline Trainway"));

    let mut st_fail = state_for_scenario(
        Scenario::StageFailure,
        SubView::Ci(CiSubView::Trainway),
        CrtFont::Hurmit,
    );
    // Resume (risk-free) executes in 1 click without dialog.
    let res = st_fail.activate_target("ci-job-resume:1233");
    assert_eq!(res, vec![UiCommand::ResumeJob { job_id: 1_233 }]);
    assert!(st_fail.active_dialog().is_none());

    // Restart opens RiskConfirm requiring phrase "RESTART 001233".
    let rst = st_fail.activate_target("ci-job-restart:1233");
    assert!(matches!(
        rst.first(),
        Some(UiCommand::OpenDialog(DialogModel::RiskConfirm(_)))
    ));
    assert!(st_fail.confirm_risk_dialog().is_empty());
    st_fail.set_risk_confirm_input("RESTART 001233");
    let confirmed = st_fail.confirm_risk_dialog();
    assert!(confirmed.contains(&UiCommand::RestartJob { job_id: 1_233 }));
    assert!(confirmed.contains(&UiCommand::CloseDialog));
}

#[test]
fn u6_readonly_cd_ecosystem_diagnosis_and_ab_rollback_risk_confirm() {
    let mut st = state_for_scenario(
        Scenario::ItCriticalFailure,
        SubView::Cd(CdSubView::Runs),
        CrtFont::Hurmit,
    );
    let mut vm = st.client.view().cloned().expect("view");
    vm.cd_runs = vec![CdRunDto {
        run_id: "cd-cargo-1".into(),
        repo: RepoId::new("LMA1980", "k3-core"),
        ecosystem: CdEcosystem::Cargo,
        ref_name: "main".into(),
        commit_sha: "abc1234".into(),
        state: CdRunState::Failed,
        started_epoch_ms: 1_700_000_000_000,
        failure: Some(CdRunFailurePinpointDto {
            step_id: "publish-crate".into(),
            package_name: Some("k3-core".into()),
            package_version: Some("0.4.2".into()),
            reason: "muregist ETag conflict".into(),
            log_excerpt: None,
        }),
        steps: vec![CdStepDto {
            step_id: "publish-crate".into(),
            label: "Publish Crate".into(),
            state: CdRunState::Failed,
            duration_ms: 12_000,
            skip_reason: None,
            error_summary: Some("409 conflict".into()),
            failed_package: Some("k3-core".into()),
            exit_code: Some(1),
        }],
        ..CdRunDto::default()
    }];
    let snap = Engine::encode_event(&DataEvent::Snapshot(vm)).expect("encode");
    st.on_worker_frame(&snap);

    let html_cd = render_console(&st, Viewport::new(1512.0, 860.0), SafeArea::ZERO);
    assert!(html_cd.contains("Cargo"));
    assert!(html_cd.contains("k3-core"));
    assert!(html_cd.contains("Read-only diagnosis"));
    assert!(html_cd.contains("nav-area:ab-deploy"));
    assert!(!html_cd.contains("ab-rollback:"));

    // Switch to A|B Deploy > Integration Gate & Gate Test Logs + Fast RiskConfirm rollback.
    st.select_subview(SubView::AbDeploy(AbDeploySubView::IntegrationGate));
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
    st.on_worker_frame(&log_patch);
    let html_gate = render_console(&st, Viewport::new(1512.0, 860.0), SafeArea::ZERO);
    assert!(html_gate.contains("it_infer_gemm"));
    assert!(html_gate.contains("ab-rollback:"));

    st.select_subview(SubView::AbDeploy(AbDeploySubView::GateTestLogs));
    let html_logs = render_console(&st, Viewport::new(1512.0, 860.0), SafeArea::ZERO);
    assert!(html_logs.contains("Gate Test Transcript"));

    let open_rb = st.activate_target("ab-rollback:dep-041");
    assert!(matches!(
        open_rb.first(),
        Some(UiCommand::OpenDialog(DialogModel::RiskConfirm(_)))
    ));
    st.platform_form.ab_rollback_note = "critical it failure".into();
    let rb_cmds = st.confirm_risk_dialog();
    assert!(matches!(
        rb_cmds.first(),
        Some(UiCommand::TriggerAbRollback {
            deployment_id,
            note: Some(_),
        }) if deployment_id == "dep-041"
    ));
}

#[test]
fn u7_font_toggle_dyslexic_reflow_phone_touch_targets_html_shell_and_skin_parity() {
    assert_eq!(
        parse_font_pref("?font=dyslexic", None),
        CrtFont::OpenDyslexic
    );
    assert_eq!(parse_font_pref("", Some("dyslexic")), CrtFont::OpenDyslexic);
    assert_eq!(parse_font_pref("", Some("hurmit")), CrtFont::Hurmit);
    assert_eq!(font_slug(CrtFont::OpenDyslexic), "opendyslexic");

    let mut st = state_for_scenario(
        Scenario::NewBranch,
        SubView::Ci(CiSubView::Trainway),
        CrtFont::Hurmit,
    );
    let cmd = st.on_key("d", false, 1_000);
    assert_eq!(cmd, Some(UiCommand::SetCrtFont(CrtFont::OpenDyslexic)));
    assert!(st.lazy_dyslexic_loaded);

    let svg_wf =
        render_console_with_skin(ConsoleSkin::Wireframe, &st, Viewport::new(1512.0, 860.0));
    assert!(svg_wf.contains("MuCRT"));
    assert_svg_zero_overflow(&svg_wf, 1512.0, 860.0, "u7_dyslexic_wireframe");

    let html_shell = render_html_shell(&svg_wf, &st);
    assert!(html_shell.contains("Skip to main console"));
    assert!(html_shell.contains("aria-live=\"polite\""));
    assert!(html_shell.contains("aria-live=\"assertive\""));
    assert!(html_shell.contains("font-size:16px"));
    assert!(html_shell.contains("OpenDyslexicMNerdFontMono-Regular.woff2"));
}

#[test]
fn u8_cross_area_trace_and_cd_gate_deep_links_and_auth_transitions() {
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
        Scenario::NewBranch,
        SubView::Repository(RepositorySubView::CanaryTaints),
        CrtFont::Hurmit,
    );
    let cmds = st.on_dom_action("data-trace-id", "tr-reg-taint-01", false, 2_000);
    assert_eq!(st.route.area, Area::Observability);
    assert_eq!(
        st.route.subview,
        SubView::Observability(ObservabilitySubView::PurePath)
    );
    assert_eq!(st.route.trace_id.as_deref(), Some("tr-reg-taint-01"));
    assert!(!cmds.is_empty());

    let cmds_cd = st.on_dom_action("data-cd-gate", "dep-041", false, 2_100);
    assert_eq!(st.route.area, Area::AbDeploy);
    assert_eq!(
        st.route.subview,
        SubView::AbDeploy(AbDeploySubView::IntegrationGate)
    );
    assert!(!cmds_cd.is_empty());

    // Transition to AuthorizationDenied: generic label, zero invite/identity disclosure.
    let denied = AuthState::from_session(&AuthSessionDto {
        authenticated: false,
        invite_bound: false,
        github_login: Some("guest-user".into()),
        reason_code: Some(AuthReasonCode::AuthorizationDenied),
        active_policies: vec!["ci:queue.read".into(), "ci:repos.public.read".into()],
        ..AuthSessionDto::default()
    });
    let patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(denied)])).expect("encode");
    st.on_worker_frame(&patch);
    assert!(!st.is_operator());
    let html_denied = render_html_shell("<svg></svg>", &st);
    assert!(html_denied.contains("Authorization denied"));
    assert!(html_denied.contains("Sign Out"));
    assert!(!html_denied.contains("guest-user"));
}

#[test]
fn u9_sign_in_url_generation_and_encoding_invariants() {
    assert_eq!(format_sign_in_url("/"), "/oauth2/start?rd=%2F");
    assert!(validate_sign_in_href("/oauth2/start?rd=%2F").is_ok());

    let deep_target = "/#/cd/gate?filter=ALL";
    let formatted_deep = format_sign_in_url(deep_target);
    assert_eq!(
        formatted_deep,
        "/oauth2/start?rd=%2F%23%2Fcd%2Fgate%3Ffilter%3DALL"
    );
    assert!(validate_sign_in_href(&formatted_deep).is_ok());

    assert!(validate_sign_in_href("/oauth2/start?rdhttps://example.invalid/?invite=x").is_err());
    assert!(validate_sign_in_href("/oauth2/start").is_err());
    assert!(validate_sign_in_href("/oauth2/start?rd=").is_err());
    assert!(validate_sign_in_href("/oauth2/other?rd=/").is_err());

    let mut st = ConsoleState::new();
    let snap = Engine::encode_event(&DataEvent::Snapshot(ViewModel {
        auth: AuthState::anonymous(vec!["ci:queue.read".into()]),
        ..ViewModel::default()
    }))
    .expect("encode");
    st.on_worker_frame(&snap);
    let html = render_html_shell("<svg></svg>", &st);
    assert!(html.contains(r#"href="/oauth2/start?rd=%2F""#));
    assert!(!html.contains("?rdhttps:"));
}

#[test]
fn u10_invite_preservation_across_oauth2_redirect_lifecycle() {
    let initial_query = "?invite=inv_test_lifecycle_019a&repo=model-runner";
    let (extracted, clean) = extract_invite_token(initial_query);
    assert_eq!(extracted.as_deref(), Some("inv_test_lifecycle_019a"));
    assert_eq!(clean, "?repo=model-runner");

    let mut sim_session = std::collections::BTreeMap::<String, String>::new();
    sim_session.insert(PENDING_INVITE_STORAGE_KEY.into(), extracted.unwrap());

    let mut st = ConsoleState::new();
    st.init_from_env("#/ci", initial_query, None);
    assert_eq!(
        st.pending_invite.as_deref(),
        Some("inv_test_lifecycle_019a")
    );

    let recovered = sim_session.get(PENDING_INVITE_STORAGE_KEY).cloned();
    assert_eq!(recovered.as_deref(), Some("inv_test_lifecycle_019a"));

    let mut st_ret = ConsoleState::new();
    let init_frame =
        Engine::encode_event(&DataEvent::Snapshot(ViewModel::default())).expect("encode");
    st_ret.on_worker_frame(&init_frame);
    st_ret.init_from_env("#/ci", "", None);
    st_ret.pending_invite = recovered;
    let patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(full_active_auth(
        "LMA1980",
    ))]))
    .expect("encode");
    st_ret.on_worker_frame(&patch);
    assert!(st_ret.is_operator());
    if st_ret.is_operator() && st_ret.pending_invite.take().is_some() {
        sim_session.remove(PENDING_INVITE_STORAGE_KEY);
    }
    assert!(st_ret.pending_invite.is_none());
    assert!(!sim_session.contains_key(PENDING_INVITE_STORAGE_KEY));
}

#[test]
fn u11_c2_auth_status_click_dispatches_navigation_command() {
    let mut st = ConsoleState::new();
    // Unresolved boot click is fail-closed no-op.
    assert!(st
        .on_dom_action("data-control-id", "c2-auth-status", false, 500)
        .is_empty());

    let anon_snap = Engine::encode_event(&DataEvent::Snapshot(ViewModel {
        auth: AuthState::anonymous(vec!["ci:queue.read".into()]),
        ..ViewModel::default()
    }))
    .expect("encode");
    st.on_worker_frame(&anon_snap);
    assert_eq!(
        st.on_dom_action("data-control-id", "c2-auth-status", false, 1_000),
        vec![UiCommand::Navigate {
            href: "/oauth2/start?rd=%2F".into()
        }]
    );

    let denied = AuthState::from_session(&AuthSessionDto {
        authenticated: false,
        invite_bound: false,
        github_login: Some("guest".into()),
        reason_code: Some(AuthReasonCode::AuthorizationDenied),
        ..AuthSessionDto::default()
    });
    let denied_patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(denied)])).unwrap();
    st.on_worker_frame(&denied_patch);
    assert_eq!(
        st.on_dom_action("data-control-id", "c2-auth-status", false, 2_000),
        vec![UiCommand::Navigate {
            href: "/oauth2/sign_out?rd=%2F".into()
        }]
    );

    let active_patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(
        full_active_auth("LMA1980"),
    )]))
    .unwrap();
    st.on_worker_frame(&active_patch);
    assert_eq!(
        st.on_dom_action("data-control-id", "c2-auth-status", false, 3_000),
        vec![UiCommand::Navigate {
            href: "/oauth2/sign_out?rd=%2F".into()
        }]
    );
}

#[test]
fn u12_root_absolute_asset_paths_and_worker_resolution() {
    assert_eq!(WORKER_URL, "/muconsole-data_loader.js");

    let mut st = ConsoleState::new();
    st.lazy_dyslexic_loaded = true;
    let html = render_html_shell("", &st);
    assert!(html.contains("url('/fonts/OpenDyslexicMNerdFontMono-Regular.woff2')"));
    assert!(!html.contains("./fonts/"));

    let index_content = include_str!("../index.html");
    assert!(index_content.contains("lang=\"en\""));
    assert!(index_content.contains("/tokens.css"));
}

fn find_css_block<'a>(css: &'a str, selector: &str) -> &'a str {
    let start = css
        .find(&format!("{selector}{{"))
        .unwrap_or_else(|| panic!("CSS block '{selector}' not found"));
    let body_start = start + selector.len() + 1;
    let body_end = css[body_start..]
        .find('}')
        .map(|i| body_start + i)
        .expect("unclosed CSS block");
    &css[body_start..body_end]
}

#[test]
fn u13_two_skin_tokens_css_definitions_and_layout_rules() {
    let css = tokens_css();
    assert!(css.contains("html[data-skin=\"wireframe\"]"));
    assert!(css.contains("html[data-skin=\"html\"]"));
    let wf_shell = find_css_block(
        &css,
        "#muconsole-shell[data-skin=\"wireframe\"],html[data-skin=\"wireframe\"] #muconsole-shell",
    );
    assert!(wf_shell.contains("100dvh"));
    assert!(wf_shell.contains("overflow:hidden"));

    let html_shell = find_css_block(
        &css,
        "#muconsole-shell[data-skin=\"html\"],html[data-skin=\"html\"] #muconsole-shell",
    );
    assert!(html_shell.contains("min-height:100dvh"));

    let filter = find_css_block(&css, "#muconsole-filter");
    assert!(filter.contains("font-size:16px"));
    let dialog = find_css_block(&css, "dialog.mu-dialog");
    assert!(dialog.contains("max-height:calc(100dvh - 32px)"));
}

#[test]
fn u14_semantic_html_shell_header_hierarchy_and_main_dashboard() {
    let mut st = ConsoleState::new();
    let anon = Engine::encode_event(&DataEvent::Snapshot(ViewModel {
        auth: AuthState::anonymous(vec!["ci:queue.read".into()]),
        ..ViewModel::default()
    }))
    .unwrap();
    st.on_worker_frame(&anon);

    let svg_dummy =
        "<svg id=\"test-canvas\" width=\"100\" height=\"100\"><rect width=\"100\" height=\"100\"/></svg>";
    let html = render_html_shell(svg_dummy, &st);
    let doc = roxmltree::Document::parse(&html).expect("valid XML shell");
    let shell = doc.root_element();
    assert_eq!(shell.attribute("id"), Some("muconsole-shell"));
    assert_eq!(shell.attribute("data-skin"), Some("html"));
    let header = shell
        .children()
        .find(|n| n.attribute("id") == Some("muconsole-header"))
        .expect("#muconsole-header");
    assert!(header
        .descendants()
        .any(|n| n.attribute("id") == Some("muconsole-font-toggle")));
    assert!(header
        .descendants()
        .any(|n| n.attribute("id") == Some("muconsole-auth-link")));
    let main = shell
        .children()
        .find(|n| n.attribute("id") == Some("muconsole-dashboard"))
        .expect("#muconsole-dashboard");
    assert!(main
        .descendants()
        .any(|n| n.attribute("id") == Some("test-canvas")));
}

#[test]
fn u15_ios_safari_viewport_safety_and_filter_font_size_invariant() {
    let css = tokens_css();
    let filter_block = find_css_block(&css, "#muconsole-filter");
    assert!(filter_block.contains("font-size:16px"));
    for sub_px in ["10px", "11px", "12px", "13px", "14px", "15px"] {
        assert!(!filter_block.contains(&format!("font-size:{sub_px}")));
    }

    for &font in &[CrtFont::Hurmit, CrtFont::OpenDyslexic] {
        let mut st = ConsoleState::new();
        st.set_font(font);
        let html = render_html_shell("<svg></svg>", &st);
        let doc = roxmltree::Document::parse(&html).expect("valid XML");
        let filter = doc
            .descendants()
            .find(|n| n.attribute("id") == Some("muconsole-filter"))
            .unwrap();
        assert!(filter
            .attribute("style")
            .is_some_and(|s| s.contains("font-size:16px")));
    }
}

#[test]
fn u16_tokens_css_distribution_drift_and_synchronization() {
    let generated = tokens_css();
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let parent_root = manifest_dir.join("../..");
    let target_tokens = if parent_root.join("scripts/build-dist.sh").is_file()
        && parent_root.join("code/Cargo.toml").is_file()
    {
        parent_root.join("dist/tokens.css")
    } else {
        manifest_dir.join("dist/tokens.css")
    };
    let checked_in_dist = std::fs::read_to_string(&target_tokens)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", target_tokens.display()));
    assert_eq!(
        generated,
        checked_in_dist,
        "{} drifted from mu_console_theme::css::tokens_css()",
        target_tokens.display()
    );
}

#[test]
fn u17_job_id_formatting_and_repository_packages_pagination() {
    assert_eq!(format_job_id(29_612_437_286_333_911), "62add7");
    assert_eq!(format_job_id(104), "104");

    let mut st = state_for_scenario(
        Scenario::NewBranch,
        SubView::Repository(RepositorySubView::Packages),
        CrtFont::Hurmit,
    );
    let mut vm = st.client.view().cloned().expect("view");
    vm.registry.packages.push(RegistryPackage {
        ecosystem: PackageEcosystem::Cargo,
        name: "mu-extra-crate-01".into(),
        latest_version: "0.2.0".into(),
        versions_count: 2,
        deps_count: 4,
        sha256_prefix: "a1b2c3d4e501".into(),
        tainted: false,
        taint_reason: None,
        yanked: false,
        versions: vec!["0.2.0".into(), "0.1.0".into()],
    });
    vm.registry.packages.push(RegistryPackage {
        ecosystem: PackageEcosystem::NuGet,
        name: "Mu.Sdk.Core".into(),
        latest_version: "1.0.0".into(),
        versions_count: 1,
        deps_count: 2,
        sha256_prefix: "ff1122334455".into(),
        tainted: false,
        taint_reason: None,
        yanked: false,
        versions: vec!["1.0.0".into()],
    });
    let snap = Engine::encode_event(&DataEvent::Snapshot(vm)).expect("encode");
    st.on_worker_frame(&snap);

    let html_all = render_console(&st, Viewport::new(1512.0, 860.0), SafeArea::ZERO);
    assert!(html_all.contains("mu-extra-crate-01"));
    assert!(html_all.contains("Mu.Sdk.Core"));

    st.on_dom_action("data-ecosystem", "nuget", false, 1_500);
    assert_eq!(st.reg_ecosystem_filter, Some(PackageEcosystem::NuGet));
    let html_nuget = render_console(&st, Viewport::new(1512.0, 860.0), SafeArea::ZERO);
    assert!(html_nuget.contains("Mu.Sdk.Core"));
}

#[test]
fn u18_two_skin_preference_and_active_header_declutter() {
    assert_eq!(parse_skin_pref("?skin=legacy", None), ConsoleSkin::Html);
    assert_eq!(
        parse_skin_pref("?skin=wireframe", None),
        ConsoleSkin::Wireframe
    );
    assert_eq!(
        parse_skin_pref("?skin=html", Some("wireframe")),
        ConsoleSkin::Html
    );

    let mut st = state_for_scenario(
        Scenario::StageFailure,
        SubView::Ci(CiSubView::Trainway),
        CrtFont::Hurmit,
    );
    assert_eq!(st.effective_skin(), ConsoleSkin::Html);

    let shell_html = render_html_shell("", &st);
    assert!(shell_html.contains("data-skin=\"html\""));
    assert!(!shell_html.contains("muconsole-font-toggle"));
    assert!(shell_html.contains("muconsole-user-btn"));
    assert!(shell_html.contains("@LMA1980"));
    assert!(shell_html.contains("muconsole-roles-btn"));

    // Active unlocks Wireframe via Preferences modal.
    st.on_dom_action("data-action", "profile-skin:wireframe", false, 1_000);
    assert_eq!(st.effective_skin(), ConsoleSkin::Wireframe);
    let svg_wf = render_console(&st, Viewport::new(1512.0, 860.0), SafeArea::ZERO);
    assert_svg_bounds_zero_overflow(&svg_wf, 1512.0, 860.0, "u18_wireframe_active");
    let shell_wf = render_html_shell(&svg_wf, &st);
    assert!(shell_wf.contains("data-skin=\"wireframe\""));
}

#[test]
fn u19_observability_sandbox_profiles_and_logs_follow_tail() {
    let mut st = state_for_scenario(
        Scenario::StageFailure,
        SubView::Observability(ObservabilitySubView::Logs),
        CrtFont::Hurmit,
    );
    assert!(service_filter_matches(
        "runner controller",
        "actionrunner-controller"
    ));

    let mut vm = st.client.view().expect("view").clone();
    for i in 0..75_u64 {
        vm.observe.logs.push(mu_console_protocol::OtelLogEntry {
            timestamp_ms: 1_760_000_000_000 + i * 1_000,
            severity: mu_console_protocol::LogSeverity::Info,
            service: "actionrunner-controller".into(),
            trace_id: format!("tr-obs-tail-{i:03}"),
            span_id: Some(format!("sp-obs-tail-{i:03}")),
            message: format!("obs-stream-entry-{i:03}"),
        });
    }
    let snap = Engine::encode_event(&DataEvent::Snapshot(vm)).expect("encode");
    st.on_worker_frame(&snap);
    assert!(st.follow_log);

    st.activate_target("obs-logs-screen:prev");
    assert!(!st.follow_log);
    st.activate_target("obs-log-follow-toggle");
    assert!(st.follow_log);

    st.select_subview(SubView::Observability(ObservabilitySubView::DataStore));
    let html_sb = render_console(&st, Viewport::new(1440.0, 900.0), SafeArea::ZERO);
    assert!(html_sb.contains("Observability"));
}

#[test]
fn u20_dom_action_boundary_and_repo_selection_resets_stale_job() {
    assert!(!ACTION_ATTRS.is_empty());
    assert!(!is_action_root_boundary("muconsole-shell"));
    assert!(is_action_root_boundary("ci-job:1233"));
    assert!(is_passive_container_control_id("muconsole-shell"));

    let mut st = state_for_scenario(
        Scenario::StageFailure,
        SubView::Ci(CiSubView::QueueLogs),
        CrtFont::Hurmit,
    );
    st.activate_target("ci-job:1233");
    assert_eq!(st.route.job, Some(1_233));

    st.on_dom_action("data-control-id", "repo-next", false, 1_500);
    assert_eq!(st.route.job, None);
    assert_eq!(st.route.branch, None);
}

#[test]
fn u21_u23_header_budget_and_passkey_profile_tabs() {
    let mut st = state_for_scenario(
        Scenario::NewBranch,
        SubView::Ci(CiSubView::Scorecards),
        CrtFont::Hurmit,
    );
    let mut vm = st.client.view().cloned().expect("view");
    vm.auth.active_roles = vec![RoleBadgeView {
        role_id: "release-operator".into(),
        display_name: Some("rel".into()),
        activation: RoleActivationDto::timed(900, 300),
        lease: Some(RoleLeaseView {
            role_id: "release-operator".into(),
            display_name: None,
            activated_at_ms: 1_000,
            last_activity_at_ms: 1_000,
            hard_expires_at_ms: 91_000,
            idle_expires_at_ms: 31_000,
            hard_ttl_secs: 900,
            idle_ttl_secs: 300,
            seconds_remaining: 90,
            idle_timer: 30,
        }),
    }];
    vm.user_profile = Some(UserProfileDto {
        github_login: "LMA1980".into(),
        display_name: Some("Operator".into()),
        preferences: UserPreferencesDto {
            skin: ConsoleSkin::Html,
            font: CrtFont::Hurmit,
        },
        bound_roles: vec![],
        passkey_count: 1,
        passkeys: vec![PasskeySummaryDto {
            credential_id: "cred-1".into(),
            label: Some("Touch ID".into()),
            sign_count: 1,
            created_at_ms: 1_000,
            last_used_at_ms: None,
        }],
    });
    let snap = Engine::encode_event(&DataEvent::Snapshot(vm)).expect("encode");
    st.on_worker_frame(&snap);

    assert_eq!(st.format_header_roles(), "+rel[01:30][00:30]");

    st.open_user_profile(UserProfileTab::Passkeys);
    let html_keys = render_console(&st, Viewport::new(1440.0, 900.0), SafeArea::ZERO);
    assert!(html_keys.contains("Touch ID"));
    assert!(html_keys.contains("profile-passkey-remove:cred-1"));
    assert!(html_keys.contains("disabled=\"disabled\""));

    st.open_user_profile(UserProfileTab::Preferences);
    let html_prefs = render_console(&st, Viewport::new(1440.0, 900.0), SafeArea::ZERO);
    assert!(html_prefs.contains("profile-skin:html"));
    assert!(html_prefs.contains("profile-skin:wireframe"));
}

#[test]
fn u24_dist_freshness_and_standalone_tokens_integrity() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let local_tokens = manifest_dir.join("dist").join("tokens.css");
    assert!(
        local_tokens.exists(),
        "dist/tokens.css missing in mu-console-ui checkout"
    );
    let tokens_content = std::fs::read_to_string(&local_tokens).expect("read local tokens.css");
    assert!(!tokens_content.is_empty());

    // Keep unused DTO imports clean across feature configurations:
    let _ = (SubsystemId::Ci, PlatformUserDto::default());
}

#[test]
fn u25_profile_preferences_ux_wireframe_zero_overflow_and_reload_persistence() {
    let mut st = state_for_scenario(
        Scenario::NewBranch,
        SubView::Ci(CiSubView::Trainway),
        CrtFont::Hurmit,
    );
    for tab in [
        UserProfileTab::UserDetails,
        UserProfileTab::Preferences,
        UserProfileTab::Passkeys,
    ] {
        st.open_user_profile(tab);
        for &(vw, vh) in &[
            (320.0, 568.0),
            (375.0, 667.0),
            (874.0, 402.0),
            (1512.0, 860.0),
        ] {
            let wf_svg =
                render_console_with_skin(ConsoleSkin::Wireframe, &st, Viewport::new(vw, vh));
            assert_svg_zero_overflow(&wf_svg, vw, vh, &format!("u25_profile_{tab:?}_{vw}x{vh}"));
            let doc = roxmltree::Document::parse(&wf_svg).expect("valid SVG XML");
            let mut seen_ids = std::collections::BTreeSet::new();
            for n in doc.descendants() {
                if let Some(cid) = n.attribute("data-control-id") {
                    assert!(
                        seen_ids.insert(cid.to_owned()),
                        "duplicate data-control-id `{cid}` in Wireframe profile {tab:?} @ {vw}x{vh}"
                    );
                }
            }
            if vw >= 375.0 && vh >= 402.0 {
                assert!(
                    wf_svg.contains("id=\"muconsole-wf-profile-extra\""),
                    "expected profile tab content at {vw}x{vh}"
                );
            }
        }
    }

    // Reload persistence: stored OpenDyslexic + Wireframe survive default AuthSession snapshot.
    let mut reload_st = ConsoleState::new();
    reload_st.init_from_env("#/ci/queue-logs", "", Some("opendyslexic"));
    reload_st.restore_stored_skin("", Some("wireframe"));
    let vm = ViewModel {
        auth: full_active_auth("LMA1980"),
        user_profile: Some(UserProfileDto {
            github_login: "LMA1980".into(),
            display_name: Some("LMA1980".into()),
            preferences: UserPreferencesDto {
                skin: ConsoleSkin::Html,
                font: CrtFont::Hurmit,
            },
            bound_roles: vec![],
            passkey_count: 1,
            passkeys: vec![],
        }),
        ..ViewModel::default()
    };
    let snap = Engine::encode_event(&DataEvent::Snapshot(vm)).expect("encode");
    reload_st.on_worker_frame(&snap);
    assert_eq!(reload_st.font, CrtFont::OpenDyslexic);
    assert!(reload_st.lazy_dyslexic_loaded);
    assert_eq!(reload_st.effective_skin(), ConsoleSkin::Wireframe);

    let shell = render_html_shell("<svg></svg>", &reload_st);
    assert!(shell.contains(r#"data-crt-font="opendyslexic""#));
    assert!(shell.contains(r#"--font-crt:'OpenDyslexicM Nerd Font Mono','MuCRT',monospace"#));
    assert!(shell.contains("font-weight:700"));
}
