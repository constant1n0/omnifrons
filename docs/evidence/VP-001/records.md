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

## VP-001 VP-S1 scenario -- run 36244318043 (2026-09-26)

The second execution of VP-S1, against the packaged AppImage on the same
pinned Linux baseline, with the probe run inside the renderer through
WebDriver exactly as VP-001-VP-S1-01 was. The outcome is `uncertain` again,
by construction, for the same reason: the scenario's claim covers "every
renderer surface, including a third-party app surface", no third-party app
surface exists in this renderer (ADR-0004), and `derive_csp` returns
`uncertain` whenever `third_party_surface_exercised` is false, by the
maintainer's decision. The row does not discharge RCS-001-R13 or
VP-001-R11.

What the retained transcript shows, on the surface that exists: the
document's scheme was `tauri:`, never a development URL; no CSP `<meta>`
element was present; the policy dump read back from the first violation
event carries the same thirteen directives as VP-001-VP-S1-01's policy
dump, in a different order, including the identical repeated `script-src`
source expression (`'self' 'self' '<hash>'`, the same hash both times);
three `securitypolicyviolation` events were recorded for the three
attempts, the inline script did not run, the fetch settled rejected, and
the frame was left holding `about:blank`.

New in this run: the probe now emits every captured
`securitypolicyviolation` event verbatim, one `observation=violation
index=<i> ...` line per event, in event order -- the follow-up work
VP-001-VP-S1-01's row recorded as missing. The three events are
individually distinguishable by `effectiveDirective` and `blockedURI`
alone, and each pairs with one attempt in the order the probe issues them:
index 0 `script-src-elem` / blockedURI `"inline"` (the inline-script
attempt); index 1 `connect-src` / blockedURI `"https://vp-s1.invalid/"`
(the external-fetch attempt); index 2 `frame-src` / blockedURI
`"https://vp-s1.invalid"` (the framed-context attempt). Every event's
`disposition` is `"enforce"`, and every event's `originalPolicy` is
byte-identical to the `policy_dump` value on the same transcript.
`sourceFile`, `lineNumber`, and `sample` are empty (`""`, `0`, `""`) on all
three events, so this row's attribution of an event to an attempt rests on
`effectiveDirective` and `blockedURI` alone, never on source location. The
fetch's own rejection still cannot be attributed: a `connect-src` block and
the reserved `.invalid` host never resolving both settle with the same
`TypeError`, as VP-001-VP-S1-01 flagged. What this row rests on is the
`connect-src` event at index 1, whose `blockedURI` is the fetch's own URL.

The AV1 digest gated in this run differs from VP-001-VP-S1-01's, and the
policy dump's directive order differs from that row's while carrying the
identical directive set. Every file that changed between the two rows' CI
heads is under `docs/` or `.github/workflows/` (`git diff --stat d6d0813
c5041d3`); no application source changed. This row states no cause for
either difference; both are recorded as observed, nothing more.

Both retained transcripts open with the same connection-refused preamble
VP-001-VP-S1-01's did: `Error serving connection: ... client error
(Connect)` is `tauri-driver` serving a request and failing to connect to
the upstream it proxies to (the workflow starts it with `--native-driver
/usr/bin/WebKitWebDriver`), not `tauri-driver` itself refusing. The only
requests the workflow sends before the first gate are its readiness polls
of `http://127.0.0.1:4444/status`; the preamble precedes that gate and is
not part of the scenario. VP-001-VP-S1-01's row attributes the same line to
a poll landing before `tauri-driver` listens; the line itself does not
support that.

`identity_gated` is derived the same way as VP-001-VP-S1-01: both
`gate=av1-digest-match` and `gate=av2-variant-scan-absent` appear before
`gate=session-created`. The feasibility gate (`gate=f1-session-created`)
and its own session-close appear in the VP-S1 transcript for the same
reason as before -- `vp-s1-linux.sh` runs `vp-s6-linux.sh
--mode=feasibility-check` itself as a subprocess -- and the separately
retained feasibility transcript is the workflow's own, earlier feasibility
step, a distinct session gated against the same digest.

`observed_state: unverified` names the same `CspObservedState::Unverified`
branch VP-001-VP-S1-01 landed on: nothing was demonstrated either way
about the claim as written; the surface that exists showed no bypass.

Each `evidence_artifact` digest below is the SHA-256 of the retained,
redacted file at the path it names -- the file in this repository, not the
CI artifact entry it was copied from, whose runner paths were rewritten to
`<extract-dir>` before retention.

