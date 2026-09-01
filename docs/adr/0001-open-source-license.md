# ADR-0001: Open-Source License

**Document role:** License decision rationale  
**Status:** Accepted  
**Accountable role:** Project Maintainer  
**Named person:** constant1n0  
**Approver role:** Project Owner  
**Approver named person:** constant1n0  
**Proposed on:** 2026-09-01  
**Accepted on:** 2026-09-01  
**Last status change:** 2026-09-01 — accepted by the Project Owner and published with the canonical license text  
**Acceptance gate:** Passed for the current original documentation; legal, contribution, dependency/asset, binary-distribution, and service-model review remain gates as those scopes are introduced  
**Supersedes:** None  
**Name status:** Selected public name; preliminary screening complete, formal trademark clearance pending

## Context

Omnifrons is intended to become a public project and a basis for paid installation, configuration, migration, training, managed operations, and support. The license should minimize adoption friction while making patent and redistribution terms clearer for organizations.

This ADR is not legal advice and does not decide trademark ownership. See [product naming](../product-naming.md) and the [ADR convention](README.md).

## Decision

License original Omnifrons source and documentation under [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0).

Apache-2.0 permits private, public, modified, redistributed, and commercial use. Its explicit contributor patent grant and termination terms provide more enterprise clarity than MIT without requiring customers to publish unrelated systems.

Paid value remains legitimate professional work: secure installation, harness integration, knowledge migration, backups, observability, upgrades, training, and support.

## Consequences

### Benefits

- Low-friction open and commercial adoption.
- Explicit patent terms for contributed code.
- Compatible service, managed-deployment, and proprietary-integration models.
- Clear notice and redistribution obligations.

### Costs and limits

- Distribution must preserve license and applicable notices.
- Every dependency, asset, font, icon, binary, and bundled model still needs review.
- Permissive licensing allows third parties to sell modified distributions without publishing changes.
- Apache-2.0 does not grant trademark rights or replace patent/legal review.

## Alternatives

### MIT

Simpler and highly permissive, with the same service-business compatibility. Not preferred because it lacks Apache-2.0's explicit patent grant and termination language.

### MPL-2.0

Would require source availability for changes to covered files while allowing proprietary combinations. Not preferred because file-level copyleft adds compliance cost not yet justified by product strategy.

### AGPL-3.0

Would protect source availability for modified network services. Not preferred because its network-use obligation would add substantial procurement and proprietary-integration friction to a primarily local product.

## Third-party boundaries

- [Engram](https://github.com/Gentleman-Programming/engram) and [obsidian-skills](https://github.com/kepano/obsidian-skills) currently declare MIT licensing. Pinned revisions, notices, transitive dependencies, and distribution method must be rechecked.
- Prefer supported Engram CLI/MCP integration rather than copying its source.
- Obsidian is separately licensed. Omnifrons may work with user-installed Obsidian and documented interfaces, but must not redistribute it or imply affiliation. Review the current [Obsidian terms](https://obsidian.md/terms) before distribution.
- Source-code permission does not grant provider names, logos, model marks, or other brand assets.

## Engram Cloud service wording

MIT permission for Engram source does not by itself grant Engram trademarks, hosted-service resale rights, co-branding, or an endorsed-partner claim.

Until separately reviewed, commercial descriptions are limited to factual customer-selected services such as installation/support of Engram, self-hosted deployment, backup, monitoring, and configuration. Any resale, hosted multi-customer offering, or branded “managed Engram Cloud” claim requires explicit legal and trademark review.

## Publication status

The canonical Apache-2.0 text is committed at [`LICENSE`](../../LICENSE). The current repository contains design documentation and no bundled runtime dependencies, third-party assets, installers, or binaries.

This acceptance does not replace qualified legal advice. Before executable distribution or an open contribution program, the project must inventory dependencies and assets, define contribution terms, review target-jurisdiction implications, and confirm trademark and service wording.

## Acceptance evidence

1. The Project Owner explicitly selected Apache-2.0 and approved public repository publication.
2. The canonical Apache-2.0 license text was committed on 2026-09-01.
3. The publication baseline contains original design documentation only.
4. Ongoing third-party, contribution, binary-distribution, trademark, and service review is retained as a release obligation rather than misrepresented as complete.

## Related artifacts

- [ADR convention](README.md)
- [ADR-0002: Desktop technology stack](0002-desktop-technology-stack.md)
- [Target architecture](../target-architecture.md)
- [Roadmap](../roadmap.md)
- [Product naming](../product-naming.md)
