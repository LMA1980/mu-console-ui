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
use crate::state::{service_filter_matches, ConsoleState, AREAS, WF_OBS_LOGS_PAGE_SIZE};
use mu_console_protocol::{
    AbDeploySubView, Area, AuthState, AuthStatus, CanaryBundleState, CdRunState, CdSubView,
    CiJobSummary, CiSubView, ConsoleSkin, CrtFont, DashboardSubView,
    DialogModel as ProtoDialogModel, GateOutcome, LogSeverity, ModelRunnerSubView,
    ObsLogWindowMeta, ObservabilitySubView, OtelLogEntry, PackageEcosystem, PlatformSubView,
    ProblemCategory, RepoStatus, RepositorySubView, RiskConfirmAction, RoleActivationKind,
    RoleLeaseView, SloProblem, Slot, SmartscapeLayer, StageState, StepUpRequirement, SubView,
    SubsystemHealthEntry, SubsystemId, SubsystemStatus, TestState, UiError, Visibility,
};
use mu_console_theme::scene::{
    self as theme_scene, ActionControl, ConsoleScene, GateChecklist, GateItem,
    InviteCreatedOnceDialog, KeyValueEntry, KeyValueList, LogEntry, LogViewer, MatrixCell, NavTab,
    Pagination, PasskeyEnrollmentDialog, PlatformPanel, RepoSelector, RiskConfirmDialog, RoleBadge,
    SceneBanner, SceneComponent, SceneSection, SceneViewport, SessionRoleOption,
    SessionRolesDialog, SparklineCard, StatusLamp, StatusMatrix, SubViewTab, TableCell,
    TableColumn, TableGrid, TableRow, TrainwayCheckpoint, TrainwayGraph, TrainwayTrack,
    UserProfileDialog,
};
use mu_console_theme::skin::{SemanticTone, SignalGlyph};
use std::collections::{BTreeMap, HashSet};

/// Build the complete [`ConsoleScene`] projection for `state` using `state.effective_skin()`.
#[must_use]
pub fn build_console_scene(state: &ConsoleState, viewport: SceneViewport) -> ConsoleScene {
    build_console_scene_for_skin(state.effective_skin(), state, viewport)
}