| field | value |
| --- | --- |
| `record_id` | VP-001-VP-S1-02 |
| `kind` | scenario |
| `scenario_id` | VP-S1 |
| `baseline_id` | VP-001-BASE-01 |
| `result` | uncertain |
| `observed_state` | unverified |
| `build_channel` | packaged-ci |
| `build_channel_digest` | 437204786229aa0a35b69a56ecca4d71d02820a5fe92867aeaa579cb28141501 |
| `exercised_artifact_digest` | 437204786229aa0a35b69a56ecca4d71d02820a5fe92867aeaa579cb28141501 |
| `variant_scan` | absent |
| `procedure_ref` | docs/evidence/VP-001/procedures/vp-s1-linux.sh |
| `blocker` | none -- the scenario ran to completion; the claim's third-party surface does not exist to exercise |
| `identity_gated` | true -- derived from `gate=av1-digest-match` and `gate=av2-variant-scan-absent`, both before the scenario session |
| `location_scheme_is_app_protocol` | true (`tauri:`) |
| `inline_script_violation` | true |
| `inline_script_ran` | false |
| `external_fetch_violation` | true |
| `external_fetch_resolved` | false (settled `rejected:TypeError`; a `connect-src` violation event was captured at index 1) |
| `framed_context_violation` | true |
| `policy_dump_captured` | true |
| `third_party_surface_exercised` | false -- no such surface exists (ADR-0004) |
| `meta_csp_present` | false -- recorded, not part of the outcome |
| `violation_count` | 3 |
| `run_date` | 2026-09-26 |
| `ci_run` | workflow tauri-build.yml, workflow_dispatch (vp001_scenario: true), run 36244318043, head c5041d3, conclusion success |
| `retention` | GitHub retains this run's `vp-001-linux-baseline` and `vp-001-transcripts` artifacts only until 2026-12-25 |
| `evidence_artifact` | 6b113eeae5c95c6c9ffc366246c179d8bd3c893c0ab77ba75d63330c16760402 -- SHA-256 of the retained redacted file `docs/evidence/VP-001/artifacts/vp-001-run-36244318043-vp-s1-transcript.txt`, copied from run 36244318043's `vp-001-transcripts` artifact entry `vp-001-vp-s1-transcript.txt` |
| `feasibility_evidence_artifact` | 445267473c2a7d6851b0fd119a7e2a431f5c5d72d9b993fe24093f2adc27d81f -- SHA-256 of the retained redacted file `docs/evidence/VP-001/artifacts/vp-001-run-36244318043-feasibility-transcript.txt`, copied from the same run's `vp-001-transcripts` artifact entry `vp-001-feasibility-transcript.txt` |

## VP-001 VP-S1 correction -- VP-001-VP-S1-01 (run 36070751147)

Corrects VP-001-VP-S1-01, filed from run 36070751147 (2026-09-24), in two
statements about the connection-refused line that opens both of that row's
retained transcripts. No new run was made. Everything else in
VP-001-VP-S1-01 stands as filed, including its `result`, its
`observed_state`, its digests and its retained artifacts. This row repeats
none of those digests or artifacts: they belong to that row, and the
store's V5 check refuses the same `evidence_artifact` twice for one
scenario and baseline.

