# Workspace Roaming Protocol

**Document role:** Workspace roaming protocol: writer epochs, claim records, fencing, divergence, recovery, and the Git Sync to Cloud cutover (RSP-001 core)  
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

This document drafts the **RSP-001 core**: writer epochs, claim records, compare-and-set, fencing, divergence detection, recovery, and the Git Sync to Cloud cutover for memory namespaces. It is the piece the [memory synchronization profile](roaming-and-engram-sync.md) explicitly left to "RSP-001 itself" — workspace writer coordination and the cutover are not redrafted there. The [target architecture](target-architecture.md) governs any conflict; its "Roaming and synchronization" section is this document's seed, the same relationship the handoff transaction protocol has to its own seed section.

Requirement and open-decision numbering continues the profile's. This document's requirements are RSP-001-R15 through RSP-001-R29; RSP-001-R1 through RSP-001-R14 remain defined in the profile. This document's open decisions are D9 through D18; D1 through D8 remain defined in the profile. The two ranges never overlap, so a bare reference such as "RSP-001 R17" or "RSP-001 D10" is unambiguous by number alone — no reader has to say which of the two RSP-001 files it names.

This draft does not redefine work already owned elsewhere:

- **The memory synchronization profile** ([roaming-and-engram-sync.md](roaming-and-engram-sync.md)) owns sync profiles, watermarks, continuity, posture, and Cloud credential custody. This document owns the workspace-writer mechanics the profile's memory-plane rules assume exist: one observed writer at a time, provably.
- **The handoff transaction protocol** ([HTP-001](handoff-transaction-protocol.md)) consumes the writer epoch defined here as a component of handoff identity, and consumes the claim primitive defined here to derive a claim record's `source_release` field. HTP-001 is not amended by this document; once this draft is accepted, HTP-001's own debt note resolves against it.
- **The adapter feed event schema** ([AEC-001](adapter-feed-events.md)) owns the wire event `sync.state` and the producer identity/signing primitive this document reuses for claim and release records. This document defines the authority-state values `sync.state` carries for the `knowledge` and `delivery` planes; the `memory` plane's values stay with the profile.
- **The migration and recovery plan** ([MRP-001](migration-and-recovery-plan.md), drafted, acceptance pending) owns backup formats, restore epochs, tombstone formats, and the upgrade graph. The cutover checklist below names the steps and gates a Git Sync to Cloud migration must pass; it points at MRP-001 for the artifact formats those steps produce rather than defining them here.
- **The threat model** ([TM-001](threat-model.md)) owns the attacker model that authenticity and replay protection depend on.

AEC-001 owns a distinct, transport-level concept it also calls epochs — alongside sequence ownership, per-transport ordering, and the rest of its transport semantics, for adapter connections. This document always writes "writer epoch" in full so the two are never mistaken for each other.

Everything below exists to satisfy the target architecture's invariant 8: pending, stale, forked, conflicted, uncertain, partial, and orphan-risk states are never shown as complete. A claim, a takeover, or a cutover step that cannot be proven does not default to "probably fine" — it defaults to one of those named states, surfaced, with a recorded reason.

## Purpose and scope

RSP-001 core governs **workspace writer coordination**: which device may currently publish a logical agent's workspace state, how that fact is proven rather than assumed, what happens when two devices disagree about it, and how a memory namespace moves from the Git Sync profile to the Cloud profile without silently dropping or duplicating work.

In scope:

- the claim record: where it lives, what it contains, how it is allocated and updated;
- compare-and-set as the only mechanism that changes who holds the claim;
- fencing: how a publication proves it comes from the current writer, and what a fenced device must do;
- divergence detection and its mapping onto the public state vocabulary;
- recovery: how forked work is preserved and how a human resolves it;
- the `sync.state` authority-state values for the `knowledge` and `delivery` planes;
- the Git Sync to Cloud cutover checklist for a memory namespace, its commit point, and both rollback paths.

This work sits in its own document rather than inside HTP-001 for the same reason HTP-001 sits apart from the memory profile: a handoff *uses* a current writer epoch, but does not decide who holds one. HTP-001 already states this split in its own scope and only consumes what is defined here; this document is the piece that makes that consumption possible.

Out of scope:

- the memory plane's sync profiles, watermarks, continuity, and posture — owned by the [memory synchronization profile](roaming-and-engram-sync.md);
- the handoff lifecycle, state vector, and claim-record replay rules — owned by [HTP-001](handoff-transaction-protocol.md), which only consumes the primitives defined here;
- the wire envelope, signing primitive, and transport epochs — owned by [AEC-001](adapter-feed-events.md);
- backup formats, restore epochs, tombstone formats, and the upgrade graph — owned by MRP-001;
- the attacker model — owned by [TM-001](threat-model.md);
- presentation — owned by [context-orb.md](context-orb.md).

## Problem statement

The same logical agent is open on two devices belonging to one user — a laptop and a workstation. Both hold the workspace; only one may write. The user edits on the laptop overnight, offline. In the morning they open the workstation, see no claim recorded anywhere, and start working there too, unaware the laptop ever diverged. Both devices eventually publish. Without writer epochs and fencing, the second publication silently overwrites or interleaves the first; with last-writer-wins, one night of work disappears and nothing in the product says so.

