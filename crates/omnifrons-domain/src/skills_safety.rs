//! The conservative `@`-import safety rule (ADR-0005 "Agent identity and
//! portable definition" decision 2, sub-slice 1c): whether a host that
//! loads `@path` text references as file imports -- Claude Code's own
//! behaviour, documented in `adapter/portable-agent/references/HOSTS.md`
//! "Known limitation: `@` handling" -- could load a given `@` in
//! canonical text or in a generated skill-index row. Pure text, like
//! [`crate::skills_index`]: nothing here reads a file, walks a
//! directory, or renders a block; this module only judges bytes and
//! strings the caller already has.
//!
//! Translated from the `base_omnifrons` prototype's
//! `adapter/portable-agent/scripts/portable_agent.py`: `AT_WORD_CHARS`,
//! `JS_SPACE`, `AT_SHOWN_BEFORE`/`AT_SHOWN_AFTER`, `unsafe_at_indexes`,
//! `inside_word`, `at_token`, and `unsafe_at_tokens`.
//!
//! The prototype's rule runs over a Python `str`, whose codepoints may
//! include an unpaired UTF-16 surrogate half: Python decodes a byte that
//! is not valid UTF-8 with the `surrogateescape` codec (one surrogate per
//! byte) so that no input ever crashes it, and shows that surrogate back
//! as `\xNN` in a reported token. A Rust `&str` is always valid UTF-8 and
//! can never hold a surrogate at all, so this module takes already-
//! decoded text; a caller holding raw bytes ([`read_skill_metadata`]
//! below) decodes them with [`String::from_utf8_lossy`] first, which
//! marks an invalid byte with `\u{FFFD}` instead. `\u{FFFD}` is neither a
//! letter, a digit, nor one of [`AT_WORD_CHARS`], so an `@` right after
//! it is judged unsafe exactly as the prototype's own surrogate would be
//! -- only the text shown for it differs.

use std::collections::HashMap;

use crate::skills_index::{SkillRow, link_target, table_cell};

/// Characters, other than a letter or digit, that keep an `@` inside a
/// word when nothing escapes them (the prototype's `AT_WORD_CHARS`):
/// `ops@example.com`, `a.b+c@x.io`, `d-@x.io`.
const AT_WORD_CHARS: &str = ".%+-";

/// How much of the word before a reported `@` is shown, in characters
/// (the prototype's `AT_SHOWN_BEFORE`).
const AT_SHOWN_BEFORE: usize = 30;

/// How much of the word after a reported `@` is shown, in characters
/// (the prototype's `AT_SHOWN_AFTER`).
const AT_SHOWN_AFTER: usize = 40;

/// JavaScript's `\s`, which Claude Code's own `@`-import pattern uses
/// (the prototype's `JS_SPACE`): wider than Rust's or Python's notion of
/// whitespace in the control-character range, and including `U+FEFF`
/// (BOM), which neither treats as whitespace by default.
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\u{000B}'
            | '\u{000C}'
            | '\r'
            | ' '
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    ) || ('\u{2000}'..='\u{200A}').contains(&c)
}

/// A conservative, bounded stand-in for Unicode general categories `Mn`
/// (nonspacing mark) and `Me` (enclosing mark), covering the four
/// combining-mark blocks most likely in skill metadata written in a
/// Latin, Greek, or Cyrillic script: Combining Diacritical Marks
/// (`U+0300`-`U+036F`, entirely `Mn`), Combining Diacritical Marks
/// Extended (`U+1AB0`-`U+1AFF`), Combining Diacritical Marks Supplement
/// (`U+1DC0`-`U+1DFF`), Combining Diacritical Marks for Symbols
/// (`U+20D0`-`U+20FF`), and Combining Half Marks (`U+FE20`-`U+FE2F`).
///
/// This crate has no Unicode-database dependency (`omnifrons-domain`
/// depends on `std` and `thiserror` only, per `docs/repository-layout.md`
/// § Crate map), so it cannot reproduce the prototype's
/// `unicodedata.category(...).startswith("M")` check over the whole
/// Unicode `M` category. A mark outside these blocks is treated as an
/// ordinary character instead of being skipped past, which makes
/// [`inside_word`] examine it directly; since it is then neither
/// alphanumeric nor in [`AT_WORD_CHARS`], the `@` before it is judged
/// unsafe where the prototype's complete table would allow it -- the
/// conservative direction the rule already takes by design, never the
/// permissive one.
fn is_combining_mark(c: char) -> bool {
    matches!(
        c,
        '\u{0300}'..='\u{036F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE20}'..='\u{FE2F}'
    )
}

