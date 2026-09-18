# Verification Evidence Store Specification

## Purpose

Defines the append-only `docs/evidence/VP-001/` record layout:
baseline-before-scenario admission (VP-001-R1), a closed
`pass`/`fail`/`uncertain` outcome vocabulary, mandatory-field validation,
provenance-clean publication, and correction-by-appended-row (VP-001-R20).

## Requirements

### Requirement: Baseline-Before-Scenario Admission

The system MUST reject a scenario result row unless a baseline record
carrying every VP-001-R1 field (OS build, architecture, WebView/runtime
version, packaging substrate, assistive-technology product, test date)
already exists for the referenced baseline.

#### Scenario: Complete baseline admits its scenario row
- GIVEN a baseline record carries every VP-001-R1 field
- WHEN a scenario row references that `baseline_id`
- THEN the row MUST be admitted

#### Scenario: Absent baseline blocks the row
- GIVEN no baseline record exists for a `baseline_id`
- WHEN a scenario row is submitted against it
- THEN it MUST be rejected as `baseline-unpinned`, never silently recorded

### Requirement: Closed Outcome Vocabulary

A row's `result` field MUST be exactly one of `pass`, `fail`, `uncertain`.
`uncertain` MUST remain a distinct third outcome — never folded into `pass`
or `fail`, and never rendered as "cleanly stopped". A row MAY also carry a
separate `observed_state` field (for example, `orphan-risk`) that records
additional observed context; `observed_state` is distinct from `result` and
is NOT constrained by this closed vocabulary. The human-readable rendering
`orphan-risk/uncertain` used elsewhere in this specification denotes the
pair `observed_state: orphan-risk` alongside `result: uncertain` — it MUST
NOT be interpreted as a fourth `result` value.

#### Scenario: Unproven termination records uncertain
- GIVEN a containment scenario cannot prove all descendants terminated
- WHEN the row is filed
- THEN `result` MUST be `uncertain` AND `observed_state` MUST be `orphan-risk`

#### Scenario: An out-of-vocabulary value is rejected
- GIVEN a row's `result` is not `pass`, `fail`, or `uncertain`
- WHEN it is validated
- THEN it MUST be rejected

### Requirement: Mandatory-Field Validation

The validator MUST reject a row or baseline record missing any mandatory
schema field, and that rejection MUST be observable in the repository's own
automated test run.

#### Scenario: Incomplete row is rejected and observed
- GIVEN a row omits a mandatory field, such as `evidence_artifact`
- WHEN the repository's test suite runs the validator against it
- THEN the validator MUST reject it and the test MUST report the rejection

#### Scenario: Complete row passes validation
- GIVEN a row carries every mandatory field
- WHEN the validator runs
- THEN the row MUST be accepted

### Requirement: Append-Only Correction

Records MUST be append-only: the store MUST NOT support rewriting or
deleting an existing row, and a wrong or superseded row MUST be corrected
only by appending a new row referencing the original by identifier. A
correction row's reference MUST resolve to an existing record, or the row
MUST be rejected. This validator runs after a row is already filed, against
Markdown files under version control; it cannot detect or refuse a direct
edit or deletion that already happened. The prohibition on rewriting an
already-filed record is therefore enforced by the repository's
signed-commit, linear-history branch protection on `main` and by review —
not by this validator.

#### Scenario: Correction appends a new row
- GIVEN a filed row is later found incorrect
- WHEN the correction is recorded
- THEN a new row referencing the original identifier MUST be appended, and the original MUST remain unchanged

#### Scenario: Unresolved correction reference is rejected
- GIVEN a row's `corrects` field names an identifier that does not resolve to an existing `record_id`
- WHEN the row is validated
- THEN it MUST be rejected as `corrects-unresolved`

### Requirement: Provenance-Clean Publication

A row published to the public store MUST NOT disclose hostname, local path,
company, estate, agent, or other infrastructure detail. A value that would
disclose such detail MUST instead be recorded privately and referenced from
the public row by identifier only.

#### Scenario: Public row carries no infrastructure detail
- GIVEN a completed row is prepared for the public store
- WHEN it is published
- THEN it MUST contain no hostname, local path, company, estate, agent, or infrastructure detail

#### Scenario: Sensitive value is referenced, not embedded
- GIVEN a field's true value would disclose infrastructure detail
- WHEN the public row is written
- THEN that value MUST be recorded privately and the public row MUST carry only its identifier

### Requirement: VP-S6 Row Content and Recorded Tension

A VP-S6 row MUST name the containment mechanism exercised
(process-group/`killpg`) and record the observed descendant state as an
observation distinct from any fallback path exercised. Unproven termination
MUST record `result: uncertain` together with `observed_state: orphan-risk`
(rendered together as `orphan-risk/uncertain`, per Closed Outcome
Vocabulary). VP-001-R6 (:227) requires the Linux fallback mechanism as its
own scenario, while the scenario catalog (:127) folds it into VP-S6; this
specification records mechanism and fallback as two distinct observations
within one row and flags — without resolving — that tension.

#### Scenario: Mechanism and fallback recorded distinctly
- GIVEN a VP-S6 run exercises process-group/`killpg` and, separately, a fallback path
- WHEN the row is filed
- THEN the mechanism observation and the fallback observation MUST appear as two distinct entries in the row

#### Scenario: Unconfirmed termination yields orphan-risk/uncertain
- GIVEN descendant termination cannot be confirmed after `killpg`
- WHEN the row is filed
- THEN `result` MUST be `uncertain` AND `observed_state` MUST be `orphan-risk`, never a pass and never "cleanly stopped"

### Requirement: Blocked or Non-Reproducible Verification Attempt

When the harness cannot complete a step a scenario requires — for example,
a WebDriver process that cannot attach to the retained binary, or a native
dialog that cannot be driven — or when a run's outcome does not reproduce
within its bounded attempts, the row MUST record `uncertain` with the
specific blocker disclosed. Such a row MUST NOT be retried in pursuit of a
`pass` outcome, MUST NOT be renamed to imply a different result, and MUST
NOT be replaced by a result obtained from a different build than the one
its digest identifies.

#### Scenario: Harness cannot complete a required step
- GIVEN the harness cannot complete a step the scenario requires, such as a WebDriver process failing to attach to the packaged artifact or a native dialog it cannot drive
- WHEN the scenario run is attempted
- THEN the row MUST record `uncertain` with the specific blocker disclosed

#### Scenario: No retry, rename, or cross-build substitution
- GIVEN the procedure runs a scenario exactly once against the digest-gated artifact
- WHEN the run completes and reports an honest `uncertain` or `fail`
- THEN the procedure MUST exit without retrying, the row MUST NOT be renamed to imply a different result, and MUST NOT be replaced by a result obtained from a different build than its digest identifies
