//! Host loader table contracts (ADR-0005 sub-slice 1d): the nine hosts
//! the prototype's `HOSTS.md` documented, transcribed as data for
//! warnings.

use omnifrons_domain::hosts;

const HOST_ORDER: [&str; 9] = [
    "Claude Code",
    "Codex",
    "Gemini CLI",
    "Antigravity",
    "Qwen Code",
    "OpenCode",
    "Kimi",
    "Cursor",
    "Copilot",
];

/// The prototype sorts warning hosts by `HOST_ORDER.index`; `HOSTS` and
/// `order_index` must agree with that same order, and an unknown name
/// resolves to `None` on both lookups.
#[test]
fn host_order_matches_the_nine_documented_hosts() {
    assert_eq!(hosts::HOSTS.len(), HOST_ORDER.len());
    for (index, name) in HOST_ORDER.iter().copied().enumerate() {
        assert_eq!(
            hosts::HOSTS[index].name,
            name,
            "HOSTS[{index}] out of order"
        );
        assert_eq!(
            hosts::order_index(name),
            Some(index),
            "order_index({name:?})"
        );
        assert!(hosts::host(name).is_some(), "host({name:?}) must resolve");
    }
    assert_eq!(hosts::host("unknown"), None);
    assert_eq!(hosts::order_index("unknown"), None);
}

/// Derived from `HOSTS` by each host's `upward_kinds`, and must equal the
/// prototype's `UPWARD_LOADERS` tuple verbatim: Claude Code and Qwen Code
/// walk upward and load every ancestor file of these kinds; `OpenCode`
/// also walks upward but is handled apart (first match wins, not modelled
/// here).
#[test]
fn upward_loaders_matches_the_prototypes_tuple() {
    let expected: Vec<(&str, &[&str])> = vec![
        ("Claude Code", &["CLAUDE.md", ".claude/CLAUDE.md"]),
        ("Qwen Code", &["QWEN.md", "AGENTS.md"]),
    ];
    let actual: Vec<(&str, &[&str])> = hosts::upward_loaders().collect();
    assert_eq!(actual, expected);
}

/// Hosts that read `AGENTS.md` without documented `@` expansion, verbatim
/// from the prototype's `IMPORT_BLIND_READERS` tuple and in its order.
#[test]
fn import_blind_readers_matches_the_prototypes_order() {
    let expected = [
        "Codex",
        "Qwen Code",
        "OpenCode",
        "Kimi",
        "Cursor",
        "Copilot",
    ];
    let actual: Vec<&str> = hosts::import_blind_readers().collect();
    assert_eq!(actual, expected);
}

/// `CODEX_MAX_BYTES` (32 KiB) and `ANTIGRAVITY_MAX_BYTES` (24,000 bytes)
/// are the only two documented size caps; every other host is uncapped.
#[test]
fn size_caps_are_set_only_for_codex_and_antigravity() {
    assert_eq!(hosts::CODEX_MAX_BYTES, 32_768);
    assert_eq!(hosts::ANTIGRAVITY_MAX_BYTES, 24_000);
    for host in &hosts::HOSTS {
        let expected = match host.name {
            "Codex" => Some(hosts::CODEX_MAX_BYTES),
            "Antigravity" => Some(hosts::ANTIGRAVITY_MAX_BYTES),
            _ => None,
        };
        assert_eq!(host.size_cap, expected, "{} size_cap", host.name);
    }
}

/// Host-specific files that are reported and never touched, verbatim
/// from the prototype's `EXTRA_READERS` mapping and in its order.
#[test]
fn extra_readers_maps_each_extra_file_to_its_prototype_host_tuple() {
    let expected: &[(&str, &[&str])] = &[
        ("CLAUDE.local.md", &["Claude Code"]),
        (".kimi/AGENTS.md", &["Kimi"]),
        (".agents/AGENTS.md", &["Antigravity"]),
        (".agents/rules", &["Antigravity"]),
        (".claude/rules", &["Claude Code"]),
        (".cursor/rules", &["Cursor"]),
        (".cursorrules", &["Cursor"]),
        (".github/copilot-instructions.md", &["Copilot"]),
    ];
    assert_eq!(hosts::EXTRA_READERS, expected);
}

