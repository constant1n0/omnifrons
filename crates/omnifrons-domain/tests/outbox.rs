//! The outbox domain types (spike slice 5, HAP-001): the project-relative
//! outbox path, the run id, the closed candidate/failure tokens, the
//! artifact classes, and content-type detection by magic bytes. Pure
//! shapes only -- nothing here touches the filesystem.

use omnifrons_domain::adapter::{AdapterEvent, ToolCallProposal};
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::outbox::{
    ArtifactClass, Attribution, CandidateEntry, CandidateState, DEFAULT_OUTBOX_PATH, DetectedType,
    OutboxFailure, OutboxPath, OutboxPathError, PUBLISH_PROPOSAL_TOOL_NAME, ProposedEntry,
    PublishProposal, RunId, RunIdError,
};
use std::path::Path;

// -- OutboxPath --

#[test]
fn the_default_outbox_path_is_omnifrons_outbox_and_is_valid() {
    assert_eq!(DEFAULT_OUTBOX_PATH, ".omnifrons/outbox");
    let path = OutboxPath::default_path();
    assert_eq!(path.as_path(), Path::new(".omnifrons/outbox"));
    assert_eq!(
        OutboxPath::new(DEFAULT_OUTBOX_PATH).expect("the default must validate"),
        path
    );
}

#[test]
fn outbox_path_accepts_a_nested_project_relative_path() {
    let path = OutboxPath::new("build/agent-output").expect("a relative path must be accepted");
    assert_eq!(path.as_path(), Path::new("build/agent-output"));
    assert_eq!(path.to_string(), "build/agent-output");
}

#[test]
fn outbox_path_rejects_an_empty_path() {
    assert_eq!(OutboxPath::new("").unwrap_err(), OutboxPathError::Empty);
}

#[test]
fn outbox_path_rejects_a_parent_traversal_component() {
    assert_eq!(
        OutboxPath::new("../outside").unwrap_err(),
        OutboxPathError::ParentTraversal
    );
    assert_eq!(
        OutboxPath::new("a/../../b").unwrap_err(),
        OutboxPathError::ParentTraversal
    );
}

#[test]
fn outbox_path_rejects_an_absolute_path() {
    // `/tmp/x` is absolute on every platform this workspace tests on for
    // the purpose of this check (`Path::has_root` is true for a leading
    // separator on Windows too); a drive-letter form is additionally
    // absolute on Windows only and is covered by the same variant there.
    assert_eq!(
        OutboxPath::new("/tmp/outbox").unwrap_err(),
        OutboxPathError::Absolute
    );
}

#[test]
fn outbox_path_rejects_a_windows_drive_prefix_on_every_platform() {
    // A policy file roams between devices, so a `C:` prefix must be
    // refused on unix as well, where `Path` would treat it as an ordinary
    // relative component.
    assert_eq!(
        OutboxPath::new("C:/outbox").unwrap_err(),
        OutboxPathError::Absolute
    );
    assert_eq!(
        OutboxPath::new(r"C:\outbox").unwrap_err(),
        OutboxPathError::Absolute
    );
}

#[test]
fn outbox_path_rejects_a_backslash_separator() {
    // Backslashes are rejected outright rather than interpreted: a roaming
    // policy must name the path with `/` so it means the same thing on
    // every device.
    assert_eq!(
        OutboxPath::new(r"build\outbox").unwrap_err(),
        OutboxPathError::Backslash
    );
}

