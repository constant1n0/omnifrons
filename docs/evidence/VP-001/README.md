# VP-001 evidence store

Append-only baseline and scenario records for the Linux desktop-stack
verification plan (`docs/desktop-stack-verification-plan.md`), validated by
`tools/evidence-validator` (`cargo test --workspace`). See
`openspec/changes/vp-001-linux-baseline/design.md` for the full rationale
(D1-D9) and the honesty machinery that keeps a blocked or flaky run from
becoming a `pass`.

## Layout

| File | Holds |
|---|---|
| `baselines.md` | One `kind: baseline` record per pinned OS/architecture/runtime combination. |
| `records.md` | One `kind: scenario` record per scenario run against a pinned baseline. |
| `procedures/` | The scripts a scenario's `procedure_ref` field points to. |
| `artifacts/` | Retained evidence files (for example redacted CI transcripts) that a record's `evidence_artifact` field names by digest. Every runner-local path in a retained file is redacted to a neutral placeholder (for example `<extract-dir>/...`) before it is committed. |

Both `.md` files are append-only: a record is added at the end of the file
and never edited or deleted in place (verification-evidence-store spec,
Append-Only Correction). A wrong or superseded row is corrected only by
appending a new row whose `corrects` field names the original record's
`record_id`; the original stays exactly as filed.

## Record grammar

Each record is one `##` heading followed by exactly one two-column table:

```markdown
## <free-form title>

| field | value |
| --- | --- |
| `record_id` | ... |
| `kind` | baseline |
...
```

`record_id` and `corrects` are store-level fields: no VP-001-R1 field names
them, so a future revision of the plan document should adopt them (design.md
Open Questions).

### Baseline record (`kind: baseline`)

Mandatory fields: the six VP-001-R1 fields (Baseline-Before-Scenario
Admission), the two store-level identity fields, and `runner_image` — the
pinned runner image version this change's own spec requires, transcribed by
hand from the run's log because no automated step produces it:

`record_id`, `kind`, `os_build`, `architecture`, `webview_runtime`,
`packaging_substrate`, `assistive_technology`, `test_date`, `runner_image`.

### Scenario record (`kind: scenario`)

Mandatory fields:

`record_id`, `kind`, `scenario_id`, `baseline_id`, `result`,
`observed_state`, `build_channel`, `variant_scan`, `procedure_ref`.

`result` MUST be exactly one of `pass`, `fail`, `uncertain` (Closed Outcome
Vocabulary) -- never a compound rendering such as `orphan-risk/uncertain`.
`observed_state` is a separate field that carries the additional observed
context (for example `orphan-risk`); it is never folded into `result`.

Optional fields, present once a packaged CI run produced them:
`build_channel_digest`, `exercised_artifact_digest`, `evidence_artifact`,
`corrects`. `build_channel_digest` and `evidence_artifact` must both be
present together, or neither -- a digest with no retained artifact is
refused as `dev-mode-only` (packaged-build-evidence spec, Packaged-Build-Only
Admissibility).

## `record_id` uniqueness

Every record's `record_id` MUST be unique across both files. A reused id is
rejected as `duplicate-record-id`. `corrects` MUST resolve to an existing
`record_id`, or it is rejected as `corrects-unresolved`.

## Provenance

Every published value here is provenance-clean: no hostname, local path,
company, estate, agent, or other infrastructure detail (Provenance-Clean
Publication). `tools/evidence-validator`'s V6 check is a lint against
absolute-path and host-shaped values, not a proof -- authors remain
responsible for reviewing a row before it is appended.
