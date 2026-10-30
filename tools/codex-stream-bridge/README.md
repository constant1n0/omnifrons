# codex-stream-bridge

Presents Codex CLI (`codex exec --json`) as a `stream-json` line agent, so the
generic `stream-json-cli` adapter can drive it. It is the VP-S29 binding of
`stream-json-cli` to Codex CLI 0.163.0. It is a compiled binary, not a script,
for the same reason as `tools/vp-s6-agent`.

## Contract

Adapter side (`crates/omnifrons-adapters/src/line_agent.rs`): the adapter runs
the approved executable with an empty argv, writes the prompt to stdin and
closes it, runs it in the active workspace root with an environment allowlist
(`PATH`, `LANG`, `LC_ALL`, `TMPDIR`, `HOME`), and parses stdout lines.

Bridge behaviour:

1. Reads the whole prompt from stdin. An empty prompt exits 2 with a stderr line.
2. Runs `codex exec --json --skip-git-repo-check -` (`-` makes Codex read the
   prompt from stdin; the flag allows a definition root that is not a Git
   repository). Working directory is inherited; Codex stderr is inherited.
3. Translates stdout line by line, flushing each line:

| Codex event | Emitted line | Exit |
| --- | --- | --- |
| `thread.started` (`thread_id`) | `{"type":"system","subtype":"init","cwd":..,"model":..,"session_id":<thread_id>}` then one retained version line | |
| `item.completed`, `agent_message` (`text`) | `assistant` message with one `text` block | |
| `item.completed`, `command_execution` / `mcp_tool_call` / `file_change` / `web_search` | `assistant` message with one `tool_use` block, `name` = item type, `input` = the item object | |
| `item.completed`, `reasoning` | skipped; a `text` block when `CODEX_STREAM_BRIDGE_REASONING=1` | |
| `turn.completed` | `{"type":"result","subtype":"success"}` | 0 |
| `turn.failed` | stderr diagnostic, then `{"type":"result","subtype":"error"}` | 1 |
| `error` | one stderr diagnostic; the run continues (see below) | |
| stdout ends with no terminal event | `{"type":"result","subtype":"unknown"}` | 1 |
| `turn.started`, `item.started`, `item.updated` | nothing | |
| anything else, malformed JSON | one stderr diagnostic line, no stdout | |

Every emitted line is a single physical line of compact JSON (the consumer
does not depend on key order). `model` is
`codex` unless an event carries a `model` field. If an item arrives before any
`thread.started`, `init` is emitted first with a generated session id
(`codex-bridge-<pid>-<nanos>`) and a stderr note says so.

`error` is not terminal: Codex also uses it for non-fatal, retried stream
errors, so the bridge only logs it. This follows the documented semantics; a
recorded real stream will confirm it. The turn ends at `turn.completed`,
`turn.failed` or the end of Codex's output. If Codex cannot be started, the
bridge reports the error on stderr, emits `init` with a generated session id
and `result/unknown`, and exits 1.

Process hygiene: `codex --version` is bounded to 5 seconds (the version line
is omitted on timeout); after the output ends, Codex gets 10 seconds to exit
before it is killed. Every run writes one stderr line with Codex's exit status
(code or signal).

Version record: right after `init` the bridge emits
`{"type":"system","subtype":"bridge","codex_version":..,"bridge_version":..}`.
The adapter's parser treats a `system` line whose subtype is not `init` as an
unknown line and keeps its bytes verbatim, so it is retained as evidence and
changes no phase. Codex's own `--version` text is used; if it cannot be
obtained the line is omitted.

## Environment variables

- `CODEX_STREAM_BRIDGE_CODEX`: path of the Codex executable (default `codex`
  through `PATH`). Used by the tests to substitute a fake.
- `CODEX_STREAM_BRIDGE_REASONING`: `1` emits reasoning items as text blocks.

The adapter's environment allowlist does not forward other variables, so these
are for tests and manual runs.

## Build and approve

```sh
cargo build --release -p codex-stream-bridge
```

The binary is `target/release/codex-stream-bridge`. Approve it in the app
through the pick, probe and approve flow for a `stream-json-cli` harness.
Approvals are device-local and are never committed. `codex` itself must be on
`PATH` for the app's environment and already signed in.

## Confirmed and inferred Codex fields

Confirmed from `codex exec --help` (0.163.0): `--json` prints events to stdout
as JSONL; `--skip-git-repo-check` exists; the prompt is read from stdin when
no argument is given or when `-` is used.

Confirmed from Codex's public non-interactive documentation sample:
`thread.started.thread_id`; the event names `turn.started`, `turn.completed`
(with `usage`), `turn.failed`, `item.started`, `item.completed`, `error`;
`item.id`, `item.type`; item types `agent_message` and `command_execution`;
`item.text` on an `agent_message`.

Inferred (not confirmed locally, no recording yet): the item type names
`reasoning`, `mcp_tool_call`, `file_change`, `web_search`; `text` on a
`reasoning` item; the message location for `turn.failed`
(`error.message`) and `error` (`message`); a `model` field on any event.
Unknown or differently shaped events are reported on stderr and never abort
the run. The real recording happens in the next sub-slice and should replace
these inferences.
