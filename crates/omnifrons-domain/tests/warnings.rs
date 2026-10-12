//! Pure-domain translations of the `base_omnifrons` prototype's
//! `test_portable_agent.py` warnings tests (ADR-0005 sub-slice 1d, last
//! part): `test_warnings_ancestor_shadowing_and_exposure`,
//! `test_warning_child_not_mentioned`,
//! `test_warning_host_imports_for_override_and_imports`,
//! `test_warning_size_caps`,
//! `test_warning_extra_host_files_and_generated_state`,
//! `test_warning_host_imports_boundary_reports_and_never_changes_text`
//! (its warning assertions only -- the write-path half belongs to
//! ADR-0005 sub-slice 2), and `test_real_atlas_and_echo_have_no_boundary_warning`
//! (translated with generic stand-in prose, never the prototype's own
//! Atlas/Echo fixtures). Every fixture here is pure data -- identity
//! kinds, decoded text, a warning chain, nested paths -- built directly,
//! never through a filesystem.

use omnifrons_domain::definition::IdentityKind;
use omnifrons_domain::warnings::{AgentInput, ChainAncestor, Warning, WarningCode, agent_warnings};

/// An [`AgentInput`] with every field empty, for tests that only need
/// to fill in a few.
fn blank_input<'a>() -> AgentInput<'a> {
    AgentInput {
        identity_kinds: &[],
        text: None,
        override_bytes: None,
        chain: &[],
        nested: &[],
        host_extras: &[],
        generated_state_dirs_present: &[],
    }
}

/// `warnings` filtered to one code, mirroring the prototype's
/// `with_code`.
fn with_code(warnings: &[Warning], code: WarningCode) -> Vec<&Warning> {
    warnings.iter().filter(|w| w.code == code).collect()
}

// ---------------------------------------------------------------------
// test_warnings_ancestor_shadowing_and_exposure
// ---------------------------------------------------------------------

#[test]
fn ancestor_shadows_kinds_the_child_lacks_and_exposes_the_one_it_has() {
    let identity_kinds = [IdentityKind::Claude];
    let chain = [ChainAncestor {
        display: "atlas",
        identity_files: &["CLAUDE.md", "AGENTS.md", "QWEN.md"],
    }];
    let warnings = agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        chain: &chain,
        ..blank_input()
    });

    let shadowing = with_code(&warnings, WarningCode::AncestorShadowing);
    assert_eq!(shadowing.len(), 1, "{warnings:?}");
    assert_eq!(shadowing[0].hosts, vec!["Qwen Code", "OpenCode"]);
    assert!(shadowing[0].detail.contains("atlas"), "{:?}", shadowing[0]);

    let exposure = with_code(&warnings, WarningCode::AncestorExposure);
    assert_eq!(exposure.len(), 1, "{warnings:?}");
    assert_eq!(exposure[0].hosts, vec!["Claude Code"]);
}

#[test]
fn ancestor_exposure_grows_once_the_child_gains_the_missing_kinds() {
    let identity_kinds = [
        IdentityKind::Claude,
        IdentityKind::Qwen,
        IdentityKind::Agents,
    ];
    let chain = [ChainAncestor {
        display: "atlas",
        identity_files: &["CLAUDE.md", "AGENTS.md", "QWEN.md"],
    }];
    let warnings = agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        chain: &chain,
        ..blank_input()
    });

    assert!(
        with_code(&warnings, WarningCode::AncestorShadowing).is_empty(),
        "{warnings:?}"
    );
    let exposure = with_code(&warnings, WarningCode::AncestorExposure);
    assert_eq!(exposure.len(), 1, "{warnings:?}");
    assert_eq!(exposure[0].hosts, vec!["Claude Code", "Qwen Code"]);
}

#[test]
fn a_claude_only_parent_shadows_a_child_with_no_claude_file() {
    let identity_kinds = [IdentityKind::Agents];
    let chain = [ChainAncestor {
        display: "p",
        identity_files: &["CLAUDE.md"],
    }];
    let warnings = agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        chain: &chain,
        ..blank_input()
    });

    let shadowing = with_code(&warnings, WarningCode::AncestorShadowing);
    assert_eq!(shadowing.len(), 1, "{warnings:?}");
    assert_eq!(shadowing[0].hosts, vec!["Claude Code"]);
    assert!(
        with_code(&warnings, WarningCode::AncestorExposure).is_empty(),
        "{warnings:?}"
    );
}

