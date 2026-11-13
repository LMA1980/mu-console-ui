// SPDX-License-Identifier: AGPL-3.0-only
//! Empirical Stress Tests and Adversarial Challenges for `muConsole`.
//!
//! Validates:
//! 1. WireframeSkin Security: exhaustive permutations of `?skin=`, `localStorage`,
//!    `[S]` keypress, and DOM button clicks cannot breach WireframeSkin.
//! 2. Telemetry Authenticity: `mu-console-data/src/store.rs` produces authentic
//!    empty states when unhydrated, with zero synthetic mock data.

#![cfg(not(miri))]

use mu_console_data::store::{Input, Store};
use mu_console_protocol::dto::{
    CiQueueDto, IngressOverviewDto, IngressRouteDto, IngressUpstreamDto, OtelLogDto,
};
use mu_console_protocol::{ConsoleSkin, DataSource, LogSeverity, UiCommand};
use mu_console_ui::state::{parse_skin_pref, ConsoleState};

// ============================================================================
// 1. WireframeSkin Security Permutations & Bypass Resistance
// ============================================================================

#[test]
fn test_empirical_skin_query_and_storage_permutations_unbreachable() {
    let queries = [
        "?skin=legacy",
        "?skin=wood",
        "?skin=crt",
        "?skin=scanlines",
        "?skin=wireframe",
        "?skin=",
        "?foo=1&skin=legacy&bar=2",
        "?skin=../../etc/shadow",
        "?skin=default",
        "",
    ];

    let stored_values = [
        Some("legacy"),
        Some("wood"),
        Some("crt"),
        Some("scanlines"),
        Some("wireframe"),
        Some(""),
        Some("malicious_skin_token"),
        None,
    ];

    // Challenge all 80 permutations of URL query param x localStorage
    for query in &queries {
        for stored in &stored_values {
            let skin = parse_skin_pref(query, *stored);
            assert_eq!(
                skin,
                ConsoleSkin::Wireframe,
                "WireframeSkin bypassed with query: '{query}', stored: '{stored:?}'"
            );
        }
    }
}

#[test]
fn test_empirical_skin_state_methods_locked_to_wireframe() {
    let mut state = ConsoleState::new();

    // Direct setter attempt
    let cmd = state.set_skin(ConsoleSkin::Wireframe);
    assert_eq!(state.skin, ConsoleSkin::Wireframe);
    assert_eq!(cmd, UiCommand::SetSkin(ConsoleSkin::Wireframe));

    // Toggle attempt
    let cmd = state.toggle_skin();
    assert_eq!(state.skin, ConsoleSkin::Wireframe);
    assert_eq!(cmd, UiCommand::SetSkin(ConsoleSkin::Wireframe));

    // Restore from localStorage attempt
    state.restore_stored_skin("?skin=legacy", Some("legacy"));
    assert_eq!(state.skin, ConsoleSkin::Wireframe);
}

#[test]
fn test_empirical_skin_keyboard_and_dom_action_resistance() {
    let mut state = ConsoleState::new();

    // Hotkey [s] and [S] must be neutralized (return None)
    assert!(
        state.on_key("s", false, 1_000).is_none(),
        "Lowercase 's' hotkey must be disabled"
    );
    assert!(
        state.on_key("S", false, 1_000).is_none(),
        "Uppercase 'S' hotkey must be disabled"
    );
    assert!(
        state.on_key("s", true, 1_000).is_none(),
        "Shift+s hotkey must be disabled"
    );
    assert_eq!(state.skin, ConsoleSkin::Wireframe);

    // DOM click action attempts must remain Wireframe
    let cmds1 = state.on_dom_action("data-action", "muconsole-skin-toggle", false, 1_000);
    assert_eq!(cmds1, vec![UiCommand::SetSkin(ConsoleSkin::Wireframe)]);
    assert_eq!(state.skin, ConsoleSkin::Wireframe);

    let cmds2 = state.on_dom_action("data-action", "c2-skin-switch", false, 1_000);
    assert_eq!(cmds2, vec![UiCommand::SetSkin(ConsoleSkin::Wireframe)]);
    assert_eq!(state.skin, ConsoleSkin::Wireframe);

    let cmds3 = state.on_dom_action("data-skin-toggle", "", false, 1_000);
    assert_eq!(cmds3, vec![UiCommand::SetSkin(ConsoleSkin::Wireframe)]);
    assert_eq!(state.skin, ConsoleSkin::Wireframe);
}

// ============================================================================
// 2. Telemetry Authenticity (store.rs zero synthetic mock data)
// ============================================================================