/// Each host's loaded-file list, transcribed from the table's "Project
/// instruction files" column; one assertion per host, literal values.
#[test]
fn host_files_match_the_documented_table() {
    assert_eq!(
        hosts::host("Claude Code").unwrap().files,
        ["CLAUDE.md", ".claude/CLAUDE.md", "CLAUDE.local.md"]
    );
    assert_eq!(
        hosts::host("Codex").unwrap().files,
        ["AGENTS.override.md", "AGENTS.md"]
    );
    assert_eq!(hosts::host("Gemini CLI").unwrap().files, ["GEMINI.md"]);
    assert_eq!(
        hosts::host("Antigravity").unwrap().files,
        ["AGENTS.md", "GEMINI.md", ".agents/rules/"]
    );
    assert_eq!(
        hosts::host("Qwen Code").unwrap().files,
        ["QWEN.md", "AGENTS.md"]
    );
    assert_eq!(
        hosts::host("OpenCode").unwrap().files,
        ["AGENTS.md", "CLAUDE.md"]
    );
    assert_eq!(
        hosts::host("Kimi").unwrap().files,
        ["AGENTS.md", ".kimi/AGENTS.md"]
    );
    assert_eq!(hosts::host("Cursor").unwrap().files, ["AGENTS.md"]);
    assert_eq!(hosts::host("Copilot").unwrap().files, ["AGENTS.md"]);
}

/// Each host's `fallback_files`: the files a host reads only when none
/// of `files` exists in the working directory or above it. Claude Code's
/// Notes document this fallback to `AGENTS.md` (v2.1.277 or later), and
/// the official Claude Code memory documentation adds `.claude/AGENTS.md`
/// to the same fallback (read at session start from the working
/// directory and above), which the table omits; every other host's Notes
/// document no such conditional fallback, so each is empty.
#[test]
fn fallback_files_are_set_only_for_claude_code() {
    for host in &hosts::HOSTS {
        let expected: &[&str] = if host.name == "Claude Code" {
            &["AGENTS.md", ".claude/AGENTS.md"]
        } else {
            &[]
        };
        assert_eq!(
            host.fallback_files, expected,
            "{} fallback_files",
            host.name
        );
    }
}

/// `reads` is the loose "can this host ever read it" question a switch
/// check asks: true when `file` is in `files` or in `fallback_files`,
/// regardless of which list it came from.
#[test]
fn reads_reports_files_and_fallback_files() {
    let claude_code = hosts::host("Claude Code").unwrap();
    assert!(claude_code.reads("AGENTS.md"));
    assert!(claude_code.reads(".claude/AGENTS.md"));
    assert!(!claude_code.reads("GEMINI.md"));

    let codex = hosts::host("Codex").unwrap();
    assert!(codex.reads("AGENTS.md"));
}

/// Walk-pruning, host-config, and generated-state directories, verbatim
/// from the prototype's `SKIP_DIRS`, `HOST_CONFIG_DIRS`, and
/// `GENERATED_STATE_DIRS`.
#[test]
fn skip_dirs_host_config_dirs_and_generated_state_dirs_match_the_prototype() {
    assert_eq!(
        hosts::SKIP_DIRS,
        [
            ".git",
            "node_modules",
            ".venv",
            "__pycache__",
            ".portable-agent"
        ]
    );
    assert_eq!(hosts::HOST_CONFIG_DIRS, [".claude", ".kimi", ".agents"]);
    assert_eq!(hosts::GENERATED_STATE_DIRS, [".atl"]);
}

