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
14. [Voice interaction contract](voice-interaction-contract.md) — consent, visibility, processing, retention, accessibility, and text fallback for voice interaction (VOC-001).
15. [Desktop stack verification plan](desktop-stack-verification-plan.md) — pinned per-OS baselines, the scenario catalog, evidence record, exception rule, and cadence for ADR-0002's acceptance gate (VP-001).
16. [ADR index and convention](adr/README.md) — decision status and governance.
17. [Product naming and trademark clearance](product-naming.md) — selected name and remaining clearance work.

## Citation convention

A cross-reference into another document never cites a line number. Line numbers drift with every edit and silently point at the wrong sentence; the anchors below survive edits because they are tied to the target's own structure, not its position on the page.

1. **Prefer an identifier over a location.** When the cited point carries an id, cite the id alone, dropping the file name whenever the id prefix already names the document — a requirement or decision (`TM-001-R7`, `RCS-001 D12`), a table row id (`TM-001 A7`, `VP-001 VP-S8`, `TM-001 SEC-3`), a target-architecture invariant (`target architecture, invariant 2`), a signal-mapping row (`TM-001 signal \`unprovisioned-secret\``), or an assignments-register row (`GOV-001, Security Reviewer assignment`).
2. **Otherwise cite the enclosing heading**, as a link in bullets and table cells (`[Actors and capabilities](threat-model.md#actors-and-capabilities)`) or, in running prose, as a section reference (`threat-model.md § Actors and capabilities`) — inside a table cell either form is allowed, and the shorter one is preferred once a link would run past roughly a third of the cell's width. When the heading text repeats within the target document, name the parent heading too, `→` child only: `(roadmap.md § Version 1.0 → Exit criteria)`, linking to the correctly numbered duplicate slug (`#exit-criteria-3`, counting duplicates in document order the way GitHub does).
3. **Pin one sentence inside a long section with a short verbatim phrase.** Once a section runs past roughly 150 words, or a paragraph carries several distinct claims, a bare heading link no longer points at anything specific enough. Add a 2–6 word phrase quoted verbatim from the target sentence, unique within the section, in its own parentheses right after the heading: `(governance.md § Review evidence and retention ("provenance-clean review"))`. Never a line number, and never a paraphrase.
4. **Text above a document's first section heading** — its header block — is cited by quoting the bold label it sits under: `(ADR-0002, "Acceptance gate")`.
5. **Chain citations to one target; switch target with a semicolon.** Within one target, chain by: same id or heading, comma-separated (`TM-001-R3, TM-001-R9`); a shared parent heading, repeating `→` per child (`roadmap.md § Beta → Scope, → Exit criteria`); two different headings in the same file, repeating `§` (`file.md § Document authority ("..."), § Purpose and scope ("...")`); or the same heading cited for two distinct sentences, merged into one citation carrying both phrases (`file.md § Heading ("phrase one", "phrase two")`) rather than repeated. `→` names only a real parent → child pair — chaining otherwise-unrelated sections always takes the repeated-`§` form instead — and any item that itself contains a comma (an assignments-register item like `GOV-001, Security Reviewer assignment`) forces semicolons between every item in its chain, so the internal comma is never mistaken for a chain boundary. A semicolon switches target entirely, as in the old line-number form `(threat-model:31, :108-138)` — replaced by `([Purpose and scope](threat-model.md#purpose-and-scope); [Actors and capabilities](threat-model.md#actors-and-capabilities) (all rows))`.
6. **Collapse a range to one anchor**, adding "all rows" when the range spans an entire table — `(all rows)` after a link, or `, all rows` after a `§` section reference in prose (`file.md § Heading, all rows`); a range that crosses two sections becomes two anchors instead of one.
7. **Always give the full relative path with `.md`** inside a citation — never a bare document name, never a `docs/`-rooted path. Links are relative to the citing file's own location, so a citation from `docs/desktop-stack-verification-plan.md` into an ADR reads `adr/0002-desktop-technology-stack.md#...`.
8. **Drop a bare relocation.** A citation that only re-states, in parentheses, the line an id was already pinned to — the old form `RCS-001-R13 (:256)` — is simplified by deleting the parenthetical and keeping the id: `RCS-001-R13`.

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
| Voice interaction contract (VOC-001) | Draft |
| Desktop stack verification plan (VP-001) | Draft |
| ADR convention | Draft |
| ADR-0001: Apache-2.0 license | Accepted |
| ADR-0002: Desktop technology stack | Proposed |
| ADR-0003: Local Markdown and tiered assets | Accepted |
| ADR-0004: Fully open platform and custom integrated apps | Accepted |
| Product name: Omnifrons | Selected; formal trademark clearance pending |

Unreconciled early visual and generic-wrapper notes are historical inputs, not current architecture. They should be restored only with explicit status and reconciliation rather than silently mixed into this baseline.
