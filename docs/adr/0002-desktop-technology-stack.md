# ADR-0002: Desktop Technology Stack

**Document role:** Desktop-stack decision rationale  
**Status:** Proposed  
**Accountable role:** Project Maintainer  
**Named person:** Unassigned  
**Approver role:** Project Owner  
**Approver named person:** Unassigned  
**Proposed on:** 2026-09-01  
**Accepted on:** None  
**Last status change:** 2026-09-01 — created as Proposed  
**Acceptance gate:** Reproducible verification plan on pinned Windows/macOS/Linux baselines, security review, and confirmed Rust ownership  
**Supersedes:** None  
**Name status:** Selected public name; preliminary screening complete, formal trademark clearance pending

## Context

Omnifrons needs a human-facing desktop interface while supervising user-installed CLI harnesses, streaming structured events, enforcing only the scope it can prove, and recovering from partial failure. The privileged process boundary is more important than renderer convenience.

The [target architecture](../target-architecture.md) owns invariants and protocols. This ADR chooses a candidate implementation stack; it does not make those protocols true merely by selecting a framework.

## Proposed decision

Subject to the acceptance gate:

- Tauri 2 desktop shell.
- Framework-independent Rust domain and application core.
- Rust process supervisor with Tokio, Serde/`serde_json`, and `tracing`.
- React, TypeScript, and Vite renderer.
- Radix Primitives for accessible interaction.
- CSS design tokens; Tailwind only after the verification plan pins a compatible WebView baseline.
- xterm.js only for declared PTY fallback and diagnostics.

Tauri remains an outer adapter. Domain, handoff, event, scope, and recovery contracts must run and test without Tauri or a WebView.

## Privilege and IPC boundary

```text
untrusted renderer
  -> generated narrow client
  -> typed commands and bounded event channels
  -> application services
  -> domain policies and ports
  -> process / Git / Engram / Markdown / secret-store adapters
```

The renderer receives task-specific operations, never a generic shell, arbitrary executable, unrestricted path, or broad Tauri shell capability. The core validates payloads, canonical paths, arguments, and authorization again.

Harness scope is labelled `sandbox-enforced`, `harness-enforced`, or `advisory`. A `cwd` and narrow IPC do not sandbox a user-installed harness. PTY mode is advisory unless an independently verified OS sandbox applies.

## Process supervision

The supervisor owns process start, stdout/stderr draining, framing, bounded queues, cancellation, graceful stop, deadlines, and observed terminal state.

Containment is platform-specific and must be proven in the planned desktop verification plan:

- Windows: Job Object policy and breakaway behavior.
- Linux: supported cgroup/process-group/watchdog mechanism and documented fallback.
- macOS: process-group/watchdog behavior and explicit daemonization limits.

A profile that daemonizes or escapes containment is unsupported unless separately handled. When descendants cannot be proven terminated, the result is `orphan-risk/uncertain`, not “cleanly stopped.”

## Integration decisions

### Executables and harnesses

Executable approval is device-local. It binds the canonical path and available identity evidence—digest/signature, version, adapter, transport, plugin inventory, and security-relevant configuration. Re-probe occurs immediately before launch; material change requires renewed approval. Portable configuration cannot authorize execution.

Harness-owned credentials remain with the harness. Built-in adapters prefer documented structured protocols. PTY is capability-reduced and may not claim complete approval mediation, event fidelity, or filesystem containment. Its byte stream, escape sequences, and OSC actions are untrusted active content; the adapter exposes only AEC-001/RCS-001-allowlisted typed terminal actions rather than passing raw actions through.

### Git

Use the installed Git executable so supported user authentication and repository behavior remain available, but do **not** preserve execution-capable configuration blindly.

The planned threat model must classify Git commands and configuration. Initial requirements are:

- use explicit argv and a bounded environment;
- prefer fetch plus inspection over implicit pull/merge;
- separate read-only inspection from mutating publication;
- detect hooks, filters, external diff/text-conversion, fsmonitor, signing programs, helpers, and other execution-capable configuration;
- block or obtain specific consent before a workflow can invoke configured code;
- never force-reset, force-push, or synchronize a live `.git` directory through consumer file sync;
- create handoff commits through isolated temporary state without changing the user's branch/index and without running user hooks.

LFS or executable filters are supported only when the portable-work protocol and verification plan explicitly validate them.

### Engram

Use supported Engram CLI/MCP contracts, never its live SQLite internals. Local Engram remains required; Engram Cloud is optional.

