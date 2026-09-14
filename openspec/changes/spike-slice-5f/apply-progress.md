# Apply Progress: spike-slice-5f — Batches 1, 2a–2c, and 3a–3c

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

## Batch 2b: Bounded Retain-First Walker (Implemented, Private-Core Gate Passed; Pending Delivery)
- Held-root `Dir::from_fd` listing admits the root once, streams at most 256 names, and opens
  candidates only relative to that root. Reports now count inspected, retained, removed, failures,
  and truncation without paths.
- Production is retain-only: `removed` remains zero and no unlink, rename, payload read, staging,
  IPC, or application hook was added. A private simulated-action seam proves the 32-action cap.
- Tasks 2.1–2.3 remain unchecked: integration/public `LocalDirBlobStore` entry, final identity
  rechecks/native unlink, FIFO coverage, and activation are deferred to dependency-closed slices.

### Batch 2b TDD Cycle Evidence
| Slice | Test file | Layer | Safety net | RED | GREEN | TRIANGULATE | REFACTOR |
|---|---|---|---|---|---|---|---|
| bounded retain-first walker | `local_staging_cleanup.rs` | Unit, real filesystem | 13/13 | missing `scan_staging_with` (E0425) | 14/14 focused | mixed/excluded, 32 simulated cap, 256 inspection cap, live/unknown, root failure | root admission and candidate open split; held-FD listing |

### Batch 2b Verification
- RED: pinned focused walker test failed with `E0425` for the missing walker; cap test then failed with missing action seam.
- GREEN: focused module tests passed 18/18; `cargo fmt --all -- --check`, Linux and Windows-GNU adapter clippy `-D warnings`, `cargo check --workspace`, and `cargo test --workspace --all-targets --all-features` passed.
- Windows evidence is compile-only; runtime evidence is Linux-only.

### Batch 2b Runtime Receipt
- Historical attempts 1 and 2 remain preserved. Generation 3, ordinal 3 is finished `passed` with
  receipt `batch2b-gate-finish-20260913-01` and runtime revision
  `sha256:eb2d617c9fb411eabda8f3c25cc1bc6878e964a33aaee0398463ca934bd56c0f`.
- Native control is complete (`complete=true`, `next_action=complete`); this is not a native Windows
  runtime claim. Batch 2b private-core is ready for standard risk review and remains pending delivery.

## Batch 2c: Public Inert Inspection Entry (Implemented, Gate Passed, Pending Delivery)
- `LocalDirBlobStore::inspect_abandoned_staging` delegates to the read-only walker; it is not called
  by open, stage, publication, IPC, or UI paths. Production still reports `removed: 0`.
- Public integration coverage proves retained exact/excluded entries and content, root-admission
  failure accounting, and the 256-entry truncated bound. Existing Batch2a/2b real-FS tests cover
  owner/mode, metadata, liveness, and private 32-policy seam without duplicating the walker.

### Batch 2c TDD Cycle Evidence
| Tasks | Test file | Layer | Safety net | RED | GREEN | TRIANGULATE | REFACTOR |
|---|---|---|---|---|---|---|---|
| 2.1–2.3 | `publication_fs.rs` | Integration, real filesystem | 18 private + 23 public | missing `inspect_abandoned_staging` (E0599) | 1/1 | retained/excluded/failure and truncated public cases: 2/2 | public wrapper only; no activation |

### Batch 2c Verification
- Independent gate #8909 passed the read-only Phase 2 contract: public integration 2/2, adapter tests 192/192, workspace 804/804, Linux and Windows-GNU clippy/check, and fmt. Windows evidence is compile-only.
- Phase 2 is complete as a read-only inspection capability. Phase 3 final rechecks/native unlink/FIFO evidence and Phase 4 activation remain deferred.

### Batch 2c Runtime Receipt
- The source and RED/GREEN work above predate native Batch 2c control; this receipt is recorded only after independent validation.
- Generation 4, ordinal 4 finished `passed` for objective `sha256:21e75779416a4b75551bc2d43660575431528c0acb2561bdf23a534ea3842e65` with receipt `batch2c-gate-finish-20260913-01` and runtime revision `sha256:6aaf1e4113a05a693efed59d878585d7d5001fd5169aad2baa3f6c5f2552d1ca`. Ordinals 1–3 remain preserved.

