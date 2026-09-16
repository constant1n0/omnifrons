# Apply Progress: vp-001-linux-baseline — Slice 1 (Phase 1, PR 1)

## Scope

Phase 1 only: "Packaging Pin and Retention" (tasks 1.1–1.7). Slices 2, 3a, 3b, 4, and 5 are
untouched — no `Cargo.toml` workspace-member change, no `tools/`, no `docs/evidence/`.

## Completed Tasks

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

## Verified During Implementation (Open Questions `[VERIFY at slice 1]`)

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

## TDD Cycle Evidence

| Task | Layer | RED | GREEN | REFACTOR |
|---|---|---|---|---|
| 1.1/1.2 | Unit (`src-tauri/tests/config.rs`) | `bundle_targets_is_an_explicit_non_default_array` failed: `bundle.targets must be an explicit array... found: String("all")` | same test passed after `tauri.conf.json` edit; full `config` suite 5/5 | moved `KNOWN_TAURI_BUNDLE_TYPES` to module scope (clippy `items_after_statements`); backtick-quoted `tauri-utils::config` (clippy `doc_markdown`) |

## Work Unit Evidence

| Evidence | Value |
|---|---|
| Focused test command and exact result | `cargo test -p omnifrons-shell --test config` → `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| Runtime harness command/scenario and exact result | N/A — per tasks.md, this slice's CI YAML has no repository test harness; the workflow's own run is checked at Slice 4's authorized boundary, not here |
| Rollback boundary | Revert 4 files (`src-tauri/tauri.conf.json`, `src-tauri/tests/config.rs`, `package.json` + `pnpm-lock.yaml`, `.github/workflows/tauri-build.yml`); packaging returns to build-only, nothing downstream exists |

## Commands Run (exact observed results)

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

## Files Changed

| File | Action | What Was Done |
|---|---|---|
| `src-tauri/tests/config.rs` | Modified | Added `bundle_targets_is_an_explicit_non_default_array` (RED then GREEN) plus module-level `KNOWN_TAURI_BUNDLE_TYPES`. |
| `src-tauri/tauri.conf.json` | Modified | `bundle.targets`: `"all"` → explicit `["deb","rpm","appimage","msi","nsis","app","dmg"]`. |
| `package.json` | Modified | Added exact-pinned `devDependencies["@tauri-apps/cli"] = "2.11.4"`. |
| `pnpm-lock.yaml` | Modified (generated) | Regenerated by `pnpm install`; excluded from the authored-line budget. |
| `.github/workflows/tauri-build.yml` | Modified | `ubuntu-latest` → `ubuntu-24.04`; `id: tauri` + pinned `tauri-action@action-v1.0.0`; new Linux-guarded "Record baseline facts", "Digest artifact", and "Upload artifact" (pinned `actions/upload-artifact@v7.0.1`) steps. |

## Review Budget

Authored (additions + deletions), excluding the generated lockfile: **109 lines**
(`104 insertions + 5 deletions` across `tauri-build.yml`, `package.json`, `tauri.conf.json`,
`config.rs`) — well under the 400-line budget and under the design's own ~180-line forecast for
this slice.

## Workload / PR Boundary

- Mode: chained PR slice (`auto-chain`, chain strategy `stacked-to-main`).
- Current work unit: Unit 1 — "Pin packaging, retain the artifact, digest it" (PR 1).
- Boundary: starts from the current build-only workflow/config and ends with a pinned, retaining,
  digesting Linux packaging step; introduces no evidence-store, harness, or scenario code (those
  are Slices 2, 3a, 3b, 4).
- Estimated review budget impact: 109 authored lines — low.

## Deviations from Design

None — implementation matches design.md's D4 (CI wiring), the Evidence field provenance table, and
the File Changes table for the four files this slice touches.

## Post-Verification Corrections

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

## Status

7/7 Phase 1 tasks complete. Ready for `sdd-verify` on this slice, or for `sdd-apply` to continue
with Phase 2 in a later batch.
