//! Switch check contracts (ADR-0005 slice 4): whether a host reaches the
//! canonical definition from the active project root, per the loader
//! table.

use omnifrons_domain::adapter::AdapterId;
use omnifrons_domain::definition::IdentityKind;
use omnifrons_domain::hosts;
use omnifrons_domain::switch_check::{
    AncestorSite, DefinitionReference, ReferenceError, Shadow, SwitchSite, SwitchVerdict,
    UncertainReason, host_for_adapter, switch_check, switch_check_for_adapter,
};

fn root(files: &[&str]) -> SwitchSite {
    SwitchSite {
        root_files: files.iter().map(|f| (*f).to_string()).collect(),
        ancestors: Vec::new(),
    }
}

fn with_ancestors(files: &[&str], ancestors: &[(&str, &[&str])]) -> SwitchSite {
    SwitchSite {
        root_files: files.iter().map(|f| (*f).to_string()).collect(),
        ancestors: ancestors
            .iter()
            .map(|(path, files)| AncestorSite {
                path: (*path).to_string(),
                files: files.iter().map(|f| (*f).to_string()).collect(),
            })
            .collect(),
    }
}

fn reached_file(verdict: &SwitchVerdict) -> Option<&str> {
    match verdict {
        SwitchVerdict::Reaches { file, .. } => Some(file.as_str()),
        _ => None,
    }
}

#[test]
fn every_host_reaches_a_root_holding_its_first_file() {
    for host in &hosts::HOSTS {
        let first = host.files[0];
        let verdict = switch_check(host.name, &root(&[first]));
        if IdentityKind::from_file_name(first).is_some() {
            assert_eq!(reached_file(&verdict), Some(first), "{}", host.name);
        } else {
            // A host whose first file is its own extra (Hermes Agent's
            // `.hermes.md`) loads that file instead of the definition.
            assert!(
                matches!(&verdict, SwitchVerdict::Unreached { shadowed_by: Some(shadow), .. } if shadow.path == "." && shadow.file == first),
                "{}: {verdict:?}",
                host.name
            );
        }
    }
}

#[test]
fn every_row_declares_at_least_one_primary_file() {
    for row in hosts::HOSTS {
        assert!(
            !row.files.is_empty(),
            "{} declares no primary file",
            row.name
        );
    }
}

#[test]
fn a_root_with_only_agents_md_reaches_exactly_the_hosts_that_list_it() {
    let site = root(&["AGENTS.md"]);
    for row in hosts::HOSTS {
        let verdict = switch_check(row.name, &site);
        if row.files.contains(&"AGENTS.md") || row.fallback_files.contains(&"AGENTS.md") {
            assert_eq!(reached_file(&verdict), Some("AGENTS.md"), "{}", row.name);
        } else {
            assert!(
                matches!(verdict, SwitchVerdict::Unreached { .. }),
                "{}",
                row.name
            );
        }
    }
}

#[test]
fn several_present_files_name_the_first_in_the_row_order() {
    let verdict = switch_check("Codex", &root(&["AGENTS.md", "AGENTS.override.md"]));
    assert_eq!(reached_file(&verdict), Some("AGENTS.override.md"));
}

#[test]
fn claude_code_falls_back_to_agents_md_only_without_a_claude_file() {
    let verdict = switch_check("Claude Code", &root(&["AGENTS.md"]));
    assert_eq!(reached_file(&verdict), Some("AGENTS.md"));
    let verdict = switch_check("Claude Code", &root(&["AGENTS.md", "CLAUDE.md"]));
    assert_eq!(reached_file(&verdict), Some("CLAUDE.md"));
}

#[test]
fn a_claude_file_in_an_ancestor_shadows_the_agents_md_fallback() {
    let site = with_ancestors(&["AGENTS.md"], &[("../parent", &["CLAUDE.md"])]);
    match switch_check("Claude Code", &site) {
        SwitchVerdict::Unreached {
            shadowed_by,
            expected,
            ..
        } => {
            assert_eq!(
                shadowed_by,
                Some(Shadow {
                    path: "../parent".to_string(),
                    file: "CLAUDE.md"
                })
            );
            assert_eq!(
                expected,
                vec!["CLAUDE.md", ".claude/CLAUDE.md", "CLAUDE.local.md"]
            );
        }
        other => panic!("expected Unreached, got {other:?}"),
    }
}

