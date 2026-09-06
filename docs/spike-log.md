# Spike Log

**Document role:** Log of ADR-0002 implementation spikes: what each slice proves, what it defers, its IPC contract, and its relationship to VP-001 acceptance evidence  
**Status:** Draft  
**Normative force:** Non-binding; a spike log, not an acceptance artifact — every result recorded here is `dev-mode-only` and therefore not admissible VP-001 evidence (VP-001-R2)  
**Accountable role:** Project Maintainer  
**Named person:** Unassigned  
**Approver role:** Project Owner  
**Approver named person:** Unassigned  
**Effective date:** None  
**Supersedes:** None  
**Name status:** Selected public name; preliminary screening complete, formal trademark clearance pending

## Document authority

This page logs the ADR-0002 implementation spikes run against the Rust workspace and Tauri shell [repository-layout.md](repository-layout.md) describes. It does not decide the stack ([ADR-0002](adr/0002-desktop-technology-stack.md) does), invariants or trust boundaries ([target architecture](target-architecture.md) does), or acceptance evidence (the [desktop stack verification plan](desktop-stack-verification-plan.md), VP-001, does). A spike slice produces candidate procedure text and working code toward those contracts; it does not itself discharge them. Where this page and any of those disagree, they govern.

## Slice 1 — supervisor to renderer streaming

### What it proves

That a demo child process's stdout and stderr can be captured by `omnifrons-supervisor`, framed, and streamed to a Tauri renderer over a typed `tauri::ipc::Channel`, end to end, on Linux, in a development build:

- A process spawned from a validated request (never a raw program path or argument vector) emits captured output frames in order, with a bounded delivery queue that drops under backpressure rather than blocking the producer, and reports the drop count on the next delivered frame.
- A line longer than the per-frame byte cap splits into consecutive frames with no bytes lost; invalid UTF-8 is lossily decoded rather than dropped or erroring.
- A process that ignores `SIGTERM` is still stopped, definitively, by escalating to `SIGKILL` within a bounded deadline.
- Every IPC error surfaces as a closed, typed error code with a fixed catalogue message, never the underlying OS error text (which can carry a real filesystem path).

