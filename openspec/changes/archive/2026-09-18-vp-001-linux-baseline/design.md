# Design: VP-001 Linux Baseline — Retained Build, Evidence Store, First Row

## Technical Approach

Three layers, none touching product code. CI pins and retains one Linux bundle. A non-product
workspace member parses append-only records, validates them, and derives one honest outcome as a
pure function. A committed procedure drives the **retained packaged artifact's own GUI** through
`tauri-driver` under `xvfb` — after a digest/variant gate proves it is the digested artifact — and
enumerates descendants from `/proc`. VP-S6 measures the code that
exists (`crates/omnifrons-supervisor/src/lib.rs`: process-group `killpg`, SIGTERM then SIGKILL);
nothing is repaired, and `demo-harness` stays non-default with no new shipped CLI surface.

## Architecture Decisions

| Decision | Alternatives rejected | Choice and rationale |
|---|---|---|
| D1 Record format | 16-column pipe row (unreadable, brittle diffs); YAML/JSON plus a rendered view (new dependency, two artifacts to sync) | One `##` section per record holding a two-column `\| field \| value \|` table, mirroring VP-001's own vertical schema table (:157-174). An append is a pure end-of-file addition; a line-based parser needs no new crate. **Not** the GOV-001 ledger shape: `docs/evidence/GOV-001/ledger-2026-09-04.md` is a flat multi-column table with a bullet preamble, and is no precedent for this layout. |
| D2 Validator | test inside a product crate (governance tooling must not link into the shipped binary); shell script (no unit surface for strict TDD); renderer test (wrong layer); `xtask` (implies a task runner this repo lacks) | New member `tools/evidence-validator/`; `members = ["crates/*", "src-tauri", "tools/*"]`. `cargo test --workspace` already runs on CI, so a malformed record fails the repository's own test run with no workflow change. Precedent: `src-tauri/tests/config.rs` validates a repo file via `CARGO_MANIFEST_DIR`. |
| D3 Layout | one file per record; one file per baseline | `docs/evidence/VP-001/{README.md,baselines.md,records.md,procedures/}`, both `.md` append-only. Store-level `record_id` plus optional `corrects`. |
| D4 CI wiring | separate evidence workflow (a second remote-authorization surface); `ubuntu-latest` | Pin the Linux matrix entry to `ubuntu-24.04`; pin `tauri-apps/tauri-action` and `actions/upload-artifact` (v7.0.1, re-verify at slice 1) by exact tag or SHA; feed `artifactPaths` into upload; add Linux-guarded facts/digest/harness/upload steps to the existing `package` job. macOS and Windows entries keep their current step set. |
| D5 VP-S6 execution (**supersedes the earlier `xvfb-run` observation script**) | enable `demo-harness` for evidence (measures a build nobody installs; `src-tauri/tests/feature_gate.rs` asserts the feature is off by default and its literal string absent from the binary); assert from the dev-mode suite (barred by VP-001-R2) | `tauri-driver` + `WebKitWebDriver` under `Xvfb`, driving the retained bundle itself — the digested file, gate-checked before session creation — through its shipped GUI. `src-tauri/src/main.rs:14-40` compiles the only non-GUI path out by default and would make the binary the child rather than the supervisor. |
| D6 Driver client | WebdriverIO/Selenium (large dependency tree, config surface, lockfile churn); Rust `fantoccini` (adds tokio + client deps to a workspace member compiled and linted on all three runners) | A zero-dependency Node script speaking W3C WebDriver over global `fetch` (Node 22 is already pinned in `package.json`; `pnpm-workspace.yaml` covers only `renderer`, so it joins no workspace). ~120 authored lines keeps slice 3a inside budget. |
| D7 Native-dialog gap | pre-seed the approval store from a `tools/` seeder linking `JsonlApprovalStore` (device state the product would have written, but bypasses the approval UI **and cannot cover the workspace slot**, which is in-memory only) | Drive the two GTK choosers with `xdotool` on the same `Xvfb` display (`Ctrl+L`, type path, `Enter`). It is the real user path; seeding stays the disclosed fallback if F2 below fails. |
| D8 Fixture agent | POSIX shell script (the Linux launch path execs a sealed memfd `/proc/self/fd/N`; shebang resolution through it is not something evidence should rest on); approving `/bin/sh` (the adapter's fixed `argv_template` flags are rejected by `sh`) | `tools/vp-s6-agent/`: a tiny ELF that ignores argv/stdin, spawns `setsid sleep <n>` as a breakaway-attempting descendant, writes `pid starttime` for itself and the descendant into its cwd, then sleeps. |
| D9 Slices | the proposal's four (pre-amendment) | The amended proposal's six, forecast below. |

## D5/D7: the GUI path and what blocks it

Verified preconditions for Start, and why WebDriver alone is insufficient:

| Fact | Source | Consequence |
|---|---|---|
| Start needs an adapter **and** an approval | `renderer/src/AgentPanel.tsx:4559` | both selects must be driven in the DOM |
| `harness_spawn` needs an active workspace | `src-tauri/src/ipc/commands.rs:1199` | a workspace pick is on the critical path |
| Executable approval opens a native file chooser | `commands.rs:1677` (`blocking_pick_file`) | outside WebKitWebDriver's DOM → `xdotool` |
| Workspace pick opens a native folder chooser, stored in memory only (`AdapterState.active_workspace`) | `commands.rs:1812`, `src-tauri/src/adapter_state.rs:18` | cannot be seeded; must be driven per launch |
| Prober accepts any regular file with an exec bit | `crates/omnifrons-adapters/src/fs_prober.rs:154` | the fixture needs no signature, but D8's ELF rule still holds |
| `stream-json-cli` is `StdinThenClose` with a fixed argv template | `crates/omnifrons-adapters/src/line_agent.rs:74,120` | the fixture must ignore argv and a closed stdin |

Sequence (one pass, no loop): `Xvfb` → `tauri-driver` → fetch the retained bundle → **AV1/AV2
identity gate** → create session against that exact file → `xdotool` workspace + executable → **Approve** → select
adapter and approval → **Start** → read the `State:` badge → read the fixture's pid file → record
`(pid, starttime)` for agent and descendant from `/proc` → **Stop** → bounded wait → re-enumerate →
emit a `key=value` transcript.

## Honesty machinery

A flaky or blocked run cannot become a `pass`:

1. The procedure emits observations only and never names an outcome; `derive` is the sole place a
   result exists, and it is a pure unit-tested function in `tools/evidence-validator`.
2. `pass` requires three positive proofs — descendant observed alive before stop, stop confirmed by
   the product's own terminal badge, every recorded `(pid, starttime)` proven gone. Any absent or
   unreadable observation yields `uncertain`.
3. No retry: one procedure invocation per authorized run, no loop, no `continue-on-error` re-run.
4. Append-only: a later run cannot replace a row, and validator check V5 forces a distinct
   `evidence_artifact` per `(scenario_id, baseline_id)`, so re-running until green is visible as
   several rows rather than one.
5. Feasibility gate (slice 3a), each branch fixed in advance:
   - **F1** `tauri-driver` creates a session against the retained packaged artifact.
   - **F2** both GTK choosers complete under `Xvfb` within a bounded time.
   - **F3** the run reaches an active state with the fixture launched.
   Any F failing stops the chain at 3a; VP-S6 records `uncertain` with the blocker disclosed.
   Never a special build, never a retry until green, never a renamed outcome. F2 alone may instead
   take D7's seeded-approval fallback, recorded as a disclosed deviation observation.
6. Artifact identity gate, every run, **before** the session exists — the mechanism behind
   *Exercised Artifact Matches Digested Artifact*:
   - **AV1** re-`sha256sum` the exact file the procedure is about to hand `tauri-driver` and compare
     it to the `build_channel_digest` the CI `Digest artifact` step recorded. A mismatch aborts
     before session creation and files no row (`artifact-digest-mismatch`).
   - **AV2** re-run the `feature_gate.rs` byte-scan (`src-tauri/tests/feature_gate.rs:28-40`) for the
     literal `--demo-harness` against the *payload* executable extracted from that same
     digest-verified file (`--appimage-extract`): the container is compressed, so scanning it
     directly would be a false negative. Found → abort (`feature-enabled-variant`).
   Direct AppImage execution needs FUSE on `ubuntu-24.04` (`[VERIFY at slice 3a]`); the fallback is
   `APPIMAGE_EXTRACT_AND_RUN=1` over that same gated file, recorded as a disclosed deviation
   observation — never a second build, never a feature-enabled one. AV1 and AV2 are preconditions of
   `pass`; a gate that cannot be performed is a blocker yielding `uncertain`.

`derive(Observations) -> (result, observed_state)`. A scenario record carries the two as **separate
fields**; `orphan-risk/uncertain` is only their human-readable rendering, never a stored `result`.

| Observation | `result` | `observed_state` |
|---|---|---|
| AV1/AV2 gated, descendant alive before stop, stop confirmed, every recorded pid/starttime proven gone | `pass` | public token `[VERIFY]` |
| any recorded pid alive with the same starttime after the bounded wait | `fail` | `orphan-risk` |
| identity gate not completed, session blocked, launch refused, stop unconfirmed, or any enumeration unreadable | `uncertain` | `orphan-risk` |

Row three is the expected outcome and is the demonstrated `uncertain` VP-001:283 requires. The row
records `mechanism=process-group killpg(SIGTERM)` and `fallback=killpg(SIGKILL) escalation` as two
distinct observations (`crates/omnifrons-supervisor/src/lib.rs:8`), carrying the VP-001-R6 tension
forward without resolving it.

## Validator contract

`parse(text) -> Result<Vec<Record>, Vec<Violation>>` over the section/table grammar;
`validate(records) -> Vec<Violation>`; `derive(Observations)`. No product crate depends on any of
them.

| # | Check | Rejection token |
|---|---|---|
| V1 | every mandatory schema field present and non-empty, per record kind — a scenario row's `result` and `observed_state` are two distinct mandatory fields | `field-missing` |
| V2 | `result` ∈ {`pass`, `fail`, `uncertain`}; a compound value such as `orphan-risk/uncertain` is rejected, the state belonging in `observed_state` | `result-out-of-vocabulary` |
| V3 | a scenario row's `baseline_id` resolves to a baseline record carrying every VP-001-R1 field | `baseline-unpinned` |
| V4 | `record_id` unique; `corrects` resolves to an existing `record_id` | `duplicate-record-id`, `corrects-unresolved` |
| V5 | rows sharing `(scenario_id, baseline_id)` carry distinct `evidence_artifact` values | `evidence-artifact-reused` |
| V6 | partial provenance lint: no value carrying an absolute-path or host-shaped token (a lint, not a proof) | `provenance-leak` |
| V7 | a scenario row's `exercised_artifact_digest` equals its `build_channel_digest`, and `variant_scan` is `absent` (AV1/AV2 recorded, not asserted) | `artifact-digest-mismatch`, `feature-enabled-variant` |
| V8 | `build_channel` is the packaged CI channel **and** the row carries both a `build_channel_digest` and the `evidence_artifact` retaining it; any other channel, or a digest with no retained artifact, is refused | `dev-mode-only` |

## Evidence field provenance (Linux-guarded steps)

| Field | Producing step |
|---|---|
| `os_build`, `architecture` | `Record baseline facts`: `/etc/os-release`, `uname -rm` |
| runner image version | transcribed by the operator from the authorized run's `Set up job` log — no source-confirmed env var; recorded as transcribed |
| `webview_runtime` | same step: `dpkg-query -W` on the installed `libwebkit2gtk-4.1` runtime package — exact package name `[VERIFY at slice 1]` |
| MSRV, toolchain, Tauri patch | same step: `Cargo.toml` `rust-version`, `rustc -V`, `node -v`, `pnpm -v`, `git --version`, `cargo tree -p tauri --depth 0` (resolved, not the caret pin) |
| `packaging_substrate` | `bundle.targets` in `tauri.conf.json` plus the retained file's format |
| `build_channel` | authored as `packaged-ci` — the only value V8 admits. A `tauri dev` or local build runs no `Digest artifact` step, so it can populate neither this field nor the next, and a row claiming otherwise is refused as `dev-mode-only` |
| `build_channel_digest` | `Digest artifact`: `sha256sum` over `steps.tauri.outputs.artifactPaths` — written by that CI step alone |
| `exercised_artifact_digest` | AV1: `sha256sum` recomputed by `vp-s6-linux.sh` over the exact file handed to `tauri-driver` |
| `variant_scan` | AV2: `absent`, `found`, or `unperformed` from the `--demo-harness` byte-scan over the extracted payload |
| `procedure_ref` | `docs/evidence/VP-001/procedures/vp-s6-linux.sh` (opaque string to the validator) |
| `evidence_artifact` | `Upload artifact` (pinned `actions/upload-artifact`): bundle, facts, VP-S6 transcript |
| `assistive_technology` | authored as `none exercised — VP-S19 out of scope`, recorded as a fact, never omitted |

Bundles are not byte-reproducible (AppImage `.digest_md5`): a row binds the digest of the one
retained artifact and never claims rebuild identity. AV1 compares that one file to its own recorded
digest — it is an identity check, not a reproducibility claim.

## Data Flow

    tauri-action (pinned, id: tauri) ──artifactPaths──> Digest ──> AV1/AV2 gate ──> vp-s6-linux.sh
                                             │                                      │
                                             │                    tauri-driver ── Xvfb ── xdotool
                                             │                                      │
                                             └── baseline facts ──> authored records ┴──> upload-artifact
                                                                            │
                                    cargo test --workspace ──> evidence-validator (parse, validate, derive)

## File Changes

| File | Action | Description |
|---|---|---|
| `.github/workflows/tauri-build.yml` | Modify | `ubuntu-24.04`; pinned actions with an `id`; Linux-guarded facts, digest, driver/harness and upload steps |
| `src-tauri/tauri.conf.json` | Modify | explicit `bundle.targets` array, replacing `"all"` |
| `package.json` | Modify | exact-pinned `@tauri-apps/cli` devDependency (lockfile generated, excluded from the authored count) |
| `src-tauri/tests/config.rs` | Modify | assert `bundle.targets` is the explicit pinned array |
| `Cargo.toml` | Modify | add `tools/*` to workspace members |
| `tools/evidence-validator/` | Create | parser, `validate`, `derive`, fixtures, tests |
| `tools/vp-s6-agent/` | Create | fixture ELF agent (D8) |
| `docs/evidence/VP-001/README.md` | Create | schema, append-only rule, correction by `corrects` |
| `docs/evidence/VP-001/procedures/vp-s6-linux.sh` | Create | observation-only orchestration |
| `docs/evidence/VP-001/procedures/webdriver-session.mjs` | Create | zero-dependency W3C client; pure builders exported separately from the `fetch` transport so they are unit-testable |
| `docs/evidence/VP-001/procedures/webdriver-session.test.mjs` | Create | `node --test` unit tests over those pure builders |
| `.github/workflows/ci.yml` | Modify | one `node --test docs/evidence/VP-001/procedures` step in `build-test` |
| `docs/evidence/VP-001/{baselines,records}.md` | Create | one baseline record, one VP-S6 row |
| `README.md`, `docs/spike-log.md` | Modify | status line; `## Slice 5f` section |

## Testing Strategy

| Layer | What to test | Approach |
|---|---|---|
| Unit | V1 — each mandatory field missing in turn, per record kind | table tests, `tools/evidence-validator` |
| Unit | V2 — `result` outside `{pass,fail,uncertain}` is rejected | table test over out-of-vocabulary values |
| Unit | V3 — a row whose `baseline_id` has no baseline record, and one whose baseline omits a VP-001-R1 field, are both rejected as `baseline-unpinned` | table tests |
| Unit | V4/V5/V6 — duplicate id, dangling `corrects`, reused `evidence_artifact`, path-shaped value | table tests |
| Unit | V7/V8 — `exercised_artifact_digest` ≠ `build_channel_digest`, `variant_scan: found`, `variant_scan` missing, a non-packaged `build_channel`, a digest with no retained `evidence_artifact` | table tests |
| Unit | every `derive` row, including the ungated-identity, unreadable-enumeration and unconfirmed-stop paths, and that `result` never carries a compound state | derivation table tests |
| Unit | `webdriver-session.mjs` pure logic — new-session payload, element-locator payload, endpoint URL composition, W3C `value` unwrapping and error-object mapping — written RED before the client, GREEN after | `node --test` (Node's built-in runner; Node 22.23.2 is already pinned by `package.json` `devEngines`, so no new dependency and no pnpm workspace member), run by the new `ci.yml` step on all three runners since the logic is pure |
| Integration | the real `docs/evidence/VP-001/*.md` parse and validate | `cargo test --workspace` |
| Config | `bundle.targets` is explicit | `src-tauri/tests/config.rs` |
| E2E | the **live browser-driving path only** — session attach, native-dialog driving, real GUI interaction — is evidence machinery, not a repository test, and never gates CI | one authorized run, transcript retained; this disclaimer does not extend to the client's pure logic, covered by the unit row above |

## Threat Matrix

| Boundary | Applicability | Design response | Planned RED tests |
|---|---|---|---|
| Documentation-like paths | Applicable: executable `.sh`/`.mjs` under `docs/` | the validator treats `procedure_ref` as an opaque string, never opening or executing it; the scripts run only from one named, authorized CI step | a record whose `procedure_ref` names an executable path validates with no filesystem access |
| Subprocess / process integration | Applicable: the fixture spawns a breakaway-attempting descendant and the product group-terminates it | single stop, bounded wait, no retry, no repair; unprovable descendants yield `uncertain`; pid identity compared by `(pid, starttime)` so recycling never reads as a survivor | derivation rows for survivor, proven-gone, recycled-pid, unreadable |
| Synthetic input (`xdotool`) | Applicable: X11 keystrokes into native choosers | confined to one `Xvfb` display created by the procedure; typed values are the harness's own paths, never harness-originated text; a chooser that does not complete within the bound is F2-blocked, never retried | a bounded-timeout branch records `blocker=` and derives `uncertain` |
| Packaged-artifact substitution | Applicable: the harness launches a distributable artifact a row then claims by digest | AV1 digest equality and AV2 `--demo-harness` payload scan both run before session creation and abort the run, filing no row; V7 independently refuses a row whose two digests differ or whose scan is not `absent`; V8 refuses a non-packaged `build_channel`; an unperformable gate is a blocker, never a `pass` | a row with mismatched digests is rejected; a row with `variant_scan: found` is rejected; a `dev-mode-only` row is rejected; `derive` yields `uncertain` when the gate did not complete |
| Executable-file classification | Applicable: the fixture is approved through the product's own gate | no bypass — the product probes and re-probes at launch; the fixture is built by CI from repository source | `derive` refuses `pass` when the launch was refused |
| Git / commit / push / PR automation | N/A: nothing stages, commits, pushes, or opens a PR | — | — |

## Migration / Rollout

No migration. Six stacked-to-main slices, each forecast under the 400 authored-line budget
(additions + deletions; generated lockfiles excluded).

| # | Slice | Forecast | Rollback |
|---|---|---|---|
| 1 | Packaging pin and retention: workflow, `tauri.conf.json`, `package.json`, `config.rs` | ~180 | revert; packaging returns to build-only |
| 2 | Evidence store and validator: `Cargo.toml`, `tools/evidence-validator/`, store README | ~280 | revert; no rows exist to orphan |
| 3a | Harness scaffolding: driver/`xvfb`/`xdotool` CI wiring, `webdriver-session.mjs`, session against the retained bundle, AV1/AV2 and F1–F3 gates | ~220 | revert; nothing depends on it |
| 3b | VP-S6 scenario: `tools/vp-s6-agent/`, dialog driving, UI-triggered stop, `/proc` enumeration, transcript, `derive` | ~260 | revert the scenario; the harness stays inert |
| 4 | Baseline record + VP-S6 row (needs 1, 2, 3a, 3b and an authorized run) | ~200 | append a correction row; never rewrite |
| 5 | `README.md`, `docs/spike-log.md` (independent; any position) | ~60 | revert |

V7/V8 land with the validator in slice 2; AV1/AV2 land with the session in slice 3a, whose ~220
covers the client, its `node --test` unit test, and the `ci.yml` step. If 3a's authored count
approaches the ceiling, it splits again (CI wiring, then client) rather than being compressed.

**Authorization.** Triggering `tauri-build.yml` is a remote operation. Explicit user authorization
is requested once, at the boundary between slice 3b and slice 4, and nowhere else; no standing
authorization exists. If declined, slice 4 does not start and the change closes with 1–3b (and
optionally 5) merged. Evidence is append-only under signed-commit, linear-history `main`
(GOV-001-R15, VP-001-R20); every published value is provenance-clean.

## Open Questions

- [ ] VP-001-R6 (:227) requires the Linux fallback as its own scenario while the catalog folds it
      into VP-S6 (:127). Carried forward, not resolved: mechanism and fallback are recorded as two
      distinct observations in one row, and the tension is reported.
- [ ] `record_id` and `corrects` are store-level fields the VP-001 record table (:157-174) does not
      name. A VP-001 revision should adopt them.
- [ ] The public state token for a `pass` is unnamed; `[VERIFY]` against
      `versioning-and-compatibility.md § Synchronization states` before slice 4.
- [ ] Whether `ubuntu-24.04` can execute the AppImage directly (FUSE) or needs
      `APPIMAGE_EXTRACT_AND_RUN=1`. `[VERIFY at slice 3a]`; either branch launches the gated file.
- [ ] `actions/upload-artifact` may need permissions beyond the workflow's `contents: read`, and
      the `libwebkit2gtk` runtime package name is unverified. Both `[VERIFY at slice 1]`.
