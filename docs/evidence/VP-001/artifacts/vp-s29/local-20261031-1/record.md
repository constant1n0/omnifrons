## VP-001 VP-S29 scenario -- local operator run local-20261031-1 (2026-10-31)

A local operator run with user-installed, credentialed harnesses. The switch leg is `uncertain` until a
checkpoint-and-restart flow exists, so this run does not discharge the ADR-0005 gate.

| field | value |
| --- | --- |
| `record_id` | VP-001-VP-S29-01 |
| `kind` | scenario |
| `scenario_id` | VP-S29 |
| `baseline_id` | VP-001-BASE-01 |
| `result` | uncertain |
| `observed_state` | uncertain |
| `build_channel` | packaged-ci |
| `build_channel_digest` | 1202b9fff3bf0bdeb4ab5c2530afc60cb6555c84f28da0eb5f7e8672f884a1fe |
| `exercised_artifact_digest` | 1202b9fff3bf0bdeb4ab5c2530afc60cb6555c84f28da0eb5f7e8672f884a1fe |
| `variant_scan` | absent |
| `procedure_ref` | docs/evidence/VP-001/procedures/vp-s29-linux.sh |
| `claude_code_leg` | uncertain -- Claude Code 2.1.316; loaded-file evidence false; indexed skill path true; user-level skill path false; no system/init line together with a line naming CLAUDE.md or AGENTS.md |
| `codex_leg` | uncertain -- Codex CLI through codex-stream-bridge 0.163.0; loaded-file evidence false; indexed skill path false; user-level skill path false; no system/init line together with a line naming AGENTS.md; the only access to the indexed skill path failed (a command execution with a failed status or a non-zero exit code) |
| `gemini_leg` | uncertain -- the leg was not run or has no observation |
| `switch_leg` | uncertain -- no checkpoint-and-restart flow exists |
| `digests_unchanged` | unknown -- digests were not compared |
| `digests_by_invocation` | after-claude-code-1 true; after-claude-code-2 true; after-stream-json-cli-1 true; after-stream-json-cli-2 true; after-pty-cli-1 missing; after-pty-cli-2 missing |
| `harness_versions` | Claude Code 2.1.316; Codex CLI through codex-stream-bridge 0.163.0 (bridge reports Codex 0.163.0, bridge 0.1.0); Gemini CLI not run |
| `run_date` | 2026-10-31 |
| `operator_role` | maintainer (the repository owner operated the packaged app; the digests, observe and record steps ran in an assistant session under the owner's supervision) |
| `evidence_artifact` | 7bc3b861521d570e9c728063cfb0e9d4d7db680b21b25a3561b183cfc7263aa2 -- SHA-256 of the retained redacted file docs/evidence/VP-001/artifacts/vp-s29/local-20261031-1/evidence-summary.txt |

### How the run went

Fixture: the `release-notes` skill under `skills/` plus a same-named user-level
skill in the real HOME (`~/.claude/skills`, `~/.agents/skills`, `~/.gemini/skills`);
the definition "Orbit Scribe" was created through the app's Create form (root
`AGENTS.md`, `CLAUDE.md` and `GEMINI.md` both `@AGENTS.md`). The form needed its
nested consent ("Create this definition on its own") because a home-level
`AGENTS.md` makes HOME an ancestor definition on this machine. Every digest
taken (`before`, `after-claude-code-1/2`, `after-stream-json-cli-1/2`) matched
the one before it; `after-pty-cli-*` are missing because that leg never ran, which
is why `digests_unchanged` stays `unknown`.

`claude_code_leg`: two runs (identity prompt; release-notes task), both
`result: success`. The agent answered as Orbit Scribe and, in the second run,
read `skills/release-notes/SKILL.md` through a `tool_use` Read (the indexed
skill path, harness-side). Claude Code's stream-json output never names the
`CLAUDE.md` it loaded, so the loaded-file evidence is absent and the leg is
`uncertain`. The first `observe` pass over the unredacted logs reported `pass`:
the only lines "naming CLAUDE.md" were entries of the operator's own command
catalogue (`system/commands_changed`) whose descriptions mention the file. That
is a false positive of the observer's rule, to be tightened; the committed
observation was produced from the redacted copies (below) and is the honest
one.

`codex_leg`: two runs through `codex-stream-bridge` (Codex CLI 0.163.0), both
`result: success`; the agent identified itself as Orbit Scribe in both. In the
task run its only access to the indexed skill path was a `command_execution`
(`cat skills/release-notes/SKILL.md`) that Codex's sandbox failed to run
(`bwrap: execvp ...: No such file or directory`, `status: failed`, exit code
1), and the answer says the local template could not be read and uses a
fallback layout. A failed access is not a read, so the indexed skill path is
`false` for this leg; the observer's rule was tightened for exactly this case
after review. `codex exec --json` also emits no line of its own naming
`AGENTS.md`, so the leg records `uncertain` on both counts.

`gemini_leg` (`pty-cli`): not run, three causes, each verified. (1) Google
retired the standalone Gemini CLI for individual accounts after 2026-06-18 and
names Antigravity CLI (`agy`) as its successor; the installed `gemini` 0.41.2
still answers `--version`. (2) `gemini` resolves to a Node launcher script
(`bundle/gemini.js`); a `#!/usr/bin/env node` script executed from a sealed
memfd through `/proc/self/fd/<n>` fails at once with `ENOENT` on
`/memfd:... (deleted)` (node resolves its entry file's real path), so that
binding was never runnable under the app's launch model. (3) Start with
`pty-cli` and that approval left the spawn pending: the transcript echoed the
prompt and nothing followed (no child process, no pseudo-terminal, no harness
log, no error banner, nothing on stderr, every app thread idle). The same
symptom reproduced in another app instance with an ELF binary (`agy`), while a
third instance spawned `agy` normally: an intermittent PTY-launch defect in the
packaged build, filed separately.

Exploratory, outside the bound legs: `pty-cli` with the approved `agy` 1.2.5
rendered its TUI as plain text (dropped-control counters shown), signed in, then
stopped at its folder-trust prompt, which the panel cannot answer; the adapter
had typed the prompt before the TUI was ready. No context-file notice was
printed before that prompt. Its log is retained locally only.

Run end: every run ended through the operator's Stop after the child had
already exited. The app confirms a self-exited child only at Stop or at the
next spawn, so until then the run stays active and its harness log has no
state line; the `exited 0` states in the logs are the children's real exit
codes. Other product findings from this run: the Agent panel's approval list
is loaded once and not refreshed after an approval; Start is a silent no-op
while a select is still on its placeholder; the stale first approval (a deleted
scratch path) yields "probe-failed" on Start.

Redaction: `transcript-claude-code.txt`, `observation-claude-code.json` and
`log-summary-claude-code.json` were produced by `observe` from copies of the
two retained logs in which the operator's Claude Code catalogue (the `system/init`
arrays `tools`, `mcp_servers`, `slash_commands`, `skills`, `agents`, `plugins`
and the two `system/commands_changed` lists) was replaced by a count marker,
keeping every entry that names `release-notes` (1926 and 1386 entries
replaced). Each whole stdout line became one frame; header, stderr and state
lines are unchanged. The raw logs and `private/` stay local.
