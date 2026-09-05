# Contributing to Omnifrons

## Before you start

Omnifrons is in its design phase. The [documentation](docs/README.md) is the normative artifact set: it describes target direction and acceptance gates, not a shipped application. Code in this repository — including the renderer scaffold and, once it lands, the Rust workspace — is the [ADR-0002](docs/adr/0002-desktop-technology-stack.md) spike: executable evidence toward that ADR's acceptance gate, not itself an accepted architecture. Read [docs/README.md](docs/README.md) before opening a pull request that touches `docs/`, so citations and status rows follow the existing convention.

## Signing

Every commit that reaches `main` must be signed, and `main`'s history stays linear: no force-push, no unsigned commits, enforced by branch protection (GOV-001-R15). Configure commit signing locally (GPG or SSH) before opening a pull request that targets `main`.

## Sign-off

Every contribution requires a sign-off: a Developer Certificate of Origin (DCO) by default, per docs/governance.md § Contributor governance. Sign and sign off in the same commit:

```sh
git commit -s -S
```

`-s` adds the DCO `Signed-off-by` trailer; `-S` signs the commit itself.

## Review

Contributions arrive through a pull request and are reviewed by a Project Maintainer before merge. A Maintainer review is advice toward merge — it is not the artifact-status approval that docs/governance.md § Status workflow and evidence per transition defines for a Draft/Proposed/Accepted transition. A pull request that changes normative text in an Accepted artifact still follows Change control below.

## Commit style

Use Conventional Commits: `type: summary`, matching the existing history (`docs: accept GOV-001 governance and support matrix`, `docs: record the second owner signing key in the assignments register`, and similar). Keep the summary in the imperative mood and under about 72 characters; put detail in the commit body. Other types (`feat`, `fix`, `test`, `ci`, `chore`) follow the same `type: summary` shape once this repository's history includes code changes.

## Tests first

A behaviour change lands with its failing test first: write the test, confirm it fails for the expected reason, then make it pass with the minimal change. A pull request that adds behaviour without a preceding failing test is expected to add one before review.

## Normative text

Changing a `MUST` clause in an Accepted artifact does not happen through an ordinary edit. It follows docs/governance.md § Change control: a `Proposed` revision carrying a signed acceptance tag, evidenced at the standard the original acceptance required or higher. Draft artifacts remain open to ordinary editing.
