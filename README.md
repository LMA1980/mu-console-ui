# mu-console-ui

Unified MuConsole web frontend featuring a skin-agnostic scene model rendered via `HtmlCoreSkin` (default) and `WireframeSkin`, fail-closed capability routing across 8 areas and 27 sub-views, WebAuthn Passkey onboarding, and zero-copy Web Worker IPC.

## Architecture

- **Skin-Agnostic Scene & Dialog Model (`scene`, `copy`)**: Projects `ViewModel`, `AuthState`, and `DialogModel` into neutral semantic primitives with centralized English UI copy (`lang="en"`), zero unauthenticated invite disclosure, and shared interactive semantics across both skins.
- **Dual-Skin Rendering (`components`)**:
  - **`HtmlCoreSkin` (`ConsoleSkin::Html`, default)**: Semantic HTML5 flex/grid natural-scroll layout (`min-height: 100dvh`), `<dialog class="mu-dialog">` modals, accessible `.skip-link`, ARIA live regions, and `16px` mobile-safe form inputs (`#muconsole-filter`).
  - **`WireframeSkin` (`ConsoleSkin::Wireframe`, opt-in)**: High-contrast fixed-viewport monochrome SVG projection unlocked on `AuthStatus::Active` via `@user -> Preferences` or `?skin=wireframe`.
- **Deep-Link Hash Router & Universal `disabled` Gating (`router`)**: Canonical `#/{area}/{subview}?repo=...&branch=...&job=...&trace=...&filter=...` routes plus legacy aliases (`#/iam`, `#/fleet`, `#/cd/gate`, `#/registry`, `#/observe`, `#/ovr`), `Escape` hierarchy unwind, and policy-gated `disabled` navigation elements.
- **Fail-Closed Console State, Role Leases & Confirmation Modals (`state`, `roles`)**:
  - Cold-boot `AuthStatus::Unresolved` disables all 8 areas with zero cached telemetry; `?invite=` tokens are stripped into `sessionStorage` without UI disclosure; `AuthorizationDenied` renders generic denial status without identity leakage.
  - Mandatory 0-Passkey gate (`DialogModel::PasskeyEnrollmentGate`) blocks console access until enrolled; `@user` profile modal exposes `User Details | Preferences | Passkeys`.
  - `+role` modal manages baseline, elective, and WebAuthn step-up `Timed` leases (`+role[MM:SS][MM:SS]` truncated safely on UTF-8 char boundaries) with explicit-activity idle resets (`pointerdown`/`keydown`), automatic partial lease expiration at `00:00`, and explicit `RiskConfirm` phrase/note modals.
- **Main-Thread WASM Bootstrapper (`boot`)**: Binds DOM delegates, manages `?invite=`/`?skin=`/`?font=` storage keys, executes WebAuthn ceremonies, and exchanges postcard frames with the `mu-console-data` Web Worker.

## Testing & Verification

```bash
cargo fmt --all -- --check
./scripts/format_docs.sh --check
cargo agents plugin validate Symposium.toml --no-check-crates
cargo clippy --all-targets -- -D warnings
cargo clippy --target wasm32-unknown-unknown --lib -- -D warnings
cargo test --all-targets
```

## License

`mu-console-ui` is licensed under the GNU Affero General Public License version 3.0 (`AGPL-3.0-only`). See [LICENSE](LICENSE).
