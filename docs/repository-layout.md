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

This skeleton is the spike vehicle the roadmap's Pre-alpha scope names, not ADR-0002's acceptance. The Pre-alpha → Alpha promotion gate lists a "desktop spike result" among its required evidence (roadmap.md § Promotion and approval gates); the code in this repository — the renderer scaffold, the Rust workspace under `crates/` (part B), and now `src-tauri` (part C) — produces that evidence. It does not by itself discharge ADR-0002's acceptance gate, which additionally requires VP-001 passing on every pinned baseline, a threat-model and renderer-content review, and named Rust-capable maintainers (adr/0002-desktop-technology-stack.md § Acceptance evidence).

## Crate map

The Rust workspace lands in this section: part B added it as a Cargo workspace of library crates under `crates/`, plus the crate boundaries the roadmap and ADR-0002 name. Part C adds `src-tauri` itself — see below.

| Crate | Depends on | Role |
| --- | --- | --- |
| `omnifrons-domain` | `std`, `thiserror` only | Framework-independent domain core (adr/0002-desktop-technology-stack.md § Proposed decision ("Framework-independent Rust domain")) |
| `omnifrons-app` | `omnifrons-domain`, `thiserror` | Application services over the domain core; defines the `ProcessSupervisor` port |
| `omnifrons-adapters` | `omnifrons-domain`, `omnifrons-app` | Process, Git, Engram, Markdown, and secret-store adapters (adr/0002-desktop-technology-stack.md § Integration decisions → Executables and harnesses, → Git, → Engram, → Obsidian and Markdown); today a placeholder naming that future scope, with a compile-time-adjacent test (`tests/deps.rs`) that fails the moment a Tokio or Tauri dependency is added |
| `omnifrons-supervisor` | `omnifrons-domain`, `omnifrons-app`, `tokio` (`rt`, `process`, `time`), `tracing`, `nix` (Unix only, `signal` feature) | Rust process supervisor; implements `ProcessSupervisor` over `tokio::process`, spawning each child in its own process group and terminating the group (SIGTERM, then SIGKILL after the deadline) on Unix (adr/0002-desktop-technology-stack.md § Proposed decision ("Rust process supervisor with Tokio")) |
| `src-tauri` (crate `omnifrons-shell`) | `tauri` 2, `tauri-build` 2 (build-dependency), `serde`, `serde_json` | Tauri 2 desktop shell; the outer adapter (adr/0002-desktop-technology-stack.md § Privilege and IPC boundary); nothing depends on the shell |

`omnifrons-domain` never depends on Tauri, Tokio, or any adapter — ADR-0002's requirement that "domain, handoff, event, scope, and recovery contracts must run and test without Tauri or a WebView" (adr/0002-desktop-technology-stack.md § Proposed decision) is a compile-time property of this dependency table, not a convention to remember. `omnifrons-app` shares that constraint: its `ProcessSupervisor` port (`crates/omnifrons-app/src/process_supervisor.rs`) is sync-with-deadline rather than `async fn`, precisely so this crate is never forced to depend on an async runtime; only `omnifrons-supervisor`, the adapter that actually needs one, depends on Tokio. `cargo tree -p omnifrons-domain` and `cargo tree -p omnifrons-app -e normal` are the standing checks for this property (see Build and test commands).

`omnifrons-app` also exposes a reusable `ProcessSupervisor` contract behind its `contract-tests` cargo feature (`crates/omnifrons-app/src/contract/process_supervisor.rs`): `process_supervisor_contract` (spawn, observe running, stop within a deadline, assert a terminal state) and `unproven_descendants_yield_orphan_risk_uncertain` (require `OrphanRiskUncertain` when descendants cannot be proven stopped). `omnifrons-app`'s own tests run both against an in-memory fake; `omnifrons-supervisor`'s tests run the baseline contract again against the real, OS-backed supervisor.

`src-tauri` is present as of part C: the Linux WebKitGTK 4.1 development headers (`webkit2gtk-4.1`, `javascriptcoregtk-4.1`, GTK 3, libsoup 3) are installed in this environment, verified via `pkg-config --modversion webkit2gtk-4.1`, so the crate is hand-written against a verified toolchain (the Tauri CLI itself is not installed here, so nothing is scaffolded by `cargo tauri init`). The root `Cargo.toml`'s `members` list now carries an explicit `"src-tauri"` entry alongside the `crates/*` glob. `omnifrons-shell` currently depends on `tauri`, `serde`, and `serde_json` only — not `omnifrons-domain`, `omnifrons-app`, `omnifrons-supervisor`, or `omnifrons-adapters` (the last of which still exports nothing, see above). The internal crates join `omnifrons-shell`'s dependency list when the first typed command needs them; today's sole command, `shell_health`, is pure shell-local logic with nothing yet to call into the domain or supervisor.

