# Update Trust Architecture

**Document role:** Update trust architecture: trust roots and online roles, release metadata, freshness, anti-rollback, platform signing, compromise recovery, and app bundle signing (UTA-001)  
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

This document drafts the planned update trust architecture (UTA-001): the trust-root model — an offline root role signing with a threshold, and the online roles it vouches for — release metadata and what it binds, client verification, freshness and anti-rollback at the binary level, key rotation and revocation, compromise response and key-loss recovery, platform signing and notarization as a necessary-but-insufficient layer, channel update policy, offline and air-gapped update bundles, third-party app and module bundle signing, and the acceptance evidence for the roadmap's Alpha → Beta update-trust gate. The [target architecture](target-architecture.md) governs any conflict.

This draft does not redefine work already owned elsewhere:

- **AEC-001** ([adapter-feed-events.md](adapter-feed-events.md)) owns producer identity: an adapter instance's own key, paired trust-on-first-use, and rotated by a signed announcement on the feed itself (`producer.key_rotated`). UTA-001's online roles borrow the same rotation *shape* — a signed announcement with an overlap window — but never the same keys; a producer key proves which adapter instance emitted an event, not that a release is trustworthy.
- **HTP-001** ([handoff-transaction-protocol.md](handoff-transaction-protocol.md)) and the **RSP-001 core** ([workspace-roaming-protocol.md](workspace-roaming-protocol.md)) own device identity keys and the signatures on handoff envelopes and claim/release records. A device proving it is the device it claims to be is a different guarantee from a release proving it came from this product's own trust roots.
- **MRP-001** ([migration-and-recovery-plan.md](migration-and-recovery-plan.md)) owns the signed backup manifest — a different signature over different content — and owns binary retention (MRP-001-R9), the downgrade window (MRP-001 D5), and the mid-migration marker (MRP-001-R21). UTA-001 decides whether a binary is trusted; MRP-001 decides when a trusted binary may run a migration. Neither substitutes for the other.
- **TM-001** ([threat-model.md](threat-model.md)) owns the attacker model this document's roles are evaluated against, and owns harness and adapter binary signing directly: threat HAR-3's re-approval mitigation and open decision D10 (a platform code signature required where the platform supports it, an unsigned binary still approvable with a stronger warning) are TM-001's own, for actor A10's compromised distribution channel. UTA-001 covers only the product's own updates and the app/module bundles it installs, never a harness or adapter's own distribution channel.
- **GOV-001** owns named roles and approval authority, including which named holders sit behind the root role's threshold; UTA-001 assumes that assignment without redefining it.
- **[ADR-0004](adr/0004-open-platform-and-custom-apps.md)** owns the distribution and monetization decision and states that custom-app packaging and signing ride on this artifact; a public app SDK or a commercial-module decision remains a separate future ADR that UTA-001 does not make.

## Purpose and scope

UTA-001 governs **whether an update or an installed app bundle may be trusted before it runs**: which roles vouch for a release, what proves the metadata describing it is current rather than stale or replayed, when a binary-level downgrade is refused, what happens when a role's key is rotated, revoked, or lost, why a platform code signature is necessary but never sufficient by itself, and how a third-party app's own code earns the same trust.

In scope:

- the trust-root model: an offline root role with a signing threshold, and the online roles — release, freshness, apps — it vouches for;
- release metadata: the fields it binds per channel, and the separate, short-lived freshness statement;
- client verification: the ordered checklist a device runs before any update is applied;
- freshness and anti-rollback: per-channel expiry, monotonic counters, and downgrade refusal at the binary level;
- key rotation, revocation, compromise response, and key-loss recovery, including root re-pinning;
- platform signing and notarization per OS, as a layer this document requires but does not treat as sufficient alone;
- channel update policy: manual availability, automatic policy per channel, and channel-switch confirmation;
- offline and air-gapped update bundles;
- third-party app and module bundle signing: author keys, pinning, and revocation;
- the acceptance evidence that satisfies the roadmap's Alpha → Beta update-trust gate.

Out of scope:

- producer identity and its rotation-by-signed-announcement pattern — owned by AEC-001;
- device identity keys and handoff/claim signatures — owned by HTP-001 and the RSP-001 core;
- the signed backup manifest, binary retention, the downgrade window, and the mid-migration marker — owned by MRP-001;
- the attacker model, and harness/adapter binary signing and re-approval specifically — owned by TM-001 (HAR-3; actor A10; open decision D10);
- named roles and approval authority — owned by GOV-001;
- any public app SDK or commercial-module decision — a separate future ADR per ADR-0004's own follow-up.

This document is itself named evidence for the roadmap's Alpha → Beta promotion gate, alongside the threat model, renderer-content review, migration/recovery plan, and legal distribution readiness — none of the other gate items substitute for it, and it does not substitute for them. The roadmap's Beta scope names the consequence directly: signed beta installers exist only after this architecture is accepted, and the 1.0 exit criteria carry a published update trust posture as one of the license, naming, governance, migration, and support items that must each be approved and retained before that promotion.

## Problem statement

A release server is compromised for an afternoon. The attacker replaces the newest installer with a signed-looking one. A platform code signature alone does not stop this if the signing key lives on the same server the attacker just reached — the signature proves the binary matches what was uploaded, not that what was uploaded should be trusted.

