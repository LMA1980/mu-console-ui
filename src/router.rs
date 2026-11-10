// SPDX-License-Identifier: AGPL-3.0-only
//! Hash router (`#/{area}/{subview}[/{owner}/{repo}...][?trace=...&filter=...]`),
//! legacy `#/ovr` / `#/overview` -> `#/ci/scorecards` and `#/jobs/{id}` redirects,
//! `Esc` up-one-level navigation and breakpoint-aware C2 breadcrumb segment builder.

use mu_console_layout::Breakpoint;
use mu_console_protocol::{
    Area, CdSubView, CiSubView, FleetSubView, ObserveSubView, RegistrySubView, RepoId, Route,
    SubView, SubmoduleId,
};

/// Short URL slug for [`Area`].
#[must_use]
pub const fn area_slug(area: Area) -> &'static str {
    match area {
        Area::Ci => "ci",
        Area::Cd => "cd",
        Area::Registry => "reg",
        Area::Observe => "obs",
        Area::Fleet => "flt",
    }
}

/// Uppercase display label for [`Area`].
#[must_use]
pub const fn area_label(area: Area) -> &'static str {
    match area {
        Area::Ci => "CI",
        Area::Cd => "CD",
        Area::Registry => "REG",
        Area::Observe => "OBS",
        Area::Fleet => "FLT",
    }
}

/// Parse an area slug (`ci|cd|reg|obs|flt` or legacy `ovr|overview`).
#[must_use]
pub fn parse_area(s: &str) -> Option<Area> {
    match s.to_ascii_lowercase().as_str() {
        "ovr" | "overview" | "ci" => Some(Area::Ci),
        "cd" => Some(Area::Cd),
        "reg" | "registry" => Some(Area::Registry),
        "obs" | "observe" => Some(Area::Observe),
        "flt" | "fleet" => Some(Area::Fleet),
        _ => None,
    }
}

/// Parse a sub-view slug within `area`.
#[must_use]
pub fn parse_subview(area: Area, slug: &str) -> Option<SubView> {
    let lower = slug.to_ascii_lowercase();
    for &sv in SubView::all_for(area) {
        if sv.slug() == lower {
            return Some(sv);
        }
    }
    match (area, lower.as_str()) {
        (Area::Ci, "cards" | "overview" | "ovr") => Some(SubView::Ci(CiSubView::Scorecards)),
        (Area::Ci, "railway") => Some(SubView::Ci(CiSubView::Trainway)),
        (Area::Ci, "queue" | "logs") => Some(SubView::Ci(CiSubView::QueueLogs)),
        (Area::Cd, "gate") => Some(SubView::Cd(CdSubView::IntegrationGate)),
        (Area::Cd, "deployments" | "slots") => Some(SubView::Cd(CdSubView::DeploymentsSlots)),
        (Area::Cd, "leases") => Some(SubView::Cd(CdSubView::ActiveLeases)),
        (Area::Registry, "pkgs") => Some(SubView::Registry(RegistrySubView::Packages)),
        (Area::Registry, "canary-taints" | "canaries" | "taints") => {
            Some(SubView::Registry(RegistrySubView::CanaryTaints))
        }
        (Area::Registry, "cas") => Some(SubView::Registry(RegistrySubView::CasHealth)),
        (Area::Observe, "topology") => Some(SubView::Observe(ObserveSubView::Smartscape)),
        (Area::Observe, "traces") => Some(SubView::Observe(ObserveSubView::PurePath)),
        (Area::Observe, "problems" | "slos") => {
            Some(SubView::Observe(ObserveSubView::ProblemsSlos))
        }
        (Area::Observe, "store") => Some(SubView::Observe(ObserveSubView::DataStore)),
        (Area::Fleet, "nodes") => Some(SubView::Fleet(FleetSubView::WorkerNodes)),
        (Area::Fleet, "sandboxes") => Some(SubView::Fleet(FleetSubView::SandboxProfiles)),
        _ => None,
    }
}

fn enc(s: &str) -> String {
    s.replace('/', "%2F")
}

fn dec(s: &str) -> String {
    s.replace("%2F", "/").replace("%2f", "/")
}

