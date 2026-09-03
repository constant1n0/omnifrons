# Handoff Transaction Protocol

**Document role:** Handoff transaction protocol: lifecycle, state vector, claim, authenticity, replay, and cleanup for cross-device and cross-harness work handoff (HTP-001)  
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

This document drafts the planned handoff transaction protocol (HTP-001): the lifecycle, immutable state vector, publication barriers, claim protocol, authenticity, replay/rollback handling, and cleanup for a handoff — the transaction that carries a logical agent's in-progress work from one device (or harness) to another. The [target architecture](target-architecture.md)'s "Handoff transaction" section is this document's seed; every bullet there is satisfied below. The target architecture governs any conflict.

This draft does not redefine work already owned elsewhere:

- **RSP-001** owns writer epochs, compare-and-set claim records, fencing of workspace writers, divergence detection and recovery, and memory-plane watermarks/continuity. HTP-001 consumes the writer epoch as a component of handoff identity and consumes RSP-001-R9's watermark gate result; it defines the handoff-level *release* obligation of the source device and the hook by which a post-claim source mutation becomes `forked`, but the fencing mechanism itself stays in RSP-001. Until the [RSP-001 core](workspace-roaming-protocol.md)'s compare-and-set claim records are accepted, a claim record's `source_release` field carries `unreleased-unfenced` as an explicit, visible condition — never an implied guarantee.
- **AEC-001** owns the wire event `handoff.state` and the producer identity/signing primitive. HTP-001 defines which values are valid in that event and adopts the producer identity primitive at device level (see Authenticity and replay).
- **[TM-001](threat-model.md)** owns the attacker model. Until it is accepted, automatic claim stays blocked and manual claim shows the device fingerprint.
- **MRP-001** owns migration, backup, and restore. **UTA-001** owns update trust. **[context-orb.md](context-orb.md)** owns presentation.
- The same-device model/harness switch described in the target architecture's Model and harness switching section is the degenerate case of this protocol — source and receiver are one device, transport is local. HTP-001 does not change that flow; it frames it.

## Purpose and scope

HTP-001 governs the **handoff transaction**: the saga that takes a source device's in-progress work — workspace, knowledge, delivery, memory, and runtime state — through preparation, publication, and claim to a receiving device, or through the same-device model/harness switch.

In scope:

- the canonical lifecycle and its mapping onto the public state vocabulary;
- the immutable state vector and its envelope schema;
- composite quiescence proof across every plane;
- publication barrier ordering, including the memory watermark barrier;
- the claim protocol: validation checklist, explicit user action, idempotency, racing claims;
- device-level authenticity and replay/rollback detection, adopted from AEC-001;
- cleanup scope and the recovery window;
- the `handoff.state` event's valid values and emission rules.

Out of scope:

- workspace writer epochs, compare-and-set claim records, fencing, and memory watermarks themselves — owned by RSP-001, consumed here;
- the wire envelope and signing primitive for adapter events generally — owned by AEC-001, adopted here at device level;
- the attacker model automatic claim depends on — owned by TM-001;
- migration, backup, and restore — owned by MRP-001;
- update trust — owned by UTA-001;
- presentation of handoff state — owned by [context-orb.md](context-orb.md);
- abuse of the transports themselves — flooding `handoff.state`, mass creation of claimable-looking handoffs — evaluated by TM-001; HTP-001 only guarantees that no such event can produce a `claimable` or `claimed` state without passing every gate below.

## Problem statement

RSP-001's scenario ends with a gap in the memory plane: a laptop session pushes Git commits overnight, but the Engram observations behind those commits never leave the device. Extend it one step. Suppose a handoff protocol existed but treated the portable-work commit as the whole transaction, the way the target architecture warns against. The laptop's handoff commit publishes cleanly; the workstation fetches it and finds working code. Nothing in the mechanics failed — the commit is real, the ref resolves, the files are there. What is missing is memory of *why* those files changed, and no field in the transaction said so. Under HTP-001 that handoff could not have left `publication-pending`: the memory barrier publishes before the managed ref, so a handoff whose memory watermark has not caught up never reaches a state a receiver can claim.

A second failure the commit-only view also misses: a receiver claims the handoff while the source keeps working. Without a release obligation and fencing, both devices believe they hold the current copy of the agent's work — the receiver because it validated and claimed, the source because nothing told it to stop. Two devices independently confident is worse than one device stuck, because neither shows as wrong. HTP-001 names this condition `forked` the moment a post-claim source mutation is observed, and it refuses to let a claim be treated as complete while the source has not released.

