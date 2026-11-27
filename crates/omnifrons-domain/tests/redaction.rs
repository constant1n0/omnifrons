//! Secret-shaped values are replaced before text reaches a checkpoint or a
//! startup brief (TM-001-R6). The rules mirror the VP-S29 observer's
//! redaction (docs/evidence/VP-001/procedures/vp-s29-observations.mjs).

use omnifrons_domain::redaction::scrub_secrets;

#[test]
fn each_secret_shape_is_redacted() {
    let long_run = "Ab3".repeat(14);
    for (text, expected) in [
        (
            "Authorization: Basic dXNlcjpwYXNz\nnext",
            "Authorization: <redacted>\nnext",
        ),
        (
            "use bearer abc.DEF_123~x/y= now",
            "use bearer <redacted> now",
        ),
        ("key sk-ant-abcdefgh1234 end", "key <redacted> end"),
        ("token ghp_abcdefgh, more", "token <redacted>, more"),
        ("AIzaSyABCDEFGH", "<redacted>"),
        (&format!("id {long_run} done") as &str, "id <redacted> done"),
    ] {
        assert_eq!(scrub_secrets(text), expected, "{text}");
    }
}

#[test]
fn ordinary_text_digests_and_paths_are_kept() {
    let digest = "ab".repeat(32);
    for text in [
        "Draft release notes for version 1.2.0.\nChanges: added an export button.",
        "skills/release-notes/SKILL.md and docs/handoff-transaction-protocol.md",
        &digest as &str,
        "sk-short",
        "bearer",
    ] {
        assert_eq!(scrub_secrets(text), text, "{text}");
    }
}

#[test]
fn scrubbing_is_idempotent() {
    let text = "Authorization: x\nbearer tok\nsk-abcdefghij";
    let once = scrub_secrets(text);
    assert_eq!(scrub_secrets(&once), once);
}

#[test]
fn a_prefixed_token_after_punctuation_is_redacted() {
    for (text, expected) in [
        ("flag --sk-ant-abcdefgh1234 end", "flag --<redacted> end"),
        ("x_ghp_abcdefgh1", "x_<redacted>"),
        ("name-sk-short", "name-sk-short"),
        ("keys/sk-ant-abcdefgh1234.txt", "keys/<redacted>.txt"),
    ] {
        assert_eq!(scrub_secrets(text), expected, "{text}");
    }
}
