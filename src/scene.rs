// SPDX-License-Identifier: AGPL-3.0-only
//! Skin-agnostic scene graph and modal projections for [`ConsoleState`].
//!
//! Both [`mu_console_theme::html::HtmlCoreSkin`] and
//! [`mu_console_theme::wireframe::WireframeSkin`] consume the exact same
//! [`ConsoleScene`] and [`theme_scene::DialogModel`] built here, preserving
//! identical fail-closed gating, capability checks, and domain boundaries
//! across both skins.

use crate::copy;
pub use crate::copy::attention_obs_status_label;
use crate::router::{self, can_access_subview};
use crate::state::{ConsoleState, AREAS};
use mu_console_protocol::{
    AbDeploySubView, Area, AuthState, AuthStatus, CanaryBundleState, CdRunState, CdSubView,
    CiJobSummary, CiSubView, CrtFont, DashboardSubView, DialogModel as ProtoDialogModel,
    GateOutcome, ModelRunnerSubView, ObservabilitySubView, PackageEcosystem, PlatformSubView,
    RepoStatus, RepositorySubView, RiskConfirmAction, RoleActivationKind, RoleLeaseView, Slot,
    StageState, StepUpRequirement, SubView, SubsystemHealthEntry, SubsystemId, SubsystemStatus,
    TestState, UiError, Visibility,
};
use mu_console_theme::scene::{
    self as theme_scene, ActionControl, ConsoleScene, GateChecklist, GateItem,
    InviteCreatedOnceDialog, KeyValueEntry, KeyValueList, LogEntry, LogViewer, MatrixCell, NavTab,
    PasskeyEnrollmentDialog, PlatformPanel, RepoSelector, RiskConfirmDialog, RoleBadge,
    SceneBanner, SceneComponent, SceneSection, SceneViewport, SessionRoleOption,
    SessionRolesDialog, SparklineCard, StatusLamp, StatusMatrix, SubViewTab, TableCell,
    TableColumn, TableGrid, TableRow, TrainwayCheckpoint, TrainwayGraph, TrainwayTrack,
    UserProfileDialog,
};
use mu_console_theme::skin::{SemanticTone, SignalGlyph};
use std::collections::HashSet;

/// Build the complete [`ConsoleScene`] projection for `state` at `_viewport`.
#[must_use]
pub fn build_console_scene(state: &ConsoleState, _viewport: SceneViewport) -> ConsoleScene {
    let auth = state.auth();
    let mut scene = ConsoleScene::new(state.route.area.slug(), state.route.area.label());
    scene.active_subview = Some(state.route.subview.slug().to_owned());
    scene.area_tabs = build_area_tabs(state, &auth);

    // 1. Fail-closed cold boot: all areas disabled, zero cached data rendered, neutral loading card
    // or explicit session-probe diagnostic with Retry / Sign Out recovery controls.
    if !auth.is_resolved() {
        if let Some(err) = state.session_probe_error() {
            let diag = match err {
                UiError::Http { status: 0, .. } => "Network error".to_owned(),
                UiError::Http { status, .. } => format!("HTTP {status}"),
                UiError::Decode(_) => "Invalid payload".to_owned(),
                UiError::AuthDenied(_) => "Access denied".to_owned(),
                _ => "Probe error".to_owned(),
            };
            scene.status_lamps.push(StatusLamp {
                control_id: "lamp-session".into(),
                label: "Session".into(),
                value: diag.clone(),
                lit: true,
                tone: SemanticTone::Fail,
                action: None,
            });
            scene.sections.push(
                SceneSection::new("sec-cold-boot-error", "Session Status").with_component(
                    SceneComponent::PlatformPanel(PlatformPanel {
                        id: "panel-session-probe-error".into(),
                        title: copy::SESSION_PROBE_FAILED_TITLE.into(),
                        summary: Some(copy::SESSION_PROBE_FAILED_BODY.into()),
                        tone: SemanticTone::Fail,
                        attributes: vec![
                            KeyValueEntry {
                                key: "Endpoint".into(),
                                value: "/api/auth/session".into(),
                                detail: None,
                                tone: SemanticTone::Neutral,
                                mono: true,
                                action: None,
                            },
                            KeyValueEntry {
                                key: "Diagnostic".into(),
                                value: diag,
                                detail: None,
                                tone: SemanticTone::Fail,
                                mono: true,
                                action: None,
                            },
                        ],
                        controls: vec![
                            ActionControl::new(
                                "session-probe-retry",
                                copy::RETRY_SESSION_LABEL,
                                "session-probe-retry",
                            )
                            .with_tone(SemanticTone::Info),
                            ActionControl::new(
                                "session-error-signout",
                                copy::SIGN_OUT_LABEL,
                                "session-error-signout",
                            )
                            .with_attr("data-href", state.sign_out_href())
                            .with_tone(SemanticTone::Warn),
                        ],
                    }),
                ),
            );
            scene.footer_status = Some(copy::SESSION_PROBE_FAILED_TITLE.into());
            return scene;
        }
        scene.status_lamps.push(StatusLamp {
            control_id: "lamp-session".into(),
            label: copy::LOADING_SESSION_TITLE.into(),
            value: "Pending".into(),
            lit: false,
            tone: SemanticTone::Neutral,
            action: None,
        });
        scene.sections.push(
            SceneSection::new("sec-cold-boot", copy::LOADING_SESSION_TITLE).with_component(
                SceneComponent::KeyValueList(KeyValueList {
                    id: "kv-cold-boot".into(),
                    title: copy::LOADING_SESSION_TITLE.into(),
                    entries: vec![KeyValueEntry {
                        key: "Session".into(),
                        value: "Verifying".into(),
                        detail: Some(copy::LOADING_SESSION_BODY.into()),
                        tone: SemanticTone::Neutral,
                        mono: false,
                        action: None,
                    }],
                }),
            ),
        );
        scene.footer_status = Some("Fail-closed cold boot".into());
        return scene;
    }

    // Header chrome: font toggle ONLY when anonymous/denied; @user + +role + Sign Out when active.
    populate_header_chrome(state, &auth, &mut scene);

    // 2. 0-Passkey onboarding guidance: non-blocking top banner + dismissible modal, locking out
    // all views ONLY when `anonymous_lockdown` is enabled.
    if auth.passkey_enrollment_required || auth.status == AuthStatus::PasskeyEnrollmentRequired {
        if let Some(ref err) = state.webauthn_error {
            scene.banners.push(SceneBanner {
                id: "banner-webauthn-error".into(),
                tone: SemanticTone::Fail,
                title: "Passkey enrollment failed".into(),
                message: err.clone(),
                action: None,
            });
        }
        scene.banners.push(SceneBanner {
            id: "banner-passkey-gate".into(),
            tone: SemanticTone::Warn,
            title: copy::PASSKEY_GATE_TITLE.into(),
            message: copy::PASSKEY_GATE_REASON.into(),
            action: Some(ActionControl::new(
                "register-passkey",
                copy::REGISTER_PASSKEY_BTN,
                "register-passkey",
            )),
        });
        if auth.anonymous_lockdown {
            scene.sections.push(
                SceneSection::new("sec-passkey-hold", copy::PASSKEY_GATE_TITLE).with_component(
                    SceneComponent::KeyValueList(KeyValueList {
                        id: "kv-passkey-hold".into(),
                        title: "Passkey Onboarding".into(),
                        entries: vec![KeyValueEntry {
                            key: "Status".into(),
                            value: "Enrollment required".into(),
                            detail: Some(copy::PASSKEY_GATE_REASON.into()),
                            tone: SemanticTone::Warn,
                            mono: false,
                            action: Some(ActionControl::new(
                                "hold-passkey-register",
                                copy::REGISTER_PASSKEY_BTN,
                                "hold-passkey-register",
                            )),
                        }],
                    }),
                ),
            );
            scene.footer_status = Some("Passkey gate active".into());
            return scene;
        }
    }

    // 3. AuthorizationDenied notice: zero invite token disclosure.
    if auth.status == AuthStatus::AuthorizationDenied {
        let msg = auth
            .reason_code
            .map_or(copy::AUTH_DENIED_STATUS, copy::auth_reason_label);
        scene.banners.push(SceneBanner {
            id: "banner-auth-denied".into(),
            tone: SemanticTone::Fail,
            title: copy::AUTH_DENIED_STATUS.into(),
            message: msg.into(),
            action: Some(
                ActionControl::new("c2-auth-status", copy::SIGN_OUT_LABEL, "c2-auth-status")
                    .with_attr("data-href", state.sign_out_href()),
            ),
        });
    }

    // 3b. Active-session WebAuthn error feedback and proactive final-2m lease renewal banners.
    if auth.status == AuthStatus::Active {
        if let Some(ref err) = state.webauthn_error {
            if !matches!(
                state.active_dialog(),
                Some(ProtoDialogModel::SessionRoles(_))
            ) {
                scene.banners.push(SceneBanner {
                    id: "banner-webauthn-error".into(),
                    tone: SemanticTone::Fail,
                    title: "Passkey verification failed".into(),
                    message: err.clone(),
                    action: None,
                });
            }
        }
        for lease in &auth.leases {
            if lease.can_renew && !lease.is_expired() {
                let extend_mins = (lease.renew_extend_secs / 60).max(1);
                let label = lease.display_name.as_deref().unwrap_or(&lease.role_id);
                scene.banners.push(SceneBanner {
                    id: format!("banner-role-renew-{}", lease.role_id),
                    tone: SemanticTone::Warn,
                    title: copy::RENEW_BANNER_TITLE.into(),
                    message: format!(
                        "{label} expires in {} (idle {}). Confirm Passkey to extend +{extend_mins}m.",
                        RoleLeaseView::format_mm_ss(lease.seconds_remaining),
                        RoleLeaseView::format_mm_ss(lease.idle_timer),
                    ),
                    action: Some(ActionControl::new(
                        format!("role-renew:{}", lease.role_id),
                        copy::RENEW_ROLE_PASSKEY_BTN,
                        format!("role-renew:{}", lease.role_id),
                    )),
                });
            }
        }
    }

    // 4. Anonymous lockdown (`RoleFolioIndex["anonymous"] == []`) vs AuthorizationDenied lockdown.
    if auth.anonymous_lockdown {
        if auth.status == AuthStatus::AuthorizationDenied {
            scene
                .sections
                .push(build_auth_denied_lockdown_section(state, &auth));
            scene.footer_status = Some(copy::AUTH_DENIED_STATUS.into());
            return scene;
        }
        if auth.status == AuthStatus::Anonymous {
            scene.sections.push(build_login_gate_section(state));
            scene.footer_status = Some("Anonymous access disabled".into());
            return scene;
        }
    }

    // 5. Unauthorized deep-link landing card (preserves hash for post-OAuth return).
    if !can_access_subview(&auth, state.route.subview) {
        scene
            .sections
            .push(build_auth_required_section(state, &auth));
        scene.footer_status = Some(copy::AUTH_REQUIRED_TITLE.into());
        return scene;
    }

    // 6. Sub-view strip & repo selector (hidden on anonymous public CI per #50).
    let is_public_anon_ci = matches!(
        auth.status,
        AuthStatus::Anonymous | AuthStatus::AuthorizationDenied
    ) && state.route.area == Area::Ci;

    if !is_public_anon_ci {
        scene.subviews = build_subview_tabs(state, &auth);
        scene.repo_selector = build_repo_selector(state);
    }

    // 7. Area & sub-view projection across all 8 canonical areas & 27 sub-views.
    match state.route.subview {
        SubView::Dashboard(DashboardSubView::Overview) => {
            build_dashboard_sections(state, &auth, &mut scene);
        }
        SubView::Ci(ci) => build_ci_sections(state, &auth, ci, is_public_anon_ci, &mut scene),
        SubView::Cd(cd) => build_cd_sections(state, &auth, cd, &mut scene),
        SubView::AbDeploy(ab) => build_ab_sections(state, &auth, ab, &mut scene),
        SubView::Repository(repo) => build_repo_sections(state, repo, &mut scene),
        SubView::Observability(obs) => build_observe_sections(state, obs, &mut scene),
        SubView::ModelRunner(mr) => build_model_runner_sections(state, mr, &mut scene),
        SubView::Platform(plat) => build_platform_sections(state, &auth, plat, &mut scene),
    }

    scene.footer_status = Some(format!(
        "{} · {}",
        state.route.area.label(),
        state.route.subview.label()
    ));
    scene
}

