# VP-S29 operator procedure (Linux)

Scenario: `docs/desktop-stack-verification-plan.md`, row VP-S29. This is a
local operator run with user-installed, credentialed CLIs, not a CI job. The
helper `vp-s29-linux.sh` prepares the fixture, takes digests, derives each
leg's observation and assembles the record; it never launches a harness or
the app. Every launch below is done by the operator through the app.

Outcome rules: the switch leg is always `uncertain` (no checkpoint-and-restart
flow exists), so the run's overall `result` is `uncertain` and never discharges
the ADR-0005 gate. A load leg is `pass` only with loaded-file evidence from the
harness itself plus the indexed skill path; anything unclear is `uncertain`,
never `pass`. For `pty-cli`, no transcript text counts as loaded-file evidence: a PTY
transcript cannot distinguish the CLI's own output from the model's prose, and
Antigravity CLI documents no loaded-context notice, so the leg records
`uncertain` on that count. The model saying
who it is, or mentioning the file, never counts; without the notice the leg is
`uncertain` even when the indexed skill path appears. For `claude-code` and the
bridge, loaded-file evidence is
non-assistant material only: a `system` line naming `CLAUDE.md`/`AGENTS.md`, or
a `tool_use` block whose input names the file (a read). An assistant text block
that merely mentions the file never counts, and the indexed skill path counts
only when it appears in such material. A user-level skill path in a transcript, or a changed digest, is
`fail`.

## 0. Bindings

| Adapter | Harness | Version | Reads |
| --- | --- | --- | --- |
| `claude-code` | Claude Code (`claude`) | 2.1.316 | `CLAUDE.md`, falling back to `AGENTS.md` |
| `stream-json-cli` | Codex CLI (`codex`) through `tools/codex-stream-bridge` | 0.163.0 | `AGENTS.md` |
| `pty-cli` | Antigravity CLI (`agy`) | 1.2.5 | `AGENTS.md` directly (and the `GEMINI.md` wrapper) |

The `pty-cli` leg was bound to Gemini CLI 0.41.2 until 2026-10-31: Google
retired that CLI for individual accounts after 2026-06-18, and its Node
launcher cannot run from the app's sealed memfd, so the binding moved to
Antigravity CLI, which still reads `AGENTS.md` and `GEMINI.md`.

## 1. Prepare

1. Build the bridge: `cargo build --release -p codex-stream-bridge` (the
   binary is `target/release/codex-stream-bridge`).
   Run the CI-packaged app, not a source build: the evidence validator accepts
   only the `packaged-ci` build channel, and a row that carries a build digest
   needs a retained evidence file. Download the AppImage that the
   `tauri-build.yml` run for the main commit produced (list the run's artifacts
   with `gh run view <run id>`, then `gh run download <run id> --name <artifact>`),
   make it executable and note `sha256sum <AppImage>`: that digest is both the
   `build_channel_digest` and the `exercised_artifact_digest` in step 5. A local
   source build (for example `pnpm exec tauri dev`) can be used to rehearse the
   procedure but its run cannot be filed in `records.md`.
2. Check the three versions and that each CLI is logged in. Save each version
   output; the helper reads these files:
   `claude --version > claude-version.txt`, `codex --version > codex-version.txt`,
   `agy --version > agy-version.txt`. A different version is recorded as
   `uncertain` for that leg.
