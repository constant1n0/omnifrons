//! Candidate assembly (spike slice 5, HAP-001-R11, R17, D22): probed
//! entries become `Candidate`s with a state, a class, and an attribution
//! decided by digest against the run's own proposals; handles held beyond
//! the D22 cap are released while the entries stay `candidate`; the
//! run-end summary counts per state.

use std::ffi::OsString;
use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use omnifrons_app::outbox_policy::OutboxPolicy;
use omnifrons_app::run_outbox::{
    CandidateProbe, EscapeReason, InventoriedEntry, MAX_HELD_HANDLES, RegularCandidate,
    assemble_run_candidates, assemble_unattributed_candidates, cap_held_handles, summarize,
};
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::outbox::{
    ArtifactClass, Attribution, CandidateState, DetectedType, ProposedEntry, PublishProposal, RunId,
};

/// A real, throwaway open handle for a fake probe to carry: the content is
/// irrelevant, only that the handle is genuine (mirroring the app
/// contract module's own fake handle discipline).
fn handle() -> File {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path: PathBuf = std::env::temp_dir().join(format!(
        "omnifrons-outbox-candidates-test-{}-{n}",
        std::process::id()
    ));
    let file = File::create(&path).expect("the fake handle file must be creatable");
    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(&path);
    }
    file
}

fn regular(name: &str, digest_byte: u8, detected: DetectedType, size: u64) -> InventoriedEntry {
    InventoriedEntry {
        name: OsString::from(name),
        probe: CandidateProbe::Regular(RegularCandidate {
            size,
            digest: Sha256Digest([digest_byte; 32]),
            detected_type: detected,
            handle: handle(),
        }),
    }
}

fn run() -> RunId {
    RunId::new("run-7").expect("valid")
}

#[test]
fn an_entry_named_by_digest_in_the_runs_proposal_is_attributed_everything_else_is_not() {
    let entries = vec![
        regular("report.pdf", 1, DetectedType::Pdf, 100),
        regular("stray.png", 2, DetectedType::Png, 50),
    ];
    let proposals = vec![PublishProposal {
        entries: vec![ProposedEntry {
            name: "report.pdf".to_string(),
            sha256: Sha256Digest([1; 32]),
        }],
    }];
    let assembled =
        assemble_run_candidates(&run(), entries, &proposals, &OutboxPolicy::default_policy());
    assert_eq!(assembled.candidates.len(), 2);

    let report = &assembled.candidates[0];
    assert_eq!(
        report.entry.name, "run-7/report.pdf",
        "names are relative to the outbox"
    );
    assert_eq!(report.entry.attribution, Attribution::Run(run()));
    assert_eq!(report.entry.class, ArtifactClass::GeneratedHeavy);
    assert_eq!(report.state, CandidateState::Candidate);
    assert!(
        report.handle.is_some(),
        "a candidate within the cap keeps its handle"
    );

    let stray = &assembled.candidates[1];
    assert_eq!(stray.entry.name, "run-7/stray.png");
    assert_eq!(
        stray.entry.attribution,
        Attribution::Unattributed,
        "location under the run subdirectory alone never attributes"
    );
    assert!(assembled.unmatched_proposals.is_empty());
}

#[test]
fn a_proposal_whose_digest_differs_from_the_handles_digest_does_not_attribute() {
    let entries = vec![regular("report.pdf", 1, DetectedType::Pdf, 100)];
    let proposals = vec![PublishProposal {
        entries: vec![ProposedEntry {
            name: "report.pdf".to_string(),
            sha256: Sha256Digest([9; 32]),
        }],
    }];
    let assembled =
        assemble_run_candidates(&run(), entries, &proposals, &OutboxPolicy::default_policy());
    assert_eq!(
        assembled.candidates[0].entry.attribution,
        Attribution::Unattributed
    );
    assert_eq!(
        assembled.unmatched_proposals,
        vec![ProposedEntry {
            name: "report.pdf".to_string(),
            sha256: Sha256Digest([9; 32]),
        }],
        "a proposal naming a digest not found in the subdirectory is surfaced"
    );
}