## Batch 3a: Final Identity Recheck Core (Implemented, Pending Independent Review)
- Private, non-mutating `final_identity_matches` now rejects a root path replacement that is a
  symlink even if it resolves to the admitted directory. It uses `symlink_metadata` to require the
  current root path itself to be the original directory identity, while candidate lookup remains
  `fstatat` on the held root handle with `AT_SYMLINK_NOFOLLOW`.
- Rechecks fail closed on missing roots/candidates, root or candidate metadata mismatch, hard-link,
  symlink, and non-regular replacements. The held root and candidate handles are rechecked against
  their captured dev/ino/UID/mode, regularity, link count, size, and high-resolution mtime facts.
- This slice remains inert: no `unlinkat`, native deletion engine, public cleanup entry, staging,
  publication, IPC, or UI activation was added. Phase 3 tasks 3.1–3.3 stay unchecked because FIFO
  and deletion obligations are intentionally deferred.

### Batch 3a TDD Cycle Evidence
| Slice | Test file | Layer | Safety net | RED | GREEN | TRIANGULATE | REFACTOR |
|---|---|---|---|---|---|---|---|
| final identity recheck core | `local_staging_cleanup.rs` | Unit, real filesystem | 20/20 focused | root-path symlink replacement failed: the old `metadata` followed it | focused recheck tests: 6/6 | mode and same-size write/mtime mutation; missing candidate/root; hard-link, symlink, and directory replacements | replaced root-path `metadata` with no-follow `symlink_metadata`; tests remain 6/6 |

### Batch 3a Verification
- RED: `RUSTUP_AUTO_INSTALL=0 RUSTUP_NO_UPDATE_CHECK=1 /home/dcm/.cargo/bin/rustup run 1.98.1 cargo test -p omnifrons-adapters final_recheck_rejects_a_root_path_replaced_by_a_symlink` failed (0/1) because `metadata` followed the replacement symlink.
- GREEN: the same pinned command passed (1/1); focused `final_recheck_` passed 6/6 and the complete `local_staging_cleanup::tests::` module passed 24/24.
- Gates passed with the pinned toolchain: `cargo fmt --all -- --check`; Linux and `x86_64-pc-windows-gnu` adapter `cargo clippy -p omnifrons-adapters --all-targets --all-features -- -D warnings`; `cargo check --workspace --all-targets --all-features`; and `cargo test --workspace --all-targets --all-features`. Windows evidence is compile-only.

### Batch 3a JD-501 Corrective Round 1 (Verified, Native Gate Passed)
- TDD safety net: `final_recheck_` passed 6/6. RED: `stat_field_comparison` failed with E0425 because the checked comparison helper did not exist. GREEN: representable Apple-width i32/u16 values pass; negative i32 and out-of-range evidence fail closed. Triangulation: two behavioral tests, six assertions.
- The helper fallibly converts every raw `fstatat` field compared to fixed-width evidence (dev, ino, uid, mode, link count, size); it preserves held-root `fstatat(AT_SYMLINK_NOFOLLOW)`, metadata, UID, mode, link, size, and mtime semantics with no mutation or activation.
- Post-GREEN: focused module tests passed 26/26; formatter, Linux and Windows-GNU adapter all-target/all-feature clippy with `-D warnings`, and workspace all-target/all-feature check passed. The Apple target is not installed; this is not a macOS compile or runtime claim.
- Skill resolution injected: `/home/dcm/.config/opencode/skills/judgment-day/SKILL.md`, `/home/dcm/.claude/skills/secret-safe-diagnostics/SKILL.md`, and `/home/dcm/.config/opencode/skills/sdd-apply/strict-tdd.md`.
- Both fresh scoped judges approved the correction: `batch3a-rejudge-a` and #8985. JD-501 is verified;
  1 of 2 corrective rounds is used. Independent Batch 3a gate #8939 passed: final 6/6, stat-width 2/2,
  module 26/26, adapter 200/200, workspace 812/812, Linux and Windows-GNU clippy/check, and fmt.

### Batch 3a Runtime Handoff
- Native attempt finished `passed`: generation 5,
  ordinal 5, work unit `batch3a-final-identity-recheck-core`, objective
  `sha256:83c37f7022de9ff348d78e3510151e7dd319c4cbd64c90e64e5d717aa4a36973`, receipt
  `batch3a-gate-finish-20260914-01`, and revision
  `sha256:1beea844b71ff99f927d1cae561570dcd7cbab9247706505538c54a223ec6368`.
