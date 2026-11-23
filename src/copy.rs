// SPDX-License-Identifier: AGPL-3.0-only
//! Centralized English UI copy, localized ARIA labels, and structured reason messages.
//!
//! All user-facing UI strings live in this module so future localization can swap
//! strings from a single seam without scattering copy across renderers or routers.
//! Security invariant: unauthenticated, cold-boot, and authorization-denied copy must
//! never mention invitations or disclose internal account binding rules.

use mu_console_protocol::{
    AbDeploySubView, Area, AuthReasonCode, AuthState, AuthStatus, CdEcosystem, CdSubView,
    CiSubView, DashboardSubView, ModelRunnerSubView, ObservabilitySubView, PlatformSubView,
    ProblemCategory, RepositorySubView, SkipReason, SloProblem, SubView, SubsystemStatus,
};

/// Root document language attribute (`<html lang="en">`).
pub const HTML_LANG: &str = "en";

/// Application brand title.
pub const APP_TITLE: &str = "MuConsole";

/// Skip-navigation landmark text.
pub const SKIP_TO_MAIN: &str = "Skip to main console";

/// Filter input accessible label.
pub const FILTER_ARIA_LABEL: &str = "Filter repositories and entities";

/// Neutral loading message during fail-closed cold boot before `/api/auth/session` responds.
pub const LOADING_SESSION_TITLE: &str = "Resolving session";
pub const LOADING_SESSION_BODY: &str = "Checking authentication and session capabilities.";

/// Cold-boot session verification error copy (when `/api/auth/session` fails or times out).
pub const SESSION_PROBE_FAILED_TITLE: &str = "Session verification failed";
pub const SESSION_PROBE_FAILED_BODY: &str =
    "Unable to verify session via /api/auth/session. Retry or sign out to reset.";
pub const RETRY_SESSION_LABEL: &str = "Retry";

/// Anonymous lockdown login gate copy (rendered in `#muconsole-login-gate`).
pub const LOGIN_GATE_TITLE: &str = "Sign in required";
pub const LOGIN_GATE_BODY: &str =
    "Anonymous access is disabled on this instance. Sign in with GitHub to continue.";

/// Generic lockdown copy for an authenticated user without console authorization.
pub const AUTH_DENIED_LOCKDOWN_BODY: &str =
    "This session is not authorized to access console views. Sign out to switch accounts.";

/// Neutral deep-link gate banner/card for unauthorized routes.
pub const AUTH_REQUIRED_TITLE: &str = "Authentication required";
pub const AUTH_REQUIRED_BODY: &str =
    "An active session with the required role is needed to view this area.";
pub const AUTH_REQUIRED_ANON_BODY: &str = "Sign in with an authorized account to view this area.";

/// Passkey-required access gate copy for signed-in users without an enrolled passkey.
pub const PASSKEY_ACCESS_REQUIRED_TITLE: &str = "Passkey enrollment required";
pub const PASSKEY_ACCESS_REQUIRED_BODY: &str =
    "Your account is signed in, but a passkey must be enrolled before operator roles and this area unlock.";
pub const PASSKEY_CI_STEPS_SUBTITLE: &str =
    "Public repositories only. Enroll a passkey to unlock operator roles, logs, and node topology.";

/// Restricted access copy for signed-in active sessions lacking required role policies.
pub const ACCESS_RESTRICTED_TITLE: &str = "Access restricted";
pub const ACCESS_RESTRICTED_ELEVATE_BODY: &str =
    "Your active session roles do not grant access to this area. Enable or unlock an assigned role in Session Roles.";
pub const ACCESS_RESTRICTED_NO_ROLE_BODY: &str =
    "Your signed-in account does not hold the required permissions for this area.";
pub const ACTIVE_CI_LOGS_RESTRICTED_NOTE: &str =
    "Build output requires ci:jobs.logs.read. Enable or unlock a role in Session Roles.";
pub const DASH_ROLE_HINT_NOTE: &str =
    "Operational alerts and fleet status require active role policies. Open Session Roles to enable assigned roles.";

/// Generic header notice for uninvited / unauthorized GitHub logins (zero invite disclosure).
pub const AUTH_DENIED_STATUS: &str = "Authorization denied";

