# Workspace Roaming and Engram Sync Protocol

**Document role:** Workspace roaming and Engram memory synchronization protocol (RSP-001)  
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

This document drafts the **memory synchronization profile** of the planned workspace roaming and Engram sync protocol (RSP-001): how the memory plane stays continuous across devices under each supported Engram sync profile, what a device may truthfully claim about its memory, and which product behaviors make that claim observable. Workspace writer coordination — writer epochs, compare-and-set claim records, fencing of workspace writers, divergence detection, and recovery — and the Git Sync to Cloud cutover are drafted in the [workspace roaming protocol](workspace-roaming-protocol.md) (RSP-001 core). The [target architecture](target-architecture.md) governs any conflict; the handoff lifecycle and state vector remain owned by the [handoff transaction protocol](handoff-transaction-protocol.md) (HTP-001); presentation is owned by the [Context Orb specification](context-orb.md).

## Purpose and scope

RSP-001 governs continuity of the **memory plane**: the curated observations and pointers Engram holds for a memory namespace, and whether a given device holds the same set the namespace's authority holds.

In scope:

- detection and labelling of the sync profile a namespace actually uses;
- the publish/receive protocol per profile and what "current" means under each;
- watermarks, staleness, failure states, and their mapping to the public state vocabulary;
- credential custody rules for the Cloud profile;
- product requirements that make replication observable.

Out of scope:

- the knowledge plane — always-local Markdown and tiered heavy assets are decided by [ADR-0003](adr/0003-local-markdown-and-tiered-assets.md);
- the delivery plane — OpenSpec content travels with the workspace through Git;
- roaming the live Engram SQLite database — it is device-local and never a roaming payload (invariant 3, explicit non-goal);
- requiring Engram Cloud — local Engram remains required, Cloud remains optional, and Git Sync is the default profile;
- Engram database internals — only supported CLI/MCP contracts are used.

## Problem statement

A maintainer works on a laptop through the night and pushes the resulting Git commits. The Engram observations recorded during that session stay on the laptop: the namespace is configured for the Cloud profile, but replication is manual, and no device has autosync enabled. The next morning a workstation pulls the commits and has the code, but no memory of the decisions behind it. Engram's own status commands could have proven the gap — the acknowledged watermark had not moved, and the workstation reported zero pending imports — but nothing in the product surfaced it. Recovery needed a manual push on the laptop, a manual import on the workstation, and, to prevent a repeat, a supervised daemon with autosync on each device.

Nothing failed. Every command behaved as documented. The gap existed because the product assumed memory follows the workspace, when the two planes have different transports, different authorities, and different watermarks.

The principle this yields, binding for everything below: **replication is never assumed; it is observed.** A device MUST NOT present a memory namespace as current unless a watermark read from a supported Engram surface proves it. Absence of evidence is `uncertain`, never "in sync".

## Definitions

| Term | Meaning |
| --- | --- |
| Device | One installation of Omnifrons with its own local Engram store. Device identity is device-local and is never derived from synchronized content. |
| Memory namespace | One Engram project: the unit of enrollment, export, import, and authority. A context scope's memory binding names exactly one namespace. |
| Sync profile | The mechanism through which a namespace replicates: Engram Git Sync or Engram Cloud. A namespace selects exactly one; the selection is observed from the Engram runtime, never trusted from an Omnifrons setting alone. |
| Chunk | An immutable, content-hashed (SHA-256 prefix), gzipped JSONL export of new memories written under `.engram/chunks/` in the memory repository. Chunks are append-only and never rewritten. |
| Manifest | `.engram/manifest.json`, the index of chunks a memory repository carries. |
| Watermark | The evidence of replication position. Git Sync: the set of chunk digests published in the manifest (publish side) and imported locally (receive side). Cloud: per-target sequence positions that this document names `enqueued` (written locally), `acknowledged` (confirmed received by the server), and `pulled` (received locally from the server). These are RSP-001 names for positions the Engram runtime keeps per sync target in its current implementation; they MUST be read through a supported Engram surface, never from its database schema. |
| Target | One replication endpoint of a namespace on a device: the memory repository under Git Sync, one enrolled server under Cloud. Watermarks are kept per target; invariant 7 requires that exactly one target is the active authority. |
| Authority | The single observed active sync authority for a namespace (invariant 7): the memory repository under Git Sync, the enrolled server under Cloud. |
| Quiescence | No in-flight mutation of the namespace on the device: no harness session writing memories, no export or autosync push in progress. |
| Fencing | Preventing a previous authority or writer from accepting mutations after a cutover. Required before any profile switch; not drafted here. |
| Continuity | A device's per-namespace verdict on whether its memory equals the authority's, computed only from watermarks. |
| Posture | Whether continuity is maintained continuously (supervised autosync), manually (user-run publish/receive), or is degraded (a continuous posture is claimed but its preconditions are not observed). |