**A handoff is complete only when every plane proves it, and anything unproven is `uncertain`.**

## Definitions

| Term | Meaning |
| --- | --- |
| Handoff | The transaction that carries a logical agent's in-progress work — workspace, knowledge, delivery, memory, and runtime state — from a source device to a receiver device, or through the same-device model/harness switch. |
| Handoff ID | A unique identifier for one handoff, monotonic within its writer epoch (RSP-001). |
| Writer epoch | The workspace-writer coordination unit owned by RSP-001; HTP-001 consumes it as a component of handoff identity and never redefines it. |
| Checkpoint | Portable task/context envelope; it references but does not replace authoritative artifacts (target architecture). |
| Predecessor | The previous handoff ID or checkpoint this handoff supersedes; part of the replay/rollback chain. |
| State vector | The immutable, per-plane snapshot of what a handoff transfers: workspace, knowledge, delivery, memory, runtime, and provenance. |
| Envelope | The versioned, signed message that carries the state vector plus lifecycle metadata, per the compatibility policy's checkpoint/handoff envelope domain. |
| Handoff commit | The disposable Omnifrons-managed Git commit that transports approved in-progress file state, per the target architecture; the workspace plane's payload. |
| Publication barrier | An ordered step — memory, then knowledge/delivery, then managed ref, then envelope — that must complete and be acknowledged before the next step runs. |
| Claim | The verified transition making one handoff current on a receiving device, per the target architecture; publication alone is not a claim. |
| Claim record | The evidence a receiver writes on claiming: handoff ID, receiver device ID, the receiver's own monotonic claim sequence, UTC time, `source_release` (`released` or `unreleased-unfenced`, set by the receiver from what it observed of the source's release/fencing state at claim time), and a signature over the canonical record. |
| Release | The source device's obligation to stop accepting mutations for the logical agent once it honors a `claimed` observation (HTP-001-R16). Distinct from the claim record's `source_release` field, which is the receiver's own observation, not the source's action. |
| Producer identity | The device-level pairing and signing identity adopted from AEC-001: `producer_id` is the device's logical ID, `producer_instance` is the installation. |
| Fencing | Preventing a previous authority or writer from accepting mutations after a cutover; owned by RSP-001 and not drafted here. |
| Watermark | The evidence of memory-plane replication position, owned by RSP-001. The state vector carries watermark values exactly as RSP-001's supported surface reports them at capture time, as opaque components; HTP-001 compares them only through RSP-001-R9's gate and never derives or interprets them itself. |
| Quiescence | No in-flight mutation on a device; HTP-001 requires quiescence proof separately for each plane before a handoff can be `prepared`. |
| Recovery window | The bounded period after `claimed` or `aborted` during which the source retains its managed ref and temp state before cleanup prunes it. |
| Startup brief | The bounded facts-and-references package handed to a harness after a switch or claim, per the target architecture; always untrusted input. |

## Lifecycle

The canonical state machine is exactly the list in the target architecture's Handoff transaction section: `prepared -> publication-pending -> published-unverified -> claimable -> claimed`, plus `aborted` and `uncertain`. Omnifrons exposes no other handoff state to a product surface.

