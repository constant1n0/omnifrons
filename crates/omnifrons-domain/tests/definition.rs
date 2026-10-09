//! Domain tests for one directory's agent-identity classification
//! (ADR-0005 "Agent identity and portable definition" § Classification
//! rules, sub-slice 1a). Translated from the `base_omnifrons` prototype's
//! `tests/test_portable_agent.py`: `test_scan_classifies_pair_states`,
//! `test_scan_classifies_secondary_files_and_host_extras`,
//! `test_scan_classifies_unsafe_inputs`,
//! `test_directories_without_identity_files_are_none`, and
//! `test_scan_titles`. The prototype drives these through its CLI's
//! `scan` command against real files on disk; this module is
//! filesystem-free, so every fixture below builds a
//! [`DirectoryIdentity`] directly and asserts through [`classify`] or
//! [`title`] instead of a scan report. Two of the prototype's checks
//! have no equivalent here and are called out at their call site: the
//! JSON schema of a symlinked identity file (no serialization exists in
//! this slice), and a scan's empty agent list (no `scan` exists in this
//! slice; see ADR-0005's sub-slice 1e).

use omnifrons_domain::definition::{
    DirectoryIdentity, DuplicateIdentityKindError, FileShape, IdentityFile, IdentityKind,
    RootState, WRAPPER, classify, title,
};

/// A fixture body with a title and a second line, byte-identical to the
/// prototype's `SIMPLE_BODY`.
const SIMPLE_BODY: &[u8] = b"# Simple\n\nYou are Simple, a fixture agent.\n";

/// A fixture body that shares `SIMPLE_BODY`'s title but not its content,
/// byte-identical to the prototype's `OTHER_BODY`.
const OTHER_BODY: &[u8] = b"# Simple\n\nYou are Simple, a different fixture agent.\n";

/// A CRLF-terminated fixture body, byte-identical to the prototype's
/// `CRLF_BODY`.
const CRLF_BODY: &[u8] = b"# Crlf\r\n\r\nYou are Crlf.\r\nLines end with CRLF.\r\n";

/// A Spanish, no-heading fixture body (no trailing newline),
/// byte-identical to the prototype's `NARA_BODY`.
const NARA_BODY: &[u8] =
    "Eres Nara, una asistente de comunicación para equipos pequeños.\n\n## Estilo\n\n\
     - Responde en español, con calma y precisión."
        .as_bytes();

/// The prototype's default `near_wrapper_agent` wrapper: indented,
/// CRLF-terminated, with a trailing blank line -- a near-wrapper by
/// shape (import-only, not byte-exact), not by any single defect.
const DEFAULT_NEAR_WRAPPER: &[u8] = b"  @./AGENTS.md\r\n\n";

/// A regular identity file holding `bytes`.
fn identity_file(kind: IdentityKind, bytes: &[u8]) -> IdentityFile {
    IdentityFile {
        kind,
        bytes: bytes.to_vec(),
        shape: FileShape::Regular,
    }
}

/// A symlinked identity file; never read, so it carries no bytes.
fn symlink_file(kind: IdentityKind, target: &str) -> IdentityFile {
    IdentityFile {
        kind,
        bytes: Vec::new(),
        shape: FileShape::Symlink {
            target: target.to_string(),
        },
    }
}

/// An identity file that exists but is neither regular nor a symlink
/// (the prototype's directory-where-a-file-was-expected case).
fn irregular_file(kind: IdentityKind) -> IdentityFile {
    IdentityFile {
        kind,
        bytes: Vec::new(),
        shape: FileShape::Irregular,
    }
}

/// A directory holding exactly `files`, with no host extras. Every
/// fixture in this file uses distinct kinds, so the only way this can
/// fail is a mistake in the fixture itself.
fn directory(files: Vec<IdentityFile>) -> DirectoryIdentity {
    DirectoryIdentity::new(files, Vec::new()).expect("fixture has no duplicate kind")
}

