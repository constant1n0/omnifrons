//! `OutboxPolicy` (spike slice 5, HAP-001 § Artifact classes and
//! classification policy): a pure classifier over a candidate's name,
//! detected type, and size. The fixed rules HAP-001-R2 and R4 imply hold
//! regardless of the project's rows -- Markdown is never
//! `generated-heavy`, an executable is always `executable` -- and a
//! project row that contradicts either is rejected when the policy is
//! built, never silently ignored.

use omnifrons_app::outbox_policy::{
    ArtifactClassifier, GIT_TRACKED_TEXT_MAX_BYTES, OutboxPolicy, POLICY_FILE_PATH,
    POLICY_SCHEMA_VERSION, PolicyRow, PolicyViolation,
};
use omnifrons_domain::outbox::{ArtifactClass, DetectedType, OutboxPath};

const MIB: u64 = 1024 * 1024;

fn default_policy() -> OutboxPolicy {
    OutboxPolicy::default_policy()
}

#[test]
fn the_policy_file_lives_under_the_reserved_namespace_with_schema_one() {
    assert_eq!(POLICY_FILE_PATH, ".omnifrons/asset-policy.json");
    assert_eq!(POLICY_SCHEMA_VERSION, 1);
    assert_eq!(GIT_TRACKED_TEXT_MAX_BYTES, MIB);
}

#[test]
fn the_default_policy_declares_the_default_outbox() {
    assert_eq!(default_policy().outbox(), &OutboxPath::default_path());
}

#[test]
fn markdown_is_never_generated_heavy_whatever_its_size() {
    let policy = default_policy();
    assert_eq!(
        policy.classify("notes.md", DetectedType::Markdown, 50 * MIB),
        ArtifactClass::PortableText
    );
}

#[test]
fn a_row_routing_markdown_to_generated_heavy_is_rejected_at_build() {
    let row = PolicyRow {
        types: vec![DetectedType::Markdown],
        extensions: vec![],
        max_size: None,
        class: ArtifactClass::GeneratedHeavy,
    };
    let error = OutboxPolicy::new(OutboxPath::default_path(), vec![row]).unwrap_err();
    assert_eq!(error, PolicyViolation::MarkdownRoutedHeavy);
}

#[test]
fn executables_are_classified_executable_by_content_or_extension() {
    let policy = default_policy();
    for (name, detected) in [
        ("tool", DetectedType::ElfExecutable),
        ("tool.exe", DetectedType::PeExecutable),
        ("tool", DetectedType::MachOExecutable),
        ("run.sh", DetectedType::Script),
        // Extension alone, when the content decided nothing.
        ("setup.exe", DetectedType::Unknown),
        ("job.bat", DetectedType::PlainText),
        ("job.cmd", DetectedType::PlainText),
        ("job.ps1", DetectedType::PlainText),
        ("job.sh", DetectedType::PlainText),
    ] {
        assert_eq!(
            policy.classify(name, detected, 1024),
            ArtifactClass::Executable,
            "{name} ({detected:?})"
        );
    }
}

#[test]
fn a_row_routing_an_executable_type_elsewhere_is_rejected_at_build() {
    let row = PolicyRow {
        types: vec![DetectedType::ElfExecutable],
        extensions: vec![],
        max_size: None,
        class: ArtifactClass::GeneratedHeavy,
    };
    let error = OutboxPolicy::new(OutboxPath::default_path(), vec![row]).unwrap_err();
    assert_eq!(error, PolicyViolation::ExecutableRoutedAway);
}

#[test]
fn the_default_rows_route_documents_media_and_archives_to_generated_heavy() {
    let policy = default_policy();
    for detected in [
        DetectedType::Pdf,
        DetectedType::OfficeDocument,
        DetectedType::Png,
        DetectedType::Jpeg,
        DetectedType::Gif,
        DetectedType::Webp,
        DetectedType::Mp3,
        DetectedType::Wav,
        DetectedType::Ogg,
        DetectedType::Flac,
        DetectedType::Mp4,
        DetectedType::Matroska,
        DetectedType::Zip,
        DetectedType::Gzip,
        DetectedType::SevenZip,
        DetectedType::Tar,
    ] {
        assert_eq!(
            policy.classify("output", detected, 40 * MIB),
            ArtifactClass::GeneratedHeavy,
            "{detected:?}"
        );
    }
}

#[test]
fn small_source_and_configuration_text_is_git_tracked_and_larger_text_is_unclassified() {
    let policy = default_policy();
    for name in [
        "main.rs",
        "Cargo.toml",
        "config.json",
        "ci.yaml",
        "ci.yml",
        "notes.txt",
    ] {
        assert_eq!(
            policy.classify(name, DetectedType::PlainText, 4096),
            ArtifactClass::GitTracked,
            "{name}"
        );
    }
    assert_eq!(
        policy.classify("Cargo.toml", DetectedType::PlainText, MIB),
        ArtifactClass::GitTracked,
        "exactly the cap is still git-tracked"
    );
    assert_eq!(
        policy.classify("Cargo.toml", DetectedType::PlainText, MIB + 1),
        ArtifactClass::Unclassified
    );
    assert_eq!(
        policy.classify("data.csv", DetectedType::PlainText, 4096),
        ArtifactClass::Unclassified,
        "text with no recognized extension has no default row"
    );
}

#[test]
fn an_unknown_blob_is_unclassified() {
    assert_eq!(
        default_policy().classify("blob.dat", DetectedType::Unknown, 10),
        ArtifactClass::Unclassified
    );
}

#[test]
fn project_rows_precede_the_defaults_and_may_match_by_extension() {
    let row = PolicyRow {
        types: vec![DetectedType::PlainText],
        extensions: vec!["csv".to_string()],
        max_size: None,
        class: ArtifactClass::GeneratedHeavy,
    };
    let policy = OutboxPolicy::new(OutboxPath::default_path(), vec![row]).expect("a valid row");
    assert_eq!(
        policy.classify("data.csv", DetectedType::PlainText, 4096),
        ArtifactClass::GeneratedHeavy
    );
    assert_eq!(
        policy.classify("main.rs", DetectedType::PlainText, 4096),
        ArtifactClass::GitTracked,
        "the defaults still apply after the project's rows"
    );
}

#[test]
fn a_row_with_no_types_matches_every_type_and_a_max_size_is_inclusive() {
    let row = PolicyRow {
        types: vec![],
        extensions: vec![],
        max_size: Some(10),
        class: ArtifactClass::GitTracked,
    };
    let policy = OutboxPolicy::new(OutboxPath::default_path(), vec![row]).expect("a valid row");
    assert_eq!(
        policy.classify("tiny.dat", DetectedType::Unknown, 10),
        ArtifactClass::GitTracked
    );
    assert_eq!(
        policy.classify("tiny.dat", DetectedType::Unknown, 11),
        ArtifactClass::Unclassified
    );
    // The fixed rules still win over a wildcard row.
    assert_eq!(
        policy.classify("tiny.md", DetectedType::Markdown, 5),
        ArtifactClass::PortableText
    );
    assert_eq!(
        policy.classify("tiny", DetectedType::Script, 5),
        ArtifactClass::Executable
    );
}

#[test]
fn a_project_can_declare_its_own_outbox_path() {
    let policy = OutboxPolicy::new(
        OutboxPath::new("build/agent-output").expect("valid"),
        vec![],
    )
    .expect("no rows is valid");
    assert_eq!(policy.outbox().to_string(), "build/agent-output");
}