#[test]
fn escape_and_linked_probes_become_their_states_with_no_digest_facts_and_no_handle() {
    let entries = vec![
        InventoriedEntry {
            name: OsString::from("link"),
            probe: CandidateProbe::Escape(EscapeReason::Link),
        },
        InventoriedEntry {
            name: OsString::from("fifo"),
            probe: CandidateProbe::Escape(EscapeReason::NotRegular),
        },
        InventoriedEntry {
            name: OsString::from("linked.bin"),
            probe: CandidateProbe::Linked { link_count: 2 },
        },
        InventoriedEntry {
            name: OsString::from("locked"),
            probe: CandidateProbe::Unreadable,
        },
    ];
    let assembled = assemble_run_candidates(&run(), entries, &[], &OutboxPolicy::default_policy());
    let states: Vec<_> = assembled
        .candidates
        .iter()
        .map(|candidate| (candidate.entry.name.as_str(), candidate.state))
        .collect();
    assert_eq!(
        states,
        vec![
            ("run-7/link", CandidateState::OutboxEscape),
            ("run-7/fifo", CandidateState::OutboxEscape),
            ("run-7/linked.bin", CandidateState::OutboxLinked),
        ]
    );
    for candidate in &assembled.candidates {
        assert!(candidate.handle.is_none());
        assert_eq!(candidate.entry.size, 0);
        assert_eq!(
            candidate.entry.digest,
            Sha256Digest([0; 32]),
            "nothing was digested"
        );
        assert_eq!(candidate.entry.class, ArtifactClass::Unclassified);
        assert_eq!(candidate.entry.attribution, Attribution::Unattributed);
    }
    assert_eq!(
        assembled.unreadable, 1,
        "an unreadable entry is counted, not listed"
    );
}

#[test]
fn whole_outbox_entries_are_unattributed_and_named_relative_to_the_outbox() {
    let root = assemble_unattributed_candidates(
        None,
        vec![regular("dropped.pdf", 3, DetectedType::Pdf, 10)],
        &OutboxPolicy::default_policy(),
    );
    assert_eq!(root.candidates[0].entry.name, "dropped.pdf");
    assert_eq!(
        root.candidates[0].entry.attribution,
        Attribution::Unattributed
    );

    let under_run = assemble_unattributed_candidates(
        Some(&run()),
        vec![regular("later.pdf", 4, DetectedType::Pdf, 10)],
        &OutboxPolicy::default_policy(),
    );
    assert_eq!(
        under_run.candidates[0].entry.name, "run-7/later.pdf",
        "the run subdirectory is a location fact in the name, never provenance"
    );
    assert_eq!(
        under_run.candidates[0].entry.attribution,
        Attribution::Unattributed
    );
}

#[test]
fn handles_beyond_the_cap_are_released_while_the_entries_stay_candidate() {
    assert_eq!(MAX_HELD_HANDLES, 32);
    let entries: Vec<InventoriedEntry> = (0..40u8)
        .map(|n| regular(&format!("f{n}.pdf"), n, DetectedType::Pdf, 1))
        .collect();
    let mut assembled =
        assemble_run_candidates(&run(), entries, &[], &OutboxPolicy::default_policy());
    let released = cap_held_handles(&mut assembled.candidates, MAX_HELD_HANDLES);
    assert_eq!(released, 8);
    let held = assembled
        .candidates
        .iter()
        .filter(|candidate| candidate.handle.is_some())
        .count();
    assert_eq!(held, MAX_HELD_HANDLES);
    assert!(
        assembled
            .candidates
            .iter()
            .all(|candidate| candidate.state == CandidateState::Candidate),
        "an entry beyond the cap stays candidate"
    );
    assert!(
        assembled.candidates[32..]
            .iter()
            .all(|candidate| candidate.handle.is_none()),
        "the excess, in inventory order, is what loses its handle"
    );
}

#[test]
fn the_summary_counts_per_state_and_attribution() {
    let entries = vec![
        regular("a.pdf", 1, DetectedType::Pdf, 1),
        regular("b.pdf", 2, DetectedType::Pdf, 1),
        InventoriedEntry {
            name: OsString::from("link"),
            probe: CandidateProbe::Escape(EscapeReason::Link),
        },
        InventoriedEntry {
            name: OsString::from("linked"),
            probe: CandidateProbe::Linked { link_count: 3 },
        },
        InventoriedEntry {
            name: OsString::from("locked"),
            probe: CandidateProbe::Unreadable,
        },
    ];
    let proposals = vec![PublishProposal {
        entries: vec![ProposedEntry {
            name: "a.pdf".to_string(),
            sha256: Sha256Digest([1; 32]),
        }],
    }];
    let assembled =
        assemble_run_candidates(&run(), entries, &proposals, &OutboxPolicy::default_policy());
    let summary = summarize(&assembled);
    assert_eq!(summary.total, 4);
    assert_eq!(summary.candidate, 2);
    assert_eq!(summary.outbox_escape, 1);
    assert_eq!(summary.outbox_linked, 1);
    assert_eq!(summary.attributed, 1);
    assert_eq!(summary.unattributed, 3);
    assert_eq!(summary.unreadable, 1);
    assert_eq!(summary.unmatched_proposals, 0);
}
