# Approvals record

This is the append-only record required by GOV-001 § Approval record and repository integrity. Rows are appended and never edited or removed; a correction is a new row naming, by date and artifact, the row it corrects. Each `Accepted` transition needs both its row here and a signed tag `accept/<artifact-id>/<date>`.

| date | artifact | prev_status | new_status | actor | role_exercised | accept_tag | evidence_ids | self_approval | advice_kind | advice_by | advice_signature_ref | dissent_or_exception | follow_up_owner_due |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 2026-09-04 | ADR-0001 | Accepted | Accepted (attributed) | constant1n0 | Project Owner | accept/ADR-0001/2026-09-04 | adr/0001-open-source-license.md § Acceptance evidence; LICENSE | yes | none | — | — | none; dry-run transition named by GOV-001 § Acceptance evidence and follow-up | — |
| 2026-09-04 | GOV-001 | Proposed | Accepted | constant1n0 | Project Owner | accept/GOV-001/2026-09-04 | GOV-001-ledger-2026-09-04; accept/ADR-0001/2026-09-04 (dry run); main protection enabled 2026-09-04 | yes | automated-lens | review-readability, review-risk | GOV-001-ledger-2026-09-04 | none | Project Owner: root shares two and three and the sealed recovery arrangement before the Alpha → Beta gate; Security Reviewer and Legal Counsel assignments |
| 2026-09-05 | target-architecture § Planned assurance artifacts | Draft (frozen table) | Draft (table revised) | constant1n0 | Project Owner | accept/target-architecture/2026-09-05 | desktop-stack-verification-plan.md and voice-interaction-contract.md exist as Draft; GOV-001-R16 | yes | none | — | — | none; pointer-only revision, no normative change | — |

`advice_kind` takes one of the five values GOV-001-R14 permits, and `self_approval` is `yes` whenever the accountable and approver roles were held by the same named person (GOV-001-R3).
