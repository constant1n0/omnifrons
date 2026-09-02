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
| `payload` | Kind-specific body |
| `signature` | Producer signature over the canonical serialization; replay protection rides `(producer, sequence)` plus `event_id` deduplication |

Delivery is at-least-once with idempotent consumption, per the target architecture. No total order is promised across producers or across independent streams.

## Event catalog

### Scope lifecycle

`scope.declared` · `scope.updated` · `scope.retired`

Payload: `scope_id`, display name, identity class (`harness-bound` | `runtime-managed`), knowledge-root reference, memory-binding reference, delivery-binding reference, agent binding (agent identity plus its harness or runtime), and brand identity (runtime accent, mark reference). Scope descriptors drive the scope selector, the idle globe's runtime identity, and per-scope layouts.

### Model observation

`model.observed`

Payload: effective model, evidence source (`session-config` | `run-record` | `fallback-event`), and the model fallen back from when applicable. The consumer applies the staleness threshold (open decision, shared with the Orb specification); a stale or absent observation renders as unknown — never a guessed brand.

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

`handoff.state` — handoff identity, state per the handoff transaction protocol (HTP-001), device references. `uncertain` is carried verbatim and never masked.

`sync.state` — data plane (`knowledge` | `memory` | `delivery`), control-plane instance identity and role, replication and authority state. Multiple instances per plane are modeled first-class; a single authority is never assumed.

### Alerts

`alert.raised` — severity, category, body reference, and an `out_of_band_delivered` flag. The feed carries alerts for display, but alerting never depends on a dashboard being open: the producer confirms out-of-band dispatch or truthfully marks it failed.

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
| Staleness threshold for `model.observed` | Open; shared with the Orb specification's open decision |
| Error and artifact reference resolution (Context Catalog vs adapter-local store) | Open |
| Producer key distribution and rotation | Open; rides the update trust architecture (UTA-001) |
| Payload schema evolution rules | Open; rides the [compatibility policy](versioning-and-compatibility.md) |

## References

- [Target architecture](target-architecture.md) — AEC-001 scope, transport and events, required failure states, portable references.
- [Context Orb specification](context-orb.md) — fleet health semantics, dashboard set, context scopes, observed model.
