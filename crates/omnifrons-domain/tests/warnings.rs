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
        host_extras: &[],
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
        host_extras: &[],
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
        host_extras: &[],
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
            host_extras: &[],
        },
        ChainAncestor {
            display: "far",
            identity_files: &["AGENTS.md"],
            host_extras: &[],
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
// Extension beyond the prototype: Claude Code's AGENTS.md fallback when
// neither this agent nor any ancestor in its chain holds a CLAUDE.md
// file.
// ---------------------------------------------------------------------

#[test]
fn claude_code_falls_back_and_fires_ancestor_exposure_for_an_agents_only_ancestor() {
    let identity_kinds = [IdentityKind::Agents];
    let chain = [ChainAncestor {
        display: "outside",
        identity_files: &["AGENTS.md"],
        host_extras: &[],
    }];
    let warnings = agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        chain: &chain,
        ..blank_input()
    });

    let exposure = with_code(&warnings, WarningCode::AncestorExposure);
    assert_eq!(exposure.len(), 1, "{warnings:?}");
    assert!(
        exposure[0].hosts.contains(&"Claude Code"),
        "{:?}",
        exposure[0]
    );
}

#[test]
fn claude_code_falls_back_and_lists_as_a_reader_for_an_agents_only_nested_child() {
    let identity_kinds = [IdentityKind::Agents];
    let nested = ["child"];
    let warnings = agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        text: Some("# Root\n\nNo mention of the nested agent here.\n"),
        nested: &nested,
        ..blank_input()
    });

    let matches = with_code(&warnings, WarningCode::ChildNotMentioned);
    assert_eq!(matches.len(), 1, "{warnings:?}");
    assert!(
        matches[0].hosts.contains(&"Claude Code"),
        "{:?}",
        matches[0]
    );
}

/// Adding a `CLAUDE.md` on the agent itself disables the fallback --
/// Claude Code's own primary file is present, so its upward kinds stay
/// `CLAUDE.md`/`.claude/CLAUDE.md` and an `AGENTS.md`-only ancestor
/// never matches them.
#[test]
fn claude_code_fallback_is_disabled_by_a_claude_md_on_the_agent() {
    let identity_kinds = [IdentityKind::Claude];
    let chain = [ChainAncestor {
        display: "outside",
        identity_files: &["AGENTS.md"],
        host_extras: &[],
    }];
    let warnings = agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        chain: &chain,
        ..blank_input()
    });

    assert!(
        !with_code(&warnings, WarningCode::AncestorExposure)
            .iter()
            .any(|w| w.hosts.contains(&"Claude Code")),
        "{warnings:?}"
    );
    assert!(
        !with_code(&warnings, WarningCode::AncestorShadowing)
            .iter()
            .any(|w| w.hosts.contains(&"Claude Code")),
        "{warnings:?}"
    );
}

/// A `CLAUDE.md` anywhere else in the chain -- not necessarily on the
/// `AGENTS.md`-only ancestor being evaluated -- also disables the
/// fallback for the whole chain, so that `AGENTS.md`-only ancestor goes
/// back to naming no Claude Code warning at all.
#[test]
fn claude_code_fallback_is_disabled_by_a_claude_md_elsewhere_in_the_chain() {
    let identity_kinds = [IdentityKind::Agents];
    let chain = [
        ChainAncestor {
            display: "near",
            identity_files: &["AGENTS.md"],
            host_extras: &[],
        },
        ChainAncestor {
            display: "far",
            identity_files: &["CLAUDE.md"],
            host_extras: &[],
        },
    ];
    let warnings = agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        chain: &chain,
        ..blank_input()
    });

    let near_warnings: Vec<_> = warnings
        .iter()
        .filter(|w| w.detail.contains("near"))
        .collect();
    assert!(
        !near_warnings
            .iter()
            .any(|w| w.hosts.contains(&"Claude Code")),
        "{near_warnings:?}"
    );
}

