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
