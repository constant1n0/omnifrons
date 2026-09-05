# Repository Layout

**Document role:** Repository skeleton, crate map, toolchain pins, and CI overview
**Status:** Draft
**Normative force:** Non-binding; implements the ADR-0002 spike, not its acceptance
**Accountable role:** Project Maintainer
**Named person:** Unassigned
**Approver role:** Project Owner
**Approver named person:** Unassigned
**Effective date:** None
**Supersedes:** None
**Name status:** Selected public name; preliminary screening complete, formal trademark clearance pending

## Document authority

This page describes the repository skeleton — directory layout, planned crate boundaries, the renderer package, toolchain pins, and CI — as it stands today. It does not decide component responsibilities: that authority stays with [target-architecture.md](target-architecture.md). It does not decide the stack: that authority stays with [ADR-0002](adr/0002-desktop-technology-stack.md). Where this page and either of those disagree, they govern.

This skeleton is the spike vehicle the roadmap's Pre-alpha scope names, not ADR-0002's acceptance. The Pre-alpha → Alpha promotion gate lists a "desktop spike result" among its required evidence (roadmap.md § Promotion and approval gates); the code in this repository — the renderer scaffold and, as of part B, the Rust workspace under `crates/` — produces that evidence. It does not by itself discharge ADR-0002's acceptance gate, which additionally requires VP-001 passing on every pinned baseline, a threat-model and renderer-content review, and named Rust-capable maintainers (adr/0002-desktop-technology-stack.md § Acceptance evidence).

## Crate map

The Rust workspace lands in this section: part B added it as a Cargo workspace of library crates under `crates/`, plus the crate boundaries the roadmap and ADR-0002 name. `src-tauri` is the one planned member not yet present — see below.

| Crate | Depends on | Role |
| --- | --- | --- |
| `omnifrons-domain` | `std`, `thiserror` only | Framework-independent domain core (adr/0002-desktop-technology-stack.md § Proposed decision ("Framework-independent Rust domain")) |
| `omnifrons-app` | `omnifrons-domain`, `thiserror` | Application services over the domain core; defines the `ProcessSupervisor` port |
| `omnifrons-adapters` | `omnifrons-domain`, `omnifrons-app` | Process, Git, Engram, Markdown, and secret-store adapters (adr/0002-desktop-technology-stack.md § Integration decisions → Executables and harnesses, → Git, → Engram, → Obsidian and Markdown); today a placeholder naming that future scope, with a compile-time-adjacent test (`tests/deps.rs`) that fails the moment a Tokio or Tauri dependency is added |
| `omnifrons-supervisor` | `omnifrons-domain`, `omnifrons-app`, `tokio` (`rt`, `process`, `time`), `tracing`, `nix` (Unix only, `signal` feature) | Rust process supervisor; implements `ProcessSupervisor` over `tokio::process`, spawning each child in its own process group and terminating the group (SIGTERM, then SIGKILL after the deadline) on Unix (adr/0002-desktop-technology-stack.md § Proposed decision ("Rust process supervisor with Tokio")) |
| `src-tauri` (crate `omnifrons-shell`) | all of the above | Tauri 2 desktop shell; the outer adapter (adr/0002-desktop-technology-stack.md § Privilege and IPC boundary); nothing depends on the shell |

`omnifrons-domain` never depends on Tauri, Tokio, or any adapter — ADR-0002's requirement that "domain, handoff, event, scope, and recovery contracts must run and test without Tauri or a WebView" (adr/0002-desktop-technology-stack.md § Proposed decision) is a compile-time property of this dependency table, not a convention to remember. `omnifrons-app` shares that constraint: its `ProcessSupervisor` port (`crates/omnifrons-app/src/process_supervisor.rs`) is sync-with-deadline rather than `async fn`, precisely so this crate is never forced to depend on an async runtime; only `omnifrons-supervisor`, the adapter that actually needs one, depends on Tokio. `cargo tree -p omnifrons-domain` and `cargo tree -p omnifrons-app -e normal` are the standing checks for this property (see Build and test commands).

`omnifrons-app` also exposes a reusable `ProcessSupervisor` contract behind its `contract-tests` cargo feature (`crates/omnifrons-app/src/contract/process_supervisor.rs`): `process_supervisor_contract` (spawn, observe running, stop within a deadline, assert a terminal state) and `unproven_descendants_yield_orphan_risk_uncertain` (require `OrphanRiskUncertain` when descendants cannot be proven stopped). `omnifrons-app`'s own tests run both against an in-memory fake; `omnifrons-supervisor`'s tests run the baseline contract again against the real, OS-backed supervisor.

`src-tauri` is not present in this repository yet: it requires the Linux WebKitGTK development headers (`webkit2gtk-4.1`, verified via `pkg-config --modversion webkit2gtk-4.1`) that this environment does not have installed, so the crate is skipped rather than hand-written against an unverified toolchain. The root `Cargo.toml`'s `members` list only glob-matches `crates/*` for the same reason; it gains an explicit `"src-tauri"` entry once that crate exists.

## Renderer package

`renderer/` holds the React/TypeScript/Vite renderer ADR-0002 pins (adr/0002-desktop-technology-stack.md § Proposed decision). It is scaffolded, tested, and built today, and is joined by the Rust workspace under `crates/` as of part B. The renderer defaults to plain text for untrusted content — `UntrustedText` renders its `content` prop as a React text child, never as markup — matching the default that renderer-content-security.md § Content classes and rendering modes ("no declared class defaults to plain text") assigns to any content source with no declared class, pending that contract's own acceptance.

## Build and test commands

From the repository root, with pnpm installed:

```sh
pnpm install --frozen-lockfile   # install every workspace package
pnpm -r lint                     # lint every workspace package
pnpm -r test                     # run every workspace package's tests
pnpm -r build                    # build every workspace package
```