- Ordinals 1–5 are preserved. Batch3a is pending delivery; macOS CI remains required.

## Batch 3b: Protected Native Unlink (Verified Private Engine, Pending Delivery)
- `cleanup_staging` remains uninvoked by `LocalDirBlobStore`, staging, publication, IPC, and UI.
  `inspect_staging` remains retain-only. The Unix-only action constructs fresh evidence from the
  admitted root and candidate handles, calls `final_identity_matches`, then calls handle-relative
  `unlinkat` on the canonical basename. Only `Ok(())` increments `removed`; mismatches retain and
  operational unlink errors retain while incrementing `failures`, with no paths or wire states.
- Linux real-filesystem tests use only owned `TestDir` fixtures and explicit `FileTimes`: an old
  exact dead-PID candidate is removed; basename substitution preserves the original held object;
  a second link added after admission rejects unlink; and an injected operational failure leaves
  the file with `removed: 0` and `failures: 1`. Existing merged tests continue to cover excluded,
  live/unknown, unsafe, young, future, root, and cap retention paths.
- The residual same-user final-check/unlink race remains explicitly documented in the low-level
  API; this slice does not claim atomicity or cross-shell exclusion.

### Batch 3b TDD Cycle Evidence
| Slice | Test file | Layer | Safety net | RED | GREEN | TRIANGULATE | REFACTOR |
|---|---|---|---|---|---|---|---|
| protected native unlink | `local_staging_cleanup.rs` | Unit, real filesystem | 26/26 focused | missing `cleanup_staging_with` and `native_unlink_if_unchanged` (E0425) | 4/4 native focused; 30/30 module | substitution and post-admission hard-link cases | shared action seam reports retain vs operational failure without paths |

### Batch 3b Verification
- RED: pinned `cargo test -p omnifrons-adapters native_cleanup_` failed with E0425 for both new
  action symbols before production implementation.
- GREEN: the same command passed 4/4; module tests passed 30/30. Pinned formatter, Linux and
  Windows-GNU adapter all-target/all-feature clippy with `-D warnings`, workspace all-target/all-
  feature check, and workspace all-target/all-feature test passed. Windows evidence is compile-only.

### Batch 3b Runtime Handoff
- Authorized append-only CAS rollover preserved generations 1–5. Generation 6 / ordinal 6 for
  `batch3b-protected-native-unlink` finished `passed` with receipt
  `batch3b-gate-retry-9051-finish-20260914-01` and revision
  `c9c934288048a7e07b989c02103c8ce2d349ffe87f2ee7e419949a78f349b2d2`.
- Gate #9051 retry passed against the frozen 322-line diff (313 additions, 9 deletions): native 4/4,
  module 30/30, doctest 1/1, workspace 816/816, Linux and Windows-GNU adapter clippy, fmt, and check. Drift audit #9057 reconciled prior arithmetic; metadata was updated after the gate.

### Batch 3b JD-601 Corrective Round 1 (Verified, Pending Independent Native Gate)
- TDD safety net: focused `local_staging_cleanup::tests::` passed 30/30. RED: a public-module
  `compile_fail` doctest referenced `cleanup_staging` as a function pointer and failed because the
  old public API compiled. GREEN: `cleanup_staging` is now `pub(crate)`; the doctest passed 1/1,
  and an owned external compile-only fixture failed specifically with E0603 (`private function`).
  Neither check invokes cleanup or provides a filesystem path.
- The correction changes only the raw cleanup entry visibility and adds targeted dormant-engine
  `dead_code` allowances required by non-test `-D warnings`; no unlink behavior, root predicate,
  read-only API, `LocalDirBlobStore` composition, warning reporting, or activation changed.
- Post-GREEN: native focused tests passed 4/4, module tests 30/30, formatter, Linux adapter
  clippy, workspace all-target/all-feature check and tests, plus Windows-GNU adapter clippy/check
   passed. Windows evidence is compile-only; whole-workspace cross-clippy was not run because it
   needs unavailable `windres`. Fresh scoped judges `batch3b-rejudge-a` and #9031 verified the correction with no new BLOCKER or CRITICAL finding. Native gate #9051 retry then passed; the engine remains private and pending delivery.

