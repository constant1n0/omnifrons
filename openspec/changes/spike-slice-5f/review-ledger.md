# Review Ledger: spike-slice-5f

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

## CI Corrective Round 1 — Unit 1

| id | location | severity | status | evidence |
|---|---|---|---|---|
| R3-101 | `local_staging_cleanup.rs:42-48` | BLOCKER | verified | Fresh scoped re-review verified the cfg-only fix; cross-Windows clippy and focused tests pass. |
| R3-102 | `local_staging_cleanup.rs:235-240` | BLOCKER | verified | Scoped review #8776 verified the millisecond fixture; fresh Windows CI pending. |

Correction budget: 2/2 used; no source scope beyond R3-102.
