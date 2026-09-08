//! Managed state for the outbox surface (spike slice 5, HAP-001 launch
//! side): the policy store, the run-subdirectory preparer, the inventory,
//! a bounded table of per-run records -- the prepared subdirectory and its
//! held handle, the proposals the run surfaced, and, once the run ended,
//! its candidates with their held handles (capped by HAP-001 D22) -- and
//! the run-id minting. The table is per *active workspace within a shell
//! session*: picking a different workspace clears it
//! (`OutboxState::forget_runs`), so one project's runs and handles never
//! count against the next project's budget.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use omnifrons_adapters::{FsOutboxInventory, FsRunOutboxPreparer, JsonOutboxPolicyStore};
use omnifrons_app::ProcessId;
use omnifrons_app::outbox_policy::OutboxPolicy;
use omnifrons_app::run_outbox::{
    Candidate, MAX_HELD_HANDLES, PreparedRunSubdirectory, cap_held_handles,
};
use omnifrons_domain::outbox::{PublishProposal, RunId};

/// The maximum number of run records remembered at once. Bounded so a
/// long session cannot grow the table -- and the handles the records hold
/// -- without limit; the oldest record is evicted first, releasing its
/// handles (its entries stay in the outbox, to be found by the next
/// whole-outbox inventory as unattributed).
pub const MAX_RUN_RECORDS: usize = 64;

/// The maximum number of proposed entries recorded per run across every
/// `artifact.publish` proposal it surfaces. Entries beyond it are counted
/// ([`RunRecord::dropped_proposal_entries`]) and surfaced as one
/// diagnostic at run end, never silently dropped, so a harness cannot grow
/// the shell's memory without bound by proposing endlessly.
pub const MAX_PROPOSED_ENTRIES: usize = 1024;