/// Primary header authentication action labels.
pub const SIGN_IN_LABEL: &str = "Sign In";
pub const SIGN_OUT_LABEL: &str = "Sign Out";

/// Font & skin labels.
pub const FONT_TOGGLE_SHORT: &str = "Aa";
pub const FONT_HURMIT_LABEL: &str = "Hurmit Mono";
pub const FONT_DYSLEXIC_LABEL: &str = "OpenDyslexicM";
pub const SKIN_HTML_LABEL: &str = "HTML Core";
pub const SKIN_WIREFRAME_LABEL: &str = "Wireframe SVG";

/// Session roles & Passkey onboarding guidance copy.
pub const PASSKEY_GATE_TITLE: &str = "Register Passkey";
pub const PASSKEY_GATE_REASON: &str = "Add a passkey (1Password, browser/OS passkey manager, or FIDO2 security key) to unlock operator roles, deployments, and platform controls. Choose Not Now to browse public Dashboard and CI views first.";
pub const REGISTER_PASSKEY_BTN: &str = "Register Passkey";
pub const DISMISS_PASSKEY_GATE_BTN: &str = "Not Now";
pub const ROLE_HEADER_EMPTY: &str = "+role";
pub const USER_PROFILE_TITLE: &str = "User Profile";
pub const SESSION_ROLES_TITLE: &str = "Session Roles";
pub const MANAGE_SESSION_ROLES_BTN: &str = "Manage Session Roles";
pub const LOCK_TIMED_ROLES_BTN: &str = "Lock All Timed Roles";
pub const ENABLE_ROLE_BTN: &str = "Enable";
pub const DISABLE_ROLE_BTN: &str = "Disable";
pub const UNASSIGNED_ROLE_BTN: &str = "Unassigned";
pub const STEP_UP_PASSKEY_BTN: &str = "Unlock with Passkey";
pub const RENEW_ROLE_PASSKEY_BTN: &str = "Renew +15m (Passkey)";
pub const RENEW_BANNER_TITLE: &str = "Role renewal available";

/// Dashboard (`Overview`) section headings and operational status copy.
pub const DASH_APPLICATIONS_TITLE: &str = "Applications";
pub const DASH_APPLICATIONS_MATRIX_TITLE: &str = "Application Status";
pub const DASH_FLEET_TITLE: &str = "Fleet";
pub const DASH_FLEET_TABLE_TITLE: &str = "Cluster Host Nodes";
pub const DASH_NEEDS_ATTENTION_TITLE: &str = "Needs Attention";
pub const DASH_DEGRADED_ANON_TITLE: &str = "Degraded Application Status";
pub const DASH_NOMINAL_TITLE: &str = "All Applications Nominal";
pub const DASH_ROLLUP_MESSAGE: &str = "Operational status across platform applications.";
pub const NO_FLEET_NODES: &str = "No cluster host nodes reported.";

/// User profile modal tab labels.
pub const TAB_USER_DETAILS: &str = "User Details";
pub const TAB_PREFERENCES: &str = "Preferences";
pub const TAB_PASSKEYS: &str = "Passkeys";

/// Public/anonymous CI step checklist copy.
pub const PUBLIC_STEPS_TITLE: &str = "Public CI Queue & Job Steps";
pub const PUBLIC_STEPS_SUBTITLE: &str =
    "Public repositories only. Sign in with an operator role to inspect logs or node topology.";
pub const OPEN_GITHUB_COMMIT: &str = "Open on GitHub";

/// Empty states and ARIA helpers.
pub const NO_ATTENTION_ITEMS: &str =
    "No active application alerts, failing jobs, or offline nodes.";
pub const NO_FILTER_MATCHES: &str = "No items match the current filter.";

/// Human-facing label for a [`Area`] tab.
#[must_use]
pub const fn area_tab_label(area: Area) -> &'static str {
    area.label()
}

