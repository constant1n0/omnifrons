# Packaged Build Evidence Specification

## Purpose

Defines what makes a Linux release artifact admissible as VP-001 evidence: a
pinned build environment, a retained artifact bound to a digest, and
exclusion of any non-packaged build (VP-001-R2). This capability makes no
build-reproducibility claim.

## Requirements

### Requirement: Pinned Build Environment

The system MUST produce every packaged-build-evidence artifact from a pinned
`ubuntu-24.04` runner, a pinned Tauri CLI version, and an explicit,
non-default `bundle.targets` list. The exact runner image version MUST be
captured and recorded with the artifact.

#### Scenario: Pinned build records its runner image
- GIVEN CI is configured with `ubuntu-24.04`, a pinned Tauri CLI version, and explicit `bundle.targets`
- WHEN the packaging job completes
- THEN the run MUST record the exact runner image version with the artifact

#### Scenario: Missing runner image version blocks admissibility
- GIVEN a completed job has no captured runner image version
- WHEN the artifact is proposed as evidence
- THEN it MUST NOT be admissible until the version is read from the job log and recorded

### Requirement: Retained, Digest-Addressable Artifact

The system MUST retain the packaged Linux artifact rather than discard it
after the job completes, and MUST compute and record its SHA-256 digest. The
digest identifies exactly one retained artifact; it MUST NOT be presented as
proof of byte-for-byte build reproducibility.

#### Scenario: Retained artifact is digest-bound
- GIVEN a packaged Linux build completes under a pinned environment
- WHEN the artifact is uploaded for retention
- THEN its SHA-256 digest MUST be computed and recorded against that one artifact

#### Scenario: Two builds are not claimed identical
- GIVEN a second build from the same source has a different digest
- WHEN the two are compared
- THEN neither digest MUST be treated as proof of the other's reproducibility

### Requirement: Packaged-Build-Only Admissibility

The system MUST NOT accept a development-mode or unpackaged build as VP-001
evidence input (VP-001-R2). Only an artifact satisfying Pinned Build
Environment and Retained, Digest-Addressable Artifact MUST be eligible to
populate an evidence row's build/digest fields.

#### Scenario: Packaged artifact is accepted
- GIVEN an artifact meets both pinning and retention requirements
- WHEN it is proposed as evidence for a scenario row
- THEN it MUST be accepted as the row's `build_channel_digest` source

#### Scenario: Dev-mode build is rejected
- GIVEN an artifact was produced by a development-mode run
- WHEN it is proposed as evidence
- THEN it MUST be rejected and recorded as `dev-mode-only`

### Requirement: Exercised Artifact Matches Digested Artifact

The system MUST ensure that the artifact a scenario run exercises through its
shipped GUI is the exact same distributable artifact whose SHA-256 digest the
evidence row records. No evidence-only cargo feature (for example,
`demo-harness`) MUST be enabled in the exercised artifact, and no alternate
build variant MUST be substituted for the retained, digest-addressable
artifact identified under Retained, Digest-Addressable Artifact.

#### Scenario: Exercised artifact matches its recorded digest
- GIVEN a scenario row records a SHA-256 digest for a retained packaged artifact
- WHEN a WebDriver harness drives a binary during that scenario run
- THEN the driven binary MUST be byte-identical to the digested artifact

#### Scenario: Feature-enabled variant is inadmissible
- GIVEN a build has an evidence-only cargo feature (such as `demo-harness`) enabled
- WHEN it is proposed as the artifact for a VP-001-R2 scenario row
- THEN it MUST be rejected as inadmissible under VP-001-R2, regardless of its own computed digest
