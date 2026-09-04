# Desktop Stack Verification Plan

**Document role:** Desktop stack verification plan: pinned per-OS baselines, the executable scenario catalog, the evidence record, the exception rule, cadence and ownership, and the acceptance evidence for ADR-0002's gate (VP-001)  
**Status:** Draft  
**Normative force:** Non-binding target direction; requirements are acceptance gates, not current guarantees  
**Accountable role:** Project Maintainer  
**Named person:** Unassigned  
**Approver role:** Project Owner  
**Approver named person:** Unassigned  
**Effective date:** None  
**Supersedes:** None  
**Name status:** Selected public name; preliminary screening complete, formal trademark clearance pending

## Document authority

This document drafts the planned desktop stack verification plan (VP-001): the per-baseline fields recorded at first execution, the scenario catalog of executable evidence run against a packaged release build on each pinned Windows/macOS/Linux baseline, the evidence record schema and its storage, signing, and retention rules, the bounded exception path, the cadence and ownership that trigger a rerun, and the acceptance evidence that discharges [ADR-0002](adr/0002-desktop-technology-stack.md)'s acceptance gate: "Reproducible verification plan on pinned Windows/macOS/Linux baselines, security review, and confirmed Rust ownership" (ADR-0002, "Acceptance gate").

This draft does not redefine work already owned elsewhere:

