# mu-console-ui

Unified CI/CD console web frontend for MuConsole: WireframeSkin SVG renderer, HTML accessibility shell, deep-link hash router, and WASM bootstrapper.

## Architecture

- **WireframeSkin SVG Renderer (`components`)**: Deterministic technical wireframe SVG renderer with dark slate frames, cyan accents, and modular instrument panels.
- **HTML Accessibility Shell (`components`)**: Accessible 100dvh flex column layout, `.skip-link`, and 16px iOS safe query filter input.
- **Deep-Link Hash Router (`router`)**: Fragment-based hash navigation (`#c1/c2`), breadcrumbs, and hierarchy collapsing.
- **Console State Machine (`state`)**: Interactive control dispatch (rotaries, annunciator lamps, switches), font selection (`Hurmit` and `OpenDyslexicM`), and view model synchronization.
- **Main-Thread WASM Bootstrapper (`boot`)**: Spawns dedicated data Web Worker, transfers zero-copy `ArrayBuffer` postcard messages, and manages browser event delegation.

## Testing & Verification

```bash
cargo test --all-targets
cargo check --target wasm32-unknown-unknown
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

## License

`mu-console-ui` is licensed under the GNU Affero General Public License version 3.0 (`AGPL-3.0-only`). See [LICENSE](LICENSE).