The cause. VP-001-VP-S1-01 attributes the line to the workflow's first
readiness poll landing before `tauri-driver` listens. The line itself,
`Error serving connection: hyper::Error(User(Service), client error
(Connect))` with a `Connection refused` cause, is `tauri-driver` serving a
request and failing to connect to the upstream it proxies to (the workflow
at that row's CI head starts it with `--native-driver
/usr/bin/WebKitWebDriver`); it does not show `tauri-driver` refusing. The
only requests the workflow sends before the first gate are its readiness
polls of `http://127.0.0.1:4444/status`, so the line is still not part of
the scenario; only its stated mechanism was wrong. VP-001-VP-S1-02 already
records this reading.

The scope. VP-001-VP-S1-01 says the same line opens every retained VP-S6
transcript. It opens two of the three: those of runs 35252892166 and
35467644986. Run 35450847942's retained VP-S6 transcript does not contain
it; its first line is `gate=av1-digest-match`.

| field | value |
| --- | --- |
| `record_id` | VP-001-VP-S1-03 |
| `kind` | scenario |
| `scenario_id` | VP-S1 |
| `baseline_id` | VP-001-BASE-01 |
| `result` | uncertain |
| `observed_state` | unverified |
| `build_channel` | packaged-ci |
| `variant_scan` | absent |
| `procedure_ref` | docs/evidence/VP-001/procedures/vp-s1-linux.sh |
| `corrects` | VP-001-VP-S1-01 |
| `blocker` | none -- a correction of VP-001-VP-S1-01's prose; its outcome and fields stand as filed |
| `run_date` | 2026-09-24 -- the date of the corrected run; no new run was made |
| `ci_run` | workflow tauri-build.yml, run 36070751147, the run VP-001-VP-S1-01 was filed from |

## VP-001 VP-S3 scenario -- run 36352840629 (2026-09-27)

The first execution of VP-S3 (typed-IPC bridge confinement), against the
packaged AppImage on the pinned Linux baseline, run through
`vp-s3-linux.sh`, which reuses `vp-s6-linux.sh --mode=feasibility-check`
for AV1/AV2 exactly as `vp-s1-linux.sh` does. Unlike either VP-S1 row, the
scenario's own `WebDriver` session was never created: session creation
exceeded the scenario's 30-second bound (`vp-s3-scenario.mjs`), so the
probe never armed and no VP-S3-specific fact was ever
observed. The outcome is `uncertain` by `derive_ipc`'s own construction on
an all-absent observation set: none of its four fixed-order failure checks
can fire with no positive observation to fire on, and `Pass` requires every
positive proof, so the row falls to `(Uncertain, Unverified)`.

What the retained VP-S3 transcript shows. It opens with the same
connection-refused preamble VP-001-VP-S1-01's and -02's transcripts do;
VP-001-VP-S1-02/-03 already established the reading used here --
`tauri-driver` serving a request and failing to reach the upstream it
proxies to, not `tauri-driver` itself refusing -- and this row does not
re-derive it. Then
`gate=av1-digest-match digest=027628e46beb188d30fd36dab0ad02a35d3928b39e0764b66e866d726f737bde`
and
`gate=av2-variant-scan-absent binary=<extract-dir>/squashfs-root/usr/bin/omnifrons-shell`,
both before any session is attempted. `vp-s3-linux.sh`'s own reused
`vp-s6-linux.sh --mode=feasibility-check` subprocess then opens and closes
one feasibility session of its own, `gate=f1-session-created` /
`gate=session-closed`, at a session id distinct from the one in the
separately retained feasibility transcript -- the workflow's own, earlier
feasibility step -- exactly the two-sessions-per-run structure the VP-S1
rows describe. After that feasibility session closed, a fresh
`omnifrons-shell` process started for the scenario's own session attempt,
and the transcript records only `blocker=session-create-failed:
session-create exceeded 30000 ms`: no `gate=session-created` line ever
appears, so the scenario session itself was never created.

`identity_gated` is derived the same way the VP-S1 rows derive it: both
`gate=av1-digest-match` and `gate=av2-variant-scan-absent` appear before
the (attempted) scenario session.

What this run cannot show: every other `IpcObservations` field the probe
itself would populate is false because none of it was ever observed, not
because it was observed absent --
`location_scheme_is_app_protocol`, `connect_src_policy_captured`,
`connect_src_is_documented_source_for_os`, `registered_call_completed`,
`registered_call_used_custom_protocol`, `postmessage_fallback_observed`,
`bridge_connect_src_violation`, `unregistered_call_rejected`,
`unregistered_call_resolved`, `artifact_load_attempted`, and
`artifact_load_succeeded`. No connect-src policy dump was captured, no
registered call transport was recorded, no `postMessage` fallback or
bridge `connect-src` violation was recorded, no unregistered-command
handling was recorded, and no `artifact:` load was attempted or observed
-- the scenario's claim about transport, policy, fallback, `artifact:`
handling, and unregistered-command rejection is not addressed by this run
in either direction.

`static_inventory_pinned` is the one `IpcObservations` field this row can
still state, since it is not the probe's own observation:
`src-tauri/tests/protocol_inventory.rs` is exercised by the `build-test
(ubuntu-latest)` CI check, confirmed (read-only, via the GitHub API's
check-runs endpoint for this run's head commit) to have succeeded on
64649de. True.

Timing context, measured from the job log of run 36352840629, not from a
retained artifact, each interval running from the app process's first
output line to `gate=session-created`: in this run every F1 session took
30.3-30.4 s and the VP-S1 scenario session 12.2 s. The VP-S3 scenario
session never reached `gate=session-created`; its `blocker=` line, emitted
when the scenario's own 30 s bound (started at its session request, before
the app printed anything) ran out, came 28.8 s after that app process's
first output line. Earlier runs 36070751147 and 36244318043 show F1
30.2-32.6 s and VP-S1 scenario sessions of 13.6 s and 11.4 s respectively. This row states no cause for the roughly 30-second
session-create duration; nothing observed here demonstrates one.

Per the evidence-store spec's Blocked or Non-Reproducible Verification
Attempt requirement, this row is not retried, renamed, or replaced. A
separate, follow-up change is expected to raise the session-create
deadline; a later run made against that longer deadline is to be filed as
its own row, never as a replacement for this one.

This run's CI job conclusion is `failure`, unlike either VP-S1 row's
`success`: `vp-s3-linux.sh` passes through `vp-s3-scenario.mjs`'s own exit
status for the scenario leg unchanged, and this row's blocker is not one
of the two statuses (124, 137) the shell renames to its own `gate=`
lines -- so the scenario script's own nonzero exit on `session-create-failed`
propagates as the job's own conclusion, independent of the `uncertain`
result the evidence-store spec requires this row to record.

The row does not discharge RCS-001-R14 or VP-001-R12: no bridge call,
`connect-src` capture, or handler-inventory fact was demonstrated on this
run, in either direction.

Each `evidence_artifact` digest below is the SHA-256 of the retained,
redacted file at the path it names -- the file in this repository, not the
CI artifact entry it was copied from, whose runner paths were rewritten to
`<extract-dir>` before retention.

| field | value |
| --- | --- |
| `record_id` | VP-001-VP-S3-01 |
| `kind` | scenario |
| `scenario_id` | VP-S3 |
| `baseline_id` | VP-001-BASE-01 |
| `result` | uncertain |
| `observed_state` | unverified |
| `build_channel` | packaged-ci |
| `build_channel_digest` | 027628e46beb188d30fd36dab0ad02a35d3928b39e0764b66e866d726f737bde |
| `exercised_artifact_digest` | 027628e46beb188d30fd36dab0ad02a35d3928b39e0764b66e866d726f737bde |
| `variant_scan` | absent |
| `procedure_ref` | docs/evidence/VP-001/procedures/vp-s3-linux.sh |
| `blocker` | session-create-failed -- the scenario's `WebDriver` session was never created: session-create exceeded 30000 ms |
| `identity_gated` | true -- derived from `gate=av1-digest-match` and `gate=av2-variant-scan-absent`, both before the scenario session was attempted |
| `location_scheme_is_app_protocol` | false -- never observed; the scenario session was never created |
| `connect_src_policy_captured` | false -- never observed, same reason |
| `connect_src_is_documented_source_for_os` | false -- never observed, same reason |
| `registered_call_completed` | false -- never observed, same reason |
| `registered_call_used_custom_protocol` | false -- never observed, same reason |
| `postmessage_fallback_observed` | false -- never observed, same reason |
| `bridge_connect_src_violation` | false -- never observed, same reason |
| `unregistered_call_rejected` | false -- never observed, same reason |
| `unregistered_call_resolved` | false -- never observed, same reason |
| `artifact_load_attempted` | false -- never observed, same reason |
| `artifact_load_succeeded` | false -- never observed, same reason |
| `static_inventory_pinned` | true -- `build-test (ubuntu-latest)` succeeded on this run's head commit 64649de, confirmed read-only via the GitHub API check-runs endpoint |
| `run_date` | 2026-09-27 |
| `ci_run` | workflow tauri-build.yml, workflow_dispatch (vp001_scenario: true), run 36352840629, head 64649de, conclusion failure |
| `retention` | GitHub retains this run's `vp-001-linux-baseline` and `vp-001-transcripts` artifacts only until 2026-12-26 |
| `evidence_artifact` | e70aa97508d1b5070da7b17eaab243ac9fbb70943fe194be18ca54068b35f5e6 -- SHA-256 of the retained redacted file `docs/evidence/VP-001/artifacts/vp-001-run-36352840629-vp-s3-transcript.txt`, copied from run 36352840629's `vp-001-transcripts` artifact entry `vp-001-vp-s3-transcript.txt` |
| `feasibility_evidence_artifact` | be58a3c7cec62a9f51854dac7174f88dec10ab6f23ed3a2ef3f2fa2fc80cc1c2 -- SHA-256 of the retained redacted file `docs/evidence/VP-001/artifacts/vp-001-run-36352840629-feasibility-transcript.txt`, copied from the same run's `vp-001-transcripts` artifact entry `vp-001-feasibility-transcript.txt` |

## VP-001 VP-S3 scenario -- run 36454804051 (2026-09-28)

The second execution of VP-S3 (typed-IPC bridge confinement), against the
packaged AppImage on the pinned Linux baseline, run through
`vp-s3-linux.sh`, exactly as VP-001-VP-S3-01. The harness changed since
that row: commit 1c51343 (#98) gave session creation its own 90-second
deadline (`SESSION_CREATE_DEADLINE_MS` in
`docs/evidence/VP-001/procedures/vp-s1-scenario.mjs`, used by
`vp-s3-scenario.mjs`), in place of the shared 30-second `CALL_DEADLINE_MS`
whose bound VP-001-VP-S3-01's session-create exceeded. Under the longer
bound, this run's scenario `WebDriver` session was created, the probe
armed, and the probe emitted every observation it records.

This row is filed as its own new record, never as a retry, rename, or
replacement of VP-001-VP-S3-01. Per the evidence-store spec's Blocked or
Non-Reproducible Verification Attempt requirement, that row's `uncertain`
result stands exactly as filed, unretried and unreplaced; VP-001-VP-S3-01
itself anticipated this run: "A separate, follow-up change is expected to
raise the session-create deadline; a later run made against that longer
deadline is to be filed as its own row, never as a replacement for this
one." This row carries no `corrects` field, for the same reason -- it does
not correct VP-001-VP-S3-01, it is the anticipated follow-up.

Both retained transcripts open with the same connection-refused preamble
VP-001-VP-S1-01's, -02's, and VP-001-VP-S3-01's transcripts do;
VP-001-VP-S1-02/-03 already established the reading used here, and this
row does not re-derive it.

In the VP-S3 transcript, the preamble is followed by `gate=av1-digest-match
digest=fd783e18ded3a1d6f6b3ccc6e60f13f96039cf55e01fcde9fed53c65b12f13b3`
and `gate=av2-variant-scan-absent
binary=<extract-dir>/squashfs-root/usr/bin/omnifrons-shell`, both before
any session is attempted. `vp-s3-linux.sh`'s own reused
`vp-s6-linux.sh --mode=feasibility-check` subprocess then opens and closes
one feasibility session of its own, `gate=f1-session-created` /
`gate=session-closed` (session `a72d1c8c-99c7-4ade-972b-3900b1759fa3`), at
a session id distinct from the one in the separately retained feasibility
transcript (`1a0c69e3-9c2e-4b98-9edb-0aee93fed03d`) -- the workflow's own,
earlier feasibility step -- exactly the two-sessions-per-run structure the
VP-S1 rows and VP-001-VP-S3-01 describe. After that feasibility session
closed, a fresh `omnifrons-shell` process started for the scenario's own
session attempt, and this time it succeeded:
`gate=session-created session_id=0815193d-92ae-4822-a64f-c39ff24dadec`,
then `gate=armed`, then `gate=location-scheme value=tauri:`.

`identity_gated` is derived the same way the VP-S1 rows and
VP-001-VP-S3-01 derive it: both `gate=av1-digest-match` and
`gate=av2-variant-scan-absent` appear before the scenario session was
created.

The AV1 digest gated in this run
(`fd783e18ded3a1d6f6b3ccc6e60f13f96039cf55e01fcde9fed53c65b12f13b3`, equal
to both `build_channel_digest` and `exercised_artifact_digest`; AV2's
`variant_scan` is `absent`) differs from VP-001-VP-S3-01's
(`027628e46beb188d30fd36dab0ad02a35d3928b39e0764b66e866d726f737bde`).
Every file that changed between the two rows' CI heads (`git diff --stat
64649de e06a4a5`) is either a retained transcript artifact from
VP-001-VP-S3-01, one of the harness scripts implementing the 90-second
session-create deadline (`vp-s1-scenario.mjs`, `vp-s1-scenario.test.mjs`,
`vp-s3-scenario.mjs`), or `records.md` itself -- no file under
`src-tauri/` or the renderer frontend changed. This row states no cause
for the digest difference; nothing observed here demonstrates one.

Exactly one `securitypolicyviolation` was recorded, at index 0:
`effectiveDirective`/`violatedDirective` `connect-src`, `blockedURI`
`"https://vp-s3.invalid/"` -- the probe's own deliberate policy-probe
fetch, not a bridge call -- `disposition` `"enforce"`, and `originalPolicy`
whose `connect-src` directive is exactly `ipc:`, the Linux/macOS bridge
source RCS-001 documents (renderer-content-security.md § CSP baseline).
`sourceFile`, `lineNumber`, and `sample` are empty, as on all three of
VP-001-VP-S1-02's violation events.

Sixteen `fetch` lines were recorded. Index 0,
`ipc://localhost/shell_health`, `resolved:200`, is the probe's registered
call; index 1, `ipc://localhost/vp_s3_unregistered_command`,
`resolved:200`, is the probe's unregistered call; index 2,
`https://vp-s3.invalid/`, `rejected:TypeError`, is the policy probe.
Indexes 3-15 are thirteen calls the app's own renderer made during the
probe's observation window -- `approvals_list`, `workspace_current`,
`outbox_status`, `publications_list`, `guidance_status` (twice),
`guidance_snapshots` (twice), `wrongroot_status`, `misplaced_list`,
`recovery_list`, `adapters_list`, `approvals_list` again -- every one to
`ipc://localhost`, every one `resolved:200`. `vp-s3-probe.mjs`'s own
`buildArmScript` issues exactly four attempts (registered-call,
unregistered-call, an `artifact:` `Image` load that never goes through the
wrapped `fetch`, and the policy-probe); it never issues the thirteen calls
at indexes 3-15. The `fetch` wrapper installed at arm time captures every
call in the window regardless of origin, so these are the app's own
routine renderer activity, incidentally observed, not the probe's own
traffic.

The summary observations were emitted exactly as follows:
`location_scheme_is_app_protocol=true`,
`connect_src_policy_captured=true`,
`connect_src_is_documented_source_for_os=true`,
`registered_call_completed=true`,
`registered_call_used_custom_protocol=true`,
`postmessage_fallback_observed=false`,
`bridge_connect_src_violation=false`,
`unregistered_call_rejected=true`, `unregistered_call_resolved=false`,
`artifact_load_attempted=true`, `artifact_load_succeeded=false`.

`static_inventory_pinned` is not a probe observation:
`src-tauri/tests/protocol_inventory.rs`
is exercised by the `build-test (ubuntu-latest)` CI check, confirmed
(read-only, via the GitHub API's check-runs endpoint for this run's head
commit) to have succeeded on e06a4a5. True.

`derive_ipc` (`tools/evidence-validator/src/derive.rs`) checks four
demonstrated-failure conditions in fixed order before it admits `Pass`.
None fires here: `connect_src_policy_captured && !connect_src_is_documented_source_for_os`
is false, since the captured source is exactly the documented one;
`postmessage_fallback_observed || bridge_connect_src_violation` is false;
`artifact_load_succeeded` is false; `unregistered_call_resolved` is false.
Every fact `Pass` requires is true -- `identity_gated`,
`location_scheme_is_app_protocol`, `connect_src_policy_captured`,
`connect_src_is_documented_source_for_os`, `registered_call_completed`,
`registered_call_used_custom_protocol`, `unregistered_call_rejected`,
`artifact_load_attempted`, and `static_inventory_pinned` -- so `derive_ipc`
yields `(Pass, Confined)`.

`unregistered_call_rejected` is `true` while fetch index 1 (the same
call's own transport) shows `resolved:200`. Tauri 2.11.5's
`scripts/ipc-protocol.js:42-44` picks the invoke's success or error
callback from the response's `Tauri-Response` header, never from the HTTP
status, so a `resolved:200` `fetch` alongside a rejected `invoke()`
promise is consistent with that client. The transcript records neither the
header nor the body. The rejection recorded here rests entirely on
`unregisteredAttempt?.outcome` in `vp-s3-probe.mjs`'s `summarize` --
the `invoke('vp_s3_unregistered_command')` promise's own settled outcome,
distinct from the `fetch` line above it -- never on the fetch's status.

`postmessage_fallback_observed` is `false`. Per `ipc-protocol.js:37-69`,
the `postMessage` fallback runs only after a custom-protocol `fetch`
rejects, which sets `customProtocolIpcFailed` and re-enters
`sendIpcMessage`. Once that flag is set, later invokes go through
`postMessage` without any `fetch`; the registered call was recorded as an
`ipc://localhost` `fetch` (index 0), so the flag was not set when it ran,
and no bridge `fetch` recorded after it rejected, so it was not set at any
point in the probe's observation window. `vp-s3-probe.mjs`'s `summarize`
computes `postmessage_fallback_observed` from `consoleWarnFallbackSeen`
(the `console.warn` text Tauri emits on that same fallback path) or
`postMessageCallCount >= 1`; the probe emits no observation of whether its
`window.ipc.postMessage` wrapper was installed. This row's `false`
therefore rests on the resolved bridge `fetch` lines above and the absence
of the fallback's console warning, not on the `postMessage` wrapper.

The `fetch` wrapper is installed only at arm time, so a call the app made
before arm is not observed; "every call" above means every call recorded
in the probe's own observation window, not literally every call the app
ever made this session.

Timing context, measured from the job log of run 36454804051, not from a
retained artifact, each interval running from the app process's first
output line to its session's `gate=session-created` (or
`gate=f1-session-created`): every F1 session in this run took 30.3-31.3 s,
the VP-S1 scenario session (also in this job) took 12.0 s, and the VP-S3
scenario session took 30.3 s. The 30-second bound VP-001-VP-S3-01 hit
started at the session request, before the app printed anything, so it
would have run out before this session was created; the 90-second bound
admitted it. This row states no cause for the
session-create duration; nothing observed here demonstrates one.

This is the Linux baseline VP-001-BASE-01 only. Per VP-001-R15, a passing
result does not transfer between baselines: Windows' documented
`connect-src` source (`http://ipc.localhost`) and macOS were not
exercised, and neither is covered by this row.

VP-001-R12 requires every bridge protocol reaching `connect-src` to be
enumerated per OS. This row enumerates the Linux bridge protocol only, so
it does not by itself discharge VP-001-R12, which is satisfied only once
every supported OS carries its own row.

RCS-001-R14 reads: "Every renderer→core request MUST be a typed IPC
message; a payload MUST carry a logical reference and MUST NOT carry a raw
filesystem path." This row's registered call (`invoke('shell_health')`,
with no payload argument) went over the typed `invoke` mechanism the first
clause names, but the probe never sent or inspected a payload, so nothing
here speaks to the second clause -- whether a payload carries a logical
reference rather than a raw filesystem path. A Linux-only pass on
transport confinement therefore does not discharge RCS-001-R14 as a whole,
and even the transport half is Linux-only under VP-001-R15.

Each `evidence_artifact` digest below is the SHA-256 of the retained,
redacted file at the path it names -- the file in this repository, not the
CI artifact entry it was copied from, whose runner paths were rewritten to
`<extract-dir>` before retention.

| field | value |
| --- | --- |
| `record_id` | VP-001-VP-S3-02 |
| `kind` | scenario |
| `scenario_id` | VP-S3 |
| `baseline_id` | VP-001-BASE-01 |
| `result` | pass |
| `observed_state` | confined |
| `build_channel` | packaged-ci |
| `build_channel_digest` | fd783e18ded3a1d6f6b3ccc6e60f13f96039cf55e01fcde9fed53c65b12f13b3 |
| `exercised_artifact_digest` | fd783e18ded3a1d6f6b3ccc6e60f13f96039cf55e01fcde9fed53c65b12f13b3 |
| `variant_scan` | absent |
| `procedure_ref` | docs/evidence/VP-001/procedures/vp-s3-linux.sh |
| `blocker` | none -- the scenario ran to completion and every `derive_ipc` `Pass` proof was observed |
| `identity_gated` | true -- derived from `gate=av1-digest-match` and `gate=av2-variant-scan-absent`, both before the scenario session was created |
| `location_scheme_is_app_protocol` | true (`tauri:`) |
| `connect_src_policy_captured` | true |
| `connect_src_is_documented_source_for_os` | true -- captured `connect-src ipc:`, exactly the Linux source RCS-001 documents |
| `registered_call_completed` | true |
| `registered_call_used_custom_protocol` | true -- the registered call's fetch went to `ipc://localhost/shell_health`, resolved:200 |
| `postmessage_fallback_observed` | false |
| `bridge_connect_src_violation` | false |
| `unregistered_call_rejected` | true -- the invoke's own promise rejected |
| `unregistered_call_resolved` | false -- the underlying fetch (index 1) nonetheless resolved:200; see prose |
| `artifact_load_attempted` | true |
| `artifact_load_succeeded` | false |
| `static_inventory_pinned` | true -- `build-test (ubuntu-latest)` succeeded on this run's head commit e06a4a5, confirmed read-only via the GitHub API check-runs endpoint |
| `run_date` | 2026-09-28 |
| `ci_run` | workflow tauri-build.yml, workflow_dispatch (vp001_scenario: true), run 36454804051, head e06a4a5, conclusion success |
| `retention` | GitHub retains this run's `vp-001-linux-baseline` and `vp-001-transcripts` artifacts only until 2026-12-27 |
| `evidence_artifact` | b2c20e7df17cd1b989dd8261766d514800b318fa0a0c4a91d22c0ff16ac1d1f5 -- SHA-256 of the retained redacted file `docs/evidence/VP-001/artifacts/vp-001-run-36454804051-vp-s3-transcript.txt`, copied from run 36454804051's `vp-001-transcripts` artifact entry `vp-001-vp-s3-transcript.txt` |
| `feasibility_evidence_artifact` | 11972446d0497fcaa64be620cc584649cde88457521ae91a49491ee35bca01d6 -- SHA-256 of the retained redacted file `docs/evidence/VP-001/artifacts/vp-001-run-36454804051-feasibility-transcript.txt`, copied from the same run's `vp-001-transcripts` artifact entry `vp-001-feasibility-transcript.txt` |

## VP-001 VP-S3 scenario -- run 36990386851 (2026-10-02)

The third execution of VP-S3 (typed-IPC bridge confinement), against the
packaged AppImage on the pinned Linux baseline, run through
`vp-s3-linux.sh`, exactly as VP-001-VP-S3-01 and VP-001-VP-S3-02. Between
VP-001-VP-S3-02's head (e06a4a5) and this row's (49f3304), `git log
--oneline e06a4a5..49f3304` shows the scenario session lifecycle hardened against SIGTERM and a
late-created session, a fake-WebDriver end-to-end test suite added for the
VP-S1/VP-S3 drivers, those drivers tightened to a 0/1/2 exit-status
contract, one liveness-checked `with-webdriver.sh` bootstrap shared across
the VP-001 steps, and -- on 49f3304, the commit this row's run was
dispatched against -- the `observation=attempt` transcript lines and the
static protocol-inventory CI step this row now cites. `git diff --stat
e06a4a5 49f3304` touches only `.github/workflows/tauri-build.yml`, files
under `docs/evidence/VP-001/procedures/`, and `records.md` itself; no file
under `src-tauri/` or the renderer frontend changed. This row states no
cause for anything in that history; it only reports what changed.

The retained VP-S3 transcript opens with the same connection-refused
preamble VP-001-VP-S3-02's does; VP-001-VP-S1-02/-03 already established
the reading used here, and this row does not re-derive it. The retained
feasibility transcript does not open with it: its first line is
`gate=av1-digest-match`, as VP-001-VP-S1-03 recorded for run 35450847942's
VP-S6 transcript. In the VP-S3 transcript, the preamble is followed by `gate=av1-digest-match
digest=e06ed9f0b6897a8f152fd655d64bb1df2031a8c0c7cb0561b090049e5db6eada`
and `gate=av2-variant-scan-absent
binary=<extract-dir>/squashfs-root/usr/bin/omnifrons-shell`, both before
any session is attempted. `vp-s3-linux.sh`'s own reused
`vp-s6-linux.sh --mode=feasibility-check` subprocess then opens and closes
one feasibility session of its own, `gate=f1-session-created` /
`gate=session-closed` (session `ddb6112a-8358-4ac7-9172-894670ca7154`), at
a session id distinct from the one in the separately retained feasibility
transcript (`c167f55d-4916-4ce2-8c83-7759f21dc185`) -- the workflow's own,
earlier feasibility step -- exactly the two-sessions-per-run structure the
VP-S1 rows, VP-001-VP-S3-01, and VP-001-VP-S3-02 describe. After that
feasibility session closed, a fresh `omnifrons-shell` process started for
the scenario's own session attempt, and it succeeded:
`gate=session-created session_id=e4b72fb2-2a3b-4234-a8a6-e4fc20e87958`,
then `gate=armed`, then `gate=location-scheme value=tauri:`.

`identity_gated` is derived the same way the VP-S1 rows, VP-001-VP-S3-01,
and VP-001-VP-S3-02 derive it: both `gate=av1-digest-match` and
`gate=av2-variant-scan-absent` appear before the scenario session was
created.

The AV1 digest gated in this run
(`e06ed9f0b6897a8f152fd655d64bb1df2031a8c0c7cb0561b090049e5db6eada`, equal
to both `build_channel_digest` and `exercised_artifact_digest`; AV2's
`variant_scan` is `absent`) is new relative to both prior VP-S3 rows. A
separate validation dispatch against the same commit 49f3304 (run
36787727082, branch `feat/vp-s3-probe-evidence`, not filed as a row and
its artifacts not retained in this repository) gated a different AV1
digest for a packaged build of that same commit,
`875b4249d9255322a4558f6e8d4ef9e473f65a2dc00d8f10a22982cce35cf708`. Two
packaged builds of the same commit therefore produced two different
AppImage digests. This row states no cause for the difference; nothing
observed here demonstrates one.

Exactly one `securitypolicyviolation` was recorded, at index 0:
`effectiveDirective`/`violatedDirective` `connect-src`, `blockedURI`
`"https://vp-s3.invalid/"` -- the probe's own deliberate policy-probe
fetch, not a bridge call -- `disposition` `"enforce"`, and `originalPolicy`
whose `connect-src` directive is exactly `ipc:`, the Linux/macOS bridge
source RCS-001 documents (renderer-content-security.md § CSP baseline).
The policy dump's directive order differs from VP-001-VP-S3-02's while
carrying the identical thirteen-directive set, the same kind of difference
VP-001-VP-S1-02 recorded for its own policy dump. `sourceFile`,
`lineNumber`, and `sample` are empty, as on all three of VP-001-VP-S1-02's
violation events and VP-001-VP-S3-02's one event.

Sixteen `fetch` lines were recorded, in the same shape VP-001-VP-S3-02's
were: index 0, `ipc://localhost/shell_health`, `resolved:200`, the probe's
registered call; index 1, `ipc://localhost/vp_s3_unregistered_command`,
`resolved:200`, the probe's unregistered call; index 2,
`https://vp-s3.invalid/`, `rejected:TypeError`, the policy probe. Indexes
3-15 are the same thirteen calls the app's own renderer made during the
probe's observation window that VP-001-VP-S3-02 recorded --
`approvals_list`, `workspace_current`, `outbox_status`,
`publications_list`, `guidance_status` (twice), `guidance_snapshots`
(twice), `wrongroot_status`, `misplaced_list`, `recovery_list`,
`adapters_list`, `approvals_list` again -- every one to `ipc://localhost`,
every one `resolved:200`.

New in this run: four `observation=attempt index=<i> name=<json>
outcome=<json>` lines, one per attempt `vp-s3-probe.mjs`'s `buildArmScript`
makes, in the format `formatAttemptLines` adds (commit 49f3304): index 0
`registered-call` `resolved`; index 1 `unregistered-call`
`rejected:Command vp_s3_unregistered_command not found`; index 2
`artifact-load` `error`; index 3 `policy-probe` `rejected:TypeError`. The
unregistered invoke's own rejection, carrying Tauri's own error message,
is now in the retained transcript itself, not only in the summary boolean.
VP-001-VP-S3-02 could rest `unregistered_call_rejected` only on
`unregisteredAttempt?.outcome` inside `vp-s3-probe.mjs`'s `summarize`,
stating: "The rejection recorded here rests entirely on
`unregisteredAttempt?.outcome` in `vp-s3-probe.mjs`'s `summarize` -- the
`invoke('vp_s3_unregistered_command')` promise's own settled outcome,
distinct from the `fetch` line above it -- never on the fetch's status."
That sentence still describes how the fact is derived, but the attempt's
own outcome text is now part of the retained evidence this row cites,
closing the gap that sentence described.

The summary observations were emitted exactly as VP-001-VP-S3-02's were:
`location_scheme_is_app_protocol=true`,
`connect_src_policy_captured=true`,
`connect_src_is_documented_source_for_os=true`,
`registered_call_completed=true`,
`registered_call_used_custom_protocol=true`,
`postmessage_fallback_observed=false`,
`bridge_connect_src_violation=false`,
`unregistered_call_rejected=true`, `unregistered_call_resolved=false`,
`artifact_load_attempted=true`, `artifact_load_succeeded=false`.

`static_inventory_pinned` no longer rests on a GitHub API check-run
lookup, unlike VP-001-VP-S3-01's and VP-001-VP-S3-02's. The retained file
`docs/evidence/VP-001/artifacts/vp-001-run-36990386851-vp-s3-static-inventory.txt`
(CI step "VP-001 VP-S3 static protocol inventory (evidence run)", added in
commit 49f3304) opens `gate=static-inventory
head=49f330402c61e6c30a8e74efbe645e2fde880cf8`, matching this run's own
head commit, and shows `src-tauri/tests/protocol_inventory.rs`'s own
output: "test result: ok. 6 passed; 0 failed". No GitHub API lookup is
needed for this row; `static_inventory_evidence_artifact` below
cites the retained file directly.

`derive_ipc` (`tools/evidence-validator/src/derive.rs`) checks its four
demonstrated-failure conditions in fixed order before admitting `Pass`, as
in VP-001-VP-S3-02: `connect_src_policy_captured &&
!connect_src_is_documented_source_for_os` is false, since the captured
source is exactly the documented one; `postmessage_fallback_observed ||
bridge_connect_src_violation` is false; `artifact_load_succeeded` is
false; `unregistered_call_resolved` is false. Every fact `Pass` requires is
true -- `identity_gated`, `location_scheme_is_app_protocol`,
`connect_src_policy_captured`, `connect_src_is_documented_source_for_os`,
`registered_call_completed`, `registered_call_used_custom_protocol`,
`unregistered_call_rejected`, `artifact_load_attempted`, and
`static_inventory_pinned` -- so `derive_ipc` yields `(Pass, Confined)`.

`postmessage_fallback_observed` is `false` for the same reason
VP-001-VP-S3-02 gives: the registered call's own fetch (index 0) resolved
before any rejection could set Tauri's `customProtocolIpcFailed` flag, and
this run's transcript records no bridge fetch rejecting and no fallback
console warning either. This row does not re-derive that reading at
length.

Timing context, measured from the job log of run 36990386851, not from a
retained artifact, each interval running from the app process's first
output line to its session's `gate=session-created` (or
`gate=f1-session-created`): every one of the four F1 sessions in this run's
job log took 30.2-31.4 s, the VP-S1 scenario
session took 11.8 s, and the VP-S3 scenario session took 30.2 s. This row
states no cause for the session-create durations; nothing observed here
demonstrates one.

