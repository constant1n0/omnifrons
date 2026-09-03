# Omnifrons Target Architecture

**Document role:** Product boundary, system invariants, shared terminology, and component responsibilities  
**Status:** Draft  
**Normative force:** Non-binding target direction; requirements are acceptance gates, not current guarantees  
**Accountable role:** Project Maintainer  
**Named person:** Unassigned  
**Approver role:** Project Owner  
**Approver named person:** Unassigned  
**Effective date:** None  
**Supersedes:** None  
**Name status:** Selected public name; preliminary screening complete, formal trademark clearance pending

## Document authority

This draft does not silently supersede earlier architecture, components, visual design, or OpenClaw-specific handover documents. Each prior claim must be marked current, amended, historical, deprecated, or superseded. An unresolved conflict blocks implementation of that behavior.

| Artifact | Owns |
| --- | --- |
| Target architecture | Boundaries, invariants, shared terminology, component responsibility |
| Roadmap | Maturity sequence, scope, evidence, and promotion |
| Compatibility policy | Public surfaces, versions, migration, deprecation, and support |
| ADRs | One technology/legal decision's rationale, alternatives, status, and evidence |
| Planned protocol/security/verification artifacts | Executable semantics and acceptance tests for their named concern |
| Product naming note | Selected name, clearance process, and third-party mark rules |

No current draft has binding precedence. After acceptance, documents govern only their owned scope; overlap requires explicit reconciliation rather than a generic “latest file wins” rule.

## Product boundary

Omnifrons is a cross-platform human-facing facade over harnesses the user installs and authenticates independently. It provides one logical agent with a consistent conversation, project, task, memory, and control surface while the harness or provider may change.

Omnifrons owns interaction, logical identity, routing, supervision, normalized state, checkpoint/handoff coordination, recovery, and truthful capability status.

It does not own model inference, provider accounts, hidden reasoning, private vendor sessions, harness credentials, an autonomous agent runtime, source-control history, canonical notes, or Engram database internals.

## Shared terminology

- **Workspace Git roaming:** Git history/conflict transport for portable workspace content.
- **Engram Git Sync profile:** Engram's supported Git-based memory export/import mode.
- **Engram Cloud profile:** Optional alternative memory synchronization authority.
- **Logical agent:** Omnifrons-owned durable identity; never a vendor session ID.
- **WorkspaceRoot:** Registered parent for projects and portable Omnifrons state.
- **ActiveProjectRoot:** Selected project and default harness `cwd`/requested scope.
- **Scope mode:** `sandbox-enforced`, `harness-enforced`, or `advisory`.
- **Handoff commit:** Disposable Omnifrons-managed Git commit that transports approved in-progress file state without changing user history.
- **Checkpoint:** Portable task/context envelope; it references but does not replace authoritative artifacts.
- **Claim:** Verified transition making one handoff current on a receiving device. Publication alone is not a claim.
- **Startup brief:** The bounded facts-and-references package handed to a harness as startup context after a switch or claim — goal, active project, task state, validations, unresolved risks, and artifact references drawn from the checkpoint. Always untrusted input: it cannot restore approvals, executable profiles, secret references, or permission elevation, and side effects require fresh approval.
- **Context Catalog:** Metadata registry for the heavy-asset tier: logical identity, relationships, provenance, remote locator, availability, size, and integrity metadata for every asset, local or remote-only. It lets the Orb and diagnostics keep the complete graph visible without pretending remote bytes are local, and it is never the content authority (ADR-0003).
- **Context Orb (Orb):** The dashboard's central visual: a read/status projection of the knowledge, memory, delivery, and catalog planes showing the active model, skills, memory, context, routines, and applications. Never a persistence, synchronization, or conflict-resolution authority. Presentation is specified in the [Context Orb presentation specification](context-orb.md).

## Proposed invariants

1. Human approval remains authoritative for provider choice, risky actions, portable-work selection, and conflict resolution.
2. Portable state is versioned, validated, secret-free by contract, and recoverable; current implementations must expose any unverified assumption.
3. Live databases, credentials, caches, indexes, process state, and live `.git` directories are never roaming payloads.
4. The renderer has no generic shell or unrestricted filesystem capability.
5. `ActiveProjectRoot` is always the requested scope, but it is a security boundary only in `sandbox-enforced` mode.
6. Structured harness transports are preferred; PTY is an explicit degraded fallback.
7. Each data plane has one **observed** active sync authority; conflicting external configuration blocks handoff.
8. Pending, stale, forked, conflicted, uncertain, partial, and orphan-risk states are never shown as complete.
9. Synchronized content is untrusted data and cannot restore approvals or authorize execution.
10. The Orb is a projection of authoritative sources; it is never itself a persistence, synchronization, or conflict-resolution authority.
11. Every Markdown file is always local and non-evictable; cloud-only content is restricted to the separate heavy-asset tier.
12. A remote-only Orb node becomes local only after explicit hydration and integrity verification.

