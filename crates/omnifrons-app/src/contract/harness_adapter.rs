//! A reusable `HarnessAdapter` contract, runnable against any
//! implementation: a fake test double (`omnifrons-app`'s own tests) or a
//! real built-in adapter (`omnifrons-adapters`'s tests, via this crate as a
//! dev-dependency with the `contract-tests` feature enabled).

use crate::harness_adapter::HarnessAdapter;

/// A fixed corpus of inputs `parse_line` must handle without panicking,
/// covering: the empty line, a truncated/malformed JSON fragment, a very
/// large line (1 MiB), and text containing the UTF-8 replacement character
/// (standing in for what a lossy upstream decode of invalid UTF-8
/// produces -- `parse_line` itself only ever receives a valid `&str`, never
/// raw invalid bytes).
fn parse_line_corpus() -> Vec<String> {
    vec![
        String::new(),
        "{".to_string(),
        "a".repeat(1024 * 1024),
        "before \u{FFFD} after".to_string(),
    ]
}

/// Exercise the baseline `HarnessAdapter` contract: `describe` is stable
/// across repeated calls, and `parse_line` is total over
/// [`parse_line_corpus`] -- it never panics, and every unparsable input in
/// the corpus yields exactly one
/// [`omnifrons_domain::adapter::AdapterEvent::Unknown`] (never an empty
/// event list, and never more than one event for a line that carries no
/// structure at all).
///
/// `make` builds a fresh adapter instance so the contract can be run
/// against implementations that hold internal state.
///
/// # Panics
///
/// Panics (via `assert*`) if `describe` is not stable, or if any corpus
/// input does not yield exactly one `Unknown`. A `parse_line`
/// implementation that itself panics on a corpus input aborts this test
/// directly, which is exactly the "never panics" property this contract
/// requires.
pub fn harness_adapter_contract<A: HarnessAdapter>(make: impl Fn() -> A) {
    let adapter = make();

    let first = adapter.describe();
    let second = adapter.describe();
    assert_eq!(
        first, second,
        "describe() must be stable across repeated calls"
    );

    for line in parse_line_corpus() {
        let events = adapter.parse_line(&line);
        assert_eq!(
            events.len(),
            1,
            "parse_line must yield exactly one event for unparsable input {line:?}, got {events:?}"
        );
        assert!(
            matches!(
                events[0],
                omnifrons_domain::adapter::AdapterEvent::Unknown { .. }
            ),
            "parse_line must yield Unknown for unparsable input {line:?}, got {events:?}"
        );
    }
}
