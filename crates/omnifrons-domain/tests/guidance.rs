//! The agent guidance note and the managed blocks Omnifrons owns in a
//! project's text files (spike slice 5c, HAP-001 § Agent guidance note,
//! D18, R42): the fixed template with only the outbox path substituted,
//! the user-named guidance file under RCS-001's file-name rule, the
//! sentinels per kind with the body digest pinned, pure locating with its
//! three refusals, the status, and the pure apply and remove plans that
//! keep line endings as found. Framework-independent and filesystem-free:
//! this crate has no hasher, so a test double supplies the digest function
//! and the module supplies the bytes it hashes.

use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::guidance::{
    ApplyAction, DEFAULT_GUIDANCE_FILE, GUIDANCE_TEMPLATE_VERSION, GuidanceNote, IGNORE_FILE,
    LineEnding, Located, ManagedBlock, ManagedFileError, ManagedFileKind, ManagedFileName,
    ManagedFileNameError, ManagedStatus, ManagedTarget, plan_apply, plan_remove, status,
};
use omnifrons_domain::outbox::OutboxPath;

/// A deterministic, non-cryptographic digest double: enough to tell two
/// bodies apart and to pin a sentinel to a body.
fn fake_digest(bytes: &[u8]) -> Sha256Digest {
    let mut out = [0u8; 32];
    for (index, byte) in bytes.iter().enumerate() {
        let lane = &mut out[index % 32];
        *lane = lane.wrapping_add(*byte).rotate_left(3) ^ 0x5a;
    }
    out[31] = u8::try_from(bytes.len() % 251).expect("fits");
    Sha256Digest(out)
}

fn outbox() -> OutboxPath {
    OutboxPath::default_path()
}

fn guidance_block() -> ManagedBlock {
    ManagedBlock::guidance(&outbox(), &fake_digest)
}

fn ignore_block() -> ManagedBlock {
    ManagedBlock::ignore(&outbox(), &fake_digest)
}

/// The rendered block in a file, between `before` and `after`.
fn file_with(before: &str, block: &ManagedBlock, after: &str) -> String {
    format!("{before}{}{after}", block.render(LineEnding::Lf))
}

// -- the template (HAP-001-R42) --

/// The note is HAP-001's fixed template, verbatim, with only the outbox
/// path substituted. The expected text below mirrors
/// `docs/heavy-asset-publication.md` § Agent guidance note, the fenced
/// block at lines 316-324 (heading, blank line, seven bullets), with
/// `<outbox path>` replaced by `.omnifrons/outbox`; the document is the
/// source of truth and this test is what keeps the two equal.
#[test]
fn the_note_is_hap_001s_template_with_only_the_outbox_path_substituted() {
    let expected = "## Generated files\n\
\n\
- Write every generated file that is not a Markdown note — documents, images, audio, video, datasets, archives, exports — into `.omnifrons/outbox`, relative to this project's root. Create the directory if it does not exist.\n\
- If your environment names an output directory for this run, write there instead; it is a subdirectory of `.omnifrons/outbox`, and a file you also declare in a publish proposal is recorded as this run's output.\n\
- Keep Markdown notes where this project already keeps its notes, never in `.omnifrons/outbox`.\n\
- Never write large or binary files into source or configuration paths, and never commit them; `.omnifrons/outbox` should be listed in this project's ignore rules.\n\
- Use a new, descriptive file name for every output; do not overwrite an existing file in `.omnifrons/outbox`.\n\
- Treat `.omnifrons/outbox` as write-only: do not read, execute, or delete what is there.\n\
- Unless this project has explicitly allowed automatic approval for outputs a run itself declared, a person reviews everything written to `.omnifrons/outbox` before it is used anywhere else.\n";
    assert_eq!(GuidanceNote::render(&outbox()), expected);
    assert_eq!(GUIDANCE_TEMPLATE_VERSION, "hap-001-guidance-v1");

    // Only the path changes: seven substitutions, nothing else.
    let other = OutboxPath::new("artifacts/outbox").expect("valid");
    let rendered = GuidanceNote::render(&other);
    assert_eq!(rendered.matches("`artifacts/outbox`").count(), 7);
    assert_eq!(
        rendered.replace("artifacts/outbox", ".omnifrons/outbox"),
        expected
    );
}

// -- the managed file names (RCS-001's file-name rule, refusing rather
// than normalizing) --

