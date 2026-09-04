# Voice Interaction Contract

**Document role:** Voice interaction contract: consent, visibility, processing, retention, accessibility, text fallback (VOC-001)  
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

This document drafts the planned voice interaction contract (VOC-001): the consent model for microphone capture, visibility while capturing or speaking, where speech processing happens and what leaves the device, retention and deletion of audio and transcripts, the accessibility relationship between voice and text, the text-fallback guarantee in both directions, and the approval semantics of a spoken utterance. VOC-001 is registered in the target architecture's planned assurance artifacts table (target-architecture:283); the renderer content-security contract and the memory synchronization profile already point at it as owner (renderer-content-security:24, :54, :337; roaming-and-engram-sync:245).

This draft does not redefine work already owned elsewhere:

- **Sanitization, allowlists, CSP, and reading order** — owned by [RCS-001](renderer-content-security.md) (renderer-content-security:16, :234, :261). VOC-001's spoken output reads only RCS-001's sanitized, reading-order-preserved text.
- **The attacker model** — owned by [TM-001](threat-model.md) (threat-model:31, :102-128). VOC-001 proposes the additions TM-001's own debt note calls for (threat-model:307) as text for TM-001's next revision; it does not edit TM-001's tables itself.
- **Roles, approvals, exceptions, and evidence** — owned by [GOV-001](governance.md) (governance:16, :33-51).
- **The typed event catalog and the approvals write path** — owned by [AEC-001](adapter-feed-events.md) (adapter-feed-events:15, :150-162).
- **Sync mechanics and credential custody** — owned by [RSP-001](roaming-and-engram-sync.md) (roaming-and-engram-sync:237-246).
- **Backup, tombstone, and restore formats** — owned by [MRP-001](migration-and-recovery-plan.md) (migration-and-recovery-plan:16).
- **Presentation of any indicator this document requires** — owned by [context-orb.md](context-orb.md).
- **The public state-token vocabulary** — owned by [versioning-and-compatibility.md](versioning-and-compatibility.md) (versioning-and-compatibility:74).
- **Naming and trademark clearance for any wake-word or voice-feature branding** — owned by [product-naming.md](product-naming.md) (product-naming:37).

The [target architecture](target-architecture.md) governs any conflict.

## Purpose and scope

VOC-001 settles the consent model for microphone capture, visibility while capturing or speaking, where speech processing happens and what leaves the device, retention and deletion of audio and transcripts, the accessibility relationship between voice and text, the text-fallback guarantee in both directions, and the approval semantics of a spoken utterance.

Audience: the Project Maintainer preparing Beta evidence (roadmap:119, :126-127); the Project Owner deciding the Alpha → Beta and Beta → 1.0 promotions; the Security Reviewer and Legal Counsel whose advice the Alpha → Beta gate requires and who are both Unassigned today (roadmap:174; governance:126-127, :209, :246); and the Compatibility Owner classifying voice rows in the support matrix (versioning-and-compatibility:86-95, :120).

That classification is one of `supported`, `preview`, `detected`, or `unsupported` per OS/build, architecture, speech-engine version, and assistive-technology combination (versioning-and-compatibility:120-127) — a classification this document's evidence feeds but does not itself assign.

Timing: voice ships at Beta per the roadmap's own scope (roadmap:119), gated by the TM-001 additions this document proposes and by the Security Reviewer and Legal Counsel advice GOV-001-R4 requires (D8; governance:246) — not yet a Project Owner-approved timing until that advice is recorded.

Non-goals — owned elsewhere and not restated here:

- A promised level of recognition quality, a specific latency, a provider's own retention behavior, or voice availability on any given device; these stay external characteristics unless the support matrix separately promises them (versioning-and-compatibility:95).
- Selecting a speech engine or a remote provider; VOC-001 states the consent, disclosure, and retention rules a chosen engine or provider must satisfy, not which one is chosen (D2, D3, below).
- Wake-word branding or naming; that clearance sits with the product naming gate's own accessibility and pronunciation item (product-naming:37).
- Ambient-voice or always-listening capture as a shipped surface; both are explicit non-goals through Beta and a post-1.0 evidence horizon (roadmap:73, :134, :187).

At 1.0, stable voice behavior is a named public surface, when voice is enabled at all (versioning-and-compatibility:88-93):

- explicit microphone consent and persistent capture/transmission indication;
- an immediate stop control;
- documented local-versus-remote processing and retention choice;
- no restoration of prior approvals from transcription or synchronized content;
- complete text fallback when voice is denied, unavailable, or fails;
- persisted voice preferences and their own migration rule.

VOC-001 is the detailed protocol and test surface that public list already points to (versioning-and-compatibility:95).

## Problem statement

