# Proposal: Safely Reclaim Abandoned Local Staging Files

## Intent

Prevent crashed local publication attempts from accumulating staging files while preserving
publication safety. Cleanup is limited to entries proven abandoned; age never establishes PID
identity or deletion authority.

## Scope

### In Scope
- Sweep only exact `.<publication-hex>.<pid>-<sequence>.part` names in a revalidated local device asset root before new local staging.
- Require the publication surface lock, minimum age, and conclusive proof that the encoded creator PID is not live; treat PID reuse as live or ambiguous.
- Recheck metadata and identity before removal; retain on ambiguity, platform uncertainty, links, non-regular entries, mismatches, parse failures, or I/O failures.
- Report cleanup failures without device paths or publication wire-state changes; blanket retention is not successful cleanup.

### Out of Scope
- The other six `.part` producers, including normalization or ownership decisions for the three non-PID grammars; they remain unresolved later boundaries.
- Generic staging hygiene, registered-artifact deletion, tombstones, outbox candidate expiry, provider integration, Git policy, or VP-001 discharge.
- Claims of cross-shell exclusion or atomic race freedom unsupported by current primitives.

## Capabilities

### New Capabilities
- `local-staging-cleanup`: Conservative reclamation of provably abandoned local-dir staging files.

### Modified Capabilities
- None.

## Approach

Introduce a narrow adapter cleanup operation invoked only from a shell path already holding the publication surface lock. Parse exact basenames, refuse links/non-regular entries, establish age,
query tri-state arbitrary-PID liveness, then revalidate identity immediately before removal.
Design must prove lock evidence, liveness semantics, age policy, and race handling; where proof is unavailable, deletion is disabled. Same-user and second-shell races remain explicit residuals,
not guarantees supplied by an in-process mutex. HAP-001 R28 retains candidates; R39 reserves registered-artifact deletion for tombstones.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/omnifrons-adapters/src/local_dir_blob_store.rs` | Modified | Exact grammar and bounded sweep |
| `src-tauri/src/publication_state.rs` / `src-tauri/src/ipc/publication.rs` | Modified | Locked invocation boundary |
| `crates/omnifrons-adapters/tests/publication_fs.rs` | Modified | Retention and deletion evidence |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| PID reuse, TOCTOU, or second-shell races | High | Retain on uncertainty; document residuals |
| Unsupported platform evidence | Medium | Disable deletion rather than infer safety |
| Authority expands beyond staging | Low | Exact root/grammar checks and R28/R39 tests |

## Rollback Plan

Remove the automatic invocation and cleanup implementation; normal commit/abort behavior remains unchanged, and retained staging files remain inert.

## Dependencies and Chain Boundary

- Planning is one stacked-to-main slice; implementation follows as an independently reviewable local-dir-only slice under 400 changed lines, with no size exception.
- Later slices must separately govern the six excluded producers and non-PID grammar ownership.

## Success Criteria
- [ ] Only exact, old-enough candidates with conclusively non-live creator PIDs are removed while the publication surface lock is held.
- [ ] Every ambiguous, active/reused-PID, unknown, unsafe-entry, mismatched, or failed case remains.
- [ ] Cleanup errors expose no device path and do not alter publication wire state.
- [ ] Tests preserve HAP-001 R28 candidate retention and R39 tombstone-only artifact deletion.