#[test]
fn a_guidance_file_is_one_markdown_component_at_the_workspace_root() {
    assert_eq!(DEFAULT_GUIDANCE_FILE, "AGENTS.md");
    assert_eq!(IGNORE_FILE, ".gitignore");
    assert_eq!(ManagedFileName::default_guidance().as_str(), "AGENTS.md");
    for valid in ["AGENTS.md", "CLAUDE.md", "notes.MD", "my agents.md"] {
        assert_eq!(
            ManagedFileName::guidance(valid).map(|name| name.as_str().to_string()),
            Ok(valid.to_string()),
            "{valid}"
        );
    }
    let too_long = format!("{}.md", "a".repeat(253));
    assert_eq!(too_long.len(), 256);
    let cases = [
        ("", ManagedFileNameError::Empty),
        ("docs/AGENTS.md", ManagedFileNameError::Separator),
        ("docs\\AGENTS.md", ManagedFileNameError::Separator),
        ("/AGENTS.md", ManagedFileNameError::Separator),
        (".", ManagedFileNameError::Traversal),
        ("..", ManagedFileNameError::Traversal),
        ("AGENTS\u{202E}.md", ManagedFileNameError::Control),
        ("AGENTS\u{2066}.md", ManagedFileNameError::Control),
        ("AGENTS\u{7}.md", ManagedFileNameError::Control),
        ("AGENTS\n.md", ManagedFileNameError::Control),
        (too_long.as_str(), ManagedFileNameError::TooLong),
        ("AGENTS.md ", ManagedFileNameError::EdgeCharacter),
        (" AGENTS.md", ManagedFileNameError::EdgeCharacter),
        ("AGENTS.md.", ManagedFileNameError::EdgeCharacter),
        ("CON.md", ManagedFileNameError::ReservedDeviceName),
        ("lpt1.md", ManagedFileNameError::ReservedDeviceName),
        ("AGENTS.txt", ManagedFileNameError::NotMarkdown),
        ("AGENTS", ManagedFileNameError::NotMarkdown),
        (".gitignore", ManagedFileNameError::NotMarkdown),
    ];
    for (raw, error) in cases {
        assert_eq!(ManagedFileName::guidance(raw), Err(error), "{raw:?}");
    }
    let exact = format!("{}.md", "a".repeat(252));
    assert_eq!(exact.len(), 255);
    assert!(ManagedFileName::guidance(&exact).is_ok(), "255 bytes pass");
}

#[test]
fn a_target_names_the_file_its_kind_manages() {
    let guidance = ManagedTarget::guidance(ManagedFileName::guidance("CLAUDE.md").expect("valid"));
    assert_eq!(guidance.kind(), ManagedFileKind::Guidance);
    assert_eq!(guidance.file_name(), "CLAUDE.md");
    let ignore = ManagedTarget::ignore();
    assert_eq!(ignore.kind(), ManagedFileKind::Ignore);
    assert_eq!(ignore.file_name(), ".gitignore");
    assert_eq!(ManagedFileKind::Guidance.as_str(), "guidance");
    assert_eq!(ManagedFileKind::Ignore.as_str(), "ignore");
    assert_eq!(
        ManagedFileKind::parse("ignore"),
        Some(ManagedFileKind::Ignore)
    );
    assert_eq!(ManagedFileKind::parse("other"), None);
}

// -- the blocks and their sentinels --

