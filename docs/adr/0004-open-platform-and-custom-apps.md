# ADR-0004: Fully Open Platform with Custom Integrated Apps

**Document role:** Distribution and monetization boundary decision
**Status:** Accepted
**Accountable role:** Project Maintainer
**Named person:** constant1n0
**Approver role:** Project Owner
**Approver named person:** constant1n0
**Proposed on:** 2026-09-02
**Accepted on:** 2026-09-02
**Last status change:** 2026-09-02 — accepted by the Project Owner
**Acceptance gate:** Licensing direction approved; app packaging, signing, and any public SDK remain future decisions
**Supersedes:** None
**Name status:** Selected public name; preliminary screening complete, formal trademark clearance pending

## Context and drivers

Omnifrons is developed in the open under Apache-2.0 (ADR-0001). Two distribution models were weighed:

1. **Open-core**: a free core plus proprietary first-party modules (panels, gadgets, connectors) sold separately.
2. **Fully open platform**: the entire general-purpose product published as free software, monetized above the platform through custom integrated apps and implementation services built per client.

The permissive license makes the paths asymmetric. Code once published under Apache-2.0 cannot be closed retroactively, but closed works built *on top of* an Apache-2.0 core remain lawful at any time, for anyone. Opening more later is trivial; planning a commercial boundary now taxes every feature decision ("free or paid?"), invites community distrust, and buys an option the permissive license already provides for free.

Bespoke client apps (monitoring, automations, client-specific gadgets and connectors) are proprietary by nature as work-for-hire, creating no licensing tension with an open platform.

## Decision

1. **Omnifrons is published fully as free software** under Apache-2.0: the core, the Orb, the dashboard shell, the gadget system, and every general-purpose gadget. No planned feature of the general product is withheld as a commercial module.
2. **Monetization happens above the platform, not inside it**: custom integrated apps built per client as private, bespoke work. Client apps live in private repositories owned per engagement and are never part of this repository.
3. The gadget/app integration surface remains an **internal contract**; a public SDK stays a 1.0 non-goal per the target architecture. Custom apps are first-party built until a separate ADR revisits that.
4. The permissive license **deliberately preserves the future option** of first-party commercial modules without relicensing; exercising that option requires a new ADR.
5. **Safety and user data are never gated under any model**: approvals, truthful capability status, hydration and integrity verification, data export, and memory access are core, free, forever.

## Consequences

### Benefits

- Maximum credibility and adoption potential for the open product; nothing is held back, so the public repository is the honest, complete artifact.
- No commercial-boundary tax on feature decisions and no "free vs paid" policing.
- Bespoke apps are proprietary to each client by nature, keeping the licensing story clean on both sides.
- All doors stay open: services now, custom apps as they arise, commercial modules later only if a future ADR elects them.

### Costs and limits

- No product-license revenue in the near term; income depends on service and custom-app capacity.
- Anyone may commercialize or host the open platform. This is accepted: first-party expertise and bespoke integrations are the differentiation.
- Custom apps must respect the same trust boundaries as the core (renderer content security, capability probing, update trust) even though their source is private.

## Alternatives

### Open-core with proprietary first-party modules

Rejected for now. It imposes the boundary tax immediately in exchange for an option the permissive license already preserves, and it risks community distrust before the product has a community. Remains available later through a new ADR.

### Copyleft relicensing (e.g., AGPL) to deter closed competitors

Rejected. It would complicate client deployments and bespoke app delivery — the chosen revenue path — and would trade the frictionless permissive core for protection the project does not currently need.

## Acceptance evidence and follow-up

- The Project Owner proposed the fully-open direction and confirmed custom integrated apps as the monetization path; the decision is recorded this date. The proposer and approver are the same person; that fact is recorded per the ADR convention.
- Legal Counsel advice on service contracts and work-for-hire terms is not yet obtained and is required before the first client engagement relies on this ADR.
- Follow-up: custom-app packaging and signing ride on the planned update-trust artifact (UTA-001); any public SDK or commercial-module decision is a separate ADR.

## Related contracts

- [ADR-0001: Open-source license](0001-open-source-license.md)
- [Target architecture](../target-architecture.md) — non-goals (no public SDK in 1.0), trust boundaries
- [Context Orb specification](../context-orb.md) — gadget system as the integration surface
- [ADR convention](README.md)