/// Build skin-agnostic modal projection matching [`ConsoleState::active_dialog`].
#[must_use]
pub fn build_dialog_model(state: &ConsoleState) -> Option<theme_scene::DialogModel> {
    let dlg = state.active_dialog()?;
    let auth = state.auth();
    let view = state.client.view();

    match dlg {
        ProtoDialogModel::PasskeyEnrollmentGate(gate) => Some(
            theme_scene::DialogModel::PasskeyEnrollmentGate(PasskeyEnrollmentDialog {
                dialog_id: "dlg-passkey-gate".into(),
                title: copy::PASSKEY_GATE_TITLE.into(),
                reason: match state.webauthn_error.as_deref() {
                    Some(err) => {
                        format!("{err} — {}", copy::PASSKEY_GATE_REASON)
                    }
                    None => copy::PASSKEY_GATE_REASON.into(),
                },
                account_subject: gate.github_login,
                passkey_count: gate.passkey_count,
                enroll_action: ActionControl::new(
                    "dlg-passkey-register",
                    copy::REGISTER_PASSKEY_BTN,
                    "dlg-passkey-register",
                )
                .with_tone(SemanticTone::Info),
                sign_out_action: Some(
                    ActionControl::new(
                        "dlg-passkey-signout",
                        copy::SIGN_OUT_LABEL,
                        "dlg-passkey-signout",
                    )
                    .with_attr("data-href", state.sign_out_href()),
                ),
                tone: SemanticTone::Warn,
            }),
        ),
        ProtoDialogModel::UserProfile(up) => {
            let role_badges: Vec<RoleBadge> = auth
                .active_roles
                .iter()
                .map(|r| {
                    let is_timed =
                        matches!(r.activation.kind, RoleActivationKind::Timed) || r.lease.is_some();
                    RoleBadge {
                        role: r.effective_label().to_owned(),
                        tone: if is_timed {
                            SemanticTone::Warn
                        } else {
                            SemanticTone::Info
                        },
                        detail: Some(match r.lease.as_ref() {
                            Some(lease) => format!("{}s remaining", lease.seconds_remaining),
                            None => "Baseline / Elective".into(),
                        }),
                    }
                })
                .collect();
            let dormant_badges: Vec<RoleBadge> = auth
                .dormant_role_badges()
                .into_iter()
                .map(|r| {
                    let is_timed = matches!(r.activation.kind, RoleActivationKind::Timed);
                    RoleBadge {
                        role: r.effective_label().to_owned(),
                        tone: if is_timed {
                            SemanticTone::Info
                        } else {
                            SemanticTone::Neutral
                        },
                        detail: Some(match r.activation.kind {
                            RoleActivationKind::Timed => "Timed (Passkey)".into(),
                            RoleActivationKind::Elective => "Elective".into(),
                            RoleActivationKind::PinnedBaseline => "Baseline".into(),
                        }),
                    }
                })
                .collect();
            let display_name = if !state.profile_display_name_draft.is_empty() {
                state.profile_display_name_draft.clone()
            } else {
                up.profile
                    .display_name
                    .clone()
                    .unwrap_or_else(|| up.profile.github_login.clone())
            };
            let handle = if up.profile.github_login.is_empty() {
                auth.github_login
                    .clone()
                    .unwrap_or_else(|| "operator".into())
            } else {
                up.profile.github_login
            };
            let has_timed = auth.active_roles.iter().any(|r| {
                matches!(r.activation.kind, RoleActivationKind::Timed) || r.lease.is_some()
            });
            Some(theme_scene::DialogModel::UserProfile(UserProfileDialog {
                dialog_id: "dlg-user-profile".into(),
                handle: handle.clone(),
                display_name,
                subject_id: handle,
                email: None,
                active_roles: role_badges,
                dormant_roles: dormant_badges,
                session_expires_at: up
                    .max_session_expires_epoch_ms
                    .map(|ms| format!("{}s", ms / 1000)),
                passkey_count: up.profile.passkey_count.max(auth.passkey_count),
                manage_roles_action: Some(
                    ActionControl::new(
                        "profile-manage-roles",
                        copy::MANAGE_SESSION_ROLES_BTN,
                        "profile-manage-roles",
                    )
                    .disabled(auth.status != AuthStatus::Active)
                    .with_tone(SemanticTone::Info),
                ),
                lock_action: Some(
                    ActionControl::new(
                        "profile-lock-timed",
                        copy::LOCK_TIMED_ROLES_BTN,
                        "profile-lock-timed",
                    )
                    .disabled(!has_timed),
                ),
                manage_passkeys_action: None,
                sign_out_action: Some(
                    ActionControl::new("profile-signout", copy::SIGN_OUT_LABEL, "profile-signout")
                        .with_attr("data-href", state.sign_out_href()),
                ),
                close_action: ActionControl::new("btn-close", "Close", "btn-close"),
            }))
        }
        ProtoDialogModel::SessionRoles(sr) => {
            let mut options: Vec<SessionRoleOption> = Vec::with_capacity(sr.roles.len() + 2);
            for r in &sr.roles {
                let is_unassigned = r.unassigned;
                let active_badge = if is_unassigned {
                    None
                } else {
                    auth.active_roles.iter().find(|b| b.role_id == r.role_id)
                };
                let active = !is_unassigned && (r.active || active_badge.is_some());
                let active_lease = if is_unassigned {
                    None
                } else {
                    auth.leases
                        .iter()
                        .find(|l| l.role_id == r.role_id && !l.is_expired())
                        .or_else(|| {
                            active_badge
                                .and_then(|b| b.lease.as_ref())
                                .filter(|l| !l.is_expired())
                        })
                        .or_else(|| r.lease.as_ref().filter(|l| !l.is_expired()))
                };
                let rem = active_lease.map(|l| l.seconds_remaining);
                let is_baseline = !is_unassigned
                    && matches!(r.activation.kind, RoleActivationKind::PinnedBaseline);
                let requires_step_up = !is_unassigned
                    && !active
                    && !is_baseline
                    && (matches!(r.step_up, StepUpRequirement::Passkey)
                        || matches!(r.activation.kind, RoleActivationKind::Timed));
                let action = if is_unassigned {
                    ActionControl::new(
                        format!("role-unassigned:{}", r.role_id),
                        copy::UNASSIGNED_ROLE_BTN,
                        format!("role-unassigned:{}", r.role_id),
                    )
                    .disabled(true)
                } else if is_baseline {
                    ActionControl::new(
                        format!("role-baseline:{}", r.role_id),
                        "Locked Baseline",
                        "role-baseline",
                    )
                    .disabled(true)
                } else if active {
                    ActionControl::new(
                        format!("role-disable:{}", r.role_id),
                        copy::DISABLE_ROLE_BTN,
                        format!("role-disable:{}", r.role_id),
                    )
                    .with_tone(SemanticTone::Warn)
                } else if requires_step_up {
                    ActionControl::new(
                        format!("role-elevate:{}", r.role_id),
                        copy::STEP_UP_PASSKEY_BTN,
                        format!("role-elevate:{}", r.role_id),
                    )
                    .with_tone(SemanticTone::Info)
                } else {
                    ActionControl::new(
                        format!("role-enable:{}", r.role_id),
                        copy::ENABLE_ROLE_BTN,
                        format!("role-enable:{}", r.role_id),
                    )
                    .with_tone(SemanticTone::Pass)
                };
                options.push(SessionRoleOption {
                    role: r.display_name.clone().unwrap_or_else(|| r.role_id.clone()),
                    description: r.description.clone(),
                    active,
                    expires_at: rem.map(|s| format!("{:02}:{:02}", s / 60, s % 60)),
                    requires_step_up,
                    tone: if active {
                        SemanticTone::Pass
                    } else {
                        SemanticTone::Neutral
                    },
                    action,
                });
                if active {
                    if let Some(lease) = active_lease.filter(|l| l.can_renew && !l.is_expired()) {
                        let extend_mins = (lease.renew_extend_secs / 60).max(1);
                        let role_label = r.display_name.as_deref().unwrap_or(&r.role_id);
                        options.push(SessionRoleOption {
                            role: format!("{role_label} — Renew +{extend_mins}m"),
                            description: format!(
                                "Final {}m renewal window ({} remaining) — confirm Passkey to extend +{extend_mins}m.",
                                (lease.renew_window_secs / 60).max(1),
                                RoleLeaseView::format_mm_ss(lease.seconds_remaining),
                            ),
                            active: true,
                            expires_at: Some(RoleLeaseView::format_mm_ss(lease.seconds_remaining)),
                            requires_step_up: true,
                            tone: SemanticTone::Warn,
                            action: ActionControl::new(
                                format!("role-renew:{}", r.role_id),
                                copy::RENEW_ROLE_PASSKEY_BTN,
                                format!("role-renew:{}", r.role_id),
                            )
                            .with_tone(SemanticTone::Info),
                        });
                    }
                }
            }

            let has_timed_active = sr.lock_all_enabled
                || auth.active_roles.iter().any(|r| {
                    matches!(r.activation.kind, RoleActivationKind::Timed) || r.lease.is_some()
                })
                || !auth.leases.is_empty();
            options.push(SessionRoleOption {
                role: copy::LOCK_TIMED_ROLES_BTN.into(),
                description: "Deactivate every active Timed step-up role immediately.".into(),
                active: has_timed_active,
                expires_at: None,
                requires_step_up: false,
                tone: SemanticTone::Warn,
                action: ActionControl::new(
                    "roles-lock-all",
                    copy::LOCK_TIMED_ROLES_BTN,
                    "roles-lock-all",
                )
                .disabled(!has_timed_active)
                .with_tone(SemanticTone::Warn),
            });

            let inline_error = sr
                .error_message
                .filter(|s| !s.trim().is_empty())
                .or_else(|| {
                    state
                        .webauthn_error
                        .clone()
                        .filter(|s| !s.trim().is_empty())
                });

            Some(theme_scene::DialogModel::SessionRoles(SessionRolesDialog {
                dialog_id: "dlg-session-roles".into(),
                title: copy::SESSION_ROLES_TITLE.into(),
                subject_handle: sr.github_login,
                step_up_verified: !auth.leases.is_empty(),
                roles: options,
                error_message: inline_error,
                close_action: ActionControl::new("roles-close", "Close", "roles-close"),
            }))
        }
        ProtoDialogModel::RiskConfirm(rc) => {
            let (title, summary, target, consequence, tone, confirm_label) = match &rc.action {
                RiskConfirmAction::RestartCiJob {
                    job_id_str, repo, ..
                } => (
                    "Confirm CI Job Restart".to_owned(),
                    format!("Restart CI job #{job_id_str} for {}/{}?", repo.owner, repo.repo),
                    format!("{}/{} #{job_id_str}", repo.owner, repo.repo),
                    "Dispatches a clean job execution against worker capacity.",
                    SemanticTone::Warn,
                    "Confirm Restart",
                ),
                RiskConfirmAction::TriggerAbRollback {
                    deployment_id,
                    repo,
                    target_node,
                    current_slot,
                    restore_slot,
                    note,
                } => (
                    "Confirm A|B Rollback".to_owned(),
                    format!(
                        "Restore slot {restore_slot:?} (replacing {current_slot:?}) on {target_node}{}",
                        note.as_deref().map_or_else(String::new, |n| format!(" · Note: {n}"))
                    ),
                    format!("{}/{} ({deployment_id})", repo.owner, repo.repo),
                    "Immediately shifts active A|B traffic back to the prior verified slot.",
                    SemanticTone::Fail,
                    "Confirm Rollback",
                ),
                RiskConfirmAction::SetSubsystemMaintenance {
                    subsystem, mode, ..
                } => (
                    "Confirm Subsystem Maintenance Change".into(),
                    format!("Apply {mode:?} maintenance policy to {subsystem:?}"),
                    format!("{subsystem:?}"),
                    "Alters active admission and health alert suppression for this subsystem.",
                    SemanticTone::Warn,
                    "Confirm Maintenance",
                ),
                RiskConfirmAction::RevokeUser { github_login } => (
                    "Confirm User Revocation".into(),
                    format!("Revoke console access for @{github_login}"),
                    github_login.clone(),
                    "Terminates active sessions and removes role bindings (last admin protected).",
                    SemanticTone::Fail,
                    "Confirm Revoke",
                ),
                RiskConfirmAction::ResetUserPasskeys { github_login } => (
                    "Confirm Passkey Reset".into(),
                    format!("Reset enrolled WebAuthn Passkeys for @{github_login}"),
                    github_login.clone(),
                    "User must re-enroll a Passkey at the mandatory onboarding gate.",
                    SemanticTone::Warn,
                    "Confirm Reset",
                ),
                RiskConfirmAction::DeleteRoleSpec { role_id } => (
                    "Confirm Role Deletion".into(),
                    format!("Delete role specification '{role_id}'"),
                    role_id.clone(),
                    "Removes role bindings and revokes active leases referencing this role.",
                    SemanticTone::Fail,
                    "Confirm Delete",
                ),
                RiskConfirmAction::DeletePolicyFolio { folio_id } => (
                    "Confirm Policy Folio Deletion".into(),
                    format!("Delete policy folio '{folio_id}'"),
                    folio_id.clone(),
                    "Active sessions lose all capabilities granted solely by this folio.",
                    SemanticTone::Fail,
                    "Confirm Delete",
                ),
                _ => (
                    "Confirm Platform Configuration Update".into(),
                    "Apply stateful platform security policy modification.".into(),
                    "Platform IAM Policy".into(),
                    "Takes effect immediately across console policy enforcement.",
                    SemanticTone::Warn,
                    "Confirm Apply",
                ),
            };
            let phrase_matched = rc
                .confirmation_phrase
                .as_deref()
                .is_none_or(|p| rc.typed_confirmation.trim() == p.trim());
            Some(theme_scene::DialogModel::RiskConfirm(RiskConfirmDialog {
                dialog_id: "dlg-risk-confirm".into(),
                title,
                summary,
                target_resource: target,
                consequence: consequence.into(),
                confirmation_phrase: rc.confirmation_phrase,
                typed_confirmation: rc.typed_confirmation,
                phrase_matched,
                phrase_input_id: "muconsole-risk-phrase".into(),
                phrase_input_action: "risk-phrase-input".into(),
                tone,
                confirm_action: ActionControl::new("btn-confirm", confirm_label, "btn-confirm")
                    .disabled(!phrase_matched)
                    .with_tone(SemanticTone::Fail),
                cancel_action: ActionControl::new("btn-cancel", "Cancel", "btn-cancel"),
            }))
        }
        ProtoDialogModel::InviteCreatedOnce(inv) => {
            let invite_hint = inv
                .note
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| inv.invite_id.clone());
            let _ = view;
            Some(theme_scene::DialogModel::InviteCreatedOnce(
                InviteCreatedOnceDialog {
                    dialog_id: "dlg-invite-created".into(),
                    title: "One-Time Invite Secret Created".into(),
                    invitee_hint: invite_hint,
                    assigned_roles: vec![inv.granted_role],
                    invite_url: inv.invite_url.clone(),
                    expires_at: format!("{}s", inv.expires_at_ms / 1000),
                    warning: "This invite URL is shown once and cannot be retrieved after closing."
                        .into(),
                    copy_action: ActionControl::new(
                        "btn-copy-inv",
                        "Copy Invite URL",
                        "btn-copy-inv",
                    )
                    .with_attr("data-invite-url", &inv.invite_url),
                    close_action: ActionControl::new("btn-close-inv", "Close", "btn-close-inv"),
                },
            ))
        }
    }
}

fn populate_header_chrome(state: &ConsoleState, auth: &AuthState, scene: &mut ConsoleScene) {
    match auth.status {
        AuthStatus::Unresolved => {}
        AuthStatus::Anonymous => {
            scene.font_toggle = Some(font_toggle_control(state.font));
            scene.header_actions.push(
                ActionControl::new("c2-auth-status", copy::SIGN_IN_LABEL, "c2-auth-status")
                    .with_attr("data-href", state.sign_in_href()),
            );
        }
        AuthStatus::AuthorizationDenied => {
            scene.font_toggle = Some(font_toggle_control(state.font));
            scene.status_lamps.push(StatusLamp {
                control_id: "lamp-auth-denied".into(),
                label: copy::AUTH_DENIED_STATUS.into(),
                value: "Denied".into(),
                lit: true,
                tone: SemanticTone::Fail,
                action: None,
            });
            scene.header_actions.push(
                ActionControl::new("c2-auth-status", copy::SIGN_OUT_LABEL, "c2-auth-status")
                    .with_attr("data-href", state.sign_out_href()),
            );
        }
        AuthStatus::PasskeyEnrollmentRequired | AuthStatus::Active => {
            // Active sessions remove `[Aa]` from top bar (settings live under `@user -> Preferences`).
            let role_gated = auth.passkey_enrollment_required
                || auth.status == AuthStatus::PasskeyEnrollmentRequired;
            scene.font_toggle = None;
            let handle = auth.github_login.as_deref().unwrap_or("operator");
            scene.header_actions.push(
                ActionControl::new("hdr-user", format!("@{handle}"), "hdr-user").disabled(false),
            );
            scene.header_actions.push(
                ActionControl::new("hdr-role", state.format_header_roles(), "hdr-role")
                    .disabled(role_gated)
                    .with_tone(SemanticTone::Info),
            );
            scene.header_actions.push(
                ActionControl::new("c2-auth-status", copy::SIGN_OUT_LABEL, "c2-auth-status")
                    .with_attr("data-href", state.sign_out_href()),
            );
        }
    }
}