A user is offered "version 3.2." It is really the year-old 3.2 that carries a known flaw, served by a mirror that never updated. Nothing about a valid signature says how old the signed thing is; without a freshness check, an attacker does not need to forge anything — replaying an old, validly signed release is enough.

A key holder leaves the project. The remaining team can no longer sign a release, and cannot revoke the departed holder's key either, because both actions needed the same single key that walked out the door.

Each of the three failures traces to the same gap: a valid signature answers "did this come from someone who once held a key," never "should this specific, current thing be trusted right now." Closing that gap is this document's scope.

**An update is trusted only when an offline root vouches for the roles that signed it — and, for a curated app, for the author key itself, never an online key acting alone — the metadata is fresh and never older than what the device has already seen, and the platform's own signature agrees; any of the three missing blocks automatic mutation and tells the user why.**

## Definitions

Terms already defined by another artifact are used here with that artifact's meaning and are not redefined; only terms this document introduces, or narrows for its own purpose, appear below.

| Term | Meaning |
| --- | --- |
| Root role | The offline threshold-signing role that vouches for every online role's keys and thresholds; never present on a connected system. |
| Root metadata | The root role's own signed listing of every role's keys, thresholds, and the root metadata's own expiry. |
| Root chain | The sequence of root metadata versions a client walks from its pinned version to the current one, each new version signed by both the outgoing and the incoming threshold. |
| Online role | Collective term for the release role, the freshness role, and the apps role — each an online, connected signer the root role vouches for. |
| Threshold signing | A signature valid only once a minimum count of a role's independent key holders have each signed, so no single held key is sufficient by itself. |
| Release metadata | The release role's signed, per-channel binding of version, artifact digests, minimum supported version, and a monotonic metadata version. |
| Freshness statement | The freshness role's short-lived signed pointer naming the current release metadata version, the current root metadata version, and its own expiry; separated from release signing so a leaked release role key alone cannot be replayed indefinitely, and carrying the root metadata version so a frozen root is detectable while the freshness role is still reachable. |
| Monotonic counter | The per-channel, ever-increasing metadata version a device compares against the last one it saw, so a lower or equal version is rejected as stale or replayed regardless of signature validity. |
| Anti-rollback | The refusal to accept a binary-level downgrade unless the exact version pair carries a signed downgrade allowance, distinct from MRP-001's own downgrade window, which this document's allowance operates inside. |
| Platform signature | The OS-level code signature or notarization a release also carries; necessary because the OS itself enforces it, never sufficient because it says nothing about freshness or root provenance. |
| Update bundle | The offline-importable artifact, release metadata, freshness statement, and root chain, verified identically to a networked update. |
| Author key | A third-party app or module author's own signing key: for a non-curated app, pinned to the app on first install (trust on first use); for a curated app, entered into the registry the root role signs, offline, at each registry publication. |
| Root re-pinning | The explicit, user-confirmed replacement of a device's pinned root metadata, used only after root compromise or a loss of holders below threshold; never automatic. |
| Downgrade allowance | A signed statement, scoped to one exact version pair, inside release metadata, that permits a binary-level downgrade the anti-rollback rule would otherwise refuse. |
| Trust on first use | The pairing pattern this document borrows from AEC-001: the first key a device sees for a given app author is accepted and shown to the user, and any later mismatch is flagged rather than silently accepted. |
| Compromise advisory | The signed statement, carried in release metadata, that discloses an online key compromise and the rotation that followed it, distinct from a general security advisory outside this document's scope. |
| Same major version | The scope MRP-001's downgrade window (D5) currently allows; UTA-001's signed downgrade allowance operates only inside whatever window is in force there, never beyond it. |

## Trust roots and online roles

Trust does not start with a signature on a release; it starts with a root role that vouches, offline, for every other role, and for a curated app's own author-key registration. Everything else in this document — freshness, anti-rollback, and revocation — is a role the root role names and can replace, never a role that can name itself; an online role may narrow trust the root role already granted (revoke), but never widen it (register).

| Role | Keys | Custody | Signs | Rotation |
| --- | --- | --- | --- | --- |
| Root role | A threshold set, default 2 of 3 (open decision D1) | Offline; never on any connected system; held by named holders GOV-001 names; hardware-backed by default (D7) | Root metadata: every role's keys and thresholds, the root metadata's own expiry (open decision D11), and the curated third-party app author-key registry at each registry publication | A new root metadata version, signed by both the outgoing and the incoming threshold, forming the root chain a client walks; refreshed before its own expiry per D11's cadence |
| Release role | Per-channel online signing keys | Release infrastructure, with access logging; never in the repository, a device backup, or portable state | Release metadata per channel | Root-signed: a new root metadata version lists the replacement keys, with an overlap window during which a client accepts either |
| Freshness role | Short-lived online signing keys | Release infrastructure, rotated frequently | The freshness statement: current release metadata version, current root metadata version, and its own expiry | Same root-signed pattern as the release role, at a shorter interval matching the keys' own short life |
| Apps role | An online revocation-signing key | Release infrastructure | Revocations against the curated third-party app author-key registry — never a new registration, which only the root role signs | Same root-signed pattern |

