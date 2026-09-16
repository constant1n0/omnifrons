//! VP-S6 fixture agent (design.md D8): a tiny ELF that ignores argv and
//! stdin, spawns a breakaway-attempting descendant, records both processes'
//! `(pid, starttime)` identity into its own working directory, then sleeps
//! -- self-bounded so it always exits on its own, even if nothing ever
//! stops it.
//!
//! Not a shell script (design.md D8): the Linux launch path execs a sealed
//! memfd (`crates/omnifrons-supervisor/src/lib.rs` (read-only)), so shebang
//! resolution through it is not something this change's evidence should
//! rest on. Not `/bin/sh` either: the `stream-json-cli` adapter's fixed,
//! empty `argv_template` (`crates/omnifrons-adapters/src/line_agent.rs`
//! (read-only)) means this fixture never receives arguments regardless, but
//! a real ELF is what any adapter's `argv_template` could ever hand a
//! caller-supplied executable.

use std::io::Read as _;
use std::process::Command;
use std::time::Duration;

/// How long the breakaway-attempting descendant (`setsid sleep <n>`) runs
/// before exiting on its own, and this fixture's own upper bound on how
/// long it sleeps after recording pid identity. This change's own safety
/// requirement: a containment failure must never leave an immortal process
/// behind, on this machine or a CI runner, even if nobody ever stops it.
const MAX_LIFETIME_SECS: u64 = 120;

/// The filename this fixture writes its declared identity into, relative
/// to its own working directory (the workspace the harness picked for this
/// run -- `crates/omnifrons-app/src/harness_adapter.rs`'s `LaunchPlan::cwd`
/// (read-only), never an argv-supplied or hardcoded path).
const PID_FILE_NAME: &str = "vp-s6-agent.pids";

/// Parse the `starttime` field (clock ticks since boot, `proc(5)` field 22)
/// out of the raw contents of a `/proc/<pid>/stat` file.
///
/// Robust to a `comm` field that itself contains spaces or `)` characters:
/// per `proc(5)`, the reliable way to skip `comm` is to find the line's
/// *last* `)`, never its first.
fn parse_proc_stat_starttime(stat_contents: &str) -> Option<u64> {
    let close_paren = stat_contents.rfind(')')?;
    let rest = stat_contents.get(close_paren + 1..)?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // After `comm)`, field 0 of `rest` is `state` (`proc(5)` field 3);
    // `starttime` (`proc(5)` field 22) therefore sits at index 19 of this
    // zero-based remainder.
    fields.get(19)?.parse().ok()
}

/// Render this fixture's declared-identity file: one line per process,
/// `<role> <pid> <starttime>`, role first so a human or a script can `grep`
/// for either without parsing position.
///
/// This is a declaration, not evidence on its own -- design.md's honesty
/// machinery has `vp-s6-linux.sh` re-read `/proc/<pid>/stat` for both pids
/// directly rather than trusting the starttime values recorded here, so a
/// compromised or misbehaving approved executable can never lie its way
/// into a false containment proof.
fn format_pid_file(
    self_pid: u32,
    self_starttime: u64,
    descendant_pid: u32,
    descendant_starttime: u64,
) -> String {
    format!(
        "self {self_pid} {self_starttime}\ndescendant {descendant_pid} {descendant_starttime}\n"
    )
}

/// Read and parse `/proc/<pid>/stat`'s `starttime` field, panicking with a
/// clear message if the file cannot be read or does not carry the field --
/// both are unrecoverable for this fixture's one job (there is nothing
/// sensible to record instead).
fn read_starttime(stat_path: &str) -> u64 {
    let contents = std::fs::read_to_string(stat_path)
        .unwrap_or_else(|error| panic!("{stat_path} must be readable: {error}"));
    parse_proc_stat_starttime(&contents)
        .unwrap_or_else(|| panic!("{stat_path} must carry a starttime field"))
}