### Batch 3b Deferred Scope
- Tasks 3.1–3.3 remain unchecked: the full stated task criteria include additional engine cases
  and deterministic FIFO/nonblocking proof, which are deferred to the next dependency-closed
  slice. No public `LocalDirBlobStore` cleanup entry, publication-mutex proof, or activation was
  added; Phase 4 owns locked invocation and observability.

## Batch 3c: Bound Cleanup Entry and FIFO Contract (JD Approved, Gate Passed, Pending Delivery)
- `LocalDirBlobStore::cleanup_abandoned_staging` composes the crate-private Unix engine through
  the provider's existing validated device-root binding. Non-Unix builds return the existing
  path-free unsupported report. No raw-root destructive API is public.
- Public real-filesystem coverage proves one owned old exact candidate whose owned child has
  exited is removed through the bound provider API. The fixture uses explicit `FileTimes` and
  removes only its own temporary root. The Windows-only public test asserts unsupported retention.
- The shell test creates only an owned FIFO, starts a worker which takes the real
  `PublicationState::lock_surface` guard and calls the bound provider API, then uses bounded
  channels to prove cleanup returns while that actual guard remains held. The FIFO is retained;
  no dummy mutex, sleep-based correctness, production callsite, IPC change, stage call, or UI
  path was added.

### Batch 3c TDD Cycle Evidence
| Tasks | Test file | Layer | Safety net | RED | GREEN | TRIANGULATE | REFACTOR |
|---|---|---|---|---|---|---|---|
| 3.1, 3.3 bound entry | `publication_fs.rs` | Public integration, real filesystem | 25/25 public | missing `cleanup_abandoned_staging` (E0599) | owned dead-child cleanup 1/1 | Unix removal plus Windows unsupported retention | provider wrapper only; private engine unchanged |
| 3.2 FIFO contract | `publication_state.rs` | Shell integration, real FIFO | 2/2 shell | bound API prerequisite unavailable (E0599) | actual `lock_surface` FIFO test 1/1 | retained FIFO plus held-guard assertion | bounded channel handshake; no sleeps |

### Batch 3c Verification
- Baseline: public adapter integration passed 25/25; shell publication-state tests passed 2/2.
- RED: pinned public integration compilation failed with E0599 because the bound provider entry
   did not exist. GREEN: its focused test passed 1/1; the real-lock FIFO test passed 1/1.
- Refactor/gates: pinned `cargo fmt --all -- --check`, workspace all-target/all-feature check and
  tests, Linux adapter clippy, and Windows-GNU adapter clippy all passed. Whole-workspace Windows
  clippy was intentionally not run because the environment lacks `windres`.
