# mu-console-ui

Unified [`mu-console`](https://github.com/LMA1980/mu-console) web frontend featuring a skin-agnostic scene model rendered via `HtmlCoreSkin` (default) and `WireframeSkin`, fail-closed capability routing across 8 areas and 27 sub-views, WebAuthn Passkey onboarding, and zero-copy Web Worker IPC (`Protocol v8`).

## Architecture

- **Skin-Agnostic Scene & Dialog Model (`scene`, `copy`)**: Projects `ViewModel`, `AuthState`, and `DialogModel` into neutral semantic primitives with centralized English UI copy (`lang="en"`), zero unauthenticated invite disclosure, Dashboard section separation (`sec-dash-applications` subsystem matrix, `sec-dash-fleet` gated by `ci:nodes.read`, and `sec-dash-attention` on active sessions), and shared interactive semantics across both skins.
- **Dual-Skin Rendering (`components`)**:
  - **`HtmlCoreSkin` (`ConsoleSkin::Html`, default)**: Semantic HTML5 flex/grid natural-scroll layout (`min-height: 100dvh`), `<dialog class="mu-dialog">` modals, accessible `.skip-link`, ARIA live regions, and `16px` mobile-safe form inputs (`#muconsole-filter`).
  - **`WireframeSkin` (`ConsoleSkin::Wireframe`, opt-in)**: High-contrast fixed-viewport monochrome SVG projection unlocked on `AuthStatus::Active` via `@user -> Preferences` or `?skin=wireframe`.
- **Deep-Link Hash Router & Universal `disabled` Gating (`router`)**: Canonical `#/{area}/{subview}?repo=...&branch=...&job=...&trace=...&filter=...` routes (with `#/registry` as the canonical v8 route slug for `Area::Repository`) plus legacy aliases (`#/iam`, `#/fleet`, `#/cd/gate`, `#/repository`, `#/reg`, `#/observe`, `#/obs`, `#/ovr`), `Escape` hierarchy unwind, and policy-gated `disabled` navigation elements.
- **Fail-Closed Console State, Role Leases & Confirmation Modals (`state`, `roles`)**:
  - Cold-boot `AuthStatus::Unresolved` disables all 8 areas with zero cached telemetry; `?invite=` tokens are stripped into `sessionStorage` without UI disclosure; `AuthorizationDenied` renders generic denial status without identity leakage.
  - Guiding 0-Passkey onboarding modal (`DialogModel::PasskeyEnrollmentGate`) auto-opens when `passkey_count == 0` with dismissal to browse baseline public Dashboard/CI views while operator areas require an enrolled passkey; `@user` profile modal exposes `User Details | Preferences | Passkeys`.
  - `+role` modal renders `SessionRoleDescriptor` entries from `catalog_role_specs` (marking unbound catalog roles as `unassigned`), manages baseline, elective, and WebAuthn step-up `Timed` leases (`+role[MM:SS][MM:SS]` truncated safely on UTF-8 char boundaries) with explicit-activity idle resets (`pointerdown`/`keydown`), supports `super-operator` Passkey-confirmed final-2m lease renewal (`RenewRoleWithPasskey` via `/api/auth/roles/{role_id}/renew/start` and `/renew`), enforces automatic partial lease expiration at `00:00`, and gates destructive actions behind `RiskConfirm` phrase/note modals.
- **Main-Thread WASM Bootstrapper (`boot`)**: Binds DOM delegates, manages `?invite=`/`?skin=`/`?font=` storage keys, executes WebAuthn ceremonies, and exchanges postcard frames with the [`mu-console-data`](https://github.com/LMA1980/mu-console-data) Web Worker.

## Ecosystem Integration

- **Parent Workspace**: [`mu-console`](https://github.com/LMA1980/mu-console)
- **Sibling Crates**: [`mu-console-protocol`](https://github.com/LMA1980/mu-console-protocol), [`mu-console-theme`](https://github.com/LMA1980/mu-console-theme), [`mu-console-layout`](https://github.com/LMA1980/mu-console-layout), [`mu-console-data`](https://github.com/LMA1980/mu-console-data)
- **Identity & RBAC**: [`mu-auth`](https://github.com/LMA1980/mu-auth)

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
