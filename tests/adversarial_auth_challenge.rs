// SPDX-License-Identifier: AGPL-3.0-only
//! Adversarial Challenge Test Harness for Console Access & GitHub OAuth2 Integration.
//!
//! Stress-tests:
//! 1. Rapid repeated clicks and idempotency on `c2-auth-status` across authentication states.
//! 2. State machine transitions: Anonymous <-> AuthorizationDenied <-> Active under load.
//! 3. Malformed return path injection and URL percent-encoding validation.
//! 4. Strict synchronization between `render_html_shell` `<a id="muconsole-auth-link">` and href methods.
//! 5. `sessionStorage` invite stashing logic and survival across full-page navigation.
#![cfg(not(miri))]

use mu_console_data::engine::{Effect, Engine};
use mu_console_data::scenario::Scenario;
use mu_console_layout::Viewport;
use mu_console_protocol::dto::AuthSessionDto;
use mu_console_protocol::{
    encode, Area, AuthReasonCode, AuthState, AuthStatus, CiSubView, ConsoleSkin, DataEvent, Delta,
    RepoId, SubView, UiCommand, UiError, ViewModel, WorkerConfig,
};
use mu_console_ui::client::WORKER_URL;
use mu_console_ui::components::{render_console_with_skin, render_html_shell};
use mu_console_ui::state::{
    extract_invite_token, format_sign_in_url, format_sign_out_url, validate_sign_in_href,
    ConsoleState, WebAuthnRequest, PENDING_INVITE_STORAGE_KEY,
};
use mu_console_ui::webauthn::{register_start_body, wrap_credential_envelope_json};

fn anon_auth() -> AuthState {
    AuthState::anonymous(vec!["ci:jobs.read".into()])
}

fn denied_auth(login: &str) -> AuthState {
    AuthState::from_session(&AuthSessionDto {
        authenticated: false,
        invite_bound: false,
        github_login: Some(login.into()),
        reason_code: Some(AuthReasonCode::AuthorizationDenied),
        active_policies: vec!["ci:jobs.read".into()],
        ..AuthSessionDto::default()
    })
}

fn active_auth(login: &str) -> AuthState {
    AuthState::from_session(&AuthSessionDto {
        authenticated: true,
        invite_bound: true,
        passkey_count: 1,
        github_login: Some(login.into()),
        baseline_role: Some("operator".into()),
        active_policies: vec!["system:dashboard.read".into(), "ci:jobs.read".into()],
        ..AuthSessionDto::default()
    })
}

fn initialized_state() -> ConsoleState {
    let mut eng = Engine::new(1);
    let cfg = WorkerConfig {
        api_base: "http://192.0.2.10:8080".into(),
        scenario: Some(Scenario::Submodules.name().to_owned()),
        ..WorkerConfig::default()
    };
    let mut state = ConsoleState::new();
    for ef in eng.handle(UiCommand::Configure(cfg), 1_000) {
        if let Effect::Post(ev) = ef {
            let bytes = Engine::encode_event(&ev).expect("encode DataEvent");
            state.on_worker_frame(&bytes);
        }
    }
    let end_ms = 1_000 + Scenario::Submodules.duration_ms();
    for t in (1_000..=end_ms).step_by(200) {
        for ef in eng.tick(t) {
            if let Effect::Post(ev) = ef {
                let bytes = Engine::encode_event(&ev).expect("encode DataEvent");
                state.on_worker_frame(&bytes);
            }
        }
    }
    let anon_patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(anon_auth())]))
        .expect("encode Delta::Auth Anonymous");
    state.on_worker_frame(&anon_patch);
    state
}

#[test]
fn test_rapid_repeated_clicks_idempotency_anonymous() {
    let mut st = initialized_state();
    assert_eq!(st.auth().status, AuthStatus::Anonymous);
    assert_eq!(st.pending_navigation, None);

    // Stress: 100 rapid clicks on c2-auth-status in Anonymous state
    for i in 0..100 {
        let now_ms = 1_000 + i * 5;
        let shift = (i % 2) == 1;
        let cmds = st.on_dom_action("data-control-id", "c2-auth-status", shift, now_ms);

        assert_eq!(
            cmds.len(),
            1,
            "Iteration {i}: Rapid click must emit exactly 1 command"
        );
        assert_eq!(
            cmds[0],
            UiCommand::Navigate {
                href: "/oauth2/start?rd=%2F".into(),
            },
            "Iteration {i}: Command must navigate to sign-in URL"
        );
        assert_eq!(
            st.pending_navigation.as_deref(),
            Some("/oauth2/start?rd=%2F"),
            "Iteration {i}: pending_navigation must remain stable"
        );
        assert_eq!(
            st.aria_polite, "Navigating to /oauth2/start?rd=%2F",
            "Iteration {i}: aria_polite message must remain stable"
        );
    }
}

