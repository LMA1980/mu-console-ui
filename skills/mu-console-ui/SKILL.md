---
name: mu-console-ui
description: "Implement, test, and consume the mu-console-ui crate for unified console frontend rendering (`HtmlCoreSkin` and `WireframeSkin`), 8 role-gated areas, accessibility shell, and WASM bootstrapping."
---

# `mu-console-ui` Architecture & Guidelines

Use these guidelines when developing or integrating `mu-console-ui`, the unified console web frontend for MuConsole:

## 1. Dual-Skin Architecture & HTML Accessibility

- **Skin-Agnostic Scene Seam (`scene`, `copy`)**:
  - Build every screen and modal through `ConsoleScene` and `DialogModel` with centralized English copy from `copy.rs` (`lang="en"`).
  - Unauthenticated (`Anonymous`, `Unresolved`), `AuthorizationDenied`, and `PasskeyEnrollmentRequired` sessions are locked to `ConsoleSkin::Html` with `[Aa]` font toggle and `[Sign In]` / `[Sign Out]`.
  - Active operators (`AuthStatus::Active`) unlock `Html` / `Wireframe` selection via `?skin=` or `@user -> Preferences` while removing `[Aa]` from the active top bar (`@user`, `+role`, `[Sign Out]`).
- **Accessibility & Mobile Invariants (`components`)**:
  - Semantic `.skip-link` target (`#muconsole-dashboard`), `aria-live` polite/assertive status regions, and `font-size: 16px` across form inputs (`#muconsole-filter`, `.mu-input`) to prevent iOS Safari auto-zoom.
  - Horizontal scrollable `.mu-segmented` area/sub-view rails and `<dialog class="mu-dialog">` focus trapping.

## 2. Fail-Closed Authentication, Role Leases & Modals

- **Fail-Closed Boot & Invite Privacy (`state`, `boot`)**:
  - Cold boot (`AuthStatus::Unresolved`) disables all 8 areas with zero cached telemetry. Strip `?invite=` into `sessionStorage` immediately without disclosing invite status in anonymous or `AuthorizationDenied` UI.
  - Do not rely solely on `AuthState::is_authenticated()`; gate active-operator capabilities strictly on `auth.status == AuthStatus::Active`.
- **Passkey Onboarding, `+role` Leases & Risk Confirmation (`roles`, `state`)**:
  - Zero-Passkey bound sessions auto-open a dismissible guiding `PasskeyEnrollmentGate` (`attestation: "none"`, `userVerification: "preferred"`, no `authenticatorAttachment`) while preserving public Dashboard/CI navigation and `@user -> Passkeys` access unless `anonymous_lockdown` is enabled.
  - `+role[seconds_remaining][idle_timer]` adheres strictly to UTF-8 `chars().count()` bounds (`…(+N)` overflow), resets idle timers only on explicit `pointerdown`/`keydown` events, and expires only elapsed `Timed` leases on `00:00`.
  - Stateful/destructive actions (CI job restart, A|B rollback, Platform admin mutations) require `RiskConfirm` (`ResumeJob` stays 1-click); `Platform > Users & Invites` enforces the last-admin protection guard.

## 3. 8-Area Router, WASM Bootstrapper & Privacy Boundaries

- **Canonical Routing (`router`)**:
  - Route `#/{area}/{subview}?repo=...` across all 8 areas (`27` sub-views) and legacy aliases (`#/iam`, `#/fleet`, `#/cd/gate`, `#/registry`, `#/observe`, `#/ovr`). Keep unpermitted tabs/sub-views/cards rendered as `disabled` with zero navigation or OAuth redirect.
- **Public Repository Cleanliness**:
  - Documentation, logs, fixtures, and skills must never expose internal hostnames or private RFC1918 network ranges.
  - Use generic placeholders (`node-a`, `worker-riscv64`, `192.0.2.x`) across all tests and documentation.
