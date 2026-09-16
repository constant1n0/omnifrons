# Apply Progress: vp-001-linux-baseline

## Slice 1 (Phase 1, PR 1): Packaging Pin and Retention

### Scope

Phase 1 only: "Packaging Pin and Retention" (tasks 1.1–1.7). Slices 2, 3a, 3b, 4, and 5 are
untouched — no `Cargo.toml` workspace-member change, no `tools/`, no `docs/evidence/`.

### Completed Tasks

- [x] 1.1 RED: `src-tauri/tests/config.rs` — `bundle_targets_is_an_explicit_non_default_array`
      asserts `bundle.targets` is an explicit array; failed against `"targets": "all"`.
- [x] 1.2 GREEN: `src-tauri/tauri.conf.json` — `bundle.targets` is now
      `["deb", "rpm", "appimage", "msi", "nsis", "app", "dmg"]` (Tauri's own `BundleType` enum,
      confirmed against `tauri-utils` source at the pinned `tauri = "2.11.5"` tag); 1.1's test passes.
- [x] 1.3 `package.json` — `@tauri-apps/cli` pinned to exact `2.11.4` (latest published 2.x release;
      no `^`/`~` range); `pnpm-lock.yaml` regenerated (generated, excluded from the authored count).
- [x] 1.4 `.github/workflows/tauri-build.yml` — matrix `ubuntu-latest` → `ubuntu-24.04`; `id: tauri`
      added to the `tauri-apps/tauri-action` step; that action pinned to `@action-v1.0.0`,
      `actions/upload-artifact` pinned to `@v7.0.1` (both exact tags, per the parent's verified
      latest-release facts).
- [x] 1.5 `.github/workflows/tauri-build.yml` — Linux-guarded "Record baseline facts" step:
      `os_build`/`architecture` (`/etc/os-release`, `uname -rm`), `webview_runtime`
      (`dpkg-query -W libwebkit2gtk-4.1-0`), MSRV/toolchain/Tauri patch (`Cargo.toml` `rust-version`,
      `rustc -V`, `node -v`, `pnpm -v`, `git --version`, `cargo tree -p tauri --depth 0`). The exact
      runner image version is documented in-step as operator-transcribed at Slice 4, not captured
      by an env var (none exists).