#[test]
fn test_state_machine_transitions_and_rapid_clicks_cycle() {
    let mut st = initialized_state();

    // Stage 1: Anonymous -> rapid clicks
    for i in 0..10 {
        let cmds = st.on_dom_action("data-control-id", "c2-auth-status", false, 100 + i);
        assert_eq!(
            cmds,
            vec![UiCommand::Navigate {
                href: "/oauth2/start?rd=%2F".into()
            }]
        );
        assert_eq!(
            st.pending_navigation.as_deref(),
            Some("/oauth2/start?rd=%2F")
        );
    }

    // Stage 2: Transition to AuthorizationDenied
    let denied_patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(denied_auth(
        "challenger-bot",
    ))]))
    .expect("encode Delta::Auth AuthorizationDenied");
    st.on_worker_frame(&denied_patch);

    assert_eq!(st.auth().status, AuthStatus::AuthorizationDenied);
    assert!(!st.is_operator());

    // 10 rapid clicks in AuthorizationDenied -> must navigate to sign_out
    for i in 0..10 {
        let cmds = st.on_dom_action("data-control-id", "c2-auth-status", true, 200 + i);
        assert_eq!(
            cmds,
            vec![UiCommand::Navigate {
                href: "/oauth2/sign_out?rd=%2F".into()
            }],
            "AuthorizationDenied state must dispatch sign_out"
        );
        assert_eq!(
            st.pending_navigation.as_deref(),
            Some("/oauth2/sign_out?rd=%2F")
        );
        assert_eq!(st.aria_polite, "Navigating to /oauth2/sign_out?rd=%2F");
    }

    // Stage 3: Transition to Active operator
    let op_patch =
        Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(active_auth("LMA1980"))]))
            .expect("encode Delta::Auth Active");
    st.on_worker_frame(&op_patch);

    assert_eq!(st.auth().status, AuthStatus::Active);
    assert_eq!(st.auth().github_login.as_deref(), Some("LMA1980"));
    assert!(st.is_operator());

    // 10 rapid clicks in Active -> must navigate to sign_out
    for i in 0..10 {
        let cmds = st.on_dom_action("data-control-id", "c2-auth-status", false, 300 + i);
        assert_eq!(
            cmds,
            vec![UiCommand::Navigate {
                href: "/oauth2/sign_out?rd=%2F".into()
            }],
            "Active state must dispatch sign_out"
        );
        assert_eq!(
            st.pending_navigation.as_deref(),
            Some("/oauth2/sign_out?rd=%2F")
        );
        assert_eq!(st.aria_polite, "Navigating to /oauth2/sign_out?rd=%2F");
    }

    // Stage 4: Transition back to Anonymous (e.g. sign-out completed or session revoked)
    let anon_patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(anon_auth())]))
        .expect("encode Delta::Auth Anonymous");
    st.on_worker_frame(&anon_patch);

    assert_eq!(st.auth().status, AuthStatus::Anonymous);
    assert!(!st.is_operator());

    // Rapid clicks in reverted Anonymous -> must navigate to sign_in again
    for i in 0..10 {
        let cmds = st.on_dom_action("data-control-id", "c2-auth-status", false, 400 + i);
        assert_eq!(
            cmds,
            vec![UiCommand::Navigate {
                href: "/oauth2/start?rd=%2F".into()
            }]
        );
        assert_eq!(
            st.pending_navigation.as_deref(),
            Some("/oauth2/start?rd=%2F")
        );
        assert_eq!(st.aria_polite, "Navigating to /oauth2/start?rd=%2F");
    }
}