/// Human-facing placeholder text for the top-bar filter input per [`Area`].
#[must_use]
pub const fn filter_placeholder_for_area(area: Area) -> &'static str {
    match area {
        Area::Dashboard => "Filter alerts and subsystems...",
        Area::Ci => "Filter repos, branches, jobs, or nodes...",
        Area::Cd => "Filter release runs or ecosystems...",
        Area::AbDeploy => "Filter A|B deployments, slots, or leases...",
        Area::Repository => "Filter packages, taints, or digests...",
        Area::Observability => "Filter traces, services, problems, or logs...",
        Area::ModelRunner => "Filter models or VPN peers...",
        Area::Platform => "Filter users, invites, roles, or folios...",
    }
}

fn dashboard_filter_placeholder(auth: &AuthState) -> &'static str {
    let has_ci_or_obs = auth.can_access_area(Area::Ci) || auth.can_access_area(Area::Observability);
    if auth.status == AuthStatus::Active && has_ci_or_obs {
        if auth.has_policy("ci:nodes.read") {
            "Filter applications, nodes, or alerts..."
        } else {
            "Filter applications or alerts..."
        }
    } else {
        "Filter applications..."
    }
}

/// Area-, subview-, and access-aware placeholder text for `#muconsole-filter`.
#[must_use]
pub fn filter_placeholder_for_view(
    auth: &AuthState,
    subview: SubView,
    filter_enabled: bool,
) -> &'static str {
    if !filter_enabled {
        return match auth.status {
            AuthStatus::Unresolved => "Resolving session...",
            AuthStatus::Anonymous => "Sign in to filter...",
            AuthStatus::AuthorizationDenied => "Access denied",
            AuthStatus::PasskeyEnrollmentRequired => "Enroll passkey to unlock...",
            AuthStatus::Active => "Access restricted for active roles",
        };
    }

    match subview {
        SubView::Dashboard(DashboardSubView::Overview) => dashboard_filter_placeholder(auth),
        SubView::Ci(CiSubView::QueueLogs) => {
            if !matches!(auth.status, AuthStatus::Active) {
                "Filter public repos, branches, or jobs..."
            } else if auth.has_policy("ci:jobs.logs.read") || auth.has_policy("obs:logs.read") {
                "Filter jobs, repos, branches, or logs..."
            } else {
                "Filter repos, branches, or jobs..."
            }
        }
        SubView::Ci(CiSubView::Scorecards) => "Filter repos, branches, or status...",
        SubView::Ci(CiSubView::Trainway) => "Filter branches, nodes, or stages...",
        SubView::Ci(CiSubView::WorkerNodes) => "Filter worker nodes or architectures...",
        SubView::Ci(CiSubView::SandboxProfiles) => "Filter sandbox profiles or backends...",
        SubView::Cd(CdSubView::Runs) => "Filter release runs, repos, or ecosystems...",
        SubView::Cd(CdSubView::Pipelines) => "Filter CD runs, steps, or states...",
        SubView::AbDeploy(AbDeploySubView::IntegrationGate) => {
            "Filter gate tests, status, or logs..."
        }
        SubView::AbDeploy(AbDeploySubView::DeploymentsSlots) => {
            "Filter slots, target nodes, or digests..."
        }
        SubView::AbDeploy(AbDeploySubView::GateTestLogs) => "Filter gate test log lines...",
        SubView::AbDeploy(AbDeploySubView::ActiveLeases) => "Filter services, slots, or leases...",
        SubView::Repository(RepositorySubView::Packages) => {
            "Filter packages, ecosystems, digests, or status..."
        }
        SubView::Repository(RepositorySubView::CanaryTaints) => {
            "Filter canary bundles, services, or states..."
        }
        SubView::Repository(RepositorySubView::CasHealth) => "Filter CAS health metrics...",
        SubView::Observability(ObservabilitySubView::Smartscape) => {
            "Filter services, hosts, processes, or layers..."
        }
        SubView::Observability(ObservabilitySubView::PurePath) => {
            "Filter trace IDs or root services..."
        }
        SubView::Observability(ObservabilitySubView::ProblemsSlos) => {
            "Filter problems, entities, or severities..."
        }
        SubView::Observability(ObservabilitySubView::Logs) => {
            "Filter log messages, services, or trace IDs..."
        }
        SubView::Observability(ObservabilitySubView::Metrics) => {
            "Filter services, hardware classes, or traces..."
        }
        SubView::Observability(ObservabilitySubView::DataStore) => "Filter store health metrics...",
        SubView::Observability(ObservabilitySubView::Ingress) => {
            "Filter ingress routes, paths, or services..."
        }
        SubView::ModelRunner(ModelRunnerSubView::RuntimeQueue) => {
            "Filter models or compute clusters..."
        }
        SubView::ModelRunner(ModelRunnerSubView::ConnectionVpn) => {
            "Filter VPN peers or allowed IPs..."
        }
        SubView::Platform(PlatformSubView::Maintenance) => {
            "Filter subsystems or maintenance state..."
        }
        SubView::Platform(PlatformSubView::UsersAndInvites) => {
            "Filter user handles or bound roles..."
        }
        SubView::Platform(PlatformSubView::RolesAndFolios) => {
            "Filter roles, lease modes, or folios..."
        }
    }
}

