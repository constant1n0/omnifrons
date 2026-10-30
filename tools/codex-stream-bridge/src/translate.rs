//! Pure line translator from `codex exec --json` JSONL to the adapter's
//! `stream-json` shapes.

use serde_json::{Value, json};

/// One thing the bridge must do in response to an input line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    /// One line for stdout (no trailing newline).
    Stdout(String),
    /// One diagnostic line for stderr (no trailing newline).
    Stderr(String),
    /// The turn is over: stop reading and exit with this code.
    Finish { exit: i32 },
}

/// Translator state carried across lines.
#[derive(Debug, Default)]
pub struct State {
    pub cwd: String,
    pub codex_version: Option<String>,
    pub reasoning: bool,
    init_done: bool,
    model: Option<String>,
}

impl State {
    #[must_use]
    pub fn new(cwd: String, codex_version: Option<String>, reasoning: bool) -> Self {
        Self {
            cwd,
            codex_version,
            reasoning,
            ..Self::default()
        }
    }
}

/// Items that are surfaced to the adapter as a `tool_use` block named after
/// the item type, with the item's own JSON object as `input`.
const TOOL_ITEMS: &[&str] = &[
    "command_execution",
    "mcp_tool_call",
    "file_change",
    "web_search",
    "collab_tool_call",
    "todo_list",
];

/// Translate one Codex JSONL line into the actions the bridge must take.
pub fn translate(line: &str, state: &mut State) -> Vec<Out> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return vec![diag(&format!("unparseable line: {}", clip(line)))];
    };
    if state.model.is_none() {
        state.model = find_model(&value);
    }
    let Some(kind) = value.get("type").and_then(Value::as_str) else {
        return vec![diag(&format!("event without a type: {}", clip(line)))];
    };
    let mut outs = Vec::new();
    match kind {
        "thread.started" => {
            let id = value.get("thread_id").and_then(Value::as_str);
            ensure_init(state, id, &mut outs);
        }
        "turn.started" | "item.started" | "item.updated" => {}
        "item.completed" => {
            if let Some(item) = value.get("item") {
                translate_item(item, state, &mut outs);
            } else {
                outs.push(diag(&format!(
                    "item.completed without item: {}",
                    clip(line)
                )));
            }
        }
        "turn.completed" => {
            ensure_init(state, None, &mut outs);
            outs.push(stdout(&json!({"type": "result", "subtype": "success"})));
            outs.push(Out::Finish { exit: 0 });
        }
        // `turn.failed` and the top-level `error` are both fatal in Codex's
        // event schema; a non-fatal error arrives as an `error` item instead.
        "turn.failed" | "error" => {
            ensure_init(state, None, &mut outs);
            outs.push(diag(&format!(
                "codex reported {kind}: {}",
                clip(&error_message(&value))
            )));
            outs.push(stdout(&json!({"type": "result", "subtype": "error"})));
            outs.push(Out::Finish { exit: 1 });
        }
        other => outs.push(diag(&format!("unknown event type {other}: {}", clip(line)))),
    }
    outs
}

/// Called when Codex's stdout ends without a terminal event.
pub fn end_of_input(state: &mut State) -> Vec<Out> {
    let mut outs = vec![diag("codex ended without a terminal event")];
    outs.extend(unknown_result(state));
    outs
}

/// Called when Codex could not be started: `init` (generated session id),
/// then `result/unknown`. The caller reports the spawn error itself.
pub fn spawn_failed(state: &mut State) -> Vec<Out> {
    unknown_result(state)
}

fn unknown_result(state: &mut State) -> Vec<Out> {
    let mut outs = Vec::new();
    ensure_init(state, None, &mut outs);
    outs.push(stdout(&json!({"type": "result", "subtype": "unknown"})));
    outs.push(Out::Finish { exit: 1 });
    outs
}

