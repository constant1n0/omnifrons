# Adapter Feed Event Schema

**Document role:** Typed event catalog and approvals write path for the adapter feed (AEC-001 feed profile)  
**Status:** Draft  
**Normative force:** Non-binding target direction; requirements are acceptance gates, not current guarantees  
**Accountable role:** Project Maintainer  
**Named person:** Unassigned  
**Approver role:** Project Owner  
**Approver named person:** Unassigned  
**Effective date:** None  
**Supersedes:** None

## Document authority

This document drafts the **feed profile** of the planned adapter/event contract (AEC-001): which typed events adapters emit, their common envelope, and the single write path (approval dispositions). Transport semantics — capabilities, epochs, sequence ownership, per-transport ordering, gap detection, deduplication, acknowledgements, replay retention, overflow, cancellation, terminal states, and PTY normalization — remain owned by AEC-001 itself, and the [target architecture](target-architecture.md) governs any conflict. Presentation of this data is owned by the [Context Orb specification](context-orb.md).

The feed is **headless-consumable by design**: dashboards, terminal/SSH clients, and future mobile surfaces are all consumers of the same typed events. Dashboards are one consumer, never the only one.

## Design rules

Binding for every event kind:

1. **Producers report facts; consumers compute verdicts.** An adapter never emits "healthy" or "broken" — it emits runs, declarations, and observations. Health vocabulary (failed, silent-past-interval, waiting, dormant, fossil) is derived consumer-side from the ledger and declarations. This makes the "a stored status field can lie" lesson structural instead of procedural.
2. **Observation, never control.** Every event kind is a read/status projection (invariant 10). The one exception is the approvals write path below, which is deliberately narrow.
3. **Bitemporal honesty.** Every event distinguishes when the fact happened from when the adapter learned it. Events synthesized from polling are marked as synthetic; they are facts about observation, not native occurrences.
4. **Single-scope attribution.** Every event belongs to exactly one context scope. Scopes never blend in the feed, mirroring the presentation rule.
5. **Unknown is a value.** A producer emits `unknown` rather than guessing. A cost of zero without provider verification is `unverified`, never free.
6. **Logical references, never raw paths.** Payloads carry logical artifact IDs and workspace-relative identity per the target architecture's portable-reference rules; devices resolve locally.

## Envelope

Every event shares one envelope:

| Field | Meaning |
| --- | --- |
| `event_id` | Globally unique identifier; the idempotency key consumers deduplicate on |
| `producer_id`, `producer_instance` | The emitting adapter and its running instance; the signing identity |
| `sequence` | Per-producer monotonic counter; a gap is the degraded-with-replay failure state, never silently skipped |
| `scope_id` | The context scope this event is attributed to |
| `kind`, `schema_version` | Typed discriminator and payload version |
| `occurred_at` | When the fact happened, best known |
| `observed_at` | When the adapter learned it |
| `synthetic` | `true` when synthesized from polling over monotonic identifiers rather than a native event stream |
| `observation_interval` | Present when `synthetic` is `true`: the interval the producer polls at — a fact about the producer, never a consumer-computed verdict |
| `payload` | Kind-specific body |
| `signature` | Producer signature over the canonical serialization; replay protection rides `(producer, sequence)` plus `event_id` deduplication |