A second failure looks different but has the same shape. A device believes it holds the claim — nothing told it otherwise — and keeps publishing after another device has taken over. The first device is not confused: it never observed the takeover. Two devices independently confident is worse than one device stuck, because neither shows as wrong.

A related risk sits one layer up, on the memory plane rather than the workspace. Moving a namespace from Git Sync to Cloud is itself a kind of authority handover: the old authority must stop being writable before the new one is trusted, or a device that missed the switch can still publish into a namespace nobody is reading from anymore. The same discipline that protects one writer at a time on the workspace protects one authority at a time on a namespace mid-cutover — proof before trust, never an assumption that everyone got the memo.

**One writer at a time is a promise the product must be able to prove; when it cannot, the work is a fork, never a casualty.**

## Definitions

| Term | Meaning |
| --- | --- |
| Writer epoch | The monotonic integer identifying one continuous period during which a device holds the write claim for one logical agent's workspace. Allocated only by a successful compare-and-set; never a lease timer. |
| Claim record | The signed record naming the current holder of a workspace's writer epoch: logical agent id, writer epoch, holder device id, predecessor writer epoch, `acquired_at`, acquisition kind, signature. |
| Release record | The signed record a holder publishes when it stops writing, marking its writer epoch `released`. Its presence is what HTP-001 reads as `source_release = released`. |
| Compare-and-set | The only mechanism that updates the claim ref: an update accepted only when the ref's current tip equals the value the writer last observed. A precondition failure means the claim moved. |
| Takeover | An explicit user action that claims a workspace without a prior release, because the holder is unreachable or has not released. Recorded with acquisition kind `takeover`. |
| Fencing | Preventing a device that no longer holds the current writer epoch from having its mutations accepted. Enforced by the compare-and-set precondition itself, not by a separate check. This is workspace-writer fencing; the cutover's "fencing of the old authority" is the memory profile's authority fencing applied to a namespace's replication authority — a distinct mechanism, scoped to a different resource. |
| Divergence | Any condition under which two devices could reasonably each believe they hold the current writer epoch, or under which the current holder cannot be determined. |
| Fork | Work preserved on the fork ref — a local-by-default *preservation* ref, never a claim — because its writer epoch was superseded or its currency could not be verified. Exempt from the lower-writer-epoch rejection (Fencing); never merged automatically; never pushed to the shared remote without explicit user action. |
| Namespace tombstone | The record pushed to the memory repository manifest at cutover step 6, marking a memory namespace migrated to Cloud and naming its final digest set. |
| Commit point | The moment in the cutover after which rollback means restoring from backup rather than dropping the Cloud enrollment (step 7). |
| Seed device | The device whose local memory set is complete and current, chosen — with the user confirming — to perform the first Cloud push at cutover (step 5). |
| Managed ref | An Omnifrons-owned Git reference the product creates, updates, and prunes itself, never a user branch; the target architecture's term, reused here for claim, release, and fork records. |
| Authority (workspace) | The single observed source of truth for who may currently write a workspace: the claim record at the tip of the workspace's writer ref. Distinct from the memory plane's Authority, defined by the profile. |
| Acquisition kind | The claim record field distinguishing a `claim` (follows an observed release) from a `takeover` (does not). Never inferred — always recorded explicitly at the moment the record is written. |
| Adopt | The recovery action that makes a preserved fork current by claiming the workspace normally and publishing the fork's content as the first act under the new writer epoch. |

## Writer epoch and claim record

Coordination is scoped to one logical agent's workspace — the same unit HTP-001 hands off. The claim record lives with the workspace's Git remote, as the tip commit of a managed ref (D9):

```text
refs/omnifrons/writers/<logical-agent-id>
```

Engram Cloud is never an authority for workspace claims; memory namespaces have their own authority under the profile, and the two are not the same coordination.

**Fields.** Logical agent id; writer epoch (monotonic integer); holder device id; predecessor writer epoch; `acquired_at` (UTC); acquisition kind (`claim`, following a release, or `takeover`, without one); a signature by the holder's device identity, using the same producer identity primitive HTP-001 adopts from AEC-001 at device level.

The logical agent id in a claim record is the same identity the workspace's registered scope names under the profile's RSP-001-R13. Registering a scope does not by itself allocate a writer epoch — no device holds a claim until one actually claims the workspace — but it fixes which claim ref the eventual claim belongs on.

**Allocation.** A successful claim allocates writer epoch = predecessor writer epoch + 1. There is no separate allocator and no lease timer: single-writer operation is a product constraint (target architecture, "Writer coordination"), not a distributed lock, and the claim record is the evidence of who holds it, not a grant that expires on a clock. Staleness is shown as the claim's age, never as a countdown to expiry.

**Compare-and-set.** The claim ref is updated only with the expected old value — Git's `--force-with-lease` semantics, matched against the exact old tip, are the reference implementation, though the requirement is stated transport-neutrally: any transport used for this ref must offer an equivalent atomic expected-value update. A failed precondition means someone else moved the claim while this device was preparing its own update. The response is divergence handling, below — never a retry that discards the intervening change.