/// A Markdown block wraps the note in HTML-comment sentinels naming the
/// kind, the template version, and the digest of the body bytes; a
/// gitignore block wraps the one ignore rule in `#` sentinels the same
/// way. The body is the canonical LF text without a trailing newline.
#[test]
fn blocks_carry_the_sentinels_of_their_kind_with_the_body_digest_pinned() {
    let guidance = guidance_block();
    let note = GuidanceNote::render(&outbox());
    let body = note.trim_end_matches('\n');
    assert_eq!(guidance.body(), body);
    assert_eq!(guidance.version(), GUIDANCE_TEMPLATE_VERSION);
    assert_eq!(guidance.digest(), fake_digest(body.as_bytes()));
    let rendered = guidance.render(LineEnding::Lf);
    assert_eq!(
        rendered,
        format!(
            "<!-- omnifrons:begin guidance hap-001-guidance-v1 sha256:{} -->\n{body}\n<!-- omnifrons:end guidance -->",
            fake_digest(body.as_bytes()).to_hex()
        )
    );
    assert!(!rendered.ends_with('\n'), "the caller adds the terminator");

    let ignore = ignore_block();
    assert_eq!(ignore.body(), "/.omnifrons/outbox/");
    assert_eq!(
        ignore.render(LineEnding::Lf),
        format!(
            "# omnifrons:begin ignore hap-001-guidance-v1 sha256:{}\n/.omnifrons/outbox/\n# omnifrons:end ignore",
            fake_digest(b"/.omnifrons/outbox/").to_hex()
        )
    );
    // A declared `./out/` spelling becomes one clean root-anchored rule.
    let dotted = ManagedBlock::ignore(&OutboxPath::new("./out/").expect("valid"), &fake_digest);
    assert_eq!(dotted.body(), "/out/");
    assert_eq!(
        ManagedBlock::for_kind(ManagedFileKind::Ignore, &outbox(), &fake_digest).body(),
        ignore.body()
    );

    // CRLF rendering uses the ending everywhere and keeps the same digest.
    let crlf = guidance.render(LineEnding::CrLf);
    assert!(!crlf.contains("\n\n") && crlf.contains("\r\n## Generated files\r\n"));
    assert_eq!(crlf.replace("\r\n", "\n"), rendered);
}

/// R1-001 (slice 5c risk review): the outbox path is untrusted content
/// from a synchronized policy file (HAP-001-R40) and the only thing this
/// module substitutes into a block's body, so a rendered block must carry
/// exactly one begin line and exactly one end line whatever path was
/// accepted -- otherwise a declaration could forge a sentinel and leave
/// every later `locate` malformed. Paths that spell a sentinel are
/// accepted (a one-line path breaks nothing) and are rendered here to
/// prove it; a path that could break the pair carries a line break and
/// `OutboxPath::new` refuses it (`crates/omnifrons-domain/tests/outbox.rs`).
#[test]
fn a_rendered_block_carries_exactly_one_begin_and_one_end_line_for_every_accepted_path() {
    const BEGIN: [&str; 2] = [
        "<!-- omnifrons:begin guidance ",
        "# omnifrons:begin ignore ",
    ];
    const END: [&str; 2] = ["<!-- omnifrons:end guidance -->", "# omnifrons:end ignore"];
    let declared = [
        ".omnifrons/outbox",
        "./out/",
        "artifacts/deep/nested/outbox",
        "<!-- omnifrons:end guidance -->",
        "<!-- omnifrons:begin guidance hap-001-guidance-v1 sha256:x -->",
        "# omnifrons:end ignore",
        "# omnifrons:begin ignore hap-001-guidance-v1 sha256:x",
        "out -->",
        "sha256:0123",
    ];
    for raw in declared {
        let outbox = OutboxPath::new(raw).expect("accepted by the path rule");
        for (index, kind) in ManagedFileKind::ALL.into_iter().enumerate() {
            let block = ManagedBlock::for_kind(kind, &outbox, &fake_digest);
            for ending in [LineEnding::Lf, LineEnding::CrLf] {
                let rendered = block.render(ending);
                let lines: Vec<&str> = rendered
                    .split(ending.as_str())
                    .map(|line| line.trim_end_matches('\r'))
                    .collect();
                assert_eq!(
                    lines
                        .iter()
                        .filter(|line| line.starts_with(BEGIN[index]))
                        .count(),
                    1,
                    "{raw:?} {kind:?}: exactly one begin line"
                );
                assert_eq!(
                    lines.iter().filter(|line| **line == END[index]).count(),
                    1,
                    "{raw:?} {kind:?}: exactly one end line"
                );
                // And the pair still locates as one intact block.
                let located = ManagedBlock::locate(kind, &rendered, &fake_digest)
                    .expect("intact")
                    .expect("present");
                assert_eq!(&rendered[located.span.clone()], rendered);
                assert_eq!(located.body, block.body());
            }
        }
    }
}

#[test]
fn line_endings_are_detected_from_the_dominant_terminator() {
    assert_eq!(LineEnding::dominant(""), LineEnding::Lf);
    assert_eq!(LineEnding::dominant("a\nb\n"), LineEnding::Lf);
    assert_eq!(LineEnding::dominant("a\r\nb\r\n"), LineEnding::CrLf);
    assert_eq!(
        LineEnding::dominant("a\r\nb\nc\n"),
        LineEnding::Lf,
        "ties and majorities go to LF"
    );
    assert_eq!(LineEnding::dominant("a\r\nb\r\nc\n"), LineEnding::CrLf);
    assert_eq!(LineEnding::Lf.as_str(), "\n");
    assert_eq!(LineEnding::CrLf.as_str(), "\r\n");
}

