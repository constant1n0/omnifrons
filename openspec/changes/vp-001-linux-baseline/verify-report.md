```yaml
schema: gentle-ai.verify-result/v1
evidence_revision: sha256:972593cbf1c08e1c196a4cd5594635a3bfa0ce972df86a61e374f57011289db8
verdict: fail
blockers: 1
critical_findings: 1
requirements: 8/11
scenarios: 18/22
test_command: cargo test --workspace
test_exit_code: 0
test_output_hash: sha256:8218065e164d15a1099dcf0f9809b73e4a83dbcf5bfd79ac65b6d1ab58d21b0f
build_command: cargo check --workspace
build_exit_code: 0
build_output_hash: sha256:4fe007f4628719861f379631180f41291b61e87db704d9b69fb2fe328ca68046
```

## Verification Report

**Change**: vp-001-linux-baseline
**Version**: N/A
**Mode**: Strict TDD
**Repository state**: `main` at `5199b0e`, working tree clean, all slices merged

### Completeness
| Metric | Value |
|---|---:|
| Tasks total | 49 |
| Tasks complete | 49 |
| Tasks incomplete | 0 |

### Build & Tests Execution

**Build**: ✅ Passed
```text
cargo check --workspace   -> exit 0 (Finished `dev` profile)
cargo fmt --all -- --check -> exit 0 (no output)
cargo clippy --workspace --all-targets -- -D warnings -> exit 0 (0 warnings, 0 errors)
```

**Tests**: ✅ 1,669 passed / ❌ 0 failed / ⚠️ 0 skipped
```text
cargo test --workspace: exit 0 — 869 passed, 0 failed across 87 harnesses,
  zero `test result: FAILED` lines.
renderer pnpm test: exit 0 — 763 passed (7 files), Vitest 4.
node --test docs/evidence/VP-001/procedures/*.test.mjs: exit 0 — 37 pass, 0 fail.
```

Change-owned suites (fresh, re-run during this verification):
`evidence-validator` 39 (lib 4, derive 7, fixtures 2, real_evidence_store 2, V1 4, V2 2,
V3 3, V4/V5/V6 8, V7/V8 7); `omnifrons-shell --test config` 5; `node --test` 37. Total 81.

**Coverage**: ➖ Not available (`openspec/config.yaml` declares `coverage.available: false`;
`coverage_threshold: 0`).

### TDD Compliance
| Check | Result | Details |
|---|---|---|
| TDD evidence reported | ✅ | 5 `TDD Cycle Evidence` tables in apply-progress.md (:55, :174, :415, :678, :989) |
| All tasks have tests | ✅ | Every code-bearing task names a test file; evidence/CI-only tasks declare `N/A` with reason |
| RED confirmed (tests exist) | ✅ | All named test files exist and were re-read |
| GREEN confirmed (tests pass) | ✅ | 81/81 change-owned tests pass on fresh execution |
| Triangulation adequate | ✅ | `derive` has 7 cases spanning pass / fail / three distinct uncertain paths / default / separation |
| Safety net for modified files | ✅ | `config.rs` and `webdriver-session.*` modifications record prior-suite baselines |

**TDD Compliance**: 6/6 checks passed

The task 4.4a RED claim was independently re-verified, not taken on trust. Driving the real
`docs/evidence/VP-001/{baselines,records}.md` text through `evidence_validator::validate` from an
isolated harness reproduced the exact recorded RED (`field-missing` on `test_date` cascading into
`baseline-unpinned` on the row) and confirmed the standing guard: because
`real_evidence_store_parses_and_validates_clean` reads both real files from disk at runtime and
asserts `violations.is_empty()`, any future malformed real record turns `cargo test --workspace`
red with no workflow change. Seven further malformed shapes of the real records were rejected with
the correct tokens (compound `result`, digest mismatch, `variant_scan: found`, non-`packaged-ci`
channel, missing baseline, duplicate `record_id`, unresolved `corrects`, injected runner-local path).

### Test Layer Distribution
| Layer | Tests | Files | Tools |
|---|---:|---:|---|
| Unit | 76 | 10 | Rust test harness, `node --test` |
| Integration | 5 | 1 | Rust harness over real repository files (`real_evidence_store.rs`, `config.rs`) |
| E2E | 0 | 0 | Deliberately none — design.md Testing Strategy declares the live browser-driving path evidence machinery, never a repository test |
| **Total (change-owned)** | **81** | **11** | |

