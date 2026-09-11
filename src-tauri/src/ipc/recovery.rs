//! The recovery-entry commands (spike slice 5e, HAP-001-R18, R22, D3):
//! `recovery_list` and `recovery_approve`.
//!
//! When a publication's outbox path stops naming the file its handle
//! holds, HAP-001-R18 keeps the held bytes as a **recovery entry** in the
//! product work area, "so that what was digested and approved is what
//! survives", and requires "a fresh explicit approval that no standing
//! policy (D3) covers" before it may be re-published. Slice 5b wrote
//! those entries; nothing listed one and nothing re-published one.
//!
//! **This is its own command, not the whole-outbox approval path.** That
//! path names an entry by name *inside the outbox* and applies outbox
//! containment; a recovery entry has no outbox entry at all -- it is
//! digest-named in a directory outside every workspace, and the thing it
//! was recovered from may no longer exist anywhere.
//!
//! **Both commands are device-wide on the way in and project-scoped on the
//! way out** (slice 5e review, R1-005). The work area holding these
//! entries is device configuration (HAP-001-R5) and the journal that
//! accounts for them is one per device; the `outbox-escape` step that
//! writes one carries no project identity, so `recovery_list` cannot scope
//! by project and does not pretend to -- it lists every entry this device
//! holds, whatever workspace is active. `recovery_approve` then publishes
//! into the project that is active **now**: the identity is derived from
//! that project (D9) and the destination is that project's asset root. An
//! entry that escaped under one project and is approved under another is
//! therefore registered as the second project's artifact, with
//! `recoveredFrom` naming a publication the second project's catalog does
//! not hold -- which is the same thing it names once the first project is
//! deleted, the case R18 exists for. The alternative, scoping the listing,
//! would need a project identity on a persisted step shape and would hide
//! exactly those entries; it is disclosed here, on `RecoveryEntryDto`, on
//! both commands and on the surface rather than implied.
//!
//! **A digest is a locator, not an identity check.** The entry's file
//! name is the digest of the bytes that were preserved, which makes it
//! tempting to treat "a file exists at that name" as "those bytes are
//! there". It is not: `recovery_approve` opens the entry exactly once
//! without following a link, takes the regular-file fact and the link
//! count from that handle, digests it from the same handle, and refuses
//! unless what it read is the name it is filed under (HAP-001-R15, R16,
//! R20 applied to a recovery entry). The listing says the same thing in
//! its own way: its `usable` flag is the probe's answer, not the name's.
//!
//! That is **one** digest comparison against **one** request field, since
//! slice 5e's review: the request carried a second digest for the surface
//! to bind to, and a recovery entry has only one digest for a surface to
//! have seen -- the name -- so the second was the first under another key
//! and could never disagree with it (R1-006 / R3-069, and
//! [`open_and_verify`]).
//!
//! The re-publication registers under the identity HAP-001-R23 derives
//! from the **bytes that were read** -- which is not always the escaped
//! publication's identity, because the file behind a held handle can have
//! been rewritten in place before the escape was detected. The original
//! publication travels as a location fact on the approval
//! (`recoveredFrom`), never as provenance: the record carries `producer:
//! unattributed`, because nothing about a recovery entry attributes it to
//! a run (HAP-001-R11, R36).

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::SystemTime;

use omnifrons_adapters::{
    FsCandidateProber, JsonlPublicationJournal, Sha2Hasher, open_recovery_dir,
};
use omnifrons_app::blob_store::DestinationError;
use omnifrons_app::content_hasher::{derive_artifact_approval_id, derive_publication_identity};
use omnifrons_app::outbox_policy::{ArtifactClassifier as _, OutboxPolicyStore as _};
use omnifrons_app::publication::CandidateSource;
use omnifrons_app::publication_journal::{PublicationJournal as _, find_approval};
use omnifrons_app::run_outbox::{
    CandidateProbe, CandidateProber as _, DirectoryHandle, RegularCandidate,
};
use omnifrons_app::work_area::WorkAreaRoot;
use omnifrons_domain::executable::DeviceLocalUser;
use omnifrons_domain::outbox::{ArtifactClass, Attribution, ContentDigest};
use omnifrons_domain::publication::{
    ApprovalSource, ArtifactApproval, JournalEntry, JournalStep, PublicationIdentity,
};
use omnifrons_supervisor::TokioProcessSupervisor;
use tauri::{AppHandle, Manager as _};

use crate::ipc::commands::active_workspace;
use crate::ipc::dto::{
    ArtifactApprovalDto, RecoveryEntryDto, ShellError, ShellErrorCode, system_time_to_millis,
};
use crate::ipc::publication::{RunActivity, hold_for_publication, run_active};
use crate::outbox_state::OutboxState;
use crate::publication_state::PublicationState;

/// What this device's journal records about one recovery entry: the
/// publication whose `outbox-escape` wrote it and when. Read from the
/// journal, never from the file name -- the name carries a digest and
/// nothing else.
#[derive(Debug, Clone, Copy)]
struct RecoveryProvenance {
    recovered_from: PublicationIdentity,
    recovered_at: SystemTime,
}

/// Every recovery entry this device's journal knows about, earliest
/// first, deduplicated by digest.
///
/// One digest can appear on more than one `outbox-escape` step: two
/// publications whose held bytes were identical share one entry, because
/// `write_recovery_entry` keeps the existing file rather than writing a
/// second copy of the same bytes. The **earliest** step is the one
/// reported, and that is a spike default disclosed in the log rather than
/// a fact: both publications' bytes really are these bytes, so either
/// attribution is true of the content, and the approval re-derives the
/// identity from what it reads anyway.
fn journaled_recovery(entries: &[JournalEntry]) -> Vec<(ContentDigest, RecoveryProvenance)> {
    let mut found: Vec<(ContentDigest, RecoveryProvenance)> = Vec::new();
    for entry in entries {
        let JournalEntry::Step(step) = entry else {
            continue;
        };
        let (Some(recovery), JournalStep::OutboxEscape) = (step.recovery, step.step) else {
            continue;
        };
        if found.iter().any(|(digest, _)| *digest == recovery) {
            continue;
        }
        found.push((
            recovery,
            RecoveryProvenance {
                recovered_from: step.publication_id,
                recovered_at: step.at,
            },
        ));
    }
    found
}