- **[ADR-0002](adr/0002-desktop-technology-stack.md)** owns the stack decision itself — Tauri 2, the Rust core, the process supervisor, and the renderer choice; VP-001 proves the platform-specific claims that decision names, never chooses among them ([Proposed decision](adr/0002-desktop-technology-stack.md#proposed-decision)).
- **[Target architecture](target-architecture.md)** owns invariants, trust boundaries, and the required failure states VP-001's outcomes must respect.
- **[RCS-001](renderer-content-security.md)** owns renderer content-security mechanics — sanitization, CSP, navigation, the OSC policy; VP-001 verifies the pinned stack actually enforces them, never restates the mechanism.
- **[TM-001](threat-model.md)** owns the attacker model, trust boundaries, and the containment threat catalog; VP-001 supplies only the per-OS containment proof TM-001's PRC-5 and HAR threats require.
- **[UTA-001](update-trust-architecture.md)** owns update-trust roles, keys, and metadata; VP-001's update-signing scenarios test platform behavior only and "do not approve automatic updates until the separate update-trust architecture is accepted" ([Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact)).
- **[GOV-001](governance.md)** owns roles, approvals, and exception authority; VP-001 borrows GOV-001's exception registry rather than defining a parallel one.
- **[Versioning and compatibility](versioning-and-compatibility.md)** owns the support matrix and minimum supported versions, recorded "separately from the spike baselines" ([Acceptance evidence](adr/0002-desktop-technology-stack.md#acceptance-evidence)); VP-001 supplies the tested evidence that classification consumes, never the classification itself.

VP-001 exists because several claims other contracts already make cannot be settled by review alone. Containment "must be proven in the planned desktop verification plan" (adr/0002-desktop-technology-stack.md § Process supervision), and "until pinned per platform, the containment claim itself is unproven" (TM-001 PRC-5). The sanitizer library and the CSP-header delivery mechanism binding "is VP-001's evidence to produce" (renderer-content-security.md § Acceptance evidence and follow-up ("does not specify the exact")). The download-size cap figure stays "pending VP-001 evidence on quarantine-directory disk behavior" (RCS-001 D12). Each is an OS runtime behavior, a measured filesystem behavior, or a per-platform implementation binding — not a claim a document can settle by asserting it.

The [target architecture](target-architecture.md) governs any conflict.

## Purpose and scope

**VP-001 governs whether the platform-specific behaviors other contracts already assume — process containment, platform signing, sanitizer/CSP delivery, IPC narrowness, and accessibility — actually hold, per pinned baseline, on packaged release builds, with measurable pass, fail, and `uncertain` outcomes.**

Its audience is the Project Maintainer preparing evidence, the Project Owner approving ADR-0002 and the Pre-alpha → Alpha promotion gate, the Compatibility Owner classifying support-matrix rows against VP-001's results, and the Security Reviewer advising at the Alpha → Beta gate.

In scope:

- pinning the per-baseline evidence fields [Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact) names, recorded at first execution and never invented ahead of it;
- the scenario catalog: executable procedures run per pinned baseline on a packaged release build, each admitting `pass`, `fail`, or `uncertain`;
- the evidence record schema and its storage, signing, support-matrix mapping, and on-failure rules;
- the bounded exception path, borrowed from GOV-001's own exception registry;
- the cadence and ownership that trigger a rerun, full or delta;
- the evidence matrix and accept/remediate/supersede recommendation ADR-0002's gate requires ([Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact));
- the acceptance evidence — schema, coverage, and disclosed debt — that lets VP-001 itself move past `Draft`.

Out of scope:

- choosing the desktop stack — that decision belongs to [ADR-0002](adr/0002-desktop-technology-stack.md) ([Proposed decision](adr/0002-desktop-technology-stack.md#proposed-decision));
- renderer content-security mechanics — sanitization, CSP, navigation — owned by [RCS-001](renderer-content-security.md) ([Document authority](renderer-content-security.md#document-authority) ("document drafts the planned renderer"));
- the attacker model and trust boundaries — owned by [TM-001](threat-model.md) ([Purpose and scope](threat-model.md#purpose-and-scope) ("residual risk remains after that mitigation"));
- update-trust roles, metadata, and any approval of automatic updates — owned by [UTA-001](update-trust-architecture.md) ([Purpose and scope](update-trust-architecture.md#purpose-and-scope) ("which roles vouch for a release")); VP-001's update-signing tests never approve automatic updates on their own ([Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact));
- roles, approvals, and exception authority — owned by [GOV-001](governance.md) ([Document authority](governance.md#document-authority) ("Planned governance artifact"));
- minimum supported versions — owned by the compatibility policy's support matrix, "recorded separately from the spike baselines" ([Acceptance evidence](adr/0002-desktop-technology-stack.md#acceptance-evidence));
- classifying a tested combination as `supported`, `preview`, `detected`, or `unsupported` — the Compatibility Owner's own recommendation-and-decision right ([Support-matrix authority](governance.md#support-matrix-authority) ("Compatibility Owner classifies a tested")).

## Problem statement

A harness profile on Windows spawns a detached child process. The supervisor's Job Object policy should terminate every descendant when the parent stops, but the child calls an API that breaks out of the job object before the stop signal reaches it. Nothing in the log shows an error: the parent exited, the process tree the supervisor can see is gone, and the evidence trail reads "cleanly stopped." The child is still running, orphaned, holding whatever access the harness held. ADR-0002 already names this exact containment claim as something that "must be proven in the planned desktop verification plan" (adr/0002-desktop-technology-stack.md § Process supervision), and the target architecture already requires the honest fallback — "When descendants cannot be proven terminated, the result is `orphan-risk/uncertain`, not 'cleanly stopped'" (adr/0002-desktop-technology-stack.md § Process supervision) — but that fallback only fires if something actually launches a breakaway-attempting child, on Windows, on a packaged build, and watches what happens.

A renderer surface is built and served in development mode. Its CSP header comes from the dev server's own middleware, and every scenario in a release checklist passes against that server: no inline script executes, no disallowed fetch escapes `connect-src`. Ship the packaged Tauri binary, and the same header is now supposed to come from the WebView's own policy-injection path instead — a different delivery mechanism, on three different OS WebView engines. ADR-0002 requires testing "packaged release builds, not only development mode" (adr/0002-desktop-technology-stack.md § Planned verification artifact) for exactly this reason: a CSP pass in dev proves the policy string is well-formed, never that the packaged WebView actually enforces it.

An update ships with a valid Authenticode signature on Windows. Update trust's client-verification checklist treats platform verification as two checks, not one: an offline check against the embedded certificate chain, and an online check confirming that certificate has not since been revoked. When the revocation-list service is unreachable, "the platform step renders `uncertain` rather than being skipped or assumed passing" (update-trust-architecture.md § Platform signing and notarization ("Platform verification itself splits into")) — but a green "signed" outcome, read quickly, hides exactly that distinction. A binary can carry a perfectly valid signature from a certificate revoked an hour ago, and the word "signed" alone discloses nothing about whether the revocation check ever ran.

None of the three is a bug in the mechanism it uses. Each did exactly what its own contract promises — the Job Object policy applied, the CSP header was well-formed, the signature check passed — and each still produced a false positive, because the platform-specific gap sits between mechanisms that work individually and a claim that assumes, without checking, that they compose.

**A result is evidence only once it is produced by the platform it claims to describe, under the conditions that platform will actually run in production — never assumed, interpolated, or carried over from a different OS, a different build mode, or a different check that happened to pass nearby.**

## Definitions

Terms already defined by another artifact are used here with that artifact's meaning and are not redefined; only terms this document introduces, or narrows for its own purpose, appear below.

| Term | Meaning |
| --- | --- |
| Baseline | One pinned Windows, macOS, or Linux target — OS build, architecture, WebView/runtime, packaging substrate, and assistive technology — recorded at first execution ([Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact)). |
| Scenario | One executable procedure in the catalog verifying one claim against one baseline; admits `pass`, `fail`, or `uncertain`. |
| Packaged release build | The distributable artifact a user would actually install; never a development-mode server or debug build ([Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact)). |
| Discharge | The relationship between a passing scenario result and the requirement id(s) — in this document or another contract — that result satisfies. |
| `uncertain` | The distinct third outcome ([Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact); target architecture, invariant 8) recorded when a claim cannot be proven true or false on the available evidence; never reported as a pass. |
| Evidence artifact | The retained file — log, transcript, screenshot, capture, or digest — a scenario's evidence field identifies. |
| Exception | A [GOV-001](governance.md) exception-registry entry scoping a bounded, time-boxed deviation from one VP-001 requirement on one scenario/baseline pair; VP-001 defines no parallel mechanism ([Exceptions](governance.md#exceptions) ("requested by the role accountable")). |
| Cross-baseline transfer | Applying one baseline's passing result to a different OS or architecture row; prohibited under VP-001-R15. |
| Evidence matrix | The complete set of evidence-record rows produced by one VP-001 run; the deliverable [Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact) names alongside the accept/remediate/supersede recommendation. |
| Operator role | The role recorded as having executed a given scenario row; distinct from the role that later signs the resulting `Accepted` transition ([Approval record and repository integrity](governance.md#approval-record-and-repository-integrity) ("repository's `main` branch is protected")). |
| Full rerun | Executing all 21 scenarios against every pinned baseline; required before each promotion gate (open decision D7). |
| Delta rerun | Executing only the scenarios that discharge requirements tied to a changed field; the default between promotion gates (open decision D7). |

## Pinned baselines

Already pinned by other contracts, and not restated here as VP-001's own decision:

- Desktop shell and core: Tauri 2; a framework-independent Rust domain/application core; a Rust process supervisor with Tokio, Serde/`serde_json`, and `tracing`; a React/TypeScript/Vite renderer; Radix Primitives; CSS design tokens; xterm.js only for declared PTY fallback ([Proposed decision](adr/0002-desktop-technology-stack.md#proposed-decision)).
- Per-OS containment mechanism family: Windows Job Object policy and breakaway behavior; Linux cgroup/process-group/watchdog with a documented fallback; macOS process-group/watchdog behavior and explicit daemonization limits ([Process supervision](adr/0002-desktop-technology-stack.md#process-supervision)).
- Per-OS platform-signing mechanism family: macOS notarization plus a Developer ID signature; Windows Authenticode; Linux a signed package or a detached AppImage signature, per UTA-001 D10 packaging targets ([Platform signing and notarization](update-trust-architecture.md#platform-signing-and-notarization) ("Notarization plus a Developer ID")).
- Linux heavy-blob backend: the official Proton Drive CLI ([Local Markdown and tiered heavy assets](target-architecture.md#local-markdown-and-tiered-heavy-assets) ("Git is the supported transport")).
- Scope is OS families only — "pinned Windows/macOS/Linux baselines" (ADR-0002, "Acceptance gate"); "reproducible Windows, macOS, and Linux evidence" ([Pre-alpha → Exit criteria](roadmap.md#exit-criteria)).
- System WebView engine family per OS, named here as stack context rather than a pinned version: WebView2 on Windows, WKWebView on macOS, WebKitGTK on Linux — the concrete engine Tauri 2 binds on each platform. The exact WebView/runtime version is a field recorded at first execution, below.

Recorded at first execution, once per baseline, before any scenario result against it is admitted (adr/0002-desktop-technology-stack.md § Planned verification artifact; VP-001-R1):

- exact OS version and build;
- CPU architecture;
- WebView/runtime version (WebView2, WKWebView, or WebKitGTK, per above);
- Rust, Node, and toolchain versions, including the minimum supported Rust version (MSRV) — a VP-001 addition beyond [Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact)'s list, tied to the Rust domain/application core and Rust process supervisor ([Proposed decision](adr/0002-desktop-technology-stack.md#proposed-decision));
- Tauri patch version — a VP-001 addition beyond [Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact)'s list, tied to the Tauri 2 desktop shell ([Proposed decision](adr/0002-desktop-technology-stack.md#proposed-decision));
- packaging substrate for that OS, beyond UTA-001 D10;
- assistive-technology product and version exercised;
- Git and Engram version, within the range the compatibility policy's support matrix promises ([Engram runtime compatibility](versioning-and-compatibility.md#engram-runtime-compatibility));
- test date.

None of these values is invented in this draft; each is a field this plan records only at first execution.

A baseline is one OS/architecture/WebView/runtime combination, not an OS family: a second Windows baseline on a non-primary architecture counts as an additional baseline only when open decision D3 or the support matrix requires evidence for that pair. The default proposal is one primary architecture per OS through the Pre-alpha → Alpha gate, expanded before Beta (open decision D3).

## Scenario catalog

Every scenario below runs once per pinned baseline — never once for the OS family — against a packaged release build, never a development-mode build (adr/0002-desktop-technology-stack.md § Planned verification artifact; VP-001-R2). Each admits exactly three outcomes: `pass`, `fail`, or `uncertain` as a distinct third result, never folded into either extreme (adr/0002-desktop-technology-stack.md § Planned verification artifact; VP-001-R3). "OS" names the baseline(s) a scenario applies to; "W/M/L" means it runs identically on Windows, macOS, and Linux, each producing its own independently recorded row. Three scenarios (VP-S5, VP-S6, VP-S7) are single-OS by nature — a Job Object policy has no Linux or macOS analogue, and the reverse holds for cgroups — so each still discharges the shared containment requirement (VP-001-R5) through a platform-specific procedure rather than a shared one.

| ID | Claim | OS | Procedure | Pass criterion | Evidence | Discharges |
| --- | --- | --- | --- | --- | --- | --- |
| VP-S1 | CSP baseline enforced by the packaged webview | W/M/L | Load every renderer surface, including a third-party app surface, in a packaged build; attempt inline script, external fetch, and a framed context | No disallowed request, no inline script, no embedded browsing context | Console/network log plus policy dump, hashed | RCS-001-R13; evidence list ([Acceptance evidence and follow-up](renderer-content-security.md#acceptance-evidence-and-follow-up) ("CSP conformance: no network request")) |
| VP-S2 | Sanitizer and CSP-delivery binding on the pinned stack | W/M/L | Run the Markdown corpus through the shipped sanitizer; record the library id/version and delivery path | HTML/script/style always stripped; delivery mechanism recorded per OS | Corpus diff plus named mechanism per baseline | [Acceptance evidence and follow-up](renderer-content-security.md#acceptance-evidence-and-follow-up) ("does not specify the exact"); [State, content, and secrets](adr/0002-desktop-technology-stack.md#state-content-and-secrets) ("Renderer content defaults to plain") |
| VP-S3 | Typed-IPC bridge is not a `connect-src` network primitive; custom protocol handlers explicit | W/M/L | Exercise renderer→core calls; enumerate protocol handlers | Every bridge protocol documented and present in `connect-src`; no undocumented exception | Handler inventory per OS | [CSP baseline](renderer-content-security.md#csp-baseline) ("`connect-src 'none'` governs network-shaped requests"); RCS-001-R14 |
| VP-S4 | Terminal frame undisguisable; OSC policy holds | W/M/L | Replay OSC 8/52/1337-family, title, notification, and DCS/APC corpus into a packaged pane | No byte draws outside the pane, forges chrome, or moves focus; unknown sequences dropped and counted | Screenshot per theme plus diagnostics counts | RCS-001-R4, R5, R6 |
| VP-S5 | Windows containment: Job Object policy and breakaway | W | Launch a harness tree including a breakaway-attempting child; terminate | All descendants observed terminated, else `orphan-risk/uncertain` | Process-tree log before/after, hashed | [Process supervision](adr/0002-desktop-technology-stack.md#process-supervision); TM-001 PRC-1/PRC-5; TM-001-R9 |
| VP-S6 | Linux containment: cgroup/process-group/watchdog plus documented fallback | L | Same, plus the fallback path when the primary mechanism is unavailable | Mechanism and fallback each recorded with an observed terminal state | Same shape, plus mechanism name | [Process supervision](adr/0002-desktop-technology-stack.md#process-supervision); TM-001 PRC-5 |
| VP-S7 | macOS containment: process-group/watchdog and daemonization limits | M | Same, plus a daemonizing profile | Daemonizing profile reported unsupported/`orphan-risk`, never "cleanly stopped" | Same shape | [Process supervision](adr/0002-desktop-technology-stack.md#process-supervision); [Required failure states](target-architecture.md#required-failure-states) |
| VP-S8 | Platform signing/notarization per OS | W/M/L | Install and launch signed, tampered, and unsigned packaged artifacts | Invalid platform signature rejected on each OS, independent of role-metadata validity | Signature-check transcript per OS | UTA-001-R6; acceptance test ([Acceptance evidence and follow-up](update-trust-architecture.md#acceptance-evidence-and-follow-up) ("platform-signature test MUST verify rejection")) |
| VP-S9 | Offline/online split; unreachable revocation → `uncertain` | W/M/L | Verify with the revocation-list service unreachable | Step renders `uncertain`, automatic update blocked, offline-only status disclosed | Status capture plus reason string | [Platform signing and notarization](update-trust-architecture.md#platform-signing-and-notarization) ("Platform verification itself splits into"), [Acceptance evidence and follow-up](update-trust-architecture.md#acceptance-evidence-and-follow-up) ("verify that an unreachable revocation-list") |
| VP-S10 | Platform-signature and role-metadata results as two distinct rows | W/M/L | Record both checks separately | No collapsed "signed" outcome anywhere | Evidence-matrix schema check | [Platform signing and notarization](update-trust-architecture.md#platform-signing-and-notarization) ("sufficient: it proves the OS") |
| VP-S11 | Signed-update failure and packaging/update recovery | W/M/L | Interrupt or corrupt an update on a packaged build; attempt recovery | Recoverable or explicitly `uncertain`; never false completion | Update transcript plus resulting state token | [Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact), [Electron fallback candidate](adr/0002-desktop-technology-stack.md#electron-fallback-candidate); [Required failure states](target-architecture.md#required-failure-states) |
| VP-S12 | Offline first run and native secret custody fails closed | W/M/L | Clean install; first launch with no network; then with the credential service unavailable | Offline first run succeeds or degrades honestly; missing binding renders `unprovisioned-secret` | Install log plus custody label capture | adr/0002-desktop-technology-stack.md § Engram, § State, content, and secrets ("Omnifrons-owned secrets use OS credential"); target-architecture.md § Required failure states, § Content, secrets, update, and Git trust ("Omnifrons secrets use OS credential") |
| VP-S13 | IPC boundary: typed commands only; path attacks rejected | W/M/L | Fuzz typed IPC with malformed payloads, raw paths, symlink/junction and traversal cases | Rejection, not best-effort interpretation; renderer holds no generic shell capability | Rejection log plus canonicalization traces | [Privilege and IPC boundary](adr/0002-desktop-technology-stack.md#privilege-and-ipc-boundary), [Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact); target architecture, invariant 4 (invariant 4); RCS-001-R14 |
| VP-S14 | Executable identity, re-probe, shadowed-path resistance | W/M/L | Approve an executable, replace or shadow it, relaunch | Material change forces renewed approval before launch | Approval-record diff | [Executables and harnesses](adr/0002-desktop-technology-stack.md#executables-and-harnesses); TM-001 HAR-3/HAR-4 |
| VP-S15 | Structured streams, bounded backpressure, reconnection, malformed events, cancellation | W/M/L | Drive high-rate stdout/stderr, kill/restart the renderer, inject malformed events, cancel mid-run | Bounded queues; degraded-with-replay rather than a silent gap; cancellation observed | Stream metrics plus state transitions | [Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact); [Required failure states](target-architecture.md#required-failure-states) |
| VP-S16 | PTY per-OS behavior as declared degraded fallback | W/M/L | Run the PTY adapter path including an escape/OSC corpus | PTY stays advisory-labelled; only allowlisted controls become typed actions | PTY transcript plus label capture | [Privilege and IPC boundary](adr/0002-desktop-technology-stack.md#privilege-and-ipc-boundary), [Executables and harnesses](adr/0002-desktop-technology-stack.md#executables-and-harnesses); target-architecture invariant 6 |
| VP-S17 | Handoff interruption on each OS | W/M/L | Interrupt at prepare, publish, claim, import, switch, cleanup | Recoverable or explicitly `uncertain`; never false completion | Interruption matrix | [Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact); [Pre-alpha → Exit criteria](roadmap.md#exit-criteria) |
| VP-S18 | Git and Engram behavior including LFS/executable filter validation | W/M/L | Execute managed Git ops against repos carrying each execution-capable configuration entry; exercise LFS | No execution without specific consent; LFS/filters validated or blocked | Git-op log per configuration key | [Git](adr/0002-desktop-technology-stack.md#git); TM-001-R3; [Acceptance evidence and follow-up](threat-model.md#acceptance-evidence-and-follow-up) ("Git classification list, both classes") |
| VP-S19 | Accessibility, IME, and rendering across system WebViews | W/M/L | Run declared journeys with the pinned assistive technology; exercise IME input and text rendering | No release-blocking accessibility/IME/rendering difference; each recorded per baseline | AT session notes plus screenshots | [Costs](adr/0002-desktop-technology-stack.md#costs), [Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact) |
| VP-S20 | Quarantine/download behavior and executable marking | W/M/L | Download executables/archives; check quarantine dir, mark-of-the-web flag or fallback; attempt an oversize download | Quarantine outside any workspace; reveal-only; oversize refused; cap figure measured | Filesystem capture plus measured cap | [Content classes and rendering modes](renderer-content-security.md#content-classes-and-rendering-modes) ("itself lives under the product's"); RCS-001 D12 |
| VP-S21 | WebView baseline compatible enough to admit Tailwind | W/M/L | Render the design-token baseline and candidate utility CSS on each pinned WebView | Compatible baseline pinned, or the decision stays blocked | Rendering comparison per baseline | [Proposed decision](adr/0002-desktop-technology-stack.md#proposed-decision) |

A scenario's `Discharges` column names the requirement id, in the owning contract, that a passing result on this scenario counts as executable evidence for — RCS-001-R13, TM-001-R9, and UTA-001-R6 among them — never a VP-001 requirement of this document's own. VP-001's own requirements, in Product requirements below, govern how every scenario is run and recorded; they do not state what any single scenario proves about another contract's claim.

## Evidence record

One row is recorded per scenario × baseline.

| Field | Meaning |
| --- | --- |
| `scenario_id` | The catalog ID (VP-S1…VP-S21) this row executes. |
| `claim` | The one-sentence claim under test, copied from the scenario catalog. |
| `baseline_id` | The pinned Windows/macOS/Linux baseline this row was executed against. |
| `os_build` | Exact OS version and build, recorded at first execution ([Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact)). |
| `architecture` | CPU architecture of the baseline. |
| `webview_runtime` | WebView/runtime engine and version (WebView2, WKWebView, or WebKitGTK). |
| `packaging_substrate` | Per-OS packaging substrate, per UTA-001 D10. |
| `assistive_technology` | Assistive-technology product and version exercised, where the scenario requires one. |
| `build_channel_digest` | Build channel and the packaged artifact's digest. |
| `procedure_ref` | Pointer to the executed procedure text or script. |
| `result` | One of `pass`, `fail`, `uncertain` ([Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact)). |
| `observed_state_token` | The public vocabulary state this result maps to ([Synchronization states](versioning-and-compatibility.md#synchronization-states)). |
| `evidence_artifact` | Identifier and digest of the retained evidence artifact. |
| `operator_role` | Role of the person who executed the scenario. |
| `test_date` | Date the scenario was executed. |
| `discharged_requirement_id` | The requirement id(s) this passing row discharges. |

Every row also carries a review-ledger overlay per GOV-001-R5's per-finding schema.

Storage is append-only, never by mere convention: a provenance-clean result — naming no hostname, local path, company, estate, agent, or infrastructure detail — is retained in this public repository under `docs/evidence/<artifact-id>/`, protected by `main`'s signed-commit and linear-history requirements (governance.md § Review evidence and retention ("Retention: a review ledger, conformance", "Both evidence stores are append-only"); GOV-001-R15). A result disclosing infrastructure or personal detail goes to the private evidence store instead, "referenced from the public artifact by identifier only" (governance.md § Review evidence and retention ("Retention: a review ledger, conformance"); GOV-001 D2).

Signing: an `Accepted` transition for VP-001 itself needs both a signed tag `accept/<artifact-id>/<date>` and a row in `docs/evidence/approvals.md` naming the role, the evidence identifiers relied on, the self-approval flag, and an `advice_kind` of `human-security-reviewer`, `human-legal-counsel`, `human-independent-reviewer`, `automated-lens`, or `none` (governance.md § Review evidence and retention ("approval record's `advice_kind` field takes"), § Approval record and repository integrity ("repository's `main` branch is protected"); GOV-001-R14). Git commit authorship is never approval (GOV-001-R13). The self-approval flag on that row is not a formality: where the accountable and approver roles are exercised by the same named person, the record must say so (governance.md § Self-approval and conflict of interest ("accountable role and the approver"); GOV-001-R3), and a missing evidence location is itself recorded as a fact rather than silently omitted (governance.md § Review evidence and retention ("Retention: a review ledger, conformance")).

Retention: every row is retained for the life of the artifact it evidences plus one subsequent major version (GOV-001-R6).

Support-matrix mapping: a passing set of rows is the "tested evidence" a combination needs to be classified `supported`; identification alone is `detected`, and "detection alone grants no support" (versioning-and-compatibility.md § Version domains ("External Git, Engram, harness, OS"), versioning-and-compatibility.md § Support matrix, all rows). Leaving `supported` requires Project Owner approval and a release note (GOV-001-R12). A combination held at `detected` is reviewed at each minor release; it receives an explicit classification decision after two consecutive minors at `detected` (GOV-001-R18; governance.md § Support-matrix authority ("Compatibility Owner reviews every `detected`")).

On failure: a failed scenario blocks the claim it discharges — "a feature cannot claim the guarantee owned by a placeholder until its artifact is accepted and its tests pass" (target-architecture.md § Planned assurance artifacts ("feature cannot claim the guarantee")). An `uncertain` result is reported as such, never as a pass — "pending, stale, forked, conflicted, uncertain, partial, and orphan-risk states are never shown as complete" (target architecture, invariant 8) — and it feeds ADR-0002's accept/remediate/supersede recommendation (adr/0002-desktop-technology-stack.md § Planned verification artifact) and the Electron-fallback triggers ADR-0002 names for a release-blocking WebView, containment, or packaging failure (adr/0002-desktop-technology-stack.md § Electron fallback candidate). Every read of an evidence row that feeds a gate decision verifies that row's artifact digest first; a mismatch is handled as its own condition in Signal mapping below, never by editing the original row.

Every field above exists to make one promise operational: "'Latest' is not reproducible evidence" (versioning-and-compatibility.md § Support matrix ("records exact build numbers, architectures")). A row that omits `os_build`, `test_date`, or `evidence_artifact` cannot be re-derived later and is therefore not evidence at all — it is an assertion wearing the evidence record's shape.

## Exception rule

VP-001 defines no parallel exception mechanism of its own. ADR-0002's acceptance evidence reads: "VP-001 passes on every pinned baseline or records an approved, bounded exception" (adr/0002-desktop-technology-stack.md § Acceptance evidence). "Bounded" means exactly one GOV-001 exception-registry entry and nothing else: every mandatory column — id, MUST clause, owner, approver, scope, rationale, expiry, evidence, rollback plan, status — is required, and "an entry missing scope, rationale, expiry, evidence, rollback plan, or status is not a valid exception" (governance.md § Exceptions ("requested by the role accountable"); GOV-001-R7). Blocking is the default outcome for a non-passing baseline; an exception is the documented departure from that default, never a substitute for it (VP-001-R17).

Scope is exactly one scenario id on one named baseline — never a whole OS family. Expiry defaults to 90 days, with at most two renewals and 270 cumulative days, after which the exception lapses for good and the underlying clause changes only through a Proposed revision (governance.md § Exceptions ("exception's default expiry is 90"); GOV-001-R7). The approver differs from the owner whenever a second registered person exists for the relevant role; where none exists, the entry records that absence rather than pretending independence (governance.md § Exceptions ("exists for GOV-001-R4: the Alpha"); GOV-001-R7).

No exception relabels `uncertain` or `unsupported` behavior as guaranteed (GOV-001-R8), and no VP-001 exception may target GOV-001-R4, the requirement that forecloses any substitute for Security Reviewer and Legal Counsel advice at the Alpha → Beta gate (governance.md § Exceptions ("requested by the role accountable"), GOV-001-R4). The registry itself is currently empty; the renewal cap remains untested until a first exception against VP-001 is actually recorded and renewed (governance.md § Acceptance evidence and follow-up ("registry is present and currently")).

**An exception never converts an `uncertain` or `failed` VP-001 scenario result into a pass; it records bounded accepted risk around a still-failing or still-unproven claim, nothing more.**

## Cadence and ownership

Cadence triggers align to the roadmap's own maturity stages — Pre-alpha, Alpha, Beta, and 1.0 (roadmap.md § Maturity, channel, and SemVer mapping) — plus any stack or OS-baseline change that falls between them. A "delta" rerun covers only the scenarios discharging requirements tied to the changed field; a "full" rerun executes all 21 scenarios against every pinned baseline.

| Trigger | Scope of rerun | Owner |
| --- | --- | --- |
| Stack upgrade (Tauri, WebView/runtime, or toolchain version change) | Delta: re-pin the changed field(s) and rerun the scenarios discharging requirements tied to that field ([Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact)) | Project Maintainer prepares; Decision Owner maintains the evidence ([Review evidence and retention](governance.md#review-evidence-and-retention) ("role accountable for an artifact's")) |
| OS baseline change (a new pinned OS version or build) | Delta at a minor release; full rerun at a promotion gate | Project Maintainer |
| Every minor release | Delta scoped to changed fields, aligned with GOV-001-R18's `detected`-review cadence ([Support-matrix authority](governance.md#support-matrix-authority) ("Compatibility Owner reviews every `detected`")) | Compatibility Owner classifies ([Support-matrix authority](governance.md#support-matrix-authority) ("Compatibility Owner classifies a tested")) |
| Approved exception nearing its expiry (GOV-001 D3) | Delta: rerun only the scoped scenario/baseline pair before the exception lapses | Project Maintainer |
| Before Pre-alpha → Alpha | Full rerun; evidence named "desktop spike result" ([Promotion and approval gates](roadmap.md#promotion-and-approval-gates); [Promotion gates: authority and evidence](governance.md#promotion-gates-authority-and-evidence) ("Portable-work proof, protocol drafts, desktop")) | Project Maintainer prepares; Project Owner approves |
| Before Alpha → Beta | Full rerun; TM-001-R14 re-evaluation; cross-checked against RCS-001 and UTA-001 evidence | Security Reviewer advises ([Promotion and approval gates](roadmap.md#promotion-and-approval-gates)); Project Owner approves |
| At the 1.0 promotion gate | Full rerun; upgrade and rollback drills pass "on each supported OS family" ([Version 1.0 → Exit criteria](roadmap.md#exit-criteria-3); [Rollback and recovery qualification](versioning-and-compatibility.md#rollback-and-recovery-qualification)) | Release Approver verifies release evidence ([Role catalog](governance.md#role-catalog) ("Verifies release evidence and signed")); Project Owner approves |

The default is delta reruns at minor releases and a full rerun at every promotion gate (open decision D7).

## Product requirements

Each requirement is an acceptance gate with a testable condition. This document's requirements are new — VP-001-R1 through VP-001-R20 — and do not continue any other document's numbering; VP-001 has its own registry entry in the target architecture's planned assurance artifacts table (target architecture, VP-001 row).

| ID | Requirement |
| --- | --- |
| VP-001-R1 | VP-001 MUST record exact OS build, architecture, WebView/runtime version, packaging substrate, assistive-technology product, and test date for a baseline before any scenario result against that baseline is admitted ([Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact)). |
| VP-001-R2 | A scenario result MUST be produced against a packaged release build; a development-mode-only result MUST NOT be recorded as evidence ([Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact)). |
| VP-001-R3 | Every scenario MUST define a measurable pass criterion and MUST admit `uncertain` as a distinct third outcome, never collapsed into pass or fail ([Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact)). |
| VP-001-R4 | VP-001 MUST produce an evidence matrix and an accept, remediate, or supersede recommendation for ADR-0002 ([Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact); [Pre-alpha → Exit criteria](roadmap.md#exit-criteria)). |
| VP-001-R5 | Per-OS containment MUST be executed on every pinned baseline; unproven descendant termination MUST be recorded `orphan-risk/uncertain`, never "cleanly stopped" ([Process supervision](adr/0002-desktop-technology-stack.md#process-supervision); TM-001-R9). |
| VP-001-R6 | The Linux fallback containment mechanism MUST be named and executed as its own scenario, separate from the primary mechanism ([Process supervision](adr/0002-desktop-technology-stack.md#process-supervision)). |
| VP-001-R7 | A daemonizing or containment-escaping profile MUST be recorded unsupported, never as a passing containment result ([Process supervision](adr/0002-desktop-technology-stack.md#process-supervision); TM-001 PRC-5). |
| VP-001-R8 | Platform-signature verification and role-metadata verification MUST be recorded as two distinct evidence rows, never collapsed into one "signed" outcome ([Platform signing and notarization](update-trust-architecture.md#platform-signing-and-notarization) ("sufficient: it proves the OS")). |
| VP-001-R9 | An update-signing scenario's pass MUST NOT be recorded or interpreted as approval of automatic updates ([Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact); UTA-001-R14). |
| VP-001-R10 | An unreachable revocation-list check during platform verification MUST be recorded `uncertain` with automatic updates blocked, never as a pass ([Platform signing and notarization](update-trust-architecture.md#platform-signing-and-notarization) ("Platform verification itself splits into")). |
| VP-001-R11 | The sanitizer implementation and the CSP-delivery mechanism MUST be recorded per baseline, and CSP enforcement MUST be verified on every renderer surface including a third-party app surface ([Acceptance evidence and follow-up](renderer-content-security.md#acceptance-evidence-and-follow-up) ("does not specify the exact"), R13). |
| VP-001-R12 | Every bridge protocol reaching `connect-src` MUST be enumerated per OS and cross-checked against RCS-001's own documented list ([CSP baseline](renderer-content-security.md#csp-baseline) ("`connect-src 'none'` governs network-shaped requests")). |
| VP-001-R13 | Quarantine-directory disk behavior MUST be measured and reported to resolve RCS-001 D12 download-size cap. |
| VP-001-R14 | LFS or executable Git filters MUST NOT be declared supported on a baseline until that baseline's scenario explicitly validates them ([Git](adr/0002-desktop-technology-stack.md#git)). |
| VP-001-R15 | A passing result MUST NOT transfer between baselines; each OS/architecture row MUST carry its own independently recorded result ([Support matrix](versioning-and-compatibility.md#support-matrix) ("release pins exact test evidence", "records exact build numbers, architectures")). |
| VP-001-R16 | Every scenario result MUST be filed at GOV-001's evidence location, append-only, and retained for the life of the artifact plus one subsequent major version (GOV-001-R6; [Review evidence and retention](governance.md#review-evidence-and-retention) ("Retention: a review ledger, conformance", "Both evidence stores are append-only")). |
| VP-001-R17 | A non-passing baseline MUST block every claim it would discharge unless a GOV-001 exception-registry entry naming that scenario and that baseline covers it ([Acceptance evidence](adr/0002-desktop-technology-stack.md#acceptance-evidence); GOV-001-R7/R8). |
| VP-001-R18 | Tailwind MUST NOT be treated as admissible until VP-001 records a compatible-WebView-baseline pinning result for every supported OS ([Proposed decision](adr/0002-desktop-technology-stack.md#proposed-decision)). |
| VP-001-R19 | Every pinned baseline MUST record accessibility, IME, and rendering evidence, including the assistive-technology product and version exercised, before any WebView compatibility/accessibility claim is treated as verified, and the result MUST be `uncertain` when the pinned assistive technology cannot be exercised ([Costs](adr/0002-desktop-technology-stack.md#costs), [Planned verification artifact](adr/0002-desktop-technology-stack.md#planned-verification-artifact)). |
| VP-001-R20 | Every gate decision that consumes an evidence row MUST verify the row's artifact digest first, and a mismatch MUST be recorded as `failed` by an appended correction row, never by editing the original. |

## Signal mapping

The public vocabulary column reuses only tokens the target architecture's required failure states and the compatibility policy's public-state list already define (target-architecture.md § Required failure states; versioning-and-compatibility.md § Synchronization states); VP-001 proposes no new public token.

| Condition | VP-001 state | Public vocabulary | Consequence |
| --- | --- | --- | --- |
| Baseline fields incomplete before first scenario execution | `baseline-unpinned` | `uncertain` | No scenario result against that baseline is admitted (VP-001-R1) |
| Scenario executed against a development-mode build only | `dev-mode-only` | `failed` | Result rejected; a packaged-build rerun is required (VP-001-R2) |
| Process descendants unproven terminated | `orphan-risk/uncertain` | `uncertain` | Never recorded as cleanly stopped ([Required failure states](target-architecture.md#required-failure-states); TM-001-R9) |
| Platform-signature and role-metadata checks disagree | `signed-mismatch` | `failed` | Applies only to the failing check; recorded as two distinct rows, never merged into one passing "signed" outcome ([Platform signing and notarization](update-trust-architecture.md#platform-signing-and-notarization) ("sufficient: it proves the OS")) |
| Platform revocation-list unreachable | `platform-revocation-unreachable` | `uncertain` | Automatic update blocked; offline-only status disclosed ([Platform signing and notarization](update-trust-architecture.md#platform-signing-and-notarization) ("Platform verification itself splits into")) |
| Bridge protocol missing from `connect-src` documentation | `undocumented-bridge` | `failed` | Scenario fails until RCS-001's CSP section is updated ([CSP baseline](renderer-content-security.md#csp-baseline) ("`connect-src 'none'` governs network-shaped requests")) |
| Secret binding missing on device | `unprovisioned-secret` | `unprovisioned-secret` | Custody label recorded; scenario records the label, not a workaround ([Required failure states](target-architecture.md#required-failure-states)) |
| Pinned assistive technology unavailable at execution time | `assistive-technology-unavailable` | `uncertain` | VP-S19 recorded `uncertain` for that baseline, never skipped or passed; VP-001-R19 not discharged |
| Cross-baseline result reused for a different OS/architecture | `cross-baseline-reuse` | `failed` | Row rejected; each baseline requires its own independently recorded result ([Support matrix](versioning-and-compatibility.md#support-matrix) ("release pins exact test evidence")) |
| Evidence artifact recorded without an identifier and digest | `unverifiable-evidence` | `failed` | Row rejected; a scenario's evidence field must resolve to a retrievable artifact before the result is admitted |
| Evidence digest does not match the retained artifact | `evidence-digest-mismatch` | `failed` | Row treated as failed; a correction row is appended naming the mismatched row by identifier; the original row is never edited or removed (append-only); the Release Approver is notified (VP-001-R20) |
| `Discharges` names a requirement id absent from any accepted or drafted contract | `undischargeable` | `failed` | Row rejected at the discharge-mapping check named under Acceptance evidence |
| Approved exception on file for the failing scenario/baseline pair | `exception-recorded` | `uncertain`/`failed` | Unchanged from the original result; bounded accepted risk recorded; the underlying result is never relabelled a pass (GOV-001-R8) |

## Open decisions

These are the questions this draft proposes a default for without yet closing; the Project Owner's recorded approval, not the presence of a default here, is what closes one — the same discipline governance.md § Open decisions ("proposes an answer to without") states for GOV-001's own open decisions.

| ID | Question | Options | Default proposal |
| --- | --- | --- | --- |
| D1 | Where are pinned baseline values recorded? | Inside VP-001; support matrix only; both, with VP-001 canonical for spike baselines | In VP-001, separate from minimum supported versions ([Acceptance evidence](adr/0002-desktop-technology-stack.md#acceptance-evidence)) |
| D2 | CI runners or manual runs? | CI only; manual only; CI for repeatable scenarios plus manual for assistive technology, signing, and notarization | Hybrid; not pinned in sources |
| D3 | Non-primary CPU architectures required? | One per OS; two; matrix-driven by UTA-001's "per supported OS/architecture pair" | One per OS at Pre-alpha → Alpha, expanded before Beta |
| D4 | Evidence store location | Public `docs/evidence/<artifact-id>/` only; private only; split by provenance | Split by provenance (GOV-001 D2) |
| D5 | Screenshot evidence required? | All UI scenarios; only where RCS-001 requires; never | Where RCS-001's checklist names a screenshot ([Acceptance evidence and follow-up](renderer-content-security.md#acceptance-evidence-and-follow-up) ("pane frame renders with visible")), provenance-clean |
| D6 | Who executes and who signs? | Maintainer executes and signs; Maintainer executes, Owner signs; Independent Reviewer executes | Maintainer executes, Project Owner signs with the self-approval flag recorded (GOV-001-R3) |
| D7 | Full or delta rerun? | Full each trigger; delta scoped to the changed field; full at gates, delta at minors | Delta at minors, full at each promotion gate |
| D8 | Failing baseline → support matrix? | Straight to `unsupported`; hold at `detected`; `preview` with a bounded exception | Hold at `detected` pending classification (GOV-001-R18) |

## Acceptance evidence and follow-up

- A schema completeness check MUST verify that every field in the Evidence record table above is present on every recorded row, with no field silently omitted.
- A scenario-completeness check MUST verify that all 21 scenarios in the catalog carry a measurable pass criterion and name at least one evidence artifact.
- A first-execution check MUST verify that every VP-001-R1 field is recorded for a baseline before any scenario result against it is admitted.
- An `uncertain`-outcome check MUST verify that at least one scenario, exercised end to end, actually produces and correctly records an `uncertain` result — not only `pass` and `fail` — proving the pipeline distinguishes the third outcome rather than discarding it or defaulting to it.
- An exception-validation check MUST verify that a GOV-001 exception-registry entry missing any mandatory column (scope, rationale, expiry, evidence, rollback plan, or status) is rejected rather than recorded (GOV-001-R7).
- A discharge-mapping check MUST verify that every scenario's `Discharges` column names a requirement id that exists in an accepted or drafted contract, and that no scenario discharges an id that does not exist.

Debt, not drafted here. This document does not pin the exact OS build, architecture, WebView/runtime, packaging substrate, assistive-technology product, or toolchain version values themselves — each is a field this plan records only at first execution, never a default this draft assumes. The pointer from the target architecture's planned-assurance-artifacts row to this document is deferred to that table's first `Proposed` revision carrying a signed acceptance tag (GOV-001-R16, governance.md § Change control ("Some sections are frozen regardless"), governance.md § Acceptance evidence and follow-up ("exercised by an attempted change")), because the table is frozen and changes only that way.

It does not define the CI-runner or manual-execution infrastructure that will actually host a scenario run — that is open decision D2, undecided here. It does not bootstrap the `docs/evidence/<artifact-id>/` directory GOV-001's storage rule requires — that directory does not yet exist. And it does not create `docs/evidence/approvals.md`, the append-only approvals file GOV-001 declares a precondition for any `Accepted` transition, including VP-001's own (governance.md § Approval record and repository integrity ("repository's `main` branch is protected")).

## Related contracts

- [ADR-0002: Desktop technology stack](adr/0002-desktop-technology-stack.md) — the acceptance gate this plan discharges, the pinned stack ([Proposed decision](adr/0002-desktop-technology-stack.md#proposed-decision)), and the per-OS containment claims ([Process supervision](adr/0002-desktop-technology-stack.md#process-supervision)) delegated here.
- [Target architecture](target-architecture.md) — the VP-001 row in planned assurance artifacts (target architecture, VP-001 row), invariants 2, 4, 5, 6, and 8, and the required failure states this plan's outcomes must respect.
- [Product roadmap](roadmap.md) — the "desktop spike result" evidence item at the Pre-alpha → Alpha gate ([Promotion and approval gates](roadmap.md#promotion-and-approval-gates)) and the Windows/macOS/Linux evidence exit criterion ([Pre-alpha → Exit criteria](roadmap.md#exit-criteria)).
- [Versioning and compatibility](versioning-and-compatibility.md) — the support-matrix classifications ([Support matrix](versioning-and-compatibility.md#support-matrix) (all rows)) this plan's evidence feeds, and the "'Latest' is not reproducible evidence" rule ([Support matrix](versioning-and-compatibility.md#support-matrix) ("records exact build numbers, architectures")).
- [Governance](governance.md) (GOV-001) — the exception-registry mechanics, evidence-retention rule, and approval-record schema this plan borrows rather than redefines; drafted, acceptance pending.
- [Threat model](threat-model.md) (TM-001) — PRC-1 through PRC-5, TM-001-R9, and the containment threat catalog this plan supplies the executable proof for; drafted, acceptance pending.
- [Update trust architecture](update-trust-architecture.md) (UTA-001) — the two-distinct-rows rule for platform-signature and role-metadata evidence ([Platform signing and notarization](update-trust-architecture.md#platform-signing-and-notarization) ("sufficient: it proves the OS")), and the offline platform-revocation `uncertain` outcome ([Platform signing and notarization](update-trust-architecture.md#platform-signing-and-notarization) ("Platform verification itself splits into")); drafted, acceptance pending.
- [Renderer content-security contract](renderer-content-security.md) (RCS-001) — the CSP-delivery and sanitizer-binding debt named in [Acceptance evidence and follow-up](renderer-content-security.md#acceptance-evidence-and-follow-up) ("does not specify the exact"), and the quarantine-directory disk-behavior evidence named in RCS-001 D12; drafted, acceptance pending.
- [Adapter feed event schema](adapter-feed-events.md) (AEC-001 feed profile) — the PTY normalization implementation VP-S4 and VP-S16 exercise against the terminal control policy RCS-001 states; drafted, acceptance pending.
- [Voice interaction contract](voice-interaction-contract.md) (VOC-001) — the VP-S19 accessibility scenario and pinned-baseline evidence VOC-001's accessibility claims depend on; drafted, acceptance pending.
- [Handoff transaction protocol](handoff-transaction-protocol.md) (HTP-001) and [Workspace roaming protocol](workspace-roaming-protocol.md) (RSP-001 core) — the interruption-recovery scenarios (VP-S11, VP-S17) this plan exercises on a packaged build; neither document names a platform of its own; drafted, acceptance pending.
- [ADR convention](adr/README.md) — the status lifecycle VP-001 follows once it leaves Draft.

## References

- [Tauri process model](https://v2.tauri.app/concept/process-model/) — the process model whose per-OS containment behavior this plan verifies.
- [Tauri security](https://v2.tauri.app/security/) — the security model ADR-0002 cites for the stack decision this plan tests.
- [Apple notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution) — the macOS platform-signing mechanism VP-S8 and VP-S10 verify.
- [Windows Authenticode](https://learn.microsoft.com/windows/win32/seccrypto/authenticode) — the Windows platform-signing mechanism VP-S8 and VP-S10 verify.
- [AppImage documentation](https://docs.appimage.org/) — the Linux packaging target VP-S8 exercises, per UTA-001 D10.
- [Target architecture](target-architecture.md) — the required failure states and invariants this plan's scenario outcomes implement.
- [Threat model](threat-model.md) — the containment threat catalog (PRC-1..5) this plan's evidence discharges.
