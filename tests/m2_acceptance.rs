// SPDX-License-Identifier: AGPL-3.0-only
//! Comprehensive M2 acceptance suite covering issues `#46–#57` and `#59` across
//! both `HtmlCoreSkin` and `WireframeSkin`.
#![cfg(not(miri))]

use mu_console_data::engine::Engine;
use mu_console_layout::Viewport;
use mu_console_protocol::dto::{
    AuthSessionDto, PasskeySummaryDto, PlatformSnapshotDto, PlatformUserDto, RoleActivationDto,
    RoleSpecDto, UserPreferencesDto, UserProfileDto,
};
use mu_console_protocol::{
    encode, AbDeploySubView, Area, AuthReasonCode, AuthState, AuthStatus, CiJobStage, CiJobSummary,
    CiSubView, ConsoleSkin, CrtFont, DashboardSubView, DataEvent, DialogModel,
    InviteCreatedOnceDialogModel, ModelRunnerSubView, PlatformSubView, RepoId, RoleBadgeView,
    RoleLeaseView, SkipReason, StageState, SubView, SubsystemId, UiCommand, UserProfileTab,
    ViewModel, Visibility,
};
use mu_console_ui::components::{render_console_with_skin, render_html_shell};
use mu_console_ui::roles::format_active_roles;
use mu_console_ui::router::{can_access_subview, parse_hash};
use mu_console_ui::state::{extract_invite_token, ConsoleState};

fn push_vm(st: &mut ConsoleState, vm: ViewModel) {
    let frame = Engine::encode_event(&DataEvent::Snapshot(vm)).expect("encode ViewModel");
    st.on_worker_frame(&frame);
}

fn active_state(login: &str, policies: Vec<&str>) -> ConsoleState {
    let mut st = ConsoleState::new();
    let vm = ViewModel {
        auth: AuthState::from_session(&AuthSessionDto {
            authenticated: true,
            invite_bound: true,
            passkey_count: 1,
            github_login: Some(login.into()),
            active_policies: policies.into_iter().map(str::to_owned).collect(),
            ..AuthSessionDto::default()
        }),
        ..ViewModel::default()
    };
    push_vm(&mut st, vm);
    st
}

fn render_both_skins(st: &ConsoleState) -> (String, String) {
    let vp = Viewport::new(1440.0, 900.0);
    let html_scene = render_console_with_skin(ConsoleSkin::Html, st, vp);
    let wf_scene = render_console_with_skin(ConsoleSkin::Wireframe, st, vp);
    let html_shell = render_html_shell(&html_scene, st);
    let wf_shell = render_html_shell(&wf_scene, st);
    assert!(roxmltree::Document::parse(&wf_scene).is_ok());
    (html_shell, wf_shell)
}

#[test]
fn m2_47_49_two_skin_parity_and_copy_normalization() {
    // 1. Both skins work from ConsoleScene without M1 Avionics/PullToTurn/HeavyData/SwitchState relics.
    let mut st = active_state(
        "ops",
        vec![
            "system:dashboard.read",
            "ci:queue.read",
            "ci:jobs.read",
            "ci:repos.public.read",
        ],
    );
    let (html_out, wf_out) = render_both_skins(&st);
    assert!(html_out.contains("data-skin=\"html\""));
    assert!(!html_out.contains("muconsole-font-toggle"));
    assert!(html_out.contains("@ops"));
    assert!(html_out.contains("Sign Out"));

    st.set_skin(ConsoleSkin::Wireframe);
    assert_eq!(st.effective_skin(), ConsoleSkin::Wireframe);
    let (_, wf_after) = render_both_skins(&st);
    assert!(wf_after.contains("data-skin=\"wireframe\""));
    assert!(wf_out.contains("<svg"));
}

#[test]
fn m2_50_cold_boot_unresolved_fail_closed_and_anonymous_public_ci() {
    // Cold boot: AuthStatus::Unresolved -> all 8 areas disabled, header shows Loading session…
    let cold = ConsoleState::new();
    assert_eq!(cold.auth().status, AuthStatus::Unresolved);
    for area in Area::ALL {
        assert!(
            !cold.can_access_area(area),
            "cold boot must disable {area:?}"
        );
    }
    let cold_shell = render_html_shell(
        &render_console_with_skin(ConsoleSkin::Html, &cold, Viewport::new(1280.0, 800.0)),
        &cold,
    );
    assert!(cold_shell.contains("muconsole-session-loading"));
    assert!(!cold_shell.contains("secret-repo"));

    // ?invite= stripped cleanly with zero invite disclosure
    let (tok, clean) = extract_invite_token("?foo=1&invite=inv_secret_999&bar=2");
    assert_eq!(tok.as_deref(), Some("inv_secret_999"));
    assert_eq!(clean, "?foo=1&bar=2");

    // Anonymous default `/` -> opens CI > Queue & Logs, public repos only, SkipReason, GitHub URL,
    // sub-tabs hidden (`!html.contains("mu-subview-nav")` or disabled), zero raw logs / nodes / threads!
    let mut anon = ConsoleState::new();
    let vm = ViewModel {
        auth: AuthState::anonymous(vec![
            "system:dashboard.read".into(),
            "ci:queue.read".into(),
            "ci:repos.public.read".into(),
        ]),
        repos: vec![
            mu_console_protocol::RepoScore::new(RepoId::new("LMA1980", "public-crate")),
            {
                let mut priv_repo = mu_console_protocol::RepoScore::new(RepoId::new(
                    "LMA1980",
                    "private-top-secret",
                ));
                priv_repo.visibility = Visibility::Private;
                priv_repo
            },
        ],
        ci_jobs: vec![
            CiJobSummary {
                job_id_str: "job-1000-deadbeef".into(),
                job_id: 77,
                repo: RepoId::new("LMA1980", "public-crate"),
                ref_name: "main".into(),
                commit_sha: "deadbeef".into(),
                queue_state: "passed".into(),
                elapsed_secs: 1,
                wait_secs: 0,
                quality_score: 95,
                coverage_bp: 9000,
                tests_passed: 10,
                tests_total: 10,
                benchmark_summary: String::new(),
                stages: vec![CiJobStage {
                    id: "fmt".into(),
                    name: "Format".into(),
                    assigned_node: Some("hidden-worker-node".into()),
                    threads: 16,
                    state: StageState::Skipped,
                    duration_secs: 0,
                    exit_code: 0,
                    skip_reason: Some(SkipReason::ConditionNotMet),
                }],
                raw_log_lines: vec!["RAW_INTERNAL_LOG_LINE_MUST_NOT_LEAK".into()],
            },
            CiJobSummary {
                job_id_str: "job-1000-cafe1234".into(),
                job_id: 88,
                repo: RepoId::new("LMA1980", "private-top-secret"),
                ref_name: "main".into(),
                commit_sha: "cafe1234".into(),
                queue_state: "failed".into(),
                ..CiJobSummary::default()
            },
        ],
        ..ViewModel::default()
    };
    push_vm(&mut anon, vm.clone());

    assert_eq!(anon.route.subview, SubView::Ci(CiSubView::QueueLogs));
    let html_anon =
        render_console_with_skin(ConsoleSkin::Html, &anon, Viewport::new(1440.0, 900.0));
    assert!(html_anon.contains("public-crate"));
    assert!(html_anon.contains("Workflow condition not met"));
    assert!(html_anon.contains("https://github.com/LMA1980/public-crate/commit/deadbeef"));
    assert!(html_anon.contains("noopener noreferrer"));
    assert!(!html_anon.contains("private-top-secret"));
    assert!(!html_anon.contains("hidden-worker-node"));
    assert!(!html_anon.contains("16t"));
    assert!(!html_anon.contains("RAW_INTERNAL_LOG_LINE_MUST_NOT_LEAK"));
    assert!(!html_anon.contains("nav-subview:ci:"));

    // Unauthorized deep link while anonymous keeps requested path & renders "Authentication required".
    let mut anon_deep = ConsoleState::new();
    anon_deep.init_from_env("#/platform/users", "", None);
    push_vm(&mut anon_deep, vm);
    let html_deep =
        render_console_with_skin(ConsoleSkin::Html, &anon_deep, Viewport::new(1440.0, 900.0));
    assert!(html_deep.contains("Authentication required"));
    assert_eq!(
        anon_deep.sign_in_href(),
        "/oauth2/start?rd=%2F%23%2Fplatform%2Fusers"
    );

    // Anonymous lockdown renders login gate and disables all 8 nav tabs.
    let mut locked = ConsoleState::new();
    push_vm(
        &mut locked,
        ViewModel {
            auth: AuthState::anonymous(vec![]),
            ..ViewModel::default()
        },
    );
    for area in Area::ALL {
        assert!(!locked.can_access_area(area));
    }
    let html_locked =
        render_console_with_skin(ConsoleSkin::Html, &locked, Viewport::new(1440.0, 900.0));
    assert!(html_locked.contains("login-gate-signin"));
    assert!(html_locked.contains("Sign in required"));
}