// -- locating (pure parsing) --

#[test]
fn locate_finds_one_intact_block_and_its_facts_whatever_the_line_ending() {
    let block = guidance_block();
    let text = file_with("# Title\n\n", &block, "\n\nMore text.\n");
    let located = ManagedBlock::locate(ManagedFileKind::Guidance, &text, &fake_digest)
        .expect("intact")
        .expect("present");
    assert_eq!(&text[located.span.clone()], block.render(LineEnding::Lf));
    assert_eq!(located.version, GUIDANCE_TEMPLATE_VERSION);
    assert_eq!(located.sentinel_digest, block.digest());
    assert_eq!(located.body, block.body());

    let crlf = format!(
        "# Title\r\n\r\n{}\r\n\r\nMore text.\r\n",
        block.render(LineEnding::CrLf)
    );
    let located = ManagedBlock::locate(ManagedFileKind::Guidance, &crlf, &fake_digest)
        .expect("intact")
        .expect("present");
    assert_eq!(&crlf[located.span.clone()], block.render(LineEnding::CrLf));
    assert_eq!(located.body, block.body(), "the body is canonical LF");
    assert_eq!(located.sentinel_digest, block.digest());

    // A gitignore block among other rules.
    let ignore = ignore_block();
    let text = file_with("/target\n*.log\n\n", &ignore, "\n");
    let located = ManagedBlock::locate(ManagedFileKind::Ignore, &text, &fake_digest)
        .expect("intact")
        .expect("present");
    assert_eq!(located.body, "/.omnifrons/outbox/");
    // The other kind's sentinels are not this kind's.
    assert_eq!(
        ManagedBlock::locate(ManagedFileKind::Guidance, &text, &fake_digest),
        Ok(None)
    );
}

#[test]
fn locate_reports_absent_malformed_and_modified() {
    let block = guidance_block();
    let rendered = block.render(LineEnding::Lf);
    let (begin, rest) = rendered.split_once('\n').expect("a begin line");
    let end = "<!-- omnifrons:end guidance -->";
    assert_eq!(
        ManagedBlock::locate(ManagedFileKind::Guidance, "# Title\n", &fake_digest),
        Ok(None)
    );
    assert_eq!(
        ManagedBlock::locate(ManagedFileKind::Guidance, "", &fake_digest),
        Ok(None)
    );
    let malformed = [
        format!("{begin}\nbody\n"),
        format!("body\n{end}\n"),
        format!("{end}\nbody\n{begin}\n"),
        format!("{rendered}\n\n{rendered}\n"),
        format!("{begin}\n{rest}\n{end}\n"),
        format!(
            "<!-- omnifrons:begin guidance hap-001-guidance-v1 sha256:nothex -->\nbody\n{end}\n"
        ),
        format!("<!-- omnifrons:begin guidance -->\nbody\n{end}\n"),
    ];
    for text in &malformed {
        assert_eq!(
            ManagedBlock::locate(ManagedFileKind::Guidance, text, &fake_digest),
            Err(ManagedFileError::Malformed),
            "{text:?}"
        );
    }
    let edited = rendered.replace("Create the directory", "Create the folder");
    assert_ne!(edited, rendered);
    assert_eq!(
        ManagedBlock::locate(ManagedFileKind::Guidance, &edited, &fake_digest),
        Err(ManagedFileError::Modified)
    );
}

// -- the status --

