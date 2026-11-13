______________________________________________________________________

## name: mu-console-ui description: "Implement, test, and consume the mu-console-ui crate for unified console frontend rendering, WireframeSkin, accessibility shell, and WASM bootstrapping."

# `mu-console-ui` Architecture & Guidelines

Use these guidelines when developing or integrating `mu-console-ui`, the unified CI/CD console web frontend for MuConsole:

## 1. Console Shell & HTML Accessibility

- **Zero-Leakage Viewport Layout**:
  - Root shell enforces `100dvh` flex-column hierarchy (`#muconsole-shell`).
  - Viewport layout solver computes non-overlapping pixel bounding boxes for all instruments, annunciators, and gauges.
- **Accessibility Invariants**:
  - Accessible `.skip-link` pointing directly to main dashboard content.
  - `#muconsole-filter` enforces a minimum `16px` font-size to prevent mobile iOS Safari auto-zoom triggers.
  - Proper ARIA live regions and annunciator status updates.

## 2. WireframeSkin & SVG Generation

- **WireframeSkin Aesthetics**:
  - Deterministic technical wireframe styling with dark slate frames, cyan accents, and modular instrument panels.
  - Parametric rendering of rotary knobs, rocker switches, LED annunciators, and yard trainway pipeline progression.
- **Typography & Dyslexic Mode**:
  - Pixel-perfect monospace alignment supporting both Hurmit and OpenDyslexicM font metrics.
  - Dynamic font toggling and persistence via browser storage keys.

## 3. WASM Bootstrapper & Worker Link

- **Main-Thread Lifecycle**:
  - Minimalist main-thread WASM entry point initializes DOM event delegates (`MouseEvent`, `KeyboardEvent`, `HashChangeEvent`, `WheelEvent`).
  - Dedicated Web Worker link transferring zero-copy `ArrayBuffer` postcard-encoded commands (`UiCommand`) and telemetry events (`DataEvent`).
- **Deep-Link Hash Router**:
  - URL fragment routing (`#c1/c2`) maintaining synchrony between browser history, breadcrumbs, and selected sub-views.

## 4. Privacy & Lab Boundary Invariants

- **Public Repository Cleanliness**:
  - Documentation, logs, and skills must never expose internal hostnames or private network ranges.
  - Use generic placeholders (`node-a`, `worker-riscv64`, `192.0.2.x`) across all tests and documentation.
