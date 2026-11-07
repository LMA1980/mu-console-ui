// SPDX-License-Identifier: AGPL-3.0-only
//! Main-thread mirror of the data worker's view model.
//!
//! Pure Rust (tested natively): decodes worker frames, applies snapshots
//! and patches, tracks connection state and badges, and detects protocol
//! version mismatches for the "reload required" banner. Phase 3 components
//! bind to [`DataClient`]; Phase 2 only logs it.

use mu_console_protocol::{
    apply_all, decode, encode, AreaBadges, CodecError, ConnState, DataEvent, DataSource, UiCommand,
    UiError, ViewModel,
};

/// Root-absolute URL of the worker loader shim emitted by Trunk or build-dist.sh
/// (`data-type="worker" data-loader-shim`).
pub const WORKER_URL: &str = "/muconsole-data_loader.js";

/// What changed after a frame was handled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientUpdate {
    Ready(DataSource),
    View,
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
            Err(e) => ClientUpdate::Error(UiError::Decode(e.to_string())),
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
                self.view = Some(v);
                ClientUpdate::View
            }
            DataEvent::Patch(p) => {
                if let Some(v) = self.view.as_mut() {
                    apply_all(v, &p);
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
            DataEvent::Error(e) => ClientUpdate::Error(e),
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
            fleet: 1,
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
}