| State | Entry condition | Exit transitions | Who acts | Evidence recorded |
| --- | --- | --- | --- | --- |
| `prepared` | Composite quiescence proven for every plane; candidate state vector built | -> `publication-pending` on publish start; -> `aborted` on user cancel; -> `uncertain` if a plane's quiescence proof lapses before publish begins | Source device (quiescence proof); user (approves the candidate per the portable-work contract) | Candidate tree hash, manifest digest, per-plane quiescence proof, approval binding |
| `publication-pending` | Publish started: memory barrier running, workspace/knowledge/delivery riding the handoff commit, managed ref pushing | -> `published-unverified` when the envelope publishes as the last barrier; -> `uncertain` if a barrier step cannot be proven within D2; -> `aborted` on user cancel before the envelope publishes | Source device | Per-barrier acknowledgement (watermark, commit, ref push) with UTC time |
| `published-unverified` | Envelope visible on the managed ref and the feed; no receiver has validated it | -> `claimable` when a receiver's validation checklist passes; stays `published-unverified` (reason "producer untrusted") on a failed authenticity check or a rollback-candidate/first-contact envelope (HTP-001-R9, HTP-001-R10) — both require human review, never `claimable`; -> `uncertain` on other failed/partial validation, such as an unknown schema major; -> `aborted` if the source withdraws the ref before any claim | Receiver device (validates); source device (may withdraw) | Envelope, signature, validation attempt log |
| `claimable` | Receiver validation passed: complete vector, watermark gate, schema, authenticity | -> `claimed` on explicit user claim; -> `stale` (public vocabulary; see mapping) on D1 expiry or supersession; -> `uncertain` if validation evidence lapses (for example, remote unreachable) | Receiver device proposes; user disposes | Validation checklist result, watermark reading |
| `claimed` | Receiver wrote and published a claim record the source honors under HTP-001-R16 | Source releases and enters cleanup once the recovery window elapses with no open condition (HTP-001-R11); -> `conflicted` if a second claim, or a claim naming an `aborted`/superseded handoff, is observed before compare-and-set fencing exists | Receiver device (writes claim record); source device (honors it, releases, cleans up) | Claim record (handoff ID, receiver device ID, claim sequence, UTC time, `source_release`, signature) |
| `aborted` | Explicit user cancellation before `claimed`, or source withdrawal of an unclaimed `published-unverified` handoff | Terminal | User, on the source device | Cancellation reason, last proven state |
| `uncertain` | Quiescence, integrity, authenticity, or fencing cannot be proven at any transition | Requires human review to re-enter the lifecycle (re-prepare) or resolve to `aborted`/`conflicted` | Whichever device detects the gap; user resolves | The specific unproven condition, diagnostics |

### Mapping onto the public vocabulary

The [compatibility policy](versioning-and-compatibility.md)'s public state vocabulary is broader than the lifecycle above; HTP-001 maps it as follows:

- `local` — a candidate before `prepared`: an approved-but-unpublished workspace snapshot. It is not yet a handoff.
- `published-unverified` — beyond the initial pre-claim window, also the state for a producer-authenticity failure (AEC-001's `untrusted` verdict) and for a rollback-candidate or first-contact envelope: proven-negative or unresolved-provenance outcomes that require human review, never `uncertain` and never `claimable`.
- `stale` — a `claimable` handoff whose source state has moved past its state vector, or that is superseded by a newer handoff from the same source (D1).
- `forked` — a mutation on the source observed after another device's `claimed`, or an offline mutation made without a verified current claim (RSP-001's rule).
- `conflicted` — a receive conflict with evidence preserved on both sides, or a racing second claim pending human review.
- `offline` — the managed ref or feed transport is unreachable; the last known state renders with its age.
- `failed` — a terminal error at any step, with evidence retained.
- `uncertain` — carried verbatim whenever quiescence, integrity, authenticity, or fencing cannot be proven, at any transition. Never masked as any other state.

## Initiation and discovery

Preparation and publication always originate on the source device: only it can quiesce its own harness and capture its own worktree. A receiver never initiates a handoff on the source's behalf.

Discovery on a receiver uses the two transports the target architecture already defines: the managed Git ref namespace `refs/omnifrons/handoffs/<logical-agent-id>/<handoff-id>` and the `handoff.state` feed event (AEC-001 feed profile). A receiver may learn of a handoff from either transport independently; validation (below) is transport-agnostic.

Claim is an explicit user action on the receiver: the system proposes a `claimable` handoff, the user disposes. Automatic claim is blocked until TM-001 is accepted and the authenticity gate passes — except for the same-device model/harness switch, which is the degenerate case of this protocol and keeps its own already-defined automation (target architecture, Model and harness switching).

## State vector and envelope

The envelope is the one schema the target architecture's Handoff transaction section and the compatibility policy's public envelope fields both describe; HTP-001 reconciles them into a single structure.

**Envelope fields:** logical identity (agent ID, scope ID), task, active project, predecessor (previous handoff ID or checkpoint), portable-work commit (the handoff commit), state vector (below), lifecycle state, provenance, and schema/integrity metadata (envelope `major.minor`; an unknown major is rejected, an unknown optional minor field is preserved, per the [compatibility policy](versioning-and-compatibility.md)).

**State vector, per plane:**