/// `dir`, with its host extras set to `extras` (the prototype's
/// `find_host_extras` result, supplied directly since this module does
/// not walk a filesystem).
fn with_host_extras(dir: DirectoryIdentity, extras: &[&str]) -> DirectoryIdentity {
    dir.with_host_extras(extras.iter().map(ToString::to_string).collect())
}

/// A canonical pair (`AGENTS.md` = [`SIMPLE_BODY`], `CLAUDE.md` =
/// [`WRAPPER`]) plus whatever secondary or extra files `extra` adds
/// (the prototype's `canonical_agent`).
fn canonical_dir(extra: Vec<IdentityFile>) -> DirectoryIdentity {
    let mut files = vec![
        identity_file(IdentityKind::Agents, SIMPLE_BODY),
        identity_file(IdentityKind::Claude, WRAPPER),
    ];
    files.extend(extra);
    directory(files)
}

/// Only `CLAUDE.md`, holding `body` (the prototype's `claude_only_agent`).
fn claude_only_dir(body: &[u8]) -> DirectoryIdentity {
    directory(vec![identity_file(IdentityKind::Claude, body)])
}

/// Only `AGENTS.md` (the prototype's `agents_only_agent`).
fn agents_only_dir() -> DirectoryIdentity {
    directory(vec![identity_file(IdentityKind::Agents, SIMPLE_BODY)])
}

/// `AGENTS.md` and `CLAUDE.md`, byte-identical but not the wrapper (the
/// prototype's `identical_copies_agent`).
fn identical_copies_dir() -> DirectoryIdentity {
    directory(vec![
        identity_file(IdentityKind::Agents, SIMPLE_BODY),
        identity_file(IdentityKind::Claude, SIMPLE_BODY),
    ])
}

/// `AGENTS.md` plus a `CLAUDE.md` that imports it without being
/// byte-exact (the prototype's `near_wrapper_agent`).
fn near_wrapper_dir(wrapper: &[u8]) -> DirectoryIdentity {
    directory(vec![
        identity_file(IdentityKind::Agents, SIMPLE_BODY),
        identity_file(IdentityKind::Claude, wrapper),
    ])
}

/// `AGENTS.md` and `CLAUDE.md`, both content-bearing and different (the
/// prototype's `conflict_agent`).
fn conflict_dir() -> DirectoryIdentity {
    directory(vec![
        identity_file(IdentityKind::Agents, SIMPLE_BODY),
        identity_file(IdentityKind::Claude, OTHER_BODY),
    ])
}

/// Fails with every mis-classified case's label and expected-vs-actual
/// state token (the prototype's `assert_states`).
fn assert_states(cases: &[(&str, DirectoryIdentity, &str)]) {
    let wrong: Vec<String> = cases
        .iter()
        .filter_map(|(label, dir, expected)| {
            let actual = classify(dir).state.as_str();
            (actual != *expected).then(|| format!("{label}: expected {expected}, got {actual}"))
        })
        .collect();
    assert!(wrong.is_empty(), "misclassified: {wrong:?}");
}

