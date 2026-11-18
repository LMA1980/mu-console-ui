// SPDX-License-Identifier: AGPL-3.0-only
//! Main-thread mirror of the data worker's view model.
//!
//! Pure Rust (tested natively): decodes worker frames, applies snapshots
//! and patches, tracks connection state and badges, and detects protocol
//! version mismatches for the "reload required" banner. Phase 3 components
//! bind to [`DataClient`]; Phase 2 only logs it.

use mu_console_protocol::{
    apply_all, decode, encode, AreaBadges, AuthState, CodecError, ConnState, DataEvent, DataSource,
    Delta, DialogModel, RoleLeaseView, SessionRolesDialogModel, UiCommand, UiError, ViewModel,
};

/// Root-absolute URL of the worker loader shim emitted by Trunk or build-dist.sh
/// (`data-type="worker" data-loader-shim`).
pub const WORKER_URL: &str = "/muconsole-data_loader.js";

/// What changed after a frame was handled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientUpdate {
    Ready(DataSource),
    View,
    /// Pure lease countdown / idle-timer tick with zero structural auth or dialog changes.
    LeaseTimers,
    Connection(ConnState),
    Badges(AreaBadges),
    Heartbeat,
    /// Protocol versions differ: show the reload banner.
    ReloadRequired {
        expected: u32,
        found: u32,
    },
    Error(UiError),
}

/// UI-side state fed by the worker.
#[derive(Debug, Default)]
pub struct DataClient {
    view: Option<ViewModel>,
    conn: Option<ConnState>,
    badges: AreaBadges,
    reload_required: Option<(u32, u32)>,
    last_error: Option<UiError>,
    last_frame_closed_dialog: bool,
    frames: u32,
}

impl DataClient {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Current view model (after the first snapshot).
    #[must_use]
    pub fn view(&self) -> Option<&ViewModel> {
        self.view.as_ref()
    }

    /// Mutable access to the current view model for local lease countdowns.
    pub fn view_mut(&mut self) -> Option<&mut ViewModel> {
        self.view.as_mut()
    }

    #[must_use]
    pub fn conn(&self) -> Option<ConnState> {
        self.conn
    }

    #[must_use]
    pub fn badges(&self) -> AreaBadges {
        self.badges
    }

    /// `(expected, found)` once a version mismatch was seen.
    #[must_use]
    pub fn reload_required(&self) -> Option<(u32, u32)> {
        self.reload_required
    }

    /// Most recent non-version worker/decode error, cleared once auth resolves or on retry.
    #[must_use]
    pub fn last_error(&self) -> Option<&UiError> {
        self.last_error.as_ref()
    }

    /// Clear any latched worker error (e.g. when retrying `/api/auth/session`).
    pub fn clear_last_error(&mut self) {
        self.last_error = None;
    }

    /// Return `true` when the most recent worker frame explicitly cleared `active_dialog` via
    /// `Delta::ActiveDialog(None)`.
    #[must_use]
    pub const fn last_frame_closed_dialog(&self) -> bool {
        self.last_frame_closed_dialog
    }

    /// Encode a command for the worker.
    ///
    /// # Errors
    /// Propagates [`CodecError::Encode`].
    pub fn encode_command(cmd: &UiCommand) -> Result<Vec<u8>, CodecError> {
        encode(cmd)
    }

    /// Handle one frame from the worker.
    pub fn on_frame(&mut self, frame: &[u8]) -> ClientUpdate {
        self.frames = self.frames.wrapping_add(1);
        match decode::<DataEvent>(frame) {
            Ok(ev) => self.on_event(ev),
            Err(CodecError::VersionMismatch { expected, found }) => self.mismatch(expected, found),
            Err(e) => {
                let err = UiError::Decode(e.to_string());
                self.last_error = Some(err.clone());
                ClientUpdate::Error(err)
            }
        }
    }

    fn mismatch(&mut self, expected: u32, found: u32) -> ClientUpdate {
        self.reload_required = Some((expected, found));
        ClientUpdate::ReloadRequired { expected, found }
    }