| Plane | Contents |
| --- | --- |
| Workspace | Base commit, handoff commit, manifest digest, inventory digest |
| Knowledge | Canonical Markdown artifact revisions and hashes |
| Delivery | OpenSpec content revision, carried in the same handoff commit |
| Memory | Per namespace: profile, authority, and watermark — chunk digest under Git Sync, acknowledged sequence under Cloud — exactly as RSP-001 defines them |
| Runtime | Harness/adapter identities and versions as IDs only; open approval IDs; running run IDs, which must be terminal or explicitly carried as `uncertain` |
| Provenance | Source device ID, producer identity (AEC-001), writer epoch (RSP-001), UTC creation time, signature |

The knowledge plane travels as revisions and hashes only; canonical Markdown content is never inlined in the envelope or the feed. It reaches the receiver through the workspace transport when the note is part of the workspace state being handed off, and by reference otherwise.

Hashes prove integrity, not authorship (target architecture, Handoff transaction).

## Quiescence per plane and composite proof

Each plane proves quiescence independently before a handoff can be `prepared`:

- **Memory** — RSP-001's quiescence definition, plus the watermark it publishes.
- **Workspace** — the harness quiesced or stopped before capture, its process descendants proven stopped, and the index/worktree inventory frozen for the candidate build.
- **Knowledge and delivery** — covered by the stopped harness plus a re-hash taken after stop, since both ride the same handoff commit.

The composite proof is the conjunction: a handoff reaches `prepared` only when every plane proves quiescence. A plane whose proof cannot be obtained yields `uncertain` rather than a partial `prepared`, and the default policy is stop-first — Omnifrons stops the harness before final inventory rather than proceeding on an unproven assumption (target architecture, Initial portable-work contract).

## Publication barriers

Publication runs in a fixed order, each step acknowledged before the next begins:

1. **Memory** publishes/pushes, so the watermark is known before anything downstream depends on it.
2. **Knowledge and delivery** ride the handoff commit — no separate step.
3. **Managed ref push** — the handoff commit reaches `refs/omnifrons/handoffs/<logical-agent-id>/<handoff-id>` on the configured remote.
4. **Envelope publication**, last. The envelope references every digest and watermark produced by the earlier steps, which makes it the actual publication barrier: nothing can be `claimable` before the envelope that describes it exists.

The handoff is `publication-pending` from the start of step 1 until every step is acknowledged; RSP-001-R9 forbids advancing while the memory watermark still lags the state vector. Once the envelope is visible but no receiver has validated it, the handoff is `published-unverified`.

The handoff commit carries in-progress file state; the envelope, state vector, claim record, and `handoff.state` payload never carry secrets, tokens, device keys, approvals, executable profiles, or raw filesystem paths (HTP-001-R18). Publishing a handoff commit to a remote Omnifrons observes as public requires an explicit per-handoff confirmation — never automatic (D8, HTP-001-R17).

## Claim protocol

**Receiver validation.** The receiver fetches the exact managed ref and verifies: project identity, base commit, handoff commit, manifest digest, every per-plane digest, the memory watermark gate (RSP-001-R9: pulled/imported watermark at or ahead of the state vector), envelope schema compatibility, and authenticity (below). Only when every check passes does the handoff render `claimable`. Mutable links alone are never sufficient (target architecture, Handoff transaction).

**Claim.** Claiming is an explicit user action. The receiver writes a claim record — handoff ID, receiver device ID, its own monotonic claim sequence, UTC time, `source_release` (`released` or `unreleased-unfenced`, set from what the receiver observed of the source's release/fencing state), and a signature over the canonical record — and publishes `claimed` through the same two transports used for discovery. The Ops checkpoint/handoff gadget shows `source_release` alongside the handoff state.

**Idempotency.** A repeated claim request from the same receiver for the same handoff, carrying a claim sequence no greater than the last one recorded, acknowledges the prior outcome; it never writes a second claim record or re-runs the claim's side effects.

**Racing claims.** Until RSP-001 defines compare-and-set claim records, a second claim observed for the same handoff from a different receiver is `conflicted` and requires human review. Omnifrons never resolves it by last-writer-wins (D5).

**Claim-record replay.** A source honors a `claimed` observation only when the claim record verifies against the paired receiver key, names a handoff the source published, and carries a claim sequence greater than the last one seen from that receiver device (HTP-001-R16). A replayed claim record is acknowledged as a duplicate and never triggers a second release. A claim record naming an `aborted` or superseded handoff is `conflicted` and requires human review.

**Source release.** On observing a `claimed` record it honors under the claim-record replay rule above, the source device releases — stops accepting mutations for that logical agent — and enters cleanup. Any source mutation observed after that point is `forked`.

## Authenticity and replay

HTP-001 reuses AEC-001's producer identity primitive at device level rather than defining a separate one (D6); see [Producer identity and key distribution](adapter-feed-events.md#producer-identity-and-key-distribution) for the pairing, rotation, and revocation mechanics, not restated here. At the device level: each Omnifrons installation is a producer (`producer_id` the device's logical ID, `producer_instance` the installation); device keys are never portable state, the same custody rule the target architecture applies to Cloud tokens; and the fingerprint shown at pairing is compared against the value displayed on the peer device itself or obtained directly from it — never against a value the transport supplies. The signature covers the full canonical serialization of the envelope — every field except the signature itself — and, for a claim record, the full canonical serialization of that record, using the canonical serialization AEC-001 defines; no field sits outside the signature.

