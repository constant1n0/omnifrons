//! V2: `result` must be exactly one of the closed vocabulary values; a
//! compound rendering like `orphan-risk/uncertain` is rejected here, since
//! the state half belongs in `observed_state` instead.

use crate::record::{Record, Violation};

/// The closed `result` vocabulary (Closed Outcome Vocabulary requirement).
/// A compound rendering such as `orphan-risk/uncertain` is never a member --
/// that state belongs in the separate `observed_state` field.
const RESULT_VOCABULARY: &[&str] = &["pass", "fail", "uncertain"];

pub(super) fn check(records: &[Record], violations: &mut Vec<Violation>) {
    for record in records {
        if let Some(result) = record.get("result")
            && !result.is_empty()
            && !RESULT_VOCABULARY.contains(&result)
        {
            violations.push(Violation::new(
                record.record_id(),
                "result-out-of-vocabulary",
                format!("result `{result}` is not one of {RESULT_VOCABULARY:?}"),
            ));
        }
    }
}
