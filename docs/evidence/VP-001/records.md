# VP-001 scenario records

Append-only. See `README.md` for the record grammar and the `kind: scenario`
mandatory-field list.

## VP-001 VP-S6 scenario -- run 35252892166 (2026-09-17)

The run never reached launch: the native GTK workspace-picker dialog could not be
driven to completion under the runner's `Xvfb` before Start became reachable, so
no fixture process was ever spawned and the containment mechanism below was not
exercised on this baseline. `observed_state: orphan-risk` here means "termination
not proven", not "an orphan was observed" -- no descendant process existed to
become one.

| field | value |
| --- | --- |
| `record_id` | VP-001-VP-S6-01 |
| `kind` | scenario |
| `scenario_id` | VP-S6 |
| `baseline_id` | VP-001-BASE-01 |
| `result` | uncertain |
| `observed_state` | orphan-risk |
| `build_channel` | packaged-ci |
| `build_channel_digest` | d7e1f946ba6bea057626c7d9b5f1986584e6adee58cbbec273085b1c21c94f68 |
| `exercised_artifact_digest` | d7e1f946ba6bea057626c7d9b5f1986584e6adee58cbbec273085b1c21c94f68 |
| `variant_scan` | absent |
| `procedure_ref` | docs/evidence/VP-001/procedures/vp-s6-linux.sh |
| `mechanism` | process-group killpg(SIGTERM) (crates/omnifrons-supervisor/src/lib.rs) -- not exercised this run: no fixture process was ever started |
| `fallback` | killpg(SIGKILL) escalation (crates/omnifrons-supervisor/src/lib.rs) -- not exercised this run, for the same reason |
| `blocker` | workspace-chooser-timeout -- the workspace-picker GTK dialog did not complete under Xvfb within the bounded wait |
| `identity_gated` | true |
| `descendant_alive_before_stop` | false |
| `stop_confirmed` | false |
| `all_pids_proven_gone` | false |
| `any_pid_alive_after_wait` | false |
| `enumeration_unreadable` | false |
| `ci_run` | workflow tauri-build.yml, workflow_dispatch (vp001_scenario: true), run 35252892166, conclusion success |
| `retention` | GitHub retains this run's `vp-001-linux-baseline` and `vp-001-transcripts` artifacts only until 2026-12-16 |
| `evidence_artifact` | e5ac5767b42ea3716ce83aa5c803a11cdbd7747dade1c021ccda460897132bf6 -- redacted copy of run 35252892166's `vp-001-transcripts` artifact entry `vp-001-vp-s6-transcript.txt`, retained at `docs/evidence/VP-001/artifacts/vp-001-run-35252892166-vp-s6-transcript.txt` |
| `feasibility_evidence_artifact` | d698e3dc4be6677209e977ac1de01aebc6ee008bd063456aa28c3770401d4dcf -- redacted copy of the same run's `vp-001-transcripts` artifact entry `vp-001-feasibility-transcript.txt`, retained at `docs/evidence/VP-001/artifacts/vp-001-run-35252892166-feasibility-transcript.txt` |

## VP-001 VP-S6 scenario -- run 35450847942 (2026-09-19)

The scenario ran to completion on the pinned baseline for the first time: the
workspace chooser was reached through a seeded GTK places bookmark rather than
a location entry, the fixture agent was approved and started, and it spawned
the breakaway descendant the scenario exists to chase. After Stop, the
product's own badge reported `exited (code unreported)` while the descendant
was still alive: the supervisor's `killpg` reaches only its own process group,
and a descendant that calls `setsid` has left it. `result: fail` records an
observed survivor, not an unproven termination -- the distinct case from
`VP-001-VP-S6-01`, which is `uncertain` because its run never reached launch.

This row does not correct VP-001-VP-S6-01 and carries no `corrects` field: that
row honestly records what its own run observed. Both stand.

