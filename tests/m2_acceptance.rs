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
    for row in &att_grid.rows {
        assert!(
            matches!(
                row.cells[3].text.as_str(),
                "Degraded" | "Down" | "Missing" | "Failed"
            ),
            "unexpected Needs Attention Status label: {:?}",
            row.cells[3].text
        );
    }

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

#[test]
fn m2_86_needs_attention_status_labels_and_target_placement() {
    use mu_console_protocol::{FabricNode, ProblemCategory, SloProblem};
    use mu_console_theme::scene::{SceneComponent, SceneViewport};
    use mu_console_ui::scene::build_console_scene;

    let mut st = active_state(
        "ops",
        vec![
            "system:dashboard.read",
            "ci:queue.read",
            "ci:jobs.read",
            "ci:repos.public.read",
            "ci:nodes.read",
            "obs:problems.read",
        ],
    );
    st.init_from_env("#/dashboard/overview", "", None);

    let mut vm = ViewModel {
        auth: st.auth(),
        ci_jobs: vec![CiJobSummary {
            job_id: 99,
            repo: RepoId::new("LMA1980", "mu-console"),
            ref_name: "main".into(),
            queue_state: "failed".into(),
            tests_passed: 8,
            tests_total: 9,
            ..CiJobSummary::default()
        }],
        nodes: vec![FabricNode {
            id: "worker-riscv64".into(),
            online: false,
            arch: "riscv64".into(),
            active_slots: 0,
            max_slots: 1,
            cpu_load_bp: 0,
            ..FabricNode::default()
        }],
        ..ViewModel::default()
    };
    vm.observe.problems = vec![
        SloProblem {
            problem_id: "PROB-LAT-99".into(),
            title: "Gateway p99 latency above SLO".into(),
            category: ProblemCategory::LatencyRegression,
            severity: "critical".into(),
            impacted_entity: "mu-ingress".into(),
            active: true,
            ..SloProblem::default()
        },
        SloProblem {
            problem_id: "PROB-CANARY-99".into(),
            title: "Canary Taint / Crash Telemetry on mu-ci-controller".into(),
            category: ProblemCategory::CanaryTaint,
            severity: "warning".into(),
            impacted_entity: "mu-ci-controller".into(),
            active: true,
            ..SloProblem::default()
        },
        SloProblem {
            problem_id: "PROB-HEARTBEAT-node-a".into(),
            title: "Node Heartbeat Staleness on node-a".into(),
            category: ProblemCategory::ErrorRateSpike,
            severity: "warning".into(),
            impacted_entity: "node-a".into(),
            active: true,
            ..SloProblem::default()
        },
        SloProblem {
            problem_id: "PROB-ERR-99".into(),
            title: "Worker offline error spike".into(),
            category: ProblemCategory::ErrorRateSpike,
            severity: "error".into(),
            impacted_entity: "mu-observable-hub".into(),
            active: true,
            ..SloProblem::default()
        },
    ];
    push_vm(&mut st, vm);

    let scene = build_console_scene(&st, SceneViewport::DESKTOP);
    let att_sec = scene
        .sections
        .iter()
        .find(|s| s.id == "sec-dash-attention")
        .expect("Needs Attention section");
    let SceneComponent::TableGrid(grid) = &att_sec.components[0] else {
        panic!("expected dash-attention-table TableGrid");
    };
    assert_eq!(grid.id, "dash-attention-table");
    let headers: Vec<&str> = grid.columns.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(headers, vec!["Scope", "Target", "Finding", "Status"]);
    assert_eq!(grid.rows.len(), 6);

    let allowed = ["Degraded", "Down", "Missing", "Failed"];
    let forbidden_raw = ["critical", "warning", "error", "Offline", "Active"];
    for row in &grid.rows {
        let target = row.cells[1].text.as_str();
        let status = row.cells[3].text.as_str();
        assert!(
            allowed.contains(&status),
            "row {} had invalid Status {status:?}",
            row.id
        );
        assert_ne!(status, target, "row {} leaked target into Status", row.id);
        assert!(
            !forbidden_raw.contains(&status),
            "row {} exposed raw status {status:?}",
            row.id
        );
    }

    let find_row = |id: &str| {
        grid.rows
            .iter()
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("missing row {id}"))
    };
    assert_eq!(find_row("att-ci-99").cells[0].text, "CI Job");
    assert_eq!(find_row("att-ci-99").cells[2].text, "8/9 tests passed");
    assert_eq!(find_row("att-ci-99").cells[3].text, "Failed");
    assert_eq!(find_row("att-obs-PROB-LAT-99").cells[1].text, "mu-ingress");
    assert_eq!(find_row("att-obs-PROB-LAT-99").cells[3].text, "Degraded");
    assert_eq!(
        find_row("att-obs-PROB-CANARY-99").cells[1].text,
        "mu-ci-controller"
    );
    assert_eq!(find_row("att-obs-PROB-CANARY-99").cells[3].text, "Down");
    assert_eq!(
        find_row("att-obs-PROB-HEARTBEAT-node-a").cells[1].text,
        "node-a"
    );
    assert_eq!(
        find_row("att-obs-PROB-HEARTBEAT-node-a").cells[3].text,
        "Missing"
    );
    assert_eq!(
        find_row("att-obs-PROB-ERR-99").cells[1].text,
        "mu-observable-hub"
    );
    assert_eq!(find_row("att-obs-PROB-ERR-99").cells[3].text, "Down");
    assert_eq!(
        find_row("att-fleet-worker-riscv64").cells[1].text,
        "worker-riscv64"
    );
    assert_eq!(find_row("att-fleet-worker-riscv64").cells[3].text, "Down");

    // Verify rendered HTML and Wireframe SVG output on #/dashboard/overview.
    let (html_out, wf_out) = render_both_skins(&st);
    let att_html_start = html_out
        .find("data-component-id=\"dash-attention-table\"")
        .expect("dash-attention-table in HTML");
    let att_html_tail = &html_out[att_html_start..];
    let att_html_end = att_html_tail
        .find("</table>")
        .expect("closing table for dash-attention-table");
    let att_table_html = &att_html_tail[..att_html_end];
    for lbl in ["Degraded", "Down", "Missing", "Failed"] {
        assert!(att_table_html.contains(lbl), "HTML missing {lbl}");
        assert!(wf_out.contains(lbl), "Wireframe missing {lbl}");
    }
    for raw in [
        ">critical<",
        ">warning<",
        ">error<",
        ">Offline<",
        ">Active<",
    ] {
        assert!(
            !att_table_html.contains(raw),
            "HTML attention table leaked raw status {raw}"
        );
    }
}