**Remote conformance.** This whole mechanism depends on the remote actually enforcing the lease; nothing in a push response proves that at the moment of the push. Conformance is therefore a deployment precondition, verified once by an explicit probe at remote setup — two conflicting lease pushes against the same expected tip, of which exactly one must fail — rather than something the protocol can detect on every push. Until a remote has been probed, its authority state is `uncertain` (D17).

**Confirming a push.** A push to the claim ref can fail silently at the network or transport layer without the writer ever learning whether its update landed — an acknowledgement is not always available. After any claim, release, or takeover push, the device MUST re-read the claim ref and confirm its tip is the record it just wrote before acting as the current writer, or as released. If the re-read tip is the device's own record, the push succeeded. If it names another device's record, the device lost the race and enters divergence handling as `conflicted`. If the ref cannot be read, the outcome is `uncertain`, and the device MUST NOT act as though the push succeeded.

**Release.** A holder that stops writing — session end, handoff publication, explicit release — publishes a release record on the same ref: a `released` marker for its writer epoch, signed. HTP-001 derives `source_release = released` from this record's presence for the source's writer epoch. Without it, a takeover on another device yields `unreleased-unfenced`, carried exactly as HTP-001 already carries that condition today.

**Takeover.** An explicit user action on another device, taken because the holder is unreachable or has not released. It is a compare-and-set against the last seen tip, recorded with acquisition kind `takeover`. The previous holder is fenced from the moment the new claim record exists — not from the moment it learns of it. Taking over says nothing about the previous holder's unpublished work: preserving it is the previous holder's own duty, discharged once it discovers the fencing (Recovery), never a guarantee the taking-over device makes or can make — it has no access to work that was never published.

**First claim.** A workspace with no prior claim record has no predecessor writer epoch to increment from. The first successful claim allocates writer epoch 1 with `predecessor writer epoch` absent, and its acquisition kind is `claim`, never `takeover` — there is no prior holder for a first claim to take over from.

## Discovery

A device never assumes it holds the current claim; it fetches the claim ref and reads it. Discovery here is pull-based, the same discipline ADR-0002 already requires for Git generally: fetch plus inspection is preferred to implicit pull, and nothing about the claim ref changes that preference.

A device fetches `refs/omnifrons/writers/<logical-agent-id>` at three points: before it begins local mutation it intends to publish, immediately before any compare-and-set update to the ref, and on reconnection after being offline. No push notification is promised for a claim change; a device that never fetches does not learn of a takeover until it next tries to publish and its compare-and-set fails.

`sync.state`'s authority state (below) gives a lighter-weight read of the same fact — writer epoch, holder, claim age — for display purposes. A compare-and-set update always re-fetches the ref directly rather than trusting a cached `sync.state` reading, because a display read and a mutating precondition carry different staleness tolerances.

A device with no prior local knowledge of the ref — a fresh install, or a workspace registered for the first time on this device — fetches it cold and treats whatever it finds as authoritative from that first read; there is no local default to fall back on, and an unreadable or absent ref at this point is `uncertain` (Divergence detection), never treated as "no claim exists yet" unless the workspace genuinely has no prior claim record (First claim, above).

## Fencing

Every claim, release, and handoff publication a writer makes for this workspace carries its writer epoch. A receiver validates that writer epoch against the current claim record before accepting the publication. A publication carrying a writer epoch lower than the current one is rejected as fenced: it renders `forked` when it transports workspace content — a handoff commit — the same state HTP-001's post-claim rule (HTP-001-R8) assigns to a source mutation observed after another device's claim; it renders `published-unverified` when it is a coordination record instead — a handoff envelope, claim record, or release record — the rollback-candidate treatment HTP-001-R10 defines for envelopes, not the rule that produces `forked`. A preserved fork record renders the same `forked` state by definition, without ever being rejected: see Fork exemption, below.

A handoff commit carries its writer epoch in the state vector's provenance component, already reserved for it in HTP-001's own state vector table. HTP-001 does not validate that field itself — fencing validation is this document's job — but it transports the value so a receiver can perform that validation without a second round trip to the claim ref.

**Fork exemption.** The fork ref is a preservation ref, not a claim, and is exempt from the rejection above: a fenced device MAY write to the fork namespace of its own superseded writer epoch — and only that namespace — even after fencing has taken effect. A fork record is signed the same way a claim record is signed. No receiver ever treats a fork ref as current or as a claim on the workspace: it never participates in compare-and-set on `refs/omnifrons/writers/<logical-agent-id>`, and it never satisfies a receiver's claim validation. RSP-001-R20 is scoped to claim, release, and handoff publications precisely so this exemption needs no separate carve-out in the requirement text.

A fenced writer, on first observing that a newer writer epoch exists, MUST stop accepting further mutations for that workspace scope on that device and MUST preserve whatever it has not published as a signed fork record — the one write still permitted, per the exemption above. Every other managed ref is closed to it: the compare-and-set precondition described above is itself the fence, because a fenced writer no longer holds the expected old value for the writer or handoff refs.

