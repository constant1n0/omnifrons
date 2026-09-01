# Omnifrons Product Roadmap

**Document role:** Product maturity, sequencing, and promotion evidence  
**Status:** Draft  
**Normative force:** Non-binding; stage requirements are proposed gates, not current guarantees  
**Accountable role:** Project Maintainer  
**Named person:** Unassigned  
**Approver role:** Project Owner  
**Approver named person:** Unassigned  
**Effective date:** None  
**Supersedes:** None  
**Name status:** Selected public name; preliminary screening complete, formal trademark clearance pending

## Authority and reading order

The [target architecture](target-architecture.md) owns boundaries, invariants, terminology, and the registry of planned assurance artifacts. [Versioning and compatibility](versioning-and-compatibility.md) owns public promises. ADRs own decision rationale and approval state. This roadmap owns sequence and promotion evidence; it does not approve an ADR or protocol.

All current artifacts are drafts or proposals. If they conflict with each other or with unreconciled earlier documents, implementation of the disputed behavior is blocked until the conflict is recorded and resolved.

## Maturity, channel, and SemVer mapping

Product maturity, distribution channel, and SemVer are separate axes.

| Maturity stage | Normal channel | SemVer form | Meaning |
| --- | --- | --- | --- |
| Pre-alpha | `nightly` or restricted `alpha` | `0.MINOR.PATCH-pre.N` or isolated nightly build | Architecture and recovery proofs; disposable or backed-up state only. |
| Alpha | `alpha` | `0.MINOR.PATCH` or `0.MINOR.PATCH-alpha.N` | Reproducible internal and technical-pilot use; breaking changes remain possible. |
| Beta | `beta` | `1.0.0-beta.N` | External validation against the intended 1.0 contracts. |
| 1.0 | `stable` | `1.0.0` | First supported compatibility contract. |
| Post-1.0 | `stable`, with optional preview features | `1.MINOR.PATCH` | Backward-compatible evolution; 2.0 only for a necessary contract break. |

A stage changes only after its evidence gate passes. Publishing a build on a channel does not promote product maturity.

## Roadmap principles

- Prove continuity and recovery before adding provider breadth.
- Prefer structured harness protocols; disclose PTY as degraded operation.
- Use `Workspace Git roaming` for portable workspace history and reserve `Engram Git Sync profile` for Engram memory synchronization.
- Keep Engram Cloud optional and give each data plane one observed active sync authority.
- Treat pending, stale, forked, uncertain, and conflicted states as first-class outcomes.
- Add a public extension surface only when its support cost and security model are evidenced.

## Pre-alpha

### Question and risk retired

Can a logical agent resume useful work across two different harnesses and two machines without relying on private vendor sessions or silently losing in-progress work?

### Scope

- Minimal text interaction.
- Two structurally different built-in harness adapters.
- Explicit workspace and active-project scopes with declared enforcement level.
- Checkpoint-and-restart switching with quiesce or an explicit uncertain state.
- Initial portable-work contract: a user-previewed, user-approved handoff commit on an Omnifrons-managed ephemeral Git ref.
- Workspace Git roaming plus one selected Engram synchronization profile.
- An always-local Markdown vault plus a separate user-selected local/on-demand heavy-asset tier; the initial supported Linux blob backend is Proton Drive CLI.
- Drafts and executable proofs for the planned handoff transaction and roaming/sync protocols registered in the target architecture.
- The desktop technology spike governed by ADR-0002 and its planned verification plan.

### Exit criteria

- The portable-work preview accounts for tracked changes, deletions, selected untracked files, and every excluded or blocked class.
- A disposable handoff commit is published and consumed without changing the user's current branch, index, or intended history.
- A task resumes through a validated state vector on a second machine and through a different built-in adapter.
- Startup diagnostics block if any Markdown file is cloud-only or unreadable; Orb hydration changes a heavy asset to local only after its download is verified.
- Interruptions at prepare, publish, claim, import, switch, and cleanup yield recoverable or explicitly uncertain states, never false completion.
- Dirty submodules, nested repositories, unsupported LFS/binary cases, unresolved secrets, and divergent Git state block automatic handoff.
- ADR-0002 records an accept, remediate, or supersede outcome from reproducible Windows, macOS, and Linux evidence.

### Explicit non-goals

- Voice, production installers, broad adapter coverage, public adapter SDK, or public headless CLI.
- Concurrent writers or an offline distributed lock.
- Automatic semantic conflict resolution.
- Production update, migration, or sandbox guarantees.

## Alpha

### Question and risk retired

Can the proven contract be installed and exercised reproducibly on the supported Windows, macOS, and Linux matrix while reporting the limits of its control truthfully?

### Scope

- Coherent text conversation and control panel.
- Built-in adapter capability negotiation, approvals, cancellation, and process supervision.
- Device-local executable approval and re-probe before launch.
- Installation diagnostics for Git, Engram, vault, secure storage, and selected harnesses.
- Workspace Git roaming with one active writer and explicit fork detection.
- Engram Git Sync profile by default; Engram Cloud profile as an optional alternative.
- Crash recovery, backups, migration fixtures, and mediated access to knowledge and delivery artifacts.
- Renderer content-security baseline and safe support-bundle behavior.

### Exit criteria