### Changed File Coverage
Coverage analysis skipped — no coverage tool configured.

### Assertion Quality
**Assertion quality**: ✅ No tautologies, ghost loops, orphan empty checks, smoke-only tests, or
assertions that never call production code were found across the 11 change-owned test files.
Rejection tests assert on a specific violation token plus the named field, not merely on
non-emptiness. One weak assertion is noted as a SUGGESTION below.

### Quality Metrics
**Linter**: ✅ `cargo clippy --workspace --all-targets -- -D warnings` clean.
**Type Checker**: ✅ `cargo check --workspace` clean. **Formatter**: ✅ `cargo fmt --check` clean.

### Spec Compliance Matrix

#### packaged-build-evidence (4 requirements, 8 scenarios)
| Requirement | Scenario | Test / runtime evidence | Result |
|---|---|---|---|
| Pinned Build Environment | Pinned build records its runner image | `config.rs > bundle_targets_is_an_explicit_non_default_array`; workflow pins `ubuntu-24.04` (tauri-build.yml:42), `tauri-action@action-v1.0.0` (:90), `upload-artifact@v7.0.1` (:153); run 35252892166 recorded image `20260907.300.1` into `baselines.md:18` | ✅ COMPLIANT |
| Pinned Build Environment | Missing runner image version blocks admissibility | (none found) — no validator check, no test | ❌ UNTESTED |
| Retained, Digest-Addressable Artifact | Retained artifact is digest-bound | `Digest artifact` step (tauri-build.yml:122,144) + `v7_v8_artifact_identity_and_channel.rs`; `records.md:24` binds `d7e1f946…` | ✅ COMPLIANT |
| Retained, Digest-Addressable Artifact | Two builds are not claimed identical | Inspection: design.md:128-130 explicitly disclaims rebuild identity; no reproducibility claim exists in the store | ✅ COMPLIANT |
| Packaged-Build-Only Admissibility | Packaged artifact is accepted | `v7_v8_artifact_identity_and_channel.rs` (V8) + `real_evidence_store.rs`; row carries `build_channel: packaged-ci` | ✅ COMPLIANT |
| Packaged-Build-Only Admissibility | Dev-mode build is rejected | `v7_v8_artifact_identity_and_channel.rs` (V8 `dev-mode-only`); re-confirmed against the real row | ✅ COMPLIANT |
| Exercised Artifact Matches Digested Artifact | Exercised artifact matches its recorded digest | AV1 in `vp-s6-linux.sh` + V7 equality; transcript `gate=av1-digest-match digest=d7e1f946…` equals `build_channel_digest`; `exercised_artifact_digest` == `build_channel_digest` | ✅ COMPLIANT |
| Exercised Artifact Matches Digested Artifact | Feature-enabled variant is inadmissible | `feature_gate.rs` (feature off by default, literal absent from binary) + V7 `feature-enabled-variant`; transcript `gate=av2-variant-scan-absent` | ✅ COMPLIANT |