This is the mechanism the target architecture's "Writer coordination" section names directly: a receiver never applies last-writer-wins, and offline mutation made without a verified current claim always resolves to `forked/unverified` rather than a silent merge — the same failure state the required failure states table lists for "Writer claim absent/diverged."

**Offline.** A writer that verified its claim before going offline keeps writing under that writer epoch — it has no way to learn the claim moved. Its continuity state is `local`, shown with the claim's age at the moment it last verified. On reconnection it re-verifies the claim before any publication. If the claim moved while it was offline, its offline work is a fork, not a candidate for silent replacement.

## Authenticity and replay

Claim and release records are signed with the holder's device identity, reusing the producer identity primitive HTP-001 adopts from AEC-001 at device level rather than defining a separate one: `producer_id` the device's logical ID, `producer_instance` the installation, device keys never portable state — the same custody rule the target architecture applies to Cloud tokens. A claim or release record whose signature does not verify against a paired device key is treated the way AEC-001's `untrusted` verdict is treated elsewhere: the record is not honored, and the workspace's claim state remains whatever it was before the record arrived.

**First contact.** A device with no prior cached claim record that fetches a readable ref whose signature does not verify renders `uncertain` and MUST NOT write — there is no fallback record to trust instead. A device that does hold a prior cached record falls back to it only if that cached record itself verified when it was read; an unverified cache is never a fallback either.

Replay is bounded the way HTP-001 bounds it for handoff envelopes: a record naming a writer epoch not greater than the last one this device has seen on that ref is either a duplicate — acknowledged idempotently if it exactly matches a record already seen — or a rollback attempt, which renders `uncertain` pending human review rather than being accepted.

Automatic takeover across devices is not part of this protocol; takeover always requires the explicit user action described above. The same-device switch (D16) is the one case automation is allowed, because the device fencing itself against is the one initiating the switch — the degenerate case HTP-001 already carves out for its own lifecycle, for the same reason: source and receiver are one device, and the transport is local. Until TM-001 is accepted, a takeover confirmation shows the previous holder's device fingerprint, the same way HTP-001 shows it for manual claim.

## Divergence detection

| Condition | Resulting state |
| --- | --- |
| Publication carries a writer epoch lower than the current claim and transports workspace content (a handoff commit) | `forked` |
| Publication carries a writer epoch lower than the current claim and is a coordination record instead (a handoff envelope, claim record, or release record) | `published-unverified` |
| Compare-and-set precondition failed — the claim moved | `conflicted`, until a human inspects it |
| Mutation made without a verified current claim | `forked/unverified` |
| Two claim records name the same writer epoch from different holders, or the claim ref shows a remote restore or rollback | `authority-conflict`; remote integrity suspected |
| A claim or release record fails replay validation — its writer epoch is not greater than the last one seen on that ref | `uncertain` |
| Claim ref missing or unreadable | `uncertain` |
| Remote unreachable | `offline` |

Every row is observed from the remote, never inferred from a local setting — the same principle the memory profile states for replication: "replication is never assumed; it is observed." A device's own belief about who holds the claim is not evidence; only a read of the claim ref is. The `authority-conflict` row implicates the remote itself rather than a device, and it has more than one cause: two valid-looking claim records for the same writer epoch mean either a bug in the compare-and-set implementation or a rewritten/forced ref history; a claim ref that reverts to an older tip a device has already seen superseded looks identical — a restored or rolled-back workspace remote produces the same symptom as a rewritten one. Either way it is treated as a possible integrity incident, not routine contention: resolution is human review, followed by a fresh takeover once the anomaly is understood, never an automatic pick of either record. MRP-001's restore epochs cover memory-namespace restores; they say nothing about a restored workspace Git remote, which has no generation marker of its own yet (D18).

## Recovery

A fork is preserved locally, not discarded — and not pushed anywhere by default. It lives on its own managed ref, scoped to the writer epoch it forked from:

```text
refs/omnifrons/forks/<logical-agent-id>/<writer-epoch>
```

Preservation is the fenced device's own duty, discharged the moment it next connects and discovers — through Discovery, above — that it has been fenced; the device that took over never promises to preserve anything on the fenced device's behalf, because it has no access to work the fenced device never published. Writing this local ref is the one act the fork exemption (Fencing) still permits after fencing takes effect.

The fork stays local unless the user explicitly pushes it to the shared remote — a separate, explicit action gated by the same public-remote confirmation HTP-001-R17 requires for a handoff commit (HTP-001 D8), and subject to the Git-surface caution TM-001 states for any publish to a shared remote. Nothing in this protocol pushes a fork automatically.

Forked work is shown side by side with the current state, never merged silently and never deleted without confirmation. Resolution is a human action:

- **Adopt** — the fork becomes current through the normal claim path: the device performs an ordinary compare-and-set claim, then publishes the fork's content as its first act under the new writer epoch. Adoption is not a special write path; it is a claim like any other.
- **Merge** — the user reconciles the fork with the current state through their normal Git workflow, then publishes the result under a freshly claimed writer epoch.
- **Discard** — the fork is deleted only after explicit confirmation naming what is being discarded; there is no bulk or silent discard.