- Clean installations complete the supported text, switch, roaming, and recovery journeys on every declared matrix entry.
- Scope is labelled `sandbox-enforced`, `harness-enforced`, or `advisory`; unsupported containment is never implied.
- Runtime Engram configuration is checked against the selected authority before handoff.
- Unsupported harness versions, changed executable identities, insecure secret custody, and conflict states produce actionable blocks.
- Persisted portable formats are versioned and migration fixtures cover the declared upgrade graph.

### Explicit non-goals

- Stable public extension API, public headless CLI, multiwriter synchronization, enterprise fleet controls, or hosted inference.
- Support for combinations classified only as detected or preview.

## Beta

### Question and risk retired

Can external pilot users install, understand, update, recover, and use text and voice without developer intervention or hidden security assumptions?

### Scope

- Signed beta installers after the update-trust architecture is accepted.
- Controlled migrations and recovery under the published upgrade graph.
- Guided Git, handoff, Engram, and secret-provisioning recovery.
- Voice input/output with explicit consent, visible capture/transmission state, retention policy, and text fallback.
- Threat-model review, renderer-content review, accessibility validation, and external pilots.
- Engram Cloud profile tested as optional; no dual-authority mode.
- Bounded support matrix and privacy-aware diagnostics.

### Exit criteria

- External users complete install, text/voice work, harness switch, device handoff, and recovery from public pilot documentation.
- Voice failure preserves the complete text workflow and never hides capture or transmission state.
- Update, rollback-before-mutation, Git conflict, forked writer, stale memory, offline, and corrupted-state drills pass.
- No release-blocking security or data-loss finding remains open.
- The legal-readiness gate below permits the intended pilot distribution and service description.

### Explicit non-goals

- Unreviewed community adapters, organization fleet management, compliance certification, multiwriter operation, native mobile, or always-listening voice.

## Version 1.0

### Question and risk retired

Can the project sustain a bounded compatibility, security, migration, recovery, and support contract?

### Scope

- Stable built-in adapter behavior and checkpoint interoperability; no public adapter SDK promise.
- Stable portable configuration, workspace namespace, checkpoint/handoff envelope, sync states, and persisted interaction preferences.
- Supported text and voice behavior defined by the compatibility and voice contracts.
- Published support matrix, migration graph, rollback boundary, update trust, and disaster-recovery procedures.
- Signed releases, accepted license/distribution model, cleared product name, and reconciled architecture baseline.

### Exit criteria

- Every 1.0 public surface has an owner, version domain, migration rule, deprecation rule, and recovery path.
- Upgrade and rollback drills pass for every edge in the declared supported upgrade graph on each supported OS family.
- Built-in adapters pass the internal conformance suite for their declared harness ranges.
- Text and voice meet the published accessibility, privacy, consent, retention, and fallback contract.
- License, naming, governance, threat-model, update-trust, migration/recovery, and support evidence is approved and retained.
- A clean-room installation and paid-deployment rehearsal complete from the distributable artifacts.

### Explicit non-goals

- Agent runtime, model hosting, vendor-session transfer, universal compatibility, public adapter SDK, public headless CLI, or concurrent writers.

## Legal-readiness gate

ADR-0001 is Accepted and the canonical [Apache License 2.0](../LICENSE) text is committed. Public source distribution is permitted under that license. Dependency, bundled-asset, contribution, trademark, binary-distribution, and service-model review remain release gates; the source license does not clear those separate risks.

Any Engram Cloud service must be described accurately as customer-selected setup/support or self-hosted operation unless separate resale, co-branding, or managed-service authority is confirmed.

## Promotion and approval gates

| Promotion | Required evidence | Accountable role | Approver |
| --- | --- | --- | --- |
| Pre-alpha -> Alpha | Portable-work proof, protocol drafts, desktop spike result, open-risk disposition | Project Maintainer | Project Owner |
| Alpha -> Beta | Threat model, renderer security, update trust, migration/recovery plan, legal distribution readiness | Project Maintainer | Project Owner after Security Reviewer and Legal Counsel advice |
| Beta -> 1.0 | Accepted public contracts, naming clearance, support matrix, external-pilot and recovery evidence | Project Maintainer | Project Owner |
| 1.x breaking proposal | Compatibility impact and migration evidence | Decision Owner | Project Owner through a major-version ADR |

Exceptions require a recorded owner, approver, scope, rationale, expiry, evidence, and rollback plan. An exception cannot relabel uncertain or unsupported behavior as guaranteed.

## Post-1.0 evidence horizons

- Publish an adapter SDK only when repeated demand, isolation, signing, compatibility, and maintenance evidence justify it.
- Publish a headless CLI only when a defined automation/recovery use case and security contract justify it.
- Design multiwriter roaming only when single-writer handoff is a demonstrated customer barrier.
- Add organizational policy, audit, identity, or fleet operations only from repeated paid demand.
- Advance Orb projections only when real vault size and navigation evidence justify them.
- Explore mobile or ambient voice only when value exceeds privacy, platform, and support cost.

**Version 2.0 exists only when a necessary change breaks an established 1.x public contract.**

## Promotion evidence

Promotion records include exact conformance results, platform builds, recovery drills, migration fixtures, security findings, pilot evidence, support burden, approved exceptions, approver identity, and decision date. A feature that cannot be supported or recovered does not qualify a stage for promotion.