/// Generic status word exposed on both permitted and `disabled` Dashboard cards.
#[must_use]
pub const fn subsystem_status_word(status: SubsystemStatus) -> &'static str {
    match status {
        SubsystemStatus::Live => "Live",
        SubsystemStatus::InMaintenance => "In Maintenance",
        SubsystemStatus::PartialMaintenance => "Partial Maintenance",
        SubsystemStatus::Offline => "Offline",
        SubsystemStatus::Degraded => "Degraded",
    }
}

/// Format structured [`SkipReason`] into human-readable English copy without exposing internals.
#[must_use]
pub fn skip_reason_label(reason: &SkipReason) -> String {
    match reason {
        SkipReason::NoCapableWorkerNode {
            capability: Some(cap),
        } if !cap.is_empty() => {
            format!("No capable worker node (requires {cap})")
        }
        SkipReason::NoCapableWorkerNode { .. } => "No capable worker node available".into(),
        SkipReason::UpstreamStageNotPassed { stage_id: Some(id) } if !id.is_empty() => {
            format!("Upstream stage did not pass ({id})")
        }
        SkipReason::UpstreamStageNotPassed { .. } => "Upstream stage did not pass".into(),
        SkipReason::ConditionNotMet => "Workflow condition not met".into(),
        SkipReason::Superseded => "Superseded by newer revision".into(),
        SkipReason::Other(detail) if !detail.trim().is_empty() => detail.clone(),
        SkipReason::Other(_) => "Skipped".into(),
    }
}

/// Localized label for a structured [`AuthReasonCode`] without disclosing private invite policy.
#[must_use]
pub const fn auth_reason_label(code: AuthReasonCode) -> &'static str {
    match code {
        AuthReasonCode::AuthenticationRequired => AUTH_REQUIRED_TITLE,
        AuthReasonCode::AuthorizationDenied => AUTH_DENIED_STATUS,
        AuthReasonCode::PasskeyEnrollmentRequired => PASSKEY_GATE_TITLE,
        AuthReasonCode::StepUpRequired => "Passkey step-up required",
        AuthReasonCode::LeaseExpired => "Elevated role lease expired",
        AuthReasonCode::SubsystemDisabled => "Subsystem unavailable in current state",
        AuthReasonCode::PinnedBaselineImmutable => "Baseline role cannot be disabled",
        AuthReasonCode::CeilingExceeded => "Requested ceiling exceeds policy limit",
    }
}

/// Ecosystem badge label for [`CdEcosystem`].
#[must_use]
pub fn cd_ecosystem_label(eco: &CdEcosystem) -> &str {
    eco.label()
}

/// Format polite ARIA navigation announcement.
#[must_use]
pub fn announce_route(area: Area, subview_label: &str) -> String {
    format!("Viewing {} — {}", area.label(), subview_label)
}

/// Map an active Observability [`SloProblem`] strictly to `{Degraded, Down, Missing}`
/// for the `Needs Attention` table `Status` column.
#[must_use]
pub fn attention_obs_status_label(prob: &SloProblem) -> &'static str {
    let contains_any = |needles: &[&str]| {
        [
            prob.problem_id.as_str(),
            prob.title.as_str(),
            prob.summary.as_str(),
        ]
        .into_iter()
        .any(|hay| {
            let lower = hay.to_ascii_lowercase();
            needles.iter().any(|needle| lower.contains(needle))
        })
    };

    if contains_any(&["heartbeat", "staleness", "missing"]) {
        "Missing"
    } else if matches!(prob.category, ProblemCategory::CanaryTaint)
        || contains_any(&["crash", "down", "offline"])
    {
        "Down"
    } else {
        "Degraded"
    }
}

