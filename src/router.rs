// SPDX-License-Identifier: AGPL-3.0-only
//! Deep-link hash router, legacy alias normalization, breadcrumbs, and minimal-access gating
//! across all 8 canonical console areas and 27 sub-views.

use mu_console_layout::Breakpoint;
use mu_console_protocol::{
    AbDeploySubView, Area, AuthState, AuthStatus, CdSubView, CiSubView, DashboardSubView,
    ModelRunnerSubView, ObservabilitySubView, PlatformSubView, RepoId, RepositorySubView, Route,
    SubView, SubmoduleId,
};

/// Canonical URL path slug for `area`.
#[must_use]
pub const fn area_slug(area: Area) -> &'static str {
    area.slug()
}

/// Human-facing tab label for `area`.
#[must_use]
pub const fn area_label(area: Area) -> &'static str {
    area.label()
}

/// Parse an area slug (including legacy aliases such as `ovr`, `fleet`, `reg`, `obs`, `iam`).
#[must_use]
pub fn parse_area(s: &str) -> Option<Area> {
    Area::from_slug(s)
}

/// Parse a sub-view slug inside `area` (accepting M2 canonical and legacy sub-view slugs).
#[must_use]
pub fn parse_subview(area: Area, slug: &str) -> Option<SubView> {
    SubView::from_area_and_slug(area, slug)
}

/// Return `true` when `hash` explicitly names an area path (as opposed to bare `/` or `#/`).
#[must_use]
pub fn has_explicit_route(hash: &str) -> bool {
    let trimmed = hash.trim();
    let no_hash = trimmed.strip_prefix('#').unwrap_or(trimmed);
    let path = match no_hash.split_once('?') {
        Some((p, _)) => p,
        None => no_hash,
    };
    !path.trim_matches('/').is_empty()
}

fn percent_decode_lossy(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

const fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn parse_query_params(route: &mut Route, query: &str) {
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, raw_v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = percent_decode_lossy(raw_v);
        match k {
            "repo" => {
                if let Some((owner, repo)) = v.split_once('/') {
                    if !owner.is_empty() && !repo.is_empty() {
                        route.repo = Some(RepoId::new(owner, repo));
                    }
                }
            }
            "submodule" if !v.is_empty() => route.submodule = Some(SubmoduleId { path: v }),
            "branch" if !v.is_empty() => route.branch = Some(v),
            "job" => route.job = v.parse::<u64>().ok(),
            "trace" if !v.is_empty() => route.trace_id = Some(v),
            "filter" if !v.is_empty() => route.filter = Some(v),
            _ => {}
        }
    }
}

/// Parse a URL hash fragment (`#/{area}/{subview}?...`), preserving query filters on bare `#/?...`
/// and mapping legacy routes (`#/iam`, `#/fleet`, `#/cd/gate`, `#/registry`, `#/observe`, `#/ovr`)
/// cleanly to their canonical v6 targets.
#[must_use]
pub fn parse_hash(hash: &str) -> Route {
    if let Some(mut route) = Route::try_from_hash(hash) {
        if let Some((_, query)) = hash
            .trim()
            .strip_prefix('#')
            .unwrap_or(hash.trim())
            .split_once('?')
        {
            route.submodule = None;
            route.branch = None;
            route.trace_id = None;
            route.filter = None;
            parse_query_params(&mut route, query);
        }
        return route;
    }

    let mut route = Route::default();
    let trimmed = hash.trim();
    let no_hash = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if let Some((_, query)) = no_hash.split_once('?') {
        parse_query_params(&mut route, query);
    }
    route
}

/// Serialize `route` back into a canonical URL hash fragment.
#[must_use]
pub fn format_hash(route: &Route) -> String {
    route.to_hash()
}

/// Step upward one hierarchy level (`filter` -> `trace` -> `job` -> `branch` -> `submodule` ->
/// default sub-view -> `Area::Dashboard`), preserving active repo context.
#[must_use]
pub fn escape_up(route: &Route) -> Route {
    let mut next = route.clone();
    if next.filter.take().is_some()
        || next.trace_id.take().is_some()
        || next.job.take().is_some()
        || next.branch.take().is_some()
        || next.submodule.take().is_some()
    {
        return next;
    }

    let area_default = SubView::default_for(next.area);
    if next.subview != area_default {
        next.subview = area_default;
        return next;
    }

    if next.area != Area::Dashboard {
        next.area = Area::Dashboard;
        next.subview = SubView::Dashboard(DashboardSubView::Overview);
    }
    next
}