/// Translated from the prototype's `test_scan_classifies_pair_states`:
/// the pair rules (6-11) and the near-wrapper shapes that stay
/// `near_wrapper` or fall to `conflict` once the wrapper carries more
/// than the import. `premigration_atlas` and `echo` stand in for the
/// prototype's real Atlas/Echo fixtures: only the pair shape decides
/// `classify`'s state, so a plain claude-only pair is the faithful
/// translation.
#[test]
fn classify_reports_the_pair_and_near_wrapper_states() {
    assert_states(&[
        ("canonical", canonical_dir(Vec::new()), "canonical"),
        ("claude_only", claude_only_dir(SIMPLE_BODY), "claude_only"),
        ("agents_only", agents_only_dir(), "agents_only"),
        (
            "identical_copies",
            identical_copies_dir(),
            "identical_copies",
        ),
        (
            "near_wrapper",
            near_wrapper_dir(DEFAULT_NEAR_WRAPPER),
            "near_wrapper",
        ),
        ("conflict", conflict_dir(), "conflict"),
        (
            "premigration_atlas",
            claude_only_dir(SIMPLE_BODY),
            "claude_only",
        ),
        ("echo", claude_only_dir(SIMPLE_BODY), "claude_only"),
        ("nara", claude_only_dir(NARA_BODY), "claude_only"),
        ("crlf", claude_only_dir(CRLF_BODY), "claude_only"),
        (
            "near_no_newline",
            near_wrapper_dir(b"@AGENTS.md"),
            "near_wrapper",
        ),
        (
            "near_crlf",
            near_wrapper_dir(b"@AGENTS.md\r\n"),
            "near_wrapper",
        ),
        (
            "near_blank_lines",
            near_wrapper_dir(b"\n@./AGENTS.md\n\n"),
            "near_wrapper",
        ),
        (
            "wrapper_plus_text",
            near_wrapper_dir(b"@AGENTS.md\nAlso be brief.\n"),
            "conflict",
        ),
    ]);

    // `assert state_of(atlas) == "canonical"` in the prototype.
    assert_eq!(
        classify(&canonical_dir(Vec::new())).state.as_str(),
        "canonical"
    );
    assert_eq!(
        classify(&canonical_dir(Vec::new())).reason,
        "CLAUDE.md is the exact AGENTS.md wrapper"
    );
    assert_eq!(
        classify(&conflict_dir()).reason,
        "CLAUDE.md and AGENTS.md hold different content"
    );
}

/// Translated from the prototype's
/// `test_scan_classifies_secondary_files_and_host_extras`: a secondary
/// file that is itself a wrapper or a byte copy never conflicts; one
/// that differs always does; a host extra only changes the state when
/// the root is otherwise canonical.
#[test]
fn secondary_files_that_differ_conflict_and_host_extras_mark_canonical() {
    assert_states(&[
        (
            "gemini_wrapper",
            canonical_dir(vec![identity_file(IdentityKind::Gemini, WRAPPER)]),
            "canonical",
        ),
        (
            "gemini_copy",
            canonical_dir(vec![identity_file(IdentityKind::Gemini, SIMPLE_BODY)]),
            "canonical",
        ),
        (
            "gemini_differs",
            canonical_dir(vec![identity_file(IdentityKind::Gemini, OTHER_BODY)]),
            "conflict",
        ),
        (
            "qwen_differs",
            canonical_dir(vec![identity_file(IdentityKind::Qwen, OTHER_BODY)]),
            "conflict",
        ),
        (
            "dot_claude_differs",
            canonical_dir(vec![identity_file(IdentityKind::ClaudeDir, OTHER_BODY)]),
            "conflict",
        ),
        (
            "override_differs",
            canonical_dir(vec![identity_file(
                IdentityKind::AgentsOverride,
                OTHER_BODY,
            )]),
            "conflict",
        ),
        (
            "claude_local",
            with_host_extras(canonical_dir(Vec::new()), &["CLAUDE.local.md"]),
            "host_extras",
        ),
        (
            "cursor_rules",
            with_host_extras(canonical_dir(Vec::new()), &[".cursor/rules"]),
            "host_extras",
        ),
        (
            "copilot",
            with_host_extras(
                canonical_dir(Vec::new()),
                &[".github/copilot-instructions.md"],
            ),
            "host_extras",
        ),
        (
            "claude_only_with_extra",
            with_host_extras(claude_only_dir(SIMPLE_BODY), &[".cursorrules"]),
            "claude_only",
        ),
    ]);

    assert_eq!(
        classify(&canonical_dir(vec![identity_file(
            IdentityKind::Gemini,
            OTHER_BODY
        )]))
        .reason,
        "GEMINI.md differ from AGENTS.md"
    );
    assert_eq!(
        classify(&with_host_extras(
            canonical_dir(Vec::new()),
            &["CLAUDE.local.md"]
        ))
        .reason,
        "canonical, with host extras: CLAUDE.local.md"
    );
}