#[test]
fn an_empty_chain_produces_no_ancestor_warnings() {
    let warnings = agent_warnings(&blank_input());
    assert!(with_code(&warnings, WarningCode::AncestorExposure).is_empty());
    assert!(with_code(&warnings, WarningCode::AncestorShadowing).is_empty());
}

/// `OpenCode`'s own upward walk resolves on the first unresolved ancestor
/// `AGENTS.md`, never every ancestor's -- a scoped addition beyond the
/// prototype's own test, since nothing there exercises two ancestors.
/// The child holds `QWEN.md` of its own so that Qwen Code's own
/// upward-loader rule (which also matches `AGENTS.md`) reads as
/// exposure, isolating `OpenCode`'s once-only rule in the assertion.
#[test]
fn opencode_shadowing_fires_once_for_the_nearest_ancestor_only() {
    let identity_kinds = [IdentityKind::Claude, IdentityKind::Qwen];
    let chain = [
        ChainAncestor {
            display: "near",
            identity_files: &["AGENTS.md"],
        },
        ChainAncestor {
            display: "far",
            identity_files: &["AGENTS.md"],
        },
    ];
    let warnings = agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        chain: &chain,
        ..blank_input()
    });

    let shadowing = with_code(&warnings, WarningCode::AncestorShadowing);
    assert_eq!(shadowing.len(), 1, "{warnings:?}");
    assert_eq!(shadowing[0].hosts, vec!["OpenCode"]);
    assert!(shadowing[0].detail.contains("near"));
    assert!(!shadowing[0].detail.contains("far"));
}

// ---------------------------------------------------------------------
// test_warning_child_not_mentioned
// ---------------------------------------------------------------------

#[test]
fn child_not_mentioned_names_the_nested_agent_and_its_readers() {
    let identity_kinds = [IdentityKind::Claude];
    let nested = ["Agente-Comentarios"];
    let warnings = agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        text: Some("# Nara\n\nSome instructions that name no nested agent.\n"),
        nested: &nested,
        ..blank_input()
    });

    let matches = with_code(&warnings, WarningCode::ChildNotMentioned);
    assert_eq!(matches.len(), 1, "{warnings:?}");
    assert!(
        matches[0].detail.contains("Agente-Comentarios"),
        "{:?}",
        matches[0]
    );
    assert_eq!(matches[0].hosts, vec!["Claude Code", "OpenCode"]);
}

#[test]
fn child_mentioned_in_the_text_produces_no_warning() {
    let identity_kinds = [IdentityKind::Claude];
    let nested = ["echo"];
    let warnings = agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        text: Some("# Atlas\n\nSee echo/ for the nested agent.\n"),
        nested: &nested,
        ..blank_input()
    });
    assert!(with_code(&warnings, WarningCode::ChildNotMentioned).is_empty());
}

// ---------------------------------------------------------------------
// test_warning_host_imports_for_override_and_imports
// ---------------------------------------------------------------------

#[test]
fn an_import_only_override_warns_that_codex_may_read_nothing() {
    let warnings = agent_warnings(&AgentInput {
        override_bytes: Some(b"@AGENTS.md\n".as_slice()),
        ..blank_input()
    });
    let imports = with_code(&warnings, WarningCode::HostImports);
    assert_eq!(imports.len(), 1, "{warnings:?}");
    assert!(imports[0].hosts.contains(&"Codex"));
    assert!(imports[0].detail.contains("AGENTS.override.md"));
}