/// `CLAUDE.local.md` among the agent's own host extras counts as one of
/// Claude Code's own files too, disabling the fallback exactly like a
/// real `CLAUDE.md` would -- the unrelated `extra_host_file` warning
/// that extra also produces is excluded from this check, since it
/// legitimately names Claude Code as that file's own reader.
#[test]
fn claude_code_fallback_is_disabled_by_claude_local_md_among_the_extras() {
    let identity_kinds = [IdentityKind::Agents];
    let host_extras = ["CLAUDE.local.md"];
    let chain = [ChainAncestor {
        display: "outside",
        identity_files: &["AGENTS.md"],
        host_extras: &[],
    }];
    let warnings = agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        host_extras: &host_extras,
        chain: &chain,
        ..blank_input()
    });

    assert!(
        !warnings
            .iter()
            .any(|w| w.code != WarningCode::ExtraHostFile && w.hosts.contains(&"Claude Code")),
        "{warnings:?}"
    );
}

/// The same chain-wide decision is shared with `child_warnings`: a
/// `CLAUDE.md` anywhere in the chain disables Claude Code as a reader
/// of an unmentioned nested agent too, not only ancestor warnings.
#[test]
fn claude_code_fallback_is_disabled_for_child_warnings_too_once_a_claude_md_exists_in_the_chain() {
    let identity_kinds = [IdentityKind::Agents];
    let nested = ["child"];
    let chain = [ChainAncestor {
        display: "somewhere",
        identity_files: &["CLAUDE.md"],
        host_extras: &[],
    }];
    let warnings = agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        text: Some("# Root\n\nNo mention of the nested agent here.\n"),
        nested: &nested,
        chain: &chain,
        ..blank_input()
    });

    let matches = with_code(&warnings, WarningCode::ChildNotMentioned);
    assert_eq!(matches.len(), 1, "{warnings:?}");
    assert!(
        !matches[0].hosts.contains(&"Claude Code"),
        "{:?}",
        matches[0]
    );
}

/// Edge case named in review: an agent with no identity kinds of its
/// own at all -- not even its own `AGENTS.md` -- and no `CLAUDE.md`
/// variant anywhere in its chain still qualifies for the fallback
/// (vacuously: it holds none of Claude Code's own files either, so
/// `claude_code_falls_back_to_agents` never returns early on that
/// check), but the `AGENTS.md`-only ancestor it shadows reads as
/// `ancestor_shadowing`, not `ancestor_exposure`: `ancestor_warnings`'
/// own `own` check tests whether *this agent's* `identity_kinds`
/// contains the matched file kind, and an empty `identity_kinds` can
/// never satisfy that, no matter which kind is checked. Contrast
/// `claude_code_falls_back_and_fires_ancestor_exposure_for_an_agents_only_ancestor`
/// above, whose agent does hold its own `AGENTS.md`.
///
/// In practice, a directory with no identity file at all is never
/// itself a scanned agent root (ADR-0005 sub-slice 1e/1f's filesystem
/// adapter only recognizes a root that holds at least one identity
/// file); this case is a contract `agent_warnings` still upholds for
/// whatever [`AgentInput`] it is given, not a state a real scan can
/// actually produce.
#[test]
fn claude_code_fallback_applies_but_reads_as_shadowing_for_an_agent_with_no_identity_kinds_at_all()
{
    let chain = [ChainAncestor {
        display: "outside",
        identity_files: &["AGENTS.md"],
        host_extras: &[],
    }];
    let warnings = agent_warnings(&AgentInput {
        chain: &chain,
        ..blank_input()
    });

    let shadowing = with_code(&warnings, WarningCode::AncestorShadowing);
    assert_eq!(shadowing.len(), 1, "{warnings:?}");
    assert!(
        shadowing[0].hosts.contains(&"Claude Code"),
        "{:?}",
        shadowing[0]
    );
    assert!(
        !with_code(&warnings, WarningCode::AncestorExposure)
            .iter()
            .any(|w| w.hosts.contains(&"Claude Code")),
        "{warnings:?}"
    );
}

// ---------------------------------------------------------------------
// Review findings on the merged 1e-B: the chain-wide fallback check and
// the per-ancestor "has this kind" check both now match a host's kinds
// against the UNION of a link's identity files and host extras, not
// identity files alone.
// ---------------------------------------------------------------------