/// Translated from the prototype's `test_scan_classifies_unsafe_inputs`:
/// every shape `classify` refuses to trust, plus the two symlink
/// spellings it accepts as the wrapper.
#[test]
fn classify_reports_unsafe_for_bad_shapes_and_missing_sources() {
    let foreign = directory(vec![
        identity_file(IdentityKind::Agents, SIMPLE_BODY),
        symlink_file(IdentityKind::Claude, "../elsewhere.md"),
    ]);
    let dot_claude_dir_link =
        canonical_dir(vec![symlink_file(IdentityKind::ClaudeDir, "../shared")]);

    assert_states(&[
        ("dot_claude_dir_link", dot_claude_dir_link, "unsafe"),
        ("claude_link_elsewhere", foreign.clone(), "unsafe"),
        (
            "claude_link_to_agents",
            directory(vec![
                identity_file(IdentityKind::Agents, SIMPLE_BODY),
                symlink_file(IdentityKind::Claude, "AGENTS.md"),
            ]),
            "canonical",
        ),
        (
            "claude_link_dot_agents",
            directory(vec![
                identity_file(IdentityKind::Agents, SIMPLE_BODY),
                symlink_file(IdentityKind::Claude, "./AGENTS.md"),
            ]),
            "canonical",
        ),
        (
            "agents_link",
            directory(vec![
                symlink_file(IdentityKind::Agents, "real.md"),
                identity_file(IdentityKind::Claude, SIMPLE_BODY),
            ]),
            "unsafe",
        ),
        (
            "claude_is_directory",
            directory(vec![
                identity_file(IdentityKind::Agents, SIMPLE_BODY),
                irregular_file(IdentityKind::Claude),
            ]),
            "unsafe",
        ),
        (
            "gemini_only",
            directory(vec![identity_file(IdentityKind::Gemini, SIMPLE_BODY)]),
            "unsafe",
        ),
        (
            "qwen_only",
            directory(vec![identity_file(IdentityKind::Qwen, SIMPLE_BODY)]),
            "unsafe",
        ),
        (
            "dot_claude_only",
            directory(vec![identity_file(IdentityKind::ClaudeDir, SIMPLE_BODY)]),
            "unsafe",
        ),
        (
            "override_only",
            directory(vec![identity_file(
                IdentityKind::AgentsOverride,
                SIMPLE_BODY,
            )]),
            "unsafe",
        ),
        (
            "wrapper_without_agents",
            directory(vec![identity_file(IdentityKind::Claude, WRAPPER)]),
            "unsafe",
        ),
        (
            "agents_self_import",
            directory(vec![
                identity_file(IdentityKind::Claude, SIMPLE_BODY),
                identity_file(IdentityKind::Agents, WRAPPER),
            ]),
            "unsafe",
        ),
    ]);

    // The prototype additionally checks the symlinked `CLAUDE.md`'s
    // `to_json` shape; this slice has no serialization yet, so the
    // nearest equivalent is that the target survives into the reason
    // verbatim.
    assert_eq!(
        classify(&foreign).reason,
        "CLAUDE.md is a symlink to ../elsewhere.md"
    );
    assert_eq!(
        classify(&directory(vec![identity_file(
            IdentityKind::Gemini,
            SIMPLE_BODY
        )]))
        .reason,
        "no CLAUDE.md or AGENTS.md source; v1 does not adapt GEMINI.md"
    );
    assert_eq!(
        classify(&directory(vec![
            identity_file(IdentityKind::Agents, SIMPLE_BODY),
            irregular_file(IdentityKind::Claude),
        ]))
        .reason,
        "CLAUDE.md is not a regular file"
    );
}