| field | value |
| --- | --- |
| `record_id` | VP-001-VP-S6-02 |
| `kind` | scenario |
| `scenario_id` | VP-S6 |
| `baseline_id` | VP-001-BASE-01 |
| `result` | fail |
| `observed_state` | orphan-risk |
| `build_channel` | packaged-ci |
| `build_channel_digest` | 17070532b5085d97b72f21badb847c5b9c3180994356c649aff3768a52edbf8e |
| `exercised_artifact_digest` | 17070532b5085d97b72f21badb847c5b9c3180994356c649aff3768a52edbf8e |
| `variant_scan` | absent |
| `procedure_ref` | docs/evidence/VP-001/procedures/vp-s6-linux.sh |
| `mechanism` | process-group killpg(SIGTERM) (crates/omnifrons-supervisor/src/lib.rs) -- exercised this run: the direct child was reaped and reported exited |
| `fallback` | killpg(SIGKILL) escalation (crates/omnifrons-supervisor/src/lib.rs) -- not reached this run: escalation fires only when the direct child itself is not reaped, and it was |
| `blocker` | none -- the scenario ran to completion |
| `identity_gated` | true |
| `descendant_alive_before_stop` | true |
| `stop_confirmed` | true |
| `all_pids_proven_gone` | false |
| `any_pid_alive_after_wait` | true |
| `enumeration_unreadable` | false |
| `run_date` | 2026-09-19 |
| `observed_pids` | agent pid 13540 gone after the bounded wait; breakaway descendant pid 13542 still alive with its original starttime |
| `ci_run` | workflow tauri-build.yml, workflow_dispatch (vp001_scenario: true), run 35450847942, head cd9e9ae, conclusion success |
| `retention` | GitHub retains this run's `vp-001-linux-baseline` and `vp-001-transcripts` artifacts only until 2026-12-18 |
| `evidence_artifact` | e9b253854d9fba71fa0ac81bb2d6e43cdb66732b8b84d0aefd78714a89226f17 -- redacted copy of run 35450847942's `vp-001-transcripts` artifact entry `vp-001-vp-s6-transcript.txt`, retained at `docs/evidence/VP-001/artifacts/vp-001-run-35450847942-vp-s6-transcript.txt` |
| `feasibility_evidence_artifact` | 4b621369c2413ac773cc609156d5fc8cf98a5afcc162bfd091caa7fac69b7775 -- redacted copy of the same run's `vp-001-transcripts` artifact entry `vp-001-feasibility-transcript.txt`, retained at `docs/evidence/VP-001/artifacts/vp-001-run-35450847942-feasibility-transcript.txt` |

## VP-001 VP-S6 scenario -- run 35467644986 (2026-09-19)

The same scenario as `VP-001-VP-S6-02`, on the same baseline, against a build
that contains the supervisor fix that row drove. The fixture agent spawned the
same breakaway descendant, and after Stop every recorded pid was proven gone
within the bounded wait: `self_after_wait=gone descendant_after_wait=gone`.

The descendant had left the process group via `setsid`, so `killpg` could not
have reached it, and the fixture's own 120-second self-bound had not elapsed
within the scenario's bounded wait. What accounted for it is the census and
pid-targeted sweep added in `crates/omnifrons-supervisor/src/descendants.rs`.

`observed_state: proven-gone` names the state the passing branch of `derive`
returns (`ObservedState::Verify`). The design left that public token unnamed;
it is chosen here to mirror `orphan-risk` and the `all_pids_proven_gone`
observation it reports. It does not claim containment in general: a descendant
born between the census and the signal remains outside both mechanisms, and
`src-tauri/src/health.rs` still reports containment as `unproven`.

| field | value |
| --- | --- |
| `record_id` | VP-001-VP-S6-03 |
| `kind` | scenario |
| `scenario_id` | VP-S6 |
| `baseline_id` | VP-001-BASE-01 |
| `result` | pass |
| `observed_state` | proven-gone |
| `build_channel` | packaged-ci |
| `build_channel_digest` | d52091e8decb5e488498b72d2bbcfd3a9653311222bb40f40f7b52b94a9ff24f |
| `exercised_artifact_digest` | d52091e8decb5e488498b72d2bbcfd3a9653311222bb40f40f7b52b94a9ff24f |
| `variant_scan` | absent |
| `procedure_ref` | docs/evidence/VP-001/procedures/vp-s6-linux.sh |
| `mechanism` | process-group killpg(SIGTERM) plus the pid-targeted sweep of recorded survivors (crates/omnifrons-supervisor/src/descendants.rs) -- exercised this run |
| `fallback` | killpg(SIGKILL) escalation -- not reached this run: the direct child was reaped, and escalation fires only when it is not |
| `blocker` | none -- the scenario ran to completion |
| `identity_gated` | true |
| `descendant_alive_before_stop` | true |
| `stop_confirmed` | true |
| `all_pids_proven_gone` | true |
| `any_pid_alive_after_wait` | false |
| `enumeration_unreadable` | false |
| `run_date` | 2026-09-19 |
| `observed_pids` | agent pid 13255 and breakaway descendant pid 13257, both proven gone after the bounded wait |
| `ci_run` | workflow tauri-build.yml, workflow_dispatch (vp001_scenario: true), run 35467644986, head 41a42ef, conclusion success |
| `retention` | GitHub retains this run's `vp-001-linux-baseline` and `vp-001-transcripts` artifacts only until 2026-12-18 |
| `evidence_artifact` | fdfc38584a25fe028f3c1a35895bf7bb7f4da38cde65c0b5d628084113aedb3c -- redacted copy of run 35467644986's `vp-001-transcripts` artifact entry `vp-001-vp-s6-transcript.txt`, retained at `docs/evidence/VP-001/artifacts/vp-001-run-35467644986-vp-s6-transcript.txt` |
| `feasibility_evidence_artifact` | 07c2c440c22779a35b5e2736a417cbe0e4080f4533754f9e35778f8b2634284a -- redacted copy of the same run's `vp-001-transcripts` artifact entry `vp-001-feasibility-transcript.txt`, retained at `docs/evidence/VP-001/artifacts/vp-001-run-35467644986-feasibility-transcript.txt` |