/// True when the `@` at `index` follows a character that keeps it inside
/// a word (the prototype's `inside_word`): a letter or digit, or one of
/// [`AT_WORD_CHARS`] after an even run of backslashes -- an odd run
/// escapes it, as `CommonMark` lexes a backslash escape as its own
/// token. A letter or digit can never be escaped. Combining marks
/// between the `@` and that character are skipped over, so an NFD
/// `jose\u{301}@` is safe (VER-H-004).
fn inside_word(chars: &[char], index: usize) -> bool {
    let mut before: isize = index.cast_signed() - 1;
    while before >= 0 && is_combining_mark(chars[before.cast_unsigned()]) {
        before -= 1;
    }
    if before < 0 {
        return false;
    }
    let before = before.cast_unsigned();
    let ch = chars[before];
    if ch.is_alphanumeric() {
        return true;
    }
    if !AT_WORD_CHARS.contains(ch) {
        return false;
    }
    let mut slashes: usize = 0;
    while before > slashes && chars[before - slashes - 1] == '\\' {
        slashes += 1;
    }
    slashes.is_multiple_of(2)
}

/// Indexes of every `@` in `chars` a host may load as an import (the
/// prototype's `unsafe_at_indexes`). `text_follows` says that more text
/// follows where this text is written (a template, a table cell, or a
/// link label), so an `@` at its very end counts as followed by
/// non-whitespace; pass `false` for a whole line of a file, whose end is
/// a line break.
fn unsafe_at_indexes(chars: &[char], text_follows: bool) -> Vec<usize> {
    let mut found = Vec::new();
    for (index, &c) in chars.iter().enumerate() {
        if c != '@' {
            continue;
        }
        if let Some(&next) = chars.get(index + 1) {
            if is_js_space(next) {
                continue; // "3 @ 5": no path follows
            }
        } else if !text_follows {
            continue; // the end of a line: no path follows
        }
        if !inside_word(chars, index) {
            found.push(index);
        }
    }
    found
}

/// The word around the `@` at `index`, shortened with `...` on a side
/// that was cut (the prototype's `at_token`, [`AT_SHOWN_BEFORE`] before
/// and [`AT_SHOWN_AFTER`] after).
fn at_token(chars: &[char], index: usize) -> String {
    let mut start = index;
    while start > 0 && !is_js_space(chars[start - 1]) && index - start < AT_SHOWN_BEFORE {
        start -= 1;
    }
    let mut end = index + 1;
    while end < chars.len() && !is_js_space(chars[end]) && end - index < AT_SHOWN_AFTER {
        end += 1;
    }
    let head = start > 0 && !is_js_space(chars[start - 1]);
    let tail = end < chars.len() && !is_js_space(chars[end]);
    let body: String = chars[start..end].iter().collect();
    format!(
        "{}{body}{}",
        if head { "..." } else { "" },
        if tail { "..." } else { "" }
    )
}

/// The word around every `@` in `text` a host may load as an import (the
/// prototype's `unsafe_at_tokens`); see the module documentation for how
/// a byte that is not valid UTF-8 is judged once it reaches this
/// function as `\u{FFFD}`.
#[must_use]
pub fn unsafe_at_tokens(text: &str, text_follows: bool) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    unsafe_at_indexes(&chars, text_follows)
        .into_iter()
        .map(|index| at_token(&chars, index))
        .collect()
}