/// Return whether any active role in `auth` satisfies the minimal-access requirement for `subview`.
///
/// Cold-boot unresolved sessions, locked-down anonymous sessions, and mandatory 0-Passkey gates
/// fail closed across every sub-view. Unauthenticated browsing is limited to Dashboard and
/// `CI > Queue & Job Steps` when permitted by `RoleFolioIndex["anonymous"]`.
#[must_use]
pub fn can_access_subview(auth: &AuthState, subview: SubView) -> bool {
    if !auth.is_resolved()
        || auth.status == AuthStatus::PasskeyEnrollmentRequired
        || auth.anonymous_lockdown
        || !auth.can_access_area(subview.area())
    {
        return false;
    }

    let has_any = |policies: &[&str]| policies.iter().any(|p| auth.has_policy(p));
    if auth.status != AuthStatus::Active {
        return match subview {
            SubView::Dashboard(DashboardSubView::Overview) => {
                has_any(&["system:dashboard.read", "system:status.read"])
                    || auth.can_access_area(Area::Dashboard)
            }
            SubView::Ci(CiSubView::QueueLogs) => has_any(&[
                "ci:queue.read",
                "ci:jobs.read",
                "ci:jobs:read",
                "ci:repos.public.read",
            ]),
            _ => false,
        };
    }

    match subview {
        SubView::Dashboard(DashboardSubView::Overview) => {
            has_any(&["system:dashboard.read", "system:status.read"])
                || auth.can_access_area(Area::Dashboard)
        }
        SubView::Ci(CiSubView::Scorecards) => has_any(&[
            "ci:queue.read",
            "ci:repos.public.read",
            "ci:repos.private.read",
            "ci:jobs.read",
            "ci:jobs:read",
        ]),
        SubView::Ci(CiSubView::Trainway) => {
            has_any(&["ci:queue.read", "ci:jobs.read", "ci:jobs:read"])
        }
        SubView::Ci(CiSubView::QueueLogs) => has_any(&[
            "ci:queue.read",
            "ci:jobs.read",
            "ci:jobs:read",
            "ci:repos.public.read",
        ]),
        SubView::Ci(CiSubView::WorkerNodes) => has_any(&["ci:nodes.read", "ci:ha.read"]),
        SubView::Ci(CiSubView::SandboxProfiles) => has_any(&["ci:sandboxes.read"]),
        SubView::Cd(CdSubView::Runs) => has_any(&["cd:runs.read", "cd:runs:read"]),
        SubView::Cd(CdSubView::Pipelines) => {
            has_any(&["cd:steps.read", "cd:runs.read", "cd:runs:read"])
        }
        SubView::AbDeploy(AbDeploySubView::IntegrationGate)
        | SubView::AbDeploy(AbDeploySubView::DeploymentsSlots) => {
            has_any(&["ab:slots.read", "ab:deploy.read", "ab:deploy:read"])
        }
        SubView::AbDeploy(AbDeploySubView::GateTestLogs) => {
            has_any(&["ab:gate.logs.read", "ab:logs.read"])
        }
        SubView::AbDeploy(AbDeploySubView::ActiveLeases) => {
            has_any(&["ab:slots.read", "ab:leases.read"])
        }
        SubView::Repository(RepositorySubView::Packages) => {
            has_any(&["reg:packages.read", "reg:read"])
        }
        SubView::Repository(RepositorySubView::CanaryTaints) => {
            has_any(&["reg:canaries.read", "reg:taints.read"])
        }
        SubView::Repository(RepositorySubView::CasHealth) => has_any(&["reg:cas.read"]),
        SubView::Observability(ObservabilitySubView::Smartscape) => {
            has_any(&["obs:smartscape.read"])
        }
        SubView::Observability(ObservabilitySubView::PurePath) => has_any(&["obs:traces.read"]),
        SubView::Observability(ObservabilitySubView::ProblemsSlos) => {
            has_any(&["obs:problems.read"])
        }
        SubView::Observability(ObservabilitySubView::Logs) => has_any(&["obs:logs.read"]),
        SubView::Observability(ObservabilitySubView::Metrics) => has_any(&["obs:metrics.read"]),
        SubView::Observability(ObservabilitySubView::DataStore) => has_any(&["obs:store.read"]),
        SubView::Observability(ObservabilitySubView::Ingress) => has_any(&["obs:ingress.read"]),
        SubView::ModelRunner(ModelRunnerSubView::RuntimeQueue) => {
            has_any(&["mr:models.read", "mr:queue.read", "mr:status:read"])
        }
        SubView::ModelRunner(ModelRunnerSubView::ConnectionVpn) => {
            has_any(&["mr:openai_api.read", "mr:wireguard_vpn.read"])
        }
        SubView::Platform(PlatformSubView::Maintenance) => {
            has_any(&["platform:maintenance.read", "platform:maintenance.write"])
        }
        SubView::Platform(PlatformSubView::UsersAndInvites) => has_any(&[
            "platform:users.read",
            "platform:invites.read",
            "platform:users.write",
            "platform:invites.write",
        ]),
        SubView::Platform(PlatformSubView::RolesAndFolios) => has_any(&[
            "platform:roles.read",
            "platform:folios.read",
            "platform:roles.write",
            "platform:folios.write",
        ]),
    }
}

/// Interactive breadcrumb item derived from the current route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreadcrumbCrumb {
    pub label: String,
    pub route: Option<Route>,
}