#[test]
fn test_empirical_store_live_empty_endpoints_zero_synthetic_data() {
    let mut store = Store::new(DataSource::Live);

    // Apply empty unhydrated cluster inputs
    store.apply(Input::Repos(vec![]), 1_000);
    store.apply(Input::ClusterNodes(vec![]), 1_000);
    store.apply(
        Input::CiQueue(CiQueueDto {
            queue_depth: 0,
            concurrency: 0,
            active_jobs: vec![],
            queued_jobs: vec![],
            recent_jobs: vec![],
        }),
        1_000,
    );
    store.apply(
        Input::ObsIngressOverview(IngressOverviewDto {
            routes: vec![],
            upstreams: vec![],
            tls_health: None,
        }),
        1_000,
    );

    let view = store.view();

    // Verify 100% authentic empty states (zero synthetic mock telemetry)
    assert!(
        view.observe.smartscape_nodes.is_empty(),
        "Expected 0 smartscape nodes, got: {} (synthetic mock nodes detected!)",
        view.observe.smartscape_nodes.len()
    );
    assert!(
        view.observe.smartscape_edges.is_empty(),
        "Expected 0 smartscape edges, got: {}",
        view.observe.smartscape_edges.len()
    );
    assert!(
        view.observe.logs.is_empty(),
        "Expected 0 logs, got: {} (synthetic fallback logs detected!)",
        view.observe.logs.len()
    );
    assert!(
        view.observe.metrics.is_empty(),
        "Expected 0 metrics, got: {} (synthetic fallback metrics detected!)",
        view.observe.metrics.len()
    );
    assert!(
        view.observe.problems.is_empty(),
        "Expected 0 problems, got: {} (synthetic fallback problems detected!)",
        view.observe.problems.len()
    );
    assert!(
        view.observe.traces.is_empty(),
        "Expected 0 traces, got: {}",
        view.observe.traces.len()
    );
    assert!(
        view.observe.ingress_routes.is_empty(),
        "Expected 0 ingress routes, got: {}",
        view.observe.ingress_routes.len()
    );
    assert!(
        view.observe.ingress_upstreams.is_empty(),
        "Expected 0 ingress upstreams, got: {}",
        view.observe.ingress_upstreams.len()
    );
    assert!(
        view.observe.tls_health.is_none(),
        "Expected None for TLS health"
    );
}

#[test]
fn test_empirical_store_live_authentic_hydration_and_zero_mock_fallback() {
    let mut store = Store::new(DataSource::Live);

    // Feed authentic live ingress routes
    let authentic_overview = IngressOverviewDto {
        routes: vec![
            IngressRouteDto {
                route_id: "route-prod-api".into(),
                path_prefix: "/api/prod/".into(),
                upstream_url: "http://10.0.0.1:8080/".into(),
                service_name: "prod-service".into(),
                strip_prefix: false,
                auth_required: true,
                request_count: 500,
                request_rate_rps: 42.5,
                error_rate_pct: 0.2,
                p95_latency_ms: 12.8,
            },
            IngressRouteDto {
                route_id: "route-auth".into(),
                path_prefix: "/api/auth/".into(),
                upstream_url: "http://10.0.0.2:8443/".into(),
                service_name: "auth-service".into(),
                strip_prefix: true,
                auth_required: false,
                request_count: 1200,
                request_rate_rps: 110.0,
                error_rate_pct: 0.0,
                p95_latency_ms: 4.1,
            },
        ],
        upstreams: vec![IngressUpstreamDto {
            upstream_id: "upstream-prod".into(),
            target_url: "http://10.0.0.1:8080/".into(),
            service_name: "prod-service".into(),
            health_state: "HEALTHY".into(),
            latency_ms: 3.2,
            success_rate_pct: 99.8,
            active_connections: 14,
            last_check_ms: 1_700_000_000_000,
        }],
        tls_health: None,
    };

    store.apply(Input::ObsIngressOverview(authentic_overview), 1_000);

    // Feed authentic live log
    let authentic_log = OtelLogDto {
        timestamp_ms: 1_700_000_000_100,
        trace_id: "0123456789abcdef0123456789abcdef".into(),
        span_id: Some("0123456789abcdef".into()),
        severity: LogSeverity::Info,
        service: "prod-service".into(),
        message: "Authentic production log line".into(),
    };

    store.apply(Input::ObsLogs(vec![authentic_log]), 1_000);

    let view = store.view();

    // Verify view has exactly the authentic items fed
    assert_eq!(view.observe.ingress_routes.len(), 2);
    assert_eq!(view.observe.ingress_routes[0].route_id, "route-prod-api");
    assert_eq!(view.observe.ingress_routes[1].route_id, "route-auth");
    assert_eq!(view.observe.ingress_upstreams.len(), 1);
    assert_eq!(
        view.observe.ingress_upstreams[0].upstream_id,
        "upstream-prod"
    );

    assert_eq!(view.observe.logs.len(), 1);
    assert_eq!(
        view.observe.logs[0].message,
        "Authentic production log line"
    );

    // Other views that were not hydrated remain strictly empty (no synthetic fallbacks!)
    assert!(view.observe.smartscape_nodes.is_empty());
    assert!(view.observe.metrics.is_empty());
    assert!(view.observe.problems.is_empty());
}