/// One skill's row data read from a `SKILL.md`'s front matter (the
/// prototype's `read_skill`'s row half: `(name, description, path)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillMetadata {
    /// The skill's name: its front matter `name:`, or its folder name,
    /// or the whole path when there is no folder (the prototype's
    /// `fields.get("name") or basename(dirname(relative)) or relative`).
    pub name: String,
    /// The skill's description: its front matter `description:`,
    /// verbatim, or empty when absent.
    pub description: String,
    /// The project-relative POSIX path of the skill's `SKILL.md`, as
    /// given.
    pub path: String,
}

/// One indexed field (`name` or `description`) whose front matter `\u`
/// escape decoded to a lone surrogate (the prototype's
/// `invalid_unicode_fields`): UTF-8 has no form for it, so the field
/// cannot be written into the skill index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidUnicodeField {
    /// `"name"` or `"description"`.
    pub field: &'static str,
    /// The lone surrogate code units the field's `\u` escapes decoded
    /// to, in order.
    pub surrogates: Vec<u16>,
}

/// Why [`read_skill_metadata`] refused a `SKILL.md` (the prototype's
/// `invalid_unicode_fields`, raised as part of `read_skill`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SkillMetadataError {
    /// `name:` or `description:` holds a JSON `\u` escape that decodes
    /// to a lone surrogate: refused before any write, since UTF-8 has
    /// no form for it. Never raised for a raw byte that is not valid
    /// UTF-8 on its own -- that is carried lossily instead (see the
    /// module documentation).
    #[error("the skill metadata holds invalid Unicode")]
    InvalidUnicode(Vec<InvalidUnicodeField>),
}

/// `posixpath.basename(posixpath.dirname(relative)) or relative`: the
/// skill's enclosing folder name, or the whole path when there is none
/// (a `SKILL.md` with no parent component).
fn folder_name(path: &str) -> String {
    let dir = path.rfind('/').map_or("", |index| &path[..index]);
    let base = dir.rfind('/').map_or(dir, |index| &dir[index + 1..]);
    if base.is_empty() {
        path.to_string()
    } else {
        base.to_string()
    }
}

/// True when `text` holds the UTF-8 replacement character: the marker
/// [`String::from_utf8_lossy`] leaves where a raw byte was not valid
/// UTF-8 (this module's stand-in for the prototype's
/// `has_undecodable_bytes`, which tests for its own surrogate-escape
/// marker instead; see the module documentation).
#[must_use]
pub fn has_undecodable_bytes(text: &str) -> bool {
    text.contains('\u{FFFD}')
}

/// A `>` or `|` front matter block-scalar header, with an optional
/// chomping indicator (the prototype's `BLOCK_SCALARS`).
fn is_block_scalar_header(head: &str) -> bool {
    matches!(head, "|" | ">" | "|-" | ">-" | "|+" | ">+")
}

/// Matches the prototype's `FRONTMATTER_KEY` regex
/// (`([A-Za-z0-9_-]+):(?:[ \t]+(.*))?`) as a full match of `line`: one or
/// more ASCII letters, digits, `_` or `-`, then `:`, then either nothing
/// or one or more spaces/tabs and the rest of the line as the value. A
/// line like `key:value`, with no space after the colon, does not match
/// at all (the regex's optional group requires `[ \t]+` to start, and a
/// `fullmatch` with leftover text fails).
fn parse_frontmatter_key(line: &str) -> Option<(&str, &str)> {
    let colon = line.find(':')?;
    let key = &line[..colon];
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    let rest = &line[colon + 1..];
    if rest.is_empty() {
        return Some((key, ""));
    }
    if rest.starts_with([' ', '\t']) {
        Some((key, rest.trim_start_matches([' ', '\t'])))
    } else {
        None
    }
}

