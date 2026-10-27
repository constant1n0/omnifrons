//! The `apply` service (ADR-0005 decision 3, sub-slice 2d): carries out
//! the plan a caller approved, one file at a time.
//!
//! [`apply`] takes only the plan id. It plans again from what is on disk
//! now with the same options ([`crate::definition_plan::plan_with_writes`]),
//! so the bytes it writes are never the caller's: a plan whose id differs
//! from the recomputed one is [`DefinitionError::StalePlan`] and nothing
//! is written. A root that is already canonical plans no operation and
//! applying it is [`ApplyOutcome::NoOp`]: no write, no snapshot, no
//! record. Applying the same plan twice therefore changes nothing the
//! second time: the second plan has no operations, and the first plan's
//! id is stale by then.
//!
//! # One operation
//!
//! For each operation, in plan order (`AGENTS.md`, the wrappers, the
//! `skills` block):
//!
//! 1. every input is read again and compared with what the plan
//!    recorded, except that a file this run already wrote must now carry
//!    the digest that write produced;
//! 2. the file's current bytes, when it exists, are snapshotted under
//!    its agent and name; the snapshot store skips a state it already
//!    holds, so an unchanged file produces no new snapshot;
//! 3. the file is written through the [`DefinitionWriter`], bound to the
//!    fingerprint the plan recorded, with the permission rule of the
//!    prototype: `AGENTS.md` and a wrapper take no wider bits than the
//!    file their text came from, and the block rewrite keeps the file's
//!    own;
//! 4. every input is read again, so a concurrent edit of any identity
//!    file the plan fingerprints, not only the one just written, stops
//!    the run before the next write.
//!
//! The re-read is one [`crate::definition_tree::DefinitionTree`]
//! snapshot of the whole workspace per check, compared as a whole: a
//! changed digest, a file that appeared, and a file that vanished are all
//! stale. Only identity files are fingerprinted: the `skills` block is
//! written from the bytes planned at the start, so a skill file added or
//! edited during the run is not detected, the block does not index it,
//! and the next plan picks it up (the ADR's stated cost). A file written twice in one run (`AGENTS.md`, then its block)
//! is snapshotted once, before its first write, and its record entry
//! keeps the first `before` and the last `after`.
//!
//! After the last operation the run stores one [`ApplyRecord`].
//!
//! # Interrupted runs
//!
//! Each file is published atomically, but the run is not one
//! transaction. Stopping after any write and before the record leaves
//! the files written so far in place, their snapshots in the store, and
//! no record. That happens when an input changes mid-run
//! ([`DefinitionError::StalePlan`]), when a later write fails
//! ([`DefinitionError::Write`]), when a snapshot cannot be stored (the
//! file it belongs to is not written), when the record cannot be stored
//! ([`DefinitionError::Record`], every file written), or when the process
//! dies. Nothing is rolled back; a snapshot is the way back.
//!
//! A rerun converges: it plans from the disk as it is, and a file already
//! written to its planned bytes needs no operation, so the new plan is
//! smaller (or empty) and is applied under its own id.
//!
//! Each state a crash can leave is recognised: the root as it was
//! (`ClaudeOnly` or `Conflict`), `AGENTS.md` written and the wrapper not
//! (`IdenticalCopies`), or both identity files canonical with only the
//! `skills` block pending. The tests `definition_apply_crash_fs` (every
//! crash point over the real adapters, each followed by a rerun) and
//! `a_crash_before_the_second_write_leaves_one_file_and_no_record` (the
//! same contract over the in-memory doubles) are the proof. A rerun
//! names a winning side with `canonical_from` only while the files still
//! disagree: the planner refuses it for any other state.
//!
//! One state is not a crash point of any write: every planned file
//! written and the record never stored (the process dies after the last
//! write, or the record store fails). It is a bounded residual, left open
//! by the decision record's crash-window question. From there a rerun
//! plans from the disk, finds nothing to do, and returns
//! [`ApplyOutcome::NoOp`], so that apply stays unrecorded while its
//! snapshots remain as evidence. Recording a provisional manifest before
//! the writes is the alternative, for a later slice. The test
//! `a_run_that_wrote_every_file_but_never_recorded_reruns_as_a_no_op_and_stays_unrecorded`
//! pins it.

use std::collections::BTreeMap;

use omnifrons_domain::definition::{FileShape, IdentityFile, IdentityKind};
use omnifrons_domain::definition_plan::{
    InputFingerprint, Operation, Plan, PlanId, PlanOptions, PlanRefusal,
};
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::publication::ProjectIdentity;

