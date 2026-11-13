// SPDX-License-Identifier: AGPL-3.0-only
//! MuConsole main-thread UI module.
//!
//! The Leptos 0.8 CSR renderer, router and input handling land in Phase 3.
//! Phase 2 adds [`client::DataClient`] (main-thread mirror of the data
//! worker's view model) and [`boot`] (worker spawn + transferable
//! `ArrayBuffer` message bridge).
#![forbid(unsafe_code)]

#[cfg(target_arch = "wasm32")]
pub mod boot;
pub mod client;
pub mod components;
pub mod router;
pub mod state;

pub use client::{reload_banner_text, ClientUpdate, DataClient, WORKER_URL};
pub use components::{render_console, render_html_shell};
pub use state::{service_filter_matches, ConsoleState};

use mu_console_layout::{solve, Area, Frames, SafeArea, Viewport};
use mu_console_theme::metrics::CrtFontMetrics;

/// Solve the frames for the current viewport.
#[must_use]
pub fn frames(
    width: f64,
    height: f64,
    safe: SafeArea,
    area: Area,
    font: &CrtFontMetrics,
) -> Frames {
    solve(Viewport::new(width, height), safe, area, font)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mu_console_theme::metrics::HURMIT;

    #[test]
    fn frames_solves_desktop_viewport() {
        let f = frames(1512.0, 860.0, SafeArea::ZERO, Area::Ci, &HURMIT);
        assert!(f.canvas.right() > 0.0);
        assert!(f.canvas.bottom() > 0.0);
        assert!(!f.regions().is_empty());
    }
}