A user says "yes" in reply to a harness's spoken confirmation before a destructive action; the harness proceeds, because a recognized affirmative sounded like an answer to the question it had just asked. Nothing forged consent — the microphone captured exactly what was said — but a spoken "yes" is content, not an act on an approval surface, and treating it as one would fold TM-001's binding principle that content is never authority (threat-model:63) into a voice-shaped exception.

One indicator lights up for both "the microphone is capturing" and "audio is being sent off-device," merged for visual economy. A user who consented only to local capture cannot tell, from the indicator alone, whether their voice already left the device — the two states the compatibility policy requires to stay distinct (versioning-and-compatibility:88) have been quietly folded into one, and the folding itself is the failure, not any single dishonest render.

A voice subsystem fails mid-session — the local engine crashes, or a remote call times out — and the surface that depended on it goes dark instead of falling back to the typed workflow underneath it. The user loses not just voice but the whole task, because nothing preserved "the complete text workflow and recovery control" the required failure states already promise for this condition (target-architecture:266; renderer-content-security:276).

**Voice may propose; only a typed or clicked act on an approval surface may dispose.**

## Definitions

- **Capture:** The device recording audio from the microphone into a live buffer for recognition. Capture consent and transmission consent are separate grants.
- **Transmission:** Audio, or data derived from it, leaving the device toward a remote speech provider — distinct from capture and requiring its own opt-in.
- **Local processing:** Speech recognition or synthesis performed entirely on-device, with no network transmission of audio or derived data.
- **Remote processing:** Speech recognition or synthesis performed by an off-device provider, disclosed by provider class before the first use that requires it.
- **Transcript:** The text output of speech recognition. A transcript retained beyond the recognition pass is a memory-plane item.
- **Utterance:** A single spoken input, treated as content under TM-001's binding principle (threat-model:63) until a typed or clicked act on an approval surface disposes of the action it proposes.
- **Bystander:** A person other than the device's user who may hear spoken output or be captured incidentally by an open microphone.
- **Capture trigger:** The mechanism that starts capture — push-to-talk (an explicit hold or tap) or a wake word (passive listening for a phrase). VOC-001 fixes push-to-talk as the only trigger through 1.0 (D1; roadmap:134, :187).
- **Provider class:** The disclosed category of a remote speech provider (for example, a named vendor family or a self-hosted class) shown to the user before the opt-in that would send audio there is granted; not the provider's exact endpoint or internal configuration.
- **Approval surface:** The product surface a destructive, elevating, or publishing action is confirmed on, showing identity evidence and the act-as identity, per TM-001-R7 (threat-model:244). Voice can reach this surface only as a proposal, never as the confirming act itself.
- **Text fallback:** The guarantee, symmetrical in both directions, that every voice command has a typed equivalent and every spoken output has a visible text equivalent (versioning-and-compatibility:92).
- **Capture indicator:** The persistent, product-controlled UI element showing that the microphone is capturing. Never a rendered-content surface, and never themeable or overridable by content (Consent and visibility, below).
- **Transmission indicator:** The persistent, product-controlled UI element showing that audio is being transmitted off-device. Distinct from the capture indicator; the two MUST NOT be merged into one signal (Consent and visibility, below).
- **Degraded state:** A voice condition — no microphone permission, an unreachable speech service, low confidence, an ambiguous command — that the product renders honestly rather than papering over with a silent retry or a guessed result (Processing and what leaves the device, below).
- **Consent record:** The device-local record of a granted or revoked capture or transmission consent, distinct from and never substituted by the OS microphone grant (Consent and visibility, below).
- **Session:** The bounded interval a transcript's default retention is scoped to; a transcript does not outlive its session unless the user opts in, per scope, to memory-item retention (Retention and deletion, below).

## Consent and visibility

Capture requires a device-local, per-feature consent record, separate from transmission consent and separate from the OS microphone grant: the OS grant is a precondition, never a substitute (versioning-and-compatibility:88). Consent is revocable without uninstalling the product, and its current state is inspectable in product UI. Revoking capture consent takes effect immediately and stops any capture in progress and blocks the next capture attempt; it does not require a restart or an explicit "apply" step distinct from the revocation itself. Like every other approval, a voice consent record is device-local and never enters portable state (target-architecture:94; invariant 2, target-architecture:56; versioning-and-compatibility:50). A synchronized preference or a transcript pulled from another device cannot grant, restore, or imply consent — consent is device-local exactly like an executable approval, and synchronized content is untrusted input under the same invariant that governs every other approval (invariant 9, target-architecture:63; versioning-and-compatibility:91).

Visibility is two states, never one:

- a persistent, product-controlled, non-themeable indicator while the microphone is capturing;
- a second, distinct indicator while audio is being transmitted off-device.