Engram stores timestamps in UTC; the product compares in UTC and renders local time with the zone visible.

## Profile A — Engram Git Sync

The default profile. No server is involved; the transport is a Git repository designated as the **memory repository** for the namespace.

### Operational protocol

| Step | Where | What happens | Who runs it |
| --- | --- | --- | --- |
| Publish | Source device | `engram sync --project <name>` exports memories not yet exported as one new chunk under `.engram/chunks/` and updates `.engram/manifest.json` | User, or a scheduler the user configured |
| Transport | Memory repository | The chunk and manifest are committed and pushed to the memory repository's remote | User, or a scheduler the user configured |
| Receive | Receiving device | The memory repository is pulled; `engram sync --import` imports every manifest chunk not yet imported | User, or a scheduler the user configured |
| Verify | Any device | `engram sync --status` reports local versus remote chunk counts and pending imports | Omnifrons, on read |

Omnifrons MAY propose any of the first three steps and MAY run them on explicit user action. It MUST NOT run them silently and MUST NOT run them on a schedule the user did not configure: the system proposes, the user disposes.

### Invariants

- Chunks are append-only and immutable; a chunk digest identifies its content, so importing the same chunk twice is idempotent.
- Each device writes its own chunks. Concurrent devices add files rather than editing shared ones, so chunk files never merge-conflict. The manifest is the one shared file; an unresolved manifest merge conflict is `conflicted` and blocks receive until a human resolves it.
- The memory repository is never the public product repository. If the project's repository is public, the namespace MUST use a separate private repository.
- `<private>…</private>` spans are stripped by Engram before storage and therefore never enter a chunk. Engram `scope` is not a privacy boundary; the enrollment dry-run remains the inclusion-set check.

### What "current" means

A device is `current` for a Git Sync namespace only when both hold:

1. every chunk listed in the manifest at the fetched remote head has been imported locally (pending imports = 0), and
2. the local export watermark is published: the newest local observation has been exported into a chunk that is present in the manifest at the remote head.

Condition 2 cannot be read from Engram alone. Omnifrons records the export watermark when it runs publish on the user's behalf; when publish ran outside the product, the export watermark is `uncertain` until the next product-run publish or an explicit user confirmation.

### Staleness and failure detection

| Observation | Continuity | Notes |
| --- | --- | --- |
| Pending imports > 0 | `stale` | Receive proposal offered |
| Local observations newer than the export watermark | `publication-pending` | Publish proposal offered |
| Remote unreachable | `offline` | Last known watermark shown with its age |
| Manifest merge conflict | `conflicted` | Receive blocked; both sides preserved |
| `engram sync --status` fails or the CLI is absent | `uncertain` | Never rendered as current |
| Memory repository remote observed as public | `authority-conflict` | Publish blocked pending human decision |

### Automation posture

Git Sync has no built-in scheduler. Its posture is `manual`; Omnifrons does not detect an external scheduler and reports `manual` regardless of one. Whether the product should offer its own scheduler is open decision D2.

## Profile B — Engram Cloud

An optional self-hosted replication server. Nothing in this profile changes the rule that the live SQLite database is device-local.

### Enrollment and explicit replication