/// R1-001 (slice 5c risk review): the declared path is untrusted content
/// from a synchronized policy file (HAP-001-R40) and is substituted into
/// the managed blocks Omnifrons writes into a project's text files, so a
/// line break or any other control character in it would let the policy
/// forge or break a sentinel and leave the guidance feature permanently
/// malformed for that project. Every control character, and the two
/// Unicode line and paragraph separators, are refused at the door.
#[test]
fn outbox_path_rejects_control_characters_and_line_separators() {
    assert_eq!(
        OutboxPath::new("out\n<!-- omnifrons:end guidance -->").unwrap_err(),
        OutboxPathError::Control,
        "a newline would forge an end sentinel inside the managed block"
    );
    for declared in [
        "out\r\n# omnifrons:end ignore",
        "out\rx",
        "out\tx",
        "out\u{0}x",
        "out\u{7}x",
        "out\u{7f}x",
        "out\u{85}x",
        "out\u{2028}x",
        "out\u{2029}x",
    ] {
        assert_eq!(
            OutboxPath::new(declared).unwrap_err(),
            OutboxPathError::Control,
            "{declared:?}"
        );
    }
    // One fixed message for the whole class, naming no path: the
    // declaration is untrusted content and never rides its own refusal.
    assert_eq!(
        OutboxPathError::Control.to_string(),
        "the outbox path must not contain control, line-separator, or bidirectional override characters"
    );
    // A path that merely looks like a sentinel stays one line and is
    // accepted: the refusal is about line breaks, not about spelling.
    assert!(OutboxPath::new("<!-- omnifrons:end guidance -->").is_ok());
}

/// R1-004 (slice 5c risk re-review): `char::is_control` covers only the
/// Cc category, so the bidirectional override, embedding and isolate
/// range (U+202A..=U+202E, U+2066..=U+2069) passed the refusal above and
/// was written verbatim into the user's `AGENTS.md` and `.gitignore`. A
/// right-to-left override there makes the block render in an order it is
/// not stored in -- Trojan-Source class spoofing in the two files a human
/// and an agent both read as instructions -- and the renderer's strip
/// protects the preview only, never the bytes on disk. The declaration is
/// untrusted content (HAP-001-R40), so the range is refused at the door
/// under the same `Control` variant this repository already uses for it in
/// `ManagedFileName::check_component` and `DisplayName::sanitize`.
#[test]
fn outbox_path_rejects_bidirectional_override_characters() {
    assert_eq!(
        OutboxPath::new("build/\u{202E}gnp.esrever/out").unwrap_err(),
        OutboxPathError::Control,
        "a right-to-left override reorders the path a reader sees"
    );
    for declared in [
        "out\u{202A}x",
        "out\u{202B}x",
        "out\u{202C}x",
        "out\u{202D}x",
        "out\u{202E}x",
        "out\u{2066}x",
        "out\u{2067}x",
        "out\u{2068}x",
        "out\u{2069}x",
    ] {
        assert_eq!(
            OutboxPath::new(declared).unwrap_err(),
            OutboxPathError::Control,
            "{declared:?}"
        );
    }
    // U+200F is a plain right-to-left mark, not an override: it stays
    // accepted, exactly where `is_bidi_control` draws the line for the
    // managed file names and the display names of the same data flow.
    assert!(OutboxPath::new("out\u{200F}x").is_ok());
}

// -- RunId --

#[test]
fn run_id_accepts_a_minted_token_and_round_trips() {
    let id = RunId::new("run-1725782400-000000001-7").expect("a minted token must validate");
    assert_eq!(id.as_str(), "run-1725782400-000000001-7");
    assert_eq!(id.to_string(), "run-1725782400-000000001-7");
}

#[test]
fn run_id_rejects_empty_separators_dots_and_odd_characters() {
    assert_eq!(RunId::new("").unwrap_err(), RunIdError::Empty);
    for token in ["a/b", r"a\b", "..", ".", "a.b", "a b", "a:b", "ünïcode"] {
        assert_eq!(
            RunId::new(token).unwrap_err(),
            RunIdError::InvalidCharacter,
            "{token:?} must be refused"
        );
    }
}

#[test]
fn run_id_rejects_a_token_over_64_characters() {
    assert_eq!(RunId::new("a".repeat(65)).unwrap_err(), RunIdError::TooLong);
    assert!(RunId::new("a".repeat(64)).is_ok());
}

// -- closed tokens --