Pinned exact versions (`workspace.dependencies`, resolved via `cargo info` at the time this crate was added): `tauri = "2.11.5"`, `tauri-build = "2.6.3"`, `serde = "1.0.229"`, `serde_json = "1.0.151"`. `omnifrons-shell` keeps Tauri's own default feature set (`wry`, `compression`, `common-controls-v6`, `dynamic-acl`, `x11`, `dbus`) — those are the runtime's own baseline capabilities, not optional plugins — and adds no Tauri plugin crate (no `tauri-plugin-fs`, `-shell`, `-http`, or `-dialog`).

`omnifrons-shell` registers exactly one typed command, `shell_health` (`src-tauri/src/lib.rs`), returning `{ "shell": "omnifrons-shell", "version": <crate version>, "containment": "unproven" }`. The pure logic lives in `health()` (`src-tauri/src/health.rs`), unit-tested without a Tauri runtime; `containment` stays `"unproven"` until VP-001 proves platform-specific process containment (adr/0002-desktop-technology-stack.md § Process supervision: Windows Job Object policy, Linux cgroup/process-group/watchdog behavior, and macOS process-group/watchdog behavior are all still unverified).

`src-tauri/capabilities/default.json` grants the main window exactly `core:default` and nothing else — no `fs`, `shell`, `http`, or `dialog` permission, matching invariant 4's "no generic shell and no unrestricted filesystem capability" (docs/target-architecture.md § Components and trust boundaries).

`src-tauri/tauri.conf.json`'s `app.security.csp` transcribes renderer-content-security.md § CSP baseline directive-for-directive, with one documented exception: the baseline sets `connect-src` to `'none'`, but Tauri's own IPC bridge rides `ipc:` (the custom-protocol transport used on Linux and macOS) and `http://ipc.localhost` (the transport Windows WebView2 requires in place of a custom scheme), so `connect-src` here is `"ipc: http://ipc.localhost"` instead. This is the platform-bridge exception renderer-content-security.md § CSP baseline itself anticipates ("Where a platform's bridge implementation requires a custom protocol handler to carry that traffic, the protocol is added to `connect-src` explicitly, documented in this section, and verified under VP-001 — never left as an implicit, undocumented exception to the baseline above"); it is a VP-001 input, not a silent loosening, and every other directive (`default-src`, `script-src`, `style-src`, `img-src`, `media-src`, `font-src`, `manifest-src`, `frame-src`, `object-src`, `base-uri`, `form-action`, `worker-src`) is transcribed unchanged. Tauri appends its own nonces and hashes to bundled scripts and styles at compile time, so `script-src 'self'` and `style-src 'self'` need no `'unsafe-inline'` or manually-managed nonce. `src-tauri/tests/config.rs` asserts this transcription and the capability's scope so a future edit cannot silently widen either.

`tauri.conf.json`'s `identifier` is `"dev.omnifrons.app"` — a placeholder. The final identifier is an owner decision tied to product naming (see this page's Name status and ADR-0002's), not a Rust-toolchain concern; it is free to change before ADR-0002's acceptance gate closes.

`src-tauri/icons/` holds placeholder app icons only (a solid, flat color, no real artwork): `32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.png`, `icon.ico`, and `icon.icns`, generated by `src-tauri/icons/generate-placeholder-icons.py` (stdlib-only: hand-rolled PNG encoding via `zlib`, PNG-in-ICO framing, and PNG-backed ICNS `ic07`/`ic11`/`ic13` chunks — no Pillow or ImageMagick required). They exist only so `tauri.conf.json`'s `bundle.icon` list points at files that exist and decode cleanly for `cargo check`/`cargo build`/`cargo test`; real app art is a separate, later concern.

Build and test `omnifrons-shell` on its own with:

```sh
cargo check -p omnifrons-shell
cargo test -p omnifrons-shell
```

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

- Package manager: pnpm, pinned exactly via `packageManager` and `devEngines.packageManager` in the root `package.json`, enforced by `engineStrict` in `pnpm-workspace.yaml`. CI installs that exact version with `npm install -g` after reading it from `package.json`, the same way on every runner, and caches the pnpm store keyed on the lockfile.
- Node runtime: pinned in `.node-version` (also mirrored in `devEngines.runtime`), which `actions/setup-node` reads in CI. `pnpm/setup` and `pnpm/action-setup` are deliberately not used: the first hung while downloading pnpm on a Windows runner, the second is documented for pnpm 10 and earlier.
- Renderer toolchain: React 19, TypeScript, Vite, Vitest with `jsdom`, `@testing-library/react`, and ESLint, all pinned in `renderer/package.json`.
- Rust toolchain: pinned via `rust-toolchain.toml` (`channel = "1.98.1"`, `components = ["rustfmt", "clippy"]`, `profile = "minimal"`); the workspace's `[workspace.package]` restates the same version as `rust-version` and pins `edition = "2024"`; `rustfmt.toml` and `clippy.toml` pin `edition = "2024"` and `msrv = "1.98.1"` respectively for their own tools.