3. Decide which HOME the whole run uses. The user-level skill must sit where
   the harness looks for it, so the choice applies to every step:
   User-level skill locations, from the vendors' documentation: Codex CLI
   ("Build skills" page of the OpenAI Codex docs) reads `$HOME/.agents/skills`
   and also scans `.agents/skills` in every directory from the working
   directory up to the repository root; Antigravity CLI (docs, "Migrating from Gemini CLI") reads global skills
   from `~/.gemini/skills/` or `~/.gemini/antigravity-cli/skills/` and
   workspace skills from `.agents/skills/` (Gemini CLI, the previous binding,
   read `~/.gemini/skills/` or the alias `~/.agents/skills/`); Claude Code reads its personal skills folder
   `~/.claude/skills/`. The definition root itself must hold neither
   `.agents/skills` nor `.gemini/skills`, or a host would load the project
   skill through its own loader instead of the indexed path: the fixture
   refuses such a root and the digest files list both folders as `absent`
   or `present`. Do not place the root inside a Git repository that has an
   `.agents/skills` folder.
   - **Real HOME.** The CLIs already have their logins. The fixture writes
     `release-notes/SKILL.md` under `.claude/skills`, `.agents/skills` and
     `.gemini/skills` and `.gemini/antigravity-cli/skills` in that HOME and
     refuses to overwrite an existing one. Afterwards remove exactly those
     four `release-notes` directories.
   - **Dedicated HOME** (a new empty directory). Nothing else in your real
     HOME can leak into the run, but each CLI must be logged in again with
     `HOME=<dedicated> <cli> ...`, and the app must be started with that
     `HOME` so the adapters' environment allowlist forwards it. Delete the
     directory afterwards; it holds credentials.
4. Pick a run id and export the helper's variables (all paths absolute):
   `VP_S29_RUN_ID` (for example `local-YYYYMMDD-1`), `VP_S29_ROOT` (a new
   directory that does not exist yet: the definition root), `VP_S29_HOME`.
   Artifacts go to `docs/evidence/VP-001/artifacts/vp-s29/<run id>/`.
5. `procedures/vp-s29-linux.sh fixture`. The root now holds only
   `skills/release-notes/SKILL.md`, so the Definition section offers
   "Create .". The manifest `private/fixture.json` in the artifact folder holds
   local paths; the `private/` folder carries a `.gitignore` that excludes
   everything in it, so it is never committed.

## 2. Create the definition through the app

1. Start the app, choose "Pick workspace" and pick `VP_S29_ROOT`. Every harness
   then starts in the definition root.
2. In the Definition section tick "Also write GEMINI.md" (Antigravity CLI reads `AGENTS.md`
   directly; the wrapper keeps the root loadable by Gemini-family hosts), then
   use "Create .".
3. Type these answers, one per question (the form asks them in this order;
   the same text is in `fixture.json` under `createAnswers`). Leave the other
   questions with their proposals or blank where optional:

   | Question | Answer |
   | --- | --- |
   | Role | A writing assistant that drafts release notes from a list of changes, using the release-notes skill. |
   | Serves | The maintainers of a small software project. |
   | Deliverables | A short release notes draft for each release |
   | Objectives | Keep every release notes draft consistent in structure |
   | Limits | It only drafts and advises; it never publishes, sends or spends; it writes only in its own folder and only when asked |
   | Never invent | Version numbers or changes the maintainers did not state |
   | Tone | Plain and concise. |
   | Language | English |
   | Name | Orbit Scribe |

4. Preview, then apply. The root now holds `AGENTS.md`, `GEMINI.md` and the
   skills folder; the skills block of `AGENTS.md` lists the one local skill.
5. `procedures/vp-s29-linux.sh digests before`.

## 3. Approve the three executables

For each of `claude`, `target/release/codex-stream-bridge` and `agy`: in
"Executable approval" use "Pick executable", read the candidate evidence, type
the candidate's short digest and "Approve". Approvals are device-local and
never committed. The `codex` CLI itself is not approved: the bridge runs it.

Before the `pty-cli` leg, trust the definition root in Antigravity CLI once
from a terminal: `cd <root> && agy`, answer "Yes, I trust this folder" and
quit. The PTY adapter types the prompt as soon as the child starts, so a trust
or sign-in dialog in the TUI swallows it; a leg that stops at such a dialog
records `uncertain`.

## 4. Run each leg

For each adapter, in this order (`claude-code`, `stream-json-cli`, `pty-cli`):

1. In the Agent panel pick the adapter and its approval.
2. Run 1: prompt `Who are you?`, Start, wait for the run to end, then immediately
   `procedures/vp-s29-linux.sh digests after-<adapter>-1`.
3. Run 2: prompt `Draft release notes for version 1.2.0. Changes: added an export button; fixed a login timeout.`, Start, wait for the run to end, then immediately
   `procedures/vp-s29-linux.sh digests after-<adapter>-2`.