AEC-001's `untrusted` verdict is a producer-level fact, not a handoff state: an envelope or claim record whose producer is unpaired, revoked, or fails signature verification renders handoff state `published-unverified` with reason "producer untrusted" and never reaches `claimable` (HTP-001-R9).

**Replay and rollback.** A receiver tracks the last seen `(writer epoch, handoff ID)` per source device. An envelope whose writer epoch is lower than the last seen epoch, or whose handoff ID is not greater than the last seen ID within the same epoch, or whose predecessor is unknown, is a rollback candidate: it stays `published-unverified` pending human review and never advances to `claimable`. An exact duplicate of an already-seen envelope is acknowledged idempotently — neither rejected nor re-processed. First contact with a source device, with no prior `(epoch, ID)` to compare against, is also `published-unverified` pending manual claim, consistent with the TM-001 gate (HTP-001-R10).

Until TM-001 is accepted, automatic claim is blocked; manual claim shows the source device's fingerprint so the user can compare it by hand.

## Cleanup and recovery window

The source prunes only Omnifrons-managed refs and temporary state, and only after the recovery window has elapsed (D3, proposed 72 hours), timed from the source's observation of `claimed` or `aborted`. The D3 timer suspends while the handoff is `forked`, `conflicted`, or under human review, and resumes once that condition clears. Cleanup then runs only for a `claimed` handoff whose `source_release` was observed as `released`, or for an `aborted` handoff, and only when no open condition remains. Until RSP-001 delivers compare-and-set claim records, a claim record's `source_release` is `unreleased-unfenced` by default (Document authority), so a `claimed` handoff's managed ref is retained pending that gap being closed; only `aborted` handoffs prune automatically today.

The envelope itself is retained as a tombstone; the receiver keeps its claim record independently. A failed publish retains the local ref and manifest for retry; a failed receive retains the fetched commit and conflict evidence — the same rule the target architecture states for the portable-work contract's receive path. Cleanup covers temporary state on every plane and never deletes user branches or work.

## Timeouts and thresholds

Every timeout in this protocol is an open decision with a default proposal, not a hardcoded constant: `claimable` expiry (D1), maximum `publication-pending` duration before `uncertain` (D2), the cleanup recovery window (D3), and whether the same-device switch keeps its own automation (D4). See Open decisions.

## `handoff.state` payload semantics

`handoff.state` (AEC-001 feed profile) carries: handoff ID, lifecycle state, per-plane gate results including the RSP-001-R9 watermark gate result, source and receiver device references, predecessor, envelope version, and `uncertain` carried verbatim, never masked. Omnifrons emits it on every lifecycle transition and includes the latest state of every open handoff in the AEC-001 bootstrap snapshot, so a cold-started consumer has no dangling handoff reference (D7).

The Ops dashboard's checkpoint/handoff gadget and the Home status line both render this event; an `uncertain` handoff is never shown as complete ([context-orb.md](context-orb.md)).

## Product requirements

Each requirement is an acceptance gate with a testable condition.

