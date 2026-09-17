# Tasks: VP-001 Linux Baseline — Retained Build, Evidence Store, First Row

## Review Workload Forecast

| Field | Value |
|---|---|
| Estimated changed lines | Slice 1 ~180, Slice 2 ~280, Slice 3a ~220, Slice 3b ~260, Slice 4 ~200, Slice 5 ~60 (design Migration/Rollout) |
| 400-line budget risk | Medium — every slice is individually under 400, but Slices 2 (~280) and 3b (~260) have the least headroom; if either grows, it splits again rather than being compressed |
| Chained PRs recommended | Yes |
| Suggested split | PR 1 → PR 2 → PR 3a → PR 3b → PR 4 → PR 5 (5 may land at any point; independent) |
| Delivery strategy | auto-chain |
| Chain strategy | stacked-to-main (fixed by design Migration/Rollout, matches session `chain_strategy`) |

Decision needed before apply: No
Chained PRs recommended: Yes
Chain strategy: stacked-to-main
400-line budget risk: Medium

### Suggested Work Units

| Unit | Goal | PR | Focused test command | Runtime harness | Rollback boundary |
|---|---|---|---|---|---|
| 1 | Pin packaging, retain the artifact, digest it | PR 1 | `cargo test -p omnifrons-shell --test config` | N/A — CI YAML has no repo harness; checked at Slice 4's authorized run | Revert 4 files; packaging returns to build-only |
| 2 | Evidence store schema, validator (V1–V8), `derive` | PR 2 | `cargo test --workspace` | N/A — pure unit/integration tests | Revert `Cargo.toml` member + `tools/evidence-validator/`; no rows exist yet |
| 3a | WebDriver client, CI wiring, AV1/AV2 gate, F1–F3 feasibility | PR 3a | `node --test docs/evidence/VP-001/procedures/webdriver-session.test.mjs` | Local `Xvfb` + `tauri-driver` feasibility run against a **locally built** artifact (not a CI trigger) | Revert workflow harness step, client files, `ci.yml` step; nothing depends on it yet |
| 3b | VP-S6 scenario: fixture, dialog driving, stop, enumeration | PR 3b | N/A — no repo test drives a live GUI/process tree | Local/authorized `Xvfb` + `tauri-driver` + `xdotool` full scenario run | Revert scenario logic + delete `tools/vp-s6-agent/`; harness stays inert |
| 4 | Baseline record + VP-S6 row from an authorized run | PR 4 | `cargo test --workspace` (validator over the real records) | **The** explicitly authorized `tauri-build.yml` run, then the full VP-S6 scenario against its retained artifact | Append a correction row via `corrects`; never rewrite the original |
| 5 | Doc reconciliation | PR 5 | N/A — docs only | N/A | Revert |

## Phase 1: Packaging Pin and Retention — Slice 1 (PR 1)

Satisfies: packaged-build-evidence / Pinned Build Environment, Retained Digest-Addressable Artifact.

- [x] 1.1 RED: extend `src-tauri/tests/config.rs` with a test asserting `bundle.targets` is an explicit non-default array; it fails against the current `"targets": "all"` string.
- [x] 1.2 GREEN: `src-tauri/tauri.conf.json` — replace `"targets": "all"` with the explicit pinned array. Re-run 1.1's test.
- [x] 1.3 `package.json` — add an exact-pinned `@tauri-apps/cli` devDependency (no range); lockfile regenerates (generated, excluded from the authored-line count). No repo test covers a devDependency pin; checked by `pnpm install --frozen-lockfile` and `pnpm exec tauri --version` reporting the pin.
- [x] 1.4 `.github/workflows/tauri-build.yml` — pin the Linux matrix entry `ubuntu-latest` → `ubuntu-24.04`; add `id: tauri` to the `tauri-apps/tauri-action` step; pin that action and `actions/upload-artifact` by exact tag/SHA (confirm `libwebkit2gtk-4.1` package name and `upload-artifact` permissions here — design Open Questions `[VERIFY at slice 1]`). No repo test covers workflow YAML; checked by YAML validity review and by the workflow's own run at Slice 4's authorized boundary.
- [x] 1.5 `.github/workflows/tauri-build.yml` — add a Linux-guarded "Record baseline facts" step: `os_build`/`architecture` (`/etc/os-release`, `uname -rm`), runner image version (transcribed from the `Set up job` log, not an env var), `webview_runtime` (`dpkg-query -W` on `libwebkit2gtk-4.1`), MSRV/toolchain/Tauri patch (`Cargo.toml` `rust-version`, `rustc -V`, `node -v`, `pnpm -v`, `git --version`, `cargo tree -p tauri --depth 0`). Checked the same way as 1.4.
- [x] 1.6 `.github/workflows/tauri-build.yml` — add a Linux-guarded "Digest artifact" step: `sha256sum` over `steps.tauri.outputs.artifactPaths`, fed into the pinned `actions/upload-artifact` step for retention. Checked the same way as 1.4.
- [x] 1.7 Rollback boundary: revert 1.2–1.6 (four files); packaging returns to build-only, nothing downstream exists yet.

