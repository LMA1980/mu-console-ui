// SPDX-License-Identifier: AGPL-3.0-only
//! Main-thread UI binary entry point. Spawns the data worker and wires the
//! message bridge on `wasm32`.

fn main() {
    #[cfg(target_arch = "wasm32")]
    mu_console_ui::boot::start();
}