This is the Linux baseline VP-001-BASE-01 only. Per VP-001-R15, a passing
result does not transfer between baselines, for the reasons
VP-001-VP-S3-02 gives; this row does not repeat them. It does not by
itself discharge VP-001-R12, which requires every bridge protocol
enumerated per OS, or RCS-001-R14, whose payload-shape clause this probe's
registered call (`invoke('shell_health')`, no payload argument) does not
address -- the same reasoning VP-001-VP-S3-02 gives for its own Linux-only
pass.

Each `evidence_artifact` digest below is the SHA-256 of the retained,
redacted file at the path it names -- the file in this repository, not the
CI artifact entry it was copied from, whose runner paths were rewritten to
`<extract-dir>` before retention.

| field | value |
| --- | --- |
| `record_id` | VP-001-VP-S3-03 |
| `kind` | scenario |
| `scenario_id` | VP-S3 |
| `baseline_id` | VP-001-BASE-01 |
| `result` | pass |
| `observed_state` | confined |
| `build_channel` | packaged-ci |
| `build_channel_digest` | e06ed9f0b6897a8f152fd655d64bb1df2031a8c0c7cb0561b090049e5db6eada |
| `exercised_artifact_digest` | e06ed9f0b6897a8f152fd655d64bb1df2031a8c0c7cb0561b090049e5db6eada |
| `variant_scan` | absent |
| `procedure_ref` | docs/evidence/VP-001/procedures/vp-s3-linux.sh |
| `blocker` | none -- the scenario ran to completion and every `derive_ipc` `Pass` proof was observed |
| `identity_gated` | true -- derived from `gate=av1-digest-match` and `gate=av2-variant-scan-absent`, both before the scenario session was created |
| `location_scheme_is_app_protocol` | true (`tauri:`) |
| `connect_src_policy_captured` | true |
| `connect_src_is_documented_source_for_os` | true -- captured `connect-src ipc:`, exactly the Linux source RCS-001 documents |
| `registered_call_completed` | true |
| `registered_call_used_custom_protocol` | true -- the registered call's fetch went to `ipc://localhost/shell_health`, resolved:200 |
| `postmessage_fallback_observed` | false |
| `bridge_connect_src_violation` | false |
| `unregistered_call_rejected` | true -- the invoke's own promise rejected, recorded verbatim as `observation=attempt index=1 name="unregistered-call" outcome="rejected:Command vp_s3_unregistered_command not found"` |
| `unregistered_call_resolved` | false -- the underlying fetch (index 1) nonetheless resolved:200; see prose |
| `artifact_load_attempted` | true |
| `artifact_load_succeeded` | false |
| `static_inventory_pinned` | true -- the retained `vp-001-run-36990386851-vp-s3-static-inventory.txt` shows `test result: ok. 6 passed; 0 failed` on this run's own head commit 49f330402c61e6c30a8e74efbe645e2fde880cf8 |
| `run_date` | 2026-10-02 |
| `ci_run` | workflow tauri-build.yml, workflow_dispatch (vp001_scenario: true), run 36990386851, head 49f3304, conclusion success |
| `retention` | GitHub retains this run's `vp-001-linux-baseline` and `vp-001-transcripts` artifacts only until 2026-12-31 |
| `evidence_artifact` | a9bb393c4e6e52e5613e4b7298afbca0490031e04b53518b95a12bb900f09cd8 -- SHA-256 of the retained redacted file `docs/evidence/VP-001/artifacts/vp-001-run-36990386851-vp-s3-transcript.txt`, copied from run 36990386851's `vp-001-transcripts` artifact entry `vp-001-vp-s3-transcript.txt` |
| `feasibility_evidence_artifact` | 2984acc0ea9cbc0ba7ca2fa2a91f6fb3548cb90f86541d66d626fe8067ae5cb1 -- SHA-256 of the retained redacted file `docs/evidence/VP-001/artifacts/vp-001-run-36990386851-feasibility-transcript.txt`, copied from the same run's `vp-001-transcripts` artifact entry `vp-001-feasibility-transcript.txt` |
| `static_inventory_evidence_artifact` | cd4daabd91ecff524e2d76be1f4bbf0f6396d4a681eaa7979e3093e83435ef12 -- SHA-256 of the retained file, byte-identical to its CI artifact entry (it holds no runner path to redact), `docs/evidence/VP-001/artifacts/vp-001-run-36990386851-vp-s3-static-inventory.txt`, copied from the same run's `vp-001-transcripts` artifact entry `vp-001-vp-s3-static-inventory.txt` |
