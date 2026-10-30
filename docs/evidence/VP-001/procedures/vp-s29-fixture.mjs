// VP-S29 fixture: the definition root (an empty project folder but for one
// indexed local skill, so the Definition section offers "Create .") and a
// same-named user-level skill under a HOME the caller names. Pure Node, no
// network, no harness. Never reads or writes the real home unless the caller
// passes it as `home`.
//
// User-level skill locations (host-documented, not defined by the domain:
// `crates/omnifrons-domain/src/hosts.rs` records which files a host loads,
// not where its user-level skills live). Each directory is the documented
// user skill location of at least one of the three bound harnesses:
//   .claude/skills  Claude Code
//   .agents/skills  Codex CLI (and a Gemini CLI alias)
//   .gemini/skills  Gemini CLI
// The skill is written to all three so every harness can see a same-named
// user-level skill wherever it looks. Sources: the Codex CLI docs ("Build
// skills": user skills in $HOME/.agents/skills; .agents/skills scanned from the
// working directory up to the repository root), the Gemini CLI docs ("Skills":
// ~/.gemini/skills or the ~/.agents/skills alias; workspace .gemini/skills or
// .agents/skills, the .agents alias first within a tier) and Claude Code's
// personal skills folder, ~/.claude/skills.
//
// The definition root must hold neither .agents/skills nor .gemini/skills: a
// host would then load the project skill through its own loader instead of the
// indexed path (PROJECT_SKILL_DIRS).
//
// CLI: node vp-s29-fixture.mjs --root <dir> --home <dir> --manifest <file>

import { existsSync, mkdirSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, isAbsolute, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

export const SKILL_NAME = 'release-notes';
export const LOCAL_SKILL_MARKER = 'LOCAL-SKILL-MARKER';
export const USER_SKILL_MARKER = 'USER-LEVEL-SKILL-MARKER';
export const USER_SKILL_DIRS = ['.claude/skills', '.agents/skills', '.gemini/skills'];
export const PROJECT_SKILL_DIRS = ['.agents/skills', '.gemini/skills'];

/** Fixed, neutral answers for the Definition section's create form, in the
 * form's own order (the keys are the answers document's field keys). */
export const CREATE_ANSWERS = {
  role: `A writing assistant that drafts release notes from a list of changes, using the ${SKILL_NAME} skill.`,
  serves: 'The maintainers of a small software project.',
  deliverables: ['A short release notes draft for each release.'],
  objectives: ['Keep every release notes draft consistent in structure.'],
  authority: ['It only drafts and advises; it never publishes, sends or spends; it writes only in its own folder and only when asked.'],
  sources: [],
  never_invent: ['Version numbers or changes the maintainers did not state.'],
  tone: 'Plain and concise.',
  work_language: 'English',
  name: 'Orbit Scribe',
  open_decisions: [],
};

export const PROMPTS = {
  identity: 'Who are you?',
  task: 'Draft release notes for version 1.2.0. Changes: added an export button; fixed a login timeout.',
};

function skillText(description, marker) {
  return `---\nname: ${SKILL_NAME}\ndescription: ${description}\n---\n\n# Release notes\n\n`
    + `Write release notes as a short list of changes under the headings Added and Fixed.\n`
    + `Skill marker: ${marker}\n`;
}

function assertAbsolute(label, path) {
  if (typeof path !== 'string' || !isAbsolute(path)) throw new Error(`${label} must be an absolute path`);
}

/** Prepares the fixture and returns its manifest. Every refusal happens
 * before the first write. */
export function prepareFixture({ root, home }) {
  assertAbsolute('root', root);
  assertAbsolute('home', home);
  const fromRoot = relative(root, home);
  if (fromRoot === '' || (!fromRoot.startsWith('..') && !isAbsolute(fromRoot))) {
    throw new Error('home is inside the root: the user-level skill would join the definition');
  }
  for (const dir of PROJECT_SKILL_DIRS) {
    if (existsSync(join(root, ...dir.split('/')))) throw new Error(`root holds ${dir}: a host would load it through its own loader`);
  }
  if (existsSync(root) && readdirSync(root).length > 0) throw new Error('root is not empty');
  const userFiles = USER_SKILL_DIRS.map((dir) => join(home, ...dir.split('/'), SKILL_NAME, 'SKILL.md'));
  for (const file of userFiles) {
    if (existsSync(file)) throw new Error(`a user-level skill already exists at ${file}`);
  }

  // Everything this call creates is tracked so a failure part-way removes
  // exactly that and nothing that existed before.
  const created = [];
  const makeDir = (dir) => {
    const first = mkdirSync(dir, { recursive: true });
    if (first) created.push(first);
  };
  const write = (file, text) => {
    created.push(file);
    writeFileSync(file, text);
  };
  try {
    const indexed = join(root, 'skills', SKILL_NAME, 'SKILL.md');
    makeDir(dirname(indexed));
    write(indexed, skillText('Drafts release notes from a list of changes.', LOCAL_SKILL_MARKER));
    for (const file of userFiles) {
      makeDir(dirname(file));
      write(file, skillText('Drafts release notes in the user-level style.', USER_SKILL_MARKER));
    }
  } catch (error) {
    for (const path of created.reverse()) rmSync(path, { recursive: true, force: true });
    throw error;
  }
  return {
    scenario: 'VP-S29',
    skillName: SKILL_NAME,
    definitionName: CREATE_ANSWERS.name,
    indexedSkillPath: `skills/${SKILL_NAME}/SKILL.md`,
    userLevelSkillPaths: USER_SKILL_DIRS.map((dir) => `${dir}/${SKILL_NAME}/SKILL.md`),
    localSkillMarker: LOCAL_SKILL_MARKER,
    userSkillMarker: USER_SKILL_MARKER,
    createAnswers: CREATE_ANSWERS,
    prompts: PROMPTS,
    local: { root, home },
  };
}

function argValue(args, name) {
  const at = args.indexOf(name);
  return at >= 0 ? args[at + 1] : undefined;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const [root, home, manifestPath] = ['--root', '--home', '--manifest'].map((name) => argValue(args, name));
  if (!root || !home || !manifestPath) {
    process.stderr.write('usage: vp-s29-fixture.mjs --root <dir> --home <dir> --manifest <file>\n');
    process.exit(2);
  }
  try {
    const manifest = prepareFixture({ root, home });
    mkdirSync(dirname(manifestPath), { recursive: true });
    writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
    // The manifest holds local paths: keep its folder out of any commit.
    writeFileSync(join(dirname(manifestPath), '.gitignore'), '*\n!.gitignore\n');
  } catch (error) {
    process.stderr.write(`vp-s29-fixture.mjs: ${error.message}\n`);
    process.exit(1);
  }
}