- Each namespace is enrolled on each device separately with `engram cloud enroll <project>`. The server URL is configured per device with `engram cloud config --server <url>`, which persists it in `~/.engram/cloud.json`; neither the enrollment nor the URL is portable.
- Explicit push is `engram sync --cloud --project <name>`; explicit pull is `engram sync --import --cloud --project <name>`. Both are manual by default.
- Per-device sync state exposes, per target, the sequence positions RSP-001 calls `enqueued`, `acknowledged`, and `pulled` (see Definitions).
- Enrollment governs push only. A device pushes exactly the namespaces it has enrolled; nothing more. Pull is governed entirely by the server's own project allowlist, not by any device's enrollment: when that allowlist has no restriction, every device authenticated with the same credential pulls every namespace any device has ever pushed, regardless of that device's own scope roots or enrollment. Scope roots (R14) and enrollment (R13) are therefore an upload boundary; RSP-001 does not currently define a download boundary, because the Engram versions this profile is checked against expose no per-device pull filter — only a server-wide allowlist. A deployment that wants Cloud to behave as several isolated pools, rather than one shared memory pool across every device holding the credential, needs a server-side allowlist per credential, which is not available today (see D8).

### Continuous posture

A device MAY claim posture `continuous` for a Cloud namespace only when all of the following are observed:

1. the local daemon (`engram serve`) is running under a per-user supervisor, for example a systemd user unit or a launchd agent — agent plugins do not start it;
2. the daemon started with `ENGRAM_CLOUD_AUTOSYNC=1`, `ENGRAM_CLOUD_TOKEN`, and `ENGRAM_CLOUD_SERVER` in its environment — a missing token or server disables autosync with an error;
3. the namespace is enrolled on the device — autosync covers every enrolled namespace on that device, and only those.

If any condition is not observed, posture is `degraded`, not `manual`: the device claimed continuity and cannot prove it.

### Credential custody

The bearer token is supplied through the daemon's environment at runtime and is device-local. It MUST be read from a mode `0600` environment file owned by the user, and MUST NOT appear in the unit or plist definition, in any Omnifrons setting, in portable state, in a checkpoint or startup brief, or in a support bundle. Custody is labelled `OS-secret-store`, `Engram-managed-local`, or `unprovisioned` per the target architecture; Omnifrons detects and labels, never copies.

### Phases and reason codes

| Engram phase | Posture | Continuity contribution |
| --- | --- | --- |
| `idle`, `healthy` | `continuous` | From watermarks only |
| `pushing` | `continuous` | `publication-pending` until acknowledged |
| `pulling` | `continuous` | `stale` until pulled reaches the server head |
| `push_failed`, `pull_failed` | `continuous` | By reason code below |
| `backoff` | `continuous` | The last failure's state persists, labelled retrying |
| `disabled` | `degraded` | From the last read watermarks; age shown |

| Reason code | Continuity | Required behavior |
| --- | --- | --- |
| `transport_failed` | `offline` | Last acknowledged watermark and its age shown |
| `auth_required` | `unprovisioned-secret` | Custody label shown; no token entry inside Omnifrons |
| `policy_forbidden` | `failed` | Server policy shown verbatim; human review |
| `internal_error` | `uncertain` | Diagnostics captured; never rendered as current |
| `upgrade_paused` | posture `degraded` | Continuity from the last watermarks; age shown |

### What "current" means

A device is `current` for a Cloud namespace only when `acknowledged` equals `enqueued` (everything written here has been confirmed by the server) and `pulled` equals the server head for that namespace (everything the server holds is here). A `healthy` phase alone is not currency; the watermarks are.

### Multiple instances

Sync health models multiple control-plane instances per plane first-class. Two servers, or a server and a memory repository, observed for one namespace is `authority-conflict`, unchanged from the target architecture: operation blocks until the user resolves which authority is active.

## Product requirements

Each requirement is an acceptance gate with a testable condition.