#[test]
fn test_malformed_return_path_injection_and_url_formatting() {
    let adversarial_inputs = [
        "/",
        "",
        "https://console.lma1980.duckdns.org/?invite=inv_cca98d8ec4e62b9e02ec5286",
        "https://console.lma1980.duckdns.org/oauth2/start?rdhttps://console.lma1980.duckdns.org/",
        "rdhttps://evil.com",
        "?rd=evil",
        "//attacker.com",
        "/dashboard?tab=fleet&filter=ok#active",
        "javascript:alert(1)",
        "/?param=1&param=2",
        "/path with spaces & query=1#hash",
        "\r\nInjected-Header: value",
        "../../etc/passwd",
        "/?invite=foo&token=bar",
        "!@#$%^&*()_+~|}{[]:;?><",
        "/\u{1F980}/test",
    ];

    for rd in adversarial_inputs {
        let sign_in = format_sign_in_url(rd);
        let sign_out = format_sign_out_url(rd);

        // 1. Both must start with the canonical path prefix followed strictly by "?rd="
        assert!(
            sign_in.starts_with("/oauth2/start?rd="),
            "sign_in_url for input '{rd}' must start with '/oauth2/start?rd=', got: '{sign_in}'"
        );
        assert!(
            sign_out.starts_with("/oauth2/sign_out?rd="),
            "sign_out_url for input '{rd}' must start with '/oauth2/sign_out?rd=', got: '{sign_out}'"
        );

        // 2. Neither must EVER produce malformed "rd" without "=" (e.g. "?rdhttps:")
        assert!(
            !sign_in.contains("?rdhttps:") && !sign_in.contains("&rdhttps:"),
            "sign_in URL contains malformed rdhttps: '{sign_in}'"
        );
        assert!(
            !sign_out.contains("?rdhttps:") && !sign_out.contains("&rdhttps:"),
            "sign_out URL contains malformed rdhttps: '{sign_out}'"
        );

        // 3. Neither must contain unencoded newlines (CRLF injection prevention)
        assert!(
            !sign_in.contains('\r') && !sign_in.contains('\n'),
            "sign_in URL must not contain raw CRLF characters: '{sign_in}'"
        );
        assert!(
            !sign_out.contains('\r') && !sign_out.contains('\n'),
            "sign_out URL must not contain raw CRLF characters: '{sign_out}'"
        );

        // 4. Validate sign_in with validate_sign_in_href (`""` normalizes to `"/"`)
        assert!(
            validate_sign_in_href(&sign_in).is_ok(),
            "Valid sign_in URL for '{rd}' failed validation: {:?}",
            validate_sign_in_href(&sign_in)
        );
    }

    assert_eq!(
        validate_sign_in_href("/oauth2/start?rd="),
        Err("empty rd parameter"),
        "Empty rd parameter must be rejected by validate_sign_in_href"
    );

    // 5. Explicit check: validate_sign_in_href must strictly reject screenshot reproduction string
    // String without '=' after rd
    let malformed_no_equals = "/oauth2/start?rdhttps://console.lma1980.duckdns.org/";
    assert_eq!(
        validate_sign_in_href(malformed_no_equals),
        Err("malformed rd parameter missing '='")
    );

    // String with unencoded nested '=' (such as in screenshot report)
    let screenshot_bug_url = "/oauth2/start?rdhttps://console.lma1980.duckdns.org/?invite=inv_cca";
    assert!(
        validate_sign_in_href(screenshot_bug_url).is_err(),
        "Must fail validation for unencoded nested invite URL"
    );

    // Missing query string
    assert_eq!(
        validate_sign_in_href("/oauth2/start"),
        Err("missing query string")
    );

    // Wrong path
    assert_eq!(
        validate_sign_in_href("/oauth2/callback?rd=/"),
        Err("invalid path, expected /oauth2/start")
    );

    // Missing rd param
    assert_eq!(
        validate_sign_in_href("/oauth2/start?foo=bar"),
        Err("missing rd parameter")
    );
}

#[test]
fn test_html_shell_auth_link_synchronization() {
    let mut st = initialized_state();

    // 1. Anonymous state check
    let shell_anon = render_html_shell("<svg></svg>", &st);
    let expected_anon_href = st.sign_in_href();
    assert_eq!(expected_anon_href, "/oauth2/start?rd=%2F");
    assert!(shell_anon.contains("id=\"muconsole-font-toggle\""));
    assert!(
        shell_anon.contains(&format!(
            r#"<a id="muconsole-auth-link" href="{expected_anon_href}" data-auth-state="anonymous">Sign In</a>"#
        )),
        "HTML shell must contain synchronized sign_in_href in Anonymous state"
    );

    // 2. AuthorizationDenied state check (generic denial, zero invite/identity disclosure)
    let denied_patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(denied_auth(
        "test-user-42",
    ))]))
    .expect("encode");
    st.on_worker_frame(&denied_patch);

    let shell_denied = render_html_shell("<svg></svg>", &st);
    let expected_denied_href = st.sign_out_href();
    assert_eq!(expected_denied_href, "/oauth2/sign_out?rd=%2F");
    assert!(shell_denied.contains("id=\"muconsole-font-toggle\""));
    assert!(shell_denied.contains(
        r#"<span id="muconsole-auth-denied" data-auth-state="authorization_denied">Authorization denied</span>"#
    ));
    assert!(
        shell_denied.contains(&format!(
            r#"<a id="muconsole-auth-link" href="{expected_denied_href}" data-auth-state="authorization_denied">Sign Out</a>"#
        )),
        "HTML shell must contain synchronized sign_out_href in AuthorizationDenied state"
    );
    assert!(!shell_denied.contains("test-user-42"));
    assert!(!shell_denied.contains("INVITE"));

    // 3. Active operator state check (`@user` + `+role` + `Sign Out`, `[Aa]` removed)
    let op_patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(active_auth(
        "admin-ops",
    ))]))
    .expect("encode");
    st.on_worker_frame(&op_patch);

    let shell_op = render_html_shell("<svg></svg>", &st);
    let expected_op_href = st.sign_out_href();
    assert_eq!(expected_op_href, "/oauth2/sign_out?rd=%2F");
    assert!(!shell_op.contains("muconsole-font-toggle"));
    assert!(shell_op.contains(
        r#"<button id="muconsole-user-btn" type="button" data-control-id="hdr-user" data-action="hdr-user">@admin-ops</button>"#
    ));
    assert!(shell_op.contains("id=\"muconsole-roles-btn\""));
    assert!(
        shell_op.contains(&format!(
            r#"<a id="muconsole-auth-link" href="{expected_op_href}" data-auth-state="active">Sign Out</a>"#
        )),
        "HTML shell must contain synchronized sign_out_href in Active state"
    );

    // 4. Verify root-absolute dyslexic font URL inside shell when loaded
    st.lazy_dyslexic_loaded = true;
    let shell_dyslexic = render_html_shell("<svg></svg>", &st);
    assert!(
        shell_dyslexic.contains("src:url('/fonts/OpenDyslexicMNerdFontMono-Regular.woff2')"),
        "HTML shell must use root-absolute font URL"
    );
    assert!(
        !shell_dyslexic.contains("./fonts/"),
        "HTML shell must not contain relative ./fonts/"
    );
}