#[test]
fn the_shadow_names_the_exact_file_found_in_the_ancestor() {
    let site = with_ancestors(&["AGENTS.md"], &[("../parent", &[".claude/CLAUDE.md"])]);
    match switch_check("Claude Code", &site) {
        SwitchVerdict::Unreached { shadowed_by, .. } => {
            assert_eq!(shadowed_by.map(|s| s.file), Some(".claude/CLAUDE.md"));
        }
        other => panic!("expected Unreached, got {other:?}"),
    }
    assert_eq!(
        switch_check("Claude Code", &site).to_string(),
        "Claude Code does not reach the definition: AGENTS.md is shadowed by .claude/CLAUDE.md in ../parent"
    );
    let both = with_ancestors(
        &["AGENTS.md"],
        &[("..", &["CLAUDE.local.md", ".claude/CLAUDE.md"])],
    );
    match switch_check("Claude Code", &both) {
        SwitchVerdict::Unreached { shadowed_by, .. } => {
            assert_eq!(shadowed_by.map(|s| s.file), Some(".claude/CLAUDE.md"));
        }
        other => panic!("expected Unreached, got {other:?}"),
    }
}

#[test]
fn a_nearer_ancestor_holding_only_a_fallback_file_does_not_suppress_a_farther_shadow() {
    let site = with_ancestors(
        &["AGENTS.md"],
        &[("..", &["AGENTS.md"]), ("../..", &["CLAUDE.md"])],
    );
    match switch_check("Claude Code", &site) {
        SwitchVerdict::Unreached { shadowed_by, .. } => {
            assert_eq!(
                shadowed_by,
                Some(Shadow {
                    path: "../..".to_string(),
                    file: "CLAUDE.md"
                })
            );
        }
        other => panic!("expected Unreached, got {other:?}"),
    }
}

#[test]
fn the_nearest_shadowing_ancestor_is_named() {
    let site = with_ancestors(
        &["AGENTS.md"],
        &[("..", &["CLAUDE.md"]), ("../..", &["CLAUDE.md"])],
    );
    match switch_check("Claude Code", &site) {
        SwitchVerdict::Unreached { shadowed_by, .. } => {
            assert_eq!(shadowed_by.map(|s| s.path), Some("..".to_string()));
        }
        other => panic!("expected Unreached, got {other:?}"),
    }
}

#[test]
fn an_ancestor_claude_file_does_not_shadow_other_hosts_or_a_missing_agents_md() {
    let site = with_ancestors(&[], &[("..", &["CLAUDE.md"])]);
    match switch_check("Claude Code", &site) {
        SwitchVerdict::Unreached { shadowed_by, .. } => assert_eq!(shadowed_by, None),
        other => panic!("expected Unreached, got {other:?}"),
    }
    let site = with_ancestors(&["AGENTS.md"], &[("..", &["CLAUDE.md"])]);
    assert_eq!(
        reached_file(&switch_check("Codex", &site)),
        Some("AGENTS.md")
    );
}

#[test]
fn an_unknown_host_is_uncertain() {
    assert_eq!(
        switch_check("Nope", &root(&["AGENTS.md"])),
        SwitchVerdict::Uncertain {
            reason: UncertainReason::UnknownHost {
                name: "Nope".to_string()
            }
        }
    );
}

#[test]
fn the_verdict_carries_the_rows_provenance() {
    let row = hosts::host("Gemini CLI").expect("row");
    let verdict = switch_check("Gemini CLI", &root(&["GEMINI.md"]));
    let SwitchVerdict::Reaches { claim, .. } = verdict else {
        panic!("expected Reaches");
    };
    assert_eq!(claim.host, row.name);
    assert_eq!(claim.source, row.source);
    assert_eq!(claim.accessed, row.accessed);
    assert_eq!(claim.status, row.status);
}

#[test]
fn the_adapter_binding_names_only_claude_code() {
    assert_eq!(
        host_for_adapter(&AdapterId::claude_code()),
        Some("Claude Code")
    );
    assert_eq!(host_for_adapter(&AdapterId::stream_json_cli()), None);
    assert_eq!(host_for_adapter(&AdapterId::pty_cli()), None);
    assert!(hosts::host("Claude Code").is_some());
}

#[test]
fn a_bound_adapter_ignores_the_chosen_host() {
    let site = root(&["CLAUDE.md"]);
    let verdict = switch_check_for_adapter(&AdapterId::claude_code(), Some("Codex"), &site);
    assert_eq!(verdict, switch_check("Claude Code", &site));
}