    fn on_event(&mut self, ev: DataEvent) -> ClientUpdate {
        match ev {
            DataEvent::Ready { source, .. } => ClientUpdate::Ready(source),
            DataEvent::Snapshot(v) => {
                self.last_frame_closed_dialog = false;
                if v.auth.is_resolved() {
                    self.last_error = None;
                }
                self.view = Some(v);
                ClientUpdate::View
            }
            DataEvent::Patch(p) => {
                self.last_frame_closed_dialog =
                    p.iter().any(|d| matches!(d, Delta::ActiveDialog(None)));
                if let Some(v) = self.view.as_mut() {
                    let lease_timers_only = is_lease_timer_only_patch(v, &p);
                    apply_all(v, &p);
                    if v.auth.is_resolved() {
                        self.last_error = None;
                    }
                    if lease_timers_only {
                        return ClientUpdate::LeaseTimers;
                    }
                }
                ClientUpdate::View
            }
            DataEvent::Connection(c) => {
                self.conn = Some(c);
                ClientUpdate::Connection(c)
            }
            DataEvent::AreaBadges(b) => {
                self.badges = b;
                ClientUpdate::Badges(b)
            }
            DataEvent::Heartbeat { .. } => ClientUpdate::Heartbeat,
            DataEvent::Error(UiError::VersionMismatch { expected, found }) => {
                self.mismatch(expected, found)
            }
            DataEvent::Error(e) => {
                self.last_error = Some(e.clone());
                ClientUpdate::Error(e)
            }
        }
    }

    /// One-line summary for the console log.
    #[must_use]
    pub fn summary(&self) -> String {
        let Some(v) = self.view.as_ref() else {
            return "no snapshot yet".to_owned();
        };
        let sel = v.selection.as_ref().map_or_else(
            || "-".to_owned(),
            |s| format!("{}/{}", s.repo.owner, s.repo.repo),
        );
        let topo = v.topology.as_ref().map_or_else(
            || "no topology".to_owned(),
            |t| {
                format!(
                    "{} sidings, {} trains, FABRIC REV {}{}",
                    t.branches.len(),
                    t.trains.len(),
                    t.fabric_rev,
                    if t.stop() { ", STOP" } else { "" }
                )
            },
        );
        let gate = v.gate.as_ref().map_or_else(
            || "no gate".to_owned(),
            |g| format!("IT {}/{} {:?}", g.passed(), g.total(), g.outcome),
        );
        format!(
            "[{:?}] {} repos, sel {sel}: {topo}; {gate}; conn {:?}; badges {:?}{}",
            v.source,
            v.repos.len(),
            self.conn,
            self.badges,
            v.stale_since_epoch_ms
                .map_or_else(String::new, |t| format!("; STALE since {t}")),
        )
    }
}

/// Banner text for a protocol version mismatch.
#[must_use]
pub fn reload_banner_text(expected: u32, found: u32) -> String {
    format!(
        "MuConsole was updated (protocol {found}, this page speaks {expected}). Reload required."
    )
}

fn leases_match_modulo_timers(a: &RoleLeaseView, b: &RoleLeaseView) -> bool {
    a.role_id == b.role_id
        && a.display_name == b.display_name
        && a.activated_at_ms == b.activated_at_ms
        && a.hard_expires_at_ms == b.hard_expires_at_ms
        && a.hard_ttl_secs == b.hard_ttl_secs
        && a.idle_ttl_secs == b.idle_ttl_secs
        && a.renew_window_secs == b.renew_window_secs
        && a.renew_extend_secs == b.renew_extend_secs
        && a.max_renewals == b.max_renewals
        && a.renewals_used == b.renewals_used
        && a.can_renew == b.can_renew
        && a.is_expired() == b.is_expired()
}

fn auth_matches_modulo_timers(old: &AuthState, new: &AuthState) -> bool {
    old.status == new.status
        && old.github_login == new.github_login
        && old.passkey_count == new.passkey_count
        && old.passkey_enrollment_required == new.passkey_enrollment_required
        && old.reason_code == new.reason_code
        && old.anonymous_lockdown == new.anonymous_lockdown
        && old.baseline_role == new.baseline_role
        && old.bound_roles == new.bound_roles
        && old.bound_role_specs == new.bound_role_specs
        && old.catalog_role_specs == new.catalog_role_specs
        && old.active_folios == new.active_folios
        && old.active_policies == new.active_policies
        && old.leases.len() == new.leases.len()
        && old
            .leases
            .iter()
            .zip(&new.leases)
            .all(|(a, b)| leases_match_modulo_timers(a, b))
        && old.active_roles.len() == new.active_roles.len()
        && old
            .active_roles
            .iter()
            .zip(&new.active_roles)
            .all(|(a, b)| {
                a.role_id == b.role_id
                    && a.display_name == b.display_name
                    && a.activation == b.activation
                    && match (&a.lease, &b.lease) {
                        (None, None) => true,
                        (Some(la), Some(lb)) => leases_match_modulo_timers(la, lb),
                        _ => false,
                    }
            })
}

