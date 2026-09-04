# Migration and Recovery Plan

**Document role:** Migration and recovery plan: upgrade graph, backups, delta recovery, tombstones, restore epochs (MRP-001)  
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

This document drafts the planned migration and recovery plan (MRP-001): the upgrade graph semantics per version domain and per plane, backup formats, cadence, verification, retention and encryption posture per store, delta recovery per plane or namespace through recovery points, tombstone formats and their propagation across profiles and backups, restore epochs — allocation, recording, propagation, concurrency, detection, and resurrection fencing — the artifact formats the [RSP-001 core](workspace-roaming-protocol.md)'s cutover checklist names (backup at step 3, tombstones at steps 6 and 8, restore epochs at step 9), binary retention for rollback, and the drills that constitute acceptance evidence. The [target architecture](target-architecture.md) governs any conflict.

This draft does not redefine work already owned elsewhere:

- **The RSP-001 core** ([workspace-roaming-protocol.md](workspace-roaming-protocol.md)) owns the Git Sync to Cloud cutover checklist and its commit point; this document owns only the artifact formats that checklist references, not its sequence or gates.
- **The memory synchronization profile** ([roaming-and-engram-sync.md](roaming-and-engram-sync.md)) owns sync profiles, watermarks, continuity, and posture. This document assumes a namespace's replication state exists and is observable; it never redefines how currency is computed.
- **The handoff transaction protocol** ([HTP-001](handoff-transaction-protocol.md)) owns the handoff lifecycle and the per-handoff state vector; this document's recovery point is a distinct, backup-scoped snapshot, never transmitted as a handoff.
- **The target architecture**'s invariant 9 and the **threat model** ([TM-001](threat-model.md)) own what may be restored as authority: approvals, executable profiles, device keys, and tokens are never restored from content. This document only backs up an inventory of what existed.
- **TM-001** and the memory profile own secrets custody; this document never backs up a secret and states only where the encryption keys for its own backups are held.
- **[context-orb.md](context-orb.md)** owns presentation. No recovery gadget exists there today; this document records that gap as an open decision rather than editing the specification.
- **The memory profile**'s Engram upstream tracking section owns Engram's own runtime behavior and version range; this document depends on it for whether a restore-epoch primitive exists under Cloud, and records that dependency rather than resolving it.

This document's requirements are new — `MRP-001-R1` through `MRP-001-R21` — and do not continue any other document's numbering. Unlike the RSP-001 core, which continues the memory profile's `RSP-001-R`/`D` sequence because it drafts the same named artifact, MRP-001 is its own artifact with its own registry entry in the target architecture, so its requirement and open-decision IDs start fresh at 1.

## Purpose and scope

MRP-001 governs **recovery from loss and disagreement about history**: what a backup must prove before it counts as protection, how a product release moves between versions without silently discarding user data, how one plane or namespace is restored independently of the others, how a deleted subject stays deleted across every store that might resurrect it, and how a restore is distinguished from ordinary new activity so no device mistakes an old copy for the latest one.

In scope:

- backup surfaces per plane: what is captured, in what format, and what is categorically excluded;
- the signed backup manifest, its verification record, encryption posture, cadence, and retention;
- the upgrade graph: supported edges per version domain, downgrade rules, and the migration journal;
- delta recovery: restoring one plane or namespace through a recovery point, independent of the others;
- tombstone format and its propagation across sync profiles and backups;
- restore epochs: allocation, recording, propagation through `sync.state`, concurrency, detection, and resurrection fencing;
- binary retention for rollback;
- the drills that constitute acceptance evidence for this plan.

Out of scope:

- the cutover checklist and its commit point — owned by the [RSP-001 core](workspace-roaming-protocol.md);
- sync profiles, watermarks, continuity, and posture — owned by the [memory synchronization profile](roaming-and-engram-sync.md);
- the handoff lifecycle — owned by [HTP-001](handoff-transaction-protocol.md);
- what may be restored as authority — owned by the target architecture's invariant 9 and [TM-001](threat-model.md);
- secrets custody — owned by TM-001 and the memory profile;
- presentation — owned by [context-orb.md](context-orb.md);
- Engram's own runtime behavior — owned by the memory profile's Engram upstream tracking section.

This document is itself named evidence for the roadmap's Alpha → Beta promotion gate, alongside the threat model, renderer security review, and update trust — none of the other gate items substitute for it, and it does not substitute for them.

## Problem statement

A device fails. Its owner restores last week's memory export onto a new device and keeps working. A second device, which had replicated up to yesterday, later pushes yesterday's observations into the same namespace. Without a restore epoch, the restored namespace silently absorbs a mix of last week and yesterday, and no one — not the user, not a teammate reading the same namespace later — can say which history the team is actually on.

A second failure has a different shape but the same root. An upgrade migrates the checkpoint envelope schema. A laptop that skipped the upgrade later publishes an envelope in the old schema; the new schema's reader on a workstation cannot read it. The workstation, downgraded to diagnose the mismatch, then finds its own already-migrated data unreadable by the older binary it just reinstalled. Neither device did anything wrong by its own lights; nothing recorded which direction each store had actually moved.