## Components and trust boundaries

```text
Human
  -> untrusted renderer
  -> typed IPC
  -> application coordinators
  -> framework-independent domain
  -> bounded ports
       -> ProcessSupervisor -> user-installed harnesses/providers
       -> GitCoordinator -> installed Git
       -> MemoryCoordinator -> supported Engram CLI/MCP
       -> KnowledgePort -> always-local Markdown / optional Obsidian capability
       -> BlobStorePort -> local or on-demand heavy assets
       -> DeliveryPort -> OpenSpec
       -> SecretStore -> OS credential service
```

The proposed Tauri/Rust/React implementation is conditional on ADR-0002. No framework choice changes the trust model.

Application coordinators own session, harness routing, checkpoint, workspace, memory, Git, recovery, and voice workflows. Provider-specific parsing remains inside adapters. Machine-local storage is reconstructible unless the compatibility policy explicitly makes a format portable.

## Harness integration

### Executable approval

Synchronized configuration may describe a harness but cannot approve it. Approval is device-local and binds canonical path plus available file identity/signature/digest, version, adapter, transport, plugin inventory, and security-relevant configuration. Material change or a failed pre-launch re-probe requires renewed approval.

### Scope modes

- `sandbox-enforced`: an OS sandbox or equivalent verified mechanism enforces declared filesystem/network/process limits.
- `harness-enforced`: a supported native harness control enforces the declared limit; Omnifrons reports its dependency and tested version.
- `advisory`: Omnifrons sets `cwd` and instructions but the harness retains normal user permissions.

PTY is advisory unless independently sandboxed. Approval mediation covers only operations visible through the verified adapter; the UI must not imply control over hidden harness/plugin actions.

### Transport and events

Preference order is documented machine protocol, structured streaming CLI, then PTY.

The planned adapter/event contract must define capabilities, epochs, sequence ownership, per-transport ordering, gap detection, deduplication, acknowledgements, replay retention, overflow, cancellation, and terminal states. Initial semantics are at-least-once with idempotent consumption. Independent stdout/stderr streams have no promised total order, and exactly-once delivery is not claimed.

PTY bytes and escape/OSC sequences are untrusted active content. AEC-001 must normalize only allowlisted terminal controls into typed actions, render unsupported sequences inert or drop them with diagnostics, and never treat raw terminal bytes as authorization. Clipboard, hyperlink/navigation, title, notification, and file-transfer actions require the RCS-001 policy described below.

## Model and harness switching

A model change is a checkpoint-and-restart handoff, not a transparent transfer of a vendor session.

1. If work is active, the outgoing adapter requests a bounded summary and Omnifrons adds deterministic evidence: current goal, active project, changed files, task state, validations, unresolved risks, and artifact references.
2. The adapter performs its harness-specific graceful-stop operation. `/exit` is only one possible implementation and is never hard-coded as a universal protocol.
3. Omnifrons starts the selected, user-installed harness in the same `ActiveProjectRoot` after device-local approval and capability probing.
4. The new harness receives the checkpoint as startup context and independently validates the referenced workspace state.

If quiescence, shutdown, checkpoint integrity, or workspace consistency cannot be proven, the switch is `uncertain` and requires human review.

## Workspace and artifact access

Portable references use logical artifact IDs and workspace-relative identity where possible. A receiving device must resolve the project unambiguously before launch.

Knowledge, OpenSpec, or metadata outside `ActiveProjectRoot` is supplied through mediated, bounded, read-only retrieval. A harness receives selected content plus provenance, not an unrestricted raw path. Direct access outside the project requires separate itemized elevation.

Canonical paths resolve symlinks/junctions before Omnifrons authorization. In advisory mode this validates Omnifrons operations, not arbitrary harness behavior.

## Initial portable-work contract

This is the selected pre-alpha contract for in-progress files.

### Preconditions and preview