| ID | Requirement |
| --- | --- |
| RSP-001-R1 | Omnifrons MUST detect the sync profile of each memory namespace from the supported Engram runtime (enrollment state, cloud configuration, memory repository presence) and label it per namespace. A profile selected in Omnifrons settings that disagrees with the observed profile is `authority-conflict`. |
| RSP-001-R2 | Omnifrons MUST read per-device watermarks and the time of the last replication for each namespace through supported Engram surfaces, labelling the source. When the read fails, continuity is `uncertain`. |
| RSP-001-R3 | The Sync health gadget MUST show continuity and posture per namespace per device, and `sync.state` (plane `memory`) MUST carry the same values; `handoff.state` MUST carry the watermark gate result from R9. `uncertain` is carried verbatim; `current` is emitted only with the proving watermark attached. |
| RSP-001-R4 | Omnifrons MUST raise a staleness warning when `publication-pending` or `stale` persists beyond the threshold in D1, emitted as `alert.raised` so it reaches out-of-band channels. |
| RSP-001-R5 | Omnifrons MAY offer "Publish memory", "Receive memory", and "Sync now" actions. Each runs only on explicit user action, invokes the supported Engram CLI with explicit argv and a bounded environment, and reports the resulting watermark. None runs silently or on a product-initiated schedule. |
| RSP-001-R6 | Omnifrons MUST NOT store, copy, display, or roam Engram Cloud tokens. Token-bearing configuration is detected and labelled; absence is `unprovisioned-secret`. |
| RSP-001-R7 | For a Cloud namespace, Omnifrons MUST detect an absent daemon or disabled autosync and report posture `degraded`. It MUST NOT silently relabel the namespace as `manual`, and it SHOULD offer the supervision steps as a proposal. |
| RSP-001-R8 | Omnifrons MUST refuse automated switching between profiles. An externally changed profile renders as `authority-conflict` with the previous authority shown, until the cutover artifact under follow-up is accepted. |
| RSP-001-R9 | The handoff claim checklist MUST include "memory watermark acknowledged". The publishing device MUST NOT advance past `publication-pending` while the namespace's acknowledged (Cloud) or published (Git Sync) watermark is behind the state vector, and the receiving device MUST NOT claim while its pulled (Cloud) or imported (Git Sync) watermark is behind the vector. |
| RSP-001-R10 | Before proposing a Git Sync publish, Omnifrons MUST verify that the memory repository's remote is not the public product repository and MUST warn on any remote whose visibility cannot be verified. |
| RSP-001-R11 | Continuity MUST be computed per namespace and per device. No aggregate "all synced" indicator may hide a namespace that is not `current`. |
| RSP-001-R12 | Every product-run publish, receive, or sync MUST be recorded in a device-local, reconstructible ledger: command, exit status, watermark before and after, and UTC time. The ledger is machine-local and never a roaming payload. |
| RSP-001-R13 | **Registered scopes imply enrollment.** When a context scope carrying an agent binding is registered on a device (its knowledge root, memory binding, and agent binding as defined in the [Context Orb specification](context-orb.md)), Omnifrons MUST ensure the namespace named by that scope's memory binding is enrolled for the selected sync profile on that device: Cloud, `engram cloud enroll <project>`; Git Sync, inclusion in publish and receive. Registering the scope is the explicit user action that authorizes this enrollment under R5 and Profile A; the registration flow MUST state that enrollment follows, and no further per-action confirmation is required. Enrollment is not optional per user; the only way to stop enrolling a registered scope's namespace is to remove the scope. Omnifrons MUST verify enrollment on every start and MUST report a missing enrollment as posture `degraded` with a repair proposal. Omnifrons MUST NOT enroll a namespace that is not bound to a registered scope — incidental projects created by running a harness elsewhere stay out of scope — and MUST surface such out-of-scope activity when the Engram runtime reports it as blocking replication: background replication acknowledges only the mutations it pushes, so pending mutations of un-enrolled namespaces stay pending and, whenever they are the only pending mutations on the device, the pull step is skipped until they are enrolled, acknowledged as local-only, or removed. |
| RSP-001-R14 | **Scope roots.** A device MAY declare memory scope roots (directories under which namespaces are eligible for replication) and a require-git rule (only namespaces backed by a Git repository are eligible). Omnifrons MUST treat the namespace of a registered scope as always in scope regardless of roots, and MUST NOT silently replicate a namespace outside the roots. |