The product ships the initial root metadata pinned at install; every later root metadata version is reached only by walking the chain from that pin, never by trusting a new root out of band without the explicit re-pinning confirmation described below.

No role key of any kind is ever stored in the repository, in a device backup, or in portable state — the same custody discipline MRP-001-R1 already applies to a backup's forbidden payloads and HTP-001-R18 already applies to a handoff envelope, extended here to every role this document defines. A root role key additionally never touches a connected system at all; the online roles' keys live only in release infrastructure with access logging.

## Release metadata

| Field | Meaning |
| --- | --- |
| Version | Semantic version of the release. |
| Channel | `nightly`, `alpha`, `beta`, or `stable`, per the compatibility policy's release channels. |
| Per-OS artifact digest and size | One entry per supported OS/architecture pair. |
| Minimum supported version | The anti-rollback floor: no client-side downgrade is verified below this version without a signed allowance. |
| Release metadata version | A monotonic counter, compared against the last version a device has seen on that channel. |
| Expiry | The UTC time after which this release metadata is no longer current on its own. |
| Migration notes pointer | A reference into MRP-001's migration journal domains for this version's migrations. |
| Signature(s) | By the release role, over the canonical serialization of every field above. |

The freshness statement is a separate, shorter document, signed by the freshness role rather than the release role: the release metadata version it names, the current root metadata version it names, that version's expiry, and the freshness role's own signature. A device checks both signatures independently; neither substitutes for the other. Naming the current root metadata version gives a client a second, faster way to notice a stale root than waiting on the root metadata's own expiry (open decision D11) — see Freshness and anti-rollback.

The migration notes pointer is release metadata's only link into MRP-001: it names which of that release's migration journal domains a client should expect, so a client can show a preview before the pre-upgrade backup and migration actually run. UTA-001 verifies the pointer's signature as part of release metadata; MRP-001 owns everything the pointer refers to.

## Client verification

Order matters here as much as the individual checks: a later step's result is meaningless if an earlier one has already failed, so a client never evaluates step 4 against an artifact whose release metadata in step 3 did not verify, and never applies a platform signature check to paper over a broken root chain in step 1.

A device runs this checklist, in order, before applying any update:

1. **Root chain.** Walk from the pinned root metadata to the current version, each step signed by both the outgoing and the incoming threshold, and check the pinned root metadata's own expiry (open decision D11). A broken chain renders the update `failed` and offers only the manual re-pinning path below; an expired pinned root renders the update `uncertain`, blocks automatic updates, and requires obtaining a newer root chain before any update — automatic or manual — proceeds.
2. **Freshness statement.** Signed by a current freshness role key, not expired, naming a release metadata version no lower than the last one this device has seen on this channel (the monotonic counter), and naming a root metadata version matching what this device has pinned. A root metadata version mismatch here is an early staleness signal, catching a frozen root while the freshness role itself is still reachable and honest — a faster check than waiting on step 1's own expiry, which is the backstop for the case where both are frozen together. A failure here is `failed`; an expired-but-otherwise-valid statement the device cannot reach a live freshness role to refresh is `uncertain`.
3. **Release metadata.** Signed by a current release role key, channel matches the device's own channel, and version and metadata version are not lower than last seen unless a signed downgrade allowance names this exact version pair.
4. **Artifact.** Digest and size match the entry the release metadata carries for this device's OS and architecture.
5. **Platform signature.** Valid platform code signature or notarization for the OS, checked in addition to, never instead of, steps 1–4.
6. **Hand off to MRP-001.** Only after every step above passes does the update proceed to the migration and recovery plan's own gates: the verified pre-upgrade backup (MRP-001-R5), the mid-migration marker (MRP-001-R21), and binary retention (MRP-001-R9).

Any single failure blocks automatic mutation, renders the update `failed` (or `uncertain` for an unreachable expiry check), and shows the user the reason. A valid platform signature alone never suffices to skip steps 1–4.

Applied to the problem statement's first scenario: a compromised release server signing its own replacement installer still fails step 3 unless it also holds a current release role key, and fails step 1 outright if it cannot also produce a valid root chain — neither of which a compromised server alone grants it, because both live outside that server under this document's custody rules.

## Freshness and anti-rollback

Freshness statement expiry is per channel (open decision D2): `stable` 7 days, `beta` 3 days, `nightly` 24 hours — shorter on a less stable channel, where staleness is a bigger risk relative to how often the channel expects to be checked. The monotonic per-channel counter resists an attacker who can manipulate the device's own clock but cannot forge a higher counter value without a valid role key.

An expired freshness statement that a device cannot refresh — no reachable freshness role — is `uncertain`, not `failed`: automatic updates stay blocked, and the device offers a manual check instead of silently continuing on stale trust. The distinction matters because `failed` and `uncertain` invite different next actions — a `failed` update is rejected outright, while an `uncertain` one is an unproven assumption the target architecture's invariant 2 already requires the product to expose rather than assume away.