| ID | Requirement |
| --- | --- |
| HTP-001-R1 | Omnifrons MUST implement the handoff lifecycle exactly as `prepared -> publication-pending -> published-unverified -> claimable -> claimed`, plus `aborted` and `uncertain`, and MUST NOT expose any other handoff state to a product surface. |
| HTP-001-R2 | A handoff MUST NOT leave `prepared` for `publication-pending` until composite quiescence is proven for every plane (workspace, knowledge, delivery, memory, runtime); an unproven plane MUST render the handoff `uncertain` and block automatic advance. |
| HTP-001-R3 | Publication MUST follow the barrier order — memory, then knowledge/delivery, then the managed ref, then the envelope — and MUST NOT advance past `publication-pending` while the namespace's watermark is behind the state vector (RSP-001-R9). |
| HTP-001-R4 | A receiver MUST validate the complete state vector — project identity, base commit, handoff commit, manifest digest, every per-plane digest, the RSP-001-R9 watermark gate, envelope schema compatibility, and authenticity — before rendering a handoff `claimable`; a partial or failed validation MUST NOT produce `claimable`. |
| HTP-001-R5 | Claim MUST be an explicit user action on the receiver. Omnifrons MUST NOT claim automatically until TM-001 is accepted and the authenticity gate passes, except for the same-device model/harness switch, which retains its own already-defined automation. |
| HTP-001-R6 | A repeated claim request from the same receiver for the same handoff, carrying a claim sequence no greater than the last one recorded, MUST be idempotent: it MUST acknowledge the prior outcome and MUST NOT write a second claim record or repeat the claim's side effects. |
| HTP-001-R7 | A second claim observed for the same handoff from a different receiver, before the RSP-001 core's compare-and-set claim records are accepted, MUST render `conflicted` and MUST require human review; Omnifrons MUST NOT resolve it by last-writer-wins. |
| HTP-001-R8 | The source device MUST release — stop accepting mutations for the logical agent — only upon observing a `claimed` record it honors under HTP-001-R16, and MUST label any source mutation observed after that point `forked`. |
| HTP-001-R9 | Every handoff envelope and claim record MUST carry a signature under AEC-001's producer identity primitive, paired by trust-on-first-use; the fingerprint MUST be compared against the value displayed on the peer device itself or obtained directly from it, never against a value the transport supplies. AEC-001's `untrusted` verdict is a producer-level fact, not a handoff state: an envelope or claim record whose producer is unpaired, revoked, or fails signature verification MUST render handoff state `published-unverified` with reason "producer untrusted" and MUST NOT reach `claimable`. Until TM-001 is accepted, manual claim MUST display the source device's fingerprint. |
| HTP-001-R10 | A receiver MUST track the last seen `(writer epoch, handoff ID)` per source device. An envelope whose writer epoch is lower than the last seen epoch, or whose handoff ID is not greater than the last seen ID within the same epoch, is a rollback candidate and MUST render `published-unverified` pending human review, never `claimable`. An exact duplicate of an already-seen envelope MUST be acknowledged idempotently — neither rejected nor re-processed. First contact with a source device, with no prior state to compare against, MUST render `published-unverified` pending manual claim, consistent with the TM-001 gate. |
| HTP-001-R11 | Cleanup MUST prune only Omnifrons-managed refs and temporary state, MUST retain the envelope as a tombstone after pruning, and MUST NOT run before the recovery window (D3) has elapsed, measured from the source's observation of `claimed` or `aborted`. The D3 timer MUST be suspended while the handoff is `forked`, `conflicted`, or under human review. Cleanup MUST prune only a `claimed` handoff whose `source_release` was observed as `released`, or an `aborted` handoff, and only when no open condition remains. |
| HTP-001-R12 | A failed publish MUST retain the local ref and manifest for retry, and a failed receive MUST retain the fetched commit and conflict evidence, without deleting user branches or work. |
| HTP-001-R13 | Omnifrons MUST reject an envelope whose major schema version it does not support and MUST preserve unknown optional minor fields rather than discarding them. |
| HTP-001-R14 | Omnifrons MUST emit `handoff.state` on every lifecycle transition and MUST include the latest state of every open handoff in the bootstrap snapshot; `uncertain` MUST be carried verbatim and MUST NOT be rendered as any other state. |
| HTP-001-R15 | An interruption at prepare, publish, claim, import, switch, or cleanup MUST yield a recoverable state or an explicit `uncertain` state; Omnifrons MUST NOT render false completion. |
| HTP-001-R16 | The source MUST honor a `claimed` observation only when the claim record verifies against the paired receiver key, names a handoff the source published, and carries a claim sequence greater than the last one seen from that receiver device; a replayed claim record MUST be acknowledged as a duplicate and MUST NOT trigger a second release. A claim record naming an `aborted` or superseded handoff MUST render `conflicted` and require human review. |
| HTP-001-R17 | Omnifrons MUST NOT push a handoff commit to a remote it observes as public without an explicit per-handoff confirmation; absent that confirmation, publication MUST remain blocked at `publication-pending`. |
| HTP-001-R18 | The envelope, state vector, claim record, and `handoff.state` payload MUST NOT carry secrets, tokens, device keys, approvals, executable profiles, or raw filesystem paths. |