fn main() {
    // Ignore argv entirely: the launching adapter's own fixed template
    // (`crates/omnifrons-adapters/src/line_agent.rs` (read-only)) is the
    // only argv this fixture is ever handed, and it never carries anything
    // this fixture needs to interpret.
    let _ = std::env::args();

    // Ignore stdin, but drain it to EOF rather than never reading it: the
    // `stream-json-cli` adapter's `StdinThenClose` contract writes a prompt
    // then closes the write end, and draining means that write can never
    // block regardless of the prompt's size.
    let mut discarded = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut discarded);

    // The breakaway attempt (design.md D8; Threat Matrix "Subprocess /
    // process integration"): `setsid` puts `sleep` in a brand-new session
    // and process group, detaching it from this process's own group --
    // exactly the group `killpg` targets
    // (`crates/omnifrons-supervisor/src/lib.rs` (read-only), which calls
    // `command.process_group(0)` at spawn time so this fixture's own pid
    // doubles as its pgid). Bounded to `MAX_LIFETIME_SECS` so an unkilled
    // descendant still exits on its own.
    //
    // Deliberately never `wait()`ed on: the whole point of the breakaway
    // attempt is that this descendant may keep running independently of
    // this fixture (that is exactly the containment gap VP-S6 measures), so
    // blocking on it here would defeat the design. `MAX_LIFETIME_SECS`
    // bounds any zombie window regardless -- once this fixture exits (at
    // the same bound or sooner, if killed), the descendant reparents to the
    // nearest subreaper, which reaps it.
    #[allow(clippy::zombie_processes)]
    let descendant = Command::new("setsid")
        .arg("sleep")
        .arg(MAX_LIFETIME_SECS.to_string())
        .spawn()
        .expect("setsid and sleep must both be on PATH for this fixture to run");

    let self_pid = std::process::id();
    let self_starttime = read_starttime("/proc/self/stat");
    let descendant_pid = descendant.id();
    let descendant_starttime = read_starttime(&format!("/proc/{descendant_pid}/stat"));

    std::fs::write(
        PID_FILE_NAME,
        format_pid_file(
            self_pid,
            self_starttime,
            descendant_pid,
            descendant_starttime,
        ),
    )
    .expect("the current working directory (the picked workspace) must be writable");

    // Self-bounding: this fixture exits on its own after MAX_LIFETIME_SECS
    // even if the harness's Stop never reaches it, so a containment
    // failure can never leave an immortal process behind.
    std::thread::sleep(Duration::from_secs(MAX_LIFETIME_SECS));
}

#[cfg(test)]
mod tests {
    use super::{format_pid_file, parse_proc_stat_starttime};

    #[test]
    fn format_pid_file_renders_role_pid_starttime_lines() {
        assert_eq!(
            format_pid_file(111, 222, 333, 444),
            "self 111 222\ndescendant 333 444\n"
        );
    }

    #[test]
    fn parse_proc_stat_starttime_reads_field_22() {
        // A synthetic but shaped-correctly `/proc/<pid>/stat` line: pid,
        // `(comm)`, then the fields through `starttime` (`proc(5)` field
        // 22, index 19 counting from `state` right after the closing
        // paren).
        let stat =
            "4242 (vp-s6-agent) S 1 4242 4242 0 -1 4194560 100 0 0 0 0 0 0 0 20 0 1 0 999888 ...";
        assert_eq!(parse_proc_stat_starttime(stat), Some(999_888));
    }

    #[test]
    fn parse_proc_stat_starttime_handles_a_comm_with_spaces_and_parens() {
        // `comm` can itself contain `)` (`proc(5)`); only the line's *last*
        // `)` reliably marks its end.
        let stat = "77 (cmd) with ) parens) S 1 77 77 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 555000 ...";
        assert_eq!(parse_proc_stat_starttime(stat), Some(555_000));
    }

    #[test]
    fn parse_proc_stat_starttime_rejects_a_line_with_too_few_fields() {
        let stat = "1 (init) S 0 1 1";
        assert_eq!(parse_proc_stat_starttime(stat), None);
    }
}