The two MUST NOT merge — the compatibility policy already requires "persistent capture/transmission indication" as stable behavior (versioning-and-compatibility:88), and the Beta exit criterion repeats it: voice failure "never hides capture or transmission state" (roadmap:127). The undisguisability discipline follows RCS-001-R5's terminal-pane precedent — product-controlled chrome that rendered content cannot theme or override (renderer-content-security:248) — and the Orb's always-visible scope/trust indicator (context-orb:294) is the closest existing precedent for a permanently visible, non-negotiable state chip. An immediate stop control reaches every surface where capture or playback is active (versioning-and-compatibility:89). If the indicator cannot render, capture does not start: an unrenderable indicator is a capture-refusal condition, not a silent-capture one (Signal mapping below).

The default capture trigger is push-to-talk through 1.0; a wake word — passive, always-listening capture — is out of scope for Alpha and Beta alike and stays a post-1.0 evidence horizon (D1; roadmap:73, :134, :187).

The product cannot obtain a bystander's consent to capture. Under the push-to-talk default (D1), every capture window is opened by the user, who is the only party able to judge who else is within range; speech from a bystander inside that window is captured and transcribed exactly like the user's own. Excluding always-listening capture (VOC-001-R20) and defaulting a retained transcript to session-only (D4) narrow this exposure but do not guarantee it away — a bystander inside an open push-to-talk window has no way to opt out of the capture itself.

Bystander protection is a per-scope opt-out: a scope can disable spoken output, or fall back to text-only, when the user indicates another person is present, without disabling capture or transcription for that same scope. Independent of that opt-out, spoken output MUST NOT speak content flagged secret-custody or redaction-flagged under RCS-001's own redaction categories (renderer-content-security:226) — a bystander who cannot see a screen must not be able to hear what a screen reader for the same user would never render in the clear either (D6). TM-001's actor table does not yet name a bystander; the Proposed threat-model additions section below proposes that addition.

## Processing and what leaves the device

Local-only processing is the default: recognition and synthesis run on-device, and nothing about a captured utterance leaves the device unless the user has separately opted in to remote processing. Remote processing requires its own explicit opt-in, distinct from the capture consent above, disclosing the provider class and — in the spirit of the enrollment dry-run's inclusion-set disclosure (target-architecture:213) — an enumerated list of what leaves the device (D3; versioning-and-compatibility:90, :95). That enumerated list names, at minimum, whether raw audio, a derived transcript, or request metadata crosses the device boundary — the same granularity the enrollment dry-run already uses for personal-observation inclusion, applied here to a spoken utterance instead of a memory namespace. Provider-specific retention, recognition quality, availability, and latency remain external characteristics VOC-001 does not promise (versioning-and-compatibility:95).

A remote provider's credential lives in OS credential storage, the same custody discipline every other Omnifrons secret follows (target-architecture:245); when absent, it is labelled `unprovisioned-secret` (target-architecture:262), with no in-product secret entry (threat-model:263). Detecting that a credential is unprovisioned is a custody fact, not a security judgment on its own — the same distinction TM-001 draws between a Git binary's presence and its trustworthiness applies here.

The local engine itself is expected to be OS-provided per platform by default, recorded as a VP-001 baseline field alongside the assistive-technology product and version VP-001-R1 already records for every other baseline (D2; desktop-stack-verification-plan:215).

