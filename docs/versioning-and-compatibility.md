# Versioning and Compatibility Policy

**Document role:** Public compatibility, migration, deprecation, and support promises  
**Status:** Draft  
**Normative force:** Non-binding until accepted for a release  
**Accountable role:** Compatibility Owner, initially the Project Maintainer  
**Named person:** Unassigned  
**Approver role:** Project Owner  
**Approver named person:** Unassigned  
**Effective date:** None  
**Supersedes:** None  
**Name status:** Selected public name; preliminary screening complete, formal trademark clearance pending

## Authority

The [target architecture](target-architecture.md) owns boundaries and terminology. The [roadmap](roadmap.md) owns maturity and sequencing. ADRs own technology and licensing decisions. This policy owns only declared public compatibility behavior. Detailed protocols remain planned assurance artifacts in the target architecture; this draft does not claim they exist or are implemented.

## Product release versioning

Omnifrons uses [Semantic Versioning](https://semver.org/).

- `MAJOR`: an established public contract breaks.
- `MINOR`: backward-compatible capability or deprecation.
- `PATCH`: backward-compatible correction or maintenance.
- Prerelease identifiers describe a build, not product maturity by themselves.

The maturity/channel/SemVer mapping is canonical in the roadmap. Pre-1.0 change freedom never permits silent data loss or reinterpretation.

## Version domains

A product release contains several independently versioned domains.

| Domain | Canonical contract / artifact | Scheme | Compatibility/negotiation |
| --- | --- | --- | --- |
| Product distribution | Release process | SemVer | Update metadata selects a compatible release path. |
| Portable configuration | Compatibility policy | Monotonic schema integer | Preflight and migrate before use; unknown required fields block. |
| Reserved workspace namespace | Compatibility policy | Layout schema integer | Migrations operate only inside `.omnifrons/` unless separately approved. |
| Checkpoint/handoff envelope | Planned handoff protocol | Envelope major/minor | Reject unknown major; preserve unknown optional minor fields. |
| Sync state/envelope | Planned roaming protocol | Protocol major/minor | Both sides negotiate; unknown major blocks claim. |
| Built-in adapter/event interoperability | Planned adapter/event contract | Internal contract major/minor plus capabilities | Core and built-in adapter negotiate; stable 1.0 behavior is not a public extension SDK. |
| Persisted interaction preferences | Compatibility policy and planned voice contract | Schema integer | Unknown required fields block; safe defaults only for documented optional fields. |
| External Git, Engram, harness, OS, and WebView versions | Support matrix | Tested ranges | Range plus capability probe; detection alone grants no support. |

Schema changes do not automatically change product `MAJOR`. A product major is required when no supported migration can preserve a public contract.

## Public 1.0 surfaces

### Portable configuration

Documented keys, types, defaults, validation, precedence, unknown-key behavior, and secret-reference semantics are public. Portable configuration is secret-free. A secret reference is device-local and may resolve to `unprovisioned` after roaming; it never authorizes execution on another device.

### Workspace layout

Omnifrons reserves the complete `.omnifrons/` namespace at 1.0. Public metadata defines which content is portable, generated, local, reconstructible, or user-editable.

Compatible releases may add paths only inside the reserved namespace. Every addition preflights collisions; an existing unexpected path blocks migration until the user chooses quarantine, rename, or an approved migration. Omnifrons never silently reinterprets user content.

### Knowledge and asset locality

Every Markdown file in the canonical knowledge vault is a normal local file and is non-evictable. A startup or handoff preflight blocks when a Markdown file is unreadable, represented only by a provider placeholder, or located in the evictable heavy-asset tier.

Heavy assets such as PDFs, media, archives, datasets, and large generated artifacts live in a separate blob tier. The user selects local or on-demand behavior. The Context Catalog preserves logical identity, provenance, relationships, availability, and integrity metadata so the Orb can render cloud-only assets without pretending their bytes are local.

Hydration and eviction are explicit state transitions. A node becomes local only after download and integrity verification; eviction never targets Markdown. Provider pin flags may be checked as diagnostics, but they are not the source of truth for the Markdown guarantee.

### Checkpoint and handoff

The public envelope covers logical identity, task, active project, predecessor, portable-work commit, state vector, lifecycle state, provenance, and schema/integrity metadata. It transfers explicit work context, not hidden reasoning or vendor-private sessions.

Automatic cross-device claim is not a 1.0 guarantee until the planned handoff and roaming protocols define and pass authenticity, replay, quiescence, publication-barrier, conflict, and recovery gates.

### Synchronization states

Public states include at least `local`, `prepared`, `publication-pending`, `published-unverified`, `claimable`, `claimed`, `stale`, `forked`, `conflicted`, `offline`, `uncertain`, `aborted`, and `failed`. User-facing wording must preserve these distinctions.

`Workspace Git roaming` and the `Engram Git Sync profile` are different mechanisms. Each data plane has one observed active authority. Switching Engram Git Sync to Engram Cloud remains unsupported automation until the planned roaming and migration protocols define a quiesced, reversible cutover; the UI must block or label any manual change as externally managed and unverified.

### Built-in harness behavior

At 1.0, documented built-in adapters provide stable capability names, normalized behavior, checkpoint interoperability, error categories, and degraded-mode disclosure for declared harness ranges.

The internal `HarnessAdapter` and event contract is versioned for first-party interoperability, but **is not a public extension API or SDK**. A public adapter SDK is post-1.0 and evidence-gated in the roadmap.

### Interaction, privacy, and fallback

Stable text behavior covers input ownership, approval visibility, cancellation, error state, and recovery. Stable voice behavior, when enabled, covers:

- explicit microphone consent and persistent capture/transmission indication;
- an immediate stop control;
- documented local versus remote processing and retention choice;
- no restoration of prior approvals from transcription or synchronized content;
- complete text fallback when voice is denied, unavailable, or fails;
- persisted voice preferences and their migration.

Recognition quality, voice availability, latency, and provider-specific retention remain external characteristics unless the support matrix explicitly promises them. The planned voice contract owns the detailed protocol and tests.

### Persisted UI state

Only documented preferences, project references, recoverable drafts, and accessibility behavior are public. Visual styling and ephemeral renderer state are private.

No public Omnifrons CLI or headless automation contract is promised for 1.0. A narrowly scoped recovery command becomes public only if a later roadmap/ADR defines its commands, output, security boundary, and tests.

## Private and reconstructible state

Internal domain types, private desktop IPC, renderer implementation, local database tables, caches, indexes, traces, and logs may change when documented behavior remains intact and reconstructible state can be safely rebuilt. This language is framework-neutral; ADR-0002 remains Proposed.

## Pre-1.0 and release channels

| Channel | State rule | Compatibility expectation |
| --- | --- | --- |
| `nightly` | Isolated disposable profile or verified copy only | Migration may be absent; it must not open or mutate stable state without a recoverable snapshot. |
| `alpha` | Backed-up technical-pilot state | Breaking changes allowed with explicit migration or clean export/recovery. |
| `beta` | Recoverable pilot state | Upgrade and rollback evidence targets the intended 1.0 graph. |
| `stable` | Supported state | Full accepted policy and support matrix apply. |

A less stable channel requires explicit action. “Incompatible” never means permission to destroy the only copy of user data.

## Support matrix

Each release pins exact test evidence and separately states minimum supported versions for OS/build, architecture, WebView/runtime, packaging substrate, assistive technology, Git, Engram, optional Engram Cloud, and every built-in harness/transport.

| Classification | Promise |
| --- | --- |
| `supported` | Tested behavior, migration, recovery, and support apply for the declared capability set. |
| `preview` | Explicit opt-in; data-preservation rules apply, but behavior/API stability and full support do not. |
| `detected` | Omnifrons can identify it but does not authorize launch or claim compatibility. |
| `unsupported` | Launch or migration is blocked unless an explicit unsafe diagnostic mode exists. |

The planned desktop verification plan records exact build numbers, architectures, runtime versions, assistive technology, and test dates. “Latest” is not reproducible evidence.

### Engram runtime compatibility

Each Omnifrons release declares the Engram version range it supports; the range is a support-matrix promise, not a detection result. Behavior verified against a specific Engram version is labelled with that version in the release evidence, and the [memory synchronization profile](roaming-and-engram-sync.md) records the version its statements were checked against. A newer Engram release is `detected`, not `supported`, until that profile is re-validated against it; memory continuity read through an unvalidated release is reported as `uncertain`. A breaking upstream change — a removed or changed CLI/API surface, a new mutation state, or a schema change behind a supported surface — requires a compatibility note in the release notes under support changes. Version-dependent Engram behavior is isolated behind one adapter so that a range change never spreads through product code.

## Migration and upgrade graph

Every release publishes a directed graph of accepted source-state versions and whether each edge is direct or stepped.

Default stable policy:

- a patch accepts every state produced by supported patches in the same minor line;
- a new 1.x minor supports direct migration from the two immediately preceding supported minor lines unless its release record requires a tested stepped path;
- older or EOL state requires a published stepped path or an export/recovery workflow;
- release artifacts and matching migration tooling remain available for every supported node and for at least the deprecation window after EOL.

Migrations are deterministic, preflighted, idempotent after interruption, and backed up when the old version cannot read the new state. Unknown user data is preserved or blocks.

Automatic snapshot rollback is allowed only when Omnifrons proves that **no authoritative post-upgrade mutation** occurred in any local or replicated authority. Authoritative mutation includes user or harness writes, synchronization/import, background processing, recovery actions, and metadata changes that alter portable state.

Every authoritative write after the upgrade commit is journaled or exported into a recovery delta, including remotely received or background mutations. If any such write exists—or the absence of writes cannot be proven—Omnifrons stops snapshot rollback and requires delta-preserving reconciliation. Destructive recovery requires preview and explicit consent.

The planned migration/recovery artifact ([migration-and-recovery-plan](migration-and-recovery-plan.md), MRP-001) owns the artifact formats the cutover checklist references (the checklist itself is the RSP-001 core's), backup verification, binary retention, tombstones, restore epochs, and Engram authority migration.

## Update trust

A valid package signature alone is insufficient. Automatic beta or stable updates remain blocked until the planned update-trust artifact defines roots and online roles, freshness/version metadata, anti-rollback, rotation, revocation, compromise response, platform signing/notarization, and key-loss recovery.

## Deprecation and emergency security action

A public surface is deprecated for at least two minor releases and six months, whichever is longer, unless an active risk requires earlier action. Notices identify replacement, migration, earliest removal, and recovery.

An emergency cannot disguise a contract break. Permanent removal requires a major release. A compatible release may temporarily disable or revoke a dangerous capability only through a separately documented security mechanism that states scope, user impact, recovery, restoration criteria, and advisory reference.

## Rollback and recovery qualification

Each supported release qualifies:

- every declared upgrade-graph edge;
- interruption before and after commit;
- snapshot rollback only with proof of zero authoritative local or replicated post-upgrade mutations;
- journal/export and delta-preserving recovery after any authoritative mutation, including sync/import, background, and recovery actions;
- cache/index reconstruction;
- failed update and unavailable-key behavior;
- all supported OS families.

The application never rewrites portable Git history to simulate rollback.

## Changelog and governance

Release notes follow [Keep a Changelog](https://keepachangelog.com/) categories and foreground breaking pre-1.0 changes, schemas, migrations, support changes, security behavior, manual actions, and known recovery limits.

Adding/removing a 1.0 public surface, shortening support, or breaking compatibility requires the approval record defined by the ADR convention and, for a break, a major-version ADR.