/// The raw (unfolded) value lines of each top-level front matter key in
/// a leading `---` block (the prototype's `frontmatter_parts`): a line
/// matching `key: value` that is not itself indented starts a new key;
/// any other non-blank line continues the previous key's value across
/// lines, regardless of its own indentation -- including a non-indented
/// line that simply does not match the `key: value` shape at all (for
/// example, no space after its colon). The first of repeated keys wins.
/// Without a closing `---` or `...` line, there is no front matter at
/// all -- a truncated block is not read partially. Line splitting is
/// `\n`/`\r\n` only: YAML front matter is not expected to carry one of
/// Unicode's other line or paragraph separators.
fn frontmatter_parts(text: &str) -> HashMap<String, Vec<String>> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut lines = text.lines();
    match lines.next() {
        Some(first) if first.trim() == "---" => {}
        _ => return HashMap::new(),
    }
    let mut values: HashMap<String, Vec<String>> = HashMap::new();
    let mut current: Option<(String, Vec<String>)> = None;
    for line in lines {
        let stripped = line.trim();
        if stripped == "---" || stripped == "..." {
            if let Some((key, parts)) = current.take() {
                values.entry(key).or_insert(parts);
            }
            return values;
        }
        let starts_indented = line.chars().next().is_some_and(char::is_whitespace);
        let rstripped = line.trim_end();
        if let Some((key, value)) = parse_frontmatter_key(rstripped)
            && !starts_indented
        {
            if let Some((prev_key, parts)) = current.take() {
                values.entry(prev_key).or_insert(parts);
            }
            current = Some((key.to_string(), vec![value.to_string()]));
            continue;
        }
        if let Some((_, parts)) = current.as_mut()
            && !stripped.is_empty()
        {
            parts.push(stripped.to_string());
        }
    }
    HashMap::new()
}

/// The `u32` value of the four hexadecimal digits at
/// `chars[start..start+4]`, or `None` when they are not all hex digits
/// or run past the end. Checks each character with
/// [`char::is_ascii_hexdigit`] first, rather than delegating straight
/// to [`u32::from_str_radix`]: that parser accepts an optional leading
/// `+` (`"+12f"` parses as `0x12f`), which `CPython`'s strict `json`
/// scanner does not allow inside a `\u` escape -- the `+` is simply
/// not one of the four required hex digits, so the whole escape is
/// malformed (review follow-up, correction pass).
fn hex4(chars: &[char], start: usize) -> Option<u32> {
    if start + 4 > chars.len() {
        return None;
    }
    let digits = &chars[start..start + 4];
    if !digits.iter().all(char::is_ascii_hexdigit) {
        return None;
    }
    let text: String = digits.iter().collect();
    u32::from_str_radix(&text, 16).ok()
}

/// The outcome of a successful [`decode_json_string`] parse: either a
/// plain decoded string, or every lone surrogate a `\u` escape produced
/// (see [`decode_json_string`]'s own documentation).
enum DecodedScalar {
    /// A valid JSON string literal, decoded with no lone surrogate.
    Valid(String),
    /// A valid JSON string literal whose `\u` escapes decoded to one or
    /// more lone surrogates, in order (the prototype's
    /// `escaped_surrogates`): UTF-8 has no form for them.
    LoneSurrogates(Vec<u16>),
}