1. The active project is a supported Git worktree with a resolvable `HEAD`.
2. Merge, rebase, cherry-pick, bisect, conflicted index, or unsupported worktree state blocks automatic handoff.
3. The harness is quiesced. If the adapter cannot prove zero in-flight mutation, Omnifrons stops it before final inventory; if state can still change externally, handoff remains `uncertain` and cannot auto-claim.
4. Omnifrons reads eligible source bytes into isolated temporary Git state, records tracked content/deletions, staging differences for explanation, approved-candidate untracked files, and every exclusion/block. It computes each candidate path's Git mode, size/type, object/content digest, and relevant symlink/LFS metadata, then builds the immutable candidate Git tree and a canonical per-file manifest **before approval**.
5. The human previews the exact base-to-candidate diff, candidate tree hash, manifest digest, per-file/content digests, selected paths, deletions, sizes/types, symlinks, binary/LFS and submodule/nested-repository status, secret warnings, and every exclusion.
6. Approval binds the handoff ID, base commit, immutable candidate tree hash, and manifest digest. For tracked paths, the candidate contains worktree bytes read at candidate-build time; the user's index is recorded but never mutated.

A later source change does not alter the approved tree. It remains local and is reported as post-candidate drift. Including later bytes requires building a new candidate tree and obtaining a new preview and approval; publication never substitutes current worktree content for approved content.

### Initial exclusions and blocks

- Ignored files are excluded and cannot be opted in until a dedicated secret/data policy supports them.
- Known credential locations and detected high-risk secret material block inclusion. Detection cannot prove absence; residual risk is disclosed before approval.
- Special files, devices, sockets, and unsupported permissions/metadata are excluded.
- Symlink target content is never followed implicitly. Links escaping `ActiveProjectRoot` are blocked; an internal target must be independently tracked/selected.
- Nested repositories are blocked unless independently checkpointed.
- Dirty submodules block. A clean submodule pointer is allowed only when the referenced commit is verified reachable by the receiver's configured transport.
- Binary or oversized files block unless their type and configured size limit are explicitly supported.
- LFS content blocks unless Git LFS, filter execution, object upload, and receiver availability are all verified.
- Execution-capable Git filters/hooks/configuration block the snapshot unless the planned threat model and explicit approval cover that exact operation.

### Commit creation and publication

Before approval, Omnifrons uses isolated temporary index/state derived from the base commit to write the candidate objects/tree and canonical manifest without creating or moving a ref. After approval, it revalidates the approved tree and manifest digests and creates a commit parented to the base that references **exactly that immutable tree**—without rereading the worktree. Any digest mismatch blocks publication and requires a new candidate/approval. It updates only:

```text
refs/omnifrons/handoffs/<logical-agent-id>/<handoff-id>
```

It does not checkout another branch, change `HEAD`, alter the user's index, run user hooks, reset, merge, force-push, or rewrite user history.

The exact managed ref is pushed explicitly to the configured Git remote. A consumer file-sync provider may carry ordinary workspace files, but Omnifrons never synchronizes a live `.git` directory through it or treats provider upload as Git convergence.

The handoff commit is disposable and may later be cherry-picked, squashed, or replaced by the user's normal commit. Omnifrons never presents it as a required permanent branch.

### Receive, conflict, recovery, and cleanup

The receiver fetches the exact managed ref and verifies project identity, base, commit, manifest, and handoff state vector. It previews application before changing a worktree.

If histories diverge, the receiving worktree is dirty, or clean application is unproven, Omnifrons opens/uses an isolated recovery worktree where supported or blocks for human resolution. It never force-resets, auto-resolves semantic conflict, or overwrites either side.

Managed refs are listed, resumable, and pruned only after claim/abort confirmation and a documented recovery window. Cleanup deletes only Omnifrons-managed refs/temp state; it never deletes user branches or work. A failed publish retains the local ref and manifest for retry. A failed receive retains the fetched commit and conflict evidence.

## Handoff transaction

The portable-work commit is one durability component, not an atomic handoff.

The planned handoff transaction protocol must define:

- a unique monotonic handoff ID within a writer epoch and predecessor checkpoint;
- immutable state vector: work commit, canonical artifact revisions/hashes, Engram authority plus supported chunk digest/acknowledged watermark, schemas, and provenance;
- lifecycle `prepared -> publication-pending -> published-unverified -> claimable -> claimed`, plus `aborted` and `uncertain`;
- quiesce proof or stop-before-capture behavior;
- publication barriers across Git, Markdown/OpenSpec, and Engram;
- retry, idempotency, supersession, cleanup, and conflict recovery.

A receiver can claim only after validating the complete vector. Mutable links alone are insufficient.