/// Every recovery entry of **this device**, with the facts taken from each
/// entry's own handle (HAP-001-R15), its provenance from the journal, and
/// the display name and class the approval would use.
///
/// `workspace` does not scope the listing and never has (R1-005): the work
/// area and its journal are one per device, so every entry this device
/// holds is listed whatever project is active. What the workspace decides
/// is the *policy* the class is read against and, for
/// [`recovery_approve_for`], the project the bytes would be registered in.
///
/// The display name and the class are derived the same way the approval
/// derives them, so the surface can show, before the decision, what the
/// artifact would be registered and written as (R1-004). The class is
/// `None` for an entry whose probe yielded no facts to classify.
///
/// # Errors
///
/// `work-area-invalid` when the work area fails its check or the journal
/// cannot be read; `outbox-invalid` when the project's classification
/// policy is present but cannot be loaded.
pub fn recovery_list_for(
    outbox: &OutboxState,
    workspace: &omnifrons_app::WorkspaceRoot,
    publication: &PublicationState,
) -> Result<Vec<RecoveryEntryDto>, ShellError> {
    // The journal is appended to by every publication; a replay that
    // interleaves with one can read a line mid-write. The same lock every
    // other reader of this surface takes.
    let _surface = publication.lock_surface();
    let work_area = WorkAreaRoot::open(&publication.work_area, &[workspace])?;
    let journal = JsonlPublicationJournal::open(&work_area)?;
    let entries = journal.replay()?;
    let Ok((dir, dir_path)) = open_recovery_dir(&work_area) else {
        return Err(ShellError::from(
            omnifrons_app::work_area::WorkAreaError::Unusable,
        ));
    };
    // The same policy the approval classifies against, loaded once for
    // the whole listing rather than per entry (R1-004).
    let policy = outbox.policy_store.load(workspace)?;
    let single_open = FsCandidateProber::new();
    Ok(journaled_recovery(&entries)
        .into_iter()
        .map(|(digest, provenance)| {
            let opened = single_open.probe(&dir, &dir_path, &OsString::from(digest.to_hex()));
            // The name the approval would register this under, derived
            // here exactly as `recovery_approve_for` derives it.
            let display_name = recovered_display_name(&entries, provenance.recovered_from, digest);
            let (size, usable, class) = match opened {
                // The name is a locator; these are the facts.
                CandidateProbe::Regular(regular) => (
                    Some(regular.size),
                    regular.digest == digest,
                    // Classified from the handle's own detected type and
                    // size beside that name -- never from the name alone,
                    // and never invented for an entry that yielded
                    // neither (the `None` arm below).
                    Some(policy.classify(
                        display_name.as_str(),
                        regular.detected_type,
                        regular.size,
                    )),
                ),
                CandidateProbe::Linked { .. }
                | CandidateProbe::Escape(_)
                | CandidateProbe::Unreadable => (None, false, None),
            };
            RecoveryEntryDto {
                sha256: digest.to_hex(),
                size,
                recovered_from: provenance.recovered_from.to_hex(),
                recovered_at: system_time_to_millis(provenance.recovered_at),
                display_name: display_name.as_str().to_string(),
                class: class.map(|class| class.as_str().to_string()),
                usable,
            }
        })
        .collect())
}

fn invalid(message: &str) -> ShellError {
    ShellError::new(ShellErrorCode::InvalidRequest, message)
}

fn refused(message: &str) -> ShellError {
    ShellError::new(ShellErrorCode::Refused, message)
}

/// HAP-001-R15, R16, R20 applied to a recovery entry: one no-follow open
/// relative to the recovery directory's own handle, every identity fact
/// and the digest taken from that handle, and the handle handed back for
/// the publication that follows.
///
/// **One digest comparison, because a recovery entry has one digest**
/// (slice 5e review, R1-006 / R3-069). This took two -- `named`, the name
/// the entry is filed under, and `bound`, "what the caller's own listing
/// showed" -- and refused unless the bytes matched both, on the reasoning
/// that either alone would let the other case through. That reasoning is
/// sound where the two can differ, and they cannot here:
/// `RecoveryEntryDto` carries exactly one digest and it *is* the name, so
/// every caller that gets its facts from `recovery_list` can only pass the
/// same value twice. The shipped surface did, and swapping the two keys
/// changed nothing observable. The two-key shape came from
/// `artifact_approve`, where `name` is an outbox file name and `sha256`
/// its digest -- genuinely independent facts -- and it does not transfer
/// to an entry whose name is its digest. A check that cannot fail is not
/// a check; it is a claim of protection. So: one parameter, one
/// comparison, and the staleness the second was meant to catch is caught
/// where it is real -- the surface re-binds its open block to the entry's
/// digest on every refetch, and this re-derives the digest from its own
/// handle regardless of anything the caller believed.
fn open_and_verify(
    work_area: &WorkAreaRoot,
    named: ContentDigest,
) -> Result<(DirectoryHandle, PathBuf, RegularCandidate), ShellError> {
    let (dir, dir_path) = open_recovery_dir(work_area)
        .map_err(|_| ShellError::from(omnifrons_app::work_area::WorkAreaError::Unusable))?;
    let file_name = OsString::from(named.to_hex());
    let regular = match FsCandidateProber::new().probe(&dir, &dir_path, &file_name) {
        CandidateProbe::Regular(regular) => regular,
        CandidateProbe::Escape(_) => {
            return Err(refused("the recovery entry is not a regular file"));
        }
        CandidateProbe::Linked { .. } => {
            return Err(refused(
                "the recovery entry's link count is greater than one",
            ));
        }
        CandidateProbe::Unreadable => {
            return Err(ShellError::new(
                ShellErrorCode::RecoveryUnknown,
                "the recovery entry could not be opened",
            ));
        }
    };
    // The name said these bytes; the handle says what they are. A digest
    // in a file name is a locator and never stands in for this check.
    if regular.digest != named {
        return Err(ShellError::new(
            ShellErrorCode::IntegrityMismatch,
            "the recovery entry's bytes are not the digest it is filed under",
        ));
    }
    Ok((dir, dir_path, regular))
}