## CI overview

Four workflows live in `.github/workflows/`, each with `permissions: read-all` (or narrower) at the top level and a concurrency group keyed on `${{ github.workflow }}-${{ github.ref }}` with `cancel-in-progress: true`, and each job carries a `timeout-minutes` bound. `ci.yml` and `tauri-build.yml` share their Rust and Node/pnpm setup through the `.github/actions/setup-toolchain` composite action, so the two workflows cannot drift on how each toolchain is installed:

- **ci.yml** — `build-test` (30-minute timeout), on push to `main` and on pull request, across an `ubuntu-latest` / `macos-latest` / `windows-latest` matrix. Runs `pnpm -r lint`, `pnpm -r test`, and `pnpm -r build`; the Rust steps (`cargo fmt`, `cargo clippy`, `cargo test`) were guarded on `rust-toolchain.toml` existing and now run, since part B added that file. As of part C it also passes `linux-prereqs: 'true'` to `setup-toolchain`, installing `libwebkit2gtk-4.1-dev` and the other Tauri Linux build prerequisites on the `ubuntu-latest` leg before `cargo test --workspace` links `omnifrons-shell` against WebKitGTK; the input is a no-op on `macos-latest` and `windows-latest` (their WebViews, WKWebView and WebView2, ship with the OS).
- **gitleaks.yml** — `gitleaks` (10-minute timeout), scanning the full history for committed secrets; `permissions: read-all` deliberately omits `pull-requests: write`, so findings surface only in the job log, not as inline PR annotations. (Renamed from `secrets.yml`: that filename matched the maintainer's global `*secret*` gitignore rule and could never be tracked by Git.)
- **docs.yml** — `docs-links` (10-minute timeout), checking every relative link and anchor (`--include-fragments`) under `docs/**/*.md`, `README.md`, `CONTRIBUTING.md`, and `SECURITY.md` in offline mode.
- **tauri-build.yml** — `build-test` (60-minute timeout), on `workflow_dispatch` and a daily schedule, same three-OS matrix; the Tauri packaging step is guarded on both `rust-toolchain.toml` and `src-tauri/tauri.conf.json` existing. Both now exist as of part C, so this step is live; it also already passed `linux-prereqs: 'true'` and `extra-apt-packages: patchelf` to `setup-toolchain` (patchelf is needed for AppImage bundling, not for `ci.yml`'s plain `cargo test`). This step's packaged build has not been run locally — VP-001, not this page, owns that evidence. No signing identity is configured for `tauri-action` today, so the packaged build is expected to produce unsigned artifacts per OS until UTA-001's signing identities exist; this is expected, not a defect, but should be confirmed by the first `workflow_dispatch` run rather than assumed.

Required check names a future branch-protection rule names against these workflows:

- `build-test (ubuntu-latest)`
- `build-test (macos-latest)`
- `build-test (windows-latest)`
- `gitleaks`
- `docs-links`

## Hygiene

`.editorconfig` fixes UTF-8, LF line endings, a final newline, trimmed trailing whitespace, and two-space indentation repository-wide, with a four-space exception for `*.rs` and no trailing-whitespace trimming for `*.md` (Markdown's own trailing-space line-break convention). `.gitignore` excludes build output (`node_modules/`, `renderer/dist/`, `.vite/`, `*.tsbuildinfo`, `coverage/`, `.pnpm-store/`), Rust/Tauri build artifacts (`/target`, `**/target`, `src-tauri/target/`, `src-tauri/gen/`, `src-tauri/WixTools/`) — the last three were staged ahead of part C and now apply to the real crate; `src-tauri/icons/` is deliberately not ignored, since its (placeholder) icons are `tauri.conf.json`'s `bundle.icon` inputs, not build output — and local secrets and keys (`.env`, `.env.*` — with a standing exception for a future `.env.example` — `*.pem`, `*.key`, `*.p12`, `*.pfx`). `Cargo.lock` is committed, not ignored: this workspace produces binaries/libraries other crates and CI depend on building reproducibly, not a library meant to float on its dependents' resolution. `CODEOWNERS`, `CONTRIBUTING.md`, and `SECURITY.md` at the repository root record ownership, contribution, and disclosure process ahead of any external contribution.
