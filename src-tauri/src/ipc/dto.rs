//! Typed IPC data shapes: the renderer only ever sees these, never a
//! domain type directly (`docs/spike-log.md` § IPC contract). Every field
//! name is `camelCase` on the wire; every closed-set discriminant (a
//! harness kind, a terminal state, an error code, a frame's stream) is a
//! fixed `kebab-case` token.

use serde::{Deserialize, Serialize};

/// The closed set of demo harness kinds a caller may request, as it
/// crosses IPC. See [`omnifrons_app::HarnessKind`] for the domain type
/// this converts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessKindDto {
    DemoLines,
    DemoIgnoresSigterm,
}

impl From<HarnessKindDto> for omnifrons_app::HarnessKind {
    fn from(kind: HarnessKindDto) -> Self {
        match kind {
            HarnessKindDto::DemoLines => Self::DemoLines,
            HarnessKindDto::DemoIgnoresSigterm => Self::DemoIgnoresSigterm,
        }
    }
}

/// A process identifier crossing IPC: transparently the platform process
/// id, matching [`omnifrons_app::ProcessId`]'s own transparency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessIdDto(pub u32);

impl From<omnifrons_app::ProcessId> for ProcessIdDto {
    fn from(id: omnifrons_app::ProcessId) -> Self {
        Self(id.0)
    }
}

impl From<ProcessIdDto> for omnifrons_app::ProcessId {
    fn from(id: ProcessIdDto) -> Self {
        Self(id.0)
    }
}

/// The closed set of terminal-state tokens, without the `exited` variant's
/// `code` -- see [`ProcessTerminalStateDto`], which pairs this with `code`
/// as an always-present sibling field rather than nesting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessTerminalStateTag {
    Exited,
    Killed,
    /// The literal wire token is `orphan-risk/uncertain` (with the slash),
    /// matching `docs/target-architecture.md` § Required failure states'
    /// own spelling of this outcome verbatim.
    #[serde(rename = "orphan-risk/uncertain")]
    OrphanRiskUncertain,
}

/// A process's terminal state, as it crosses IPC: `state` is always
/// present as one of the three closed tokens, and `code` is always
/// present too -- `null` except when `state` is `exited` and the platform
/// reported an exit code.
///
/// Deliberately flat (not an internally tagged enum with a per-variant
/// field set) so the exact same shape also flattens cleanly into
/// [`HarnessFrame::State`]'s `id`/`seq`/`droppedBefore` fields, and so
/// [`crate::ipc::commands::harness_stop`] can return it directly with no
/// extra wrapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ProcessTerminalStateDto {
    pub state: ProcessTerminalStateTag,
    pub code: Option<i32>,
}

impl From<omnifrons_app::ProcessTerminalState> for ProcessTerminalStateDto {
    fn from(state: omnifrons_app::ProcessTerminalState) -> Self {
        use omnifrons_app::ProcessTerminalState as Domain;
        match state {
            Domain::Exited { code } => Self {
                state: ProcessTerminalStateTag::Exited,
                code,
            },
            Domain::Killed => Self {
                state: ProcessTerminalStateTag::Killed,
                code: None,
            },
            Domain::OrphanRiskUncertain => Self {
                state: ProcessTerminalStateTag::OrphanRiskUncertain,
                code: None,
            },
        }
    }
}

/// A process's observed status, as it crosses IPC: either still running,
/// or terminal with the same flat state/code shape as
/// [`ProcessTerminalStateDto`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum ProcessStatusDto {
    Running,
    Terminal {
        #[serde(flatten)]
        terminal: ProcessTerminalStateDto,
    },
}

impl From<omnifrons_app::ProcessStatus> for ProcessStatusDto {
    fn from(status: omnifrons_app::ProcessStatus) -> Self {
        match status {
            omnifrons_app::ProcessStatus::Running => Self::Running,
            omnifrons_app::ProcessStatus::Terminal(state) => Self::Terminal {
                terminal: state.into(),
            },
        }
    }
}

/// One streamed frame delivered over `harness_spawn`'s `onFrame` channel.
///
/// Adjacently tagged: `{"stream": "stdout" | "stderr" | "state", "body":
/// {...}}`. `stdout`/`stderr` carry decoded text; `state` carries the same
/// flat terminal-state shape as [`ProcessTerminalStateDto`], flattened in
/// alongside `id`/`seq`/`droppedBefore` rather than nested.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "stream",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HarnessFrame {
    Stdout {
        id: ProcessIdDto,
        seq: u64,
        dropped_before: u64,
        text: String,
    },
    Stderr {
        id: ProcessIdDto,
        seq: u64,
        dropped_before: u64,
        text: String,
    },
    State {
        id: ProcessIdDto,
        seq: u64,
        dropped_before: u64,
        #[serde(flatten)]
        terminal: ProcessTerminalStateDto,
    },
}

impl HarnessFrame {
    /// Build the wire frame for `id` from a captured
    /// [`omnifrons_app::OutputFrame`].
    #[must_use]
    pub fn from_domain(id: ProcessIdDto, frame: omnifrons_app::OutputFrame) -> Self {
        match frame.payload {
            omnifrons_app::FramePayload::Text {
                stream: omnifrons_app::OutputStream::Stdout,
                text,
            } => Self::Stdout {
                id,
                seq: frame.seq,
                dropped_before: frame.dropped_before,
                text,
            },
            omnifrons_app::FramePayload::Text {
                stream: omnifrons_app::OutputStream::Stderr,
                text,
            } => Self::Stderr {
                id,
                seq: frame.seq,
                dropped_before: frame.dropped_before,
                text,
            },
            omnifrons_app::FramePayload::State(state) => Self::State {
                id,
                seq: frame.seq,
                dropped_before: frame.dropped_before,
                terminal: state.into(),
            },
        }
    }
}

