// `node --test` unit tests for `vp-s6-xdotool.mjs`'s pure helpers -- the
// window-id stability reducer, output parsing, and diagnostic formatting.
// The actual X11 interaction has no repository test: it needs a real
// display, and `vp-s6-scenario.mjs`'s own live run under Xvfb is its only
// exercise (see that module's header).

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  compactLines,
  countWindowIdsInTree,
  formatDiagnostics,
  nextStabilityState,
  isWindowDrivable,
  parseMapState,
  parseSearchIds,
  redactProvenance,
} from './vp-s6-xdotool.mjs';

test('parseSearchIds', () => {
  assert.deepEqual(parseSearchIds('12582917\n12582918\n'), ['12582917', '12582918']);
  assert.deepEqual(parseSearchIds(''), []);
  assert.deepEqual(parseSearchIds('\n\n  \n'), []);
});

test('parseMapState', () => {
  assert.equal(parseMapState('xwininfo: Window id: 0x1e00003\n\n  Map State: IsViewable'), 'IsViewable');
  assert.equal(parseMapState('  Map State: IsUnmapped'), 'IsUnmapped');
  assert.equal(parseMapState('xwininfo: error: no such window with id 0x1e00003.'), null);
});

test('countWindowIdsInTree', async (t) => {
  await t.test('counts window id tokens, not geometry substrings', () => {
    const tree = [
      'xwininfo: Window id: 0x1 (the root window) (has no name)',
      '  Root window id: 0x1 (the root window) (has no name)',
      '  Parent window id: 0x0 (none)',
      '     0x1e00001 "omnifrons-shell": 1024x768+0+0',
      '     0x1e00003 "Select Folder": 400x300+50+50',
    ].join('\n');
    // 0x1 (twice), 0x0, 0x1e00001, 0x1e00003 -- five id tokens; "400x300"
    // itself contains a literal "0x300" that must NOT be counted.
    assert.equal(countWindowIdsInTree(tree), 5);
  });

  await t.test('returns 0 when no window ids are present', () => {
    assert.equal(countWindowIdsInTree('<xwininfo -root -tree unavailable: spawn ENOENT>'), 0);
    assert.equal(countWindowIdsInTree('     400x300+50+50  +50+50'), 0);
  });
});

test('nextStabilityState', () => {
  assert.deepEqual(nextStabilityState({ id: null, streak: 0 }, null), { id: null, streak: 0 });
  assert.deepEqual(nextStabilityState({ id: 'A', streak: 2 }, null), { id: null, streak: 0 });
  const afterFirst = nextStabilityState({ id: null, streak: 0 }, 'A');
  assert.deepEqual(afterFirst, { id: 'A', streak: 1 });
  assert.deepEqual(nextStabilityState(afterFirst, 'A'), { id: 'A', streak: 2 });
  assert.deepEqual(nextStabilityState({ id: 'A', streak: 2 }, 'B'), { id: 'B', streak: 1 });
});

test('redactProvenance', () => {
  assert.equal(
    redactProvenance('binary at /home/runner/work/omnifrons/omnifrons/target/release/x'),
    'binary at <home>',
  );
  assert.equal(redactProvenance('/Users/alice/project/file'), '<home>');
  assert.equal(redactProvenance('cache in /root/.cache/thing'), 'cache in <root>');
  assert.equal(redactProvenance('0x1e00003 "Select Folder"'), '0x1e00003 "Select Folder"');
});

test('compactLines', () => {
  assert.equal(compactLines('a\nb\nc', 10, 100), 'a | b | c');
  assert.equal(compactLines('a\nb\nc\nd', 2, 100), 'a | b');
  assert.equal(compactLines('abcdefghij', 5, 4), 'abcd...');
});

test('formatDiagnostics', () => {
  const lines = formatDiagnostics({
    windowTitle: 'Select Folder',
    windowId: '12582917',
    attempts: 3,
    mapState: 'IsViewable',
    tree: '0x1e00001 "omnifrons-shell" at /home/runner/work/omnifrons/omnifrons',
  });
  assert.deepEqual(lines, [
    'window_title=Select Folder window_id=12582917 attempts=3 map_state=IsViewable',
    'tree=0x1e00001 "omnifrons-shell" at <home>',
  ]);
});

// CI run 35437413086: `xwininfo` is not installed on a GitHub ubuntu-24.04
// runner, every map-state probe failed with ENOENT, and the caller read that
// as "not viewable yet" until the chooser budget ran out.
test('isWindowDrivable: a viewable window is drivable on the strong check', () => {
  assert.deepEqual(isWindowDrivable('IsViewable'), { drivable: true, degraded: false });
});

test('isWindowDrivable: an unmapped window is not drivable', () => {
  assert.deepEqual(isWindowDrivable('IsUnMapped'), { drivable: false, degraded: false });
});

test('isWindowDrivable: a failed probe on an existing window is not drivable', () => {
  assert.deepEqual(isWindowDrivable(null), { drivable: false, degraded: false });
});

test('isWindowDrivable: an unavailable probe degrades to the visible-search signal', () => {
  assert.deepEqual(isWindowDrivable('probe-unavailable'), { drivable: true, degraded: true });
});
