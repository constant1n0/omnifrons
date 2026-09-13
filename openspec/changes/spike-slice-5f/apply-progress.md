# Apply Progress: spike-slice-5f — Batches 1 and 2a

## Completed
- [x] 1.1 Exact local staging basename parser and retention tables.
- [x] 1.2 Inert age, limits, and path-free report primitives.
- [x] 1.3 Safe PID liveness seams and real owned-child lifecycle coverage.

## TDD Cycle Evidence
| Task | Layer | RED | GREEN | REFACTOR |
|---|---|---|---|---|
| 1.1 | Unit | unresolved parser import | 3/3 | bounds/uppercase: 5 parser tests |
| 1.2 | Unit | unresolved age/report imports | 5/5 | named 24h limit |
| 1.3 | Unit | unresolved PID imports | 8/8 | 9/9 after probe mappings |

## Commands
- RED: `RUSTUP_AUTO_INSTALL=0 RUSTUP_NO_UPDATE_CHECK=1 /home/dcm/.cargo/bin/rustup run 1.98.1 cargo test -p omnifrons-adapters local_staging_cleanup` failed on unresolved parser, age/report, then PID imports.
- GREEN: the same command passed 3/3, 5/5, and 8/8; triangulation/refactor finished 9/9.
- Gates passed: `cargo fmt --all -- --check`, adapter clippy/check, and `cargo test --workspace --all-targets --all-features`.
- CI correction RED: Windows-target adapter clippy reproduced `derivable_impls` (exit 101).
- CI correction GREEN: cross-Windows and Linux adapter clippy, fmt, and the focused 9 tests passed; Windows runtime remains CI-only.
- R3-102: Windows-native RED was 30/31; the one-millisecond test fixture is locally green (9 focused); fresh Windows CI is required.

## Batch 1 Boundary
Batch 1 was inert: no `LocalDirBlobStore`, filesystem, IPC, UI, or deletion-path changes.
JD-001/JD-002 regressions are preserved; JD-B-003 remains an out-of-scope bounded-prefix limitation.
Current slice: stacked-to-main PR 1 from `origin/main`; rollback removes this inert module and nix `signal` feature.

## Batch 2a: Root and Candidate Evidence (Implemented, Pending Delivery)
**Objective**: add Unix-only, retain-first root and one-candidate handle evidence without a directory
walker, public `LocalDirBlobStore` hook, unlink, rename, or staging-path invocation.

**Why split**: the full Phase 2 scanner, cap accounting, public inert hook, and its integration tests
would exceed the 400-line chained-slice budget after Phase 1 measured 396 lines. This autonomous
subslice establishes the prerequisite handle-relative evidence only; tasks 2.1–2.3 remain unchecked
until their complete conditions are met.

**Base and delivery**: `origin/main` / PR #22 merged at
`ef0b16cedec8fbd20a883402dd2f36e55f9ea6fd`; new stacked-to-main branch
`spike/slice-5f-inspection`. The historical Unit 1 CI set passed (Ubuntu, macOS, Windows, gitleaks,
and docs-links). Unit 1's corrective budget is closed and is not reopened by Batch 2a.

**Corrective finalization**: JD-201 round 1 is verified by scoped reviews #8814 and
`batch2a-rejudge-b`; automatic gate retry #8791 passed. The correction retains initial
root and candidate handles and the root dev/ino/UID/mode snapshot only. It does not claim a
RootAsset logical binding, final recheck, walker, deletion, or activation.

### Batch 2a TDD Cycle Evidence
| Slice | Test file | Layer | Safety net | RED | GREEN | TRIANGULATE | REFACTOR |
|---|---|---|---|---|---|---|---|
| 2a root/candidate evidence | `local_staging_cleanup.rs` | Unit, real filesystem | 9/9 existing tests | unresolved `open_staging_evidence` | 12/12 focused tests | exact owner-only file; excluded, hard-linked, symbolic-link, directory, and permissive-mode cases | handle-relative `openat` with `O_NOFOLLOW|O_NONBLOCK`; no payload reads |
| 2a JD-201 retained evidence | `local_staging_cleanup.rs` | Unit, real filesystem | 12/12 existing tests | missing `root` and `handle` fields | 13/13 focused tests | owned fixture root-path replacement proves both held descriptors and immutable root identity | root ownership and initial metadata snapshot retained; formatter-only cleanup |

### Batch 2a Verification
- RED: `RUSTUP_AUTO_INSTALL=0 RUSTUP_NO_UPDATE_CHECK=1 /home/dcm/.cargo/bin/rustup run 1.98.1 cargo test -p omnifrons-adapters local_staging_cleanup::tests::opens_owner_only_root_and_records_regular_single_link_candidate_evidence` failed with `E0425` because `open_staging_evidence` did not exist.
- GREEN: the focused `local_staging_cleanup::tests::` command passed 12/12.
- Gates passed with the same pinned toolchain: `cargo fmt --all -- --check`; Linux and `x86_64-pc-windows-gnu` adapter `cargo clippy --all-targets --all-features -- -D warnings`; `cargo check --workspace`; and `cargo test --workspace --all-targets --all-features`.
- JD-201 RED: the focused root-replacement test failed with `E0609` because candidate-only evidence had no retained root or candidate handle.
- JD-201 GREEN: the focused `local_staging_cleanup::tests::` command passed 13/13; formatter, Linux and Windows GNU adapter clippy, and workspace check passed with the pinned toolchain.

### Batch 2a Runtime Receipt
- The authorized append-only rollover preserved completed Phase 1 attempt 1. Generation 2 objective `sha256:4958b15971075fade62fd0aadb9aeb964a1e6e24c0b384f4d9e4916ca8b28461`, ordinal 2, finished `passed` with verified receipt `batch2a-gate-retry-8791-finish-20260913` and evidence revision `sha256:e36a7b628b18b36ecb1372925b1877c77b42f1fc46d645f148aff36a25234139`.
- The settled runtime revision is `sha256:53f832c46237d09358632c21c6aaae4fe1b3247f8fcf5ed223bcbb9218129b40`; the native attempt is complete with no active attempt. This metadata update follows runtime verification and does not backdate a receipt.

### Batch 2a Safety Proof
- The production helper opens only an owner-only root and one exact basename using read-only,
  handle-relative `openat` with `O_NOFOLLOW|O_NONBLOCK`; it reads metadata only.
- It contains no `unlink`, `remove_file`, `rename`, directory walker, `LocalDirBlobStore` hook,
  staging invocation, IPC change, or path-bearing report. It remains deliberately uninvoked until
  the bounded scanner is implemented in the next slice.
- Tasks 2.1–2.3 remain unchecked: scanning, cap/count/truncation semantics, final identity
  rechecks, public inert entry, and all deletion/activation behavior are still pending.