## Signal mapping

The public vocabulary column uses the states in the [compatibility policy](versioning-and-compatibility.md) plus the target architecture's required failure states.

| Engram signal | RSP-001 state | Public vocabulary | User-facing consequence |
| --- | --- | --- | --- |
| Git: pending imports = 0 and export watermark published | `current` | `current` (D5) | Namespace shown current with watermark and age |
| Git: pending imports > 0 | `stale` | `stale` | Receive proposal; handoff claim blocked |
| Git: unexported local observations | `publication-pending` | `publication-pending` | Publish proposal; handoff publication blocked |
| Git: remote unreachable | `offline` | `offline` | Last watermark shown with age |
| Git: manifest merge conflict | `conflicted` | `conflicted` | Receive blocked; both sides preserved |
| Git: status unreadable | `uncertain` | `uncertain` | Never current; diagnostics |
| Cloud: acknowledged = enqueued and pulled = server head | `current` | `current` (D5) | As above |
| Cloud: acknowledged < enqueued | `publication-pending` | `publication-pending` | Sync-now proposal; handoff publication blocked |
| Cloud: pulled < server head | `stale` | `stale` | Sync-now proposal; handoff claim blocked |
| Cloud: `transport_failed` | `offline` | `offline` | Last watermark shown with age |
| Cloud: `auth_required` | `unprovisioned-secret` | `unprovisioned-secret` | Custody label; no in-product token entry |
| Cloud: `policy_forbidden` | `failed` | `failed` | Policy shown; human review |
| Cloud: `internal_error` | `uncertain` | `uncertain` | Never current; diagnostics |
| Cloud: daemon absent or phase `disabled` | `uncertain` (posture `degraded`) | `uncertain` | Supervision proposal; continuity claim withdrawn until a watermark is read |
| Either: local observations written while `stale` | `forked` | `forked` | Preserved locally, unioned on receive; handoff claim blocked until both watermarks are current |
| Either: two authorities observed | `authority-conflict` | `authority-conflict` | Operation blocked pending human resolution |
| Either: observed profile disagrees with the selected one | `authority-conflict` | `authority-conflict` | As above |

## Open decisions

| ID | Question | Options | Default proposal |
| --- | --- | --- | --- |
| D1 | Staleness threshold | Fixed interval; per-posture interval; session-boundary trigger | `continuous`: 15 minutes past the last acknowledged replication while newer observations exist; `manual`: at session end and at handoff preparation, plus 24 hours; shared with the Orb specification's staleness decision |
| D2 | Product-run scheduler for Git Sync | None (external only); opt-in product scheduler with explicit interval; session-end proposal | Opt-in scheduler deferred; a session-end proposal ("publish memory now?") ships first |
| D3 | Does a Cloud namespace require autosync to be labelled `continuous`? | Yes; no, manual sync within D1 counts | Yes — `continuous` is a claim about the future, and only a supervised daemon can make it |
| D4 | Treatment of Cloud devices with autosync disabled | `degraded`; `manual`; block | `degraded` with a supervision proposal; never silent `manual` |
| D5 | Is `current` added to the public state vocabulary? | Add `current`; express as absence of pending/stale states | Add `current`, emitted only with a proving watermark; rides the compatibility policy |
| D6 | Secret-detection heuristics for chunks before publish | None (rely on `<private>`); known-pattern scan with warning; block on detection | Known-pattern scan with warning before the publish proposal; detection cannot prove absence and residual risk is disclosed |
| D7 | Handling of a manifest merge conflict | Block for human resolution; product-assisted union | Block for human resolution; union tooling deferred |
| D8 | Cloud pull scope, given enrollment governs push only | One shared memory pool per credential (accept, no warning); the same, with a one-time warning when the server allowlist has no restriction; per-device pull scoping (needs an upstream server-side allowlist per credential, not available today) | One shared memory pool per credential, accepted without a warning — the owner was asked directly and chose it. Revisit if Engram ever exposes a per-device or per-credential pull filter. |