#[test]
fn candidate_state_tokens_are_exactly_the_three_wire_tokens() {
    assert_eq!(CandidateState::Candidate.as_str(), "candidate");
    assert_eq!(CandidateState::OutboxEscape.as_str(), "outbox-escape");
    assert_eq!(CandidateState::OutboxLinked.as_str(), "outbox-linked");
    // Maintenance trip-wire: adding a variant makes this non-exhaustive.
    let all = [
        CandidateState::Candidate,
        CandidateState::OutboxEscape,
        CandidateState::OutboxLinked,
    ];
    for state in all {
        match state {
            CandidateState::Candidate
            | CandidateState::OutboxEscape
            | CandidateState::OutboxLinked => {}
        }
    }
}

#[test]
fn outbox_failure_tokens_are_exactly_the_two_launch_side_tokens() {
    assert_eq!(OutboxFailure::OutboxInvalid.as_str(), "outbox-invalid");
    assert_eq!(
        OutboxFailure::OutboxUnavailable.as_str(),
        "outbox-unavailable"
    );
}

#[test]
fn artifact_class_tokens_are_the_five_declared_classes() {
    let cases = [
        (ArtifactClass::GeneratedHeavy, "generated-heavy"),
        (ArtifactClass::GitTracked, "git-tracked"),
        (ArtifactClass::PortableText, "portable-text"),
        (ArtifactClass::Executable, "executable"),
        (ArtifactClass::Unclassified, "unclassified"),
    ];
    for (class, token) in cases {
        assert_eq!(class.as_str(), token);
        assert_eq!(
            ArtifactClass::parse(token),
            Some(class),
            "{token} must parse back"
        );
    }
    assert_eq!(ArtifactClass::parse("asset-root"), None);
}

// -- attribution and the candidate entry --

#[test]
fn attribution_is_run_or_unattributed_and_carries_the_run_id() {
    let run = RunId::new("run-1").expect("valid");
    let attributed = Attribution::Run(run.clone());
    assert_ne!(attributed, Attribution::Unattributed);
    assert_eq!(attributed.run_id(), Some(&run));
    assert_eq!(Attribution::Unattributed.run_id(), None);
}

#[test]
fn candidate_entry_is_a_plain_comparable_value() {
    let entry = CandidateEntry {
        name: "run-1/report.pdf".to_string(),
        size: 42,
        digest: Sha256Digest([1; 32]),
        detected_type: DetectedType::Pdf,
        attribution: Attribution::Unattributed,
        class: ArtifactClass::GeneratedHeavy,
    };
    assert_eq!(entry.clone(), entry);
}

// -- content digest parsing (reused by proposals) --

#[test]
fn sha256_digest_parses_its_own_hex_and_rejects_bad_input() {
    let digest = Sha256Digest([0xab; 32]);
    assert_eq!(Sha256Digest::from_hex(&digest.to_hex()), Some(digest));
    assert_eq!(Sha256Digest::from_hex(&"ab".repeat(31)), None);
    assert_eq!(Sha256Digest::from_hex(&"zz".repeat(32)), None);
    assert_eq!(
        Sha256Digest::from_hex(&"AB".repeat(32)),
        Some(digest),
        "uppercase hex is accepted"
    );
}

// -- the publish proposal --

#[test]
fn publish_proposal_names_entries_by_digest_and_has_a_fixed_tool_name() {
    assert_eq!(PUBLISH_PROPOSAL_TOOL_NAME, "artifact.publish");
    let proposal = PublishProposal {
        entries: vec![ProposedEntry {
            name: "report.pdf".to_string(),
            sha256: Sha256Digest([7; 32]),
        }],
    };
    let event = AdapterEvent::ArtifactPublish(proposal.clone());
    assert_ne!(
        event,
        AdapterEvent::ToolCall(ToolCallProposal {
            name: PUBLISH_PROPOSAL_TOOL_NAME.to_string(),
            arguments_text: String::new(),
        }),
        "a recognized publish proposal is its own event, distinct from a plain tool call"
    );
    assert!(proposal.names_digest(&Sha256Digest([7; 32])));
    assert!(!proposal.names_digest(&Sha256Digest([8; 32])));
}