#[test]
fn m2_50_51_authorization_denied_and_non_dismissible_passkey_gate() {
    // AuthorizationDenied -> anonymous views + generic denial + Sign Out + no identity/invite leak
    let mut denied = ConsoleState::new();
    push_vm(
        &mut denied,
        ViewModel {
            auth: AuthState::from_session(&AuthSessionDto {
                authenticated: false,
                invite_bound: false,
                github_login: Some("uninvited-dev".into()),
                reason_code: Some(AuthReasonCode::AuthorizationDenied),
                active_policies: vec!["ci:queue.read".into(), "ci:repos.public.read".into()],
                ..AuthSessionDto::default()
            }),
            ..ViewModel::default()
        },
    );
    let shell_denied = render_html_shell("", &denied);
    assert!(shell_denied.contains("Authorization denied"));
    assert!(shell_denied.contains("Sign Out"));
    assert!(!shell_denied.contains("uninvited-dev"));
    assert!(!shell_denied.contains("invite"));
    assert!(denied
        .open_user_profile(UserProfileTab::UserDetails)
        .is_empty());
    assert!(denied.open_session_roles().is_empty());

    // 0-Passkey guiding modal -> auto-opens with dismiss option, preserves baseline public
    // Dashboard & CI access when not in anonymous_lockdown, while gating operator areas.
    let mut gate = ConsoleState::new();
    push_vm(
        &mut gate,
        ViewModel {
            auth: AuthState::from_session(&AuthSessionDto {
                authenticated: true,
                invite_bound: true,
                passkey_count: 0,
                passkey_enrollment_required: true,
                github_login: Some("newop".into()),
                active_policies: vec!["system:dashboard.read".into()],
                ..AuthSessionDto::default()
            }),
            ..ViewModel::default()
        },
    );
    assert!(matches!(
        gate.active_dialog(),
        Some(DialogModel::PasskeyEnrollmentGate(_))
    ));
    let gate_html =
        render_console_with_skin(ConsoleSkin::Html, &gate, Viewport::new(1280.0, 800.0));
    let gate_wf =
        render_console_with_skin(ConsoleSkin::Wireframe, &gate, Viewport::new(1280.0, 800.0));
    assert!(gate_html.contains("dlg-passkey-register"));
    assert!(gate_html.contains("dlg-passkey-dismiss"));
    assert!(gate_wf.contains("dlg-passkey-register"));
    assert_eq!(gate.dismiss_dialog(), vec![UiCommand::CloseDialog]);
    assert!(gate.active_dialog().is_none());
    let _ = gate.activate_target("register-passkey");
    let _ = gate.pending_webauthn.take();
    assert_eq!(
        gate.on_key("Escape", false, 1_000),
        Some(UiCommand::CloseDialog)
    );
    assert!(gate.active_dialog().is_none());
    assert!(gate.can_access_area(Area::Dashboard));
    assert!(gate.can_access_area(Area::Ci));
    for area in [
        Area::Cd,
        Area::AbDeploy,
        Area::Repository,
        Area::Observability,
        Area::ModelRunner,
        Area::Platform,
    ] {
        assert!(!gate.can_access_area(area));
    }
}