use crate::apply_record::{AppliedFile, ApplyRecord, ApplyRecordId, ApplyRecordStore};
use crate::clock::Clock;
use crate::content_hasher::{ContentHasher, derive_project_identity};
use crate::definition::{DefinitionError, DefinitionPorts, load_target};
use crate::definition_plan::{PlanOutcome, PlannedWrite, input_fingerprints, plan_with_writes};
use crate::definition_tree::DefinitionTree;
use crate::definition_writer::{DefinitionWriteError, DefinitionWriter, ModePolicy, WriteRequest};
use crate::harness_adapter::WorkspaceRoot;
use crate::snapshot_store::{
    DefinitionSnapshotManifest, DefinitionSnapshotStore, DefinitionSubject,
    SNAPSHOT_SCHEMA_VERSION, SnapshotId, definition_snapshot_id_preimage,
};

/// The ports [`apply`] runs over.
pub struct ApplyPorts<'a> {
    /// The read-only pass over the workspace, used to plan and to check
    /// the inputs again.
    pub tree: &'a dyn DefinitionTree,
    /// SHA-256 over file bytes.
    pub hasher: &'a dyn ContentHasher,
    /// Publishes each planned file.
    pub writer: &'a dyn DefinitionWriter,
    /// Keeps the snapshot of each file before it is overwritten.
    pub snapshots: &'a mut dyn DefinitionSnapshotStore,
    /// Keeps the record of the apply.
    pub records: &'a dyn ApplyRecordStore,
    /// The time of each snapshot and of the record.
    pub clock: &'a dyn Clock,
}

/// What [`apply`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// Every operation ran and the apply was recorded.
    Applied {
        /// The plan that was applied.
        plan_id: PlanId,
        /// The stored name of the apply record.
        record: ApplyRecordId,
        /// Every file the apply wrote, as recorded.
        files: Vec<AppliedFile>,
    },
    /// The plan has no operation: nothing was written, snapshotted, or
    /// recorded.
    NoOp {
        /// The plan that needed nothing.
        plan_id: PlanId,
    },
    /// The root cannot be adapted; nothing was written.
    Refused(PlanRefusal),
}

/// Applies the plan `expected` for `target` (a clean, workspace-relative
/// POSIX path; `None` means the workspace root) under `options`.
///
/// # Errors
///
/// The errors [`crate::definition_plan::plan`] documents, plus
/// [`DefinitionError::StalePlan`] when the recomputed plan id differs
/// from `expected` or an input changes during the run;
/// [`DefinitionError::Snapshot`] when a snapshot cannot be stored;
/// [`DefinitionError::Write`] for a write that fails for any other
/// reason; and [`DefinitionError::Record`] when the record cannot be
/// stored. See the module documentation for what each leaves behind.
///
/// # Panics
///
/// If the planner returns a different number of writes than operations,
/// which is a defect in the planner, not a condition of the workspace.
pub fn apply(
    ports: &mut ApplyPorts<'_>,
    workspace: &WorkspaceRoot,
    target: Option<&str>,
    options: &PlanOptions,
    expected: &PlanId,
) -> Result<ApplyOutcome, DefinitionError> {
    let definition = DefinitionPorts {
        tree: ports.tree,
        hasher: ports.hasher,
    };
    let (outcome, writes) = plan_with_writes(&definition, workspace, target, options)?;
    let plan = match outcome {
        PlanOutcome::Refused(refusal) => return Ok(ApplyOutcome::Refused(refusal)),
        PlanOutcome::Planned(plan) => plan,
    };
    if plan.id != *expected {
        return Err(DefinitionError::StalePlan {
            expected: *expected,
            actual: Some(plan.id),
        });
    }
    if plan.operations.is_empty() {
        return Ok(ApplyOutcome::NoOp { plan_id: plan.id });
    }
    let mut run = Run {
        project: derive_project_identity(ports.hasher, workspace),
        inputs: plan.inputs.clone(),
        files: Vec::new(),
        ports,
        workspace,
        plan: &plan,
    };
    assert_eq!(
        plan.operations.len(),
        writes.len(),
        "the planner pairs every operation with its bytes"
    );
    for (operation, write) in plan.operations.iter().zip(&writes) {
        run.step(operation, write)?;
    }
    run.finish()
}

/// One apply in progress.
struct Run<'r, 'p> {
    ports: &'r mut ApplyPorts<'p>,
    workspace: &'r WorkspaceRoot,
    plan: &'r Plan,
    project: ProjectIdentity,
    /// What every input must look like right now.
    inputs: BTreeMap<String, InputFingerprint>,
    files: Vec<AppliedFile>,
}

impl Run<'_, '_> {
    fn step(&mut self, operation: &Operation, write: &PlannedWrite) -> Result<(), DefinitionError> {
        let current = self.check_inputs()?;
        let path = operation.path();
        let first_touch = !self.files.iter().any(|file| file.path == path);
        let snapshot = if first_touch {
            self.snapshot(path, &current)?
        } else {
            None
        };
        self.publish(operation, write)?;
        self.inputs.insert(
            path.to_string(),
            InputFingerprint::Sha256(*operation.after()),
        );
        self.check_inputs()?;
        if let Some(file) = self.files.iter_mut().find(|file| file.path == path) {
            file.after = Some(*operation.after());
        } else {
            self.files.push(AppliedFile {
                path: path.to_string(),
                before: operation.before().clone(),
                after: Some(*operation.after()),
                snapshot,
            });
        }
        Ok(())
    }