fn translate_item(item: &Value, state: &mut State, outs: &mut Vec<Out>) {
    let ty = item.get("type").and_then(Value::as_str).unwrap_or("");
    let text = item.get("text").and_then(Value::as_str);
    match (ty, text) {
        ("agent_message", Some(text)) => {
            ensure_init(state, None, outs);
            outs.push(assistant(&json!({"type": "text", "text": text})));
        }
        ("reasoning", Some(text)) => {
            if state.reasoning {
                ensure_init(state, None, outs);
                outs.push(assistant(&json!({"type": "text", "text": text})));
            }
        }
        ("error", _) => outs.push(diag(&format!(
            "codex non-fatal error item: {}",
            clip(&error_message(item))
        ))),
        (ty, _) if TOOL_ITEMS.contains(&ty) => {
            ensure_init(state, None, outs);
            outs.push(assistant(
                &json!({"type": "tool_use", "name": ty, "input": item}),
            ));
        }
        _ => outs.push(diag(&format!(
            "unhandled item {ty:?}: {}",
            clip(&item.to_string())
        ))),
    }
}

/// Emit the `init` line (and the retained bridge-version line) exactly once.
fn ensure_init(state: &mut State, session_id: Option<&str>, outs: &mut Vec<Out>) {
    if state.init_done {
        return;
    }
    state.init_done = true;
    let session = session_id.map_or_else(
        || {
            outs.push(diag(
                "no thread id before the first item; generated a session id",
            ));
            generated_session_id()
        },
        str::to_string,
    );
    let model = state.model.clone().unwrap_or_else(|| "codex".to_string());
    outs.push(stdout(&json!({
        "type": "system", "subtype": "init",
        "cwd": state.cwd, "model": model, "session_id": session,
    })));
    if let Some(version) = &state.codex_version {
        // The adapter's parser keeps a `system` line whose subtype is not
        // `init` as an unknown line, verbatim, so this is retained evidence.
        outs.push(stdout(&json!({
            "type": "system", "subtype": "bridge",
            "codex_version": version, "bridge_version": env!("CARGO_PKG_VERSION"),
        })));
    }
}

fn generated_session_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("codex-bridge-{}-{nanos}", std::process::id())
}