#[test]
fn status_distinguishes_absent_current_outdated_modified_and_malformed() {
    let block = guidance_block();
    let current = file_with("# Title\n\n", &block, "\n");
    assert_eq!(status(None, &block, &fake_digest), ManagedStatus::Absent);
    assert_eq!(
        status(Some("# Title\n"), &block, &fake_digest),
        ManagedStatus::Absent
    );
    assert_eq!(
        status(Some(&current), &block, &fake_digest),
        ManagedStatus::Current
    );
    // An intact block of an earlier version.
    let older = current.replace("hap-001-guidance-v1", "hap-001-guidance-v0");
    assert_eq!(
        status(Some(&older), &block, &fake_digest),
        ManagedStatus::Outdated {
            version: "hap-001-guidance-v0".to_string()
        }
    );
    // An intact block of this version rendered for another outbox path:
    // outdated too, reported under the version it carries.
    let elsewhere = ManagedBlock::guidance(
        &OutboxPath::new("artifacts/outbox").expect("valid"),
        &fake_digest,
    );
    let moved = file_with("", &elsewhere, "\n");
    assert_eq!(
        status(Some(&moved), &block, &fake_digest),
        ManagedStatus::Outdated {
            version: GUIDANCE_TEMPLATE_VERSION.to_string()
        }
    );
    let edited = current.replace("Create the directory", "Create the folder");
    assert_eq!(
        status(Some(&edited), &block, &fake_digest),
        ManagedStatus::Modified
    );
    let broken = current.replace("<!-- omnifrons:end guidance -->", "");
    assert_eq!(
        status(Some(&broken), &block, &fake_digest),
        ManagedStatus::Malformed
    );
    for tag in [
        ManagedStatus::Absent,
        ManagedStatus::Current,
        ManagedStatus::Modified,
        ManagedStatus::Malformed,
    ] {
        assert!(!tag.as_str().is_empty());
    }
    assert_eq!(
        ManagedStatus::Outdated {
            version: "x".to_string()
        }
        .as_str(),
        "outdated"
    );
}

// -- the apply plan --

#[test]
fn plan_apply_inserts_after_one_blank_line_creating_the_file_when_absent() {
    let block = guidance_block();
    let rendered = block.render(LineEnding::Lf);
    let plan = plan_apply(None, &block, &fake_digest).expect("plans");
    assert_eq!(plan.action, ApplyAction::Insert);
    assert_eq!(plan.result, format!("{rendered}\n"));
    assert_eq!(
        plan_apply(Some(""), &block, &fake_digest)
            .expect("plans")
            .result,
        format!("{rendered}\n")
    );
    for (current, expected) in [
        ("# Title", format!("# Title\n\n{rendered}\n")),
        ("# Title\n", format!("# Title\n\n{rendered}\n")),
        ("# Title\n\n", format!("# Title\n\n{rendered}\n")),
        ("# Title\n\n\n", format!("# Title\n\n\n{rendered}\n")),
    ] {
        let plan = plan_apply(Some(current), &block, &fake_digest).expect("plans");
        assert_eq!(plan.action, ApplyAction::Insert, "{current:?}");
        assert_eq!(plan.result, expected, "{current:?}");
    }
    // A CRLF file gets a CRLF block and keeps its own endings.
    let plan = plan_apply(Some("# Title\r\n"), &block, &fake_digest).expect("plans");
    assert_eq!(
        plan.result,
        format!("# Title\r\n\r\n{}\r\n", block.render(LineEnding::CrLf))
    );
    assert_eq!(ApplyAction::Insert.as_str(), "insert");
    assert_eq!(ApplyAction::Replace.as_str(), "replace");
    assert_eq!(ApplyAction::NoOp.as_str(), "no-op");
}

#[test]
fn plan_apply_replaces_exactly_the_located_span_and_noops_when_current() {
    let block = guidance_block();
    let older =
        ManagedBlock::guidance(&OutboxPath::new("old/outbox").expect("valid"), &fake_digest);
    let text = file_with("# Title\n\n", &older, "\n\nKeep this.\n");
    let plan = plan_apply(Some(&text), &block, &fake_digest).expect("plans");
    assert_eq!(plan.action, ApplyAction::Replace);
    assert_eq!(
        plan.result,
        file_with("# Title\n\n", &block, "\n\nKeep this.\n")
    );
    // CRLF file: the replacement is rendered with CRLF.
    let crlf = format!("# Title\r\n\r\n{}\r\n", older.render(LineEnding::CrLf));
    let plan = plan_apply(Some(&crlf), &block, &fake_digest).expect("plans");
    assert_eq!(plan.action, ApplyAction::Replace);
    assert_eq!(
        plan.result,
        format!("# Title\r\n\r\n{}\r\n", block.render(LineEnding::CrLf))
    );
    let current = file_with("# Title\n\n", &block, "\n");
    let plan = plan_apply(Some(&current), &block, &fake_digest).expect("plans");
    assert_eq!(plan.action, ApplyAction::NoOp);
    assert_eq!(plan.result, current);
}

