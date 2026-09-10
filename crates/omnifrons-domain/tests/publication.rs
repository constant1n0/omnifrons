//! The publication domain types (spike slice 5b, HAP-001 § Publication
//! transaction, § Catalog record, § Artifact states): identities, the
//! sanitized display name (HAP-001-R24 under RCS-001's file-name rule),
//! the portable reference (AEC-001's `ref` shape), and the closed state
//! tokens. Framework-independent and filesystem-free.

use omnifrons_domain::executable::{DeviceLocalUser, Sha256Digest};
use omnifrons_domain::outbox::{ArtifactClass, Attribution, DetectedType, RunId};
use omnifrons_domain::publication::{
    ARTIFACT_APPROVAL_ID_DOMAIN, ArtifactApproval, ArtifactApprovalId, ArtifactState, AssetRootId,
    AssetRootIdError, CatalogId, DisplayName, PortableReference, ProjectIdentity, ProviderState,
    PublicationIdentity, artifact_approval_id_preimage, publication_identity_preimage,
};
use std::time::{Duration, SystemTime};

fn digest(byte: u8) -> Sha256Digest {
    Sha256Digest([byte; 32])
}

// -- identities --

/// HAP-001-R23: the publication identity is derived from exactly the
/// project identity followed by the content digest, nothing else -- the
/// preimage is the 64 bytes `project ‖ digest`, in that order.
#[test]
fn publication_identity_preimage_is_project_then_digest() {
    let project = ProjectIdentity(digest(0x11));
    let content = digest(0x22);
    let preimage = publication_identity_preimage(&project, &content);
    assert_eq!(preimage.len(), 64);
    assert_eq!(&preimage[..32], &[0x11; 32]);
    assert_eq!(&preimage[32..], &[0x22; 32]);
}

/// The artifact approval id preimage is the fixed domain tag, the
/// publication identity, and the approval instant in nanoseconds since
/// the epoch (big-endian `u128`), so it can never collide with the
/// slice-2 executable approval id derivation or any other hash.
#[test]
fn artifact_approval_id_preimage_is_domain_publication_and_nanos() {
    assert_eq!(
        ARTIFACT_APPROVAL_ID_DOMAIN,
        b"omnifrons-artifact-approval-v1"
    );
    let publication = PublicationIdentity(digest(0x33));
    // A sub-second part that is a multiple of 100 ns: Windows keeps
    // `SystemTime` in 100 ns steps, so 7 ns would round to 0 there.
    let at = SystemTime::UNIX_EPOCH + Duration::new(5, 700);
    let preimage = artifact_approval_id_preimage(&publication, at);
    let nanos: u128 = 5_000_000_700;
    let mut expected = ARTIFACT_APPROVAL_ID_DOMAIN.to_vec();
    expected.extend_from_slice(&[0x33; 32]);
    expected.extend_from_slice(&nanos.to_be_bytes());
    assert_eq!(preimage, expected);
}

#[test]
fn publication_identity_renders_and_parses_as_64_hex() {
    let identity = PublicationIdentity(digest(0xab));
    let hex = identity.to_hex();
    assert_eq!(hex, "ab".repeat(32));
    assert_eq!(PublicationIdentity::from_hex(&hex), Some(identity));
    assert_eq!(PublicationIdentity::from_hex("ab"), None);
}

/// An asset root identity is one token of `[A-Za-z0-9_-]{1,64}`, like a
/// run id: never a separator, never `.` or `..`, never a path.
#[test]
fn asset_root_id_admits_one_token_and_refuses_paths() {
    assert!(AssetRootId::new("main").is_ok());
    assert!(AssetRootId::new("root_2-a").is_ok());
    assert_eq!(AssetRootId::new(""), Err(AssetRootIdError::Empty));
    assert_eq!(
        AssetRootId::new("a/b"),
        Err(AssetRootIdError::InvalidCharacter)
    );
    assert_eq!(
        AssetRootId::new(".."),
        Err(AssetRootIdError::InvalidCharacter)
    );
    assert_eq!(
        AssetRootId::new("x".repeat(65)),
        Err(AssetRootIdError::TooLong)
    );
}

/// The artifact Catalog identity pairs the asset root identity with the
/// publication identity (HAP-001 § Definitions): `<asset root id>/<hex>`.
#[test]
fn catalog_id_pairs_the_asset_root_with_the_publication() {
    let catalog_id = CatalogId {
        asset_root_id: AssetRootId::new("main").expect("valid"),
        publication_id: PublicationIdentity(digest(0xcd)),
    };
    assert_eq!(catalog_id.to_string(), format!("main/{}", "cd".repeat(32)));
}

