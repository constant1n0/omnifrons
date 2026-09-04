# Governance and Support Matrix

**Document role:** Governance: role catalog, assignments, approvals, status workflow, exceptions, evidence retention, change control, and support-matrix authority (GOV-001)  
**Status:** Draft  
**Normative force:** Non-binding target direction; requirements are acceptance gates, not current guarantees  
**Accountable role:** Project Maintainer  
**Named person:** Unassigned  
**Approver role:** Project Owner  
**Approver named person:** Unassigned  
**Effective date:** None  
**Supersedes:** the interim role glossary and approval matrix in docs/adr/README.md once accepted  
**Name status:** Selected public name; preliminary screening complete, formal trademark clearance pending

## Document authority

This document drafts the planned governance artifact (GOV-001) that the [ADR convention](adr/README.md)'s "Planned governance artifact" paragraph names: the role catalog and decision rights, the assignments register recording who holds each role, the approval workflow for artifact status transitions and its evidence, the authority and evidence rules behind the roadmap's promotion gates, the exception process and registry, evidence retention for review ledgers and conformance results, conflict-of-interest and solo-maintainer rules, change control for normative text, release authority, contributor governance basics, and who may reclassify a support-matrix entry. The [target architecture](target-architecture.md) governs any conflict.

This draft does not redefine work already owned elsewhere:

- **Promotion-gate criteria** — owned by the [roadmap](roadmap.md); this document owns only who approves each gate and what evidence that approval relies on.
- **Support-matrix content and classifications** — owned by [versioning-and-compatibility.md](versioning-and-compatibility.md); this document owns only who may change a classification and how a support exception is recorded.
- **Security, content, and update-trust judgment** — owned by [TM-001](threat-model.md), [RCS-001](renderer-content-security.md), and [UTA-001](update-trust-architecture.md); this document says only whose advice a gate requires and how that advice is recorded.
- **Licensing** — owned by [ADR-0001](adr/0001-open-source-license.md).
- **Naming and trademark clearance** — owned by [product-naming.md](product-naming.md).
- **Engram upstream version tracking** — owned by the RSP-001 memory synchronization profile ([roaming-and-engram-sync.md](roaming-and-engram-sync.md)).

Once accepted, this document replaces the ADR convention's interim role glossary and approval matrix. ADRs keep their own status lifecycle exactly as [adr/README.md](adr/README.md) defines it; this document aligns roles to that lifecycle rather than replacing it.

The role-catalog, assignments-register, and open-decision sections below follow the specification convention that **the owner disposes**: proposed defaults and role assignments become binding only through the Project Owner's recorded approval. That authority doctrine is unrelated to the end-user product principle "system proposes, user disposes" that governs Omnifrons's own runtime approval behavior for a human user; the two describe different actors and different surfaces, and this document does not conflate them.

This document's requirements are new — `GOV-001-R1` through `GOV-001-R18` — and do not continue any other document's numbering; GOV-001 has its own registry entry in the target architecture's planned assurance artifacts table.

## Purpose and scope

In scope:

- the role catalog and each role's decision rights;
- the assignments register and how a named person or body is recorded against a role;
- the approval workflow for artifact status transitions (`Draft` → `Proposed` → `Accepted` → `Superseded`/`Deprecated`) and the evidence each transition requires;
- the authority and evidence rules behind the roadmap's promotion gates;
- the exception process and registry;
- evidence retention for review ledgers, conformance results, and drills;
- conflict-of-interest and solo-maintainer rules, including self-approval;
- change control for normative (`MUST`) text in an Accepted artifact, including which sections stay frozen regardless of Draft status;
- the repository-integrity rules — branch protection, signed acceptance tags, and the append-only approvals record — that make an approval attributable rather than merely asserted;
- release authority and key-custody role assignment;
- who may reclassify a support-matrix entry and how a support exception is recorded;
- contributor governance basics: review, sign-off, and license;
- which role's advice is required at each promotion gate, distinct from the gate's pass/fail criteria;
- the status-change record fields every artifact transition preserves.

Out of scope:

- promotion-gate pass/fail criteria themselves (the roadmap);
- support-matrix content, classifications, and version domains (the compatibility policy);
- security, privacy, and content judgments (TM-001, RCS-001, UTA-001);
- licensing decisions (ADR-0001) and naming decisions (product-naming.md);
- Engram upstream version tracking (the RSP-001 memory profile);
- a full conduct policy — named as a follow-up, not resolved here.

This document proposes; the owner disposes. Every table below is a proposal until the Project Owner records approval of it, per the header's own `Approver role`.

## Problem statement

