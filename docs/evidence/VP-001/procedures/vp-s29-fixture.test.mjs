// `node --test` coverage for `vp-s29-fixture.mjs`: the VP-S29 fixture over a
// temp directory, no network and no harness.

import assert from 'node:assert/strict';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import {
  CREATE_ANSWERS, PROMPTS, SKILL_NAME, USER_SKILL_DIRS, USER_SKILL_MARKER, prepareFixture,
} from './vp-s29-fixture.mjs';

function scratch(run) {
  const dir = mkdtempSync(join(tmpdir(), 'vp-s29-fixture-'));
  try {
    return run(dir);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test('prepares the indexed skill under the root and the user-level skill under HOME', () => {
  scratch((dir) => {
    const root = join(dir, 'definition');
    const home = join(dir, 'home');
    const manifest = prepareFixture({ root, home });
    assert.deepEqual(readdirSync(root), ['skills'], 'no AGENTS.md: the Create control must appear');
    const indexed = readFileSync(join(root, 'skills', SKILL_NAME, 'SKILL.md'), 'utf8');
    assert.match(indexed, new RegExp(`^---\\nname: ${SKILL_NAME}\\ndescription: [^\\n]+\\n---\\n`));
    assert.doesNotMatch(indexed, /@|\]\(/, 'no @ import and no link');
    assert.doesNotMatch(indexed, new RegExp(USER_SKILL_MARKER));
    for (const rel of USER_SKILL_DIRS) {
      const user = readFileSync(join(home, rel, SKILL_NAME, 'SKILL.md'), 'utf8');
      assert.match(user, new RegExp(`^---\\nname: ${SKILL_NAME}\\n`));
      assert.match(user, new RegExp(USER_SKILL_MARKER));
      assert.doesNotMatch(user, /@|\]\(/);
    }
    assert.equal(manifest.skillName, SKILL_NAME);
    assert.equal(manifest.indexedSkillPath, `skills/${SKILL_NAME}/SKILL.md`);
    assert.deepEqual(manifest.userLevelSkillPaths, USER_SKILL_DIRS.map((d) => `${d}/${SKILL_NAME}/SKILL.md`));
    assert.equal(manifest.local.root, root);
    assert.equal(manifest.local.home, home);
    assert.equal(manifest.definitionName, CREATE_ANSWERS.name);
  });
});

test('the fixed answers cover every required field and name the skill', () => {
  for (const key of ['role', 'deliverables', 'authority', 'work_language', 'name']) {
    assert.ok(CREATE_ANSWERS[key] && CREATE_ANSWERS[key].length > 0, key);
  }
  assert.match(CREATE_ANSWERS.role, new RegExp(SKILL_NAME));
  assert.match(PROMPTS.task, /release notes/i);
  assert.equal(PROMPTS.identity, 'Who are you?');
  for (const value of Object.values(CREATE_ANSWERS).flat()) assert.doesNotMatch(String(value), /@|\]\(/);
});

test('refuses a root that already holds entries, and a HOME inside the root', () => {
  scratch((dir) => {
    const root = join(dir, 'definition');
    mkdirSync(root);
    writeFileSync(join(root, 'AGENTS.md'), 'x');
    assert.throws(() => prepareFixture({ root, home: join(dir, 'home') }), /not empty/);
    const fresh = join(dir, 'fresh');
    assert.throws(() => prepareFixture({ root: fresh, home: join(fresh, 'home') }), /inside the root/);
    assert.equal(existsSync(join(fresh, 'skills')), false, 'nothing is written on a refusal');
  });
});

test('refuses relative paths', () => {
  assert.throws(() => prepareFixture({ root: 'rel/root', home: '/abs/home' }), /absolute/);
  assert.throws(() => prepareFixture({ root: '/abs/root', home: 'rel/home' }), /absolute/);
});

test('refuses to overwrite an existing user-level skill', () => {
  scratch((dir) => {
    const home = join(dir, 'home');
    const existing = join(home, USER_SKILL_DIRS[0], SKILL_NAME);
    mkdirSync(existing, { recursive: true });
    writeFileSync(join(existing, 'SKILL.md'), 'someone else');
    assert.throws(() => prepareFixture({ root: join(dir, 'definition'), home }), /already exists/);
    assert.equal(readFileSync(join(existing, 'SKILL.md'), 'utf8'), 'someone else');
    assert.equal(existsSync(join(dir, 'definition')), false);
  });
});
