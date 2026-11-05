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
pub use state::ConsoleState;

use mu_console_layout::{solve, Area, Frames, SafeArea, Viewport};
use mu_console_theme::defs::write_defs;
use mu_console_theme::instruments::surface::{plate, wood};
use mu_console_theme::metrics::CrtFontMetrics;
use mu_console_theme::svg::SvgWriter;
use mu_console_theme::tokens::Effects;

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

/// Render the L0 static layer (wood + plates) for a frame set.
#[must_use]
pub fn static_layer(f: &Frames, effects: Effects) -> String {
    let mut w = SvgWriter::document(f.canvas.right(), f.canvas.bottom());
    write_defs(&mut w, effects);
    wood(&mut w, f.canvas.right(), f.canvas.bottom());
    for r in f.regions() {
        plate(&mut w, r, true);
    }
    w.finish_closing()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mu_console_theme::metrics::HURMIT;

    #[test]
    #[cfg_attr(miri, ignore)]
    fn static_layer_renders_every_region() {
        let f = frames(1512.0, 860.0, SafeArea::ZERO, Area::Ci, &HURMIT);
        let svg = static_layer(&f, Effects::LITE);
        assert!(svg.starts_with("<svg"));
        assert!(svg.ends_with("</svg>"));
        assert_eq!(
            svg.matches("filter=\"url(#mu-brushed)\"").count(),
            f.regions().len()
        );
    }
}