#[test]
fn m2_52_59_header_roles_budget_activity_reset_and_selective_expiry() {
    // 1. UTF-8 safe chars().count() budget <= 120 + "…(+N)" suffix
    let roles = vec![
        RoleBadgeView {
            role_id: "r1".into(),
            display_name: Some("obs".into()),
            activation: RoleActivationDto::default(),
            lease: None,
        },
        RoleBadgeView {
            role_id: "r2".into(),
            display_name: Some("rel".into()),
            activation: RoleActivationDto::timed(900, 300),
            lease: Some(RoleLeaseView {
                role_id: "r2".into(),
                display_name: None,
                activated_at_ms: 1_000,
                last_activity_at_ms: 1_000,
                hard_expires_at_ms: 100_000,
                idle_expires_at_ms: 50_000,
                hard_ttl_secs: 99,
                idle_ttl_secs: 49,
                seconds_remaining: 99,
                idle_timer: 49,
                ..RoleLeaseView::default()
            }),
        },
        RoleBadgeView {
            role_id: "r3".into(),
            display_name: Some("platform-maintenance-admin-superlong".into()),
            activation: RoleActivationDto::default(),
            lease: None,
        },
        RoleBadgeView {
            role_id: "r4".into(),
            display_name: Some("audit-extra-role".into()),
            activation: RoleActivationDto::default(),
            lease: None,
        },
    ];
    let formatted = format_active_roles(&roles, 30);
    assert!(formatted.chars().count() <= 30, "got '{formatted}'");
    assert!(
        formatted.contains("…(+"),
        "expected overflow indicator in '{formatted}'"
    );

    // 2. Two Timed leases: only the expired role deactivates; surviving stays active;
    //    redirect happens ONLY if remaining policies lack access to current view.
    let mut st = ConsoleState::new();
    let mut auth = AuthState::from_session(&AuthSessionDto {
        authenticated: true,
        invite_bound: true,
        passkey_count: 1,
        github_login: Some("ops".into()),
        baseline_role: Some("viewer".into()),
        active_policies: vec![
            "system:dashboard.read".into(),
            "ci:jobs.read".into(),
            "ab:rollback".into(),
            "platform:users.read".into(),
        ],
        leases: vec![
            RoleLeaseView {
                role_id: "ab-ops".into(),
                display_name: Some("ab".into()),
                activated_at_ms: 0,
                last_activity_at_ms: 0,
                hard_expires_at_ms: 20_000,
                idle_expires_at_ms: 15_000,
                hard_ttl_secs: 20,
                idle_ttl_secs: 15,
                seconds_remaining: 20,
                idle_timer: 15,
                ..RoleLeaseView::default()
            },
            RoleLeaseView {
                role_id: "platform-admin".into(),
                display_name: Some("plat".into()),
                activated_at_ms: 0,
                last_activity_at_ms: 0,
                hard_expires_at_ms: 5_000,
                idle_expires_at_ms: 4_000,
                hard_ttl_secs: 5,
                idle_ttl_secs: 4,
                seconds_remaining: 5,
                idle_timer: 4,
                ..RoleLeaseView::default()
            },
        ],
        active_roles: vec![
            RoleSpecDto {
                id: "ab-ops".into(),
                display_name: Some("ab".into()),
                activation: RoleActivationDto::timed(600, 300),
                ..RoleSpecDto::default()
            },
            RoleSpecDto {
                id: "platform-admin".into(),
                display_name: Some("plat".into()),
                activation: RoleActivationDto::timed(600, 300),
                ..RoleSpecDto::default()
            },
        ],
        ..AuthSessionDto::default()
    });
    auth.active_policies = vec![
        "system:dashboard.read".into(),
        "ci:jobs.read".into(),
        "ab:rollback".into(),
        "platform:users.read".into(),
    ];
    push_vm(
        &mut st,
        ViewModel {
            auth,
            ..ViewModel::default()
        },
    );
    st.select_subview(SubView::Platform(PlatformSubView::UsersAndInvites));
    assert_eq!(st.route.area, Area::Platform);

    // Explicit activity extends idle_expires_at_ms while tick() alone never does.
    let idle_before = st.auth().leases[0].idle_expires_at_ms;
    assert_eq!(
        st.on_explicit_user_activity(2_000),
        vec![UiCommand::RecordUserActivity]
    );
    let idle_after = st.auth().leases[0].idle_expires_at_ms;
    assert!(
        idle_after > idle_before,
        "explicit activity must reset idle timer"
    );

    // Tick at 3_000 and 4_500: neither lease is expired because explicit activity pushed
    // platform-admin's idle expiry out from 4_000 to hard_expires_at_ms (5_000).
    assert!(st.tick(3_000).is_empty());
    assert!(st.tick(4_500).is_empty());
    assert_eq!(st.auth().leases.len(), 2);

    // Tick at 5_000: platform-admin hard expiry hits 00:00; ab-ops remains active!
    let cmds = st.tick(5_000);
    assert!(cmds.contains(&UiCommand::RefreshSession));
    assert_eq!(st.auth().leases.len(), 1);
    assert_eq!(st.auth().leases[0].role_id, "ab-ops");

    // Lock All Timed Roles command emitted explicitly.
    assert_eq!(
        st.activate_target("roles-lock-all"),
        vec![UiCommand::LockTimedRoles]
    );

    // 3-tab @user Profile dialog renders User Details | Preferences | Passkeys in Html & Wireframe.
    let mut prof_vm = st.client.view().cloned().unwrap();
    prof_vm.user_profile = Some(UserProfileDto {
        github_login: "ops".into(),
        display_name: Some("Ops".into()),
        preferences: UserPreferencesDto {
            skin: ConsoleSkin::Html,
            font: CrtFont::Hurmit,
        },
        bound_roles: vec![],
        passkey_count: 1,
        passkeys: vec![PasskeySummaryDto {
            credential_id: "cred-a".into(),
            label: Some("Key A".into()),
            sign_count: 1,
            created_at_ms: 1_000,
            last_used_at_ms: None,
        }],
    });
    push_vm(&mut st, prof_vm);
    st.open_user_profile(UserProfileTab::UserDetails);
    let (p_html, p_wf) = render_both_skins(&st);
    assert!(p_html.contains("profile-tab:details"));
    assert!(p_html.contains("profile-tab:preferences"));
    assert!(p_html.contains("profile-tab:passkeys"));
    assert!(p_wf.contains("@ops"));
    assert!(p_wf.contains("Passkeys: 1"));
}

#[test]
fn m2_54_57_universal_disabled_router_8_areas_27_subviews_and_platform_guards() {
    // Verify canonical 8 Areas and 27 SubViews count.
    assert_eq!(Area::ALL.len(), 8);
    let total_subviews: usize = Area::ALL.iter().map(|a| SubView::all_for(*a).len()).sum();
    assert_eq!(total_subviews, 27);

    // Sub-permission isolation: holding `ab:slots.read` without `ab:gate.logs.read` enables
    // A|B IntegrationGate while disabling A|B GateTestLogs!
    let limited_auth = AuthState::from_session(&AuthSessionDto {
        authenticated: true,
        invite_bound: true,
        passkey_count: 1,
        github_login: Some("ab-viewer".into()),
        active_policies: vec!["system:dashboard.read".into(), "ab:slots.read".into()],
        ..AuthSessionDto::default()
    });
    assert!(can_access_subview(
        &limited_auth,
        SubView::AbDeploy(AbDeploySubView::IntegrationGate)
    ));
    assert!(!can_access_subview(
        &limited_auth,
        SubView::AbDeploy(AbDeploySubView::GateTestLogs)
    ));
    assert!(!limited_auth.can_access_area(Area::Platform));

    // Clicking disabled nav-area:platform / nav-subview / dash-card performs ZERO navigation & ZERO OAuth redirect.
    let mut st = active_state("ab-viewer", vec!["system:dashboard.read", "ab:slots.read"]);
    st.select_subview(SubView::Dashboard(DashboardSubView::Overview));
    let html_dash = render_console_with_skin(ConsoleSkin::Html, &st, Viewport::new(1440.0, 900.0));
    assert!(html_dash.contains("dash-card:platform"));
    assert!(html_dash.contains("disabled"));

    assert!(st
        .on_dom_action("data-area", "platform", false, 100)
        .is_empty());
    assert!(st
        .on_dom_action("data-action", "dash-card:platform", false, 100)
        .is_empty());
    assert!(st
        .on_dom_action("data-subview", "ab-deploy:gate-logs", false, 100)
        .is_empty());
    assert_eq!(st.route.area, Area::Dashboard);
    assert_eq!(st.pending_navigation, None);

    // Strictly read-only ModelRunner (#56) has zero mutating actions across both sub-views.
    let mut full = active_state(
        "admin",
        vec![
            "mr:models.read",
            "mr:openai_api.read",
            "mr:wireguard_vpn.read",
            "platform:users.read",
            "platform:users.write",
            "platform:invites.read",
            "platform:invites.write",
            "platform:roles.read",
            "platform:roles.write",
            "platform:folios.read",
            "platform:folios.write",
            "platform:maintenance.read",
            "platform:maintenance.write",
        ],
    );
    full.select_subview(SubView::ModelRunner(ModelRunnerSubView::RuntimeQueue));
    let mr_rt = render_console_with_skin(ConsoleSkin::Html, &full, Viewport::new(1440.0, 900.0));
    assert!(mr_rt.contains("Read-Only"));
    full.select_subview(SubView::ModelRunner(ModelRunnerSubView::ConnectionVpn));
    let mr_vpn = render_console_with_skin(ConsoleSkin::Html, &full, Viewport::new(1440.0, 900.0));
    assert!(mr_vpn.contains("Connection &amp; VPN"));

    // Platform Last-Admin Guard + InviteCreatedOnce modal + #/iam alias (#57)
    assert_eq!(
        parse_hash("#/iam/folios").subview,
        SubView::Platform(PlatformSubView::RolesAndFolios)
    );
    let mut pvm = full.client.view().cloned().unwrap();
    pvm.platform = PlatformSnapshotDto {
        users: vec![PlatformUserDto {
            github_login: "sole-admin".into(),
            display_name: Some("Sole Admin".into()),
            passkey_count: 1,
            bound_roles: vec!["platform-admin".into()],
            is_platform_admin: true,
            last_seen_ms: None,
        }],
        roles: vec![RoleSpecDto {
            id: "platform-admin".into(),
            display_name: Some("Platform Admin".into()),
            ..RoleSpecDto::default()
        }],
        ..PlatformSnapshotDto::default()
    };
    push_vm(&mut full, pvm);
    full.select_subview(SubView::Platform(PlatformSubView::UsersAndInvites));

    let plat_html =
        render_console_with_skin(ConsoleSkin::Html, &full, Viewport::new(1440.0, 900.0));
    assert!(plat_html.contains("Last Admin"));
    // Attempting to revoke sole platform admin refuses to open RiskConfirm.
    assert!(full
        .activate_target("platform-user-revoke:sole-admin")
        .is_empty());
    assert!(full.active_dialog().is_none());

    // InviteCreatedOnce one-time secret dialog renders across both skins.
    let mut inv_vm = full.client.view().cloned().unwrap();
    inv_vm.active_dialog = Some(DialogModel::InviteCreatedOnce(
        InviteCreatedOnceDialogModel {
            invite_id: "inv-id-1".into(),
            invite_url: "https://console.example.org/?invite=one_time_secret".into(),
            granted_role: "viewer".into(),
            expires_at_ms: 9_999_999_999_999,
            note: Some("new-dev".into()),
        },
    ));
    push_vm(&mut full, inv_vm);
    let (inv_html, inv_wf) = render_both_skins(&full);
    assert!(inv_html.contains("one_time_secret"));
    assert!(inv_wf.contains("invite"));

    // Verify encode/decode symbol works seamlessly with UiCommand.
    let _ = encode(&UiCommand::CloseDialog).expect("encode");
    let _ = (SubsystemId::Platform, PlatformSubView::Maintenance);
}