/// Decodes a double-quoted front matter scalar's escapes -- the JSON
/// string grammar the prototype's `json.loads` applies through
/// `fold_scalar` -- or `None` when `CPython`'s strict `json.loads` would
/// raise on this exact text. `json.loads` raises on: an escape it does
/// not define (anything after `\` other than `" \ / b f n r t u`); a
/// `\u` not followed by exactly four hexadecimal digits; a trailing
/// lone backslash right before the closing quote; a literal, unescaped
/// control character (`U+0000`-`U+001F`) in the text; or an unescaped
/// `"`, which ends the JSON string literal early and always leaves
/// trailing, unparsed content, since `inner` never includes the
/// value's own final quote. On `None`, [`fold_scalar`] falls back to
/// the raw, unescaped `inner` text unchanged -- the prototype's own
/// `except ValueError: return value[1:-1]`, which returns the *whole*
/// value with only its outer quotes stripped, never a partially
/// decoded string: an earlier, otherwise-valid escape in the same text
/// is not decoded either once a later one fails.
///
/// A complete decode may still produce every lone surrogate a `\u`
/// escape produced, when any did. A high surrogate (`U+D800`-
/// `U+DBFF`) immediately followed by another `\u` escape of a low
/// surrogate (`U+DC00`-`U+DFFF`) combines into the one character the
/// pair encodes; every other surrogate escape -- a high one with
/// nothing or something else after it, a low one on its own, or a
/// low-then-high pair, which does not combine -- is lone. Unlike the
/// prototype, a raw byte that is not valid UTF-8 can never surface here
/// as a surrogate: it was already replaced with `U+FFFD` before this
/// text was decoded, so the two can never be confused.
fn decode_json_string(inner: &str) -> Option<DecodedScalar> {
    let chars: Vec<char> = inner.chars().collect();
    let mut out = String::new();
    let mut lone: Vec<u16> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            return None; // an unescaped quote ends the literal early (trailing data)
        }
        if ('\u{0000}'..='\u{001F}').contains(&c) {
            return None; // a literal, unescaped control character
        }
        if c != '\\' {
            out.push(c);
            i += 1;
            continue;
        }
        let Some(&next) = chars.get(i + 1) else {
            return None; // a trailing lone backslash before the closing quote
        };
        match next {
            '"' => {
                out.push('"');
                i += 2;
            }
            '\\' => {
                out.push('\\');
                i += 2;
            }
            '/' => {
                out.push('/');
                i += 2;
            }
            'b' => {
                out.push('\u{0008}');
                i += 2;
            }
            'f' => {
                out.push('\u{000C}');
                i += 2;
            }
            'n' => {
                out.push('\n');
                i += 2;
            }
            'r' => {
                out.push('\r');
                i += 2;
            }
            't' => {
                out.push('\t');
                i += 2;
            }
            'u' => {
                let Some(value) = hex4(&chars, i + 2) else {
                    return None; // not exactly four hexadecimal digits
                };
                if (0xD800..=0xDBFF).contains(&value) {
                    let paired_low = (chars.get(i + 6) == Some(&'\\')
                        && chars.get(i + 7) == Some(&'u'))
                    .then(|| hex4(&chars, i + 8))
                    .flatten()
                    .filter(|low| (0xDC00..=0xDFFF).contains(low));
                    if let Some(low) = paired_low {
                        let combined = 0x10000 + ((value - 0xD800) << 10) + (low - 0xDC00);
                        if let Some(c) = char::from_u32(combined) {
                            out.push(c);
                        }
                        i += 12;
                        continue;
                    }
                    lone.push(u16::try_from(value).expect("a surrogate code unit fits in u16"));
                    i += 6;
                } else if (0xDC00..=0xDFFF).contains(&value) {
                    lone.push(u16::try_from(value).expect("a surrogate code unit fits in u16"));
                    i += 6;
                } else if let Some(c) = char::from_u32(value) {
                    out.push(c);
                    i += 6;
                } else {
                    i += 6;
                }
            }
            _ => return None, // an escape the JSON string grammar does not define
        }
    }
    Some(if lone.is_empty() {
        DecodedScalar::Valid(out)
    } else {
        DecodedScalar::LoneSurrogates(lone)
    })
}

/// One front matter value: quoted, plain, or a `>`/`|` block (folded)
/// (the prototype's `fold_scalar`). Returns every lone surrogate a
/// double-quoted value's `\u` escapes produced, when any did
/// ([`decode_json_string`]). When the double-quoted text is not a
/// valid JSON string literal at all, returns the raw inner text
/// unchanged, exactly as the prototype's own
/// `except ValueError: return value[1:-1]` fallback does.
fn fold_scalar(parts: &[String]) -> Result<String, Vec<u16>> {
    let head = parts[0].trim();
    if is_block_scalar_header(head) {
        return Ok(parts[1..].join(" "));
    }
    let mut joined = vec![head.to_string()];
    joined.extend(parts[1..].iter().cloned());
    let value = joined.join(" ").trim().to_string();
    let chars: Vec<char> = value.chars().collect();
    if chars.len() >= 2 && chars[0] == '"' && chars[chars.len() - 1] == '"' {
        let inner: String = chars[1..chars.len() - 1].iter().collect();
        return match decode_json_string(&inner) {
            Some(DecodedScalar::Valid(decoded)) => Ok(decoded),
            Some(DecodedScalar::LoneSurrogates(lone)) => Err(lone),
            None => Ok(inner),
        };
    }
    if chars.len() >= 2 && chars[0] == '\'' && chars[chars.len() - 1] == '\'' {
        let inner: String = chars[1..chars.len() - 1].iter().collect();
        return Ok(inner.replace("''", "'"));
    }
    Ok(value
        .split(" #")
        .next()
        .unwrap_or("")
        .trim_end()
        .to_string())
}