/// Parse a location hash (`#/...`) into a [`Route`].
///
/// - Empty `#/`, `#/ovr`, and `#/overview` redirect to `#/ci/scorecards`.
/// - Legacy `#/jobs/{id}` links redirect to `#/ci/trainway/_/_/main/{id}`.
/// - Supports `#/{area}/{subview}[/{owner}/{repo}...][?trace={id}&filter={pill}]`.
#[must_use]
pub fn parse_hash(hash: &str) -> Route {
    let raw = hash.trim().trim_start_matches('#');
    let (path_part, query_part) = raw.split_once('?').unwrap_or((raw, ""));
    let trimmed = path_part.trim_matches('/');
    let mut trace_id = None;
    let mut filter = None;
    let mut query_repo = None;
    let mut query_sub = None;

    for pair in query_part.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            let val = dec(v.trim());
            if val.is_empty() {
                continue;
            }
            match k.trim() {
                "trace" | "trace_id" => trace_id = Some(val),
                "filter" => filter = Some(val),
                "repo" => {
                    if let Some((o, r)) = val.split_once('/') {
                        query_repo = Some(RepoId::new(o, r));
                    }
                }
                "sub" | "submodule" => query_sub = Some(SubmoduleId { path: val }),
                _ => {}
            }
        }
    }

    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("ovr")
        || trimmed.eq_ignore_ascii_case("overview")
    {
        return Route {
            area: Area::Ci,
            subview: SubView::Ci(CiSubView::Scorecards),
            repo: query_repo,
            submodule: query_sub,
            branch: None,
            job: None,
            trace_id,
            filter,
        };
    }

    let parts: Vec<&str> = trimmed.split('/').collect();
    if parts.first().copied() == Some("jobs") {
        let job = parts.get(1).and_then(|s| s.parse::<u64>().ok());
        return Route {
            area: Area::Ci,
            subview: SubView::Ci(CiSubView::Trainway),
            repo: Some(RepoId::new("_", "_")),
            submodule: None,
            branch: Some("main".into()),
            job,
            trace_id,
            filter,
        };
    }

    let area = parts
        .first()
        .and_then(|s| parse_area(s))
        .unwrap_or(Area::Ci);

    // Check whether parts[1] is a subview slug or an owner in legacy #/ci/{owner}/{repo}.
    let (subview, repo_offset) = match parts.get(1).and_then(|s| parse_subview(area, s)) {
        Some(sv) => (sv, 2_usize),
        None => (SubView::default_for(area), 1_usize),
    };

    let repo = match (parts.get(repo_offset), parts.get(repo_offset + 1)) {
        (Some(&o), Some(&r)) if !o.is_empty() && !r.is_empty() => {
            Some(RepoId::new(&dec(o), &dec(r)))
        }
        _ => query_repo,
    };
    let submodule = parts
        .get(repo_offset + 2)
        .filter(|s| !s.is_empty() && **s != "-")
        .map(|s| SubmoduleId { path: dec(s) })
        .or(query_sub);
    let branch = parts
        .get(repo_offset + 3)
        .filter(|s| !s.is_empty() && **s != "-")
        .map(|s| dec(s));
    let job = parts
        .get(repo_offset + 4)
        .and_then(|s| s.parse::<u64>().ok());

    Route {
        area,
        subview,
        repo,
        submodule,
        branch,
        job,
        trace_id,
        filter,
    }
}

/// Format a [`Route`] as a canonical `#/{area}/{subview}...` location hash.
#[must_use]
pub fn format_hash(route: &Route) -> String {
    let sv = if route.subview.area() == route.area {
        route.subview
    } else {
        SubView::default_for(route.area)
    };
    let mut out = format!("#/{}/{}", area_slug(route.area), sv.slug());
    if let Some(repo) = route.repo.as_ref() {
        out.push('/');
        out.push_str(&enc(&repo.owner));
        out.push('/');
        out.push_str(&enc(&repo.repo));
        if route.submodule.is_some() || route.branch.is_some() || route.job.is_some() {
            out.push('/');
            match route.submodule.as_ref() {
                Some(sub) => out.push_str(&enc(&sub.path)),
                None => out.push('-'),
            }
        }
        if route.branch.is_some() || route.job.is_some() {
            out.push('/');
            match route.branch.as_deref() {
                Some(b) => out.push_str(&enc(b)),
                None => out.push('-'),
            }
        }
        if let Some(j) = route.job {
            out.push('/');
            out.push_str(&j.to_string());
        }
    }
    let mut query = Vec::new();
    if let Some(tr) = route.trace_id.as_deref().filter(|s| !s.is_empty()) {
        query.push(format!("trace={}", enc(tr)));
    }
    if let Some(flt) = route.filter.as_deref().filter(|s| !s.is_empty()) {
        query.push(format!("filter={}", enc(flt)));
    }
    if !query.is_empty() {
        out.push('?');
        out.push_str(&query.join("&"));
    }
    out
}

/// Step up one hierarchy level on `Esc`:
/// `trace_id -> filter -> job -> branch -> submodule -> repo -> default subview -> CI/Scorecards`.
#[must_use]
pub fn escape_up(route: &Route) -> Route {
    let mut next = route.clone();
    if next.trace_id.is_some() {
        next.trace_id = None;
    } else if next.filter.is_some() {
        next.filter = None;
    } else if next.job.is_some() {
        next.job = None;
    } else if next.branch.is_some() {
        next.branch = None;
    } else if next.submodule.is_some() {
        next.submodule = None;
    } else if next.repo.is_some() {
        next.repo = None;
    } else if next.subview != SubView::default_for(next.area) {
        next.subview = SubView::default_for(next.area);
    } else {
        next.area = Area::Ci;
        next.subview = SubView::Ci(CiSubView::Scorecards);
    }
    next
}

/// One segment of the C2 Breadcrumb bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreadcrumbCrumb {
    pub id: &'static str,
    pub label: &'static str,
    pub value: String,
}