#[test]
fn test_session_storage_invite_stashing_and_navigation_survival() {
    // Stage 1: User arrives with invite token in query string
    let initial_query =
        "?invite=inv_test_secret_99887766554433221100&repo=model-runner&font=hurmit";
    let (extracted, clean_query) = extract_invite_token(initial_query);

    assert_eq!(
        extracted.as_deref(),
        Some("inv_test_secret_99887766554433221100")
    );
    assert_eq!(
        clean_query, "?repo=model-runner&font=hurmit",
        "clean_search must strip invite parameter while preserving all other query parameters"
    );

    // Stage 2: Initial boot stashes extracted token into simulated sessionStorage
    let mut simulated_session_storage: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    if let Some(ref token) = extracted {
        simulated_session_storage.insert(PENDING_INVITE_STORAGE_KEY.to_string(), token.clone());
    }

    // Initialize state from environment
    let mut st = initialized_state();
    st.init_from_env("#ci/scorecards", initial_query, None);
    assert_eq!(
        st.pending_invite.as_deref(),
        Some("inv_test_secret_99887766554433221100")
    );

    // Stage 3: User clicks sign in button -> navigation command preserves unauthorized deep link
    let nav_cmds = st.on_dom_action("data-control-id", "c2-auth-status", false, 1_000);
    assert_eq!(
        nav_cmds,
        vec![UiCommand::Navigate {
            href: "/oauth2/start?rd=%2F%23%2Fci%2Fscorecards".into()
        }]
    );
    assert_eq!(
        st.pending_navigation.as_deref(),
        Some("/oauth2/start?rd=%2F%23%2Fci%2Fscorecards")
    );

    // Stage 4: Browser performs full-page navigation to GitHub and returns to origin root "/"
    // During this navigation, sessionStorage persists within the origin tab.
    assert!(
        simulated_session_storage.contains_key(PENDING_INVITE_STORAGE_KEY),
        "sessionStorage must retain pending invite token across origin navigation"
    );

    // Stage 5: Reload / return landing at "/" with empty query string
    let return_query = "";
    let (return_extracted, return_clean) = extract_invite_token(return_query);
    assert_eq!(return_extracted, None);
    assert_eq!(return_clean, "");

    // Recover token from sessionStorage
    let stored_invite = return_extracted.or_else(|| {
        simulated_session_storage
            .get(PENDING_INVITE_STORAGE_KEY)
            .cloned()
            .filter(|t| !t.is_empty())
    });

    assert_eq!(
        stored_invite.as_deref(),
        Some("inv_test_secret_99887766554433221100"),
        "Stored invite must be recovered from sessionStorage upon return from OAuth2 flow"
    );

    // Stage 6: Recreated ConsoleState boots from returned env and sessionStorage
    let mut st_return = initialized_state();
    st_return.init_from_env("", return_query, None);
    if st_return.pending_invite.is_none() {
        st_return.pending_invite.clone_from(&stored_invite);
    }
    assert_eq!(
        st_return.pending_invite.as_deref(),
        Some("inv_test_secret_99887766554433221100")
    );

    // Stage 7: RedemeInvite command is prepared for the worker
    let redeem_cmd = UiCommand::RedeemInvite {
        token: stored_invite.expect("valid token"),
    };
    assert_eq!(
        redeem_cmd,
        UiCommand::RedeemInvite {
            token: "inv_test_secret_99887766554433221100".into()
        }
    );

    // Stage 8: Worker authenticates and emits Active session state
    let operator_patch =
        Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(active_auth("LMA1980"))]))
            .expect("encode");
    st_return.on_worker_frame(&operator_patch);
    assert!(st_return.is_operator());

    // When active session arrives, boot.rs purges pending_invite from sessionStorage
    if st_return.is_operator() && st_return.pending_invite.take().is_some() {
        simulated_session_storage.remove(PENDING_INVITE_STORAGE_KEY);
    }

    assert_eq!(st_return.pending_invite, None);
    assert!(
        !simulated_session_storage.contains_key(PENDING_INVITE_STORAGE_KEY),
        "sessionStorage must be purged after successful operator invite redemption"
    );
}