#[test]
fn a_generic_adapter_uses_the_chosen_host_or_is_uncertain() {
    let site = root(&["GEMINI.md"]);
    let verdict = switch_check_for_adapter(&AdapterId::pty_cli(), Some("Gemini CLI"), &site);
    assert_eq!(reached_file(&verdict), Some("GEMINI.md"));
    assert_eq!(
        switch_check_for_adapter(&AdapterId::pty_cli(), None, &site),
        SwitchVerdict::Uncertain {
            reason: UncertainReason::NoHostChosen {
                adapter: AdapterId::pty_cli()
            }
        }
    );
}

#[test]
fn verdict_codes_are_stable() {
    let site = root(&["GEMINI.md"]);
    assert_eq!(switch_check("Gemini CLI", &site).code(), "reaches");
    assert_eq!(switch_check("Codex", &site).code(), "unreached");
    assert_eq!(switch_check("Nope", &site).code(), "uncertain");
}

#[test]
fn verdicts_read_as_one_plain_line() {
    let row = hosts::host("Claude Code").expect("row");
    assert_eq!(
        switch_check("Claude Code", &root(&["CLAUDE.md"])).to_string(),
        format!(
            "Claude Code reaches the definition through CLAUDE.md (row accessed {}, {})",
            row.accessed,
            row.status.to_lowercase()
        )
    );
    assert_eq!(
        switch_check("Gemini CLI", &root(&["AGENTS.md"])).to_string(),
        "Gemini CLI does not reach the definition: it loads GEMINI.md, which is absent"
    );
    let shadowed = with_ancestors(&["AGENTS.md"], &[("../parent", &["CLAUDE.md"])]);
    assert_eq!(
        switch_check("Claude Code", &shadowed).to_string(),
        "Claude Code does not reach the definition: AGENTS.md is shadowed by CLAUDE.md in ../parent"
    );
    assert_eq!(
        switch_check_for_adapter(&AdapterId::pty_cli(), None, &root(&[])).to_string(),
        "No host is chosen for adapter pty-cli: the check is uncertain"
    );
    assert_eq!(
        switch_check("Codex", &root(&[])).to_string(),
        "Codex does not reach the definition: it loads AGENTS.override.md or AGENTS.md, none of which is present"
    );
}

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn reference(path: &str, digest: &str) -> DefinitionReference {
    DefinitionReference::new(path, digest).expect("a valid reference")
}

#[test]
fn the_definition_reference_has_a_length_prefixed_canonical_line_and_a_short_display() {
    let reference = reference("my project/AGENTS.md", DIGEST);
    assert_eq!(reference.path(), "my project/AGENTS.md");
    assert_eq!(reference.digest(), DIGEST);
    assert_eq!(
        reference.canonical(),
        format!("definition path=20:my project/AGENTS.md digest=64:{DIGEST}")
    );
    assert_eq!(reference.to_string(), "my project/AGENTS.md@0123456789ab");
}

#[test]
fn the_canonical_prefix_counts_bytes_not_characters() {
    let reference = reference("caf\u{e9}/AGENTS.md", DIGEST);
    assert_eq!(reference.path().chars().count(), 14);
    assert_eq!(
        reference.canonical(),
        format!("definition path=15:caf\u{e9}/AGENTS.md digest=64:{DIGEST}")
    );
}

#[test]
fn the_display_keeps_the_first_twelve_digest_characters() {
    assert_eq!(
        reference("AGENTS.md", DIGEST).to_string(),
        "AGENTS.md@0123456789ab"
    );
}

#[test]
fn a_reference_refuses_an_empty_path() {
    assert_eq!(
        DefinitionReference::new("", DIGEST),
        Err(ReferenceError::EmptyPath)
    );
}

#[test]
fn a_reference_refuses_a_path_that_would_break_the_canonical_line() {
    for path in ["AGENTS.md\nx", "AGENTS.md\r", "AGE\0NTS.md"] {
        assert_eq!(
            DefinitionReference::new(path, DIGEST),
            Err(ReferenceError::PathNotSingleLine),
            "{path:?}"
        );
    }
}

#[test]
fn a_reference_refuses_a_digest_that_is_not_64_lowercase_hex() {
    let upper = DIGEST.to_uppercase();
    for digest in ["", "abc", &DIGEST[..63], &format!("{DIGEST}0"), &upper] {
        assert_eq!(
            DefinitionReference::new("AGENTS.md", digest),
            Err(ReferenceError::DigestNotSha256Hex),
            "{digest:?}"
        );
    }
    let non_hex = format!("g{}", &DIGEST[1..]);
    assert_eq!(
        DefinitionReference::new("AGENTS.md", &non_hex),
        Err(ReferenceError::DigestNotSha256Hex)
    );
}