Neither failure is a bug in Git, in Engram, or in the compatibility policy's own migration machinery. Each mechanism did what its own contract promises: the export restored cleanly, the push replicated, the migration ran. The gap is that nothing in the product distinguished a restored history from an unbroken one, or recorded which direction a store had moved across an upgrade boundary — the same seam this document exists to close.

**A restore is a new restore epoch, never a rewind; a backup is evidence only once it has been restored somewhere; and nothing recovered from content restores authority.**

## Definitions

| Term | Meaning |
| --- | --- |
| Backup | A point-in-time copy of one plane's supported portable surface, captured under this document's manifest and verification rules. |
| Verified backup | A backup whose digest check has passed and whose most recent restore drill succeeded. Only a verified backup counts as protection. |
| Uncertain backup | A backup whose digest check or restore drill has not yet passed, or has failed. Never counted toward retention or protection claims. |
| Re-establishment list | The inventory a backup carries of subjects that existed but are never restored as authority — approvals, executable profiles, device pairings, keys, tokens. Its schema is deliberately thin: kind, display name, last-approved time, and count — never a path, a fingerprint, identity evidence, or a secret reference. A device re-establishes each listed subject locally rather than the backup silently granting it. |
| Recovery point | An explicit tuple of per-plane watermarks or revisions — workspace base/handoff commit, knowledge artifact revisions, delivery revision, and memory watermark per namespace — recorded at backup time. Distinct from HTP-001's state vector: a state vector accompanies one handoff transaction; a recovery point accompanies one backup or restore operation, stored in the backup manifest and never transmitted as a handoff. |
| Migration journal entry | The record of one applied migration: version pair, domain, UTC time, and outcome. Migrations are deterministic, idempotent after interruption, and resumable; the journal is what makes idempotency verifiable rather than assumed. |
| Binary retention | Keeping the previous product binary installed and launchable until the new version has a verified backup, so a failed upgrade has an immediate way back that does not depend on a reinstall. |
| Tombstone (MRP-001) | The record that a subject — a namespace, an observation, an artifact, or a handoff — was deleted, carried through backups and restores so a restore never resurrects it. Distinct from the RSP-001 core's namespace tombstone, which is this format's `kind: namespace` case, pushed to the memory repository manifest at cutover step 6; this document defines the format all four kinds share. |
| Tombstone kind `artifact` | Deletion of a knowledge-plane artifact or a heavy-asset-tier object; owning plane: knowledge (ADR-0003). |
| Tombstone kind `handoff` | HTP-001's own retained envelope, tombstoned once its recovery window's cleanup runs (HTP-001-R11); owning plane: runtime state, riding the workspace bundle. |
| Restore-epoch log | The permanent, never-pruned record of every restore epoch this document allocates: restore epoch, restore point, seed digest (where a seed device is involved), and UTC time. Distinct from a per-subject tombstone, which is bounded by the tombstone retention default (D6) and can be pruned — the log is what lets fencing by a restore epoch (R15) hold even after the tombstone that explains a subject's absence is gone. |
| Restore epoch | The monotonic marker recorded on a namespace or plane's restored store, identifying which restore generation its content belongs to. Always written in full in this document, never shortened to "epoch," so it is never confused with the RSP-001 core's writer epoch — a different mechanism, scoped to a different resource: who may currently write, versus which restore generation a store's content belongs to. |
| Recovery drill | A scheduled or triggered restore of a verified backup into a scratch location, exercised to prove the backup is actually restorable rather than merely digest-intact. |
| Downgrade | Reinstalling a lower product version over a higher one. Supported only within the same major version and only when no irreversible migration has run since the target version. |
| Irreversible migration | A migration a release's notes declare cannot be undone by downgrading; its presence blocks downgrade past it regardless of the downgrade window otherwise in force. |

## Principles

- **Backups are evidence, not promises.** A backup counts as protection only after digest verification and a restore drill; before that it is `uncertain`.
- **A restore allocates a new restore epoch and is announced, never hidden.** Every restored store carries a higher restore epoch than it did before, observable through `sync.state`.
- **Per-plane independence, cross-plane consistency through a recovery point.** Restoring the memory plane does not require restoring the workspace, and vice versa; when planes end up at different points after a partial restore, the inconsistency is reported, never absorbed silently.
- **Nothing restored from content restores authority.** Approvals, executable profiles, device pairings, keys, and tokens are re-established on the device; a backup carries only the re-establishment list of what existed (target architecture invariant 9; TM-001).
- **Portable Git history is never rewritten to simulate rollback** (compatibility policy, Rollback and recovery qualification).

## Backup surfaces per plane

A backup surface is defined per plane because each plane already has its own authority and its own transport (target architecture, invariant 7); there is no single "back up everything" operation, only six independently scheduled and independently verified surfaces.