#### verification-evidence-store (7 requirements, 14 scenarios)
| Requirement | Scenario | Test / runtime evidence | Result |
|---|---|---|---|
| Baseline-Before-Scenario Admission | Complete baseline admits its scenario row | `v3_baseline_before_scenario.rs`, `real_evidence_store.rs` (2/2) | ✅ COMPLIANT |
| Baseline-Before-Scenario Admission | Absent baseline blocks the row | `v3_baseline_before_scenario.rs`; re-confirmed: the real row alone yields `baseline-unpinned` | ✅ COMPLIANT |
| Closed Outcome Vocabulary | Unproven termination records uncertain | `derive.rs` (7 cases); `records.md:21-22` carries `uncertain` + `orphan-risk` | ✅ COMPLIANT |
| Closed Outcome Vocabulary | An out-of-vocabulary value is rejected | `v2_result_vocabulary.rs`; re-confirmed: compound `orphan-risk/uncertain` rejected as `result-out-of-vocabulary` | ✅ COMPLIANT |
| Mandatory-Field Validation | Incomplete row is rejected and observed | `v1_mandatory_fields.rs` (each field omitted in turn, both kinds) + `real_evidence_store.rs` standing guard | ✅ COMPLIANT |
| Mandatory-Field Validation | Complete row passes validation | `complete_baseline_and_scenario_pass_v1`, `real_evidence_store_parses_and_validates_clean` | ✅ COMPLIANT |
| Append-Only Correction | Correction appends a new row | `v4_v5_v6_identifiers_and_provenance.rs` (V4 `corrects` resolution, V5 artifact reuse); protocol documented in store README:20-23 | ✅ COMPLIANT |
| Append-Only Correction | In-place edit is refused | No mechanical enforcement; verified by history inspection instead (see WARNING 3) | ⚠️ PARTIAL |
| Provenance-Clean Publication | Public row carries no infrastructure detail | `v4_v5_v6_identifiers_and_provenance.rs` (V6) + independent scan of both records and both retained transcripts: no hostname, path, user, or address token | ✅ COMPLIANT |
| Provenance-Clean Publication | Sensitive value is referenced, not embedded | Both transcripts redact the runner-local extract path to `<extract-dir>/…`; `evidence_artifact` names the retained file by SHA-256 | ✅ COMPLIANT |
| VP-S6 Row Content and Recorded Tension | Mechanism and fallback recorded distinctly | `records.md:28` (`mechanism`) and `:29` (`fallback`) are two separate fields, each marked not exercised this run | ✅ COMPLIANT |
| VP-S6 Row Content and Recorded Tension | Unconfirmed termination yields orphan-risk/uncertain | `derive.rs`; independently reproduced — `derive()` over the transcript's own six observations returns exactly `(Uncertain, OrphanRisk)` | ✅ COMPLIANT |
| Blocked or Non-Reproducible Verification Attempt | Driver cannot attach to the packaged binary | Generic `blocker=` → `uncertain` path implemented and proven at runtime, but by a different trigger (chooser timeout, not attach failure — F1 succeeded) | ⚠️ PARTIAL |
| Blocked or Non-Reproducible Verification Attempt | Flaky run does not reproduce within bounded attempts | No-retry is structural (one invocation, one bounded `pollUntil` per observation, no loop, no `continue-on-error`); verified by inspection, never exercised | ⚠️ PARTIAL |

**Compliance summary**: 18/22 scenarios compliant (3 partial, 1 untested); 8/11 requirements
fully compliant.

### Correctness (Static Evidence)
| Requirement | Status | Notes |
|---|---|---|
| Pinned Build Environment | ⚠️ Partial | Pinning and recording are implemented and exercised; the admissibility block for a missing runner image version is absent |
| Retained, Digest-Addressable Artifact | ✅ Implemented | `sha256sum` over `artifactPaths` feeding pinned `upload-artifact`; identity-only, no reproducibility claim |
| Packaged-Build-Only Admissibility | ✅ Implemented | V8 admits only `packaged-ci` and refuses a digest with no retained artifact |
| Exercised Artifact Matches Digested Artifact | ✅ Implemented | AV1/AV2 gate before session creation; V7 independently re-checks the filed row |
| Baseline-Before-Scenario Admission | ✅ Implemented | V3 resolves `baseline_id` and re-checks every VP-001-R1 field on the resolved baseline |
| Closed Outcome Vocabulary | ✅ Implemented | V2 closed set; `result` and `observed_state` are two distinct mandatory fields in V1 |
| Mandatory-Field Validation | ✅ Implemented | V1 per-kind mandatory sets; rejection observable through `cargo test --workspace` on the real files |
| Append-Only Correction | ⚠️ Partial | `corrects` protocol and V4/V5 implemented; in-place rewrite is prevented by convention and branch policy, not by the store |
| Provenance-Clean Publication | ✅ Implemented | V6 lint, honestly documented as a lint and not a proof (store README:79-82) |
| VP-S6 Row Content and Recorded Tension | ✅ Implemented | Two distinct observations filed; VP-001-R6 (:227) vs catalog (:127) tension carried forward unresolved in design.md:213-215 and the spec itself |
| Blocked or Non-Reproducible Verification Attempt | ✅ Implemented | The row records `uncertain` with `blocker=workspace-chooser-timeout` disclosed; no retry occurred and no rename was attempted |

