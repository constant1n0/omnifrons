//! The demo harness's actual business logic, shared by the test-only
//! `demo-harness` binary (`src/bin/demo-harness.rs`) and, behind the
//! `demo-harness` cargo feature, the shell's hidden
//! `--demo-harness <kind> <rate_hz> <lines>` argv branch.
//!
//! No program path or argument vector for this crosses IPC
//! (`docs/spike-log.md` § IPC contract): the shell only ever receives a
//! validated `omnifrons_app::HarnessRequest`, and [`kind_arg`]/[`parse_kind`]
//! are the one stable, closed vocabulary the supervisor and this binary
//! agree on to encode [`HarnessKind`] as a single argv token.

use std::io::Write as _;
use std::process::ExitCode;
use std::time::Duration;

use omnifrons_app::HarnessKind;

/// Why [`kind_arg`] could not encode a [`HarnessKind`] as an argv token:
/// `kind` was [`HarnessKind::Approved`] or [`HarnessKind::Adapter`],
/// neither of which ever crosses the demo harness's argv-encoding path at
/// all.
///
/// Structurally unreachable in practice --
/// `omnifrons_app::HarnessRequest::new` already rejects both variants with
/// `InvalidRequest::KindNotDemo` before a `HarnessRequest` (the only thing
/// `kind_arg`'s one real caller, `TokioProcessSupervisor::spawn_harness`,
/// ever holds a `kind` from) can even be constructed -- but `kind_arg`
/// itself takes a bare `HarnessKind`, not a `HarnessRequest`, so it stays a
/// total function over every value its own parameter type allows, an
/// error, never a panic, if that invariant is ever violated by some future
/// caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NotADemoKind;

impl std::fmt::Display for NotADemoKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HarnessKind::Approved and HarnessKind::Adapter are not demo harness kinds")
    }
}

impl std::error::Error for NotADemoKind {}

/// The stable argv token for `kind`: [`kind_arg`] encodes it (used by
/// `TokioProcessSupervisor::spawn_harness`), [`parse_kind`] decodes it
/// (used by the `demo-harness` binary's own argv parsing).
///
/// # Errors
///
/// Returns [`NotADemoKind`] if `kind` is [`HarnessKind::Approved`]: see
/// that type's own doc comment for why this is an error, not a panic,
/// even though `spawn_harness`'s own caller has already ruled it out.
pub fn kind_arg(kind: &HarnessKind) -> Result<&'static str, NotADemoKind> {
    match kind {
        HarnessKind::DemoLines => Ok("demo-lines"),
        HarnessKind::DemoIgnoresSigterm => Ok("demo-ignores-sigterm"),
        HarnessKind::Approved(_) | HarnessKind::Adapter { .. } => Err(NotADemoKind),
    }
}

/// Parse a [`kind_arg`] token back into a [`HarnessKind`], or `None` if it
/// is not one of the two recognized tokens.
#[must_use]
pub fn parse_kind(arg: &str) -> Option<HarnessKind> {
    match arg {
        "demo-lines" => Some(HarnessKind::DemoLines),
        "demo-ignores-sigterm" => Some(HarnessKind::DemoIgnoresSigterm),
        _ => None,
    }
}