`rust-toolchain.toml` now exists, so the equivalent Rust commands run today and join CI (wired in ci.yml and tauri-build.yml, previously guarded on that file's existence):

```sh
rustup show
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo tree -p omnifrons-domain               # must show only thiserror
cargo tree -p omnifrons-app -e normal        # must show no tokio
```

`--all-features` is what enables `omnifrons-app`'s `contract-tests` feature, which in turn is what makes `omnifrons-supervisor`'s contract test (`tests/contract.rs`) and `omnifrons-app`'s own fake-backed contract tests (`tests/contract_process_supervisor.rs`) compile and run; without it those two test binaries build with zero tests.

## Toolchain pins and VP-001

The pins recorded here — package manager, Node runtime, and renderer toolchain versions — are candidate values for the "Rust, Node, and toolchain versions" field VP-001 records once per pinned baseline, at first execution (desktop-stack-verification-plan.md § Pinned baselines). This page names what is installed; it is not itself VP-001 evidence, and VP-001 stays the sole authority for the exact-OS-build, test-date, and evidence-artifact fields a baseline row requires (desktop-stack-verification-plan.md § Evidence record).

Current pins:

- Package manager: pnpm, pinned exactly via `packageManager` and `devEngines.packageManager` in the root `package.json`, enforced by `engineStrict` in `pnpm-workspace.yaml`. CI's `pnpm/setup` action (see CI overview) reads this same pin, so there is one source of truth for the pnpm version.
- Node runtime: pinned for CI via `devEngines.runtime` in the root `package.json`, which `pnpm/setup` installs directly (pnpm 11 dropped support for `pnpm/action-setup`, documented for pnpm <=10 only, so CI no longer pairs a separate Node setup action with it); `.node-version` remains the pin for local tooling that reads it (e.g. `nvm`, `fnm`), and `engines.node` in the root `package.json` states the minimum supported range.
- Renderer toolchain: React 19, TypeScript, Vite, Vitest with `jsdom`, `@testing-library/react`, and ESLint, all pinned in `renderer/package.json`.
- Rust toolchain: pinned via `rust-toolchain.toml` (`channel = "1.98.1"`, `components = ["rustfmt", "clippy"]`, `profile = "minimal"`); the workspace's `[workspace.package]` restates the same version as `rust-version` and pins `edition = "2024"`; `rustfmt.toml` and `clippy.toml` pin `edition = "2024"` and `msrv = "1.98.1"` respectively for their own tools.

## CI overview

Four workflows live in `.github/workflows/`, each with `permissions: read-all` (or narrower) at the top level and a concurrency group keyed on `${{ github.workflow }}-${{ github.ref }}` with `cancel-in-progress: true`, and each job carries a `timeout-minutes` bound. `ci.yml` and `tauri-build.yml` share their Rust and Node/pnpm setup through the `.github/actions/setup-toolchain` composite action, so the two workflows cannot drift on how each toolchain is installed:

- **ci.yml** — `build-test` (30-minute timeout), on push to `main` and on pull request, across an `ubuntu-latest` / `macos-latest` / `windows-latest` matrix. Runs `pnpm -r lint`, `pnpm -r test`, and `pnpm -r build`; the Rust steps (`cargo fmt`, `cargo clippy`, `cargo test`) were guarded on `rust-toolchain.toml` existing and now run, since part B added that file.
- **gitleaks.yml** — `gitleaks` (10-minute timeout), scanning the full history for committed secrets; `permissions: read-all` deliberately omits `pull-requests: write`, so findings surface only in the job log, not as inline PR annotations. (Renamed from `secrets.yml`: that filename matched the maintainer's global `*secret*` gitignore rule and could never be tracked by Git.)
- **docs.yml** — `docs-links` (10-minute timeout), checking every relative link and anchor (`--include-fragments`) under `docs/**/*.md`, `README.md`, `CONTRIBUTING.md`, and `SECURITY.md` in offline mode.
- **tauri-build.yml** — `build-test` (60-minute timeout), on `workflow_dispatch` and a daily schedule, same three-OS matrix; the Tauri packaging step is guarded on both `rust-toolchain.toml` and `src-tauri/tauri.conf.json` existing. The latter file still does not exist — part B's environment lacked the Linux `webkit2gtk-4.1` development headers `src-tauri` needs, so the crate was skipped (see Crate map) and this step stays inert until a later change adds it.

Required check names a future branch-protection rule names against these workflows:

- `build-test (ubuntu-latest)`
- `build-test (macos-latest)`
- `build-test (windows-latest)`
- `gitleaks`
- `docs-links`

## Hygiene

`.editorconfig` fixes UTF-8, LF line endings, a final newline, trimmed trailing whitespace, and two-space indentation repository-wide, with a four-space exception for `*.rs` and no trailing-whitespace trimming for `*.md` (Markdown's own trailing-space line-break convention). `.gitignore` excludes build output (`node_modules/`, `renderer/dist/`, `.vite/`, `*.tsbuildinfo`, `coverage/`, `.pnpm-store/`), Rust/Tauri build artifacts (`/target`, `**/target`, `src-tauri/target/`, `src-tauri/gen/`, `src-tauri/WixTools/`) — the last three stay ready for whenever `src-tauri` is added — and local secrets and keys (`.env`, `.env.*` — with a standing exception for a future `.env.example` — `*.pem`, `*.key`, `*.p12`, `*.pfx`). `Cargo.lock` is committed, not ignored: this workspace produces binaries/libraries other crates and CI depend on building reproducibly, not a library meant to float on its dependents' resolution. `CODEOWNERS`, `CONTRIBUTING.md`, and `SECURITY.md` at the repository root record ownership, contribution, and disclosure process ahead of any external contribution.
