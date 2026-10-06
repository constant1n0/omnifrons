// `node --test` coverage for `vp-s14-linux.sh` (mirrors
// vp-s13-linux.test.mjs's own structure): exit contract, FIXTURE_NAMES
// resolution (missing/non-plain/dot-segment rejection, mirroring
// vp-s13-linux.sh's TRAP_NAMES guard, plus its own wall-clock bound on a
// read that never settles), a missing `true`/`false` source binary (and
// that no scratch directory is ever created for it), the fixture copy
// (content and mode 755), the scratch directory being removed on every
// exit path (including a timeout/kill), and every symlink shape inside
// it -- an escape link, case (c)'s own in-tree shape, and a truly
// dangling one -- being removed on cleanup without ever being followed.
// Stubs stand in for `vp-s6-linux.sh` and `vp-s14-scenario.mjs` (the
// latter also supplies FIXTURE_NAMES, exactly as the real file does --
// there is no separate probe-constants module to import it from here,
// unlike vp-s13-linux.sh's TRAP_NAMES/vp-s13-probe.mjs split).
// Linux-only (GNU timeout/mapfile/type -P).

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import {
  existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, symlinkSync, writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const DIGEST = 'a'.repeat(64);
const SCENARIO_MARKER = 'stub=scenario-ran';
const LINUX_ONLY =
  process.platform === 'linux' ? {} : { skip: 'vp-s14-linux.sh needs GNU timeout/mapfile/type -P (Linux only)' };
const AV_PASS = 'echo "gate=av-stub-pass"; exit 0';

// Every scenario-body fixture below must export FIXTURE_NAMES itself: the
// driver imports it straight from vp-s14-scenario.mjs (not a separate
// probe module), so whatever this harness writes as "the scenario" is
// also the one place the driver reads the three fixture names from.
const FIXTURE_NAMES_EXPORT =
  "export const FIXTURE_NAMES = Object.freeze({ approvedExe: 'approved-exe', replacementExe: 'replacement-exe', shadowTargetExe: 'shadow-target-exe' });";

// Every "active" fixture body below guards its logic behind
// `process.argv[1]`: the driver's own node -e snippet imports this same
// file to read FIXTURE_NAMES before the scratch dir or argv[2..4] even
// exist, exactly like the real vp-s14-scenario.mjs's own "only run when
// executed directly" guard at its bottom (`node -e` leaves `argv[1]`
// undefined; running the file as a script sets it to the script's own
// path) -- without this, the import-for-names step itself would try to
// run scenario logic against an undefined scratch dir and crash.

// Reads all three fixture files back out of the scratch dir (argv[4]) and
// reports their sizes, content equality, and the approved file's mode --
// proof that the driver copied the right bytes into the right names at
// the right permissions, never a raw scratch path.
const DEFAULT_SCENARIO_BODY = `
${FIXTURE_NAMES_EXPORT}
import { readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
if (process.argv[1]) {
  const scratchDir = process.argv[4];
  const approvedPath = join(scratchDir, FIXTURE_NAMES.approvedExe);
  const approved = readFileSync(approvedPath);
  const replacement = readFileSync(join(scratchDir, FIXTURE_NAMES.replacementExe));
  const shadowTarget = readFileSync(join(scratchDir, FIXTURE_NAMES.shadowTargetExe));
  const mode = (statSync(approvedPath).mode & 0o777).toString(8);
  process.stdout.write(
    '${SCENARIO_MARKER} approved_size=' + approved.length + ' mode=' + mode
    + ' approved_eq_shadow=' + approved.equals(shadowTarget)
    + ' approved_eq_replacement=' + approved.equals(replacement) + '\\n',
  );
}
`;

// A stand-in blocker: the scenario's own failure, never a driver-level gate.
const BLOCKER_SCENARIO_BODY = `${FIXTURE_NAMES_EXPORT}\nif (process.argv[1]) process.exitCode = 1;\n`;

// Outlives the driver's own SCENARIO_TIMEOUT, so `timeout` fires (status 124).
const TIMEOUT_SCENARIO_BODY = `${FIXTURE_NAMES_EXPORT}\nif (process.argv[1]) setTimeout(() => {}, 30_000);\n`;

// Ignores SIGTERM and sleeps past the 10 s --kill-after grace period, so
// `timeout` must escalate to SIGKILL (status 137).
const IGNORE_SIGTERM_SCENARIO_BODY = `
${FIXTURE_NAMES_EXPORT}
if (process.argv[1]) {
  process.on('SIGTERM', () => {});
  setTimeout(() => {}, 30_000);
}
`;

// A separator in one fixture name would let it escape the scratch directory.
const BAD_NAMES_SCENARIO_BODY =
  "export const FIXTURE_NAMES = Object.freeze({ approvedExe: '../AGENTS.md', replacementExe: 'replacement-exe', shadowTargetExe: 'shadow-target-exe' });\n";

// A bare "." names the scratch directory itself, not a file inside it.
const DOT_SEGMENT_SCENARIO_BODY =
  "export const FIXTURE_NAMES = Object.freeze({ approvedExe: '.', replacementExe: 'replacement-exe', shadowTargetExe: 'shadow-target-exe' });\n";

// Same for "..", on a different field than the dot case above.
const DOT_DOT_SEGMENT_SCENARIO_BODY =
  "export const FIXTURE_NAMES = Object.freeze({ approvedExe: 'approved-exe', replacementExe: 'replacement-exe', shadowTargetExe: '..' });\n";

// Two of the three names collide: approvedExe and replacementExe would
// name the same file, so the second `cp` silently overwrites the first.
const DUPLICATE_NAMES_SCENARIO_BODY =
  "export const FIXTURE_NAMES = Object.freeze({ approvedExe: 'dup-exe', replacementExe: 'dup-exe', shadowTargetExe: 'shadow-target-exe' });\n";

// The import itself fails, so FIXTURE_NAMES can never be read at all --
// fewer names than expected, same failure family as a malformed name.
const BROKEN_IMPORT_SCENARIO_BODY = "throw new Error('scenario import failed');\n";

// A top-level await that never resolves blocks the whole module's
// evaluation, so the driver's own `node -e` dynamic import() of this file
// never settles either -- proof that the fixture-names read step has its
// own wall-clock bound, not just the AV/scenario legs. The `setInterval`
// inside the executor is deliberate: a bare `new Promise(() => {})` has no
// pending handle of its own, so with nothing else keeping the event loop
// alive, node simply exits early (verified empirically) instead of
// genuinely hanging -- which would make this fixture accidentally land in
// the ordinary fixture-names-invalid path (an empty read) rather than
// actually exercising the timeout this test means to prove.
const NEVER_SETTLES_SCENARIO_BODY =
  `${FIXTURE_NAMES_EXPORT}\nawait new Promise(() => { setInterval(() => {}, 1_000_000); });\n`;

// Records the scratch dir (argv[4]) to a side file named by an env var,
// then exits normally -- lets the test assert the directory is gone
// afterward without the scenario ever printing a raw path itself.
const RECORD_SCRATCH_SCENARIO_BODY = `
${FIXTURE_NAMES_EXPORT}
import { writeFileSync } from 'node:fs';
if (process.argv[1]) {
  const scratchDir = process.argv[4];
  const recordPath = process.env.VP001_TEST_SCRATCH_RECORD_PATH;
  if (recordPath) writeFileSync(recordPath, scratchDir);
  process.stdout.write('${SCENARIO_MARKER} recorded=true\\n');
}
`;

// Same recording, but then hangs past SCENARIO_TIMEOUT -- proves cleanup
// still runs (via the EXIT trap) even when `timeout` has to kill the scenario.
const RECORD_AND_HANG_SCENARIO_BODY = `
${FIXTURE_NAMES_EXPORT}
import { writeFileSync } from 'node:fs';
if (process.argv[1]) {
  const scratchDir = process.argv[4];
  const recordPath = process.env.VP001_TEST_SCRATCH_RECORD_PATH;
  if (recordPath) writeFileSync(recordPath, scratchDir);
  setTimeout(() => {}, 30_000);
}
`;

// Sets up three symlink shapes inside the scratch tree, then records the
// scratch directory itself (so the test can assert it is gone afterward)
// before exiting normally:
// - the real case (c) shape: approvedExe -> shadowTargetExe, both still
//   existing, both still inside the scratch tree -- exactly what the
//   actual scenario does to approvedExe for case (c), never a stand-in on
//   a different fixture;
// - an escape link, under its OWN name (never approvedExe): points at a
//   file OUTSIDE the scratch tree entirely, so a cleanup that ever
//   dereferenced it would delete something this harness did not create
//   inside scratchDir;
// - a truly dangling link, also under its own name: points at a
//   scratch-internal path that is then removed, so by the time cleanup
//   runs the link's target does not exist anywhere at all, not merely
//   outside the tree.
const DANGLING_SYMLINKS_SCENARIO_BODY = `
${FIXTURE_NAMES_EXPORT}
import { readlinkSync, symlinkSync, unlinkSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
if (process.argv[1]) {
  const scratchDir = process.argv[4];
  const externalTarget = process.env.VP001_TEST_EXTERNAL_TARGET;
  const recordPath = process.env.VP001_TEST_SCRATCH_RECORD_PATH;
  if (recordPath) writeFileSync(recordPath, scratchDir);

  // The real case (c) shape: approvedExe becomes a symlink to the
  // in-tree shadowTargetExe (not replacementExe, and not pointing outside).
  const approvedPath = join(scratchDir, FIXTURE_NAMES.approvedExe);
  const shadowTargetPath = join(scratchDir, FIXTURE_NAMES.shadowTargetExe);
  unlinkSync(approvedPath);
  symlinkSync(shadowTargetPath, approvedPath);
  const caseCShape = readlinkSync(approvedPath) === shadowTargetPath;

  // An escape link under its own name, never approvedExe.
  const escapeLinkPath = join(scratchDir, 'escape-link.tmp');
  symlinkSync(externalTarget, escapeLinkPath);

  // A truly dangling link, also under its own name: its target is
  // removed before this process exits.
  const danglingTargetPath = join(scratchDir, 'dangling-target.tmp');
  const danglingLinkPath = join(scratchDir, 'dangling-link.tmp');
  writeFileSync(danglingTargetPath, 'temporary');
  symlinkSync(danglingTargetPath, danglingLinkPath);
  unlinkSync(danglingTargetPath);

  process.stdout.write('${SCENARIO_MARKER} symlinked=true case_c_shape=' + caseCShape + '\\n');
}
`;

/** A PATH that resolves `true`/`false` to two fake scripts written into a
 * fresh directory prepended ahead of the real PATH -- everything else
 * (timeout, node, mktemp, cp, chmod, rm) still resolves normally from the
 * inherited PATH behind it. Used to control the two source binaries'
 * content and behavior directly, rather than depending on whatever
 * `true`/`false` happen to be on this machine. */
function buildPathWithFakeTrueFalse({ trueScript, falseScript }) {
  const binDir = mkdtempSync(join(tmpdir(), 'vp-s14-fakebin-'));
  writeFileSync(join(binDir, 'true'), trueScript, { mode: 0o755 });
  writeFileSync(join(binDir, 'false'), falseScript, { mode: 0o755 });
  return { binDir, path: `${binDir}:${process.env.PATH}` };
}

/** A PATH containing every entry under /usr/bin and /bin EXCEPT `true`/
 * `false` (symlinked, not copied, so this stays cheap): used by the
 * fixture-source-missing test to make `type -P true`/`type -P false`
 * genuinely fail to resolve, while every other tool the driver needs
 * (timeout, node, mktemp, cp, chmod, rm) still resolves normally. */
function buildPathWithoutTrueOrFalse() {
  const binDir = mkdtempSync(join(tmpdir(), 'vp-s14-nobin-'));
  for (const sourceDir of ['/usr/bin', '/bin']) {
    let entries;
    try {
      entries = readdirSync(sourceDir);
    } catch {
      continue;
    }
    for (const name of entries) {
      if (name === 'true' || name === 'false') continue;
      const target = join(binDir, name);
      if (existsSync(target)) continue; // first directory's version wins, like PATH order
      try {
        symlinkSync(join(sourceDir, name), target);
      } catch {
        // A broken entry (e.g. a dangling symlink already in /usr/bin) is
        // simply skipped -- this farm only needs to cover the tools the
        // driver actually calls, not reproduce the whole directory.
      }
    }
  }
  return binDir;
}

/** Runs vp-s14-linux.sh with `avBody` for vp-s6-linux.sh's AV1/AV2 check
 * (bounded 1 s) and `scenarioBody` for vp-s14-scenario.mjs. `pathOverride`
 * replaces PATH entirely for the one test that needs `true`/`false` to be
 * genuinely unresolvable; `extraEnv` adds further env vars (e.g. the
 * scratch-dir recorder's own path) without disturbing the rest. */
function runWithAvStub({
  avBody, scenarioBody = DEFAULT_SCENARIO_BODY, scenarioTimeout = '30s', pathOverride = null, extraEnv = {},
}) {
  const dir = mkdtempSync(join(tmpdir(), 'vp-s14-linux-'));
  try {
    writeFileSync(join(dir, 'vp-s14-linux.sh'), readFileSync(join(HERE, 'vp-s14-linux.sh')));
    writeFileSync(join(dir, 'leg-status.sh'), readFileSync(join(HERE, 'leg-status.sh')));
    writeFileSync(join(dir, 'vp-s6-linux.sh'), `#!/usr/bin/env bash\n${avBody}\n`, { mode: 0o755 });
    writeFileSync(join(dir, 'vp-s14-scenario.mjs'), scenarioBody);
    writeFileSync(join(dir, 'artifact.AppImage'), '');
    const env = { ...process.env, VP001_AV_TIMEOUT: '1s', VP001_SCENARIO_TIMEOUT: scenarioTimeout, ...extraEnv };
    if (pathOverride) env.PATH = pathOverride;
    const result = spawnSync('bash', [join(dir, 'vp-s14-linux.sh'), join(dir, 'artifact.AppImage'), DIGEST], {
      encoding: 'utf8',
      env,
      timeout: 30_000,
    });
    return { status: result.status, stdout: result.stdout };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test('vp-s14-linux.sh', LINUX_ONLY, async (t) => {
  // One run per fixture, inside the Linux gate, shared across assertions below.
  const passingRun = runWithAvStub({ avBody: AV_PASS });
  const rejectedRun = runWithAvStub({ avBody: 'echo "gate=av-stub-rejected"; exit 1' });
  const usageErrorRun = runWithAvStub({ avBody: 'exit 2' });
  const scenarioBlockerRun = runWithAvStub({ avBody: AV_PASS, scenarioBody: BLOCKER_SCENARIO_BODY });
  const scenarioTimeoutRun = runWithAvStub({ avBody: AV_PASS, scenarioBody: TIMEOUT_SCENARIO_BODY, scenarioTimeout: '1s' });
  const scenarioKilledRun = runWithAvStub({ avBody: AV_PASS, scenarioBody: IGNORE_SIGTERM_SCENARIO_BODY, scenarioTimeout: '1s' });
  const badNamesRun = runWithAvStub({ avBody: AV_PASS, scenarioBody: BAD_NAMES_SCENARIO_BODY });
  const dotSegmentRun = runWithAvStub({ avBody: AV_PASS, scenarioBody: DOT_SEGMENT_SCENARIO_BODY });
  const dotDotSegmentRun = runWithAvStub({ avBody: AV_PASS, scenarioBody: DOT_DOT_SEGMENT_SCENARIO_BODY });
  const brokenImportRun = runWithAvStub({ avBody: AV_PASS, scenarioBody: BROKEN_IMPORT_SCENARIO_BODY });
  const duplicateNamesRun = runWithAvStub({ avBody: AV_PASS, scenarioBody: DUPLICATE_NAMES_SCENARIO_BODY });

  await t.test('FIXTURE_NAMES that are missing, not one plain name, a dot segment, two names colliding, or unreadable (import failure) stop the run with a named gate, before any trap or scenario', () => {
    for (const run of [badNamesRun, dotSegmentRun, dotDotSegmentRun, brokenImportRun, duplicateNamesRun]) {
      assert.equal(run.status, 1, run.stdout);
      assert.ok(run.stdout.includes('gate=fixture-names-invalid'), run.stdout);
      assert.ok(!run.stdout.includes(SCENARIO_MARKER), run.stdout);
      assert.ok(!run.stdout.includes('gate=fixtures-prepared'), run.stdout);
    }
  });

  await t.test('AV1/AV2 pass: copies true into approvedExe and shadowTargetExe, false into replacementExe, all at mode 755, runs the scenario, exits 0', () => {
    assert.equal(passingRun.status, 0, passingRun.stdout);
    assert.ok(passingRun.stdout.includes('gate=fixtures-prepared'), passingRun.stdout);
    assert.ok(passingRun.stdout.includes(SCENARIO_MARKER), passingRun.stdout);
    assert.ok(passingRun.stdout.includes('mode=755'), passingRun.stdout);
    assert.ok(passingRun.stdout.includes('approved_eq_shadow=true'), passingRun.stdout);
    assert.ok(passingRun.stdout.includes('approved_eq_replacement=false'), passingRun.stdout);
  });

  await t.test('AV1/AV2 failure paths: a rejection keeps its gate/exit 1 and skips fixture prep/scenario; a usage error keeps exit 2', () => {
    assert.equal(rejectedRun.status, 1);
    assert.ok(rejectedRun.stdout.includes('gate=av-stub-rejected'));
    assert.ok(!rejectedRun.stdout.includes('gate=fixtures-prepared'));
    assert.ok(!rejectedRun.stdout.includes(SCENARIO_MARKER));
    assert.equal(usageErrorRun.status, 2);
    assert.ok(!usageErrorRun.stdout.includes(SCENARIO_MARKER));
  });

  await t.test("the scenario's own blocker keeps exit 1 with no scenario-* gate; outliving SCENARIO_TIMEOUT is a timeout (124), and SIGTERM-ignoring outright kill is 137", () => {
    assert.equal(scenarioBlockerRun.status, 1);
    assert.ok(!scenarioBlockerRun.stdout.includes('gate=scenario-'));
    assert.ok(scenarioBlockerRun.stdout.includes('gate=fixtures-prepared'));

    assert.equal(scenarioTimeoutRun.status, 1);
    assert.ok(scenarioTimeoutRun.stdout.includes('gate=scenario-timeout'), scenarioTimeoutRun.stdout);

    assert.equal(scenarioKilledRun.status, 1);
    assert.ok(scenarioKilledRun.stdout.includes('gate=scenario-killed'), scenarioKilledRun.stdout);
  });

  await t.test('a FIXTURE_NAMES read that never settles is bounded by its own timeout, never hangs the whole run', () => {
    const run = runWithAvStub({
      avBody: AV_PASS, scenarioBody: NEVER_SETTLES_SCENARIO_BODY, extraEnv: { VP001_FIXTURE_NAMES_TIMEOUT: '1s' },
    });
    assert.equal(run.status, 1, run.stdout);
    assert.ok(run.stdout.includes('gate=fixture-names-timeout'), run.stdout);
    assert.ok(!run.stdout.includes('gate=fixtures-prepared'), run.stdout);
    assert.ok(!run.stdout.includes(SCENARIO_MARKER), run.stdout);
  });

  await t.test('byte-identical true/false sources (as from a multi-call binary) stop the run with a named gate, before any fixture is copied or the scenario runs', () => {
    // Same script, same bytes -- the exact shape a multi-call true/false
    // (busybox, uutils) would present: renamed copies that cannot be told
    // apart by content, so a bytes-only rewrite in case (b) would change
    // nothing and the scenario would misread a false "changed" Fail.
    const identicalScript = '#!/bin/sh\nexit 0\n';
    const { binDir, path } = buildPathWithFakeTrueFalse({ trueScript: identicalScript, falseScript: identicalScript });
    try {
      const run = runWithAvStub({ avBody: AV_PASS, pathOverride: path });
      assert.equal(run.status, 1, run.stdout);
      assert.ok(run.stdout.includes('gate=fixture-sources-indistinct'), run.stdout);
      assert.ok(!run.stdout.includes('gate=fixtures-prepared'), run.stdout);
      assert.ok(!run.stdout.includes(SCENARIO_MARKER), run.stdout);
    } finally {
      rmSync(binDir, { recursive: true, force: true });
    }
  });

  await t.test('a fixture that does not behave as its role requires (e.g. a renamed multi-call applet) stops the run with a named gate before the scenario runs', () => {
    // Distinct content (so the source-digest guard above passes), but the
    // "false" source still exits 0 -- the behavioural half of the same
    // multi-call-binary risk: distinguishable bytes are not proof of
    // distinguishable behavior once copied and renamed.
    const trueScript = '#!/bin/sh\nexit 0\n';
    const wrongFalseScript = '#!/bin/sh\n# deliberately wrong: a role that must exit 1 here exits 0\nexit 0\n';
    const { binDir, path } = buildPathWithFakeTrueFalse({ trueScript, falseScript: wrongFalseScript });
    try {
      const run = runWithAvStub({ avBody: AV_PASS, pathOverride: path });
      assert.equal(run.status, 1, run.stdout);
      assert.ok(run.stdout.includes('gate=fixture-behaviour-mismatch'), run.stdout);
      assert.ok(run.stdout.includes('status=0'), run.stdout);
      assert.ok(!run.stdout.includes('gate=fixtures-prepared'), run.stdout);
      assert.ok(!run.stdout.includes(SCENARIO_MARKER), run.stdout);
    } finally {
      rmSync(binDir, { recursive: true, force: true });
    }
  });

  await t.test('a missing true/false binary stops the run with a named gate, before any scratch directory (under TMPDIR) or scenario', (st) => {
    const binDir = buildPathWithoutTrueOrFalse();
    try {
      // Decided before the driver ever runs, not after: this environment's
      // restricted PATH is only useful if it actually still resolves every
      // other tool the driver needs.
      if (!existsSync(join(binDir, 'node')) || !existsSync(join(binDir, 'timeout')) || !existsSync(join(binDir, 'mktemp'))) {
        st.skip('this environment does not keep node/timeout/mktemp under /usr/bin or /bin; cannot build the restricted PATH');
        return;
      }
      // GNU mktemp honors $TMPDIR for `mktemp -d` with no explicit
      // directory argument (verified empirically) -- vp-s14-linux.sh's own
      // `scratch_root="$(mktemp -d)"` call is exactly that shape, so
      // pointing TMPDIR at a fresh, otherwise-empty directory and asserting
      // it is still empty afterward directly proves no scratch directory
      // was ever created, not merely that none was left behind.
      const tmpdirOverride = mkdtempSync(join(tmpdir(), 'vp-s14-tmpdir-'));
      try {
        const run = runWithAvStub({ avBody: AV_PASS, pathOverride: binDir, extraEnv: { TMPDIR: tmpdirOverride } });
        assert.equal(run.status, 1, run.stdout);
        assert.ok(run.stdout.includes('gate=fixture-source-missing'), run.stdout);
        assert.ok(!run.stdout.includes('gate=fixtures-prepared'), run.stdout);
        assert.ok(!run.stdout.includes(SCENARIO_MARKER), run.stdout);
        assert.deepEqual(readdirSync(tmpdirOverride), [], 'no scratch directory must ever be created under TMPDIR');
      } finally {
        rmSync(tmpdirOverride, { recursive: true, force: true });
      }
    } finally {
      rmSync(binDir, { recursive: true, force: true });
    }
  });

  await t.test('the scratch directory is removed on every exit path, including a timeout that has to kill the scenario', () => {
    const recordDir = mkdtempSync(join(tmpdir(), 'vp-s14-record-'));
    try {
      const recordPath = join(recordDir, 'scratch-path.txt');
      const normalRun = runWithAvStub({
        avBody: AV_PASS, scenarioBody: RECORD_SCRATCH_SCENARIO_BODY, extraEnv: { VP001_TEST_SCRATCH_RECORD_PATH: recordPath },
      });
      assert.equal(normalRun.status, 0, normalRun.stdout);
      assert.ok(existsSync(recordPath), normalRun.stdout);
      const recordedScratchDir = readFileSync(recordPath, 'utf8');
      assert.ok(recordedScratchDir.length > 0);
      assert.equal(existsSync(recordedScratchDir), false, 'the scratch directory must be gone once the driver exits');

      const killedRecordPath = join(recordDir, 'scratch-path-killed.txt');
      const killedRun = runWithAvStub({
        avBody: AV_PASS, scenarioBody: RECORD_AND_HANG_SCENARIO_BODY, scenarioTimeout: '1s',
        extraEnv: { VP001_TEST_SCRATCH_RECORD_PATH: killedRecordPath },
      });
      assert.equal(killedRun.status, 1, killedRun.stdout);
      assert.ok(existsSync(killedRecordPath), killedRun.stdout);
      const killedScratchDir = readFileSync(killedRecordPath, 'utf8');
      assert.equal(existsSync(killedScratchDir), false, 'cleanup must still run when timeout has to kill the scenario');
    } finally {
      rmSync(recordDir, { recursive: true, force: true });
    }
  });

  await t.test('cleanup removes every symlink inside the scratch tree -- an escape link, case (c)\'s own in-tree shape, and a truly dangling one -- without ever following any of them, and the scratch directory itself ends up gone', () => {
    const externalDir = mkdtempSync(join(tmpdir(), 'vp-s14-external-'));
    const recordDir = mkdtempSync(join(tmpdir(), 'vp-s14-record-'));
    try {
      const externalTarget = join(externalDir, 'external.txt');
      writeFileSync(externalTarget, 'do-not-delete-me');
      const recordPath = join(recordDir, 'scratch-path.txt');
      const run = runWithAvStub({
        avBody: AV_PASS, scenarioBody: DANGLING_SYMLINKS_SCENARIO_BODY,
        extraEnv: { VP001_TEST_EXTERNAL_TARGET: externalTarget, VP001_TEST_SCRATCH_RECORD_PATH: recordPath },
      });
      assert.equal(run.status, 0, run.stdout);
      assert.ok(run.stdout.includes('symlinked=true'), run.stdout);
      // The real case (c) shape is approvedExe -> shadowTargetExe (both
      // in-tree); the scenario body reports this directly, since by the
      // time cleanup runs and this test can look, the scratch tree (and
      // therefore any live symlink to inspect) is already gone.
      assert.ok(run.stdout.includes('case_c_shape=true'), run.stdout);
      assert.ok(existsSync(externalTarget), 'the external target must survive the scratch directory\'s own cleanup');
      assert.equal(readFileSync(externalTarget, 'utf8'), 'do-not-delete-me');
      assert.ok(existsSync(recordPath), run.stdout);
      const recordedScratchDir = readFileSync(recordPath, 'utf8');
      assert.equal(existsSync(recordedScratchDir), false, 'the scratch directory itself must be gone once cleanup runs');
    } finally {
      rmSync(externalDir, { recursive: true, force: true });
      rmSync(recordDir, { recursive: true, force: true });
    }
  });
});