#[test]
fn an_at_path_outside_inline_code_is_an_import_blind_warning() {
    let text = "# Simple\n\nYou are Simple.\n\n@docs/extra.md\n\n\
                The `@AGENTS.md` form is not an import here.\n";
    let warnings = agent_warnings(&AgentInput {
        text: Some(text),
        ..blank_input()
    });
    let imports = with_code(&warnings, WarningCode::HostImports);
    assert_eq!(imports.len(), 1, "{warnings:?}");
    assert!(imports[0].hosts.contains(&"OpenCode"));
    assert!(!imports[0].hosts.contains(&"Claude Code"));
    assert!(
        imports[0].detail.contains("@docs/extra.md"),
        "{}",
        imports[0].detail
    );
}

#[test]
fn plain_text_with_no_at_path_has_no_host_imports_warning() {
    let warnings = agent_warnings(&AgentInput {
        text: Some("# Gemini\n\nPlain content, nothing to import.\n"),
        ..blank_input()
    });
    assert!(with_code(&warnings, WarningCode::HostImports).is_empty());
}

// ---------------------------------------------------------------------
// test_warning_size_caps
// ---------------------------------------------------------------------

#[test]
fn size_warnings_fire_past_each_hosts_own_cap() {
    let cases: [(usize, &[WarningCode]); 4] = [
        (24_000, &[]),
        (24_001, &[WarningCode::SizeAntigravity]),
        (32_768, &[WarningCode::SizeAntigravity]),
        (
            32_769,
            &[WarningCode::SizeCodex, WarningCode::SizeAntigravity],
        ),
    ];
    for (size, expected) in cases {
        let body = format!("# Big\n{}\n", "x".repeat(size - 7));
        assert_eq!(body.len(), size);
        let warnings = agent_warnings(&AgentInput {
            text: Some(&body),
            ..blank_input()
        });
        let codes: Vec<WarningCode> = warnings
            .iter()
            .filter(|w| w.code == WarningCode::SizeCodex || w.code == WarningCode::SizeAntigravity)
            .map(|w| w.code)
            .collect();
        assert_eq!(codes, expected, "size {size}: {warnings:?}");
    }
}

// ---------------------------------------------------------------------
// test_warning_extra_host_files_and_generated_state
// ---------------------------------------------------------------------

#[test]
fn extra_host_files_and_a_generated_state_dir_are_each_reported() {
    let host_extras = [
        "CLAUDE.local.md",
        ".cursorrules",
        ".github/copilot-instructions.md",
    ];
    let generated = [".atl"];
    let warnings = agent_warnings(&AgentInput {
        host_extras: &host_extras,
        generated_state_dirs_present: &generated,
        ..blank_input()
    });

    let extras = with_code(&warnings, WarningCode::ExtraHostFile);
    assert_eq!(extras.len(), 3, "{warnings:?}");
    assert!(
        extras
            .iter()
            .any(|w| w.detail.contains("CLAUDE.local.md") && w.hosts == vec!["Claude Code"])
    );
    assert!(
        extras
            .iter()
            .any(|w| w.detail.contains(".cursorrules") && w.hosts == vec!["Cursor"])
    );
    assert!(extras.iter().any(|w| {
        w.detail.contains(".github/copilot-instructions.md") && w.hosts == vec!["Copilot"]
    }));

    let generated_warnings = with_code(&warnings, WarningCode::GeneratedStateDir);
    assert_eq!(generated_warnings.len(), 1, "{warnings:?}");
    assert!(generated_warnings[0].detail.contains(".atl"));
    assert!(generated_warnings[0].hosts.is_empty());
}

#[test]
fn a_clean_agent_has_no_extra_or_generated_state_warnings() {
    let warnings = agent_warnings(&blank_input());
    assert!(with_code(&warnings, WarningCode::ExtraHostFile).is_empty());
    assert!(with_code(&warnings, WarningCode::GeneratedStateDir).is_empty());
}

// ---------------------------------------------------------------------
// test_warning_host_imports_boundary_reports_and_never_changes_text
// (warning assertions only; the write path belongs to sub-slice 2)
// ---------------------------------------------------------------------

const BOUNDARY_BODY: &str = "# Boundary\n\
\n\
Run `make`@notes.md before you start.\n\
Reply from ops@example.com or jos\u{e9}@ejemplo.es.\n\
Quote `@AGENTS.md` only as inline code.\n\
@docs/extra.md\n\
Ping me @\n\
\n\
```text\n\
Keep `x`@fenced.md here.\n\
```\n";

