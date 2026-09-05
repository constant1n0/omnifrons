//! Scope modes and process terminal states.
//!
//! `ScopeMode` encodes target architecture invariant 5: `ActiveProjectRoot`
//! is always the requested scope, but it is a security boundary only in
//! `sandbox-enforced` mode (docs/target-architecture.md § Proposed
//! invariants, item 5). TM-001 restates the same rule for the threat model:
//! "only `sandbox-enforced` is a security boundary" (docs/threat-model.md
//! § Trust boundaries).

/// The scope-enforcement mode a harness runs under.
///
/// Only [`ScopeMode::SandboxEnforced`] is a security boundary; the other two
/// variants are labels describing the observed containment posture, never a
/// protection claim (docs/target-architecture.md § Proposed invariants, item
/// 5; docs/threat-model.md § Trust boundaries).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScopeMode {
    /// An OS sandbox or equivalent verified mechanism enforces the declared
    /// filesystem/network/process limits.
    SandboxEnforced,
    /// A supported native harness control enforces the declared limit;
    /// Omnifrons reports its dependency and tested version.
    HarnessEnforced,
    /// Omnifrons sets `cwd` and instructions but the harness retains normal
    /// user permissions.
    Advisory,
}

impl ScopeMode {
    /// Whether this scope mode is a security boundary.
    ///
    /// True only for [`ScopeMode::SandboxEnforced`]; `harness-enforced` and
    /// `advisory` MUST NOT be presented as protection (docs/threat-model.md
    /// § Trust boundaries: "only `sandbox-enforced` is a security
    /// boundary").
    #[must_use]
    pub const fn is_security_boundary(&self) -> bool {
        matches!(self, Self::SandboxEnforced)
    }
}

/// The observed terminal state of a supervised process.
///
/// `OrphanRiskUncertain` is the required failure state when descendants
/// cannot be proven stopped: "Process descendants unproven stopped ->
/// orphan-risk/uncertain" (docs/target-architecture.md § Required failure
/// states).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProcessTerminalState {
    /// The process exited on its own or after a graceful stop signal, with
    /// the observed exit code where the platform reports one.
    Exited {
        /// The process exit code, when the platform reports one.
        code: Option<i32>,
    },
    /// The process was forcibly terminated (e.g. `SIGKILL`) after failing to
    /// stop gracefully within the deadline.
    Killed,
    /// Process descendants unproven stopped -> orphan-risk/uncertain
    /// (docs/target-architecture.md § Required failure states). Reported
    /// when the supervisor cannot prove that every descendant process has
    /// stopped, so the result MUST NOT be presented as "cleanly stopped".
    OrphanRiskUncertain,
}

#[cfg(test)]
mod tests {
    use super::{ProcessTerminalState, ScopeMode};

    #[test]
    fn scope_mode_is_security_boundary_only_for_sandbox_enforced() {
        let cases = [
            (ScopeMode::SandboxEnforced, true),
            (ScopeMode::HarnessEnforced, false),
            (ScopeMode::Advisory, false),
        ];
        for (mode, expected) in cases {
            assert_eq!(
                mode.is_security_boundary(),
                expected,
                "{mode:?}.is_security_boundary() must be {expected}"
            );
        }
    }

    #[test]
    fn process_terminal_state_variants_are_distinct() {
        assert_ne!(
            ProcessTerminalState::Exited { code: Some(0) },
            ProcessTerminalState::Killed
        );
        assert_ne!(
            ProcessTerminalState::Killed,
            ProcessTerminalState::OrphanRiskUncertain
        );
        assert_ne!(
            ProcessTerminalState::Exited { code: Some(0) },
            ProcessTerminalState::OrphanRiskUncertain
        );
    }
}