### Coherence (Design)
| Decision | Followed? | Notes |
|---|---|---|
| D1 record format (`##` section + two-column table) | ✅ Yes | Both real files follow it; parser needs no new crate |
| D2 validator as non-product workspace member | ✅ Yes | `members = ["crates/*", "src-tauri", "tools/*"]`; no product crate depends on it |
| D3 store layout | ✅ Yes | `README.md`, `baselines.md`, `records.md`, `procedures/`, plus `artifacts/` added and documented |
| D4 CI wiring and pins | ✅ Yes | `ubuntu-24.04`, pinned action tags, Linux-guarded facts/digest/harness/upload steps |
| D5 driving the retained bundle's own GUI | ✅ Yes | `tauri-driver` under `Xvfb` against the gate-checked file; `demo-harness` never enabled |
| D6 zero-dependency Node WebDriver client | ✅ Yes | Pure builders exported separately from the `fetch` transport; 37 unit tests, no new dependency |
| D7 `xdotool` chooser driving, seeding as fallback | ✅ Yes | Primary path used; the fallback was not needed and correctly not silently substituted |
| D8 fixture ELF agent | ✅ Yes | `tools/vp-s6-agent/` exists; never reached on this run, and the row says so |
| D9 six slices | ✅ Yes | Six slices, each under the 400-line budget (largest authored commit 148 lines) |
| Honesty machinery #1 (procedure emits observations only) | ✅ Yes | Transcript carries `observation=` lines and a `blocker=`; it names no outcome |
| Honesty machinery #2 (`pass` needs three positive proofs) | ✅ Yes | Independently re-derived: no single flip of this run's observations could have produced `pass` |
| Honesty machinery #3 (no retry) | ✅ Yes | One invocation; the blocked run was filed, not re-run |
| Honesty machinery #4 (append-only, V5 distinct artifact) | ✅ Yes | Git history shows no record was ever rewritten |
| Honesty machinery #6 (AV1/AV2 before the session) | ✅ Yes | Both gates appear in the transcript before `gate=f1-session-created` |
| Validator contract V8 ("carries both digest and artifact") | ⚠️ Deviation | Only the digest-without-artifact direction is enforced (see WARNING 1) |
| `variant_scan` vocabulary (`absent`/`found`/`unperformed`) | ⚠️ Deviation | V7 admits only `absent` (see WARNING 2) |

### Scope Discipline (declared out of scope, re-checked)
| Out-of-scope claim | Held? | Evidence |
|---|---|---|
| No cgroup or watchdog implementation | ✅ | Only occurrence in Rust source is the pre-existing disclaimer comment `src-tauri/src/health.rs:19-20` (commit 9232d5a, predates this change) |
| No other VP scenarios filed | ✅ | `records.md` holds exactly one row, `scenario_id: VP-S6` |
| No acceptance of VP-001 or ADR-0002 | ✅ | `health.rs` still reports `containment: "unproven"`; neither `docs/desktop-stack-verification-plan.md` nor `adr/0002-…` was touched by this change |
| No evidence-only cargo feature in the exercised artifact | ✅ | `demo-harness = []` stays non-default; `feature_gate.rs` passes; AV2 scan recorded `absent` |
| No new shipped CLI surface | ✅ | `src-tauri/src/main.rs` untouched (last commit 8c5e831, predates the change) |

### Issues Found

**CRITICAL**:
1. **`Pinned Build Environment` scenario 2 is unenforced and untested.** The spec requires that an
   artifact whose job captured no runner image version "MUST NOT be admissible until the version is
   read from the job log and recorded"
   (`specs/packaged-build-evidence/spec.md:24-27`). Nothing implements that block.
   `runner_image` is absent from `BASELINE_MANDATORY`
   (`tools/evidence-validator/src/validate/v1_mandatory_fields.rs:11-20`) and from `VP001_R1_FIELDS`
   (`.../v3_baseline_before_scenario.rs:9-16`), and no test references the field anywhere in the
   repository. Verified empirically: deleting `runner_image` from the real baseline
   (`docs/evidence/VP-001/baselines.md:18`) and re-running `parse` + `validate` returns **zero
   violations** — the record is admitted. The value is recorded for this one artifact, and
   design.md:116 documents that it is *transcribed by hand* from the run log, which is exactly the
   case this scenario exists to guard. The positive half of the requirement is met; the blocking
   half does not exist.