/// Format polite announcement when a disabled nav item or card is activated.
#[must_use]
pub fn announce_disabled_area(area: Area) -> String {
    format!("{} is disabled for active roles.", area.label())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_placeholder_for_view_covers_disabled_dashboard_and_all_27_subviews() {
        let unresolved = AuthState::unresolved();
        assert_eq!(
            filter_placeholder_for_view(
                &unresolved,
                SubView::Dashboard(DashboardSubView::Overview),
                false
            ),
            "Resolving session..."
        );

        let anon = AuthState::anonymous(vec![
            "system:dashboard.read".into(),
            "ci:queue.read".into(),
            "ci:repos.public.read".into(),
        ]);
        assert_eq!(
            filter_placeholder_for_view(
                &anon,
                SubView::Platform(PlatformSubView::Maintenance),
                false
            ),
            "Sign in to filter..."
        );
        assert_eq!(
            filter_placeholder_for_view(
                &anon,
                SubView::Dashboard(DashboardSubView::Overview),
                true
            ),
            "Filter applications..."
        );
        assert_eq!(
            filter_placeholder_for_view(&anon, SubView::Ci(CiSubView::QueueLogs), true),
            "Filter public repos, branches, or jobs..."
        );

        let denied = AuthState {
            status: AuthStatus::AuthorizationDenied,
            ..AuthState::default()
        };
        assert_eq!(
            filter_placeholder_for_view(
                &denied,
                SubView::Platform(PlatformSubView::Maintenance),
                false
            ),
            "Access denied"
        );

        let passkey_req = AuthState {
            status: AuthStatus::PasskeyEnrollmentRequired,
            ..AuthState::default()
        };
        assert_eq!(
            filter_placeholder_for_view(&passkey_req, SubView::Ci(CiSubView::Scorecards), false),
            "Enroll passkey to unlock..."
        );

        let active_dash_only = AuthState {
            status: AuthStatus::Active,
            passkey_count: 1,
            active_policies: vec!["system:dashboard.read".into()],
            ..AuthState::default()
        };
        assert_eq!(
            filter_placeholder_for_view(
                &active_dash_only,
                SubView::Platform(PlatformSubView::UsersAndInvites),
                false
            ),
            "Access restricted for active roles"
        );
        assert_eq!(
            filter_placeholder_for_view(
                &active_dash_only,
                SubView::Dashboard(DashboardSubView::Overview),
                true
            ),
            "Filter applications..."
        );

        let active_obs = AuthState {
            status: AuthStatus::Active,
            passkey_count: 1,
            active_policies: vec!["system:dashboard.read".into(), "obs:problems.read".into()],
            ..AuthState::default()
        };
        assert_eq!(
            filter_placeholder_for_view(
                &active_obs,
                SubView::Dashboard(DashboardSubView::Overview),
                true
            ),
            "Filter applications or alerts..."
        );

        let active_full = AuthState {
            status: AuthStatus::Active,
            passkey_count: 1,
            active_policies: vec![
                "system:dashboard.read".into(),
                "ci:queue.read".into(),
                "ci:nodes.read".into(),
                "ci:jobs.logs.read".into(),
            ],
            ..AuthState::default()
        };
        assert_eq!(
            filter_placeholder_for_view(
                &active_full,
                SubView::Dashboard(DashboardSubView::Overview),
                true
            ),
            "Filter applications, nodes, or alerts..."
        );
        assert_eq!(
            filter_placeholder_for_view(&active_full, SubView::Ci(CiSubView::QueueLogs), true),
            "Filter jobs, repos, branches, or logs..."
        );

        for area in Area::ALL {
            for &sv in SubView::all_for(area) {
                let text = filter_placeholder_for_view(&active_full, sv, true);
                assert!(
                    text.starts_with("Filter "),
                    "unexpected text for {sv:?}: {text}"
                );
            }
        }
    }
}
