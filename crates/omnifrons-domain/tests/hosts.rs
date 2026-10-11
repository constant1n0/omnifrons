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
