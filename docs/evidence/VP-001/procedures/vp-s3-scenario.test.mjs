// `node --test` unit tests for `vp-s3-scenario.mjs`'s pure seams; `main` is
// the E2E-only path and never runs on import (its direct-execution guard).

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { isSupportedOs } from './vp-s3-scenario.mjs';

test('isSupportedOs', async (t) => {
  await t.test("accepts exactly vp-s3-probe.mjs's three supported OS tokens", () => {
    for (const os of ['linux', 'macos', 'windows']) {
      assert.equal(isSupportedOs(os), true, os);
    }
  });

  await t.test('rejects any other value, including empty, undefined, null, and a near-miss token', () => {
    for (const os of ['bsd', 'Linux', '', undefined, null]) {
      assert.equal(isSupportedOs(os), false, String(os));
    }
  });
});
