// SPDX-License-Identifier: AGPL-3.0-only
//! Centralized English UI copy, localized ARIA labels, and structured reason messages.
//!
//! All user-facing UI strings live in this module so future localization can swap
//! strings from a single seam without scattering copy across renderers or routers.
//! Security invariant: unauthenticated, cold-boot, and authorization-denied copy must
//! never mention invitations or disclose internal account binding rules.

use mu_console_protocol::{Area, AuthReasonCode, CdEcosystem, SkipReason, SubsystemStatus};

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

/// Anonymous lockdown login gate copy (rendered in `#muconsole-login-gate`).
pub const LOGIN_GATE_TITLE: &str = "Sign in required";
pub const LOGIN_GATE_BODY: &str =
    "Anonymous access is disabled on this instance. Sign in with GitHub to continue.";

/// Neutral deep-link gate banner/card for unauthorized routes.
pub const AUTH_REQUIRED_TITLE: &str = "Authentication required";
pub const AUTH_REQUIRED_BODY: &str =
    "An active session with the required role is needed to view this area.";

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

/// Session roles & mandatory Passkey gate copy.
pub const PASSKEY_GATE_TITLE: &str = "Register Passkey";
pub const PASSKEY_GATE_REASON: &str =
    "Passkey enrollment is required before baseline console access activates. Register a synced passkey (such as 1Password) or a physical FIDO2 security key.";
pub const REGISTER_PASSKEY_BTN: &str = "Register Passkey";
pub const ROLE_HEADER_EMPTY: &str = "+role";
pub const USER_PROFILE_TITLE: &str = "User Profile";
pub const SESSION_ROLES_TITLE: &str = "Session Roles";
pub const LOCK_TIMED_ROLES_BTN: &str = "Lock All Timed Roles";
pub const ENABLE_ROLE_BTN: &str = "Enable";
pub const DISABLE_ROLE_BTN: &str = "Disable";
pub const STEP_UP_PASSKEY_BTN: &str = "Unlock with Passkey";

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
pub const NO_ATTENTION_ITEMS: &str = "No active subsystem alerts or failing jobs.";
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

/// Format polite announcement when a disabled nav item or card is activated.
#[must_use]
pub fn announce_disabled_area(area: Area) -> String {
    format!("{} is disabled for active roles.", area.label())
}