fn session_roles_match_modulo_timers(
    old: &SessionRolesDialogModel,
    new: &SessionRolesDialogModel,
) -> bool {
    old.github_login == new.github_login
        && old.lock_all_enabled == new.lock_all_enabled
        && old.error_message == new.error_message
        && old.roles.len() == new.roles.len()
        && old.roles.iter().zip(&new.roles).all(|(a, b)| {
            a.role_id == b.role_id
                && a.display_name == b.display_name
                && a.description == b.description
                && a.activation == b.activation
                && a.step_up == b.step_up
                && a.active == b.active
                && a.step_up_verified == b.step_up_verified
                && a.unassigned == b.unassigned
                && match (&a.lease, &b.lease) {
                    (None, None) => true,
                    (Some(la), Some(lb)) => leases_match_modulo_timers(la, lb),
                    _ => false,
                }
        })
}

fn is_lease_timer_only_patch(prev: &ViewModel, patch: &[Delta]) -> bool {
    if patch.is_empty() {
        return false;
    }
    let mut cur_auth = &prev.auth;
    let mut cur_dlg = prev.active_dialog.as_ref();
    for d in patch {
        match d {
            Delta::Auth(next_auth) => {
                if !auth_matches_modulo_timers(cur_auth, next_auth) {
                    return false;
                }
                cur_auth = next_auth;
            }
            Delta::ActiveDialog(Some(next_dlg @ DialogModel::SessionRoles(new_sr))) => {
                let Some(DialogModel::SessionRoles(old_sr)) = cur_dlg else {
                    return false;
                };
                if !session_roles_match_modulo_timers(old_sr, new_sr) {
                    return false;
                }
                cur_dlg = Some(next_dlg);
            }
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use mu_console_protocol::{Delta, FabricNode, PROTOCOL_VERSION};

    fn frame(ev: &DataEvent) -> Vec<u8> {
        encode(ev).expect("encode")
    }

    #[test]
    fn snapshot_then_patch_updates_the_mirror() {
        let mut c = DataClient::new();
        assert_eq!(c.summary(), "no snapshot yet");
        assert_eq!(
            c.on_frame(&frame(&DataEvent::Snapshot(ViewModel::default()))),
            ClientUpdate::View
        );
        c.on_frame(&frame(&DataEvent::Patch(vec![Delta::NodeUpsert(
            FabricNode::new("node-a", false),
        )])));
        assert_eq!(c.view().map(|v| v.nodes.len()), Some(1));
        c.on_frame(&frame(&DataEvent::Connection(ConnState::Live)));
        assert_eq!(c.conn(), Some(ConnState::Live));
        let b = AreaBadges {
            ci: 1,
            ..AreaBadges::default()
        };
        assert_eq!(
            c.on_frame(&frame(&DataEvent::AreaBadges(b))),
            ClientUpdate::Badges(b)
        );
        assert!(c.summary().contains("0 repos"));
    }

    #[test]
    fn version_mismatch_from_either_side_requires_reload() {
        let mut c = DataClient::new();
        let mut stale = postcard::to_allocvec(&(PROTOCOL_VERSION + 1)).expect("v");
        stale.extend(postcard::to_allocvec(&0_u8).expect("b"));
        assert_eq!(
            c.on_frame(&stale),
            ClientUpdate::ReloadRequired {
                expected: PROTOCOL_VERSION,
                found: PROTOCOL_VERSION + 1
            }
        );
        let mut c = DataClient::new();
        c.on_frame(&frame(&DataEvent::Error(UiError::VersionMismatch {
            expected: 9,
            found: PROTOCOL_VERSION,
        })));
        assert_eq!(c.reload_required(), Some((9, PROTOCOL_VERSION)));
        assert!(reload_banner_text(2, 3).contains("Reload required"));
        assert!(matches!(c.on_frame(&[]), ClientUpdate::Error(_)));
    }

    #[test]
    fn last_frame_closed_dialog_tracks_worker_active_dialog_none_patches() {
        let mut c = DataClient::new();
        assert!(!c.last_frame_closed_dialog());

        c.on_frame(&frame(&DataEvent::Snapshot(ViewModel::default())));
        assert!(!c.last_frame_closed_dialog());

        c.on_frame(&frame(&DataEvent::Patch(vec![Delta::ActiveDialog(None)])));
        assert!(c.last_frame_closed_dialog());

        c.on_frame(&frame(&DataEvent::Patch(vec![Delta::NodeUpsert(
            FabricNode::new("node-a", true),
        )])));
        assert!(!c.last_frame_closed_dialog());
    }

    #[test]
    fn lease_timer_only_patches_return_lease_timers_while_structural_changes_return_view() {
        use mu_console_protocol::{
            AuthStatus, RoleActivationDto, RoleBadgeView, SessionRoleDescriptor,
            SessionRolesDialogModel, StepUpRequirement,
        };

        let mut c = DataClient::new();
        let make_lease = |rem: u64, idle: u64, can_renew: bool| RoleLeaseView {
            role_id: "super-operator".into(),
            display_name: Some("Super Operator".into()),
            activated_at_ms: 0,
            last_activity_at_ms: 0,
            hard_expires_at_ms: 900_000,
            idle_expires_at_ms: idle * 1_000,
            hard_ttl_secs: 900,
            idle_ttl_secs: 120,
            renew_window_secs: 120,
            renew_extend_secs: 900,
            max_renewals: 1,
            renewals_used: 0,
            seconds_remaining: rem,
            idle_timer: idle,
            can_renew,
        };
        let make_auth = |rem: u64, idle: u64, can_renew: bool| {
            let lease = make_lease(rem, idle, can_renew);
            AuthState {
                status: AuthStatus::Active,
                github_login: Some("ops".into()),
                passkey_count: 1,
                active_roles: vec![RoleBadgeView {
                    role_id: "super-operator".into(),
                    display_name: Some("Super Operator".into()),
                    activation: RoleActivationDto::super_operator(),
                    lease: Some(lease.clone()),
                }],
                leases: vec![lease],
                active_policies: vec!["system:dashboard.read".into()],
                ..AuthState::default()
            }
        };
        let make_sr = |rem: u64, idle: u64, can_renew: bool| {
            DialogModel::SessionRoles(SessionRolesDialogModel {
                github_login: "ops".into(),
                roles: vec![SessionRoleDescriptor {
                    role_id: "super-operator".into(),
                    display_name: Some("Super Operator".into()),
                    description: "All-access Passkey role".into(),
                    activation: RoleActivationDto::super_operator(),
                    step_up: StepUpRequirement::Passkey,
                    active: true,
                    lease: Some(make_lease(rem, idle, can_renew)),
                    step_up_verified: true,
                    unassigned: false,
                }],
                lock_all_enabled: true,
                error_message: None,
            })
        };

        assert_eq!(
            c.on_frame(&frame(&DataEvent::Snapshot(ViewModel {
                auth: make_auth(900, 120, false),
                ..ViewModel::default()
            }))),
            ClientUpdate::View
        );

        // Pure 1s countdown patch -> LeaseTimers.
        assert_eq!(
            c.on_frame(&frame(&DataEvent::Patch(vec![Delta::Auth(make_auth(
                899, 119, false
            ))]))),
            ClientUpdate::LeaseTimers
        );

        // Activity touch extending idle_timer -> LeaseTimers.
        let mut act_auth = make_auth(899, 120, false);
        act_auth.leases[0].last_activity_at_ms = 1_500;
        act_auth.active_roles[0]
            .lease
            .as_mut()
            .unwrap()
            .last_activity_at_ms = 1_500;
        assert_eq!(
            c.on_frame(&frame(&DataEvent::Patch(vec![Delta::Auth(act_auth)]))),
            ClientUpdate::LeaseTimers
        );

        // Opening SessionRoles is a structural change -> View.
        assert_eq!(
            c.on_frame(&frame(&DataEvent::Patch(vec![Delta::ActiveDialog(Some(
                make_sr(899, 120, false)
            ))]))),
            ClientUpdate::View
        );
        // Subsequent countdown tick with SessionRoles open -> LeaseTimers.
        assert_eq!(
            c.on_frame(&frame(&DataEvent::Patch(vec![
                Delta::Auth(make_auth(898, 119, false)),
                Delta::ActiveDialog(Some(make_sr(898, 119, false))),
            ]))),
            ClientUpdate::LeaseTimers
        );

        // Entering renewal window (`can_renew` flips true) -> View.
        assert_eq!(
            c.on_frame(&frame(&DataEvent::Patch(vec![Delta::Auth(make_auth(
                120, 120, true
            ))]))),
            ClientUpdate::View
        );

        // Reaching 00:00 expiry -> View.
        assert_eq!(
            c.on_frame(&frame(&DataEvent::Patch(vec![Delta::Auth(make_auth(
                0, 0, false
            ))]))),
            ClientUpdate::View
        );
    }
}
