# Tasks: Safely Reclaim Abandoned Local Staging Files

## Review Workload Forecast

| Field | Value |
|---|---|
| Estimated implementation total | 970 changed lines (code, tests, docs, progress) |
| Planning aggregate (exact) | 437 lines: Plan A 368 + replacement Plan B 69 |
| Delivery | `auto-chain`; stacked-to-main; no size exception |
| Dominant risks | Unix identity/TOCTOU proof, FIFO blocking, locked activation |

Decision needed before apply: No
Chained PRs recommended: Yes
Chain strategy: stacked-to-main
400-line budget risk: Medium

### Suggested Work Units

| Unit | Likely PR / base | Start → finish; verification; rollback | Est. |
|---|---|---|---:|
| Plan A | PR #18 → main | Existing OpenSpec/config artifacts → reviewed design gate; inspect line count; revert docs only | 368 |
| Plan B | Replacement PR → main | This task plan plus verified CI ledger status → task guard recorded; inspect line count; revert docs only | 69 |
| 1 | PR 1 → main | No cleanup API → inert grammar/age/PID seams; Rust tests; revert inert module | 220 |
| 2 | PR 2 → main | Inert seams → bounded retain-first scanner/evidence; Rust tests; revert scanner | 280 |
| 3 | PR 3 → main | Scanner not invoked → Unix unlink engine; Rust tests; revert engine, still uninvoked | 280 |
| 4 | PR 4 → main | Uninvoked engine → locked shell call/traces; all gates; revert invocation | 190 |

No parallel writers. Planning PRs land before code; every PR targets `main` after its predecessor lands.

## Phase 1: Inert Safety Primitives — PR 1

- [ ] 1.1 **RED**: add parser tables in `crates/omnifrons-adapters/src/local_staging_cleanup.rs` for exact local grammar and malformed/foreign/non-UTF-8 names; assert R28/R39 entries retain.
- [ ] 1.2 **GREEN**: implement canonical basename, 24-hour age, limits, `CleanupReport`, and cfg-gated `PidState`; keep cleanup unreachable from `LocalDirBlobStore`.
- [ ] 1.3 **RED/GREEN/REFACTOR evidence**: test zero/out-of-signed-`pid_t` PIDs return `Unknown` without `kill`; child process proves real `Live` then `NotLive`; Windows returns `Unknown`.

## Phase 2: Bounded Retain-First Scanner — PR 2

- [ ] 2.1 **RED**: extend `crates/omnifrons-adapters/tests/publication_fs.rs` for eligible root/owner/mode, age/time/metadata/liveness uncertainty, excluded producers, 256 inspected/32 removed caps, counts and `truncated`.
- [ ] 2.2 **GREEN**: implement root-handle evidence and bounded scanner in `local_staging_cleanup.rs`; retain on root/link/identity/time/I/O uncertainty and do not invoke unlink.
- [ ] 2.3 **REFACTOR evidence**: expose the inert entry through `local_dir_blob_store.rs` and `lib.rs` without calling it; focused tests plus `cargo test --workspace`, fmt, clippy, and check pass.

## Phase 3: Unix Positive Deletion Engine — PR 3

- [ ] 3.1 **RED**: add Unix real-FS tests for only old exact dead-PID regular single-link deletion; links, swaps, final identity mismatch, failed unlink, and nonregular entries retain with correct counts.
- [ ] 3.2 **RED**: add deterministic FIFO coverage proving `O_NONBLOCK|O_NOFOLLOW` refusal returns without blocking while the publication mutex is held (JD-002).
- [ ] 3.3 **GREEN/REFACTOR evidence**: use handle-relative no-follow open, `fstat`/`fstatat` root-and-candidate rechecks, and `unlinkat`; count only `Ok(())`, disclose residual race, retain unsupported cfg.

## Phase 4: Locked Activation and Regression Gates — PR 4

- [ ] 4.1 **RED**: add `src-tauri/src/ipc/publication.rs` tests proving `open_provider` → optional cleanup → staging under `lock_surface`, and cleanup failure preserves publication and IPC frames.
- [ ] 4.2 **GREEN**: invoke cleanup after revalidated provider opening; trace only `supported`, counts, `truncated`, and failures—never paths or publication wire states.
- [ ] 4.3 **REFACTOR/verification evidence**: document same-user final-check/unlink residual in `docs/heavy-asset-publication.md`; run Cargo workspace test/fmt/clippy/check, Linux and Windows cfg builds, and unchanged renderer `pnpm test`, `pnpm lint`, `pnpm build`.

## Phase 5: Review Boundaries

- [ ] 5.1 Keep each PR below 400 additions+deletions including tests/docs/progress; record actual `git diff --numstat` before review and split again if needed.
- [ ] 5.2 Preserve JD-001/JD-002 regression evidence; record JD-B-003 only as the known bounded-prefix limitation, and do not create a remediation task for refuted JD-B-004.
- [ ] 5.3 Do not activate deletion before PR 4 review; keep local-dir-only scope, six other producers, providers, and VP-001 out of scope.