Once resolved, the device may claim the workspace anew.

None of the three paths runs unattended. A fork is, by definition, work whose currency the product could not verify; letting the product also decide its fate without a human in the loop would recreate the exact silent-loss risk this protocol exists to close.

There is no automatic merge and no last-writer-wins path anywhere in this protocol. Consistent with the roadmap's pre-alpha exit criterion for the wider handoff surface, every interruption to claim, takeover, or fencing leaves either a recoverable state or an explicitly `uncertain` one — never a state that looks complete but silently dropped work.

## Managed refs

| Ref | Purpose | Written by | Pruned by |
| --- | --- | --- | --- |
| `refs/omnifrons/writers/<logical-agent-id>` | Current claim and release records for one workspace's writer epoch | The current or claiming device, only via compare-and-set | Never automatically — it is the workspace's live authority |
| `refs/omnifrons/forks/<logical-agent-id>/<writer-epoch>` | Preservation ref for work that could not be published as current; local by default, exempt from the lower-writer-epoch rejection (Fencing) | The fenced device only, once, to the path matching its own superseded writer epoch | Only after human resolution (D12); pushed to the shared remote only on explicit user action |
| `refs/omnifrons/handoffs/<logical-agent-id>/<handoff-id>` | One handoff's transported state (HTP-001, target architecture) | The source device, per handoff | HTP-001's cleanup, after its recovery window elapses |

All three follow the same Omnifrons-managed-ref convention the target architecture establishes for the handoff ref: none is a user branch, and cleanup for any of them never touches user history. The fork ref is the one exception to the fencing rule the other two obey — see Fork exemption in Fencing.

## Timeouts and thresholds

Nothing in this protocol times a claim out on its own — writer epochs do not expire, and a compare-and-set precondition, not a clock, is what tells a device it has lost currency. The thresholds that do exist are open decisions with defaults, not hardcoded constants: the takeover warning threshold (D11), defaulting to a warning plus the claim's age shown rather than a hard minimum age before takeover is allowed; and fork retention before an optional prune (D12), defaulting to keep-until-resolved with no automatic deletion. The cutover checklist's own waits are gated on observed conditions — quiescence proven, a Cloud push acknowledged — rather than on elapsed time, so no cutover-specific timeout is proposed here.

## Authority state for `sync.state`

For the `knowledge` and `delivery` planes, `sync.state` (AEC-001 feed profile) carries: authority identity (the workspace's configured Git remote), instance identity (the remote's identity as observed), and authority state — the writer epoch, its holder, the claim's age, and whether a release record is present for it. Observing more than one remote for the same workspace is `authority-conflict`, the same rule the memory profile already applies to its own planes: a single authority is never assumed.

When a fork exists for the workspace, `sync.state` also announces its writer epoch, size, and age — never its content, and never a push destination; the fork itself stays local until the user explicitly pushes it (Recovery).

The `memory` plane's `sync.state` values remain defined by the profile; this document does not redefine them.

Presentation of this authority state — alongside the profile's Sync health gadget for the memory plane — is the Context Orb specification's concern, not this document's; the Orb remains a read/status projection of the authority defined here, never itself the authority (target architecture, invariant 10).

## Git Sync to Cloud cutover

Moving a memory namespace from the Git Sync profile to the Cloud profile is a checklist with gates, not a setting flip. It belongs here rather than in MRP-001 because it is fundamentally an authority handover — the same shape as the workspace-writer handover above, applied to a namespace's replication authority instead of a workspace's write claim. MRP-001 owns the backup, restore-epoch, and tombstone artifact *formats* the checklist references; this document owns the sequence and the gates. Each step below is a gate the next step depends on.