/// The process-wide run sequence: every run id minted in this process
/// takes the next value, whichever state or table asked, so two ids minted
/// in one process differ even when the clock reports the same instant --
/// Windows's `SystemTime` has 100 ns resolution and no sub-second
/// guarantee, and two shells' worth of state in one process would
/// otherwise restart the count.
static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// The next value of [`RUN_SEQUENCE`].
fn next_run_sequence() -> u64 {
    RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

/// Mint a fresh run id from the wall clock and the process-wide sequence
/// ([`next_run_sequence`]); see [`mint_run_id`] for the shape.
#[must_use]
pub fn mint_next_run_id() -> RunId {
    mint_run_id(SystemTime::now(), next_run_sequence())
}

/// Mint a run id from an instant and a sequence number:
/// `run-<seconds>-<nanoseconds>-<sequence>`, unique on this device for
/// the product's lifetime -- the sequence part, taken from
/// [`RUN_SEQUENCE`] by [`mint_next_run_id`], keeps two ids apart even when
/// their instants are equal -- and a valid single path component by
/// construction (`omnifrons_domain::outbox::RunId`).
///
/// # Panics
///
/// Panics if the minted token fails `RunId`'s own validation -- it is
/// digits and dashes only, so this is a construction invariant, not a
/// runtime condition.
#[must_use]
pub fn mint_run_id(now: SystemTime, sequence: u64) -> RunId {
    let since_epoch = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    RunId::new(format!(
        "run-{}-{:09}-{sequence}",
        since_epoch.as_secs(),
        since_epoch.subsec_nanos()
    ))
    .expect("a minted run id is digits and dashes only")
}

/// One adapter launch's outbox record.
#[derive(Debug)]
pub struct RunRecord {
    subdirectory: PreparedRunSubdirectory,
    policy: OutboxPolicy,
    proposals: Vec<PublishProposal>,
    recorded_entries: usize,
    dropped_entries: u64,
    candidates: Option<Vec<Candidate>>,
}

impl RunRecord {
    /// A record for a freshly prepared run subdirectory under the policy
    /// in force at launch.
    #[must_use]
    pub const fn new(subdirectory: PreparedRunSubdirectory, policy: OutboxPolicy) -> Self {
        Self {
            subdirectory,
            policy,
            proposals: Vec::new(),
            recorded_entries: 0,
            dropped_entries: 0,
            candidates: None,
        }
    }

    /// The run's id.
    #[must_use]
    pub const fn run_id(&self) -> &RunId {
        &self.subdirectory.run_id
    }

    /// The policy in force when the run was launched.
    #[must_use]
    pub const fn policy(&self) -> &OutboxPolicy {
        &self.policy
    }

    /// Every publish proposal the run surfaced so far.
    #[must_use]
    pub fn proposals(&self) -> &[PublishProposal] {
        &self.proposals
    }

    /// Record one publish proposal the run surfaced, up to
    /// [`MAX_PROPOSED_ENTRIES`] entries per run: a proposal crossing the cap
    /// keeps its first entries up to the remaining slots, and every entry
    /// beyond is counted, never silently dropped. A proposal that records
    /// nothing is not kept.
    pub fn push_proposal(&mut self, mut proposal: PublishProposal) {
        let remaining = MAX_PROPOSED_ENTRIES.saturating_sub(self.recorded_entries);
        if proposal.entries.len() > remaining {
            let excess = proposal.entries.len() - remaining;
            proposal.entries.truncate(remaining);
            self.dropped_entries = self
                .dropped_entries
                .saturating_add(u64::try_from(excess).unwrap_or(u64::MAX));
        }
        if proposal.entries.is_empty() {
            return;
        }
        self.recorded_entries += proposal.entries.len();
        self.proposals.push(proposal);
    }

    /// How many proposed entries are recorded across the run's proposals.
    #[must_use]
    pub const fn recorded_proposal_entries(&self) -> usize {
        self.recorded_entries
    }

    /// How many proposed entries were counted beyond the cap.
    #[must_use]
    pub const fn dropped_proposal_entries(&self) -> u64 {
        self.dropped_entries
    }

    /// How many handles this record holds.
    fn held_handles(&self) -> usize {
        self.candidates.as_deref().map_or(0, |candidates| {
            candidates
                .iter()
                .filter(|candidate| candidate.handle.is_some())
                .count()
        })
    }

    /// A duplicate of the held run-subdirectory handle, so an inventory
    /// can run outside the table's lock.
    ///
    /// # Errors
    ///
    /// Returns any `io::Error` the handle duplication can produce.
    pub fn subdirectory_clone(&self) -> std::io::Result<PreparedRunSubdirectory> {
        self.subdirectory.try_clone()
    }

    /// The run-end inventory, once taken.
    #[must_use]
    pub fn candidates(&self) -> Option<&[Candidate]> {
        self.candidates.as_deref()
    }

    /// Store the run-end inventory. Private: [`RunTable::store_candidates`]
    /// is the way in, so the project-wide handle cap is always applied.
    fn set_candidates(&mut self, candidates: Vec<Candidate>) {
        self.candidates = Some(candidates);
    }
}

/// The bounded per-run table: oldest evicted once [`MAX_RUN_RECORDS`] is
/// exceeded.
#[derive(Debug, Default)]
pub struct RunTable {
    order: VecDeque<ProcessId>,
    entries: HashMap<ProcessId, RunRecord>,
}

impl RunTable {
    /// Remember `record` under `id`, evicting the oldest record if the
    /// table is now over [`MAX_RUN_RECORDS`].
    pub fn insert(&mut self, id: ProcessId, record: RunRecord) {
        if self.entries.insert(id, record).is_none() {
            self.order.push_back(id);
        }
        while self.order.len() > MAX_RUN_RECORDS {
            if let Some(evicted) = self.order.pop_front() {
                self.entries.remove(&evicted);
            }
        }
    }

    /// The record for `id`, if remembered.
    #[must_use]
    pub fn get(&self, id: ProcessId) -> Option<&RunRecord> {
        self.entries.get(&id)
    }

    /// The record for `id`, mutably, if remembered.
    pub fn get_mut(&mut self, id: ProcessId) -> Option<&mut RunRecord> {
        self.entries.get_mut(&id)
    }

    /// The record whose run id is `run_id`, if remembered.
    #[must_use]
    pub fn find_by_run_id(&self, run_id: &RunId) -> Option<&RunRecord> {
        self.entries
            .values()
            .find(|record| record.run_id() == run_id)
    }

    /// How many candidate handles are held across every remembered run --
    /// the figure HAP-001 D22's cap bounds, per active workspace within a
    /// shell session (the table is cleared when the workspace changes).
    #[must_use]
    pub fn held_handles(&self) -> usize {
        self.entries.values().map(RunRecord::held_handles).sum()
    }

    /// Forget every remembered run, releasing every held handle with it:
    /// the active workspace changed, and the records belonged to the
    /// previous one. Returns how many records were forgotten.
    pub fn clear(&mut self) -> usize {
        let forgotten = self.entries.len();
        self.entries.clear();
        self.order.clear();
        forgotten
    }

    /// Store `candidates` as `id`'s run-end inventory, keeping at most the
    /// remainder of [`MAX_HELD_HANDLES`] held across every remembered run
    /// of the active workspace -- every other run's held handles counted
    /// first, in inventory order -- and releasing the rest while the
    /// entries stay `candidate` (HAP-001-R17, D22). Returns how many
    /// handles were released; if `id` is not remembered (evicted between
    /// the inventory's phases, or forgotten by a workspace change) the
    /// candidates are handed back untouched as the `Err`, so the caller
    /// releases them explicitly and says so rather than losing them
    /// silently.
    pub fn store_candidates(
        &mut self,
        id: ProcessId,
        mut candidates: Vec<Candidate>,
    ) -> Result<usize, Vec<Candidate>> {
        let Some(held_by_this_run) = self.entries.get(&id).map(RunRecord::held_handles) else {
            return Err(candidates);
        };
        let held_elsewhere = self.held_handles().saturating_sub(held_by_this_run);
        let released = cap_held_handles(
            &mut candidates,
            MAX_HELD_HANDLES.saturating_sub(held_elsewhere),
        );
        let Some(record) = self.entries.get_mut(&id) else {
            return Err(candidates);
        };
        record.set_candidates(candidates);
        Ok(released)
    }
}

/// This shell's managed state for the outbox surface.
pub struct OutboxState {
    /// Loads the active project's classification policy.
    pub policy_store: JsonOutboxPolicyStore,
    /// Prepares run subdirectories and opens the outbox by handle.
    pub preparer: FsRunOutboxPreparer,
    /// Lists directories without following links.
    pub inventory: FsOutboxInventory,
    /// The per-run records, shared with each launch's forwarder thread.
    pub runs: Arc<Mutex<RunTable>>,
}

impl OutboxState {
    /// Build the managed state with an empty run table.
    #[must_use]
    pub fn new() -> Self {
        Self {
            policy_store: JsonOutboxPolicyStore::new(),
            preparer: FsRunOutboxPreparer::new(),
            inventory: FsOutboxInventory::new(),
            runs: Arc::new(Mutex::new(RunTable::default())),
        }
    }

    /// The active workspace changed: forget every remembered run and
    /// release its handles. Returns how many records were forgotten.
    pub fn forget_runs(&self) -> usize {
        self.runs
            .lock()
            .expect("run table mutex poisoned by a prior panic")
            .clear()
    }
}

impl Default for OutboxState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_PROPOSED_ENTRIES, MAX_RUN_RECORDS, RunRecord, RunTable, mint_next_run_id, mint_run_id,
    };
    use omnifrons_app::ProcessId;
    use std::time::{Duration, SystemTime};

    /// Minted run ids validate as a single path component and differ across
    /// sequence numbers and instants.
    #[test]
    fn minted_run_ids_are_valid_and_distinct() {
        // Not a whole number of minutes, so the instant reads as the epoch
        // seconds it is (clippy would otherwise ask for a larger unit).
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_725_782_401);
        let a = mint_run_id(t0, 1);
        let b = mint_run_id(t0, 2);
        // A whole second apart, deliberately: `SystemTime` has 100 ns
        // resolution on Windows, so a 1 ns step rounds back onto `t0` and
        // once produced two identical ids on CI. Keeping ids apart at equal
        // instants is the process-wide sequence's job, tested separately.
        let c = mint_run_id(t0 + Duration::from_secs(1), 1);
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert!(a.as_str().starts_with("run-1725782401-"), "got {a}");
        assert!(a.as_str().len() <= omnifrons_domain::outbox::RunId::MAX_CHARS);
        assert_eq!(
            std::path::Path::new(a.as_str()).components().count(),
            1,
            "a run id is one path component"
        );
    }

    /// A drop-guard temp project for the table test.
    struct TempProject(std::path::PathBuf);

    impl TempProject {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "omnifrons-outbox-state-test-{}-{label}-{n}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).expect("fixture dir");
            Self(dir)
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn record(project: &TempProject, token: &str) -> RunRecord {
        use omnifrons_app::run_outbox::RunOutboxPreparer as _;
        let workspace = omnifrons_app::WorkspaceRoot::new(&project.0).expect("valid workspace");
        let prepared = omnifrons_adapters::FsRunOutboxPreparer::new()
            .prepare(
                &workspace,
                &omnifrons_domain::outbox::OutboxPath::default_path(),
                &omnifrons_domain::outbox::RunId::new(token).expect("valid"),
            )
            .expect("prepare");
        RunRecord::new(
            prepared,
            omnifrons_app::outbox_policy::OutboxPolicy::default_policy(),
        )
    }

    /// The run table is bounded: the `(MAX_RUN_RECORDS + 1)`th insert
    /// evicts the oldest record, and a record is found by run id as well
    /// as by process id.
    #[test]
    fn the_run_table_is_bounded_and_finds_records_by_run_id() {
        assert_eq!(MAX_RUN_RECORDS, 64);
        let project = TempProject::new("table");
        let mut table = RunTable::default();
        for n in 0..=MAX_RUN_RECORDS {
            let token = format!("run-{n}");
            table.insert(
                ProcessId(u32::try_from(n).expect("fits")),
                record(&project, &token),
            );
        }
        let remembered = (0..=MAX_RUN_RECORDS)
            .filter(|n| {
                table
                    .get(ProcessId(u32::try_from(*n).expect("fits")))
                    .is_some()
            })
            .count();
        assert_eq!(remembered, MAX_RUN_RECORDS, "exactly the cap is remembered");
        assert!(
            table.get(ProcessId(0)).is_none(),
            "the oldest record is evicted"
        );
        assert!(table.get(ProcessId(1)).is_some());
        let found = table
            .find_by_run_id(&omnifrons_domain::outbox::RunId::new("run-64").expect("valid"))
            .expect("found by run id");
        assert_eq!(found.run_id().as_str(), "run-64");
        assert!(
            table
                .find_by_run_id(&omnifrons_domain::outbox::RunId::new("run-0").expect("valid"))
                .is_none()
        );
    }

    /// The fixture files behind `held_candidate` handles, removed when the
    /// guard drops -- on every platform: declared before the table that
    /// holds the handles, so it drops after them, once every handle is
    /// closed (Windows refuses to unlink an open file).
    #[derive(Default)]
    struct FixtureFiles(Vec<std::path::PathBuf>);

    impl Drop for FixtureFiles {
        fn drop(&mut self) {
            for path in &self.0 {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    /// A candidate with a real (content-irrelevant) held handle.
    fn held_candidate(fixtures: &mut FixtureFiles, n: u8) -> omnifrons_app::run_outbox::Candidate {
        use omnifrons_domain::outbox::{
            ArtifactClass, Attribution, CandidateEntry, CandidateState, DetectedType,
        };
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let k = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "omnifrons-outbox-state-handle-{}-{k}",
            std::process::id()
        ));
        let file = std::fs::File::create(&path).expect("fixture handle");
        fixtures.0.push(path);
        omnifrons_app::run_outbox::Candidate {
            entry: CandidateEntry {
                name: format!("f{n}.pdf"),
                size: 1,
                digest: omnifrons_domain::executable::Sha256Digest([n; 32]),
                detected_type: DetectedType::Pdf,
                attribution: Attribution::Unattributed,
                class: ArtifactClass::GeneratedHeavy,
            },
            state: CandidateState::Candidate,
            handle: Some(file),
        }
    }

    /// R1-001: `store_candidates` keeps at most the project-wide remainder
    /// of `MAX_HELD_HANDLES` held, across every remembered run, and
    /// releases the rest while the entries stay `candidate`.
    #[test]
    fn store_candidates_enforces_the_handle_cap_across_runs() {
        use omnifrons_app::run_outbox::MAX_HELD_HANDLES;
        let project = TempProject::new("cap");
        let mut fixtures = FixtureFiles::default();
        let mut table = RunTable::default();
        table.insert(ProcessId(1), record(&project, "run-a"));
        table.insert(ProcessId(2), record(&project, "run-b"));
        assert_eq!(table.held_handles(), 0);

        let released_a = table
            .store_candidates(
                ProcessId(1),
                (0..20).map(|n| held_candidate(&mut fixtures, n)).collect(),
            )
            .expect("run-a is remembered");
        assert_eq!(released_a, 0);
        assert_eq!(table.held_handles(), 20);

        let released_b = table
            .store_candidates(
                ProcessId(2),
                (20..40).map(|n| held_candidate(&mut fixtures, n)).collect(),
            )
            .expect("run-b is remembered");
        assert_eq!(
            released_b, 8,
            "20 + 20 exceeds the cap of {MAX_HELD_HANDLES} by 8"
        );
        assert_eq!(table.held_handles(), MAX_HELD_HANDLES);
        let b = table
            .get(ProcessId(2))
            .expect("record")
            .candidates()
            .expect("stored");
        assert_eq!(b.len(), 20);
        assert!(b[..12].iter().all(|candidate| candidate.handle.is_some()));
        assert!(b[12..].iter().all(|candidate| candidate.handle.is_none()));
        assert!(b.iter().all(
            |candidate| candidate.state == omnifrons_domain::outbox::CandidateState::Candidate
        ));
        assert!(
            table.store_candidates(ProcessId(99), Vec::new()).is_err(),
            "an unknown run stores nothing and hands the candidates back"
        );
    }

    /// R1-001: evicting a record releases its held handles.
    #[test]
    fn evicting_a_record_releases_its_handles() {
        let project = TempProject::new("evict");
        let mut fixtures = FixtureFiles::default();
        let mut table = RunTable::default();
        table.insert(ProcessId(0), record(&project, "run-held"));
        table
            .store_candidates(
                ProcessId(0),
                (0..5).map(|n| held_candidate(&mut fixtures, n)).collect(),
            )
            .expect("remembered");
        assert_eq!(table.held_handles(), 5);
        for n in 1..=MAX_RUN_RECORDS {
            table.insert(
                ProcessId(u32::try_from(n).expect("fits")),
                record(&project, &format!("run-{n}")),
            );
        }
        assert!(
            table.get(ProcessId(0)).is_none(),
            "the oldest record is evicted"
        );
        assert_eq!(table.held_handles(), 0, "its handles went with it");
    }

    /// R1-006: proposal entries are recorded up to `MAX_PROPOSED_ENTRIES`
    /// per run; a proposal crossing the cap is truncated to the remaining
    /// slots and the excess counted, never silently dropped.
    #[test]
    fn push_proposal_caps_the_recorded_entries_and_counts_the_excess() {
        use omnifrons_domain::outbox::{ProposedEntry, PublishProposal};
        let project = TempProject::new("proposals");
        let mut record = record(&project, "run-proposals");
        let entry = |n: usize| ProposedEntry {
            name: format!("f{n}"),
            sha256: omnifrons_domain::executable::Sha256Digest(
                [u8::try_from(n % 251).expect("fits"); 32],
            ),
        };
        for n in 0..(MAX_PROPOSED_ENTRIES - 1) {
            record.push_proposal(PublishProposal {
                entries: vec![entry(n)],
            });
        }
        assert_eq!(record.recorded_proposal_entries(), MAX_PROPOSED_ENTRIES - 1);
        // A three-entry proposal with one slot left: one kept, two counted.
        record.push_proposal(PublishProposal {
            entries: vec![entry(2000), entry(2001), entry(2002)],
        });
        assert_eq!(record.recorded_proposal_entries(), MAX_PROPOSED_ENTRIES);
        assert_eq!(record.dropped_proposal_entries(), 2);
        assert!(
            record
                .proposals()
                .last()
                .expect("kept")
                .names_digest(&entry(2000).sha256),
            "the first entry of the crossing proposal is kept"
        );
        // Nothing left: counted, nothing recorded.
        record.push_proposal(PublishProposal {
            entries: vec![entry(3000)],
        });
        assert_eq!(record.recorded_proposal_entries(), MAX_PROPOSED_ENTRIES);
        assert_eq!(record.dropped_proposal_entries(), 3);
    }

    /// R1-010: clearing the table forgets every record and releases every
    /// held handle, reporting how many records were forgotten.
    #[test]
    fn clear_forgets_every_record_and_releases_every_handle() {
        let project = TempProject::new("clear");
        let mut fixtures = FixtureFiles::default();
        let mut table = RunTable::default();
        table.insert(ProcessId(1), record(&project, "run-a"));
        table.insert(ProcessId(2), record(&project, "run-b"));
        table
            .store_candidates(
                ProcessId(1),
                (0..4).map(|n| held_candidate(&mut fixtures, n)).collect(),
            )
            .expect("remembered");
        assert_eq!(table.held_handles(), 4);

        assert_eq!(table.clear(), 2);
        assert_eq!(table.held_handles(), 0);
        assert!(table.get(ProcessId(1)).is_none());
        assert!(table.get(ProcessId(2)).is_none());
        assert_eq!(table.clear(), 0, "clearing an empty table forgets nothing");
    }

    /// The sequence part of a run id is process-wide, never per state or
    /// table: two ids minted back to back -- here with a fresh `OutboxState`
    /// constructed in between, which must not reset anything -- carry
    /// distinct, increasing sequence numbers, so they differ even when the
    /// clock reports the same instant (Windows's clock offers no sub-second
    /// resolution the ids could rely on).
    #[test]
    fn ids_minted_back_to_back_carry_distinct_sequence_numbers() {
        let first = mint_next_run_id();
        let _another_state = super::OutboxState::new();
        let second = mint_next_run_id();
        let sequence = |id: &omnifrons_domain::outbox::RunId| -> u64 {
            id.as_str()
                .rsplit('-')
                .next()
                .expect("a run id ends in its sequence number")
                .parse()
                .expect("the sequence is a number")
        };
        assert!(
            sequence(&second) > sequence(&first),
            "the sequence must advance across states, got {first} then {second}"
        );
        assert_ne!(first, second);
    }
}
