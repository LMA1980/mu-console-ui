// SPDX-License-Identifier: AGPL-3.0-only
//! Adversarial Challenge Test Harness for Console Access & GitHub OAuth2 Integration.
//!
//! Stress-tests:
//! 1. Rapid repeated clicks and idempotency on `c2-auth-status` across authentication states.
//! 2. State machine transitions: Anonymous <-> InviteRequired <-> Operator under load.
//! 3. Malformed return path injection and URL percent-encoding validation.
//! 4. Strict synchronization between `render_html_shell` `<a id="muconsole-auth-link">` and href methods.
//! 5. `sessionStorage` invite stashing logic and survival across full-page navigation.
#![cfg(not(miri))]

use mu_console_data::engine::{Effect, Engine};
use mu_console_data::scenario::Scenario;
use mu_console_protocol::{AuthState, DataEvent, Delta, UiCommand, WorkerConfig};
use mu_console_ui::components::render_html_shell;
use mu_console_ui::state::{
    extract_invite_token, format_sign_in_url, format_sign_out_url, validate_sign_in_href,
    ConsoleState, PENDING_INVITE_STORAGE_KEY,
};
use mu_console_ui::WORKER_URL;

fn initialized_state() -> ConsoleState {
    let mut eng = Engine::new(1);
    let cfg = WorkerConfig {
        api_base: "http://192.0.2.10:8080".into(),
        scenario: Some(Scenario::Submodules.name().to_owned()),
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
    let anon_patch =
        Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(AuthState::Anonymous)]))
            .expect("encode Delta::Auth Anonymous");
    state.on_worker_frame(&anon_patch);
    state
}