## Phase 2: Evidence Store and Validator — Slice 2 (PR 2)

Satisfies: verification-evidence-store / Baseline-Before-Scenario Admission, Closed Outcome Vocabulary, Mandatory-Field Validation, Append-Only Correction, Provenance-Clean Publication; packaged-build-evidence / Packaged-Build-Only Admissibility, Exercised Artifact Matches Digested Artifact (validator half).

- [x] 2.1 `Cargo.toml` — add `"tools/*"` to `[workspace] members`.
- [x] 2.2 RED: `tools/evidence-validator/` table tests — each mandatory field missing in turn, per record kind; a scenario row's `result` and `observed_state` are two distinct mandatory fields (V1, `field-missing`).
- [x] 2.3 GREEN: implement `parse(text) -> Result<Vec<Record>, Vec<Violation>>` over the `##`-section / two-column table grammar, and V1 in `validate`.
- [x] 2.4 RED: table test — `result` outside `{pass, fail, uncertain}`, including a compound value like `orphan-risk/uncertain`, rejected as `result-out-of-vocabulary` (V2).
- [x] 2.5 GREEN: implement V2.
- [x] 2.6 RED: table tests — a scenario row with no matching baseline, and a baseline missing a VP-001-R1 field, both rejected as `baseline-unpinned` (V3).
- [x] 2.7 GREEN: implement V3 (baseline-before-scenario admission).
- [x] 2.8 RED: table tests — duplicate `record_id`, dangling `corrects`, reused `evidence_artifact` for the same `(scenario_id, baseline_id)`, and a path/host-shaped value (V4/V5/V6).
- [x] 2.9 GREEN: implement V4/V5/V6.
- [x] 2.10 RED: table tests — `exercised_artifact_digest` ≠ `build_channel_digest`, `variant_scan: found`, `variant_scan` missing, a non-`packaged-ci` `build_channel`, a digest with no retained `evidence_artifact` (V7/V8).
- [x] 2.11 GREEN: implement V7/V8.
- [x] 2.12 RED: derivation table tests for every `derive` row — `pass` (all three positive proofs), `fail` (recorded pid alive, same starttime), `uncertain`/`orphan-risk` for ungated-identity, unreadable-enumeration, and unconfirmed-stop; assert `result` and `observed_state` stay two separate fields, never a compound `result`.
- [x] 2.13 GREEN: implement `derive(Observations) -> (result, observed_state)` as a pure function.
- [x] 2.14 `docs/evidence/VP-001/README.md` — create: record schema (mirrors VP-001's own vertical `| field | value |` table), append-only rule, correction-by-`corrects` protocol, `record_id` uniqueness.
- [x] 2.15 Add fixtures under `tools/evidence-validator/tests/fixtures/` so the integration test (`cargo test --workspace`) has something to parse today; the real `docs/evidence/VP-001/{baselines,records}.md` files are created in Slice 4.
- [x] 2.16 Rollback boundary: revert `Cargo.toml`'s member addition and `tools/evidence-validator/`; no rows exist yet to orphan.

## Phase 3a: Harness Scaffolding and Feasibility Gate — Slice 3a (PR 3a)

Satisfies: packaged-build-evidence / Exercised Artifact Matches Digested Artifact (harness/AV1-AV2 half). Not blocked on CI-trigger authorization — the feasibility run uses a locally built artifact, not a `tauri-build.yml` dispatch.

- [x] 3a.1 RED: `docs/evidence/VP-001/procedures/webdriver-session.test.mjs` — `node --test` cases for the pure builders: new-session payload, element-locator payload, endpoint URL composition, W3C `value` unwrapping, error-object mapping. Written against not-yet-existing exports.
- [x] 3a.2 GREEN: `docs/evidence/VP-001/procedures/webdriver-session.mjs` — implement the pure builders (exported separately from the `fetch` transport) to pass 3a.1.
- [x] 3a.3 `.github/workflows/ci.yml` — add one `node --test docs/evidence/VP-001/procedures` step to `build-test`, run on all three runners (the logic is pure). **Deviation**: `node --test <directory>` (bare, no glob) tries to load the directory itself as a single CJS module on this pinned Node (22.23.2) and fails with `MODULE_NOT_FOUND`, rather than walking it for test files as documented; the step instead runs `node --test docs/evidence/VP-001/procedures/*.test.mjs`, letting the shell glob expand to the concrete file (verified locally with two independent minimal repros before deviating).
- [x] 3a.4 `docs/evidence/VP-001/procedures/vp-s6-linux.sh` — create the scaffold: **AV1** re-`sha256sum` the exact file about to be handed to `tauri-driver`, compare to `build_channel_digest`, abort (`artifact-digest-mismatch`) before any session on mismatch; **AV2** re-run the `feature_gate.rs` (read-only) byte-scan technique for `--demo-harness` against the `--appimage-extract` payload, abort (`feature-enabled-variant`) if found; then create a `tauri-driver` session against the gated file. No scenario logic yet (Slice 3b).
- [x] 3a.5 `.github/workflows/tauri-build.yml` — add the Linux-guarded harness step: install/launch `Xvfb`, `tauri-driver`, `xdotool`; invoke `vp-s6-linux.sh` in feasibility-check mode only.
- [x] 3a.6 **Feasibility gate execution — not a repository test.** Run `vp-s6-linux.sh` under a local `Xvfb` against a locally packaged artifact and record: **F1** `tauri-driver` creates a session against it; **F2** both GTK choosers complete under `Xvfb` via `xdotool` within a bounded time (if not, retry once with D7's seeded-approval fallback, recorded as a disclosed deviation, before declaring F2 failed); **F3** the run reaches an active state with an approved executable launched (a minimal placeholder satisfying the argv/stdin contract; full descendant-spawning behavior is re-confirmed once Slice 3b's `tools/vp-s6-agent/` lands). **Result: F1 PASS, F2 PASS, F3 FAIL** — see apply-progress.md for the exact evidence and the discovered blocker (an `ApprovalId` IPC-precision defect in already-shipped product code, not a harness/tooling limitation).
- [x] 3a.7 **Disclosed-uncertain branch**: if any of F1/F2/F3 fails with no fallback resolving it, stop the chain here. Do not proceed to 3b or 4. The blocker is recorded for the eventual VP-S6 row (Slice 4) once the evidence store exists to hold it; no further code slice lands. **Taken**: F3 failed with no applicable fallback (design's only fallbacks are F2's seeded-approval and AV2's `APPIMAGE_EXTRACT_AND_RUN`, neither of which covers an IPC-layer id mismatch). Slices 3b and 4 do not start in this apply batch.
- [x] 3a.8 Rollback boundary: revert the workflow harness step, both `webdriver-session.*` files, and the `ci.yml` step; nothing downstream depends on it yet. Confirmed: no later slice's code exists yet to depend on any Slice 3a file.

## Phase 3b: VP-S6 Scenario — Slice 3b (PR 3b)

Depends on: Slice 3a's F1–F3 gate having passed (directly, or via the F2 seeded-approval fallback) — do not start 3b if 3a filed the disclosed-uncertain branch. Not blocked on CI-trigger authorization.

**Status update (2026-09-17)**: this dependency is now satisfied. Slice 3a originally filed the
disclosed-uncertain branch (F3 FAIL) on a genuine, pre-existing `ApprovalId` IPC-precision product
defect — fixed on `main` by commit `5c5f474`, which this branch is rebased onto. See
apply-progress.md's "Addendum (2026-09-17)" under Slice 3a for the parent-confirmed local re-run
showing F1/F2/F3 all PASS. Phase 3b starts in this apply batch.

Satisfies: verification-evidence-store / VP-S6 Row Content and Recorded Tension, Blocked or Non-Reproducible Verification Attempt (scenario half); Threat Matrix rows Subprocess/process integration, Synthetic input, Executable-file classification.

- [x] 3b.1 `tools/vp-s6-agent/` — create the fixture ELF (D8): ignores argv/stdin (`StdinThenClose` contract, `crates/omnifrons-adapters/src/line_agent.rs` (read-only)), spawns `setsid sleep <n>` as a breakaway-attempting descendant, writes `pid starttime` for itself and the descendant into its cwd, then sleeps. No repo test drives a live process tree from here; checked by one manual `setsid`/`/proc` invocation before wiring into the harness (see apply-progress.md).
- [x] 3b.2 RED: confirm 2.12's derivation table tests already cover the exact survivor/proven-gone/recycled-pid/unreadable shapes this scenario emits; add any case discovered only while wiring the real scenario. **Confirmed, no gap found** — see apply-progress.md.
- [x] 3b.3 `docs/evidence/VP-001/procedures/vp-s6-linux.sh` — extend the 3a scaffold: `xdotool` drives the executable-approval and workspace-pick GTK choosers (`Ctrl+L`, type path, `Enter`) on the same `Xvfb` display; select adapter and approval; trigger Start; read the `State:` badge; read the fixture's pid file; record `(pid, starttime)` for agent and descendant from `/proc`; trigger Stop through the UI; bounded wait; re-enumerate; emit a `key=value` transcript. No repo test drives this GUI path; checked by one run producing a retained transcript (see apply-progress.md).
- [x] 3b.4 **Disclosed-deviation branch**: if 3a's F2 needed the seeded-approval fallback, record that deviation observation in the transcript rather than silently succeeding via the primary `xdotool` path. **N/A this run** — 3a's F2 passed directly (no seeded-approval fallback needed); the mechanism (a distinct `deviation=` transcript line) is not wired since it is not currently reachable, and is deferred until a run actually needs D7's fallback.
- [x] 3b.5 **Bounded-timeout branch**: a chooser that does not complete within the bound records `blocker=` and is never retried; `derive` maps this to `result: uncertain`. Implemented (`vp-s6-scenario.mjs`'s `blocker` mechanism, one bounded `pollUntil` per observation, no retry); not triggered in the local preview run since every chooser completed within its bound.
- [x] 3b.6 **Executable-classification branch**: no bypass of the product's own approval probe (`crates/omnifrons-adapters/src/fs_prober.rs` (read-only)); if launch is refused, `derive` refuses `pass` (already covered by 2.12; this task confirms the real scenario cannot short-circuit it). Confirmed: the local preview run approved and launched the fixture through the real UI flow with no shortcut.
- [x] 3b.7 Rollback boundary: revert the scenario logic added to `vp-s6-linux.sh`; delete `tools/vp-s6-agent/`; the harness (3a) stays inert but intact. Confirmed: reverting the four `vp-s6-scenario.mjs`/`vp-s6-xdotool.mjs`/`vp-s6-observations.mjs`/`vp-s6-observations.test.mjs` files, the scenario-mode block in `vp-s6-linux.sh`, and `tools/vp-s6-agent/` returns the repository to Slice 3a's state; nothing downstream (Slice 4) exists yet to depend on any of it.

## Phase 4: Baseline Record and VP-S6 Row — Slice 4 (PR 4)

Depends on: Slices 1, 2, 3a, 3b landed, and the Slice 3a feasibility-gate outcome.

Satisfies: both spec files' remaining requirements end to end.

- [ ] 4.1 **BLOCKED — explicit user authorization required.** Triggering `.github/workflows/tauri-build.yml` (`workflow_dispatch`) is a remote operation; request authorization once, exactly at this boundary, per design (no standing authorization exists). Do not proceed without an explicit grant.
- [ ] 4.2 **Depends on 4.1.** Trigger the authorized run on `ubuntu-24.04`; capture the retained artifact, its SHA-256 digest, and Slice 1's recorded baseline facts.
- [ ] 4.3 **Depends on 4.1/4.2 and the Slice 3a gate outcome.** If 3a's F1–F3 (and AV1/AV2) passed, run the full VP-S6 scenario (Slice 3b) against this run's retained artifact. If 3a filed the disclosed-uncertain branch, skip straight to 4.5's uncertain branch — never force a scenario run once feasibility already failed.
- [ ] 4.4 `docs/evidence/VP-001/baselines.md` — create: append one baseline record with every VP-001-R1 field (OS build, architecture, WebView/runtime version, packaging substrate, assistive-technology product authored as `none exercised — VP-S19 out of scope`, test date), sourced from 4.2. Validate with `cargo test --workspace` before proceeding.
- [ ] 4.5 `docs/evidence/VP-001/records.md` — create: append one VP-S6 row referencing that `baseline_id`, `build_channel: packaged-ci`, `build_channel_digest`, `exercised_artifact_digest`, `variant_scan`, mechanism (`process-group killpg(SIGTERM)`) and fallback (`killpg(SIGKILL) escalation`) as two distinct observations, and the honest outcome:
  - **disclosed-uncertain (expected)**: gates incomplete or termination unconfirmed → `result: uncertain`, `observed_state: orphan-risk`, blocker disclosed — the demonstrated `uncertain` this change exists to produce.
  - **fail**: a recorded pid alive with the same starttime after the bounded wait → `result: fail`, `observed_state: orphan-risk`.
  - **pass**: only if AV1/AV2 gated, descendant observed alive before stop, stop confirmed by the product's own terminal badge, every recorded pid/starttime proven gone.
  Validate against Slice 2's V1–V8 before considering the row filed.
- [ ] 4.6 Rollback boundary: a wrong row is corrected only by appending a new row referencing the original via `corrects`; never rewrite or delete it.

## Phase 5: Doc Reconciliation — Slice 5 (PR 5, independent, any position)

- [ ] 5.1 `README.md` — update the status line (currently line 7, "design phase") to reflect the merged repository state once Slices 1–4 (or 1–3b/5 if authorization was declined) have landed.
- [ ] 5.2 `docs/spike-log.md` — add a VP-001-appropriate entry reconciling this change; **note**: design's File Changes table names a `## Slice 5f` heading for this file, but no such heading or numbering fits this change — `docs/spike-log.md`'s existing sections run `## Slice 1` through `## Slice 5e` for prior ADR-0002 spikes, and `Slice 5f` textually matches only the already-archived, unrelated `2026-09-14-spike-slice-5f` change. Confirm the intended heading/content with the user before writing it; do not silently reuse that literal heading text.
- [ ] 5.3 Rollback boundary: revert both files; no code depends on their content.

## Key Learnings

1. Design's Migration/Rollout table is the authoritative per-slice forecast; each of the six slices lands independently under the 400-line budget.
2. The Slice 3a feasibility gate (F1–F3) and the Slice 4 CI-trigger authorization are two separate gates: only the latter needs the one-time explicit user grant, since 3a's proof uses a locally built artifact, not a workflow dispatch.
3. `derive` keeps `result` and `observed_state` as two always-separate fields; no task may collapse them into a compound value like `orphan-risk/uncertain`.
4. Design's File Changes table names a `## Slice 5f` heading for `docs/spike-log.md` that textually matches only the unrelated, already-archived 2026-09-14 change — flagged rather than silently propagated.