#[test]
fn test_root_absolute_invariants_across_all_modules() {
    assert!(
        WORKER_URL.starts_with('/'),
        "WORKER_URL must be root-absolute"
    );
    assert_eq!(WORKER_URL, "/muconsole-data_loader.js");

    let st = initialized_state();
    assert_eq!(st.sign_in_href(), "/oauth2/start?rd=%2F");
    assert_eq!(st.sign_out_href(), "/oauth2/sign_out?rd=%2F");

    let shell = render_html_shell("", &st);
    assert!(
        shell.contains(r#"href="/oauth2/start?rd=%2F""#),
        "Shell must have root-absolute href"
    );
}

#[test]
fn test_interleaved_clicks_and_worker_frames() {
    let mut st = initialized_state();

    // 1. Start in Anonymous -> Click
    let cmds1 = st.on_dom_action("data-control-id", "c2-auth-status", false, 1_000);
    assert_eq!(
        cmds1,
        vec![UiCommand::Navigate {
            href: "/oauth2/start?rd=%2F".into()
        }]
    );
    assert_eq!(
        st.pending_navigation.as_deref(),
        Some("/oauth2/start?rd=%2F")
    );

    // 2. Interleaved worker frame: AuthorizationDenied arrives
    let uninv_patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(denied_auth(
        "interleaved-bot",
    ))]))
    .expect("encode");
    st.on_worker_frame(&uninv_patch);

    // Click immediately
    let cmds2 = st.on_dom_action("data-control-id", "c2-auth-status", false, 1_050);
    assert_eq!(
        cmds2,
        vec![UiCommand::Navigate {
            href: "/oauth2/sign_out?rd=%2F".into()
        }]
    );
    assert_eq!(
        st.pending_navigation.as_deref(),
        Some("/oauth2/sign_out?rd=%2F")
    );

    // 3. Interleaved worker frame: Active arrives
    let op_patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(active_auth(
        "interleaved-bot",
    ))]))
    .expect("encode");
    st.on_worker_frame(&op_patch);

    // Click immediately
    let cmds3 = st.on_dom_action("data-control-id", "c2-auth-status", false, 1_100);
    assert_eq!(
        cmds3,
        vec![UiCommand::Navigate {
            href: "/oauth2/sign_out?rd=%2F".into()
        }]
    );
    assert_eq!(
        st.pending_navigation.as_deref(),
        Some("/oauth2/sign_out?rd=%2F")
    );

    // 4. Interleaved worker frame: Revoked back to Anonymous
    let anon_patch =
        Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(anon_auth())])).expect("encode");
    st.on_worker_frame(&anon_patch);

    // Click immediately
    let cmds4 = st.on_dom_action("data-control-id", "c2-auth-status", false, 1_150);
    assert_eq!(
        cmds4,
        vec![UiCommand::Navigate {
            href: "/oauth2/start?rd=%2F".into()
        }]
    );
    assert_eq!(
        st.pending_navigation.as_deref(),
        Some("/oauth2/start?rd=%2F")
    );
}

#[test]
fn test_parameter_pollution_and_encoding_roundtrip() {
    let adversarial_targets = [
        "/app?query=foo&rd=evil&sign_in=true",
        "/?invite=inv_nested&rd=https://evil.com/?foo=1&bar=2",
        "/console#token=secret&rd=/oauth2/start",
        "/path with spaces/&equals=true?q=1#top",
    ];

    for target in adversarial_targets {
        let sign_in = format_sign_in_url(target);
        assert!(validate_sign_in_href(&sign_in).is_ok());

        // Ensure that the target was percent-encoded so that the outer query only has ONE 'rd' parameter
        let (_, query) = sign_in.split_once('?').expect("query string");
        let pairs: Vec<&str> = query.split('&').collect();
        assert_eq!(
            pairs.len(),
            1,
            "Outer query must not have multiple parameters injected"
        );
        assert!(pairs[0].starts_with("rd="));

        // Sign-out URL
        let sign_out = format_sign_out_url(target);
        assert!(sign_out.starts_with("/oauth2/sign_out?rd="));
        let (_, out_query) = sign_out.split_once('?').expect("query string");
        let out_pairs: Vec<&str> = out_query.split('&').collect();
        assert_eq!(out_pairs.len(), 1);
    }
}