## Signal mapping

| Condition | HTP-001 state | Public vocabulary | User-facing consequence |
| --- | --- | --- | --- |
| Candidate approved, not yet published | — | `local` | No handoff shown; candidate preview only |
| Composite quiescence proven, publication not started | `prepared` | `prepared` | Publish action offered |
| Publication barriers in progress | `publication-pending` | `publication-pending` | Per-barrier progress shown; claim blocked |
| Memory watermark behind the state vector (RSP-001-R9) | `publication-pending` (blocked) | `publication-pending` | Memory sync proposal surfaced; publication cannot advance |
| Workspace remote observed as public | `publication-pending` (blocked) | `publication-pending` | Explicit per-handoff confirmation required, or abort (D8) |
| Envelope published, no receiver has validated it | `published-unverified` | `published-unverified` | Cross-device claim requires human review until TM-001 is accepted |
| AEC-001 producer verdict `untrusted` | `published-unverified` (reason "producer untrusted") | `published-unverified` | Manual review; never `claimable` |
| Receiver validation passed | `claimable` | `claimable` | Claim action offered |
| Receiver watermark behind the state vector (RSP-001-R9) | `claimable` (withheld) | `uncertain` | Claim blocked; memory sync proposal surfaced |
| Receiver wrote a claim record | `claimed` | `claimed` | Source releases; cleanup scheduled |
| `claimable` past D1 expiry, or superseded by a newer handoff from the same source | — | `stale` | Withdrawn from claim candidates |
| Source mutation after `claimed`, or offline mutation without a verified claim | — | `forked` | Preserved locally; cannot publish as current automatically |
| Receive conflict, divergent history, or dirty receiving worktree | — | `conflicted` | Isolated recovery worktree or human resolution; both sides preserved |
| Second claim observed for the same handoff | — | `conflicted` | Human review required; no last-writer-wins |
| Managed ref or feed transport unreachable | — | `offline` | Last known state shown with its age |
| Terminal error at any step | — | `failed` | Evidence retained for retry or review |
| User cancels before `claimed` | `aborted` | `aborted` | Terminal; evidence retained |
| Quiescence, integrity, authenticity, or fencing unproven at any transition | `uncertain` | `uncertain` | Never rendered as complete; human review |

## Open decisions

| ID | Question | Options | Default proposal |
| --- | --- | --- | --- |
| D1 | How long does a `claimable` handoff remain valid before becoming `stale`? | No expiry; fixed interval; supersession-only (a newer handoff from the same source retires the old one) | 7 days, or immediate supersession by a newer handoff from the same source, whichever comes first |
| D2 | How long may a handoff remain `publication-pending` before the source must surface `uncertain`? | No timeout; fixed total interval; per-barrier interval | 10 minutes total across all barriers |
| D3 | How long does the source retain a `claimed`/`aborted` handoff's managed ref and temp state before cleanup? | No window (prune immediately); fixed window; user-configurable window | 72 hours |
| D4 | Does the same-device model/harness switch retain automatic claim? | Require explicit claim like cross-device; keep the existing automation | Allowed, governed by the existing switch section — source and receiver are the same device and transport is local |
| D5 | How are racing claims resolved before the RSP-001 core's compare-and-set claim records are accepted? | Last-writer-wins; first-writer-wins; `conflicted` plus human review | `conflicted` plus human review; the [RSP-001 core](workspace-roaming-protocol.md) drafts the claim primitive; acceptance pending |
| D6 | Does device-level authenticity reuse AEC-001's producer identity primitive, or does HTP-001 define its own? | Reuse AEC-001 producer identity (pairing, fingerprint, rotation/revocation events); define a separate handoff-specific identity | Yes — reuse AEC-001 producer identity; one paired-key trust set per device, not one per concern |
| D7 | Is `handoff.state` emitted per transition only, or also as a periodic projection? | Per transition only; per transition plus periodic heartbeat; per transition plus bootstrap snapshot inclusion | Per transition, plus inclusion in the AEC-001 bootstrap snapshot so a cold start has no dangling handoff reference |
| D8 | Should Omnifrons publish a handoff commit to a remote it observes as public? | Block; confirm per handoff; allow | Confirm per handoff — never automatic |

