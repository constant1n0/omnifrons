// Pure VP-S6 observation helpers, exported separately from
// `vp-s6-scenario.mjs`'s orchestration (design.md D5/D7, "Honesty
// machinery") so they are unit-testable with no process, filesystem, or
// display -- mirroring `webdriver-session.mjs`'s own split between pure
// builders and its `fetch`-based transport.

/**
 * Parse the fixture's own declared-identity file (`tools/vp-s6-agent`'s
 * `PID_FILE_NAME`) into `{ selfPid, descendantPid }`. Only the pids are
 * trusted from this file -- `vp-s6-scenario.mjs` always re-derives
 * starttime from `/proc` instead of trusting a value the fixture itself
 * recorded (design.md Threat Matrix "Executable-file classification": no
 * bypass of an independently observed fact).
 */
export function parsePidFile(contents) {
  const roles = {};
  for (const line of contents.split('\n')) {
    const [role, pid] = line.trim().split(/\s+/);
    if (role === 'self' || role === 'descendant') {
      roles[role] = Number(pid);
    }
  }
  if (!Number.isInteger(roles.self) || !Number.isInteger(roles.descendant)) {
    throw new Error(
      `pid file carries no valid self/descendant pid pair: ${JSON.stringify(contents)}`,
    );
  }
  return { selfPid: roles.self, descendantPid: roles.descendant };
}

/**
 * Parse the `starttime` field (clock ticks since boot, `proc(5)` field 22)
 * out of the raw contents of a `/proc/<pid>/stat` file. Mirrors
 * `tools/vp-s6-agent/src/main.rs`'s Rust implementation of the same
 * `proc(5)` parsing rule (skip to the line's *last* `)`, since `comm` may
 * itself contain spaces or parens); duplicated rather than shared because
 * the two run in different languages/processes.
 */
export function parseProcStatStarttime(statContents) {
  const closeParen = statContents.lastIndexOf(')');
  if (closeParen === -1) return null;
  const fields = statContents
    .slice(closeParen + 1)
    .trim()
    .split(/\s+/);
  const starttime = fields[19];
  if (starttime === undefined) return null;
  const parsed = Number(starttime);
  return Number.isInteger(parsed) ? parsed : null;
}

/**
 * Compare a pid's baseline starttime (recorded before Stop) against its
 * current starttime after the bounded post-Stop wait (`null` if the pid no
 * longer exists). Never reads a pid whose starttime changed as a survivor
 * (design.md Threat Matrix "Subprocess / process integration"): a changed
 * starttime means the OS recycled the pid onto an unrelated process, which
 * `derive` must treat the same as `gone`, never as `alive`.
 */
export function classifyPidAfterWait(baselineStarttime, currentStarttimeOrNull) {
  if (currentStarttimeOrNull === null) return 'gone';
  return currentStarttimeOrNull === baselineStarttime ? 'alive' : 'recycled';
}