4. The product retains each run's harness output as a log file. The run's first
   event in the panel announces `harness log: harness-logs/<run-id>.jsonl`:
   note it. The file lives in the `harness-logs` folder of the app's local data
   directory (on Linux, the app's folder under the user's local share
   directory, the same place as the approval store). Copy it into the run
   folder as `log-<adapter>-<n>.jsonl` (`n` is 1 for the identity prompt, 2 for
   the task). The log is readable only by you (mode 0600) and holds the
   harness output unscrubbed, so it may contain secrets until `observe` scrubs
   it: never commit the raw logs.
5. If a run hangs or the harness refuses (login, trust prompt, quota), use
   Stop, keep the log the run produced, and continue: the leg records
   `uncertain`. Do not retry until it looks like a pass.

A log holds frames as the capture side saw them. A truncated log (size cap), a
log with dropped frames (including drops recorded on its final state line), a
log with no final state line (a crash, or a copy taken before the run ended) and
a log that ends mid-line each make their leg at most `uncertain`, and a log whose
header names another adapter fails the leg.

## 5. Derive and record

1. One digest was taken after each of the six invocations (step 4), so a run
   that changed a file and a later one that restored it cannot hide. Compare
   each one: `procedures/vp-s29-linux.sh compare <run dir>/digests-before.txt <run dir>/digests-after-<adapter>-<n>.txt`.
   `record` does the same comparison for all six and lists each one.
   `.omnifrons/` and the outbox may change; the digests cover only
   `AGENTS.md`, `CLAUDE.md`, `GEMINI.md` and every `skills/**/SKILL.md`.
2. Per leg: `procedures/vp-s29-linux.sh observe <adapter> <log-1> <log-2> <version-file>`
   with the adapter id (`claude-code`, `stream-json-cli`, `pty-cli`). It writes
   `observation-<adapter>.json`, a redacted reassembled `transcript-<adapter>.txt`
   (HOME prefixes become `<home>`) and `log-summary-<adapter>.json` (each log's
   header, final state, truncation and dropped-frame counts).
3. `procedures/vp-s29-linux.sh record --run-date YYYY-MM-DD --baseline-id <id> --build-channel-digest <sha256 of the AppImage> --exercised-artifact-digest <the same sha256>`
   writes `record.md` and `evidence-summary.txt`. `--build-channel` defaults
   to `packaged-ci` and `--evidence-artifact` (a file inside the run folder)
   to `evidence-summary.txt`; the record's `evidence_artifact` is that file's
   digest. `--observed-state` defaults to `uncertain`: the validator accepts
   any token there, and `uncertain` is the public synchronization state of the
   switch outcome this partial run observes. Fill the remaining
   `<operator: ...>` placeholders (record id, operator role) and check the
   legs' text against the transcripts. A run of a local source build has no
   packaged digest and cannot be filed.
4. Append the filled record to the end of `docs/evidence/VP-001/records.md`
   without editing earlier records. The record's `result` is `uncertain`
   (or `fail`); per-leg results stay in the narrative rows.
5. Commit `observation-*.json`, `transcript-*.txt`, `log-summary-*.json`, `digests-*.txt`,
   `evidence-summary.txt` and `record.md`. Never commit the `private/` folder,
   credentials or a path under a home directory. `observe` scrubs HOME
   prefixes and secret-shaped values (bearer and authorization values, common
   key prefixes, long token-like runs) with `<redacted>` and exits 1 if one
   remains, but you still review every retained file by hand before committing:
   tokens, session ids and paths in login or quota errors are not always
   recognizable. The catch-all token scrub keeps a run that is exactly a lowercase
   sha256 digest and redacts every other long base64-like run, including the
   `signature` values of a stream-json transcript: that loss is accepted. A
   symlinked `SKILL.md` is not expected in the fixture; its digest line shows the
   link's target as given, so check an absolute target for a local path.

## 6. Clean up

Remove the user-level skills (real HOME) or the dedicated HOME, and the
fixture root. Leave the approvals in the app's device-local store.