// -- detection by magic bytes --

#[test]
fn detected_type_recognizes_the_common_document_image_audio_video_and_archive_magics() {
    let cases: Vec<(&str, Vec<u8>, DetectedType)> = vec![
        ("a.pdf", b"%PDF-1.7\n".to_vec(), DetectedType::Pdf),
        (
            "a.png",
            vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0],
            DetectedType::Png,
        ),
        ("a.jpg", vec![0xFF, 0xD8, 0xFF, 0xE0], DetectedType::Jpeg),
        ("a.gif", b"GIF89a".to_vec(), DetectedType::Gif),
        (
            "a.webp",
            b"RIFF\x00\x00\x00\x00WEBPVP8 ".to_vec(),
            DetectedType::Webp,
        ),
        (
            "a.wav",
            b"RIFF\x00\x00\x00\x00WAVEfmt ".to_vec(),
            DetectedType::Wav,
        ),
        ("a.mp3", b"ID3\x04\x00".to_vec(), DetectedType::Mp3),
        ("b.mp3", vec![0xFF, 0xFB, 0x90, 0x00], DetectedType::Mp3),
        ("a.ogg", b"OggS\x00".to_vec(), DetectedType::Ogg),
        ("a.flac", b"fLaC\x00".to_vec(), DetectedType::Flac),
        (
            "a.mp4",
            b"\x00\x00\x00\x18ftypisom".to_vec(),
            DetectedType::Mp4,
        ),
        (
            "a.mkv",
            vec![0x1A, 0x45, 0xDF, 0xA3, 0x00],
            DetectedType::Matroska,
        ),
        ("a.zip", b"PK\x03\x04\x14\x00".to_vec(), DetectedType::Zip),
        (
            "a.docx",
            b"PK\x03\x04\x14\x00".to_vec(),
            DetectedType::OfficeDocument,
        ),
        (
            "a.xlsx",
            b"PK\x03\x04\x14\x00".to_vec(),
            DetectedType::OfficeDocument,
        ),
        ("a.gz", vec![0x1F, 0x8B, 0x08], DetectedType::Gzip),
        (
            "a.7z",
            vec![0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C],
            DetectedType::SevenZip,
        ),
    ];
    for (name, head, expected) in cases {
        assert_eq!(DetectedType::detect(name, &head), expected, "{name}");
    }
}

#[test]
fn detected_type_recognizes_a_tar_by_its_ustar_marker_at_offset_257() {
    let mut head = vec![0u8; 512];
    head[257..262].copy_from_slice(b"ustar");
    assert_eq!(DetectedType::detect("a.tar", &head), DetectedType::Tar);
}

#[test]
fn detected_type_recognizes_executables_by_magic_and_shebang() {
    let cases: Vec<(&str, Vec<u8>, DetectedType)> = vec![
        (
            "a.bin",
            vec![0x7F, b'E', b'L', b'F', 2, 1, 1],
            DetectedType::ElfExecutable,
        ),
        ("a.exe", b"MZ\x90\x00".to_vec(), DetectedType::PeExecutable),
        (
            "a",
            vec![0xFE, 0xED, 0xFA, 0xCF, 0],
            DetectedType::MachOExecutable,
        ),
        (
            "b",
            vec![0xCF, 0xFA, 0xED, 0xFE, 0],
            DetectedType::MachOExecutable,
        ),
        (
            "c",
            vec![0xCA, 0xFE, 0xBA, 0xBE, 0],
            DetectedType::MachOExecutable,
        ),
        (
            "run.sh",
            b"#!/bin/sh\necho hi\n".to_vec(),
            DetectedType::Script,
        ),
    ];
    for (name, head, expected) in cases {
        let detected = DetectedType::detect(name, &head);
        assert_eq!(detected, expected, "{name}");
        assert!(detected.is_executable(), "{name} must be executable-shaped");
    }
    assert!(!DetectedType::Pdf.is_executable());
}