/// AEC-001's `ref` shape: `{ kind: "artifact", id: <publication identity>,
/// locator: <catalog id> }`, never a device path.
#[test]
fn portable_reference_is_kind_id_locator_with_no_path() {
    let reference = PortableReference::new(CatalogId {
        asset_root_id: AssetRootId::new("main").expect("valid"),
        publication_id: PublicationIdentity(digest(0xef)),
    });
    assert_eq!(PortableReference::KIND, "artifact");
    assert_eq!(reference.id().to_hex(), "ef".repeat(32));
    assert_eq!(
        reference.locator().to_string(),
        format!("main/{}", "ef".repeat(32))
    );
}

// -- states --

/// HAP-001-R26: every state token this slice exposes is one the signal
/// mapping lists, spelled exactly as the contract spells it.
#[test]
fn artifact_state_tokens_are_the_signal_mapping_spellings() {
    let cases = [
        (ArtifactState::Candidate, "candidate"),
        (ArtifactState::PublishedLocal, "published-local"),
        (ArtifactState::Registered, "registered"),
        (ArtifactState::ProviderSynced, "provider-synced"),
        (ArtifactState::RegistrationPending, "registration-pending"),
        (ArtifactState::Refused, "refused"),
        (ArtifactState::IntegrityMismatch, "integrity-mismatch"),
        (ArtifactState::DuplicatePublication, "duplicate-publication"),
        (ArtifactState::OutboxEscape, "outbox-escape"),
        (ArtifactState::OutboxLinked, "outbox-linked"),
    ];
    for (state, token) in cases {
        assert_eq!(state.as_str(), token);
        assert_eq!(ArtifactState::parse(token), Some(state));
    }
    assert_eq!(ArtifactState::ALL.len(), cases.len());
    assert_eq!(ArtifactState::parse("misplaced"), None);
}

#[test]
fn provider_state_tokens_match_the_catalog_record_vocabulary() {
    let cases = [
        (ProviderState::Pending, "pending"),
        (ProviderState::Synced, "synced"),
        (ProviderState::Failed, "failed"),
        (ProviderState::Unavailable, "unavailable"),
    ];
    for (state, token) in cases {
        assert_eq!(state.as_str(), token);
        assert_eq!(ProviderState::parse(token), Some(state));
    }
}

// -- display names (HAP-001-R24, RCS-001's file-name rule) --

/// A traversal sequence never survives: separators and `..` components
/// are removed and the remaining components joined, so the name can never
/// be resolved as a path.
#[test]
fn display_name_strips_traversal_sequences() {
    assert_eq!(
        DisplayName::sanitize("../../etc/passwd").as_str(),
        "etc_passwd"
    );
    assert_eq!(DisplayName::sanitize("a/b\\c.pdf").as_str(), "a_b_c.pdf");
    assert_eq!(DisplayName::sanitize("./report.pdf").as_str(), "report.pdf");
    assert_eq!(DisplayName::sanitize("..").as_str(), "unnamed");
}

/// A reserved device name (Windows: CON, PRN, AUX, NUL, COM1-9, LPT1-9),
/// with or without an extension and in any case, is prefixed so it never
/// names a device.
#[test]
fn display_name_neutralizes_reserved_device_names() {
    assert_eq!(DisplayName::sanitize("CON").as_str(), "_CON");
    assert_eq!(DisplayName::sanitize("nul.pdf").as_str(), "_nul.pdf");
    assert_eq!(DisplayName::sanitize("com1.txt").as_str(), "_com1.txt");
    assert_eq!(DisplayName::sanitize("LPT9").as_str(), "_LPT9");
    assert_eq!(DisplayName::sanitize("console.pdf").as_str(), "console.pdf");
    assert_eq!(DisplayName::sanitize("com10.txt").as_str(), "com10.txt");
}