/// Build the complete [`ConsoleScene`] projection for `state` under an explicit [`ConsoleSkin`].
#[must_use]
pub fn build_console_scene_for_skin(
    skin: ConsoleSkin,
    state: &ConsoleState,
    _viewport: SceneViewport,
) -> ConsoleScene {
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
        scene.footer_status = Some(auth_required_title(&auth).into());
        return scene;
    }

    // 6. Sub-view strip & repo selector (hidden on public-CI-only sessions per #50).
    let is_public_anon_ci =
        router::is_public_ci_only_session(&auth) && state.route.area == Area::Ci;

    if !is_public_anon_ci {
        scene.subviews = build_subview_tabs(state, &auth);
        scene.repo_selector = build_repo_selector(state, &auth);
    }

    // 7. Area & sub-view projection across all 8 canonical areas & 27 sub-views.
    match state.route.subview {
        SubView::Dashboard(DashboardSubView::Overview) => {
            build_dashboard_sections(state, &auth, &mut scene);
        }
        SubView::Ci(ci) => build_ci_sections(skin, state, &auth, ci, is_public_anon_ci, &mut scene),
        SubView::Cd(cd) => build_cd_sections(state, &auth, cd, &mut scene),
        SubView::AbDeploy(ab) => build_ab_sections(state, &auth, ab, &mut scene),
        SubView::Repository(repo) => build_repo_sections(state, &auth, repo, &mut scene),
        SubView::Observability(obs) => build_observe_sections(state, obs, skin, &mut scene),
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
            let roles_plain = state.format_header_roles();
            scene.header_actions.push(
                ActionControl::new("hdr-role", roles_plain.clone(), "hdr-role")
                    .with_aria_label(roles_plain)
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
        Area::Ci => u32::try_from(unresolved_failed_ci_jobs(state).len())
            .ok()
            .filter(|&n| n > 0),
        Area::Observability => state
            .client
            .view()
            .and_then(|v| {
                u32::try_from(
                    v.observe
                        .problems
                        .iter()
                        .filter(|p| p.active && !is_suppressed_obs_problem(p))
                        .count(),
                )
                .ok()
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

fn build_repo_selector(state: &ConsoleState, auth: &AuthState) -> Option<RepoSelector> {
    if matches!(
        state.route.area,
        Area::Dashboard | Area::Observability | Area::ModelRunner | Area::Platform
    ) || matches!(
        state.route.subview,
        SubView::Ci(CiSubView::WorkerNodes | CiSubView::SandboxProfiles)
            | SubView::Repository(RepositorySubView::CasHealth)
            | SubView::AbDeploy(AbDeploySubView::ActiveLeases)
    ) || (!router::effective_has_policy(auth, "ci:repos.public.read")
        && !router::effective_has_policy(auth, "ci:repos.private.read"))
    {
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

const fn auth_required_title(auth: &AuthState) -> &'static str {
    match auth.status {
        AuthStatus::PasskeyEnrollmentRequired => copy::PASSKEY_ACCESS_REQUIRED_TITLE,
        AuthStatus::Active => copy::ACCESS_RESTRICTED_TITLE,
        AuthStatus::Unresolved | AuthStatus::Anonymous | AuthStatus::AuthorizationDenied => {
            copy::AUTH_REQUIRED_TITLE
        }
    }
}

fn build_auth_required_section(state: &ConsoleState, auth: &AuthState) -> SceneSection {
    let title = auth_required_title(auth);
    let summary = match auth.status {
        AuthStatus::Anonymous => copy::AUTH_REQUIRED_ANON_BODY,
        AuthStatus::AuthorizationDenied | AuthStatus::Unresolved => copy::AUTH_REQUIRED_BODY,
        AuthStatus::PasskeyEnrollmentRequired => copy::PASSKEY_ACCESS_REQUIRED_BODY,
        AuthStatus::Active => {
            if state.has_dormant_bound_roles() {
                copy::ACCESS_RESTRICTED_ELEVATE_BODY
            } else {
                copy::ACCESS_RESTRICTED_NO_ROLE_BODY
            }
        }
    };
    let mut attributes = vec![KeyValueEntry {
        key: "Requested Route".into(),
        value: router::format_hash(&state.route),
        detail: None,
        tone: SemanticTone::Neutral,
        mono: true,
        action: None,
    }];
    if auth.status == AuthStatus::Active {
        let roles_lbl = state.format_header_roles();
        let active_roles = if auth.active_roles.is_empty()
            || roles_lbl.is_empty()
            || roles_lbl == copy::ROLE_HEADER_EMPTY
        {
            "None".to_owned()
        } else {
            roles_lbl
        };
        attributes.push(KeyValueEntry {
            key: "Active Roles".into(),
            value: active_roles,
            detail: None,
            tone: SemanticTone::Neutral,
            mono: true,
            action: None,
        });
    }
    let controls = build_auth_required_controls(state, auth);
    SceneSection::new("sec-auth-required", title).with_component(SceneComponent::PlatformPanel(
        PlatformPanel {
            id: "panel-auth-required".into(),
            title: title.into(),
            summary: Some(summary.into()),
            tone: SemanticTone::Warn,
            attributes,
            controls,
        },
    ))
}

fn build_auth_required_controls(state: &ConsoleState, auth: &AuthState) -> Vec<ActionControl> {
    let mut controls = Vec::new();
    match auth.status {
        AuthStatus::Anonymous => {
            controls.push(
                ActionControl::new("auth-gate-signin", copy::SIGN_IN_LABEL, "auth-gate-signin")
                    .with_attr("data-href", state.sign_in_href())
                    .with_tone(SemanticTone::Info),
            );
        }
        AuthStatus::AuthorizationDenied => {
            controls.push(
                ActionControl::new("auth-gate-signin", copy::SIGN_OUT_LABEL, "auth-gate-signin")
                    .with_attr("data-href", state.sign_out_href())
                    .with_tone(SemanticTone::Warn),
            );
        }
        AuthStatus::PasskeyEnrollmentRequired => {
            controls.push(
                ActionControl::new(
                    "auth-gate-register-passkey",
                    copy::REGISTER_PASSKEY_BTN,
                    "auth-gate-register-passkey",
                )
                .with_tone(SemanticTone::Info),
            );
        }
        AuthStatus::Active => {
            if !auth.bound_roles.is_empty() || !auth.bound_role_specs.is_empty() {
                controls.push(
                    ActionControl::new("hdr-role", copy::SESSION_ROLES_TITLE, "hdr-role")
                        .with_tone(SemanticTone::Info),
                );
            }
            if state.can_access_area(Area::Dashboard) {
                controls.push(
                    ActionControl::new(
                        "auth-gate-dashboard",
                        "Return to Dashboard",
                        "auth-gate-dashboard",
                    )
                    .with_area("dashboard")
                    .with_tone(SemanticTone::Info),
                );
            }
            if !AREAS.iter().any(|&a| state.can_access_area(a)) {
                controls.push(
                    ActionControl::new(
                        "auth-gate-signout",
                        copy::SIGN_OUT_LABEL,
                        "auth-gate-signout",
                    )
                    .with_attr("data-href", state.sign_out_href())
                    .with_tone(SemanticTone::Warn),
                );
            }
        }
        AuthStatus::Unresolved => {}
    }
    controls
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

    let has_attention_source = state.can_access_area(Area::Ci)
        || state.can_access_area(Area::Observability)
        || auth.has_policy("ci:nodes.read");
    if is_active && has_attention_source {
        scene
            .sections
            .push(build_attention_section(state, attention_rows));
    } else if is_active && state.has_dormant_bound_roles() {
        scene.sections.push(build_dash_role_hint_section());
    }
}

fn build_dash_role_hint_section() -> SceneSection {
    SceneSection::new("sec-dash-role-hint", "Session Role Elevation").with_component(
        SceneComponent::KeyValueList(KeyValueList {
            id: "dash-role-hint-kv".into(),
            title: "Session Role Elevation".into(),
            entries: vec![KeyValueEntry {
                key: "Roles".into(),
                value: copy::DASH_ROLE_HINT_NOTE.into(),
                detail: None,
                tone: SemanticTone::Info,
                mono: false,
                action: Some(
                    ActionControl::new("hdr-role", copy::SESSION_ROLES_TITLE, "hdr-role")
                        .with_tone(SemanticTone::Info),
                ),
            }],
        }),
    )
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
        .filter_map(|&(area, sub_id, title)| {
            let permitted = state.can_access_area(area);
            let slug = area.slug();
            let entry: Option<&SubsystemHealthEntry> = health.and_then(|h| h.get(sub_id));
            let (status, tone) = subsystem_status_and_tone(state, area, entry);
            // Generic status word ONLY on unpermitted cards (#50, #55).
            let status_word = copy::subsystem_status_word(status);
            let detail = permitted.then(|| subsystem_permitted_detail(state, area, entry));
            let hay = match &detail {
                Some(d) => format!("{title} {slug} {status_word} {d}"),
                None => format!("{title} {slug} {status_word}"),
            };
            if !crate::state::service_filter_matches(&state.filter_query, &hay) {
                return None;
            }
            Some(MatrixCell {
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
            })
        })
        .collect()
}

fn dash_fleet_row(n: &mu_console_protocol::FabricNode) -> TableRow {
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
}

fn build_fleet_section(state: &ConsoleState) -> SceneSection {
    let rows: Vec<TableRow> = state
        .client
        .view()
        .map(|v| {
            v.nodes
                .iter()
                .filter(|n| {
                    let status_label = if n.online { "Online" } else { "Offline" };
                    let hay = format!("{} {} {status_label}", n.id, n.arch);
                    crate::state::service_filter_matches(&state.filter_query, &hay)
                })
                .map(dash_fleet_row)
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
            empty_message: Some(if state.filter_query.trim().is_empty() {
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
            empty_message: Some(if state.filter_query.trim().is_empty() {
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
    let mut rows = Vec::new();
    if state.can_access_area(Area::Ci) {
        append_ci_attention_rows(state, &mut rows);
    }
    if state.can_access_area(Area::Observability) {
        append_obs_attention_rows(state, &mut rows);
    }
    if auth.has_policy("ci:nodes.read") {
        append_fleet_attention_rows(state, &mut rows);
    }
    rows
}

fn is_test_problem(prob: &SloProblem) -> bool {
    matches!(prob.category, ProblemCategory::FlakyTest) || prob.problem_id.starts_with("PROB-TEST-")
}

fn is_suppressed_obs_problem(prob: &SloProblem) -> bool {
    prob.problem_id.starts_with("PROB-BENCH-")
        || (is_test_problem(prob) && prob.severity.eq_ignore_ascii_case("warning"))
}

fn is_non_runtime_daemon_problem(prob: &SloProblem) -> bool {
    is_test_problem(prob) || prob.problem_id.starts_with("PROB-BENCH-")
}

fn is_terminal_ci_state(queue_state: &str) -> bool {
    matches!(
        queue_state.trim().to_ascii_lowercase().as_str(),
        "passed" | "succeeded" | "completed" | "failed"
    )
}

fn unresolved_failed_ci_jobs(state: &ConsoleState) -> Vec<CiJobSummary> {
    let jobs = state.visible_ci_jobs();
    let mut latest_terminal: BTreeMap<(String, String, String), (usize, &CiJobSummary)> =
        BTreeMap::new();
    for (idx, job) in jobs.iter().enumerate() {
        if !is_terminal_ci_state(&job.queue_state) {
            continue;
        }
        let key = (
            job.repo.owner.to_ascii_lowercase(),
            job.repo.repo.to_ascii_lowercase(),
            job.ref_name.trim_start_matches("refs/heads/").to_owned(),
        );
        let replace = latest_terminal
            .get(&key)
            .is_none_or(|(_, cur)| job.job_id >= cur.job_id);
        if replace {
            latest_terminal.insert(key, (idx, job));
        }
    }
    let mut failed_indices: HashSet<usize> = latest_terminal
        .into_values()
        .filter(|(_, job)| job.queue_state.eq_ignore_ascii_case("failed"))
        .map(|(idx, _)| idx)
        .collect();
    jobs.into_iter()
        .enumerate()
        .filter_map(|(idx, job)| failed_indices.remove(&idx).then_some(job))
        .collect()
}

fn ci_job_failure_finding(job: &CiJobSummary) -> String {
    if let Some(st) = job.stages.iter().find(|s| s.state == StageState::Failed) {
        if st.exit_code != 0 {
            return format!("Stage '{}' failed (exit {})", st.id, st.exit_code);
        }
        return format!("Stage '{}' failed", st.id);
    }
    if job.tests_total > 0 && job.tests_passed < job.tests_total {
        format!("{}/{} tests passed", job.tests_passed, job.tests_total)
    } else if job.tests_total > 0 {
        format!(
            "CI job failed ({}/{} unit tests passed)",
            job.tests_passed, job.tests_total
        )
    } else {
        "CI job execution failed".into()
    }
}

fn append_ci_attention_rows(state: &ConsoleState, rows: &mut Vec<TableRow>) {
    for job in unresolved_failed_ci_jobs(state) {
        let target = format!(
            "{}/{} #{} ({})",
            job.repo.owner, job.repo.repo, job.job_id, job.ref_name
        );
        let finding = ci_job_failure_finding(&job);
        let hay = format!("CI Job {target} {finding} Failed");
        if !service_filter_matches(&state.filter_query, &hay) {
            continue;
        }
        rows.push(TableRow {
            id: format!("att-ci-{}", job.job_id),
            tone: SemanticTone::Fail,
            cells: vec![
                TableCell {
                    text: "CI Job".into(),
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

fn append_obs_attention_rows(state: &ConsoleState, rows: &mut Vec<TableRow>) {
    let Some(view) = state.client.view() else {
        return;
    };
    let obs_slug = Area::Observability.slug();
    for prob in &view.observe.problems {
        if !prob.active || is_suppressed_obs_problem(prob) {
            continue;
        }
        let target = if prob.impacted_entity.is_empty() {
            "Observability".into()
        } else {
            prob.impacted_entity.clone()
        };
        let test_prob = is_test_problem(prob);
        let scope = if test_prob { "CI Test" } else { "Application" };
        let status = if test_prob {
            "Failed"
        } else {
            copy::attention_obs_status_label(prob)
        };
        let hay = format!("{scope} {target} {} {status}", prob.title);
        if !service_filter_matches(&state.filter_query, &hay) {
            continue;
        }
        rows.push(TableRow {
            id: format!("att-obs-{}", prob.problem_id),
            tone: SemanticTone::Fail,
            cells: vec![
                TableCell {
                    text: scope.into(),
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

fn append_fleet_attention_rows(state: &ConsoleState, rows: &mut Vec<TableRow>) {
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
        let hay = format!("Fleet {} {finding} Down", node.id);
        if !service_filter_matches(&state.filter_query, &hay) {
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
        unresolved_failed_ci_jobs(state).len()
    } else {
        0
    };
    let obs = if state.can_access_area(Area::Observability) {
        state.client.view().map_or(0, |v| {
            v.observe
                .problems
                .iter()
                .filter(|p| p.active && !is_suppressed_obs_problem(p))
                .count()
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
        Area::Observability => state.client.view().is_some_and(|v| {
            v.observe
                .problems
                .iter()
                .any(|p| p.active && !is_non_runtime_daemon_problem(p))
        }),
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
                    || v.observe
                        .problems
                        .iter()
                        .any(|p| p.active && !is_suppressed_obs_problem(p))
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
            let active_problems = view
                .observe
                .problems
                .iter()
                .filter(|p| p.active && !is_suppressed_obs_problem(p))
                .count();
            if active_problems > 0 {
                let logs_summary = match view.observe.log_window.total_matched {
                    Some(total) => format!("{}/{total} logs", view.observe.logs.len()),
                    None => format!("{} logs", view.observe.logs.len()),
                };
                format!(
                    "{logs_summary} · {} traces · {} metrics · {active_problems} active SLO problem(s)",
                    view.observe.traces.len(),
                    view.observe.metrics.len(),
                )
            } else {
                let logs_summary = match view.observe.log_window.total_matched {
                    Some(total) => format!("{}/{total} logs", view.observe.logs.len()),
                    None => format!("{} logs", view.observe.logs.len()),
                };
                format!(
                    "{logs_summary} · {} traces · {} metrics",
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

const WF_SCORECARDS_PAGE_SIZE: usize = 8;

fn build_ci_sections(
    skin: ConsoleSkin,
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
    match subview {
        CiSubView::QueueLogs => unreachable!(),
        CiSubView::Scorecards => build_ci_scorecards(skin, state, scene),
        CiSubView::Trainway => build_ci_trainway(state, auth, scene),
        CiSubView::WorkerNodes => build_ci_worker_nodes(state, scene),
        CiSubView::SandboxProfiles => build_ci_sandboxes(state, scene),
    }
}

fn build_ci_scorecards(skin: ConsoleSkin, state: &ConsoleState, scene: &mut ConsoleScene) {
    let ordered = state.ordered_carousel_repos();
    let jobs = state.visible_ci_jobs();
    let active_repo = state.route.repo.as_ref().or_else(|| {
        state
            .client
            .view()
            .and_then(|v| v.selection.as_ref().map(|s| &s.repo))
    });
    let (start, limit) = match skin {
        ConsoleSkin::Html => (0_usize, ordered.len()),
        ConsoleSkin::Wireframe => {
            let total_pages = ordered.len().div_ceil(WF_SCORECARDS_PAGE_SIZE).max(1);
            let page = (state.carousel_page as usize).min(total_pages - 1);
            (page * WF_SCORECARDS_PAGE_SIZE, WF_SCORECARDS_PAGE_SIZE)
        }
    };
    let mut sec = SceneSection::new("sec-ci-scorecards", "CI Scorecards Carousel");
    for (idx, r) in ordered.iter().enumerate().skip(start).take(limit) {
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
        let is_selected = active_repo == Some(&r.id);
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
            action: Some(
                ActionControl::new(
                    format!("repo-select:{idx}"),
                    "Select",
                    format!("repo-select:{idx}"),
                )
                .with_pressed(is_selected),
            ),
        }));
    }
    if ordered.is_empty() && !state.filter_query.trim().is_empty() {
        sec = sec.with_component(SceneComponent::KeyValueList(KeyValueList {
            id: "ci-scorecards-empty".into(),
            title: "CI Scorecards Carousel".into(),
            entries: vec![KeyValueEntry {
                key: "Filter".into(),
                value: copy::NO_FILTER_MATCHES.into(),
                detail: None,
                tone: SemanticTone::Neutral,
                mono: false,
                action: None,
            }],
        }));
    }
    scene.sections.push(sec);
}

fn build_ci_trainway(state: &ConsoleState, auth: &AuthState, scene: &mut ConsoleScene) {
    let view = state.client.view();
    let topo = view.and_then(|v| v.topology.as_ref());
    let stations: Vec<String> = topo
        .map(|t| t.stations.iter().map(|s| s.label.clone()).collect())
        .unwrap_or_else(|| vec!["FMT".into(), "CLIPPY".into(), "TEST".into()]);
    let station_hay = stations.join(" ");
    let tracks: Vec<TrainwayTrack> = topo
        .map(|t| {
            t.trains
                .iter()
                .filter(|tr| {
                    let node_str = tr.node_id.as_deref().unwrap_or("");
                    let hay = format!("{} {} {station_hay}", tr.ref_name, node_str);
                    service_filter_matches(&state.filter_query, &hay)
                })
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
                summary: sel_job
                    .as_ref()
                    .map(|j| format!("{}/{} · {}", j.repo.owner, j.repo.repo, j.benchmark_summary)),
                tone: sel_job.as_ref().map_or(SemanticTone::Neutral, job_tone),
                attributes: vec![],
                controls,
            })),
    );
}

fn build_ci_worker_nodes(state: &ConsoleState, scene: &mut ConsoleScene) {
    let rows: Vec<TableRow> = state
        .client
        .view()
        .map(|v| {
            v.nodes
                .iter()
                .filter(|n| {
                    let status = if n.online { "online" } else { "offline" };
                    let hay = format!("{} {} {status}", n.id, n.arch);
                    service_filter_matches(&state.filter_query, &hay)
                })
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
                empty_message: Some(if state.filter_query.trim().is_empty() {
                    "No worker nodes reported".into()
                } else {
                    copy::NO_FILTER_MATCHES.into()
                }),
                pagination: None,
            }),
        ),
    );
}

fn build_ci_sandboxes(state: &ConsoleState, scene: &mut ConsoleScene) {
    let rows: Vec<TableRow> = state
        .client
        .view()
        .map(|v| {
            v.sandboxes
                .iter()
                .filter(|s| {
                    let hay = format!("{} {}", s.profile_name, s.backend);
                    service_filter_matches(&state.filter_query, &hay)
                })
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
        SceneSection::new("sec-ci-sandboxes", "MuIsolation Sandbox Profiles").with_component(
            SceneComponent::TableGrid(TableGrid {
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
                empty_message: Some(if state.filter_query.trim().is_empty() {
                    "No sandbox profiles available".into()
                } else {
                    copy::NO_FILTER_MATCHES.into()
                }),
                pagination: None,
            }),
        ),
    );
}

fn build_ci_queue_filter_bar(state: &ConsoleState) -> StatusMatrix {
    let cells = [
        (
            "ci-qf-all",
            "ci-queue-filter:all",
            "All",
            state.ci_queue_filter == 0,
            SemanticTone::Info,
        ),
        (
            "ci-qf-running",
            "ci-queue-filter:running",
            "Running",
            state.ci_queue_filter == 1,
            SemanticTone::Info,
        ),
        (
            "ci-qf-failed",
            "ci-queue-filter:failed",
            "Failed",
            state.ci_queue_filter == 2,
            SemanticTone::Fail,
        ),
    ]
    .into_iter()
    .map(
        |(cell_id, action_id, label, selected, active_tone)| MatrixCell {
            id: cell_id.into(),
            label: "Status".into(),
            value: label.into(),
            detail: None,
            tone: if selected {
                active_tone
            } else {
                SemanticTone::Neutral
            },
            signal: None,
            action: Some(ActionControl::new(action_id, label, action_id).with_selected(selected)),
        },
    )
    .collect();
    StatusMatrix {
        id: "ci-queue-filter-bar".into(),
        title: "Job Status Filter".into(),
        cells,
    }
}

fn build_ci_stage_checklist(
    job: &CiJobSummary,
    auth: &AuthState,
    is_public_anon: bool,
) -> GateChecklist {
    let can_view_nodes = auth.status == AuthStatus::Active && auth.has_policy("ci:nodes.read");
    let checklist_items: Vec<GateItem> = job
        .stages
        .iter()
        .map(|st| {
            let reason_text = st.skip_reason.as_ref().map(copy::skip_reason_label);
            let detail = if can_view_nodes {
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
            } else {
                // Never leak internal node IDs or thread counts without `ci:nodes.read` (#50).
                reason_text
                    .or_else(|| Some(format!("{}s · exit {}", st.duration_secs, st.exit_code)))
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

    GateChecklist {
        id: format!("ci-job-checklist-{}", job.job_id),
        title: format!(
            "Job #{} Stage Checklist ({}/{})",
            job.job_id, job.repo.owner, job.repo.repo
        ),
        environment: Some(job.ref_name.clone()),
        overall_tone: job_tone(job),
        items: checklist_items,
        actions,
    }
}

fn build_ci_log_component(
    state: &ConsoleState,
    auth: &AuthState,
    job: &CiJobSummary,
) -> Option<SceneComponent> {
    let can_view_logs = auth.status == AuthStatus::Active
        && (auth.has_policy("ci:jobs.logs.read") || auth.has_policy("obs:logs.read"));
    if can_view_logs {
        let filtered: Vec<(usize, &String)> = job
            .raw_log_lines
            .iter()
            .enumerate()
            .filter(|(_, line)| service_filter_matches(&state.filter_query, line))
            .collect();
        let use_lines: Vec<(usize, &String)> =
            if !state.filter_query.trim().is_empty() && !filtered.is_empty() {
                filtered
            } else {
                job.raw_log_lines.iter().enumerate().collect()
            };
        let entries: Vec<LogEntry> = use_lines
            .into_iter()
            .map(|(i, line)| LogEntry {
                line_no: u32::try_from(i + 1).ok(),
                timestamp: None,
                level: SemanticTone::Neutral,
                signal: None,
                text: line.clone(),
                trace_id: None,
                anchor_key: None,
                action: None,
            })
            .collect();
        return Some(SceneComponent::LogViewer(LogViewer {
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
    if auth.status == AuthStatus::Active {
        let role_action = state.has_dormant_bound_roles().then(|| {
            ActionControl::new("hdr-role", copy::SESSION_ROLES_TITLE, "hdr-role")
                .with_tone(SemanticTone::Info)
        });
        return Some(SceneComponent::KeyValueList(KeyValueList {
            id: "ci-job-logs-restricted".into(),
            title: format!("Job #{} Build Output", job.job_id),
            entries: vec![KeyValueEntry {
                key: "Logs".into(),
                value: copy::ACTIVE_CI_LOGS_RESTRICTED_NOTE.into(),
                detail: None,
                tone: SemanticTone::Warn,
                mono: false,
                action: role_action,
            }],
        }));
    }
    None
}

fn ci_queue_job_row(j: &CiJobSummary) -> TableRow {
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
}

fn build_ci_queue_logs(
    state: &ConsoleState,
    auth: &AuthState,
    is_public_anon: bool,
    scene: &mut ConsoleScene,
) {
    let jobs: Vec<CiJobSummary> = state
        .visible_ci_jobs()
        .into_iter()
        .filter(|j| match state.ci_queue_filter {
            1 => !is_terminal_ci_state(&j.queue_state),
            2 => j.queue_state.trim().eq_ignore_ascii_case("failed"),
            _ => true,
        })
        .collect();
    let job_rows: Vec<TableRow> = jobs.iter().map(ci_queue_job_row).collect();

    let sel = selected_ci_job(state)
        .filter(|s| jobs.iter().any(|j| j.job_id == s.job_id))
        .or_else(|| jobs.first().cloned());
    let filter_active = !state.filter_query.trim().is_empty() || state.ci_queue_filter != 0;
    let table_title = if is_public_anon {
        if auth.status == AuthStatus::PasskeyEnrollmentRequired {
            copy::PASSKEY_CI_STEPS_SUBTITLE
        } else {
            copy::PUBLIC_STEPS_SUBTITLE
        }
    } else {
        "Filtered CI Jobs"
    };
    let mut section = SceneSection::new(
        "sec-ci-queue-logs",
        if is_public_anon {
            copy::PUBLIC_STEPS_TITLE
        } else {
            "CI Queue & Job Steps"
        },
    )
    .with_component(SceneComponent::StatusMatrix(build_ci_queue_filter_bar(
        state,
    )))
    .with_component(SceneComponent::TableGrid(TableGrid {
        id: "ci-queue-table".into(),
        title: table_title.into(),
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
        empty_message: Some(if filter_active {
            copy::NO_FILTER_MATCHES.into()
        } else {
            "No matching CI jobs".into()
        }),
        pagination: None,
    }));

    if let Some(job) = sel {
        section = section.with_component(SceneComponent::GateChecklist(build_ci_stage_checklist(
            &job,
            auth,
            is_public_anon,
        )));
        if let Some(log_comp) = build_ci_log_component(state, auth, &job) {
            section = section.with_component(log_comp);
        }
    }

    scene.sections.push(section);
}

fn filter_empty_kv_entry() -> KeyValueEntry {
    KeyValueEntry {
        key: "Filter".into(),
        value: copy::NO_FILTER_MATCHES.into(),
        detail: None,
        tone: SemanticTone::Neutral,
        mono: false,
        action: None,
    }
}

fn retain_matching_kv(entries: &mut Vec<KeyValueEntry>, filter_query: &str) {
    entries.retain(|e| {
        let hay = format!(
            "{} {} {}",
            e.key,
            e.value,
            e.detail.as_deref().unwrap_or("")
        );
        service_filter_matches(filter_query, &hay)
    });
    if entries.is_empty() && !filter_query.trim().is_empty() {
        entries.push(filter_empty_kv_entry());
    }
}

fn build_cd_sections(
    state: &ConsoleState,
    _auth: &AuthState,
    subview: CdSubView,
    scene: &mut ConsoleScene,
) {
    // Strictly read-only CD (#56): Cargo / .NET / Maven pinpoint failure diagnosis + links.
    let mut sec = SceneSection::new(
        "sec-cd-readonly",
        format!("Continuous Delivery · {}", SubView::Cd(subview).label()),
    );
    sec = match subview {
        CdSubView::Runs => {
            sec.with_component(SceneComponent::PlatformPanel(build_cd_runs_panel(state)))
        }
        CdSubView::Pipelines => {
            sec.with_component(SceneComponent::TableGrid(build_cd_pipelines_table(state)))
        }
    };
    scene.sections.push(sec);
}

fn build_cd_runs_panel(state: &ConsoleState) -> PlatformPanel {
    let can_repo = state.can_access_area(Area::Repository);
    let can_ab = state.can_access_area(Area::AbDeploy);
    let mut entries = vec![KeyValueEntry {
        key: "Policy".into(),
        value: "Read-only diagnosis".into(),
        detail: Some("Promotions and rollbacks execute under A|B Deploy".into()),
        tone: SemanticTone::Neutral,
        mono: false,
        action: None,
    }];
    let mut matched = 0_usize;
    if let Some(v) = state.client.view() {
        for run in &v.cd_runs {
            let eco = copy::cd_ecosystem_label(&run.ecosystem);
            let fail_hay = run.failure.as_ref().map_or_else(String::new, |f| {
                format!(
                    "{} {} {} {}",
                    f.step_id,
                    f.package_name.as_deref().unwrap_or(""),
                    f.package_version.as_deref().unwrap_or(""),
                    f.reason
                )
            });
            let hay = format!(
                "{eco} {} {}/{} {} {:?} {fail_hay}",
                run.run_id, run.repo.owner, run.repo.repo, run.ref_name, run.state
            );
            if !service_filter_matches(&state.filter_query, &hay) {
                continue;
            }
            matched += 1;
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
    if matched == 0 && !state.filter_query.trim().is_empty() {
        entries.push(filter_empty_kv_entry());
    }
    PlatformPanel {
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
    }
}

fn build_cd_pipelines_table(state: &ConsoleState) -> TableGrid {
    let rows: Vec<TableRow> = state
        .client
        .view()
        .map(|v| {
            v.cd_runs
                .iter()
                .flat_map(|r| {
                    r.steps
                        .iter()
                        .filter(move |st| {
                            let hay = format!(
                                "{} {}/{} {} {} {:?}",
                                r.run_id, r.repo.owner, r.repo.repo, st.step_id, st.label, st.state
                            );
                            service_filter_matches(&state.filter_query, &hay)
                        })
                        .map(move |st| TableRow {
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
    TableGrid {
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
        empty_message: Some(if state.filter_query.trim().is_empty() {
            "No CD pipeline steps reported".into()
        } else {
            copy::NO_FILTER_MATCHES.into()
        }),
        pagination: None,
    }
}

fn build_ab_sections(
    state: &ConsoleState,
    auth: &AuthState,
    subview: AbDeploySubView,
    scene: &mut ConsoleScene,
) {
    let sec = match subview {
        AbDeploySubView::IntegrationGate => build_ab_gate_section(state, auth),
        AbDeploySubView::DeploymentsSlots => build_ab_slots_section(state, auth),
        AbDeploySubView::GateTestLogs => build_ab_gate_logs_section(state),
        AbDeploySubView::ActiveLeases => build_ab_leases_section(state, auth),
    };
    scene.sections.push(sec);
}

fn build_ab_gate_section(state: &ConsoleState, auth: &AuthState) -> SceneSection {
    let can_rollback = auth.has_policy("ab:rollback");
    let gate = state.client.view().and_then(|v| v.gate.as_ref());
    let items: Vec<GateItem> = if let Some(g) = gate {
        if g.tests.is_empty()
            && !router::effective_has_policy(auth, "ab:gate.logs.read")
            && !router::effective_has_policy(auth, "ab:logs.read")
        {
            vec![GateItem {
                id: "gate-test-policy-note".into(),
                label: "Gate test details restricted".into(),
                status_label: "Policy Required".into(),
                detail: Some("Requires ab:gate.logs.read to inspect gate test items.".into()),
                signal: SignalGlyph::BlockPending,
                tone: SemanticTone::Info,
                required: false,
                action: None,
            }]
        } else {
            g.tests
                .iter()
                .filter(|t| {
                    let hay = format!(
                        "{} {:?} {} {} {}",
                        t.name,
                        t.state,
                        t.log_excerpt.as_deref().unwrap_or(""),
                        g.deployment_id,
                        g.target_node
                    );
                    service_filter_matches(&state.filter_query, &hay)
                })
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
        }
    } else {
        Vec::new()
    };

    let dep_id = gate.map_or("active", |g| g.deployment_id.as_str());
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
    )
}

fn build_ab_slots_section(state: &ConsoleState, auth: &AuthState) -> SceneSection {
    let can_rollback = auth.has_policy("ab:rollback");
    let gate = state.client.view().and_then(|v| v.gate.as_ref());
    let cells = if let Some(g) = gate {
        let val_a = if g.restore_slot == Slot::A {
            "Active / Restore"
        } else {
            "Staged"
        };
        let val_b = if g.staged_slot == Slot::B {
            "Candidate"
        } else {
            "Standby"
        };
        let mut candidates = vec![
            MatrixCell {
                id: "ab-slot-a".into(),
                label: "@a".into(),
                value: val_a.into(),
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
                value: val_b.into(),
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
        ];
        candidates.retain(|cell| {
            let hay = format!(
                "{} {} {} {} {}",
                cell.label, cell.value, g.artifact_digest, g.target_node, g.deployment_id
            );
            service_filter_matches(&state.filter_query, &hay)
        });
        candidates
    } else {
        vec![]
    };
    SceneSection::new("sec-ab-slots", "Symmetric @a / @b Slots").with_component(
        SceneComponent::StatusMatrix(StatusMatrix {
            id: "ab-slots-matrix".into(),
            title: "Dual-Slot A|B State".into(),
            cells,
        }),
    )
}

fn build_ab_gate_logs_section(state: &ConsoleState) -> SceneSection {
    // Separate `ab:gate-logs.read`-gated permission tab (#56).
    let gate = state.client.view().and_then(|v| v.gate.as_ref());
    let entries: Vec<LogEntry> = gate
        .and_then(|g| g.tests.get(state.selected_test_idx))
        .and_then(|t| t.log_excerpt.as_deref())
        .map(|txt| {
            txt.lines()
                .enumerate()
                .filter(|(_, l)| service_filter_matches(&state.filter_query, l))
                .map(|(i, l)| LogEntry {
                    line_no: u32::try_from(i + 1).ok(),
                    timestamp: None,
                    level: SemanticTone::Info,
                    signal: None,
                    text: l.to_owned(),
                    trace_id: None,
                    anchor_key: None,
                    action: None,
                })
                .collect()
        })
        .unwrap_or_default();
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
    )
}

fn build_ab_leases_section(state: &ConsoleState, auth: &AuthState) -> SceneSection {
    let can_rollback = auth.has_policy("ab:rollback");
    let rows: Vec<TableRow> = state
        .client
        .view()
        .map(|v| {
            v.leases
                .iter()
                .filter(|l| {
                    let slots = format!("{:?} -> {:?}", l.active_slot, l.candidate_slot);
                    let hay = format!("{} {} {} {slots}", l.service, l.lease_id, l.deployment_id);
                    service_filter_matches(&state.filter_query, &hay)
                })
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
            empty_message: Some(if state.filter_query.trim().is_empty() {
                "No active slot leases".into()
            } else {
                copy::NO_FILTER_MATCHES.into()
            }),
            pagination: None,
        }),
    )
}

fn build_repo_sections(
    state: &ConsoleState,
    auth: &AuthState,
    subview: RepositorySubView,
    scene: &mut ConsoleScene,
) {
    let mut sec = SceneSection::new(
        "sec-repo",
        format!("Registry · {}", SubView::Repository(subview).label()),
    );
    sec = match subview {
        RepositorySubView::Packages => build_repo_packages_components(state, auth, sec),
        RepositorySubView::CanaryTaints => {
            sec.with_component(SceneComponent::TableGrid(build_repo_canaries_table(state)))
        }
        RepositorySubView::CasHealth => {
            sec.with_component(SceneComponent::KeyValueList(build_repo_cas_kv(state)))
        }
    };
    scene.sections.push(sec);
}

fn build_repo_eco_filter(state: &ConsoleState) -> StatusMatrix {
    let filter_cells = [
        ("reg-eco-all", "reg-eco:all", "All", "all", None),
        (
            "reg-eco-cargo",
            "reg-eco:cargo",
            "Cargo",
            "cargo",
            Some(PackageEcosystem::Cargo),
        ),
        (
            "reg-eco-nuget",
            "reg-eco:nuget",
            "NuGet",
            "nuget",
            Some(PackageEcosystem::NuGet),
        ),
        (
            "reg-eco-maven",
            "reg-eco:maven",
            "Maven",
            "maven",
            Some(PackageEcosystem::Maven),
        ),
    ]
    .into_iter()
    .map(|(cell_id, action_id, label, attr_val, eco)| {
        let selected = state.reg_ecosystem_filter == eco;
        MatrixCell {
            id: cell_id.into(),
            label: "Ecosystem".into(),
            value: label.into(),
            detail: None,
            tone: if selected {
                SemanticTone::Info
            } else {
                SemanticTone::Neutral
            },
            signal: None,
            action: Some(
                ActionControl::new(action_id, label, action_id)
                    .with_selected(selected)
                    .with_attr("data-ecosystem", attr_val),
            ),
        }
    })
    .collect();
    StatusMatrix {
        id: "reg-ecosystem-filter".into(),
        title: "Package Ecosystem Filter".into(),
        cells: filter_cells,
    }
}

fn repo_pkg_row(p: &mu_console_protocol::RegistryPackage) -> TableRow {
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
                text: format!("{status_lbl} · {} deps · {}", p.deps_count, p.sha256_prefix),
                tone,
                signal: None,
                mono: true,
                action: None,
            },
        ],
        row_action: None,
    }
}

fn build_repo_pkg_table(state: &ConsoleState) -> TableGrid {
    let pkg_rows: Vec<TableRow> = state
        .client
        .view()
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
                    let status_lbl = if p.tainted {
                        "Tainted"
                    } else if p.yanked {
                        "Yanked"
                    } else {
                        "Active"
                    };
                    let hay = format!(
                        "{} {} {} {} {status_lbl}",
                        p.name,
                        p.ecosystem.label(),
                        p.latest_version,
                        p.sha256_prefix
                    );
                    service_filter_matches(&state.filter_query, &hay)
                })
                .map(repo_pkg_row)
                .collect()
        })
        .unwrap_or_default();

    let pkg_filter_active =
        !state.filter_query.trim().is_empty() || state.reg_ecosystem_filter.is_some();
    TableGrid {
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
        empty_message: Some(if pkg_filter_active {
            copy::NO_FILTER_MATCHES.into()
        } else {
            "No matching packages".into()
        }),
        pagination: None,
    }
}

fn build_repo_list_table(state: &ConsoleState) -> TableGrid {
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
    TableGrid {
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
        empty_message: Some(if state.filter_query.trim().is_empty() {
            "No matching repositories".into()
        } else {
            copy::NO_FILTER_MATCHES.into()
        }),
        pagination: None,
    }
}

fn build_repo_packages_components(
    state: &ConsoleState,
    auth: &AuthState,
    mut sec: SceneSection,
) -> SceneSection {
    sec = sec
        .with_component(SceneComponent::StatusMatrix(build_repo_eco_filter(state)))
        .with_component(SceneComponent::TableGrid(build_repo_pkg_table(state)));
    let can_read_repos = router::effective_has_policy(auth, "ci:repos.public.read")
        || router::effective_has_policy(auth, "ci:repos.private.read");
    if can_read_repos {
        sec = sec.with_component(SceneComponent::TableGrid(build_repo_list_table(state)));
    }
    sec
}

fn build_repo_canaries_table(state: &ConsoleState) -> TableGrid {
    let rows: Vec<TableRow> = state
        .client
        .view()
        .map(|v| {
            v.registry
                .canaries
                .iter()
                .filter(|c| {
                    let hay = format!("{} {} {}", c.bundle_id, c.service, c.state.label());
                    service_filter_matches(&state.filter_query, &hay)
                })
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
    TableGrid {
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
        empty_message: Some(if state.filter_query.trim().is_empty() {
            "No canary bundles".into()
        } else {
            copy::NO_FILTER_MATCHES.into()
        }),
        pagination: None,
    }
}

fn build_repo_cas_kv(state: &ConsoleState) -> KeyValueList {
    let cas = state.client.view().map(|v| &v.registry.cas);
    let mut entries = vec![
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
    ];
    retain_matching_kv(&mut entries, &state.filter_query);
    KeyValueList {
        id: "repo-cas-kv".into(),
        title: "CAS Storage Health".into(),
        entries,
    }
}

fn matches_obs_service(selected: Option<&str>, candidate: &str) -> bool {
    selected.is_none_or(|svc| service_filter_matches(svc, candidate))
}

fn build_obs_service_bar(state: &ConsoleState, services: &[String]) -> PlatformPanel {
    PlatformPanel {
        id: "obs-service-filter".into(),
        title: "Service Scope".into(),
        summary: Some(
            state
                .obs_service_filter
                .clone()
                .unwrap_or_else(|| "All Services".into()),
        ),
        tone: SemanticTone::Info,
        attributes: vec![],
        controls: vec![
            ActionControl::new("obs-service-prev", "Prev Service", "obs-service-prev")
                .disabled(services.is_empty()),
            ActionControl::new("obs-service-next", "Next Service", "obs-service-next")
                .disabled(services.is_empty()),
            ActionControl::new("obs-service-clear", "Clear", "obs-service-clear")
                .with_aria_label("All Services")
                .disabled(state.obs_service_filter.is_none()),
        ],
    }
}

fn build_obs_smartscape_filter_bar(state: &ConsoleState) -> StatusMatrix {
    let cells = [
        ("obs-sf-all", "obs-smartscape-filter:all", "All", 0),
        (
            "obs-sf-services",
            "obs-smartscape-filter:services",
            "Services",
            1,
        ),
        (
            "obs-sf-processes",
            "obs-smartscape-filter:processes",
            "Processes",
            2,
        ),
        ("obs-sf-hosts", "obs-smartscape-filter:hosts", "Hosts", 3),
    ]
    .into_iter()
    .map(|(cell_id, action_id, label, filter_idx)| {
        let selected = state.obs_smartscape_filter == filter_idx;
        MatrixCell {
            id: cell_id.into(),
            label: "Layer".into(),
            value: label.into(),
            detail: None,
            tone: if selected {
                SemanticTone::Info
            } else {
                SemanticTone::Neutral
            },
            signal: None,
            action: Some(ActionControl::new(action_id, label, action_id).with_selected(selected)),
        }
    })
    .collect();
    StatusMatrix {
        id: "obs-smartscape-filter".into(),
        title: "Smartscape Layer Filter".into(),
        cells,
    }
}

fn build_observe_sections(
    state: &ConsoleState,
    subview: ObservabilitySubView,
    skin: ConsoleSkin,
    scene: &mut ConsoleScene,
) {
    let services = state.observed_services();
    let is_service_scoped = !matches!(subview, ObservabilitySubView::DataStore);
    if is_service_scoped && !services.is_empty() {
        scene.sections.push(
            SceneSection::new("sec-obs-service-filter", "Service Filter").with_component(
                SceneComponent::PlatformPanel(build_obs_service_bar(state, &services)),
            ),
        );
    }
    let mut sec = SceneSection::new(
        "sec-observe",
        format!(
            "Observability · {}",
            SubView::Observability(subview).label()
        ),
    );
    sec = match subview {
        ObservabilitySubView::Metrics => build_obs_metrics_components(state, sec),
        ObservabilitySubView::Logs => sec.with_component(SceneComponent::LogViewer(
            build_obs_logs_viewer(state, skin),
        )),
        ObservabilitySubView::PurePath => {
            sec.with_component(SceneComponent::TableGrid(build_obs_purepath_table(state)))
        }
        ObservabilitySubView::ProblemsSlos => {
            sec.with_component(SceneComponent::TableGrid(build_obs_problems_table(state)))
        }
        ObservabilitySubView::Smartscape => sec
            .with_component(SceneComponent::StatusMatrix(
                build_obs_smartscape_filter_bar(state),
            ))
            .with_component(SceneComponent::TableGrid(build_obs_smartscape_table(state))),
        ObservabilitySubView::DataStore => {
            sec.with_component(SceneComponent::KeyValueList(build_obs_datastore_kv(state)))
        }
        ObservabilitySubView::Ingress => build_obs_ingress_components(state, sec),
    };
    scene.sections.push(sec);
}

fn build_obs_metrics_components(state: &ConsoleState, mut sec: SceneSection) -> SceneSection {
    let selected_svc = state.obs_service_filter.as_deref();
    let mut added = 0_usize;
    if let Some(v) = state.client.view() {
        for (idx, m) in v.observe.metrics.iter().enumerate() {
            if !matches_obs_service(selected_svc, &m.service) {
                continue;
            }
            let hay = format!("{} {} {}", m.service, m.hardware_class, m.exemplar_trace_id);
            if !service_filter_matches(&state.filter_query, &hay) {
                continue;
            }
            added += 1;
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
    if added == 0 && (!state.filter_query.trim().is_empty() || selected_svc.is_some()) {
        sec = sec.with_component(SceneComponent::KeyValueList(KeyValueList {
            id: "obs-metrics-empty".into(),
            title: "Hardware & Golden Signals".into(),
            entries: vec![filter_empty_kv_entry()],
        }));
    }
    sec
}

fn obs_log_anchor_key(l: &OtelLogEntry) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    };
    feed(&l.timestamp_ms.to_le_bytes());
    feed(l.service.as_bytes());
    feed(l.trace_id.as_bytes());
    if let Some(ref sp) = l.span_id {
        feed(sp.as_bytes());
    }
    feed(l.message.as_bytes());
    format!("{:016x}-{hash:016x}", l.timestamp_ms)
}

fn build_observe_log_entries(
    state: &ConsoleState,
    logs: &[OtelLogEntry],
    window: &ObsLogWindowMeta,
) -> Vec<LogEntry> {
    let apply_client_query = state.obs_log_apply_client_query(window);
    let mut occ_counts: BTreeMap<String, u32> = BTreeMap::new();
    logs.iter()
        .enumerate()
        .filter(|(_, l)| state.obs_log_matches_filters(l, apply_client_query))
        .map(|(raw_idx, l)| {
            let level = match l.severity {
                LogSeverity::Error => SemanticTone::Fail,
                LogSeverity::Warn => SemanticTone::Warn,
                LogSeverity::Info => SemanticTone::Info,
            };
            let base = obs_log_anchor_key(l);
            let occ = occ_counts.entry(base.clone()).or_insert(0);
            let anchor = format!("{base}-{occ}");
            *occ += 1;
            LogEntry {
                line_no: u32::try_from(window.window_start_index as usize + raw_idx + 1).ok(),
                timestamp: Some(l.timestamp_ms.to_string()),
                level,
                signal: None,
                text: format!("[{}] {}", l.service, l.message),
                trace_id: Some(l.trace_id.clone()),
                anchor_key: Some(anchor),
                action: None,
            }
        })
        .collect()
}

fn build_obs_logs_viewer(state: &ConsoleState, skin: ConsoleSkin) -> LogViewer {
    let (all_entries, window, raw_len) = state
        .client
        .view()
        .map(|v| {
            let w = v.observe.log_window.clone();
            (
                build_observe_log_entries(state, &v.observe.logs, &w),
                w,
                v.observe.logs.len(),
            )
        })
        .unwrap_or_default();

    let filtered_count = all_entries.len();
    let client_subset = filtered_count < raw_len;
    let (entries, wf_page, local_wf_pages, page_start, page_end) = if skin == ConsoleSkin::Wireframe
    {
        let local_pages = filtered_count.div_ceil(WF_OBS_LOGS_PAGE_SIZE).max(1);
        let page = (state.subview_page as usize).min(local_pages - 1);
        state
            .rendered_obs_log_page
            .set(u32::try_from(page).unwrap_or(0));
        let start = page * WF_OBS_LOGS_PAGE_SIZE;
        let end = (start + WF_OBS_LOGS_PAGE_SIZE).min(filtered_count);
        (
            all_entries[start..end].to_vec(),
            page,
            local_pages,
            start,
            end,
        )
    } else {
        (all_entries, 0, 1, 0, filtered_count)
    };

    let client_filter_active = state.log_severity_filter.is_some()
        || state.obs_service_filter.is_some()
        || state.obs_log_apply_client_query(&window);
    let title = if filtered_count == 0 {
        if window.search_active || client_filter_active {
            "Distributed OpenTelemetry Logs (0 matching)".to_owned()
        } else {
            "Distributed OpenTelemetry Logs".to_owned()
        }
    } else if client_subset {
        let win_start = window.window_start_index as usize + 1;
        let win_end = window.window_start_index as usize + raw_len;
        if skin == ConsoleSkin::Wireframe && local_wf_pages > 1 {
            format!(
                "Distributed OpenTelemetry Logs ({}–{} of {filtered_count} matched in window {win_start}–{win_end})",
                page_start + 1,
                page_end
            )
        } else {
            format!(
                "Distributed OpenTelemetry Logs ({filtered_count} matched in window {win_start}–{win_end})"
            )
        }
    } else {
        let start_num = window.window_start_index as usize + page_start + 1;
        let end_num = window.window_start_index as usize + page_end;
        if let Some(total) = window.total_matched {
            format!("Distributed OpenTelemetry Logs ({start_num}–{end_num} of {total})")
        } else if window.cached_total as usize > entries.len() || window.window_start_index > 0 {
            format!(
                "Distributed OpenTelemetry Logs ({start_num}–{end_num} · {} cached)",
                window.cached_total
            )
        } else if skin == ConsoleSkin::Wireframe && local_wf_pages > 1 {
            format!(
                "Distributed OpenTelemetry Logs ({start_num}–{end_num} of {filtered_count} entries)"
            )
        } else {
            format!("Distributed OpenTelemetry Logs ({filtered_count} entries)")
        }
    };

    let mut controls = Vec::new();
    if window.window_start_index > 0
        || window.pending_live_count > 0
        || (skin == ConsoleSkin::Wireframe && wf_page > 0)
    {
        let label = if window.pending_live_count > 0 {
            format!("↑ Latest (+{})", window.pending_live_count)
        } else {
            "↑ Latest".to_owned()
        };
        controls.push(ActionControl::new(
            "obs-logs-latest",
            label,
            "obs-logs-latest",
        ));
    }

    let has_local_wf_paging = skin == ConsoleSkin::Wireframe && local_wf_pages > 1;
    let newer_enabled = (skin == ConsoleSkin::Wireframe && wf_page > 0) || window.has_newer;
    let older_enabled =
        (skin == ConsoleSkin::Wireframe && (wf_page + 1) < local_wf_pages) || window.has_older;
    let pagination = if window.has_newer
        || window.has_older
        || window.loading_newer
        || window.loading_older
        || window.window_start_index > 0
        || has_local_wf_paging
    {
        let (current_page, total_pages, total_items) = if skin == ConsoleSkin::Wireframe {
            let page_sz = u32::try_from(WF_OBS_LOGS_PAGE_SIZE).unwrap_or(32);
            let page_offset = if client_subset {
                0
            } else {
                window.window_start_index.div_ceil(page_sz)
            };
            let cur = page_offset + u32::try_from(wf_page).unwrap_or(0) + 1;
            let total_items = if client_subset {
                u32::try_from(filtered_count).ok()
            } else {
                window
                    .total_matched
                    .and_then(|t| u32::try_from(t).ok())
                    .or((window.cached_total > 0).then_some(window.cached_total))
                    .or_else(|| u32::try_from(filtered_count).ok())
            };
            let est_pages = total_items.map_or(cur, |t| t.div_ceil(page_sz).max(1));
            let local_end = page_offset + u32::try_from(local_wf_pages).unwrap_or(1);
            let tot = est_pages
                .max(local_end + u32::from(window.has_older))
                .max(1);
            (cur, tot, total_items)
        } else {
            let current_page = (window.window_start_index / 200) + 1;
            let total_items = window
                .total_matched
                .and_then(|t| u32::try_from(t).ok())
                .or((window.cached_total > 0).then_some(window.cached_total));
            let estimated_pages = total_items.map_or(current_page, |t| t.div_ceil(200).max(1));
            let total_pages = estimated_pages
                .max(current_page + u32::from(window.has_older))
                .max(1);
            (current_page, total_pages, total_items)
        };
        let newer_loading = wf_page == 0 && window.loading_newer;
        let older_loading = (wf_page + 1) >= local_wf_pages && window.loading_older;
        Some(Pagination {
            current_page,
            total_pages,
            total_items,
            prev: ActionControl::new(
                "obs-logs-newer",
                if newer_loading {
                    "Loading…"
                } else {
                    "↑ Newer"
                },
                "obs-logs-newer",
            )
            .disabled(!newer_enabled || newer_loading),
            next: ActionControl::new(
                "obs-logs-older",
                if older_loading {
                    "Loading…"
                } else {
                    "↓ Load Older"
                },
                "obs-logs-older",
            )
            .disabled(!older_enabled || older_loading),
        })
    } else {
        None
    };

    LogViewer {
        id: "obs-logs-viewer".into(),
        title,
        font_alias: Some("MuCRT".into()),
        wrap: true,
        stream_id: Some("otel-logs".into()),
        entries,
        controls,
        pagination,
    }
}

fn build_obs_purepath_table(state: &ConsoleState) -> TableGrid {
    let selected_svc = state.obs_service_filter.as_deref();
    let rows: Vec<TableRow> = state
        .client
        .view()
        .map(|v| {
            v.observe
                .traces
                .iter()
                .filter(|t| {
                    let svc_ok = selected_svc.is_none_or(|svc| {
                        service_filter_matches(svc, &t.root_service)
                            || t.spans
                                .iter()
                                .any(|sp| service_filter_matches(svc, &sp.service))
                    });
                    if !svc_ok {
                        return false;
                    }
                    let hay = format!(
                        "{} {} {} {}",
                        t.trace_id, t.root_service, t.root_operation, t.domain
                    );
                    service_filter_matches(&state.filter_query, &hay)
                })
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
    let filter_active = !state.filter_query.trim().is_empty() || selected_svc.is_some();
    TableGrid {
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
        empty_message: Some(if filter_active {
            copy::NO_FILTER_MATCHES.into()
        } else {
            "No traces captured".into()
        }),
        pagination: None,
    }
}

fn build_obs_problems_table(state: &ConsoleState) -> TableGrid {
    let selected_svc = state.obs_service_filter.as_deref();
    let rows: Vec<TableRow> = state
        .client
        .view()
        .map(|v| {
            v.observe
                .problems
                .iter()
                .filter(|p| p.active && !is_suppressed_obs_problem(p))
                .filter(|p| {
                    let svc_ok = selected_svc.is_none_or(|svc| {
                        service_filter_matches(svc, &p.impacted_entity)
                            || service_filter_matches(svc, &p.root_cause_service)
                            || p.affected_services
                                .iter()
                                .any(|s| service_filter_matches(svc, s))
                    });
                    if !svc_ok {
                        return false;
                    }
                    let hay = format!(
                        "{} {} {} {}",
                        p.problem_id, p.title, p.impacted_entity, p.severity
                    );
                    service_filter_matches(&state.filter_query, &hay)
                })
                .map(|p| TableRow {
                    id: format!("prob-{}", p.problem_id),
                    tone: SemanticTone::Fail,
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
    let filter_active = !state.filter_query.trim().is_empty() || selected_svc.is_some();
    TableGrid {
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
        empty_message: Some(if filter_active {
            copy::NO_FILTER_MATCHES.into()
        } else {
            "Zero open SLO alerts".into()
        }),
        pagination: None,
    }
}

fn build_obs_smartscape_table(state: &ConsoleState) -> TableGrid {
    let selected_svc = state.obs_service_filter.as_deref();
    let rows: Vec<TableRow> = state
        .client
        .view()
        .map(|v| {
            v.observe
                .smartscape_nodes
                .iter()
                .filter(|n| match state.obs_smartscape_filter {
                    1 => n.layer == SmartscapeLayer::Service,
                    2 => n.layer == SmartscapeLayer::Process,
                    3 => n.layer == SmartscapeLayer::Host,
                    _ => true,
                })
                .filter(|n| matches_obs_service(selected_svc, &n.label))
                .filter(|n| {
                    let hay = format!("{} {} {}", n.id, n.label, n.layer.label());
                    service_filter_matches(&state.filter_query, &hay)
                })
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
    let filter_active = !state.filter_query.trim().is_empty()
        || selected_svc.is_some()
        || state.obs_smartscape_filter != 0;
    TableGrid {
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
        empty_message: Some(if filter_active {
            copy::NO_FILTER_MATCHES.into()
        } else {
            "No topology nodes".into()
        }),
        pagination: None,
    }
}

fn build_obs_datastore_kv(state: &ConsoleState) -> KeyValueList {
    let ds = state.client.view().map(|v| &v.observe.store);
    let mut entries = vec![KeyValueEntry {
        key: "Hot DuckDB Rows".into(),
        value: ds.map_or(0, |s| s.hot_duckdb_rows).to_string(),
        detail: None,
        tone: SemanticTone::Pass,
        mono: true,
        action: None,
    }];
    retain_matching_kv(&mut entries, &state.filter_query);
    KeyValueList {
        id: "obs-datastore-kv".into(),
        title: "DuckDB & Parquet Store Health".into(),
        entries,
    }
}

fn obs_ingress_row(r: &mu_console_protocol::dto::IngressRouteDto) -> TableRow {
    let tone = if r.error_rate_pct > 1.0 {
        SemanticTone::Fail
    } else {
        SemanticTone::Pass
    };
    TableRow {
        id: format!("ing-{}", r.route_id),
        tone,
        cells: vec![
            TableCell {
                text: r.route_id.clone(),
                tone: SemanticTone::Info,
                signal: None,
                mono: true,
                action: None,
            },
            TableCell {
                text: r.path_prefix.clone(),
                tone: SemanticTone::Neutral,
                signal: None,
                mono: true,
                action: None,
            },
            TableCell {
                text: r.service_name.clone(),
                tone: SemanticTone::Info,
                signal: None,
                mono: true,
                action: None,
            },
            TableCell {
                text: r.upstream_url.clone(),
                tone: SemanticTone::Neutral,
                signal: None,
                mono: true,
                action: None,
            },
        ],
        row_action: None,
    }
}

fn build_obs_ingress_components(state: &ConsoleState, mut sec: SceneSection) -> SceneSection {
    let selected_svc = state.obs_service_filter.as_deref();
    let filtered_routes: Vec<_> = state
        .client
        .view()
        .map(|v| {
            v.observe
                .ingress_routes
                .iter()
                .filter(|r| matches_obs_service(selected_svc, &r.service_name))
                .filter(|r| {
                    let hay = format!(
                        "{} {} {} {}",
                        r.route_id, r.path_prefix, r.service_name, r.upstream_url
                    );
                    service_filter_matches(&state.filter_query, &hay)
                })
                .collect()
        })
        .unwrap_or_default();
    let rows: Vec<TableRow> = filtered_routes.into_iter().map(obs_ingress_row).collect();
    let filter_active = !state.filter_query.trim().is_empty() || selected_svc.is_some();
    sec = sec
        .with_component(SceneComponent::KeyValueList(KeyValueList {
            id: "obs-ingress-kv".into(),
            title: "Ingress & TLS Telemetry".into(),
            entries: vec![KeyValueEntry {
                key: "Routes".into(),
                value: rows.len().to_string(),
                detail: None,
                tone: SemanticTone::Info,
                mono: true,
                action: None,
            }],
        }))
        .with_component(SceneComponent::TableGrid(TableGrid {
            id: "obs-ingress-table".into(),
            title: "Active Ingress Routes".into(),
            columns: vec![
                TableColumn {
                    key: "route".into(),
                    label: "Route".into(),
                    numeric: false,
                },
                TableColumn {
                    key: "path".into(),
                    label: "Path".into(),
                    numeric: false,
                },
                TableColumn {
                    key: "service".into(),
                    label: "Service".into(),
                    numeric: false,
                },
                TableColumn {
                    key: "upstream".into(),
                    label: "Upstream".into(),
                    numeric: false,
                },
            ],
            rows,
            empty_message: Some(if filter_active {
                copy::NO_FILTER_MATCHES.into()
            } else {
                "No ingress routes configured".into()
            }),
            pagination: None,
        }));
    sec
}

fn build_model_runner_sections(
    state: &ConsoleState,
    subview: ModelRunnerSubView,
    scene: &mut ConsoleScene,
) {
    // Strictly read-only area (#56): zero mutation controls.
    let mut sec = SceneSection::new(
        "sec-model-runner",
        format!("Model Runner · {}", SubView::ModelRunner(subview).label()),
    );
    sec = match subview {
        ModelRunnerSubView::RuntimeQueue => {
            sec.with_component(SceneComponent::TableGrid(build_mr_runtime_table(state)))
        }
        ModelRunnerSubView::ConnectionVpn => {
            sec.with_component(SceneComponent::TableGrid(build_mr_vpn_table(state)))
        }
    };
    scene.sections.push(sec);
}

fn build_mr_runtime_table(state: &ConsoleState) -> TableGrid {
    let rows: Vec<TableRow> = state
        .client
        .view()
        .and_then(|v| v.model_runner.status.as_ref())
        .map(|s| {
            s.models
                .iter()
                .filter(|r| {
                    let hay = format!("{} {}", r.model_id, r.cluster);
                    service_filter_matches(&state.filter_query, &hay)
                })
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
    TableGrid {
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
        empty_message: Some(if state.filter_query.trim().is_empty() {
            "No active model runtimes loaded".into()
        } else {
            copy::NO_FILTER_MATCHES.into()
        }),
        pagination: None,
    }
}

fn build_mr_vpn_table(state: &ConsoleState) -> TableGrid {
    let rows: Vec<TableRow> = state
        .client
        .view()
        .and_then(|v| v.model_runner.connection.as_ref())
        .map(|c| {
            c.peers
                .iter()
                .filter(|p| {
                    let hay = format!("{} {}", p.public_key_prefix, p.allowed_ips);
                    service_filter_matches(&state.filter_query, &hay)
                })
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
    TableGrid {
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
        empty_message: Some(if state.filter_query.trim().is_empty() {
            "No overlay peers reported".into()
        } else {
            copy::NO_FILTER_MATCHES.into()
        }),
        pagination: None,
    }
}

fn build_platform_sections(
    state: &ConsoleState,
    auth: &AuthState,
    subview: PlatformSubView,
    scene: &mut ConsoleScene,
) {
    let sec = match subview {
        PlatformSubView::Maintenance => build_plat_maintenance_section(state, auth),
        PlatformSubView::UsersAndInvites => build_plat_users_section(state, auth),
        PlatformSubView::RolesAndFolios => build_plat_roles_section(state, auth),
    };
    scene.sections.push(sec);
}

fn build_plat_maintenance_section(state: &ConsoleState, auth: &AuthState) -> SceneSection {
    let can_maint = auth.has_policy("platform:maintenance.write");
    let max_hdr = state.client.view().map_or(
        mu_console_protocol::DEFAULT_MAX_ACTIVE_ROLES_HEADER_CHARS,
        |v| v.max_active_roles_header_chars,
    );
    let mut attributes = vec![KeyValueEntry {
        key: "Selected Subsystem".into(),
        value: format!("{:?}", state.platform_form.selected_subsystem),
        detail: Some(format!("{:?}", state.platform_form.maintenance_mode)),
        tone: SemanticTone::Info,
        mono: true,
        action: None,
    }];
    retain_matching_kv(&mut attributes, &state.filter_query);
    SceneSection::new("sec-platform-maint", "Platform Maintenance").with_component(
        SceneComponent::PlatformPanel(PlatformPanel {
            id: "plat-maintenance-panel".into(),
            title: "Infrastructure Maintenance & Timers".into(),
            summary: Some(format!(
                "Header role budget: {max_hdr} chars · Maintenance updates require confirmation.",
            )),
            tone: SemanticTone::Info,
            attributes,
            controls: vec![ActionControl::new(
                "platform-maint-apply",
                "Apply Maintenance Policy",
                "platform-maint-apply",
            )
            .disabled(!can_maint)
            .with_tone(SemanticTone::Warn)],
        }),
    )
}

fn plat_user_row(
    u: &mu_console_protocol::dto::PlatformUserDto,
    admin_count: usize,
    can_users: bool,
) -> TableRow {
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
}

fn build_plat_users_section(state: &ConsoleState, auth: &AuthState) -> SceneSection {
    let can_view_invites =
        auth.has_policy("platform:invites.read") || auth.has_policy("platform:invites.write");
    let can_view_users =
        auth.has_policy("platform:users.read") || auth.has_policy("platform:users.write");
    let can_users = auth.has_policy("platform:users.write");
    let can_invites = auth.has_policy("platform:invites.write");
    let users = state
        .client
        .view()
        .map_or(&[][..], |v| v.platform.users.as_slice());
    // Compute total admin count BEFORE filtering so Last-Admin guard stays accurate when filtered.
    let admin_count = users.iter().filter(|u| u.is_platform_admin).count();
    let rows: Vec<TableRow> = users
        .iter()
        .filter(|u| {
            let hay = format!(
                "{} @{} {}",
                u.github_login,
                u.github_login,
                u.bound_roles.join(" ")
            );
            service_filter_matches(&state.filter_query, &hay)
        })
        .map(|u| plat_user_row(u, admin_count, can_users))
        .collect();
    let mut sec = SceneSection::new("sec-platform-users", "Users & One-Time Invites");
    if can_view_invites {
        sec = sec.with_component(SceneComponent::PlatformPanel(PlatformPanel {
            id: "plat-invite-creator".into(),
            title: "Create One-Time Onboarding Invite".into(),
            summary: Some(
                "Invite URLs are minted once and displayed only in the confirmation modal.".into(),
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
        }));
    }
    if can_view_users {
        sec = sec.with_component(SceneComponent::TableGrid(TableGrid {
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
            empty_message: Some(if state.filter_query.trim().is_empty() {
                "No enrolled users".into()
            } else {
                copy::NO_FILTER_MATCHES.into()
            }),
            pagination: None,
        }));
    }
    sec
}

fn build_plat_roles_section(state: &ConsoleState, auth: &AuthState) -> SceneSection {
    let can_roles = auth.has_policy("platform:roles.write");
    let rows: Vec<TableRow> = state
        .client
        .view()
        .map(|v| {
            v.platform
                .roles
                .iter()
                .filter(|r| {
                    let folios = v
                        .platform
                        .role_folio_index
                        .role_folios
                        .get(&r.id)
                        .map_or(String::new(), |fs| fs.join(" "));
                    let hay = format!(
                        "{} {} {:?} {folios}",
                        r.id,
                        r.effective_display_name(),
                        r.activation.kind
                    );
                    service_filter_matches(&state.filter_query, &hay)
                })
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
            empty_message: Some(if state.filter_query.trim().is_empty() {
                "No roles defined".into()
            } else {
                copy::NO_FILTER_MATCHES.into()
            }),
            pagination: None,
        }),
    )
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
        encode, CiJobSummary, DataEvent, FabricNode, ProblemCategory, RepoId, RepoScore,
        SloProblem, UiCommand, ViewModel,
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
        assert_eq!(ci_row.cells[0].text, "CI Job");
        assert_eq!(ci_row.cells[2].text, "3/4 tests passed");
        assert_eq!(ci_row.cells[3].text, "Failed");
    }

    #[test]
    fn ci_failures_and_inner_step_problems_do_not_degrade_apps_or_obs_slos() {
        use mu_console_protocol::{
            CiJobStage, SubsystemHealthEntry, SubsystemHealthSummary, SubsystemStatus,
        };

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
                    "obs:problems.read".into(),
                ],
                ..AuthState::default()
            },
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
            ci_jobs: vec![
                CiJobSummary {
                    job_id: 101,
                    repo: RepoId::new("LMA1980", "jero-core"),
                    ref_name: "main".into(),
                    queue_state: "failed".into(),
                    tests_passed: 639,
                    tests_total: 639,
                    ..CiJobSummary::default()
                },
                CiJobSummary {
                    job_id: 102,
                    repo: RepoId::new("LMA1980", "mu-console"),
                    ref_name: "main".into(),
                    queue_state: "failed".into(),
                    tests_passed: 10,
                    tests_total: 10,
                    stages: vec![CiJobStage {
                        id: "miri-check".into(),
                        name: "Miri".into(),
                        assigned_node: Some("node-a".into()),
                        threads: 6,
                        state: StageState::Failed,
                        duration_secs: 15,
                        exit_code: 1,
                        skip_reason: None,
                    }],
                    ..CiJobSummary::default()
                },
            ],
            observe: mu_console_protocol::ObserveView {
                problems: vec![
                    SloProblem {
                        problem_id: "PROB-TEST-jero-core-miri".into(),
                        title: "Flaky / Failing Test jero-core::foo (cargo-miri)".into(),
                        category: ProblemCategory::FlakyTest,
                        severity: "warning".into(),
                        impacted_entity: "lma1980/jero-core".into(),
                        active: true,
                        ..SloProblem::default()
                    },
                    SloProblem {
                        problem_id: "PROB-BENCH-jero-core-loop".into(),
                        title: "Benchmark Regression in jero-core::loop".into(),
                        category: ProblemCategory::LatencyRegression,
                        severity: "error".into(),
                        impacted_entity: "lma1980/jero-core".into(),
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

        // 1. Applications cards stay Live/Pass despite failed CI jobs and CI inner-step items.
        let app_sec = scene
            .sections
            .iter()
            .find(|s| s.id == "sec-dash-applications")
            .expect("Applications section");
        let SceneComponent::StatusMatrix(mat) = &app_sec.components[0] else {
            panic!("expected StatusMatrix");
        };
        assert_eq!(mat.cells[0].id, "dash-card:ci");
        assert_eq!(mat.cells[0].value, "Live");
        assert_eq!(mat.cells[0].tone, SemanticTone::Pass);
        assert_eq!(mat.cells[4].id, "dash-card:observability");
        assert_eq!(mat.cells[4].value, "Live");
        assert_eq!(mat.cells[4].tone, SemanticTone::Pass);
        assert_eq!(
            mat.cells[4].detail.as_deref(),
            Some("0 logs · 0 traces · 0 metrics")
        );

        // 2. Needs Attention shows only CI Job rows (zero PROB-TEST / PROB-BENCH rows).
        let att_sec = scene
            .sections
            .iter()
            .find(|s| s.id == "sec-dash-attention")
            .expect("Needs Attention section");
        let SceneComponent::TableGrid(grid) = &att_sec.components[0] else {
            panic!("expected TableGrid");
        };
        assert_eq!(grid.rows.len(), 2);
        let j101 = grid
            .rows
            .iter()
            .find(|r| r.id == "att-ci-101")
            .expect("job 101 row");
        assert_eq!(j101.cells[0].text, "CI Job");
        assert_eq!(
            j101.cells[2].text,
            "CI job failed (639/639 unit tests passed)"
        );
        assert_eq!(j101.cells[3].text, "Failed");
        assert_eq!(
            j101.row_action.as_ref().map(|a| a.action.as_str()),
            Some("ci-job:101")
        );

        let j102 = grid
            .rows
            .iter()
            .find(|r| r.id == "att-ci-102")
            .expect("job 102 row");
        assert_eq!(j102.cells[0].text, "CI Job");
        assert_eq!(j102.cells[2].text, "Stage 'miri-check' failed (exit 1)");
        assert_eq!(j102.cells[3].text, "Failed");

        // 3. Observability tab badge & Problems/SLOs view also exclude CI inner-step items.
        let obs_tab = scene
            .area_tabs
            .iter()
            .find(|t| t.id == "observability")
            .expect("obs tab");
        assert_eq!(obs_tab.badge, None);

        st.select_subview(SubView::Observability(ObservabilitySubView::ProblemsSlos));
        let obs_scene = build_console_scene(&st, SceneViewport::DESKTOP);
        let obs_sec = obs_scene
            .sections
            .iter()
            .find(|s| s.id == "sec-observe")
            .expect("sec-observe");
        let SceneComponent::TableGrid(obs_grid) = &obs_sec.components[0] else {
            panic!("expected obs-problems-table");
        };
        assert!(obs_grid.rows.is_empty());
    }

    #[test]
    fn active_failing_test_surfaces_until_fixed_and_superseded_ci_job_clears() {
        use mu_console_protocol::{
            ObservabilitySubView, SubsystemHealthEntry, SubsystemHealthSummary, SubsystemStatus,
        };

        let mut st = ConsoleState::new();
        st.init_from_env("#/dashboard/overview", "", None);

        let make_vm = |jobs: Vec<CiJobSummary>, problems: Vec<SloProblem>| ViewModel {
            auth: AuthState {
                status: AuthStatus::Active,
                github_login: Some("ops".into()),
                active_policies: vec![
                    "system:dashboard.read".into(),
                    "ci:queue.read".into(),
                    "ci:jobs.read".into(),
                    "obs:problems.read".into(),
                ],
                ..AuthState::default()
            },
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

        // 1. Failing test (severity=error) + failed CI job both alert while Observability card stays Live.
        let vm1 = make_vm(
            vec![CiJobSummary {
                job_id: 201,
                repo: RepoId::new("LMA1980", "jero-core"),
                ref_name: "main".into(),
                queue_state: "failed".into(),
                tests_passed: 9,
                tests_total: 10,
                ..CiJobSummary::default()
            }],
            vec![SloProblem {
                problem_id: "PROB-TEST-jero-core-cargo-test-foo".into(),
                title: "Failing Test jero-core::foo (cargo-test)".into(),
                category: ProblemCategory::FlakyTest,
                severity: "error".into(),
                impacted_entity: "lma1980/jero-core".into(),
                active: true,
                ..SloProblem::default()
            }],
        );
        st.on_worker_frame(&encode(&DataEvent::Snapshot(vm1)).expect("encode vm1"));
        let scene1 = build_console_scene(&st, SceneViewport::DESKTOP);

        assert_eq!(
            scene1
                .area_tabs
                .iter()
                .find(|t| t.id == "ci")
                .and_then(|t| t.badge),
            Some(1)
        );
        assert_eq!(
            scene1
                .area_tabs
                .iter()
                .find(|t| t.id == "observability")
                .and_then(|t| t.badge),
            Some(1)
        );
        let app_sec1 = scene1
            .sections
            .iter()
            .find(|s| s.id == "sec-dash-applications")
            .unwrap();
        let SceneComponent::StatusMatrix(mat1) = &app_sec1.components[0] else {
            panic!("expected StatusMatrix");
        };
        assert_eq!(mat1.cells[4].id, "dash-card:observability");
        assert_eq!(mat1.cells[4].value, "Live");
        assert_eq!(mat1.cells[4].tone, SemanticTone::Pass);

        let att_sec1 = scene1
            .sections
            .iter()
            .find(|s| s.id == "sec-dash-attention")
            .unwrap();
        let SceneComponent::TableGrid(grid1) = &att_sec1.components[0] else {
            panic!("expected TableGrid");
        };
        assert_eq!(grid1.rows.len(), 2);
        let test_row = grid1
            .rows
            .iter()
            .find(|r| r.id == "att-obs-PROB-TEST-jero-core-cargo-test-foo")
            .expect("CI Test attention row");
        assert_eq!(test_row.cells[0].text, "CI Test");
        assert_eq!(test_row.cells[3].text, "Failed");

        st.select_subview(SubView::Observability(ObservabilitySubView::ProblemsSlos));
        let obs_scene1 = build_console_scene(&st, SceneViewport::DESKTOP);
        let obs_sec1 = obs_scene1
            .sections
            .iter()
            .find(|s| s.id == "sec-observe")
            .unwrap();
        let SceneComponent::TableGrid(obs_grid1) = &obs_sec1.components[0] else {
            panic!("expected obs-problems-table");
        };
        assert_eq!(obs_grid1.rows.len(), 1);

        // 2. Newer CI job passes on the same repo/branch and test alert clears -> both alerts vanish.
        st.select_subview(SubView::Dashboard(DashboardSubView::Overview));
        let vm2 = make_vm(
            vec![
                CiJobSummary {
                    job_id: 202,
                    repo: RepoId::new("LMA1980", "jero-core"),
                    ref_name: "main".into(),
                    queue_state: "passed".into(),
                    tests_passed: 10,
                    tests_total: 10,
                    ..CiJobSummary::default()
                },
                CiJobSummary {
                    job_id: 201,
                    repo: RepoId::new("LMA1980", "jero-core"),
                    ref_name: "main".into(),
                    queue_state: "failed".into(),
                    tests_passed: 9,
                    tests_total: 10,
                    ..CiJobSummary::default()
                },
            ],
            Vec::new(),
        );
        st.on_worker_frame(&encode(&DataEvent::Snapshot(vm2)).expect("encode vm2"));
        let scene2 = build_console_scene(&st, SceneViewport::DESKTOP);
        assert_eq!(
            scene2
                .area_tabs
                .iter()
                .find(|t| t.id == "ci")
                .and_then(|t| t.badge),
            None
        );
        assert_eq!(
            scene2
                .area_tabs
                .iter()
                .find(|t| t.id == "observability")
                .and_then(|t| t.badge),
            None
        );
        let att_sec2 = scene2
            .sections
            .iter()
            .find(|s| s.id == "sec-dash-attention")
            .unwrap();
        let SceneComponent::TableGrid(grid2) = &att_sec2.components[0] else {
            panic!("expected TableGrid");
        };
        assert!(grid2.rows.is_empty());
    }

    fn test_state_with_vm(hash: &str, vm: ViewModel) -> ConsoleState {
        let mut st = ConsoleState::new();
        st.init_from_env(hash, "", None);
        let frame = encode(&DataEvent::Snapshot(vm)).expect("encode ViewModel");
        st.on_worker_frame(&frame);
        st
    }

    #[test]
    fn repo_selector_is_area_subview_and_policy_gated() {
        let full_policies: Vec<String> = [
            "system:dashboard.read",
            "ci:queue.read",
            "ci:repos.public.read",
            "ci:nodes.read",
            "ci:sandboxes.read",
            "cd:runs.read",
            "ab:slots.read",
            "ab:leases.read",
            "reg:packages.read",
            "reg:cas.read",
            "obs:smartscape.read",
            "obs:traces.read",
            "obs:problems.read",
            "obs:logs.read",
            "obs:metrics.read",
            "obs:store.read",
            "obs:ingress.read",
            "mr:models.read",
            "platform:maintenance.read",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();

        let vm = ViewModel {
            auth: AuthState {
                status: AuthStatus::Active,
                github_login: Some("ops".into()),
                passkey_count: 1,
                active_policies: full_policies,
                ..AuthState::default()
            },
            ..ViewModel::default()
        };

        let mut st = test_state_with_vm("#/ci/scorecards", vm);
        assert!(build_console_scene(&st, SceneViewport::DESKTOP)
            .repo_selector
            .is_some());

        for sv in ObservabilitySubView::ALL {
            st.select_subview(SubView::Observability(sv));
            assert!(
                build_console_scene(&st, SceneViewport::DESKTOP)
                    .repo_selector
                    .is_none(),
                "Observability::{sv:?} must omit repo_selector"
            );
        }

        for sv in [
            SubView::Dashboard(DashboardSubView::Overview),
            SubView::Ci(CiSubView::WorkerNodes),
            SubView::Ci(CiSubView::SandboxProfiles),
            SubView::Repository(RepositorySubView::CasHealth),
            SubView::AbDeploy(AbDeploySubView::ActiveLeases),
            SubView::ModelRunner(ModelRunnerSubView::RuntimeQueue),
            SubView::Platform(PlatformSubView::Maintenance),
        ] {
            st.select_subview(sv);
            assert!(
                build_console_scene(&st, SceneViewport::DESKTOP)
                    .repo_selector
                    .is_none(),
                "{sv:?} must omit repo_selector"
            );
        }

        for sv in [
            SubView::Ci(CiSubView::Trainway),
            SubView::Cd(CdSubView::Runs),
            SubView::AbDeploy(AbDeploySubView::IntegrationGate),
            SubView::Repository(RepositorySubView::Packages),
        ] {
            st.select_subview(sv);
            assert!(
                build_console_scene(&st, SceneViewport::DESKTOP)
                    .repo_selector
                    .is_some(),
                "{sv:?} should render repo_selector when ci:repos.public.read is held"
            );
        }

        // Without ci:repos.public.read or ci:repos.private.read, repo_selector is omitted even on Scorecards.
        let vm_no_repos = ViewModel {
            auth: AuthState {
                status: AuthStatus::Active,
                github_login: Some("ops".into()),
                passkey_count: 1,
                active_policies: vec!["ci:jobs.read".into()],
                ..AuthState::default()
            },
            ..ViewModel::default()
        };
        let st_no_repos = test_state_with_vm("#/ci/scorecards", vm_no_repos);
        assert!(build_console_scene(&st_no_repos, SceneViewport::DESKTOP)
            .repo_selector
            .is_none());
    }

    #[test]
    fn auth_gates_distinguish_anonymous_passkey_required_and_active_without_access() {
        use mu_console_protocol::{CiJobStage, SkipReason};

        let sample_job = CiJobSummary {
            job_id: 71,
            repo: RepoId::new("LMA1980", "public-crate"),
            ref_name: "main".into(),
            queue_state: "failed".into(),
            stages: vec![CiJobStage {
                id: "clippy".into(),
                name: "Clippy".into(),
                assigned_node: Some("secret-node-01".into()),
                threads: 12,
                state: StageState::Failed,
                duration_secs: 9,
                exit_code: 1,
                skip_reason: None,
            }],
            raw_log_lines: vec!["SECRET_RAW_BUILD_LOG".into()],
            ..CiJobSummary::default()
        };
        let skipped_job = CiJobSummary {
            job_id: 72,
            repo: RepoId::new("LMA1980", "public-crate"),
            ref_name: "feat/x".into(),
            queue_state: "passed".into(),
            stages: vec![CiJobStage {
                id: "test".into(),
                name: "Test".into(),
                assigned_node: None,
                threads: 0,
                state: StageState::Skipped,
                duration_secs: 0,
                exit_code: 0,
                skip_reason: Some(SkipReason::ConditionNotMet),
            }],
            ..CiJobSummary::default()
        };

        // 1. Anonymous deep link -> Authentication required + Sign In.
        let anon_vm = ViewModel {
            auth: AuthState::anonymous(vec![
                "system:status.read".into(),
                "ci:queue.read".into(),
                "ci:repos.public.read".into(),
            ]),
            ..ViewModel::default()
        };
        let st_anon = test_state_with_vm("#/platform/users", anon_vm);
        let sc_anon = build_console_scene(&st_anon, SceneViewport::DESKTOP);
        assert_eq!(
            sc_anon.footer_status.as_deref(),
            Some(copy::AUTH_REQUIRED_TITLE)
        );
        let SceneComponent::PlatformPanel(p_anon) = &sc_anon.sections[0].components[0] else {
            panic!("expected PlatformPanel");
        };
        assert_eq!(p_anon.title, copy::AUTH_REQUIRED_TITLE);
        assert_eq!(
            p_anon.summary.as_deref(),
            Some(copy::AUTH_REQUIRED_ANON_BODY)
        );
        assert_eq!(p_anon.controls[0].control_id, "auth-gate-signin");

        // 2. PasskeyEnrollmentRequired deep link -> Passkey enrollment required + Register Passkey.
        let pk_vm = ViewModel {
            auth: AuthState {
                status: AuthStatus::PasskeyEnrollmentRequired,
                github_login: Some("newdev".into()),
                passkey_count: 0,
                passkey_enrollment_required: true,
                active_policies: vec![
                    "system:status.read".into(),
                    "ci:queue.read".into(),
                    "ci:repos.public.read".into(),
                ],
                ..AuthState::default()
            },
            ci_jobs: vec![sample_job.clone(), skipped_job],
            ..ViewModel::default()
        };
        let st_pk_deep = test_state_with_vm("#/ci/scorecards", pk_vm.clone());
        let sc_pk_deep = build_console_scene(&st_pk_deep, SceneViewport::DESKTOP);
        assert_eq!(
            sc_pk_deep.footer_status.as_deref(),
            Some(copy::PASSKEY_ACCESS_REQUIRED_TITLE)
        );
        let SceneComponent::PlatformPanel(p_pk) = &sc_pk_deep.sections[0].components[0] else {
            panic!("expected PlatformPanel");
        };
        assert_eq!(p_pk.title, copy::PASSKEY_ACCESS_REQUIRED_TITLE);
        assert_eq!(p_pk.controls[0].control_id, "auth-gate-register-passkey");

        // PasskeyEnrollmentRequired on CI > QueueLogs hides subviews, repo_selector, logs, node/threads.
        let st_pk_ci = test_state_with_vm("#/ci/logs", pk_vm);
        let sc_pk_ci = build_console_scene(&st_pk_ci, SceneViewport::DESKTOP);
        assert!(sc_pk_ci.subviews.is_empty());
        assert!(sc_pk_ci.repo_selector.is_none());
        let ci_sec = sc_pk_ci
            .sections
            .iter()
            .find(|s| s.id == "sec-ci-queue-logs")
            .expect("ci queue section");
        assert!(ci_sec.components.iter().any(|c| matches!(
            c,
            SceneComponent::TableGrid(t) if t.title == copy::PASSKEY_CI_STEPS_SUBTITLE
        )));
        assert!(!ci_sec
            .components
            .iter()
            .any(|c| matches!(c, SceneComponent::LogViewer(_))));
        let SceneComponent::GateChecklist(gc_pk) = &ci_sec.components[2] else {
            panic!("expected GateChecklist");
        };
        assert_eq!(gc_pk.items[0].detail.as_deref(), Some("9s · exit 1"));

        // 3. Active without access -> Access restricted + Active Roles + dormant elevation / Dashboard / Sign Out.
        let active_no_roles_vm = ViewModel {
            auth: AuthState {
                status: AuthStatus::Active,
                github_login: Some("guest-op".into()),
                passkey_count: 1,
                bound_roles: vec!["ci-triage".into()],
                active_policies: vec!["system:dashboard.read".into()],
                ..AuthState::default()
            },
            ..ViewModel::default()
        };
        let mut st_active_deep =
            test_state_with_vm("#/dashboard/overview", active_no_roles_vm.clone());
        st_active_deep.apply_hash("#/platform/users");
        let sc_active_deep = build_console_scene(&st_active_deep, SceneViewport::DESKTOP);
        assert_eq!(
            sc_active_deep.footer_status.as_deref(),
            Some(copy::ACCESS_RESTRICTED_TITLE)
        );
        let SceneComponent::PlatformPanel(p_act) = &sc_active_deep.sections[0].components[0] else {
            panic!("expected PlatformPanel");
        };
        assert_eq!(p_act.title, copy::ACCESS_RESTRICTED_TITLE);
        assert_eq!(
            p_act.summary.as_deref(),
            Some(copy::ACCESS_RESTRICTED_ELEVATE_BODY)
        );
        assert_eq!(p_act.attributes[1].key, "Active Roles");
        assert_eq!(p_act.attributes[1].value, "None");
        assert!(p_act.controls.iter().any(|c| c.control_id == "hdr-role"));
        assert!(p_act
            .controls
            .iter()
            .any(|c| c.control_id == "auth-gate-dashboard"));
        assert!(!p_act
            .controls
            .iter()
            .any(|c| c.control_id == "auth-gate-signout"));

        // Active with zero accessible areas shows auth-gate-signout.
        let zero_area_vm = ViewModel {
            auth: AuthState {
                status: AuthStatus::Active,
                github_login: Some("zero-op".into()),
                passkey_count: 1,
                ..AuthState::default()
            },
            ..ViewModel::default()
        };
        let st_zero = test_state_with_vm("#/platform/users", zero_area_vm);
        let sc_zero = build_console_scene(&st_zero, SceneViewport::DESKTOP);
        let SceneComponent::PlatformPanel(p_zero) = &sc_zero.sections[0].components[0] else {
            panic!("expected PlatformPanel");
        };
        assert_eq!(
            p_zero.summary.as_deref(),
            Some(copy::ACCESS_RESTRICTED_NO_ROLE_BODY)
        );
        assert!(p_zero
            .controls
            .iter()
            .any(|c| c.control_id == "auth-gate-signout"));

        // Dashboard with only system:dashboard.read and dormant role shows sec-dash-role-hint instead of attention.
        let st_dash = test_state_with_vm("#/dashboard/overview", active_no_roles_vm);
        let sc_dash = build_console_scene(&st_dash, SceneViewport::DESKTOP);
        assert!(!sc_dash
            .sections
            .iter()
            .any(|s| s.id == "sec-dash-attention"));
        assert!(sc_dash
            .sections
            .iter()
            .any(|s| s.id == "sec-dash-role-hint"));

        // Active with ci:queue.read only (no logs/nodes) renders ci-job-logs-restricted and hides node/thread.
        let ci_queue_only_vm = ViewModel {
            auth: AuthState {
                status: AuthStatus::Active,
                github_login: Some("ci-viewer".into()),
                passkey_count: 1,
                bound_roles: vec!["ci-ops".into()],
                active_policies: vec!["ci:queue.read".into()],
                ..AuthState::default()
            },
            ci_jobs: vec![sample_job],
            ..ViewModel::default()
        };
        let st_ci_q = test_state_with_vm("#/ci/logs", ci_queue_only_vm);
        let sc_ci_q = build_console_scene(&st_ci_q, SceneViewport::DESKTOP);
        let ci_q_sec = sc_ci_q
            .sections
            .iter()
            .find(|s| s.id == "sec-ci-queue-logs")
            .expect("ci sec");
        assert!(ci_q_sec.components.iter().any(|c| matches!(
            c,
            SceneComponent::KeyValueList(kv) if kv.id == "ci-job-logs-restricted"
        )));
        assert!(!ci_q_sec
            .components
            .iter()
            .any(|c| matches!(c, SceneComponent::LogViewer(_))));
        let SceneComponent::GateChecklist(gc_q) = &ci_q_sec.components[2] else {
            panic!("expected GateChecklist");
        };
        assert_eq!(gc_q.items[0].detail.as_deref(), Some("9s · exit 1"));
    }

    #[test]
    fn filtering_and_policy_gating_work_across_all_8_areas() {
        use mu_console_protocol::dto::{
            CdEcosystem, CdRunDto, CdStepDto, IngressRouteDto, ModelRunnerConnectionDto,
            ModelRunnerStatusDto, PlatformSnapshotDto, PlatformUserDto, ResidentModelStatusDto,
            RoleActivationDto, RoleFolioIndexDto, RoleSpecDto, WireGuardPeerSummaryDto,
        };
        use mu_console_protocol::{
            CanaryBundle, CanaryBundleState, CdLease, DataStoreHealth, DeploymentGate, GateTest,
            HardwareMetricSeries, LogSeverity, ObserveView, OtelLogEntry, PurePathTrace,
            RegistryPackage, RegistryView, RepoScore, SandboxProfileSummary, SmartscapeNode,
            SubsystemHealthEntry, SubsystemHealthSummary, SubsystemStatus,
        };

        let all_policies: Vec<String> = [
            "system:dashboard.read",
            "ci:queue.read",
            "ci:jobs.read",
            "ci:jobs.logs.read",
            "ci:nodes.read",
            "ci:sandboxes.read",
            "ci:repos.public.read",
            "cd:runs.read",
            "cd:steps.read",
            "ab:slots.read",
            "ab:gate.logs.read",
            "ab:leases.read",
            "reg:packages.read",
            "reg:canaries.read",
            "reg:cas.read",
            "obs:smartscape.read",
            "obs:traces.read",
            "obs:problems.read",
            "obs:logs.read",
            "obs:metrics.read",
            "obs:store.read",
            "obs:ingress.read",
            "mr:models.read",
            "mr:wireguard_vpn.read",
            "platform:maintenance.read",
            "platform:users.read",
            "platform:users.write",
            "platform:roles.read",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();

        let mut vm = ViewModel {
            auth: AuthState {
                status: AuthStatus::Active,
                github_login: Some("ops".into()),
                passkey_count: 1,
                active_policies: all_policies,
                ..AuthState::default()
            },
            repos: vec![RepoScore::new(RepoId::new("LMA1980", "alpha"))],
            ci_jobs: vec![
                CiJobSummary {
                    job_id: 1,
                    repo: RepoId::new("LMA1980", "alpha"),
                    ref_name: "main".into(),
                    queue_state: "running".into(),
                    ..CiJobSummary::default()
                },
                CiJobSummary {
                    job_id: 2,
                    repo: RepoId::new("LMA1980", "alpha"),
                    ref_name: "feat/y".into(),
                    queue_state: "failed".into(),
                    raw_log_lines: vec!["line one ok".into(), "line two alpha panic".into()],
                    ..CiJobSummary::default()
                },
            ],
            nodes: vec![
                FabricNode {
                    id: "node-a".into(),
                    online: true,
                    arch: "x86_64".into(),
                    max_slots: 1,
                    ..FabricNode::default()
                },
                FabricNode {
                    id: "node-b".into(),
                    online: false,
                    arch: "riscv64".into(),
                    max_slots: 1,
                    ..FabricNode::default()
                },
            ],
            sandboxes: vec![SandboxProfileSummary {
                profile_name: "default_ci".into(),
                backend: "MuIsolation".into(),
                max_pids: 256,
                max_slots: 1,
                thread_budget: 4,
                hardware_target: "x86_64".into(),
            }],
            cd_runs: vec![CdRunDto {
                run_id: "cd-1".into(),
                ecosystem: CdEcosystem::Cargo,
                repo: RepoId::new("LMA1980", "alpha"),
                ref_name: "main".into(),
                state: CdRunState::Succeeded,
                steps: vec![CdStepDto {
                    step_id: "pub".into(),
                    label: "Publish Crate".into(),
                    state: CdRunState::Succeeded,
                    ..CdStepDto::default()
                }],
                ..CdRunDto::default()
            }],
            gate: Some(DeploymentGate {
                deployment_id: "dep-1".into(),
                repo: RepoId::new("LMA1980", "alpha"),
                service: "gw".into(),
                artifact_digest: "sha256:abcd1234".into(),
                target_node: "node-a".into(),
                staged_slot: Slot::B,
                restore_slot: Slot::A,
                lease_expires_epoch_ms: 99_999,
                outcome: GateOutcome::Pending,
                rollback_reason: None,
                tainted: false,
                tests: vec![GateTest {
                    name: "it_smoke".into(),
                    critical: true,
                    state: TestState::Passed,
                    duration_ms: 50,
                    crate_name: None,
                    log_excerpt: Some("boot ok\nhealth ok".into()),
                }],
            }),
            leases: vec![CdLease {
                lease_id: "lease-1".into(),
                deployment_id: "dep-1".into(),
                target_node: "node-a".into(),
                service: "gw".into(),
                artifact_digest: "sha256:abcd1234".into(),
                active_slot: Slot::A,
                candidate_slot: Slot::B,
                ttl_remaining_ms: 30_000,
                promoted: false,
            }],
            registry: RegistryView {
                packages: vec![RegistryPackage {
                    ecosystem: PackageEcosystem::Cargo,
                    name: "mu-core".into(),
                    latest_version: "0.2.0".into(),
                    versions_count: 1,
                    sha256_prefix: "deadbeef12".into(),
                    deps_count: 2,
                    yanked: false,
                    tainted: true,
                    taint_reason: None,
                    versions: vec![],
                }],
                canaries: vec![CanaryBundle {
                    bundle_id: "cny-1".into(),
                    repo: RepoId::new("LMA1980", "alpha"),
                    service: "gw".into(),
                    artifact_digest: "sha256:abcd".into(),
                    state: CanaryBundleState::Tainted,
                    taint_reason: None,
                    deployment_id: "dep-1".into(),
                    trace_id: "tr-1".into(),
                }],
                cas: mu_console_protocol::CasHealth {
                    blob_count: 12,
                    corrupt_blobs: 0,
                    ..mu_console_protocol::CasHealth::default()
                },
            },
            observe: ObserveView {
                smartscape_nodes: vec![
                    SmartscapeNode {
                        id: "svc-gw".into(),
                        label: "gw".into(),
                        layer: SmartscapeLayer::Service,
                        req_per_sec_bp: 100,
                        error_rate_bp: 0,
                        p95_ms: 10,
                        healthy: true,
                    },
                    SmartscapeNode {
                        id: "host-a".into(),
                        label: "node-a".into(),
                        layer: SmartscapeLayer::Host,
                        req_per_sec_bp: 100,
                        error_rate_bp: 0,
                        p95_ms: 10,
                        healthy: true,
                    },
                ],
                traces: vec![PurePathTrace {
                    trace_id: "tr-1".into(),
                    root_service: "gw".into(),
                    root_operation: "GET /".into(),
                    domain: "edge".into(),
                    duration_ms: 12,
                    error: false,
                    slow_p99: false,
                    started_epoch_ms: 1,
                    spans: vec![],
                }],
                problems: vec![SloProblem {
                    problem_id: "prob-1".into(),
                    title: "High latency".into(),
                    severity: "critical".into(),
                    impacted_entity: "gw".into(),
                    active: true,
                    ..SloProblem::default()
                }],
                logs: vec![OtelLogEntry {
                    timestamp_ms: 1,
                    severity: LogSeverity::Info,
                    service: "gw".into(),
                    span_id: None,
                    trace_id: "tr-1".into(),
                    message: "ready".into(),
                }],
                metrics: vec![HardwareMetricSeries {
                    service: "gw".into(),
                    hardware_class: "x86_64".into(),
                    rps: 10,
                    error_rate_bp: 0,
                    p50_ms: 5,
                    p95_ms: 10,
                    p99_ms: 20,
                    exemplar_trace_id: "tr-1".into(),
                    samples: vec![5, 10],
                }],
                store: DataStoreHealth {
                    hot_duckdb_rows: 42,
                    ..DataStoreHealth::default()
                },
                ingress_routes: vec![IngressRouteDto {
                    route_id: "rt-gw".into(),
                    path_prefix: "/api".into(),
                    upstream_url: "http://192.0.2.5:8080".into(),
                    service_name: "gw".into(),
                    strip_prefix: false,
                    auth_required: true,
                    request_count: 1,
                    request_rate_rps: 1.0,
                    error_rate_pct: 0.0,
                    p95_latency_ms: 5.0,
                }],
                ..ObserveView::default()
            },
            model_runner: mu_console_protocol::ModelRunnerView {
                status: Some(ModelRunnerStatusDto {
                    total_queue_depth: 1,
                    models: vec![ResidentModelStatusDto {
                        model_id: "qwen-7b".into(),
                        cluster: "A100".into(),
                        is_alive: true,
                        ..ResidentModelStatusDto::default()
                    }],
                    ..ModelRunnerStatusDto::default()
                }),
                connection: Some(ModelRunnerConnectionDto {
                    peers: vec![WireGuardPeerSummaryDto {
                        public_key_prefix: "peer-abc".into(),
                        allowed_ips: "192.0.2.9/32".into(),
                        latest_handshake_epoch_ms: Some(1_000),
                        ..WireGuardPeerSummaryDto::default()
                    }],
                    ..ModelRunnerConnectionDto::default()
                }),
            },
            platform: PlatformSnapshotDto {
                users: vec![
                    PlatformUserDto {
                        github_login: "sole-admin".into(),
                        bound_roles: vec!["platform-admin".into()],
                        passkey_count: 1,
                        is_platform_admin: true,
                        ..PlatformUserDto::default()
                    },
                    PlatformUserDto {
                        github_login: "viewer".into(),
                        bound_roles: vec!["operator".into()],
                        passkey_count: 1,
                        is_platform_admin: false,
                        ..PlatformUserDto::default()
                    },
                ],
                roles: vec![RoleSpecDto {
                    id: "platform-admin".into(),
                    display_name: Some("Platform Admin".into()),
                    activation: RoleActivationDto::timed(900, 300),
                    ..RoleSpecDto::default()
                }],
                role_folio_index: RoleFolioIndexDto {
                    anonymous: vec![],
                    role_folios: BTreeMap::from([(
                        "platform-admin".to_owned(),
                        vec!["folio:platform:all".to_owned()],
                    )]),
                },
                ..PlatformSnapshotDto::default()
            },
            subsystem_health: SubsystemHealthSummary {
                ci: SubsystemHealthEntry {
                    status: SubsystemStatus::Live,
                    summary: "SECRET_SUMMARY_ONLY_IF_PERMITTED".into(),
                    ..SubsystemHealthEntry::default()
                },
                ..SubsystemHealthSummary::default()
            },
            ..ViewModel::default()
        };
        let mut st = test_state_with_vm("#/ci/logs", vm.clone());

        // CI queue filter: running vs failed + log line filtering fallback.
        st.activate_target("ci-queue-filter:running");
        let sc_run = build_console_scene(&st, SceneViewport::DESKTOP);
        let SceneComponent::TableGrid(q_run) = &sc_run.sections[0].components[1] else {
            panic!("expected ci-queue-table");
        };
        assert_eq!(q_run.rows.len(), 1);
        assert_eq!(q_run.rows[0].id, "ci-job-1");

        st.activate_target("ci-queue-filter:failed");
        st.set_filter_query("feat/y", 10);
        let sc_branch = build_console_scene(&st, SceneViewport::DESKTOP);
        let SceneComponent::LogViewer(lv_all) = &sc_branch.sections[0].components[3] else {
            panic!("expected LogViewer for branch filter");
        };
        assert_eq!(lv_all.entries.len(), 2);

        st.set_filter_query("alpha", 11);
        let sc_fail = build_console_scene(&st, SceneViewport::DESKTOP);
        let SceneComponent::LogViewer(lv) = &sc_fail.sections[0].components[3] else {
            panic!("expected LogViewer");
        };
        assert_eq!(lv.entries.len(), 1);
        assert_eq!(lv.entries[0].line_no, Some(2));

        // Observability: Smartscape layer filter + service filter + Ingress table.
        st.set_filter_query("", 20);
        st.select_subview(SubView::Observability(ObservabilitySubView::Smartscape));
        st.activate_target("obs-smartscape-filter:hosts");
        let sc_hosts = build_console_scene(&st, SceneViewport::DESKTOP);
        assert!(sc_hosts
            .sections
            .iter()
            .any(|s| s.id == "sec-obs-service-filter"));
        let hosts_sec = sc_hosts
            .sections
            .iter()
            .find(|s| s.id == "sec-observe")
            .expect("sec-observe");
        let SceneComponent::TableGrid(ss_hosts) = &hosts_sec.components[1] else {
            panic!("expected smartscape table");
        };
        assert_eq!(ss_hosts.rows.len(), 1);
        assert_eq!(ss_hosts.rows[0].id, "host-a");

        st.activate_target("obs-smartscape-filter:all");
        st.activate_target("obs-service-next");
        st.select_subview(SubView::Observability(ObservabilitySubView::Ingress));
        let sc_ing = build_console_scene(&st, SceneViewport::DESKTOP);
        let ing_sec = sc_ing
            .sections
            .iter()
            .find(|s| s.id == "sec-observe")
            .expect("sec-observe");
        let SceneComponent::TableGrid(ing_tbl) = &ing_sec.components[1] else {
            panic!("expected obs-ingress-table");
        };
        assert_eq!(ing_tbl.rows.len(), 1);
        assert_eq!(ing_tbl.rows[0].id, "ing-rt-gw");
        st.activate_target("obs-service-clear");
        assert!(st.obs_service_filter.is_none());

        // Registry: sha256_prefix + Tainted matching.
        st.select_subview(SubView::Repository(RepositorySubView::Packages));
        st.set_filter_query("deadbeef12 Tainted", 30);
        let sc_reg = build_console_scene(&st, SceneViewport::DESKTOP);
        let SceneComponent::TableGrid(pkg_tbl) = &sc_reg.sections[0].components[1] else {
            panic!("expected reg-packages-table");
        };
        assert_eq!(pkg_tbl.rows.len(), 1);

        // Platform: filtering down to sole-admin still preserves Last Admin guard, and plat-invite-creator hides without invites policy.
        st.select_subview(SubView::Platform(PlatformSubView::UsersAndInvites));
        st.set_filter_query("sole-admin", 40);
        let sc_plat = build_console_scene(&st, SceneViewport::DESKTOP);
        assert!(!sc_plat.sections[0].components.iter().any(
            |c| matches!(c, SceneComponent::PlatformPanel(p) if p.id == "plat-invite-creator")
        ));
        let SceneComponent::TableGrid(u_tbl) = &sc_plat.sections[0].components[0] else {
            panic!("expected plat-users-table");
        };
        assert_eq!(u_tbl.rows.len(), 1);
        assert_eq!(
            u_tbl.rows[0].row_action.as_ref().map(|a| a.label.as_str()),
            Some("Last Admin")
        );

        // Dashboard: unpermitted card detail must never match filter_query.
        vm.auth.active_policies = vec!["system:dashboard.read".into()];
        let mut st_unperm = test_state_with_vm("#/dashboard/overview", vm);
        st_unperm.set_filter_query("SECRET_SUMMARY_ONLY_IF_PERMITTED", 50);
        let sc_unperm = build_console_scene(&st_unperm, SceneViewport::DESKTOP);
        let SceneComponent::StatusMatrix(mat) = &sc_unperm.sections[0].components[0] else {
            panic!("expected dash-subsystem-cards");
        };
        assert!(mat.cells.is_empty());
    }

    #[test]
    fn observability_logs_scene_filters_by_query_service_severity_and_maps_tone() {
        let mut st = ConsoleState::new();
        st.init_from_env("#/observability/logs", "", None);

        let vm = ViewModel {
            auth: AuthState {
                status: AuthStatus::Active,
                github_login: Some("ops".into()),
                active_policies: vec!["system:dashboard.read".into(), "obs:logs.read".into()],
                ..AuthState::default()
            },
            observe: mu_console_protocol::ObserveView {
                logs: vec![
                    OtelLogEntry {
                        timestamp_ms: 1_001,
                        severity: LogSeverity::Info,
                        service: "muregist".into(),
                        trace_id: "tr-reg-1".into(),
                        span_id: None,
                        message: "CAS upload ready".into(),
                    },
                    OtelLogEntry {
                        timestamp_ms: 1_002,
                        severity: LogSeverity::Error,
                        service: "mu-registry".into(),
                        trace_id: "tr-reg-2".into(),
                        span_id: None,
                        message: "HTTP 500 error".into(),
                    },
                    OtelLogEntry {
                        timestamp_ms: 1_003,
                        severity: LogSeverity::Warn,
                        service: "mu-ci-controller".into(),
                        trace_id: "tr-ci-1".into(),
                        span_id: None,
                        message: "queue backpressure".into(),
                    },
                    OtelLogEntry {
                        timestamp_ms: 1_004,
                        severity: LogSeverity::Info,
                        service: "mu-console".into(),
                        trace_id: "tr-web-1".into(),
                        span_id: None,
                        message: "HTTP 200 ok".into(),
                    },
                ],
                ..mu_console_protocol::ObserveView::default()
            },
            ..ViewModel::default()
        };
        st.on_worker_frame(&encode(&DataEvent::Snapshot(vm)).expect("encode"));

        let get_entries = |state: &ConsoleState| -> Vec<LogEntry> {
            let scene = build_console_scene(state, SceneViewport::DESKTOP);
            let sec = scene
                .sections
                .iter()
                .find(|s| s.id == "sec-observe")
                .expect("sec-observe");
            let SceneComponent::LogViewer(viewer) = &sec.components[0] else {
                panic!("expected LogViewer");
            };
            viewer.entries.clone()
        };

        // Unfiltered: all 4 rows with mapped severity tones.
        let all = get_entries(&st);
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].level, SemanticTone::Info);
        assert_eq!(all[1].level, SemanticTone::Fail);
        assert_eq!(all[2].level, SemanticTone::Warn);
        assert_eq!(all[3].level, SemanticTone::Info);

        // Canonical alias filtering matches both `muregist` and `mu-registry`.
        st.set_filter_query("muregist", 1_000);
        let reg = get_entries(&st);
        assert_eq!(reg.len(), 2);
        assert_eq!(reg[0].text, "[muregist] CAS upload ready");
        assert_eq!(reg[1].text, "[mu-registry] HTTP 500 error");
        assert_eq!(reg[0].line_no, Some(1));
        assert_eq!(reg[1].line_no, Some(2));

        // Multi-term alias + message/severity query narrows further.
        st.set_filter_query("muregist error", 1_100);
        let reg_err = get_entries(&st);
        assert_eq!(reg_err.len(), 1);
        assert_eq!(reg_err[0].text, "[mu-registry] HTTP 500 error");
        assert_eq!(reg_err[0].level, SemanticTone::Fail);

        // Trace ID matching works in filter_query.
        st.set_filter_query("tr-ci-1", 1_200);
        let by_trace = get_entries(&st);
        assert_eq!(by_trace.len(), 1);
        assert_eq!(by_trace[0].text, "[mu-ci-controller] queue backpressure");

        // log_severity_filter and obs_service_filter also filter rendered entries.
        st.set_filter_query("", 1_300);
        st.log_severity_filter = Some(LogSeverity::Warn);
        let warn_only = get_entries(&st);
        assert_eq!(warn_only.len(), 1);
        assert_eq!(warn_only[0].text, "[mu-ci-controller] queue backpressure");

        st.log_severity_filter = None;
        st.obs_service_filter = Some("muregist".into());
        let svc_only = get_entries(&st);
        assert_eq!(svc_only.len(), 2);
    }

    #[test]
    fn observability_omits_repo_selector_across_all_subviews() {
        let mut st = ConsoleState::new();
        st.init_from_env("#/observability/smartscape", "", None);
        let vm = ViewModel {
            auth: AuthState {
                status: AuthStatus::Active,
                github_login: Some("ops".into()),
                active_policies: vec![
                    "system:dashboard.read".into(),
                    "ci:queue.read".into(),
                    "ci:repos.public.read".into(),
                    "cd:runs.read".into(),
                    "ab:slots.read".into(),
                    "reg:packages.read".into(),
                    "obs:smartscape.read".into(),
                    "obs:traces.read".into(),
                    "obs:problems.read".into(),
                    "obs:logs.read".into(),
                    "obs:metrics.read".into(),
                    "obs:store.read".into(),
                    "obs:ingress.read".into(),
                    "mr:models.read".into(),
                    "platform:maintenance.read".into(),
                ],
                ..AuthState::default()
            },
            repos: vec![RepoScore::new(RepoId::new("LMA1980", "mu-registry"))],
            ..ViewModel::default()
        };
        st.on_worker_frame(&encode(&DataEvent::Snapshot(vm)).expect("encode vm"));

        for subview in ObservabilitySubView::ALL {
            let _ = st.select_subview(SubView::Observability(subview));
            let scene = build_console_scene(&st, SceneViewport::DESKTOP);
            assert!(
                scene.repo_selector.is_none(),
                "Observability subview {subview:?} must omit repo_selector"
            );
        }

        for hidden_area in [Area::Dashboard, Area::ModelRunner, Area::Platform] {
            let _ = st.select_area(hidden_area);
            let scene = build_console_scene(&st, SceneViewport::DESKTOP);
            assert!(
                scene.repo_selector.is_none(),
                "{hidden_area:?} must omit repo_selector"
            );
        }

        for visible_area in [Area::Ci, Area::Cd, Area::AbDeploy, Area::Repository] {
            let _ = st.select_area(visible_area);
            let scene = build_console_scene(&st, SceneViewport::DESKTOP);
            assert!(
                scene.repo_selector.is_some(),
                "{visible_area:?} must include repo_selector"
            );
        }
    }

    #[test]
    fn obs_logs_viewer_preserves_server_matches_window_line_numbers_anchors_and_pagination() {
        let mut st = ConsoleState::new();
        st.init_from_env("#/observability/logs?filter=worker-riscv64", "", None);
        st.pending_filter_at_ms = None;

        let vm = ViewModel {
            auth: AuthState {
                status: AuthStatus::Active,
                github_login: Some("ops".into()),
                active_policies: vec!["system:dashboard.read".into(), "obs:logs.read".into()],
                ..AuthState::default()
            },
            observe: mu_console_protocol::ObserveView {
                logs: vec![OtelLogEntry {
                    timestamp_ms: 1_794_000_000_500,
                    severity: LogSeverity::Warn,
                    service: "actionrunner-rs".into(),
                    trace_id: "tr-srv-1".into(),
                    span_id: Some("sp-worker-riscv64".into()),
                    message: "Heartbeat tick".into(),
                }],
                log_window: mu_console_protocol::ObsLogWindowMeta {
                    window_start_index: 500,
                    cached_total: 1_200,
                    total_matched: Some(3_450),
                    has_older: true,
                    has_newer: true,
                    loading_older: false,
                    loading_newer: false,
                    search_active: true,
                    pending_live_count: 4,
                },
                ..mu_console_protocol::ObserveView::default()
            },
            ..ViewModel::default()
        };
        st.on_worker_frame(&encode(&DataEvent::Snapshot(vm)).expect("encode vm"));

        let scene = build_console_scene(&st, SceneViewport::DESKTOP);
        let sec = scene
            .sections
            .iter()
            .find(|s| s.id == "sec-observe")
            .expect("sec-observe");
        let SceneComponent::LogViewer(viewer) = &sec.components[0] else {
            panic!("expected LogViewer");
        };

        // Server-returned hit must NOT be dropped even though `worker-riscv64` wasn't in message/service.
        assert_eq!(viewer.entries.len(), 1);
        assert_eq!(viewer.entries[0].line_no, Some(501));
        assert!(viewer.entries[0].anchor_key.is_some());
        assert!(viewer.title.contains("501"));
        assert!(viewer.title.contains("3450"));
        assert!(viewer
            .controls
            .iter()
            .any(|c| c.control_id == "obs-logs-latest" && c.label == "↑ Latest (+4)"));
        let pg = viewer.pagination.as_ref().expect("window pagination");
        assert_eq!(pg.prev.control_id, "obs-logs-newer");
        assert!(!pg.prev.disabled);
        assert_eq!(pg.next.control_id, "obs-logs-older");
        assert!(!pg.next.disabled);
        assert_eq!(pg.total_items, Some(3_450));
    }

    #[test]
    fn obs_logs_anchor_uniqueness_prefilter_line_numbers_and_no_stale_flash() {
        let mut st = ConsoleState::new();
        st.init_from_env("#/observability/logs", "", None);

        let dup = OtelLogEntry {
            timestamp_ms: 1_794_000_000_700,
            severity: LogSeverity::Info,
            service: "muregist".into(),
            trace_id: "tr-dup-1".into(),
            span_id: Some("sp-dup".into()),
            message: "identical burst line".into(),
        };
        let target_row = OtelLogEntry {
            timestamp_ms: 1_794_000_000_699,
            severity: LogSeverity::Error,
            service: "mu-ci-controller".into(),
            trace_id: "tr-err-1".into(),
            span_id: Some("sp-err".into()),
            message: "old-query-stale-row".into(),
        };
        let vm = ViewModel {
            auth: AuthState {
                status: AuthStatus::Active,
                github_login: Some("ops".into()),
                active_policies: vec!["system:dashboard.read".into(), "obs:logs.read".into()],
                ..AuthState::default()
            },
            observe: mu_console_protocol::ObserveView {
                logs: vec![dup.clone(), dup.clone(), dup, target_row],
                log_window: mu_console_protocol::ObsLogWindowMeta {
                    window_start_index: 200,
                    cached_total: 800,
                    search_active: true,
                    ..mu_console_protocol::ObsLogWindowMeta::default()
                },
                ..mu_console_protocol::ObserveView::default()
            },
            ..ViewModel::default()
        };
        st.on_worker_frame(&encode(&DataEvent::Snapshot(vm)).expect("encode vm"));

        let get_viewer = |state: &ConsoleState, skin: ConsoleSkin| -> LogViewer {
            let scene = build_console_scene_for_skin(skin, state, SceneViewport::DESKTOP);
            let sec = scene
                .sections
                .iter()
                .find(|s| s.id == "sec-observe")
                .expect("sec-observe");
            let SceneComponent::LogViewer(viewer) = &sec.components[0] else {
                panic!("expected LogViewer");
            };
            viewer.clone()
        };

        // Identical same-ms lines produce distinct per-occurrence anchor_keys (`-0`, `-1`, `-2`).
        let all = get_viewer(&st, ConsoleSkin::Html);
        assert_eq!(all.entries.len(), 4);
        let keys: HashSet<&str> = all
            .entries
            .iter()
            .filter_map(|e| e.anchor_key.as_deref())
            .collect();
        assert_eq!(keys.len(), 4);
        assert_ne!(all.entries[0].anchor_key, all.entries[1].anchor_key);
        assert_ne!(all.entries[1].anchor_key, all.entries[2].anchor_key);

        // Client filter keeps true window line_no (200 + raw_idx 3 + 1 = 204) and accurate title.
        st.log_severity_filter = Some(LogSeverity::Error);
        let err_only = get_viewer(&st, ConsoleSkin::Html);
        assert_eq!(err_only.entries.len(), 1);
        assert_eq!(err_only.entries[0].line_no, Some(204));
        assert!(err_only.title.contains("1 matched in window 201–204"));
        st.log_severity_filter = None;

        // Enter (`submit_filter_now`) while `window.search_active == true` keeps client query active
        // until the worker replies, avoiding stale-row flashes.
        st.set_filter_query("nonexistent-query", 1_000);
        let cmd = st.submit_filter_now();
        assert!(matches!(cmd, Some(UiCommand::Subscribe(_))));
        assert_eq!(st.pending_filter_at_ms, None);
        assert!(st.awaiting_obs_search_reply);
        let in_flight = get_viewer(&st, ConsoleSkin::Html);
        assert!(in_flight.entries.is_empty());
        assert!(in_flight.title.contains("0 matching"));
    }

    #[test]
    fn obs_logs_wireframe_subpaging_steps_32_line_pages_without_affecting_html() {
        let mut st = ConsoleState::new();
        st.init_from_env("#/observability/logs", "", None);
        st.skin = ConsoleSkin::Wireframe;

        let logs: Vec<OtelLogEntry> = (0..500_u64)
            .map(|i| OtelLogEntry {
                timestamp_ms: 1_794_000_500_000 - i,
                severity: LogSeverity::Info,
                service: "muregist".into(),
                trace_id: format!("tr-wf-{i:03}"),
                span_id: None,
                message: format!("line-{i:03}"),
            })
            .collect();
        let vm = ViewModel {
            auth: AuthState {
                status: AuthStatus::Active,
                github_login: Some("ops".into()),
                active_policies: vec!["system:dashboard.read".into(), "obs:logs.read".into()],
                ..AuthState::default()
            },
            observe: mu_console_protocol::ObserveView {
                logs,
                log_window: mu_console_protocol::ObsLogWindowMeta {
                    window_start_index: 0,
                    cached_total: 1_200,
                    has_older: true,
                    has_newer: false,
                    ..mu_console_protocol::ObsLogWindowMeta::default()
                },
                ..mu_console_protocol::ObserveView::default()
            },
            ..ViewModel::default()
        };
        st.on_worker_frame(&encode(&DataEvent::Snapshot(vm)).expect("encode vm"));

        // HtmlCoreSkin renders the full 500-entry scrollable window.
        let html_scene =
            build_console_scene_for_skin(ConsoleSkin::Html, &st, SceneViewport::DESKTOP);
        let SceneComponent::LogViewer(html_viewer) = &html_scene
            .sections
            .iter()
            .find(|s| s.id == "sec-observe")
            .expect("sec-observe")
            .components[0]
        else {
            panic!("expected LogViewer");
        };
        assert_eq!(html_viewer.entries.len(), 500);

        // WireframeSkin slices into 32-line sub-pages (16 total across 500 lines).
        let wf_viewer = |state: &ConsoleState| -> LogViewer {
            let sc =
                build_console_scene_for_skin(ConsoleSkin::Wireframe, state, SceneViewport::DESKTOP);
            let SceneComponent::LogViewer(v) = &sc
                .sections
                .iter()
                .find(|s| s.id == "sec-observe")
                .expect("sec-observe")
                .components[0]
            else {
                panic!("expected LogViewer");
            };
            v.clone()
        };

        let p0 = wf_viewer(&st);
        assert_eq!(p0.entries.len(), WF_OBS_LOGS_PAGE_SIZE);
        assert_eq!(p0.entries[0].line_no, Some(1));
        assert_eq!(p0.entries[31].line_no, Some(32));
        let pg0 = p0.pagination.as_ref().expect("pagination");
        assert!(pg0.prev.disabled);
        assert!(!pg0.next.disabled);

        // Local sub-page step emits no worker command until the 500-line boundary.
        assert!(st.activate_target("obs-logs-older").is_empty());
        assert_eq!(st.subview_page, 1);
        let p1 = wf_viewer(&st);
        assert_eq!(p1.entries[0].line_no, Some(33));
        assert_eq!(p1.entries[31].line_no, Some(64));
        let pg1 = p1.pagination.as_ref().expect("pagination");
        assert!(!pg1.prev.disabled);
        assert!(!pg1.next.disabled);

        for _ in 0..14 {
            assert!(st.activate_target("obs-logs-older").is_empty());
        }
        assert_eq!(st.subview_page, 15);
        let p15 = wf_viewer(&st);
        assert_eq!(p15.entries.len(), 20);
        assert_eq!(p15.entries[0].line_no, Some(481));
        assert_eq!(p15.entries[19].line_no, Some(500));
        assert_eq!(
            st.activate_target("obs-logs-older"),
            vec![UiCommand::LoadOlderObsLogs]
        );

        assert_eq!(
            st.activate_target("obs-logs-latest"),
            vec![UiCommand::JumpToLatestObsLogs]
        );
        assert_eq!(st.subview_page, 0);
    }

    #[test]
    fn ci_scorecards_render_full_catalog_in_html_and_align_repo_select_with_ordered_carousel() {
        use mu_console_protocol::{RepoId, RepoScore};

        let mut st = ConsoleState::new();
        st.init_from_env("#/ci/scorecards", "", None);
        st.skin = ConsoleSkin::Html;

        let mut repos: Vec<RepoScore> = (0..12_usize)
            .map(|i| {
                let mut r = RepoScore::new(RepoId::new("LMA1980", &format!("repo-{i:02}")));
                r.status = RepoStatus::Passing;
                r.score = 95;
                r
            })
            .collect();
        // Place a failing repository late alphabetically so ordered_carousel_repos() promotes it
        // ahead of filtered_repos()[0].
        repos[10].status = RepoStatus::Failing;
        repos[10].score = 40;

        let vm = ViewModel {
            auth: AuthState {
                status: AuthStatus::Active,
                github_login: Some("ops".into()),
                active_policies: vec![
                    "system:dashboard.read".into(),
                    "ci:queue.read".into(),
                    "ci:jobs.read".into(),
                    "ci:repos.public.read".into(),
                ],
                ..AuthState::default()
            },
            repos,
            ..ViewModel::default()
        };
        st.on_worker_frame(&encode(&DataEvent::Snapshot(vm)).expect("encode vm"));

        // HtmlCoreSkin renders all 12 scorecards; WireframeSkin slices 8 per carousel page.
        let html_scene =
            build_console_scene_for_skin(ConsoleSkin::Html, &st, SceneViewport::DESKTOP);
        let html_sec = html_scene
            .sections
            .iter()
            .find(|s| s.id == "sec-ci-scorecards")
            .expect("sec-ci-scorecards");
        assert_eq!(html_sec.components.len(), 12);

        st.skin = ConsoleSkin::Wireframe;
        let wf_p0 =
            build_console_scene_for_skin(ConsoleSkin::Wireframe, &st, SceneViewport::DESKTOP);
        assert_eq!(
            wf_p0
                .sections
                .iter()
                .find(|s| s.id == "sec-ci-scorecards")
                .expect("sec-ci-scorecards")
                .components
                .len(),
            8
        );
        st.carousel_page = 1;
        let wf_p1 =
            build_console_scene_for_skin(ConsoleSkin::Wireframe, &st, SceneViewport::DESKTOP);
        assert_eq!(
            wf_p1
                .sections
                .iter()
                .find(|s| s.id == "sec-ci-scorecards")
                .expect("sec-ci-scorecards")
                .components
                .len(),
            4
        );

        // Activating repo-select:0 selects ordered_carousel_repos()[0] (the failing repo-10).
        let ordered_first = st.ordered_carousel_repos()[0].id.clone();
        assert_eq!(ordered_first, RepoId::new("LMA1980", "repo-10"));
        let cmds = st.activate_target("repo-select:0");
        assert_eq!(
            cmds,
            vec![UiCommand::SelectRepo {
                repo: RepoId::new("LMA1980", "repo-10"),
                submodule: None,
            }]
        );
        assert_eq!(st.route.repo, Some(RepoId::new("LMA1980", "repo-10")));
        assert_eq!(st.outer_index, 10);

        let selected_scene =
            build_console_scene_for_skin(ConsoleSkin::Html, &st, SceneViewport::DESKTOP);
        let selected_sec = selected_scene
            .sections
            .iter()
            .find(|s| s.id == "sec-ci-scorecards")
            .expect("sec-ci-scorecards");
        let SceneComponent::SparklineCard(first_card) = &selected_sec.components[0] else {
            panic!("expected SparklineCard");
        };
        assert_eq!(first_card.title, "LMA1980/repo-10");
        assert_eq!(
            first_card.action.as_ref().and_then(|a| a.pressed),
            Some(true)
        );
    }
}
