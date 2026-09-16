//! Parses the append-only evidence record grammar: `docs/evidence/VP-001/
//! {baselines,records}.md` hold one `##` section per record, each carrying
//! exactly one two-column `| field | value |` markdown table (design.md
//! D1). This module only recognizes the shape; semantic admission rules
//! (V1-V8) live in [`crate::validate`].

use std::collections::BTreeMap;

/// One parsed record: every `field` -> `value` cell found in its table,
/// keyed exactly as written with surrounding backticks stripped.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Record {
    pub fields: BTreeMap<String, String>,
}

impl Record {
    /// Looks up a field's value, treating a missing key the same as an
    /// empty value -- callers never need to distinguish the two.
    #[must_use]
    pub fn get(&self, field: &str) -> Option<&str> {
        self.fields.get(field).map(String::as_str)
    }

    #[must_use]
    pub fn kind(&self) -> Option<&str> {
        self.get("kind")
    }

    #[must_use]
    pub fn record_id(&self) -> Option<&str> {
        self.get("record_id")
    }
}

/// A structural or semantic rejection. `token` is the closed vocabulary from
/// design.md's validator contract for [`crate::validate`] output;
/// structural parse failures use their own tokens outside that table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub record_id: Option<String>,
    pub token: String,
    pub detail: String,
}

impl Violation {
    pub(crate) fn new(record_id: Option<&str>, token: &str, detail: impl Into<String>) -> Self {
        Self {
            record_id: record_id.map(str::to_string),
            token: token.to_string(),
            detail: detail.into(),
        }
    }
}

/// Parses `text` into zero or more records. Fails only on a structural
/// grammar violation -- a `##` section with no parseable two-column table.
/// Missing or empty mandatory fields, including `kind` itself, are the
/// semantic V1 concern of [`crate::validate::validate`], not this function.
///
/// # Errors
///
/// Returns every structural violation found (a section with no table, no
/// separator row, a row with no value column, or a duplicate field key
/// within one record) instead of stopping at the first one.
pub fn parse(text: &str) -> Result<Vec<Record>, Vec<Violation>> {
    let mut records = Vec::new();
    let mut violations = Vec::new();

    for section in split_sections(text) {
        match parse_table(&section) {
            Ok(fields) => records.push(Record { fields }),
            Err(detail) => violations.push(Violation::new(None, "malformed-table", detail)),
        }
    }

    if violations.is_empty() {
        Ok(records)
    } else {
        Err(violations)
    }
}

/// Splits `text` on top-level `## ` headings. Text before the first heading
/// (a document title, an intro paragraph) is not a record and is dropped.
fn split_sections(text: &str) -> Vec<String> {
    let mut sections = Vec::new();
    let mut current: Option<String> = None;

    for line in text.lines() {
        if line.starts_with("## ") {
            if let Some(section) = current.take() {
                sections.push(section);
            }
            current = Some(String::new());
        } else if let Some(section) = current.as_mut() {
            section.push_str(line);
            section.push('\n');
        }
    }
    if let Some(section) = current {
        sections.push(section);
    }
    sections
}

/// Extracts a section's two-column table into a field -> value map. The
/// header and separator rows (`| field | value |`, `| --- | --- |`) are
/// skipped; every following `| key | value |` row becomes one field.
fn parse_table(section: &str) -> Result<BTreeMap<String, String>, String> {
    let rows: Vec<Vec<String>> = section
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('|'))
        .map(split_row)
        .collect();

    let Some((header, rest)) = rows.split_first() else {
        return Err("section has no markdown table".to_string());
    };
    if header.len() < 2 {
        return Err("table header must have at least two columns".to_string());
    }
    let Some((_separator, data_rows)) = rest.split_first() else {
        return Err("table has no separator row".to_string());
    };

    let mut fields = BTreeMap::new();
    for row in data_rows {
        if row.len() < 2 {
            return Err(format!("row {row:?} does not have a value column"));
        }
        let key = row[0].trim_matches('`').trim().to_string();
        let value = row[1].trim_matches('`').trim().to_string();
        if fields.insert(key.clone(), value).is_some() {
            return Err(format!("duplicate field `{key}` in one record"));
        }
    }
    Ok(fields)
}

fn split_row(line: &str) -> Vec<String> {
    line.trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parses_one_record_into_a_field_map() {
        let text = "## Record\n\n| field | value |\n| --- | --- |\n| `kind` | baseline |\n| `os_build` | Ubuntu |\n";
        let records = parse(text).expect("well-formed table must parse");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].get("kind"), Some("baseline"));
        assert_eq!(records[0].get("os_build"), Some("Ubuntu"));
    }

    #[test]
    fn parses_multiple_sections_into_separate_records() {
        let text = "## A\n\n| field | value |\n| --- | --- |\n| `record_id` | R1 |\n\n\
                    ## B\n\n| field | value |\n| --- | --- |\n| `record_id` | R2 |\n";
        let records = parse(text).expect("well-formed table must parse");
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].get("record_id"), Some("R1"));
        assert_eq!(records[1].get("record_id"), Some("R2"));
    }

    #[test]
    fn section_with_no_table_is_a_malformed_table_violation() {
        let text = "## Record\n\nNo table here.\n";
        let violations = parse(text).expect_err("a section with no table must fail to parse");
        assert_eq!(violations[0].token, "malformed-table");
    }

    #[test]
    fn text_with_no_heading_parses_to_zero_records() {
        let records = parse("Just a title\n\nSome prose.\n").expect("empty input has no records");
        assert!(records.is_empty());
    }
}