/// Run the demo harness: print a leading `"ready"` line to stdout, then
/// `lines` lines to stdout at `rate_hz` (`"line <n> out"`), and every 5th
/// line also to stderr (`"line <n> err"`), then exit `0`.
///
/// [`HarnessKind::DemoIgnoresSigterm`] additionally installs a `SIGTERM`
/// ignore handler (unix only) before emitting, so only a forceful stop
/// (`SIGKILL`) can terminate it early; on any other platform this is
/// currently indistinguishable from [`HarnessKind::DemoLines`].
///
/// The `"ready"` line is printed only *after* that handler is installed --
/// for both kinds, symmetrically, even though [`HarnessKind::DemoLines`]
/// installs no handler of its own -- so a caller that subscribes to this
/// process's output and waits for `"ready"` before calling `stop` has a
/// genuine synchronization point for "the handler, if any, is in place
/// now", rather than a fixed sleep and a guess at how long installation
/// takes (`crates/omnifrons-supervisor/tests/demo_harness.rs`).
///
/// # Panics
///
/// Panics if stdout or stderr cannot be written to or flushed (e.g. a
/// broken pipe), or (unix, `DemoIgnoresSigterm` only) if the `SIGTERM`
/// ignore handler could not be installed.
#[must_use]
pub fn run(kind: &HarnessKind, rate_hz: u16, lines: u32) -> ExitCode {
    if matches!(kind, HarnessKind::DemoIgnoresSigterm) {
        #[cfg(unix)]
        unix::ignore_sigterm();
    }

    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();

    writeln!(stdout, "ready").expect("stdout must accept the demo harness's ready line");
    stdout.flush().expect("stdout must flush");

    let interval = Duration::from_secs_f64(1.0 / f64::from(rate_hz.max(1)));

    for n in 1..=lines {
        if n > 1 {
            std::thread::sleep(interval);
        }
        writeln!(stdout, "line {n} out").expect("stdout must accept the demo harness's output");
        stdout.flush().expect("stdout must flush");
        if n % 5 == 0 {
            writeln!(stderr, "line {n} err").expect("stderr must accept the demo harness's output");
            stderr.flush().expect("stderr must flush");
        }
    }

    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use omnifrons_app::HarnessKind;
    use omnifrons_domain::adapter::{AdapterId, AgentPrompt};
    use omnifrons_domain::executable::ApprovalId;

    use super::{NotADemoKind, kind_arg, parse_kind};

    /// R3-011: neither launch kind that reaches a real executable ever
    /// encodes as a demo-harness argv token -- an error, never a token and
    /// never a panic.
    #[test]
    fn kind_arg_refuses_approved_and_adapter_kinds() {
        let approved = HarnessKind::Approved(ApprovalId(7));
        let adapter = HarnessKind::Adapter {
            adapter: AdapterId::claude_code(),
            approval: ApprovalId(7),
            prompt: AgentPrompt::new("do the thing").expect("a short prompt is valid"),
        };

        assert_eq!(kind_arg(&approved), Err(NotADemoKind));
        assert_eq!(kind_arg(&adapter), Err(NotADemoKind));
    }

    #[test]
    fn kind_arg_and_parse_kind_round_trip_the_two_demo_kinds() {
        for kind in [HarnessKind::DemoLines, HarnessKind::DemoIgnoresSigterm] {
            let token = kind_arg(&kind).expect("a demo kind always encodes");
            assert!(
                matches!(parse_kind(token), Some(decoded) if std::mem::discriminant(&decoded) == std::mem::discriminant(&kind)),
                "{token} must decode back to the kind that produced it"
            );
        }
        assert_eq!(parse_kind("approved"), None);
        assert_eq!(parse_kind("adapter"), None);
    }
}

#[cfg(unix)]
mod unix {
    /// Install a `SIGTERM`-ignoring disposition.
    ///
    /// `nix::sys::signal::signal` is `unsafe` in general (installing an
    /// arbitrary signal-handler function pointer is inherently unsafe --
    /// the handler must be async-signal-safe), but the only disposition
    /// ever installed here is `SigHandler::SigIgn`, which runs no handler
    /// function of ours at all, so nothing here can violate async-signal-
    /// safety. This one call is therefore isolated behind an explicit,
    /// documented `#[allow(unsafe_code)]`, matching the workspace's
    /// `unsafe_code = "deny"` default everywhere else.
    #[allow(unsafe_code)]
    pub(super) fn ignore_sigterm() {
        // SAFETY: SigIgn installs no handler function of ours, so there is
        // nothing that could violate async-signal-safety; ignoring
        // SIGTERM is the documented, deliberate behavior under test
        // (`DemoIgnoresSigterm`).
        unsafe {
            nix::sys::signal::signal(
                nix::sys::signal::Signal::SIGTERM,
                nix::sys::signal::SigHandler::SigIgn,
            )
        }
        .expect("failed to install the SIGTERM-ignore handler");
    }
}
