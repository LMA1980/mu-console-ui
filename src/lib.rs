// SPDX-License-Identifier: AGPL-3.0-only
//! MuConsole UI application shell.
//!
//! Renders the skin-agnostic scene projection via either
//! [`mu_console_theme::html::HtmlCoreSkin`] (default) or
//! [`mu_console_theme::wireframe::WireframeSkin`], connects to the Web Worker
//! over `mu-console-protocol`, tracks interactive client state cleanly, and
//! binds DOM/WebAuthn interactions on `wasm32`.

#![forbid(unsafe_code)]

#[cfg(target_arch = "wasm32")]
pub mod boot;
pub mod client;
pub mod components;
pub mod copy;
pub mod roles;
pub mod router;
pub mod scene;
pub mod state;
pub mod webauthn;

pub use components::{
    filter_placeholder, render_console, render_console_with_skin, render_html_shell,
};
pub use copy::HTML_LANG;
pub use mu_console_layout::{Frames, SafeArea, Viewport};
pub use mu_console_protocol::{ConsoleSkin, DialogModel};
pub use roles::format_active_roles;
pub use state::{
    ConsoleState, PlatformFormState, WebAuthnRequest, FONT_STORAGE_KEY, PENDING_HASH_STORAGE_KEY,
    PENDING_INVITE_STORAGE_KEY, REPO_STORAGE_KEY, SKIN_STORAGE_KEY,
};

/// Compute pixel frames (`C1..C3`, bars, canvas) for `state` at `vp` / `safe`.
#[must_use]
pub fn frames(state: &ConsoleState, vp: Viewport, safe: SafeArea) -> Frames {
    mu_console_layout::solve(vp, safe, state.route.area, state.font_metrics())
}
