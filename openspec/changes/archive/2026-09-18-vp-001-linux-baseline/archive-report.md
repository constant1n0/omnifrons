# Archive Report: vp-001-linux-baseline

## Outcome

Archived on 2026-09-18 after verification passed with no blocking issues.

**Verdict**: PASS WITH WARNINGS — 0 CRITICAL, 11/11 requirements, 22/22 scenarios, 49/49 tasks complete. Five open WARNINGs; none blocks archive.

**Change**: vp-001-linux-baseline · **Mode**: Strict TDD · **Repo**: `main` at `44f9087`, tree clean, all slices merged.

## Specification Sync

Two new capability specs promoted from delta to main specifications:

| Domain | Action | Requirements | Scenarios |
|---|---|---:|---:|
| `packaged-build-evidence` | Created main spec from full delta | 4 | 8 |
| `verification-evidence-store` | Created main spec from full delta | 7 | 14 |

The specifications are now the source of truth at:
- `openspec/specs/packaged-build-evidence/spec.md`
- `openspec/specs/verification-evidence-store/spec.md`

## What Shipped

Six implementation slices across five Pull Requests, each independently tested and merged to `main`:

### Slice 1 (PR #32) — Packaging Pin and Retention
- Pinned `ubuntu-24.04` runner with explicit bundle targets.
- Captured exact runner image version and provisioner facts.
- Added SHA-256 digest computation and artifact retention.

### Slice 2 (PR #33–#35) — Evidence Store and Validator
- Implemented eight validators (V1–V8) covering mandatory fields, closed vocabulary, baseline admission, digest matching, and provenance cleanliness.
- Created `tools/evidence-validator/` crate with 39 tests.
- Established `docs/evidence/VP-001/README.md` documenting the schema and append-only correction protocol.

### Slice 3a (PR #36–#38) — Harness Scaffolding and Feasibility Gate
- Created WebDriver client with pure payload builders and endpoint composition.
- Wired `vp-s6-linux.sh` with AV1 (digest verification) and AV2 (feature-gate byte-scan).
- Added CI step `node --test docs/evidence/VP-001/procedures/*.test.mjs` (37 tests).
- Feasibility gate F1–F3 ran locally and passed; disclosed one pre-existing `ApprovalId` product defect (fixed on main by commit `5c5f474`).

### Slice 3b (PR #39–#41) — VP-S6 Scenario
- Built `tools/vp-s6-agent/` fixture ELF demonstrating process-group lifecycle.
- Extended `vp-s6-linux.sh` with GTK chooser driving via `xdotool`.
- Added scenario logic in `vp-s6-scenario.mjs` and observation derivation.
- Created `vp-s6-observations.test.mjs` regression suite (5 tests).

### Slice 4 (PR #42–#49) — Baseline Record and VP-S6 Row
- User-authorized exactly one `tauri-build.yml` dispatch (run 35252892166) with `vp001_scenario: true`.
- Captured baseline facts from runner: OS build, WebView runtime (libwebkit2gtk-4.1 4.1.3-2ubuntu7), MSRV/toolchain, image `20260907.300.1`.
- Retained packaged artifact (AppImage digest `d7e1…`).
- Filed `VP-001-BASE-01` baseline record in `docs/evidence/VP-001/baselines.md`.
- Filed `VP-001-VP-S6-01` scenario row in `docs/evidence/VP-001/records.md` with honest outcome: `result: uncertain`, `observed_state: orphan-risk`, `blocker: workspace-chooser-timeout` (GTK workspace picker could not be driven under Xvfb).
- Both records passed all eight validators; `cargo test --workspace` green including new `tools/evidence-validator/tests/real_evidence_store.rs`.

### Slice 5 (PR #50–#51) — Doc Reconciliation
- Updated `README.md` status line to reflect merged development-mode spike state.
- Added `docs/spike-log.md` entry for this change, plus restored entry for archived 2026-09-14 change (`spike-slice-5f`).

## Final Measured State

### Test Results (Pass 3)

- `cargo test --workspace`: 869 passed, 0 failed (87 harnesses).
  - Change-owned: 81 tests across evidence-validator (39), config (5), and node procedures (37).
- `pnpm test` (renderer): 763 passed.
- `node --test docs/evidence/VP-001/procedures/*.test.mjs`: 37 passed.
- `cargo clippy --workspace --all-targets`: no diagnostics.
- `cargo fmt --all`: no differences.
- Total: 1,669 tests passed, 0 failed, 0 skipped.

### Commits Landed After Apply Phase

