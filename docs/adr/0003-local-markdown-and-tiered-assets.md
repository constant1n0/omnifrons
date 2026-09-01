# ADR-0003: Local Markdown and Tiered Heavy Assets

**Document role:** Knowledge and heavy-asset locality decision
**Status:** Accepted
**Accountable role:** Project Maintainer
**Named person:** constant1n0
**Approver role:** Project Owner
**Approver named person:** constant1n0
**Proposed on:** 2026-09-01
**Accepted on:** 2026-09-01
**Last status change:** 2026-09-01 — accepted by the Project Owner
**Acceptance gate:** Product rule approved; implementation diagnostics and provider conformance remain required
**Supersedes:** Any design that places canonical Markdown in an evictable or placeholder-backed tier
**Name status:** Selected public name; preliminary screening complete, formal trademark clearance pending

## Context and drivers

Obsidian and CLI agents need reliable local access to the complete Markdown corpus. Users with limited disk space also need large PDFs, media, archives, datasets, and inactive artifacts to remain available without occupying local storage permanently.

A provider pin flag alone is too weak as the invariant: provider clients differ by platform, placeholder state can drift, and Linux does not offer one universal Files-On-Demand interface.

## Decision

1. Every Markdown file is a normal local file in the Git-backed, Obsidian-compatible knowledge tier.
2. Markdown is non-evictable. Onboarding, startup, and handoff preflight block if a Markdown file is unreadable or cloud-only.
3. Heavy assets live in a physically and logically separate blob tier. The user chooses local or on-demand policy.
4. A Context Catalog retains logical identity, relationships, provenance, remote locator, availability, size, and integrity metadata without becoming the content authority.
5. The Orb renders the complete graph. Local nodes use their active color; remote-only nodes are grey. Opening a remote-only node triggers explicit hydration and changes its state only after verified download.
6. Git transports Markdown and small portable state. Live `.git` directories are never synchronized through a consumer cloud folder.
7. The initial supported Linux heavy-blob backend is the official [Proton Drive CLI](https://proton.me/support/drive-cli), using explicit list/upload/download operations and machine-readable output.
8. The Proton Drive CLI is not treated as a mounted filesystem or background synchronization engine.
9. User-operated `rsync` is allowed as an external workflow but is unsupported: Omnifrons provides no convergence, conflict, or recovery guarantee for it.

## Consequences

### Benefits

- Obsidian, agents, search, and recovery can always read the complete Markdown knowledge graph.
- Disk pressure is managed without weakening the canonical knowledge tier.
- The Orb can show remote relationships without pretending bytes are available.
- Provider differences stay behind a bounded storage adapter.

### Costs and limits

- The catalog, remote objects, and local cache need integrity and lifecycle checks.
- Broken remote locators and unavailable providers require explicit failure states.
- Provider-native offline flags are diagnostic signals, not the source of truth.
- Large assets cannot be opened offline unless previously hydrated.

## Alternatives

### Put the whole vault in a provider virtual drive

Rejected because Markdown could become a placeholder, availability differs by provider and OS, and Obsidian/agents would lose the always-readable corpus.

### Keep every file permanently local

Rejected because it makes Omnifrons impractical on storage-constrained devices.

### Support arbitrary synchronization tools

Rejected for the supported product path because their conflict, deletion, locking, and recovery semantics cannot be promised uniformly. External tools may coexist without gaining a support guarantee.

## Acceptance evidence and follow-up

- The Project Owner explicitly selected always-local Markdown and user-selectable heavy-asset locality.
- The Project Owner selected Proton Drive CLI as the supported initial Linux blob path and excluded `rsync` from support.
- Implementation must add conformance tests for catalog state, hydration integrity, eviction safety, offline behavior, and provider failure.

## Related contracts

- [Target architecture](../target-architecture.md)
- [Versioning and compatibility](../versioning-and-compatibility.md)
- [Product roadmap](../roadmap.md)
- [ADR convention](README.md)
