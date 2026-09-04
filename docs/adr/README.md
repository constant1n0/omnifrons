# Architecture Decision Record Convention

**Document role:** ADR format, status, evidence, and interim decision governance  
**Status:** Draft  
**Normative force:** Non-binding proposed convention  
**Accountable role:** Project Maintainer  
**Named person:** Unassigned  
**Approver role:** Project Owner  
**Approver named person:** Unassigned  
**Effective date:** None  
**Supersedes:** None  
**Name status:** Selected public name; preliminary screening complete, formal trademark clearance pending

## Authority

ADRs explain one consequential choice, alternatives, consequences, and approval evidence. The [target architecture](../target-architecture.md) owns system boundaries; the [roadmap](../roadmap.md) owns sequencing; the [compatibility policy](../versioning-and-compatibility.md) owns public promises.

No draft or Proposed ADR overrides another document. An Accepted ADR governs only its declared decision scope. Cross-document conflict requires an explicit reconciliation or superseding record; unresolved conflict blocks implementation of that behavior.

## File and content convention

- Path: `docs/adr/NNNN-kebab-case-title.md`
- Number: four digits, monotonic, never reused.
- Scope: one independently reviewable decision.
- Links: repository-relative and bidirectional when superseding.

Required metadata:

- `Status`
- `Accountable role`
- `Named person` (`Unassigned` is permitted only while Draft/Proposed)
- `Approver role`
- `Approver named person` (`Unassigned` is permitted only while Draft/Proposed)
- `Proposed on`
- `Accepted on`
- `Last status change`
- `Acceptance gate`
- `Supersedes`
- `Name status`

Required sections:

1. context and drivers;
2. proposal or accepted decision;
3. consequences;
4. alternatives with genuine tradeoffs;
5. acceptance evidence and follow-up;
6. related contracts.

## Canonical status lifecycle

| Status | Meaning |
| --- | --- |
| `Proposed` | Under review; not binding and not implemented as a settled dependency. |
| `Accepted` | Named approver accepted it after the recorded gate passed. |
| `Rejected` | Considered but not selected; retained with rationale. |
| `Superseded` | Replaced by a linked later ADR. |
| `Deprecated` | Historically accepted but no longer recommended; removal/replacement is linked. |

Conditions belong in `Acceptance gate`, not in the Status value. Status changes update metadata with actor, date, evidence, and rationale. Accepted reasoning is not rewritten to hide history. A material decision change creates a new ADR.

## Interim role glossary

These roles are design placeholders until a dedicated governance artifact is accepted. Every draft header names accountable and approver roles and uses `Named person: Unassigned` or a real name. An Accepted ADR, approved promotion, or release cannot retain `Unassigned` for its accountable person or approver. One person may hold several roles, but each role/person assignment is explicit; Git authorship does not imply approval.

| Role | Responsibility | Decision right |
| --- | --- | --- |
| Project Owner | Accountable product and commercial steward | Final approval for ADRs, maturity promotion, residual risk, license, and name. |
| Project Maintainer | Prepares artifacts, implementation, evidence, and status records | May propose; cannot imply approval merely by editing. |
| Decision Owner | Owns one decision's analysis and gate completion | Recommends an outcome and maintains evidence. |
| Security Reviewer | Reviews threat, exception, update, content, and data-loss risk | Advises; unresolved release-blocking findings prevent a claimed security gate. |
| Compatibility Owner | Maintains public surfaces, matrix, migration graph, and deprecations | Recommends compatibility changes. |
| Release Approver | Verifies release evidence and signed artifacts | Authorizes a release only within accepted policy. Initially the Project Owner. |
| Legal Counsel | Advises on license, trademark, distribution, services, and contributions | Provides legal advice; the Project Owner accepts residual legal risk. |
| Independent Reviewer | Reviews high-risk design or implementation without continuity bias | Advises and records findings; does not silently approve. |

Conflicts of interest and combined roles are recorded in the decision evidence.

## Approval matrix

| Decision | Proposer/owner | Required advice/evidence | Approver |
| --- | --- | --- | --- |
| Ordinary architecture ADR | Decision Owner | Affected owners; independent review when high risk | Project Owner |
| License/distribution | Decision Owner | Legal Counsel; dependency/asset inventory | Project Owner |
| Product name/trademark | Decision Owner | Legal Counsel; market and registry searches | Project Owner |
| Security exception | Decision Owner | Security Reviewer; scope, expiry, compensating control, rollback | Project Owner accepts residual risk |
| Public compatibility break | Compatibility Owner | Migration and impact evidence; major-version ADR | Project Owner |
| Maturity promotion | Project Maintainer | Roadmap gate evidence and open-risk disposition | Project Owner |
| Release | Release Approver | Support matrix, signatures, migrations, known risks | Project Owner until governance changes |

No approval is inferred from silence. If the same person proposes and approves, the record states that fact and the independent-review requirement, if any.

## Status-change record

Every transition records:

- previous and new status;
- named actor and role;
- date;
- prerequisite evidence;
- dissent, conflict, or accepted exception;
- follow-up owner and due/expiry date;
- commit or persistent observation that preserves the evidence.

An exception never changes a failed test into a pass. It records bounded accepted risk.

## Index

| ADR | Status | Proposal / outcome | Acceptance gate |
| --- | --- | --- | --- |
| [ADR-0001: Open-source license](0001-open-source-license.md) | Accepted | Apache-2.0 for original Omnifrons work. | Owner approved and canonical license committed; ongoing review applies when dependencies, assets, binaries, or contribution workflows are added. |
| [ADR-0002: Desktop technology stack](0002-desktop-technology-stack.md) | Proposed | Proposes Tauri 2 with framework-independent Rust core and React/TypeScript renderer. | Reproducible cross-platform verification plan, security review, and confirmed Rust ownership. |
| [ADR-0003: Local Markdown and tiered assets](0003-local-markdown-and-tiered-assets.md) | Accepted | Keeps Markdown always local and separates optional heavy-asset storage. | Owner decision recorded; implementation diagnostics and provider conformance remain required. |
| [ADR-0004: Fully open platform with custom integrated apps](0004-open-platform-and-custom-apps.md) | Accepted | Publishes the entire general product as free software; monetization through per-client custom apps above the platform. | Owner decision recorded; legal advice on service terms required before first reliant engagement. |

## Planned governance artifact

A dedicated governance/support-matrix artifact must replace these interim rules before beta. It will name actual people or bodies, quorum/delegation, evidence retention, conflict handling, contributor governance, release authority, and status-change mechanics. Until it is accepted, these draft rules cannot be used to claim external governance maturity.

Drafted as [governance](../governance.md) (GOV-001); acceptance pending — until then these interim rules apply.