/// A link's own host extras count toward the chain-wide fallback check
/// too, not only its identity files: an ancestor holding `AGENTS.md` as
/// an identity file and `CLAUDE.local.md` as a host extra holds one of
/// Claude Code's own `HostLoader::files` through that extra alone,
/// disabling the fallback for the whole chain exactly as a real
/// `CLAUDE.md` there would -- so no Claude Code `ancestor_*` warning
/// fires at all.
#[test]
fn claude_code_fallback_is_disabled_by_claude_local_md_among_an_ancestors_host_extras() {
    let identity_kinds = [IdentityKind::Agents];
    let chain = [ChainAncestor {
        display: "outside",
        identity_files: &["AGENTS.md"],
        host_extras: &["CLAUDE.local.md"],
    }];
    let warnings = agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        chain: &chain,
        ..blank_input()
    });

    assert!(
        !with_code(&warnings, WarningCode::AncestorExposure)
            .iter()
            .any(|w| w.hosts.contains(&"Claude Code")),
        "{warnings:?}"
    );
    assert!(
        !with_code(&warnings, WarningCode::AncestorShadowing)
            .iter()
            .any(|w| w.hosts.contains(&"Claude Code")),
        "{warnings:?}"
    );
}

/// The per-ancestor "has this kind" check gets the same union: an
/// ancestor holding `.claude/AGENTS.md` only as a host extra -- never
/// an identity file, since it is not one of `IdentityKind`'s six kinds
/// -- still matches Claude Code's `fallback_files` once the fallback
/// applies, firing `ancestor_exposure` for an `AGENTS.md`-only agent
/// exactly as an identity-file match would. The warning's own detail
/// must also name that extra: with no identity file of its own, this
/// ancestor matched *solely* through `.claude/AGENTS.md`, so a
/// `described` text built from `identity_files` alone would read "the
/// ancestor agent outside ()", silently dropping the one file that
/// caused the warning.
#[test]
fn claude_code_fallback_fires_ancestor_exposure_for_an_ancestors_claude_agents_host_extra() {
    let identity_kinds = [IdentityKind::Agents];
    let chain = [ChainAncestor {
        display: "outside",
        identity_files: &[],
        host_extras: &[".claude/AGENTS.md"],
    }];
    let warnings = agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        chain: &chain,
        ..blank_input()
    });

    let exposure = with_code(&warnings, WarningCode::AncestorExposure);
    let claude = exposure
        .iter()
        .find(|w| w.hosts.contains(&"Claude Code"))
        .unwrap_or_else(|| panic!("no Claude Code ancestor_exposure warning in {warnings:?}"));
    assert!(
        claude.detail.contains(".claude/AGENTS.md"),
        "an ancestor that matched solely through a host extra must have that extra named in \
         the warning detail, got {claude:?}"
    );
}

/// An ancestor holding both an identity file and a host extra that
/// matches the same host's kinds: the warning's `described` text must
/// list the identity file first, then the extra -- mirroring the
/// prototype's own `ancestor_warnings` (~1019-1046), which joins one
/// merged `ancestor_files` mapping in discovery order, never the extra
/// alone and never the extra before the identity file.
#[test]
fn ancestor_description_lists_identity_files_before_a_matching_host_extra() {
    let identity_kinds = [IdentityKind::Agents];
    let chain = [ChainAncestor {
        display: "outside",
        identity_files: &["AGENTS.md"],
        host_extras: &[".claude/AGENTS.md"],
    }];
    let warnings = agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        chain: &chain,
        ..blank_input()
    });

    let exposure = with_code(&warnings, WarningCode::AncestorExposure);
    let claude = exposure
        .iter()
        .find(|w| w.hosts.contains(&"Claude Code"))
        .unwrap_or_else(|| panic!("no Claude Code ancestor_exposure warning in {warnings:?}"));
    assert!(
        claude.detail.contains("(AGENTS.md, .claude/AGENTS.md)"),
        "the identity file must be listed before the matching host extra, got {claude:?}"
    );
}