fn font_toggle_control(font: CrtFont) -> ActionControl {
    let label = match font {
        CrtFont::Hurmit => format!("{} {}", copy::FONT_TOGGLE_SHORT, copy::FONT_HURMIT_LABEL),
        CrtFont::OpenDyslexic => {
            format!("{} {}", copy::FONT_TOGGLE_SHORT, copy::FONT_DYSLEXIC_LABEL)
        }
    };
    ActionControl::new("c2-font-switch", label, "c2-font-switch")
        .with_aria_label("Toggle console monospace font")
}

fn build_area_tabs(state: &ConsoleState, auth: &AuthState) -> Vec<NavTab> {
    let gated = !auth.is_resolved() || auth.anonymous_lockdown;

    AREAS
        .iter()
        .map(|&area| {
            let permitted = !gated && state.can_access_area(area);
            let slug = area.slug();
            NavTab {
                id: slug.into(),
                control_id: format!("nav-area:{slug}"),
                label: copy::area_tab_label(area).into(),
                badge: area_badge(state, area, permitted),
                tone: if !permitted {
                    SemanticTone::Neutral
                } else if area == Area::Dashboard && failing_attention_count(state) > 0 {
                    SemanticTone::Fail
                } else {
                    SemanticTone::Info
                },
                active: state.route.area == area,
                disabled: !permitted,
            }
        })
        .collect()
}

fn area_badge(state: &ConsoleState, area: Area, permitted: bool) -> Option<u32> {
    if !permitted {
        return None;
    }
    match area {
        Area::Ci => u32::try_from(
            state
                .visible_ci_jobs()
                .iter()
                .filter(|j| j.queue_state.eq_ignore_ascii_case("failed"))
                .count(),
        )
        .ok()
        .filter(|&n| n > 0),
        Area::Observability => state
            .client
            .view()
            .and_then(|v| {
                u32::try_from(v.observe.problems.iter().filter(|p| p.active).count()).ok()
            })
            .filter(|&n| n > 0),
        _ => None,
    }
}

fn build_subview_tabs(state: &ConsoleState, auth: &AuthState) -> Vec<SubViewTab> {
    let area = state.route.area;
    let area_id = area.slug();
    let subviews: Vec<SubView> = match area {
        Area::Dashboard => vec![],
        Area::Ci => CiSubView::ALL.into_iter().map(SubView::Ci).collect(),
        Area::Cd => CdSubView::ALL.into_iter().map(SubView::Cd).collect(),
        Area::AbDeploy => AbDeploySubView::ALL
            .into_iter()
            .map(SubView::AbDeploy)
            .collect(),
        Area::Repository => RepositorySubView::ALL
            .into_iter()
            .map(SubView::Repository)
            .collect(),
        Area::Observability => ObservabilitySubView::ALL
            .into_iter()
            .map(SubView::Observability)
            .collect(),
        Area::ModelRunner => ModelRunnerSubView::ALL
            .into_iter()
            .map(SubView::ModelRunner)
            .collect(),
        Area::Platform => PlatformSubView::ALL
            .into_iter()
            .map(SubView::Platform)
            .collect(),
    };

    subviews
        .into_iter()
        .map(|sv| {
            let permitted = can_access_subview(auth, sv);
            let sv_slug = sv.slug();
            SubViewTab {
                id: format!("{area_id}/{sv_slug}"),
                control_id: format!("nav-subview:{area_id}:{sv_slug}"),
                area: area_id.into(),
                label: sv.label().into(),
                badge: None,
                tone: if permitted {
                    SemanticTone::Info
                } else {
                    SemanticTone::Neutral
                },
                active: state.route.subview == sv,
                disabled: !permitted,
            }
        })
        .collect()
}

fn build_repo_selector(state: &ConsoleState) -> Option<RepoSelector> {
    if matches!(
        state.route.area,
        Area::Dashboard | Area::ModelRunner | Area::Platform
    ) {
        return None;
    }
    let repos = state.filtered_repos();
    let total = repos.len();
    let outer_idx = state.outer_index.min(total.saturating_sub(1));
    let outer_label = state
        .route
        .repo
        .as_ref()
        .map(|r| format!("{}/{}", r.owner, r.repo))
        .or_else(|| {
            repos
                .get(outer_idx)
                .map(|r| format!("{}/{}", r.id.owner, r.id.repo))
        })
        .unwrap_or_else(|| "All Repositories".into());
    let submodules = repos
        .get(outer_idx)
        .map_or(&[][..], |r| r.submodules.as_slice());
    let inner_total = submodules.len();
    let inner_idx = state.inner_index.min(inner_total.saturating_sub(1));
    let inner_label = state
        .route
        .submodule
        .as_ref()
        .map(|s| s.path.clone())
        .unwrap_or_else(|| "root".into());

    Some(RepoSelector {
        control_id: "c2-repo-selector".into(),
        outer_label,
        outer_index: outer_idx,
        outer_total: total,
        inner_label,
        inner_index: inner_idx,
        inner_total,
        filter_query: state.filter_query.clone(),
        prev_outer: ActionControl::new("repo-prev", "Prev Repo", "repo-prev").disabled(total <= 1),
        next_outer: ActionControl::new("repo-next", "Next Repo", "repo-next").disabled(total <= 1),
        prev_inner: ActionControl::new("sub-prev", "Prev Submodule", "sub-prev")
            .disabled(inner_total == 0),
        next_inner: ActionControl::new("sub-next", "Next Submodule", "sub-next")
            .disabled(inner_total == 0),
    })
}

fn build_login_gate_section(state: &ConsoleState) -> SceneSection {
    SceneSection::new("muconsole-login-gate", copy::LOGIN_GATE_TITLE).with_component(
        SceneComponent::PlatformPanel(PlatformPanel {
            id: "muconsole-login-gate-panel".into(),
            title: copy::LOGIN_GATE_TITLE.into(),
            summary: Some(copy::LOGIN_GATE_BODY.into()),
            tone: SemanticTone::Info,
            attributes: vec![KeyValueEntry {
                key: "Access".into(),
                value: "Sign in required".into(),
                detail: None,
                tone: SemanticTone::Neutral,
                mono: false,
                action: None,
            }],
            controls: vec![ActionControl::new(
                "login-gate-signin",
                copy::SIGN_IN_LABEL,
                "login-gate-signin",
            )
            .with_attr("data-href", state.sign_in_href())
            .with_tone(SemanticTone::Info)],
        }),
    )
}

fn build_auth_denied_lockdown_section(state: &ConsoleState, auth: &AuthState) -> SceneSection {
    let summary = auth
        .reason_code
        .filter(|code| *code != mu_console_protocol::AuthReasonCode::AuthorizationDenied)
        .map_or(copy::AUTH_DENIED_LOCKDOWN_BODY, copy::auth_reason_label);
    SceneSection::new("sec-auth-denied-lockdown", copy::AUTH_DENIED_STATUS).with_component(
        SceneComponent::PlatformPanel(PlatformPanel {
            id: "panel-auth-denied-lockdown".into(),
            title: copy::AUTH_DENIED_STATUS.into(),
            summary: Some(summary.into()),
            tone: SemanticTone::Fail,
            attributes: vec![KeyValueEntry {
                key: "Access".into(),
                value: copy::AUTH_DENIED_STATUS.into(),
                detail: None,
                tone: SemanticTone::Fail,
                mono: false,
                action: None,
            }],
            controls: vec![ActionControl::new(
                "auth-gate-signin",
                copy::SIGN_OUT_LABEL,
                "auth-gate-signin",
            )
            .with_attr("data-href", state.sign_out_href())
            .with_tone(SemanticTone::Warn)],
        }),
    )
}

fn build_auth_required_section(state: &ConsoleState, auth: &AuthState) -> SceneSection {
    let mut controls = Vec::new();
    if auth.status == AuthStatus::Anonymous {
        controls.push(
            ActionControl::new("auth-gate-signin", copy::SIGN_IN_LABEL, "auth-gate-signin")
                .with_attr("data-href", state.sign_in_href())
                .with_tone(SemanticTone::Info),
        );
    } else if auth.status == AuthStatus::AuthorizationDenied {
        controls.push(
            ActionControl::new("auth-gate-signin", copy::SIGN_OUT_LABEL, "auth-gate-signin")
                .with_attr("data-href", state.sign_out_href())
                .with_tone(SemanticTone::Warn),
        );
    } else if auth.status == AuthStatus::PasskeyEnrollmentRequired {
        controls.push(
            ActionControl::new(
                "auth-gate-register-passkey",
                copy::REGISTER_PASSKEY_BTN,
                "auth-gate-register-passkey",
            )
            .with_tone(SemanticTone::Info),
        );
    } else if auth.status == AuthStatus::Active {
        controls.push(
            ActionControl::new("hdr-role", copy::SESSION_ROLES_TITLE, "hdr-role")
                .with_tone(SemanticTone::Info),
        );
    }
    SceneSection::new("sec-auth-required", copy::AUTH_REQUIRED_TITLE).with_component(
        SceneComponent::PlatformPanel(PlatformPanel {
            id: "panel-auth-required".into(),
            title: copy::AUTH_REQUIRED_TITLE.into(),
            summary: Some(copy::AUTH_REQUIRED_BODY.into()),
            tone: SemanticTone::Warn,
            attributes: vec![KeyValueEntry {
                key: "Requested Route".into(),
                value: router::format_hash(&state.route),
                detail: None,
                tone: SemanticTone::Neutral,
                mono: true,
                action: None,
            }],
            controls,
        }),
    )
}

fn build_dashboard_sections(state: &ConsoleState, auth: &AuthState, scene: &mut ConsoleScene) {
    let is_active = auth.status == AuthStatus::Active;
    let attention_rows = if is_active {
        build_attention_rows(state, auth)
    } else {
        Vec::new()
    };
    let cells = build_application_cells(state);
    let any_app_fail = cells.iter().any(|c| c.tone == SemanticTone::Fail);

    let (rollup_tone, rollup_title) = if is_active && !attention_rows.is_empty() {
        (
            SemanticTone::Fail,
            format!("{} Item(s) Need Attention", attention_rows.len()),
        )
    } else if any_app_fail {
        (SemanticTone::Fail, copy::DASH_DEGRADED_ANON_TITLE.into())
    } else {
        (SemanticTone::Pass, copy::DASH_NOMINAL_TITLE.into())
    };
    scene.banners.push(SceneBanner {
        id: "dash-rollup-banner".into(),
        tone: rollup_tone,
        title: rollup_title,
        message: copy::DASH_ROLLUP_MESSAGE.into(),
        action: None,
    });

    scene.sections.push(
        SceneSection::new("sec-dash-applications", copy::DASH_APPLICATIONS_TITLE).with_component(
            SceneComponent::StatusMatrix(StatusMatrix {
                id: "dash-subsystem-cards".into(),
                title: copy::DASH_APPLICATIONS_MATRIX_TITLE.into(),
                cells,
            }),
        ),
    );

    if is_active && auth.has_policy("ci:nodes.read") {
        scene.sections.push(build_fleet_section(state));
    }

    if is_active {
        scene
            .sections
            .push(build_attention_section(state, attention_rows));
    }
}

fn build_application_cells(state: &ConsoleState) -> Vec<MatrixCell> {
    let health = state.client.view().map(|v| &v.subsystem_health);
    let applications: [(Area, SubsystemId, &str); 7] = [
        (Area::Ci, SubsystemId::Ci, "Continuous Integration"),
        (Area::Cd, SubsystemId::Cd, "Continuous Delivery"),
        (Area::AbDeploy, SubsystemId::AbDeploy, "A|B Deployment"),
        (Area::Repository, SubsystemId::Repository, "Registry"),
        (
            Area::Observability,
            SubsystemId::Observability,
            "Observability",
        ),
        (Area::ModelRunner, SubsystemId::ModelRunner, "Model Runner"),
        (Area::Platform, SubsystemId::Platform, "Platform"),
    ];

    applications
        .iter()
        .map(|&(area, sub_id, title)| {
            let permitted = state.can_access_area(area);
            let slug = area.slug();
            let entry: Option<&SubsystemHealthEntry> = health.and_then(|h| h.get(sub_id));
            let (status, tone) = subsystem_status_and_tone(state, area, entry);
            // Generic status word ONLY on unpermitted cards (#50, #55).
            let status_word = copy::subsystem_status_word(status);
            let detail = if permitted {
                Some(subsystem_permitted_detail(state, area, entry))
            } else {
                None
            };
            MatrixCell {
                id: format!("dash-card:{slug}"),
                label: title.into(),
                value: status_word.into(),
                detail,
                signal: Some(tone_signal(tone)),
                tone,
                action: Some(
                    ActionControl::new(
                        format!("dash-card:{slug}"),
                        if permitted { "Open" } else { status_word },
                        format!("dash-card:{slug}"),
                    )
                    .with_area(slug)
                    .with_tone(tone)
                    .disabled(!permitted),
                ),
            }
        })
        .collect()
}