/// One skill's index row, read from its `SKILL.md` bytes (the
/// prototype's `read_skill`): `name`/`description` from front matter,
/// with the fallbacks [`SkillMetadata::name`] documents, and this
/// module's lossy decoding for a byte that is not valid UTF-8.
///
/// # Errors
///
/// Returns [`SkillMetadataError::InvalidUnicode`] when `name:` or
/// `description:` holds a JSON `\u` escape that decodes to a lone
/// surrogate (the prototype's `invalid_unicode_fields`): refused before
/// any write, since UTF-8 has no form for it.
pub fn read_skill_metadata(path: &str, bytes: &[u8]) -> Result<SkillMetadata, SkillMetadataError> {
    let text = String::from_utf8_lossy(bytes);
    let parts = frontmatter_parts(&text);
    let mut invalid = Vec::new();
    let mut fields: HashMap<&'static str, String> = HashMap::new();
    for field in ["name", "description"] {
        if let Some(raw_parts) = parts.get(field) {
            match fold_scalar(raw_parts) {
                Ok(value) => {
                    fields.insert(field, value);
                }
                Err(surrogates) => invalid.push(InvalidUnicodeField { field, surrogates }),
            }
        }
    }
    if !invalid.is_empty() {
        return Err(SkillMetadataError::InvalidUnicode(invalid));
    }
    let name = fields
        .remove("name")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| folder_name(path));
    let description = fields.remove("description").unwrap_or_default();
    Ok(SkillMetadata {
        name,
        description,
        path: path.to_string(),
    })
}

/// Appended to a flagged name or description that holds a byte that is
/// not valid UTF-8 (the prototype's `UNDECODABLE_NOTE`, adapted: the
/// prototype shows the byte as `\xNN`; this module shows `U+FFFD`
/// instead, as the module documentation explains).
const UNDECODABLE_NOTE: &str =
    "; \u{FFFD} shows a byte that is not valid UTF-8, so re-save that SKILL.md as UTF-8";

/// `"a", "b"`: each distinct word once, double-quoted, in order (the
/// prototype's `quoted_words`). Neither the word nor the surrounding
/// quotes are escaped, matching the prototype exactly.
fn quoted_words(words: &[String]) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for word in words {
        if !seen.contains(&word.as_str()) {
            seen.push(word.as_str());
        }
    }
    seen.into_iter()
        .map(|word| format!("\"{word}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A bounded stand-in for Python's `repr()` of a string (the
/// prototype's `{relative!r}`): wraps in single quotes, switching to
/// double quotes when the text holds a single quote but no double
/// quote, and escapes a backslash, the chosen quote character, and
/// `\n`/`\r`/`\t`. Scoped to what a project-relative path or a short
/// word needs; it does not reproduce Python's escaping of every other
/// control character or non-printable codepoint.
fn python_repr(value: &str) -> String {
    let quote = if value.contains('\'') && !value.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::new();
    out.push(quote);
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            other => out.push(other),
        }
    }
    out.push(quote);
    out
}

/// Python's `repr()` of one character (the prototype's `repr(char)`
/// inside `unsafe_rows_detail`): [`python_repr`] applied to a one-char
/// string.
fn repr_char(c: char) -> String {
    python_repr(&c.to_string())
}

