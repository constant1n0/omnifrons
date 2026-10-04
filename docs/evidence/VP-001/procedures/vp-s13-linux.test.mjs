// `node --test` coverage for `vp-s13-linux.sh` (mirrors vp-s3-linux.test.mjs):
// exit contract, trap workspace and canary, the outside snapshot (including
// its timeout/kill indeterminacy -- no observation line at all there -- and
// a vanished outside directory), the GTK bookmark seeded under a scratch
// $HOME (restored byte-for-byte, or skipped over a symlinked path), and
// dot-segment TRAP_NAMES rejection. Stubs stand in for `vp-s6-linux.sh` and
// `vp-s13-scenario.mjs`; the real `vp-s13-probe.mjs` supplies TRAP_NAMES.
// Linux-only (GNU timeout/stat).

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import {
  accessSync, chmodSync, constants, copyFileSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const DIGEST = 'a'.repeat(64);
const SCENARIO_MARKER = 'stub=scenario-ran';
const LINUX_ONLY =
  process.platform === 'linux' ? {} : { skip: 'vp-s13-linux.sh needs GNU timeout/stat/mapfile (Linux only)' };
const AV_PASS = 'echo "gate=av-stub-pass"; exit 0';

// Follows the guidance-file symlink and compares its target to the env
// canary, never echoing it (concatenation, as in vp-s3-linux.test.mjs).
const DEFAULT_SCENARIO_BODY = `
import { readFileSync, readlinkSync } from 'node:fs';
import { TRAP_NAMES } from './vp-s13-probe.mjs';
const workspaceDir = process.argv[4];
const canary = process.env.VP001_CANARY ?? '';
const target = readlinkSync(workspaceDir + '/' + TRAP_NAMES.guidanceFile);
const content = readFileSync(target, 'utf8');
process.stdout.write('${SCENARIO_MARKER} canary_matches=' + (content === canary) + '\\n');
`;

// A stand-in escape: writes a new file into the outside dir, workspaceDir's sibling.
const ESCAPE_SCENARIO_BODY = `
import { writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
const workspaceDir = process.argv[4];
writeFileSync(join(dirname(workspaceDir), 'outside', 'escaped.txt'), 'escaped');
process.stdout.write('${SCENARIO_MARKER} wrote-outside=true\\n');
`;

// Overwrites the canary file's own content through the workspace symlink --
// no new file, unlike ESCAPE_SCENARIO_BODY above -- still a modification.
const MODIFY_THROUGH_SYMLINK_SCENARIO_BODY = `
import { writeFileSync } from 'node:fs';
import { TRAP_NAMES } from './vp-s13-probe.mjs';
const workspaceDir = process.argv[4];
writeFileSync(workspaceDir + '/' + TRAP_NAMES.guidanceFile, 'modified-through-symlink');
process.stdout.write('${SCENARIO_MARKER} modified-through-symlink=true\\n');
`;

// Removes the outside directory itself -- the after-snapshot's own target.
const REMOVE_OUTSIDE_DIR_SCENARIO_BODY = `
import { rmSync } from 'node:fs';
import { dirname, join } from 'node:path';
const workspaceDir = process.argv[4];
rmSync(join(dirname(workspaceDir), 'outside'), { recursive: true, force: true });
process.stdout.write('${SCENARIO_MARKER} removed-outside=true\\n');
`;

// Ignores SIGTERM and then sleeps well past the driver's own 10 s
// --kill-after grace period, so `timeout` must escalate to SIGKILL: the
// process cannot handle that signal, so its own exit status is 128+9=137,
// the "killed outright" counterpart to the plain 124 timeout.
const IGNORE_SIGTERM_SCENARIO_BODY = `
process.on('SIGTERM', () => {});
setTimeout(() => {}, 30_000);
`;

// Reads $HOME's own bookmarks file (through any symlink, untouched if one)
// and reports whether it is exactly the seeded workspace line.
const BOOKMARK_READ_SCENARIO_BODY = `
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
const workspaceDir = process.argv[4];
const bookmarksPath = join(process.env.HOME, '.config', 'gtk-3.0', 'bookmarks');
const content = readFileSync(bookmarksPath, 'utf8');
process.stdout.write('${SCENARIO_MARKER} bookmarks_seeded=' + (content === 'file://' + workspaceDir + '\\n') + '\\n');
`;

// Runs vp-s13-linux.sh with `avBody` for vp-s6-linux.sh's AV1/AV2 check
// (bounded 1 s) and `scenarioBody` for vp-s13-scenario.mjs. HOME is this
// fixture's own scratch dir, never the real one: the script seeds and
// restores a GTK bookmark under $HOME/.config/gtk-3.0/bookmarks.
// `homeBookmarks` pre-creates that file before the run: `{ content }` for a
// plain pre-existing file, `{ content, unreadable: true }` for a file whose
// backup copy must fail (see below), or `{ symlink: true, targetContent }`
// for a symlinked path (its target lives alongside, under `dir`). The
// returned bookmarks* fields reflect post-run state, read before `dir` is
// removed.
function runWithAvStub({
  avBody, scenarioBody = DEFAULT_SCENARIO_BODY, scenarioTimeout = '30s', probeBody = null, homeBookmarks = null,
}) {
  const dir = mkdtempSync(join(tmpdir(), 'vp-s13-linux-'));
  try {
    copyFileSync(join(HERE, 'vp-s13-linux.sh'), join(dir, 'vp-s13-linux.sh'));
    copyFileSync(join(HERE, 'leg-status.sh'), join(dir, 'leg-status.sh'));
    if (probeBody === null) copyFileSync(join(HERE, 'vp-s13-probe.mjs'), join(dir, 'vp-s13-probe.mjs'));
    else writeFileSync(join(dir, 'vp-s13-probe.mjs'), `${probeBody}\n`);
    writeFileSync(join(dir, 'vp-s6-linux.sh'), `#!/usr/bin/env bash\n${avBody}\n`, { mode: 0o755 });
    writeFileSync(join(dir, 'vp-s13-scenario.mjs'), `${scenarioBody}\n`);
    writeFileSync(join(dir, 'artifact.AppImage'), '');
    const bookmarksPath = join(dir, '.config', 'gtk-3.0', 'bookmarks');
    const bookmarksTarget = join(dir, 'bookmarks-target.txt');
    let unreadableHeld = false;
    if (homeBookmarks) {
      mkdirSync(dirname(bookmarksPath), { recursive: true });
      if (homeBookmarks.symlink) {
        writeFileSync(bookmarksTarget, homeBookmarks.targetContent ?? 'target-content\n');
        symlinkSync(bookmarksTarget, bookmarksPath);
      } else {
        writeFileSync(bookmarksPath, homeBookmarks.content);
        // Unreadable, not just unwritable: makes the script's own `cp -p`
        // fail reading the source, while `mktemp` -- which only needs to
        // create a new file in the (still writable) directory -- still
        // succeeds, so the failure lands on the backup copy itself, the way
        // vp-s13-linux.sh's own reason=backup-failed branch expects.
        if (homeBookmarks.unreadable) {
          chmodSync(bookmarksPath, 0o000);
          // Mode 000 does not stop a reader with CAP_DAC_OVERRIDE (root, as
          // in many CI containers): there the script's own `cp -p` succeeds
          // and the backup-failed branch cannot be provoked this way, so the
          // case records whether the fixture actually held.
          try {
            accessSync(bookmarksPath, constants.R_OK);
          } catch {
            unreadableHeld = true;
          }
        }
      }
    }
    const result = spawnSync('bash', [join(dir, 'vp-s13-linux.sh'), join(dir, 'artifact.AppImage'), DIGEST], {
      encoding: 'utf8',
      env: { ...process.env, HOME: dir, VP001_AV_TIMEOUT: '1s', VP001_SCENARIO_TIMEOUT: scenarioTimeout },
      timeout: 30_000,
    });
    // Restored before this harness inspects it: the fixture made it
    // unreadable to provoke the script's own failure, but that must not
    // also block this test's own read of the post-run bookmarks state.
    if (homeBookmarks && !homeBookmarks.symlink && homeBookmarks.unreadable) chmodSync(bookmarksPath, 0o600);
    const bookmarksIsSymlink = existsSync(bookmarksPath) && lstatSync(bookmarksPath).isSymbolicLink();
    return {
      status: result.status,
      stdout: result.stdout,
      bookmarksExists: existsSync(bookmarksPath),
      bookmarksIsSymlink,
      unreadableHeld,
      bookmarksContent: existsSync(bookmarksPath) && !bookmarksIsSymlink ? readFileSync(bookmarksPath, 'utf8') : null,
      bookmarksTargetContent: existsSync(bookmarksTarget) ? readFileSync(bookmarksTarget, 'utf8') : null,
    };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test('vp-s13-linux.sh', LINUX_ONLY, async (t) => {
  // One run per fixture, inside the Linux gate, shared across assertions below.
  const passingRun = runWithAvStub({ avBody: AV_PASS });
  const escapingRun = runWithAvStub({ avBody: AV_PASS, scenarioBody: ESCAPE_SCENARIO_BODY });
  const symlinkModifyRun = runWithAvStub({ avBody: AV_PASS, scenarioBody: MODIFY_THROUGH_SYMLINK_SCENARIO_BODY });
  const removeOutsideRun = runWithAvStub({ avBody: AV_PASS, scenarioBody: REMOVE_OUTSIDE_DIR_SCENARIO_BODY });
  const rejectedRun = runWithAvStub({ avBody: 'echo "gate=av-stub-rejected"; exit 1' });
  const usageErrorRun = runWithAvStub({ avBody: 'exit 2' });
  const scenarioBlockerRun = runWithAvStub({ avBody: AV_PASS, scenarioBody: 'process.exitCode = 1;' });
  const scenarioTimeoutRun = runWithAvStub({ avBody: AV_PASS, scenarioBody: 'setTimeout(() => {}, 30_000);', scenarioTimeout: '1s' });
  const scenarioKilledRun = runWithAvStub({ avBody: AV_PASS, scenarioBody: IGNORE_SIGTERM_SCENARIO_BODY, scenarioTimeout: '1s' });
  const badNamesRun = runWithAvStub({
    avBody: AV_PASS, probeBody: "export const TRAP_NAMES = { guidanceFile: '../AGENTS.md', absentGuidanceFile: 'x.md' };",
  });
  const brokenProbeRun = runWithAvStub({ avBody: AV_PASS, probeBody: "throw new Error('probe import failed');" });
  const dotSegmentGuidanceRun = runWithAvStub({
    avBody: AV_PASS, probeBody: "export const TRAP_NAMES = { guidanceFile: '.', absentGuidanceFile: 'x.md' };",
  });
  const dotDotSegmentAbsentRun = runWithAvStub({
    avBody: AV_PASS, probeBody: "export const TRAP_NAMES = { guidanceFile: 'AGENTS.md', absentGuidanceFile: '..' };",
  });
  const bookmarkSeedRun = runWithAvStub({ avBody: AV_PASS, scenarioBody: BOOKMARK_READ_SCENARIO_BODY });
  const existingBookmarksContent = 'file:///home/someone/Documents\n';
  const bookmarkRestoreRun = runWithAvStub({
    avBody: AV_PASS, scenarioBody: BOOKMARK_READ_SCENARIO_BODY, homeBookmarks: { content: existingBookmarksContent },
  });
  const bookmarkSymlinkTargetContent = 'symlink-target-content\n';
  const bookmarkSymlinkRun = runWithAvStub({
    avBody: AV_PASS, scenarioBody: BOOKMARK_READ_SCENARIO_BODY,
    homeBookmarks: { symlink: true, targetContent: bookmarkSymlinkTargetContent },
  });
  const bookmarkBackupFailedContent = 'file:///home/someone/Documents\n';
  // The default scenario (not BOOKMARK_READ_SCENARIO_BODY): the bookmarks
  // file stays unreadable (by this same test-runner user) for the whole
  // run, so a scenario that tried to read it through $HOME would itself
  // fail; this case asserts the untouched byte-for-byte state from the
  // harness's own post-run read instead, once permissions are restored.
  const bookmarkBackupFailedRun = runWithAvStub({
    avBody: AV_PASS, homeBookmarks: { content: bookmarkBackupFailedContent, unreadable: true },
  });

  await t.test('TRAP_NAMES that are missing, not one plain name, or a dot segment stop the run with a named gate, before any trap or scenario', () => {
    for (const run of [badNamesRun, brokenProbeRun, dotSegmentGuidanceRun, dotDotSegmentAbsentRun]) {
      assert.equal(run.status, 1, run.stdout);
      assert.ok(run.stdout.includes('gate=trap-names-invalid'), run.stdout);
      assert.ok(!run.stdout.includes(SCENARIO_MARKER), run.stdout);
    }
  });

  await t.test('AV1/AV2 pass: prepares the trap workspace, runs the scenario, exits 0, and emits a length-only canary gate', () => {
    assert.equal(passingRun.status, 0);
    assert.ok(passingRun.stdout.includes(SCENARIO_MARKER), passingRun.stdout);
    const line = passingRun.stdout.split('\n').find((candidate) => candidate.startsWith('gate=canary-created'));
    assert.ok(line, passingRun.stdout);
    assert.match(line, /^gate=canary-created length=\d+$/);
    assert.ok(passingRun.stdout.includes('canary_matches=true'), passingRun.stdout);
  });

  await t.test('AV1/AV2 failure paths: a rejection keeps its gate/exit 1 and skips canary/scenario; a usage error keeps exit 2', () => {
    assert.equal(rejectedRun.status, 1);
    assert.ok(rejectedRun.stdout.includes('gate=av-stub-rejected'));
    assert.ok(!rejectedRun.stdout.includes('gate=canary-created'));
    assert.ok(!rejectedRun.stdout.includes(SCENARIO_MARKER));
    assert.equal(usageErrorRun.status, 2);
    assert.ok(!usageErrorRun.stdout.includes(SCENARIO_MARKER));
  });

  await t.test('outside snapshot: unmodified on a no-op scenario, modified via a new file, via the symlinked canary itself, or by removing the outside directory outright; no raw /tmp path either way', () => {
    assert.ok(passingRun.stdout.includes('gate=outside-snapshot-taken'), passingRun.stdout);
    assert.ok(passingRun.stdout.includes('observation=outside_target_modified value=false'), passingRun.stdout);
    assert.ok(!passingRun.stdout.includes('/tmp/'), passingRun.stdout);

    assert.equal(escapingRun.status, 0);
    assert.ok(escapingRun.stdout.includes('wrote-outside=true'), escapingRun.stdout);
    assert.ok(escapingRun.stdout.includes('gate=outside-snapshot-taken'), escapingRun.stdout);
    assert.ok(escapingRun.stdout.includes('observation=outside_target_modified value=true'), escapingRun.stdout);
    assert.ok(!escapingRun.stdout.includes('/tmp/'), escapingRun.stdout);

    assert.equal(symlinkModifyRun.status, 0);
    assert.ok(symlinkModifyRun.stdout.includes('modified-through-symlink=true'), symlinkModifyRun.stdout);
    assert.ok(symlinkModifyRun.stdout.includes('gate=outside-snapshot-taken'), symlinkModifyRun.stdout);
    assert.ok(symlinkModifyRun.stdout.includes('observation=outside_target_modified value=true'), symlinkModifyRun.stdout);

    assert.equal(removeOutsideRun.status, 0);
    assert.ok(removeOutsideRun.stdout.includes('removed-outside=true'), removeOutsideRun.stdout);
    assert.ok(removeOutsideRun.stdout.includes('gate=outside-snapshot-taken'), removeOutsideRun.stdout);
    assert.ok(removeOutsideRun.stdout.includes('observation=outside_target_modified value=true'), removeOutsideRun.stdout);
    assert.ok(!removeOutsideRun.stdout.includes('gate=outside-snapshot-failed'), removeOutsideRun.stdout);
  });

  await t.test("the scenario's own blocker keeps exit 1 (outside snapshot still ran, unmodified); outliving SCENARIO_TIMEOUT is a timeout, and an unchanged snapshot there is indeterminate, never taken", () => {
    assert.equal(scenarioBlockerRun.status, 1);
    assert.ok(!scenarioBlockerRun.stdout.includes('gate=scenario-'));
    assert.ok(scenarioBlockerRun.stdout.includes('gate=outside-snapshot-taken'));

    assert.equal(scenarioTimeoutRun.status, 1);
    assert.ok(scenarioTimeoutRun.stdout.includes('gate=scenario-timeout'), scenarioTimeoutRun.stdout);
    assert.ok(scenarioTimeoutRun.stdout.includes('gate=outside-snapshot-indeterminate'), scenarioTimeoutRun.stdout);
    assert.ok(!scenarioTimeoutRun.stdout.includes('gate=outside-snapshot-taken'), scenarioTimeoutRun.stdout);
    // No observation line at all here: absence, not a false value, is what
    // must tell derive_ipc_boundary this was never observed.
    assert.ok(!scenarioTimeoutRun.stdout.includes('observation=outside_target_modified'), scenarioTimeoutRun.stdout);
  });

  await t.test('the scenario killed outright (status 137) is indeterminate too: same absence of an observation line, same exit status as a timeout', () => {
    assert.equal(scenarioKilledRun.status, 1);
    assert.ok(scenarioKilledRun.stdout.includes('gate=scenario-killed'), scenarioKilledRun.stdout);
    assert.ok(scenarioKilledRun.stdout.includes('gate=outside-snapshot-indeterminate'), scenarioKilledRun.stdout);
    assert.ok(!scenarioKilledRun.stdout.includes('gate=outside-snapshot-taken'), scenarioKilledRun.stdout);
    // No observation line at all here either, for the same reason as the
    // timeout case above: an unchanged snapshot under a kill does not prove
    // nothing happened, since the session delete may not have ended the app.
    assert.ok(!scenarioKilledRun.stdout.includes('observation=outside_target_modified'), scenarioKilledRun.stdout);
  });

  await t.test('GTK bookmark backup failure: the original file is left byte-for-byte untouched, named gate=bookmark-seed-skipped reason=backup-failed, and never seeded', (st) => {
    if (!bookmarkBackupFailedRun.unreadableHeld) {
      st.skip('mode 000 does not block reads for this user (e.g. root), so the backup failure cannot be provoked');
      return;
    }
    assert.equal(bookmarkBackupFailedRun.status, 0);
    assert.ok(bookmarkBackupFailedRun.stdout.includes('gate=bookmark-seed-skipped reason=backup-failed'), bookmarkBackupFailedRun.stdout);
    assert.ok(!bookmarkBackupFailedRun.stdout.includes('gate=bookmark-seed-skipped reason=symlink'), bookmarkBackupFailedRun.stdout);
    assert.equal(bookmarkBackupFailedRun.bookmarksIsSymlink, false);
    assert.equal(bookmarkBackupFailedRun.bookmarksContent, bookmarkBackupFailedContent);
  });

  await t.test('GTK bookmark: seeded then removed with no prior file; restored byte-for-byte over a prior file; left untouched (gate=bookmark-seed-skipped) over a symlinked path', () => {
    assert.equal(bookmarkSeedRun.status, 0);
    assert.ok(bookmarkSeedRun.stdout.includes('bookmarks_seeded=true'), bookmarkSeedRun.stdout);
    assert.equal(bookmarkSeedRun.bookmarksExists, false);

    assert.equal(bookmarkRestoreRun.status, 0);
    assert.ok(bookmarkRestoreRun.stdout.includes('bookmarks_seeded=true'), bookmarkRestoreRun.stdout);
    assert.equal(bookmarkRestoreRun.bookmarksExists, true);
    assert.equal(bookmarkRestoreRun.bookmarksIsSymlink, false);
    assert.equal(bookmarkRestoreRun.bookmarksContent, existingBookmarksContent);

    assert.equal(bookmarkSymlinkRun.status, 0);
    assert.ok(bookmarkSymlinkRun.stdout.includes('gate=bookmark-seed-skipped reason=symlink'), bookmarkSymlinkRun.stdout);
    assert.ok(bookmarkSymlinkRun.stdout.includes('bookmarks_seeded=false'), bookmarkSymlinkRun.stdout);
    assert.equal(bookmarkSymlinkRun.bookmarksIsSymlink, true);
    assert.equal(bookmarkSymlinkRun.bookmarksTargetContent, bookmarkSymlinkTargetContent);
  });
});
