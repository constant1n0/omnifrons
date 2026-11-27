//! Replace secret-shaped values in text before it reaches a portable
//! artifact: a switch checkpoint or a startup brief (TM-001-R6,
//! HTP-001-R18).
//!
//! The rules mirror the VP-S29 observer's redaction
//! (`docs/evidence/VP-001/procedures/vp-s29-observations.mjs`), so the two
//! agree on what a secret looks like:
//! - the rest of a line after `authorization:` (any case);
//! - the token after `bearer` and spaces (any case);
//! - a token that starts with a known key prefix and has at least eight
//!   more characters;
//! - any run of 40 or more token characters that is not exactly a
//!   lowercase SHA-256 digest.
//!
//! This is a heuristic: it cannot prove a text holds no secret (RSP-001
//! D6), and the threat model discloses that gap.

/// What replaces each redacted value.
pub const REDACTED: &str = "<redacted>";

/// Known key prefixes, longest first where one extends another.
const PREFIXES: [&str; 6] = ["sk-ant-", "sk-", "ghp_", "gho_", "xoxb-", "AIza"];
/// How many characters must follow a prefix for the token to count.
const PREFIX_MIN_TAIL: usize = 8;
/// The shortest run of token characters redacted on length alone.
const LONG_RUN_MIN: usize = 40;

/// `text` with every secret-shaped value replaced by [`REDACTED`].
/// Idempotent: scrubbing a scrubbed text changes nothing.
#[must_use]
pub fn scrub_secrets(text: &str) -> String {
    let text = redact_authorization(text);
    let text = redact_bearer(&text);
    let text = redact_runs(&text, is_prefix_class, is_prefixed_token);
    redact_runs(&text, is_long_class, |run| {
        run.len() >= LONG_RUN_MIN && !is_sha256_hex(run)
    })
}

const fn is_prefix_class(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

const fn is_long_class(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'=' | b'_' | b'-')
}

const fn is_bearer_class(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'~' | b'+' | b'/' | b'=' | b'-')
}

fn is_prefixed_token(run: &str) -> bool {
    PREFIXES.iter().any(|prefix| {
        run.strip_prefix(prefix)
            .is_some_and(|tail| tail.len() >= PREFIX_MIN_TAIL)
    })
}

fn is_sha256_hex(run: &str) -> bool {
    run.len() == 64
        && run
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn redact_authorization(text: &str) -> String {
    const LABEL: &str = "authorization:";
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let ending = &line[body.len()..];
        match body.to_ascii_lowercase().find(LABEL) {
            Some(at) => {
                let after = at + LABEL.len();
                let value = &body[after..];
                let spaces = value.len() - value.trim_start_matches([' ', '\t']).len();
                out.push_str(&body[..after + spaces]);
                out.push_str(REDACTED);
            }
            None => out.push_str(body),
        }
        out.push_str(ending);
    }
    out
}

fn redact_bearer(text: &str) -> String {
    const WORD: &str = "bearer";
    let lower = text.to_ascii_lowercase();
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let (mut copied, mut search) = (0, 0);
    while let Some(found) = lower[search..].find(WORD) {
        let mut pos = search + found + WORD.len();
        let spaces_start = pos;
        while pos < bytes.len() && matches!(bytes[pos], b' ' | b'\t') {
            pos += 1;
        }
        let token_start = pos;
        while pos < bytes.len() && is_bearer_class(bytes[pos]) {
            pos += 1;
        }
        if token_start > spaces_start && pos > token_start {
            out.push_str(&text[copied..token_start]);
            out.push_str(REDACTED);
            copied = pos;
        }
        search = pos;
    }
    out.push_str(&text[copied..]);
    out
}

/// Replace every maximal run of `class` bytes for which `secret` holds.
/// The class is ASCII-only, so every cut is a character boundary.
fn redact_runs(text: &str, class: fn(u8) -> bool, secret: impl Fn(&str) -> bool) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let (mut index, mut copied) = (0, 0);
    while index < bytes.len() {
        if !class(bytes[index]) {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && class(bytes[index]) {
            index += 1;
        }
        if secret(&text[start..index]) {
            out.push_str(&text[copied..start]);
            out.push_str(REDACTED);
            copied = index;
        }
    }
    out.push_str(&text[copied..]);
    out
}