/// Approve the recovery entry `digest` names for re-publication
/// (HAP-001-R18, R22, D3): a fresh, explicit, per-artifact action, bound
/// to identity facts taken from the entry's own handle and never from the
/// name it is filed under.
///
/// # Errors
///
/// `invalid-request` for a digest that is not 64 hex characters or an
/// approval id collision; `run-active` while a supervised process runs;
/// `work-area-invalid` when the work area fails its check or the journal
/// cannot be used; `recovery-unknown` when no `outbox-escape` of this
/// device's journal wrote that entry, or it cannot be opened at all;
/// `outbox-invalid` when the project's classification policy is present
/// but cannot be loaded -- the class decides whether this approval is
/// allowed at all, so an unknowable policy blocks it (R1-014 / R3-077);
/// `refused` when the entry is not a regular file with a link count of
/// one or its class is not `generated-heavy`; `integrity-mismatch` when
/// the bytes read are not the digest the entry is filed under;
/// `destination-invalid` when the project declares no asset root.
pub fn recovery_approve_for(
    outbox: &OutboxState,
    workspace: &omnifrons_app::WorkspaceRoot,
    publication: &PublicationState,
    run_activity: &dyn RunActivity,
    digest: &str,
    now: SystemTime,
) -> Result<ArtifactApprovalDto, ShellError> {
    let _surface = publication.lock_surface();
    if run_activity.any_running() {
        return Err(run_active());
    }
    let named = ContentDigest::from_hex(digest)
        .ok_or_else(|| invalid("the recovery entry's digest is not 64 hex characters"))?;

    let work_area = WorkAreaRoot::open(&publication.work_area, &[workspace])?;
    let mut journal = JsonlPublicationJournal::open(&work_area)?;
    let entries = journal.replay()?;
    let provenance = journaled_recovery(&entries)
        .into_iter()
        .find_map(|(found, provenance)| (found == named).then_some(provenance))
        .ok_or_else(|| {
            ShellError::new(
                ShellErrorCode::RecoveryUnknown,
                "no recovery entry with that digest is recorded for this device",
            )
        })?;

    let policy = outbox.policy_store.load(workspace)?;
    let asset_root_id = policy
        .asset_root_id()
        .cloned()
        .ok_or_else(|| ShellError::from(DestinationError::Unconfigured))?;

    let (dir, dir_path, regular) = open_and_verify(&work_area, named)?;

    let hasher = Sha2Hasher::new();
    let project = omnifrons_app::content_hasher::derive_project_identity(&hasher, workspace);
    // The display name is the escaped publication's, when this device's
    // journal still carries its approval; otherwise the digest itself,
    // which is honest about what is known rather than inventing a name.
    let display_name = recovered_display_name(&entries, provenance.recovered_from, named);
    let class = policy.classify(display_name.as_str(), regular.detected_type, regular.size);
    if class != ArtifactClass::GeneratedHeavy {
        return Err(refused(
            "only a generated-heavy recovery entry can be approved for publication",
        ));
    }

    // HAP-001-R23: the identity is derived from the bytes that were read.
    // It equals the escaped publication's whenever the entry holds what
    // that publication approved, and differs when the file behind the
    // held handle had been rewritten in place before the escape -- in
    // which case registering under the old identity would register these
    // bytes as content they are not.
    let publication_id = derive_publication_identity(&hasher, &project, &regular.digest);
    let approval = ArtifactApproval {
        approval_id: derive_artifact_approval_id(&hasher, &publication_id, now),
        publication_id,
        project,
        run_id: None,
        name: named.to_hex(),
        display_name,
        digest: regular.digest,
        size: regular.size,
        detected_type: regular.detected_type,
        class,
        attribution: Attribution::Unattributed,
        asset_root_id,
        adapter_id: None,
        executable_approval: None,
        approver: DeviceLocalUser,
        approved_at: now,
        source: ApprovalSource::Recovery {
            recovered_from: provenance.recovered_from,
        },
    };
    if find_approval(&entries, approval.approval_id).is_some() {
        return Err(invalid(
            "an approval with this derived id is already recorded",
        ));
    }
    journal.append(&JournalEntry::Approved(Box::new(approval.clone())))?;
    let handle_held = hold_for_publication(
        &outbox.runs,
        approval.approval_id,
        CandidateSource {
            dir,
            dir_path,
            file_name: OsString::from(named.to_hex()),
            handle: regular.handle,
        },
    );
    Ok(ArtifactApprovalDto::from_approval(&approval, handle_held))
}

/// The name a recovery entry is registered and written under (D9): the
/// escaped publication's own display name when this device's journal still
/// carries that approval, and the entry's digest otherwise -- honest about
/// what is known rather than inventing a name.
///
/// **One derivation, two callers.** `recovery_list_for` shows this before
/// the decision and `recovery_approve_for` registers under it, and the
/// class that decides whether the approval is allowed at all is read off
/// it -- so the listing showing one name and the approval using another
/// would be a surface that lies about the thing it is asking permission
/// for (R1-004).
fn recovered_display_name(
    entries: &[JournalEntry],
    publication: PublicationIdentity,
    named: ContentDigest,
) -> omnifrons_domain::publication::DisplayName {
    entries
        .iter()
        .find_map(|entry| match entry {
            JournalEntry::Approved(approval) if approval.publication_id == publication => {
                Some(approval.display_name.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| omnifrons_domain::publication::DisplayName::sanitize(&named.to_hex()))
}

/// Every recovery entry **this device** holds (HAP-001-R18), each carrying
/// the display name and class its approval would use (R1-004).
///
/// The listing is device-wide, not project-scoped (R1-005): an active
/// workspace is required -- it names the policy the class is read against,
/// and the project an approval would register into -- but it does not
/// filter the entries. An entry an escape wrote while another project was
/// open is listed here, and approving it publishes those bytes into the
/// project that is active now.
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`recovery_list_for`]'s errors.
#[tauri::command]
pub async fn recovery_list(app: AppHandle) -> Result<Vec<RecoveryEntryDto>, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let outbox = app.state::<OutboxState>();
        let publication = app.state::<PublicationState>();
        recovery_list_for(&outbox, &workspace, &publication)
    })
    .await
    .expect("the blocking recovery-list task panicked")
}