/// Build responsive breadcrumb trail for `route` at `bp`.
#[must_use]
pub fn breadcrumb_segments(route: &Route, bp: Breakpoint) -> Vec<BreadcrumbCrumb> {
    if bp == Breakpoint::Phone {
        return vec![
            BreadcrumbCrumb {
                label: route.area.short_label().to_owned(),
                route: Some(Route::for_area(route.area)),
            },
            BreadcrumbCrumb {
                label: route.subview.label().to_owned(),
                route: None,
            },
        ];
    }

    let mut crumbs = vec![
        BreadcrumbCrumb {
            label: route.area.label().to_owned(),
            route: Some(Route::for_area(route.area)),
        },
        BreadcrumbCrumb {
            label: route.subview.label().to_owned(),
            route: Some(Route::for_subview(route.subview)),
        },
    ];

    if let Some(repo) = &route.repo {
        crumbs.push(BreadcrumbCrumb {
            label: format!("{}/{}", repo.owner, repo.repo),
            route: None,
        });
    }
    if let Some(sub) = &route.submodule {
        crumbs.push(BreadcrumbCrumb {
            label: sub.path.clone(),
            route: None,
        });
    }
    if let Some(branch) = &route.branch {
        crumbs.push(BreadcrumbCrumb {
            label: branch.clone(),
            route: None,
        });
    }
    if let Some(job) = route.job {
        crumbs.push(BreadcrumbCrumb {
            label: format!("#{job}"),
            route: None,
        });
    }
    if let Some(trace) = &route.trace_id {
        crumbs.push(BreadcrumbCrumb {
            label: trace.clone(),
            route: None,
        });
    }
    crumbs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_eight_areas_and_legacy_aliases() {
        for area in Area::ALL {
            let r = parse_hash(&format!("#/{}", area.slug()));
            assert_eq!(r.area, area);
            assert_eq!(r.subview, SubView::default_for(area));
        }
        assert_eq!(
            parse_hash("#/iam/users").subview,
            SubView::Platform(PlatformSubView::UsersAndInvites)
        );
        assert_eq!(
            parse_hash("#/fleet").subview,
            SubView::Ci(CiSubView::WorkerNodes)
        );
        assert_eq!(
            parse_hash("#/cd/gate").subview,
            SubView::AbDeploy(AbDeploySubView::IntegrationGate)
        );
        assert_eq!(
            parse_hash("#/observe/purepath").subview,
            SubView::Observability(ObservabilitySubView::PurePath)
        );
        assert_eq!(
            parse_hash("#/registry/canaries").subview,
            SubView::Repository(RepositorySubView::CanaryTaints)
        );
        assert!(!has_explicit_route(""));
        assert!(!has_explicit_route("#/"));
        assert!(has_explicit_route("#/dashboard"));
    }

    #[test]
    fn round_trips_percent_encoded_params_and_escape_up() {
        let parsed = parse_hash("#/ci/trainway?repo=LMA1980/mu-console&branch=feat%2Fm2-ui&job=99");
        assert_eq!(parsed.area, Area::Ci);
        assert_eq!(parsed.subview, SubView::Ci(CiSubView::Trainway));
        assert_eq!(parsed.repo, Some(RepoId::new("LMA1980", "mu-console")));
        assert_eq!(parsed.branch.as_deref(), Some("feat/m2-ui"));
        assert_eq!(parsed.job, Some(99));

        let step1 = escape_up(&parsed);
        assert_eq!(step1.job, None);
        assert_eq!(step1.branch.as_deref(), Some("feat/m2-ui"));

        let step2 = escape_up(&step1);
        assert_eq!(step2.branch, None);

        let step3 = escape_up(&step2);
        assert_eq!(step3.subview, SubView::Ci(CiSubView::Scorecards));

        let step4 = escape_up(&step3);
        assert_eq!(step4.area, Area::Dashboard);
        assert_eq!(step4.repo, Some(RepoId::new("LMA1980", "mu-console")));
    }

    #[test]
    fn minimal_access_enforces_fail_closed_and_separate_gate_logs() {
        assert!(!can_access_subview(
            &AuthState::unresolved(),
            SubView::Dashboard(DashboardSubView::Overview)
        ));

        let anon = AuthState {
            status: AuthStatus::Anonymous,
            active_policies: vec![
                "system:dashboard.read".into(),
                "ci:queue.read".into(),
                "ci:repos.public.read".into(),
            ],
            anonymous_lockdown: false,
            ..AuthState::default()
        };
        assert!(can_access_subview(
            &anon,
            SubView::Dashboard(DashboardSubView::Overview)
        ));
        assert!(can_access_subview(&anon, SubView::Ci(CiSubView::QueueLogs)));
        assert!(!can_access_subview(
            &anon,
            SubView::Ci(CiSubView::WorkerNodes)
        ));
        assert!(!can_access_subview(
            &anon,
            SubView::Platform(PlatformSubView::Maintenance)
        ));

        let ab_reader = AuthState {
            status: AuthStatus::Active,
            passkey_count: 1,
            active_policies: vec!["ab:slots.read".into()],
            ..AuthState::default()
        };
        assert!(can_access_subview(
            &ab_reader,
            SubView::AbDeploy(AbDeploySubView::DeploymentsSlots)
        ));
        assert!(!can_access_subview(
            &ab_reader,
            SubView::AbDeploy(AbDeploySubView::GateTestLogs)
        ));
    }
}
