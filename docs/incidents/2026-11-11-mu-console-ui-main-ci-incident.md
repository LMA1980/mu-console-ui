# Incident Report: CI Test Failure on `main` Branch in `mu-console-ui`

**Date**: 2026-11-11\
**Repository**: `LMA1980/mu-console-ui`\
**Affected Commit**: `368930f7b5a66fc129ffb785f4c54628940d3bb0` (Pull Request #40)\
**Resolution Branch**: `feat/fix-main-ci-u24-dist-freshness` (Pull Request linked to Issue #41)\
**Severity**: High (Continuous Integration blocking pipeline on `main`)

______________________________________________________________________

## 1. Executive Summary

On 2026-11-11, following the merge of PR #40 ("chore(deps): align git deps to branch = main and add u24 freshness gate (#39)"), the automated CI pipeline on `main` failed at the `test-check` stage (`cargo-test` exited with code 101). The regression was traced to integration test `u24_dist_freshness_and_submodule_alignment`, which made an unconditional assumption that the aggregate umbrella repository `mu-console` and its built WebAssembly distribution artifacts (`dist/muconsole-ui_bg.wasm`) would always reside in a sibling directory relative to the repository manifest. When executed on isolated ActionRunner worker nodes where `mu-console-ui` is checked out in standalone isolation, path resolution failed, causing the test suite to panic.

______________________________________________________________________

## 2. Root Cause Analysis

In commit `368930f7b5a66fc129ffb785f4c54628940d3bb0`, integration test `u24_dist_freshness_and_submodule_alignment` was introduced in `tests/ux_gate.rs`:

```rust
let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
let repo_root = if manifest_dir
    .parent()
    .and_then(|p| p.file_name())
    .is_some_and(|n| n == "code")
{
    manifest_dir.parent().unwrap().parent().unwrap().to_path_buf()
} else {
    manifest_dir.parent().unwrap().join("mu-console")
};
let dist_dir = repo_root.join("dist");
let ui_wasm = dist_dir.join("muconsole-ui_bg.wasm");
assert!(ui_wasm.exists(), "dist/muconsole-ui_bg.wasm missing; run scripts/build-dist.sh");
```

### Path Resolution Inconsistency

1. **Local Developer Monorepo Workspace**: In local developer checkouts containing all submodules side-by-side (`/Users/lma1980/repository/mu-console-ui` and `/Users/lma1980/repository/mu-console`), `manifest_dir.parent().unwrap().join("mu-console")` resolves to the local `mu-console` repository, which contained pre-built `dist/muconsole-ui_bg.wasm` artifacts. The test therefore passed locally.
1. **Umbrella Crate CI Execution**: In the parent `mu-console` workspace, `mu-console-ui` is nested under `code/ui`. `manifest_dir.parent()` is `code`, so `repo_root` resolved to the root of `mu-console`, where `dist/` is present.
1. **Isolated Submodule CI Execution**: When `ActionRunner` checks out and tests `LMA1980/mu-console-ui` in an isolated job sandbox, only `mu-console-ui` is cloned. Sibling directory `../mu-console` does not exist on the runner node. As a result, `dist_dir.join("muconsole-ui_bg.wasm")` evaluated to a non-existent path, causing an assertion failure and terminating `cargo test` with exit status 101.

______________________________________________________________________

## 3. Propagation to `main`

The change was introduced via PR #40 to address Issue #39 ("Align git dependency branches and add distribution freshness gate"). The PR was merged while checks were still failing or had not completed verification on the target runner cluster, allowing the broken test logic to reach the `main` branch.

______________________________________________________________________

## 4. Remediation

1. **Standalone Detection & Graceful Fallback**: Modified `u24_dist_freshness_and_submodule_alignment` in `tests/ux_gate.rs` to detect when the umbrella distribution artifacts are absent (`!dist_dir.join("muconsole-ui_bg.wasm").exists()`).
1. **Local Artifact Integrity Verification**: In standalone checkouts, the test asserts the existence and non-emptiness of the local distribution file `dist/tokens.css` before cleanly returning.
1. **Full Invariant Preservation**: When running within the umbrella workspace or in an environment where `mu-console/dist` is present, the test continues to strictly enforce all four original verification phases:
   - Non-trivial size validation (>100KB) for `muconsole-ui_bg.wasm` and `muconsole-data_bg.wasm`.
   - JavaScript bindings and loader shim versioning verification.
   - Cache-busting query parameter assertions on `index.html`.
   - Git commit timestamp freshness comparison between distribution artifacts and source submodules.

______________________________________________________________________

## 5. Preventive Measures

1. **Hermetic Test Isolation**: Tests within submodule crates must never depend on paths outside their repository boundary (`env!("CARGO_MANIFEST_DIR")`) unless guarded by explicit capability or presence checks.
1. **Strict CI Merge Gate Enforcement**: Ensure that branch protection rules and merge workflows strictly require all status checks (`k3-ci` and `continuous integration`) to pass before any PR is merged to `main`.
1. **Multi-Context Test Matrix**: Automated PR verification must validate both standalone crate builds and composite umbrella workspace builds before green-lighting merges.