Five degraded states recur across this document and never retry silently, never guess, and never render success (invariant 8, target-architecture:62; AEC-001's design rule 5, "Unknown is a value," adapter-feed-events:27):

- no microphone permission;
- an unreachable speech service;
- low recognition confidence;
- an ambiguous command;
- a mid-capture loss of microphone hardware, OS permission, or consent record (Signal mapping below).

A remote call that times out or a provider that returns an error is `service-unreachable`, not silently retried against a different, undisclosed provider (Signal mapping below).

Provider substitution — falling back from one remote provider to another without asking again — would itself be a new transmission the user never opted into for that second provider; it is treated as a fresh remote-processing decision requiring its own opt-in under D3, not a resilience feature of the first opt-in.

## Retention and deletion

Audio is buffered for recognition only and is never persisted by default; it never enters a portable artifact, a backup, or a support export (renderer-content-security:258).

A retained transcript is opt-in per scope and, once retained, is a memory-plane item: it inherits Engram's `<private>` stripping before storage, which strips only explicitly tagged spans and is not itself a confidentiality boundary — a retained transcript synchronizes to every paired device and to Cloud exactly like any other memory observation (roaming-and-engram-sync:85; threat-model:93, :184). This residual exposure is why the default below is session-only rather than a retain-by-default opt-out (D4). Deleting a retained transcript writes a tombstone of kind `observation` that propagates through backup and restore (migration-and-recovery-plan:203-212).

The default transcript retention is session-only — a transcript does not outlive the session that produced it unless the user opts in, per scope, to memory-item retention (D4). Where a transcript is retained as a memory item, VOC-001 MAY set a shorter retention default than the memory plane's own, and MUST NOT set a longer one: the memory plane's 180-day tombstone-retention default (migration-and-recovery-plan:321, MRP-001 D6) is a ceiling this document does not raise. Deletion is always a tombstone, never a hard delete indistinguishable from content that never existed — a restore MUST NOT resurrect a tombstoned transcript (migration-and-recovery-plan:212).

The checkpoint/handoff envelope's own public field list already excludes anything audio-shaped: logical identity, task, active project, predecessor, portable-work commit, state vector, lifecycle state, provenance, and schema/integrity metadata are the whole list (versioning-and-compatibility:68). VOC-001-R6's prohibition on audio entering a portable artifact therefore adds no new envelope field to police — it closes off a category the envelope never opened.

Independent of transcript retention, a backup that happens to contain a still-live transcript follows the memory plane's own backup retention window (MRP-001 D2, migration-and-recovery-plan:317), not a separate voice-specific one; VOC-001 does not define a second retention clock alongside the memory plane's.

If a diagnostic capture involving audio or a transcript must ever be shared as part of a support export, it passes through RCS-001's redaction serializer and user preview like any other support content — a support export or scrollback export "MUST pass through one serializer that produces a redaction manifest and a user preview before the export can be shared" (RCS-001-R15, renderer-content-security:258). VOC-001 does not define a separate, voice-specific export path that could bypass that gate.

Applied to a restore: a device restored from a pre-deletion backup does not resurrect a transcript the user deleted before that backup's recovery point — the restore process checks the tombstone set for the restore window and excludes the subject it names, "even if the backup being restored predates the tombstone" (migration-and-recovery-plan:212). A retained transcript's tombstone therefore outlives any single backup copy of it, which is the property Retention and deletion exists to guarantee.

## Accessibility and text fallback

Voice is additive: no capability in the product is reachable only by voice, and the typed-or-clicked path remains the reference every voice command mirrors. Spoken output synthesizes only from RCS-001's sanitized, reading-order-preserved text — never raw markup, an unstripped attribute, or a terminal escape sequence (renderer-content-security:236). Every accessibility claim VOC-001's evidence makes is validated by a VP-001 scenario run against a pinned baseline's assistive technology; where that assistive technology is unavailable at execution time, the result is recorded `uncertain`, never skipped and never assumed passing (desktop-stack-verification-plan:140, :233, :249). VP-001's own accessibility scenario, VP-S19, already runs declared journeys with the pinned assistive technology alongside IME input and text rendering (desktop-stack-verification-plan:140); a voice journey is one more declared journey under that same scenario, not a separate evidence track. Adopting an external accessibility standard alongside the pinned-baseline evidence remains a Project Owner call rather than a default this document sets (D7).

Where voice output would read a third-party app's own content, it follows the same rule as core content with no relaxation: RCS-001's inheritance rule, which gives a third-party app no relaxed CSP or navigation rule beyond what the core gets, extends to what voice may read aloud from that app as well (D5; renderer-content-security:230, :260).

Text fallback is symmetrical and total: every voice command has a typed equivalent, and every spoken output has a simultaneous visible text equivalent. When voice is denied, unavailable, or fails, the complete text workflow and recovery control remain, and capture or transmission state is never hidden as a side effect of the failure (target-architecture:266; roadmap:127).

Applied to the problem statement's third scenario above: a local engine crash or a remote timeout renders `voice-unavailable`, not a blank surface — the user keeps typing, clicking, and confirming through the identical typed path a voice command would otherwise have shortcut, because that typed path was never voice's dependency to begin with (target-architecture:266).

## Approval semantics

An utterance is content. It can propose an action; it cannot approve, authorize, elevate, or restore a prior approval — the rule TM-001 states for every other content source applies to voice without a voice-shaped exception (threat-model:63; TM-001-R1, threat-model:238). A destructive, elevating, or publishing action a voice command proposes requires the same typed or clicked confirmation, on an approval surface showing identity evidence and the act-as identity, that any other proposal requires (TM-001-R7, threat-model:244); that confirmation reaches the product only through AEC-001's approvals write path, whose disposition is authorized by "human and local" action, never inferred from feed or spoken contents (adapter-feed-events:154, :162).

Applied to the problem statement's first scenario above: the spoken "yes" is logged as heard text, not executed. The action it named surfaces on the approval surface exactly as if a typed command had proposed it, showing identity evidence and the act-as identity, and only a subsequent typed or clicked confirmation on that surface disposes of it (TM-001-R7, threat-model:244; adapter-feed-events:154, :162).

This is D10's default below, not yet a Project Owner-approved decision: an utterance may open an approval item, never carry the proof that disposes of one (Open decisions, below).

A spoken staleness warning, sync proposal, or other notification is proposal-only, exactly like its typed counterpart: the system proposes, the user disposes (roaming-and-engram-sync:78, :245; threat-model:215, :220; invariant 1, target-architecture:55).

Residual risk exists on this surface exactly as it does on RCS-001's rendered content and TM-001's every other content source: a technically compliant confirmation dialog can still be misread by a rushed user, and a manipulated harness can still misuse authority it legitimately holds through a properly confirmed action (threat-model:222).

VOC-001's job is to make the boundary — that an utterance alone never disposes — hold structurally; it does not claim to make every confirmation impossible to misread.

## Proposed threat-model additions

TM-001 names ambient-voice surfaces as unevaluated and states that each "would need its own attacker-surface addition here before it ships" (threat-model:307). VOC-001 proposes the following as text for TM-001's next revision rather than editing `docs/threat-model.md` directly — the same precedent MRP-001 used when it proposed an answer to the workspace roaming protocol's open generation-record question rather than editing that document itself (migration-and-recovery-plan:247). TM-001's own tables are unchanged by this document.

Proposed additions to the asset table (threat-model:86-100):

| Asset | Where it lives | Why it matters |
| --- | --- | --- |
| Live audio buffer | Device-local, recognition-scoped, never persisted by default | Contains the raw captured utterance before recognition; the shortest-lived and least-reviewed asset this document names |
| Retained transcript | Memory plane, under RSP-001/MRP-001 tombstone and retention rules once opted in | A speech-derived record that may carry the same sensitive detail as any other memory observation |
| Speech-provider credential | OS credential storage; `unprovisioned-secret` when absent | Grants remote recognition or synthesis on the user's behalf; compromise exposes captured audio to an unintended party |

The retained transcript row carries a residual risk mirroring SEC-3's own disclosure (threat-model:192): `<private>` stripping and tombstone deletion remove what they are told to, not what a reviewer failed to tag or a restore failed to exclude, and a transcript already synchronized before deletion has already reached every device and Cloud target the namespace replicates to.

Proposed additions to the actors table (threat-model:104-115):

| ID | Actor | Capabilities | Cannot |
| --- | --- | --- | --- |
| (proposed) | Bystander in physical space | Hears spoken output; may be captured incidentally by an open microphone | Consent on the user's behalf; be assumed absent by the product |
| (proposed) | Remote speech provider | Receives audio or derived data the user opted to transmit; retains it under its own, external policy | Receive audio the user did not opt to transmit; be treated as equivalent in trust to local processing |

Both proposed actors sit in the same class as TM-001's existing A4/A5 network-attacker and remote-service-insider rows (threat-model:109-110): capability bounded by what was actually transmitted, never a source of authority over the device.

Both proposed actor rows also carry a residual risk. The bystander row's is structural, not incidental: push-to-talk (D1) puts the decision to open a capture window solely in the user's hands, and the product has no channel to detect or exclude a bystander within range before that window opens — the mitigations in Consent and visibility narrow the exposure but do not close it. The remote speech provider row's "Cannot receive audio the user did not opt to transmit" claim is enforced by opt-in policy and by the core owning the audio path, not by verified control over a third-party provider SDK's own network behavior; verifying that boundary belongs to a VP-001 scenario to be added, not to this table's capability claim alone.

Proposed addition to the prompt-injection source list (threat-model:213): the spoken-misheard-re-entered round trip — a transcript that is misheard, spoken back, or re-entered as instruction-like content reaching a harness — alongside the existing "terminal or command output that re-enters a harness's own context" source, under the same posture and the same TM-001-R1/TM-001-R2 requirements (threat-model:213, :238-239).

This addition is required before voice ships (threat-model:307).

It is not itself an acceptance of these rows into TM-001 — that acceptance runs through TM-001's own change-control process, the same `Proposed`-revision path any other change to an Accepted or Draft artifact's normative content follows (governance:184-192).

TM-001's actor A7 (a stolen or lost device) and its open decision D6 (stolen-device posture) already cover offline disk access generally (threat-model:112, :275); neither names an audio buffer, a retained transcript, or a speech-provider credential specifically. That silence is exactly the gap the asset-table rows above close — a lost device's offline disk access already reaches a retained transcript today under A7's general capability, but TM-001's own asset table gives a reviewer nothing to check that against until VOC-001's addition is accepted.

Worked example tying the proposed bystander actor to D6's binding default above: a scope with bystander protection opted out still speaks a routine status update, but it never speaks content flagged secret-custody or redaction-flagged, exactly as if a bystander were an unauthenticated reader of the screen rather than a listener — the same custody classes RCS-001's redaction already tracks (renderer-content-security:226) bound the same way for an ear as they do for an eye.

## Product requirements

Each requirement is an acceptance gate with a testable condition. This document's requirements are new — VOC-001-R1 through VOC-001-R20 — and do not continue any other document's numbering (governance:31; desktop-stack-verification-plan:211).

Persisted voice preferences share the interaction-preferences version domain the compatibility policy already names — a monotonic schema integer, unknown-required-field blocking, and safe defaults only for documented optional fields (versioning-and-compatibility:41) — rather than inventing a separate versioning scheme of their own.

The requirements below cluster into five groups by requirement number: consent and visibility (R1-R4), processing and retention (R5-R8), rendering and approval semantics (R9-R12, R15-R16), accessibility and text fallback (R13, R14, R19), and scope and preference boundaries (R17, R18, R20).

| ID | Requirement |
| --- | --- |
| VOC-001-R1 | Capture MUST NOT begin without a device-local, per-feature, revocable consent record distinct from the OS grant, and the disclosure that grants that consent MUST state that any bystander speech captured inside the capture window is captured and cannot be consented to by the product (versioning-and-compatibility:88). |
| VOC-001-R2 | A persistent product-controlled indicator MUST be shown while capturing and a distinct one while transmitting; the two MUST NOT merge (versioning-and-compatibility:88; roadmap:127). |
| VOC-001-R3 | Capture MUST NOT start when the indicator cannot render and MUST stop when the indicator stops being visible; capture MUST also stop immediately when the OS permission, the microphone hardware, or the consent record disappears mid-capture. |
| VOC-001-R4 | An immediate stop control MUST be reachable from every surface while capture or playback is active (versioning-and-compatibility:89). |
| VOC-001-R5 | Local-only processing MUST be the default; remote processing MUST NOT run without a separate, explicit opt-in disclosing provider class and what leaves the device (versioning-and-compatibility:90). |
| VOC-001-R6 | Audio MUST NOT persist beyond the recognition pass by default and MUST NOT enter any portable artifact, backup, or support export (renderer-content-security:258). |
| VOC-001-R7 | A retained transcript MUST be treated as a memory-plane item with `<private>` stripping and tombstone deletion, and its retention default MUST NOT exceed the memory plane's own; the per-scope opt-in that retains a transcript MUST disclose, before it is granted, that the transcript will synchronize like any other memory observation and that only explicitly tagged spans are stripped (roaming-and-engram-sync:85; migration-and-recovery-plan:210-212, :321). |
| VOC-001-R8 | Deleting a transcript MUST write a tombstone that survives backup and restore (migration-and-recovery-plan:212). |
| VOC-001-R9 | Voice output MUST synthesize only from RCS-001's sanitized, reading-order-preserved text (renderer-content-security:236). |
| VOC-001-R10 | A recognized utterance MUST NOT approve, authorize, elevate, or restore a prior approval (threat-model:238). |
| VOC-001-R11 | A destructive, elevating, or publishing action a voice command proposes MUST require the same typed or clicked confirmation, on an approval surface with identity evidence, that any other proposal requires (threat-model:244; adapter-feed-events:162). |
| VOC-001-R12 | Every spoken warning, proposal, or notification MUST be proposal-only and MUST carry a simultaneous visible text equivalent (roaming-and-engram-sync:245, :78). |
| VOC-001-R13 | Every voice command MUST have a typed equivalent, and a capability reachable only by voice MUST NOT exist. |
| VOC-001-R14 | When voice is denied, unavailable, or fails, the complete text workflow and recovery control MUST remain, and capture or transmission state MUST NOT be hidden (target-architecture:266; roadmap:127). |
| VOC-001-R15 | Low confidence or an ambiguous command MUST render `uncertain`, MUST NOT execute, and MUST show the heard text for typed confirmation (target-architecture:62). |
| VOC-001-R16 | A transcript that re-enters as command or context input MUST carry a visible provenance label and MUST be treated as untrusted (threat-model:239; target-architecture:197). |
| VOC-001-R17 | A persisted voice preference MUST carry the interaction-preferences schema integer, MUST block on an unknown required field, and MUST use safe defaults only for documented optional fields (versioning-and-compatibility:41). |
| VOC-001-R18 | The product MUST NOT display or document a recognition-quality, availability, latency, or provider-retention guarantee for voice unless the support matrix states it (versioning-and-compatibility:95). |
| VOC-001-R19 | An accessibility claim MUST be evidenced by a VP-001 scenario against a pinned baseline's assistive technology, and MUST be recorded `uncertain` when that assistive technology is unavailable (desktop-stack-verification-plan:233, :249). |
| VOC-001-R20 | Always-listening and wake-word capture MUST NOT ship before the roadmap gate permitting them and MUST NOT be enabled by default (roadmap:134, :187). |

## Signal mapping

VOC-001 proposes no new public token: every state below reuses a token the compatibility policy's public-state list or support-matrix classifications already define (versioning-and-compatibility:74, :126-127), or a token an existing sibling document already established as public precedent (threat-model:263; renderer-content-security:259).

The live capturing/speaking indicator required in Consent and visibility above is transient UI state, not a persisted state, and needs no token of its own — that is exactly why the problem statement's second scenario, a merged capture/transmission indicator, is a design failure this table cannot detect by itself; the honesty of the indicator lives in Consent and visibility's requirement, not in a signal row. Reuse rather than a dedicated capture-state token is this document's own default for D9, below.

| Condition | VOC-001 state | Public vocabulary | Consequence |
| --- | --- | --- | --- |
| Microphone permission never granted or revoked at OS level | `capture-unavailable` | `failed` | Voice input disabled; typed path unchanged; consent shown as not granted |
| Indicator cannot be rendered | `indicator-unavailable` | `failed` | Capture refused; never started silently (versioning-and-compatibility:88) |
| Microphone hardware removed, OS permission revoked, or consent revoked mid-capture | `capture-interrupted` | `uncertain` | Capture stops immediately, both indicators clear, buffered audio is discarded, and any partial heard text is shown as `uncertain` and never executed |
| Remote speech service unreachable | `service-unreachable` | `offline` | Local-only processing continues where available, otherwise voice is unavailable and text fallback applies |
| Confidence below threshold, command ambiguous, or the transcript's language does not match the expected input | `low-confidence` | `uncertain` | Nothing executes; heard text shown for typed confirmation |
| Remote provider returns a partial or corrupt result | `result-corrupt` | `uncertain` | Nothing executes; no silent retry; heard text, if any, is shown for correction |
| Voice subsystem unavailable at session start | `voice-unavailable` | `uncertain` | Full text and recovery control preserved (target-architecture:266; renderer-content-security:276) |
| Playback device unavailable while spoken output is due | `playback-unavailable` | `failed` | The visible text equivalent remains and is the only output |
| Transcript re-enters as instruction or context input | `transcript-as-content` | `untrusted` | Provenance label shown; every action on it disabled (renderer-content-security:259) |
| Speech-provider credential absent | `provider-secret-missing` | `unprovisioned-secret` | Custody label shown; no in-product secret entry (threat-model:263) |
| Remote processing required, opt-in absent | `remote-not-opted-in` | `failed` | The voice operation fails with a disclosure naming the missing opt-in; the typed path is unchanged |
| Engine/assistive-technology combination identified but untested | `combination-untested` | `detected` | No support claim made; reviewed at GOV-001-R18's cadence (governance:260) |
| Persisted voice preference carries an unknown required field | `preference-schema-unknown` | `failed` | Load blocked (versioning-and-compatibility:41) |
| Pinned assistive technology unavailable for a voice accessibility scenario | `assistive-technology-unavailable` | `uncertain` | Recorded, never skipped (desktop-stack-verification-plan:249) |

## Open decisions

As with every other planned artifact, the Project Owner's recorded approval closes a decision, not the presence of a default (governance:29).

The ten decisions below are distinct from the requirements above: a requirement is already binding as an acceptance gate once VOC-001 itself is accepted, while a decision's default proposal below remains a proposal until the Project Owner records approval of it specifically, per row.

| ID | Question | Options | Default proposal |
| --- | --- | --- | --- |
| D1 | Capture trigger through 1.0 | Push-to-talk only; wake word (local); both | Push-to-talk only through 1.0 (roadmap:134, :187) |
| D2 | Local speech-engine class | OS-provided per platform; a bundled model; either per baseline | OS-provided, recorded as a VP-001 baseline field (desktop-stack-verification-plan:215) |
| D3 | Remote provider policy | Never; opt-in with provider-class disclosure; a named allowlist | Opt-in with provider-class disclosure and an enumerated leaves-the-device list (versioning-and-compatibility:95) |
| D4 | Transcript retention | Never; session-only; a memory observation under the memory window | Session-only default; memory-item retention as an explicit per-scope opt-in, capped by MRP-001's D6 retention default (migration-and-recovery-plan:321) |
| D5 | Voice output reading third-party app content | Never; declared classes; the same rule as core content | Same rule as core content, no relaxation (renderer-content-security:230, :260) |
| D6 | Bystander protection | None; a headphones heuristic; a per-scope opt-out; summary-only output; a per-scope capture pause | Per-scope opt-out of spoken output plus a per-scope capture pause, both reachable from the stop control; never speaking secret-custody or redaction-flagged content; not yet in TM-001 (see Proposed threat-model additions) |
| D7 | Accessibility baseline | Pinned assistive technology per VP-001 baseline; an external standard; both | Pinned assistive-technology baseline exercised by VP-001's VP-S19 scenario; an external standard is a Project Owner call (desktop-stack-verification-plan:140) |
| D8 | Voice before 1.0 | Beta, per the roadmap; post-1.0 only; output-only at Beta | Beta, per roadmap:119, gated by the TM-001 additions above and GOV-001-R4 advice (governance:246) |
| D9 | New public token for voice states | Reuse existing tokens; add a capture-state token | Reuse existing tokens (see Signal mapping above) |
| D10 | Utterance reaching the AEC-001 approvals path | Never; only as a proposal opening an approval item a human disposes; as a disposition carrying its own proof | Never as a disposition; may only open an item a human disposes through a typed or clicked act (adapter-feed-events:162) |

## Acceptance evidence and follow-up

A conformance check MUST verify each of the following before VOC-001's guarantees are treated as met:

- A consent record for capture exists, is device-local, and is distinct from the OS microphone grant (VOC-001-R1).
- The capture indicator and the transmission indicator never merge into one signal, on any tested surface (VOC-001-R2).
- Capture is refused, not silently skipped, when the indicator cannot render (VOC-001-R3).
- The stop control is reachable from every surface where capture or playback is active (VOC-001-R4).
- Audio is absent from backups, exports, and every other portable artifact produced during or after a capture session (VOC-001-R6).
- A deleted transcript's tombstone survives a backup-and-restore cycle (VOC-001-R8).
- A spoken utterance, replayed against the approval surface, cannot open or dispose of an approval by itself (VOC-001-R10, VOC-001-R11).
- Every voice command exercised in test has a typed equivalent that produces the same outcome (VOC-001-R13).
- A VP-001 accessibility scenario for voice is executed against a pinned baseline, or recorded `uncertain` when the pinned assistive technology is unavailable (VOC-001-R19).

Debt, not drafted here.

- This document does not draft the proposed TM-001 additions above into `docs/threat-model.md` itself — that acceptance is TM-001's own (threat-model:307).
- It does not obtain the Security Reviewer and Legal Counsel advice GOV-001-R4 requires for the Alpha → Beta gate; both roles are Unassigned today (governance:126-127, :246).
- It does not choose a speech engine or a remote provider — those are implementation decisions downstream of D2 and D3, below.
- It does not specify the capture and transmission indicators' visual presentation, which remains [context-orb.md](context-orb.md)'s to design.
- The pointer from the target architecture's planned-assurance-artifacts row (target-architecture:283) to this document is deferred to that table's first `Proposed` revision carrying a signed acceptance tag (GOV-001-R16; governance:192, :258, :288), because the table is frozen and changes only that way.

## Related contracts

- [Target architecture](target-architecture.md) — the VOC-001 row in planned assurance artifacts (target-architecture:283), invariants 1, 8, and 9, and the required failure state this document's degraded-voice rows respect (target-architecture:55, :62-63, :266).
- [Renderer content-security contract](renderer-content-security.md) (RCS-001) — the sanitized, reading-order-preserved text voice output reads from (renderer-content-security:236, R18 :261); drafted, acceptance pending.
- [Versioning and compatibility](versioning-and-compatibility.md) — the stable voice behavior list, the persisted-preferences schema rule, and the public state vocabulary this document's Signal mapping reuses (versioning-and-compatibility:86-95, :41, :74).
- [Product roadmap](roadmap.md) — the Beta scope and exit criteria, and the Alpha → Beta and Beta → 1.0 gates this document's evidence feeds (roadmap:119, :126-127, :134, :187).
- [Threat model](threat-model.md) (TM-001) — the binding principle, TM-001-R1 and TM-001-R7, and the ambient-voice debt this document proposes an answer to (threat-model:63, :238, :244, :307); drafted, acceptance pending.
- [Adapter feed event schema](adapter-feed-events.md) (AEC-001 feed profile) — the approvals write path a voice-proposed disposition reaches only through a human, local act (adapter-feed-events:150-162); drafted, acceptance pending.
- [Workspace roaming and Engram sync protocol](roaming-and-engram-sync.md) (RSP-001 memory profile) — proposal-only spoken staleness warnings, and `<private>` stripping for a retained transcript (roaming-and-engram-sync:78, :85, :245); drafted, acceptance pending.
- [Migration and recovery plan](migration-and-recovery-plan.md) (MRP-001) — tombstone kinds, propagation, and the retention default this document's transcript retention is capped by (migration-and-recovery-plan:203-212, :321); drafted, acceptance pending.
- [Governance](governance.md) (GOV-001) — the Security Reviewer and Legal Counsel advice the Alpha → Beta gate requires, and the frozen planned-assurance-artifacts table this document's own registry entry sits in (governance:192, :209, :246); drafted, acceptance pending.
- [Desktop stack verification plan](desktop-stack-verification-plan.md) (VP-001) — the accessibility scenario and pinned-baseline evidence this document's accessibility claims depend on (desktop-stack-verification-plan:140, :233, :249); drafted, acceptance pending.
- [ADR-0002: Desktop technology stack](adr/0002-desktop-technology-stack.md) — the Radix Primitives baseline and the WebView accessibility matrix VP-001 exercises (ADR-0002:30, :120, :148).
- [Context Orb specification](context-orb.md) — the presentation of the capture and transmission indicators this document requires but does not design.
- [ADR convention](adr/README.md) — the status lifecycle VOC-001 follows once it leaves Draft.

## References

- [Web Content Accessibility Guidelines (WCAG) 2.2](https://www.w3.org/TR/WCAG22/) — the general text-equivalence and timing-adjustable guidance the Accessibility and text fallback section draws on; VOC-001 does not itself claim WCAG conformance.