#[test]
fn test_invite_token_extraction_variations() {
    // 1. Multiple parameters with invite in middle
    let (t1, c1) = extract_invite_token("?alpha=1&invite=inv_mid_123.hmac456&beta=2");
    assert_eq!(t1.as_deref(), Some("inv_mid_123.hmac456"));
    assert_eq!(c1, "?alpha=1&beta=2");

    // 2. Invite at start
    let (t2, c2) = extract_invite_token("?invite=inv_start_789&omega=3");
    assert_eq!(t2.as_deref(), Some("inv_start_789"));
    assert_eq!(c2, "?omega=3");

    // 3. Invite at end
    let (t3, c3) = extract_invite_token("?gamma=4&invite=inv_end_000");
    assert_eq!(t3.as_deref(), Some("inv_end_000"));
    assert_eq!(c3, "?gamma=4");

    // 4. Sole invite parameter
    let (t4, c4) = extract_invite_token("?invite=inv_sole_111");
    assert_eq!(t4.as_deref(), Some("inv_sole_111"));
    assert_eq!(c4, "");

    // 5. Empty invite value
    let (t5, c5) = extract_invite_token("?invite=&param=1");
    assert_eq!(t5, None);
    assert_eq!(c5, "?param=1");

    // 6. No invite parameter present
    let (t6, c6) = extract_invite_token("?tab=overview&repo=k3");
    assert_eq!(t6, None);
    assert_eq!(c6, "?tab=overview&repo=k3");

    // 7. Empty string
    let (t7, c7) = extract_invite_token("");
    assert_eq!(t7, None);
    assert_eq!(c7, "");
}

#[test]
fn test_html_shell_sync_with_unusual_github_logins() {
    let mut st = initialized_state();

    let logins = ["octocat", "bot[ca]", "user-123_test", "admin.infra", "op"];
    for login in logins {
        // Test AuthorizationDenied (must not leak @login or invite state)
        let denied_patch =
            Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(denied_auth(login))]))
                .expect("encode");
        st.on_worker_frame(&denied_patch);

        let shell = render_html_shell("<svg></svg>", &st);
        let expected_href = st.sign_out_href();
        assert!(shell.contains(&format!(r#"href="{expected_href}""#)));
        assert!(shell.contains(r#"data-auth-state="authorization_denied""#));
        assert!(shell.contains("Authorization denied"));
        assert!(!shell.contains(&format!("@{login}")));
        assert!(!shell.contains("INVITE"));

        // Test Active (`@login` rendered in `#muconsole-user-btn` + `Sign Out` without `[Aa]`)
        let op_patch =
            Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(active_auth(login))]))
                .expect("encode");
        st.on_worker_frame(&op_patch);

        let shell_op = render_html_shell("<svg></svg>", &st);
        let doc_op = roxmltree::Document::parse(&shell_op).expect("active shell XML must parse");
        let user_btn = doc_op
            .descendants()
            .find(|n| n.attribute("id") == Some("muconsole-user-btn"))
            .expect("user button must exist");
        let expected_handle = format!("@{login}");
        assert_eq!(user_btn.text(), Some(expected_handle.as_str()));
        let link = doc_op
            .descendants()
            .find(|n| n.attribute("id") == Some("muconsole-auth-link"))
            .expect("auth link must exist");
        assert_eq!(link.attribute("href"), Some(expected_href.as_str()));
        assert_eq!(link.attribute("data-auth-state"), Some("active"));
        assert_eq!(link.text(), Some("Sign Out"));
        assert!(!shell_op.contains("muconsole-font-toggle"));
    }
}

