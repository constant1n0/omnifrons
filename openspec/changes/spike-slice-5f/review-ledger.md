# Review Ledger: spike-slice-5f

## Apply Batch 2b — Private Retain-First Walker

JUDGMENT: APPROVED. Both blind judges completed one exhaustive sweep of the 332-line private-core diff; both findings ledgers are empty (#8863, #8868). No corrective round was needed. Independent automatic gate #8843 passed: focused 18/18, workspace 802/802, Linux and Windows-GNU all-target/all-feature clippy/check, and fmt.
Native generation 3, ordinal 3 is finished `passed` with receipt `batch2b-gate-finish-20260913-01` and runtime revision `sha256:eb2d617c9fb411eabda8f3c25cc1bc6878e964a33aaee0398463ca934bd56c0f`; old ordinals 1/2 are preserved and `complete=true`, `next_action=complete`. Windows evidence is compile-only, not a native runtime claim.
This approves Batch 2b private core pending delivery only. Broader Phase 2 remains partial: public integration, final identity rechecks, native deletion, and activation remain deferred; whole tasks 2.1–2.3 stay unchecked.

## Design — Judgment Day, corrective round 1

Target: `design.md` and its proposal/specification contracts; no implementation exists.
First-pass dual review independently confirmed two critical findings; both fresh judges verified their round-1 corrections. Scoped re-review used only the ledger and fix diff.
Current state: JUDGMENT: APPROVED. Automatic design-gate refresh passed (#8528); implementation evidence is tracked in the batch sections below.

| id | lens | location | severity | status | evidence |
|---|---|---|---|---|---|
| JD-001 | judgment-day | openspec/changes/spike-slice-5f/design.md:13,26-29,33-36,68 | CRITICAL | verified | Both scoped judges verified checked strictly-positive signed pid_t conversion, Unknown/retain on failure, no invalid kill call, and explicit boundary regressions. |
| JD-002 | judgment-day | openspec/changes/spike-slice-5f/design.md:14,25-27,68 | CRITICAL | verified | Both scoped judges verified directory-constrained root opening, nonblocking/no-follow candidate opening before fstat, and the deterministic FIFO no-block regression requirement. |
| JD-B-003 | judgment-day | openspec/changes/spike-slice-5f/design.md:16,65 | WARNING | info | A repeated 256-entry scan has no progress mechanism; a stable prefix of final artifacts can indefinitely hide later staging files. Reported once; does not block or enter the fix/re-review loop. |
| JD-B-004 | judgment-day | openspec/changes/spike-slice-5f/design.md:13 | CRITICAL | refuted | Judge B alone claimed removing the duplicate signal-feature sentence made the design unimplementable. The original File Changes row for crates/omnifrons-adapters/Cargo.toml explicitly says Enable nix signal; that row was not changed by the supplied fix diff. This is a missing-context false positive, not a missing feature requirement; no fix is warranted. |

Fix budget: at most two rounds; one used. Scoped re-review must receive only this ledger and the round-1 fix diff, not the full original design.
Mirrors: Engram `sdd/spike-slice-5f/review-ledger`; design gate `sdd/spike-slice-5f/design-gate` (#8528).

## Planning Delivery — macOS CI

PR #16 at `541fad9880c0d04b71e3dd7130c1fbdcbeba2b60` is signed and GitHub-verified. Ubuntu, Windows, docs-links, and gitleaks passed; macOS failed. No merge or rerun-to-green occurred.
One reliability sweep and one general batched refuter independently confirmed the following pre-existing test-contract defect. The separate test-only PR #17 merged by normal fast-forward at `f1b9cfe2f48791c3aa4d29aa7a0fe98154e232bc`; GitHub signature and DCO verification passed, as did Ubuntu, macOS, Windows, docs-links, and gitleaks.

| id | lens | location | severity | status | evidence |
|---|---|---|---|---|---|
| R3-001 | reliability | crates/omnifrons-supervisor/tests/output_backpressure.rs:96-105 | BLOCKER | verified | Scoped review confirms the 92-line test-only correction preserves integration guarantees and deterministically proves saturation and refill/drop accounting. Independent execution audit selected and passed each of the two new unit tests and the integration test (1/1 each). Linux workspace tests, fmt and supervisor clippy also pass. Separate PR #17 is merged at the recorded SHA with all required cross-platform checks passing. |

Evidence: PR #17 https://github.com/constant1n0/omnifrons/pull/17, merged by normal fast-forward at `f1b9cfe2f48791c3aa4d29aa7a0fe98154e232bc`.

## Task Plan Delivery — Windows CI

Plan A replacement PR #18 is merged. Plan B PR #19 at `5d369c54078a12f61f779c93c467c49099507963` changes only the task plan; Ubuntu, macOS, docs-links, and gitleaks passed, but Windows failed. No rerun-to-green occurred.
One reliability sweep and one general batched refuter confirmed a separate pre-existing test-contract defect. Separate test-only PR #20 merged by normal fast-forward at `47204379f535c13037926467fef6d4d01e8dd722`; GitHub signature and DCO verification passed, as did Ubuntu, macOS, Windows, docs-links, and gitleaks.

| id | lens | location | severity | status | evidence |
|---|---|---|---|---|---|
| R3-002 | reliability | crates/omnifrons-supervisor/tests/bookkeeping_caps.rs:120-155 | BLOCKER | verified | A 30-line test-only correction preserves the 16/17 process-cap and OrphanRiskUncertain assertions, waits through the existing confirmed-terminal helper, and proves quota release with a new spawn. Linux focused bookkeeping tests passed 3/3; supervisor/workspace tests, fmt and clippy passed. No runtime changes; scoped review #8721 and all required cross-platform CI are verified. |

Evidence: PR #20 https://github.com/constant1n0/omnifrons/pull/20, merged by normal fast-forward at `47204379f535c13037926467fef6d4d01e8dd722`.

## Apply Batch 1 — Inert Safety Primitives

JUDGMENT: APPROVED. Two blind judges performed one exhaustive sweep each; neither found a BLOCKER or CRITICAL defect. Judge A's findings ledger is empty.
Automatic batch gate #8740 passed: nine focused tests, workspace tests, adapter checks/clippy, and formatting. Windows was compile-checked only; runtime CI remains required before delivery.

| id | lens | location | severity | status | evidence |
|---|---|---|---|---|---|
| JD-B-101 | judgment-day | openspec/changes/spike-slice-5f/apply-progress.md:11 | SUGGESTION | info | The progress table says five parser tests; the current module has four parser-focused functions on Unix and three on non-Unix. The nine-test total and recorded progression are otherwise consistent. Reported once; no fix or re-review is driven by this entry. |

No corrective round was needed. Tasks 1.1–1.3 are complete; scanning, filesystem deletion, provider/IPC activation, and all later phases remain unimplemented.
Standard risk review #8743: EMPTY; delivery is authorized with source/config unchanged.

## CI Corrections — Unit 1 (Closed)

| id | lens | location | severity | status | evidence |
|---|---|---|---|---|---|
| R3-101 | reliability | crates/omnifrons-adapters/src/local_staging_cleanup.rs:42-48 | BLOCKER | verified | Scoped review verified the cfg-only fix; cross-Windows clippy, focused tests, and final native CI passed. |
| R3-102 | reliability | crates/omnifrons-adapters/src/local_staging_cleanup.rs:235-240 | BLOCKER | verified | Scoped review #8776 verified the millisecond fixture; final native Windows CI passed before PR #22 integration. |

Correction budget: 2/2 used; no source scope beyond R3-102.
Delivery: PR #22 merged at `ef0b16cedec8fbd20a883402dd2f36e55f9ea6fd` with all five required checks passing.

## Apply Batch 2a — Initial Root/Candidate Evidence

JUDGMENT: APPROVED. Both fresh scoped judges verified the approved round-1 JD-201 correction; automatic gate retry #8791 passed and Batch 2a is pending delivery.

| id | lens | location | severity | status | evidence |
|---|---|---|---|---|---|
| JD-201 | judgment-day | crates/omnifrons-adapters/src/local_staging_cleanup.rs:109-195,399-446 | CRITICAL | verified | Round 1 retains owned root and candidate handles plus the initial root dev/ino/UID/mode snapshot. The real-FS regression replaces the root path and proves both handles remain bound to their admitted objects. Scoped reviews #8814 and `batch2a-rejudge-b` verified the correction. This is not a RootAsset logical binding, final recheck, walker, or deletion claim. |

Native attempt 2 is settled after the authorized append-only Phase 1 rollover: verified receipt `batch2a-gate-retry-8791-finish-20260913` recorded a passed result. Phase 1 attempt 1 remains immutable. This metadata update does not backdate a receipt.
Batch 2a corrective budget: at most two rounds; one used, scoped re-reviews and automatic runtime gate retry #8791 passed, pending delivery. No scanner, deletion, or application activation is present.
Standard risk review #8818: EMPTY; the 337-line Batch 2a diff is authorized for delivery.