    /// Reads every input again and fails as stale unless each one is
    /// what the run expects. Returns the identity files as read, so the
    /// snapshot that follows holds the bytes that were just checked.
    fn check_inputs(&self) -> Result<Vec<IdentityFile>, DefinitionError> {
        let definition = DefinitionPorts {
            tree: self.ports.tree,
            hasher: self.ports.hasher,
        };
        let loaded = load_target(&definition, self.workspace, &self.plan.agent)?;
        if input_fingerprints(self.ports.hasher, &loaded.identity_files) == self.inputs {
            Ok(loaded.identity_files)
        } else {
            Err(self.stale(None))
        }
    }

    fn stale(&self, actual: Option<PlanId>) -> DefinitionError {
        DefinitionError::StalePlan {
            expected: self.plan.id,
            actual,
        }
    }

    /// Snapshots the regular file `path` as `current` holds it, or
    /// returns `None` when there is nothing to keep.
    fn snapshot(
        &mut self,
        path: &str,
        current: &[IdentityFile],
    ) -> Result<Option<SnapshotId>, DefinitionError> {
        let Some(file) = current
            .iter()
            .find(|file| file.kind.file_name() == path && file.shape == FileShape::Regular)
        else {
            return Ok(None);
        };
        let subject = DefinitionSubject::new(&self.plan.agent, path).ok_or_else(|| {
            DefinitionError::Write(DefinitionWriteError::InvalidPath {
                path: within(&self.plan.agent, path),
            })
        })?;
        let digest = self.ports.hasher.sha256(&file.bytes);
        let taken_at = self.ports.clock.now();
        let preimage = definition_snapshot_id_preimage(&self.project, &subject, &digest, taken_at);
        let manifest = DefinitionSnapshotManifest {
            schema: SNAPSHOT_SCHEMA_VERSION,
            id: snapshot_id(self.ports.hasher.sha256(&preimage)),
            subject,
            existed: true,
            sha256: digest,
            size: file.bytes.len() as u64,
            taken_at,
            pinned: false,
        };
        self.ports
            .snapshots
            .record_definition(&self.project, manifest, &file.bytes)
            .map(Some)
            .map_err(DefinitionError::Snapshot)
    }

    fn publish(&self, operation: &Operation, write: &PlannedWrite) -> Result<(), DefinitionError> {
        let agent = &self.plan.agent;
        let path = within(agent, operation.path());
        let mode = mode_policy(agent, operation);
        let request = WriteRequest {
            path: &path,
            bytes: &write.bytes,
            expected: operation.before(),
            mode: &mode,
        };
        match self.ports.writer.write(self.workspace, &request) {
            Ok(written) if written == *operation.after() => Ok(()),
            Ok(_) => Err(DefinitionError::Write(
                DefinitionWriteError::ReadBackMismatch { path },
            )),
            Err(DefinitionWriteError::Stale { .. }) => Err(self.stale(None)),
            Err(error) => Err(DefinitionError::Write(error)),
        }
    }

    fn finish(self) -> Result<ApplyOutcome, DefinitionError> {
        let record = ApplyRecord {
            plan_id: self.plan.id,
            workspace: self.plan.workspace.clone(),
            agent: self.plan.agent.clone(),
            taken_at: self.ports.clock.now(),
            files: self.files.clone(),
            operations: self.plan.operations.clone(),
        };
        let id = self
            .ports
            .records
            .record(&self.project, &record)
            .map_err(DefinitionError::Record)?;
        Ok(ApplyOutcome::Applied {
            plan_id: self.plan.id,
            record: id,
            files: self.files,
        })
    }
}

/// The permission rule for `operation`: a file made from a source takes
/// no wider bits than that source; the block rewrite keeps its own.
fn mode_policy(agent: &str, operation: &Operation) -> ModePolicy {
    match operation {
        Operation::WriteAgents { source, .. } => ModePolicy::NoWiderThan {
            source: within(agent, source.file_name()),
        },
        Operation::WriteWrapper { .. } => ModePolicy::NoWiderThan {
            source: within(agent, IdentityKind::Agents.file_name()),
        },
        Operation::WriteGenerated { .. } | Operation::WriteBlock { .. } => ModePolicy::Keep,
    }
}

/// `file` inside the agent directory `agent`, relative to the workspace.
fn within(agent: &str, file: &str) -> String {
    if agent == "." {
        file.to_string()
    } else {
        format!("{agent}/{file}")
    }
}

/// The snapshot id: the first 8 bytes of the preimage's digest,
/// big-endian, as [`crate::content_hasher::derive_snapshot_id`] does for
/// guidance snapshots.
fn snapshot_id(digest: Sha256Digest) -> SnapshotId {
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest.0[..8]);
    SnapshotId(u64::from_be_bytes(bytes))
}