#[test]
fn boundary_import_reports_the_make_notes_token_and_excludes_everything_else() {
    let warnings = agent_warnings(&AgentInput {
        text: Some(BOUNDARY_BODY),
        ..blank_input()
    });

    let boundary = with_code(&warnings, WarningCode::HostImportsBoundary);
    assert_eq!(boundary.len(), 1, "{warnings:?}");
    let detail = &boundary[0].detail;
    assert_eq!(boundary[0].hosts, vec!["Claude Code"]);
    assert!(detail.contains("\"`make`@notes.md\""), "{detail}");
    assert!(
        detail.contains("Claude Code may load this as an import"),
        "{detail}"
    );
    assert!(detail.contains("other hosts treat it as text"), "{detail}");
    for quiet in [
        "ops@",
        "jos\u{e9}@",
        "@AGENTS.md",
        "@docs/extra.md",
        "fenced",
        "Ping",
    ] {
        assert!(!detail.contains(quiet), "{quiet} leaked into {detail}");
    }

    let imports = with_code(&warnings, WarningCode::HostImports);
    assert_eq!(imports.len(), 1, "{warnings:?}");
    assert!(imports[0].hosts.contains(&"OpenCode"));
    assert!(!imports[0].hosts.contains(&"Claude Code"));
    assert!(
        imports[0].detail.contains("@docs/extra.md"),
        "{}",
        imports[0].detail
    );
}

/// A dedicated, minimal case isolating `masked_inline_code`'s own
/// purpose from [`BOUNDARY_BODY`]'s larger fixture: an `@` strictly
/// *inside* an inline-code span's content is removed by masking before
/// it is ever found, so it is neither a `host_imports` nor a
/// `host_imports_boundary` warning (the same `` `@AGENTS.md` `` case
/// above, alone on its own line).
#[test]
fn an_at_fully_inside_inline_code_is_never_a_boundary_warning() {
    let warnings = agent_warnings(&AgentInput {
        text: Some("Quote `@foo.md` only as inline code.\n"),
        ..blank_input()
    });
    assert!(with_code(&warnings, WarningCode::HostImportsBoundary).is_empty());
    assert!(with_code(&warnings, WarningCode::HostImports).is_empty());
}

// ---------------------------------------------------------------------
// test_real_atlas_and_echo_have_no_boundary_warning, translated with
// generic stand-in prose -- never the prototype's own Atlas/Echo
// fixtures.
// ---------------------------------------------------------------------

#[test]
fn ordinary_agent_prose_has_no_import_or_boundary_warnings() {
    let text = "# Example Agent\n\n\
                This agent answers support questions and writes short \
                summaries. Contact the team at support@example.org for \
                anything out of scope.\n\n\
                ## Style\n\nBe concise and cite sources.\n";
    let warnings = agent_warnings(&AgentInput {
        text: Some(text),
        ..blank_input()
    });
    assert!(with_code(&warnings, WarningCode::HostImports).is_empty());
    assert!(with_code(&warnings, WarningCode::HostImportsBoundary).is_empty());
}

// ---------------------------------------------------------------------
// Addition beyond the seven prototype tests: final `WARNING_ORDER`
// ordering across several simultaneously-triggered codes.
// ---------------------------------------------------------------------

#[test]
fn warnings_come_out_sorted_by_warning_order() {
    let host_extras = ["CLAUDE.local.md"];
    let generated = [".atl"];
    let big_text = format!("# Big\n{}\n", "x".repeat(40_000));
    let warnings = agent_warnings(&AgentInput {
        text: Some(&big_text),
        host_extras: &host_extras,
        generated_state_dirs_present: &generated,
        ..blank_input()
    });
    assert!(warnings.len() >= 4, "{warnings:?}");
    let positions: Vec<usize> = warnings
        .iter()
        .map(|w| WarningCode::ALL.iter().position(|c| *c == w.code).unwrap())
        .collect();
    let mut sorted = positions.clone();
    sorted.sort_unstable();
    assert_eq!(positions, sorted, "{warnings:?}");
}