| Plane | Authority | What is backed up | Format | Never backed up |
| --- | --- | --- | --- | --- |
| Workspace | Git remotes (primary durability) | Omnifrons-managed refs (`refs/omnifrons/writers\|forks\|handoffs/...`) | Git bundle per logical agent | User branches (the user's own hosting responsibility); uncommitted working-tree changes, captured only by handoff/fork mechanics under the RSP-001 core, never by MRP-001 |
| Knowledge | Markdown vault | The complete vault | File archive plus hash manifest | Nothing excluded by design; heavy-asset-tier content is a separate, best-effort backup (D10) |
| Delivery | Rides the workspace Git transport | OpenSpec content | Carried inside the workspace bundle | — |
| Memory | One namespace authority per the memory profile | Git Sync: the memory repository's chunks, append-only and durable only as far as the hosting retains them — hosting durability is the operator's, not this document's; Cloud: the device-side supported export/import surface | Git bundle (Git Sync); Engram export file (Cloud, device-side) | The live SQLite database, under either profile (invariant 3); under Cloud, the server-side backup is the operator's, not this document's |
| Runtime state | Managed refs (claim, fork, handoff envelopes) | Carried inside the workspace bundle | Git bundle | Approvals, executable profiles, device pairings, keys — inventory only, in the re-establishment list |
| Configuration | Device-local product configuration | Configuration minus secrets; the migration journal | Structured export | Every secret, under every custody class |

The migration journal (Upgrade graph) is itself a configuration-plane backup surface, not a per-migration artifact of the store it describes: every entry is captured in every backup's manifest regardless of which plane triggered the backup, and it is written to storage outside the store any given migration is modifying — so a failed migration never corrupts the record of its own failure alongside the data it was migrating.

The memory-plane backup inherits the memory profile's `<private>` stripping and the secret-detection heuristic's own caveat: detection cannot prove absence (RSP-001 D6; TM-001 SEC-3). A memory backup is therefore not claimed exhaustively secret-free — only exhaustively free of the specific fields and patterns those upstream mechanisms already strip or flag.

## Backup manifest, verification, encryption, cadence, and retention

**Manifest.** Every backup carries a signed manifest:

| Field | Meaning |
| --- | --- |
| Device id | The device that captured the backup |
| Captured at | UTC time the backup was taken |
| Product version | The release that produced the backup |
| Per-plane entries | One per backed-up plane, each with its digest and format version |
| Recovery point | The tuple defined in Delta recovery and recovery points, as of capture time |
| Envelope/schema versions | The checkpoint/handoff and sync-state/envelope versions in force at capture time |
| Signature | Over the canonical serialization of the fields above |

**Verification.** A backup gets a verification record only after two checks pass: a digest check against the manifest, and a periodic restore drill into a scratch location, at the cadence in force (D4). The verification record itself carries the digest-check outcome, the drill outcome, the scratch-location identity, and the UTC time of each. A backup that has not passed both checks is `uncertain` and is never counted as protection — the same rule the target architecture states for every unverified assumption (invariant 2).

A drill's pass criteria are plane-specific — a digest match alone is not a pass:

| Plane | Drill pass criteria |
| --- | --- |
| Workspace | Bundle verification passes, and the bundle fetches cleanly into a scratch repository with every managed ref present |
| Knowledge | The archive extracts cleanly and its contents match the manifest hash |
| Memory | The export imports into a scratch runtime store, with observation count and a sample of identifiers matching the source |
| Configuration | The export parses and its schema version is recognized |

**Encryption.** A backup that leaves the device is encrypted with a user-held key (custody per D3). A local-only backup relies on the device's own full-disk encryption, carrying the same warning TM-001 states for an undetected posture (TM-001 SEC-5, D6): the product cannot verify full-disk encryption is actually enabled, and a stolen local backup is only as protected as the disk it sits on.

**Key loss.** Losing both the OS-keychain-held key and the recovery passphrase (D3) makes every off-device backup encrypted under that custody unrecoverable — there is no third path back into it. Omnifrons MUST disclose this plainly at initial setup and at every passphrase creation or rotation, not as a one-time notice. The restore drill (D4) MUST include a passphrase-decrypt step, so key loss is caught by the drill itself rather than discovered during an actual restore. A verified local backup under the device's own full-disk encryption remains the fallback once off-device custody is lost.

**Cadence and retention.** Defaults, both open decisions (D1, D2): a daily local backup of the knowledge vault and the memory export; a mandatory backup immediately before every upgrade and every cutover, regardless of the standing schedule; retention of 30 days plus every pre-upgrade and pre-cutover backup, kept until the new state has its own first verified backup.

A restore drill proves more than digest integrity: it proves the backup can actually be read back by the product version that will need it, catching format drift or partial-write corruption a digest check alone cannot see. A digest match with a failed drill is still `uncertain` — the drill outcome, not the digest alone, gates whether the backup counts as protection.

## Upgrade graph

Nodes are product versions; edges are the directed migrations a release supports, per the compatibility policy's Migration and upgrade graph section. Direct `N-1 -> N` is always supported; a larger version jump is supported only through the listed intermediate migrations that release publishes.

Each of the compatibility policy's version domains — portable configuration, the reserved workspace namespace, the checkpoint/handoff envelope, the sync state/envelope, adapter/event interoperability, persisted interaction preferences — declares its own migrations per release; a product release is not one migration but a set, one per affected domain.

Every migration is deterministic, idempotent after interruption, resumable, and journaled. A migration journal entry records:

| Field | Meaning |
| --- | --- |
| Version pair | The source and target versions the migration moves between |
| Domain | Which version domain the migration applies to (portable configuration, workspace namespace, checkpoint/handoff envelope, sync state/envelope, adapter/event interoperability, persisted interaction preferences) |
| Applied at | UTC time the migration ran |
| Outcome | `succeeded`, `failed`, or `resumed` |

An interrupted migration re-runs to the same result rather than half-applying twice; the journal is what makes that idempotency verifiable rather than assumed.

**Mid-migration marker.** While a migration is in progress, the store being migrated carries a marker naming the migration explicitly: source version X, target version Y. Any binary other than Y that attempts to open a store carrying that marker MUST refuse and report `failed`, offering exactly two recovery options — resume the migration with version Y, or restore the pre-upgrade backup. A store never appears silently readable to a binary that did not perform its migration.

**Downgrade** is supported only within the same major version and only when no irreversible migration ran since the version being downgraded to; an irreversible migration is declared as such in its release's notes. A pre-upgrade backup is mandatory before every migration. The previous binary is retained, installed and launchable, until the new version has its own verified backup — closing the window where a failed upgrade has nowhere to go back to.

Applied to the problem statement's second scenario: the checkpoint-envelope migration that changed the schema is an irreversible migration once a device has written the new schema, so the workstation's downgrade is blocked outright rather than allowed to run against data it cannot read. The laptop that skipped the upgrade is not silently accepted either — its old-schema envelope is rejected by the new reader per the compatibility policy's envelope negotiation, and the mandatory pre-upgrade backup gives the laptop a supported path forward once it does upgrade.

## Delta recovery and recovery points

A restore targets one plane, or one namespace within the memory plane, without requiring the others to restore alongside it. Point-in-time recovery is expressed through a recovery point rather than one global snapshot, because the planes have independent authorities and independent transports (target architecture, invariant 7):

| Plane | Recovery-point field |
| --- | --- |
| Workspace | Base commit and handoff commit |
| Knowledge | Canonical Markdown artifact revisions and hashes |
| Delivery | OpenSpec content revision |
| Memory (per namespace) | Chunk digest set under Git Sync; acknowledged sequence under Cloud |

A restore preview MUST show the recovery point being restored against the plane's current state before the restore applies, following the same preview-before-destructive-action discipline the compatibility policy states for snapshot rollback: destructive recovery requires preview and explicit consent.

After a restore, the restored plane or namespace carries a new restore epoch. When a partial restore leaves planes at different points — memory restored to last week while the workspace stays current — the resulting cross-plane state is reported as `stale` or `uncertain` on the affected scope, never silently treated as consistent.

A handoff whose state vector references memory watermarks newer than the plane's restore point cannot be claimed. This is not a new check: HTP-001's complete-vector validation (HTP-001-R4) and the memory profile's watermark gate (RSP-001-R9) already fail such a claim on their own terms, comparing the receiver's watermark against the state vector's — a restored plane whose watermark has moved backward simply never clears that gate. MRP-001 does not redefine either check; it only names the consequence in this document's own vocabulary: the handoff renders `uncertain` until the plane's watermark is re-validated against the now-restored authority.

## Tombstones

**Format.**

| Field | Meaning |
| --- | --- |
| Id | Unique identifier for the tombstone record |
| Kind | `namespace` \| `observation` \| `artifact` \| `handoff` |
| Subject id | The deleted subject's own identifier |
| Restore epoch | The restore epoch in force on the owning plane when the tombstone was written |
| Written at | UTC time |
| Reason | Free-text or coded deletion reason |
| Signature | Over the canonical serialization of the fields above |

**Kinds.**

| Kind | Owning plane | Meaning |
| --- | --- | --- |
| `namespace` | Memory | Engram's own namespace-level soft-delete; also the RSP-001 core's namespace tombstone at cutover step 6 |
| `observation` | Memory | Engram's own observation-level soft-delete |
| `artifact` | Knowledge | Deletion of a knowledge-plane artifact or a heavy-asset-tier object (ADR-0003) |
| `handoff` | Runtime state (workspace) | HTP-001's own retained envelope, tombstoned once its recovery window's cleanup runs (HTP-001-R11) |

Engram's own soft-delete is the memory plane's tombstone at the `observation` and `namespace` kinds; this document carries it through backups and restores rather than defining a separate mechanism for memory. The RSP-001 core's namespace tombstone — pushed to the memory repository manifest at cutover step 6 — is this format's `kind: namespace` case; the two never diverge in shape.

**Propagation.** Git Sync: a manifest or chunk entry recording the tombstone alongside ordinary content. Cloud: a delete mutation the server applies and later devices observe. Retention is at least as long as any device might plausibly resurrect the subject — default 180 days (D6).

A restore never resurrects a tombstoned subject, and neither does an ordinary publication that merely arrives late: the restore process checks the tombstone set for the restored recovery point's window and excludes any subject it names, even if the backup being restored predates the tombstone, and a device publishing stale content after the tombstone's own retention window (D6) has elapsed is still fenced by the restore epoch it fails to meet (Restore-epoch log, below) — the guarantee does not expire when the tombstone itself is pruned.

## Restore epochs

**Scope.** Per namespace, and per plane more generally where a plane supports independent restore.

**Allocation and recording.** A restore epoch is allocated when a restore is applied, and recorded in the restored store's own metadata — never inferred from a timestamp or a device's local belief.

**Restore-epoch log.** Every restore-epoch allocation is also appended to a permanent, never-pruned restore-epoch log: restore epoch, restore point, seed digest (recorded whenever a seed device is involved — a Cloud restore or an RSP-001-core cutover alike), and UTC time. The log is distinct from a per-subject tombstone, which is bounded by the tombstone retention default (D6) and can be pruned. A device that returns after a tombstone's retention window has elapsed, carrying pre-deletion content, is fenced by the restore epoch it fails to meet (R15) regardless of whether the specific tombstone that would have named its content is still retained — the log, never expiring, is what makes that fencing hold past D6. Content fenced this way renders `forked` and requires human review; it is never silently resurrected.

**Concurrency.** Restore-epoch allocation MUST use a compare-and-set against the recording store's exact expected value, so two concurrent restores of the same namespace cannot both win. Under Git Sync, the restore-epoch entry is written to the memory repository manifest under the same lease precondition the RSP-001 core uses for its own managed refs (its R29 pattern): the write succeeds only against the manifest ref's exact prior tip. Of two racing restores, exactly one wins; the loser, on re-reading the ref, observes the newer restore epoch and its own restore renders `conflicted`, pending human resolution — never silently retried by overwriting the winner. Two restore-epoch entries naming the *same* restore epoch with *different* content is a distinct condition — the same `authority-conflict` shape the RSP-001 core applies to its own claim ref — treated as a possible integrity incident rather than routine contention (Restore-epoch conflicts, below).

Under Cloud, the memory runtime exposes no compare-and-set primitive for a restore (Encoding per profile, D9), so concurrent restores of one namespace are not safe and MUST be serialized by the human operator: one restore in flight at a time, confirmed complete before the next begins. To make a wrong or compromised seed detectable after the fact even without a runtime lease, the restore-epoch log records the seed device's export digest alongside the restore epoch, restore point, and UTC time — cross-referencing TM-001's A5 (remote-service insider or compromise) and A8 (compromised paired peer device) actor entries, which name exactly this class of after-the-fact detection need.

**Restore-epoch conflicts.**

| Condition | Resulting state |
| --- | --- |
| Compare-and-set precondition failed on restore-epoch allocation — a concurrent restore already won | `conflicted`, pending human resolution |
| Two restore-epoch entries name the same restore epoch with different content | `authority-conflict`; remote integrity suspected |

**Announcement.** The restore epoch is announced through the affected plane's `sync.state` (AEC-001 feed profile), the same event the memory profile and the RSP-001 core already use to carry authority state for their planes; MRP-001 adds the restore-epoch field to that event rather than defining a new one.

**Re-reconciliation.** A device that observes a restore epoch higher than the one it knows MUST re-reconcile from the restored authority. Its own local unpublished changes made since the restore point become `forked` — never merged silently into the restored state, the same discipline the RSP-001 core applies to a superseded writer epoch.

**Resurrection fencing.** A publication carrying a restore epoch lower than the one currently recorded is rejected as `forked` — whether that publication is the direct output of an explicit restore or an ordinary stale republication from a device that merely missed the restore announcement: a device that missed a restore and keeps publishing against the old generation cannot silently overwrite the restored state with stale content, and the rejection does not depend on whether the tombstone that would explain the content's absence is still within its retention window.

**Encoding per profile.**

| Profile | Encoding | Gap |
| --- | --- | --- |
| Git Sync | Carried in the memory repository's manifest, alongside the tombstone entries it accompanies | None; the manifest is already the shared authority for chunks and tombstones |
| Cloud | Carried by the namespace tombstone plus a re-seed of the namespace's content | The memory runtime exposes no restore-epoch primitive today; tracked under the memory profile's Engram upstream tracking section (D9) |
| Workspace remote | Proposed generation record, not yet part of the RSP-001 core | The workspace Git remote has no restore-epoch or generation primitive of its own yet (D7) |

**Workspace remote.** The workspace's Git remote has no restore-epoch primitive of its own yet — the RSP-001 core names this gap as its own D18. This document proposes an answer rather than editing that document directly: a generation record on a managed ref `refs/omnifrons/generation/<logical-agent-id>`, bumped on any administrative restore of the remote, so a claim record written against a pre-restore generation is distinguishable from one written after (D7).

Applied to the problem statement's first scenario: the new device's restored memory namespace carries the restore epoch allocated at restore time. When the second device — still on its own, higher local sequence — later pushes yesterday's observations, its publication carries the pre-restore epoch and is rejected as `forked` rather than silently merged; the user is shown both the restored namespace and the fork, and resolves them explicitly, the same recovery discipline the RSP-001 core already applies to a superseded writer epoch.

## Recovery presentation (gap)

No recovery gadget exists in the [Context Orb specification](context-orb.md) today; its Omnifrons-native gadget roster lists Checkpoint/handoff and Sync health, and neither shows a recovery point, a verified-backup age, or a restore epoch. This document proposes a "Recovery" section in the Ops surface, showing:

- the current recovery point per plane;
- the age of the last verified backup per plane;
- the restore epoch per namespace;
- any pending re-reconciliation after a restore epoch bump.

This is an open decision (D8) rather than an edit to context-orb.md, which remains that specification's own owner; the gadget itself would obey the same projection invariants — a read/status view of the authority defined here, never itself the authority (target architecture, invariant 10).

## Timeouts and thresholds

Nothing in this plan hardcodes a timing constant. Every cadence, retention window, and drill frequency below is an open decision with a default proposal, not a fixed value: backup cadence (D1), retention window (D2), restore drill frequency (D4), and tombstone retention (D6). A restore epoch itself never expires and is never timed out — like the RSP-001 core's writer epoch, it is superseded only by a further restore, never by a clock.

## Product requirements

Each requirement is an acceptance gate with a testable condition.

| ID | Requirement |
| --- | --- |
| MRP-001-R1 | A backup MUST cover only the surfaces listed in Backup surfaces per plane, and MUST NOT include a live SQLite database, a secret under any custody class, an approval, an executable profile, a device pairing, or a key or token. |
| MRP-001-R2 | Every backup MUST carry a signed manifest with device id, UTC time, product version, per-plane digest and format version, recovery point, and envelope/schema versions. |
| MRP-001-R3 | A backup MUST NOT be counted as protection until it holds a verification record: a passed digest check and a passed restore drill into a scratch location. Absent either, the backup MUST render `uncertain`. |
| MRP-001-R4 | A backup that leaves the device MUST be encrypted with a user-held key. A local-only backup MUST carry the undetected-full-disk-encryption warning TM-001 defines (SEC-5, D6) when that state cannot be verified. |
| MRP-001-R5 | Omnifrons MUST take a backup immediately before every upgrade and before every RSP-001-core cutover, regardless of the standing cadence, and MUST NOT start the migration or cutover until that backup holds a verification record — a passed digest check and a passed restore drill for every affected plane. An unverified pre-upgrade or pre-cutover backup blocks the migration; digest passage alone is never sufficient. |
| MRP-001-R6 | Omnifrons MUST retain backups per the retention policy in force (D2) and MUST NOT prune a pre-upgrade or pre-cutover backup before the resulting new state has its own first verified backup. |
| MRP-001-R7 | Every migration MUST be deterministic, idempotent after interruption, resumable, and recorded as a migration journal entry with version pair, domain, UTC time, and outcome. |
| MRP-001-R8 | A downgrade MUST be blocked outside the same major version, and MUST be blocked within it whenever an irreversible migration has run since the target version, unless the release notes declare that specific migration reversible. |
| MRP-001-R9 | The previous binary MUST remain installed and launchable until the newly upgraded version has its own verified backup. |
| MRP-001-R10 | A restore MUST target one plane or one namespace independently of the others, through an explicit recovery point, and a partial restore that leaves planes at different points MUST render the affected scope `stale` or `uncertain`, never silently consistent. |
| MRP-001-R11 | Omnifrons MUST carry Engram's soft-delete as a tombstone in this document's format through every backup and restore, alongside the `namespace`, `artifact`, and `handoff` kinds. |
| MRP-001-R12 | A tombstone MUST propagate per profile — a manifest/chunk entry under Git Sync, a delete mutation under Cloud — and MUST be retained at least as long as the retention default in force (D6). A restore MUST NOT resurrect a subject named by a tombstone within the restored recovery point's window, and this guarantee MUST extend to ordinary stale republication after the tombstone's own retention window has elapsed: the restore-epoch log, never pruned, is what makes the guarantee hold past D6. |
| MRP-001-R13 | A restore MUST allocate a new restore epoch through a compare-and-set against the recording store's exact expected value — under Git Sync, the memory repository manifest ref's exact prior tip, per the RSP-001-core lease pattern (RSP-001-R29) — record it in the restored store's own metadata and in the restore-epoch log, and announce it through the affected plane's `sync.state`. Under Cloud, where no such primitive exists, concurrent restores of one namespace MUST be serialized by the human operator rather than allowed to race. |
| MRP-001-R14 | A device that observes a restore epoch higher than the one it knows MUST re-reconcile from the restored authority, and MUST render its own unpublished changes since the restore point `forked` rather than merging them silently. |
| MRP-001-R15 | A publication carrying a restore epoch lower than the one currently recorded MUST be rejected and rendered `forked`, whether that publication is the output of an explicit restore or an ordinary stale republication from a device that merely missed the restore announcement — the rejection MUST NOT depend on whether the originating tombstone, if any, is still within its retention window. |
| MRP-001-R16 | A backup and restore MUST NOT restore approvals, executable profiles, device pairings, keys, or tokens as authority; a backup MAY carry a re-establishment list naming what existed, limited to kind, display name, last-approved time, and count, and MUST NOT include a path, a fingerprint, identity evidence, or a secret reference, for the device to re-establish locally. |
| MRP-001-R17 | Every release's notes MUST state its migrations, any irreversible migration, and known recovery limits, per the compatibility policy's changelog obligations. |
| MRP-001-R18 | A restore MUST show the user a preview of the recovery point being restored against the plane's current state, and MUST require explicit consent before applying — a destructive recovery action MUST NOT proceed on preview alone. |
| MRP-001-R19 | A heavy-asset-tier backup MAY be attempted best-effort and MUST NOT block a knowledge-plane or memory-plane backup on its own failure. |
| MRP-001-R20 | Omnifrons MUST disclose, at initial setup and at every passphrase creation or rotation, that losing both the OS-keychain-held key and the recovery passphrase makes every off-device backup encrypted under that custody unrecoverable. The restore drill (D4) MUST include a passphrase-decrypt step. A verified local backup under the device's own full-disk encryption MUST remain available as the fallback once off-device custody is lost. |
| MRP-001-R21 | A store mid-migration MUST carry a marker naming its source and target versions. A binary other than the target version MUST refuse to open a marked store, MUST render `failed`, and MUST offer exactly two recovery options: resume the migration with the target version, or restore the pre-upgrade backup. |

## Signal mapping

The public vocabulary column uses the states in the [compatibility policy](versioning-and-compatibility.md) plus the target architecture's required failure states.

| Condition | MRP-001 state | Public vocabulary | Consequence |
| --- | --- | --- | --- |
| Backup digest check or restore drill not yet passed | `uncertain-backup` | `uncertain` | Never counted as protection |
| Restore epoch observed higher than locally known | `stale-restore` | `stale` | Re-reconciliation from the restored authority required |
| Local changes made after the restore point, before re-reconciliation | `forked` | `forked` | Preserved locally; never merged silently |
| Publication — explicit restore or ordinary stale republication — carrying a restore epoch lower than current | `forked` | `forked` | Rejected; resurrection fencing, independent of tombstone retention |
| Restore-epoch compare-and-set precondition failed (a concurrent restore already won) | `conflicted` | `conflicted` | Restore blocked pending human resolution |
| Two restore-epoch entries name the same restore epoch with different content | `authority-conflict` | `authority-conflict` | Restore blocked; remote integrity suspected |
| Missing pre-upgrade or pre-cutover backup | `blocked-upgrade` | `failed` | Upgrade or cutover blocked with reason |
| Downgrade requested past an irreversible migration | `blocked-downgrade` | `failed` | Downgrade blocked with reason |
| Tombstoned subject present in a restored backup | `tombstone-skip` | — | Excluded from restore and reported, never resurrected |
| Partial restore leaves planes at different recovery points | `partial-restore` | `uncertain` | Shown only on the affected scope |
| Handoff state vector references memory watermarks newer than the restore point | `unclaimable` | `uncertain` | Cannot be claimed until re-validated (HTP-001-R4; RSP-001-R9) |

## Open decisions

| ID | Question | Options | Default proposal |
| --- | --- | --- | --- |
| D1 | Backup cadence | Daily; user-configurable interval; on significant change only | Daily local backup of the knowledge vault and the memory export |
| D2 | Retention window | Fixed 30 days; user-configurable; unlimited pre-upgrade/pre-cutover retention | 30 days, plus every pre-upgrade and pre-cutover backup kept until the new state's first verified backup |
| D3 | Encryption key custody for off-device backups | OS-keychain-derived key; user passphrase; both, with a recovery path | OS keychain, with a recovery passphrase as fallback |
| D4 | Restore drill frequency | Monthly; on every backup; on every pre-upgrade backup only; monthly plus on every pre-upgrade backup | Monthly, plus on every pre-upgrade backup, completed before the upgrade starts |
| D5 | Downgrade window | Same major version only; any prior supported version | Same major version only |
| D6 | Tombstone retention | 90 days; 180 days; indefinite | 180 days |
| D7 | Workspace remote generation record | Add `refs/omnifrons/generation/<logical-agent-id>`, bumped on administrative restore; leave unresolved, rely on human review (RSP-001 core D18) | Proposed here as the answer to RSP-001 core D18: add the generation record |
| D8 | Recovery UI home | A "Recovery" section in Ops; fold into the existing Sync health gadget; no dedicated surface yet | A dedicated "Recovery" section in Ops (context-orb.md remains the owner of the specification itself) |
| D9 | Cloud restore epoch, until the Engram runtime exposes one | Tombstone plus re-seed (this document's proposal); block Cloud restore until upstream support lands | Tombstone plus re-seed, tracked under the memory profile's Engram upstream tracking section |
| D10 | Heavy-asset-tier backup | Required, same as knowledge; best-effort, listed but not guaranteed | Best-effort: catalogued and attempted, never blocking on failure |

## Acceptance evidence and follow-up

- Conformance tests MUST cover: every plane's backup surface and its forbidden payloads; manifest signing and every declared field; the verification record gating `uncertain` versus counted protection; encryption for off-device backups and the local full-disk-encryption warning; mandatory pre-upgrade and pre-cutover backups; every declared upgrade-graph edge, including a multi-step jump through its published intermediates; migration idempotency after interruption; the downgrade rule and its irreversible-migration block; binary retention until the new version's verified backup; delta recovery of one plane or namespace independent of the others; every tombstone kind, its propagation per profile, and its retention; restore-epoch allocation, announcement through `sync.state`, re-reconciliation, and resurrection fencing; and the re-establishment list never restoring approvals, executable profiles, pairings, keys, or tokens.
- Reproducing the problem statement's first scenario — a device restored from an old export, another device pushing newer content into the same namespace — MUST render the newer publication `forked` against the restore epoch the restoring device allocated, never silently absorbed.
- Reproducing the second scenario — a skipped envelope-schema upgrade, then a downgrade to diagnose — MUST render the mismatch as a blocked downgrade (an irreversible migration already ran) rather than a corrupted read.
- A destructive-recovery test MUST verify that no restore applies without an explicit preview-and-consent step (MRP-001-R18), and a heavy-asset-tier backup failure test MUST verify that it never blocks the knowledge- or memory-plane backup it accompanies (MRP-001-R19).
- A concurrent-restore test MUST verify that of two racing restore-epoch allocations exactly one wins the compare-and-set, the loser renders `conflicted`, and two restore-epoch entries naming the same epoch with different content render `authority-conflict` (MRP-001-R13).
- A post-retention resurrection test MUST verify that a device returning after a tombstone's retention window has elapsed, carrying pre-deletion content, is still fenced by the restore-epoch log and renders `forked` rather than silently resurrected (MRP-001-R12, R15).
- A key-loss disclosure test MUST verify that both setup and every passphrase creation or rotation show the key-loss warning, and that the restore drill includes a passphrase-decrypt step (MRP-001-R20).
- A verified-pre-upgrade-backup gate test MUST verify that a migration cannot start until its pre-upgrade backup holds a passed digest check and a passed restore drill for every affected plane (MRP-001-R5).
- A mid-migration-marker test MUST verify that a binary other than the migration's target version refuses to open a marked store and offers exactly the two named recovery options (MRP-001-R21).
- Debt, not drafted here. The Cloud restore epoch (D9) depends on Engram exposing a supported primitive this draft was not checked against; until then the tombstone-plus-re-seed proposal is unverified in practice. The workspace remote generation record (D7) is a proposal answering the RSP-001 core's own D18 and requires that document's acceptance to take effect. `sync.state`'s restore-epoch field is additive to AEC-001's feed profile and is not yet reflected there. The recovery presentation gap (D8) requires context-orb.md's own acceptance to add a gadget; this document only proposes the content it would show.

## Related contracts

- [Target architecture](target-architecture.md) — invariants 2, 3, 7, and 9; required failure states; the MRP-001 row in planned assurance artifacts.
- [Versioning and compatibility](versioning-and-compatibility.md) — version domains, the migration and upgrade graph, rollback and recovery qualification, and changelog obligations this document implements.
- [Workspace roaming protocol](workspace-roaming-protocol.md) (RSP-001 core) — the Git Sync to Cloud cutover checklist that names the backup, tombstone, and restore-epoch artifacts this document formats; its own open decision D18, answered here as a proposal (D7).
- [Workspace roaming and Engram sync protocol](roaming-and-engram-sync.md) — sync profiles, watermarks, and the Engram upstream tracking section this document's Cloud restore-epoch gap depends on.
- [Handoff transaction protocol](handoff-transaction-protocol.md) (HTP-001) — the state vector this document's recovery point is deliberately distinct from.
- [Threat model](threat-model.md) (TM-001) — the stolen-device warning (SEC-5, D6) this document's local-backup encryption posture cites; invariant 9's restored-authority boundary; the secret-detection "cannot prove absence" caveat (SEC-3) the memory-plane backup inherits; and actor entries A5/A8, cross-referenced by the restore-epoch log's seed-digest recording.
- [Adapter feed event schema](adapter-feed-events.md) (AEC-001) — the `sync.state` event this document's restore-epoch announcement extends.
- [Update trust architecture](update-trust-architecture.md) (UTA-001) — decides whether a binary is trusted; MRP-001 decides when it may migrate; drafted, acceptance pending.
- [ADR-0003: Local Markdown and tiered heavy assets](adr/0003-local-markdown-and-tiered-assets.md) — the knowledge vault's always-local guarantee and the heavy-asset tier this document's best-effort backup (D10) covers.
- [ADR-0002: Desktop technology stack](adr/0002-desktop-technology-stack.md) — Git executed through explicit argv and isolated temporary state, the mechanism workspace backups rely on.
- [Product roadmap](roadmap.md) — the Alpha stage's backup and migration-fixture scope items, the Beta stage's controlled-migration and guided-recovery scope items, and the Alpha → Beta gate that names this plan directly as required evidence.
- [Context Orb specification](context-orb.md) — presentation of the states this document names; the recovery-gadget gap recorded in D8.
- [Governance](governance.md) (GOV-001) — roles, approvals, exceptions, and evidence retention this document's acceptance relies on; drafted, acceptance pending.

## References

- [Keep a Changelog](https://keepachangelog.com/) — the release-notes categories MRP-001-R17's migration and recovery disclosures follow.
- [Semantic Versioning](https://semver.org/) — the version-graph nodes this document's upgrade edges connect.
- [Git bundle documentation](https://git-scm.com/docs/git-bundle) — the workspace and memory-repository backup format referenced in Backup surfaces per plane.
- [Target architecture](target-architecture.md) — invariants 2, 3, 7, and 9, and the required failure states this document's signal mapping reuses.
- [Versioning and compatibility](versioning-and-compatibility.md) — the migration and upgrade graph section this document drafts.