Today one person holds every role the ADR convention's interim glossary names: Project Owner, Project Maintainer, Decision Owner, Compatibility Owner, and Release Approver, plus the root role UTA-001 assumes someone holds. Every draft artifact in this repository names its accountable and approver roles in its header, but records `Named person: Unassigned`. Approvals happen in conversation, outside any retained record. A reader comparing two artifacts cannot tell an accepted decision from a draft opinion, nor who advised what before a gate opened. ADR-0004's own acceptance evidence already states plainly that its proposer and approver are the same person — an honest disclosure this document generalizes into a rule, not a novel problem it introduces.

**Every approval MUST name the role it exercises, the evidence it relied on, and whether it was a self-approval; authority is recorded, not merely assumed from authorship.**

Today's concentration of every role in one person is disclosed throughout this document, not hidden; nothing above should be read as a present-tense claim of separation of duties — that claim becomes true only once the assignments register names distinct people for the roles that require independence.

The consequence of not fixing this now compounds: every artifact this project accepts before GOV-001 is accepted inherits an implicit, unrecorded approval chain, and unwinding that later — for a security incident, an audit, or a new maintainer's onboarding — is far harder than recording it at each transition going forward.

## Definitions

| Term | Meaning |
| --- | --- |
| Role | A named decision right, independent of who currently holds it. |
| Named person | The natural person or body recorded in the assignments register against a role for a given artifact or decision; distinct from Git commit authorship. |
| Assignments register | The table in this document recording who currently holds each role. |
| Self-approval | An approval where the accountable role and the approver role are exercised by the same named person for the same decision. |
| Review ledger | A retained per-finding record — id, lens, location, severity, status, evidence — plus the review's verdict and its fix/re-review trail, produced by a review pass. |
| Independent Reviewer advice | A fresh-context review, human or an automated review lens with a retained, labelled ledger, recorded as advice; it is never itself an approval. |
| Conformance evidence | A recorded pass/fail result from a defined conformance suite or drill, such as a MRP-001 restore drill or a UTA-001 key-loss drill. |
| Exception | A recorded, time-boxed, approved deviation from a `MUST` clause; never a relabeling of `uncertain` or `unsupported` behavior as guaranteed. |
| Evidence retention | The location and duration for which a review ledger, conformance result, or drill record remains retrievable. |
| Change control | The rule governing how normative (`MUST`) text in an Accepted artifact may change. |
| Support classification | One of the four values the compatibility policy's support matrix defines (`supported`, `preview`, `detected`, `unsupported`), applied to one tested combination. |
| Promotion gate | A roadmap maturity boundary (Pre-alpha → Alpha, Alpha → Beta, Beta → 1.0, or a 1.x breaking proposal) whose pass/fail criteria the roadmap owns and whose sign-off authority and evidence this document owns. |
| Evidence class | The kind of proof a decision required at its original acceptance — for example, a threat-model review, a signed conformance run, or a recorded drill — that a later `Proposed` revision of the same `MUST` clause must match or exceed. |
| Frozen section | A table or section that changes only through a `Proposed` revision carrying a signed acceptance tag, regardless of its containing document's own `Status` (see Change control). |
| Advice kind | The `advice_kind` field on an approval record: one of `human-security-reviewer`, `human-independent-reviewer`, or `automated-lens`, naming who or what gave the advice an approval relies on (see Review evidence and retention). |
| Sealed recovery arrangement | An offline root-share custody plan — a second and third root share held by a named deputy or escrow, with written recovery instructions — required before the Alpha → Beta gate (see Release authority and key custody; open decision D6). |

## Role catalog

The catalog below is a proposal — the owner disposes. It intentionally separates "who is accountable" from "who currently holds the role," so that filling an unfilled role later requires only an assignments-register edit, never a rewrite of every artifact that names the role.