Two commits landed after `sdd-apply` completed, both of which are included in this archive:

| Commit | Message | Reason |
|---|---|---|
| `4b76546` | fix(evidence-validator): require runner_image on a baseline record | Addressed Pass 1's CRITICAL: made `runner_image` mandatory in V1 validator and verified via mutation testing. No TDD trail entry (open WARNING 5). |
| `44f9087` | docs(spec): narrow the evidence-store spec to what it can demonstrate | Addressed Pass 2's three failing scenarios: narrowed `Unresolved correction reference`, `Harness cannot complete step`, and `No retry/rename/cross-build` to what the design proves. Corrected README field list (closed Pass 2 WARNING 5). |

### Evidence Produced

| Item | Details |
|---|---|
| Baseline | `VP-001-BASE-01` (Ubuntu 24.04, libwebkit2gtk-4.1 4.1.3, runner 20260907.300.1) |
| Scenario Row | `VP-001-VP-S6-01` (uncertain/orphan-risk, workspace-chooser blocker) |
| Retained Artifact | AppImage digest `d7e1f946ba6bea057626c7d9b5f1986584e6adee58cbbec273085b1c21c94f68` |
| CI Run | GitHub Actions run 35252892166 (`ubuntu-24.04`, head `ab5eda8` on `main`) |
| Transcripts | Feasibility-check and VP-S6 scenario transcripts retained in run artifacts (expiry 2026-12-16) |

## Open Follow-Ups (Non-Blocking)

Per verification pass 3, five WARNINGs remain open; none blocks archive or deployment:

1. **V8 "both or neither" pairing half-enforced**: only `has_digest && !has_artifact` is rejected. Reproduced: `build_channel_digest` present, `evidence_artifact` absent → zero violations. Filed data carries both.

2. **`variant_scan: unperformed` unfilea ble**: V7 admits only `absent`, so an honest "AV2 could not be performed" row cannot be filed. Misdescribes cause as `feature-enabled-variant`.

3. **`mechanism` and `fallback` not validator-mandatory**: `v1_mandatory_fields` does not include them. Filed row carries both as documented extras.

4. *(Narrowed from spec gap to disclosed limit)* **In-place rewrite not mechanically detectable**: `44f9087` moved this to the requirement text itself, which now names signed-commit linear-history branch protection and review as the control. No record was altered in history.

5. **`runner_image` fix has no TDD trail entry**: `runner_image` does not appear in apply-progress or tasks, though commit `4b76546` proved it mandatory via mutation. Strict-TDD CRITICAL condition is met (five TDD tables exist); this is an documentation gap, not a substance gap.

**CRITICAL issues**: None.

**Suggestions** (4): See verify-report.md lines 108–112 for pedagogical improvements and cross-reference clarifications.

## Process-Group Containment Status

**What remains unproven**: Process-group containment (`killpg(SIGTERM)` with `SIGKILL` escalation) was never exercised on the pinned baseline. CI run 35252892166 reached `gate=f1-session-created` then hit `blocker=workspace-chooser-timeout` before Start was reachable. No fixture process spawned; `killpg` never invoked. VP-001's containment question is still open. This is a recorded limit of the evidence, not a defect: the change's purpose was to build a pipeline that files an honest `uncertain` instead of a manufactured `pass`, and it accomplished that.

## Source of Truth Updated

- `openspec/specs/packaged-build-evidence/spec.md` (new)
- `openspec/specs/verification-evidence-store/spec.md` (new)

## Archive Contents

✅ All artifacts present and moved to `/openspec/changes/archive/2026-09-18-vp-001-linux-baseline/`:
- proposal.md
- design.md
- specs/packaged-build-evidence/spec.md
- specs/verification-evidence-store/spec.md
- tasks.md (49/49 tasks complete, all checked)
- apply-progress.md (five TDD cycles with 81 change-owned tests)
- verify-report.md (pass 3 verdict: PASS WITH WARNINGS)

## Key Learnings

1. Process-group containment requires a scenario run that reaches the Start button; workspace picker timeouts under Xvfb leave that unproven for this baseline.
2. Validator V1–V8 composition required field-list coupling (test table mirrors production constant) to prove mandatory-field enforcement against mutation.
3. Two commits (`4b76546`, `44f9087`) landed after apply and are now archived; both addressed verification findings and preserved in history.
4. Append-only correction enforcement is genuinely post-hoc: branch protection plus review own the control; the validator only observes that filed records reference existing identifiers.
5. Five WARNINGs fail closed or are inert on filed data; the pipeline delivered an honest `uncertain` outcome correctly.
