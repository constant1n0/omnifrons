## Exploration: spike-slice-5f safe staging `.part` cleanup

### Current State
`LocalDirBlobStore::stage` creates `.<publication-hex>.<pid>-<sequence>.part` exclusively in the validated device asset root, then `LocalStaged::commit` renames it or removes it on error; `abort` removes it. A crash between those calls leaves it intentionally unreclaimed (`crates/omnifrons-adapters/src/local_dir_blob_store.rs:15-24,80-95,116-168`).

The shell's `PublicationState::lock_surface` serializes one shell's approve/publish/list/repair command bodies, but not another shell process (`src-tauri/src/publication_state.rs:16-22,55-84`). No arbitrary-PID liveness port exists: `ProcessSupervisor::observe` only knows processes that that supervisor spawned (`crates/omnifrons-app/src/process_supervisor.rs:138-176`). The existing outbox identity/unlink port is Unix-only for identity and explicitly defers removal on Windows (`crates/omnifrons-app/src/outbox_entry_ops.rs:1-55`; `crates/omnifrons-app/src/contract/wrong_root.rs:249-275`).

The verified inventory differs slightly from the six-site checkpoint: seven production functions create a `.part` name. Four include pid+sequence: snapshot writer (`fs_snapshot_store.rs:109-133`), managed guidance replacement (`fs_project_text_file.rs:212-257`), catalog repair (`jsonl_catalog_store.rs:464-510`), and local blob staging. Three do not: recovery write (`publication.rs:228-269`, also uses `create(true).truncate(true)`), wrong-root copy-in (`wrong_root.rs:573-631`), and quarantine copy-in (`quarantine.rs:532-594`). Only the first four are parseable as pid-owned; the last three must remain outside an automatic sweep until ownership rules are separately decided.

### Affected Areas
- `crates/omnifrons-adapters/src/local_dir_blob_store.rs` — sole provider staging lifecycle and safe candidate grammar.
- `src-tauri/src/publication_state.rs` and `src-tauri/src/ipc/publication.rs` — surface-lock scope and a possible locked sweep invocation.
- `crates/omnifrons-app/src/outbox_entry_ops.rs` — closest identity/remove precedent; insufficient cross-platform primitive for staging files.
- `crates/omnifrons-adapters/tests/publication_fs.rs` — existing distinct-stage and abort lifecycle coverage (`911-938`).
- `docs/heavy-asset-publication.md` — HAP-001-R28 forbids candidate deletion before verified publication+registration (`232-245,379`); registered-artifact/tombstone deletion remains owned by MRP-001 (`22-26`).

### Approaches
1. **Narrow local-provider sweep** — Under the already-held publication surface lock, inspect only the local asset root; delete only exact local-blob grammar candidates when metadata age exceeds a fixed bound and a platform liveness probe conclusively says the encoded creator PID is not live.
   - Pros: bounds authority to abandoned, unregistered staging; preserves HAP-001 R28 and all legacy/foreign names; independently mergeable.
   - Cons: needs an explicit conservative Windows/unknown-liveness policy and a lock-presence contract; PID reuse can only be mitigated by the age bound.
   - Effort: Medium.

2. **Generic all-`.part` janitor** — Sweep every known root/grammar.
   - Pros: greater cleanup coverage.
   - Cons: mixes workspace, recovery, quarantine, snapshot, and provider ownership; three grammars do not establish creator PID; broadens deletion authority and needs unresolved governance.
   - Effort: High.

### Recommendation
Choose approach 1 as slice 5f: one local-dir staging sweep before new local staging, called only by a shell path that holds `lock_surface`. Use exact basename parsing, `symlink_metadata` refusal for links/non-regular files, a post-age liveness check, and best-effort failure reporting without deleting on any uncertainty. Do not claim a Windows guarantee: if liveness or identity cannot be proven, retain the candidate. Failed cleanup, PID reuse, TOCTOU, live producers, unknown/legacy names, symlinks, and files younger than the bound are retain-only outcomes.

The missing invariant is not a reusable deletion API: it is an adapter-visible, cross-platform proof that the caller owns the publication surface lock plus a tri-state arbitrary-PID liveness observation. Existing `OutboxEntryOps` protects a held outbox candidate, not an unheld asset-root staging entry, and cannot supply this proof.

### Risks
- PID reuse or unavailable liveness can make a stale-looking name ambiguous; age reduces but cannot eliminate reuse risk, so uncertainty MUST retain.
- `symlink_metadata` followed by removal has a TOCTOU window; absent a handle-relative cross-platform delete primitive, scope must retain on any mismatch/error and document the residual.
- A second shell is outside `lock_surface`; it remains an explicit residual, not authorization to delete.
- No provider integration, Git policy, registered-artifact deletion/tombstones, or VP-001 execution claim belongs in this slice.

### Ready for Proposal
Yes — propose the narrow local-dir-only boundary, with explicit retain-on-uncertainty behavior and no decision about non-pid/legacy `.part` ownership. Rough implementation+tests: 280–380 changed lines, so it is feasible as one stacked slice under the 400-line budget; current cumulative planning estimate is 280–380 lines.