fn build_fleet_section(state: &ConsoleState) -> SceneSection {
    let q = state.filter_query.trim().to_ascii_lowercase();
    let rows: Vec<TableRow> = state
        .client
        .view()
        .map(|v| {
            v.nodes
                .iter()
                .filter(|n| {
                    q.is_empty()
                        || n.id.to_ascii_lowercase().contains(&q)
                        || n.arch.to_ascii_lowercase().contains(&q)
                })
                .map(|n| {
                    let node_tone = if n.online {
                        SemanticTone::Pass
                    } else {
                        SemanticTone::Fail
                    };
                    let status_label = if n.online { "Online" } else { "Offline" };
                    TableRow {
                        id: format!("dash-fleet-{}", n.id),
                        tone: node_tone,
                        cells: vec![
                            TableCell {
                                text: n.id.clone(),
                                tone: SemanticTone::Info,
                                signal: Some(if n.online {
                                    SignalGlyph::BlockPassed
                                } else {
                                    SignalGlyph::BlockFailed
                                }),
                                mono: true,
                                action: None,
                            },
                            TableCell {
                                text: n.arch.clone(),
                                tone: SemanticTone::Neutral,
                                signal: None,
                                mono: true,
                                action: None,
                            },
                            TableCell {
                                text: status_label.into(),
                                tone: node_tone,
                                signal: None,
                                mono: true,
                                action: None,
                            },
                            TableCell {
                                text: format!("{}/{}", n.active_slots, n.max_slots),
                                tone: SemanticTone::Neutral,
                                signal: None,
                                mono: true,
                                action: None,
                            },
                            TableCell {
                                text: format!("{:.1}%", f64::from(n.cpu_load_bp) / 100.0),
                                tone: SemanticTone::Neutral,
                                signal: None,
                                mono: true,
                                action: None,
                            },
                        ],
                        row_action: None,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    SceneSection::new("sec-dash-fleet", copy::DASH_FLEET_TITLE).with_component(
        SceneComponent::TableGrid(TableGrid {
            id: "dash-fleet-table".into(),
            title: copy::DASH_FLEET_TABLE_TITLE.into(),
            columns: vec![
                TableColumn {
                    key: "node".into(),
                    label: "Node".into(),
                    numeric: false,
                },
                TableColumn {
                    key: "arch".into(),
                    label: "Arch".into(),
                    numeric: false,
                },
                TableColumn {
                    key: "status".into(),
                    label: "Status".into(),
                    numeric: false,
                },
                TableColumn {
                    key: "slots".into(),
                    label: "Slots".into(),
                    numeric: true,
                },
                TableColumn {
                    key: "cpu".into(),
                    label: "CPU Load".into(),
                    numeric: true,
                },
            ],
            rows,
            empty_message: Some(if state.filter_query.is_empty() {
                copy::NO_FLEET_NODES.into()
            } else {
                copy::NO_FILTER_MATCHES.into()
            }),
            pagination: None,
        }),
    )
}

fn build_attention_section(state: &ConsoleState, rows: Vec<TableRow>) -> SceneSection {
    SceneSection::new("sec-dash-attention", copy::DASH_NEEDS_ATTENTION_TITLE).with_component(
        SceneComponent::TableGrid(TableGrid {
            id: "dash-attention-table".into(),
            title: copy::DASH_NEEDS_ATTENTION_TITLE.into(),
            columns: vec![
                TableColumn {
                    key: "scope".into(),
                    label: "Scope".into(),
                    numeric: false,
                },
                TableColumn {
                    key: "target".into(),
                    label: "Target".into(),
                    numeric: false,
                },
                TableColumn {
                    key: "finding".into(),
                    label: "Finding".into(),
                    numeric: false,
                },
                TableColumn {
                    key: "status".into(),
                    label: "Status".into(),
                    numeric: false,
                },
            ],
            rows,
            empty_message: Some(if state.filter_query.is_empty() {
                copy::NO_ATTENTION_ITEMS.into()
            } else {
                copy::NO_FILTER_MATCHES.into()
            }),
            pagination: None,
        }),
    )
}

fn build_attention_rows(state: &ConsoleState, auth: &AuthState) -> Vec<TableRow> {
    if auth.status != AuthStatus::Active {
        return Vec::new();
    }
    let q = state.filter_query.trim().to_ascii_lowercase();
    let mut rows = Vec::new();
    if state.can_access_area(Area::Ci) {
        append_ci_attention_rows(state, &q, &mut rows);
    }
    if state.can_access_area(Area::Observability) {
        append_obs_attention_rows(state, &q, &mut rows);
    }
    if auth.has_policy("ci:nodes.read") {
        append_fleet_attention_rows(state, &q, &mut rows);
    }
    rows
}

fn append_ci_attention_rows(state: &ConsoleState, q: &str, rows: &mut Vec<TableRow>) {
    for job in state.visible_ci_jobs() {
        if !job.queue_state.eq_ignore_ascii_case("failed") {
            continue;
        }
        let target = format!(
            "{}/{} #{} ({})",
            job.repo.owner, job.repo.repo, job.job_id, job.ref_name
        );
        let finding = if job.tests_total > 0 {
            format!("{}/{} tests passed", job.tests_passed, job.tests_total)
        } else {
            "Pipeline run failed".into()
        };
        if !q.is_empty()
            && !target.to_ascii_lowercase().contains(q)
            && !finding.to_ascii_lowercase().contains(q)
        {
            continue;
        }
        rows.push(TableRow {
            id: format!("att-ci-{}", job.job_id),
            tone: SemanticTone::Fail,
            cells: vec![
                TableCell {
                    text: "Application".into(),
                    tone: SemanticTone::Fail,
                    signal: Some(SignalGlyph::BlockFailed),
                    mono: true,
                    action: None,
                },
                TableCell {
                    text: target,
                    tone: SemanticTone::Fail,
                    signal: None,
                    mono: true,
                    action: None,
                },
                TableCell {
                    text: finding,
                    tone: SemanticTone::Fail,
                    signal: None,
                    mono: true,
                    action: None,
                },
                TableCell {
                    text: "Failed".into(),
                    tone: SemanticTone::Fail,
                    signal: None,
                    mono: true,
                    action: None,
                },
            ],
            row_action: Some(ActionControl::new(
                format!("ci-job:{}", job.job_id),
                "Inspect",
                format!("ci-job:{}", job.job_id),
            )),
        });
    }
}

fn append_obs_attention_rows(state: &ConsoleState, q: &str, rows: &mut Vec<TableRow>) {
    let Some(view) = state.client.view() else {
        return;
    };
    let obs_slug = Area::Observability.slug();
    for prob in &view.observe.problems {
        if !prob.active {
            continue;
        }
        let target = if prob.impacted_entity.is_empty() {
            "Observability".into()
        } else {
            prob.impacted_entity.clone()
        };
        if !q.is_empty()
            && !prob.title.to_ascii_lowercase().contains(q)
            && !target.to_ascii_lowercase().contains(q)
        {
            continue;
        }
        let status = copy::attention_obs_status_label(prob);
        rows.push(TableRow {
            id: format!("att-obs-{}", prob.problem_id),
            tone: SemanticTone::Fail,
            cells: vec![
                TableCell {
                    text: "Application".into(),
                    tone: SemanticTone::Fail,
                    signal: Some(SignalGlyph::BlockFailed),
                    mono: true,
                    action: None,
                },
                TableCell {
                    text: target,
                    tone: SemanticTone::Fail,
                    signal: None,
                    mono: true,
                    action: None,
                },
                TableCell {
                    text: prob.title.clone(),
                    tone: SemanticTone::Fail,
                    signal: None,
                    mono: false,
                    action: None,
                },
                TableCell {
                    text: status.into(),
                    tone: SemanticTone::Fail,
                    signal: None,
                    mono: true,
                    action: None,
                },
            ],
            row_action: Some(
                ActionControl::new(
                    format!("nav-area:{obs_slug}"),
                    "Inspect",
                    format!("nav-area:{obs_slug}"),
                )
                .with_area(obs_slug),
            ),
        });
    }
}

fn append_fleet_attention_rows(state: &ConsoleState, q: &str, rows: &mut Vec<TableRow>) {
    let Some(view) = state.client.view() else {
        return;
    };
    let ci_slug = Area::Ci.slug();
    for node in &view.nodes {
        if node.online {
            continue;
        }
        let finding = if node.arch.is_empty() {
            "Node unreachable".into()
        } else {
            format!("{} worker unreachable", node.arch)
        };
        if !q.is_empty()
            && !node.id.to_ascii_lowercase().contains(q)
            && !finding.to_ascii_lowercase().contains(q)
        {
            continue;
        }
        rows.push(TableRow {
            id: format!("att-fleet-{}", node.id),
            tone: SemanticTone::Fail,
            cells: vec![
                TableCell {
                    text: "Fleet".into(),
                    tone: SemanticTone::Fail,
                    signal: Some(SignalGlyph::BlockFailed),
                    mono: true,
                    action: None,
                },
                TableCell {
                    text: node.id.clone(),
                    tone: SemanticTone::Fail,
                    signal: None,
                    mono: true,
                    action: None,
                },
                TableCell {
                    text: finding,
                    tone: SemanticTone::Fail,
                    signal: None,
                    mono: false,
                    action: None,
                },
                TableCell {
                    text: "Down".into(),
                    tone: SemanticTone::Fail,
                    signal: None,
                    mono: true,
                    action: None,
                },
            ],
            row_action: Some(
                ActionControl::new("nav-ci:nodes", "Inspect", "nav-ci:nodes").with_area(ci_slug),
            ),
        });
    }
}

fn failing_attention_count(state: &ConsoleState) -> usize {
    let auth = state.auth();
    if auth.status != AuthStatus::Active {
        return 0;
    }
    let ci = if state.can_access_area(Area::Ci) {
        state
            .visible_ci_jobs()
            .iter()
            .filter(|j| j.queue_state.eq_ignore_ascii_case("failed"))
            .count()
    } else {
        0
    };
    let obs = if state.can_access_area(Area::Observability) {
        state.client.view().map_or(0, |v| {
            v.observe.problems.iter().filter(|p| p.active).count()
        })
    } else {
        0
    };
    let fleet = if auth.has_policy("ci:nodes.read") {
        state
            .client
            .view()
            .map_or(0, |v| v.nodes.iter().filter(|n| !n.online).count())
    } else {
        0
    };
    ci + obs + fleet
}

fn area_has_active_failure(state: &ConsoleState, area: Area) -> bool {
    match area {
        Area::Ci => state
            .visible_ci_jobs()
            .iter()
            .any(|j| j.queue_state.eq_ignore_ascii_case("failed")),
        Area::Cd => state
            .client
            .view()
            .is_some_and(|v| v.cd_runs.iter().any(|r| r.state == CdRunState::Failed)),
        Area::Observability => state
            .client
            .view()
            .is_some_and(|v| v.observe.problems.iter().any(|p| p.active)),
        _ => false,
    }
}

fn subsystem_status_and_tone(
    state: &ConsoleState,
    area: Area,
    entry: Option<&SubsystemHealthEntry>,
) -> (SubsystemStatus, SemanticTone) {
    if let Some(e) = entry {
        return match e.status {
            SubsystemStatus::Live => {
                if area_has_active_failure(state, area) {
                    (SubsystemStatus::Degraded, SemanticTone::Fail)
                } else {
                    (SubsystemStatus::Live, SemanticTone::Pass)
                }
            }
            SubsystemStatus::InMaintenance | SubsystemStatus::PartialMaintenance => {
                (e.status, SemanticTone::Info)
            }
            SubsystemStatus::Degraded | SubsystemStatus::Offline => (e.status, SemanticTone::Fail),
        };
    }
    if state.client.view().is_none() {
        return (SubsystemStatus::Live, SemanticTone::Neutral);
    }
    if area_has_active_failure(state, area) {
        (SubsystemStatus::Degraded, SemanticTone::Fail)
    } else {
        (SubsystemStatus::Live, SemanticTone::Pass)
    }
}

fn subsystem_permitted_detail(
    state: &ConsoleState,
    area: Area,
    entry: Option<&SubsystemHealthEntry>,
) -> String {
    let view = state.client.view();
    if let Some(e) = entry {
        let obs_override = area == Area::Observability
            && view.is_some_and(|v| {
                e.summary.is_empty()
                    || e.summary.eq_ignore_ascii_case("operational")
                    || v.observe.problems.iter().any(|p| p.active)
            });
        if !e.summary.is_empty() && !obs_override {
            return e.summary.clone();
        }
    }
    let Some(view) = view else {
        return "Idle".into();
    };
    match area {
        Area::Ci => format!("{} visible job(s)", state.visible_ci_jobs().len()),
        Area::Cd => format!("{} CD run(s)", view.cd_runs.len()),
        Area::AbDeploy => view.gate.as_ref().map_or_else(
            || "A|B slots idle".into(),
            |g| format!("{}/{}: {:?}", g.passed(), g.total(), g.outcome),
        ),
        Area::Repository => format!("{} package(s)", view.registry.packages.len()),
        Area::Observability => {
            let active_problems = view.observe.problems.iter().filter(|p| p.active).count();
            if active_problems > 0 {
                format!(
                    "{} logs · {} traces · {} metrics · {active_problems} active SLO problem(s)",
                    view.observe.logs.len(),
                    view.observe.traces.len(),
                    view.observe.metrics.len(),
                )
            } else {
                format!(
                    "{} logs · {} traces · {} metrics",
                    view.observe.logs.len(),
                    view.observe.traces.len(),
                    view.observe.metrics.len(),
                )
            }
        }
        Area::ModelRunner => view.model_runner.status.as_ref().map_or_else(
            || "Model runner idle".into(),
            |s| format!("queue={}", s.total_queue_depth),
        ),
        Area::Platform => format!("{} role(s)", view.platform.roles.len()),
        Area::Dashboard => "Overview".into(),
    }
}

fn build_ci_sections(
    state: &ConsoleState,
    auth: &AuthState,
    subview: CiSubView,
    is_public_anon: bool,
    scene: &mut ConsoleScene,
) {
    if is_public_anon || matches!(subview, CiSubView::QueueLogs) {
        build_ci_queue_logs(state, auth, is_public_anon, scene);
        return;
    }
    let view = state.client.view();
    match subview {
        CiSubView::QueueLogs => unreachable!(),
        CiSubView::Scorecards => {
            let ordered = state.ordered_carousel_repos();
            let jobs = state.visible_ci_jobs();
            let mut sec = SceneSection::new("sec-ci-scorecards", "CI Scorecards Carousel");
            for (idx, r) in ordered.iter().take(8).enumerate() {
                let tone = match r.status {
                    RepoStatus::Passing => SemanticTone::Pass,
                    RepoStatus::Running => SemanticTone::Info,
                    RepoStatus::Failing => SemanticTone::Fail,
                    RepoStatus::Unknown => SemanticTone::Neutral,
                };
                let pass_jobs = jobs
                    .iter()
                    .filter(|j| j.repo == r.id && !j.queue_state.eq_ignore_ascii_case("failed"))
                    .count();
                sec = sec.with_component(SceneComponent::SparklineCard(SparklineCard {
                    id: format!("ci-scorecard:{idx}"),
                    title: format!("{}/{}", r.id.owner, r.id.repo),
                    metric_value: format!("{}%", r.score),
                    unit: None,
                    delta_label: Some(format!(
                        "cov {:.1}% · {pass_jobs} ok",
                        f64::from(r.coverage_bp) / 100.0
                    )),
                    tone,
                    samples: vec![60.0, 75.0, 82.0, f64::from(r.score)],
                    range_label: Some(r.default_branch.clone()),
                    action: Some(ActionControl::new(
                        format!("repo-select:{idx}"),
                        "Select",
                        format!("repo-select:{idx}"),
                    )),
                }));
            }
            scene.sections.push(sec);
        }
        CiSubView::Trainway => {
            let topo = view.and_then(|v| v.topology.as_ref());
            let stations: Vec<String> = topo
                .map(|t| t.stations.iter().map(|s| s.label.clone()).collect())
                .unwrap_or_else(|| vec!["FMT".into(), "CLIPPY".into(), "TEST".into()]);
            let tracks: Vec<TrainwayTrack> = topo
                .map(|t| {
                    t.trains
                        .iter()
                        .enumerate()
                        .map(|(idx, tr)| TrainwayTrack {
                            id: format!("c5-track-{}", tr.ref_name),
                            ref_name: tr.ref_name.clone(),
                            lane: u32::try_from(idx).unwrap_or(0),
                            fork_stage: 0,
                            dashed: tr.node_lost,
                            fading: false,
                            checkpoints: tr
                                .stage_states
                                .iter()
                                .enumerate()
                                .map(|(si, st)| TrainwayCheckpoint {
                                    stage_index: u32::try_from(si).unwrap_or(0),
                                    stage_name: stations.get(si).cloned().unwrap_or_default(),
                                    signal: stage_state_signal(*st),
                                    tone: stage_state_tone(*st),
                                    label: Some(format!("{st:?}")),
                                })
                                .collect(),
                            node_tag: tr.node_id.clone(),
                            node_lost: tr.node_lost,
                            action: Some(ActionControl::new(
                                format!("c5-track-{}", tr.ref_name),
                                "Inspect",
                                format!("c5-track-{}", tr.ref_name),
                            )),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let can_resume = auth.has_policy("ci:jobs.resume");
            let can_restart = auth.has_policy("ci:jobs.restart");
            let sel_job = selected_ci_job(state);
            let controls = if let Some(job) = sel_job.as_ref() {
                vec![
                    ActionControl::new(
                        format!("ci-job-resume:{}", job.job_id),
                        "Resume",
                        format!("ci-job-resume:{}", job.job_id),
                    )
                    .disabled(!can_resume),
                    ActionControl::new(
                        format!("ci-job-restart:{}", job.job_id),
                        "Restart",
                        format!("ci-job-restart:{}", job.job_id),
                    )
                    .disabled(!can_restart)
                    .with_tone(SemanticTone::Warn),
                ]
            } else {
                vec![]
            };
            scene.sections.push(
                SceneSection::new("sec-ci-trainway", "CI Pipeline Trainway")
                    .with_component(SceneComponent::TrainwayGraph(TrainwayGraph {
                        id: "tw-ci-main".into(),
                        title: "Active Branch Lanes".into(),
                        stations,
                        tracks,
                    }))
                    .with_component(SceneComponent::PlatformPanel(PlatformPanel {
                        id: "c5-selected-inspector".into(),
                        title: sel_job.as_ref().map_or_else(
                            || "Selected Track Inspector".into(),
                            |j| format!("Selected Job #{} ({})", j.job_id, j.ref_name),
                        ),
                        summary: sel_job.as_ref().map(|j| {
                            format!("{}/{} · {}", j.repo.owner, j.repo.repo, j.benchmark_summary)
                        }),
                        tone: sel_job.as_ref().map_or(SemanticTone::Neutral, job_tone),
                        attributes: vec![],
                        controls,
                    })),
            );
        }
        CiSubView::WorkerNodes => {
            let rows: Vec<TableRow> = view
                .map(|v| {
                    v.nodes
                        .iter()
                        .map(|n| TableRow {
                            id: format!("node-{}", n.id),
                            tone: if n.online {
                                SemanticTone::Pass
                            } else {
                                SemanticTone::Fail
                            },
                            cells: vec![
                                TableCell {
                                    text: n.id.clone(),
                                    tone: SemanticTone::Info,
                                    signal: Some(if n.online {
                                        SignalGlyph::BlockPassed
                                    } else {
                                        SignalGlyph::BlockFailed
                                    }),
                                    mono: true,
                                    action: None,
                                },
                                TableCell {
                                    text: n.arch.clone(),
                                    tone: SemanticTone::Neutral,
                                    signal: None,
                                    mono: true,
                                    action: None,
                                },
                                TableCell {
                                    text: format!("{}/{}", n.active_slots, n.max_slots),
                                    tone: SemanticTone::Neutral,
                                    signal: None,
                                    mono: true,
                                    action: None,
                                },
                                TableCell {
                                    text: format!("{:.1}%", f64::from(n.cpu_load_bp) / 100.0),
                                    tone: SemanticTone::Neutral,
                                    signal: None,
                                    mono: true,
                                    action: None,
                                },
                            ],
                            row_action: None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            scene.sections.push(
                SceneSection::new("sec-ci-nodes", "ActionRunner Worker Nodes").with_component(
                    SceneComponent::TableGrid(TableGrid {
                        id: "tbl-ci-nodes".into(),
                        title: "Registered Fabric Nodes (max_slots = 1)".into(),
                        columns: vec![
                            TableColumn {
                                key: "id".into(),
                                label: "Node".into(),
                                numeric: false,
                            },
                            TableColumn {
                                key: "arch".into(),
                                label: "Arch".into(),
                                numeric: false,
                            },
                            TableColumn {
                                key: "slots".into(),
                                label: "Slots".into(),
                                numeric: true,
                            },
                            TableColumn {
                                key: "cpu".into(),
                                label: "CPU".into(),
                                numeric: true,
                            },
                        ],
                        rows,
                        empty_message: Some("No worker nodes reported".into()),
                        pagination: None,
                    }),
                ),
            );
        }
        CiSubView::SandboxProfiles => {
            let rows: Vec<TableRow> = view
                .map(|v| {
                    v.sandboxes
                        .iter()
                        .map(|s| TableRow {
                            id: format!("sb-{}", s.profile_name),
                            tone: SemanticTone::Info,
                            cells: vec![
                                TableCell {
                                    text: s.profile_name.clone(),
                                    tone: SemanticTone::Info,
                                    signal: None,
                                    mono: true,
                                    action: None,
                                },
                                TableCell {
                                    text: s.backend.clone(),
                                    tone: SemanticTone::Pass,
                                    signal: None,
                                    mono: true,
                                    action: None,
                                },
                                TableCell {
                                    text: format!(
                                        "slots={} pids={} threads={}",
                                        s.max_slots, s.max_pids, s.thread_budget
                                    ),
                                    tone: SemanticTone::Neutral,
                                    signal: None,
                                    mono: true,
                                    action: None,
                                },
                            ],
                            row_action: None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            scene.sections.push(
                SceneSection::new("sec-ci-sandboxes", "MuIsolation Sandbox Profiles")
                    .with_component(SceneComponent::TableGrid(TableGrid {
                        id: "tbl-ci-sandboxes".into(),
                        title: "Configured Sandbox Ceilings".into(),
                        columns: vec![
                            TableColumn {
                                key: "profile".into(),
                                label: "Profile".into(),
                                numeric: false,
                            },
                            TableColumn {
                                key: "backend".into(),
                                label: "Backend".into(),
                                numeric: false,
                            },
                            TableColumn {
                                key: "limits".into(),
                                label: "Limits".into(),
                                numeric: false,
                            },
                        ],
                        rows,
                        empty_message: Some("No sandbox profiles available".into()),
                        pagination: None,
                    })),
            );
        }
    }
}

fn build_ci_queue_logs(
    state: &ConsoleState,
    auth: &AuthState,
    is_public_anon: bool,
    scene: &mut ConsoleScene,
) {
    let jobs = state.visible_ci_jobs();
    let job_rows: Vec<TableRow> = jobs
        .iter()
        .map(|j| {
            let tone = job_tone(j);
            TableRow {
                id: format!("ci-job-{}", j.job_id),
                tone,
                cells: vec![
                    TableCell {
                        text: format!("#{}", j.job_id),
                        tone,
                        signal: Some(tone_signal(tone)),
                        mono: true,
                        action: None,
                    },
                    TableCell {
                        text: format!("{}/{}", j.repo.owner, j.repo.repo),
                        tone: SemanticTone::Neutral,
                        signal: None,
                        mono: true,
                        action: None,
                    },
                    TableCell {
                        text: j.ref_name.clone(),
                        tone: SemanticTone::Neutral,
                        signal: None,
                        mono: true,
                        action: None,
                    },
                    TableCell {
                        text: j.queue_state.clone(),
                        tone,
                        signal: None,
                        mono: false,
                        action: None,
                    },
                ],
                row_action: Some(ActionControl::new(
                    format!("ci-job:{}", j.job_id),
                    "Select",
                    format!("ci-job:{}", j.job_id),
                )),
            }
        })
        .collect();

    let sel = selected_ci_job(state).or_else(|| jobs.first().cloned());
    let mut section = SceneSection::new(
        "sec-ci-queue-logs",
        if is_public_anon {
            copy::PUBLIC_STEPS_TITLE
        } else {
            "CI Queue & Job Steps"
        },
    )
    .with_component(SceneComponent::TableGrid(TableGrid {
        id: "ci-queue-table".into(),
        title: if is_public_anon {
            copy::PUBLIC_STEPS_SUBTITLE.into()
        } else {
            "Filtered CI Jobs".into()
        },
        columns: vec![
            TableColumn {
                key: "job".into(),
                label: "Job".into(),
                numeric: true,
            },
            TableColumn {
                key: "repo".into(),
                label: "Repository".into(),
                numeric: false,
            },
            TableColumn {
                key: "ref".into(),
                label: "Branch".into(),
                numeric: false,
            },
            TableColumn {
                key: "status".into(),
                label: "Status".into(),
                numeric: false,
            },
        ],
        rows: job_rows,
        empty_message: Some("No matching CI jobs".into()),
        pagination: None,
    }));

    if let Some(job) = sel {
        let checklist_items: Vec<GateItem> = job
            .stages
            .iter()
            .map(|st| {
                let reason_text = st.skip_reason.as_ref().map(copy::skip_reason_label);
                let detail = if is_public_anon {
                    // Anonymous view: never leak internal node IDs or thread counts (#50).
                    reason_text
                        .or_else(|| Some(format!("{}s · exit {}", st.duration_secs, st.exit_code)))
                } else {
                    let base = format!(
                        "{} · {}t · {}s",
                        st.assigned_node.as_deref().unwrap_or("unassigned"),
                        st.threads,
                        st.duration_secs
                    );
                    Some(match reason_text {
                        Some(r) => format!("{base} · {r}"),
                        None => base,
                    })
                };
                GateItem {
                    id: format!("ci-stage-{}", st.id),
                    label: st.name.clone(),
                    status_label: format!("{:?}", st.state),
                    detail,
                    signal: stage_state_signal(st.state),
                    tone: stage_state_tone(st.state),
                    required: true,
                    action: None,
                }
            })
            .collect();

        let github_url = format!(
            "https://github.com/{}/{}/commit/{}",
            job.repo.owner,
            job.repo.repo,
            if job.commit_sha.is_empty() {
                "main"
            } else {
                &job.commit_sha
            }
        );
        let mut actions = vec![ActionControl::new(
            format!("ci-github-link:{}", job.job_id),
            copy::OPEN_GITHUB_COMMIT,
            "open-external-url",
        )
        .with_attr("data-external-href", github_url)
        .with_attr("target", "_blank")
        .with_attr("rel", "noopener noreferrer")];

        if !is_public_anon {
            let can_resume = auth.has_policy("ci:jobs.resume");
            let can_restart = auth.has_policy("ci:jobs.restart");
            actions.push(
                ActionControl::new(
                    format!("ci-job-resume:{}", job.job_id),
                    "Resume",
                    format!("ci-job-resume:{}", job.job_id),
                )
                .disabled(!can_resume),
            );
            actions.push(
                ActionControl::new(
                    format!("ci-job-restart:{}", job.job_id),
                    "Restart",
                    format!("ci-job-restart:{}", job.job_id),
                )
                .disabled(!can_restart)
                .with_tone(SemanticTone::Warn),
            );
        }

        section = section.with_component(SceneComponent::GateChecklist(GateChecklist {
            id: format!("ci-job-checklist-{}", job.job_id),
            title: format!(
                "Job #{} Stage Checklist ({}/{})",
                job.job_id, job.repo.owner, job.repo.repo
            ),
            environment: Some(job.ref_name.clone()),
            overall_tone: job_tone(&job),
            items: checklist_items,
            actions,
        }));

        // Authenticated mode ONLY: render job.raw_log_lines (never OTel log fallback in anonymous CI).
        if !is_public_anon {
            let entries: Vec<LogEntry> = job
                .raw_log_lines
                .iter()
                .enumerate()
                .map(|(i, line)| LogEntry {
                    line_no: u32::try_from(i + 1).ok(),
                    timestamp: None,
                    level: SemanticTone::Neutral,
                    signal: None,
                    text: line.clone(),
                    trace_id: None,
                    action: None,
                })
                .collect();
            section = section.with_component(SceneComponent::LogViewer(LogViewer {
                id: "ci-job-raw-logs".into(),
                title: format!("Job #{} Build Output", job.job_id),
                font_alias: Some("MuCRT".into()),
                wrap: true,
                stream_id: Some(format!("ci-{}", job.job_id)),
                entries,
                controls: vec![ActionControl::new(
                    "ci-log-follow-toggle",
                    if state.follow_log {
                        "Follow: On"
                    } else {
                        "Follow: Off"
                    },
                    "ci-log-follow-toggle",
                )
                .with_pressed(state.follow_log)],
                pagination: None,
            }));
        }
    }

    scene.sections.push(section);
}

fn build_cd_sections(
    state: &ConsoleState,
    _auth: &AuthState,
    subview: CdSubView,
    scene: &mut ConsoleScene,
) {
    // Strictly read-only CD (#56): Cargo / .NET / Maven pinpoint failure diagnosis + links.
    let can_repo = state.can_access_area(Area::Repository);
    let can_ab = state.can_access_area(Area::AbDeploy);
    let view = state.client.view();

    let mut sec = SceneSection::new(
        "sec-cd-readonly",
        format!("Continuous Delivery · {}", SubView::Cd(subview).label()),
    );
    match subview {
        CdSubView::Runs => {
            let mut entries = vec![KeyValueEntry {
                key: "Policy".into(),
                value: "Read-only diagnosis".into(),
                detail: Some("Promotions and rollbacks execute under A|B Deploy".into()),
                tone: SemanticTone::Neutral,
                mono: false,
                action: None,
            }];
            if let Some(v) = view {
                for run in &v.cd_runs {
                    let eco = copy::cd_ecosystem_label(&run.ecosystem);
                    let detail = run.failure.as_ref().map_or_else(
                        || format!("{}/{} ({})", run.repo.owner, run.repo.repo, run.ref_name),
                        |f| {
                            format!(
                                "{} {} ({}): {}",
                                f.step_id,
                                f.package_name.as_deref().unwrap_or(""),
                                f.package_version.as_deref().unwrap_or(""),
                                f.reason
                            )
                        },
                    );
                    entries.push(KeyValueEntry {
                        key: format!("{eco} · {}", run.run_id),
                        value: format!("{:?}", run.state),
                        detail: Some(detail),
                        tone: match run.state {
                            CdRunState::Succeeded => SemanticTone::Pass,
                            CdRunState::Failed => SemanticTone::Fail,
                            CdRunState::Blocked | CdRunState::RolledBack => SemanticTone::Warn,
                            CdRunState::Pending | CdRunState::Running => SemanticTone::Info,
                        },
                        mono: true,
                        action: None,
                    });
                }
            }
            sec = sec.with_component(SceneComponent::PlatformPanel(PlatformPanel {
                id: "cd-diagnosis-panel".into(),
                title: "Pinpoint Dependency & Ecosystem Diagnosis".into(),
                summary: Some("Cargo / .NET / Maven publish & artifact staging runs.".into()),
                tone: SemanticTone::Info,
                attributes: entries,
                controls: vec![
                    ActionControl::new(
                        format!("nav-area:{}", Area::Repository.slug()),
                        "Open Registry",
                        format!("nav-area:{}", Area::Repository.slug()),
                    )
                    .with_area(Area::Repository.slug())
                    .disabled(!can_repo),
                    ActionControl::new(
                        "nav-area:ab-deploy",
                        "Open A|B Deploy",
                        "nav-area:ab-deploy",
                    )
                    .with_area("ab-deploy")
                    .disabled(!can_ab),
                ],
            }));
        }
        CdSubView::Pipelines => {
            let rows: Vec<TableRow> = view
                .map(|v| {
                    v.cd_runs
                        .iter()
                        .flat_map(|r| {
                            r.steps.iter().map(move |st| TableRow {
                                id: format!("cd-step-{}-{}", r.run_id, st.step_id),
                                tone: SemanticTone::Info,
                                cells: vec![
                                    TableCell {
                                        text: r.run_id.clone(),
                                        tone: SemanticTone::Info,
                                        signal: None,
                                        mono: true,
                                        action: None,
                                    },
                                    TableCell {
                                        text: st.label.clone(),
                                        tone: SemanticTone::Neutral,
                                        signal: None,
                                        mono: false,
                                        action: None,
                                    },
                                    TableCell {
                                        text: format!("{:?}", st.state),
                                        tone: SemanticTone::Neutral,
                                        signal: None,
                                        mono: true,
                                        action: None,
                                    },
                                ],
                                row_action: None,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            sec = sec.with_component(SceneComponent::TableGrid(TableGrid {
                id: "cd-pipelines-table".into(),
                title: "Pipeline Stage Matrix".into(),
                columns: vec![
                    TableColumn {
                        key: "run".into(),
                        label: "CD Run".into(),
                        numeric: false,
                    },
                    TableColumn {
                        key: "step".into(),
                        label: "Step".into(),
                        numeric: false,
                    },
                    TableColumn {
                        key: "state".into(),
                        label: "State".into(),
                        numeric: false,
                    },
                ],
                rows,
                empty_message: Some("No CD pipeline steps reported".into()),
                pagination: None,
            }));
        }
    }
    scene.sections.push(sec);
}

fn build_ab_sections(
    state: &ConsoleState,
    auth: &AuthState,
    subview: AbDeploySubView,
    scene: &mut ConsoleScene,
) {
    let can_rollback = auth.has_policy("ab:rollback");
    let view = state.client.view();
    let gate = view.and_then(|v| v.gate.as_ref());

    match subview {
        AbDeploySubView::IntegrationGate => {
            let items: Vec<GateItem> = gate
                .map(|g| {
                    g.tests
                        .iter()
                        .map(|t| GateItem {
                            id: format!("gate-test-{}", t.name),
                            label: t.name.clone(),
                            status_label: format!("{:?}", t.state),
                            detail: t.log_excerpt.clone(),
                            signal: match t.state {
                                TestState::Passed => SignalGlyph::TestPassed,
                                TestState::Failed => SignalGlyph::BlockFailed,
                                TestState::Running => SignalGlyph::TestRunning,
                                TestState::Pending => SignalGlyph::TestPending,
                            },
                            tone: match t.state {
                                TestState::Passed => SemanticTone::Pass,
                                TestState::Failed => SemanticTone::Fail,
                                TestState::Running => SemanticTone::Info,
                                TestState::Pending => SemanticTone::Neutral,
                            },
                            required: t.critical,
                            action: None,
                        })
                        .collect()
                })
                .unwrap_or_default();

            let dep_id = gate.map_or("active", |g| g.deployment_id.as_str());
            scene.sections.push(
                SceneSection::new("sec-ab-gate", "A|B Integration Gate").with_component(
                    SceneComponent::GateChecklist(GateChecklist {
                        id: format!("gate:{dep_id}"),
                        title: format!("Deployment {dep_id} Verification Gate"),
                        environment: gate.map(|g| g.target_node.clone()),
                        overall_tone: gate.map_or(SemanticTone::Neutral, |g| match g.outcome {
                            GateOutcome::Promoted => SemanticTone::Pass,
                            GateOutcome::RolledBack | GateOutcome::Tainted => SemanticTone::Fail,
                            GateOutcome::Pending => SemanticTone::Info,
                        }),
                        items,
                        actions: vec![ActionControl::new(
                            format!("ab-rollback:{dep_id}"),
                            "Emergency Rollback",
                            format!("ab-rollback:{dep_id}"),
                        )
                        .disabled(!can_rollback)
                        .with_tone(SemanticTone::Fail)],
                    }),
                ),
            );
        }
        AbDeploySubView::DeploymentsSlots => {
            let cells = if let Some(g) = gate {
                vec![
                    MatrixCell {
                        id: "ab-slot-a".into(),
                        label: "@a".into(),
                        value: if g.restore_slot == Slot::A {
                            "Active / Restore".into()
                        } else {
                            "Staged".into()
                        },
                        detail: Some(g.artifact_digest.clone()),
                        signal: Some(SignalGlyph::BlockPassed),
                        tone: SemanticTone::Pass,
                        action: Some(
                            ActionControl::new(
                                format!("ab-rollback:{}", g.deployment_id),
                                "Rollback to @a",
                                format!("ab-rollback:{}", g.deployment_id),
                            )
                            .disabled(!can_rollback)
                            .with_tone(SemanticTone::Fail),
                        ),
                    },
                    MatrixCell {
                        id: "ab-slot-b".into(),
                        label: "@b".into(),
                        value: if g.staged_slot == Slot::B {
                            "Candidate".into()
                        } else {
                            "Standby".into()
                        },
                        detail: Some(g.target_node.clone()),
                        signal: Some(SignalGlyph::BlockPending),
                        tone: SemanticTone::Info,
                        action: Some(
                            ActionControl::new(
                                format!("ab-rollback-b:{}", g.deployment_id),
                                "Rollback to @b",
                                format!("ab-rollback:{}", g.deployment_id),
                            )
                            .disabled(!can_rollback)
                            .with_tone(SemanticTone::Fail),
                        ),
                    },
                ]
            } else {
                vec![]
            };
            scene.sections.push(
                SceneSection::new("sec-ab-slots", "Symmetric @a / @b Slots").with_component(
                    SceneComponent::StatusMatrix(StatusMatrix {
                        id: "ab-slots-matrix".into(),
                        title: "Dual-Slot A|B State".into(),
                        cells,
                    }),
                ),
            );
        }
        AbDeploySubView::GateTestLogs => {
            // Separate `ab:gate-logs.read`-gated permission tab (#56).
            let entries: Vec<LogEntry> = gate
                .and_then(|g| g.tests.get(state.selected_test_idx))
                .and_then(|t| t.log_excerpt.as_deref())
                .map(|txt| {
                    txt.lines()
                        .enumerate()
                        .map(|(i, l)| LogEntry {
                            line_no: u32::try_from(i + 1).ok(),
                            timestamp: None,
                            level: SemanticTone::Info,
                            signal: None,
                            text: l.to_owned(),
                            trace_id: None,
                            action: None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            scene.sections.push(
                SceneSection::new("sec-ab-gate-logs", "Integration Gate Test Logs").with_component(
                    SceneComponent::LogViewer(LogViewer {
                        id: "ab-gate-log-viewer".into(),
                        title: "Gate Test Transcript".into(),
                        font_alias: Some("MuCRT".into()),
                        wrap: true,
                        stream_id: Some("ab-gate-logs".into()),
                        entries,
                        controls: vec![],
                        pagination: None,
                    }),
                ),
            );
        }
        AbDeploySubView::ActiveLeases => {
            let rows: Vec<TableRow> = view
                .map(|v| {
                    v.leases
                        .iter()
                        .map(|l| TableRow {
                            id: format!("lease-{}", l.lease_id),
                            tone: if l.promoted {
                                SemanticTone::Pass
                            } else {
                                SemanticTone::Info
                            },
                            cells: vec![
                                TableCell {
                                    text: l.service.clone(),
                                    tone: SemanticTone::Info,
                                    signal: None,
                                    mono: true,
                                    action: None,
                                },
                                TableCell {
                                    text: format!("{:?} -> {:?}", l.active_slot, l.candidate_slot),
                                    tone: SemanticTone::Neutral,
                                    signal: None,
                                    mono: true,
                                    action: None,
                                },
                                TableCell {
                                    text: format!("{}s", l.ttl_remaining_ms / 1000),
                                    tone: SemanticTone::Neutral,
                                    signal: None,
                                    mono: true,
                                    action: None,
                                },
                            ],
                            row_action: Some(
                                ActionControl::new(
                                    format!("ab-rollback:{}", l.deployment_id),
                                    "Rollback",
                                    format!("ab-rollback:{}", l.deployment_id),
                                )
                                .disabled(!can_rollback)
                                .with_tone(SemanticTone::Fail),
                            ),
                        })
                        .collect()
                })
                .unwrap_or_default();
            scene.sections.push(
                SceneSection::new("sec-ab-leases", "Active TTL Slot Leases").with_component(
                    SceneComponent::TableGrid(TableGrid {
                        id: "ab-leases-table".into(),
                        title: "Active Slot Leases".into(),
                        columns: vec![
                            TableColumn {
                                key: "svc".into(),
                                label: "Service".into(),
                                numeric: false,
                            },
                            TableColumn {
                                key: "slots".into(),
                                label: "Slots".into(),
                                numeric: false,
                            },
                            TableColumn {
                                key: "ttl".into(),
                                label: "TTL".into(),
                                numeric: true,
                            },
                        ],
                        rows,
                        empty_message: Some("No active slot leases".into()),
                        pagination: None,
                    }),
                ),
            );
        }
    }
}

fn build_repo_sections(state: &ConsoleState, subview: RepositorySubView, scene: &mut ConsoleScene) {
    let view = state.client.view();
    let mut sec = SceneSection::new(
        "sec-repo",
        format!("Registry · {}", SubView::Repository(subview).label()),
    );
    match subview {
        RepositorySubView::Packages => {
            let filter_cells = vec![
                MatrixCell {
                    id: "reg-eco-all".into(),
                    label: "Ecosystem".into(),
                    value: "All".into(),
                    detail: None,
                    tone: if state.reg_ecosystem_filter.is_none() {
                        SemanticTone::Info
                    } else {
                        SemanticTone::Neutral
                    },
                    signal: None,
                    action: Some(
                        ActionControl::new("reg-eco:all", "All", "reg-eco:all")
                            .with_selected(state.reg_ecosystem_filter.is_none())
                            .with_attr("data-ecosystem", "all"),
                    ),
                },
                MatrixCell {
                    id: "reg-eco-cargo".into(),
                    label: "Ecosystem".into(),
                    value: "Cargo".into(),
                    detail: None,
                    tone: if state.reg_ecosystem_filter == Some(PackageEcosystem::Cargo) {
                        SemanticTone::Info
                    } else {
                        SemanticTone::Neutral
                    },
                    signal: None,
                    action: Some(
                        ActionControl::new("reg-eco:cargo", "Cargo", "reg-eco:cargo")
                            .with_selected(
                                state.reg_ecosystem_filter == Some(PackageEcosystem::Cargo),
                            )
                            .with_attr("data-ecosystem", "cargo"),
                    ),
                },
                MatrixCell {
                    id: "reg-eco-nuget".into(),
                    label: "Ecosystem".into(),
                    value: "NuGet".into(),
                    detail: None,
                    tone: if state.reg_ecosystem_filter == Some(PackageEcosystem::NuGet) {
                        SemanticTone::Info
                    } else {
                        SemanticTone::Neutral
                    },
                    signal: None,
                    action: Some(
                        ActionControl::new("reg-eco:nuget", "NuGet", "reg-eco:nuget")
                            .with_selected(
                                state.reg_ecosystem_filter == Some(PackageEcosystem::NuGet),
                            )
                            .with_attr("data-ecosystem", "nuget"),
                    ),
                },
                MatrixCell {
                    id: "reg-eco-maven".into(),
                    label: "Ecosystem".into(),
                    value: "Maven".into(),
                    detail: None,
                    tone: if state.reg_ecosystem_filter == Some(PackageEcosystem::Maven) {
                        SemanticTone::Info
                    } else {
                        SemanticTone::Neutral
                    },
                    signal: None,
                    action: Some(
                        ActionControl::new("reg-eco:maven", "Maven", "reg-eco:maven")
                            .with_selected(
                                state.reg_ecosystem_filter == Some(PackageEcosystem::Maven),
                            )
                            .with_attr("data-ecosystem", "maven"),
                    ),
                },
            ];
            sec = sec.with_component(SceneComponent::StatusMatrix(StatusMatrix {
                id: "reg-ecosystem-filter".into(),
                title: "Package Ecosystem Filter".into(),
                cells: filter_cells,
            }));

            let pkg_rows: Vec<TableRow> = view
                .map(|v| {
                    v.registry
                        .packages
                        .iter()
                        .filter(|p| {
                            state
                                .reg_ecosystem_filter
                                .is_none_or(|eco| p.ecosystem == eco)
                        })
                        .filter(|p| {
                            let hay =
                                format!("{} {} {}", p.name, p.ecosystem.label(), p.latest_version);
                            crate::state::service_filter_matches(&state.filter_query, &hay)
                        })
                        .map(|p| {
                            let (status_lbl, tone) = if p.tainted {
                                ("Tainted", SemanticTone::Fail)
                            } else if p.yanked {
                                ("Yanked", SemanticTone::Warn)
                            } else {
                                ("Active", SemanticTone::Pass)
                            };
                            TableRow {
                                id: format!(
                                    "pkg-{}-{}",
                                    p.ecosystem.label().to_ascii_lowercase(),
                                    p.name
                                ),
                                tone,
                                cells: vec![
                                    TableCell {
                                        text: p.name.clone(),
                                        tone: SemanticTone::Info,
                                        signal: Some(tone_signal(tone)),
                                        mono: true,
                                        action: None,
                                    },
                                    TableCell {
                                        text: p.ecosystem.label().to_owned(),
                                        tone: SemanticTone::Neutral,
                                        signal: None,
                                        mono: true,
                                        action: None,
                                    },
                                    TableCell {
                                        text: p.latest_version.clone(),
                                        tone: SemanticTone::Neutral,
                                        signal: None,
                                        mono: true,
                                        action: None,
                                    },
                                    TableCell {
                                        text: format!(
                                            "{status_lbl} · {} deps · {}",
                                            p.deps_count, p.sha256_prefix
                                        ),
                                        tone,
                                        signal: None,
                                        mono: true,
                                        action: None,
                                    },
                                ],
                                row_action: None,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();

            sec = sec.with_component(SceneComponent::TableGrid(TableGrid {
                id: "reg-packages-table".into(),
                title: "Registry Packages".into(),
                columns: vec![
                    TableColumn {
                        key: "name".into(),
                        label: "Package".into(),
                        numeric: false,
                    },
                    TableColumn {
                        key: "eco".into(),
                        label: "Ecosystem".into(),
                        numeric: false,
                    },
                    TableColumn {
                        key: "ver".into(),
                        label: "Latest".into(),
                        numeric: false,
                    },
                    TableColumn {
                        key: "status".into(),
                        label: "Status".into(),
                        numeric: false,
                    },
                ],
                rows: pkg_rows,
                empty_message: Some("No matching packages".into()),
                pagination: None,
            }));

            let rows: Vec<TableRow> = state
                .filtered_repos()
                .iter()
                .map(|r| TableRow {
                    id: format!("repo-{}-{}", r.id.owner, r.id.repo),
                    tone: SemanticTone::Info,
                    cells: vec![
                        TableCell {
                            text: format!("{}/{}", r.id.owner, r.id.repo),
                            tone: SemanticTone::Info,
                            signal: None,
                            mono: true,
                            action: None,
                        },
                        TableCell {
                            text: if r.visibility == Visibility::Public {
                                "Public".into()
                            } else {
                                "Private".into()
                            },
                            tone: if r.visibility == Visibility::Public {
                                SemanticTone::Pass
                            } else {
                                SemanticTone::Warn
                            },
                            signal: None,
                            mono: false,
                            action: None,
                        },
                        TableCell {
                            text: r.default_branch.clone(),
                            tone: SemanticTone::Neutral,
                            signal: None,
                            mono: true,
                            action: None,
                        },
                    ],
                    row_action: None,
                })
                .collect();
            sec = sec.with_component(SceneComponent::TableGrid(TableGrid {
                id: "repo-packages-table".into(),
                title: "Repositories & Registry Packages".into(),
                columns: vec![
                    TableColumn {
                        key: "repo".into(),
                        label: "Repository".into(),
                        numeric: false,
                    },
                    TableColumn {
                        key: "vis".into(),
                        label: "Visibility".into(),
                        numeric: false,
                    },
                    TableColumn {
                        key: "branch".into(),
                        label: "Branch".into(),
                        numeric: false,
                    },
                ],
                rows,
                empty_message: Some("No matching repositories".into()),
                pagination: None,
            }));
        }
        RepositorySubView::CanaryTaints => {
            let rows: Vec<TableRow> = view
                .map(|v| {
                    v.registry
                        .canaries
                        .iter()
                        .map(|c| {
                            let tone = match c.state {
                                CanaryBundleState::Promoted => SemanticTone::Pass,
                                CanaryBundleState::Tainted => SemanticTone::Fail,
                                _ => SemanticTone::Info,
                            };
                            TableRow {
                                id: format!("cny-{}", c.bundle_id),
                                tone,
                                cells: vec![
                                    TableCell {
                                        text: c.bundle_id.clone(),
                                        tone,
                                        signal: Some(tone_signal(tone)),
                                        mono: true,
                                        action: None,
                                    },
                                    TableCell {
                                        text: c.service.clone(),
                                        tone: SemanticTone::Neutral,
                                        signal: None,
                                        mono: true,
                                        action: None,
                                    },
                                    TableCell {
                                        text: c.state.label().into(),
                                        tone,
                                        signal: None,
                                        mono: false,
                                        action: None,
                                    },
                                ],
                                row_action: None,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            sec = sec.with_component(SceneComponent::TableGrid(TableGrid {
                id: "repo-canaries-table".into(),
                title: "Canary Bundles & Taints".into(),
                columns: vec![
                    TableColumn {
                        key: "bundle".into(),
                        label: "Bundle".into(),
                        numeric: false,
                    },
                    TableColumn {
                        key: "svc".into(),
                        label: "Service".into(),
                        numeric: false,
                    },
                    TableColumn {
                        key: "state".into(),
                        label: "State".into(),
                        numeric: false,
                    },
                ],
                rows,
                empty_message: Some("No canary bundles".into()),
                pagination: None,
            }));
        }
        RepositorySubView::CasHealth => {
            let cas = view.map(|v| &v.registry.cas);
            sec = sec.with_component(SceneComponent::KeyValueList(KeyValueList {
                id: "repo-cas-kv".into(),
                title: "CAS Storage Health".into(),
                entries: vec![
                    KeyValueEntry {
                        key: "Blobs".into(),
                        value: cas.map_or(0, |c| c.blob_count).to_string(),
                        detail: None,
                        tone: SemanticTone::Info,
                        mono: true,
                        action: None,
                    },
                    KeyValueEntry {
                        key: "Corrupt Blobs".into(),
                        value: cas.map_or(0, |c| c.corrupt_blobs).to_string(),
                        detail: None,
                        tone: if cas.is_some_and(|c| c.corrupt_blobs > 0) {
                            SemanticTone::Fail
                        } else {
                            SemanticTone::Pass
                        },
                        mono: true,
                        action: None,
                    },
                ],
            }));
        }
    }
    scene.sections.push(sec);
}

fn build_observe_sections(
    state: &ConsoleState,
    subview: ObservabilitySubView,
    scene: &mut ConsoleScene,
) {
    let view = state.client.view();
    let mut sec = SceneSection::new(
        "sec-observe",
        format!(
            "Observability · {}",
            SubView::Observability(subview).label()
        ),
    );
    match subview {
        ObservabilitySubView::Metrics => {
            if let Some(v) = view {
                for (idx, m) in v.observe.metrics.iter().enumerate() {
                    let tone = if m.error_rate_bp > 100 {
                        SemanticTone::Fail
                    } else {
                        SemanticTone::Pass
                    };
                    sec = sec.with_component(SceneComponent::SparklineCard(SparklineCard {
                        id: format!("obs-metric:{idx}"),
                        title: format!("{} ({})", m.service, m.hardware_class),
                        metric_value: format!("p95 {}ms", m.p95_ms),
                        unit: None,
                        delta_label: Some(format!("{} rps", m.rps)),
                        tone,
                        samples: m.samples.iter().map(|&x| f64::from(x)).collect(),
                        range_label: None,
                        action: Some(
                            ActionControl::new(
                                format!("obs-metric:{idx}"),
                                "Trace",
                                format!("obs-metric:{idx}"),
                            )
                            .with_attr("data-trace-id", &m.exemplar_trace_id),
                        ),
                    }));
                }
            }
        }
        ObservabilitySubView::Logs => {
            let entries: Vec<LogEntry> = view
                .map(|v| {
                    v.observe
                        .logs
                        .iter()
                        .enumerate()
                        .map(|(i, l)| LogEntry {
                            line_no: u32::try_from(i + 1).ok(),
                            timestamp: Some(l.timestamp_ms.to_string()),
                            level: SemanticTone::Info,
                            signal: None,
                            text: format!("[{}] {}", l.service, l.message),
                            trace_id: Some(l.trace_id.clone()),
                            action: None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            sec = sec.with_component(SceneComponent::LogViewer(LogViewer {
                id: "obs-logs-viewer".into(),
                title: "Distributed OpenTelemetry Logs".into(),
                font_alias: Some("MuCRT".into()),
                wrap: true,
                stream_id: Some("otel-logs".into()),
                entries,
                controls: vec![],
                pagination: None,
            }));
        }
        ObservabilitySubView::PurePath => {
            let rows: Vec<TableRow> = view
                .map(|v| {
                    v.observe
                        .traces
                        .iter()
                        .map(|t| TableRow {
                            id: format!("trace-{}", t.trace_id),
                            tone: if t.error {
                                SemanticTone::Fail
                            } else {
                                SemanticTone::Pass
                            },
                            cells: vec![
                                TableCell {
                                    text: t.trace_id.clone(),
                                    tone: SemanticTone::Info,
                                    signal: None,
                                    mono: true,
                                    action: None,
                                },
                                TableCell {
                                    text: t.root_service.clone(),
                                    tone: SemanticTone::Neutral,
                                    signal: None,
                                    mono: true,
                                    action: None,
                                },
                                TableCell {
                                    text: format!("{}ms", t.duration_ms),
                                    tone: SemanticTone::Neutral,
                                    signal: None,
                                    mono: true,
                                    action: None,
                                },
                            ],
                            row_action: None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            sec = sec.with_component(SceneComponent::TableGrid(TableGrid {
                id: "obs-traces-table".into(),
                title: "W3C Trace Waterfalls".into(),
                columns: vec![
                    TableColumn {
                        key: "id".into(),
                        label: "Trace ID".into(),
                        numeric: false,
                    },
                    TableColumn {
                        key: "svc".into(),
                        label: "Root Service".into(),
                        numeric: false,
                    },
                    TableColumn {
                        key: "dur".into(),
                        label: "Duration".into(),
                        numeric: true,
                    },
                ],
                rows,
                empty_message: Some("No traces captured".into()),
                pagination: None,
            }));
        }
        ObservabilitySubView::ProblemsSlos => {
            let rows: Vec<TableRow> = view
                .map(|v| {
                    v.observe
                        .problems
                        .iter()
                        .map(|p| TableRow {
                            id: format!("prob-{}", p.problem_id),
                            tone: if p.active {
                                SemanticTone::Fail
                            } else {
                                SemanticTone::Pass
                            },
                            cells: vec![
                                TableCell {
                                    text: p.title.clone(),
                                    tone: SemanticTone::Warn,
                                    signal: None,
                                    mono: false,
                                    action: None,
                                },
                                TableCell {
                                    text: p.impacted_entity.clone(),
                                    tone: SemanticTone::Info,
                                    signal: None,
                                    mono: true,
                                    action: None,
                                },
                            ],
                            row_action: None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            sec = sec.with_component(SceneComponent::TableGrid(TableGrid {
                id: "obs-problems-table".into(),
                title: "Active Problems & SLO Burn".into(),
                columns: vec![
                    TableColumn {
                        key: "title".into(),
                        label: "Problem".into(),
                        numeric: false,
                    },
                    TableColumn {
                        key: "entity".into(),
                        label: "Impacted Entity".into(),
                        numeric: false,
                    },
                ],
                rows,
                empty_message: Some("Zero open SLO alerts".into()),
                pagination: None,
            }));
        }
        ObservabilitySubView::Smartscape => {
            let rows: Vec<TableRow> = view
                .map(|v| {
                    v.observe
                        .smartscape_nodes
                        .iter()
                        .map(|n| TableRow {
                            id: n.id.clone(),
                            tone: if n.healthy {
                                SemanticTone::Pass
                            } else {
                                SemanticTone::Fail
                            },
                            cells: vec![
                                TableCell {
                                    text: n.label.clone(),
                                    tone: SemanticTone::Info,
                                    signal: None,
                                    mono: true,
                                    action: None,
                                },
                                TableCell {
                                    text: n.layer.label().into(),
                                    tone: SemanticTone::Neutral,
                                    signal: None,
                                    mono: false,
                                    action: None,
                                },
                                TableCell {
                                    text: format!("{}ms p95", n.p95_ms),
                                    tone: SemanticTone::Neutral,
                                    signal: None,
                                    mono: true,
                                    action: None,
                                },
                            ],
                            row_action: None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            sec = sec.with_component(SceneComponent::TableGrid(TableGrid {
                id: "obs-smartscape-table".into(),
                title: "Smartscape Topology".into(),
                columns: vec![
                    TableColumn {
                        key: "node".into(),
                        label: "Entity".into(),
                        numeric: false,
                    },
                    TableColumn {
                        key: "layer".into(),
                        label: "Layer".into(),
                        numeric: false,
                    },
                    TableColumn {
                        key: "p95".into(),
                        label: "Latency".into(),
                        numeric: true,
                    },
                ],
                rows,
                empty_message: Some("No topology nodes".into()),
                pagination: None,
            }));
        }
        ObservabilitySubView::DataStore => {
            let ds = view.map(|v| &v.observe.store);
            sec = sec.with_component(SceneComponent::KeyValueList(KeyValueList {
                id: "obs-datastore-kv".into(),
                title: "DuckDB & Parquet Store Health".into(),
                entries: vec![KeyValueEntry {
                    key: "Hot DuckDB Rows".into(),
                    value: ds.map_or(0, |s| s.hot_duckdb_rows).to_string(),
                    detail: None,
                    tone: SemanticTone::Pass,
                    mono: true,
                    action: None,
                }],
            }));
        }
        ObservabilitySubView::Ingress => {
            sec = sec.with_component(SceneComponent::KeyValueList(KeyValueList {
                id: "obs-ingress-kv".into(),
                title: "Ingress & TLS Telemetry".into(),
                entries: vec![KeyValueEntry {
                    key: "Routes".into(),
                    value: view
                        .map_or(0, |v| v.observe.ingress_routes.len())
                        .to_string(),
                    detail: None,
                    tone: SemanticTone::Info,
                    mono: true,
                    action: None,
                }],
            }));
        }
    }
    scene.sections.push(sec);
}

fn build_model_runner_sections(
    state: &ConsoleState,
    subview: ModelRunnerSubView,
    scene: &mut ConsoleScene,
) {
    // Strictly read-only area (#56): zero mutation controls.
    let view = state.client.view();
    let mr = view.map(|v| &v.model_runner);
    let mut sec = SceneSection::new(
        "sec-model-runner",
        format!("Model Runner · {}", SubView::ModelRunner(subview).label()),
    );

    match subview {
        ModelRunnerSubView::RuntimeQueue => {
            let rows: Vec<TableRow> = mr
                .and_then(|m| m.status.as_ref())
                .map(|s| {
                    s.models
                        .iter()
                        .map(|r| TableRow {
                            id: format!("mr-rt-{}", r.model_id),
                            tone: if r.is_alive {
                                SemanticTone::Pass
                            } else {
                                SemanticTone::Fail
                            },
                            cells: vec![
                                TableCell {
                                    text: r.model_id.clone(),
                                    tone: SemanticTone::Info,
                                    signal: None,
                                    mono: true,
                                    action: None,
                                },
                                TableCell {
                                    text: r.cluster.clone(),
                                    tone: SemanticTone::Neutral,
                                    signal: None,
                                    mono: true,
                                    action: None,
                                },
                                TableCell {
                                    text: format!("{} MiB", r.mlock_bytes / (1024 * 1024)),
                                    tone: SemanticTone::Pass,
                                    signal: None,
                                    mono: true,
                                    action: None,
                                },
                            ],
                            row_action: None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            sec = sec.with_component(SceneComponent::TableGrid(TableGrid {
                id: "mr-runtime-table".into(),
                title: "Resident Models & Bounded Memlock Queue (Read-Only)".into(),
                columns: vec![
                    TableColumn {
                        key: "model".into(),
                        label: "Model".into(),
                        numeric: false,
                    },
                    TableColumn {
                        key: "cluster".into(),
                        label: "Cluster".into(),
                        numeric: false,
                    },
                    TableColumn {
                        key: "mem".into(),
                        label: "Memlock".into(),
                        numeric: true,
                    },
                ],
                rows,
                empty_message: Some("No active model runtimes loaded".into()),
                pagination: None,
            }));
        }
        ModelRunnerSubView::ConnectionVpn => {
            let rows: Vec<TableRow> = mr
                .and_then(|m| m.connection.as_ref())
                .map(|c| {
                    c.peers
                        .iter()
                        .map(|p| TableRow {
                            id: format!("mr-vpn-{}", p.public_key_prefix),
                            tone: if p.latest_handshake_epoch_ms.is_some() {
                                SemanticTone::Pass
                            } else {
                                SemanticTone::Warn
                            },
                            cells: vec![
                                TableCell {
                                    text: p.public_key_prefix.clone(),
                                    tone: SemanticTone::Info,
                                    signal: Some(if p.latest_handshake_epoch_ms.is_some() {
                                        SignalGlyph::BlockPassed
                                    } else {
                                        SignalGlyph::BlockPending
                                    }),
                                    mono: true,
                                    action: None,
                                },
                                TableCell {
                                    text: p.allowed_ips.clone(),
                                    tone: SemanticTone::Neutral,
                                    signal: None,
                                    mono: true,
                                    action: None,
                                },
                            ],
                            row_action: None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            sec = sec.with_component(SceneComponent::TableGrid(TableGrid {
                id: "mr-vpn-table".into(),
                title: "Overlay Mesh & Connection State (Read-Only)".into(),
                columns: vec![
                    TableColumn {
                        key: "peer".into(),
                        label: "Peer".into(),
                        numeric: false,
                    },
                    TableColumn {
                        key: "ips".into(),
                        label: "Allowed IPs".into(),
                        numeric: false,
                    },
                ],
                rows,
                empty_message: Some("No overlay peers reported".into()),
                pagination: None,
            }));
        }
    }
    scene.sections.push(sec);
}

fn build_platform_sections(
    state: &ConsoleState,
    auth: &AuthState,
    subview: PlatformSubView,
    scene: &mut ConsoleScene,
) {
    let can_maint = auth.has_policy("platform:maintenance.write");
    let can_users = auth.has_policy("platform:users.write");
    let can_invites = auth.has_policy("platform:invites.write");
    let can_roles = auth.has_policy("platform:roles.write");
    let view = state.client.view();
    let max_hdr = view.map_or(
        mu_console_protocol::DEFAULT_MAX_ACTIVE_ROLES_HEADER_CHARS,
        |v| v.max_active_roles_header_chars,
    );

    match subview {
        PlatformSubView::Maintenance => {
            scene.sections.push(
                SceneSection::new("sec-platform-maint", "Platform Maintenance").with_component(
                    SceneComponent::PlatformPanel(PlatformPanel {
                        id: "plat-maintenance-panel".into(),
                        title: "Infrastructure Maintenance & Timers".into(),
                        summary: Some(format!(
                            "Header role budget: {max_hdr} chars · Maintenance updates require confirmation.",
                        )),
                        tone: SemanticTone::Info,
                        attributes: vec![KeyValueEntry {
                            key: "Selected Subsystem".into(),
                            value: format!("{:?}", state.platform_form.selected_subsystem),
                            detail: Some(format!("{:?}", state.platform_form.maintenance_mode)),
                            tone: SemanticTone::Info,
                            mono: true,
                            action: None,
                        }],
                        controls: vec![
                            ActionControl::new(
                                "platform-maint-apply",
                                "Apply Maintenance Policy",
                                "platform-maint-apply",
                            )
                            .disabled(!can_maint)
                            .with_tone(SemanticTone::Warn),
                        ],
                    }),
                ),
            );
        }
        PlatformSubView::UsersAndInvites => {
            let users = view.map_or(&[][..], |v| v.platform.users.as_slice());
            let admin_count = users.iter().filter(|u| u.is_platform_admin).count();
            let rows: Vec<TableRow> = users
                .iter()
                .map(|u| {
                    let is_last_admin = u.is_platform_admin && admin_count <= 1;
                    TableRow {
                        id: format!("plat-user-{}", u.github_login),
                        tone: SemanticTone::Info,
                        cells: vec![
                            TableCell {
                                text: format!("@{}", u.github_login),
                                tone: SemanticTone::Info,
                                signal: None,
                                mono: true,
                                action: None,
                            },
                            TableCell {
                                text: u.bound_roles.join(", "),
                                tone: SemanticTone::Neutral,
                                signal: None,
                                mono: true,
                                action: None,
                            },
                            TableCell {
                                text: format!("{} key(s)", u.passkey_count),
                                tone: SemanticTone::Pass,
                                signal: None,
                                mono: true,
                                action: None,
                            },
                        ],
                        row_action: Some(
                            ActionControl::new(
                                format!("platform-user-revoke:{}", u.github_login),
                                if is_last_admin {
                                    "Last Admin"
                                } else {
                                    "Revoke"
                                },
                                format!("platform-user-revoke:{}", u.github_login),
                            )
                            .disabled(!can_users || is_last_admin)
                            .with_tone(SemanticTone::Fail),
                        ),
                    }
                })
                .collect();
            scene.sections.push(
                SceneSection::new("sec-platform-users", "Users & One-Time Invites")
                    .with_component(SceneComponent::PlatformPanel(PlatformPanel {
                        id: "plat-invite-creator".into(),
                        title: "Create One-Time Onboarding Invite".into(),
                        summary: Some(
                            "Invite URLs are minted once and displayed only in the confirmation modal."
                                .into(),
                        ),
                        tone: SemanticTone::Info,
                        attributes: vec![],
                        controls: vec![ActionControl::new(
                            "platform-invite-create",
                            "Create One-Time Invite",
                            "platform-invite-create",
                        )
                        .disabled(!can_invites)
                        .with_tone(SemanticTone::Info)],
                    }))
                    .with_component(SceneComponent::TableGrid(TableGrid {
                        id: "plat-users-table".into(),
                        title: "Enrolled Subjects & Last-Admin Guard".into(),
                        columns: vec![
                            TableColumn {
                                key: "handle".into(),
                                label: "Handle".into(),
                                numeric: false,
                            },
                            TableColumn {
                                key: "roles".into(),
                                label: "Roles".into(),
                                numeric: false,
                            },
                            TableColumn {
                                key: "keys".into(),
                                label: "Passkeys".into(),
                                numeric: true,
                            },
                        ],
                        rows,
                        empty_message: Some("No enrolled users".into()),
                        pagination: None,
                    })),
            );
        }
        PlatformSubView::RolesAndFolios => {
            let rows: Vec<TableRow> = view
                .map(|v| {
                    v.platform
                        .roles
                        .iter()
                        .map(|r| {
                            let folio_count = v
                                .platform
                                .role_folio_index
                                .role_folios
                                .get(&r.id)
                                .map_or(0, Vec::len);
                            TableRow {
                                id: format!("plat-role-{}", r.id),
                                tone: SemanticTone::Info,
                                cells: vec![
                                    TableCell {
                                        text: r.effective_display_name().to_owned(),
                                        tone: SemanticTone::Info,
                                        signal: None,
                                        mono: true,
                                        action: None,
                                    },
                                    TableCell {
                                        text: format!("{:?}", r.activation.kind),
                                        tone: SemanticTone::Neutral,
                                        signal: None,
                                        mono: true,
                                        action: None,
                                    },
                                    TableCell {
                                        text: format!("{folio_count} folio(s)"),
                                        tone: SemanticTone::Pass,
                                        signal: None,
                                        mono: true,
                                        action: None,
                                    },
                                ],
                                row_action: Some(
                                    ActionControl::new(
                                        format!("platform-role-delete:{}", r.id),
                                        "Delete Role",
                                        format!("platform-role-delete:{}", r.id),
                                    )
                                    .disabled(!can_roles)
                                    .with_tone(SemanticTone::Fail),
                                ),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            scene.sections.push(
                SceneSection::new("sec-platform-roles", "Roles & Policy Folios").with_component(
                    SceneComponent::TableGrid(TableGrid {
                        id: "plat-roles-table".into(),
                        title: "Server-Defined Roles & Folios".into(),
                        columns: vec![
                            TableColumn {
                                key: "role".into(),
                                label: "Role".into(),
                                numeric: false,
                            },
                            TableColumn {
                                key: "mode".into(),
                                label: "Lease Mode".into(),
                                numeric: false,
                            },
                            TableColumn {
                                key: "folios".into(),
                                label: "Folios".into(),
                                numeric: true,
                            },
                        ],
                        rows,
                        empty_message: Some("No roles defined".into()),
                        pagination: None,
                    }),
                ),
            );
        }
    }
}

fn selected_ci_job(state: &ConsoleState) -> Option<CiJobSummary> {
    let id = state.active_job_id()?;
    let jobs = state.visible_ci_jobs();
    let visible_ids: HashSet<u64> = jobs.iter().map(|j| j.job_id).collect();
    if visible_ids.contains(&id) {
        jobs.into_iter().find(|j| j.job_id == id)
    } else {
        jobs.into_iter().next()
    }
}

fn job_tone(j: &CiJobSummary) -> SemanticTone {
    if j.queue_state.eq_ignore_ascii_case("failed") {
        SemanticTone::Fail
    } else if j.queue_state.eq_ignore_ascii_case("passed") {
        SemanticTone::Pass
    } else {
        SemanticTone::Info
    }
}

fn tone_signal(tone: SemanticTone) -> SignalGlyph {
    match tone {
        SemanticTone::Pass => SignalGlyph::BlockPassed,
        SemanticTone::Fail => SignalGlyph::BlockFailed,
        SemanticTone::Warn => SignalGlyph::BlockFailed,
        SemanticTone::Info => SignalGlyph::TestRunning,
        SemanticTone::Neutral => SignalGlyph::BlockPending,
    }
}

fn stage_state_signal(st: StageState) -> SignalGlyph {
    match st {
        StageState::Passed => SignalGlyph::BlockPassed,
        StageState::Failed => SignalGlyph::BlockFailed,
        StageState::Running => SignalGlyph::BlockRunning,
        StageState::Pending | StageState::Skipped => SignalGlyph::BlockPending,
    }
}

fn stage_state_tone(st: StageState) -> SemanticTone {
    match st {
        StageState::Passed => SemanticTone::Pass,
        StageState::Failed => SemanticTone::Fail,
        StageState::Running => SemanticTone::Info,
        StageState::Pending | StageState::Skipped => SemanticTone::Neutral,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mu_console_protocol::{
        encode, CiJobSummary, DataEvent, FabricNode, ProblemCategory, RepoId, SloProblem, ViewModel,
    };

    #[test]
    fn attention_obs_status_label_maps_strictly_to_allowed_labels() {
        let base = SloProblem {
            problem_id: "PROB-LAT-1".into(),
            title: "Latency regression".into(),
            category: ProblemCategory::LatencyRegression,
            severity: "critical".into(),
            impacted_entity: "mu-ingress".into(),
            active: true,
            ..SloProblem::default()
        };
        assert_eq!(attention_obs_status_label(&base), "Degraded");

        let mut error_prob = base.clone();
        error_prob.category = ProblemCategory::ErrorRateSpike;
        error_prob.severity = "error".into();
        assert_eq!(attention_obs_status_label(&error_prob), "Degraded");

        let mut flaky_prob = base.clone();
        flaky_prob.category = ProblemCategory::FlakyTest;
        flaky_prob.severity = "warning".into();
        assert_eq!(attention_obs_status_label(&flaky_prob), "Degraded");

        let mut canary_prob = base.clone();
        canary_prob.category = ProblemCategory::CanaryTaint;
        assert_eq!(attention_obs_status_label(&canary_prob), "Down");

        let mut crash_prob = base.clone();
        crash_prob.title = "Worker Crash loop".into();
        assert_eq!(attention_obs_status_label(&crash_prob), "Down");

        let mut offline_prob = base.clone();
        offline_prob.summary = "Upstream target offline".into();
        assert_eq!(attention_obs_status_label(&offline_prob), "Down");

        let mut down_prob = base.clone();
        down_prob.problem_id = "PROB-DOWN-svc".into();
        assert_eq!(attention_obs_status_label(&down_prob), "Down");

        let mut hb_prob = base.clone();
        hb_prob.problem_id = "PROB-HEARTBEAT-node-a".into();
        hb_prob.title = "Node Heartbeat Staleness on node-a".into();
        assert_eq!(attention_obs_status_label(&hb_prob), "Missing");

        let mut missing_prob = base;
        missing_prob.category = ProblemCategory::CanaryTaint;
        missing_prob.summary = "Missing span heartbeat".into();
        assert_eq!(attention_obs_status_label(&missing_prob), "Missing");
    }

    #[test]
    fn dash_attention_table_places_entity_in_target_and_canonical_labels_in_status() {
        let mut st = ConsoleState::new();
        st.init_from_env("#/dashboard/overview", "", None);
        let vm = ViewModel {
            auth: AuthState {
                status: AuthStatus::Active,
                github_login: Some("ops".into()),
                active_policies: vec![
                    "system:dashboard.read".into(),
                    "ci:queue.read".into(),
                    "ci:jobs.read".into(),
                    "ci:nodes.read".into(),
                    "obs:problems.read".into(),
                ],
                ..AuthState::default()
            },
            ci_jobs: vec![CiJobSummary {
                job_id: 42,
                repo: RepoId::new("LMA1980", "mu-console-ui"),
                ref_name: "main".into(),
                queue_state: "failed".into(),
                tests_passed: 3,
                tests_total: 4,
                ..CiJobSummary::default()
            }],
            nodes: vec![FabricNode {
                id: "worker-riscv64".into(),
                online: false,
                arch: "riscv64".into(),
                max_slots: 1,
                ..FabricNode::default()
            }],
            observe: mu_console_protocol::ObserveView {
                problems: vec![
                    SloProblem {
                        problem_id: "PROB-LAT-1".into(),
                        title: "Gateway latency spike".into(),
                        category: ProblemCategory::LatencyRegression,
                        severity: "critical".into(),
                        impacted_entity: "mu-ingress".into(),
                        active: true,
                        ..SloProblem::default()
                    },
                    SloProblem {
                        problem_id: "PROB-CANARY-1".into(),
                        title: "Canary Taint / Crash Telemetry".into(),
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
                        severity: "error".into(),
                        impacted_entity: "node-a".into(),
                        active: true,
                        ..SloProblem::default()
                    },
                ],
                ..mu_console_protocol::ObserveView::default()
            },
            ..ViewModel::default()
        };
        let frame = encode(&DataEvent::Snapshot(vm)).expect("encode ViewModel");
        st.on_worker_frame(&frame);

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

        assert_eq!(grid.rows.len(), 5);
        for row in &grid.rows {
            let status = row.cells[3].text.as_str();
            assert!(
                matches!(status, "Degraded" | "Down" | "Missing" | "Failed"),
                "unexpected Status cell: {status:?}"
            );
            assert_ne!(status, row.cells[1].text.as_str());
        }

        let obs_lat = grid
            .rows
            .iter()
            .find(|r| r.id == "att-obs-PROB-LAT-1")
            .expect("latency row");
        assert_eq!(obs_lat.cells[1].text, "mu-ingress");
        assert_eq!(obs_lat.cells[3].text, "Degraded");

        let obs_canary = grid
            .rows
            .iter()
            .find(|r| r.id == "att-obs-PROB-CANARY-1")
            .expect("canary row");
        assert_eq!(obs_canary.cells[1].text, "mu-ci-controller");
        assert_eq!(obs_canary.cells[3].text, "Down");

        let obs_hb = grid
            .rows
            .iter()
            .find(|r| r.id == "att-obs-PROB-HEARTBEAT-node-a")
            .expect("heartbeat row");
        assert_eq!(obs_hb.cells[1].text, "node-a");
        assert_eq!(obs_hb.cells[3].text, "Missing");

        let fleet_row = grid
            .rows
            .iter()
            .find(|r| r.id == "att-fleet-worker-riscv64")
            .expect("fleet row");
        assert_eq!(fleet_row.cells[1].text, "worker-riscv64");
        assert_eq!(fleet_row.cells[3].text, "Down");

        let ci_row = grid
            .rows
            .iter()
            .find(|r| r.id == "att-ci-42")
            .expect("ci row");
        assert_eq!(ci_row.cells[3].text, "Failed");
    }
}
