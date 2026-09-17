# Fixture baselines (synthetic -- not real VP-001 evidence)

Everything below is a synthetic fixture for
`tools/evidence-validator`'s integration test. It exists only so
`cargo test --workspace` has a realistic multi-record file to parse and
validate before Slice 4 files the real `docs/evidence/VP-001/baselines.md`
row.

## Fixture baseline

| field | value |
| --- | --- |
| `record_id` | VP-001-SYNTHETIC-BASE-01 |
| `kind` | baseline |
| `os_build` | Synthetic Linux 0.0 (fixture only) |
| `architecture` | x86_64 |
| `webview_runtime` | Fixture-WebKitGTK 0.0.0 |
| `packaging_substrate` | appimage |
| `assistive_technology` | none exercised - VP-S19 out of scope |
| `test_date` | 2000-01-01 |
| `runner_image` | synthetic-runner, image version 0.0.0 (fixture only) |