/// Pairs with the previous tests: the same `.claude/AGENTS.md`-only
/// ancestor, but this agent's own `host_extras` also holds
/// `CLAUDE.local.md`, already enough to disable the fallback before
/// the chain is ever consulted
/// (`claude_code_falls_back_to_agents`'s own early return). No Claude
/// Code `ancestor_*` warning fires, even though the ancestor's own
/// extra would have matched had the fallback stayed enabled; the
/// unrelated `extra_host_file` warning that this agent's own
/// `CLAUDE.local.md` produces is excluded from the check, since it
/// legitimately names Claude Code as that file's own reader.
#[test]
fn claude_code_fallback_is_disabled_by_the_agents_own_extra_even_with_a_matching_ancestor_extra() {
    let identity_kinds = [IdentityKind::Agents];
    let host_extras = ["CLAUDE.local.md"];
    let chain = [ChainAncestor {
        display: "outside",
        identity_files: &[],
        host_extras: &[".claude/AGENTS.md"],
    }];
    let warnings = agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        host_extras: &host_extras,
        chain: &chain,
        ..blank_input()
    });

    assert!(
        !warnings
            .iter()
            .any(|w| w.code != WarningCode::ExtraHostFile && w.hosts.contains(&"Claude Code")),
        "{warnings:?}"
    );
}

/// Finding from review: the per-ancestor "has this kind" check's
/// ancestor side already matches the union of identity files and host
/// extras via `ancestor_has_any_kind`, but its `own` side -- whether
/// this agent also has the matched kind -- checked only
/// `identity_kinds`, never `host_extras`. An agent whose own root holds
/// `.claude/AGENTS.md` only as a host extra, with no Claude identity
/// file of its own, still has Claude Code read that file as its
/// documented fallback; shadowing a plain `AGENTS.md`-only ancestor for
/// Claude Code was therefore wrong -- it should be exposure.
#[test]
fn claude_code_fallback_reads_as_exposure_for_an_agent_whose_own_extra_matches_the_fallback_kind() {
    let identity_kinds = [IdentityKind::Qwen];
    let host_extras = [".claude/AGENTS.md"];
    let chain = [ChainAncestor {
        display: "outside",
        identity_files: &["AGENTS.md"],
        host_extras: &[],
    }];
    let warnings = agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        host_extras: &host_extras,
        chain: &chain,
        ..blank_input()
    });

    let exposure = with_code(&warnings, WarningCode::AncestorExposure);
    assert!(
        exposure.iter().any(|w| w.hosts.contains(&"Claude Code")),
        "{warnings:?}"
    );
    assert!(
        !with_code(&warnings, WarningCode::AncestorShadowing)
            .iter()
            .any(|w| w.hosts.contains(&"Claude Code")),
        "{warnings:?}"
    );
}

/// Pins the rule the previous test does not change: without the
/// agent's own `.claude/AGENTS.md` extra (or an `AGENTS.md` identity
/// file of its own), the same `AGENTS.md`-only ancestor still shadows
/// Claude Code -- this agent holds nothing Claude Code's fallback
/// would read for itself.
#[test]
fn claude_code_fallback_still_shadows_without_the_agents_own_agents_md_or_extra() {
    let identity_kinds = [IdentityKind::Qwen];
    let chain = [ChainAncestor {
        display: "outside",
        identity_files: &["AGENTS.md"],
        host_extras: &[],
    }];
    let warnings = agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        chain: &chain,
        ..blank_input()
    });

    let shadowing = with_code(&warnings, WarningCode::AncestorShadowing);
    assert!(
        shadowing.iter().any(|w| w.hosts.contains(&"Claude Code")),
        "{warnings:?}"
    );
    assert!(
        !with_code(&warnings, WarningCode::AncestorExposure)
            .iter()
            .any(|w| w.hosts.contains(&"Claude Code")),
        "{warnings:?}"
    );
}