- [x] 1.6 `.github/workflows/tauri-build.yml` — Linux-guarded "Digest artifact" step: `sha256sum`
      over every path in `steps.tauri.outputs.artifactPaths` (a JSON array, confirmed from
      `tauri-action`'s `action-v1.0.0` bundled source), captured into a multiline step output; a
      Linux-guarded "Upload artifact" step (pinned `actions/upload-artifact@v7.0.1`) retains the
      built bundle(s), the facts file, and the digest file.
- [x] 1.7 Rollback boundary confirmed: reverting `src-tauri/tauri.conf.json`,
      `src-tauri/tests/config.rs`, `package.json` + `pnpm-lock.yaml`, and
      `.github/workflows/tauri-build.yml` returns packaging to build-only; nothing downstream
      (Slices 2–5) exists yet to orphan.

### Verified During Implementation (Open Questions `[VERIFY at slice 1]`)

- `libwebkit2gtk-4.1` package name: confirmed as `libwebkit2gtk-4.1-0` (runtime) /
  `libwebkit2gtk-4.1-dev` (already installed by `setup-toolchain` on Linux) via local
  `apt-cache`/`dpkg -l` — matches the already-`-dev`-installed prerequisite in
  `.github/actions/setup-toolchain/action.yml`.
- `actions/upload-artifact` permissions: its README documents no `permissions:` requirement beyond
  the default token; the workflow's existing `contents: read` is unchanged and sufficient.
- Tauri `bundle.targets` explicit array behavior: confirmed via `tauri-bundler`'s
  `Settings::package_types()` (pinned `tauri` tag `tauri-v2.11.5`) that an explicit list is filtered
  to the current platform's applicable subset — reproducing prior `"all"` behavior per-OS while
  being an explicit, non-default config value.
- `@tauri-apps/cli` pin: `2.11.4` is the latest published stable 2.x release (checked via
  `npm view @tauri-apps/cli versions`), closest to the already-pinned Rust `tauri = "2.11.5"` /
  `@tauri-apps/api@2.11.1`.

### TDD Cycle Evidence

| Task | Layer | RED | GREEN | REFACTOR |
|---|---|---|---|---|
| 1.1/1.2 | Unit (`src-tauri/tests/config.rs`) | `bundle_targets_is_an_explicit_non_default_array` failed: `bundle.targets must be an explicit array... found: String("all")` | same test passed after `tauri.conf.json` edit; full `config` suite 5/5 | moved `KNOWN_TAURI_BUNDLE_TYPES` to module scope (clippy `items_after_statements`); backtick-quoted `tauri-utils::config` (clippy `doc_markdown`) |

### Work Unit Evidence

| Evidence | Value |
|---|---|
| Focused test command and exact result | `cargo test -p omnifrons-shell --test config` → `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| Runtime harness command/scenario and exact result | N/A — per tasks.md, this slice's CI YAML has no repository test harness; the workflow's own run is checked at Slice 4's authorized boundary, not here |
| Rollback boundary | Revert 4 files (`src-tauri/tauri.conf.json`, `src-tauri/tests/config.rs`, `package.json` + `pnpm-lock.yaml`, `.github/workflows/tauri-build.yml`); packaging returns to build-only, nothing downstream exists |

### Commands Run (exact observed results)

- `cargo test -p omnifrons-shell --test config` (RED, before 1.2): 1 failed (`bundle_targets_is_an_explicit_non_default_array`), 4 passed.
- `cargo test -p omnifrons-shell --test config` (GREEN, after 1.2): `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`.
- `pnpm install` (after 1.3's `package.json` edit): `Packages: +2`, `devDependencies: + @tauri-apps/cli 2.11.4`, `Done in 3s`.
- `pnpm install --frozen-lockfile`: `Already up to date`, `Done in 504ms`.
- `pnpm exec tauri --version`: `tauri-cli 2.11.4`.
- `cargo fmt --all -- --check`: no output (clean).
- `cargo clippy --workspace --all-targets -- -D warnings`: `Finished` (no warnings/errors) after fixing two lints in the new test.
- `cargo test --workspace`: all reported suites `test result: ok`, zero `FAILED` lines across the full run (Rust + doctests).
- `python3 -c "import yaml; yaml.safe_load(...)"` against `.github/workflows/tauri-build.yml`: `YAML parses OK`; matrix/step structure printed and matched expectations (`id: tauri`, `id: digest`, pinned `uses:` values).
- `git diff --stat -- . ':!pnpm-lock.yaml'`: `4 files changed, 104 insertions(+), 5 deletions(-)`.

### Files Changed

| File | Action | What Was Done |
|---|---|---|
| `src-tauri/tests/config.rs` | Modified | Added `bundle_targets_is_an_explicit_non_default_array` (RED then GREEN) plus module-level `KNOWN_TAURI_BUNDLE_TYPES`. |
| `src-tauri/tauri.conf.json` | Modified | `bundle.targets`: `"all"` → explicit `["deb","rpm","appimage","msi","nsis","app","dmg"]`. |
| `package.json` | Modified | Added exact-pinned `devDependencies["@tauri-apps/cli"] = "2.11.4"`. |
| `pnpm-lock.yaml` | Modified (generated) | Regenerated by `pnpm install`; excluded from the authored-line budget. |
| `.github/workflows/tauri-build.yml` | Modified | `ubuntu-latest` → `ubuntu-24.04`; `id: tauri` + pinned `tauri-action@action-v1.0.0`; new Linux-guarded "Record baseline facts", "Digest artifact", and "Upload artifact" (pinned `actions/upload-artifact@v7.0.1`) steps. |

### Review Budget

Authored (additions + deletions), excluding the generated lockfile: **109 lines**
(`104 insertions + 5 deletions` across `tauri-build.yml`, `package.json`, `tauri.conf.json`,
`config.rs`) — well under the 400-line budget and under the design's own ~180-line forecast for
this slice.

### Workload / PR Boundary

- Mode: chained PR slice (`auto-chain`, chain strategy `stacked-to-main`).
- Current work unit: Unit 1 — "Pin packaging, retain the artifact, digest it" (PR 1).
- Boundary: starts from the current build-only workflow/config and ends with a pinned, retaining,
  digesting Linux packaging step; introduces no evidence-store, harness, or scenario code (those
  are Slices 2, 3a, 3b, 4).
- Estimated review budget impact: 109 authored lines — low.

### Deviations from Design

None — implementation matches design.md's D4 (CI wiring), the Evidence field provenance table, and
the File Changes table for the four files this slice touches.

### Post-Verification Corrections

Native risk assessment rated this candidate `high` (`shell_source` signal on the workflow), so an
independent verifier reviewed the diff after the writer's own checks. Two defects were found and
corrected in `.github/workflows/tauri-build.yml`, in the `Digest artifact` step:

1. `${{ steps.tauri.outputs.artifactPaths }}` was spliced twice directly into the shell body. It now
   reaches the script through `env: ARTIFACT_PATHS_JSON` and is referenced as a quoted variable, so a
   path or `productName` containing a quote can no longer be parsed as shell.
2. The digest file recorded absolute runner paths. It now records `<sha256>  <basename>`, keeping the
   retained public evidence provenance-clean (desktop-stack-verification-plan.md:178).

Both corrections are confined to that one step. `cargo test -p omnifrons-shell --test config` was
re-run after them: 5 passed, 0 failed.

### Status

7/7 Phase 1 tasks complete. Ready for `sdd-verify` on this slice, or for `sdd-apply` to continue
with Phase 2 in a later batch.

## Slice 2 (Phase 2, PR 2): Evidence Store and Validator

### Scope

Phase 2 only: "Evidence Store and Validator" (tasks 2.1–2.16). No `.github/workflows/`, no
WebDriver harness, no `tools/vp-s6-agent/`. `docs/evidence/VP-001/{baselines,records}.md` are
bootstrapped as empty (header-only) files — no real evidence row is authored; all record examples
live under `tools/evidence-validator/tests/fixtures/` and are explicitly named `synthetic-*`.

### Completed Tasks

- [x] 2.1 `Cargo.toml` — added `"tools/*"` to `[workspace] members`.
- [x] 2.2–2.3 (V1): `tools/evidence-validator/src/record.rs` implements
      `parse(text) -> Result<Vec<Record>, Vec<Violation>>` over the `##`-section / two-column
      table grammar; `src/validate.rs` implements V1 (`field-missing`) per record kind
      (`baseline` / `scenario`), with `result` and `observed_state` as two independently
      mandatory scenario fields.
- [x] 2.4–2.5 (V2): closed `result` vocabulary (`pass`/`fail`/`uncertain`); a compound value such
      as `orphan-risk/uncertain` is rejected as `result-out-of-vocabulary`.
- [x] 2.6–2.7 (V3): a scenario row's `baseline_id` must resolve to a baseline record carrying
      every VP-001-R1 field; an unresolved id or an incomplete baseline is `baseline-unpinned`.
- [x] 2.8–2.9 (V4/V5/V6): unique `record_id` (`duplicate-record-id`), resolvable `corrects`
      (`corrects-unresolved`), distinct `evidence_artifact` per `(scenario_id, baseline_id)`
      (`evidence-artifact-reused`), and a provenance lint against absolute-path/host-shaped
      values (`provenance-leak`).
- [x] 2.10–2.11 (V7/V8): `exercised_artifact_digest` must equal `build_channel_digest`
      (`artifact-digest-mismatch`); `variant_scan` must be exactly `absent`
      (`feature-enabled-variant`); `build_channel` must be `packaged-ci`, and a
      `build_channel_digest` may never appear with no retained `evidence_artifact`
      (both `dev-mode-only`).
- [x] 2.12–2.13: `src/derive.rs` implements `derive(Observations) -> (Outcome, ObservedState)` as
      a pure function — two always-separate enum values, never a compound result.
- [x] 2.14 `docs/evidence/VP-001/README.md` — record grammar, mandatory-field tables per kind,
      append-only + `corrects` correction protocol, `record_id` uniqueness, provenance rule.
- [x] 2.15 Fixtures under `tools/evidence-validator/tests/fixtures/synthetic-{baselines,records}.md`
      (obviously synthetic ids/digests) plus an integration test parsing and validating them
      clean, and confirming a `corrects` row resolves while the original stays unchanged.
- [x] 2.16 Rollback boundary confirmed: reverting `Cargo.toml`'s member addition and deleting
      `tools/evidence-validator/` returns the workspace to Slice 1's state; no evidence rows exist
      yet to orphan (`docs/evidence/VP-001/{baselines,records}.md` stay header-only either way).

### TDD Cycle Evidence

| Task(s) | Test file | RED (observed) | GREEN (observed) |
|---|---|---|---|
| 2.2/2.3 (V1) | `tests/v1_mandatory_fields.rs` | Compile error: `record`/`validate` modules did not exist | `cargo test -p evidence-validator`: 4/4 passed after implementing `parse` + V1 |
| 2.4/2.5 (V2) | `tests/v2_result_vocabulary.rs` | `out_of_vocabulary_result_is_rejected` failed: `got []` | 2/2 passed after implementing V2 |
| 2.6/2.7 (V3) | `tests/v3_baseline_before_scenario.rs` | 2/3 failed (`no matching baseline`, `missing VP-001-R1 field`): `got []` | 3/3 passed after implementing V3 |
| 2.8/2.9 (V4/V5/V6) | `tests/v4_v5_v6_identifiers_and_provenance.rs` | 5/8 failed (duplicate id, dangling corrects, reused artifact, absolute path, host-shaped): `got []` | 8/8 passed after implementing V4/V5/V6 |
| 2.10/2.11 (V7/V8) | `tests/v7_v8_artifact_identity_and_channel.rs` | 4/7 failed (digest mismatch, non-packaged channel, digest-no-artifact, variant found): `got []` | 7/7 passed after implementing V7/V8 |
| 2.12/2.13 (derive) | `tests/derive.rs` | Compile error: `derive` module did not exist | `cargo test -p evidence-validator`: 7/7 passed after implementing `derive` |

Triangulation: every check above ships with both a rejecting case and an accepting case in the
same or a companion test (e.g. `each_closed_vocabulary_value_is_accepted`,
`packaged_row_with_matching_digests_and_artifact_is_accepted`), so no GREEN result is a trivial
pass from an empty check. Refactor: after GREEN, `cargo clippy -p evidence-validator --all-targets
-- -D warnings` and `cargo fmt --all` were run to convergence (see Commands Run); no test was
weakened to reach green.

### Work Unit Evidence

| Evidence | Value |
|---|---|
| Focused test command and exact result | `cargo test -p evidence-validator` → all 6 test binaries + unit tests `test result: ok` (37 tests: 4 unit + 33 integration across the six V-group files, `derive.rs`, and `fixtures_parse_and_validate.rs`) |
| Runtime harness command/scenario and exact result | N/A — pure unit/integration tests only; no runtime boundary exists for a non-product validator crate (per tasks.md's own Unit 2 row) |
| Rollback boundary | Revert `Cargo.toml`'s `tools/*` member line; delete `tools/evidence-validator/` and `docs/evidence/VP-001/`; no evidence rows exist yet to orphan |

### Commands Run (exact observed results)

- `cargo test -p evidence-validator` (repeated after every GREEN step): final run — 4 unit tests
  (`record::tests`) + 33 integration tests across 6 test files, all `test result: ok`.
- `cargo test --workspace`: 85 `test result: ok` blocks total across the whole workspace, zero
  `FAILED`/`error` lines (verified by `grep -c "test result: ok"` and a `FAILED|error` grep
  returning no matches).
- `cargo fmt --all -- --check`: initially reported diffs in the new test files (argument wrapping,
  import ordering); `cargo fmt --all` applied them; re-run: clean, no output.
- `cargo clippy --workspace --all-targets -- -D warnings`: initially reported 9 findings in the new
  crate (`doc_markdown`, `missing_errors_doc`, three `collapsible_if`, `nonminimal_bool`, and two
  `format_push_string` occurrences in tests); all fixed (let-chains, `is_none_or`, `write!` via
  `std::fmt::Write`, one documented `#[allow(clippy::struct_excessive_bools)]` on `Observations`
  matching existing repo precedent in `crates/omnifrons-supervisor/src/bin/fake-agent.rs:109`);
  final run: `Finished` with no warnings or errors.
- `git diff --stat -- . ':!Cargo.lock'` (after `git add -N tools docs/evidence/VP-001`):
  `18 files changed, 1484 insertions(+), 1 deletion(-)`.

### Files Changed

| File | Action | What Was Done |
|---|---|---|
| `Cargo.toml` | Modified | Added `"tools/*"` to `[workspace] members`. |
| `tools/evidence-validator/Cargo.toml` | Created | New non-product workspace member, `publish = false`, workspace lints. |
| `tools/evidence-validator/src/lib.rs` | Created | Crate doc comment; `pub mod derive; pub mod record; pub mod validate;`. |
| `tools/evidence-validator/src/record.rs` | Created | `Record`, `Violation`, `parse` (section/table grammar) plus 4 unit tests. |
| `tools/evidence-validator/src/validate.rs` | Created | `validate` running V1–V8 as 8 focused check functions. |
| `tools/evidence-validator/src/derive.rs` | Created | `Outcome`, `ObservedState`, `Observations`, pure `derive`. |
| `tools/evidence-validator/tests/{v1..v7_v8,derive,fixtures_parse_and_validate}.rs` | Created | 33 integration tests, one file per validator-contract group plus `derive` and the fixture-file integration test. |
| `tools/evidence-validator/tests/fixtures/synthetic-{baselines,records}.md` | Created | Obviously-synthetic fixture records (`SYNTHETIC` ids, fixture digests) for the integration test; not real evidence. |
| `docs/evidence/VP-001/README.md` | Created | Record grammar, per-kind mandatory fields, append-only + `corrects` protocol, `record_id` uniqueness, provenance rule. |
| `docs/evidence/VP-001/baselines.md` | Created | Header-only; no baseline filed yet (Slice 4). |
| `docs/evidence/VP-001/records.md` | Created | Header-only; no scenario row filed yet (Slice 4). |

### Review Budget

Authored (additions + deletions), excluding the generated `Cargo.lock`: **1485 lines**
(`1484 insertions + 1 deletion` across 18 files) — **substantially over the 400-line budget** and
over design.md's own ~280-line forecast for this slice.

**Why it does not shrink further.** Per the apply skill's guard ("never delete comments, blank
lines, docs, or tests, and never compress or restyle code, to fit under the review budget"), this
total is not compressible without weakening the deliverable:

- `src/` (production code, including in-file unit tests): 587 lines across `derive.rs` (73),
  `lib.rs` (10), `record.rs` (183, incl. 4 parser unit tests), `validate.rs` (321, 8 independent
  V1–V8 check functions).
- `tests/` (integration tests): 727 lines across 6 files — Strict TDD requires each of the 8
  closed-vocabulary checks (V1–V8) plus `derive` to have its own RED-then-GREEN table test with
  both a rejecting and an accepting case, per tasks.md 2.2–2.13.
- `tests/fixtures/*.md` + `docs/evidence/VP-001/*`: 157 lines — the store bootstrap (README,
  header-only `baselines.md`/`records.md`) and the synthetic fixtures tasks.md 2.14/2.15 require.
- `tools/evidence-validator/Cargo.toml` + root `Cargo.toml`: 14 lines.

This is one cohesive, interdependent unit — tasks.md itself defines "Evidence Store and Validator"
as a single PR-2 work unit (Suggested Work Units, Unit 2), and every V-check test depends on the
same `parse`/`validate` module pair. **Recommendation: `size:exception` for this slice**, or —
if the orchestrator prefers — split PR 2's *delivery* (not its already-completed implementation)
into two stacked sub-PRs at commit time: 2a = `Cargo.toml` + `record.rs` + V1–V3 (parser, 2.1–2.7,
~640 lines) and 2b = V4–V8 + `derive` + docs/fixtures (2.8–2.16, ~845 lines). No code was
compressed, restyled, or had tests/comments removed to chase the 400-line number.

### Delivery Split (superseding the `size:exception` recommendation above)

The maintainer chose to split delivery rather than take a `size:exception`. Slice 2's already-
implemented work (all 16 tasks, unchanged in content and behavior from the sections above) was
restructured into per-check modules under `src/validate/` (one file per design.md validator-
contract row group: `v1_mandatory_fields.rs`, `v2_result_vocabulary.rs`,
`v3_baseline_before_scenario.rs`, `v4_v5_v6_identifiers_and_provenance.rs`,
`v7_v8_artifact_identity_and_channel.rs`, orchestrated by `src/validate/mod.rs`) and delivered as
**six chained code commits plus one documentation commit (this one)** on
`vp-001/slice-1-packaging-retention`, each compiling, green (`cargo test -p evidence-validator`
plus a full `cargo test --workspace`, `cargo fmt --all -- --check`, and `cargo clippy --workspace
--all-targets -- -D warnings` pass observed immediately before that commit), and individually under
the 400-authored-line budget:

| # | Commit | Subject | Authored lines (excl. `Cargo.lock`) | Contents |
|---|---|---|---|---|
| 1 | `d279938` | scaffold crate skeleton and record parser | 325 | `Cargo.toml` member, crate `Cargo.toml`, `src/lib.rs` (record-only), `src/record.rs`, synthetic fixtures, a parse-only fixtures test |
| 2 | `24fc72d` | implement V1 and V2 admission checks | 275 | `src/validate/mod.rs` (V1–V2 wiring), `v1_mandatory_fields.rs`, `v2_result_vocabulary.rs`, their tests |
| 3 | `ec5dc25` | implement V3 baseline-before-scenario check | 137 | `v3_baseline_before_scenario.rs`, its test, `mod.rs` wiring |
| 4 | `723485c` | implement V4-V6 identifier and provenance checks | 302 | `v4_v5_v6_identifiers_and_provenance.rs`, its test, `mod.rs` wiring |
| 5 | `388be68` | implement V7-V8 artifact identity checks | 233 | `v7_v8_artifact_identity_and_channel.rs`, its test, `mod.rs` wiring (all 8 checks now live) |
| 6 | `253c573` | add derive mapping and bootstrap the evidence store | 286 | `src/derive.rs`, `tests/derive.rs`, `pub mod derive;`, the full-validation assertion restored to `fixtures_parse_and_validate.rs`, `docs/evidence/VP-001/{README.md,baselines.md,records.md}` |
| 7 | (this commit) | record the delivery split in apply-progress and tasks | see this commit's own `git diff --stat` | `apply-progress.md` (this section), `tasks.md` checkbox state — documentation only, no source change |

**Why seven commits, not four.** The apply prompt suggested four commits (skeleton; V1–V3; V4–V6;
V7–V8 + `derive` + docs). Measured precisely, V1+V2+V3 together (source + per-check module wiring
+ their existing tests) come to 388 raw lines before any orchestration code, leaving no room under
400 for the `validate()` wiring and `lib.rs` diff — so V3 was split into its own commit (commit 3).
Likewise V7–V8 (228 raw) plus `derive` (164 raw) plus the docs bootstrap (92 raw) sum to 484 raw
lines before overhead, also over budget — so the docs/evidence bootstrap stayed with `derive` and
V7–V8 was split out into its own commit (commit 5). Finally, adding this delivery-split record to
`apply-progress.md` plus the `tasks.md` checkbox state on top of commit 6's own code would itself
have exceeded 400 lines, so that documentation became its own seventh commit. No check's test file
was split, weakened, or had assertions removed; V4/V5/V6 stayed grouped in one commit because their
combined size (300 raw) already fits comfortably.

**Test provenance across the split.** Every test in commits 2–6 is byte-identical to the version
originally authored and passing before the split, with one exception: the fixtures-integration
test (`tests/fixtures_parse_and_validate.rs`) is *sequenced* rather than reduced. Commit 1 exercises
`parse` alone (`synthetic_baseline_and_scenario_fixtures_parse_into_three_records`, since `validate`
does not exist yet at that commit); commit 6 restores the original, stronger test
(`synthetic_baseline_and_scenario_fixtures_parse_and_validate_clean`, asserting `validate` returns
zero violations against the fixture set) once every check is wired. The final state after commit 6
is identical to the pre-split file. No assertion was ever weakened or dropped — only reordered to
respect what each commit's `lib.rs`/`validate` surface actually exposes.

### Workload / PR Boundary

- Mode: chained PR slice (`auto-chain`, chain strategy `stacked-to-main`); PR 2 itself is delivered
  as seven stacked commits (six code, one documentation) per the Delivery Split above, each
  independently under the 400-line budget.
- Current work unit: Unit 2 — "Evidence store schema, validator (V1–V8), `derive`" (PR 2).
- Boundary: starts from Slice 1's pinned/retaining/digesting packaging and ends with a fully
  tested, non-product validator crate plus a bootstrapped evidence store; introduces no harness,
  scenario, or real evidence-row code (those are Slices 3a, 3b, 4).
- Estimated review budget impact: seven commits, largest 325 authored lines — low per commit; 1485
  lines of code across the six code commits, same content as before the split, now reviewable
  incrementally.

### Deviations from Design

None in content — implementation matches design.md D1 (record grammar), D2 (validator location
and members line), D3 (store layout), and the full V1–V8 / `derive` contract. Two deviations, both
about delivery mechanics rather than behavior:

1. **Line count**: design.md's Migration/Rollout table forecasts ~280 lines for this slice; the
   actual authored total is 1485, driven by Strict TDD's per-check RED/GREEN table-test requirement
   across 8 independent validator checks plus `derive`.
2. **Commit count**: delivered as seven chained commits (six code, one documentation) rather than
   the ~280-line forecast's implicit one, and rather than the four this delivery-split task
   suggested — see "Why seven commits, not four" above.

### Status

16/16 Phase 2 tasks complete; `cargo test --workspace`, `cargo fmt --all -- --check`, and
`cargo clippy --workspace --all-targets -- -D warnings` all pass at every one of the six code
commits. Delivered as seven chained commits, each under the 400-line budget — see Delivery Split
above. Ready for `sdd-verify` on this slice, or for `sdd-apply` to continue with Phase 3a in a
later batch.

## Slice 3a (Phase 3a, PR 3a): Harness Scaffolding and Feasibility Gate

### Scope

Phase 3a only: "Harness Scaffolding and Feasibility Gate" (tasks 3a.1–3a.8). No `docs/evidence/VP-001/{baselines,records}.md`
row is written — the feasibility-gate outcome below is a **local feasibility
observation**, not VP-001 evidence, per this batch's own instructions. No
`tools/vp-s6-agent/` (Slice 3b) exists yet; the throwaway placeholder
executable used for F3 was compiled locally, used only for this run, and is
not part of the repository.

**Outcome: the disclosed-uncertain branch (3a.7) was taken.** F1 and F2
passed; F3 failed on a genuine, pre-existing product defect (not a
harness/tooling limitation) — see "Feasibility Gate Execution" below. Per
this batch's own instructions ("If a gate cannot be passed, that is a
legitimate result... STOP the slice there"), Slices 3b and 4 do not start in
this apply batch. The green scaffolding this slice produced (webdriver
client, its unit tests, the CI step, the AV1/AV2/F1 shell scaffold, and the
workflow wiring) is committed regardless, per the same instructions.

### Completed Tasks

- [x] 3a.1 RED: `docs/evidence/VP-001/procedures/webdriver-session.test.mjs` — 22 `node --test`
      assertions across 6 groups (`buildNewSessionRequest`, `buildElementLocator`,
      `buildEndpointUrl`, `unwrapValue`, `unwrapElementId`, `toWebDriverError`), written against
      not-yet-existing exports.
- [x] 3a.2 GREEN: `docs/evidence/VP-001/procedures/webdriver-session.mjs` — pure builders
      implemented, exported separately from the `fetch`-based transport (`createSession`,
      `deleteSession`, `findElement`, `clickElement`, `sendKeysToElement`, `getElementText`).
- [x] 3a.3 `.github/workflows/ci.yml` — one `node --test` step added to `build-test`, run
      unconditionally on all three runners. **Deviation** (see below): the exact command tasks.md
      names does not work on the pinned Node; the step uses a shell glob instead.
- [x] 3a.4 `docs/evidence/VP-001/procedures/vp-s6-linux.sh` — `--mode=feasibility-check`: AV1
      (re-`sha256sum` vs. `build_channel_digest`, abort `artifact-digest-mismatch`), AV2
      (`--appimage-extract` + byte-scan the payload for the literal `--demo-harness`, abort
      `feature-enabled-variant`), then create and close one `tauri-driver` session against the
      gated file (F1). No scenario logic (dialogs, Start/Stop, `/proc` enumeration) — Slice 3b's job.
- [x] 3a.5 `.github/workflows/tauri-build.yml` — Linux-guarded steps: `extra-apt-packages` gains
      `xvfb webkit2gtk-driver xdotool`; a new "Install tauri-driver" step pins `tauri-driver@2.0.6`
      (`--locked`, matching the parent's own verified local install); a new "VP-001 feasibility gate
      (AV1/AV2, F1)" step locates the built `.AppImage`, reads its recorded digest, and runs
      `xvfb-run -- bash -c '... vp-s6-linux.sh --mode=feasibility-check ...'` with a bounded
      `/status` readiness poll before invoking it.
- [x] 3a.6 **Feasibility gate execution (not a repository test)** — see below. **F1 PASS, F2 PASS,
      F3 FAIL.**
- [x] 3a.7 **Disclosed-uncertain branch taken** — see below. Slices 3b and 4 do not start.
- [x] 3a.8 Rollback boundary confirmed: reverting the two commits below (workflow harness step, both
      `webdriver-session.*` files, `vp-s6-linux.sh`, the `ci.yml` step) returns the repository to
      Slice 2's state; nothing downstream exists yet to depend on any of it.

### Deviations from Design

1. **`node --test <directory>` does not work as tasks.md 3a.3 literally specifies.** On the pinned
   Node 22.23.2, `node --test docs/evidence/VP-001/procedures` (a bare directory, no glob) tries to
   `import()`/`require()` the directory path itself as a single module and fails with
   `ERR_MODULE_NOT_FOUND`/`MODULE_NOT_FOUND`, rather than walking the directory for test files as
   Node's own docs describe. Verified with two independent minimal repros outside this repository
   (a fresh scratch directory, and a directory literally named `test/`) before concluding this is a
   genuine behavior of this Node version/invocation shape, not a repository-specific issue. `node
   --test` with **no** path argument, run from inside a directory, does perform the documented
   recursive walk correctly. The `ci.yml` step instead runs
   `node --test docs/evidence/VP-001/procedures/*.test.mjs`, letting the shell (not Node) expand the
   glob to the concrete file — verified locally to run and report all 22 tests correctly.
2. **AV1/AV2/F1's node invocation reaches the client through `process.env`, not a spliced path or
   argv.** `vp-s6-linux.sh` calls `node -e '...'` with the procedures directory, base URL, and
   application path passed as environment variables and dynamically `import()`ed/read from
   `process.env` inside the script, rather than interpolating the shell variables into the JS source
   text. This mirrors the `Digest artifact`/new `VP001_DIGEST_PATHS` pattern already established in
   `tauri-build.yml` (and Slice 1's own post-verification correction) for exactly the same reason:
   no path ever gets spliced into a script body that something else could break out of.
3. Everything else in 3a.1–3a.5 matches design.md D5–D8 and the Honesty Machinery section with no
   content deviation.

### TDD Cycle Evidence

| Task | Layer | RED (observed) | GREEN (observed) |
|---|---|---|---|
| 3a.1/3a.2 | Unit (`node --test`) | `ERR_MODULE_NOT_FOUND: Cannot find module '.../webdriver-session.mjs'` (module did not exist yet) | `node --test docs/evidence/VP-001/procedures/webdriver-session.test.mjs` → `# tests 22 / # pass 22 / # fail 0` |

### Work Unit Evidence

| Evidence | Value |
|---|---|
| Focused test command and exact result | `node --test docs/evidence/VP-001/procedures/*.test.mjs` → `# tests 22`, `# pass 22`, `# fail 0` (run from the repository root, the exact `ci.yml` step command) |
| Runtime harness command/scenario and exact result | See "Feasibility Gate Execution" below — F1/F2 PASS, F3 FAIL, with the exact commands and output |
| Rollback boundary | Revert commits `85b4b97` and `58321a0` (4 files: `webdriver-session.mjs`, `webdriver-session.test.mjs`, `vp-s6-linux.sh`, plus the `ci.yml`/`tauri-build.yml` edits); nothing downstream exists yet |

### Commits

| # | Commit | Subject | Authored lines (excl. lockfiles) | Contents |
|---|---|---|---|---|
| 1 | `85b4b97` | feat(evidence): add zero-dependency W3C WebDriver client | 317 | `webdriver-session.mjs` (182), `webdriver-session.test.mjs` (135) |
| 2 | `58321a0` | feat(evidence): wire the VP-S6 feasibility gate (AV1/AV2, F1) | 190 | `ci.yml` (+10), `tauri-build.yml` (+48), `vp-s6-linux.sh` (133, new, mode `0755`) |

Both commits observed green immediately before committing:
`node --test docs/evidence/VP-001/procedures/*.test.mjs` → 22/22 pass (both commits — commit 2
touches no JS); `cargo test --workspace` → all suites `test result: ok` (no Rust files touched by
either commit, re-run as a regression check); `.github/workflows/{ci,tauri-build}.yml` parsed
successfully with `/usr/bin/python3.12 -c "import yaml; yaml.safe_load(...)"`;
`shellcheck docs/evidence/VP-001/procedures/vp-s6-linux.sh` reported no findings.

### Files Changed

| File | Action | What Was Done |
|---|---|---|
| `docs/evidence/VP-001/procedures/webdriver-session.mjs` | Created | Zero-dependency W3C WebDriver client: pure builders + `fetch`-based transport (design.md D6). |
| `docs/evidence/VP-001/procedures/webdriver-session.test.mjs` | Created | 22 `node --test` assertions over the pure builders only. |
| `.github/workflows/ci.yml` | Modified | New "Test VP-001 WebDriver client (node --test)" step in `build-test`, unconditional on all three runners. |
| `docs/evidence/VP-001/procedures/vp-s6-linux.sh` | Created (mode `0755`) | `--mode=feasibility-check`: AV1, AV2, F1 (session create + close). |
| `.github/workflows/tauri-build.yml` | Modified | `extra-apt-packages` gains `xvfb webkit2gtk-driver xdotool`; new "Install tauri-driver" and "VP-001 feasibility gate (AV1/AV2, F1)" steps. |

### Review Budget

Authored (additions + deletions): **507 lines** across both commits (317 + 190) — under the
400-line-per-commit budget on each individual commit; over it as a single combined diff, which is
why this landed as two commits rather than one, consistent with Slice 1/2's own practice.

### Feasibility Gate Execution (3a.6) — local observation, not VP-001 evidence

**Local build.** `pnpm -r build` (renderer), then `pnpm exec tauri build --bundles deb,rpm,appimage`
(release profile, `1m 08s` incremental compile on a warm `target/`). Produced
`target/release/bundle/appimage/Omnifrons_0.1.0_amd64.AppImage`
(SHA-256 `47dec7632184f46f3cea8c01d377a7412e75e3f888c91b247b989580000f54b7`).

**AV1/AV2/F1 — via the committed `vp-s6-linux.sh` script itself:**

```
$ xvfb-run --auto-servernum -- bash -c '
    tauri-driver --native-driver /usr/bin/WebKitWebDriver &
    ...
    docs/evidence/VP-001/procedures/vp-s6-linux.sh --mode=feasibility-check "$1" "$2"
  ' vp-s6-feasibility-check "<AppImage>" "<digest>"
gate=av1-digest-match digest=47dec7632184f46f3cea8c01d377a7412e75e3f888c91b247b989580000f54b7
gate=av2-variant-scan-absent binary=/tmp/tmp.XXXXXXXXXX/squashfs-root/usr/bin/omnifrons-shell
gate=f1-session-created session_id=e491650f-ae0d-485d-a283-d32e85574c56
gate=session-closed session_id=e491650f-ae0d-485d-a283-d32e85574c56
```
Exit code `0`. **F1: PASS.** This also answers design.md's open question "whether `ubuntu-24.04` can
execute the AppImage directly (FUSE) or needs `APPIMAGE_EXTRACT_AND_RUN=1`": FUSE worked directly
here — the AppImage launched with no `APPIMAGE_EXTRACT_AND_RUN` fallback needed. `libEGL`/DRI3
warnings appeared in `tauri-driver`'s own stderr (software rendering under Xvfb has no DRI3 device);
harmless, expected, and did not affect session creation.

**F2 — GTK chooser driving (ad hoc, not a repository test; reuses the committed
`webdriver-session.mjs` exports directly via a throwaway Node script, never duplicating its logic):**
a second, persistent `Xvfb :57` + `tauri-driver` pair was started so the same session could be
driven interactively. `xdotool windowfocus` was used in place of design's literal `windowactivate`
sequence — this bare `Xvfb` has no window manager installed (none was in the parent's authorized
tooling list, and none was installed), so `_NET_ACTIVE_WINDOW`-based activation is unavailable;
`windowfocus` sets input focus directly via `XSetInputFocus` and worked identically for the
Ctrl+L/type/Enter sequence design.md prescribes. Both choosers completed:
- "Pick executable" → `Ctrl+L`, typed a throwaway local placeholder path, `Enter` → the
  "Candidate evidence" panel appeared (`SHA-256 (short): 528d1053`), confirmed via
  `findElement`+`getElementText` on `//div[@aria-label='Candidate evidence']`.
- "Pick workspace" → `Ctrl+L`, typed a scratch directory path, `Enter` → `Workspace: <path>`
  rendered, confirmed the same way.

**F2: PASS** — both GTK choosers completed under `Xvfb` via `xdotool` within a bounded time (well
under a minute each); the D7 seeded-approval fallback was never needed.

**F3 — FAIL, blocked by a discovered product defect, not a tooling limitation.** After approving a
throwaway placeholder executable (a locally compiled ELF that ignores argv, drains stdin to EOF,
then sleeps — never committed to the repository, matching design D8's "no shell script" rationale
for anything the process-launch path executes) and correctly selecting the `Stream-JSON CLI` adapter
and that new approval in the **Agent** section's own controls (confirmed via
`document.querySelector('#agent-adapter').value === 'stream-json-cli'` and
`#agent-approval.value` holding the new approval's id), clicking **Start** produced:

```
untrusted unapproved: this executable has not been approved
```

`State:` stayed `idle`; no child process was spawned. Root cause, traced with `codegraph_explore`:
- `crates/omnifrons-adapters/src/jsonl_approval_store.rs`'s `derive_approval_id` mints every
  `ApprovalId` as `u64::from_be_bytes(sha256(...)[..8])` — a uniformly-distributed random 64-bit
  integer, never a small counter.
- `renderer/src/ipc/harness.ts` declares `export type ApprovalId = number` — a plain JS/TS number
  (IEEE-754 f64), safely exact only up to `Number.MAX_SAFE_INTEGER` (2^53 ≈ 9.007×10^15).
- A uniformly random 64-bit value has only a ~1-in-2048 chance of falling under 2^53, so crossing
  Tauri's JSON-based IPC bridge loses precision for the approval id on essentially every real
  approval — reproduced here with the observed id `16973651968280922000` (~1.7×10^19, three orders
  of magnitude past the safe-integer ceiling).
- The rounded value the `<select>` sends back as `approval_id` no longer matches the exact `u64`
  `JsonlApprovalStore` holds on file, so `LaunchGate::decide`
  (`crates/omnifrons-app/src/launch_gate.rs:96-99`) correctly returns
  `DenialReason::Unapproved` — the backend is behaving exactly as designed against a numerically
  corrupted input; the defect is the corruption itself, upstream of `LaunchGate`.
- This is pre-existing product code, entirely untouched by Slices 1–3a. Design.md's Technical
  Approach states this change's three new layers touch "none... product code"; fixing this defect is
  out of scope for this change and is not attempted here. No workaround (seeding approval state
  differently, retrying, or otherwise bypassing the approval UI) was applied — D7 already forbids
  bypassing the approval UI, and this batch's own instructions forbid working around a failed gate.

**Additional observation (did not itself block anything — worked around by an ordinary in-app
action, not a bypass):** `AgentPanel`'s own `approvals_list()` fetch runs once on mount
(`useEffect(..., [])`), with no refresh triggered by `ApprovalSurface`'s `executable_approve`
success — a `<select id="agent-approval">` opened before an approval exists will not show one newly
created afterward without a page reload. Worked around here with one `location.reload()` via the
WebDriver `execute/sync` endpoint (an ordinary browser/webview action, not a store bypass); a real
user hitting the same ordering would need the same reload. Independent of, and does not explain, the
`ApprovalId` defect above (Start still failed identically after the reload, with the approval
correctly visible and selected).

### Disclosed-Uncertain Branch (3a.7)

**Taken.** F3 failed with no applicable fallback: design.md's only two named fallbacks are F2's
seeded-approval path (D7) and AV2's `APPIMAGE_EXTRACT_AND_RUN=1` (neither applies to an IPC-layer
numeric-precision defect). Per this batch's instructions, the chain stops here: **Slices 3b and 4 do
not start in this apply batch.** The blocker above is recorded here for the eventual VP-S6 row
(Slice 4) once the evidence store exists to hold it; nothing is written to
`docs/evidence/VP-001/{baselines,records}.md` — those stay exactly as Slice 2 left them
(header-only).

### Process Hygiene

Every process started for this feasibility check was terminated before finishing:

```
$ curl -s -X DELETE http://127.0.0.1:4444/session/<session-id>          # WebDriver session closed
$ kill <tauri-driver-pid>; kill <Xvfb-:57-pid>
$ pgrep -af "Xvfb|tauri-driver|WebKitWebDriver|omnifrons-shell|vp-s6-placeholder" | grep -v "eval|zsh -c"
confirmed: no leftover processes
```

The throwaway placeholder executable and its scratch workspace directory live only in a
session-local scratch directory outside the repository.

### Workload / PR Boundary

- Mode: chained PR slice (`auto-chain`, chain strategy `stacked-to-main`); delivered as two stacked
  commits (`85b4b97`, `58321a0`), each independently under the 400-line budget.
- Current work unit: Unit 3a — "WebDriver client, CI wiring, AV1/AV2 gate, F1–F3 feasibility" (PR 3a).
- Boundary: starts from Slice 2's evidence-store/validator baseline and ends with a working,
  committed feasibility-gate scaffold (client, tests, CI step, shell script, workflow wiring) plus a
  disclosed, non-repository-test feasibility observation. Introduces no VP-S6 scenario code, no
  fixture agent, and no evidence rows (those are Slices 3b/4, currently blocked).
- Estimated review budget impact: two commits, 317 and 190 authored lines — low per commit.

### Status

8/8 Phase 3a tasks complete (3a.1–3a.8). **F1 PASS, F2 PASS, F3 FAIL** — disclosed-uncertain branch
taken per 3a.7. Slices 3b and 4 are blocked on the `ApprovalId` IPC-precision defect described above
until it is fixed (out of scope for this change). Ready for `sdd-verify` on this slice's own scope
(the scaffolding, its tests, and the honest recording of the feasibility outcome); not ready to
continue to Phase 3b in this change.

### Addendum (2026-09-17): the ApprovalId defect above is now fixed, unblocking Slice 3b

This note is appended below the original Slice 3a record above without altering it — the original
run, its `F3 FAIL` outcome, and its root-cause trace remain exactly as recorded.

Commit `5c5f474` ("fix(ipc): carry executable approval ids as hex strings"), landed on `main` after
this batch's own record above, fixes the `ApprovalId` IPC-precision defect at its source: approval
ids now cross IPC as 16-lowercase-hex-character strings (`src-tauri/src/ipc/dto.rs`'s
`ApprovalIdDto`, strict on (de)serialization; `renderer/src/ipc/harness.ts`'s `ApprovalId` is now
`type ApprovalId = string`), never as a JSON `number` that could round past
`Number.MAX_SAFE_INTEGER`. This branch is rebased onto `main` at `5c5f474` (confirmed:
`git merge-base --is-ancestor 5c5f474 HEAD`).

The orchestrator confirmed, in a local run on the combined code (this branch's Slice 1–3a code atop
`5c5f474`), that the exact same feasibility-gate sequence 3a.6 exercised now reaches **F1 PASS, F2
PASS, F3 PASS**: the approved fixture placeholder reached `State: running` after Start, then
`State: exited (code unreported)` after Stop through the UI. This is the parent's own local
confirmation, not a re-run this apply batch performed itself; this batch's own local preview of the
full VP-S6 scenario (fixture, dialog driving, Stop, `/proc` enumeration, `derive`) is recorded
separately under Slice 3b below.

**Consequence for the slice chain**: Phase 3b's own "Depends on" precondition (3a's F1–F3 gate
having passed) is now satisfied. Slice 3b starts in this apply batch. Slice 4 remains gated
separately on its own explicit CI-trigger authorization (design.md's Migration/Rollout
"Authorization" note), unrelated to this fix.