## Acceptance evidence and follow-up

- Conformance tests MUST cover profile detection, watermark reading under both profiles, every row of the signal mapping, the `degraded` posture, token non-custody, refusal of profile switching, the handoff watermark gate, enrollment of every registered scope's namespace on start (R13), and non-enrollment of namespaces outside the declared scope roots (R14).
- Reproducing the problem statement — a session on one device, a Git push, no memory replication — MUST render `publication-pending` on the source device and `stale` on the receiving device, and MUST block the handoff claim.
- Debt, not drafted here — Omnifrons-native autosync onboarding. This profile only detects the continuous posture and proposes it (R7, D3, D4); nothing in the product provisions it for a user. Missing for every user: a per-device supervisor for the Engram daemon with the three autosync variables loaded from a device-local credential file, enrollment of registered scopes on start (R13), and a supported way to keep out-of-scope activity local-only without blocking replication. The last item depends on Engram exposing a supported surface for acknowledging un-enrolled pending mutations; until then any implementation would have to touch Engram's database directly, which this profile forbids. Device tooling used by maintainers to reach that posture by hand is operations, not product code, and does not discharge this item.
- Workspace writer epochs, compare-and-set claim records, fencing, divergence detection, recovery, and the Git Sync to Cloud cutover are drafted in the [workspace roaming protocol](workspace-roaming-protocol.md) (RSP-001 core).

## Engram upstream tracking

RSP-001 depends on Engram runtime behavior that changes between releases. A later major line may, for example, add a mutation disposition state and a store-level helper that acknowledges un-enrolled pending mutations, neither of which the line this draft was checked against provides (see References); such a change alters what R13 can observe and repair. Therefore:

- Omnifrons MUST declare the supported Engram version range per release, under the [compatibility policy](versioning-and-compatibility.md) (Engram runtime compatibility).
- Omnifrons MUST isolate every schema-dependent or version-dependent Engram behavior behind a single adapter; product code outside it sees RSP-001 states only.
- Omnifrons MUST re-validate this profile against each new Engram minor or major release before declaring that release supported; until then the release is `detected`, not `supported`, and continuity read through it is `uncertain`.
- Omnifrons MUST NOT read Engram's SQLite schema from product code; only supported CLI/API surfaces are used, as the Watermark definition already requires.

## Related contracts

- [Target architecture](target-architecture.md) — Engram authority, handoff state vector, required failure states, planned artifacts.
- [Adapter feed event schema](adapter-feed-events.md) — `sync.state` and `handoff.state` events (AEC-001 feed profile).
- [Handoff transaction protocol](handoff-transaction-protocol.md) (HTP-001) — the lifecycle and state vector the watermark gate attaches to.
- [ADR-0002](adr/0002-desktop-technology-stack.md), Engram subsection — supported contracts and credential custody.
- [ADR-0003](adr/0003-local-markdown-and-tiered-assets.md) — knowledge-plane locality, out of scope here.
- [Versioning and compatibility](versioning-and-compatibility.md) — public synchronization states and Engram runtime compatibility.
- [Voice interaction contract](voice-interaction-contract.md) (VOC-001) — a spoken staleness warning or sync proposal follows the same proposal-only and text-fallback rules; drafted, acceptance pending.
- [Governance](governance.md) (GOV-001) — roles, approvals, exceptions, and evidence retention this document's acceptance relies on; drafted, acceptance pending.

## References

- Engram documentation, Git Sync section (`DOCS.md` in the upstream repository, <https://github.com/Gentleman-Programming/engram>) — chunked export/import, content-hashed chunks, manifest, status, and the `<private>` redaction spans.
- Engram documentation, Cloud Autosync section (same source, together with `docs/engram-cloud/README.md`) — enrollment, explicit replication, daemon environment, phases, and reason codes. Statements in this draft were checked against Engram CLI 1.20.0.
- [Context Orb specification](context-orb.md) — Sync health gadget and memory binding per context scope.
