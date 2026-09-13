# Review Ledger: spike-slice-5f

## Design — Judgment Day, corrective round 1

Target: `design.md` and its proposal/specification contracts; no implementation exists.
First-pass dual review independently confirmed two critical findings; both fresh judges verified their round-1 corrections. Scoped re-review used only the ledger and fix diff.
Current state: JUDGMENT: APPROVED. Automatic design-gate refresh passed (#8528); planning tasks are complete, but cleanup implementation has not started.

| id | lens | location | severity | status | evidence |
|---|---|---|---|---|---|
| JD-001 | judgment-day | openspec/changes/spike-slice-5f/design.md:13,26-29,33-36,68 | CRITICAL | verified | Both scoped judges verified checked strictly-positive signed pid_t conversion, Unknown/retain on failure, no invalid kill call, and explicit boundary regressions. |
| JD-002 | judgment-day | openspec/changes/spike-slice-5f/design.md:14,25-27,68 | CRITICAL | verified | Both scoped judges verified directory-constrained root opening, nonblocking/no-follow candidate opening before fstat, and the deterministic FIFO no-block regression requirement. |
| JD-B-003 | judgment-day | openspec/changes/spike-slice-5f/design.md:16,65 | WARNING | info | A repeated 256-entry scan has no progress mechanism; a stable prefix of final artifacts can indefinitely hide later staging files. Reported once; does not block or enter the fix/re-review loop. |
| JD-B-004 | judgment-day | openspec/changes/spike-slice-5f/design.md:13 | CRITICAL | refuted | Judge B alone claimed removing the duplicate signal-feature sentence made the design unimplementable. The original File Changes row for crates/omnifrons-adapters/Cargo.toml explicitly says Enable nix signal; that row was not changed by the supplied fix diff. This is a missing-context false positive, not a missing feature requirement; no fix is warranted. |

Fix budget: at most two rounds; one used. Scoped re-review must receive only this ledger and the round-1 fix diff, not the full original design.
Mirrors: Engram `sdd/spike-slice-5f/review-ledger`; design gate `sdd/spike-slice-5f/design-gate` (#8528).

## Planning Delivery — macOS CI

PR #16 at `541fad9880c0d04b71e3dd7130c1fbdcbeba2b60` is signed and GitHub-verified. Ubuntu, Windows, docs-links, and gitleaks passed; macOS failed. No merge or rerun-to-green occurred.
One reliability sweep and one general batched refuter independently confirmed the following pre-existing test-contract defect. The separate test-only PR #17 merged by normal fast-forward at `f1b9cfe2f48791c3aa4d29aa7a0fe98154e232bc`; GitHub signature and DCO verification passed, as did Ubuntu, macOS, Windows, docs-links, and gitleaks.

| id | lens | location | severity | status | evidence |
|---|---|---|---|---|---|
| R3-001 | reliability | crates/omnifrons-supervisor/tests/output_backpressure.rs:96-105 | BLOCKER | verified | Scoped review confirms the 92-line test-only correction preserves integration guarantees and deterministically proves saturation and refill/drop accounting. Independent execution audit selected and passed each of the two new unit tests and the integration test (1/1 each). Linux workspace tests, fmt and supervisor clippy also pass. Separate PR #17 is merged at the recorded SHA with all required cross-platform checks passing. |

Evidence: PR #17 https://github.com/constant1n0/omnifrons/pull/17, merged by normal fast-forward at `f1b9cfe2f48791c3aa4d29aa7a0fe98154e232bc`.
