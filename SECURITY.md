# Security Policy

## Reporting

Report a suspected vulnerability through GitHub's private vulnerability reporting on this repository (repository **Security** tab → **Report a vulnerability**). Do not open a public issue for an unreported vulnerability.

Alternate contact: `<security contact: owner decision pending>`.

## Status

Omnifrons is pre-alpha. There is no supported release and no working application yet; the [README](README.md) states this plainly. Treat any report against this repository as a report against a design-phase codebase, not a shipped product.

## Scope

docs/threat-model.md (TM-001) defines the actor classes, protected assets, and trust boundaries in scope. A residual risk TM-001 already discloses is a known limitation, not a new finding — for example, TM-001 SEC-4 records that the Engram memory daemon accepts any same-user process without authentication through alpha, accepted and disclosed rather than mitigated. Check TM-001 before filing; a report that reproduces a row TM-001 already carries is closed as already-known, but a report identifying a gap TM-001 does not cover is in scope and welcome.

## Out of scope (pre-alpha)

- **Update trust while UTA-001 is unaccepted.** docs/update-trust-architecture.md (UTA-001) is Draft, not Accepted. Until it is accepted there is no update-trust guarantee to violate; a report that only restates "there is no signed-update guarantee yet" duplicates known, tracked work rather than disclosing a new issue.
- **Advisory and harness-enforced scope are not security boundaries.** Per docs/target-architecture.md § Scope modes, a harness profile labelled `advisory` or `harness-enforced` is not a sandbox: `advisory` mode sets `cwd` and instructions while the harness keeps normal user permissions, and `harness-enforced` mode depends on a native harness control Omnifrons does not independently verify. Demonstrating that an advisory- or harness-enforced-mode limit can be bypassed is expected behavior, not a new finding, unless it also defeats a `sandbox-enforced` claim.
- **Disclosed debt of Draft artifacts.** An open decision or disclosed residual risk already recorded in a Draft artifact (docs/README.md § Decision status lists every artifact's current status) is known debt under active design, not a silent gap.

## Keys

No release signing key is registered yet. There is no signed release to verify against, and no root-signed operation (rotation, curated registry publication, revocation) is possible today (GOV-001, Root Key Holder assignment).
