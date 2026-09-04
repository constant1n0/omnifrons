# Omnifrons Design Documentation

Omnifrons is currently in the design phase. These documents describe target direction and acceptance gates; they do not claim that the application or its guarantees are implemented.

## Reading order

1. [Target architecture](target-architecture.md) — product boundary, invariants, trust boundaries, components, handoff, memory, and storage.
2. [Product roadmap](roadmap.md) — maturity stages and evidence required to advance.
3. [Versioning and compatibility](versioning-and-compatibility.md) — intended public contracts, migration, and support policy.
4. [Governance](governance.md) — role catalog, assignments, approvals, status workflow, exceptions, evidence retention, and support-matrix authority (GOV-001).
5. [Context Orb presentation specification](context-orb.md) — dashboard visual structure, theming, interaction, and usage widgets.
6. [Adapter feed event schema](adapter-feed-events.md) — AEC-001 feed profile: typed event catalog, approvals write path, producer identity.
7. [Workspace roaming and Engram sync protocol](roaming-and-engram-sync.md) — memory-plane continuity across devices, sync profiles, watermarks, and observable replication.
8. [Workspace roaming protocol](workspace-roaming-protocol.md) — writer epochs, claim records, fencing, divergence, recovery, and the Git Sync to Cloud cutover (RSP-001 core).
9. [Migration and recovery plan](migration-and-recovery-plan.md) — upgrade graph, backups, delta recovery, tombstones, and restore epochs (MRP-001).
10. [Handoff transaction protocol](handoff-transaction-protocol.md) — the handoff lifecycle, state vector, claim, authenticity, and cleanup.
11. [Threat model](threat-model.md) — attacker classes, protected assets, trust boundaries, and the harness/Git/remote-content/secrets/process/prompt-injection threat catalog.
12. [Renderer content-security contract](renderer-content-security.md) — content classes, sanitization, CSP, navigation, terminal control policy, clipboard, attachments and downloads, and redacted exports (RCS-001).
13. [Update trust architecture](update-trust-architecture.md) — trust roots and online roles, release metadata, freshness, anti-rollback, platform signing, compromise recovery, and app bundle signing (UTA-001).
14. [Desktop stack verification plan](desktop-stack-verification-plan.md) — pinned per-OS baselines, the scenario catalog, evidence record, exception rule, and cadence for ADR-0002's acceptance gate (VP-001).
15. [ADR index and convention](adr/README.md) — decision status and governance.
16. [Product naming and trademark clearance](product-naming.md) — selected name and remaining clearance work.

## Decision status

| Artifact | Status |
| --- | --- |
| Target architecture | Draft |
| Roadmap | Draft |
| Versioning and compatibility | Draft |
| Governance (GOV-001) | Draft |
| Context Orb presentation specification | Draft |
| Adapter feed event schema (AEC-001 feed profile) | Draft |
| Workspace roaming and Engram sync protocol (RSP-001) | Draft |
| Workspace roaming protocol (RSP-001 core) | Draft |
| Migration and recovery plan (MRP-001) | Draft |
| Handoff transaction protocol (HTP-001) | Draft |
| Threat model (TM-001) | Draft |
| Renderer content-security contract (RCS-001) | Draft |
| Update trust architecture (UTA-001) | Draft |
| Desktop stack verification plan (VP-001) | Draft |
| ADR convention | Draft |
| ADR-0001: Apache-2.0 license | Accepted |
| ADR-0002: Desktop technology stack | Proposed |
| ADR-0003: Local Markdown and tiered assets | Accepted |
| ADR-0004: Fully open platform and custom integrated apps | Accepted |
| Product name: Omnifrons | Selected; formal trademark clearance pending |

Unreconciled early visual and generic-wrapper notes are historical inputs, not current architecture. They should be restored only with explicit status and reconciliation rather than silently mixed into this baseline.
