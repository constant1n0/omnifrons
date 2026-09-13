# Apply Progress: spike-slice-5f — Batch 1

## Completed
- [x] 1.1 Exact local staging basename parser and retention tables.
- [x] 1.2 Inert age, limits, and path-free report primitives.
- [x] 1.3 Safe PID liveness seams and real owned-child lifecycle coverage.

## TDD Cycle Evidence
| Task | Layer | RED | GREEN | REFACTOR |
|---|---|---|---|---|
| 1.1 | Unit | unresolved parser import | 3/3 | bounds/uppercase: 5 parser tests |
| 1.2 | Unit | unresolved age/report imports | 5/5 | named 24h limit |
| 1.3 | Unit | unresolved PID imports | 8/8 | 9/9 after probe mappings |

## Commands
- RED: `RUSTUP_AUTO_INSTALL=0 RUSTUP_NO_UPDATE_CHECK=1 /home/dcm/.cargo/bin/rustup run 1.98.1 cargo test -p omnifrons-adapters local_staging_cleanup` failed on unresolved parser, age/report, then PID imports.
- GREEN: the same command passed 3/3, 5/5, and 8/8; triangulation/refactor finished 9/9.
- Gates passed: `cargo fmt --all -- --check`, adapter clippy/check, and `cargo test --workspace --all-targets --all-features`.
- CI correction RED: Windows-target adapter clippy reproduced `derivable_impls` (exit 101).
- CI correction GREEN: cross-Windows and Linux adapter clippy, fmt, and the focused 9 tests passed; Windows runtime remains CI-only.

## Boundary
Cleanup remains inert: no `LocalDirBlobStore`, filesystem, IPC, UI, or deletion-path changes.
JD-001/JD-002 regressions are preserved; JD-B-003 remains an out-of-scope bounded-prefix limitation.
Current slice: stacked-to-main PR 1 from `origin/main`; rollback removes this inert module and nix `signal` feature.
