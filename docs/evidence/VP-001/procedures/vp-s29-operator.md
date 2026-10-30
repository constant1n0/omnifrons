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
never `pass`. A user-level skill path in a transcript, or a changed digest, is
`fail`.

## 0. Bindings

| Adapter | Harness | Version | Reads |
| --- | --- | --- | --- |
| `claude-code` | Claude Code (`claude`) | 2.1.307 | `CLAUDE.md`, falling back to `AGENTS.md` |
| `stream-json-cli` | Codex CLI (`codex`) through `tools/codex-stream-bridge` | 0.163.0 | `AGENTS.md` |
| `pty-cli` | Gemini CLI (`gemini`) | 0.41.2 | `GEMINI.md` (the wrapper) |

## 1. Prepare

1. Build the bridge: `cargo build --release -p codex-stream-bridge` (the
   binary is `target/release/codex-stream-bridge`). Build or start the app the
   way you normally do (for example `pnpm exec tauri dev`).
2. Check the three versions and that each CLI is logged in. Save each version
   output; the helper reads these files:
   `claude --version > claude-version.txt`, `codex --version > codex-version.txt`,
   `gemini --version > gemini-version.txt`. A different version is recorded as
   `uncertain` for that leg.
3. Decide which HOME the whole run uses. The user-level skill must sit where
   the harness looks for it, so the choice applies to every step:
   - **Real HOME.** The CLIs already have their logins. The fixture writes
     `release-notes/SKILL.md` under `.claude/skills`, `.agents/skills` and
     `.gemini/skills` in that HOME and refuses to overwrite an existing one.
     Afterwards remove exactly those three `release-notes` directories.
   - **Dedicated HOME** (a new empty directory). Nothing else in your real
     HOME can leak into the run, but each CLI must be logged in again with
     `HOME=<dedicated> <cli> ...`, and the app must be started with that
     `HOME` so the adapters' environment allowlist forwards it. Delete the
     directory afterwards; it holds credentials.
4. Pick a run id and export the helper's variables (all paths absolute):
   `VP029_RUN_ID` (for example `local-YYYYMMDD-1`), `VP029_ROOT` (a new
   directory that does not exist yet: the definition root), `VP029_HOME`.
   Artifacts go to `docs/evidence/VP-001/artifacts/vp-s29/<run id>/`.
5. `procedures/vp-s29-linux.sh fixture`. The root now holds only
   `skills/release-notes/SKILL.md`, so the Definition section offers
   "Create .". `fixture.json` in the artifact folder holds local paths:
   never commit it.

## 2. Create the definition through the app

1. Start the app, choose "Pick workspace" and pick `VP029_ROOT`. Every harness
   then starts in the definition root.
2. In the Definition section tick "Also write GEMINI.md", then use "Create .".
3. Type these answers, one per question (the form asks them in this order;
   the same text is in `fixture.json` under `createAnswers`). Leave the other
   questions with their proposals or blank where optional:

   | Question | Answer |
   | --- | --- |
   | Role | A writing assistant that drafts release notes from a list of changes, using the release-notes skill. |
   | Serves | The maintainers of a small software project. |
   | Deliverables | A short release notes draft for each release. |
   | Objectives | Keep every release notes draft consistent in structure. |
   | Limits | It only drafts and advises; it never publishes, sends or spends; it writes only in its own folder and only when asked. |
   | Never invent | Version numbers or changes the maintainers did not state. |
   | Tone | Plain and concise. |
   | Language | English |
   | Name | Orbit Scribe |

4. Preview, then apply. The root now holds `AGENTS.md`, `GEMINI.md` and the
   skills folder; the skills block of `AGENTS.md` lists the one local skill.
5. `procedures/vp-s29-linux.sh digests before`.

## 3. Approve the three executables

For each of `claude`, `target/release/codex-stream-bridge` and `gemini`: in
"Executable approval" use "Pick executable", read the candidate evidence, type
the candidate's short digest and "Approve". Approvals are device-local and
never committed. The `codex` CLI itself is not approved: the bridge runs it.

## 4. Run each leg

For each adapter, in this order (`claude-code`, `stream-json-cli`, `pty-cli`):

1. In the Agent panel pick the adapter and its approval.
2. Run 1: prompt `Who are you?`, Start, wait for the run to end.
3. Run 2: prompt `Draft release notes for version 1.2.0. Changes: added an export button; fixed a login timeout.`, Start, wait for the run to end.
4. Copy the text of the "Agent transcript" region of both runs, in order, into
   one file per leg (`claude-transcript.txt`, `codex-transcript.txt`,
   `gemini-transcript.txt`). The first lines are the startup capture.
5. If a run hangs or the harness refuses (login, trust prompt, quota), use
   Stop, keep whatever transcript exists, and continue: the leg records
   `uncertain`. Do not retry until it looks like a pass.

## 5. Derive and record

1. `procedures/vp-s29-linux.sh digests after`, then
   `procedures/vp-s29-linux.sh compare <run dir>/digests-before.txt <run dir>/digests-after.txt`.
   `.omnifrons/` and the outbox may change; the digests cover only
   `AGENTS.md`, `CLAUDE.md`, `GEMINI.md` and every `skills/**/SKILL.md`.
2. Per leg: `procedures/vp-s29-linux.sh observe <adapter> <transcript> <version-file>`
   with the adapter id (`claude-code`, `stream-json-cli`, `pty-cli`). It writes
   `observation-<adapter>.json` and a redacted `transcript-<adapter>.txt`
   (HOME prefixes become `<home>`).
3. `procedures/vp-s29-linux.sh record --run-date YYYY-MM-DD --baseline-id <id> ...`
   writes `record.md` and `evidence-summary.txt`. Fill every
   `<operator: ...>` placeholder (record id, build channel, observed state,
   operator role) and check the legs' text against the transcripts.
4. Append the filled record to the end of `docs/evidence/VP-001/records.md`
   without editing earlier records. The record's `result` is `uncertain`
   (or `fail`); per-leg results stay in the narrative rows.
5. Commit `observation-*.json`, `transcript-*.txt`, `digests-*.txt`,
   `evidence-summary.txt` and `record.md`. Never commit `fixture.json`,
   credentials or a path under a home directory. Check
   the files for a home path before committing.

## 6. Clean up

Remove the user-level skills (real HOME) or the dedicated HOME, and the
fixture root. Leave the approvals in the app's device-local store.
