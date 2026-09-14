# Review Ledger: spike-slice-5f

## Apply Batch 4 — Locked Publication Activation

JUDGMENT: APPROVED. Both blind judges completed one exhaustive sweep of the 140-line activation diff. Neither found a BLOCKER or CRITICAL defect; Judge B's ledger is empty. Production ordering and lock lifetime are correct.

| id | lens | location | severity | status | evidence |
|---|---|---|---|---|---|
| JD-A-801 | judgment-day | src-tauri/src/ipc/publication.rs:1805-1824 | WARNING | info | The invalid-root test's candidate is independently ineligible, so it does not isolate whether cleanup ran before root validation. Reported once; not a fix/re-review driver. |

Gate #9156 PASSED the scoped Phase 4 contract: `publishing_` 8/8, adapter `cleanup` 31/31, shell 236/236, adapter 205/205, workspace 820/820, Rust fmt/check/Linux clippy, Windows-GNU adapter check/clippy, and renderer test/lint/build all passed. The actual full candidate is 147 additions + 3 deletions = 150 lines, below the native 200-line cap and review 400-line cap. Native generation 8 / ordinal 8 finished `passed` with receipt `batch4-gate-finish-20260914-01` and runtime revision `sha256:fd093b19d416c6b451d2db53897b3cac12129f64ab8d98e6727a78bcd8bff7af`; ordinals 1–8 are preserved, native control is complete, and Batch 4 is pending delivery. Cleanup during tests is restricted to owned temporary fixtures. The SDD change remains 12/15 with tasks 5.1–5.3 pending the standard risk review and stacked delivery boundary; it is not archive-ready.

## Apply Batch 3c — Bound Provider Cleanup and FIFO Proof