Root metadata freeze is a related but distinct staleness risk: an attacker who cannot forge a new root can still win by preventing a client from ever learning one exists, replaying an old-but-validly-chained root indefinitely. Two independent checks catch this. Every freshness statement names the current root metadata version, so a client compares it against its own pinned version at every check (client verification step 2) — catching a frozen root while the freshness role itself remains reachable and honest. Root metadata also carries its own expiry (open decision D11), checked directly against the root metadata itself (client verification step 1) independent of any other role — catching the case where an attacker has frozen the freshness role too, replaying both together. An expired pinned root renders the update `uncertain`, blocks automatic updates, and requires obtaining a newer root chain before any update — automatic or manual — proceeds.

Downgrade at the binary level is refused unless both conditions hold: the target version falls within the downgrade window MRP-001 currently allows (MRP-001 D5), and a signed downgrade allowance for the exact version pair exists in release metadata (open decision D6). Neither condition alone is sufficient.

Rolling back to the binary MRP-001 retains under its own binary-retention requirement (MRP-001-R9) is not a downgrade in this sense and needs no signed allowance: that retained binary's release metadata was valid when it was installed and stays pinned with it, so restoring it restores a state this document already verified rather than reaching backward past it. That exemption covers only R7's downgrade allowance, never R9's revocation check: launching the retained binary re-evaluates whether the release role key that signed it is still valid under the *current* root metadata, not the metadata pinned with the binary at install time. A retained binary whose signing key has since been revoked renders `failed` at launch, with guidance to obtain a verified build out of band rather than an instruction to retry the same binary.

## Key rotation, revocation, compromise, and key loss

**Online key rotation.** A new root metadata version, root-signed, lists the replacement keys for the affected role. Clients accept both the outgoing and the incoming key during the stated overlap window — the same shape AEC-001 uses for a producer's own key rotation, without sharing keys with it.

**Revocation.** Root-signed, the same way as rotation. Anything signed by a since-revoked role key — root, release, freshness, apps, or packaging — dated after the root metadata version that revoked it, renders `failed`.

**Online key compromise.** The response is an emergency root-signed rotation plus revocation of the compromised key, a signed advisory carried in release metadata, and a product-shown notice — "update trust rotated; verify before continuing" — that persists until the device completes its next fully verified release check.

**Root compromise, or loss below threshold.** Re-pinning happens out of band: a new product build carries the new root metadata, distributed through the normal platform-signed channels, and the device shows the new root's fingerprint with an instruction to compare it against at least one of the out-of-band sources this document names (open decision D12): the project's own security page over HTTPS, a signed release note or tag in the public source repository, or the platform store listing where one exists. Re-pinning never happens silently, and no automatic update path re-pins a root on its own.

**Key loss.** A key-loss recovery drill — losing one root role share while the remaining threshold still signs successfully — is part of this document's acceptance evidence, alongside the unavailable-key behavior the compatibility policy's rollback and recovery qualification already requires of every supported release.

Applied to the problem statement's third scenario: a departed key holder taking one root role share out of the door degrades the threshold but does not by itself block signing, because the default threshold (2 of 3, open decision D1) tolerates exactly one lost share; only losing a second share forces the out-of-band re-pinning path above, and revoking the departed holder's own share never depends on that holder's cooperation, because revocation is root-signed by the *remaining* threshold, not by the key being revoked.

## Platform signing and notarization