| Role | Responsibility | Decision rights | May combine with | Today |
| --- | --- | --- | --- | --- |
| Project Owner | Accountable product and commercial steward | Final approval for ADRs, maturity promotion, residual risk, license, name, and this register | Any role, except acting as its own Independent Reviewer where one is required | Held by the Maintainer |
| Project Maintainer | Prepares artifacts, implementation, evidence, and status records | Moves an artifact `Draft` → `Proposed`; proposes decisions; cannot imply approval merely by editing | Any role | Held by the same person as Project Owner |
| Decision Owner (per decision/ADR) | Owns one decision's analysis and gate completion | Recommends an outcome and maintains its evidence | Any role | Held by the Maintainer for every current decision |
| Security Reviewer | Reviews threat, exception, update, content, and data-loss risk at gates | Advises; accepts TM-001, RCS-001, and UTA-001 findings; unresolved release-blocking findings prevent a claimed security gate | Any role, subject to the self-approval rule below at the security/legal gate | Unfilled; no advice recorded yet |
| Compatibility Owner | Maintains public surfaces, matrix, migration graph, and deprecations | Classifies support-matrix combinations (see Support-matrix authority) | Any role; initially the Maintainer | Held by the Maintainer |
| Release Approver | Verifies release evidence and signed artifacts | Authorizes channel promotions and release-notes sign-off within accepted policy; initially the Project Owner | Any role | Held by the Owner |
| Legal Counsel | Advises on license, trademark, distribution, services, and contributions | Advises; the Project Owner accepts residual legal risk | Any role, subject to the self-approval rule below at the security/legal gate | Unfilled; no advice recorded yet |
| Independent Reviewer | Reviews high-risk design or implementation without continuity bias | Advises and records findings; never silently approves; an automated review lens may fill this role only when its ledger is retained and labelled as automated advice | Any role except Decision Owner or Approver for the same decision | Unfilled, or filled only by a labelled automated ledger |
| Root Key Holder | Holds a UTA-001 root role share | Root metadata signing within the declared threshold | Any role, except being the sole holder below threshold without this document's recorded risk and target | Held by the Maintainer alone, who holds one share, not a threshold; no root-signed operation is possible today (risk owner: Project Owner; target: before the Alpha → Beta gate) |
| App Registry Curator | Publishes the UTA-001 curated app-author-key registry, root-signed | Registry publication authority within UTA-001's own rules | Any role | Unfilled; the role activates once UTA-001's curated registry ships |
| Contributor | Submits changes through pull request | Proposes only; no approval right | N/A | Open to the public once contribution is accepted |

The "Today" column states the interim reality as fact rather than concealing it, following the same discipline as the ADR convention's interim glossary: the Project Owner, Project Maintainer, Decision Owner, Compatibility Owner, Release Approver, and sole Root Key Holder roles above are held by one person; Security Reviewer, Legal Counsel, and Independent Reviewer are unfilled or filled only by recorded advice; App Registry Curator is unfilled and inactive until UTA-001's curated registry ships. Conflicts of interest and combined roles are recorded in the decision evidence, per the ADR convention.

Role names above are identical to the ADR convention's interim role glossary wherever the two overlap; Root Key Holder, App Registry Curator, and Contributor are new because the interim glossary predates UTA-001 and never named a contribution pathway.

## Assignments register

| Role | Named person or body | Effective date | Note |
| --- | --- | --- | --- |
| Project Owner | constant1n0 | 2026-09-01 | Signing key: not yet recorded (see Approval record and repository integrity) |
| Project Maintainer | constant1n0 | 2026-09-01 | Same person as Project Owner |
| Decision Owner (default) | constant1n0 | 2026-09-01 | Default until an artifact's own header names a distinct Decision Owner |
| Compatibility Owner | constant1n0 | 2026-09-01 | Interim, per versioning-and-compatibility.md's own header |
| Release Approver | constant1n0 | 2026-09-01 | Interim, per the ADR convention's approval matrix |
| Root Key Holder | constant1n0 | 2026-09-01 | Holds one share of the root role; no root-signed operation (rotation, curated registry publication, revocation) is possible; UTA-001's threshold guarantees do not hold; automatic beta/stable updates stay blocked and the curated registry stays empty until two more holders exist; risk owner: Project Owner; target: before the Alpha → Beta gate |
| Security Reviewer | Unassigned | — | No advice recorded yet |
| Legal Counsel | Unassigned | — | No advice recorded yet |
| Independent Reviewer | Unassigned | — | No advice recorded yet |
| App Registry Curator | Unassigned | — | Role inactive until UTA-001's curated registry is accepted |

This register lives in this document (open decision D1). A change to it is a commit approved by the Project Owner. An artifact's header `Named person` and `Approver named person` fields are filled from this register once the artifact reaches `Proposed`; a Draft artifact may keep `Unassigned` (open decision D8). Git commit authorship never implies approval of the change it carries — the ADR convention already states this, and this document restates it as a requirement (GOV-001-R13).

