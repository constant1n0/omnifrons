# OpenSpec artifacts

This directory holds the specification and delivery record of each change, kept
next to the code it governs.

- `specs/` — the accepted capability specifications. Each file states what a
  capability must do, in requirements with Given/When/Then scenarios. A
  specification lands here only when its change is archived, so this directory
  describes the system as agreed, not as proposed.
- `changes/` — one directory per change in flight, holding its proposal,
  specification deltas, design, task breakdown and, once it runs, its
  verification report.
- `changes/archive/` — the same artifacts for closed changes, dated. They are
  kept because a reader asking why something is the way it is usually needs the
  reasoning, not just the result.
- `config.yaml` — tooling configuration: the project context, the test commands
  each phase runs, and the rules a change is held to.

A specification here says what must be true. Evidence that it actually is true,
on a pinned baseline, lives in [`docs/evidence/`](../docs/evidence/).
