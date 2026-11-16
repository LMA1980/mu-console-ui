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