#[test]
fn test_cold_boot_session_probe_failure_surfaces_retry_and_sign_out_without_sign_in_loop() {
    let mut st = ConsoleState::new();
    let err_frame = encode(&DataEvent::Error(UiError::Http {
        status: 404,
        message: "GET /api/auth/session".into(),
    }))
    .expect("encode error");
    st.on_worker_frame(&err_frame);

    assert_eq!(st.auth().status, AuthStatus::Unresolved);
    assert!(st.session_probe_error().is_some());
    for area in Area::ALL {
        assert!(
            !st.can_access_area(area),
            "failed session probe must keep {area:?} disabled"
        );
    }

    for (w, h) in [(360.0, 640.0), (1280.0, 800.0)] {
        let vp = Viewport::new(w, h);
        let html = render_console_with_skin(ConsoleSkin::Html, &st, vp);
        let shell = render_html_shell(&html, &st);
        assert!(shell.contains("Session verification failed"));
        assert!(shell.contains("HTTP 404"));
        assert!(!shell.contains("Session verification failedHTTP 404"));
        assert_eq!(
            shell
                .matches(r#"data-control-id="session-probe-retry""#)
                .count(),
            1,
            "cold-boot session error must render a unique Retry control"
        );
        assert!(shell.contains("session-probe-retry"));
        assert!(shell.contains(r#"href="/oauth2/sign_out?rd=%2F""#));
        assert!(!shell.contains("Sign in required"));
        assert!(!shell.contains("/oauth2/start"));
        assert!(!shell.to_ascii_lowercase().contains("invite"));

        let svg = render_console_with_skin(ConsoleSkin::Wireframe, &st, vp);
        roxmltree::Document::parse(&svg).expect("wireframe error SVG must be valid XML");
        assert!(svg.contains("Session verification failed"));
        assert!(svg.contains("session-probe-retry"));
        assert!(svg.contains("session-error-signout"));
    }

    let retry_cmds = st.on_dom_action("data-control-id", "session-probe-retry", false, 100);
    assert_eq!(retry_cmds, vec![UiCommand::Retry]);
    assert!(st.session_probe_error().is_none());

    st.on_worker_frame(&err_frame);
    assert!(st.session_probe_error().is_some());
    let signout_cmds = st.on_dom_action("data-control-id", "session-error-signout", false, 200);
    assert_eq!(
        signout_cmds,
        vec![UiCommand::Navigate {
            href: "/oauth2/sign_out?rd=%2F".into()
        }]
    );
}

#[test]
fn test_authorization_denied_with_anonymous_lockdown_renders_sign_out_not_sign_in_gate() {
    let mut st = ConsoleState::new();
    let vm = ViewModel {
        auth: AuthState::from_session(&AuthSessionDto {
            authenticated: false,
            invite_bound: false,
            github_login: Some("uninvited-lockdown".into()),
            reason_code: Some(AuthReasonCode::AuthorizationDenied),
            active_policies: vec![],
            anonymous_lockdown: true,
            ..AuthSessionDto::default()
        }),
        ..ViewModel::default()
    };
    let snap = encode(&DataEvent::Snapshot(vm)).expect("encode snapshot");
    st.on_worker_frame(&snap);

    assert_eq!(st.auth().status, AuthStatus::AuthorizationDenied);
    assert!(st.auth().anonymous_lockdown);

    for (w, h) in [(360.0, 640.0), (1280.0, 800.0)] {
        let vp = Viewport::new(w, h);
        let html = render_console_with_skin(ConsoleSkin::Html, &st, vp);
        let shell = render_html_shell(&html, &st);
        assert!(shell.contains("Authorization denied"));
        assert!(shell.contains("Sign Out"));
        assert!(shell.contains(r#"data-href="/oauth2/sign_out?rd=%2F""#));
        assert!(!shell.contains("Sign in required"));
        assert!(!shell.contains("login-gate-signin"));
        assert!(!shell.contains("/oauth2/start"));
        assert!(!shell.contains("uninvited-lockdown"));
        assert!(!shell.to_ascii_lowercase().contains("invite"));

        let svg = render_console_with_skin(ConsoleSkin::Wireframe, &st, vp);
        roxmltree::Document::parse(&svg).expect("wireframe denied lockdown SVG must be valid XML");
        assert!(svg.contains("Authorization denied"));
        assert!(!svg.contains("Sign in required"));
        assert!(!svg.contains("uninvited-lockdown"));
    }

    let cmds = st.on_dom_action("data-control-id", "auth-gate-signin", false, 300);
    assert_eq!(
        cmds,
        vec![UiCommand::Navigate {
            href: "/oauth2/sign_out?rd=%2F".into()
        }]
    );
}

#[test]
fn test_stashed_deep_link_hash_normalizes_oauth2_rd_and_restores_route() {
    let mut st = initialized_state();
    st.stashed_deep_link = Some("#/ci/scorecards?repo=LMA1980/ActionRunner-controller".into());

    let href = st.sign_in_href();
    assert_eq!(
        href,
        "/oauth2/start?rd=%2F%23%2Fci%2Fscorecards%3Frepo%3DLMA1980%2FActionRunner-controller"
    );
    assert!(
        validate_sign_in_href(&href).is_ok(),
        "normalized root-relative deep-link rd must pass validation: {href}"
    );

    // Reject bare hash, protocol-relative, and /oauth2* self-loop rd parameters.
    assert!(validate_sign_in_href("/oauth2/start?rd=%23%2Fci%2Fscorecards").is_err());
    assert!(validate_sign_in_href("/oauth2/start?rd=%2F%2Fattacker.example").is_err());
    assert!(validate_sign_in_href("/oauth2/start?rd=%2Foauth2%2Fstart").is_err());
    assert!(validate_sign_in_href("/oauth2/start?rd=%2Foauth2%2Fcallback").is_err());

    // Transition to Active restores Ci > Scorecards with LMA1980/ActionRunner-controller.
    let active_patch =
        Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(active_auth("alice"))]))
            .expect("encode active auth patch");
    st.on_worker_frame(&active_patch);
    assert_eq!(st.auth().status, AuthStatus::Active);
    assert_eq!(st.route.area, Area::Ci);
    assert_eq!(st.route.subview, SubView::Ci(CiSubView::Scorecards));
    assert_eq!(
        st.route.repo,
        Some(RepoId::new("LMA1980", "ActionRunner-controller"))
    );
}

#[test]
fn test_passkey_gate_surfaces_webauthn_errors_and_queues_step_up_when_active() {
    let mut st = ConsoleState::new();
    let gate_vm = ViewModel {
        auth: AuthState::from_session(&AuthSessionDto {
            authenticated: true,
            invite_bound: true,
            github_login: Some("alice".into()),
            passkey_count: 0,
            passkey_enrollment_required: true,
            reason_code: Some(AuthReasonCode::PasskeyEnrollmentRequired),
            ..AuthSessionDto::default()
        }),
        ..ViewModel::default()
    };
    let gate_snap = encode(&DataEvent::Snapshot(gate_vm)).expect("encode gate snapshot");
    st.on_worker_frame(&gate_snap);

    // Clicking dlg-passkey-register queues registration without step-up when passkey_count == 0.
    let cmds = st.on_dom_action("data-control-id", "dlg-passkey-register", false, 10);
    assert!(cmds.is_empty());
    assert_eq!(
        st.pending_webauthn.take(),
        Some(WebAuthnRequest::RegisterPasskey {
            label: Some("Security Key".into()),
            requires_step_up: false,
        })
    );

    // WebAuthn ceremony error updates aria_assertive and renders in HTML and valid XML Wireframe.
    st.on_webauthn_error("NotAllowedError: user cancelled ceremony");
    assert!(st
        .aria_assertive
        .contains("NotAllowedError: user cancelled ceremony"));

    for (w, h) in [(360.0, 640.0), (1280.0, 800.0)] {
        let vp = Viewport::new(w, h);
        let html = render_console_with_skin(ConsoleSkin::Html, &st, vp);
        assert!(html.contains("banner-webauthn-error"));
        assert!(html.contains("NotAllowedError: user cancelled ceremony"));
        assert!(!html.to_ascii_lowercase().contains("invite"));

        let svg = render_console_with_skin(ConsoleSkin::Wireframe, &st, vp);
        roxmltree::Document::parse(&svg).expect("wireframe passkey error SVG must be valid XML");
        assert!(svg.contains("NotAllowedError: user cancelled ceremony"));
        assert!(!svg.to_ascii_lowercase().contains("invite"));
    }

    // Retrying clears stale error.
    let _ = st.on_dom_action("data-control-id", "dlg-passkey-register", false, 20);
    assert!(st.webauthn_error.is_none());
    let _ = st.pending_webauthn.take();

    // Active session with passkey_count == 1 queues step-up for profile-passkey-add.
    let active_patch =
        Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(active_auth("alice"))]))
            .expect("encode active auth");
    st.on_worker_frame(&active_patch);
    let _ = st.on_dom_action("data-control-id", "profile-passkey-add", false, 30);
    assert_eq!(
        st.pending_webauthn.take(),
        Some(WebAuthnRequest::RegisterPasskey {
            label: Some("Security Key".into()),
            requires_step_up: true,
        })
    );

    // Envelope helpers format controller-compatible JSON.
    let start = register_start_body(
        Some("Security Key"),
        Some("ch-step"),
        Some(r#"{"id":"pk0","type":"public-key"}"#),
    );
    assert!(start.contains(r#""step_up_challenge_id":"ch-step""#));
    assert!(start.contains(r#""step_up_credential":{"id":"pk0","type":"public-key"}"#));
    let finish = wrap_credential_envelope_json(Some("ch-reg"), r#"{"id":"pk1"}"#);
    assert_eq!(
        finish,
        r#"{"challenge_id":"ch-reg","credential":{"id":"pk1"}}"#
    );
}