/// Approve one for re-publication through the existing `artifact_publish`
/// (HAP-001-R18: a fresh explicit approval no standing policy covers).
///
/// The entry is the device's; the publication is the **active project's**
/// (R1-005). The identity is derived from this project and the bytes that
/// were read, and the destination is this project's asset root, whichever
/// project's publication originally escaped.
///
/// # Errors
///
/// [`ShellErrorCode::WorkspaceUnavailable`] if no workspace is active, and
/// [`recovery_approve_for`]'s errors.
#[tauri::command]
pub async fn recovery_approve(
    app: AppHandle,
    digest: String,
) -> Result<ArtifactApprovalDto, ShellError> {
    let workspace = active_workspace(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let outbox = app.state::<OutboxState>();
        let publication = app.state::<PublicationState>();
        let supervisor = app.state::<TokioProcessSupervisor>().inner().clone();
        recovery_approve_for(
            &outbox,
            &workspace,
            &publication,
            &supervisor,
            &digest,
            SystemTime::now(),
        )
    })
    .await
    .expect("the blocking recovery-approve task panicked")
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, SystemTime};

    use omnifrons_adapters::{JsonlCatalogStore, JsonlPublicationJournal, Sha2Hasher};
    use omnifrons_app::WorkspaceRoot;
    use omnifrons_app::catalog_store::CatalogStore as _;
    use omnifrons_app::content_hasher::{ContentHasher as _, derive_publication_identity};
    use omnifrons_app::outbox_policy::POLICY_FILE_PATH;
    use omnifrons_app::publication_journal::PublicationJournal as _;
    use omnifrons_app::work_area::WorkAreaRoot;
    use omnifrons_domain::executable::{DeviceLocalUser, Sha256Digest};
    use omnifrons_domain::outbox::{ArtifactClass, Attribution, DetectedType};
    use omnifrons_domain::publication::{
        ApprovalSource, ArtifactApproval, ArtifactApprovalId, AssetRootId, DisplayName,
        JournalEntry, JournalStep, ProjectIdentity, PublicationIdentity, StandingApprovalPolicy,
        StepEntry, StepOutcome,
    };

    use super::{recovery_approve_for, recovery_list_for};
    use crate::ipc::dto::{ArtifactStateFrame, ArtifactStateTag, ShellErrorCode};
    use crate::ipc::publication::{RunActivity, publish_approved};
    use crate::outbox_state::OutboxState;
    use crate::publication_state::PublicationState;

    struct Idle;
    impl RunActivity for Idle {
        fn any_running(&self) -> bool {
            false
        }
    }

    struct Busy;
    impl RunActivity for Busy {
        fn any_running(&self) -> bool {
            true
        }
    }

    const PDF: &[u8] = b"%PDF-1.7\nomnifrons recovered artifact\n";

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "omnifrons-shell-recovery-{}-{label}-{n}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).expect("fixture dir");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A project whose policy names asset root `main`, a device directory
    /// holding the work area and the asset roots, and a recovery entry
    /// **arranged as the end state an `outbox-escape` leaves** -- the
    /// bytes under `recovery/<digest>` and the journal step that names
    /// them -- rather than by racing a publication to produce one.
    struct Fixture {
        project: TempDir,
        device: TempDir,
        outbox: OutboxState,
        publication: PublicationState,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let project = TempDir::new(label);
            let device = TempDir::new(&format!("{label}-device"));
            let policy_path = project.path().join(POLICY_FILE_PATH);
            std::fs::create_dir_all(policy_path.parent().expect("parent")).expect("mkdir");
            std::fs::write(&policy_path, r#"{"schema": 1, "assetRootId": "main"}"#)
                .expect("policy");
            Self {
                project,
                publication: PublicationState::under(device.path()),
                device,
                outbox: OutboxState::new(),
            }
        }

        fn workspace(&self) -> WorkspaceRoot {
            WorkspaceRoot::new(self.project.path()).expect("workspace")
        }

        fn work_area(&self) -> WorkAreaRoot {
            WorkAreaRoot::open(&self.publication.work_area, &[&self.workspace()])
                .expect("work area")
        }

        fn project_identity(&self) -> ProjectIdentity {
            omnifrons_app::content_hasher::derive_project_identity(
                &Sha2Hasher::new(),
                &self.workspace(),
            )
        }

        /// Plant the end state of an `outbox-escape`: `bytes` under
        /// `recovery/<digest of bytes>`, and the journal step naming that
        /// digest for `escaped_from`.
        fn plant_recovery(&self, bytes: &[u8], escaped_from: PublicationIdentity) -> Sha256Digest {
            let digest = Sha2Hasher::new().sha256(bytes);
            let work_area = self.work_area();
            std::fs::write(work_area.recovery_dir().join(digest.to_hex()), bytes)
                .expect("recovery entry");
            let mut journal = JsonlPublicationJournal::open(&work_area).expect("journal");
            journal
                .append(&JournalEntry::Step(Box::new(StepEntry {
                    publication_id: escaped_from,
                    approval_id: omnifrons_domain::publication::ArtifactApprovalId(0x0102_0304),
                    step: JournalStep::OutboxEscape,
                    outcome: StepOutcome::Failed,
                    digest,
                    reason: Some("path-names-a-different-file".to_string()),
                    at: Self::escaped_at(),
                    record: None,
                    recovery: Some(digest),
                })))
                .expect("journaled");
            digest
        }

        /// Plant the `approved` line the escaped publication itself left
        /// in this device's journal, so the display-name branch of D9 has
        /// something to find. Everything but the publication identity and
        /// the display name is irrelevant to that lookup.
        fn plant_escaped_approval(&self, publication: PublicationIdentity, display_name: &str) {
            let work_area = self.work_area();
            let mut journal = JsonlPublicationJournal::open(&work_area).expect("journal");
            journal
                .append(&JournalEntry::Approved(Box::new(ArtifactApproval {
                    approval_id: ArtifactApprovalId(0x0a0b_0c0d),
                    publication_id: publication,
                    project: self.project_identity(),
                    run_id: None,
                    name: display_name.to_string(),
                    display_name: DisplayName::sanitize(display_name),
                    digest: Sha2Hasher::new().sha256(PDF),
                    size: PDF.len() as u64,
                    detected_type: DetectedType::Pdf,
                    class: ArtifactClass::GeneratedHeavy,
                    attribution: Attribution::Unattributed,
                    asset_root_id: AssetRootId::new("main").expect("valid"),
                    adapter_id: None,
                    executable_approval: None,
                    approver: DeviceLocalUser,
                    approved_at: Self::escaped_at(),
                    source: ApprovalSource::Outbox,
                })))
                .expect("journaled");
        }

        fn escaped_at() -> SystemTime {
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_725_782_399)
        }

        fn now() -> SystemTime {
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_725_782_401)
        }
    }

    /// **The listing shows what the approval will register it as**
    /// (slice 5e review, R1-004): the display name `recovery_approve`
    /// derives and the class it classifies that name and those bytes
    /// under, both taken the same way the approval takes them, so the
    /// decision is made in front of the facts that decide it rather than
    /// after.
    #[test]
    fn the_listing_carries_the_display_name_and_class_the_approval_would_use() {
        let fixture = Fixture::new("list-name-and-class");
        let escaped_from = PublicationIdentity(Sha256Digest([0x99; 32]));
        let digest = fixture.plant_recovery(PDF, escaped_from);
        fixture.plant_escaped_approval(escaped_from, "quarterly report.pdf");

        let listed = recovery_list_for(&fixture.outbox, &fixture.workspace(), &fixture.publication)
            .expect("listing");
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert_eq!(listed[0].display_name, "quarterly report.pdf");
        assert_eq!(listed[0].class.as_deref(), Some("generated-heavy"));

        // And the approval agrees, which is the whole point of showing it.
        let approval = recovery_approve_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Idle,
            &digest.to_hex(),
            Fixture::now(),
        )
        .expect("approved");
        assert_eq!(approval.display_name, listed[0].display_name);
        assert_eq!(Some(approval.class), listed[0].class);
    }

    /// The listing takes D9's other branch too: with no approval for the
    /// escaped publication in this device's journal the display name is
    /// the entry's own digest, exactly as the approval would name it.
    #[test]
    fn the_listing_falls_back_to_the_digest_for_a_display_name() {
        let fixture = Fixture::new("list-name-fallback");
        let digest = fixture.plant_recovery(PDF, PublicationIdentity(Sha256Digest([0x99; 32])));

        let listed = recovery_list_for(&fixture.outbox, &fixture.workspace(), &fixture.publication)
            .expect("listing");
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert_eq!(listed[0].display_name, digest.to_hex());
        assert_eq!(listed[0].class.as_deref(), Some("generated-heavy"));
    }

    /// **The class is the fact that decides the outcome, and the listing
    /// says it before the decision** (R1-004). The same bytes under a
    /// recovered name carrying an executable extension classify as
    /// `executable` (HAP-001-R4 is name-and-type, never content shape),
    /// and `recovery_approve` refuses an `executable` entry -- so an
    /// approval surface that did not carry the class showed nothing the
    /// user could have read the refusal off.
    #[test]
    fn a_recovered_name_that_classifies_as_executable_is_listed_as_executable() {
        let fixture = Fixture::new("list-class-executable");
        let escaped_from = PublicationIdentity(Sha256Digest([0x99; 32]));
        let digest = fixture.plant_recovery(PDF, escaped_from);
        fixture.plant_escaped_approval(escaped_from, "installer.exe");

        let listed = recovery_list_for(&fixture.outbox, &fixture.workspace(), &fixture.publication)
            .expect("listing");
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert_eq!(listed[0].display_name, "installer.exe");
        assert_eq!(
            listed[0].class.as_deref(),
            Some("executable"),
            "the listing says why the approval below refuses it: {listed:?}",
        );
        assert!(
            listed[0].usable,
            "the bytes are still what the entry is filed under; it is the class that refuses",
        );

        let error = recovery_approve_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Idle,
            &digest.to_hex(),
            Fixture::now(),
        )
        .expect_err("only a generated-heavy recovery entry may be approved");
        assert_eq!(error.code, ShellErrorCode::Refused);
    }

    /// An entry that yielded no facts from its own handle is classified
    /// by nothing: the class comes from the detected type and the size
    /// the probe read, and the probe read neither. `null` rather than a
    /// token standing in for a classification that was never made --
    /// the same discipline [`RecoveryEntryDto::size`] already follows.
    #[test]
    fn an_entry_that_yielded_no_facts_carries_no_class() {
        let fixture = Fixture::new("list-class-null");
        let digest = fixture.plant_recovery(PDF, PublicationIdentity(Sha256Digest([0x99; 32])));
        let entry = fixture.work_area().recovery_dir().join(digest.to_hex());
        std::fs::remove_file(&entry).expect("remove the planted file");
        std::fs::create_dir(&entry).expect("a directory at the entry's name");

        let listed = recovery_list_for(&fixture.outbox, &fixture.workspace(), &fixture.publication)
            .expect("listing");
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert!(!listed[0].usable);
        assert_eq!(listed[0].size, None);
        assert_eq!(listed[0].class, None, "{listed:?}");
        assert_eq!(
            listed[0].display_name,
            digest.to_hex(),
            "the name is still known -- it comes from the journal, not the handle",
        );
    }

    /// The listing reaches the policy store, so it reaches that store's
    /// failure too: a policy file that is present but cannot be parsed is
    /// `outbox-invalid`, the same token the approval answers with
    /// (R1-014 / R3-077 -- the approval's `# Errors` list omitted it).
    #[test]
    fn a_policy_that_cannot_be_loaded_is_outbox_invalid_for_both_commands() {
        let fixture = Fixture::new("policy-corrupt");
        fixture.plant_recovery(PDF, PublicationIdentity(Sha256Digest([0x99; 32])));
        let digest = Sha2Hasher::new().sha256(PDF);
        std::fs::write(
            fixture.project.path().join(POLICY_FILE_PATH),
            b"{ not json at all",
        )
        .expect("corrupt policy");

        let listing_error =
            recovery_list_for(&fixture.outbox, &fixture.workspace(), &fixture.publication)
                .expect_err("the policy decides the class, so the listing needs it");
        assert_eq!(listing_error.code, ShellErrorCode::OutboxInvalid);
        assert!(
            !listing_error.message.contains('/'),
            "{}",
            listing_error.message
        );

        let approve_error = recovery_approve_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Idle,
            &digest.to_hex(),
            Fixture::now(),
        )
        .expect_err("and the approval classifies too");
        assert_eq!(approve_error.code, ShellErrorCode::OutboxInvalid);
        assert!(
            !approve_error.message.contains('/'),
            "{}",
            approve_error.message
        );
    }

    /// HAP-001-R18: the listing reads the provenance from this device's
    /// journal -- which publication escaped, and when -- and the size from
    /// the entry's own handle. The file name carries a digest and nothing
    /// else, so it could never have said either.
    #[test]
    fn the_listing_reads_each_entrys_provenance_from_the_journal() {
        let fixture = Fixture::new("list");
        let escaped_from = PublicationIdentity(Sha256Digest([0x99; 32]));
        let digest = fixture.plant_recovery(PDF, escaped_from);

        let listed = recovery_list_for(&fixture.outbox, &fixture.workspace(), &fixture.publication)
            .expect("listing");
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert_eq!(listed[0].sha256, digest.to_hex());
        assert_eq!(listed[0].size, Some(PDF.len() as u64));
        assert_eq!(listed[0].recovered_from, escaped_from.to_hex());
        assert_eq!(listed[0].recovered_at, 1_725_782_399_000);
        assert!(listed[0].usable);
    }

    /// The digest is a **locator**. An entry whose bytes are no longer
    /// what it was filed as is listed as unusable rather than reported at
    /// a digest nothing there carries.
    #[test]
    fn an_entry_whose_bytes_are_not_its_name_is_listed_as_unusable() {
        let fixture = Fixture::new("list-unusable");
        let escaped_from = PublicationIdentity(Sha256Digest([0x99; 32]));
        let digest = fixture.plant_recovery(PDF, escaped_from);
        std::fs::write(
            fixture.work_area().recovery_dir().join(digest.to_hex()),
            b"something else entirely",
        )
        .expect("rewrite");

        let listed = recovery_list_for(&fixture.outbox, &fixture.workspace(), &fixture.publication)
            .expect("listing");
        assert_eq!(listed.len(), 1);
        assert!(
            !listed[0].usable,
            "the name still says one thing and the bytes another: {listed:?}",
        );
    }

    /// HAP-001-R18: a digest this device's journal never recorded as a
    /// recovery entry is `recovery-unknown`, whatever sits in the
    /// directory -- the provenance comes from the journal, never from the
    /// presence of a file.
    #[test]
    fn a_digest_the_journal_never_recorded_is_recovery_unknown() {
        let fixture = Fixture::new("unknown");
        let work_area = fixture.work_area();
        let digest = Sha2Hasher::new().sha256(PDF);
        std::fs::write(work_area.recovery_dir().join(digest.to_hex()), PDF).expect("planted");

        let error = recovery_approve_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Idle,
            &digest.to_hex(),
            Fixture::now(),
        )
        .expect_err("no journal line names it");
        assert_eq!(error.code, ShellErrorCode::RecoveryUnknown);
    }

    /// **A digest in a file name is not an identity check.** The entry
    /// this device journaled is rewritten in place, and the request binds
    /// honestly to the bytes that are *there now* -- so the caller's own
    /// binding agrees with the handle and only the name disagrees. The
    /// approval is refused anyway: the name is the digest of what was
    /// preserved, and an entry that no longer holds that is not the
    /// recovery entry the journal is talking about (HAP-001-R15, R16,
    /// R18). Without this check the re-publication would register
    /// substituted content under a journal line attesting to other bytes.
    #[test]
    fn bytes_that_are_not_the_name_they_are_filed_under_are_refused() {
        let fixture = Fixture::new("name-is-not-identity");
        let escaped_from = PublicationIdentity(Sha256Digest([0x99; 32]));
        let digest = fixture.plant_recovery(PDF, escaped_from);
        let substituted: &[u8] = b"%PDF-1.7\nnot the approved bytes\n";
        std::fs::write(
            fixture.work_area().recovery_dir().join(digest.to_hex()),
            substituted,
        )
        .expect("rewrite");

        let error = recovery_approve_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Idle,
            &digest.to_hex(),
            Fixture::now(),
        )
        .expect_err("the name is a locator, not proof");
        assert_eq!(error.code, ShellErrorCode::IntegrityMismatch);
        assert!(!error.message.contains('/'), "{}", error.message);
        // And nothing was recorded: an approval is the journal's own
        // record of one, so a refused approval leaves none.
        let work_area = fixture.work_area();
        let journal = JsonlPublicationJournal::open(&work_area).expect("journal");
        assert!(
            journal
                .replay()
                .expect("replay")
                .iter()
                .all(|entry| matches!(entry, JournalEntry::Step(_))),
            "no approval line was written",
        );
    }

    /// A digest the journal does not account for is `recovery-unknown`
    /// before anything is opened -- which is what a caller naming bytes
    /// this device never preserved gets, and is the case the removed
    /// second parameter was reaching for from the wrong side (R1-006).
    ///
    /// The test that stood here passed the entry's own name as `digest`
    /// and other bytes as `sha256`, so it asserted `integrity-mismatch`
    /// against a combination no caller could produce: the only surface
    /// that calls this reads both values off one field of one row.
    #[test]
    fn a_digest_naming_bytes_this_device_never_preserved_is_recovery_unknown() {
        let fixture = Fixture::new("unpreserved-digest");
        fixture.plant_recovery(PDF, PublicationIdentity(Sha256Digest([0x99; 32])));

        let error = recovery_approve_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Idle,
            &"ab".repeat(32),
            Fixture::now(),
        )
        .expect_err("the journal names no such entry");
        assert_eq!(error.code, ShellErrorCode::RecoveryUnknown);
        assert!(!error.message.contains('/'), "{}", error.message);
    }

    /// HAP-001-R18 and D3: the approval names no run, carries the
    /// publication its bytes were recovered from as a location fact, and
    /// **no standing approval policy covers it** -- asserted against a
    /// policy that would otherwise cover this class and size, so the
    /// exclusion is the rule's and not the absence of any policy.
    #[test]
    fn the_approval_names_no_run_and_no_standing_policy_covers_it() {
        let fixture = Fixture::new("approve");
        let escaped_from = PublicationIdentity(Sha256Digest([0x99; 32]));
        let digest = fixture.plant_recovery(PDF, escaped_from);

        let approval = recovery_approve_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Idle,
            &digest.to_hex(),
            Fixture::now(),
        )
        .expect("approved");
        assert_eq!(approval.run_id, None);
        assert_eq!(
            approval.recovered_from.as_deref(),
            Some(escaped_from.to_hex().as_str()),
        );
        assert_eq!(
            approval.publication_id,
            derive_publication_identity(&Sha2Hasher::new(), &fixture.project_identity(), &digest)
                .to_hex(),
            "HAP-001-R23 derives the identity from the bytes that were read",
        );
        assert!(
            approval.handle_held,
            "held for the publication that follows"
        );

        // The recorded approval itself, against a policy that covers this
        // class under a cap far above this size.
        let work_area = fixture.work_area();
        let journal = JsonlPublicationJournal::open(&work_area).expect("journal");
        let recorded = journal
            .replay()
            .expect("replay")
            .into_iter()
            .find_map(|entry| match entry {
                JournalEntry::Approved(recorded) => Some(*recorded),
                JournalEntry::Step(_) => None,
            })
            .expect("the approval is the journal's own record of it");
        assert_eq!(
            recorded.source,
            ApprovalSource::Recovery {
                recovered_from: escaped_from,
            },
            "and it survives the journal, so a restart still knows what this is",
        );
        let standing = StandingApprovalPolicy {
            class: recorded.class,
            max_size: recorded.size + 1_000_000,
        };
        assert!(
            !standing.covers(&recorded),
            "HAP-001-R18: no standing policy covers a recovery entry",
        );
    }

    /// D9's first branch, which no test reached: the display name is the
    /// **escaped publication's**, taken from the approval this device's
    /// journal still carries for it.
    #[test]
    fn the_display_name_is_the_escaped_publications_when_the_journal_has_its_approval() {
        let fixture = Fixture::new("display-name-from-journal");
        let escaped_from = PublicationIdentity(Sha256Digest([0x99; 32]));
        let digest = fixture.plant_recovery(PDF, escaped_from);
        fixture.plant_escaped_approval(escaped_from, "quarterly report.pdf");

        let approval = recovery_approve_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Idle,
            &digest.to_hex(),
            Fixture::now(),
        )
        .expect("approved");
        assert_eq!(
            approval.display_name, "quarterly report.pdf",
            "the name the escaped publication was approved under",
        );
        assert_eq!(
            approval.class, "generated-heavy",
            "and the class the classifier gives that name and these bytes",
        );
    }

    /// D9's other branch, which every other fixture here takes silently:
    /// with no approval for the escaped publication in this device's
    /// journal, the display name is the entry's **own digest** -- honest
    /// about what is known rather than inventing a name.
    #[test]
    fn the_display_name_falls_back_to_the_digest_when_the_journal_has_no_approval() {
        let fixture = Fixture::new("display-name-fallback");
        let escaped_from = PublicationIdentity(Sha256Digest([0x99; 32]));
        let digest = fixture.plant_recovery(PDF, escaped_from);

        let approval = recovery_approve_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Idle,
            &digest.to_hex(),
            Fixture::now(),
        )
        .expect("approved");
        assert_eq!(approval.display_name, digest.to_hex());
        assert_eq!(approval.class, "generated-heavy");
    }

    /// **The recovered display name feeds the classifier**, so the branch
    /// above is not cosmetic: a name that carries an executable extension
    /// classifies as `executable` whatever the bytes are (HAP-001-R4 is
    /// name-and-type, never content shape), and an `executable` recovery
    /// entry is refused. The same bytes under the digest fallback are
    /// `generated-heavy` and approve, which is what makes this a branch
    /// and not a formatting choice.
    #[test]
    fn a_recovered_display_name_that_classifies_as_executable_is_refused() {
        let fixture = Fixture::new("display-name-classifies");
        let escaped_from = PublicationIdentity(Sha256Digest([0x99; 32]));
        let digest = fixture.plant_recovery(PDF, escaped_from);
        fixture.plant_escaped_approval(escaped_from, "installer.exe");

        let error = recovery_approve_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Idle,
            &digest.to_hex(),
            Fixture::now(),
        )
        .expect_err("only a generated-heavy recovery entry may be approved");
        assert_eq!(error.code, ShellErrorCode::Refused);
        assert!(!error.message.contains('/'), "{}", error.message);
    }

    /// The digest is parsed before anything is opened: the entry's name
    /// may be nothing but 64 hex characters, and the refusal names no
    /// path.
    #[test]
    fn a_digest_that_is_not_64_hex_characters_is_invalid_request() {
        let fixture = Fixture::new("bad-hex");
        fixture.plant_recovery(PDF, PublicationIdentity(Sha256Digest([0x99; 32])));
        for entry in [
            "not-hex",
            "ab",
            "",
            &"ab".repeat(31),
            &format!("{}z", "ab".repeat(31)),
        ] {
            let error = recovery_approve_for(
                &fixture.outbox,
                &fixture.workspace(),
                &fixture.publication,
                &Idle,
                entry,
                Fixture::now(),
            )
            .expect_err("{entry}");
            assert_eq!(error.code, ShellErrorCode::InvalidRequest, "{entry}");
            assert!(!error.message.contains('/'), "{}", error.message);
        }
    }

    /// A project that declares no asset root has nowhere to publish to,
    /// and that is decided before the entry is opened at all.
    #[test]
    fn a_project_with_no_asset_root_is_destination_invalid() {
        let fixture = Fixture::new("no-asset-root");
        let digest = fixture.plant_recovery(PDF, PublicationIdentity(Sha256Digest([0x99; 32])));
        std::fs::write(
            fixture.project.path().join(POLICY_FILE_PATH),
            r#"{"schema": 1}"#,
        )
        .expect("a policy that declares no asset root");

        let error = recovery_approve_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Idle,
            &digest.to_hex(),
            Fixture::now(),
        )
        .expect_err("nowhere to publish to");
        assert_eq!(error.code, ShellErrorCode::DestinationInvalid);
    }

    /// A recovery entry that is not a regular file is `refused`, not
    /// `outbox-escape`: it is a file in the owner-only work area, and
    /// reusing the outbox's token for it would tell the surface something
    /// untrue about where the refusal happened (the deviation this slice
    /// records).
    #[test]
    fn an_entry_that_is_not_a_regular_file_is_refused() {
        let fixture = Fixture::new("not-regular");
        let digest = fixture.plant_recovery(PDF, PublicationIdentity(Sha256Digest([0x99; 32])));
        let entry = fixture.work_area().recovery_dir().join(digest.to_hex());
        std::fs::remove_file(&entry).expect("remove the planted file");
        std::fs::create_dir(&entry).expect("a directory at the entry's name");

        let error = recovery_approve_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Idle,
            &digest.to_hex(),
            Fixture::now(),
        )
        .expect_err("not a regular file");
        assert_eq!(error.code, ShellErrorCode::Refused);
        assert!(!error.message.contains('/'), "{}", error.message);

        // And the listing says the same thing its own way.
        let listed = recovery_list_for(&fixture.outbox, &fixture.workspace(), &fixture.publication)
            .expect("listing");
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].usable);
        assert_eq!(listed[0].size, None);
    }

    /// A second name for the same bytes is `refused`, and the listing
    /// reports it unusable with **no size** although the entry opened as
    /// a regular file perfectly well -- which is the case the `size: null`
    /// wording had to be corrected for.
    ///
    /// Unix only: the link count is read from the handle there, and `std`
    /// offers no by-handle equivalent on Windows, so `link_count` returns
    /// `None` and this refusal cannot be made at all (the residual this
    /// slice widened by one site).
    #[cfg(unix)]
    #[test]
    fn an_entry_with_a_link_count_above_one_is_refused() {
        let fixture = Fixture::new("linked");
        let digest = fixture.plant_recovery(PDF, PublicationIdentity(Sha256Digest([0x99; 32])));
        let recovery = fixture.work_area().recovery_dir();
        std::fs::hard_link(recovery.join(digest.to_hex()), recovery.join("second-name"))
            .expect("a second name for the same bytes");

        let error = recovery_approve_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Idle,
            &digest.to_hex(),
            Fixture::now(),
        )
        .expect_err("the link count is above one");
        assert_eq!(error.code, ShellErrorCode::Refused);
        assert!(!error.message.contains('/'), "{}", error.message);

        let listed = recovery_list_for(&fixture.outbox, &fixture.workspace(), &fixture.publication)
            .expect("listing");
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].usable, "{listed:?}");
        assert_eq!(
            listed[0].size, None,
            "the entry opened as a regular file; it is the link count that refuses it",
        );
    }

    /// The paired residual for the refusal above, asserted rather than
    /// left implied: with no by-handle link count in `std` on this
    /// platform, `link_count` returns `None`, the probe cannot see the
    /// second name at all, and the entry is **approved**. HAP-001-R19
    /// requires this disclosed, not closed — it is the same residual
    /// `outbox-linked` already carries for an outbox entry and for a
    /// misplaced file, now at a third site.
    ///
    /// Its unix counterpart above is what says this body is really
    /// checked: with this gate flipped to `#[cfg(unix)]` it fails on
    /// Linux, where the link count exists and refuses the approval.
    ///
    /// The second link may or may not exist on this platform (re-review,
    /// R3-108, following this test's own sibling in `publication.rs`):
    /// `CreateHardLinkW` needs NTFS or `ReFS` on a single volume and is
    /// refused on FAT, `exFAT`, or a mapped network temp, and the fixture
    /// sits under the system temp directory. This test asserts the
    /// *absence* of link detection, so a link that could not be created
    /// costs it nothing -- the approval must succeed and the entry must
    /// list as usable either way -- and a fixture the filesystem refuses
    /// must not be read as a product failure on the one CI leg this
    /// repository cannot reproduce locally.
    #[cfg(not(unix))]
    #[test]
    fn a_second_name_for_a_recovery_entry_cannot_be_refused_without_a_link_count() {
        let fixture = Fixture::new("linked-residual");
        let digest = fixture.plant_recovery(PDF, PublicationIdentity(Sha256Digest([0x99; 32])));
        let recovery = fixture.work_area().recovery_dir();
        // Tolerated, never required: see the note above.
        let _ = std::fs::hard_link(recovery.join(digest.to_hex()), recovery.join("second-name"));

        let approval = recovery_approve_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Idle,
            &digest.to_hex(),
            Fixture::now(),
        )
        .expect("the second name is invisible to the probe, so nothing refuses it");
        assert_eq!(approval.sha256_short, digest.to_hex()[..8]);

        let listed = recovery_list_for(&fixture.outbox, &fixture.workspace(), &fixture.publication)
            .expect("listing");
        assert_eq!(listed.len(), 1);
        assert!(
            listed[0].usable,
            "and the listing cannot see it either: {listed:?}",
        );
    }

    /// The approval id is derived from the publication identity and the
    /// instant, so approving the same entry twice at the same instant
    /// derives the same id -- which the journal already carries. Refused
    /// as `invalid-request` rather than recording a second line under an
    /// id that is meant to name one approval.
    #[test]
    fn a_second_approval_at_the_same_instant_collides_and_is_refused() {
        let fixture = Fixture::new("collision");
        let digest = fixture.plant_recovery(PDF, PublicationIdentity(Sha256Digest([0x99; 32])));
        let approve = || {
            recovery_approve_for(
                &fixture.outbox,
                &fixture.workspace(),
                &fixture.publication,
                &Idle,
                &digest.to_hex(),
                Fixture::now(),
            )
        };
        approve().expect("the first approval");
        let error = approve().expect_err("the derived id is already recorded");
        assert_eq!(error.code, ShellErrorCode::InvalidRequest);
        assert!(!error.message.contains('/'), "{}", error.message);
    }

    /// **A recovery entry is device-local, and this is the decision, not
    /// an oversight.** The work area and its journal are one per device,
    /// not one per project, so an entry an escape wrote while project A
    /// was active is listed from project B and can be approved into B.
    ///
    /// That is correct under HAP-001-R18 as this slice reads it: what is
    /// preserved is *bytes the device held*, the re-publication derives
    /// its identity from the **active** project and the bytes it reads
    /// (D9), and `recoveredFrom` is a location fact only -- the module
    /// doc already says the publication it names "may no longer exist
    /// anywhere". So B registers these bytes under B's own identity, and
    /// `recoveredFrom` names a publication B's catalog does not hold,
    /// which is the same thing it would name after A's project was
    /// deleted. Scoping the listing per project would need the escape
    /// step to carry a project identity, which it does not, and is a
    /// change to a persisted shape rather than a fix -- so it is
    /// disclosed here and in the spike log instead.
    #[test]
    fn a_recovery_entry_is_device_local_and_reaches_a_second_project() {
        let device = TempDir::new("two-projects-device");
        let first = TempDir::new("two-projects-a");
        let second = TempDir::new("two-projects-b");
        for project in [&first, &second] {
            let policy_path = project.path().join(POLICY_FILE_PATH);
            std::fs::create_dir_all(policy_path.parent().expect("parent")).expect("mkdir");
            std::fs::write(&policy_path, r#"{"schema": 1, "assetRootId": "main"}"#)
                .expect("policy");
        }
        // One device, one work area, one journal -- two projects.
        let fixture = Fixture {
            project: first,
            publication: PublicationState::under(device.path()),
            device,
            outbox: OutboxState::new(),
        };
        let escaped_from = PublicationIdentity(Sha256Digest([0x99; 32]));
        let digest = fixture.plant_recovery(PDF, escaped_from);

        let workspace_b = WorkspaceRoot::new(second.path()).expect("workspace");
        let listed = recovery_list_for(&fixture.outbox, &workspace_b, &fixture.publication)
            .expect("listing");
        assert_eq!(
            listed.len(),
            1,
            "the entry is listed from the second project: {listed:?}",
        );

        let approval = recovery_approve_for(
            &fixture.outbox,
            &workspace_b,
            &fixture.publication,
            &Idle,
            &digest.to_hex(),
            Fixture::now(),
        )
        .expect("approved into the second project");
        let project_b = omnifrons_app::content_hasher::derive_project_identity(
            &Sha2Hasher::new(),
            &workspace_b,
        );
        assert_eq!(
            approval.publication_id,
            derive_publication_identity(&Sha2Hasher::new(), &project_b, &digest).to_hex(),
            "the identity is the second project's, never the first's",
        );
        assert_eq!(
            approval.recovered_from.as_deref(),
            Some(escaped_from.to_hex().as_str()),
            "and `recoveredFrom` names a publication this project's catalog does not hold",
        );
        assert_ne!(
            approval.publication_id,
            escaped_from.to_hex(),
            "which is exactly why it is a location fact and never the identity",
        );
    }

    /// The approval is a write on the publication surface: frozen while a
    /// supervised process runs, like every other one.
    #[test]
    fn an_approval_is_refused_while_a_run_is_active() {
        let fixture = Fixture::new("run-active");
        let digest = fixture.plant_recovery(PDF, PublicationIdentity(Sha256Digest([0x99; 32])));
        let error = recovery_approve_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Busy,
            &digest.to_hex(),
            Fixture::now(),
        )
        .expect_err("frozen");
        assert_eq!(error.code, ShellErrorCode::RunActive);
    }

    /// **A recovery entry is re-opened under `recovery/`, never under the
    /// outbox.** The end state a restart -- or HAP-001 D22's held-handle
    /// cap -- leaves is arranged directly: the held handle is taken out of
    /// the table and dropped, so the publication has to find the bytes for
    /// itself. Its name is a 64-hex digest, which is a perfectly valid
    /// outbox-root entry name, so a re-open that went through the outbox
    /// path would look for it in the project and refuse.
    #[test]
    fn a_recovery_approval_whose_handle_was_released_re_opens_under_the_work_area() {
        let fixture = Fixture::new("reopen");
        let escaped_from = PublicationIdentity(Sha256Digest([0x99; 32]));
        let digest = fixture.plant_recovery(PDF, escaped_from);
        let approval = recovery_approve_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Idle,
            &digest.to_hex(),
            Fixture::now(),
        )
        .expect("approved");

        // The state the cap, an eviction, or a restart leaves: nothing
        // holds the handle any more.
        {
            let id =
                omnifrons_domain::publication::ArtifactApprovalId::from_hex(&approval.approval_id)
                    .expect("hex");
            let mut runs = fixture.outbox.runs.lock().expect("table");
            drop(runs.take_approved(id).expect("it was held"));
        }

        let published = publish_approved(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Idle,
            &approval.approval_id,
            &mut |_| {},
        )
        .expect("the bytes are found under the work area, not the outbox");
        assert_eq!(published.state, ArtifactStateTag::Registered);
    }

    /// The existing `artifact_publish` carries it unchanged: the bytes
    /// come from the recovery entry, the record is registered, and the
    /// entry is **still there** afterwards (HAP-001-R18, R39 -- this
    /// contract defines no deletion path for one).
    #[test]
    fn publishing_a_recovery_approval_registers_it_and_keeps_the_entry() {
        let fixture = Fixture::new("publish");
        let escaped_from = PublicationIdentity(Sha256Digest([0x99; 32]));
        let digest = fixture.plant_recovery(PDF, escaped_from);
        let entry_path = fixture.work_area().recovery_dir().join(digest.to_hex());

        let approval = recovery_approve_for(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Idle,
            &digest.to_hex(),
            Fixture::now(),
        )
        .expect("approved");

        let mut frames: Vec<ArtifactStateFrame> = Vec::new();
        let published = publish_approved(
            &fixture.outbox,
            &fixture.workspace(),
            &fixture.publication,
            &Idle,
            &approval.approval_id,
            &mut |frame| frames.push(frame),
        )
        .expect("published");

        assert_eq!(published.state, ArtifactStateTag::Registered);
        assert_eq!(published.publication_id, approval.publication_id);
        assert!(
            entry_path.exists(),
            "the recovery entry is kept after a successful re-publication",
        );
        // The record is registered as unattributed: nothing about a
        // recovery entry attributes it to a run (HAP-001-R36).
        let records = JsonlCatalogStore::open(&fixture.workspace())
            .expect("store")
            .list()
            .expect("catalog reads");
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].provenance.producer,
            omnifrons_domain::publication::Producer::Unattributed { found_under: None },
        );
        // And the copy at the device asset path is the recovered bytes.
        let copy = fixture
            .device
            .path()
            .join(crate::publication_state::ASSET_ROOTS_DIR)
            .join("main")
            .join(approval.publication_id.clone());
        assert_eq!(std::fs::read(&copy).expect("the published copy"), PDF);
        assert_eq!(
            frames
                .iter()
                .map(|frame| match frame {
                    ArtifactStateFrame::ArtifactState { state, .. } => *state,
                })
                .collect::<Vec<ArtifactStateTag>>(),
            vec![
                ArtifactStateTag::PublishedLocal,
                ArtifactStateTag::Registered
            ],
        );
    }
}