/// Translated from the prototype's
/// `test_directories_without_identity_files_are_none`: no identity file
/// means no root, even when host extras are present -- extras alone
/// never create a root to report them on. The prototype's third
/// assertion, that a `scan` of an empty directory finds no agents, has
/// no equivalent here: this slice has no `scan`.
#[test]
fn directories_without_identity_files_are_none_even_with_host_extras() {
    let empty = DirectoryIdentity::default();
    let extras_only = with_host_extras(DirectoryIdentity::default(), &[".cursorrules"]);

    assert_eq!(classify(&empty).state, RootState::None);
    assert_eq!(classify(&extras_only).state, RootState::None);
    assert_eq!(classify(&empty).reason, "no identity files");
}

/// Translated from the prototype's `test_scan_titles`. The prototype
/// reads the title off the scan report for a real nested agent and
/// three synthetic ones; since `title` takes bytes directly, this
/// translation drops the scan report and calls it on equivalent bytes,
/// including a stand-in for the real, nested Echo fixture's heading.
#[test]
fn title_finds_the_first_unfenced_heading_or_none() {
    let echo_body = b"# Echo\n\nYou are Echo, a nested fixture agent.\n";
    let fenced_body = b"```\n# Not a title\n```\n\n# Real title\n";

    assert_eq!(title(echo_body), Some("Echo".to_string()));
    assert_eq!(title(NARA_BODY), None);
    assert_eq!(title(CRLF_BODY), Some("Crlf".to_string()));
    assert_eq!(title(fenced_body), Some("Real title".to_string()));
}

/// Review follow-up: `irregular_shape`'s offender and the "no source"
/// reason both used to iterate `identity_files` in the caller's vector
/// order, even though [`DirectoryIdentity`] documents that order as
/// unspecified -- two directories holding the same files in a different
/// push order could get different reasons (review finding 1). Both now
/// iterate [`IdentityKind::ALL`] instead, so the reason is the same
/// regardless of construction order, including with two offenders.
#[test]
fn classify_reasons_are_independent_of_identity_files_vector_order() {
    let forward = directory(vec![
        symlink_file(IdentityKind::Claude, "../elsewhere.md"),
        irregular_file(IdentityKind::Qwen),
    ]);
    let backward = directory(vec![
        irregular_file(IdentityKind::Qwen),
        symlink_file(IdentityKind::Claude, "../elsewhere.md"),
    ]);
    assert_eq!(classify(&forward), classify(&backward));
    assert_eq!(
        classify(&forward).reason,
        "CLAUDE.md is a symlink to ../elsewhere.md"
    );

    let forward_no_source = directory(vec![
        identity_file(IdentityKind::Qwen, SIMPLE_BODY),
        identity_file(IdentityKind::Gemini, SIMPLE_BODY),
    ]);
    let backward_no_source = directory(vec![
        identity_file(IdentityKind::Gemini, SIMPLE_BODY),
        identity_file(IdentityKind::Qwen, SIMPLE_BODY),
    ]);
    assert_eq!(classify(&forward_no_source), classify(&backward_no_source));
    assert_eq!(
        classify(&forward_no_source).reason,
        "no CLAUDE.md or AGENTS.md source; v1 does not adapt GEMINI.md, QWEN.md"
    );
}

/// Review follow-up: `Classification::source` was so far only checked
/// indirectly, through the reason strings; this pins it directly for
/// every state (review finding 2).
#[test]
fn classification_source_matches_the_state_for_every_outcome() {
    assert_eq!(
        classify(&claude_only_dir(SIMPLE_BODY)).source,
        Some(IdentityKind::Claude)
    );
    assert_eq!(
        classify(&agents_only_dir()).source,
        Some(IdentityKind::Agents)
    );
    assert_eq!(
        classify(&canonical_dir(Vec::new())).source,
        Some(IdentityKind::Agents)
    );
    assert_eq!(
        classify(&near_wrapper_dir(DEFAULT_NEAR_WRAPPER)).source,
        Some(IdentityKind::Agents)
    );
    assert_eq!(
        classify(&identical_copies_dir()).source,
        Some(IdentityKind::Agents)
    );
    assert_eq!(
        classify(&with_host_extras(
            canonical_dir(Vec::new()),
            &["CLAUDE.local.md"]
        ))
        .source,
        Some(IdentityKind::Agents)
    );
    assert_eq!(classify(&conflict_dir()).source, None);
    assert_eq!(
        classify(&directory(vec![identity_file(
            IdentityKind::Gemini,
            SIMPLE_BODY
        )]))
        .source,
        None
    );
    assert_eq!(classify(&DirectoryIdentity::default()).source, None);
}

