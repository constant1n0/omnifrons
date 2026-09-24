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

## VP-001 VP-S1 scenario -- run 36070751147 (2026-09-24)

The first execution of VP-S1, against the packaged AppImage on the pinned
Linux baseline, with the probe `docs/evidence/VP-001/procedures/vp-s1-probe.mjs`
run inside the renderer through WebDriver. Every fact the probe was built to
observe was observed, and the outcome is `uncertain` by construction: the
scenario's claim covers "every renderer surface, including a third-party app
surface", and no third-party app surface exists in this renderer (ADR-0004
defers app packaging and any SDK; the renderer is one window, one document,
three panels). `derive_csp` returns `uncertain` whenever
`third_party_surface_exercised` is false, by the maintainer's decision; the
row does not discharge RCS-001-R13 or VP-001-R11.

What the retained transcript shows, on the surface that exists: the
document's scheme was `tauri:`, never a development URL; no CSP `<meta>`
element was present, so on this baseline the policy reached the webview by
another path (the vendored Tauri source names an HTTP header on the
custom-protocol response; this run did not observe the header itself, only
the absence of the element); the policy dump read back from the first
violation event matches `src-tauri/tauri.conf.json`'s `app.security.csp`
directive for directive, except `script-src`, which reads
`'self' 'self' '<hash>'`: the configured `'self'` once more, and one hash
Tauri injects for its own initialization script; a repeated source
expression changes nothing the policy permits; three
`securitypolicyviolation` events were recorded for the three attempts, the
inline script did not run, the fetch settled rejected, and the frame was
left holding `about:blank`.

What it does not show: which directive each event named. The three
`*_violation` facts are `summarize`'s prefix matches over the events'
`effectiveDirective` (`script-src*`, `connect-src*`, `frame-src*`), and the
transcript carries those booleans and the count, not the events themselves
-- the probe captures each event's directive and blocked URI but does not
yet emit them. So the transcript alone cannot re-attribute an event to an
attempt, and cannot by itself separate "the fetch was rejected because
`connect-src` blocked it" from "a `connect-src` event fired and the fetch
also failed to resolve": the reserved `.invalid` host never resolves, and a
plain network failure settles with the same `TypeError`. That the
`connect-src` event fired at all is what the row rests on. Emitting the
events verbatim is recorded follow-up work for the probe.

Both retained transcripts open with a connection-refused line from
`tauri-driver`: the workflow polls `http://127.0.0.1:4444/status` until the
driver answers, and the first poll lands before it listens. The same line
opens every retained VP-S6 transcript; it precedes the first gate and is
not part of the scenario.

`identity_gated` is not an observation line: it is derived from the two
gates the transcript does carry, `gate=av1-digest-match` and
`gate=av2-variant-scan-absent`, both before `gate=session-created`, exactly
as the VP-S6 rows derive it. Those gates appear in the VP-S1 transcript
because `vp-s1-linux.sh` runs `vp-s6-linux.sh --mode=feasibility-check`
itself, which also opens and closes one feasibility session of its own
(`gate=f1-session-created`); the separately retained feasibility transcript
is the workflow's earlier feasibility step, a distinct session. The run
therefore gated identity twice, once per step, against the same artifact
digest.

`observed_state: unverified` names the state `derive_csp`'s `uncertain`
branch returns (`CspObservedState::Unverified`), chosen there as VP-S6-03
chose `proven-gone`: RCS-001's signal mapping has no entry for this. It
means nothing was demonstrated either way *about the claim as written*; the
surface that exists showed no bypass.

Each `evidence_artifact` digest below is the SHA-256 of the retained,
redacted file at the path it names -- the file in this repository, not the
CI artifact entry it was copied from, whose runner paths were rewritten to
`<extract-dir>` before retention.

| field | value |
| --- | --- |
| `record_id` | VP-001-VP-S1-01 |
| `kind` | scenario |
| `scenario_id` | VP-S1 |
| `baseline_id` | VP-001-BASE-01 |
| `result` | uncertain |
| `observed_state` | unverified |
| `build_channel` | packaged-ci |
| `build_channel_digest` | 03c56b3462a6c6343d16c52c77dcc45fa5e36d59ba91f52bb9119454660fa45f |
| `exercised_artifact_digest` | 03c56b3462a6c6343d16c52c77dcc45fa5e36d59ba91f52bb9119454660fa45f |
| `variant_scan` | absent |
| `procedure_ref` | docs/evidence/VP-001/procedures/vp-s1-linux.sh |
| `blocker` | none -- the scenario ran to completion; the claim's third-party surface does not exist to exercise |
| `identity_gated` | true -- derived from `gate=av1-digest-match` and `gate=av2-variant-scan-absent`, both before the scenario session |
| `location_scheme_is_app_protocol` | true (`tauri:`) |
| `inline_script_violation` | true |
| `inline_script_ran` | false |
| `external_fetch_violation` | true |
| `external_fetch_resolved` | false (settled `rejected:TypeError`; a `connect-src` violation was recorded in the same run) |
| `framed_context_violation` | true |
| `policy_dump_captured` | true |
| `third_party_surface_exercised` | false -- no such surface exists (ADR-0004) |
| `meta_csp_present` | false -- recorded, not part of the outcome |
| `violation_count` | 3 |
| `run_date` | 2026-09-24 |
| `ci_run` | workflow tauri-build.yml, workflow_dispatch (vp001_scenario: true), run 36070751147, head d6d0813, conclusion success |
| `retention` | GitHub retains this run's `vp-001-linux-baseline` and `vp-001-transcripts` artifacts only until 2026-12-23 |
| `evidence_artifact` | a859527a03d2ab8b69a66cbda9260f742bec2dc1f10b7d4c74acff2ef34568cb -- SHA-256 of the retained redacted file `docs/evidence/VP-001/artifacts/vp-001-run-36070751147-vp-s1-transcript.txt`, copied from run 36070751147's `vp-001-transcripts` artifact entry `vp-001-vp-s1-transcript.txt` |
| `feasibility_evidence_artifact` | ffec85b83a20f971aec899996bda9b376d810cc611ef0d6d656d914dac4e77bf -- SHA-256 of the retained redacted file `docs/evidence/VP-001/artifacts/vp-001-run-36070751147-feasibility-transcript.txt`, copied from the same run's `vp-001-transcripts` artifact entry `vp-001-feasibility-transcript.txt` |
