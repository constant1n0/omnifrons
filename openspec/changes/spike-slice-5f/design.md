# Design: Safely Reclaim Abandoned Local Staging Files

## Technical Approach

Add local-dir maintenance after `open_provider` revalidates the asset root, while the per-shell
mutex remains held. Run it before `publish` can stage; it is bounded and Unix-positive.

## Architecture Decisions

| Decision | Alternatives considered | Choice and rationale |
|---|---|---|
| Ownership and origin | Filename alone; sidecar registry | Require exact grammar, an owner-only root handle owned by the effective UID, and a candidate with that UID and owner-only mode. This admits existing leftovers under TM-001 A6 while refusing foreign-owned lookalikes; same-user creation remains a disclosed residual. |
| Liveness | `ProcessSupervisor`; `/proc`; shell/unsafe kill | Keep canonical `u32` grammar separate from the probe domain: convert only a strictly-positive PID representable by signed `pid_t` before `Pid::from_raw` and `kill(Pid, None)`. Conversion failure is `Unknown` and retains; success/`EPERM` is `Live`, only `ESRCH` is `NotLive`, otherwise `Unknown`. Unsupported platforms return `Unknown`. |
| Filesystem proof | `lstat` then path removal | On Unix, open the root as a directory and candidates handle-relative, nonblocking, and `O_NOFOLLOW` before `fstat`; refuse and retain FIFOs and all nonregular entries without stalling under the publication mutex. Require regular, single-link, owned, stable evidence. Revalidate canonical root and candidate with `fstatat` before `unlinkat`, without claiming atomicity. |
| Placement | Application port; adapter lock token | Keep this local-dir adapter maintenance. Shell composition calls it in the locked sequence; the adapter neither proves a shell token nor depends on shell code. |
| Work/reporting | Unbounded sweep; fail publication | Inspect at most 256 entries and remove at most 32. Return counts plus `truncated`, `supported`, and `failures`; trace only these fields and never alter publication or IPC state. |

## Data Flow

    publish_approved [existing surface mutex]
      -> open_provider [existing DeviceAssetPath revalidation]
      -> cleanup_abandoned_staging [proposed, optional]
      -> publish -> LocalDirBlobStore::stage [existing]

Entry flow: parse -> checked PID probe conversion -> directory-constrained root open -> nonblocking,
no-follow candidate open -> verify regular origin/metadata -> age -> PID state -> root and
candidate recheck -> `unlinkat`; count removal only on `Ok(())`.

## Safety Contracts

- Grammar consumes the ASCII basename `.<64 lowercase hex>.<pid>-<sequence>.part`. PID is canonical
  decimal `1..=u32::MAX`; sequence is canonical decimal `0..=u64::MAX`. Leading zeroes, signs,
  whitespace, overflow, extra separators, and non-UTF-8 names are retained. Parsing does not imply
  probe eligibility: zero, negative, or out-of-range `pid_t` conversion is `Unknown` and retains;
  it is never passed to `Pid::from_raw`.
- `MIN_STAGING_AGE` is 24 hours. Eligibility requires `now.duration_since(mtime) > 24h`;
  exact equality, future timestamps, and conversion/metadata errors retain the entry.
- `Live` includes this process, active shells, zombies, reused PIDs, and permission-denied probes,
  preserving multi-shell staging. PID namespaces are outside the desktop threat model; unavailable
  OS evidence is `Unknown` and disables deletion.
- Initial evidence records root and file `(dev, ino, uid, mode)` plus file `nlink`, size, and
  nanosecond mtime from open handles. Any final mismatch, missing evidence, or I/O error retains.
- Handle-relative unlink prevents ancestor redirection. A same-user process can still replace the
  basename between `fstatat` and `unlinkat`; reporting must disclose this residual.

## Interfaces / Contracts

Existing: `LocalDirBlobStore::stage`, `DeviceAssetPath::open`, `PublicationState::lock_surface`, and
`publish_approved`. Proposed: `cleanup_abandoned_staging() -> CleanupReport`, internal `PidState`,
and injected clock/liveness/filesystem seams. Reports and traces contain no paths or IPC changes.

## File Changes

| File | Action | Description |
|---|---|---|
| `crates/omnifrons-adapters/src/local_staging_cleanup.rs` | Create | Parser, limits, liveness, Unix proof/removal, fallback, and unit tests. |
| `crates/omnifrons-adapters/src/local_dir_blob_store.rs` | Modify | Expose the real cleanup entry point; staging semantics remain unchanged. |
| `crates/omnifrons-adapters/src/lib.rs` | Modify | Register/re-export the maintenance result needed by shell composition. |
| `crates/omnifrons-adapters/Cargo.toml` | Modify | Enable nix `signal`; add no dependency or unsafe code. |
| `crates/omnifrons-adapters/tests/publication_fs.rs` | Modify | Exercise positive Unix deletion and unsafe-entry retention. |
| `src-tauri/src/ipc/publication.rs` | Modify | Invoke in locked order and trace path-free aggregates. |

## Testing Strategy and Delivery Chain

| Stacked-to-main slice | Code + tests estimate | Independent verification |
|---|---:|---|
| 1. Grammar, age, and Unix liveness primitives (inert) | 180 | Tables cover grammar bounds/overflow and probe-domain boundaries: zero and values above signed `pid_t::MAX` retain as `Unknown` without calling `kill`; a child proves real `Live` then `NotLive`; unsupported cfg is `Unknown`. |
| 2. Root/candidate evidence and bounded engine (not invoked) | 360 | Fake clock/liveness/faults cover retention; Unix real-FS tests cover deletion, links, swaps, caps, and unlink evidence, plus a deterministic FIFO candidate that is refused and returns without blocking while the publication mutex is held. |
| 3. Locked invocation and path-free observability | 150 | Shell tests prove ordering and failure isolation; Linux and Windows cfg builds compile. |

Each slice stays below 400 changed lines and lands sequentially on main. Strict TDD uses focused
tests, then `cargo test --workspace`; unchanged renderer verification is `pnpm test`. No E2E or
coverage tooling exists.

## Migration / Rollout and Rollback

No migration or flag is required. Roll back slice 3 to disable cleanup; earlier inert primitives and
commit/abort remain. Six other producers, deletion policy, providers, UI tokens, and VP-001 stay out.

## Open Questions

None. The authority boundary relies on the already documented TM-001 A6 same-user residual; changing
that threat model would block activation and require a separately approved ownership mechanism.