| OS | Mechanism | Necessary because |
| --- | --- | --- |
| macOS | Notarization plus a Developer ID signature | The OS itself refuses to launch an unnotarized build without an explicit override |
| Windows | Authenticode signature | SmartScreen and driver-signing enforcement key off this signature |
| Linux | A signed package or a detached AppImage signature (per open decision D10's packaging targets), signed by the release role key or a dedicated packaging role key listed in root metadata | Distribution-specific package managers and update tooling check this signature where it exists; the verifying public key travels inside root metadata, never through a separate channel |

Platform verification itself splits into an offline and an online check. A stapled notarization ticket (macOS) or an embedded certificate chain (Windows Authenticode, a Linux package or detached signature) is verified entirely offline, without reaching the network. Confirming the signing certificate has not since been revoked is the online half, checked against the platform's own revocation-list service. When that revocation-list check cannot reach the network, the platform step renders `uncertain` rather than being skipped or assumed passing: automatic updates stay blocked, and a manual update may proceed only with that offline-only status disclosed to the user.

Platform signing is necessary but never sufficient: it proves the OS accepted the binary as unmodified from what was signed, not that the signed thing itself should be trusted, was current, or came from this product's own release role. The role metadata this document defines is the sufficiency layer platform signing does not provide. The planned desktop verification plan (VP-001) records the per-OS evidence for both layers separately — a passed platform-signature check and a passed role-metadata check are two distinct rows in that evidence, never collapsed into one "signed" outcome — and ADR-0002 states plainly that its own update-signing tests "do not approve automatic updates until the separate update-trust architecture is accepted."

## Channels and update policy

Manual update, with the full verification checklist above, is always available regardless of channel or automatic policy.

Automatic updates on `beta` and `stable` remain blocked — the rule the target architecture and the compatibility policy already state — until UTA-001 is accepted; once accepted, automatic updates on those channels run the full checklist and MRP-001's gates before applying. Automatic updates on `nightly` and `alpha` run only with the user's explicit opt-in, never by default, reflecting those channels' own weaker state-preservation rules under the compatibility policy.

A channel switch requires explicit confirmation and never skips verification: switching channels is treated as a fresh update decision, not an extension of trust already granted on the previous channel. Automatic policy governs only whether the client verification checklist runs without being asked; every channel, automatic or manual, runs the same checklist and the same MRP-001 gates once triggered — a more permissive channel never means a shorter checklist.

## Offline and air-gapped updates

An update bundle — the artifact, its release metadata, its freshness statement, and the root chain needed to verify all three — is importable from a file, and runs the identical client verification checklist a networked update runs.

An offline bundle's freshness statement is often already expired by the time it reaches an air-gapped device. It may be accepted only with the user's explicit confirmation, and the resulting update is shown as `uncertain` rather than fully verified (open decision D3) — the device is trusting that the bundle it was handed is the one it appears to be, without the live freshness check that would normally confirm it.

Applied to the problem statement's second scenario: a stale offline bundle carrying the year-old "3.2" still fails the client verification checklist's monotonic-counter step (step 2) against any release metadata version the device has already recorded from an earlier import or a prior network check — the anti-rollback rule does not relax simply because the bundle arrived offline.

## Third-party app and module bundles

An app author signs their bundle with an author key. For a non-curated app, the product pins that key the first time the app is installed — trust on first use, with the fingerprint shown and the install flow naming the out-of-band source to compare it against: the author's own published fingerprint on the app's site or its public source repository, the same discipline AEC-001 uses for a producer's key. For a curated app, the author key is instead entered into the registry the root role signs, offline, at each registry publication; the apps role, online, may only revoke a registered entry, never add one.

The product then bundles and hashes the app's own code at install time, the treatment RCS-001-R17 already requires: an app never ships script the renderer's CSP baseline would not already admit from the core, and signing under this document is the layer that makes that bundled, hashed code itself trustworthy once accepted.

An app update must be signed by the pinned author key, or by a rotation that key itself signed; a signature from any other key renders the update `failed`. The apps role may revoke a curated author key known to be malicious — never register one — after which anything signed by that key renders `failed` regardless of when it was originally signed; a non-curated author key carries no registry to revoke against and is instead re-pinned or rejected locally per device, the same trust-on-first-use discipline that pinned it.

Revocation is checked at run time, not only at update time: Omnifrons re-checks every installed app's author key against the current revocation list at each app launch, mirroring the re-probe discipline TM-001's HAR-3 applies to a harness or adapter's own identity evidence. An app whose author key has been revoked since its last launch is blocked from running until it is re-approved, shown with an explicit warning rather than failing silently.

Whether an unsigned app is installable at all in the first place is not yet settled — open decision D4 — but where it is allowed, both an unsigned bundle and one presenting an author key the device has never seen before — no prior trust-on-first-use pinning and no curated registry entry — are installable only behind an explicit warning naming which case applies, mirroring the stance TM-001's own open decision D10 takes for an unsigned harness or adapter binary.

A future commercial module, should ADR-0004's own follow-up ever elect one, is a bundle in this document's sense like any other third-party app: it earns trust through the same author-key pinning, the same bundling-and-hashing treatment at install time, and the same apps-role revocation path. This document does not need a separate mechanism for "module" versus "app"; it needs only the bundle to declare which one it is, for presentation and licensing purposes those other documents own.

## Boundaries restated

Three boundaries recur enough in the sections above to state once more, plainly, in one place:

- **Harness and adapter binaries** are TM-001's own signing and re-approval concern (open decision D10; threat HAR-3; actor A10), never this document's. A harness or adapter's distribution channel is outside UTA-001 entirely; TM-001 mitigates a compromise there only through device-local re-probe and renewed approval at next launch, not through a root-vouched signature.
- **Identity keys** — a producer's own key (AEC-001) and a device's own key (HTP-001; the RSP-001 core) — are never a root, release, freshness, or apps role key under this document. Proving which adapter instance or which device sent something is a different guarantee from proving a release or an app bundle should be trusted.
- **The signed backup manifest** (MRP-001) answers a different question with a different signature: this document decides whether a binary is trusted at all before it runs; MRP-001 decides when an already-trusted binary may run a migration against existing state, and what proves a backup of that state is itself recoverable.

None of these three boundaries is a gap this document defers to a future revision; each is a deliberate exclusion, stated once in Document authority above and restated here because a reader arriving at the mechanism sections directly, without reading Document authority first, should still find the same answer.

## Product requirements

Each requirement is an acceptance gate with a testable condition.

| ID | Requirement |
| --- | --- |
| UTA-001-R1 | A client MUST verify every update against its pinned root metadata by walking the root chain to the current version and checking the root metadata's own expiry at every verification; the update MUST render `failed` if the chain breaks at any step, or `uncertain` if the root metadata has expired, blocking automatic updates until a newer root chain is obtained. |
| UTA-001-R2 | The root role MUST sign with a threshold (default 2 of 3, open decision D1); a root metadata rotation MUST be signed by both the outgoing and the incoming threshold; and root metadata MUST declare its own expiry (default one year, open decision D11) and be re-signed on the refresh cadence D11 sets. |
| UTA-001-R3 | A client MUST verify the freshness statement's signature by a current freshness role key, its expiry, and that its named release metadata version is not lower than the last version this device has seen on this channel. |
| UTA-001-R4 | Every channel MUST enforce its own monotonic release metadata version counter; a client MUST reject a release metadata version not greater than the last one it has recorded for that channel, independent of wall-clock time. |
| UTA-001-R5 | A client MUST verify release metadata's signature by a current release role key, that its channel matches the device's own channel, and that its artifact digest and size match the entry for the device's OS and architecture. |
| UTA-001-R6 | A valid platform code signature or notarization MUST be treated as necessary and MUST NOT be treated as sufficient on its own to apply an update. |
| UTA-001-R7 | A binary-level downgrade MUST be refused unless the target version is within the downgrade window MRP-001 currently allows (MRP-001 D5) and a signed downgrade allowance for the exact version pair exists in release metadata. |
| UTA-001-R8 | The exemption in R7 — no signed downgrade allowance required — applies only to a rollback to the binary MRP-001 retains under its own binary-retention requirement (MRP-001-R9); it MUST NOT be read as exempting that binary from R9's revocation check. |
| UTA-001-R9 | Anything signed by a root, release, freshness, apps, or packaging role key revoked as of the current root metadata version MUST render `failed`, regardless of when the signature was produced; this check MUST be re-evaluated at every launch of every binary — including one MRP-001 retains for rollback — against the current root metadata, never against the release metadata pinned with that binary. |
| UTA-001-R10 | An online key compromise MUST trigger an emergency root-signed rotation and revocation, a signed advisory in release metadata, and a persistent product notice, and MUST keep automatic updates blocked until the device completes its next fully verified release check. |
| UTA-001-R11 | A root re-pin MUST occur only through a new product build distributed over platform-signed channels, MUST show the new root's fingerprint and instruct the user to compare it against at least one of the out-of-band sources named in open decision D12 before confirming; no automatic path MAY re-pin a root silently. |
| UTA-001-R12 | No root, release, freshness, apps, or packaging role key material MAY appear in the repository, a device backup, or portable state, under any custody class. |
| UTA-001-R13 | Manual update, with the full client verification checklist, MUST remain available regardless of channel or the standing automatic-update policy. |
| UTA-001-R14 | Automatic updates on `beta` and `stable` MUST remain blocked until UTA-001 is accepted; automatic updates on `nightly` and `alpha` MUST require explicit user opt-in. |
| UTA-001-R15 | An offline or air-gapped update bundle MUST run the identical client verification checklist as a networked update; an expired freshness statement inside such a bundle MAY be accepted only with explicit user confirmation and MUST render the result `uncertain`. |
| UTA-001-R16 | A third-party app update MUST be signed by the app's pinned author key — pinned via trust on first use for a non-curated app, or via the root-signed curated registry for a curated app — or by a rotation that key itself signed; any other signature MUST render the update `failed`. |
| UTA-001-R17 | The apps role MAY revoke a curated author key but MUST NOT register one; a curated author-key registration MUST be signed by the root role, offline, at each registry publication. Anything signed by a revoked author key MUST render `failed` regardless of when it was signed, and Omnifrons MUST re-check every installed app's author key against the current revocation list at each app launch, blocking the app from running until it is re-approved with an explicit warning shown. |
| UTA-001-R18 | Every blocked, `failed`, or `uncertain` update decision MUST disclose its reason to the user; no update MAY be silently withheld. |

## Signal mapping

The public vocabulary column uses the states the [compatibility policy](versioning-and-compatibility.md) publishes plus the target architecture's required failure states; this document proposes no new public token.

| Condition | UTA-001 state | Public vocabulary | Consequence |
| --- | --- | --- | --- |
| Freshness statement expired and unreachable for refresh | `freshness-expired` | `uncertain` | Automatic update blocked; manual check offered |
| Signature by a revoked role or author key | `revoked-key` | `failed` | Update rejected with reason shown |
| Downgrade requested without a signed allowance | `blocked-downgrade` | `failed` | Downgrade path (signed allowance) shown as the alternative |
| Platform code signature or notarization invalid | `platform-signature-invalid` | `failed` | Update rejected regardless of role-metadata validity |
| Root chain breaks between the pinned version and the current one | `root-chain-break` | `failed` | Automatic mutation blocked; manual re-pinning path offered |
| Root re-pin awaiting the user's fingerprint confirmation | `root-rotation-pending` | `uncertain` | No automatic mutation until confirmed |
| Update trust artifact not yet accepted, or its client unavailable | `update-trust-unavailable` | (required failure state) | Block automatic mutation; retain recovery evidence |
| Third-party app author key mismatch | `author-key-mismatch` | `failed` | App update rejected; existing pinned version unaffected |
| Unsigned third-party app installed under open decision D4 | `unsigned-app-installed` | `uncertain` | Explicit unsigned warning required before install proceeds |
| Pinned root metadata expired | `root-metadata-expired` | `uncertain` | Automatic update blocked; newer root chain required before any update proceeds |
| Freshness statement names a root metadata version the device has not pinned | `root-version-mismatch` | `uncertain` | Signals a possible frozen or stale root; newer root metadata required before automatic update resumes |
| Platform revocation-list check unreachable (offline signature/notarization still valid) | `platform-revocation-unreachable` | `uncertain` | Automatic update blocked; manual proceeds only with the offline-only status disclosed |

## Open decisions

| ID | Question | Options | Default proposal |
| --- | --- | --- | --- |
| D1 | Root role signing threshold | 2 of 3; 3 of 5; a single key | 2 of 3 |
| D2 | Freshness statement expiry per channel | Uniform expiry across channels; per-channel expiry tied to release cadence | `stable` 7 days, `beta` 3 days, `nightly` 24 hours |
| D3 | Offline bundle expiry relaxation | Refuse an expired freshness statement outright; accept with explicit confirmation, shown `uncertain` | Accept with explicit confirmation, shown `uncertain` |
| D4 | Unsigned or unknown-author-key third-party apps | Disallow entirely; allow with an explicit warning naming which case applies | Allow with an explicit warning in v1; revisit before Beta |
| D5 | Metadata layout | A TUF-compatible layout; a custom layout | A TUF-compatible layout |
| D6 | Downgrade allowance mechanism | Per version pair, listed in release metadata; a separate allowance document | Per version pair, in release metadata |
| D7 | Root key custody hardware requirement | Hardware-backed required; software custody accepted | Hardware-backed, required by default |
| D8 | Revocation propagation | Carried in every freshness statement; carried only in release metadata | In every freshness statement |
| D9 | Disclosure home for update-trust state | An Ops status line; the Recovery section MRP-001 D8 proposes | Whichever context-orb.md accepts first; both name the same underlying state |
| D10 | Linux packaging targets to sign, and the key that signs them | AppImage only; a package family only; both; signed by the release role key vs. a dedicated packaging role key listed in root metadata | AppImage plus one package family, signed by the release role key initially; a dedicated packaging role key, listed in root metadata like any other role key, remains open if packaging cadence outpaces release cadence |
| D11 | Root metadata expiry and refresh cadence | No expiry, relying on rotation alone; a short expiry (90 days); a long expiry (one year); a multi-year expiry | One year expiry; the root role re-signs a fresh root metadata version at least 60 days before that expiry so a client with normal connectivity never encounters a frozen root |
| D12 | Out-of-band sources for confirming a re-pinned root's fingerprint | A single source; two independent sources; three including a platform listing | At least two independent, reachable-without-the-update-path sources: the project's security page over HTTPS and a signed release note or tag in the public repository; the platform store listing where one exists, as a third; the re-pin flow instructs the user to compare against at least one named source before confirming |

## Acceptance evidence and follow-up

Tests the roadmap's Alpha → Beta update-trust gate requires:

- An expired-freshness test MUST verify that an unreachable, expired freshness statement blocks automatic update and renders `uncertain` (UTA-001-R3).
- A rollback-refusal test MUST verify that a release metadata version not greater than the last seen is rejected (UTA-001-R4).
- A signed-downgrade-allowance test MUST verify that a downgrade succeeds only with a matching version-pair allowance inside the MRP-001 D5 window, and is refused otherwise (UTA-001-R7).
- A revoked-key test MUST verify that a release role, freshness role, or author key revoked as of the current root metadata is rejected regardless of signature validity (UTA-001-R9, R17).
- A root-rotation test MUST verify that a properly chained root rotation is accepted and that a broken chain is rejected (UTA-001-R1, R2).
- A platform-signature test MUST verify rejection on each supported OS when the platform signature or notarization is invalid, independent of role-metadata validity (UTA-001-R6).
- An offline-bundle test MUST verify that an imported bundle runs the identical checklist and that an expired freshness statement inside it is accepted only with explicit confirmation, shown `uncertain` (UTA-001-R15).
- An author-key-mismatch test MUST verify that an app update signed by any key other than the pinned author key, or a rotation it signed, is rejected (UTA-001-R16).
- A key-loss drill MUST verify that the root role's remaining threshold still signs successfully after one share is lost.
- A revocation-without-cooperation test MUST verify that a departed or compromised holder's own root role share can be revoked by the remaining threshold without that holder's participation.
- An unavailable-key behavior drill MUST cover the compatibility policy's own rollback and recovery qualification requirement for failed update and unavailable-key behavior.
- A key-material scan MUST verify that no root, release, freshness, apps, or packaging role key material appears in the repository, a device backup, or portable state (UTA-001-R12).
- A channel-switch test MUST verify that switching channels requires explicit confirmation and runs the full client verification checklist rather than inheriting trust from the previous channel (UTA-001-R14).
- An unsigned-app-warning test MUST verify that, wherever unsigned third-party apps are allowed under open decision D4, installation shows an explicit unsigned warning rather than proceeding silently.
- A root-metadata-freeze test MUST verify that an expired pinned root metadata renders the update `uncertain`, blocks automatic updates, and requires a newer root chain before any update proceeds; and that a freshness statement naming a root metadata version the device has not pinned is flagged as a staleness signal before the root's own expiry would otherwise catch it (UTA-001-R1, R2).
- A root re-pin source test MUST verify that the re-pin flow shows the new root's fingerprint and names at least one of the out-of-band sources in open decision D12 for the user to compare against before confirming (UTA-001-R11).
- A curated-registry test MUST verify that a curated author-key registration is accepted only when signed by the root role, offline, at each registry publication, and that an apps-role-only signature on a new registration is rejected; a curated-revocation test MUST verify that the apps role alone can revoke an existing entry (UTA-001-R16, R17).
- A retained-binary-revocation test MUST verify that launching MRP-001's retained binary re-evaluates its signing release role key against the current root metadata and renders `failed` with out-of-band guidance if that key has since been revoked, independent of R7's downgrade-allowance exemption (UTA-001-R8, R9).
- An installed-app re-probe test MUST verify that every installed app's author key is re-checked against the current revocation list at each launch, and that a revoked key blocks the app until re-approved with an explicit warning (UTA-001-R17).
- A platform-offline-verification test MUST verify that an unreachable revocation-list check renders the platform step `uncertain`, blocks automatic updates, and discloses the offline-only status before a manual update proceeds.
- Security Reviewer and Legal Counsel advice is recorded, per the roadmap's Alpha → Beta gate, before the Project Owner accepts this document.

Reproducing the problem statement's own scenarios is itself part of this checklist:

- Reproducing the first scenario — a compromised release server signing a replacement installer — MUST render the replacement `failed` at the client verification checklist despite carrying a valid platform signature.
- Reproducing the second scenario — a stale, validly signed "3.2" served by a mirror — MUST render it `failed` or `uncertain` under the monotonic-counter and offline-bundle rules, never silently applied.
- Reproducing the third scenario — a departed key holder — MUST verify the root role's remaining threshold still signs after one share is lost, and that revocation of the departed holder's own share does not require that holder's cooperation.

Debt, not drafted here. This document does not resolve whether unsigned third-party apps are allowed at all past v1 (open decision D4), the final choice between a TUF-compatible and a custom metadata layout (D5), or where update-trust state is disclosed in the interface (D9) — that last item depends on context-orb.md's own acceptance the same way MRP-001's own recovery-presentation gap does.

## Related contracts

- [Target architecture](target-architecture.md) — the UTA-001 row in planned assurance artifacts; the sentence gating automatic beta/stable update on update trust; the "Migration/update trust unavailable" required failure state this document's signal mapping reuses; invariants 1, 2, and 9.
- [Versioning and compatibility](versioning-and-compatibility.md) — the release channels, support matrix, migration and upgrade graph, the Update trust section this document drafts in full, and the rollback and recovery qualification's failed-update and unavailable-key behavior.
- [ADR-0002: Desktop technology stack](adr/0002-desktop-technology-stack.md) — the Tauri 2 decision, per-OS packaging and signing differences, the planned verification plan (VP-001), and the update-signing tests that do not themselves approve automatic updates.
- [ADR-0004: Fully open platform with custom integrated apps](adr/0004-open-platform-and-custom-apps.md) — custom-app packaging and signing riding on this artifact; the app SDK and commercial-module decision this document does not make.
- [Migration and recovery plan](migration-and-recovery-plan.md) (MRP-001) — binary retention (R9), the downgrade window (D5), and the mid-migration marker (R21); this document decides whether a binary is trusted, MRP-001 decides when it may run a migration.
- [Renderer content-security contract](renderer-content-security.md) (RCS-001) — third-party app bundling and hashing at install time (R17), signed per this document once accepted.
- [Threat model](threat-model.md) (TM-001) — actor A10 and threat HAR-3, the re-approval mitigation for a compromised harness or adapter distribution channel, and open decision D10, all outside this document's scope by design.
- [Adapter feed event schema](adapter-feed-events.md) (AEC-001) — producer identity and its own signed-announcement rotation pattern, distinct from this document's roles.
- [Handoff transaction protocol](handoff-transaction-protocol.md) (HTP-001) and [Workspace roaming protocol](workspace-roaming-protocol.md) (RSP-001 core) — device identity keys and handoff/claim signatures, distinct from this document's roles.
- [Product roadmap](roadmap.md) — the Alpha → Beta promotion gate naming update trust directly as required evidence, approved by the Project Owner after Security Reviewer and Legal Counsel advice; the Beta scope's signed installers and the 1.0 exit criteria's published update trust posture.
- [Context Orb specification](context-orb.md) — presentation of the states this document names; the disclosure-home open decision (D9) this document leaves to that specification's own acceptance.
- Governance and support matrix (GOV-001) — named roles and approval authority, including who holds a root role share; not yet drafted.

## References

- [The Update Framework specification](https://theupdateframework.io/) — the root/online-role separation, threshold signing, and freshness-statement pattern this document adapts.
- [Apple notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution) — the macOS platform-signing mechanism the platform signing table cites.
- [Windows Authenticode](https://learn.microsoft.com/windows/win32/seccrypto/authenticode) — the Windows platform-signing mechanism the platform signing table cites.
- [AppImage documentation](https://docs.appimage.org/) — the Linux packaging target referenced in open decision D10.
- [Semantic Versioning](https://semver.org/) — the version field release metadata carries.
- [Keep a Changelog](https://keepachangelog.com/) — the release-notes categories a compromise advisory and a revoked key are disclosed under, per the compatibility policy's own changelog obligations.
- [Target architecture](target-architecture.md) — the required failure states this document's signal mapping reuses.
- [Versioning and compatibility](versioning-and-compatibility.md) — the Update trust section this document drafts in full.