Engram Cloud credentials have explicit custody status. Prefer OS-secret-store custody and per-process injection when supported. If Engram stores a token in its own machine-local configuration, Omnifrons labels custody as externally managed, never copies that configuration, and does not claim to secure or roam it. Missing device-local bindings become `unprovisioned-secret`.

### Obsidian and Markdown

Markdown and relative logical references are primary. Every Markdown file is always local, readable, and outside the evictable heavy-asset tier. Large attachments use a separate storage port and explicit hydration. Obsidian CLI capability is optional, and the Obsidian application is not redistributed.

## State, content, and secrets

- Portable TOML and envelopes are versioned and secret-free.
- The Markdown vault is always local and non-evictable; heavy blobs are separate and may be local or on demand according to user policy.
- Local databases, caches, indexes, and UI runtime state are reconstructible and machine-local.
- Omnifrons-owned secrets use OS credential storage; absence fails closed.
- Known structured secret fields and tracked secret values are redacted before persistence/renderer delivery.
- Arbitrary harness output cannot be guaranteed secret-free; raw output is excluded from support bundles by default and residual leakage risk is disclosed.
- Renderer content defaults to plain text. Rich Markdown/attachments require the planned renderer-content-security contract: strict sanitization, CSP, denied in-app navigation, confirmed allowlisted external links, and safe download handling.
- The terminal renderer uses an allowlist of required escape sequences. OSC clipboard and file-transfer actions are disabled by default; hyperlink/navigation providers and schemes are constrained and confirmed; title/notification changes are sanitized; unknown controls are inert/dropped; scrollback and export are redacted and serialized through the safe support-content path.

## Consequences

### Benefits

- Rust provides a strong, testable boundary for long-lived process and recovery logic.
- Tauri can expose narrower IPC than a renderer with Node access.
- React/TypeScript supports interface iteration and accessible primitives.
- A framework-independent core preserves future desktop or headless options.

### Costs

- Sustained Rust ownership and review are mandatory.
- System WebViews create a real compatibility/accessibility matrix.
- Packaging, credential services, PTY, containment, discovery, and signing differ by OS.
- Narrow IPC and truthful degraded states require more design than a generic shell bridge.

## Alternatives

### Electron fallback candidate

Electron is not selected by this Proposed ADR. A later ADR may supersede this proposal with Electron if one bounded remediation cannot solve any of:

- insufficient Rust ownership;
- release-blocking WebView accessibility, IME, rendering, or streaming differences;
- materially unreliable Tier-1 PTY/process containment;
- packaging/update recovery failure;
- pressure to move privileged provider logic into the renderer.

Any Electron design retains a sandboxed renderer, no Node integration, context isolation, sender/payload validation, a narrow preload bridge, and an explicit Linux update strategy.

### Flutter

Not selected because Dart and platform-channel/plugin work add an ecosystem without reducing the core process, Git, and CLI integration risks. Reconsider only after a mobile-first product change supported by a new ADR.

## Planned verification artifact

Executable spike scenarios belong in planned artifact `VP-001 — Desktop stack verification plan`, registered in the target architecture. It does not yet exist.

The ADR gate requires that plan to:

- pin exact OS build, architecture, WebView/runtime, packaging substrate, assistive technology, and test date;
- test packaged release builds, not only development mode;
- cover structured streams, bounded backpressure, renderer reconnection, malformed events, approvals/cancellation, PTY, process containment, executable identity, typed IPC/path attacks, handoff interruption, native secrets, Git/Engram behavior, accessibility, and signed-update failure;
- define measurable pass/fail and `uncertain` outcomes;
- produce an evidence matrix and accept/remediate/supersede recommendation.

Update signing tests do not approve automatic updates until the separate update-trust architecture is accepted.

## Acceptance evidence

1. VP-001 passes on every pinned baseline or records an approved, bounded exception.
2. The threat model and renderer-content review approve the actual IPC, path, Git, process, content, and secret boundaries.
3. The team names maintainers capable of owning and reviewing Rust.
4. The compatibility matrix records minimum supported versions separately from the spike baselines.
5. The ADR status change follows the approval convention.

## Primary references

- [Tauri process model](https://v2.tauri.app/concept/process-model/)
- [Tauri security](https://v2.tauri.app/security/)
- [Electron security](https://www.electronjs.org/docs/latest/tutorial/security)

## Related artifacts

- [ADR convention](README.md)
- [ADR-0001](0001-open-source-license.md)
- [Target architecture](../target-architecture.md)
- [Compatibility policy](../versioning-and-compatibility.md)