/// An ancestor can hold two different host extras that each match a
/// different host's kinds at once: `.claude/AGENTS.md` matches only
/// Claude Code's fallback kind, and `QWEN.md` matches only Qwen Code's
/// upward kind. This agent's own `.claude/AGENTS.md` extra covers
/// Claude Code, so it reads as exposure; this agent holds nothing of
/// its own Qwen Code would read, so Qwen Code reads as shadowing for
/// the same ancestor instead. The two resulting warnings' own
/// `described` text must each name only the extra that matched the
/// host *that warning* names: the exposure warning (`Claude Code`
/// only) must never cite `QWEN.md`, which only ever matched Qwen Code,
/// and the shadowing warning (`Qwen Code` only) must never cite
/// `.claude/AGENTS.md`, which only ever matched Claude Code.
#[test]
fn ancestor_warning_details_attribute_each_extra_only_to_the_host_that_matched_it() {
    let host_extras = [".claude/AGENTS.md"];
    let chain = [ChainAncestor {
        display: "outside",
        identity_files: &[],
        host_extras: &[".claude/AGENTS.md", "QWEN.md"],
    }];
    let warnings = agent_warnings(&AgentInput {
        host_extras: &host_extras,
        chain: &chain,
        ..blank_input()
    });

    let exposure = with_code(&warnings, WarningCode::AncestorExposure);
    let claude = exposure
        .iter()
        .find(|w| w.hosts.contains(&"Claude Code"))
        .unwrap_or_else(|| panic!("no Claude Code ancestor_exposure warning in {warnings:?}"));
    assert!(
        claude.detail.contains(".claude/AGENTS.md"),
        "the exposure warning must still name the extra that matched Claude Code, got {claude:?}"
    );
    assert!(
        !claude.detail.contains("QWEN.md"),
        "the exposure warning names `Claude Code` only, so its own detail must never cite \
         `QWEN.md`, an extra that matched Qwen Code alone, got {claude:?}"
    );

    let shadowing = with_code(&warnings, WarningCode::AncestorShadowing);
    let qwen = shadowing
        .iter()
        .find(|w| w.hosts.contains(&"Qwen Code"))
        .unwrap_or_else(|| panic!("no Qwen Code ancestor_shadowing warning in {warnings:?}"));
    assert!(
        qwen.detail.contains("QWEN.md"),
        "the shadowing warning must still name the extra that matched Qwen Code, got {qwen:?}"
    );
    assert!(
        !qwen.detail.contains(".claude/AGENTS.md"),
        "the shadowing warning names `Qwen Code` only, so its own detail must never cite \
         `.claude/AGENTS.md`, an extra that matched Claude Code alone, got {qwen:?}"
    );
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

/// Pinned: the prototype's `child_warnings` (~1054-1072) never skips the
/// warning when `readers` is empty -- it has no upward-loader kind and
/// no `AGENTS.md`/`CLAUDE.md` to add `OpenCode` either, exactly a
/// Gemini-only agent's own files. It still appends one `child_not_mentioned`
/// warning per unmentioned nested agent, with an empty `hosts` list and
/// `host_list([])` ("") spliced into the detail, producing a double
/// space right before "can load them inside" rather than any host name.
/// The port mirrors this exactly, rather than skipping or rewording it.
#[test]
fn child_not_mentioned_still_fires_with_no_readers_mirroring_the_prototype() {
    let identity_kinds = [IdentityKind::Gemini];
    let nested = ["sub-agent"];
    let warnings = agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        text: Some("# Gemini-only\n\nNo mention of the nested agent here.\n"),
        nested: &nested,
        ..blank_input()
    });

    let matches = with_code(&warnings, WarningCode::ChildNotMentioned);
    assert_eq!(matches.len(), 1, "{warnings:?}");
    assert!(matches[0].hosts.is_empty(), "{:?}", matches[0]);
    assert!(
        matches[0]
            .detail
            .contains("instructions;  can load them inside sub-agent/"),
        "{}",
        matches[0].detail
    );
}