The Root Key Holder row is the register's clearest interim risk: UTA-001's own default threshold is 2 of 3 distinct holders (UTA-001's D1), and the register currently names one — a single share, not a threshold. No root-signed operation is possible today: no root rotation, no curated third-party app registry publication, and no revocation. UTA-001's threshold guarantees therefore do not hold, automatic beta and stable updates stay blocked, and the curated app registry stays empty, until two more holders exist. This risk is owned by the Project Owner with a target of closing it before the Alpha → Beta gate (GOV-001-R17; open decision D6).

## Status workflow and evidence per transition

| Status | Who acts | Entry requirement | Evidence retained |
| --- | --- | --- | --- |
| `Draft` | Maintainer edits freely | None | None |
| `Proposed` | Maintainer | Named persons filled from the register (or left `Unassigned` on a Draft, per D8); reviews recorded with retained ledgers; a conformance/drill plan is named | Review ledger location(s); named plan |
| `Accepted` | Owner | Recorded approval naming role, evidence class, and self-approval flag | Approval record; the artifact's `Effective date` is set |
| `Superseded`/`Deprecated` | Owner, via the replacing artifact or an ADR | The replacing artifact or ADR names the supersession, bidirectionally linked | The link itself, per the ADR convention |

ADRs keep their own status lifecycle exactly as the ADR convention defines it (`Proposed`, `Accepted`, `Rejected`, `Superseded`, `Deprecated`); this document aligns roles to that lifecycle rather than replacing it. A policy document such as this one, the roadmap, or a protocol draft uses the same header `Status` field and the same four working states above.

Every transition — ADR or otherwise — records the same fields the ADR convention already requires, generalized to every artifact this document governs: previous and new status; named actor and role; date; prerequisite evidence; dissent, conflict, or accepted exception; follow-up owner and due/expiry date; and the commit or persistent observation that preserves the evidence. An exception recorded at a transition never changes a failed gate into a pass; it records bounded accepted risk, per the Exceptions section below.

A `Superseded` or `Deprecated` transition is announced through the replacing artifact's own header `Supersedes` field and, for an ADR, through the index in the ADR convention; no separate governance announcement channel exists today, and none is proposed here.

## Self-approval and conflict of interest

When the accountable role and the approver role are exercised by the same named person for one decision, the approval record MUST say so — the discipline ADR-0004's acceptance evidence already applies by naming its own self-approval. For the items the roadmap's Alpha → Beta gate ties to Security Reviewer and Legal Counsel advice — the threat model, renderer security review, update trust architecture, migration and recovery plan, and legal distribution readiness — a self-approval alone is insufficient: the gate stays open until advice from a person other than the Maintainer/Owner is recorded. No exception under this document may substitute for that advice (GOV-001-R4); this requirement has no exception path. Where no second person is available to give it, the gate simply stays open (open decision D5).

Independent Reviewer advice — a fresh-context review, human or an automated review lens with a retained, labelled ledger — satisfies review evidence generally, but it never substitutes for the Security Reviewer or Legal Counsel advice those two named gate items specifically require. A Security Reviewer's advice cannot be filled by a relabeled Independent Reviewer, and the reverse does not hold either.

Concretely: today's sole maintainer approving their own threat model (TM-001) as both Decision Owner and Project Owner is a self-approval that MUST be recorded as such; it does not by itself open the Alpha → Beta gate, because that gate additionally requires Security Reviewer advice under GOV-001-R4. Until that advice is recorded, the gate simply does not open; no exception shortens that wait.

## Review evidence and retention

A review ledger records, per finding: id, lens, location, severity, status, and evidence, plus the review's overall verdict and its fix/re-review trail.

Retention: a review ledger, conformance result, or drill record is retained with the artifact it evidences for the life of that artifact plus one subsequent major version. Location: a provenance-clean review of a specification or design document — one that names no hostname, local path, company, estate, agent, or infrastructure detail — is retained in this public repository under `docs/evidence/<artifact-id>/`; a review or drill result that would disclose infrastructure detail or personal information is retained in a private evidence store and referenced from the public artifact by identifier only (open decision D2). Conformance results and drills for MRP-001, UTA-001, TM-001, and RCS-001 follow the same rule: a MRP-001 restore drill or a UTA-001 key-loss drill is public evidence when its record is itself provenance-clean, and private-store-referenced otherwise. An empty evidence location is recorded as a fact — see Acceptance evidence below — never silently omitted.

The role accountable for an artifact's evidence — its Decision Owner, or the Maintainer where none is separately named — is also accountable for filing the ledger at the correct location; retention is not self-executing and is not delegated to whoever happens to run a review.

Both evidence stores are append-only by rule, never by mere convention. The public store's entries are protected by `main`'s signed-commit and linear-history requirements (GOV-001-R15). The private store's custodian is the Project Owner, who grants auditor access on request and retains an access log of every such grant. Rewriting history on `main` to alter or remove a retained entry is prohibited under the same rule that protects an acceptance tag (see Approval record and repository integrity).

An approval record's `advice_kind` field takes exactly one value: `human-security-reviewer` (naming the reviewer's register handle and carrying their signature), `human-independent-reviewer`, or `automated-lens` (naming the tool identifier). Security Reviewer advice MUST be signed by that human reviewer's own registered key; an automated lens's ledger can never carry the Security Reviewer role, labelled or not (GOV-001-R14; open decision D7 asks whether an automated lens's advice separately counts as Independent Reviewer advice).

## Exceptions

An exception is requested by the role accountable for the `MUST` clause it deviates from, and it is entered in the table below only once the approver named in that same row has recorded approval; a request without an entry has no effect. Every column is mandatory — an entry missing scope, rationale, expiry, evidence, rollback plan, or status is not a valid exception and MUST NOT be treated as one (GOV-001-R7).

No exception exists for GOV-001-R4: the Alpha → Beta gate's Security Reviewer and Legal Counsel advice has no substitute, formal or informal, and no entry in this registry may target it. For every other exception, where a second registered person exists for the relevant role, the approver MUST be that different person, never the owner themselves; where no second person is registered, the entry records that absence rather than pretending independence (GOV-001-R7).

| ID | MUST clause | Owner | Approver | Scope | Rationale | Expiry | Evidence | Rollback plan | Status |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| _(none recorded)_ | | | | | | | | | |

This registry is currently empty; no exception has been approved. An exception's default expiry is 90 days (open decision D3). An exception can never relabel `uncertain` or `unsupported` behavior as guaranteed, per the roadmap's own exception rule. An exception MUST NOT be renewed more than twice, and its cumulative duration across all renewals MUST NOT exceed 270 days; past that limit it lapses for good, and the underlying `MUST` clause changes only through a Proposed revision (see Change control). An expired exception lapses automatically; continuing the same accepted risk within the renewal cap requires a new entry, never a silent extension. This registry lives in this document until volume warrants a dedicated file (open decision D1).

A renewal is a fresh row with a new ID and its own rationale and evidence, cross-referencing the lapsed entry it continues and counting toward that entry's renewal cap; it is never an edit to the lapsed row's own expiry date, so the history of how long a risk was actually carried stays reconstructible from the table alone.

## Change control

A `MUST` clause in an Accepted artifact changes only through a `Proposed` revision approved by the Owner, evidenced to the same evidence class as the original acceptance. A Draft artifact changes freely under the Maintainer, without a revision record. A target-architecture invariant changes only through an ADR — this document does not grant itself authority to loosen or add an invariant. Every normative change that reaches `Accepted` is listed in release notes under the compatibility policy's changelog obligations.

A material decision change — one that would alter an already-Accepted outcome rather than merely clarify it — creates a new ADR rather than silently rewriting the old one, per the ADR convention. Accepted reasoning is not rewritten to hide history; a superseding record links both directions.

No Draft or Proposed artifact, including this one, overrides another document merely by being newer; a cross-document conflict requires an explicit reconciliation or superseding record, and an unresolved conflict blocks implementation of the behavior in question, per the target architecture's own precedence rule.

Some sections are frozen regardless of their containing document's own `Status`: the target architecture's Proposed invariants table, the target architecture's planned assurance artifacts table, the roadmap's promotion and approval gates table, and this document's own Governance requirements table. Each changes only through a `Proposed` revision carrying a signed acceptance tag (GOV-001-R16); a Draft document's general editing freedom under Status workflow above does not extend to these four tables, even while the surrounding document is itself still Draft.

## Approval record and repository integrity

An approval is attributable and tamper-evident, not merely asserted in prose. The Project Owner's signing key is recorded in the assignments register once generated; today no such key is recorded, and that absence is itself part of the interim risk this document discloses rather than conceals.

The repository's `main` branch is protected: force-push is disabled, history stays linear, and every commit is signed (GOV-001-R15). An artifact's transition to `Accepted` is evidenced by two things, both required: a signed Git tag `accept/<artifact-id>/<date>`, made with the Owner's registered key, and a row appended to `docs/evidence/approvals.md` — an append-only record naming the artifact, the role exercised, the evidence identifiers relied on, the self-approval flag, and the `advice_kind` (see Review evidence and retention for its three permitted values). Neither substitutes for the other: the tag proves who signed, the row records what they signed for.

Git commit authorship is still never approval (GOV-001-R13); the signed tag and the `docs/evidence/approvals.md` row are what make an `Accepted` transition attributable. Today the same person authors commits, signs them, and would sign the acceptance tag — this record makes that fact attributable and checkable, not independent. Independence still requires a second named person, per Self-approval and conflict of interest above.

## Promotion gates: authority and evidence

This table's rows correspond one-to-one to the roadmap's own promotion and approval gates table; only the Evidence retained and Advice required columns are added here.

| Promotion | Accountable role | Approver | Evidence retained | Advice required |
| --- | --- | --- | --- | --- |
| Pre-alpha → Alpha | Project Maintainer | Project Owner | Portable-work proof, protocol drafts, desktop spike result, open-risk disposition — per each artifact's own acceptance-evidence section | None named beyond the Maintainer's own preparation |
| Alpha → Beta | Project Maintainer | Project Owner | Threat model (TM-001), renderer security (RCS-001), update trust (UTA-001), migration/recovery plan (MRP-001), legal distribution readiness — each artifact's own acceptance-evidence section | Security Reviewer and Legal Counsel advice; self-approval alone is insufficient (see Self-approval above) |
| Beta → 1.0 | Project Maintainer | Project Owner | Accepted public contracts, naming clearance, support matrix, external-pilot and recovery evidence, and this document's own governance evidence | Compatibility Owner recommendation |
| 1.x breaking proposal | Decision Owner | Project Owner, through a major-version ADR | Compatibility impact and migration evidence | Compatibility Owner recommendation |

The roadmap owns each gate's pass/fail criteria; this document owns only who signs off and what evidence that signoff relies on.

A gate does not open on partial evidence: every item this table's Evidence retained column names must be retained, and every item its Advice required column names must be recorded, before the Approver's sign-off is valid. A gate opened without one of those is itself an unrecorded exception and MUST be logged as one under Exceptions above (GOV-001-R7).

## Release authority and key custody

The Release Approver authorizes channel promotions and signs off release notes. Root Key Holders and the App Registry Curator are named in the assignments register above; UTA-001 assumes those names without redefining who selects them, and it owns the threshold, rotation, and revocation mechanics for the keys those roles hold. No release is authorized by a person whose only recorded role is Contributor.

Naming a Root Key Holder in the register is not itself proof the threshold is met. Today the register names one holder — one share, not a threshold — so no root-signed operation is possible: no root rotation, no curated third-party app registry publication, and no revocation. UTA-001's threshold guarantees therefore do not hold, automatic beta and stable updates stay blocked, and the curated app registry stays empty, until two more holders exist. This is recorded as risk with the Project Owner accountable and a target of closing it before the Alpha → Beta gate through a sealed recovery arrangement (GOV-001-R17; open decision D6). UTA-001's own acceptance evidence — a key-loss drill against the recorded holders — is the proof the threshold is actually met, and this document only supplies the names and the arrangement that drill exercises.

## Support-matrix authority

The Compatibility Owner classifies a tested combination under one of the four values the compatibility policy's support matrix defines. Moving a combination out of `supported` requires Project Owner approval and a release note under the compatibility policy's changelog obligations; moving a combination into `supported` requires the Compatibility Owner's recommendation and the tested evidence that classification implies. A support exception — accepting a combination below its normal classification for a bounded period — goes through the exception registry above; the matrix's content, classes, and promises remain owned by versioning-and-compatibility.md.

The Compatibility Owner's classification authority is a recommendation right combined with a decision right in the interim "today" state (one person), but the two are separable: a future Compatibility Owner distinct from the Project Owner recommends, and the Project Owner still approves a reclassification out of `supported`, per GOV-001-R12.

The Compatibility Owner reviews every `detected` combination at each minor release; `detected` is a starting classification, not a resting one. A combination that remains `detected` for more than two consecutive minor releases requires an explicit classification decision — into `supported`, `preview`, or `unsupported` — recorded in that release's notes (GOV-001-R18).

## Contributor governance

Contributions arrive through pull request and are reviewed by a Project Maintainer before merge. Each contribution requires a sign-off: either a Developer Certificate of Origin or a signed contributor agreement (open decision D4; DCO is the default). License terms for a contribution follow ADR-0001. A Maintainer review is advice toward merge, not itself the artifact-status approval this document defines elsewhere; a pull request that changes normative text in an Accepted artifact still follows Change control above. A full conduct policy is a follow-up, out of scope here.

A Contributor accumulates no standing role by contributing; recognition beyond Contributor — Decision Owner on a specific ADR, or a seat on any other role — is a separate, recorded assignments-register entry, not an automatic consequence of merged pull requests.

## Governance requirements

Each requirement below is testable against a specific artifact, register entry, or record; it is written to be checked, not merely read.

| ID | Requirement |
| --- | --- |
| GOV-001-R1 | Every artifact whose header `Status` is `Accepted` MUST record `Named person` and `Approver named person` from the assignments register; neither field MAY read `Unassigned`. |
| GOV-001-R2 | A change to the assignments register MUST be a commit approved by the Project Owner. |
| GOV-001-R3 | Every approval record MUST name the role exercised, the evidence relied on, and whether the accountable and approver roles were held by the same named person. |
| GOV-001-R4 | For the Alpha → Beta gate items the roadmap ties to Security Reviewer and Legal Counsel advice, a self-approval alone MUST NOT satisfy the gate; the gate MUST stay open until advice from a person other than the Maintainer/Owner is recorded. No exception under this document MAY substitute for that advice; this requirement has no exception path. |
| GOV-001-R5 | A review ledger MUST record, per finding, id, lens, location, severity, status, and evidence, plus the review's overall verdict and its fix/re-review trail. |
| GOV-001-R6 | A review ledger, conformance result, or drill record MUST be retained for the life of the artifact it evidences plus one subsequent major version, at the location Review evidence and retention defines. |
| GOV-001-R7 | An exception registry entry MUST record id, MUST clause, owner, approver, scope, rationale, expiry, evidence, rollback plan, and status; an entry without a recorded expiry MUST NOT be approved. The approver MUST differ from the owner whenever a second registered person exists for the relevant role. An exception MUST NOT be renewed more than twice or exceed 270 cumulative days; beyond that limit it lapses for good and the underlying `MUST` clause changes only through a Proposed revision. No exception entry MAY target GOV-001-R4. |
| GOV-001-R8 | An exception MUST NOT relabel `uncertain` or `unsupported` behavior as guaranteed. |
| GOV-001-R9 | A `MUST` clause in an Accepted artifact MUST change only through a `Proposed` revision approved by the Project Owner, evidenced to the same evidence class as the original acceptance. |
| GOV-001-R10 | A target-architecture invariant MUST change only through an ADR. |
| GOV-001-R11 | No release MAY be authorized by a person whose only recorded role is Contributor. |
| GOV-001-R12 | Moving a support-matrix combination out of `supported` MUST have Project Owner approval and a release note. |
| GOV-001-R13 | Git commit authorship MUST NOT be treated as approval of the change it carries; an `Accepted` transition's approval is evidenced only by the signed acceptance tag and its `docs/evidence/approvals.md` row (see Approval record and repository integrity), never by commit authorship or message alone. |
| GOV-001-R14 | A review performed by an automated review lens MUST be labelled as automated advice in its ledger and recorded with `advice_kind: automated-lens` naming the tool identifier; it MUST NOT be recorded as Security Reviewer or Legal Counsel advice under any circumstance. Security Reviewer advice MUST carry `advice_kind: human-security-reviewer`, the reviewer's register handle, and their signature. |
| GOV-001-R15 | The repository's `main` branch MUST be protected: force-push MUST be disabled, history MUST remain linear, and every commit MUST be signed. |
| GOV-001-R16 | The target architecture's Proposed invariants table and planned assurance artifacts table, the roadmap's promotion and approval gates table, and this document's own Governance requirements table change only through a `Proposed` revision carrying a signed acceptance tag, regardless of the containing document's own `Status`. |
| GOV-001-R17 | Before the Alpha → Beta gate, a sealed recovery arrangement MUST exist for the root role: a second and third root share created and held offline by a named deputy or escrow, with written recovery instructions, and the assignments register and exception registry MUST remain recoverable from the protected repository. |
| GOV-001-R18 | The Compatibility Owner MUST review every `detected` support-matrix combination at each minor release; a combination `detected` for more than two consecutive minor releases MUST receive an explicit classification decision recorded in that release's notes. |

## Open decisions

These are the questions this document proposes an answer to without yet closing; the Project Owner's approval of a specific option, not the presence of a default here, is what closes one.

| ID | Question | Options | Default proposal |
| --- | --- | --- | --- |
| D1 | Where do the assignments register and the exception registry live? | In this document (GOV-001); as separate files once volume warrants it | In this document until volume warrants a dedicated file |
| D2 | Where is review and conformance evidence retained, and under what integrity rule? | Public repository only; a private evidence store only; split by provenance | Public repository under `docs/evidence/<artifact-id>/`, append-only under `main`'s signed-commit and linear-history protection (GOV-001-R15), for provenance-clean spec/design reviews; a private evidence store, append-only, custodied by the Project Owner with auditor access on request and a retained access log, referenced by identifier for anything carrying infrastructure or personal detail |
| D3 | What is an exception's default expiry, and how many times may it renew? | 30/90/180-day default expiries; unlimited renewal; a capped renewal count and cumulative duration | 90-day default expiry; at most two renewals; 270 days cumulative, after which the exception lapses for good and the underlying `MUST` clause changes only through a Proposed revision |
| D4 | What contributor sign-off mechanism applies? | Developer Certificate of Origin; a signed contributor agreement | Developer Certificate of Origin (DCO) |
| D5 | What happens at a security/legal gate under a solo maintainer, given GOV-001-R4 forecloses any exception at this gate? | The gate stays open indefinitely until external advice is recorded; the project actively recruits a Security Reviewer and Legal Counsel ahead of attempting the gate | The gate stays open until external advice is recorded, with no exception path (GOV-001-R4); recruiting external advice ahead of attempting the Alpha → Beta gate is the practical mitigation, not a formal alternative |
| D6 | Who succeeds the Owner, or what quorum applies, when the Owner is unavailable? | No successor until a second person exists; a named deputy recorded in the register; a sealed recovery arrangement with offline root shares held by a deputy or escrow | A sealed recovery arrangement is required before the Alpha → Beta gate: the second and third root shares are created and held offline by a named deputy or escrow, with written recovery instructions; the register and the exception registry remain recoverable from the protected repository; until the arrangement exists, the risk is recorded with a target date |
| D7 | Does an automated review lens's advice count as Independent Reviewer advice? | Yes, with a retained, labelled ledger; no, only a human review counts | Yes, with a retained ledger labelled as automated advice |
| D8 | May a Draft artifact's header leave `Named person` `Unassigned`? | Yes, until `Proposed`; no, every Draft must name a person | Yes, until the artifact reaches `Proposed` |

D1 through D4 are process questions this document can answer for itself at acceptance. D5 through D8 recur across other planned artifacts (UTA-001's D1 threshold, TM-001's D5 acceptance-evidence cadence) as instances of the same underlying question this table answers once, centrally, rather than per document.

## Acceptance evidence and follow-up

- The assignments register above is filled for every currently Accepted artifact (ADR-0001, ADR-0003, ADR-0004) and for the Active product-naming decision.
- The exception registry is present and currently empty; no exception has been recorded against GOV-001-R4, and none is valid — that gate has no exception path. The registry's renewal cap (at most two renewals, 270 cumulative days) is untested until a first exception is recorded and renewed.
- The evidence location this document defines is live, referenced by identifier, with at least the ledgers of the drafts accepted so far, once those ledgers exist; none exist yet because this document is itself Draft.
- The ADR convention's interim role glossary and approval matrix point to this document once it is accepted, per the wiring recorded there.
- The roadmap's 1.0 exit criterion — "governance ... evidence is approved and retained" — is satisfied only once this document is `Accepted` and at least one status transition has run under it with its evidence retained.
- The Root Key Holder shortfall the assignments register records (one holder, one share, not a threshold; no root-signed operation possible; automatic updates and the curated registry blocked) is carried as an open risk, owned by the Project Owner, targeted for closure before the Alpha → Beta gate — not for this document's own acceptance.
- `main`'s signed-commit and linear-history protection (GOV-001-R15) and `docs/evidence/approvals.md` do not yet exist; both are required before this document's own first `Accepted` transition can produce a valid signed acceptance tag.
- The frozen-sections list (GOV-001-R16) has not yet been exercised by an attempted change; the first test is a `Proposed` revision to one of its four listed tables, carrying a signed acceptance tag.
- The sealed root-share recovery arrangement GOV-001-R17 requires (open decision D6) does not yet exist; it is targeted before the Alpha → Beta gate, tracked alongside the Root Key Holder shortfall above.
- The `advice_kind` field and its three permitted values (GOV-001-R14) are defined here but not yet exercised by any recorded approval; the first Security Reviewer advice recorded must carry a signature to test the requirement.
- No `detected` support-matrix combination has yet reached a minor release under GOV-001-R18's review cadence; the rule is untested.
- A dry run of one status transition, with its record, is required follow-up evidence before this document itself can move past `Proposed`.

## Related artifacts

- [ADR convention](adr/README.md) — the interim role glossary and approval matrix this document supersedes once accepted; the status lifecycle this document aligns roles to.
- [Target architecture](target-architecture.md) — the GOV-001 row in planned assurance artifacts; the rule that an invariant changes only through an ADR.
- [Product roadmap](roadmap.md) — promotion and approval gates; the 1.0 exit criterion this document's evidence satisfies; the exception rule this document inherits.
- [Versioning and compatibility](versioning-and-compatibility.md) — support-matrix content and classifications; the changelog obligations a normative or support change follows.
- [Threat model](threat-model.md) (TM-001) — Security Reviewer acceptance of findings; its own D5 on acceptance-evidence ownership and cadence, which this document's roles answer.
- [Update trust architecture](update-trust-architecture.md) (UTA-001) — the root role holders and the App Registry Curator this document names in the assignments register.
- [Renderer content-security contract](renderer-content-security.md) (RCS-001) — Security Reviewer acceptance of findings.
- [Migration and recovery plan](migration-and-recovery-plan.md) (MRP-001) — drill evidence retained under this document's rules.
- [Product naming and trademark clearance](product-naming.md) — the Legal Counsel advice gate this document names.
- [ADR-0001: Open-source license](adr/0001-open-source-license.md) — the license contributor sign-off follows.
- [ADR-0004: Fully open platform with custom integrated apps](adr/0004-open-platform-and-custom-apps.md) — the self-approval disclosure this document generalizes into a requirement.
- [ADR-0002: Desktop technology stack](adr/0002-desktop-technology-stack.md) — a currently Proposed decision the status workflow above applies to directly, pending its own acceptance gate.

## References

- [Keep a Changelog](https://keepachangelog.com/) — the release-notes categories a normative or support-matrix change is disclosed under.
- [Developer Certificate of Origin](https://developercertificate.org/) — the default contributor sign-off (open decision D4).