/// Build the C2 Breadcrumb segments for `route` and `breakpoint`, collapsing
/// middle segments to `…` on compact/phone viewports when needed.
#[must_use]
pub fn breadcrumb_segments(route: &Route, bp: Breakpoint) -> Vec<BreadcrumbCrumb> {
    let sv = if route.subview.area() == route.area {
        route.subview
    } else {
        SubView::default_for(route.area)
    };
    let mut crumbs = vec![
        BreadcrumbCrumb {
            id: "c2-area",
            label: "AREA",
            value: area_label(route.area).to_owned(),
        },
        BreadcrumbCrumb {
            id: "c2-subview",
            label: "VIEW",
            value: sv.label().to_owned(),
        },
    ];
    if !matches!(route.area, Area::Observe | Area::Fleet) {
        if let Some(repo) = route.repo.as_ref() {
            crumbs.push(BreadcrumbCrumb {
                id: "c2-repo",
                label: "REPO",
                value: format!("{}/{}", repo.owner, repo.repo),
            });
        }
    }
    if let Some(sub) = route.submodule.as_ref() {
        crumbs.push(BreadcrumbCrumb {
            id: "c2-submodule",
            label: "SUB",
            value: sub.path.clone(),
        });
    }
    if let Some(branch) = route.branch.as_ref() {
        crumbs.push(BreadcrumbCrumb {
            id: "c2-branch",
            label: "REF",
            value: branch.clone(),
        });
    }
    if let Some(job) = route.job {
        crumbs.push(BreadcrumbCrumb {
            id: "c2-job",
            label: "JOB",
            value: format!("#{job}"),
        });
    }
    if let Some(tr) = route.trace_id.as_ref() {
        crumbs.push(BreadcrumbCrumb {
            id: "c2-trace",
            label: "TRACE",
            value: tr.clone(),
        });
    }
    if matches!(bp, Breakpoint::Phone | Breakpoint::CompactLandscape) && crumbs.len() > 3 {
        let last = crumbs.pop().expect("len > 3");
        crumbs.truncate(2);
        crumbs.push(BreadcrumbCrumb {
            id: "c2-ellipsis",
            label: "",
            value: "…".into(),
        });
        crumbs.push(last);
    }
    crumbs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_round_trips_all_areas_and_redirects_legacy() {
        for area in Area::ALL {
            for &sv in SubView::all_for(area) {
                let r = Route {
                    area,
                    subview: sv,
                    repo: Some(RepoId::new("LMA1980", "MuConsole")),
                    submodule: None,
                    branch: None,
                    job: None,
                    trace_id: None,
                    filter: None,
                };
                let h = format_hash(&r);
                assert_eq!(parse_hash(&h), r, "failed round-trip for {h}");
            }
        }

        // Legacy #/ovr and #/overview redirect to #/ci/scorecards.
        let ovr = parse_hash("#/ovr");
        assert_eq!(ovr.area, Area::Ci);
        assert_eq!(ovr.subview, SubView::Ci(CiSubView::Scorecards));

        // Trace deep-link round-trip.
        let trace_route = parse_hash("#/obs/purepath?trace=tr-reg-taint-01&filter=ERRORS");
        assert_eq!(trace_route.area, Area::Observe);
        assert_eq!(
            trace_route.subview,
            SubView::Observe(ObserveSubView::PurePath)
        );
        assert_eq!(trace_route.trace_id.as_deref(), Some("tr-reg-taint-01"));
        assert_eq!(trace_route.filter.as_deref(), Some("ERRORS"));
        assert_eq!(
            format_hash(&trace_route),
            "#/obs/purepath?trace=tr-reg-taint-01&filter=ERRORS"
        );

        // Legacy #/jobs/{id} redirect.
        let legacy = parse_hash("#/jobs/9876");
        assert_eq!(legacy.area, Area::Ci);
        assert_eq!(legacy.subview, SubView::Ci(CiSubView::Trainway));
        assert_eq!(legacy.branch.as_deref(), Some("main"));
        assert_eq!(legacy.job, Some(9876));
    }

    #[test]
    fn escape_steps_up_hierarchy_and_breadcrumbs_collapse_on_phone() {
        let mut r =
            parse_hash("#/ci/trainway/LMA1980/MuConsole/code%2Fengine/feat%2Frvv/42?trace=tr-1");
        let phone = breadcrumb_segments(&r, Breakpoint::Phone);
        assert!(phone.iter().any(|c| c.value == "…"));

        r = escape_up(&r);
        assert!(r.trace_id.is_none() && r.job.is_some());
        r = escape_up(&r);
        assert!(r.job.is_none() && r.branch.is_some());
        r = escape_up(&r);
        assert!(r.branch.is_none() && r.submodule.is_some());
        r = escape_up(&r);
        assert!(r.submodule.is_none() && r.repo.is_some());
        r = escape_up(&r);
        assert!(r.repo.is_none());
        r = escape_up(&r);
        assert_eq!(r.subview, SubView::Ci(CiSubView::Scorecards));
    }
}