/// Each host's documented walk shape, `@`-import support, source URL,
/// access date, and status text, transcribed from the table's
/// Notes/Status/Sources columns (not copied from `hosts.rs`); one
/// assertion group per host, literal values. The source URL is the
/// first one listed under each row's footnote when a footnote lists
/// more than one.
#[test]
fn host_loader_documentary_fields_match_the_documented_table() {
    let claude_code = hosts::host("Claude Code").unwrap();
    assert_eq!(claude_code.walk, hosts::Walk::AncestorsAtLaunch);
    assert_eq!(claude_code.imports, hosts::ImportSupport::AtPath);
    assert_eq!(claude_code.source, "https://code.claude.com/docs/en/memory");
    assert_eq!(claude_code.accessed, "2026-09-25");
    assert_eq!(claude_code.status, "Verified");

    let codex = hosts::host("Codex").unwrap();
    assert_eq!(codex.walk, hosts::Walk::GitRootToCwd);
    assert_eq!(codex.imports, hosts::ImportSupport::None);
    assert_eq!(
        codex.source,
        "https://learn.chatgpt.com/docs/agent-configuration/agents-md"
    );
    assert_eq!(codex.accessed, "2026-09-25");
    assert_eq!(codex.status, "Verified");

    let gemini_cli = hosts::host("Gemini CLI").unwrap();
    assert_eq!(gemini_cli.walk, hosts::Walk::Undocumented);
    assert_eq!(gemini_cli.imports, hosts::ImportSupport::AtPath);
    assert_eq!(
        gemini_cli.source,
        "https://geminicli.com/docs/cli/gemini-md/"
    );
    assert_eq!(gemini_cli.accessed, "2026-09-25");
    assert_eq!(
        gemini_cli.status,
        "Verified (docs page); configuration reference unverified"
    );

    let antigravity = hosts::host("Antigravity").unwrap();
    assert_eq!(antigravity.walk, hosts::Walk::Undocumented);
    assert_eq!(antigravity.imports, hosts::ImportSupport::Includes);
    assert_eq!(antigravity.source, "https://antigravity.google/docs/rules/");
    assert_eq!(antigravity.accessed, "2026-09-25");
    assert_eq!(antigravity.status, "Verified");

    let qwen_code = hosts::host("Qwen Code").unwrap();
    assert_eq!(qwen_code.walk, hosts::Walk::Upward);
    assert_eq!(qwen_code.imports, hosts::ImportSupport::None);
    assert_eq!(
        qwen_code.source,
        "https://github.com/QwenLM/qwen-code/blob/main/packages/core/src/utils/memory-constants.ts"
    );
    assert_eq!(qwen_code.accessed, "2026-09-25");
    assert_eq!(qwen_code.status, "Verified (source); user docs unverified");

    let opencode = hosts::host("OpenCode").unwrap();
    assert_eq!(opencode.walk, hosts::Walk::Upward);
    assert_eq!(opencode.imports, hosts::ImportSupport::None);
    assert_eq!(opencode.source, "https://opencode.ai/docs/rules/");
    assert_eq!(opencode.accessed, "2026-09-25");
    assert_eq!(opencode.status, "Verified (docs); source file unverified");

    let kimi = hosts::host("Kimi").unwrap();
    assert_eq!(kimi.walk, hosts::Walk::GitRootToCwd);
    assert_eq!(kimi.imports, hosts::ImportSupport::None);
    assert_eq!(
        kimi.source,
        "https://github.com/MoonshotAI/kimi-cli/blob/main/src/kimi_cli/soul/agent.py"
    );
    assert_eq!(kimi.accessed, "2026-09-25");
    assert_eq!(
        kimi.status,
        "Verified (kimi-cli source); kimi-code docs unverified"
    );

    let cursor = hosts::host("Cursor").unwrap();
    assert_eq!(cursor.walk, hosts::Walk::NestedOnly);
    assert_eq!(cursor.imports, hosts::ImportSupport::None);
    assert_eq!(cursor.source, "https://cursor.com/docs/context/rules");
    assert_eq!(cursor.accessed, "2026-09-25");
    assert_eq!(cursor.status, "Verified; `.cursorrules` unverified");

    let copilot = hosts::host("Copilot").unwrap();
    assert_eq!(copilot.walk, hosts::Walk::NearestInTree);
    assert_eq!(copilot.imports, hosts::ImportSupport::None);
    assert_eq!(
        copilot.source,
        "https://docs.github.com/en/copilot/how-tos/configure-custom-instructions/add-repository-instructions"
    );
    assert_eq!(copilot.accessed, "2026-09-25");
    assert_eq!(copilot.status, "Verified");
}

/// Cross-field invariant: a host blind to `@` imports
/// (`import_blind == true`) must document no import support, and a
/// host documenting import support (`imports != None`) must not be
/// import-blind. Both directions are checked explicitly even though
/// each is the other's contrapositive, so either clause failing points
/// straight at the host that disagrees.
#[test]
fn import_blind_agrees_with_documented_import_support_for_every_host() {
    for host in &hosts::HOSTS {
        if host.import_blind {
            assert_eq!(
                host.imports,
                hosts::ImportSupport::None,
                "{}: import_blind is true but imports is {:?}",
                host.name,
                host.imports
            );
        }
        if host.imports != hosts::ImportSupport::None {
            assert!(
                !host.import_blind,
                "{}: imports is {:?} but import_blind is true",
                host.name, host.imports
            );
        }
    }
}