/// R3-005 (slice 5c reliability review): the block at the file's very
/// start (nothing before the begin sentinel) and at its very end (with and
/// without a terminator after the end sentinel), in LF and in CRLF.
/// `locate` finds the same span at either edge and a replace swaps exactly
/// it, leaving whatever text stands on the other side untouched.
#[test]
fn locate_and_replace_handle_a_block_at_the_start_and_at_the_end_of_the_file() {
    let block = guidance_block();
    let older =
        ManagedBlock::guidance(&OutboxPath::new("old/outbox").expect("valid"), &fake_digest);
    for ending in [LineEnding::Lf, LineEnding::CrLf] {
        let nl = ending.as_str();
        let old = older.render(ending);
        let new = block.render(ending);
        let cases = [
            // At the very start, text after it.
            (
                format!("{old}{nl}{nl}# Title{nl}"),
                format!("{new}{nl}{nl}# Title{nl}"),
            ),
            // The whole file, with no terminator of its own.
            (old.clone(), new.clone()),
            // At the very end, with the file's last terminator.
            (
                format!("# Title{nl}{nl}{old}{nl}"),
                format!("# Title{nl}{nl}{new}{nl}"),
            ),
            // At the very end, no terminator after the end sentinel.
            (
                format!("# Title{nl}{nl}{old}"),
                format!("# Title{nl}{nl}{new}"),
            ),
        ];
        for (text, expected) in cases {
            let located = ManagedBlock::locate(ManagedFileKind::Guidance, &text, &fake_digest)
                .expect("intact")
                .expect("present");
            assert_eq!(&text[located.span.clone()], old, "{text:?}");
            assert_eq!(located.body, older.body(), "{text:?}");
            let plan = plan_apply(Some(&text), &block, &fake_digest).expect("plans");
            assert_eq!(plan.action, ApplyAction::Replace, "{text:?}");
            assert_eq!(plan.result, expected, "{text:?}");
        }
    }
}

#[test]
fn plan_apply_refuses_a_modified_or_malformed_block() {
    let block = guidance_block();
    let current = file_with("", &block, "\n");
    let edited = current.replace("Create the directory", "Create the folder");
    assert_eq!(
        plan_apply(Some(&edited), &block, &fake_digest).map(|plan| plan.action),
        Err(ManagedFileError::Modified)
    );
    let broken = current.replace("<!-- omnifrons:end guidance -->", "");
    assert_eq!(
        plan_apply(Some(&broken), &block, &fake_digest).map(|plan| plan.action),
        Err(ManagedFileError::Malformed)
    );
}

// -- the remove plan --

fn locate(text: &str) -> Located {
    ManagedBlock::locate(ManagedFileKind::Guidance, text, &fake_digest)
        .expect("intact")
        .expect("present")
}

#[test]
fn plan_remove_deletes_only_the_block_and_one_adjacent_blank_line() {
    let block = guidance_block();
    let cases = [
        // What an insert added, taken back: the blank line before it goes.
        (file_with("# Title\n\n", &block, "\n"), Some("# Title\n")),
        // In the middle: one blank line before it goes, the rest stays.
        (file_with("a\n\n", &block, "\n\nb\n"), Some("a\n\nb\n")),
        // At the start: the blank line after it goes.
        (file_with("", &block, "\n\n# Title\n"), Some("# Title\n")),
        // No terminator after the block: the blank line before it goes.
        (file_with("# Title\n\n", &block, ""), Some("# Title\n")),
        // No blank line anywhere near: only the block goes.
        (file_with("a\n", &block, "\nb\n"), Some("a\nb\n")),
        // CRLF, taken back the same way.
        (
            format!("# Title\r\n\r\n{}\r\n", block.render(LineEnding::CrLf)),
            Some("# Title\r\n"),
        ),
    ];
    for (text, expected) in cases {
        let located = locate(&text);
        let plan = plan_remove(&text, &located, false);
        assert_eq!(plan.result.as_deref(), expected, "{text:?}");
    }
    // A file Omnifrons created that would be left empty is removed instead;
    // one the user created stays, empty.
    let only = file_with("", &block, "\n");
    let located = locate(&only);
    assert_eq!(plan_remove(&only, &located, true).result, None);
    assert_eq!(
        plan_remove(&only, &located, false).result.as_deref(),
        Some("")
    );
}