`observation_interval` is additive to this draft, proposed in [Staleness of the observed model](#staleness-of-the-observed-model); the `ref` object proposed in [Reference resolution](#reference-resolution) is additive on the same terms. Both are new fields with `schema_version` unchanged — the document is a Draft, not yet a compatibility surface.

Delivery is at-least-once with idempotent consumption, per the target architecture. No total order is promised across producers or across independent streams.

## Event catalog

### Scope lifecycle

`scope.declared` · `scope.updated` · `scope.retired`

Payload: `scope_id`, display name, identity class (`harness-bound` | `runtime-managed`), knowledge-root reference, memory-binding reference, delivery-binding reference, agent binding (agent identity plus its harness or runtime), and brand identity (runtime accent, mark reference). Scope descriptors drive the scope selector, the idle globe's runtime identity, and per-scope layouts.

### Model observation

`model.observed`

Payload: effective model, evidence source (`session-config` | `run-record` | `fallback-event`), and the model fallen back from when applicable. The producer never labels an observation `stale`; the consumer computes that verdict from `observed_at`, the evidence source, and — when `synthetic` is `true` — the envelope's `observation_interval`, per [Staleness of the observed model](#staleness-of-the-observed-model). A stale or absent observation renders as unknown — never a guessed brand.

### Run ledger

`run.started` — `run_id`, executor, trigger (`schedule` | `human` | `agent` | `wake`), expected bound when declared.

`run.finished` — `run_id`, verdict (`succeeded` | `failed` | `cancelled` | `partial`), error reference, cost (tokens, currency, `verified` flag), artifact references.

Health verdicts derive from the ledger using the last **conclusive** run; `cancelled` is excluded as evidence of health or sickness.

### Executor declaration

`executor.declared`

Payload: executor identity, schedule or interval, skip-empty-tick policy, deliberate-dormancy flag, owner. Combining declarations with the run ledger, the consumer derives the health vocabulary: broken states (failed last conclusive run, silent past its own declared interval, quota-exhausted) versus healthy-quiet states (waiting, dormant, never-started). An error older than the dormancy start renders as a fossil — wake before repairing.

### Usage

`usage.pool` — shared quota pool: pool identity, the executors and accounts sharing it, billing window, consumed, limit, reset time, `verified` flag. Pools are first-class because a per-executor cap cannot protect an account-wide pool.

`usage.metered` — pay-per-token spend: provider, window, tokens, cost, `verified` flag.

### Approvals (feed side)

`approval.requested` — approval identity, category (domain proposal, executable approval, itemized elevation, or runtime-defined), summary reference, requested-by identity, the identity the disposition will mutate as, expiry time, and the **explicit expiry disposition**. No item may be created without a bounded outcome: work left undisposed can re-wake its executor indefinitely.

`approval.disposed` — approval identity, disposition (`approved` | `rejected` | `expired`), disposed-by identity, effect state (`applied` | `uncertain`), evidence reference. Expiry emits a disposal event like any other outcome — never a silent drop.

### Handoff and sync

`handoff.state` — handoff identity, state per the [handoff transaction protocol](handoff-transaction-protocol.md) (HTP-001), device references. `uncertain` is carried verbatim and never masked.

`sync.state` — data plane (`knowledge` | `memory` | `delivery`), control-plane instance identity and role, replication and authority state. Multiple instances per plane are modeled first-class; a single authority is never assumed.

### Alerts

`alert.raised` — severity, category, body reference, and an `out_of_band_delivered` flag. The feed carries alerts for display, but alerting never depends on a dashboard being open: the producer confirms out-of-band dispatch or truthfully marks it failed.

### Producer identity

`producer.key_rotated` — `producer_id`, `producer_instance`, new key fingerprint, overlap window; signed with the current key.

`producer.key_revoked` — `producer_id`, `producer_instance`, revoked key fingerprint; signed with the current key.

Both events carry the paired-key trust set a device holds per producer instance, defined in [Producer identity and key distribution](#producer-identity-and-key-distribution).

## Staleness of the observed model

Design rule 1 applies to `model.observed` exactly as to every other event: the producer reports a fact, never a verdict. It never emits "stale" — it states `observed_at`, the evidence source already in the catalog (`session-config` | `run-record` | `fallback-event`), and, when the observation is synthesized by polling, the envelope's `observation_interval`: the interval the producer polls at, a fact about the producer rather than about the observation's age.

The consumer computes one of three verdicts per scope: `fresh`, `stale`, or `unknown` — `unknown` is a value, not an absence of one. The horizon differs by evidence source:

- A synthetic observation (`synthetic` is `true`) is `fresh` while its age is at most 2 × `observation_interval`; past that horizon it is `stale`.
- An event-sourced observation (`session-config`, `fallback-event`, or `run-record`) never goes stale by time alone. It stays `fresh` until superseded by a newer observation for the same scope, or invalidated when the scope retires (`scope.retired`) or the run it came from finishes (`run.finished`) — at which point the verdict is `unknown` until the next observation arrives.
- A producer that declares no interval falls back to the shared default horizon proposed here: 15 minutes, the same figure [RSP-001's D1](roaming-and-engram-sync.md) uses for the continuous posture's staleness threshold, so a user reads one notion of "recent" across memory and model.

Rendering follows the Orb's honest-status rule: `stale` renders as unknown, with the age shown, never a guessed brand. Consumers never extrapolate a model beyond the last observed event in a fallback chain.

Status: proposed in this draft; decided by the owner.

## Reference resolution

Two reference classes appear in the feed. Feed-internal references — `scope_id`, `run_id`, `approval_id`, `request_id`, and a producer's own `(producer_id, sequence)` — resolve from the feed itself; every entity they name is either present in the current snapshot or arrives on the feed like any other event. External references — error references, the artifact references in `run.finished`, and alert payload references — are `ref` objects: `{ kind, id, locator }`, where `locator` is logical, never a raw path (design rule 6). A `locator` is either a Context Catalog identity, per the target architecture's portable-reference rules, or an adapter-scoped locator naming the producer that privately holds the artifact.

Resolution order for external references: the Context Catalog first, for anything with workspace-relative identity; then adapter-local retrieval through AEC-001 transport capabilities, for producer-private artifacts such as raw logs and run records; if neither resolves, the reference renders as `unresolved` — a value, shown with its logical `id`, never a broken link and never a guess. `unresolved` differs from `unknown`: the reference exists and names something real, it just cannot be dereferenced here.

Ordering interacts with resolution. At-least-once delivery carries no cross-producer order, so a consumer can see an event before the feed entity it references. Consumers hold such events in a bounded per-producer buffer — proposed at 100 events or 30 seconds, whichever comes first — then render them with their feed-internal references `unresolved` rather than blocking the feed. A bootstrap snapshot closes this gap at cold start: it carries every feed entity referenced by the events it includes — declared scopes, open approvals, the latest `model.observed` per scope, running runs — so a cold start has no dangling feed-internal reference. A snapshot that cannot guarantee that closure says so, and the consumer renders the affected references `unresolved`.

This proposal records one division of labor: the Context Catalog is the resolver of record for artifacts and errors carrying workspace identity; the adapter-local store serves producer-private ones through AEC-001 transport capabilities; the feed profile owns only the `ref` shape and the `unresolved` state, not the resolvers themselves.

Status: proposed in this draft; decided by the owner.

## Producer identity and key distribution

This corrects the open question's current pointer: producer identity does not ride the update trust architecture (UTA-001). UTA-001 owns software update trust — keys, metadata, freshness, anti-rollback, and compromise recovery for the product's own updates, per the target architecture's planned artifacts. A producer's identity is adapter identity, already governed by [ADR-0002](adr/0002-desktop-technology-stack.md)'s device-local executable approval, which binds identity evidence — digest/signature, version, adapter, transport, plugin inventory, security-relevant configuration — at approval time. The threat model (TM-001) owns the attacker model this identity is evaluated against.

First contact is pairing. When a producer — an adapter instance — is approved on a device, the device records the producer's public key fingerprint as part of that same device-local approval and shows it to the user at approval time: trust on first use with an explicit approval, in the style of a known-hosts entry. Keys are scoped per `producer_instance`; a producer running on several hosts has several instances and a fingerprint for each.

Verification checks every event's `signature` against the paired key for its `(producer_id, producer_instance)`. An event that fails verification, or arrives from an unpaired instance, renders as `untrusted` — a value — and is excluded from health evidence; it is never silently dropped and never rendered as if trusted. The approvals write path never accepts a disposition from an untrusted producer.

Rotation runs through the feed itself: a producer announces a new key with `producer.key_rotated`, signed with the current key and carrying the new fingerprint and an overlap window; the device accepts either key during the window, then only the new one. Revocation happens by user action — removing the producer's approval — or by producer action — emitting `producer.key_revoked`, signed with the current key. After either, events under that key render `untrusted`. Compromise recovery is re-pairing: a fresh approval with a fresh fingerprint. No central key server or PKI exists in this first version.

Trust stays device-local, per ADR-0002: each device pairs its producers independently. A headless consumer on the same device — a terminal, an SSH session — shares that device's trust set; nothing about producer trust travels as portable state. A second device shows the same fingerprint for the user to compare by hand.

Status: proposed in this draft; decided by the owner.

## Approvals write path

Everything above is projection. The **only** mutation an adapter accepts from an Omnifrons surface is the approval disposition:

Request: `approval_id`, disposition, acting-identity proof, `request_id`.

Rules:

- **Identity-bound.** The producer applies the effect as the correct actor named at request time — never as a shared service identity.
- **Idempotent.** A repeated `request_id` acknowledges the prior outcome; it never applies twice.
- **Bounded.** Every approval item reaches a terminal disposition — approved, rejected, or expired. The queue never leaves an item open as a side effect.
- **Effect-verified.** `approval.disposed` with effect `applied` is emitted only after the producer verifies the effect; otherwise the effect is `uncertain` and surfaced as such.
- **Authorization is human and local.** The right to dispose comes from an Omnifrons-side human action under device-local identity; it is never inferred from feed contents.

## Bootstrap and replay

A consumer bootstraps from a **snapshot**: current scope descriptors, executor declarations, open approvals, and latest observations, stamped with a per-producer sequence watermark. Events after the watermark replay on top. A sequence gap or replay-retention overflow places the consumer in the degraded-with-replay state from the target architecture's required failure table; the consumer re-snapshots rather than pretending continuity.

## Consumer surface mapping

| Gadget / surface | Consumes |
| --- | --- |
| Sessions monitor | `run.*`, `model.observed` |
| Routines board | `executor.declared` + run ledger (derived health vocabulary) |
| Approval queue | `approval.*` |
| Usage and spend | `usage.pool`, `usage.metered` |
| Sync health, checkpoint/handoff | `sync.state`, `handoff.state` |
| Orb core, idle globe, scope selector | `scope.*`, `model.observed` |
| Out-of-band alerting (not a gadget) | `alert.raised` is a mirror; the primary channel is independent |

## Open questions

| Topic | Status |
| --- | --- |
| Staleness threshold for `model.observed` | Proposed in this draft ([Staleness of the observed model](#staleness-of-the-observed-model)); owner decision pending |
| Error and artifact reference resolution (Context Catalog vs adapter-local store) | Proposed in this draft ([Reference resolution](#reference-resolution)); owner decision pending |
| Producer key distribution and rotation | Proposed in this draft ([Producer identity and key distribution](#producer-identity-and-key-distribution)); owner decision pending |
| Payload schema evolution rules | Open; rides the [compatibility policy](versioning-and-compatibility.md) |

## References

- [Target architecture](target-architecture.md) — AEC-001 scope, transport and events, required failure states, portable references.
- [Context Orb specification](context-orb.md) — fleet health semantics, dashboard set, context scopes, observed model.
- [ADR-0002](adr/0002-desktop-technology-stack.md) — device-local executable approval and the identity evidence it binds, which governs producer identity.
- Threat model (TM-001) — the planned artifact owning the attacker model producer verification is evaluated against; see [target architecture](target-architecture.md), planned assurance artifacts.
- [Workspace roaming and Engram sync protocol](roaming-and-engram-sync.md) — D1's 15-minute continuous-posture staleness threshold, reused here as the default observation horizon.