/// Pinned prototype parity, not a port gap: the prototype's own
/// `agent_warnings` only calls `child_warnings` when `text is not
/// None` -- `text: Optional[bytes]` being `None` skips the call
/// entirely, rather than treating `None` as empty text that would warn
/// for every nested agent. The port mirrors this exactly: a root with
/// nested agents but no readable canonical text emits no
/// `child_not_mentioned` warning at all.
#[test]
fn no_canonical_text_skips_child_warnings_mirroring_the_prototype() {
    let identity_kinds = [IdentityKind::Claude];
    let nested = ["child-one", "child-two"];
    let warnings = agent_warnings(&AgentInput {
        identity_kinds: &identity_kinds,
        text: None,
        nested: &nested,
        ..blank_input()
    });
    assert!(
        with_code(&warnings, WarningCode::ChildNotMentioned).is_empty(),
        "{warnings:?}"
    );
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

/// Pinned exact detail text, thousands-grouped like the prototype's own
/// `f"{size:,}"`: oracle via `python3 -c "print(f'{32769:,}')"` ->
/// "32,769", `python3 -c "print(f'{32768:,}')"` -> "32,768", and
/// `python3 -c "print(f'{24000:,}')"` -> "24,000".
#[test]
fn size_warning_details_use_thousands_grouping() {
    let size = 32_769;
    let body = format!("# Big\n{}\n", "x".repeat(size - 7));
    assert_eq!(body.len(), size);
    let warnings = agent_warnings(&AgentInput {
        text: Some(&body),
        ..blank_input()
    });

    let codex = with_code(&warnings, WarningCode::SizeCodex);
    assert_eq!(codex.len(), 1, "{warnings:?}");
    assert_eq!(
        codex[0].detail,
        "the canonical text is 32,769 bytes; Codex stops adding instruction files at 32 KiB (32,768 bytes) by default"
    );

    let antigravity = with_code(&warnings, WarningCode::SizeAntigravity);
    assert_eq!(antigravity.len(), 1, "{warnings:?}");
    assert_eq!(
        antigravity[0].detail,
        "the canonical text is 32,769 bytes; Antigravity truncates a rule file above 24,000 bytes"
    );
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

/// `.claude/AGENTS.md` among `host_extras` is `hosts::EXTRA_READERS`'s
/// own entry for it: one `extra_host_file` warning naming only Claude
/// Code, in the prototype's own detail wording.
#[test]
fn claude_agents_md_host_extra_yields_one_extra_host_file_warning_for_claude_code() {
    let host_extras = [".claude/AGENTS.md"];
    let warnings = agent_warnings(&AgentInput {
        host_extras: &host_extras,
        ..blank_input()
    });

    let extras = with_code(&warnings, WarningCode::ExtraHostFile);
    assert_eq!(extras.len(), 1, "{warnings:?}");
    assert_eq!(extras[0].hosts, vec!["Claude Code"]);
    assert_eq!(
        extras[0].detail,
        ".claude/AGENTS.md is read only by Claude Code; it is reported and never changed"
    );
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

/// Pinned, mirroring a gap the prototype's own two functions share (see
/// `boundary_import_warnings`'s doc comment): `import_warnings` matches
/// against a line with inline-code spans *removed*, while
/// `boundary_import_warnings` matches (for its own exclusion check)
/// against a line with those spans only *masked*, same length, same
/// positions. For `` `code`@notes.md ``, removing the span leaves
/// `@notes.md` at the very start of the remaining text -- a new match
/// that masking alone never creates, since masking leaves that `@`
/// sitting right after a backtick, which the import pattern always
/// refuses. The result: both functions report the very same token. This
/// is not a port bug to fix; it is what the prototype itself does for
/// this exact shape, so the port mirrors it rather than inventing an
/// exclusion the prototype never had.
#[test]
fn an_at_right_after_inline_code_is_reported_by_both_import_functions() {
    let warnings = agent_warnings(&AgentInput {
        text: Some("`code`@notes.md\n"),
        ..blank_input()
    });

    let imports = with_code(&warnings, WarningCode::HostImports);
    assert_eq!(imports.len(), 1, "{warnings:?}");
    assert!(
        imports[0].detail.contains("@notes.md"),
        "{}",
        imports[0].detail
    );
    assert!(imports[0].hosts.contains(&"OpenCode"));
    assert!(!imports[0].hosts.contains(&"Claude Code"));

    let boundary = with_code(&warnings, WarningCode::HostImportsBoundary);
    assert_eq!(boundary.len(), 1, "{warnings:?}");
    assert_eq!(boundary[0].hosts, vec!["Claude Code"]);
    assert!(
        boundary[0].detail.contains("\"`code`@notes.md\""),
        "{}",
        boundary[0].detail
    );
}

/// Pinned prototype parity, not a port gap: the prototype's own
/// `is_fence` also strips a line before testing its prefix, so a
/// backtick or tilde run indented by four or more spaces -- an
/// indented code block in `CommonMark`, never a fence -- still opens
/// or closes a fence in the prototype's own `unfenced`, exactly as it
/// does here. The `@notes.md` import between the indented delimiters
/// below is therefore hidden from both `host_imports` and
/// `host_imports_boundary`; the same text with the indentation removed
/// shows the warning fires normally, confirming the indented fence --
/// not something else -- is what hid it.
#[test]
fn an_indented_backtick_run_still_toggles_fencing_mirroring_the_prototype() {
    let wrapped = "# Doc\n\n    ```\n@notes.md\n    ```\n\nAfter.\n";
    let warnings = agent_warnings(&AgentInput {
        text: Some(wrapped),
        ..blank_input()
    });
    assert!(
        with_code(&warnings, WarningCode::HostImports).is_empty(),
        "{warnings:?}"
    );
    assert!(
        with_code(&warnings, WarningCode::HostImportsBoundary).is_empty(),
        "{warnings:?}"
    );

    let unwrapped = "# Doc\n\n@notes.md\n\nAfter.\n";
    let warnings = agent_warnings(&AgentInput {
        text: Some(unwrapped),
        ..blank_input()
    });
    assert_eq!(
        with_code(&warnings, WarningCode::HostImports).len(),
        1,
        "{warnings:?}"
    );
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

// ---------------------------------------------------------------------
// An unrecognized `host_extras` entry (`agent_warnings` ~1172-1175)
// ---------------------------------------------------------------------

/// The prototype's `EXTRA_READERS[extra]` is a plain dict lookup: an
/// `extra` outside the table raises `KeyError`, crashing the whole scan.
/// The port deliberately does not mirror that crash (see
/// `agent_warnings`'s own `continue` comment) -- a caller is expected to
/// supply only names [`omnifrons_domain::hosts::extra_readers`]
/// resolves, and an unrecognized one is skipped without a warning
/// instead.
#[test]
fn an_unknown_host_extra_is_skipped_without_a_warning() {
    let host_extras = ["totally-unknown-file"];
    let warnings = agent_warnings(&AgentInput {
        host_extras: &host_extras,
        ..blank_input()
    });
    assert!(
        with_code(&warnings, WarningCode::ExtraHostFile).is_empty(),
        "{warnings:?}"
    );
}

// ---------------------------------------------------------------------
// WarningCode tokens and order
// ---------------------------------------------------------------------

/// `as_str`/`parse` round trip for every one of the nine codes, and an
/// unrecognized token parses to `None`.
#[test]
fn every_warning_code_round_trips_through_as_str_and_parse() {
    for code in WarningCode::ALL {
        assert_eq!(WarningCode::parse(code.as_str()), Some(code), "{code:?}");
    }
    assert_eq!(WarningCode::parse("not-a-code"), None);
}

/// Each code's own discriminant equals its position in `ALL` -- the
/// prototype's `WARNING_ORDER.index(code)`, which `agent_warnings` sorts
/// warnings by.
#[test]
fn every_warning_code_discriminant_matches_its_position_in_all() {
    for (index, code) in WarningCode::ALL.into_iter().enumerate() {
        assert_eq!(code as usize, index, "{code:?}");
    }
}

/// The nine tokens, spelled exactly as the prototype's `WARNING_ORDER`
/// tuple.
#[test]
fn warning_code_tokens_match_the_prototypes_warning_order() {
    let expected = [
        "ancestor_exposure",
        "ancestor_shadowing",
        "child_not_mentioned",
        "host_imports",
        "host_imports_boundary",
        "extra_host_file",
        "size_codex",
        "size_antigravity",
        "generated_state_dir",
    ];
    let actual: Vec<&str> = WarningCode::ALL.iter().map(WarningCode::as_str).collect();
    assert_eq!(actual, expected);
}
