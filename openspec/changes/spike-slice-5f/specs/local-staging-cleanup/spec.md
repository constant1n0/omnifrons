# Local Staging Cleanup Specification

## Purpose

Conservatively reclaim only abandoned local-dir staging files before new local staging;
all uncertain entries remain untouched.

## Requirements

### Requirement: Bounded Cleanup Authority

The system MUST sweep only a revalidated local device asset root while its publication
surface lock is held. It MUST consider only exact basenames matching
`.<publication-hex>.<pid>-<sequence>.part`; a filename alone MUST NOT prove origin.
The six other `.part` producers, unparseable names, other namespaces, registered final
artifacts, and outbox entries MUST remain outside this capability.

#### Scenario: Exact local candidate is considered

- GIVEN the locked, revalidated local asset root contains an exact provider-grammar entry
- WHEN local staging cleanup runs before creating new staging
- THEN the entry MAY proceed to conservative eligibility checks

#### Scenario: Excluded entry is retained

- GIVEN an entry is an excluded producer, another namespace, unparseable, final artifact, or outbox entry
- WHEN cleanup runs
- THEN the entry MUST remain and MUST NOT be reported as removed

### Requirement: Proven Abandonment Gate

The system MUST require a fixed conservative minimum age selected by design and MUST
conclusively classify the encoded PID as `NotLive`. Active, reused, or `Unknown` PID
results, invalid or future timestamps, metadata errors, metadata mismatch, and I/O errors
MUST retain the entry. Age MUST NOT establish PID identity or deletion authority.

#### Scenario: Old candidate with conclusively dead PID qualifies

- GIVEN an exact candidate exceeds the fixed minimum age and its creator PID is `NotLive`
- WHEN all required safety evidence remains valid
- THEN the candidate MAY proceed to final identity verification

#### Scenario: Ambiguous liveness or time retains candidate

- GIVEN a candidate has an active, reused, or `Unknown` PID, or invalid or future time
- WHEN cleanup evaluates it
- THEN the candidate MUST remain

### Requirement: Safe Deletion Evidence

The system MUST refuse links and non-regular entries. On supported platforms it MUST
obtain positive no-follow regular single-link identity evidence, revalidate that identity
immediately before unlink, and count removal only after positive deletion evidence.
Unsupported platforms MAY retain candidates conservatively but MUST NOT represent retention
as removal or cleanup success.

#### Scenario: Stable regular candidate is removed

- GIVEN a supported platform supplies matching no-follow regular single-link identity evidence
- WHEN final revalidation succeeds and unlink positively completes
- THEN the candidate MUST be reported as removed

#### Scenario: Unsafe or changed entry is retained

- GIVEN an entry is linked, non-regular, or its final identity differs from initial evidence
- WHEN cleanup attempts final verification
- THEN it MUST refuse removal and retain the entry

### Requirement: Failure Reporting and Residual Races

The system MUST report optional cleanup failures without device paths or new publication
wire states and MUST preserve normal staging and publication when cleanup fails. The lock
MUST be per-shell only; the system MUST NOT claim cross-process exclusion or race-free
unlink. It MUST distinguish observable identity-mismatch refusal from the disclosed
same-user final-check/unlink race that current platform primitives cannot eliminate.

#### Scenario: Optional cleanup failure does not interrupt publication

- GIVEN cleanup encounters an operational failure
- WHEN the shell continues its publication path
- THEN it MUST preserve normal behavior and emit no device path or publication wire state

#### Scenario: Cross-process race is not overstated

- GIVEN another same-user process can alter an entry after the final check
- WHEN cleanup documentation or reporting describes the result
- THEN it MUST disclose that residual race rather than claim identity-mismatch refusal