Hashes prove integrity, not authorship. The protocol and threat model must define trusted device/project provenance, authenticity, generation/replay rules, and rollback detection. Until accepted and implemented, cross-device publication remains `published-unverified` and requires human review; automatic claim is blocked.

A startup brief contains bounded facts and references. Synchronized Markdown, memories, transcripts, and briefs are treated as untrusted input. They cannot restore approvals, executable profiles, secret references, or permission elevation; side effects require fresh approval.

## Roaming and synchronization

### Writer coordination

Single-writer operation is a product constraint, not a distributed lock. The planned roaming protocol must define writer epochs, compare-and-set claim records, fencing, divergence detection, and recovery.

Offline mutation without a verified current claim enters `forked/unverified`. It may be preserved locally but cannot be published as current automatically. Omnifrons never silently applies last-writer-wins.

### Engram authority

Local Engram remains required; its live SQLite database remains device-local.

Each memory namespace selects one profile: Engram Git Sync or Engram Cloud. Before sync/handoff, Omnifrons must inspect the actual supported Engram runtime/autosync status. If observed configuration conflicts with the selected profile, operation blocks as `authority-conflict`. A setting inside Omnifrons alone cannot enforce exclusivity.

Before enrollment, a dry-run shows the effective project and personal observation inclusion set. Engram `scope` is not treated as a privacy boundary. Local-only memories use a separate non-enrolled namespace. User approval records the inventory and residual risk.

Engram Cloud credentials remain device-local. Custody is labelled `OS-secret-store`, `Engram-managed-local`, or `unprovisioned`; token-bearing configuration is never copied into portable state.

Automated switching between Engram Git Sync and Engram Cloud is unsupported until the planned roaming and migration artifacts define quiescence, final watermark, backup, stable mutation IDs, reconciliation, fencing of the old authority, initial sync, commit point, tombstones, restore epochs, Git-history erasure limits, and resurrection conflicts.

## Knowledge, memory, and delivery

