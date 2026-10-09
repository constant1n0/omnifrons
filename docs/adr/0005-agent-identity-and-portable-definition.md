# ADR-0005: Agent Identity and Portable Definition

**Document role:** Agent definition layer decision  
**Status:** Proposed  
**Accountable role:** Project Maintainer  
**Named person:** constant1n0  
**Approver role:** Project Owner  
**Approver named person:** constant1n0  
**Proposed on:** 2026-10-09  
**Accepted on:** None  
**Last status change:** 2026-10-09 — created as Proposed  
**Acceptance gate:** A new VP-001 scenario, run on the pinned Linux baseline with each built-in adapter bound to a named harness, shows one agent definition loading identically through each adapter and surviving a model switch; the migration of prototype-adapted agents, the write path's refusals and its idempotency are asserted on fixtures; an `uncertain` run never discharges the gate  
**Supersedes:** None  
**Name status:** Selected public name; preliminary screening complete, formal trademark clearance pending

## Context and drivers

Omnifrons provides "one logical agent" while the harness or provider changes ([Product boundary](../target-architecture.md#product-boundary)). Nothing in the product yet says what that agent is told to be, or which file a harness reads to find out.

### What exists today

- **The guidance note is advisory, never identity.** `crates/omnifrons-domain/src/guidance.rs` writes one product-owned block, the [agent guidance note](../heavy-asset-publication.md#agent-guidance-note), into a project's guidance file (`AGENTS.md` by default) under hash-locked sentinels (HAP-001 D18, HAP-001-R42). The module treats the note as "content the product proposes and the user disposes of, never authority". It says where generated files go, not who the agent is.
- **Three built-in adapters, no loader model.** The closed `AdapterId` set in `crates/omnifrons-domain/src/adapter.rs` names `stream-json-cli`, `claude-code` and `pty-cli`. Each launches a user-installed CLI in the picked workspace. None models which instruction file that CLI then reads from the folder or from its ancestors.
- **No skills concept.** Neither the domain nor the adapters model skills, although the Context Orb specification already renders them and states where they come from ([Agent identity classes and model authority](../context-orb.md#agent-identity-classes-and-model-authority) ("supplied by the scope's context root")).
- **"Logical agent" means runtime continuity.** The term names the identity that survives harness restarts, not a file on disk ([Shared terminology](../target-architecture.md#shared-terminology) ("Omnifrons-owned durable identity")).

### Why it matters now

A model change is a checkpoint-and-restart handoff that starts the selected harness in the same project folder ([Model and harness switching](../target-architecture.md#model-and-harness-switching) ("in the same `ActiveProjectRoot`")). The Pre-alpha exit criteria require a task to resume through another adapter ([Pre-alpha → Exit criteria](../roadmap.md#exit-criteria) ("through a different built-in adapter")).

Each harness, however, chooses its own instruction file in that folder. Claude Code reads `CLAUDE.md`, and reads `AGENTS.md` only when no `CLAUDE.md` exists in the working directory or above it. Codex, OpenCode and Kimi read `AGENTS.md`. Gemini CLI reads `GEMINI.md` by default. Hosts that walk upward also load an ancestor folder's file. These claims come from the prototype's loader table (below), which records a vendor source and an access date for each and was not runtime-tested.

A restart through a different adapter can therefore load a different identity, or none, from the same root. The per-host instruction file is a channel the switch does not model.

### What the prototype established

The `base_omnifrons` prototype (private repository, commit d2d0cb9) made a folder-based agent portable across hosts:

- a canonical `AGENTS.md`, a `CLAUDE.md` that is exactly `@AGENTS.md` plus a newline, and a local `skills/` tree indexed from `AGENTS.md`;
- a consent-gated adapter (`adapter/portable-agent/scripts/portable_agent.py`) and a new-agent writer (`adapter/new-agent/scripts/new_agent.py`), standard-library Python, each writing only through dry-run, plan id and apply; the adapter backs up every file it replaces and records a manifest;
- a loader table for nine hosts, each row with its sources and verification status (`adapter/portable-agent/references/HOSTS.md`, accessed 2026-09-25);
- 126 automated tests, all structural file checks (`tests/test_portable_agent.py`, `tests/test_new_agent.py`, `tests/test_prototype_contract.py`).

Its manual validation record (`prototype/VALIDATION.md`) found a real hazard. With a skill of the same name installed both globally and in the agent's folder, Claude Code used the global copy instead of the indexed local file, and Gemini Antigravity reported an ambiguous location. The prototype then added an unconditional local-precedence rule in a generated, hash-locked block. The same case passed on tool-trace evidence in six hosts as the record counts them: Claude Code, Codex, Kimi, Gemini Antigravity, DeepSeek and Qwen (Kimi, DeepSeek and Qwen ran through OpenCode, Qwen also through its own CLI). The record bounds that evidence itself: six hosts, not "every host, version, configuration, or provider".

### Drivers

- One definition behind every built-in adapter: a switch changes the harness, never who the agent is.
- Local skills win over same-named global ones, as a product-owned rule backed by evidence.
- Writes into a user's instruction files follow the guidance note's discipline: preview, consent, snapshot, scoped removal.
- No new authority: a file in a project never approves an executable or widens a permission.
- Nested agents are independent: consent for a parent never covers a child.
- Truthful status: what Omnifrons cannot observe in a host is reported as unverified, never as supported.

## Proposed decision

Subject to the acceptance gate:

1. **The agent definition is a project root.** It is the folder that holds, or will hold once Omnifrons works in it, the reserved `.omnifrons/` namespace, the outbox and the guidance note (HAP-001), and it contains:
   - a canonical `AGENTS.md`, the single source of truth for the agent's instructions;
   - a `CLAUDE.md` that is exactly `@AGENTS.md` plus a newline, so a host that reads `CLAUDE.md` first reaches the same text;
   - an optional `skills/` tree, one folder per skill with its `SKILL.md`, indexed from `AGENTS.md`.

   Another wrapper, such as a `GEMINI.md` holding the same single import line, is added only when the user chooses it.

   A nested definition is a nested project root. Each definition is created, adapted or migrated only with its own consent; a parent's consent never covers a child, and no operation recurses.

   ```text
   <project root>/              one agent definition
   ├── AGENTS.md                canonical instructions plus product-owned blocks
   ├── CLAUDE.md                "@AGENTS.md" and a newline, nothing else
   ├── skills/<name>/SKILL.md   optional local skills, indexed from AGENTS.md
   ├── .omnifrons/              reserved namespace (HAP-001)
   └── <subfolder>/             a nested definition: its own root, its own consent
   ```

2. **One sentinel family, with a new `skills` kind.** Product-owned blocks in `AGENTS.md` use the existing Omnifrons family in `crates/omnifrons-domain/src/guidance.rs`: `<!-- omnifrons:begin <kind> <version> sha256:<digest> -->` through `<!-- omnifrons:end <kind> -->`.
   - The `guidance` kind, and the `ignore` kind the family keeps in `.gitignore`, stay as they are.
   - The `skills` kind is new. It holds index rows for local skills not already indexed by hand, plus the local-skill precedence rule: for a matching task the indexed workspace path is authoritative, a native skill tool may be used only when it demonstrably resolves to that exact path, and the rule's own words are "Never substitute a same-named user, global, or provider skill."
   - Each kind carries its own template version and the digest of its body. A block edited by hand is refused as `block-modified`, never replaced. Both kinds follow HAP-001 D18: a snapshot before every write, deduplicated and pinnable snapshots, idempotent reapply, and removal of only the lines Omnifrons owns.
   - Agents the prototype adapted carry `<!-- portable-agent:skills:begin sha256=… -->` blocks. A one-time migration rewrites an intact block into the `skills` kind through the write path below, with its own preview and consent. A hand-edited block is reported, not migrated. An existing `.portable-agent/` backup folder is reported and left in place; Omnifrons never deletes it.

3. **One write path: dry-run, plan id, apply.** Every write to a definition (create, adapt, migrate, remove) follows the same sequence:
   - the dry-run writes nothing and returns the operations, the block diff and a plan id: a SHA-256 digest of the definition's path, each input file's digest and the planned operations;
   - a refusal stops the plan instead of working around it: a `CLAUDE.md` and an `AGENTS.md` holding different text wait for the user to pick the canonical file, and nothing is merged; a symlinked or irregular identity file is refused, and so is a nested definition without its own consent;
   - apply accepts only that plan id, re-reads the inputs, refuses a stale plan, and writes each file atomically;
   - before any file is replaced, it is snapshotted in the existing snapshot store in the product work area (device-local, never a roaming payload), not in a backup folder inside the definition as the prototype does; each apply also records a manifest of its plan id and every file's before and after digests;
   - the path is an application service reached only through typed IPC commands, as the guidance installer's `guidance_preview` and `guidance_apply` are today ([Slice 5c](../spike-log.md#slice-5c--unmediated-producers-unattributed-approval-and-the-guidance-note-installer-hap-001-d18)); the renderer never holds a generic filesystem capability.

   Text the user wrote is never rewritten, summarized or translated. The generated blocks are the only automatic additions.

4. **The loader table is versioned data.** One table records, per host, the instruction files it loads, how it walks nested and ancestor folders, what shadows what, and its sources with an access date and a verification status. It starts from the nine hosts the prototype documented: Claude Code, Codex, Gemini CLI, Antigravity, Qwen Code, OpenCode, Kimi, Cursor and Copilot. Omnifrons uses it for two things only:
   - warnings in previews: an ancestor definition shadowing a nested one, a missing wrapper, an `@` import hazard;
   - the switch check: before a model switch restarts a harness, the preview states whether the target host reaches the canonical file from the `ActiveProjectRoot`.

   A row is a claim about a host, never a guarantee. Omnifrons guarantees behaviour only through its built-in adapters, on its own evidence, and the table never extends the closed `AdapterId` set.

5. **The definition is portable configuration, never authority.** A definition is untrusted content, like any other repository file (TM-001 INJ-1, TM-001 INJ-4). It never authorizes execution, restores an approval or widens a permission ([Executables and harnesses](0002-desktop-technology-stack.md#executables-and-harnesses) ("Portable configuration cannot authorize execution"); target architecture, invariant 9). An executable that a definition or a skill names still passes the device-local launch gate (TM-001 HAR-3, TM-001 HAR-4; VP-001 VP-S14). The precedence rule is an instruction to the agent, not a sandbox; scope labels stay as the adapter declares them.

6. **New definitions come from an interview.** A folder with no definition gets one through an interview held in the user's language, one question per turn. Omnifrons then writes `AGENTS.md` in English from one fixed, versioned template. The preview shows the user's own words beside each generated line; those words never enter the file. Anything the user did not confirm is written under an `## Unconfirmed` heading. Creation writes only new files, `AGENTS.md` and the wrapper, and never overwrites one; a folder that already holds identity files is adapted instead.

7. **Memory, handoff and switching stay with the runtime.** Persistent memory, session handoff and model switching remain the runtime's ([Model and harness switching](../target-architecture.md#model-and-harness-switching); [Handoff transaction protocol](../handoff-transaction-protocol.md)). The definition feeds them as input (the startup brief can reference it by path and digest) and holds no memory or session state of its own.

### Terminology

"Logical agent" keeps its meaning: the Omnifrons-owned durable identity that carries one conversation, task and memory across harness and provider changes. The folder artifact is the **agent definition**: what a Logical agent runs. The [shared terminology](../target-architecture.md#shared-terminology) carries a proposed entry for it that points here.

### Invariants this proposal keeps

| Invariant | How this proposal keeps it |
| --- | --- |
| Typed IPC only ([Privilege and IPC boundary](0002-desktop-technology-stack.md#privilege-and-ipc-boundary); target architecture, invariant 4; VP-001 VP-S13) | Definition writes are typed commands over an application service; no generic filesystem capability reaches the renderer |
| Portable configuration cannot authorize execution (target architecture, invariant 9) | Decision 5: no approval restored, no permission widened, the launch gate unchanged |
| Guidance write discipline (HAP-001 D18) | Decisions 2 and 3: one sentinel family, the existing snapshot store, pinning, idempotent reapply, scoped removal |
| Closed adapter set (`crates/omnifrons-domain/src/adapter.rs`) | Decision 4: the loader table is data for warnings and adds no `AdapterId` |
| Proposed records bind nothing ([Authority](README.md#authority)) | Nothing here holds precedence over another document until accepted |

## Consequences

### Benefits

- One canonical file per definition: a switch between built-in adapters changes the harness, not the agent.
- The duplicate-name hazard the prototype found becomes a product-owned, hash-locked rule with a testable claim.
- The guidance module, its sentinel family and its D18 discipline are reused instead of being joined by a second installer.
- Every write is previewed before any byte moves, and every replaced file can be restored from a snapshot.
- Host differences live in data that can be rechecked and versioned, not in adapter code.
- A definition stays plain Markdown that any host can load without Omnifrons.

### Costs and limits

- Nested project roots must be modelled. The architecture names one `ActiveProjectRoot`, while hosts that walk upward also load ancestor definitions.
- The Python prototype is a specification, not a dependency. The port reimplements it in Rust, and its 126 tests are translated, not run.
- Agents the prototype adapted need the one-time block migration, each with its own consent.
- A newly added local skill stays out of the index until the next previewed apply; verify reports it as unindexed rather than indexing it silently.
- Hosts outside the built-in set get warnings, not guarantees, and a loader-table row ages with every host release.
- The precedence rule is an instruction. A host or a model can still disregard it; only evidence shows that it held.
- The wrapper relies on Claude Code's `@path` import and on its rule that a `CLAUDE.md` hides `AGENTS.md`. A change in either reopens the wrapper design.
- No third-party skill content is vendored. The prototype's imported skills carry Apache-2.0 and MIT notices; Omnifrons ships none of them, so it takes on neither their attribution duties nor their updates.

## Alternatives

### Vendor the prototype's Python tools

The fastest route to a working adapter, with 126 passing tests on day one. Rejected: it adds a Python runtime beside the Rust core, writes files outside typed IPC, and duplicates the guidance module's write discipline in a second codebase. The port keeps the tests as its specification instead.

### A separate agent-directory root

Keep every definition in one product-owned directory and point harnesses at it. One place lists every agent, and no project file is written. Rejected: harnesses load instruction files from their working directory and its ancestors, so a definition outside the `ActiveProjectRoot` is invisible to them unless it is copied or linked in. That reopens the unmodelled channel and adds a synchronization problem.

### One agent per workspace

Allow one definition per `WorkspaceRoot` and no nesting: nothing nested to model, nothing to shadow. Rejected: folder agents nest in practice (the prototype's own example agent contains a nested one), and forbidding nesting forces users to restructure folders while removing the shadowing warnings they most need.

### Guarantee all nine documented hosts

Promise behaviour wherever users run the folder. Rejected: Omnifrons does not launch those hosts, cannot observe how they load files, and cannot follow their releases. A guarantee without evidence would contradict truthful capability status. The nine hosts stay data for warnings.

### Built-in adapters only, with no loader table

The smallest surface, claiming only proven behaviour. Rejected: users run the same folder in other hosts, and without the table a preview cannot warn about ancestor shadowing or a missing wrapper. The switch check would also lack data for the built-in adapters' own harnesses, which are hosts in the same table.

### A second sentinel family (`portable-agent:`)

Keep the prototype's markers, so prototype-adapted agents need no migration. Rejected: one file would carry two block grammars, two parsers and two status models, with the D18 snapshot and removal discipline duplicated or bridged. A one-time migration costs less than a permanent second grammar.

### One merged block

A single Omnifrons block holding both the guidance note and the skills content: one sentinel pair per file. Rejected: the note's fixed, versioned template (HAP-001-R42) would be coupled to an index that changes whenever a skill is added, removing either would rewrite the other, and a hand edit to one would block updates to both.

## Acceptance evidence and follow-up

### Acceptance gate

The gate has four parts.

**1. A new VP scenario, numbered when the plan is amended.** It runs on the pinned Linux baseline; the other baselines follow as VP-001 pins them, before the capability is claimed there. The plan amendment that numbers it binds each built-in adapter to one named, user-installed harness, so the evidence has a defined collection procedure, and an `uncertain` run never discharges this gate. Its shape in the [scenario catalog](../desktop-stack-verification-plan.md#scenario-catalog):

| ID | Claim | OS | Procedure | Pass criterion | Evidence | Discharges |
| --- | --- | --- | --- | --- | --- | --- |
| (new) | One agent definition loads identically through each built-in adapter and survives a model switch | L first, then W/M | Create a definition with one indexed local skill and a same-named user-level skill; launch each built-in adapter's harness in the definition root; ask who the agent is, then give a task matching the skill; switch mid-task to a different built-in adapter and resume | Every run shows the canonical `AGENTS.md`, directly or through the wrapper, and reads only the indexed skill path; the resumed run carries the same definition digest; the digests of `AGENTS.md`, the wrapper and every `skills/**/SKILL.md` are unchanged after every run, while `.omnifrons/` and the outbox may change; a harness that yields no loaded-file evidence records `uncertain`, never `pass` | Tool trace or harness log per adapter; definition digests before and after; startup brief capture | This record's acceptance gate; [Pre-alpha → Exit criteria](../roadmap.md#exit-criteria) ("through a different built-in adapter") |

**2. The write path, verified on copies.** A copy of a prototype-adapted agent migrates from the `portable-agent:` block to the `skills` kind: instruction text outside the blocks stays byte-identical, a snapshot and its manifest are recorded, and the original stays untouched. Each refusal decision 3 commits to is asserted on its own fixture, as a structural file check at the level of the prototype's tests, with the fixture's bytes unchanged afterwards: apply with a stale plan id is refused; a `CLAUDE.md` and an `AGENTS.md` holding different text halt the plan; a symlinked or irregular identity file is refused; a nested definition without its own consent is refused; a hand-edited `portable-agent:` block is reported, not migrated. The same fixtures assert the positive side of decision 2: reapplying an unchanged plan is a no-op, a hand-edited `skills` block is reported as `block-modified`, and an unchanged file produces no new snapshot. Acceptance cannot pass with a refusal or one of these assertions unimplemented.

**3. The prototype's traces, admitted as external evidence with their stated boundary.** They support the design; they do not discharge the gate. The boundary is the prototype's own (`prototype/VALIDATION.md`): the 126 tests are structural file checks, not host behaviour; the host checks cover six hosts; and the record leaves two results open. The Codex check of unique local routing was contaminated by higher-priority user-level hooks, and OpenCode's logs cannot show whether an ancestor instruction file was also loaded.

**4. The self-approval, recorded.** The proposer and the named approver are the same person, constant1n0, and the acceptance record states that fact ([Approval matrix](README.md#approval-matrix); [Self-approval and conflict of interest](../governance.md#self-approval-and-conflict-of-interest)).

### Follow-up slices

Each slice is reviewable on its own and files its evidence under the project's practice ([VP-001 scenario records](../evidence/VP-001/records.md)). The prototype's 126 tests are the port's specification.

1. **Domain model and verify, read-only.** The definition, the wrapper, the skills index, nested roots and loader-table rows as domain types; verify reports state and problems and writes nothing.
2. **Adapt.** Dry-run, plan id and apply with snapshots; the `skills` kind; the `portable-agent:` migration; a crash-point convergence test matching the prototype's `test_apply_crash_points_converge`, so an interrupted multi-file apply leaves only a recognised intermediate state that a rerun converges from.
3. **Create.** The interview, the fixed English template and the `## Unconfirmed` section.
4. **The loader table in model switching.** The pre-restart check and the definition's reference in the startup brief.
5. **The VP scenario.** The plan amendment that numbers it, then the first Linux run.

### Open questions

- **The `@` import hazard.** Claude Code treats an `@path` at the start of a text token or after whitespace as an import, including right after a code span or a backslash escape (`adapter/portable-agent/references/HOSTS.md`, "Known limitation"). The prototype refuses to generate an index row whose skill name, description or path could hold such an import. Whether the port keeps that rule, and how it reports imports already present in text the user wrote, is open.
- **TOCTOU and crash windows.** The prototype re-hashes its inputs before each write, but an editor save landing between that re-hash and the atomic replace is overwritten; `portable_agent.py` records this as VER-B-003, a sub-millisecond window in one local measurement. Its multi-file apply is a sequence of atomic single-file writes, so a crash leaves a recognised intermediate state that a rerun converges from (`tests/test_portable_agent.py`, `test_apply_crash_points_converge`). Whether the port narrows the TOCTOU window or records it as a bounded residual is open; the crash-point convergence itself is required by slice 2.
- **Nested-root modelling.** How a nested definition relates to `WorkspaceRoot` and `ActiveProjectRoot`, which root owns the outbox and the guidance note when roots nest, and how ancestor exposure is shown to the user.
- **Loader-table freshness.** How often a row is rechecked against its sources, and how a preview shows a row whose access date predates the host version in use.
- **The harnesses behind the VP scenario.** A fixture harness shows what Omnifrons hands a harness, not what a vendor host loads, which is why the gate binds each built-in adapter to a named, user-installed harness. Which harness and version each adapter is bound to, and how a run records that version, is left to the plan amendment.
- **Independent review.** Whether acceptance requires an Independent Reviewer's advice on the write path, and whether TM-001 needs a row of its own for it.

## Related contracts

- [ADR convention](README.md): metadata, sections, approval matrix
- [ADR-0002: Desktop technology stack](0002-desktop-technology-stack.md): the typed IPC boundary; portable configuration cannot authorize execution
- [Target architecture](../target-architecture.md): shared terminology, model and harness switching, invariants 4 and 9
- [Roadmap](../roadmap.md): the Pre-alpha exit criterion on resuming through a different built-in adapter
- [Threat model](../threat-model.md) (TM-001): HAR-3, HAR-4, INJ-1, INJ-4
- [Heavy-asset publication contract](../heavy-asset-publication.md) (HAP-001): D18, HAP-001-R42, the agent guidance note
- [Desktop stack verification plan](../desktop-stack-verification-plan.md) (VP-001): VP-S13, VP-S14, the proposed scenario
- [Handoff transaction protocol](../handoff-transaction-protocol.md) (HTP-001): the handoff the definition feeds
- [Context Orb presentation specification](../context-orb.md): skills supplied by the scope's context root
- [Spike log](../spike-log.md): the guidance installer's typed commands (slice 5c)
- [Governance](../governance.md) (GOV-001): self-approval
- The `base_omnifrons` prototype (private repository, commit d2d0cb9): `adapter/portable-agent/scripts/portable_agent.py`, `adapter/portable-agent/references/HOSTS.md`, `adapter/new-agent/scripts/new_agent.py` and `prototype/VALIDATION.md`, cited by name because they are not linkable from this repository