/// The closed set of error codes a failed IPC command reports. Fixed and
/// exhaustive: every value the renderer can compare against structurally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShellErrorCode {
    UnknownProcess,
    SpawnFailed,
    AlreadySubscribed,
    InvalidRequest,
    TooManyProcesses,
}

/// A failed IPC command's error payload. `message` is always a catalogue
/// string, never the underlying error's own text -- see
/// `crate::ipc::commands`' `SupervisorError`/`InvalidRequest` mappings,
/// which is where an `io::Error` carrying a real path would otherwise leak
/// (`docs/spike-log.md` § IPC contract).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShellError {
    pub code: ShellErrorCode,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::{
        HarnessFrame, HarnessKindDto, ProcessIdDto, ProcessStatusDto, ProcessTerminalStateDto,
        ProcessTerminalStateTag, ShellError, ShellErrorCode,
    };

    fn json(value: &impl serde::Serialize) -> serde_json::Value {
        serde_json::to_value(value).expect("serialization must succeed")
    }

    #[test]
    fn harness_kind_dto_deserializes_from_kebab_case_tokens() {
        let demo_lines: HarnessKindDto =
            serde_json::from_str("\"demo-lines\"").expect("demo-lines must deserialize");
        assert_eq!(demo_lines, HarnessKindDto::DemoLines);

        let ignores_sigterm: HarnessKindDto = serde_json::from_str("\"demo-ignores-sigterm\"")
            .expect("demo-ignores-sigterm must deserialize");
        assert_eq!(ignores_sigterm, HarnessKindDto::DemoIgnoresSigterm);
    }

    #[test]
    fn process_id_dto_serializes_as_a_bare_number() {
        assert_eq!(json(&ProcessIdDto(42)), serde_json::json!(42));
    }

    #[test]
    fn process_terminal_state_dto_json_shapes() {
        assert_eq!(
            json(&ProcessTerminalStateDto {
                state: ProcessTerminalStateTag::Exited,
                code: Some(0),
            }),
            serde_json::json!({"state": "exited", "code": 0})
        );
        assert_eq!(
            json(&ProcessTerminalStateDto {
                state: ProcessTerminalStateTag::Killed,
                code: None,
            }),
            serde_json::json!({"state": "killed", "code": null})
        );
        assert_eq!(
            json(&ProcessTerminalStateDto {
                state: ProcessTerminalStateTag::OrphanRiskUncertain,
                code: None,
            }),
            serde_json::json!({"state": "orphan-risk/uncertain", "code": null}),
            "the orphan-risk/uncertain token must be transcribed with its slash intact"
        );
    }

    #[test]
    fn process_status_dto_json_shapes() {
        assert_eq!(
            json(&ProcessStatusDto::Running),
            serde_json::json!({"status": "running"})
        );
        assert_eq!(
            json(&ProcessStatusDto::Terminal {
                terminal: ProcessTerminalStateDto {
                    state: ProcessTerminalStateTag::Killed,
                    code: None,
                },
            }),
            serde_json::json!({"status": "terminal", "state": "killed", "code": null})
        );
    }

    #[test]
    fn harness_frame_stdout_json_shape() {
        let frame = HarnessFrame::Stdout {
            id: ProcessIdDto(7),
            seq: 3,
            dropped_before: 0,
            text: "line 1 out".to_string(),
        };
        assert_eq!(
            json(&frame),
            serde_json::json!({
                "stream": "stdout",
                "body": {"id": 7, "seq": 3, "droppedBefore": 0, "text": "line 1 out"}
            })
        );
    }

    #[test]
    fn harness_frame_with_nonzero_dropped_before() {
        let frame = HarnessFrame::Stderr {
            id: ProcessIdDto(7),
            seq: 41,
            dropped_before: 12,
            text: "line 205 err".to_string(),
        };
        assert_eq!(
            json(&frame),
            serde_json::json!({
                "stream": "stderr",
                "body": {"id": 7, "seq": 41, "droppedBefore": 12, "text": "line 205 err"}
            })
        );
    }

    #[test]
    fn harness_frame_state_json_shape() {
        let frame = HarnessFrame::State {
            id: ProcessIdDto(7),
            seq: 13,
            dropped_before: 0,
            terminal: ProcessTerminalStateDto {
                state: ProcessTerminalStateTag::Exited,
                code: Some(0),
            },
        };
        assert_eq!(
            json(&frame),
            serde_json::json!({
                "stream": "state",
                "body": {"id": 7, "seq": 13, "droppedBefore": 0, "state": "exited", "code": 0}
            })
        );
    }

    #[test]
    fn too_many_processes_code_serializes_as_kebab_case() {
        let error = ShellError {
            code: ShellErrorCode::TooManyProcesses,
            message: "too many processes are already running".to_string(),
        };
        assert_eq!(
            json(&error),
            serde_json::json!({
                "code": "too-many-processes",
                "message": "too many processes are already running"
            })
        );
    }

    #[test]
    fn shell_error_json_shape() {
        let error = ShellError {
            code: ShellErrorCode::SpawnFailed,
            message: "failed to start the requested process".to_string(),
        };
        assert_eq!(
            json(&error),
            serde_json::json!({"code": "spawn-failed", "message": "failed to start the requested process"})
        );
    }
}