| Plane | Authority | Purpose |
| --- | --- | --- |
| Knowledge | Obsidian-compatible Markdown vault | Canonical long-form notes and human navigation |
| Operational memory | [Engram](https://github.com/Gentleman-Programming/engram) | Curated observations and pointers, not full-vault duplication |
| Delivery/specification | OpenSpec | Normative requirements, design, tasks, verification, and change lifecycle |
| Projection | Orb | Read/status projection only |

[`obsidian-skills`](https://github.com/kepano/obsidian-skills) supplies format and tool procedures, while the vault organization follows [Karpathy's LLM-Wiki pattern](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f). Omnifrons owns the knowledge lifecycle and write policy. Engram-to-Obsidian output is isolated generated content and never overwrites canonical human notes.

### Local Markdown and tiered heavy assets

Every Markdown note is a normal local file in the Git-backed, Obsidian-compatible vault. Markdown is non-evictable. Onboarding, startup, and handoff diagnostics block when a Markdown file is unreadable or represented only by a cloud placeholder. Provider-specific “available offline” flags may be verified as a fallback, but they do not define the invariant.

PDFs, media, archives, datasets, and other large objects live in a separate heavy-asset tier. The user chooses local or on-demand policy. A Context Catalog stores logical identity, relationships, provenance, availability, size, and integrity metadata. The Orb therefore keeps the full graph visible: local nodes use their active color; remote-only nodes are grey. Opening a remote-only node starts an explicit download, and its state changes to local only after integrity verification.

Git is the supported transport for Markdown and small portable state. On Linux, the initial supported heavy-blob backend is the official [Proton Drive CLI](https://proton.me/support/drive-cli), used for explicit list/upload/download operations and JSON output. It is not treated as a mounted Files-On-Demand filesystem or a background synchronization engine. User-operated `rsync` remains an external, unsupported workflow with no Omnifrons convergence or recovery guarantee.

Other provider adapters may use native placeholder and hydration APIs on Windows or macOS, but every adapter must implement the same catalog, hydration, integrity, failure, and eviction contract. No consumer provider becomes the authority for Markdown.

## Content, secrets, update, and Git trust

- Renderer output defaults to plain text. Rich content waits for the planned renderer-security contract covering sanitization, CSP, navigation, external links, attachments, and downloads.
- PTY output is untrusted active content. The terminal parser allowlists required control sequences; OSC clipboard and file-transfer actions are disabled by default; links use constrained providers/schemes and explicit confirmation; title/notification actions are sanitized; terminal scrollback and export pass through the same redaction and safe-serialization boundary as other support content.
- Redaction covers known structured fields and tracked values; arbitrary external output may leak transformed secrets. Raw vendor output is excluded from support bundles by default.
- Harness credentials remain harness-owned. Omnifrons secrets use OS credential storage. Engram-owned local token storage is detected and labelled, not silently copied or claimed secure.
- Git commands use explicit argv and bounded environment. Fetch/inspect is preferred to implicit pull. Hooks, filters, signing programs, helpers, fsmonitor, and other execution-capable configuration require the threat-model policy and specific consent or block.
- Automatic beta/stable update remains blocked until update trust covers root/online roles, freshness, anti-rollback, key rotation/revocation, compromise, platform signing, and recovery.
- Portable content never authorizes execution.

## Required failure states

| Condition | Required state/behavior |
| --- | --- |
| Unsupported/changed executable | Block; show identity difference and require approval |
| Scope not enforceable | Label advisory; remove containment claims |
| Quiescence unproven | Stop-first or uncertain; block automatic switch |
| Git publish/receive conflict | Preserve both sides; block claim |
| Missing vector component | Published-unverified or stale; never current |
| Writer claim absent/diverged | Forked/unverified |
| Engram authority mismatch | Authority-conflict |
| Personal-memory inclusion unknown | Block enrollment pending dry-run approval |
| Secret unavailable on device | Unprovisioned-secret |
| Process descendants unproven stopped | Orphan-risk/uncertain |
| Event gap/overflow | Degraded with replay/restart action |
| Checkpoint authenticity/replay unproven | Unverified; fresh human review |
| Renderer/voice unavailable | Preserve full text and recovery control |
| Migration/update trust unavailable | Block automatic mutation; retain recovery evidence |

## Planned assurance artifacts

These are placeholders, not existing files or implemented guarantees.

| ID | Planned artifact | Intended ownership |
| --- | --- | --- |
| HTP-001 | Handoff transaction protocol | Saga, state vector, authenticity, replay, claim, cleanup; the lifecycle, state vector, claim, authenticity, and cleanup protocol is drafted in [handoff-transaction-protocol](handoff-transaction-protocol.md) |
| RSP-001 | Workspace roaming and Engram sync protocol | Epochs, fencing, authority detection, privacy inventory, cutover; the memory synchronization profile (Git Sync and Cloud continuity, watermarks, and product requirements) is drafted in [roaming-and-engram-sync](roaming-and-engram-sync.md) |
| TM-001 | Threat model | Harness, Git, remote content, secrets, process, prompt injection; the attacker model, trust boundaries, Git classification policy, and acceptance evidence are drafted in [threat-model](threat-model.md) |
| UTA-001 | Update trust architecture | Keys, metadata, freshness, anti-rollback, compromise recovery |
| MRP-001 | Migration and recovery plan | Upgrade graph, backups, delta recovery, tombstones, restore |
| GOV-001 | Governance and support matrix | Named roles, approvals, exceptions, evidence, supported combinations |
| AEC-001 | Adapter and event contract | Capabilities, ordering, at-least-once replay, overflow, terminal states, and PTY byte/control normalization into allowlisted typed actions; the feed profile (event catalog and approvals write path) is drafted in [adapter-feed-events](adapter-feed-events.md) |
| RCS-001 | Renderer content-security contract | Plain/rich/terminal content, CSP, constrained URLs/navigation, OSC clipboard/link/file actions, attachments/downloads, and redacted scrollback/support exports |
| VOC-001 | Voice interaction contract | Consent, visibility, processing, retention, accessibility, text fallback |
| VP-001 | Desktop stack verification plan | Pinned multi-OS executable evidence for ADR-0002 |

A feature cannot claim the guarantee owned by a placeholder until its artifact is accepted and its tests pass.

## Explicit non-goals

- Reimplementing an agent runtime or hosting models.
- Transferring hidden reasoning or private vendor sessions.
- Syncing live Engram SQLite or live `.git` directories.
- Treating `cwd`, PTY, or UI approvals as a sandbox.
- Making Orb/cache/generated exports canonical.
- Automatic semantic conflict resolution or multiwriter operation.
- Requiring Engram Cloud, a running Obsidian app, or one consumer sync provider.
- Public adapter SDK or public headless CLI in the current 1.0 scope.

## Reconciliation plan

Inventory every existing design document, compare claims by owned scope, preserve valid visual intent, replace generic-wrapper/OpenClaw runtime assumptions, separate the four knowledge planes, and record explicit status/supersession links. No artifact disappears merely because the target changed; the reasoning trail remains reviewable.