/// Characters of `path` that would break its `<path>` link target: `<`,
/// `>`, or a line break (the prototype's `link_breakers`), sorted and
/// de-duplicated. Line breaks are `\n` and `\r` only, the same bounded
/// scope [`crate::skills_index`] documents for its own line splitting;
/// the prototype's `str.splitlines()` recognizes a few further Unicode
/// separators a path is not expected to carry.
fn link_breakers(path: &str) -> Vec<char> {
    let mut found: Vec<char> = Vec::new();
    for c in path.chars() {
        if (c == '<' || c == '>' || c == '\n' || c == '\r') && !found.contains(&c) {
            found.push(c);
        }
    }
    found.sort_unstable();
    found
}

/// Each generated row field whose rendered cell holds a possible `@`
/// import, or a path holding a character that breaks its own link (the
/// prototype's `unsafe_row_fields`, VER-H-003). The name, description,
/// and path cells are checked exactly as
/// [`crate::skills_index::render_row`] writes them: text always
/// follows a cell (a closing `|`, or `]` after the name's link label,
/// or `)` after the path), so the end of a cell counts as
/// non-whitespace, matching [`unsafe_at_tokens`]'s default. A row
/// already written in the canonical text is never checked this way --
/// only a row this module is about to generate is; `scan` warns about
/// an existing one separately (ADR-0005 sub-slice 1d).
#[must_use]
pub fn unsafe_row_fields(rows: &[SkillRow]) -> Vec<String> {
    let mut problems = Vec::new();
    for SkillRow {
        name,
        description,
        path,
    } in rows
    {
        for (field_name, value) in [("name", name), ("description", description)] {
            let words = unsafe_at_tokens(&table_cell(value), true);
            if !words.is_empty() {
                let note = if has_undecodable_bytes(value) {
                    UNDECODABLE_NOTE
                } else {
                    ""
                };
                problems.push(format!(
                    "{} (its {field_name} holds {}{note})",
                    python_repr(path),
                    quoted_words(&words)
                ));
            }
        }
        let words = unsafe_at_tokens(&link_target(path), true);
        if !words.is_empty() {
            problems.push(format!(
                "{} (its path holds {})",
                python_repr(path),
                quoted_words(&words)
            ));
        }
        let breakers = link_breakers(path);
        if !breakers.is_empty() {
            let shown = breakers
                .iter()
                .map(|c| repr_char(*c))
                .collect::<Vec<_>>()
                .join(", ");
            problems.push(format!(
                "{} (its path holds {shown}, which breaks the link)",
                python_repr(path)
            ));
        }
    }
    problems
}

/// Why a generated skill-index row cannot be written, as a stable
/// reason token for the app layer (ADR-0005 sub-slice 1e) to report --
/// the house convention this crate's other reason enums use
/// ([`crate::guidance::ManagedFileKind`], [`crate::guidance::ManagedStatus`]):
/// a kebab-case [`Self::as_str`], a round-tripping [`Self::parse`], and
/// an exhaustive [`Self::ALL`].
///
/// The prototype raises its `UnsafeSkillMetadata` exception, under the
/// one exit-level token `unsafe_skill_metadata`, for both causes this
/// module can find: a lone surrogate from [`read_skill_metadata`]
/// ([`SkillMetadataError::InvalidUnicode`]), and a possible `@` import
/// or a link-breaking path character from [`unsafe_row_fields`]. Both
/// map to this one variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillSafetyRefusal {
    /// A generated row cannot be written: either its metadata holds
    /// invalid Unicode, or its rendered name, description, or path
    /// holds a possible `@` import or a link-breaking character.
    UnsafeSkillMetadata,
}

impl SkillSafetyRefusal {
    /// Every refusal, for exhaustive iteration and parsing.
    pub const ALL: [Self; 1] = [Self::UnsafeSkillMetadata];

    /// This refusal's stable token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::UnsafeSkillMetadata => "unsafe-skill-metadata",
        }
    }

    /// Parse a token produced by [`Self::as_str`].
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|refusal| refusal.as_str() == token)
    }
}