Both `HarnessKind`s emit a leading `"ready"` stdout line before any `"line <n> out"` output, printed only once a `DemoIgnoresSigterm` process's `SIGTERM`-ignore handler is already installed (`DemoLines` prints it too, for symmetry, even though it installs no handler). A caller can subscribe to the process's output and wait for this line as a genuine synchronization point before calling `stop` against it, closing a handler-install race a fixed sleep could only ever paper over with a guess at "long enough" (`crates/omnifrons-supervisor/src/demo.rs`'s `run`; `crates/omnifrons-supervisor/tests/demo_harness.rs`).

### What it defers

- **Any acceptance claim.** This is a development-mode build on one contributor's Linux machine, not a packaged release build on a pinned baseline. See [Relationship to VP-001](#relationship-to-vp-001).
- **Windows and macOS.** The output-capture reader tasks and the demo harness are platform-generic (Tokio, `std::io`), but the final `State` frame is only ever sent once a reap is confirmed, and `omnifrons-supervisor`'s Windows stop path never confirms a reap yet (`repository-layout.md` § Crate map; pending VP-001 VP-S5, the Job Object policy). Windows output capture therefore never emits a terminal marker today — an inherited gap, not a new one.
- **Any real harness.** `HarnessKind` names exactly two synthetic demo behaviors (`DemoLines`, `DemoIgnoresSigterm`); no user-supplied program, script, or arbitrary executable is reachable through this slice's IPC surface at all.
- **Malformed-payload fuzzing and reconnection/cancellation behavior.** This slice proves the happy path and the two edge cases above; it does not run the adversarial input corpus or renderer-restart scenarios [VP-S13](#procedure-notes-for-vp-s13) and [VP-S15](#procedure-notes-for-vp-s15) call for.
- **Drop-on-supervisor-drop.** If the supervisor itself is dropped without an explicit `stop` (a panicking caller, an early return), its existing best-effort `SIGKILL` cleanup still runs, but no final `State` frame is sent for any process still subscribed at that point — the output channel is simply abandoned. A future slice should decide whether that needs fixing or is an acceptable spike-only limitation.
- **Launcher identity re-probing.** `TokioProcessSupervisor::with_demo_launcher` resolves the launcher path once, via `tauri::process::current_binary`, and caches it for the supervisor's whole lifetime; every `spawn_harness` call execs that cached path with no re-probe of its identity (digest, signature, or modification time) immediately before each launch. [Threat model TM-001, HAR-3](threat-model.md) requires exactly that re-probe for a harness whose executable identity could go stale between launches — "re-probe immediately before launch; material change requires renewed approval and re-binds the identity evidence." This slice's cached-path shortcut is acceptable only because the "harness" it launches is this application's own binary (`--demo-harness`, gated by the non-default `demo-harness` cargo feature), re-exec'd in place — not a third-party executable a supply-chain compromise (TM-001 actor A10) could swap out between launches. A future slice adding a real, user-supplied harness must not carry this shortcut forward; it needs HAR-3's re-probe.

### The IPC contract

Three typed commands, registered next to `shell_health`:

| Command | Params | Returns |
| --- | --- | --- |
| `harness_spawn` | `kind: HarnessKindDto`, `rateHz: number`, `lines: number`, `onFrame: Channel<HarnessFrame>` | `ProcessIdDto` |
| `harness_stop` | `id: ProcessIdDto`, `deadlineMs: number` | `ProcessTerminalStateDto` |
| `harness_observe` | `id: ProcessIdDto` | `ProcessStatusDto` |

No program path or argument vector ever crosses this boundary in either direction: `kind` names one of two closed, synthetic behaviors, and `rateHz`/`lines` are small bounded numbers, validated (`1..=1000`, `1..=100_000`) into an `omnifrons_app::HarnessRequest` before the supervisor ever sees them. The supervisor turns a validated request into a real `ProcessSpec` internally, execing a launcher path fixed once at construction (`tauri::process::current_binary`) — never a path carried by any command argument.

Wire shapes (`src-tauri/src/ipc/dto.rs`, unit-tested field-for-field):

```jsonc
// HarnessKindDto (input only)
"demo-lines" | "demo-ignores-sigterm"

// ProcessIdDto
42

// ProcessTerminalStateDto — code is always present, null unless exited
{"state": "exited", "code": 0}
{"state": "killed", "code": null}
{"state": "orphan-risk/uncertain", "code": null}

// ProcessStatusDto (harness_observe's return)
{"status": "running"}
{"status": "terminal", "state": "killed", "code": null}

// HarnessFrame (harness_spawn's onFrame payload) — adjacently tagged
{"stream": "stdout", "body": {"id": 42, "seq": 3, "droppedBefore": 0, "text": "line 1 out"}}
{"stream": "stderr", "body": {"id": 42, "seq": 41, "droppedBefore": 12, "text": "line 205 err"}}
{"stream": "state",  "body": {"id": 42, "seq": 13, "droppedBefore": 0, "state": "exited", "code": 0}}

// ShellError (every command's error variant)
{"code": "spawn-failed", "message": "failed to start the requested process"}
```

`ShellErrorCode` is closed: `unknown-process`, `spawn-failed`, `already-subscribed`, `invalid-request`, `too-many-processes`. `message` is always one of a small, fixed catalogue of strings — never the underlying `SupervisorError::Spawn`'s `io::Error` text, which can carry a real path. `invalid-request` now also covers `harness_stop`'s `deadlineMs`, validated into `1..=30_000` before it ever reaches the supervisor (an unbounded caller-supplied deadline would otherwise let a single `harness_stop` call occupy its blocking-thread slot indefinitely). `too-many-processes` is returned when a supervisor already tracks its maximum number of concurrently running processes — see the capacity limits below.

Capacity and framing limits (`omnifrons-supervisor/src/output_capture.rs` and `lib.rs`): a per-process delivery channel holds at most **1024** frames (`try_send` only — full means the newest frame is dropped and counted, never that the reader blocks), **except** the final `State` frame, which is guaranteed-delivered via a dedicated blocking-send thread rather than dropped alongside text when the channel is full; a single text frame carries at most **8 KiB** of decoded text (`MAX_LINE_BYTES`), longer lines splitting into consecutive frames at a UTF-8 character boundary, never mid-character; `\r\n` is treated as a line end with the `\r` stripped, and a final line with no trailing newline is still delivered once EOF is reached. A supervisor also caps its own process bookkeeping: at most **16** concurrently running processes (`spawn`/`spawn_harness` refuse a 17th with `SupervisorError::TooManyProcesses`, mapped to `ShellErrorCode::TooManyProcesses`, without ever spawning it) and at most **256** retained terminal entries, oldest evicted first by termination order — an evicted id then reports `unknown-process`/`None` from a later `stop`/`observe`, never a re-signal of a pid the OS may have since recycled. An output-capture entry is removed once both finalized and subscribed-out (its receiver already taken via `subscribe`); an entry nobody ever subscribes to is retained indefinitely instead.

### Procedure notes for VP-S13

[VP-001 VP-S13](desktop-stack-verification-plan.md#scenario-catalog) ("IPC boundary: typed commands only; path attacks rejected") will eventually need this slice's commands exercised with a malformed-payload corpus. Candidate cases, once a packaged build exists to run them against:

- Raw filesystem paths (absolute, relative, UNC, `file://`) submitted as `kind`, where only the two closed string tokens are accepted.
- Path-traversal strings (`../../etc/passwd`, symlink targets) submitted the same way.
- Wrong JSON types for every field (`rateHz` as a string, a float, or an array; `id` as a string or negative number).
- Oversized numbers: `rateHz`/`lines` beyond `u16`/`u32` range, `id` beyond `u32` range, and boundary values just inside/outside the validated ranges (`0`, `1`, `1000`, `1001`, `100_000`, `100_001` for `rateHz`/`lines`; `0`, `1`, `30_000`, `30_001` for `harness_stop`'s `deadlineMs`).
- Extra, unexpected JSON keys alongside the accepted ones for every command (e.g. `{"kind": "demo-lines", "rateHz": 10, "lines": 10, "onFrame": ..., "path": "/etc/passwd"}`). Tauri's own per-parameter command binding (each `#[tauri::command]` argument bound by name from the invoke payload) *ignores* keys it has no matching parameter for, rather than rejecting the payload outright — confirmed against this slice's own commands, which declare no catch-all parameter. The corpus therefore cannot assert rejection for this case; it must instead assert that an extra key has *no effect whatsoever* (the command behaves identically with or without it present), since silent tolerance of an unrecognized key is not itself a path or argument reaching `omnifrons-supervisor`. Whether a future slice should close this permissiveness outright — an `AppManifest`-generated strict schema, or a wrapper request struct deriving `#[serde(deny_unknown_fields)]` for each command's arguments — is an open decision this slice does not make.

Expected outcome per [target architecture, invariant 4](target-architecture.md#proposed-invariants) and [RCS-001-R14](renderer-content-security.md): rejection with a typed `ShellError`, never a best-effort coercion, and never a path or argument reaching `omnifrons-supervisor`'s `spawn` — except the extra-JSON-keys case above, whose expected outcome is no-effect tolerance, not rejection.

### Procedure notes for VP-S15

[VP-001 VP-S15](desktop-stack-verification-plan.md#scenario-catalog) ("Structured streams, bounded backpressure, reconnection, malformed events, cancellation") is this slice's closest analogue among VP-001's scenarios. Candidate procedure, once a packaged build exists:

- Drive `harness_spawn` at the top of its accepted range (`rateHz = 1000`, `lines = 100_000`) against a renderer that does not drain `onFrame`, and confirm: `harness_spawn` and `harness_stop` still return promptly; the delivered frame count stays near the 1024-frame channel capacity, not the full line count; at least one delivered frame's `droppedBefore` is nonzero once the renderer resumes draining (`crates/omnifrons-supervisor/tests/output_backpressure.rs` is this slice's dev-mode analogue).
- Kill and restart the renderer mid-run; confirm `harness_observe` still reports the harness's true status and a fresh `harness_spawn` subscription is possible for a *new* process (this slice's `subscribe` is one-shot per process id, not reconnect-capable for an existing subscription — a gap this scenario should surface explicitly, not paper over).
- Inject malformed `HarnessFrame`-shaped events at the transport layer (not achievable through this slice's own commands, which only ever emit well-formed frames) to confirm the renderer's own parsing rejects them rather than crashing.
- Cancel mid-run via `harness_stop` with a short `deadlineMs` against a `DemoIgnoresSigterm` process, confirming escalation to `SIGKILL` and a definite terminal state within the deadline (`crates/omnifrons-supervisor/tests/demo_harness.rs` is this slice's dev-mode analogue).

### Renderer

The renderer half of this slice: a typed IPC client plus a minimal control panel driving it, all under `renderer/src/`.

- `ipc/harness.ts` — the *only* renderer module that imports `@tauri-apps/api/core`. Exports types mirroring `src-tauri/src/ipc/dto.rs` field-for-field (`HarnessKind`, `ProcessId`, `ProcessTerminalState`, `ProcessStatus`, `HarnessFrame`, `ShellError`), plus `harnessSpawn(request, onFrame)` (constructs a `Channel<HarnessFrame>`, wires its `onmessage` to `onFrame`, and invokes `harness_spawn` with exactly `{ kind, rateHz, lines, onFrame }`), `harnessStop(id, deadlineMs)`, `harnessObserve(id)`, and the `isShellError` type guard.
- `controlCharacters.ts` — `stripControlCharacters`, kept out of `PlainTextLine.tsx` so that component file only exports a component (the project's `react-refresh/only-export-components` lint rule).
- `PlainTextLine.tsx` — renders one line of untrusted text as plain text: strips C0 control characters (U+0000-U+001F except tab), DEL (U+007F), C1 control characters (U+0080-U+009F), and the bidirectional/format controls U+200B-U+200D (zero-width space/non-joiner/joiner), U+200E/U+200F (LRM/RLM), U+202A-U+202E (explicit bidi embeddings/overrides), and U+2066-U+2069 (explicit bidi isolates) — treated as format controls, not text, per RCS-001-R18, the same distinction RCS-001 already draws for OSC 0/2 titles and OSC 9 notifications ("control and bidirectional-override characters stripped"). Stripping U+200C/U+200D is a deliberate trade-off against scripts (Arabic, Indic) that rely on ZWJ/ZWNJ for correct shaping (`controlCharacters.ts`'s doc comment weighs the cost). Keeps every other character, no markup, no links, no OSC interpretation (RCS-001-R1, RCS-001-R18). `UntrustedText.tsx` is untouched.
- `HarnessPanel.tsx` — kind select (`demo-lines`/`demo-ignores-sigterm`), rate/lines number inputs validated client-side with `Number.parseInt` against the same bounds `omnifrons_app::harness_catalog` enforces (`1..=1000` Hz, `1..=100_000` lines; advisory only here, the Rust catalog is the sole authority and validates again regardless), disabling Start and showing an inline message for an empty, non-numeric, negative, or out-of-range value. Start/Stop buttons — Start additionally disabled while a spawn is pending or a harness is already active, since this slice allows only one active harness per panel; Stop disabled once no harness is active, including right after a successful stop resets it. A state badge renders the wire token verbatim (`running`, `exited (code N)` or `exited (code unreported)` when the platform reported no code, `killed`, `orphan-risk/uncertain` — never `done`, per target-architecture.md invariant 8). An output log renders every frame through `PlainTextLine` prefixed by its stream name, capped at the most recent 2000 entries with a `<mark>` marker ("showing the last 2000 lines; N earlier lines are not shown", N cumulative) once older entries are trimmed, plus a `<mark>` row ("N frames dropped before this line") whenever a frame's `droppedBefore` is nonzero. An error banner renders a `ShellError`'s `code` and `message` as plain text through `PlainTextLine` — or the fixed string "unexpected error", with no message text, for any rejection that isn't `ShellError`-shaped. A mount effect reads every process id remembered in `sessionStorage` (one key, `JSON.parse`/`JSON.stringify`, wrapped in try/catch) and calls `harnessObserve` for each: a still-`running` id shows "stream reconnected; earlier output not replayed"; a terminal status, an `unknown-process` rejection (a stale id, forgotten silently, no banner), or a completed `harness_stop` all prune the id from `sessionStorage`; any other rejected `ShellError` shows the banner once and also prunes the id, since it cannot be reconnected to either way. A `mountedRef` set false in that effect's cleanup guards both its promise handlers and the frame callback, so neither does anything once the component has unmounted. Mounted from `App.tsx` below the existing content.

Tests (Vitest + `jsdom`, `@testing-library/react`, `@tauri-apps/api/mocks`' `mockIPC`/`clearMocks`), all green:

- `ipc/harness.test.ts` (8 tests): `harness_spawn` is invoked with exactly the keys `kind`, `rateHz`, `lines`, `onFrame` and no path-shaped key; a frame sent on the live `Channel` reaches the `onFrame` callback; a rejected `harness_spawn` propagates its `ShellError` payload; `harness_stop`/`harness_observe` pass `id`/`deadlineMs` correctly and return the mocked shape verbatim (including `orphan-risk/uncertain`); `isShellError`'s positive and negative cases.
- `PlainTextLine.test.tsx` (11 tests): every C0 control character except tab is stripped, DEL is stripped, C1 control characters are stripped, the bidi marks/embeddings/isolates and the zero-width space/non-joiner/joiner are each stripped, tab is kept, non-ASCII is kept, and rendered markup stays literal (no `<b>` element, text visible).
- `HarnessPanel.test.tsx` (25 tests): initial render (kind options, input bounds, controls present); Rate/Lines validation (empty, negative, non-numeric, above-max all disable Start and show an inline message; correcting a value re-enables it); literal `<b>` frame rendering; the OSC 8 test (below, exact-equality); the dropped-frames `<mark>` text; the log-cap `<mark>` marker across 2500 pushed frames; the badge reading `killed` (with the exact `harness_stop` args asserted) and resetting `activeId`/disabling Stop once it resolves; `exited (code N)` and `exited (code unreported)`; `orphan-risk/uncertain` verbatim from a mocked `state` frame (asserting `done` never appears); the `spawn-failed` error banner with no `/` in its message, control characters stripped from a banner's code/message, and the generic "unexpected error" banner for a non-`ShellError` rejection; a second click not spawning twice while Start is disabled; a mount-time reconnection marker for a `running` remembered id, pruning on a terminal status, silent forgetting on `unknown-process`, the banner-once-and-prune path for another `ShellError`, and the mounted-guard test proving the mount effect no-ops (no `sessionStorage` write) once unmounted.

**What the OSC 8 test proves, and does not prove.** The panel is fed one `stdout` line containing a full OSC 8 hyperlink sequence — `ESC ]8;;https://example.invalid BEL label ESC ]8;; BEL` — and the test asserts the rendered output contains no `<a>` element and that the visible text is exactly the input with its ESC (`0x1B`) and BEL (`0x07`) bytes removed: `]8;;https://example.invalidlabel]8;;`. Every other byte, including the OSC marker's own punctuation (`]8;;`) and the URL text, survives literally, because `PlainTextLine` strips individual C0/DEL code points and never recognizes or interprets a sequence. This proves the stripping is per-byte and blind to OSC structure (so a malformed or partial escape sequence cannot smuggle a link past it) and that RCS-001-R18 ("sanitization must not remove textual content") holds on this path. It does **not** prove anything about a terminal content class or an OSC policy: this slice has no terminal pane, no confined terminal frame, and no OSC allowlist (renderer-content-security.md § Content classes and rendering modes defines that class; nothing here implements it). The harness output this slice renders is plain text — the default every content source with no declared class gets — never PTY output.

**Mocking `Channel` in `mockIPC`.** `@tauri-apps/api/mocks`' `mockIPC(cb)` replaces `window.__TAURI_INTERNALS__.invoke`; the real `invoke()` in `@tauri-apps/api/core` calls that function with its `args` object passed straight through, with no JSON serialization at that layer (serialization to the wire happens deeper, inside the real bridge `mockIPC` replaces). Confirmed against the installed package's own source (`@tauri-apps/api@2.11.1`'s `core.js` and `mocks.js`): the mock handler's `args.onFrame` is therefore the exact live `Channel` instance `harnessSpawn` constructed, not a serialized placeholder. Calling `args.onFrame.onmessage(frame)` from inside the mock handler is the approach that worked; no `Channel` mock or module-level hook was needed. Separately, jsdom has no WebCrypto implementation, and the mock's channel-id registry needs `window.crypto.getRandomValues` to mint callback ids for every `Channel` a test constructs; `testSupport/installJsdomCrypto.ts` installs a minimal, non-cryptographic fill for it (Tauri's own docs demonstrate this with Node's `crypto.randomFillSync`, but that does not type-check under `tsconfig.app.json`, which excludes Node's ambient types, so the fill here uses no Node built-in). Both this polyfill and React Testing Library's `cleanup()` after every test are installed once, centrally, by `testSupport/setup.ts` (`vite.config.ts`'s `test.setupFiles`), rather than each test file repeating its own `beforeAll`/manual `cleanup()` call. `vite.config.ts` also sets `test.allowOnly` explicitly to `!process.env.CI` (Vitest's own default, made explicit so a stray `.only` reads as a deliberate, reviewable choice rather than an accident that happens to also be caught in CI).

**Strict TypeScript.** `tsconfig.app.json` and `tsconfig.node.json` both set `"strict": true`; the renderer code (including every test file, since `tsconfig.app.json` includes `src`) already type-checked clean under it with no changes needed.

### Relationship to VP-001

Every result behind this slice — every passing test listed above — was produced against a `cargo test` development build, on one contributor's Linux machine, not a packaged release build on a pinned Windows/macOS/Linux baseline. [VP-001-R2](desktop-stack-verification-plan.md#product-requirements) states plainly: "A scenario result MUST be produced against a packaged release build; a development-mode-only result MUST NOT be recorded as evidence." This slice's results are exactly that excluded case — `dev-mode-only` — and are recorded here, in a spike log, precisely because they are **not** admissible as VP-S13 or VP-S15 evidence rows. They are inputs to writing those scenarios' procedures well, nothing more.