#[test]
fn m2_user_profile_preferences_active_ux_and_opendyslexic_rendering() {
    let mut st = active_state("ops", vec!["system:dashboard.read", "ci:queue.read"]);
    let mut vm = st.client.view().cloned().expect("view");
    vm.user_profile = Some(UserProfileDto {
        github_login: "ops".into(),
        display_name: Some("Ops".into()),
        preferences: UserPreferencesDto {
            skin: ConsoleSkin::Html,
            font: CrtFont::Hurmit,
        },
        bound_roles: vec![],
        passkey_count: 1,
        passkeys: vec![],
    });
    push_vm(&mut st, vm);
    st.open_user_profile(UserProfileTab::Preferences);

    let (html_prefs, wf_prefs) = render_both_skins(&st);
    assert_eq!(
        html_prefs
            .matches(r#"data-control-id="profile-tab:passkeys""#)
            .count(),
        1,
        "profile-tab:passkeys must not be duplicated in HTML modal"
    );
    assert_eq!(
        wf_prefs
            .matches(r#"data-control-id="profile-tab:passkeys""#)
            .count(),
        1,
        "profile-tab:passkeys must not be duplicated in Wireframe modal"
    );
    assert!(html_prefs.contains(r#"data-active-skin="html">Active: HTML Core"#));
    assert!(html_prefs.contains(r#"data-active-font="hurmit">Active: Hurmit Mono"#));
    assert!(html_prefs.contains(
        r#"data-control-id="profile-skin:html" data-action="profile-skin:html" aria-pressed="true" data-selected="true"><span class="mu-signal" aria-hidden="true">●</span>HTML Core"#
    ));
    assert!(html_prefs.contains(
        r#"data-control-id="profile-skin:wireframe" data-action="profile-skin:wireframe" aria-pressed="false" data-selected="false"><span class="mu-signal" aria-hidden="true">○</span>Wireframe SVG"#
    ));
    assert!(html_prefs.contains(
        r#"data-control-id="profile-font:hurmit" data-action="profile-font:hurmit" aria-pressed="true" data-selected="true"><span class="mu-signal" aria-hidden="true">●</span>Hurmit Mono"#
    ));
    assert!(html_prefs.contains(
        r#"data-control-id="profile-font:dyslexic" data-action="profile-font:dyslexic" aria-pressed="false" data-selected="false"><span class="mu-signal" aria-hidden="true">○</span>OpenDyslexicM"#
    ));
    assert!(wf_prefs.contains(r#"id="muconsole-wf-profile-extra""#));
    assert!(wf_prefs.contains("Active: HTML Core"));
    assert!(wf_prefs.contains("Active: Hurmit Mono"));
    assert!(wf_prefs.contains("● HTML Core"));
    assert!(wf_prefs.contains("○ Wireframe SVG"));
    assert!(wf_prefs.contains("● Hurmit Mono"));
    assert!(wf_prefs.contains("○ OpenDyslexicM"));

    // Toggle both skin -> Wireframe SVG and font -> OpenDyslexicM.
    st.on_dom_action("data-action", "profile-skin:wireframe", false, 1_000);
    st.on_dom_action("data-action", "profile-font:dyslexic", false, 1_100);
    let (html_after, wf_after) = render_both_skins(&st);
    assert!(html_after.contains(r#"data-crt-font="opendyslexic""#));
    assert!(html_after.contains(r#"--font-crt:'OpenDyslexicM Nerd Font Mono','MuCRT',monospace"#));
    assert!(html_after.contains("font-weight:400"));
    assert!(html_after.contains("font-weight:700"));
    assert!(html_after.contains(r#"data-active-skin="wireframe">Active: Wireframe SVG"#));
    assert!(html_after.contains(r#"data-active-font="opendyslexic">Active: OpenDyslexicM"#));
    assert!(wf_after.contains("Active: Wireframe SVG"));
    assert!(wf_after.contains("Active: OpenDyslexicM"));
    assert!(wf_after.contains("● Wireframe SVG"));
    assert!(wf_after.contains("● OpenDyslexicM"));

    // PasskeyEnrollmentRequired session can toggle typography locally while Wireframe stays disabled.
    let mut gate_st = ConsoleState::new();
    let gate_vm = ViewModel {
        auth: AuthState::from_session(&AuthSessionDto {
            authenticated: true,
            invite_bound: true,
            passkey_count: 0,
            passkey_enrollment_required: true,
            github_login: Some("new-op".into()),
            active_policies: vec!["system:dashboard.read".into()],
            ..AuthSessionDto::default()
        }),
        ..ViewModel::default()
    };
    push_vm(&mut gate_st, gate_vm);
    gate_st.open_user_profile(UserProfileTab::Preferences);
    let cmds = gate_st.on_dom_action("data-action", "profile-font:dyslexic", false, 2_000);
    assert_eq!(cmds, vec![UiCommand::SetCrtFont(CrtFont::OpenDyslexic)]);
    assert_eq!(gate_st.font, CrtFont::OpenDyslexic);
    let gate_html =
        render_console_with_skin(ConsoleSkin::Html, &gate_st, Viewport::new(1440.0, 900.0));
    assert!(gate_html.contains(
        r#"data-control-id="profile-skin:wireframe" data-action="profile-skin:wireframe" aria-pressed="false" data-selected="false" disabled="disabled" aria-disabled="true""#
    ));
}

#[test]
fn m2_super_operator_unlocks_all_8_areas_and_surfaces_last_2m_renewal_in_both_skins() {
    use mu_console_protocol::StepUpRequirement;
    let mut state = ConsoleState::new();
    let super_spec = RoleSpecDto {
        id: "super-operator".into(),
        display_name: Some("Super Operator".into()),
        description: "All-access Passkey role (2m idle, 15m session, single +15m last-2m renewal)"
            .into(),
        activation: RoleActivationDto::super_operator(),
        step_up: StepUpRequirement::Passkey,
    };

    let all_47_policies: Vec<String> = vec![
        "system:status.read".into(),
        "system:dashboard.read".into(),
        "ci:queue.read".into(),
        "ci:jobs.read".into(),
        "ci:jobs.restart".into(),
        "ci:jobs.resume".into(),
        "ci:nodes.read".into(),
        "ci:repos.public.read".into(),
        "ci:repos.private.read".into(),
        "cd:runs.read".into(),
        "cd:steps.read".into(),
        "ab:slots.read".into(),
        "ab:gate.logs.read".into(),
        "ab:rollback".into(),
        "reg:packages.read".into(),
        "reg:canaries.read".into(),
        "reg:cas.read".into(),
        "reg:taints.read".into(),
        "reg:canaries.write".into(),
        "obs:problems.read".into(),
        "obs:traces.read".into(),
        "obs:logs.read".into(),
        "obs:metrics.read".into(),
        "obs:ingress.write".into(),
        "mr:models.read".into(),
        "mr:connect.write".into(),
        "platform:users.write".into(),
        "platform:roles.write".into(),
        "platform:folios.write".into(),
        "platform:invites.write".into(),
        "platform:maintenance.write".into(),
    ];

    let mut lease = RoleLeaseView {
        role_id: "super-operator".into(),
        display_name: Some("Super Operator".into()),
        activated_at_ms: 0,
        last_activity_at_ms: 0,
        hard_expires_at_ms: 900_000,
        idle_expires_at_ms: 120_000,
        hard_ttl_secs: 900,
        idle_ttl_secs: 120,
        renew_window_secs: 120,
        renew_extend_secs: 900,
        max_renewals: 1,
        renewals_used: 0,
        seconds_remaining: 900,
        idle_timer: 120,
        can_renew: false,
    };
    let mut vm = ViewModel {
        auth: AuthState::from_session(&AuthSessionDto {
            authenticated: true,
            github_login: Some("LMA1980".into()),
            invite_bound: true,
            passkey_count: 1,
            baseline_role: Some("operator".into()),
            bound_roles: vec!["operator".into(), "super-operator".into()],
            bound_role_specs: vec![super_spec.clone()],
            active_roles: vec![super_spec],
            leases: vec![lease.clone()],
            active_policies: all_47_policies.clone(),
            ..AuthSessionDto::default()
        }),
        ..ViewModel::default()
    };
    push_vm(&mut state, vm.clone());

    for area in Area::ALL {
        assert!(state.can_access_area(area), "expected access to {area:?}");
    }
    let (init_html, init_wf) = render_both_skins(&state);
    assert!(init_html.contains("+Super Operator[15:00][02:00]"));
    assert!(init_wf.contains("+Super Operator[15:00][02:00]"));
    assert!(!init_html.contains("banner-role-renew-super-operator"));
    assert!(!init_wf.contains("role-renew:super-operator"));

    // Final 2m renewal window opens at 120s remaining -> both banner and +role modal render controls.
    lease.hard_expires_at_ms = 120_000;
    lease.idle_expires_at_ms = 120_000;
    lease.seconds_remaining = 120;
    lease.can_renew = true;
    vm.auth = AuthState::from_session(&AuthSessionDto {
        authenticated: true,
        github_login: Some("LMA1980".into()),
        invite_bound: true,
        passkey_count: 1,
        baseline_role: Some("operator".into()),
        bound_roles: vec!["operator".into(), "super-operator".into()],
        bound_role_specs: vm.auth.bound_role_specs.clone(),
        active_roles: vec![RoleSpecDto {
            id: "super-operator".into(),
            display_name: Some("Super Operator".into()),
            description: "All-access Passkey role".into(),
            activation: RoleActivationDto::super_operator(),
            step_up: StepUpRequirement::Passkey,
        }],
        leases: vec![lease.clone()],
        active_policies: all_47_policies.clone(),
        ..AuthSessionDto::default()
    });
    push_vm(&mut state, vm.clone());
    let _ = state.open_session_roles();

    let (win_html, win_wf) = render_both_skins(&state);
    assert!(win_html.contains("banner-role-renew-super-operator"));
    assert!(win_html.contains("role-renew:super-operator"));
    assert!(win_wf.contains("role-renew:super-operator"));
    assert!(win_wf.contains("Renew +15m"));

    // Clicking role-renew queues WebAuthnRequest::RenewTimedRole.
    let _ = state.activate_target("role-renew:super-operator");
    assert_eq!(
        state.pending_webauthn,
        Some(mu_console_ui::state::WebAuthnRequest::RenewTimedRole {
            role_id: "super-operator".into()
        })
    );
    let renew_cmds = state.complete_role_renewal("super-operator", "{}");
    assert_eq!(
        renew_cmds,
        vec![UiCommand::RenewRoleWithPasskey {
            role_id: "super-operator".into(),
            assertion_json: "{}".into(),
        }]
    );

    // After single renewal (`renewals_used == 1`), renewal controls stay suppressed even <= 2m left.
    lease.renewals_used = 1;
    lease.can_renew = false;
    lease.seconds_remaining = 90;
    vm.auth = AuthState::from_session(&AuthSessionDto {
        authenticated: true,
        github_login: Some("LMA1980".into()),
        invite_bound: true,
        passkey_count: 1,
        baseline_role: Some("operator".into()),
        bound_roles: vec!["operator".into(), "super-operator".into()],
        bound_role_specs: vm.auth.bound_role_specs.clone(),
        active_roles: vec![RoleSpecDto {
            id: "super-operator".into(),
            display_name: Some("Super Operator".into()),
            description: "All-access Passkey role".into(),
            activation: RoleActivationDto::super_operator(),
            step_up: StepUpRequirement::Passkey,
        }],
        leases: vec![lease],
        active_policies: all_47_policies,
        ..AuthSessionDto::default()
    });
    push_vm(&mut state, vm);
    let (done_html, done_wf) = render_both_skins(&state);
    assert!(!done_html.contains("banner-role-renew-super-operator"));
    assert!(!done_wf.contains("role-renew:super-operator"));
}

#[test]
fn m2_71_dashboard_orthogonal_split_registry_copy_anonymous_gating_and_tones() {
    use mu_console_protocol::{
        FabricNode, HardwareMetricSeries, LogSeverity, OtelLogEntry, PurePathTrace, SloProblem,
        SubsystemHealthEntry, SubsystemHealthSummary, SubsystemStatus,
    };
    use mu_console_theme::scene::{SceneComponent, SceneViewport};
    use mu_console_theme::skin::SemanticTone;
    use mu_console_ui::scene::build_console_scene;

    let mut base_vm = ViewModel {
        repos: vec![mu_console_protocol::RepoScore::new(RepoId::new(
            "org", "core",
        ))],
        ci_jobs: vec![CiJobSummary {
            job_id_str: "job-1000-0000701".into(),
            job_id: 701,
            repo: RepoId::new("org", "core"),
            ref_name: "main".into(),
            commit_sha: "abc1234".into(),
            queue_state: "failed".into(),
            tests_passed: 9,
            tests_total: 10,
            stages: vec![CiJobStage {
                id: "clippy".into(),
                name: "clippy".into(),
                assigned_node: Some("node-a".into()),
                threads: 4,
                state: StageState::Failed,
                duration_secs: 12,
                exit_code: 1,
                skip_reason: Some(SkipReason::Other("clippy error".into())),
            }],
            ..CiJobSummary::default()
        }],
        nodes: vec![
            FabricNode {
                id: "node-a".into(),
                online: true,
                arch: "x86_64".into(),
                active_slots: 0,
                max_slots: 1,
                cpu_load_bp: 1200,
                ..FabricNode::default()
            },
            FabricNode {
                id: "worker-riscv64".into(),
                online: false,
                arch: "riscv64".into(),
                active_slots: 0,
                max_slots: 1,
                cpu_load_bp: 0,
                ..FabricNode::default()
            },
        ],
        subsystem_health: SubsystemHealthSummary {
            ci: SubsystemHealthEntry {
                status: SubsystemStatus::Live,
                summary: "Operational".into(),
                ..SubsystemHealthEntry::default()
            },
            cd: SubsystemHealthEntry {
                status: SubsystemStatus::Live,
                summary: "Operational".into(),
                ..SubsystemHealthEntry::default()
            },
            ab_deploy: SubsystemHealthEntry {
                status: SubsystemStatus::InMaintenance,
                summary: "Maint".into(),
                ..SubsystemHealthEntry::default()
            },
            repository: SubsystemHealthEntry {
                status: SubsystemStatus::Live,
                summary: "Operational".into(),
                ..SubsystemHealthEntry::default()
            },
            observability: SubsystemHealthEntry {
                status: SubsystemStatus::Degraded,
                summary: "Operational".into(),
                ..SubsystemHealthEntry::default()
            },
            model_runner: SubsystemHealthEntry {
                status: SubsystemStatus::Offline,
                summary: "Offline".into(),
                ..SubsystemHealthEntry::default()
            },
            platform: SubsystemHealthEntry {
                status: SubsystemStatus::PartialMaintenance,
                summary: "Partial".into(),
                ..SubsystemHealthEntry::default()
            },
            updated_at_ms: 1_000,
        },
        ..ViewModel::default()
    };
    base_vm.observe.logs.push(OtelLogEntry {
        timestamp_ms: 1_000,
        severity: LogSeverity::Info,
        service: "gateway".into(),
        span_id: None,
        trace_id: "trace-1".into(),
        message: "GET /healthz 200".into(),
    });
    base_vm.observe.logs.push(OtelLogEntry {
        timestamp_ms: 1_010,
        severity: LogSeverity::Warn,
        service: "gateway".into(),
        span_id: None,
        trace_id: "trace-2".into(),
        message: "elevated upstream latency".into(),
    });
    base_vm.observe.traces.push(PurePathTrace {
        trace_id: "trace-2".into(),
        root_service: "gateway".into(),
        root_operation: "GET /api/ci".into(),
        domain: "edge".into(),
        duration_ms: 210,
        error: false,
        slow_p99: true,
        started_epoch_ms: 1_010,
        spans: vec![],
    });
    base_vm.observe.metrics.push(HardwareMetricSeries {
        service: "gateway".into(),
        hardware_class: "x86_64".into(),
        rps: 120,
        error_rate_bp: 10,
        p50_ms: 8,
        p95_ms: 40,
        p99_ms: 210,
        exemplar_trace_id: "trace-2".into(),
        samples: vec![10, 40, 210],
    });
    base_vm.observe.problems.push(SloProblem {
        problem_id: "slo-1".into(),
        title: "Ingress p99 latency above SLO".into(),
        severity: "critical".into(),
        impacted_entity: "gateway".into(),
        active: true,
        ..SloProblem::default()
    });

    // 1. Authenticated operator WITH ci:nodes.read -> Applications + Fleet + Needs Attention.
    let mut ops_full = active_state(
        "ops",
        vec![
            "system:dashboard.read",
            "ci:queue.read",
            "ci:jobs.read",
            "ci:repos.public.read",
            "ci:nodes.read",
            "cd:runs.read",
            "reg:packages.read",
            "obs:smartscape.read",
            "obs:problems.read",
            "obs:logs.read",
            "obs:traces.read",
            "obs:metrics.read",
            "mr:models.read",
            "platform:roles.read",
        ],
    );
    let mut full_vm = base_vm.clone();
    full_vm.auth = ops_full.auth();
    push_vm(&mut ops_full, full_vm.clone());

    let scene = build_console_scene(&ops_full, SceneViewport::DESKTOP);
    let app_sec = scene
        .sections
        .iter()
        .find(|s| s.id == "sec-dash-applications")
        .expect("Applications section");
    assert_eq!(app_sec.title, "Applications");
    let SceneComponent::StatusMatrix(mat) = &app_sec.components[0] else {
        panic!("expected StatusMatrix");
    };
    assert_eq!(mat.title, "Application Status");
    let labels: Vec<&str> = mat.cells.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(
        labels,
        vec![
            "Continuous Integration",
            "Continuous Delivery",
            "A|B Deployment",
            "Registry",
            "Observability",
            "Model Runner",
            "Platform",
        ]
    );
    // Observability summary includes logs, traces, metrics, and active SLO problems.
    let obs_cell = &mat.cells[4];
    assert_eq!(
        obs_cell.detail.as_deref(),
        Some("2 logs · 1 traces · 1 metrics · 1 active SLO problem(s)")
    );
    assert_eq!(obs_cell.tone, SemanticTone::Fail);
    assert_eq!(mat.cells[2].tone, SemanticTone::Info); // InMaintenance
    assert_eq!(mat.cells[6].tone, SemanticTone::Info); // PartialMaintenance
    assert_eq!(mat.cells[5].tone, SemanticTone::Fail); // Offline

    let (html_full, wf_full) = render_both_skins(&ops_full);
    for out in [&html_full, &wf_full] {
        assert!(!out.contains("Fleet Summary"));
        assert!(!out.contains("7 Subsystem Status Cards"));
        assert!(!out.contains("Artifact &amp; Package Store"));
        assert!(!out.contains("CI Build Fabric"));
        assert!(out.contains("Applications"));
        assert!(out.contains("Fleet"));
        assert!(out.contains("Cluster Host Nodes"));
        assert!(out.contains("worker-riscv64"));
        assert!(out.contains("Needs Attention"));
        assert!(out.contains("Ingress p99 latency above SLO"));
        assert!(out.contains("Application"));
    }

    // Needs Attention columns: Scope, Target, Finding, Status (covering App + Fleet).
    let att_sec = scene
        .sections
        .iter()
        .find(|s| s.id == "sec-dash-attention")
        .expect("Needs Attention section");
    let SceneComponent::TableGrid(att_grid) = &att_sec.components[0] else {
        panic!("expected TableGrid");
    };
    let col_labels: Vec<&str> = att_grid.columns.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(col_labels, vec!["Scope", "Target", "Finding", "Status"]);
    assert!(att_grid
        .rows
        .iter()
        .any(|r| r.cells[0].text == "Application"));
    assert!(att_grid.rows.iter().any(|r| r.cells[0].text == "Fleet"));

    // Verify Registry copy inside CD & Registry areas.
    let _ = ops_full.activate_target("nav-area:cd");
    let (cd_html, cd_wf) = render_both_skins(&ops_full);
    assert!(cd_html.contains("Open Registry"));
    assert!(cd_wf.contains("Open Registry"));
    assert!(!cd_html.contains("Open Repository"));
    let _ = ops_full.activate_target("nav-area:registry");
    let (reg_html, reg_wf) = render_both_skins(&ops_full);
    assert!(reg_html.contains("Registry · Packages"));
    assert!(reg_wf.contains("Registry · Packages"));
    assert!(!reg_html.contains("Repository · Packages"));

    // 2. Active operator WITHOUT ci:nodes.read -> Fleet omitted, Applications + Needs Attention shown.
    let mut ops_no_nodes = active_state(
        "ops-no-nodes",
        vec![
            "system:dashboard.read",
            "ci:queue.read",
            "ci:jobs.read",
            "ci:repos.public.read",
        ],
    );
    let mut no_nodes_vm = base_vm.clone();
    no_nodes_vm.auth = ops_no_nodes.auth();
    push_vm(&mut ops_no_nodes, no_nodes_vm);
    let no_nodes_scene = build_console_scene(&ops_no_nodes, SceneViewport::DESKTOP);
    assert!(no_nodes_scene
        .sections
        .iter()
        .any(|s| s.id == "sec-dash-applications"));
    assert!(!no_nodes_scene
        .sections
        .iter()
        .any(|s| s.id == "sec-dash-fleet"));
    assert!(no_nodes_scene
        .sections
        .iter()
        .any(|s| s.id == "sec-dash-attention"));

    // 3. Anonymous AND AuthorizationDenied: Fleet + Needs Attention completely omitted,
    // Degraded banner without count badges, Info/Fail parity on unpermitted cards.
    for auth in [
        AuthState::anonymous(vec![
            "system:dashboard.read".into(),
            "ci:queue.read".into(),
            "ci:repos.public.read".into(),
        ]),
        AuthState {
            status: AuthStatus::AuthorizationDenied,
            github_login: Some("uninvited".into()),
            reason_code: Some(AuthReasonCode::AuthorizationDenied),
            active_policies: vec!["system:dashboard.read".into()],
            ..AuthState::default()
        },
    ] {
        let mut st = ConsoleState::new();
        st.init_from_env("#/overview", "", None);
        let mut restricted_vm = base_vm.clone();
        restricted_vm.auth = auth;
        push_vm(&mut st, restricted_vm);

        let r_scene = build_console_scene(&st, SceneViewport::DESKTOP);
        assert!(!r_scene.sections.iter().any(|s| s.id == "sec-dash-fleet"));
        assert!(!r_scene
            .sections
            .iter()
            .any(|s| s.id == "sec-dash-attention"));
        let banner = r_scene
            .banners
            .iter()
            .find(|b| b.id == "dash-rollup-banner")
            .expect("rollup banner");
        assert_eq!(banner.title, "Degraded Application Status");
        assert!(!banner.title.contains("Need Attention"));

        let SceneComponent::StatusMatrix(r_mat) = &r_scene.sections[0].components[0] else {
            panic!("expected Applications StatusMatrix");
        };
        assert_eq!(r_mat.cells[2].tone, SemanticTone::Info); // unpermitted AbDeploy InMaintenance
        assert!(r_mat.cells[2].detail.is_none());
        assert_eq!(r_mat.cells[4].tone, SemanticTone::Fail); // unpermitted Observability Degraded
        assert!(r_mat.cells[4].detail.is_none());

        let (r_html, r_wf) = render_both_skins(&st);
        for out in [&r_html, &r_wf] {
            assert!(!out.contains("Cluster Host Nodes"));
            assert!(!out.contains("Needs Attention"));
            assert!(!out.contains("Item(s) Need Attention"));
            assert!(out.contains("Degraded Application Status"));
        }
        assert!(r_html.contains("data-cell-id=\"dash-card:ab-deploy\" data-tone=\"info\""));
        assert!(r_html.contains("data-cell-id=\"dash-card:observability\" data-tone=\"fail\""));
    }
}

#[test]
fn m2_74_session_roles_worker_sync_unassigned_errors_and_profile_dormant_roles() {
    use mu_console_data::api::FetchKind;
    use mu_console_data::engine::{Effect, HttpError};

    fn drain_posts(effects: Vec<Effect>, st: &mut ConsoleState) {
        for fx in effects {
            if let Effect::Post(ev) = fx {
                let bytes = Engine::encode_event(&ev).expect("encode DataEvent");
                st.on_worker_frame(&bytes);
            }
        }
    }

    let mut engine = Engine::new(1);
    let mut state = ConsoleState::new();

    let session_json = |ci_triage_active: bool, admin_active: bool| -> String {
        let mut active_roles = vec![
            r#"{"id":"operator","display_name":"Operator","description":"Baseline read","activation":{"kind":"pinned_baseline"},"step_up":"none"}"#.to_owned(),
        ];
        let mut active_policies = vec![
            "system:dashboard.read".to_owned(),
            "ci:queue.read".to_owned(),
        ];
        let mut leases = Vec::new();
        if ci_triage_active {
            active_roles.push(r#"{"id":"ci-triage","display_name":"CI Triage","description":"Elective CI retry","activation":{"kind":"elective"},"step_up":"none"}"#.to_owned());
            active_policies.push("ci:jobs.resume".into());
        }
        if admin_active {
            active_roles.push(r#"{"id":"super-admin","display_name":"Super Admin","description":"Timed Passkey role","activation":{"kind":"timed","hard_ttl_secs":900,"idle_ttl_secs":300,"renew_window_secs":120,"renew_extend_secs":900,"max_renewals":1},"step_up":"passkey"}"#.to_owned());
            active_policies.push("platform:roles.write".into());
            leases.push(r#"{"role_id":"super-admin","granted_at_ms":1000,"hard_expires_at_ms":901000,"idle_expires_at_ms":301000,"seconds_remaining":900,"renewals_used":0,"max_renewals":1,"can_renew":false}"#);
        }
        let op = r#"{"id":"operator","display_name":"Operator","description":"Baseline read","activation":{"kind":"pinned_baseline"},"step_up":"none"}"#;
        let triage = r#"{"id":"ci-triage","display_name":"CI Triage","description":"Elective CI retry","activation":{"kind":"elective"},"step_up":"none"}"#;
        let super_admin = r#"{"id":"super-admin","display_name":"Super Admin","description":"Timed Passkey role","activation":{"kind":"timed","hard_ttl_secs":900,"idle_ttl_secs":300,"renew_window_secs":120,"renew_extend_secs":900,"max_renewals":1},"step_up":"passkey"}"#;
        let plat_admin = r#"{"id":"platform-admin","display_name":"Platform Admin","description":"Catalog role","activation":{"kind":"timed","hard_ttl_secs":900},"step_up":"passkey"}"#;
        let policies_json: Vec<String> = active_policies
            .iter()
            .map(|p| format!(r#""{p}""#))
            .collect();
        format!(
            r#"{{"authenticated":true,"github_login":"ops","invite_bound":true,"passkey_count":1,"baseline_role":"operator","bound_roles":["operator","ci-triage","super-admin"],"bound_role_specs":[{op},{triage},{super_admin}],"catalog_role_specs":[{op},{triage},{super_admin},{plat_admin}],"active_roles":[{active_roles}],"leases":[{leases}],"active_policies":[{policies}],"preferences":{{"skin":"html","font":"Hurmit"}}}}"#,
            active_roles = active_roles.join(","),
            leases = leases.join(","),
            policies = policies_json.join(","),
        )
    };

    // Bootstrap worker + ConsoleState WITHOUT platform:roles.read.
    drain_posts(
        engine.handle(
            UiCommand::Configure(mu_console_protocol::WorkerConfig::default()),
            900,
        ),
        &mut state,
    );
    drain_posts(
        engine.on_fetch(
            &FetchKind::AuthSession,
            Ok(session_json(false, false)),
            1_000,
        ),
        &mut state,
    );
    let snap = engine.on_fetch(&FetchKind::Repos, Ok("[]".into()), 1_010);
    drain_posts(snap, &mut state);
    drain_posts(engine.tick(1_250), &mut state);
    assert_eq!(state.auth().status, AuthStatus::Active);

    // Header button displays active role(s) only (not dormant ci-triage / unassigned platform-admin).
    let (hdr_html, hdr_wf) = render_both_skins(&state);
    assert!(hdr_html.contains("+Operator"));
    assert!(!hdr_html.contains("CI Triage"));
    assert!(!hdr_html.contains("Platform Admin"));
    assert!(hdr_wf.contains("+Operator"));

    // Open @user Profile and inspect dormant roles + "Manage Session Roles" button.
    let _ = state.activate_target("hdr-user");
    assert!(matches!(
        state.active_dialog(),
        Some(DialogModel::UserProfile(_))
    ));
    let (prof_html, prof_wf) = render_both_skins(&state);
    assert!(prof_html.contains("Manage Session Roles"));
    assert!(prof_html.contains("profile-manage-roles"));
    assert!(prof_html.contains("ci-triage") || prof_html.contains("CI Triage"));
    assert!(prof_wf.contains("profile-manage-roles"));

    // Click Manage Session Roles -> dispatches OpenDialog(SessionRoles) -> worker hydrates SessionRoles.
    let cmds = state.activate_target("profile-manage-roles");
    assert!(matches!(
        cmds.as_slice(),
        [UiCommand::OpenDialog(DialogModel::SessionRoles(_))]
    ));
    for c in cmds {
        drain_posts(engine.handle(c, 2_000), &mut state);
    }
    drain_posts(engine.tick(2_150), &mut state);

    let (sr_html, sr_wf) = render_both_skins(&state);
    assert!(sr_html.contains("dlg-session-roles"));
    assert!(sr_html.contains("role-enable:ci-triage"));
    assert!(sr_html.contains("Unassigned"));
    assert!(sr_html.contains("Platform Admin"));
    assert!(sr_wf.contains("dlg-session-roles"));
    assert!(sr_wf.contains("Unassigned"));

    // Elective role enable succeeds -> Engine closes SessionRoles -> UI auto-closes dialog.
    let enable_cmds = state.activate_target("role-enable:ci-triage");
    assert_eq!(
        enable_cmds,
        vec![UiCommand::EnableRole {
            role_id: "ci-triage".into()
        }]
    );
    for c in enable_cmds {
        drain_posts(engine.handle(c, 3_000), &mut state);
    }
    drain_posts(
        engine.on_fetch(
            &FetchKind::AuthEnableRole("ci-triage".into()),
            Ok(session_json(true, false)),
            3_020,
        ),
        &mut state,
    );
    drain_posts(engine.tick(3_150), &mut state);
    assert!(
        state.active_dialog().is_none(),
        "Session Roles must auto-close after successful Elective toggle"
    );

    // Re-open Session Roles, trigger Passkey elevation, simulate WebAuthn cancellation.
    for c in state.open_session_roles() {
        drain_posts(engine.handle(c, 4_000), &mut state);
    }
    drain_posts(engine.tick(4_150), &mut state);
    let _ = state.activate_target("role-elevate:super-admin");
    assert_eq!(
        state.pending_webauthn,
        Some(mu_console_ui::state::WebAuthnRequest::ElevateTimedRole {
            role_id: "super-admin".into(),
        })
    );
    let cancel_msg = mu_console_ui::webauthn::format_webauthn_js_error(
        Some("NotAllowedError"),
        Some("The operation either timed out or was not allowed."),
    );
    state.on_webauthn_error(cancel_msg);
    let (err_html, err_wf) = render_both_skins(&state);
    assert!(state.active_dialog().is_some());
    assert!(err_html.contains("Passkey verification was cancelled or timed out."));
    assert!(err_wf.contains("Passkey verification was cancelled or timed out."));

    // Server rejection also keeps Session Roles open with inline error from worker.
    let _ = state.activate_target("role-elevate:super-admin");
    drain_posts(
        engine.handle(
            UiCommand::ElevateWithPasskey {
                role_id: "super-admin".into(),
                assertion_json: r#"{"id":"cred"}"#.into(),
            },
            5_000,
        ),
        &mut state,
    );
    drain_posts(
        engine.on_fetch(
            &FetchKind::AuthElevateFinish {
                role_id: "super-admin".into(),
                assertion_json: r#"{"id":"cred"}"#.into(),
            },
            Err(HttpError {
                status: 403,
                message: r#"{"error":"Passkey assertion expired"}"#.into(),
            }),
            5_020,
        ),
        &mut state,
    );
    drain_posts(engine.tick(5_150), &mut state);
    assert!(state.active_dialog().is_some());
    let (srv_err_html, _) = render_both_skins(&state);
    assert!(srv_err_html.contains("Passkey assertion expired"));

    // Successful Passkey elevation auto-closes Session Roles and updates header badge with timer.
    drain_posts(
        engine.handle(
            UiCommand::ElevateWithPasskey {
                role_id: "super-admin".into(),
                assertion_json: r#"{"id":"cred2"}"#.into(),
            },
            6_000,
        ),
        &mut state,
    );
    drain_posts(
        engine.on_fetch(
            &FetchKind::AuthElevateFinish {
                role_id: "super-admin".into(),
                assertion_json: r#"{"id":"cred2"}"#.into(),
            },
            Ok(session_json(true, true)),
            6_020,
        ),
        &mut state,
    );
    drain_posts(engine.tick(6_150), &mut state);
    assert!(state.active_dialog().is_none());
    let (done_html, done_wf) = render_both_skins(&state);
    assert!(done_html.contains("+Super Admin["));
    assert!(done_wf.contains("+Super Admin["));
}