#[test]
fn m2_91_timer_only_dom_patching_preserves_banner_and_navigation_under_super_operator() {
    use mu_console_protocol::{Delta, StepUpRequirement};
    use mu_console_ui::client::ClientUpdate;
    use mu_console_ui::components::{render_html_shell, strip_lease_timer_values};
    use mu_console_ui::roles::resolve_lease_timer_text;

    let mut state = ConsoleState::new();
    let super_spec = RoleSpecDto {
        id: "super-operator".into(),
        display_name: Some("Super Operator".into()),
        description: "All-access Passkey role".into(),
        activation: RoleActivationDto::super_operator(),
        step_up: StepUpRequirement::Passkey,
    };
    let make_lease = |rem: u64, idle: u64, act_ms: u64| RoleLeaseView {
        role_id: "super-operator".into(),
        display_name: Some("Super Operator".into()),
        activated_at_ms: 0,
        last_activity_at_ms: act_ms,
        hard_expires_at_ms: 900_000,
        idle_expires_at_ms: act_ms + idle * 1_000,
        hard_ttl_secs: 900,
        idle_ttl_secs: 120,
        renew_window_secs: 120,
        renew_extend_secs: 900,
        max_renewals: 1,
        renewals_used: 0,
        seconds_remaining: rem,
        idle_timer: idle,
        can_renew: true,
    };
    let make_auth = |rem: u64, idle: u64, act_ms: u64| {
        let mut a = AuthState::from_session(&AuthSessionDto {
            authenticated: true,
            github_login: Some("LMA1980".into()),
            invite_bound: true,
            passkey_count: 1,
            baseline_role: Some("operator".into()),
            bound_roles: vec!["operator".into(), "super-operator".into()],
            bound_role_specs: vec![super_spec.clone()],
            active_roles: vec![super_spec.clone()],
            leases: vec![make_lease(rem, idle, act_ms)],
            active_policies: vec![
                "system:dashboard.read".into(),
                "ci:queue.read".into(),
                "platform:users.read".into(),
            ],
            ..AuthSessionDto::default()
        });
        a.leases[0].can_renew = true;
        if let Some(l) = a.active_roles[0].lease.as_mut() {
            l.can_renew = true;
        }
        a
    };

    let init_vm = ViewModel {
        auth: make_auth(120, 118, 0),
        user_profile: Some(UserProfileDto {
            github_login: "LMA1980".into(),
            display_name: Some("LMA1980".into()),
            preferences: UserPreferencesDto::default(),
            bound_roles: vec!["operator".into(), "super-operator".into()],
            passkey_count: 1,
            passkeys: vec![],
        }),
        ..ViewModel::default()
    };
    push_vm(&mut state, init_vm);

    // 1. Both Desktop and Phone HTML/Wireframe outputs emit [data-lease-timer] nodes and valid XML.
    let desktop = Viewport::new(1512.0, 860.0);
    let phone = Viewport::new(402.0, 720.0);
    let html_t0 = render_console_with_skin(ConsoleSkin::Html, &state, desktop);
    let wf_t0 = render_console_with_skin(ConsoleSkin::Wireframe, &state, desktop);
    let wf_phone_t0 = render_console_with_skin(ConsoleSkin::Wireframe, &state, phone);
    let shell_t0 = render_html_shell("<svg></svg>", &state);
    roxmltree::Document::parse(&wf_t0).expect("desktop wireframe XML");
    roxmltree::Document::parse(&wf_phone_t0).expect("phone wireframe XML");
    roxmltree::Document::parse(&shell_t0).expect("shell XML");

    assert!(shell_t0.contains(r#"data-lease-timer="hdr:super-operator""#));
    assert!(html_t0.contains(r#"data-lease-timer="hdr:super-operator""#));
    assert!(html_t0.contains(r#"data-lease-timer="renew-rem:super-operator""#));
    assert!(html_t0.contains(r#"data-lease-timer="renew-idle:super-operator""#));
    assert!(wf_t0.contains(r#"data-lease-timer="renew-rem:super-operator""#));
    assert!(wf_t0.contains(r#"data-lease-timer="renew-idle:super-operator""#));

    // When a Wireframe header badge fits the countdown inside its 340px header strip,
    // WireframeSkin also wraps ONLY the `[MM:SS][MM:SS]` or truncated prefix in `<tspan>`.
    let mut short_auth = make_auth(120, 118, 0);
    short_auth.active_roles[0].display_name = Some("S".into());
    short_auth.leases[0].display_name = Some("S".into());
    let mut short_st = ConsoleState::new();
    push_vm(
        &mut short_st,
        ViewModel {
            auth: short_auth,
            ..ViewModel::default()
        },
    );
    let short_wf = render_console_with_skin(ConsoleSkin::Wireframe, &short_st, desktop);
    roxmltree::Document::parse(&short_wf).expect("short wireframe XML");
    assert!(
        short_wf.contains(r#"data-lease-timer="hdr:super-operator""#)
            || short_wf.contains(r#"data-lease-timer="hdr-trunc:super-operator:"#),
        "expected Wireframe hdr <tspan> timer in {short_wf}"
    );

    // 2. Worker countdown patch returns ClientUpdate::LeaseTimers and leaves stripped scene & banner byte-identical.
    let patch_t1 = encode(&DataEvent::Patch(vec![Delta::Auth(make_auth(119, 117, 0))])).unwrap();
    assert_eq!(state.on_worker_frame(&patch_t1), ClientUpdate::LeaseTimers);

    let html_t1 = render_console_with_skin(ConsoleSkin::Html, &state, desktop);
    let wf_t1 = render_console_with_skin(ConsoleSkin::Wireframe, &state, desktop);
    assert_ne!(html_t0, html_t1);
    assert_eq!(
        strip_lease_timer_values(&html_t0),
        strip_lease_timer_values(&html_t1),
        "HTML banner and scene must not structurally change on 1s timer tick"
    );
    assert_eq!(
        strip_lease_timer_values(&wf_t0),
        strip_lease_timer_values(&wf_t1),
        "Wireframe banner and scene must not structurally change on 1s timer tick"
    );
    assert_eq!(
        strip_lease_timer_values(&state.format_header_roles_html()),
        strip_lease_timer_values(
            r#"+Super Operator<span class="mu-lease-timer" data-lease-timer="hdr:super-operator">[02:00][01:58]</span>"#
        )
    );

    // 3. Explicit activity touch resets idle_timer cleanly (117s -> 119s) and resolves updated [data-lease-timer] values.
    let _ = state.on_explicit_user_activity(781_000);
    let cur_auth = state.auth();
    assert_eq!(
        resolve_lease_timer_text(&cur_auth, "renew-rem:super-operator").as_deref(),
        Some("01:59")
    );
    assert_eq!(
        resolve_lease_timer_text(&cur_auth, "renew-idle:super-operator").as_deref(),
        Some("01:59")
    );

    // 4. Open SessionRoles and UserProfile dialogs also carry [data-lease-timer] spans/tspans without structural churn.
    let _ = state.open_session_roles();
    let sr_html_0 = render_console_with_skin(ConsoleSkin::Html, &state, desktop);
    let sr_wf_0 = render_console_with_skin(ConsoleSkin::Wireframe, &state, desktop);
    roxmltree::Document::parse(&sr_wf_0).expect("SessionRoles wireframe XML");
    assert!(sr_html_0.contains(r#"data-lease-timer="dlg-rem:super-operator""#));
    assert!(
        sr_wf_0.contains(r#"data-lease-timer="dlg-rem:super-operator""#)
            || sr_wf_0.contains(r#"data-lease-timer="dlg-rem-trunc:super-operator:"#),
        "expected SessionRoles wireframe timer <tspan> in {sr_wf_0}"
    );

    let patch_t2 = encode(&DataEvent::Patch(vec![Delta::Auth(make_auth(118, 116, 0))])).unwrap();
    assert_eq!(state.on_worker_frame(&patch_t2), ClientUpdate::LeaseTimers);
    let sr_html_1 = render_console_with_skin(ConsoleSkin::Html, &state, desktop);
    let sr_wf_1 = render_console_with_skin(ConsoleSkin::Wireframe, &state, desktop);
    assert_eq!(
        strip_lease_timer_values(&sr_html_0),
        strip_lease_timer_values(&sr_html_1)
    );
    assert_eq!(
        strip_lease_timer_values(&sr_wf_0),
        strip_lease_timer_values(&sr_wf_1)
    );
    let _ = state.dismiss_dialog();

    let _ = state.open_user_profile(UserProfileTab::UserDetails);
    let up_html_0 = render_console_with_skin(ConsoleSkin::Html, &state, desktop);
    assert!(up_html_0.contains(r#"data-lease-timer="profile-secs:super-operator""#));
    let patch_t3 = encode(&DataEvent::Patch(vec![Delta::Auth(make_auth(117, 115, 0))])).unwrap();
    assert_eq!(state.on_worker_frame(&patch_t3), ClientUpdate::LeaseTimers);
    let up_html_1 = render_console_with_skin(ConsoleSkin::Html, &state, desktop);
    assert_eq!(
        strip_lease_timer_values(&up_html_0),
        strip_lease_timer_values(&up_html_1)
    );
    let _ = state.dismiss_dialog();

    // 5. Single-click navigation works immediately while super-operator lease is active.
    let nav_cmds = state.on_dom_action("data-area", "platform", false, 783_000);
    assert!(!nav_cmds.is_empty());
    assert_eq!(state.route.area, Area::Platform);
}

#[test]
fn m2_93_ci_job_scope_and_live_application_cards_exclude_ci_inner_step_alerts() {
    use mu_console_protocol::{
        ObservabilitySubView, ProblemCategory, SloProblem, SubsystemHealthEntry,
        SubsystemHealthSummary, SubsystemStatus,
    };
    use mu_console_theme::scene::SceneComponent;
    use mu_console_theme::skin::SemanticTone;
    use mu_console_ui::scene::build_console_scene;

    let mut st = active_state(
        "ops",
        vec![
            "system:dashboard.read",
            "ci:queue.read",
            "ci:jobs.read",
            "ci:repos.public.read",
            "obs:problems.read",
        ],
    );
    st.init_from_env("#/dashboard/overview", "", None);

    let mut vm = ViewModel {
        auth: st.auth(),
        subsystem_health: SubsystemHealthSummary {
            ci: SubsystemHealthEntry {
                status: SubsystemStatus::Live,
                summary: "Operational".into(),
                ..SubsystemHealthEntry::default()
            },
            observability: SubsystemHealthEntry {
                status: SubsystemStatus::Live,
                summary: "Operational".into(),
                ..SubsystemHealthEntry::default()
            },
            ..SubsystemHealthSummary::default()
        },
        ci_jobs: vec![CiJobSummary {
            job_id: 202,
            repo: RepoId::new("LMA1980", "jero-core"),
            ref_name: "refs/heads/main".into(),
            queue_state: "failed".into(),
            tests_passed: 639,
            tests_total: 639,
            stages: vec![CiJobStage {
                id: "regression-gate".into(),
                name: "Regression Gate".into(),
                assigned_node: Some("node-a".into()),
                threads: 4,
                state: StageState::Failed,
                duration_secs: 2,
                exit_code: 1,
                skip_reason: None,
            }],
            ..CiJobSummary::default()
        }],
        ..ViewModel::default()
    };
    vm.observe.problems = vec![
        SloProblem {
            problem_id: "PROB-TEST-jero-core-miri".into(),
            title: "Flaky / Failing Test jero-core::shm (cargo-miri)".into(),
            category: ProblemCategory::FlakyTest,
            severity: "warning".into(),
            impacted_entity: "lma1980/jero-core".into(),
            active: true,
            ..SloProblem::default()
        },
        SloProblem {
            problem_id: "PROB-BENCH-jero-core-dot".into(),
            title: "Benchmark Regression in jero-core::dot".into(),
            category: ProblemCategory::LatencyRegression,
            severity: "error".into(),
            impacted_entity: "lma1980/jero-core".into(),
            active: true,
            ..SloProblem::default()
        },
    ];
    push_vm(&mut st, vm);

    let scene = build_console_scene(&st, mu_console_theme::scene::SceneViewport::DESKTOP);
    let app_sec = scene
        .sections
        .iter()
        .find(|s| s.id == "sec-dash-applications")
        .expect("Applications section");
    let SceneComponent::StatusMatrix(mat) = &app_sec.components[0] else {
        panic!("expected StatusMatrix");
    };
    assert_eq!(mat.cells[0].value, "Live");
    assert_eq!(mat.cells[0].tone, SemanticTone::Pass);
    assert_eq!(mat.cells[4].value, "Live");
    assert_eq!(mat.cells[4].tone, SemanticTone::Pass);

    let att_sec = scene
        .sections
        .iter()
        .find(|s| s.id == "sec-dash-attention")
        .expect("Needs Attention section");
    let SceneComponent::TableGrid(grid) = &att_sec.components[0] else {
        panic!("expected dash-attention-table");
    };
    assert_eq!(grid.rows.len(), 1);
    assert_eq!(grid.rows[0].id, "att-ci-202");
    assert_eq!(grid.rows[0].cells[0].text, "CI Job");
    assert_eq!(
        grid.rows[0].cells[2].text,
        "Stage 'regression-gate' failed (exit 1)"
    );
    assert_eq!(grid.rows[0].cells[3].text, "Failed");

    let (html_out, wf_out) = render_both_skins(&st);
    assert!(html_out.contains("CI Job"));
    assert!(wf_out.contains("CI Job"));
    assert!(!html_out.contains("PROB-TEST-"));
    assert!(!wf_out.contains("PROB-TEST-"));
    assert!(!html_out.contains("Flaky / Failing Test"));
    assert!(!wf_out.contains("Flaky / Failing Test"));

    st.select_subview(SubView::Observability(ObservabilitySubView::ProblemsSlos));
    let (obs_html, obs_wf) = render_both_skins(&st);
    assert!(obs_html.contains("Zero open SLO alerts"));
    assert!(!obs_wf.contains("PROB-TEST-"));
    assert!(!obs_wf.contains("Flaky / Failing Test"));
    assert!(!obs_wf.contains("Benchmark Regression"));
}

#[test]
fn m2_95_active_failing_test_surfaces_across_skins_and_clears_immediately_when_fixed() {
    use mu_console_protocol::{
        ObservabilitySubView, ProblemCategory, SloProblem, SubsystemHealthEntry,
        SubsystemHealthSummary, SubsystemStatus,
    };
    use mu_console_theme::scene::SceneComponent;
    use mu_console_theme::skin::SemanticTone;
    use mu_console_ui::scene::build_console_scene;

    let mut st = active_state(
        "ops",
        vec![
            "system:dashboard.read",
            "ci:queue.read",
            "ci:jobs.read",
            "ci:repos.public.read",
            "obs:problems.read",
        ],
    );
    st.init_from_env("#/dashboard/overview", "", None);

    let auth = st.auth();
    let mk_vm = |jobs: Vec<CiJobSummary>, problems: Vec<SloProblem>| ViewModel {
        auth: auth.clone(),
        subsystem_health: SubsystemHealthSummary {
            ci: SubsystemHealthEntry {
                status: SubsystemStatus::Live,
                summary: "Operational".into(),
                ..SubsystemHealthEntry::default()
            },
            observability: SubsystemHealthEntry {
                status: SubsystemStatus::Live,
                summary: "Operational".into(),
                ..SubsystemHealthEntry::default()
            },
            ..SubsystemHealthSummary::default()
        },
        ci_jobs: jobs,
        observe: mu_console_protocol::ObserveView {
            problems,
            ..mu_console_protocol::ObserveView::default()
        },
        ..ViewModel::default()
    };

    // 1. Failing test (severity = "error") and failing CI job surface across both skins without degrading Live cards.
    push_vm(
        &mut st,
        mk_vm(
            vec![CiJobSummary {
                job_id: 301,
                repo: RepoId::new("LMA1980", "jero-core"),
                ref_name: "main".into(),
                queue_state: "failed".into(),
                tests_passed: 638,
                tests_total: 639,
                ..CiJobSummary::default()
            }],
            vec![SloProblem {
                problem_id: "PROB-TEST-jero-core-cargo-test-test_alpha".into(),
                title: "Failing Test jero-core::test_alpha (cargo-test)".into(),
                category: ProblemCategory::FlakyTest,
                severity: "error".into(),
                impacted_entity: "lma1980/jero-core".into(),
                active: true,
                ..SloProblem::default()
            }],
        ),
    );

    let (dash_html_1, dash_wf_1) = render_both_skins(&st);
    assert!(dash_html_1.contains("CI Test"));
    assert!(dash_wf_1.contains("CI Test"));
    assert!(dash_html_1.contains("Failing Test jero-core::test_alpha"));
    assert!(dash_wf_1.contains("Failing Test jero-core::test_alpha"));

    let scene1 = build_console_scene(&st, mu_console_theme::scene::SceneViewport::DESKTOP);
    let app_sec1 = scene1
        .sections
        .iter()
        .find(|s| s.id == "sec-dash-applications")
        .unwrap();
    let SceneComponent::StatusMatrix(mat1) = &app_sec1.components[0] else {
        panic!("expected StatusMatrix");
    };
    assert_eq!(mat1.cells[4].value, "Live");
    assert_eq!(mat1.cells[4].tone, SemanticTone::Pass);

    st.select_subview(SubView::Observability(ObservabilitySubView::ProblemsSlos));
    let (obs_html_1, obs_wf_1) = render_both_skins(&st);
    assert!(obs_html_1.contains("Failing Test jero-core::test_alpha"));
    assert!(obs_wf_1.contains("Failing Test jero-core::test_alpha"));

    // 2. Passing run on newer job_id = 302 clears both CI Job and PROB-TEST alerts immediately.
    push_vm(
        &mut st,
        mk_vm(
            vec![
                CiJobSummary {
                    job_id: 302,
                    repo: RepoId::new("LMA1980", "jero-core"),
                    ref_name: "main".into(),
                    queue_state: "passed".into(),
                    tests_passed: 639,
                    tests_total: 639,
                    ..CiJobSummary::default()
                },
                CiJobSummary {
                    job_id: 301,
                    repo: RepoId::new("LMA1980", "jero-core"),
                    ref_name: "main".into(),
                    queue_state: "failed".into(),
                    tests_passed: 638,
                    tests_total: 639,
                    ..CiJobSummary::default()
                },
            ],
            Vec::new(),
        ),
    );
    let (obs_html_2, obs_wf_2) = render_both_skins(&st);
    assert!(obs_html_2.contains("Zero open SLO alerts"));
    assert!(!obs_wf_2.contains("PROB-TEST-"));
    assert!(!obs_html_2.contains("Failing Test jero-core::test_alpha"));
    assert!(!obs_wf_2.contains("Failing Test jero-core::test_alpha"));

    st.select_subview(SubView::Dashboard(DashboardSubView::Overview));
    let (dash_html_2, dash_wf_2) = render_both_skins(&st);
    assert!(!dash_html_2.contains("Failing Test jero-core::test_alpha"));
    assert!(!dash_wf_2.contains("Failing Test jero-core::test_alpha"));
    assert!(!dash_html_2.contains("#301"));
    assert!(!dash_wf_2.contains("#301"));
}

#[test]
fn observability_hides_repo_selector_in_html_and_wireframe_skins() {
    use mu_console_protocol::{ObservabilitySubView, RepoScore};

    let mut st = active_state(
        "ops",
        vec![
            "ci:queue.read",
            "ci:repos.public.read",
            "obs:smartscape.read",
            "obs:traces.read",
            "obs:problems.read",
            "obs:logs.read",
            "obs:metrics.read",
            "obs:store.read",
            "obs:ingress.read",
        ],
    );
    let mut vm = st.client.view().cloned().expect("view");
    vm.repos = vec![RepoScore::new(RepoId::new("LMA1980", "mu-registry"))];
    push_vm(&mut st, vm);

    st.select_subview(SubView::Ci(CiSubView::Scorecards));
    let (ci_html, ci_wf) = render_both_skins(&st);
    assert!(ci_html.contains("c2-repo-selector"));
    assert!(ci_html.contains("mu-repo-selector"));
    assert!(ci_html.contains("Prev Repo"));
    assert!(ci_html.contains("Prev Submodule"));
    assert!(ci_wf.contains("c2-repo-selector"));

    for subview in ObservabilitySubView::ALL {
        st.select_subview(SubView::Observability(subview));
        let (obs_html, obs_wf) = render_both_skins(&st);
        for token in [
            "c2-repo-selector",
            "mu-repo-selector",
            "Prev Repo",
            "Next Repo",
            "Prev Submodule",
            "Next Submodule",
        ] {
            assert!(
                !obs_html.contains(token),
                "HtmlCoreSkin rendered {token} in Observability {subview:?}"
            );
            assert!(
                !obs_wf.contains(token),
                "WireframeSkin rendered {token} in Observability {subview:?}"
            );
        }
    }
}

fn full_area_access_policies() -> Vec<&'static str> {
    vec![
        "system:dashboard.read",
        "ci:queue.read",
        "ci:jobs.read",
        "ci:jobs.logs.read",
        "ci:jobs.resume",
        "ci:jobs.restart",
        "ci:repos.public.read",
        "ci:repos.private.read",
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
}

fn assert_repo_selector_absent(st: &ConsoleState, ctx: &str) {
    use mu_console_theme::scene::SceneViewport;
    use mu_console_ui::scene::build_console_scene;

    let scene = build_console_scene(st, SceneViewport::DESKTOP);
    assert!(
        scene.repo_selector.is_none(),
        "{ctx}: scene.repo_selector must be None"
    );
    let (html_out, wf_out) = render_both_skins(st);
    for out in [&html_out, &wf_out] {
        for forbidden in [
            "c2-repo-selector",
            "mu-repo-selector",
            "Prev Repo",
            "Prev Submodule",
        ] {
            assert!(
                !out.contains(forbidden),
                "{ctx}: output must omit '{forbidden}'"
            );
        }
    }
}

fn assert_filter_disabled_16px(st: &ConsoleState, expected_placeholder: &str, ctx: &str) {
    assert!(
        !st.is_filter_applicable(),
        "{ctx}: filter must not be applicable"
    );
    let (html_shell, wf_shell) = render_both_skins(st);
    let doc = roxmltree::Document::parse(&wf_shell)
        .unwrap_or_else(|e| panic!("{ctx} (wireframe): invalid XML shell: {e}"));
    let filter = doc
        .descendants()
        .find(|n| n.attribute("id") == Some("muconsole-filter"))
        .unwrap_or_else(|| panic!("{ctx} (wireframe): missing #muconsole-filter"));
    assert_eq!(filter.attribute("disabled"), Some("disabled"), "{ctx}");
    assert_eq!(filter.attribute("aria-disabled"), Some("true"), "{ctx}");
    assert_eq!(
        filter.attribute("placeholder"),
        Some(expected_placeholder),
        "{ctx} (wireframe) placeholder mismatch"
    );
    assert!(
        filter
            .attribute("style")
            .is_some_and(|s| s.contains("font-size:16px")),
        "{ctx} (wireframe): disabled #muconsole-filter must preserve font-size:16px"
    );

    let html_filter_line = html_shell
        .split('<')
        .find(|tag| tag.contains("id=\"muconsole-filter\""))
        .unwrap_or_else(|| panic!("{ctx} (html): missing #muconsole-filter"));
    assert!(html_filter_line.contains("disabled=\"disabled\""), "{ctx}");
    assert!(html_filter_line.contains("aria-disabled=\"true\""), "{ctx}");
    assert!(
        html_filter_line.contains(&format!("placeholder=\"{expected_placeholder}\"")),
        "{ctx} (html) placeholder mismatch in {html_filter_line}"
    );
    assert!(
        html_filter_line.contains("font-size:16px"),
        "{ctx} (html): disabled #muconsole-filter must preserve font-size:16px"
    );
}

fn multi_area_fixture_vm(auth: AuthState) -> ViewModel {
    use mu_console_protocol::dto::{
        CdEcosystem, CdRunDto, CdRunFailurePinpointDto, CdStepDto, IngressRouteDto,
        ModelRunnerConnectionDto, ModelRunnerStatusDto, ResidentModelStatusDto,
        SubsystemHealthEntry, SubsystemHealthSummary, SubsystemStatus, WireGuardPeerSummaryDto,
    };
    use mu_console_protocol::{
        Branch, CanaryBundle, CanaryBundleState, CasHealth, CdLease, CdRunState, DeploymentGate,
        FabricNode, GateOutcome, GateTest, HardwareMetricSeries, LogSeverity, OtelLogEntry,
        PackageEcosystem, PrState, ProblemCategory, PurePathTrace, RegistryPackage, RegistryView,
        RepoScore, RepoStatus, RepoTopology, SandboxProfileSummary, SidingPhase, SloProblem, Slot,
        SmartscapeLayer, SmartscapeNode, Station, SubmoduleRef, TestState, Train,
    };

    let mut parent = RepoScore::new(RepoId::new("LMA1980", "alpha-superproject"));
    parent.default_branch = "release-alpha".into();
    parent.status = RepoStatus::Failing;
    parent.score = 82;
    parent.submodules = vec![SubmoduleRef {
        path: "code/core".into(),
        repo: RepoId::new("LMA1980", "core"),
        pinned_sha: "abc1234".into(),
    }];
    let mut beta_repo = RepoScore::new(RepoId::new("LMA1980", "beta-service"));
    beta_repo.default_branch = "main".into();
    beta_repo.status = RepoStatus::Passing;
    beta_repo.score = 99;
    let mut priv_repo = RepoScore::new(RepoId::new("LMA1980", "private-vault"));
    priv_repo.visibility = Visibility::Private;
    priv_repo.status = RepoStatus::Passing;

    let mut vm = ViewModel {
        auth,
        repos: vec![parent, beta_repo, priv_repo],
        topology: Some(RepoTopology {
            repo: RepoId::new("LMA1980", "alpha-superproject"),
            submodule: None,
            default_branch: "main".into(),
            stations: vec![
                Station {
                    id: "fmt".into(),
                    label: "FMT".into(),
                },
                Station {
                    id: "rvv-bench".into(),
                    label: "RVV".into(),
                },
            ],
            branches: vec![Branch {
                ref_name: "feat/alpha-rvv".into(),
                base_ref: "main".into(),
                fork_stage: 0,
                pr_state: PrState::Open,
                last_job: Some(101),
                phase: SidingPhase::Active,
            }],
            trains: vec![
                Train {
                    job_id: 101,
                    ref_name: "feat/alpha-rvv".into(),
                    stage: 1,
                    stage_states: vec![StageState::Passed, StageState::Running],
                    node_id: Some("worker-alpha".into()),
                    node_lost: false,
                },
                Train {
                    job_id: 102,
                    ref_name: "feat/beta-docs".into(),
                    stage: 0,
                    stage_states: vec![StageState::Passed, StageState::Passed],
                    node_id: Some("worker-beta".into()),
                    node_lost: false,
                },
            ],
            submodules: vec![],
            fabric_rev: 7,
            submodule_bump_pending: false,
        }),
        ci_jobs: vec![
            CiJobSummary {
                job_id_str: "job-101".into(),
                job_id: 101,
                repo: RepoId::new("LMA1980", "alpha-superproject"),
                ref_name: "feat/alpha-rvv".into(),
                commit_sha: "111aaaa".into(),
                queue_state: "running".into(),
                stages: vec![CiJobStage {
                    id: "rvv".into(),
                    name: "RVV Suite".into(),
                    assigned_node: Some("worker-alpha".into()),
                    threads: 8,
                    state: StageState::Running,
                    duration_secs: 14,
                    exit_code: 0,
                    skip_reason: None,
                }],
                raw_log_lines: vec![
                    "LOG_NEEDLE running rvv tile".into(),
                    "unrelated stdout line".into(),
                ],
                ..CiJobSummary::default()
            },
            CiJobSummary {
                job_id_str: "job-102".into(),
                job_id: 102,
                repo: RepoId::new("LMA1980", "beta-service"),
                ref_name: "main".into(),
                commit_sha: "222bbbb".into(),
                queue_state: "failed".into(),
                stages: vec![CiJobStage {
                    id: "clippy".into(),
                    name: "Clippy".into(),
                    assigned_node: Some("worker-beta".into()),
                    threads: 4,
                    state: StageState::Failed,
                    duration_secs: 9,
                    exit_code: 1,
                    skip_reason: None,
                }],
                raw_log_lines: vec!["clippy error line".into()],
                ..CiJobSummary::default()
            },
            CiJobSummary {
                job_id_str: "job-103".into(),
                job_id: 103,
                repo: RepoId::new("LMA1980", "beta-service"),
                ref_name: "release".into(),
                commit_sha: "333cccc".into(),
                queue_state: "passed".into(),
                ..CiJobSummary::default()
            },
        ],
        nodes: vec![
            FabricNode {
                id: "worker-alpha".into(),
                online: false,
                arch: "riscv64".into(),
                active_slots: 0,
                max_slots: 1,
                cpu_load_bp: 0,
                ..FabricNode::default()
            },
            FabricNode {
                id: "worker-beta".into(),
                online: true,
                arch: "x86_64".into(),
                active_slots: 1,
                max_slots: 1,
                cpu_load_bp: 4200,
                ..FabricNode::default()
            },
        ],
        sandboxes: vec![
            SandboxProfileSummary {
                profile_name: "sb-kvm-strict".into(),
                backend: "HypervisorSandbox".into(),
                max_pids: 256,
                max_slots: 1,
                thread_budget: 6,
                hardware_target: "riscv64".into(),
            },
            SandboxProfileSummary {
                profile_name: "sb-kernel-fast".into(),
                backend: "KernelSandbox".into(),
                max_pids: 256,
                max_slots: 1,
                thread_budget: 8,
                hardware_target: "x86_64".into(),
            },
        ],
        cd_runs: vec![
            CdRunDto {
                run_id: "cd-alpha-1".into(),
                ecosystem: CdEcosystem::Cargo,
                repo: RepoId::new("LMA1980", "alpha-superproject"),
                ref_name: "main".into(),
                state: CdRunState::Failed,
                failure: Some(CdRunFailurePinpointDto {
                    step_id: "publish-crate".into(),
                    package_name: Some("alpha-core".into()),
                    package_version: Some("0.3.0".into()),
                    reason: "ETag mismatch".into(),
                    log_excerpt: None,
                }),
                steps: vec![CdStepDto {
                    step_id: "publish-crate".into(),
                    label: "Publish Crate".into(),
                    state: CdRunState::Failed,
                    ..CdStepDto::default()
                }],
                ..CdRunDto::default()
            },
            CdRunDto {
                run_id: "cd-beta-2".into(),
                ecosystem: CdEcosystem::Maven,
                repo: RepoId::new("LMA1980", "beta-service"),
                ref_name: "main".into(),
                state: CdRunState::Succeeded,
                steps: vec![CdStepDto {
                    step_id: "deploy-pom".into(),
                    label: "Deploy Maven Artifact".into(),
                    state: CdRunState::Succeeded,
                    ..CdStepDto::default()
                }],
                ..CdRunDto::default()
            },
        ],
        gate: Some(DeploymentGate {
            deployment_id: "dep-alpha-01".into(),
            repo: RepoId::new("LMA1980", "alpha-superproject"),
            service: "alpha-api".into(),
            artifact_digest: "sha256:alpha9911".into(),
            target_node: "node-a".into(),
            staged_slot: Slot::B,
            restore_slot: Slot::A,
            lease_expires_epoch_ms: 999_000,
            outcome: GateOutcome::Pending,
            rollback_reason: None,
            tainted: false,
            tests: vec![
                GateTest {
                    name: "it_alpha_gemm".into(),
                    critical: true,
                    state: TestState::Failed,
                    duration_ms: 1_200,
                    crate_name: Some("core".into()),
                    log_excerpt: Some("GATE_NEEDLE gemm nan\nother gate line".into()),
                },
                GateTest {
                    name: "it_beta_smoke".into(),
                    critical: false,
                    state: TestState::Passed,
                    duration_ms: 300,
                    crate_name: Some("ui".into()),
                    log_excerpt: Some("smoke ok".into()),
                },
            ],
        }),
        leases: vec![
            CdLease {
                lease_id: "lease-alpha".into(),
                deployment_id: "dep-alpha-01".into(),
                target_node: "node-a".into(),
                service: "alpha-api".into(),
                artifact_digest: "sha256:alpha9911".into(),
                active_slot: Slot::A,
                candidate_slot: Slot::B,
                ttl_remaining_ms: 120_000,
                promoted: false,
            },
            CdLease {
                lease_id: "lease-beta".into(),
                deployment_id: "dep-beta-02".into(),
                target_node: "node-b".into(),
                service: "beta-worker".into(),
                artifact_digest: "sha256:beta8822".into(),
                active_slot: Slot::B,
                candidate_slot: Slot::A,
                ttl_remaining_ms: 60_000,
                promoted: true,
            },
        ],
        registry: RegistryView {
            packages: vec![
                RegistryPackage {
                    ecosystem: PackageEcosystem::Cargo,
                    name: "k3-alpha-crate".into(),
                    latest_version: "0.4.0".into(),
                    versions_count: 1,
                    sha256_prefix: "deadbeef1111".into(),
                    deps_count: 3,
                    yanked: false,
                    tainted: false,
                    taint_reason: None,
                    versions: vec!["0.4.0".into()],
                },
                RegistryPackage {
                    ecosystem: PackageEcosystem::NuGet,
                    name: "Mu.Beta.Package".into(),
                    latest_version: "2.1.0".into(),
                    versions_count: 1,
                    sha256_prefix: "cafe2222bbbb".into(),
                    deps_count: 2,
                    yanked: false,
                    tainted: true,
                    taint_reason: Some("canary fail".into()),
                    versions: vec!["2.1.0".into()],
                },
            ],
            canaries: vec![
                CanaryBundle {
                    bundle_id: "bundle-alpha-ok".into(),
                    repo: RepoId::new("LMA1980", "alpha-superproject"),
                    service: "alpha-api".into(),
                    artifact_digest: "sha256:alpha9911".into(),
                    state: CanaryBundleState::Promoted,
                    taint_reason: None,
                    deployment_id: "dep-alpha-01".into(),
                    trace_id: "tr-alpha-1".into(),
                },
                CanaryBundle {
                    bundle_id: "bundle-beta-taint".into(),
                    repo: RepoId::new("LMA1980", "beta-service"),
                    service: "beta-worker".into(),
                    artifact_digest: "sha256:beta8822".into(),
                    state: CanaryBundleState::Tainted,
                    taint_reason: Some("error rate spike".into()),
                    deployment_id: "dep-beta-02".into(),
                    trace_id: "tr-beta-2".into(),
                },
            ],
            cas: CasHealth {
                blob_count: 420,
                corrupt_blobs: 1,
                ..CasHealth::default()
            },
        },
        subsystem_health: SubsystemHealthSummary {
            ci: SubsystemHealthEntry {
                status: SubsystemStatus::Live,
                summary: "Operational".into(),
                ..SubsystemHealthEntry::default()
            },
            observability: SubsystemHealthEntry {
                status: SubsystemStatus::Degraded,
                summary: "Latency alert".into(),
                ..SubsystemHealthEntry::default()
            },
            ..SubsystemHealthSummary::default()
        },
        ..ViewModel::default()
    };

    vm.observe.smartscape_nodes = vec![
        SmartscapeNode {
            id: "svc-alpha".into(),
            label: "alpha-api".into(),
            layer: SmartscapeLayer::Service,
            req_per_sec_bp: 1200,
            error_rate_bp: 10,
            p95_ms: 18,
            healthy: true,
        },
        SmartscapeNode {
            id: "proc-beta".into(),
            label: "beta-worker".into(),
            layer: SmartscapeLayer::Process,
            req_per_sec_bp: 300,
            error_rate_bp: 50,
            p95_ms: 42,
            healthy: false,
        },
        SmartscapeNode {
            id: "host-gamma".into(),
            label: "gamma-host".into(),
            layer: SmartscapeLayer::Host,
            req_per_sec_bp: 0,
            error_rate_bp: 0,
            p95_ms: 5,
            healthy: true,
        },
    ];
    vm.observe.traces = vec![
        PurePathTrace {
            trace_id: "tr-alpha-01".into(),
            root_service: "alpha-api".into(),
            root_operation: "GET /v1/alpha".into(),
            domain: "edge".into(),
            duration_ms: 19,
            error: false,
            slow_p99: false,
            started_epoch_ms: 1_000,
            spans: vec![],
        },
        PurePathTrace {
            trace_id: "tr-beta-02".into(),
            root_service: "beta-worker".into(),
            root_operation: "POST /v1/beta".into(),
            domain: "compute".into(),
            duration_ms: 88,
            error: true,
            slow_p99: true,
            started_epoch_ms: 2_000,
            spans: vec![],
        },
    ];
    vm.observe.problems = vec![
        SloProblem {
            problem_id: "PROB-ALPHA-1".into(),
            title: "Alpha API p99 latency breach".into(),
            category: ProblemCategory::LatencyRegression,
            severity: "critical".into(),
            impacted_entity: "alpha-api".into(),
            active: true,
            ..SloProblem::default()
        },
        SloProblem {
            problem_id: "PROB-BETA-2".into(),
            title: "Beta worker crash loop".into(),
            category: ProblemCategory::ErrorRateSpike,
            severity: "error".into(),
            impacted_entity: "beta-worker".into(),
            active: true,
            ..SloProblem::default()
        },
    ];
    vm.observe.logs = vec![
        OtelLogEntry {
            timestamp_ms: 1_000,
            severity: LogSeverity::Info,
            service: "alpha-api".into(),
            span_id: None,
            trace_id: "tr-alpha-01".into(),
            message: "alpha request completed".into(),
        },
        OtelLogEntry {
            timestamp_ms: 2_000,
            severity: LogSeverity::Error,
            service: "beta-worker".into(),
            span_id: None,
            trace_id: "tr-beta-02".into(),
            message: "beta panic detected".into(),
        },
    ];
    vm.observe.metrics = vec![
        HardwareMetricSeries {
            service: "alpha-api".into(),
            hardware_class: "x86_64".into(),
            rps: 140,
            error_rate_bp: 5,
            p50_ms: 6,
            p95_ms: 18,
            p99_ms: 30,
            exemplar_trace_id: "tr-alpha-01".into(),
            samples: vec![6, 12, 18],
        },
        HardwareMetricSeries {
            service: "beta-worker".into(),
            hardware_class: "riscv64".into(),
            rps: 30,
            error_rate_bp: 200,
            p50_ms: 20,
            p95_ms: 80,
            p99_ms: 140,
            exemplar_trace_id: "tr-beta-02".into(),
            samples: vec![20, 50, 80],
        },
    ];
    vm.observe.store.hot_duckdb_rows = 12_345;
    vm.observe.ingress_routes = vec![
        IngressRouteDto {
            route_id: "ing-alpha".into(),
            path_prefix: "/v1/alpha".into(),
            upstream_url: "http://192.0.2.10:8080".into(),
            service_name: "alpha-api".into(),
            strip_prefix: false,
            auth_required: true,
            request_count: 100,
            request_rate_rps: 12.0,
            error_rate_pct: 0.1,
            p95_latency_ms: 8.5,
        },
        IngressRouteDto {
            route_id: "ing-beta".into(),
            path_prefix: "/v1/beta".into(),
            upstream_url: "http://192.0.2.20:8080".into(),
            service_name: "beta-worker".into(),
            strip_prefix: false,
            auth_required: true,
            request_count: 50,
            request_rate_rps: 4.0,
            error_rate_pct: 2.5,
            p95_latency_ms: 35.0,
        },
    ];
    vm.model_runner.status = Some(ModelRunnerStatusDto {
        models: vec![
            ResidentModelStatusDto {
                model_id: "qwen-7b".into(),
                cluster: "cluster-a100".into(),
                is_alive: true,
                mlock_bytes: 4 * 1024 * 1024 * 1024,
                ..ResidentModelStatusDto::default()
            },
            ResidentModelStatusDto {
                model_id: "llama-3b".into(),
                cluster: "cluster-x100".into(),
                is_alive: true,
                mlock_bytes: 2 * 1024 * 1024 * 1024,
                ..ResidentModelStatusDto::default()
            },
        ],
        ..ModelRunnerStatusDto::default()
    });
    vm.model_runner.connection = Some(ModelRunnerConnectionDto {
        peers: vec![
            WireGuardPeerSummaryDto {
                public_key_prefix: "pk-alpha".into(),
                allowed_ips: "10.42.0.2/32".into(),
                latest_handshake_epoch_ms: Some(1_000),
                ..WireGuardPeerSummaryDto::default()
            },
            WireGuardPeerSummaryDto {
                public_key_prefix: "pk-beta".into(),
                allowed_ips: "10.42.0.3/32".into(),
                latest_handshake_epoch_ms: None,
                ..WireGuardPeerSummaryDto::default()
            },
        ],
        ..ModelRunnerConnectionDto::default()
    });
    vm.platform = PlatformSnapshotDto {
        users: vec![
            PlatformUserDto {
                github_login: "sole-admin".into(),
                display_name: Some("Sole Admin".into()),
                passkey_count: 1,
                bound_roles: vec!["platform-admin".into()],
                is_platform_admin: true,
                last_seen_ms: None,
            },
            PlatformUserDto {
                github_login: "dev-member".into(),
                display_name: Some("Dev Member".into()),
                passkey_count: 1,
                bound_roles: vec!["operator".into()],
                is_platform_admin: false,
                last_seen_ms: None,
            },
        ],
        roles: vec![
            RoleSpecDto {
                id: "operator".into(),
                display_name: Some("Baseline Operator".into()),
                activation: RoleActivationDto::pinned_baseline(),
                ..RoleSpecDto::default()
            },
            RoleSpecDto {
                id: "super-operator".into(),
                display_name: Some("Super Operator".into()),
                activation: RoleActivationDto::super_operator(),
                ..RoleSpecDto::default()
            },
        ],
        role_folio_index: mu_console_protocol::dto::RoleFolioIndexDto {
            role_folios: [(
                "super-operator".to_owned(),
                vec!["folio-super-admin".to_owned()],
            )]
            .into_iter()
            .collect(),
            ..mu_console_protocol::dto::RoleFolioIndexDto::default()
        },
        ..PlatformSnapshotDto::default()
    };
    vm
}

#[test]
fn observability_and_non_repo_subviews_omit_repo_selector() {
    use mu_console_protocol::{ObservabilitySubView, RepositorySubView};
    use mu_console_theme::scene::SceneViewport;
    use mu_console_ui::scene::build_console_scene;

    // 1. Authorized operator holding full policies: non-repo sub-views & all 7 Observability sub-views omit repo_selector.
    let mut full = active_state("ops", full_area_access_policies());
    let vm = multi_area_fixture_vm(full.auth());
    push_vm(&mut full, vm.clone());

    for obs_sv in ObservabilitySubView::ALL {
        full.select_subview(SubView::Observability(obs_sv));
        assert_repo_selector_absent(&full, &format!("Observability::{obs_sv:?}"));
    }
    for sv in [
        SubView::Ci(CiSubView::WorkerNodes),
        SubView::Ci(CiSubView::SandboxProfiles),
        SubView::Repository(RepositorySubView::CasHealth),
        SubView::AbDeploy(AbDeploySubView::ActiveLeases),
    ] {
        full.select_subview(sv);
        assert_repo_selector_absent(&full, &format!("Non-repo {sv:?}"));
    }

    // 2. Authorized user on CI::Scorecards WITH ci:repos.public.read sees repo_selector across both skins.
    full.select_subview(SubView::Ci(CiSubView::Scorecards));
    let scorecards_scene = build_console_scene(&full, SceneViewport::DESKTOP);
    assert!(scorecards_scene.repo_selector.is_some());
    let (sc_html, sc_wf) = render_both_skins(&full);
    assert!(sc_html.contains("c2-repo-selector"));
    assert!(sc_html.contains("mu-repo-selector"));
    assert!(sc_html.contains("Prev Repo"));
    assert!(sc_html.contains("Prev Submodule"));
    assert!(sc_wf.contains("c2-repo-selector"));

    // 3. Active operator lacking ci:repos.public.read / ci:repos.private.read omits repo_selector even on repo sub-views.
    let mut no_repos = active_state(
        "no-repos",
        vec![
            "ci:queue.read",
            "ci:jobs.read",
            "cd:runs.read",
            "ab:slots.read",
            "reg:packages.read",
        ],
    );
    let mut no_repos_vm = vm;
    no_repos_vm.auth = no_repos.auth();
    push_vm(&mut no_repos, no_repos_vm);
    for sv in [
        SubView::Ci(CiSubView::Scorecards),
        SubView::Ci(CiSubView::Trainway),
        SubView::Ci(CiSubView::QueueLogs),
        SubView::Cd(mu_console_protocol::CdSubView::Runs),
        SubView::AbDeploy(AbDeploySubView::IntegrationGate),
        SubView::Repository(RepositorySubView::Packages),
    ] {
        no_repos.select_subview(sv);
        assert_repo_selector_absent(&no_repos, &format!("No ci:repos.* {sv:?}"));
    }
}

#[test]
fn anonymous_vs_passkey_required_vs_active_without_accesses() {
    use mu_console_theme::scene::SceneViewport;
    use mu_console_ui::components::filter_placeholder_for_state;
    use mu_console_ui::scene::build_console_scene;

    // 1. Anonymous: protected deep link -> "Authentication required" + Sign In; placeholders omit "alerts" / "nodes".
    let anon_auth = AuthState::anonymous(vec![
        "system:dashboard.read".into(),
        "ci:queue.read".into(),
        "ci:repos.public.read".into(),
    ]);
    let base_vm = multi_area_fixture_vm(anon_auth.clone());

    let mut anon_dash = ConsoleState::new();
    anon_dash.init_from_env("#/dashboard/overview", "", None);
    push_vm(&mut anon_dash, base_vm.clone());
    assert!(!filter_placeholder_for_state(&anon_dash).contains("alerts"));
    let (anon_dash_html, anon_dash_wf) = render_both_skins(&anon_dash);
    assert!(!anon_dash_html.contains("alerts..."));
    assert!(!anon_dash_wf.contains("alerts..."));

    let mut anon_ci = ConsoleState::new();
    anon_ci.init_from_env("#/ci/logs", "", None);
    push_vm(&mut anon_ci, base_vm.clone());
    assert!(!filter_placeholder_for_state(&anon_ci).contains("nodes"));

    let mut anon_deep = ConsoleState::new();
    anon_deep.init_from_env("#/platform/users", "", None);
    push_vm(&mut anon_deep, base_vm.clone());
    let (anon_deep_html, anon_deep_wf) = render_both_skins(&anon_deep);
    for out in [&anon_deep_html, &anon_deep_wf] {
        assert!(out.contains("Authentication required"));
        assert!(out.contains("Sign In"));
        assert!(!out.contains("Passkey enrollment required"));
        assert!(!out.contains("Access restricted"));
    }

    // 2. PasskeyEnrollmentRequired: public Dashboard + public CI > Queue & Logs only;
    //    protected deep link -> "Passkey enrollment required" + Register Passkey (never "Authentication required").
    let pk_auth = AuthState::from_session(&AuthSessionDto {
        authenticated: true,
        invite_bound: true,
        passkey_count: 0,
        passkey_enrollment_required: true,
        github_login: Some("passkey-rookie".into()),
        reason_code: Some(AuthReasonCode::PasskeyEnrollmentRequired),
        ..AuthSessionDto::default()
    });
    let mut pk_vm = base_vm;
    pk_vm.auth = pk_auth.clone();
    pk_vm.ci_jobs[0].raw_log_lines = vec!["PK_RAW_LOG_MUST_STAY_HIDDEN".into()];
    pk_vm.ci_jobs[0].stages[0].assigned_node = Some("pk-secret-worker".into());
    pk_vm.ci_jobs[0].stages[0].threads = 32;

    assert!(can_access_subview(
        &pk_auth,
        SubView::Dashboard(DashboardSubView::Overview)
    ));
    assert!(can_access_subview(
        &pk_auth,
        SubView::Ci(CiSubView::QueueLogs)
    ));
    for forbidden_ci in [
        CiSubView::Scorecards,
        CiSubView::Trainway,
        CiSubView::WorkerNodes,
        CiSubView::SandboxProfiles,
    ] {
        assert!(!can_access_subview(&pk_auth, SubView::Ci(forbidden_ci)));
    }

    let mut pk_ci = ConsoleState::new();
    pk_ci.init_from_env("#/ci/logs", "", None);
    push_vm(&mut pk_ci, pk_vm.clone());
    let _ = pk_ci.dismiss_dialog();
    assert!(!filter_placeholder_for_state(&pk_ci).contains("nodes"));
    let (pk_ci_html, pk_ci_wf) = render_both_skins(&pk_ci);
    for out in [&pk_ci_html, &pk_ci_wf] {
        assert!(out.contains(r#"data-auth-state="passkey_required""#));
        assert!(out.contains("Enroll a passkey"));
        assert!(!out.contains("nav-subview:ci:"));
        assert!(!out.contains("c2-repo-selector"));
        assert!(!out.contains("PK_RAW_LOG_MUST_STAY_HIDDEN"));
        assert!(!out.contains("pk-secret-worker"));
        assert!(!out.contains("32t"));
        assert!(!out.contains("ci-job-raw-logs"));
        assert!(!out.contains("private-vault"));
    }

    let mut pk_deep = ConsoleState::new();
    pk_deep.init_from_env("#/ci/scorecards", "", None);
    push_vm(&mut pk_deep, pk_vm);
    let _ = pk_deep.dismiss_dialog();
    let (pk_deep_html, pk_deep_wf) = render_both_skins(&pk_deep);
    for out in [&pk_deep_html, &pk_deep_wf] {
        assert!(out.contains("Passkey enrollment required"));
        assert!(out.contains("Register Passkey"));
        assert!(out.contains("auth-gate-register-passkey"));
        assert!(out.contains(r#"data-auth-state="passkey_required""#));
        assert!(!out.contains("Authentication required"));
    }

    // 3. Active without accesses: protected deep link -> "Access restricted" + Session Roles (NEVER "Authentication required").
    let op_spec = RoleSpecDto {
        id: "operator".into(),
        display_name: Some("Operator".into()),
        activation: RoleActivationDto::pinned_baseline(),
        ..RoleSpecDto::default()
    };
    let ci_spec = RoleSpecDto {
        id: "ci-operator".into(),
        display_name: Some("CI Operator".into()),
        activation: RoleActivationDto::elective(),
        ..RoleSpecDto::default()
    };
    let active_dash_only_auth = AuthState::from_session(&AuthSessionDto {
        authenticated: true,
        invite_bound: true,
        passkey_count: 1,
        github_login: Some("limited-op".into()),
        baseline_role: Some("operator".into()),
        bound_roles: vec!["operator".into(), "ci-operator".into()],
        bound_role_specs: vec![op_spec.clone(), ci_spec.clone()],
        active_roles: vec![op_spec.clone()],
        active_policies: vec!["system:dashboard.read".into()],
        ..AuthSessionDto::default()
    });
    let mut act_dash = ConsoleState::new();
    act_dash.init_from_env("#/dashboard/overview", "", None);
    push_vm(&mut act_dash, multi_area_fixture_vm(active_dash_only_auth));
    let dash_scene = build_console_scene(&act_dash, SceneViewport::DESKTOP);
    assert!(!dash_scene
        .sections
        .iter()
        .any(|s| s.id == "sec-dash-attention"));
    assert!(dash_scene
        .sections
        .iter()
        .any(|s| s.id == "sec-dash-role-hint"));
    let (act_dash_html, act_dash_wf) = render_both_skins(&act_dash);
    for out in [&act_dash_html, &act_dash_wf] {
        assert!(!out.contains("Needs Attention"));
        assert!(out.contains("Session Role Elevation"));
        assert!(out.contains("Session Roles"));
    }

    // Protected deep link while Active without area policy renders "Access restricted" + Session Roles.
    act_dash.init_from_env("#/ab-deploy/gate", "", None);
    let (act_deep_html, act_deep_wf) = render_both_skins(&act_dash);
    for out in [&act_deep_html, &act_deep_wf] {
        assert!(out.contains("Access restricted"));
        assert!(out.contains("Session Roles"));
        assert!(out.contains("Return to Dashboard"));
        assert!(!out.contains("Authentication required"));
    }

    // Active CI > Queue & Logs without ci:jobs.logs.read or ci:nodes.read hides raw logs and node/thread details.
    let mut act_ci = active_state(
        "ci-no-logs",
        vec!["ci:queue.read", "ci:jobs.read", "ci:repos.public.read"],
    );
    let mut ci_vm = multi_area_fixture_vm(act_ci.auth());
    ci_vm.ci_jobs[0].raw_log_lines = vec!["ACTIVE_RAW_LOG_SHOULD_HIDE".into()];
    ci_vm.ci_jobs[0].stages[0].assigned_node = Some("secret-ci-node".into());
    ci_vm.ci_jobs[0].stages[0].threads = 64;
    act_ci.init_from_env("#/ci/logs", "", None);
    push_vm(&mut act_ci, ci_vm);
    let (act_ci_html, act_ci_wf) = render_both_skins(&act_ci);
    for out in [&act_ci_html, &act_ci_wf] {
        assert!(out.contains("ci-job-logs-restricted"));
        assert!(!out.contains("ci-job-raw-logs"));
        assert!(!out.contains("ACTIVE_RAW_LOG_SHOULD_HIDE"));
        assert!(!out.contains("secret-ci-node"));
        assert!(!out.contains("64t"));
    }

    // Platform > Users & Invites gates invite creator and users table independently.
    let mut users_only = active_state("users-reader", vec!["platform:users.read"]);
    users_only.init_from_env("#/platform/users", "", None);
    let u_vm = multi_area_fixture_vm(users_only.auth());
    push_vm(&mut users_only, u_vm);
    let (u_html, u_wf) = render_both_skins(&users_only);
    for out in [&u_html, &u_wf] {
        assert!(out.contains("plat-users-table"));
        assert!(!out.contains("plat-invite-creator"));
    }

    let mut invites_only = active_state("invites-reader", vec!["platform:invites.read"]);
    invites_only.init_from_env("#/platform/users", "", None);
    let i_vm = multi_area_fixture_vm(invites_only.auth());
    push_vm(&mut invites_only, i_vm);
    let (i_html, i_wf) = render_both_skins(&invites_only);
    for out in [&i_html, &i_wf] {
        assert!(out.contains("plat-invite-creator"));
        assert!(!out.contains("plat-users-table"));
    }
}

fn check_disabled_filter_across_states() {
    let cold = ConsoleState::new();
    assert_filter_disabled_16px(&cold, "Resolving session...", "cold_boot");

    let mut lockdown = ConsoleState::new();
    push_vm(
        &mut lockdown,
        ViewModel {
            auth: AuthState::anonymous(vec![]),
            ..ViewModel::default()
        },
    );
    assert_filter_disabled_16px(&lockdown, "Sign in to filter...", "anon_lockdown");

    let mut denied = ConsoleState::new();
    denied.init_from_env("#/platform/users", "", None);
    push_vm(
        &mut denied,
        ViewModel {
            auth: AuthState::from_session(&AuthSessionDto {
                authenticated: false,
                invite_bound: false,
                github_login: Some("outsider".into()),
                reason_code: Some(AuthReasonCode::AuthorizationDenied),
                ..AuthSessionDto::default()
            }),
            ..ViewModel::default()
        },
    );
    assert_filter_disabled_16px(&denied, "Access denied", "auth_denied_deep_link");

    let mut pk_deep = ConsoleState::new();
    pk_deep.init_from_env("#/ci/trainway", "", None);
    push_vm(
        &mut pk_deep,
        ViewModel {
            auth: AuthState::from_session(&AuthSessionDto {
                authenticated: true,
                invite_bound: true,
                passkey_count: 0,
                passkey_enrollment_required: true,
                github_login: Some("new-op".into()),
                reason_code: Some(AuthReasonCode::PasskeyEnrollmentRequired),
                ..AuthSessionDto::default()
            }),
            ..ViewModel::default()
        },
    );
    assert_filter_disabled_16px(&pk_deep, "Enroll passkey to unlock...", "passkey_deep");

    let mut active_unauth = active_state("limited", vec!["system:dashboard.read"]);
    active_unauth.init_from_env("#/platform/users", "", None);
    let mut unauth_vm = multi_area_fixture_vm(active_unauth.auth());
    unauth_vm.auth = active_unauth.auth();
    push_vm(&mut active_unauth, unauth_vm);
    assert_filter_disabled_16px(
        &active_unauth,
        "Access restricted for active roles",
        "active_unauthorized",
    );
    // `/` shortcut stays inert when filter is disabled.
    assert!(active_unauth.on_key("/", false, 100).is_none());
    assert!(!active_unauth.filter_focused);
}

#[test]
fn area_and_access_aware_filtering_across_all_8_areas() {
    use mu_console_protocol::{
        CdSubView, ModelRunnerSubView, ObservabilitySubView, PlatformSubView, RepositorySubView,
    };

    check_disabled_filter_across_states();

    let mut st = active_state("ops", full_area_access_policies());
    let full_vm = multi_area_fixture_vm(st.auth());
    push_vm(&mut st, full_vm);

    // 1. Dashboard: active with ci:nodes.read + obs + ci -> enabled placeholder, filters Fleet/Attention/Apps.
    st.select_subview(SubView::Dashboard(DashboardSubView::Overview));
    assert!(st.is_filter_applicable());
    assert_eq!(st.on_key("/", false, 200), None);
    assert!(st.filter_focused);
    st.set_filter_query("alpha", 210);
    let (dash_html, dash_wf) = render_both_skins(&st);
    for out in [&dash_html, &dash_wf] {
        assert!(out.contains("worker-alpha"));
        assert!(!out.contains("worker-beta"));
        assert!(out.contains("alpha-api"));
    }
    assert!(dash_html.contains("Filter applications, nodes, or alerts..."));

    // 2. CI: QueueLogs status filters + log/job text filtering + Scorecards/Trainway/WorkerNodes/SandboxProfiles.
    st.set_filter_query("", 300);
    st.select_subview(SubView::Ci(CiSubView::QueueLogs));
    assert!(st.activate_target("ci-queue-filter:running").is_empty());
    let (ci_run_html, _) = render_both_skins(&st);
    assert!(ci_run_html.contains("#101"));
    assert!(!ci_run_html.contains("#102"));
    assert!(st.activate_target("ci-queue-filter:failed").is_empty());
    let (ci_fail_html, ci_fail_wf) = render_both_skins(&st);
    assert!(ci_fail_html.contains("#102") && ci_fail_wf.contains("#102"));
    assert!(!ci_fail_html.contains("#101"));
    assert!(st.activate_target("ci-queue-filter:all").is_empty());

    st.set_filter_query("LOG_NEEDLE", 310);
    let (ci_log_html, _) = render_both_skins(&st);
    assert!(ci_log_html.contains("LOG_NEEDLE running rvv tile"));
    assert!(!ci_log_html.contains("unrelated stdout line"));

    st.select_subview(SubView::Ci(CiSubView::Scorecards));
    st.set_filter_query("release-alpha", 320);
    let (sc_html, _) = render_both_skins(&st);
    assert!(sc_html.contains("alpha-superproject"));
    assert!(!sc_html.contains("beta-service"));

    st.select_subview(SubView::Ci(CiSubView::Trainway));
    st.set_filter_query("alpha-rvv", 330);
    let (tw_html, _) = render_both_skins(&st);
    assert!(tw_html.contains("feat/alpha-rvv"));
    assert!(!tw_html.contains("feat/beta-docs"));

    st.select_subview(SubView::Ci(CiSubView::WorkerNodes));
    st.set_filter_query("riscv64", 340);
    let (wn_html, _) = render_both_skins(&st);
    assert!(wn_html.contains("worker-alpha"));
    assert!(!wn_html.contains("worker-beta"));

    st.select_subview(SubView::Ci(CiSubView::SandboxProfiles));
    st.set_filter_query("HypervisorSandbox", 350);
    let (sb_html, _) = render_both_skins(&st);
    assert!(sb_html.contains("sb-kvm-strict"));
    assert!(!sb_html.contains("sb-kernel-fast"));

    // 3. CD: Runs & Pipelines.
    st.select_subview(SubView::Cd(CdSubView::Runs));
    st.set_filter_query("Cargo", 400);
    let (cd_runs_html, _) = render_both_skins(&st);
    assert!(cd_runs_html.contains("cd-alpha-1"));
    assert!(!cd_runs_html.contains("cd-beta-2"));

    st.select_subview(SubView::Cd(CdSubView::Pipelines));
    st.set_filter_query("Deploy Maven", 410);
    let (cd_pipe_html, _) = render_both_skins(&st);
    assert!(cd_pipe_html.contains("cd-beta-2"));
    assert!(!cd_pipe_html.contains("cd-alpha-1"));

    // 4. A|B Deploy: Gate, Slots, GateTestLogs, ActiveLeases.
    st.select_subview(SubView::AbDeploy(AbDeploySubView::IntegrationGate));
    st.set_filter_query("it_alpha_gemm", 500);
    let (ab_gate_html, _) = render_both_skins(&st);
    assert!(ab_gate_html.contains("it_alpha_gemm"));
    assert!(!ab_gate_html.contains("it_beta_smoke"));

    st.select_subview(SubView::AbDeploy(AbDeploySubView::DeploymentsSlots));
    st.set_filter_query("Candidate", 510);
    let (ab_slot_html, _) = render_both_skins(&st);
    assert!(ab_slot_html.contains("ab-slot-b"));
    assert!(!ab_slot_html.contains("ab-slot-a"));

    st.select_subview(SubView::AbDeploy(AbDeploySubView::GateTestLogs));
    st.set_filter_query("GATE_NEEDLE", 520);
    let (ab_logs_html, _) = render_both_skins(&st);
    assert!(ab_logs_html.contains("GATE_NEEDLE gemm nan"));
    assert!(!ab_logs_html.contains("other gate line"));

    st.select_subview(SubView::AbDeploy(AbDeploySubView::ActiveLeases));
    st.set_filter_query("beta-worker", 530);
    let (ab_lease_html, _) = render_both_skins(&st);
    assert!(ab_lease_html.contains("lease-beta"));
    assert!(!ab_lease_html.contains("lease-alpha"));

    // 5. Registry: Packages (digests + ecosystem), CanaryTaints, CasHealth.
    st.select_subview(SubView::Repository(RepositorySubView::Packages));
    st.set_filter_query("deadbeef1111", 600);
    let (reg_dig_html, _) = render_both_skins(&st);
    assert!(reg_dig_html.contains("k3-alpha-crate"));
    assert!(!reg_dig_html.contains("Mu.Beta.Package"));
    st.set_filter_query("", 605);
    st.on_dom_action("data-ecosystem", "nuget", false, 606);
    let (reg_eco_html, _) = render_both_skins(&st);
    assert!(reg_eco_html.contains("Mu.Beta.Package"));
    assert!(!reg_eco_html.contains("k3-alpha-crate"));

    st.select_subview(SubView::Repository(RepositorySubView::CanaryTaints));
    st.set_filter_query("TAINTED", 610);
    let (cny_html, _) = render_both_skins(&st);
    assert!(cny_html.contains("bundle-beta-taint"));
    assert!(!cny_html.contains("bundle-alpha-ok"));

    st.select_subview(SubView::Repository(RepositorySubView::CasHealth));
    st.set_filter_query("Corrupt", 620);
    let (cas_html, _) = render_both_skins(&st);
    assert!(cas_html.contains("Corrupt Blobs"));

    // 6. Observability: all 7 sub-views (Smartscape layer filter + service selector + Ingress).
    st.set_filter_query("", 700);
    st.select_subview(SubView::Observability(ObservabilitySubView::Smartscape));
    assert!(st
        .activate_target("obs-smartscape-filter:services")
        .is_empty());
    let (ss_svc_html, _) = render_both_skins(&st);
    assert!(ss_svc_html.contains("alpha-api"));
    assert!(!ss_svc_html.contains("beta-worker"));
    assert!(!ss_svc_html.contains("gamma-host"));

    assert!(st
        .activate_target("obs-smartscape-filter:processes")
        .is_empty());
    let (ss_proc_html, _) = render_both_skins(&st);
    assert!(ss_proc_html.contains("beta-worker"));
    assert!(!ss_proc_html.contains("alpha-api"));

    assert!(st.activate_target("obs-smartscape-filter:hosts").is_empty());
    let (ss_host_html, _) = render_both_skins(&st);
    assert!(ss_host_html.contains("gamma-host"));
    assert!(!ss_host_html.contains("alpha-api"));

    assert!(st.activate_target("obs-smartscape-filter:all").is_empty());
    // Cycle service selector to first sorted service ("alpha-api").
    assert!(st.activate_target("obs-service-next").is_empty());
    assert_eq!(st.obs_service_filter.as_deref(), Some("alpha-api"));
    for obs_sv in [
        ObservabilitySubView::Smartscape,
        ObservabilitySubView::PurePath,
        ObservabilitySubView::ProblemsSlos,
        ObservabilitySubView::Logs,
        ObservabilitySubView::Metrics,
        ObservabilitySubView::Ingress,
    ] {
        st.select_subview(SubView::Observability(obs_sv));
        let (obs_html, obs_wf) = render_both_skins(&st);
        assert!(
            obs_html.contains("alpha-api"),
            "{obs_sv:?} missing alpha-api"
        );
        assert!(
            !obs_html.contains("beta-worker"),
            "{obs_sv:?} leaked beta-worker under service filter"
        );
        assert!(obs_wf.contains("obs-service-filter"));
    }
    assert!(st.activate_target("obs-service-clear").is_empty());
    assert_eq!(st.obs_service_filter, None);
    st.select_subview(SubView::Observability(ObservabilitySubView::Ingress));
    st.set_filter_query("/v1/beta", 710);
    let (ing_html, ing_wf) = render_both_skins(&st);
    assert!(ing_html.contains("ing-beta") && ing_wf.contains("ing-beta"));
    assert!(!ing_html.contains("ing-alpha"));

    st.select_subview(SubView::Observability(ObservabilitySubView::DataStore));
    st.set_filter_query("DuckDB", 720);
    let (ds_html, _) = render_both_skins(&st);
    assert!(ds_html.contains("12345"));

    // 7. Model Runner: RuntimeQueue & ConnectionVpn.
    st.select_subview(SubView::ModelRunner(ModelRunnerSubView::RuntimeQueue));
    st.set_filter_query("cluster-a100", 800);
    let (mr_rt_html, _) = render_both_skins(&st);
    assert!(mr_rt_html.contains("qwen-7b"));
    assert!(!mr_rt_html.contains("llama-3b"));

    st.select_subview(SubView::ModelRunner(ModelRunnerSubView::ConnectionVpn));
    st.set_filter_query("10.42.0.3", 810);
    let (mr_vpn_html, _) = render_both_skins(&st);
    assert!(mr_vpn_html.contains("pk-beta"));
    assert!(!mr_vpn_html.contains("pk-alpha"));

    // 8. Platform: Maintenance, UsersAndInvites (Last Admin preserved when filtered to 1 admin!), RolesAndFolios.
    st.select_subview(SubView::Platform(PlatformSubView::Maintenance));
    st.set_filter_query("Subsystem", 900);
    let (pm_html, _) = render_both_skins(&st);
    assert!(pm_html.contains("Selected Subsystem"));

    st.select_subview(SubView::Platform(PlatformSubView::UsersAndInvites));
    st.set_filter_query("sole-admin", 910);
    let (pu_html, pu_wf) = render_both_skins(&st);
    assert!(pu_html.contains("@sole-admin") && pu_wf.contains("sole-admin"));
    assert!(!pu_html.contains("@dev-member"));
    assert!(pu_html.contains("Last Admin"));
    assert!(st
        .activate_target("platform-user-revoke:sole-admin")
        .is_empty());
    assert!(st.active_dialog().is_none());

    // Pre-filter admin_count stays accurate with 2 admins even when filtered down to 1 admin.
    let mut two_admin_vm = multi_area_fixture_vm(st.auth());
    two_admin_vm.platform.users.push(PlatformUserDto {
        github_login: "backup-admin".into(),
        display_name: Some("Backup Admin".into()),
        passkey_count: 1,
        bound_roles: vec!["platform-admin".into()],
        is_platform_admin: true,
        last_seen_ms: None,
    });
    push_vm(&mut st, two_admin_vm);
    let (pu2_html, _) = render_both_skins(&st);
    assert!(pu2_html.contains("@sole-admin"));
    assert!(!pu2_html.contains("@backup-admin"));
    assert!(!pu2_html.contains("Last Admin"));

    st.select_subview(SubView::Platform(PlatformSubView::RolesAndFolios));
    st.set_filter_query("folio-super-admin", 920);
    let (pr_html, _) = render_both_skins(&st);
    assert!(pr_html.contains("Super Operator"));
    assert!(!pr_html.contains("Baseline Operator"));
}

#[test]
fn stored_repo_non_url_hint_deep_link_precedence_and_repos_changed_sync() {
    use mu_console_data::api::{FetchKind, StreamId};
    use mu_console_data::engine::{Effect, Engine};
    use mu_console_protocol::{RepoId, WorkerConfig};
    use mu_console_ui::router::format_hash;
    use mu_console_ui::state::StoredRepoSync;

    let drain_posts = |effects: Vec<Effect>, state: &mut ConsoleState| {
        for ef in effects {
            if let Effect::Post(ev) = ef {
                let bytes = Engine::encode_event(&ev).expect("encode DataEvent");
                state.on_worker_frame(&bytes);
            }
        }
    };

    let anon_public_session = r#"{
        "authenticated": false,
        "anonymous_lockdown": false,
        "active_policies": ["system:status.read", "system:dashboard.read", "ci:repos.public.read", "ci:queue.read"]
    }"#;
    let active_session = r#"{
        "authenticated": true,
        "github_login": "ops",
        "invite_bound": true,
        "passkey_count": 1,
        "passkey_enrollment_required": false,
        "baseline_role": "operator",
        "bound_roles": ["operator"],
        "active_policies": ["system:status.read", "system:dashboard.read", "ci:repos.public.read", "ci:repos.private.read", "ci:queue.read"]
    }"#;
    let public_repos = r#"[
        {"owner":"LMA1980","repo":"alpha-public","visibility":"public","status":"passing","score":95,"coverage":90.0,"bench_delta":0.0},
        {"owner":"LMA1980","repo":"beta-public","visibility":"public","status":"running","score":80,"coverage":82.0,"bench_delta":0.5}
    ]"#;
    let active_repos = r#"[
        {"owner":"LMA1980","repo":"alpha-public","visibility":"public","status":"passing","score":95,"coverage":90.0,"bench_delta":0.0},
        {"owner":"LMA1980","repo":"beta-public","visibility":"public","status":"running","score":80,"coverage":82.0,"bench_delta":0.5},
        {"owner":"LMA1980","repo":"gamma-private","visibility":"private","status":"passing","score":99,"coverage":96.0,"bench_delta":1.0}
    ]"#;
    let pruned_repos = r#"[
        {"owner":"LMA1980","repo":"beta-public","visibility":"public","status":"running","score":80,"coverage":82.0,"bench_delta":0.5}
    ]"#;

    // 1. Clean #/dashboard/overview + Sign In rd= URL when localStorage holds a retired repo,
    //    and private repo hint preservation across Anonymous cold boot -> OAuth login.
    let mut eng = Engine::new(1);
    let mut st = ConsoleState::new();
    st.init_from_env("#/dashboard/overview", "", None);
    let cfg = WorkerConfig::from_query("", "https://console.example.org")
        .with_preferred_repo(Some("LMA1980/gamma-private"));
    drain_posts(eng.handle(UiCommand::Configure(cfg), 0), &mut st);
    drain_posts(
        eng.handle(UiCommand::Subscribe(st.route.clone()), 1),
        &mut st,
    );
    drain_posts(
        eng.on_fetch(&FetchKind::AuthSession, Ok(anon_public_session.into()), 5),
        &mut st,
    );
    drain_posts(
        eng.on_fetch(&FetchKind::Repos, Ok(public_repos.into()), 10),
        &mut st,
    );
    drain_posts(eng.tick(200), &mut st);

    assert_eq!(st.route.repo, None);
    assert_eq!(format_hash(&st.route), "#/dashboard/overview");
    assert_eq!(
        st.sign_in_href(),
        "/oauth2/start?rd=%2F%23%2Fdashboard%2Foverview"
    );
    assert_eq!(
        st.stored_repo_sync(Some("LMA1980/gamma-private")),
        StoredRepoSync::Preserve
    );

    // OAuth login activates private repo hint via refreshed /api/repos.
    drain_posts(
        eng.on_fetch(&FetchKind::AuthSession, Ok(active_session.into()), 300),
        &mut st,
    );
    drain_posts(
        eng.on_fetch(&FetchKind::Repos, Ok(active_repos.into()), 310),
        &mut st,
    );
    drain_posts(eng.tick(500), &mut st);
    assert_eq!(st.route.repo, None);
    assert_eq!(format_hash(&st.route), "#/dashboard/overview");
    assert_eq!(
        st.stored_repo_sync(Some("LMA1980/gamma-private")),
        StoredRepoSync::Set("LMA1980/gamma-private".into())
    );

    // 2. Explicit ?repo= deep link overrides stored hint and reconciles on repos_changed SSE.
    let mut eng2 = Engine::new(2);
    let mut st2 = ConsoleState::new();
    st2.init_from_env("#/ci/scorecards?repo=LMA1980/alpha-public", "", None);
    let cfg2 = WorkerConfig::from_query("", "https://console.example.org")
        .with_preferred_repo(Some("LMA1980/gamma-private"));
    drain_posts(eng2.handle(UiCommand::Configure(cfg2), 0), &mut st2);
    drain_posts(
        eng2.handle(UiCommand::Subscribe(st2.route.clone()), 1),
        &mut st2,
    );
    drain_posts(
        eng2.on_fetch(&FetchKind::AuthSession, Ok(active_session.into()), 5),
        &mut st2,
    );
    drain_posts(
        eng2.on_fetch(&FetchKind::Repos, Ok(active_repos.into()), 10),
        &mut st2,
    );
    drain_posts(eng2.tick(200), &mut st2);
    assert_eq!(st2.route.repo, Some(RepoId::new("LMA1980", "alpha-public")));

    // Prune alpha-public via repos_changed SSE -> UI route & storage reconcile to beta-public.
    let sse_fx = eng2.on_stream_chunk(StreamId::Ci, b"event: repos_changed\ndata: {}\n\n", 400);
    assert!(sse_fx.iter().any(|e| matches!(
        e,
        mu_console_data::engine::Effect::Fetch {
            kind: FetchKind::Repos,
            ..
        }
    )));
    drain_posts(
        eng2.on_fetch(&FetchKind::Repos, Ok(pruned_repos.into()), 410),
        &mut st2,
    );
    drain_posts(eng2.tick(600), &mut st2);
    assert!(st2.route_sync_dirty);
    assert_eq!(st2.route.repo, Some(RepoId::new("LMA1980", "beta-public")));
    assert_eq!(
        format_hash(&st2.route),
        "#/ci/scorecards?repo=LMA1980/beta-public"
    );
    assert_eq!(
        st2.stored_repo_sync(Some("LMA1980/alpha-public")),
        StoredRepoSync::Set("LMA1980/beta-public".into())
    );
}