#[test]
fn detected_type_prefers_content_over_extension() {
    // A PDF named like a note is a PDF (HAP-001-R1: the detected type wins).
    assert_eq!(
        DetectedType::detect("notes.md", b"%PDF-1.4"),
        DetectedType::Pdf
    );
    // Text named like a PDF is not a PDF.
    assert_eq!(
        DetectedType::detect("report.pdf", b"just text"),
        DetectedType::PlainText
    );
}

#[test]
fn detected_type_falls_back_to_markdown_plain_text_or_unknown() {
    assert_eq!(
        DetectedType::detect("notes.md", b"# Title\n"),
        DetectedType::Markdown
    );
    assert_eq!(
        DetectedType::detect("NOTES.MARKDOWN", b"# Title\n"),
        DetectedType::Markdown
    );
    assert_eq!(
        DetectedType::detect("config.toml", b"[a]\nb = 1\n"),
        DetectedType::PlainText
    );
    assert_eq!(DetectedType::detect("empty", b""), DetectedType::PlainText);
    assert_eq!(
        DetectedType::detect("blob.dat", &[0x00, 0xFF, 0xFE, 0x01]),
        DetectedType::Unknown
    );
}

#[test]
fn detected_type_tokens_round_trip_through_parse() {
    for detected in DetectedType::ALL {
        assert_eq!(
            DetectedType::parse(detected.as_str()),
            Some(detected),
            "{} must parse back",
            detected.as_str()
        );
    }
    assert_eq!(DetectedType::parse("not-a-type"), None);
    assert_eq!(DetectedType::SevenZip.as_str(), "7z");
    assert_eq!(DetectedType::MachOExecutable.as_str(), "mach-o-executable");
}

/// R1-012: a project-relative path is bounded before it is displayed.
/// HAP-001's acceptance list expects excessive length normalized or
/// rejected before display, and nothing else bounds this value: a finding's
/// name is chosen by whatever wrote the file, and without a cap it crosses
/// IPC at whatever length the file system allowed.
#[test]
fn a_project_relative_path_is_bounded_in_length() {
    use omnifrons_domain::outbox::{
        MAX_PROJECT_RELATIVE_BYTES, MAX_PROJECT_RELATIVE_COMPONENT_BYTES, validate_project_relative,
    };

    // Each component takes `DisplayName`'s own cap, which is the cap every
    // name that reaches a device or a surface already takes.
    assert_eq!(
        MAX_PROJECT_RELATIVE_COMPONENT_BYTES,
        omnifrons_domain::publication::DisplayName::MAX_BYTES
    );

    let longest_component = "a".repeat(MAX_PROJECT_RELATIVE_COMPONENT_BYTES);
    validate_project_relative(&format!("docs/{longest_component}"))
        .expect("a component exactly at the cap is accepted");
    assert_eq!(
        validate_project_relative(&format!("docs/{longest_component}a")).unwrap_err(),
        OutboxPathError::TooLong,
        "one byte past the per-component cap is refused"
    );

    let mut at_the_cap = String::new();
    while at_the_cap.len() + 8 <= MAX_PROJECT_RELATIVE_BYTES {
        at_the_cap.push_str("aaaaaaa/");
    }
    while at_the_cap.len() < MAX_PROJECT_RELATIVE_BYTES {
        at_the_cap.push('a');
    }
    assert_eq!(at_the_cap.len(), MAX_PROJECT_RELATIVE_BYTES);
    validate_project_relative(&at_the_cap).expect("a path exactly at the cap is accepted");
    assert_eq!(
        validate_project_relative(&format!("{at_the_cap}c")).unwrap_err(),
        OutboxPathError::TooLong,
        "one byte past the whole-path cap is refused"
    );
    assert_eq!(
        OutboxPathError::TooLong.to_string(),
        "the outbox path is too long"
    );
}