1. **Quiescence** on every device holding the namespace, per the profile's definition — no in-flight mutation anywhere.
2. **Final watermark.** Publish and import under Git Sync until every device is `current`. Record the resulting chunk digest set as the final set for this cutover.
3. **Backup** of each device's local memory store, in MRP-001's format, retained through the cutover.
4. **Stable mutation identity.** Every observation's stable sync identifier must survive the profile change. This is verified, not assumed: the seed device runs a round-trip check — a sample of observation identifiers recorded before Cloud enrollment must still resolve to the same identifiers after it. A mismatch blocks the cutover at this step. The underlying dependency on the Engram release in use at cutover time remains tracked under the profile's Engram upstream tracking section (D15).
5. **Reconciliation and initial sync.** Choose the seed device — the one holding the complete final set, with the user confirming (D14) — and enroll the namespace on Cloud from it; this enrollment is the namespace's initial sync to the new authority.
6. **Fencing of the old authority.** Push a namespace tombstone to the memory repository's manifest, as the manifest ref's new tip. This push MUST use a compare-and-set precondition against the manifest ref's exact prior tip — the same lease mechanism Compare-and-set uses for the writer claim ref (RSP-001-R29) — so the tombstone is a fence, not merely an observation: once it is the tip, a stale device's own manifest publish fails the precondition, forcing it to fetch, at which point it sees the tombstone and refuses to publish further. A device that observes the tombstone refuses further Git Sync publish for that namespace, and switches its own profile only once its local set equals the final set. This guarantee holds only for Omnifrons-run publishes to the manifest ref; a manual push made outside the lease — by a script or by hand, without the precondition — is advisory only and cannot be relied on to fence anything (RSP-001-R29).
7. **Commit point.** The cutover has committed once the tombstone is pushed and the seed device's Cloud push is acknowledged at a sequence covering the complete final set. Before this point, rollback means dropping the Cloud enrollment. After it, rollback means restoring from the step-3 backup under a new restore epoch (MRP-001).
8. **Tombstones** for observations deleted before cutover are carried through to the Cloud side, not dropped.
9. **Restore epochs** are recorded so a later restore causes devices to re-reconcile rather than treat old chunks as newly arrived (MRP-001 format).
10. **Git-history erasure limits.** Chunk contents remain in the memory repository's Git history after cutover. This protocol makes no promise of erasure; the limitation is stated plainly rather than implied away.
11. **Resurrection conflicts.** A device that reconnects after the commit point with unpublished Git Sync chunks holds `forked` memory. It is imported only by explicit human action, never merged automatically into the Cloud namespace.

Step 6's "fencing of the old authority" fences the memory-plane authority — the Git Sync memory repository for this namespace — from further publish. It is a distinct mechanism from the workspace-writer fencing defined above: it protects a different resource, on a different managed surface (the memory repository's manifest ref, not `refs/omnifrons/writers/...`) — though it borrows the identical compare-and-set lease pattern (RSP-001-R29) to get the same proof-before-trust guarantee. The two never share a ref or a claim record.

A first implementation of this checklist is exercised on disposable or backed-up state before it is offered against a namespace anyone depends on, following the same channel discipline the compatibility policy already states for `nightly` and `alpha`: a less stable channel requires explicit action, and "incompatible" never means permission to destroy the only copy of user data. The cutover's own step 3 backup exists precisely so that guarantee holds even once the checklist is offered broadly.

Reverse cutover — Cloud back to Git Sync — is deferred (D13).

## Product requirements

Each requirement is an acceptance gate with a testable condition.

| ID | Requirement |
| --- | --- |
| RSP-001-R15 | Omnifrons MUST hold the claim record for a workspace's writer epoch on the managed ref `refs/omnifrons/writers/<logical-agent-id>` of the workspace's Git remote, carrying logical agent id, writer epoch, holder device id, predecessor writer epoch, `acquired_at`, acquisition kind, and a signature. Engram Cloud MUST NOT be treated as an authority for workspace claims. |
| RSP-001-R16 | Writer epoch allocation MUST be monotonic (predecessor writer epoch + 1) and MUST occur only through a successful compare-and-set. Omnifrons MUST NOT implement a lease timer or expiry for a writer epoch; staleness MUST be shown as the claim's age, never as a countdown. |
| RSP-001-R17 | An update to the claim ref MUST be a compare-and-set against the exact expected old value. A failed precondition MUST NOT be retried by overwriting the intervening change; it MUST route to divergence handling. After any claim, release, or takeover push, the device MUST re-read the claim ref and confirm its tip is the record it just wrote before acting as writer or as released; an unacknowledged push MUST be resolved only by that re-read — own record: success; another device's record: `conflicted`; unreadable: `uncertain`, and the device MUST NOT write. |
| RSP-001-R18 | A holder that stops writing MUST publish a release record marking its writer epoch `released`. HTP-001's `source_release` field MUST be derived from this record's presence; its absence MUST yield `unreleased-unfenced`. |
| RSP-001-R19 | A takeover MUST be an explicit user action, recorded with acquisition kind `takeover`, performed as a compare-and-set against the last seen tip. The previous holder MUST be treated as fenced from the moment the new claim record exists. |
| RSP-001-R20 | Every claim, release, and handoff publication for a workspace MUST carry its writer's current writer epoch. A receiver MUST reject one whose writer epoch is lower than the current claim's, rendering it `forked` when it carries a handoff commit and `published-unverified` when it is a handoff envelope, claim record, or release record. Fork records are exempt from this requirement; see RSP-001-R21. |
| RSP-001-R21 | A device MUST stop accepting mutations for a workspace scope on first observing a writer epoch newer than the one it holds, and MUST preserve any unpublished work as a signed fork record on a local-by-default preservation ref rather than discarding it. Writing that record to the fork namespace of its own superseded writer epoch is exempt from RSP-001-R20 and remains permitted after fencing takes effect; no other publication is. The device that takes over MUST NOT be treated as responsible for that preservation. |
| RSP-001-R22 | A device that verified its claim before going offline MAY continue writing under that writer epoch, shown as `local` with the claim's last-verified age. It MUST re-verify the claim before any publication on reconnection, and MUST treat its offline work as a fork if the claim moved. |
| RSP-001-R23 | Omnifrons MUST detect and label every divergence case in the table above — including a replay or rollback attempt on a claim or release record — and MUST compute the result from a remote read, never from a local setting alone. |
| RSP-001-R24 | A fork MUST be preserved on its own managed ref under `refs/omnifrons/forks/<logical-agent-id>/<writer-epoch>`, local by default, and MUST NOT be deleted without explicit human confirmation. Pushing a fork ref to the shared remote MUST require the same explicit per-item confirmation HTP-001-R17 requires for a handoff commit to a publicly observed remote. Omnifrons MUST NOT auto-merge a fork or resolve it by last-writer-wins. |
| RSP-001-R25 | `sync.state` for the `knowledge` and `delivery` planes MUST carry authority identity, instance identity, and authority state (writer epoch, holder, claim age, release presence); more than one remote observed for one workspace MUST render `authority-conflict`. |
| RSP-001-R26 | Omnifrons MUST gate a Git Sync to Cloud cutover on all eleven checklist steps above, each evaluated pass or fail before the next step runs; a failed step MUST block the cutover rather than being skipped. Omnifrons MUST NOT cross the commit point (step 7) without both the pushed tombstone — written under RSP-001-R29's compare-and-set precondition — and the seed device's acknowledged push covering the final set, and MUST support both named rollback paths depending on whether the commit point has passed. |
| RSP-001-R27 | A device that reconnects with unpublished Git Sync chunks after the cutover commit point MUST render its memory `forked` and MUST require explicit human action to import it into the Cloud namespace. |
| RSP-001-R28 | An interruption at claim, takeover, fencing, any divergence check, or any cutover step MUST yield a recoverable state or an explicit `uncertain` state; Omnifrons MUST NOT render false completion. |
| RSP-001-R29 | An Omnifrons-run publish to a memory repository's manifest ref during a Git Sync to Cloud cutover MUST use a compare-and-set precondition against the manifest ref's exact tip. A manual push to that ref without an equivalent precondition is advisory only and MUST NOT be relied on to fence the old authority. |

