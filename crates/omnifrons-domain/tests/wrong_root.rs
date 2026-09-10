//! Wrong-root domain contracts (spike slice 5d, HAP-001 § Wrong-root
//! detection and remedies, D16): the output-discipline report HAP-001-R33
//! and R34 fix, the three remedies HAP-001-R32 fixes, and the finding
//! shape that never carries a device path (RCS-001-R14).

use omnifrons_domain::scope::ScopeMode;
use omnifrons_domain::wrong_root::{DeclaredWriteSet, OutputDiscipline};

/// HAP-001-R33: `enforced` only under `sandbox-enforced` scope whose write
/// set is the project root; `advisory` under every other combination.
#[test]
fn output_discipline_is_enforced_only_for_a_sandbox_over_the_project_root() {
    assert_eq!(
        OutputDiscipline::for_scope(ScopeMode::SandboxEnforced, DeclaredWriteSet::ProjectRoot),
        OutputDiscipline::Enforced,
        "a sandbox whose write set is the project root is the one enforced case"
    );
    let advisory = [
        (ScopeMode::SandboxEnforced, DeclaredWriteSet::Wider),
        (ScopeMode::HarnessEnforced, DeclaredWriteSet::ProjectRoot),
        (ScopeMode::HarnessEnforced, DeclaredWriteSet::Wider),
        (ScopeMode::Advisory, DeclaredWriteSet::ProjectRoot),
        (ScopeMode::Advisory, DeclaredWriteSet::Wider),
    ];
    for (mode, write_set) in advisory {
        assert_eq!(
            OutputDiscipline::for_scope(mode, write_set),
            OutputDiscipline::Advisory,
            "{mode:?} with {write_set:?} must report advisory"
        );
    }
}

/// HAP-001-R34: in every mode, including `sandbox-enforced`, the report
/// discloses that a write inside the project but outside the outbox is
/// detected after the run, never prevented. HAP-001-R33 adds the
/// outside-the-project disclosure under `advisory` only.
#[test]
fn every_discipline_carries_the_in_project_disclosure_and_advisory_adds_its_own() {
    for discipline in OutputDiscipline::ALL {
        assert!(
            discipline
                .disclosures()
                .contains(&OutputDiscipline::IN_PROJECT_DISCLOSURE),
            "{discipline:?} must carry HAP-001-R34's disclosure"
        );
    }
    assert_eq!(
        OutputDiscipline::Enforced.disclosures(),
        vec![OutputDiscipline::IN_PROJECT_DISCLOSURE],
        "an enforced report claims nothing beyond HAP-001-R34's disclosure"
    );
    assert_eq!(
        OutputDiscipline::Advisory.disclosures(),
        vec![
            OutputDiscipline::IN_PROJECT_DISCLOSURE,
            OutputDiscipline::OUTSIDE_PROJECT_DISCLOSURE,
        ],
        "an advisory report also discloses that a write outside the project is possible"
    );
    assert_eq!(OutputDiscipline::Enforced.as_str(), "enforced");
    assert_eq!(OutputDiscipline::Advisory.as_str(), "advisory");
}