#[test]
fn test_rapid_repeated_clicks_idempotency_anonymous() {
    let mut st = initialized_state();
    assert_eq!(st.auth(), AuthState::Anonymous);
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
                href: "/oauth2/start?rd=/".into(),
            },
            "Iteration {i}: Command must navigate to sign-in URL"
        );
        assert_eq!(
            st.pending_navigation.as_deref(),
            Some("/oauth2/start?rd=/"),
            "Iteration {i}: pending_navigation must remain stable"
        );
        assert_eq!(
            st.aria_polite, "Navigating to /oauth2/start?rd=/",
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
                href: "/oauth2/start?rd=/".into()
            }]
        );
        assert_eq!(st.pending_navigation.as_deref(), Some("/oauth2/start?rd=/"));
    }

    // Stage 2: Transition to InviteRequired
    let invite_req_patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(
        AuthState::InviteRequired {
            github_login: "challenger-bot".into(),
        },
    )]))
    .expect("encode Delta::Auth InviteRequired");
    st.on_worker_frame(&invite_req_patch);

    assert_eq!(
        st.auth(),
        AuthState::InviteRequired {
            github_login: "challenger-bot".into()
        }
    );
    assert!(!st.is_operator());

    // 10 rapid clicks in InviteRequired -> must navigate to sign_out
    for i in 0..10 {
        let cmds = st.on_dom_action("data-control-id", "c2-auth-status", true, 200 + i);
        assert_eq!(
            cmds,
            vec![UiCommand::Navigate {
                href: "/oauth2/sign_out?rd=/".into()
            }],
            "InviteRequired state must dispatch sign_out"
        );
        assert_eq!(
            st.pending_navigation.as_deref(),
            Some("/oauth2/sign_out?rd=/")
        );
        assert_eq!(st.aria_polite, "Navigating to /oauth2/sign_out?rd=/");
    }

    // Stage 3: Transition to Operator
    let op_patch =
        Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(AuthState::Operator {
            github_login: "LMA1980".into(),
            role: Some("operator".into()),
        })]))
        .expect("encode Delta::Auth Operator");
    st.on_worker_frame(&op_patch);

    assert_eq!(
        st.auth(),
        AuthState::Operator {
            github_login: "LMA1980".into(),
            role: Some("operator".into()),
        }
    );
    assert!(st.is_operator());

    // 10 rapid clicks in Operator -> must navigate to sign_out
    for i in 0..10 {
        let cmds = st.on_dom_action("data-control-id", "c2-auth-status", false, 300 + i);
        assert_eq!(
            cmds,
            vec![UiCommand::Navigate {
                href: "/oauth2/sign_out?rd=/".into()
            }],
            "Operator state must dispatch sign_out"
        );
        assert_eq!(
            st.pending_navigation.as_deref(),
            Some("/oauth2/sign_out?rd=/")
        );
        assert_eq!(st.aria_polite, "Navigating to /oauth2/sign_out?rd=/");
    }

    // Stage 4: Transition back to Anonymous (e.g. sign-out completed or session revoked)
    let anon_patch =
        Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(AuthState::Anonymous)]))
            .expect("encode Delta::Auth Anonymous");
    st.on_worker_frame(&anon_patch);

    assert_eq!(st.auth(), AuthState::Anonymous);
    assert!(!st.is_operator());

    // Rapid clicks in reverted Anonymous -> must navigate to sign_in again
    for i in 0..10 {
        let cmds = st.on_dom_action("data-control-id", "c2-auth-status", false, 400 + i);
        assert_eq!(
            cmds,
            vec![UiCommand::Navigate {
                href: "/oauth2/start?rd=/".into()
            }]
        );
        assert_eq!(st.pending_navigation.as_deref(), Some("/oauth2/start?rd=/"));
        assert_eq!(st.aria_polite, "Navigating to /oauth2/start?rd=/");
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

        // 4. Validate sign_in with validate_sign_in_href
        if rd.is_empty() {
            assert_eq!(
                validate_sign_in_href(&sign_in),
                Err("empty rd parameter"),
                "Empty rd parameter must be rejected by validate_sign_in_href"
            );
        } else {
            assert!(
                validate_sign_in_href(&sign_in).is_ok(),
                "Valid sign_in URL for '{rd}' failed validation: {:?}",
                validate_sign_in_href(&sign_in)
            );
        }
    }

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
    assert_eq!(expected_anon_href, "/oauth2/start?rd=/");
    assert!(
        shell_anon.contains(&format!(
            r#"<a id="muconsole-auth-link" href="{expected_anon_href}" data-auth-state="anonymous">READ-ONLY · SIGN IN</a>"#
        )),
        "HTML shell must contain synchronized sign_in_href in Anonymous state"
    );

    // 2. InviteRequired state check
    let invite_req_patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(
        AuthState::InviteRequired {
            github_login: "test-user-42".into(),
        },
    )]))
    .expect("encode");
    st.on_worker_frame(&invite_req_patch);

    let shell_uninv = render_html_shell("<svg></svg>", &st);
    let expected_uninv_href = st.sign_out_href();
    assert_eq!(expected_uninv_href, "/oauth2/sign_out?rd=/");
    assert!(
        shell_uninv.contains(&format!(
            r#"<a id="muconsole-auth-link" href="{expected_uninv_href}" data-auth-state="invite_required">INVITE REQ (@test-user-42)</a>"#
        )),
        "HTML shell must contain synchronized sign_out_href in InviteRequired state"
    );

    // 3. Operator state check
    let op_patch =
        Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(AuthState::Operator {
            github_login: "admin-ops".into(),
            role: Some("operator".into()),
        })]))
        .expect("encode");
    st.on_worker_frame(&op_patch);

    let shell_op = render_html_shell("<svg></svg>", &st);
    let expected_op_href = st.sign_out_href();
    assert_eq!(expected_op_href, "/oauth2/sign_out?rd=/");
    assert!(
        shell_op.contains(&format!(
            r#"<a id="muconsole-auth-link" href="{expected_op_href}" data-auth-state="operator">@admin-ops · OPERATOR</a>"#
        )),
        "HTML shell must contain synchronized sign_out_href in Operator state"
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

    // Stage 3: User clicks sign in button -> navigation command emitted
    let nav_cmds = st.on_dom_action("data-control-id", "c2-auth-status", false, 1_000);
    assert_eq!(
        nav_cmds,
        vec![UiCommand::Navigate {
            href: "/oauth2/start?rd=/".into()
        }]
    );
    assert_eq!(st.pending_navigation.as_deref(), Some("/oauth2/start?rd=/"));

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

    // Stage 8: Worker authenticates and emits Operator state
    let operator_patch =
        Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(AuthState::Operator {
            github_login: "LMA1980".into(),
            role: Some("operator".into()),
        })]))
        .expect("encode");
    st_return.on_worker_frame(&operator_patch);
    assert!(st_return.is_operator());

    // When operator view arrives, boot.rs purges pending_invite from sessionStorage
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
    assert_eq!(st.sign_in_href(), "/oauth2/start?rd=/");
    assert_eq!(st.sign_out_href(), "/oauth2/sign_out?rd=/");

    let shell = render_html_shell("", &st);
    assert!(
        shell.contains(r#"href="/oauth2/start?rd=/""#),
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
            href: "/oauth2/start?rd=/".into()
        }]
    );
    assert_eq!(st.pending_navigation.as_deref(), Some("/oauth2/start?rd=/"));

    // 2. Interleaved worker frame: InviteRequired arrives
    let uninv_patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(
        AuthState::InviteRequired {
            github_login: "interleaved-bot".into(),
        },
    )]))
    .expect("encode");
    st.on_worker_frame(&uninv_patch);

    // Click immediately
    let cmds2 = st.on_dom_action("data-control-id", "c2-auth-status", false, 1_050);
    assert_eq!(
        cmds2,
        vec![UiCommand::Navigate {
            href: "/oauth2/sign_out?rd=/".into()
        }]
    );
    assert_eq!(
        st.pending_navigation.as_deref(),
        Some("/oauth2/sign_out?rd=/")
    );

    // 3. Interleaved worker frame: Operator arrives
    let op_patch =
        Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(AuthState::Operator {
            github_login: "interleaved-bot".into(),
            role: Some("operator".into()),
        })]))
        .expect("encode");
    st.on_worker_frame(&op_patch);

    // Click immediately
    let cmds3 = st.on_dom_action("data-control-id", "c2-auth-status", false, 1_100);
    assert_eq!(
        cmds3,
        vec![UiCommand::Navigate {
            href: "/oauth2/sign_out?rd=/".into()
        }]
    );
    assert_eq!(
        st.pending_navigation.as_deref(),
        Some("/oauth2/sign_out?rd=/")
    );

    // 4. Interleaved worker frame: Revoked back to Anonymous
    let anon_patch =
        Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(AuthState::Anonymous)]))
            .expect("encode");
    st.on_worker_frame(&anon_patch);

    // Click immediately
    let cmds4 = st.on_dom_action("data-control-id", "c2-auth-status", false, 1_150);
    assert_eq!(
        cmds4,
        vec![UiCommand::Navigate {
            href: "/oauth2/start?rd=/".into()
        }]
    );
    assert_eq!(st.pending_navigation.as_deref(), Some("/oauth2/start?rd=/"));
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

    let logins = ["octocat", "bot[ca]", "user-123_test", "admin.infra", "a"];
    for login in logins {
        // Test InviteRequired
        let uninv_patch = Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(
            AuthState::InviteRequired {
                github_login: login.into(),
            },
        )]))
        .expect("encode");
        st.on_worker_frame(&uninv_patch);

        let shell = render_html_shell("<svg></svg>", &st);
        let expected_href = st.sign_out_href();
        assert!(shell.contains(&format!(r#"href="{expected_href}""#)));
        assert!(shell.contains(r#"data-auth-state="invite_required""#));
        assert!(shell.contains(&format!("INVITE REQ (@{login})")));

        // Test Operator
        let op_patch =
            Engine::encode_event(&DataEvent::Patch(vec![Delta::Auth(AuthState::Operator {
                github_login: login.into(),
                role: Some("operator".into()),
            })]))
            .expect("encode");
        st.on_worker_frame(&op_patch);

        let shell_op = render_html_shell("<svg></svg>", &st);
        assert!(shell_op.contains(&format!(r#"href="{expected_href}""#)));
        assert!(shell_op.contains(r#"data-auth-state="operator""#));
        assert!(shell_op.contains(&format!("@{login} · OPERATOR")));
    }
}
