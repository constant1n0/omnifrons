# Fixture scenario records (synthetic -- not real VP-001 evidence)

Everything below is a synthetic fixture for
`tools/evidence-validator`'s integration test. It references
`synthetic-baselines.md`'s fixture baseline and exists only so
`cargo test --workspace` has a realistic multi-record file to parse and
validate before Slice 4 files the real `docs/evidence/VP-001/records.md`
row.

## Fixture VP-S6 row

| field | value |
| --- | --- |
| `record_id` | VP-001-SYNTHETIC-VP-S6-01 |
| `kind` | scenario |
| `scenario_id` | VP-S6 |
| `baseline_id` | VP-001-SYNTHETIC-BASE-01 |
| `result` | uncertain |
| `observed_state` | orphan-risk |
| `build_channel` | packaged-ci |
| `variant_scan` | absent |
| `procedure_ref` | docs/evidence/VP-001/procedures/vp-s6-linux.sh |
| `build_channel_digest` | sha256:fixture-digest-0000000000000000000000000000000000000000000000000000000000 |
| `exercised_artifact_digest` | sha256:fixture-digest-0000000000000000000000000000000000000000000000000000000000 |
| `evidence_artifact` | fixture-artifact-01 |

## Fixture correction row

Corrects the row above (append-only correction, never an in-place edit).

| field | value |
| --- | --- |
| `record_id` | VP-001-SYNTHETIC-VP-S6-02 |
| `kind` | scenario |
| `scenario_id` | VP-S6 |
| `baseline_id` | VP-001-SYNTHETIC-BASE-01 |
| `result` | fail |
| `observed_state` | orphan-risk |
| `build_channel` | packaged-ci |
| `variant_scan` | absent |
| `procedure_ref` | docs/evidence/VP-001/procedures/vp-s6-linux.sh |
| `build_channel_digest` | sha256:fixture-digest-0000000000000000000000000000000000000000000000000000000000 |
| `exercised_artifact_digest` | sha256:fixture-digest-0000000000000000000000000000000000000000000000000000000000 |
| `evidence_artifact` | fixture-artifact-02 |
| `corrects` | VP-001-SYNTHETIC-VP-S6-01 |
