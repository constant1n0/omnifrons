# Proposal: VP-001 Linux Baseline — Retained Build, Evidence Store, First Row

## Intent

VP-001 stays `Draft` until one scenario, run end to end, genuinely produces and records an
`uncertain` result (desktop-stack-verification-plan.md:283); ADR-0002 stays `Proposed` because its
gate consumes VP-001 evidence (VP-001-R17). Today VP-001-R2 bars dev-mode results, CI retains no
Linux bundle, `docs/evidence/VP-001/` does not exist (:289), and no scenario has a row.

## Scope

### In Scope
- Retain the packaged Linux build: pin the matrix to `ubuntu-24.04`, pin the Tauri CLI, set explicit `bundle.targets`, feed `tauri-action` `artifactPaths` into `actions/upload-artifact`.
- Bootstrap `docs/evidence/VP-001/`: the record schema (:157-174), a baseline record carrying every VP-001-R1 field (:100-110), and a validator rejecting any row that omits a mandatory field.
- Exercise VP-S6 through a WebDriver harness (`tauri-driver` under `xvfb`) that drives the retained packaged binary through its real GUI on the pinned `ubuntu-24.04` runner: approve a test executable as the agent, launch it so it spawns a breakaway-attempting descendant, trigger stop through the UI, then enumerate descendants.
- Record one row — VP-S6 Linux containment against that packaged build — with its honest `fail` or `uncertain` outcome.
- Reconcile README.md:7 and the absent `## Slice 5f` section of docs/spike-log.md.

### Out of Scope
- Any evidence-only cargo feature: `demo-harness` stays non-default and no new shipped CLI surface is introduced. The measured binary is the one a user installs.
- The other 25 Linux scenarios, Tier A harnesses, VP-S5.
- cgroup/watchdog, signing, keyring, Git, handoff, accessibility tooling. VP-S6 measures the code that exists (process-group/`killpg`, crates/omnifrons-supervisor/src/lib.rs:181); it is never repaired to force a pass.
- Accepting VP-001 or ADR-0002.

## Capabilities

### New Capabilities
- `packaged-build-evidence`: a pinned, retained, digest-addressable Linux release artifact admissible under VP-001-R2, measured through its shipped GUI.
- `verification-evidence-store`: append-only VP-001 record layout, baseline-before-scenario admission (VP-001-R1), mandatory-field validation, correction-by-appended-row (VP-001-R20).

### Modified Capabilities
- None.

## Approach

This resolves VP-001 open decision D2 as `ubuntu-24.04` **for this baseline only**, recording the
exact runner image version. Linux bundles are not byte-reproducible (AppImage `.digest_md5`), so a
row binds a SHA-256 of the one retained artifact, never a rebuild-identity claim. Evidence stays
provenance-clean (:178); anything disclosing infrastructure detail goes to the private store by
identifier. Running the workflow is a remote operation: explicit user authorization is obtained
before any CI trigger, and no standing authorization exists.

**Why WebDriver.** The packaged binary exposes no non-GUI path to `TokioProcessSupervisor::stop`:
`src-tauri/src/main.rs:14-40` compiles the `--demo-harness` branch out by default and runs it
in-process, making the binary the child rather than the supervisor, and
`src-tauri/tests/feature_gate.rs` asserts the feature is off by default and the literal string
absent from the built binary. Enabling that feature to produce evidence would measure a build
nobody installs, contradicting VP-001's own "distributable artifact a user would actually install"
(docs/desktop-stack-verification-plan.md:78). VP-S6 therefore drives the real GUI.

**Honesty rule.** If the driver cannot drive the packaged binary on the pinned runner, the row
records `uncertain` with the blocker disclosed. This change never switches to a special build,
never retries until green, and never renames an outcome to obtain a `pass`.

## Slice Chain — stacked to main, each under 400 authored lines (added + deleted)

| # | Slice and files | Forecast | Rollback |
|---|---|---|---|
| 1 | `.github/workflows/tauri-build.yml`, `src-tauri/tauri.conf.json`, pinned `@tauri-apps/cli` + lockfile | ~180 authored (lockfile generated, excluded) | Revert; packaging returns to build-only |
| 2 | `docs/evidence/VP-001/` schema, validator, fixtures | ~280 | Revert; no rows exist yet to orphan |
| 3a | Harness scaffolding: `tauri-driver` + `xvfb` CI wiring, a driver session against the retained bundle, feasibility gate | ~220 | Revert; nothing depends on it yet |
| 3b | VP-S6 scenario: approve and launch the fixture agent, breakaway-attempting descendant, UI-triggered stop, descendant enumeration, transcript | ~260 | Revert the scenario; the harness stays inert |
| 4 | Baseline record + VP-S6 row (needs 1, 2, 3a, 3b, and an authorized CI run) | ~200 | Append a correction row; never rewrite |
| 5 | `README.md`, `docs/spike-log.md` (independent; any position) | ~60 | Revert |

The harness is split at 3a/3b because dev tooling, CI wiring, a fixture agent, and descendant
enumeration do not fit one reviewable slice under the 400-line budget.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| `tauri-driver` may not be able to target the binary installed from the bundle | High | Prove or disprove at slice 3a, before any row exists; if it cannot, VP-S6 records a disclosed `uncertain` and the chain stops there |
| WebDriver runs are flaky | High | A flaky or non-decidable run is recorded `uncertain` with the blocker; never retried into a `pass`, never re-run until green |
| Harness dev tooling inflates the diff | Med | Split 3a/3b with a per-slice forecast; only generated lockfiles are excluded from the authored count |
| Runner `ImageVersion` capture mechanism unconfirmed | High | Read it from the authorized run's job log; assume no env var |
| `upload-artifact` / CLI pins drift (v7.0.1 verified 2026-09-16) | Med | Pin an exact tag or SHA; re-verify at slice 1 |
| VP-001-R6 wants the fallback as its own scenario; the catalog folds it into VP-S6 | Med | Record mechanism and fallback as distinct observations; report the tension, do not resolve it here |
| A row is filed wrong | Low | Append a correction row under signed, linear `main` (GOV-001-R15, VP-001-R20) |

## Success Criteria
- [ ] One retained Linux artifact with a recorded SHA-256 from a pinned `ubuntu-24.04` run.
- [ ] `docs/evidence/VP-001/` holds a baseline record with every VP-001-R1 field, and the validator rejects a row missing any mandatory field.
- [ ] One VP-S6 row records `fail` or `uncertain` with mechanism and observed state, never "cleanly stopped" (VP-001-R5).
- [ ] The artifact VP-S6 measures is the same distributable artifact whose digest the evidence row records.
- [ ] Every published artifact is provenance-clean, and no CI run occurred without explicit user authorization.
- [ ] README.md and docs/spike-log.md match the merged repository state.