#[test]
fn the_shadow_names_the_fallback_actually_selected_at_the_root() {
    let site = with_ancestors(&[".claude/AGENTS.md"], &[("../parent", &["CLAUDE.md"])]);
    let verdict = switch_check("Claude Code", &site);
    match &verdict {
        SwitchVerdict::Unreached { fallback, .. } => {
            assert_eq!(*fallback, Some(".claude/AGENTS.md"));
        }
        other => panic!("expected Unreached, got {other:?}"),
    }
    assert_eq!(
        verdict.to_string(),
        "Claude Code does not reach the definition: .claude/AGENTS.md is shadowed by CLAUDE.md in ../parent"
    );
}

#[test]
fn the_selected_fallback_is_the_first_present_in_the_rows_order() {
    let site = with_ancestors(
        &[".claude/AGENTS.md", "AGENTS.md"],
        &[("../parent", &["CLAUDE.md"])],
    );
    match switch_check("Claude Code", &site) {
        SwitchVerdict::Unreached { fallback, .. } => assert_eq!(fallback, Some("AGENTS.md")),
        other => panic!("expected Unreached, got {other:?}"),
    }
}

#[test]
fn an_unshadowed_unreached_verdict_selects_no_fallback() {
    match switch_check("Gemini CLI", &root(&["AGENTS.md"])) {
        SwitchVerdict::Unreached { fallback, .. } => assert_eq!(fallback, None),
        other => panic!("expected Unreached, got {other:?}"),
    }
}

#[test]
fn an_unknown_host_name_with_a_line_break_is_escaped_on_one_line() {
    let line = switch_check("Gem\ni\r\u{2028}x\u{7f}", &root(&[])).to_string();
    assert_eq!(
        line,
        "No loader row for host Gem\\ni\\r\\u{2028}x\\u{007F}: the check is uncertain"
    );
    assert!(!line.contains(['\n', '\r', '\u{2028}', '\u{2029}']));
}

#[test]
fn a_shadow_path_with_a_carriage_return_is_escaped_on_one_line() {
    let site = with_ancestors(&["AGENTS.md"], &[("../par\rent\n", &["CLAUDE.md"])]);
    let line = switch_check("Claude Code", &site).to_string();
    assert_eq!(
        line,
        "Claude Code does not reach the definition: AGENTS.md is shadowed by CLAUDE.md in ../par\\rent\\n"
    );
    assert!(!line.contains(['\n', '\r']));
}

/// A host-specific file outside the six identity kinds is not the
/// definition: Hermes Agent loads `.hermes.md` ahead of `AGENTS.md`, so a
/// root holding both displaces the canonical file at the root itself.
#[test]
fn a_host_file_outside_the_identity_kinds_displaces_the_definition_at_the_root() {
    let verdict = switch_check("Hermes Agent", &root(&["AGENTS.md", ".hermes.md"]));
    assert_eq!(
        verdict,
        SwitchVerdict::Unreached {
            expected: hosts::host("Hermes Agent").unwrap().files.to_vec(),
            shadowed_by: Some(Shadow {
                path: ".".to_string(),
                file: ".hermes.md",
            }),
            fallback: None,
            claim: match switch_check("Hermes Agent", &root(&["AGENTS.md"])) {
                SwitchVerdict::Reaches { claim, .. } => claim,
                other => panic!("expected a claim, got {other:?}"),
            },
        }
    );
    assert!(
        verdict
            .to_string()
            .contains("AGENTS.md is displaced by .hermes.md at the root"),
        "{verdict}"
    );
}

#[test]
fn hermes_agent_reaches_a_root_holding_only_agents_md() {
    let verdict = switch_check("Hermes Agent", &root(&["AGENTS.md"]));
    assert_eq!(reached_file(&verdict), Some("AGENTS.md"));
}

/// `AGENTS.override.md` is an identity kind (secondary), so a host that
/// lists it first still reaches the definition through it, as before.
#[test]
fn an_identity_kind_listed_first_still_reaches() {
    let verdict = switch_check("Hermes Agent", &root(&["AGENTS.md", "AGENTS.override.md"]));
    assert_eq!(reached_file(&verdict), Some("AGENTS.override.md"));
}