**WARNING**:
1. **V8's "both or neither" pairing is only half-enforced.** The store README states
   `build_channel_digest` and `evidence_artifact` "must both be present together, or neither"
   (`docs/evidence/VP-001/README.md:66-68`), and design.md:109 says the row must carry both.
   `check_packaged_channel` only rejects `has_digest && !has_artifact`
   (`.../v7_v8_artifact_identity_and_channel.rs:66-78`). Confirmed: removing `build_channel_digest`
   while keeping `evidence_artifact` is admitted with zero violations. The filed row carries both,
   so no current record is affected.
2. **`variant_scan: unperformed` cannot be filed.** design.md:123 lists `unperformed` as a
   legitimate AV2 output and design.md:78 states an unperformable gate "is a blocker yielding
   `uncertain`" — but V7 admits only `absent`
   (`.../v7_v8_artifact_identity_and_channel.rs:36-44`), rejecting `unperformed` as
   `feature-enabled-variant`. An honest "AV2 could not be performed" row is therefore unfileable,
   and the rejection token misdescribes the cause. AV2 did run on this baseline, so the filed row is
   unaffected.
3. **In-place rewrite of a filed record is not mechanically detectable.** `Append-Only Correction`
   scenario 2 requires a direct edit or deletion to be refused
   (`specs/verification-evidence-store/spec.md:78-81`). No content-hash chain, history test, or
   validator check would notice a silently edited record; enforcement rests on review and the
   signed-commit, linear-history `main` policy (design.md:208-209), which is outside this change.
   Verified manually that nothing was rewritten: the only deletions across the two evidence files
   (`fcbc9a8`) removed the "no record has been filed yet" placeholder prose from the file headers;
   no record line was ever altered.
4. **`mechanism` and `fallback` are not validator-mandatory.** The spec requires a VP-S6 row to name
   the containment mechanism and record the fallback as two distinct entries
   (`specs/verification-evidence-store/spec.md:100-115`), but neither field appears in
   `SCENARIO_MANDATORY` (`.../v1_mandatory_fields.rs:25-35`), so a future VP-S6 row omitting both
   would validate clean. The filed row carries both, correctly marked "not exercised this run".

**SUGGESTION**:
1. `derive.rs:84-91` (`outcome_and_observed_state_are_always_two_separate_fields`) asserts only that
   two `Debug` renderings differ. The real guarantee is the tuple-of-two-enums return type
   (compile-time) and V2 (runtime); the assertion is a weak proxy for both.
2. No `derive` case uses the exact observation vector the filed row records
   (`identity_gated: true`, the other five `false`). Independently confirmed here that it yields
   `(Uncertain, OrphanRisk)`, but the shape that produced the repository's only real row deserves
   its own named regression case.
3. The two `Blocked or Non-Reproducible Verification Attempt` scenarios are implemented generically
   and never exercised by their own triggers. Graded PARTIAL rather than UNTESTED because the shared
   `blocker=` → `uncertain` path was proven at runtime by a real blocked run.
4. Design Open Questions remain open by intent and are correctly not resolved: the VP-001-R6 (:227)
   versus scenario-catalog (:127) tension, `record_id`/`corrects` absent from the VP-001 record
   table, and the unnamed public `pass` token. `ObservedState::Verify` was implemented for that
   unnamed token and has never been filed on a record.

### What remains unproven
Process-group containment was **never exercised on the pinned baseline**. The run reached
`gate=f1-session-created` and then stopped at `blocker=workspace-chooser-timeout` before Start was
reachable, so no fixture process was spawned, `killpg(SIGTERM)` was never invoked, and the
`killpg(SIGKILL)` escalation was never reached. VP-001's containment question is therefore still
open, and `health.rs` correctly continues to report `containment: "unproven"`.

The `result: uncertain` / `observed_state: orphan-risk` outcome is judged **contract-conformant, not
a failure**: it is the only pair `derive()` admits from the transcript's own observations, it was
filed once with the blocker disclosed, it was not retried in pursuit of a `pass`, it was not
renamed, and the row explicitly states that `orphan-risk` here means "termination not proven" rather
than "an orphan was observed". The evidence pipeline produced, validated, and recorded the honest
outcome correctly. The CRITICAL finding above concerns a missing admissibility check, not this
outcome.

### Verdict
FAIL
49/49 tasks complete and every executed command green (tests, build, lint, format), but
`Pinned Build Environment` scenario 2 has no implementation and no covering test: a baseline with no
`runner_image` is admitted today, which the spec forbids.