- JD approved: both blind judges completed an exhaustive sweep with empty ledgers (#9112, #9114).
- Independent gate #9117 passed: public cleanup 1/1, native 4/4, real `PublicationState` FIFO
  1/1, adapter 201/201, shell 234/234, workspace 818/818, doctest 1/1, Linux and Windows-GNU
  adapter clippy/check, and `cargo fmt --all -- --check`. The frozen PR #21 diff is 211 lines
  (208 additions, 3 deletions).

### Batch 3c Runtime Handoff
- Authorized append-only rollover from completed generation 6 used request
  `batch3c-rollover-20260914-01`; it preserved ordinals 1–6. Historical pre-gate state: generation 7 / ordinal 7 was
  `batch3c-bound-cleanup-fifo-contract`, objective
  `prove-bound-cleanup-and-nonblocking-fifo-under-publication-lock`, revision
  `sha256:7d52511395ec8f57dd29f78d9a3574be584adf27e6bd7400b659d3c5d7b1462c`.
- Generation 7 / ordinal 7 finished `passed` with receipt
  `batch3c-gate-finish-20260914-01` and revision
  `sha256:1bbba8d61e0967f0e75ebbd42fd8d2e9e5ca89f8106447ac8bd08a85feb85637`.
  Ordinals 1–6 remain preserved. Batch 3c is JD approved, gate passed, and pending delivery;
  no production activation, publication/IPC/UI callsite, or provider-policy expansion occurred.

## Batch 4: Locked Publication Activation (Implemented, Gate #9156 Passed, Native Finished, Pending Delivery)
- `publish_approved` holds the existing `PublicationState::lock_surface` guard, opens and
  revalidates the provider root, runs provider-bound cleanup, then enters the unchanged staging
  transaction. The cleanup report is optional: its aggregates are emitted through tracing and never
  alter publication errors, state frames, outbox handling, catalog handling, or journal handling.
- Tracing emits only `supported`, `inspected`, `retained`, `removed`, `failures`, and `truncated`
  with a static message. No device root, outbox, recovery, candidate, or content data is emitted.
- Cleanup remains local-directory-only and dev-mode-only. The public inspection API remains
  read-only; Windows retains entries through its unsupported report and has no native runtime claim.

### Batch 4 TDD Cycle Evidence
| Tasks | Test file | Layer | Safety net | RED | GREEN | TRIANGULATE | REFACTOR |
|---|---|---|---|---|---|---|---|
| 4.1–4.2 locked activation | `src-tauri/src/ipc/publication.rs` | Shell integration, real owned filesystem | existing publication path 1/1 | stale owned candidate remained after a normal approved publication | stale owned candidate removed and normal `published-local`, `registered` frames preserved | invalid destination retains the owned candidate and emits no frames; existing Unix FIFO test proves the actual surface guard remains held during bound cleanup | provider report is consumed only as path-free tracing fields; staging transaction remains unchanged |
| 4.3 rollout documentation and gates | `docs/heavy-asset-publication.md` | Documentation and regression | N/A | N/A | workspace and renderer gates passed | local cleanup activation and invalid-root branches | concise rollout-limit section preserves scope and residual disclosures |

### Batch 4 Verification
- Safety net: pinned `cargo test -p omnifrons-shell publishing_an_approval_registers_the_artifact_and_emits_the_transitions` passed 1/1.
- RED: pinned `cargo test -p omnifrons-shell publishing_cleans_an_owned_stale_local_staging_fixture_before_staging` failed 0/1 because the owned stale fixture remained.
- GREEN/triangulation: the focused stale-fixture test passed 1/1; it removes one owned old dead-PID candidate, retains an owned unsafe symlink whose no-follow open reports a cleanup failure, and preserves normal frames. `publishing_` passed 8/8, including destination revalidation before cleanup. Every cleanup execution used only an owned temporary fixture root; no configured asset root, outbox, recovery entry, or user data was supplied to cleanup.
- Gates passed with pinned Rust 1.98.1: `cargo fmt --all -- --check`; workspace all-target/all-feature test, clippy with `-D warnings`, and check; Linux workspace coverage; and adapter-only `x86_64-pc-windows-gnu` clippy/check. Whole-workspace Windows linking/clippy remains out of scope because `windres` is unavailable; this is not a Windows or macOS runtime claim.
- Unchanged renderer gates passed from `renderer/`: `pnpm test` (7 files, 762 tests), `pnpm lint`, and `pnpm build`.
- Gate #9156 PASSED the scoped Phase 4 contract: `publishing_` 8/8, adapter `cleanup` 31/31, shell 236/236, adapter 205/205, workspace 820/820, Rust fmt/check/Linux clippy, Windows-GNU adapter check/clippy, and renderer test/lint/build all passed. The actual full candidate is 147 additions + 3 deletions = 150 lines, below the native 200-line cap and review 400-line cap.

### Batch 4 Native Runtime Handoff
- Authorized reset request `batch4-rollover-20260914-01` preserved generations 1–7 from settled
  generation 7 / ordinal 7, objective `sha256:e7668d4f5977082b9982fbdec5ffab146af92147409e9458c68efdd915a3ad5d`,
  revision `sha256:1bbba8d61e0967f0e75ebbd42fd8d2e9e5ca89f8106447ac8bd08a85feb85637`.
- Generation 8 / ordinal 8 for `batch4-locked-publication-activation` finished `passed` after
  Gate #9156 with receipt `batch4-gate-finish-20260914-01`, objective
  `sha256:11ad96d097a66cdb241d071d7a77d836e31cb8b8014a675e003201ac5a66cc9b`, evidence revision
  `sha256:e67619879c6a3947d8cf623b031292d584a31d5f8cefe50f4f8e034647396e1f`, and runtime revision
  `sha256:fd093b19d416c6b451d2db53897b3cac12129f64ab8d98e6727a78bcd8bff7af`.
- Native control is complete (`complete=true`, `next_action=complete`, `decision_required=false`);
  the active attempt is absent. Ordinals 1–8 and all historical charges are preserved. Batch 4 is
  gate passed and pending delivery; the SDD change remains 12/15, with tasks 5.1–5.3 pending the
  standard risk review and stacked delivery boundary. It is not archive-ready.