fn find_model(value: &Value) -> Option<String> {
    value
        .get("model")
        .or_else(|| value.get("item").and_then(|i| i.get("model")))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn error_message(value: &Value) -> String {
    value
        .get("message")
        .or_else(|| value.get("error").and_then(|e| e.get("message")))
        .and_then(Value::as_str)
        .unwrap_or("(no message)")
        .to_string()
}

fn assistant(block: &Value) -> Out {
    stdout(&json!({"type": "assistant", "message": {"content": [block]}}))
}

fn stdout(value: &Value) -> Out {
    Out::Stdout(value.to_string())
}

fn diag(text: &str) -> Out {
    Out::Stderr(format!("codex-stream-bridge: {text}"))
}

/// Bound a diagnostic and keep it on one physical line.
fn clip(text: &str) -> String {
    let clipped: String = text.chars().take(200).collect();
    clipped.escape_debug().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        State::new(
            "/work".to_string(),
            Some("codex-cli 0.163.0".to_string()),
            false,
        )
    }

    fn stdout_values(outs: &[Out]) -> Vec<Value> {
        outs.iter()
            .filter_map(|o| match o {
                Out::Stdout(s) => Some(serde_json::from_str(s).expect("valid json")),
                _ => None,
            })
            .collect()
    }

    const THREAD: &str = r#"{"type":"thread.started","thread_id":"0199a213-81c0"}"#;

    #[test]
    fn thread_started_emits_init_then_version_line_once() {
        let mut st = state();
        let outs = translate(THREAD, &mut st);
        let v = stdout_values(&outs);
        assert_eq!(
            v[0],
            json!({"type":"system","subtype":"init","cwd":"/work","model":"codex","session_id":"0199a213-81c0"})
        );
        assert_eq!(
            v[1],
            json!({"type":"system","subtype":"bridge","codex_version":"codex-cli 0.163.0","bridge_version":env!("CARGO_PKG_VERSION")})
        );
        assert_eq!(v.len(), 2);
        assert!(stdout_values(&translate(THREAD, &mut st)).is_empty());
    }

    #[test]
    fn agent_message_becomes_text_block() {
        let mut st = state();
        translate(THREAD, &mut st);
        let line = r#"{"type":"item.completed","item":{"id":"item_3","type":"agent_message","text":"hello"}}"#;
        let v = stdout_values(&translate(line, &mut st));
        assert_eq!(
            v,
            vec![
                json!({"type":"assistant","message":{"content":[{"type":"text","text":"hello"}]}})
            ]
        );
    }

    #[test]
    fn tool_like_items_become_tool_use_with_item_as_input() {
        for ty in [
            "collab_tool_call",
            "todo_list",
            "command_execution",
            "mcp_tool_call",
            "file_change",
            "web_search",
        ] {
            let mut st = state();
            translate(THREAD, &mut st);
            let item = json!({"id":"item_1","type":ty,"status":"completed"});
            let line = json!({"type":"item.completed","item":item}).to_string();
            let v = stdout_values(&translate(&line, &mut st));
            assert_eq!(
                v,
                vec![
                    json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":ty,"input":item}]}})
                ],
                "{ty}"
            );
        }
    }

    #[test]
    fn item_started_is_silent() {
        let mut st = state();
        translate(THREAD, &mut st);
        let line = r#"{"type":"item.started","item":{"id":"item_1","type":"command_execution","command":"ls","status":"in_progress"}}"#;
        assert!(translate(line, &mut st).is_empty());
        assert!(translate(r#"{"type":"turn.started"}"#, &mut st).is_empty());
    }

    #[test]
    fn reasoning_skipped_unless_enabled() {
        let line =
            r#"{"type":"item.completed","item":{"id":"i","type":"reasoning","text":"thinking"}}"#;
        let mut st = state();
        translate(THREAD, &mut st);
        assert!(translate(line, &mut st).is_empty());
        let mut on = State::new("/work".into(), None, true);
        translate(THREAD, &mut on);
        let v = stdout_values(&translate(line, &mut on));
        assert_eq!(
            v,
            vec![
                json!({"type":"assistant","message":{"content":[{"type":"text","text":"thinking"}]}})
            ]
        );
    }

    #[test]
    fn turn_completed_finishes_with_success() {
        let mut st = state();
        translate(THREAD, &mut st);
        let outs = translate(
            r#"{"type":"turn.completed","usage":{"input_tokens":1}}"#,
            &mut st,
        );
        assert_eq!(
            stdout_values(&outs),
            vec![json!({"type":"result","subtype":"success"})]
        );
        assert_eq!(outs.last(), Some(&Out::Finish { exit: 0 }));
    }

    #[test]
    fn turn_failed_finishes_with_error() {
        let mut st = state();
        translate(THREAD, &mut st);
        let outs = translate(
            r#"{"type":"turn.failed","error":{"message":"boom"}}"#,
            &mut st,
        );
        assert_eq!(
            stdout_values(&outs),
            vec![json!({"type":"result","subtype":"error"})]
        );
        assert_eq!(outs.last(), Some(&Out::Finish { exit: 1 }));
        assert!(
            outs.iter()
                .any(|o| matches!(o, Out::Stderr(s) if s.contains("boom")))
        );
    }

    #[test]
    fn a_fatal_error_event_ends_the_run_with_an_error_result() {
        let mut st = state();
        translate(THREAD, &mut st);
        let outs = translate(r#"{"type":"error","message":"fatal"}"#, &mut st);
        assert_eq!(
            stdout_values(&outs),
            vec![json!({"type":"result","subtype":"error"})]
        );
        assert_eq!(outs.last(), Some(&Out::Finish { exit: 1 }));
        assert!(
            outs.iter()
                .any(|o| matches!(o, Out::Stderr(s) if s.contains("fatal")))
        );
    }

    #[test]
    fn an_error_item_is_a_diagnostic_and_the_run_continues() {
        let mut st = state();
        translate(THREAD, &mut st);
        let line =
            r#"{"type":"item.completed","item":{"id":"i","type":"error","message":"soft\nfail"}}"#;
        let outs = translate(line, &mut st);
        assert_eq!(outs.len(), 1);
        assert!(matches!(&outs[0], Out::Stderr(s) if s.contains("soft") && !s.contains('\n')));
        let next =
            r#"{"type":"item.completed","item":{"id":"i","type":"agent_message","text":"x"}}"#;
        assert_eq!(stdout_values(&translate(next, &mut st)).len(), 1);
    }

    #[test]
    fn error_messages_are_clipped_to_one_line() {
        for line in [
            json!({"type":"turn.failed","error":{"message":"a\nb"}}).to_string(),
            json!({"type":"error","message":"a\nb"}).to_string(),
        ] {
            let mut st = state();
            translate(THREAD, &mut st);
            for o in translate(&line, &mut st) {
                if let Out::Stderr(s) = o {
                    assert!(!s.contains('\n'));
                    assert!(s.contains("a\\nb"));
                }
            }
        }
    }

    #[test]
    fn unknown_events_go_to_stderr_only() {
        let mut st = state();
        translate(THREAD, &mut st);
        for line in [
            r#"{"type":"something.new","x":1}"#,
            r#"{"type":"item.completed","item":{"id":"i","type":"brand_new_item"}}"#,
            r#"{"no_type":true}"#,
            "not json at all",
            "",
        ] {
            let outs = translate(line, &mut st);
            assert_eq!(outs.len(), 1, "{line}");
            assert!(
                matches!(&outs[0], Out::Stderr(s) if !s.contains('\n')),
                "{line}"
            );
        }
    }

    #[test]
    fn item_before_thread_started_emits_generated_init_first() {
        let mut st = state();
        let line =
            r#"{"type":"item.completed","item":{"id":"i","type":"agent_message","text":"x"}}"#;
        let outs = translate(line, &mut st);
        let v = stdout_values(&outs);
        assert_eq!(v[0]["subtype"], "init");
        let sid = v[0]["session_id"].as_str().unwrap();
        assert!(sid.starts_with("codex-bridge-"));
        assert_eq!(v.last().unwrap()["type"], "assistant");
        assert!(outs.iter().any(|o| matches!(o, Out::Stderr(_))));
    }

    #[test]
    fn model_is_taken_from_an_event_that_carries_it() {
        let mut st = state();
        translate(r#"{"type":"turn.started","model":"gpt-x"}"#, &mut st);
        let v = stdout_values(&translate(THREAD, &mut st));
        assert_eq!(v[0]["model"], "gpt-x");
    }

    #[test]
    fn end_of_input_without_terminal_event_is_unknown_result() {
        let mut st = state();
        translate(THREAD, &mut st);
        let outs = end_of_input(&mut st);
        assert_eq!(
            stdout_values(&outs),
            vec![json!({"type":"result","subtype":"unknown"})]
        );
        assert_eq!(outs.last(), Some(&Out::Finish { exit: 1 }));
    }

    #[test]
    fn multiline_text_stays_one_physical_line() {
        let mut st = state();
        translate(THREAD, &mut st);
        let line = json!({"type":"item.completed","item":{"id":"i","type":"agent_message","text":"a\nb\r\nc"}}).to_string();
        let outs = translate(&line, &mut st);
        for o in &outs {
            if let Out::Stdout(s) = o {
                assert!(!s.contains('\n') && !s.contains('\r'));
            }
        }
        assert_eq!(
            stdout_values(&outs)[0]["message"]["content"][0]["text"],
            "a\nb\r\nc"
        );
    }
}