/// Review follow-up: a byte-order mark and an empty file, checked
/// directly against the prototype's `classify`/`is_import_only` (review
/// finding 3). A BOM is not whitespace, so it keeps its token from
/// matching the import spelling; an empty file has no tokens at all, so
/// it is vacuously not import-only either -- both read as
/// content-bearing, exactly as the prototype's `data.split()` does.
/// None of the three expectations below contradicts the prototype.
#[test]
fn bom_and_empty_identity_files_classify_as_the_prototype_does() {
    assert_eq!(
        classify(&near_wrapper_dir(b"\xEF\xBB\xBF@AGENTS.md\n"))
            .state
            .as_str(),
        "conflict"
    );
    assert_eq!(classify(&near_wrapper_dir(b"")).state.as_str(), "conflict");
    assert_eq!(
        classify(&directory(vec![
            identity_file(IdentityKind::Agents, b""),
            identity_file(IdentityKind::Claude, WRAPPER),
        ]))
        .state
        .as_str(),
        "canonical"
    );
}

/// Review follow-up: two `title` behaviors not covered by the
/// prototype-translated test (review finding 4) -- a byte-order mark is
/// stripped before heading detection, and a tilde fence hides a heading
/// exactly like a backtick fence.
#[test]
fn title_strips_a_bom_and_reads_a_tilde_fence() {
    assert_eq!(title(b"\xEF\xBB\xBF# Title\n"), Some("Title".to_string()));
    assert_eq!(
        title(b"~~~\n# Not a title\n~~~\n\n# Real title\n"),
        Some("Real title".to_string())
    );
}

/// Review follow-up: `CommonMark`'s ATX closing-sequence rule (review
/// finding 5) -- a trailing `#` run is a closing sequence, and so
/// stripped, only when it is preceded by whitespace or is the entire
/// heading. `# C#` keeps its `#` (nothing precedes it but the letter
/// `C`); `# Title #` drops it (preceded by a space); `# #` reduces to
/// empty, which the prototype's `extract_title` also treats as no
/// title, since `str.rstrip("#")` empties it the same way.
#[test]
fn title_strips_a_closing_hash_run_only_when_commonmark_allows_it() {
    assert_eq!(title(b"# C#\n"), Some("C#".to_string()));
    assert_eq!(title(b"# Title #\n"), Some("Title".to_string()));
    assert_eq!(title(b"# #\n"), None);
}

/// Review follow-up: every token `as_str` produces parses back to the
/// same variant, and an unrecognized token parses to `None` (review
/// finding 7).
#[test]
fn root_state_and_identity_kind_tokens_round_trip() {
    for state in RootState::ALL {
        assert_eq!(RootState::parse(state.as_str()), Some(state));
    }
    assert_eq!(RootState::parse("not-a-state"), None);

    for kind in IdentityKind::ALL {
        assert_eq!(IdentityKind::parse(kind.as_str()), Some(kind));
    }
    assert_eq!(IdentityKind::parse("not-a-kind"), None);
}

/// Review follow-up: construction rejects more than one identity file
/// of the same kind, rather than leaving `classify`'s kind-keyed
/// lookups to silently pick whichever happens to be found first (review
/// finding 6).
#[test]
fn directory_identity_new_rejects_a_duplicate_kind() {
    let duplicate = DirectoryIdentity::new(
        vec![
            identity_file(IdentityKind::Claude, SIMPLE_BODY),
            identity_file(IdentityKind::Claude, OTHER_BODY),
        ],
        Vec::new(),
    );
    assert!(matches!(
        duplicate,
        Err(DuplicateIdentityKindError {
            kind: IdentityKind::Claude
        })
    ));
}
