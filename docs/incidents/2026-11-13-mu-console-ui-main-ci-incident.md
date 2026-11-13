# Incident Report: CI Build Failure on `main` Branch in `mu-console-ui`

**Date**: 2026-11-13\
**Repository**: `LMA1980/mu-console-ui`\
**Affected Commit**: `a05d056f3b9e178357f843257c130ca9d8292cdb` (Pull Request #44)\
**Resolution Branch**: `feat/fix-main-ci-sibling-lockfile-refresh` (Pull Request linked to Issue #45)\
**Severity**: High (Continuous Integration blocking status on `main`)

______________________________________________________________________

## 1. Executive Summary

On 2026-11-13, following the merge of PR #44 ("feat(ui): render affected_services in WireframeSkin and integrate search filtering"), the automated `k3-ci` status check on the `main` branch of `mu-console-ui` reported a failure at commit `a05d056f3b9e178357f843257c130ca9d8292cdb`. The CI run (`job-1794433721-a05d056f`) failed during the `build-check` stage with `Tool 'cargo-build' exited with status code None`.

The root cause was traced to a desynchronization in `mu-console-ui/Cargo.lock`, where sibling git dependency `mu-console-data` was locked to an older commit (`a8080b76723519da9caf7bf2985814b61c095315`) while the upstream `mu-console-data` repository had advanced on `main` to `acd9f2364cbd2bd4cb2b1895646ad530123e7481`. When executed inside the offline `MuIsolation::KernelSandbox` (`network_egress_cidrs: []`), Cargo attempted remote network resolution for the superseded commit, hanging until supervisor termination.

______________________________________________________________________

## 2. Root Cause Analysis

In `Cargo.toml`, sibling components are specified with git repository references:

```toml
[dev-dependencies]
mu-console-data = { version = "0.1.1", git = "https://github.com/LMA1980/mu-console-data.git", branch = "main" }
```

In `mu-console-ui/Cargo.lock`:

```toml
[[package]]
name = "mu-console-data"
version = "0.1.1"
source = "git+https://github.com/LMA1980/mu-console-data.git?branch=main#a8080b76723519da9caf7bf2985814b61c095315"
```

### Sandbox Network Egress Boundary Violation

1. **Hermetic CI Environment**: ActionRunner worker nodes execute pipeline stages inside `MuIsolation::KernelSandbox` profiles where `network_egress_cidrs: []` strictly denies all outbound socket operations.
1. **Cache Eviction and Resolution Stall**: The worker node's shared Cargo cache in `/var/lib/action-runner/.cargo` had updated references tracking the active `main` HEAD of `mu-console-data` (`acd9f236`). When `cargo build --all-targets` executed for `mu-console-ui`, Cargo attempted to reconcile the pinned commit `a8080b76` by querying GitHub over HTTPS.
1. **Supervisor Termination (`exit code None`)**: Because outbound network egress is blocked by Landlock LSM and network namespaces, the Cargo git checkout process stalled indefinitely. Upon reaching the stage deadline, the supervisor terminated the child process with `SIGKILL`, resulting in an exit code of `None` and failing the CI stage.

______________________________________________________________________

## 3. Propagation to `main`

The pre-merge CI run for PR #44 (`job-1794429729-b2db8b61`) passed in 30 seconds because the worker assigned to that PR job still had the older git commit cached locally at the moment of PR verification. Following the PR merge, subsequent topological merges and cache refreshes across sibling repositories invalidated the local cache state, exposing the lockfile discrepancy during the post-merge push event on `main`.

______________________________________________________________________

## 4. Remediation

1. **Lockfile Synchronization**: Executed `cargo update -p mu-console-data` to update `Cargo.lock` to the active `origin/main` HEAD (`acd9f2364cbd2bd4cb2b1895646ad530123e7481`).
1. **Sibling Parity Verification**: Verified that all sibling git dependencies (`mu-console-protocol` at `862b1b89`, `mu-console-layout` at `1d8291d1`, and `mu-console-theme` at `6e0b1f93`) are aligned with their latest `main` branches.
1. **Local Pre-PR Gate Execution**: Ran the complete local CI validation suite (`cargo fmt`, `mdformat`, `cargo clippy`, `cargo test`, and `cargo miri`) cleanly before raising the fix PR.

______________________________________________________________________

## 5. Preventive Measures

1. **Pre-Merge Sibling Lockfile Verification**: Integrate a manifest check that verifies all sibling git dependencies declared in `Cargo.lock` match the current tip of `origin/main` before PR merges are authorized.
1. **Fast-Fail on Missing Offline Dependencies**: Ensure worker execution environments detect un-cached git dependencies prior to entering offline sandboxes, failing immediately with a clear diagnostics error rather than stalling on network I/O.
1. **Unified Topological Workspace Sync**: When performing cross-repository dependency merges, update both the umbrella workspace lockfiles and standalone crate lockfiles in the same release wave.