## Signal mapping

The public vocabulary column uses the states in the [compatibility policy](versioning-and-compatibility.md) plus the target architecture's required failure states, the same source the memory profile's own signal mapping cites.

| Condition | RSP-001 state | Public vocabulary | User-facing consequence |
| --- | --- | --- | --- |
| Publication fenced by a lower writer epoch, carrying a handoff commit | `forked` | `forked` | Preserved on a fork ref; not published as current |
| Publication fenced by a lower writer epoch, a coordination record instead (handoff envelope, claim or release record) | `published-unverified` | `published-unverified` | Human review before any claim |
| Compare-and-set precondition failed | `conflicted` | `conflicted` | Human inspection before retry |
| Mutation without a verified current claim | `forked/unverified` | `forked` | Preserved locally; cannot publish as current automatically |
| Two claim records, same writer epoch, different holders, or a restored/rolled-back claim ref | `authority-conflict` | `authority-conflict` | Operation blocked pending human resolution |
| Claim or release record fails replay validation | `uncertain` | `uncertain` | Never rendered as current; no write |
| Claim ref missing or unreadable | `uncertain` | `uncertain` | Never rendered as current |
| Remote unreachable | `offline` | `offline` | Last known claim shown with its age |
| More than one remote observed for one workspace (`sync.state`) | `authority-conflict` | `authority-conflict` | Operation blocked pending human resolution |
| Device reconnects with unpublished chunks after the cutover commit point | `forked` | `forked` | Explicit human import required; never auto-merged into Cloud |

## Open decisions