/// Bidirectional override and isolate characters, and every C0/DEL
/// control, are stripped -- a name that would render `report\u{202E}fdp.exe`
/// as `reportexe.pdf` loses the override.
#[test]
fn display_name_strips_bidi_overrides_and_controls() {
    assert_eq!(
        DisplayName::sanitize("report\u{202E}fdp.exe").as_str(),
        "reportfdp.exe"
    );
    assert_eq!(
        DisplayName::sanitize(
            "a\u{202A}b\u{202B}c\u{202C}d\u{202D}e\u{2066}f\u{2067}g\u{2068}h\u{2069}"
        )
        .as_str(),
        "abcdefgh"
    );
    assert_eq!(
        DisplayName::sanitize("na\u{1b}me\u{0}.pdf\u{7f}").as_str(),
        "name.pdf"
    );
}

/// Excessive length is truncated to the cap on a character boundary --
/// never a split code point -- and a trailing dot or space (which Windows
/// drops silently) is trimmed.
#[test]
fn display_name_caps_length_on_a_char_boundary_and_trims_trailing_dots() {
    let long = "é".repeat(200);
    let sanitized = DisplayName::sanitize(&long);
    assert!(sanitized.as_str().len() <= DisplayName::MAX_BYTES);
    assert!(sanitized.as_str().chars().all(|c| c == 'é'));
    assert_eq!(sanitized.as_str().len(), DisplayName::MAX_BYTES - 1);
    assert_eq!(DisplayName::sanitize("report. . ").as_str(), "report");
    assert_eq!(DisplayName::sanitize("   ").as_str(), "unnamed");
}

/// R3-004: the cap is exact -- 255 single-byte characters pass unchanged,
/// 256 are cut to 255.
#[test]
fn display_name_length_boundary_is_exact_at_the_cap() {
    let exact = "a".repeat(DisplayName::MAX_BYTES);
    assert_eq!(DisplayName::sanitize(&exact).as_str(), exact);
    let over = "a".repeat(DisplayName::MAX_BYTES + 1);
    assert_eq!(DisplayName::sanitize(&over).as_str(), exact);
    assert_eq!(DisplayName::MAX_BYTES, 255);
}

/// R3-008: leading whitespace is trimmed -- it would only hide where the
/// name starts in a list -- while interior whitespace is kept.
#[test]
fn display_name_trims_leading_whitespace_and_keeps_interior_whitespace() {
    assert_eq!(DisplayName::sanitize("  report.pdf").as_str(), "report.pdf");
    assert_eq!(
        DisplayName::sanitize("my report.pdf").as_str(),
        "my report.pdf"
    );
    assert_eq!(
        DisplayName::sanitize("  ").as_str(),
        DisplayName::EMPTY_FALLBACK
    );
}

// -- the approval's location fact (spike slice 5c, HAP-001-R11, R36) --

/// An approval fixture named `name`, listed by `run_id`'s inventory when
/// `Some`, or by the whole-outbox inventory (no run) when `None`.
fn approval_named(run_id: Option<RunId>, name: &str) -> ArtifactApproval {
    ArtifactApproval {
        approval_id: ArtifactApprovalId(1),
        publication_id: PublicationIdentity(digest(0x33)),
        project: ProjectIdentity(digest(0x11)),
        run_id,
        name: name.to_string(),
        display_name: DisplayName::sanitize(name),
        digest: digest(0x22),
        size: 1,
        detected_type: DetectedType::Pdf,
        class: ArtifactClass::GeneratedHeavy,
        attribution: Attribution::Unattributed,
        asset_root_id: AssetRootId::new("main").expect("valid"),
        adapter_id: None,
        executable_approval: None,
        approver: DeviceLocalUser,
        approved_at: SystemTime::UNIX_EPOCH,
    }
}

/// HAP-001-R11, R36: the run subdirectory an entry was found under is a
/// location fact only -- the run whose inventory listed it, or, for an
/// entry approved from the whole-outbox inventory (no run), the run id its
/// outbox-relative name carries as a prefix; an outbox-root entry was found
/// under none, and a prefix that is not a run id is not a location fact.
#[test]
fn approval_found_under_is_the_listing_run_or_the_names_run_prefix() {
    let run = RunId::new("run-1").expect("valid");
    let old = RunId::new("run-old").expect("valid");
    assert_eq!(approval_named(None, "dropped.pdf").found_under(), None);
    assert_eq!(
        approval_named(None, "run-old/stray.png").found_under(),
        Some(old)
    );
    assert_eq!(
        approval_named(Some(run.clone()), "run-1/report.pdf").found_under(),
        Some(run)
    );
    assert_eq!(
        approval_named(None, "not a run id/x.pdf").found_under(),
        None,
        "a prefix that is not a run id is not a location fact"
    );
}
