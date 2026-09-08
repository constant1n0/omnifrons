//! `PtyCli`: the second structurally different built-in `HarnessAdapter`
//! (spike slice 4, `docs/spike-log.md` § Slice 4) -- a pseudo-terminal
//! transport with a typed prompt and no structured events, the declared
//! degraded fallback of `docs/target-architecture.md`'s invariant 6.
//!
//! Where the slice-3 line agents share a transport class
//! (`StructuredStreamingCli`), a process topology (pipes), a prompt channel
//! (stdin, then close) and an event producer (a line parser), this adapter
//! differs on every one of those axes: `TransportClass::Pty`, a controlling
//! terminal with the child as session leader, `PromptChannel::PtyTyped`,
//! and a byte normalizer (`omnifrons_app::TerminalNormalizer`) in core
//! instead of a parser here. This adapter itself stays pure: every piece of
//! PTY plumbing lives in `omnifrons-supervisor`, and this crate's
//! dependency set is unchanged (`tests/deps.rs`).
//!
//! `parse_line` is never called on the PTY path (there are no lines to
//! parse -- the normalizer consumes bytes); it is still total, mapping any
//! input to exactly one byte-identical `Unknown`, so the `HarnessAdapter`
//! contract holds for this adapter exactly as for the others.

use omnifrons_app::harness_adapter::{
    AdapterDescriptor, EnvPlan, HarnessAdapter, LaunchPlan, LaunchPlanError, LaunchRequest,
    validate_cwd_within_workspace,
};
use omnifrons_domain::adapter::{
    AdapterEvent, AdapterId, PromptChannel, StdinPlan, TransportClass,
};
use omnifrons_domain::scope::ScopeMode;

/// The user-facing disclosure every `adapters_list` caller sees: what this
/// adapter gives up, what it renders, and what the terminal is not.
const NOTES: &str = "degraded fallback with no structured events: the executable runs inside a \
                     pseudo-terminal and its output is rendered as plain text with layout \
                     controls dropped and counted; a title or notification it asks for is \
                     sanitized and shown as text only; hyperlinks, clipboard access and file \
                     transfer are disabled; the terminal is not a sandbox and grants no \
                     authority; not available on Windows in this slice";

/// Whether a prompt byte would be interpreted by the terminal's line
/// discipline (default `termios`: canonical mode, `ISIG`, `ICRNL`) rather
/// than typed: every C0 control except newline (ends a line the child
/// reads as such) and tab (typed literally), and DEL (`VERASE` on Linux and
/// macOS). Among the refused bytes are `VINTR` (0x03), `VEOF` (0x04),
/// `VSUSP` (0x1A), `VQUIT` (0x1C), ESC (would open a sequence), and `\r`
/// (would act as Enter mid-text). Multi-byte UTF-8 bytes are all
/// `>= 0x80` and never match.
fn line_discipline_interprets(byte: u8) -> bool {
    matches!(byte, 0x00..=0x08 | 0x0B..=0x1F | 0x7F)
}

/// The pseudo-terminal `HarnessAdapter`: an empty argv template, the
/// prompt typed into the terminal, the base environment allowlist only.
#[derive(Debug, Clone, Copy, Default)]
pub struct PtyCli;

impl PtyCli {
    /// Build the adapter. It holds no state: its descriptor is fixed.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl HarnessAdapter for PtyCli {
    fn describe(&self) -> AdapterDescriptor {
        AdapterDescriptor {
            id: AdapterId::pty_cli(),
            display_name: "PTY CLI".to_string(),
            transport_class: TransportClass::Pty,
            prompt_channel: PromptChannel::PtyTyped,
            argv_template: Vec::new(),
            declared_env: Vec::new(),
            scope_mode: ScopeMode::Advisory,
            notes: NOTES.to_string(),
        }
    }

    fn build_launch(&self, request: &LaunchRequest) -> Result<LaunchPlan, LaunchPlanError> {
        let descriptor = self.describe();
        let env = EnvPlan::new(&descriptor.declared_env)?;

        // The prompt is typed through the terminal's line discipline, which
        // interprets rather than types every control byte but newline and
        // tab: refused here, before anything is launched.
        if request
            .prompt
            .as_str()
            .bytes()
            .any(line_discipline_interprets)
        {
            return Err(LaunchPlanError::PromptNotTypeable);
        }

        // Always the workspace's own path as cwd, validated exactly as
        // `LineAgent::build_launch` does.
        validate_cwd_within_workspace(request.workspace.path(), &request.workspace)?;

        Ok(LaunchPlan {
            argv: descriptor.argv_template,
            env,
            cwd: request.workspace.clone(),
            // Unused on the PTY path: the child's stdin is the terminal's
            // slave end, and the prompt is typed into it, never piped.
            stdin: StdinPlan::Null,
            prompt: Some(request.prompt.clone()),
            scope_mode: descriptor.scope_mode,
            transport: TransportClass::Pty,
            output_dir: None,
        })
    }

    fn parse_line(&self, line: &str) -> Vec<AdapterEvent> {
        vec![AdapterEvent::Unknown {
            raw: line.as_bytes().to_vec(),
            truncated: false,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::PtyCli;
    use omnifrons_app::harness_adapter::HarnessAdapter as _;

    #[test]
    fn describe_is_stable() {
        let adapter = PtyCli::new();
        assert_eq!(adapter.describe(), adapter.describe());
    }
}