JUDGMENT: APPROVED. Both blind judges completed one exhaustive sweep of the frozen 211-line change (208 additions, 3 deletions); both ledgers are empty (#9112, #9114). Gate #9117 PASSED: public cleanup 1/1, native 4/4, real `PublicationState` FIFO 1/1, adapter 201/201, shell 234/234, workspace 818/818, doctest 1/1, Linux and Windows-GNU adapter clippy/check, and fmt. The provider-bound API and real publication-mutex FIFO proof support Phase 3 completion without application activation.
Native generation 7 / ordinal 7 finished `passed` with receipt `batch3c-gate-finish-20260914-01` and revision `sha256:1bbba8d61e0967f0e75ebbd42fd8d2e9e5ca89f8106447ac8bd08a85feb85637`; ordinals 1–6 are preserved. Batch 3c is JD approved, gate passed, and pending delivery. Source mutations during verification were restricted to owned temporary test fixtures. Native Windows/macOS runtime CI remains deferred to later delivery work.

## Apply Batch 3b — Protected Native Unlink

JUDGMENT: APPROVED. JD-601 is verified by fresh scoped judges `batch3b-rejudge-a` and #9031; no new BLOCKER or CRITICAL finding was raised.

| id | lens | location | severity | status | evidence |
|---|---|---|---|---|---|
| JD-601 | judgment-day | crates/omnifrons-adapters/src/local_staging_cleanup.rs:348; crates/omnifrons-adapters/src/lib.rs:76-79 | CRITICAL | verified | Corrective round 1 narrows `cleanup_staging` to `pub(crate)` without changing unlink logic or the public read-only API. A public-module `compile_fail` doctest uses a function pointer only; RED proved the prior export compiled, GREEN passed after the boundary change, and an owned external compile-only fixture failed specifically with E0603. Fresh scoped judges `batch3b-rejudge-a` and #9031 verified the correction. |
| JD-602 | judgment-day | crates/omnifrons-adapters/src/local_staging_cleanup.rs:262-303,464-481 | WARNING | info | Both judges observed that final-recheck I/O errors collapse into false and are counted as retention rather than failures. Deletion stays fail-closed, but reporting may appear clean. Reported once; not a fix/re-review driver. |

The engine remains uninvoked by application flows. Native generation 6 / ordinal 6 finished `passed` with receipt `batch3b-gate-retry-9051-finish-20260914-01` and revision `c9c934288048a7e07b989c02103c8ce2d349ffe87f2ee7e419949a78f349b2d2`; ordinals 1–5 are preserved. Batch 3b corrective budget: one of at most two rounds used; judgment approved, pending delivery.

## Apply Batch 3a — Final Identity Rechecks

JUDGMENT: APPROVED. JD-501 is verified; independent Batch 3a gate #8939 passed and native runtime verification finished passed. Pending delivery.

| id | lens | location | severity | status | evidence |
|---|---|---|---|---|---|
| JD-501 | judgment-day | crates/omnifrons-adapters/src/local_staging_cleanup.rs:311-316,323-330 | BLOCKER | verified | Round 1 normalizes every compared `fstatat` field with `u64::try_from`, failing closed on conversion failure; Apple-width i32/u16 valid, negative, and overflow inputs have RED/GREEN coverage. Both fresh scoped judges approved: `batch3a-rejudge-a` and #8985. Independent gate #8939 passed; pending delivery. |
| JD-A-502 | judgment-day | crates/omnifrons-adapters/src/local_staging_cleanup.rs:827-964 | WARNING | info | Several negative tests unlink the original first; held nlink becomes zero, so earlier checks reject before final no-follow basename lookup. This limits the coverage those tests demonstrate. Reported once; not a fix/re-review driver. |
| JD-B-502 | judgment-day | crates/omnifrons-adapters/src/local_staging_cleanup.rs:936-945 | WARNING | info | The hard-link replacement test changes inode, so it does not isolate the case of adding a link to the originally held candidate. Reported once; not a fix/re-review driver. |
| JD-B-503 | judgment-day | crates/omnifrons-adapters/src/local_staging_cleanup.rs:895-904 | WARNING | info | The mtime-change test relies on a one-millisecond sleep and may be nondeterministic on coarse-resolution filesystems. Reported once; not a fix/re-review driver. |

Batch 3a remains uncommitted and nonmutating. Native generation 5 / ordinal 5 finished `passed` with receipt `batch3a-gate-finish-20260914-01` and revision `sha256:1beea844b71ff99f927d1cae561570dcd7cbab9247706505538c54a223ec6368`; ordinals 1–5 are preserved. Corrective budget: 1 of 2 fix rounds used; judgment approved, pending delivery. macOS CI remains required.

## Apply Batch 2c — Public Inert Inspection Entry

JUDGMENT: APPROVED. Both blind judges completed one exhaustive sweep; both ledgers are empty (#8897, #8899). Independent gate #8909 passed the read-only Phase 2 contract: public 2/2, adapter 192/192, workspace 804/804, Linux and Windows-GNU clippy/check, and fmt. No destructive behavior or application invocation is introduced.
Native generation 4, ordinal 4 finished `passed` with receipt `batch2c-gate-finish-20260913-01` and runtime revision `sha256:6aaf1e4113a05a693efed59d878585d7d5001fd5169aad2baa3f6c5f2552d1ca`; ordinals 1–3 remain preserved. Final identity rechecks, native deletion, and activation remain Phase 3/4 work.

## Apply Batch 2b — Private Retain-First Walker

JUDGMENT: APPROVED. Both blind judges completed one exhaustive sweep of the 332-line private-core diff; both findings ledgers are empty (#8863, #8868). No corrective round was needed. Independent automatic gate #8843 passed: focused 18/18, workspace 802/802, Linux and Windows-GNU all-target/all-feature clippy/check, and fmt.
Native generation 3, ordinal 3 is finished `passed` with receipt `batch2b-gate-finish-20260913-01` and runtime revision `sha256:eb2d617c9fb411eabda8f3c25cc1bc6878e964a33aaee0398463ca934bd56c0f`; old ordinals 1/2 are preserved and `complete=true`, `next_action=complete`. Windows evidence is compile-only, not a native runtime claim.
This approves Batch 2b private core pending delivery only. Broader Phase 2 remains partial: public integration, final identity rechecks, native deletion, and activation remain deferred; whole tasks 2.1–2.3 stay unchecked.

## Apply Batch 2c — Public Inert Inspection Entry

Historical pre-gate record: the public adapter entry delegates only to the read-only retain-first walker; no stage, publication, IPC, UI, deletion, or final identity recheck path invokes it. Phase 2 task completion is limited to inert inspection; Phase 3 and 4 obligations remain deferred.

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