## Acceptance evidence and follow-up

- Conformance tests MUST cover: every lifecycle transition and no undocumented state; the composite quiescence gate rejecting a partial proof; publication barrier ordering, including a memory watermark that has not caught up (RSP-001-R9); the complete-vector validation checklist blocking `claimable` on any missing digest; idempotent claim repetition; a racing second claim rendering `conflicted`; release and `forked` detection after `claimed`; authenticity verification and fingerprint display at manual claim; replay/rollback rejection on a non-monotonic handoff ID or unknown predecessor; cleanup pruning scope and the D3 recovery window; envelope major/minor negotiation; and `handoff.state` emission on every transition with `uncertain` carried verbatim.
- Conformance tests MUST additionally cover: claim-record replay rejection and duplicate-claim acknowledgment without a second release (HTP-001-R16); cross-epoch rollback, duplicate-envelope idempotency, and first-contact handling (HTP-001-R10); D3 timer suspension while `forked`, `conflicted`, or under human review, and pruning gated on `source_release = released` or `aborted` (HTP-001-R11); absence of secrets, tokens, device keys, approvals, executable profiles, and raw filesystem paths from the envelope, state vector, claim record, and `handoff.state` payload (HTP-001-R18); and the explicit per-handoff confirmation gate before publishing a handoff commit to a remote observed as public (HTP-001-R17).
- Reproducing the roadmap's pre-alpha exit criterion — interruption at prepare, publish, claim, import, switch, or cleanup — MUST yield a recoverable state or an explicit `uncertain` state, never false completion.
- Debt, not drafted here. Fencing of workspace writers and compare-and-set claim records remain owned by RSP-001; the [RSP-001 core](workspace-roaming-protocol.md) drafts the claim primitive, acceptance pending, so every claim record's `source_release` field carries an explicit `unreleased-unfenced` condition rather than implying the source is fenced, and cleanup of a `claimed` handoff's managed ref is retained accordingly (see Cleanup and recovery window). The attacker model (TM-001) that gates automatic claim is not drafted here; until it is accepted, cross-device claim stays a manual, human-reviewed action. Authenticity relies on AEC-001's producer identity primitive as adopted, not redefined; a change there propagates here without a separate HTP-001 revision.

## Related contracts

- [Target architecture](target-architecture.md) — the Handoff transaction section, invariants 7 and 9, required failure states, and planned artifacts.
- [Workspace roaming and Engram sync protocol](roaming-and-engram-sync.md) — writer epochs, compare-and-set claim records, fencing of workspace writers, divergence detection and recovery, and the RSP-001-R9 watermark gate.
- [Adapter feed event schema](adapter-feed-events.md) — the `handoff.state` wire event and the producer identity/signing primitive (AEC-001 feed profile).
- [Threat model](threat-model.md) (TM-001) — the attacker model that gates automatic claim; drafted, acceptance pending.
- Migration and recovery plan (MRP-001) — backup, restore, and cutover sagas; not yet drafted.
- Update trust architecture (UTA-001) — key rotation and compromise recovery for product updates, distinct from device-level producer identity; not yet drafted.
- [Context Orb specification](context-orb.md) — the checkpoint/handoff gadget and status line presentation of handoff state.
- [Versioning and compatibility](versioning-and-compatibility.md) — the checkpoint/handoff envelope version domain and the public state vocabulary.
- [ADR-0002](adr/0002-desktop-technology-stack.md) — device-local executable approval and handoff commits created through isolated temporary state.
- [Product roadmap](roadmap.md) — the pre-alpha exit criteria this protocol's requirements satisfy.

## References

- [Git references documentation](https://git-scm.com/book/en/v2/Git-Internals-Git-References) — the managed ref namespace `refs/omnifrons/handoffs/<logical-agent-id>/<handoff-id>` follows standard Git reference conventions.
- [Semantic Versioning](https://semver.org/) — the envelope's `major.minor` negotiation follows the same reject-unknown-major, preserve-unknown-minor convention as the compatibility policy's other envelope domains.
- [Target architecture](target-architecture.md) — the Handoff transaction section this document drafts.
- [Workspace roaming and Engram sync protocol](roaming-and-engram-sync.md) — the watermark gate (RSP-001-R9) this protocol consumes.
- [Adapter feed event schema](adapter-feed-events.md) — the `handoff.state` event and the producer identity primitive adopted here.