| ID | Question | Options | Default proposal |
| --- | --- | --- | --- |
| D9 | Claim ref name and record format | Managed ref with a signed commit body; a separate signed sidecar object | Managed ref `refs/omnifrons/writers/<logical-agent-id>` with a signed commit, as drafted above |
| D10 | Should Cloud host a mirror of workspace claim records for cross-device discovery? | Yes, as a discovery convenience; no, Git remains the sole authority | No in v1 — Engram Cloud is not a workspace-claim authority (Document authority) |
| D11 | Takeover safeguards | Require the holder's claim age above a threshold; always allow, with a warning and the age shown | Warning plus age shown; no threshold |
| D12 | Fork retention (local forks; a pushed fork's remote copy is a separate, user-initiated artifact) | Keep until resolved; prune after N days with confirmation | Keep until resolved |
| D13 | Reverse cutover, Cloud back to Git Sync | Define now; defer | Deferred |
| D14 | Seed device selection at cutover | Most complete set, chosen automatically; user's explicit choice | The device holding the complete final set, with the user confirming |
| D15 | Stable mutation identity verification cadence | Ad hoc; tied to each Engram release | Tied to the profile's Engram upstream tracking section |
| D16 | May `takeover` be automated for a same-device switch (the harness/model degenerate case)? | Require explicit action even on the same device; allow automation | Yes — mirrors HTP-001 D4, since source and receiver are the same device |
| D17 | Should Omnifrons probe a Git remote for lease conformance before relying on it for compare-and-set fencing? | Probe at setup with two conflicting lease pushes, exactly one must fail; trust the remote's stated capability without probing | Probe at setup; until probed, the remote's authority state is `uncertain` |
| D18 | Should a claim record carry a remote generation marker, to distinguish a legitimate takeover from a restored or rolled-back claim ref? | Yes, add a generation marker; no, rely on human review of the anomaly alone | Not yet — human review plus a fresh takeover already resolves the anomaly; revisit if generation confusion recurs in practice |

## Acceptance evidence and follow-up

- Conformance tests MUST cover: claim record creation and every field, including the first-claim case with no predecessor writer epoch; compare-and-set success and precondition failure; release-record presence and absence, checked against HTP-001's `source_release` derivation; takeover recorded with the correct acquisition kind; fencing of a stale writer epoch on claim, release, and handoff publications (never on fork records); signature verification and replay rejection for claim and release records; every row of the divergence table; fork preservation and its ref naming; the `sync.state` authority-state fields for `knowledge` and `delivery`; and all eleven cutover steps including both rollback paths around the commit point.
- Reproducing the problem statement — two devices, one offline overnight, no observed claim on the second — MUST render the second device's work as `forked`, never as a silent overwrite of the first.
- Reproducing the resurrection case — a device rejoining with unpublished Git Sync chunks after a cutover's commit point — MUST render `forked` memory requiring explicit human import, never an automatic merge into the Cloud namespace.
- Conformance tests MUST additionally cover the `authority-conflict` case treated as a possible integrity incident, including a restored/rolled-back claim ref as a cause (Divergence detection), and the three fork resolution paths — adopt, merge, discard — including that discard always requires explicit confirmation naming what is discarded.
- Conformance tests MUST also cover: the fork-namespace exemption, including that a fenced device writing to any ref other than its own writer epoch's fork path is still rejected; the post-push re-read resolving every ambiguous push outcome (own record, another device's record, unreadable); the manifest-ref lease actually fencing a stale device's Git Sync publish once the tombstone is the tip, and a manual push outside the lease correctly disclosed as advisory only (RSP-001-R29); the remote conformance probe at setup detecting a remote that does not enforce the lease, with the remote's authority state `uncertain` until probed (D17); and first contact with a readable but signature-invalid claim ref rendering `uncertain` with no write.
- Conformance tests MUST cover the stable-mutation-identity round-trip check on the seed device (step 4) blocking the cutover on a mismatch, and each of the eleven cutover steps evaluated pass or fail rather than assumed.
- Debt, not drafted here. This document assumes [MRP-001](migration-and-recovery-plan.md)'s backup, restore-epoch, and tombstone formats without defining them; the cutover checklist names the gates those formats must satisfy, but the artifact itself remains MRP-001's. Reverse cutover (D13) is deferred entirely. Cloud-side discovery of workspace claims (D10) is declined for v1 and not designed further here.

## Related contracts

- [Target architecture](target-architecture.md) — the "Roaming and synchronization" section this document drafts, invariant 7 (one observed active authority) and invariant 9 (synchronized content is untrusted).
- [Workspace roaming and Engram sync protocol](roaming-and-engram-sync.md) — the memory synchronization profile this document completes; RSP-001-R1 through R14 and D1 through D8.
- [Handoff transaction protocol](handoff-transaction-protocol.md) (HTP-001) — consumes the writer epoch and the claim/release primitives defined here.
- [Adapter feed event schema](adapter-feed-events.md) (AEC-001) — the `sync.state` wire event and the producer identity primitive reused for claim and release signatures.
- [Migration and recovery plan](migration-and-recovery-plan.md) (MRP-001, drafted, acceptance pending) — backup, restore-epoch, and tombstone formats the cutover checklist depends on.
- [Threat model](threat-model.md) (TM-001) — the attacker model authenticity and replay protection depend on.
- [Versioning and compatibility](versioning-and-compatibility.md) — the public state vocabulary this document's signal mapping rides, and the pre-1.0 channel rules the cutover checklist's staged rollout follows.
- [Product roadmap](roadmap.md) — the Alpha stage's "one active writer and explicit fork detection" scope item and the pre-alpha recoverable-or-uncertain exit criterion this protocol satisfies.
- [ADR-0002](adr/0002-desktop-technology-stack.md), Git subsection — fetch-plus-inspection preference, explicit argv, and isolated temporary state for managed-ref writes.
- [Context Orb specification](context-orb.md) — presentation of writer, claim, and fork state, out of scope here.

## References

- [Git references documentation](https://git-scm.com/book/en/v2/Git-Internals-Git-References) — the managed ref namespaces used for claim, release, and fork records.
- [`git push --force-with-lease`](https://git-scm.com/docs/git-push#Documentation/git-push.txt---force-with-leaseltrefnamegtltexpectgt) — the reference implementation for the compare-and-set update this protocol requires.
- [Target architecture](target-architecture.md) — the "Roaming and synchronization" section this document drafts.
- [Workspace roaming and Engram sync protocol](roaming-and-engram-sync.md) — the memory synchronization profile this document completes.
- [Handoff transaction protocol](handoff-transaction-protocol.md) — the debt note this document's claim primitive resolves.
