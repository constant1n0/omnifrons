// Prevents an extra console window from opening on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Hidden argv branch, compiled only under the non-default `demo-harness`
/// cargo feature: `omnifrons-shell --demo-harness <kind> <rate_hz>
/// <lines>` runs the demo harness in-process instead of the Tauri
/// application, so `TokioProcessSupervisor::with_demo_launcher` can exec
/// this very binary (`tauri::process::current_binary`) as the harness
/// launcher. Parsed here, before `tauri::Builder` ever runs.
///
/// Without the `demo-harness` feature, this function -- and the literal
/// `--demo-harness` string -- do not exist in the compiled binary at all
/// (`tests/feature_gate.rs`).
#[cfg(feature = "demo-harness")]
fn try_run_demo_harness() -> Option<std::process::ExitCode> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--demo-harness") {
        return None;
    }
    let kind_arg = args
        .next()
        .expect("--demo-harness requires a kind argument");
    let kind = omnifrons_supervisor::demo::parse_kind(&kind_arg)
        .unwrap_or_else(|| panic!("unknown demo harness kind: {kind_arg}"));
    let rate_hz: u16 = args
        .next()
        .and_then(|value| value.parse().ok())
        .expect("--demo-harness requires a valid rate_hz argument");
    let lines: u32 = args
        .next()
        .and_then(|value| value.parse().ok())
        .expect("--demo-harness requires a valid lines argument");
    Some(omnifrons_supervisor::demo::run(kind, rate_hz, lines))
}

fn main() -> std::process::ExitCode {
    #[cfg(feature = "demo-harness")]
    if let Some(exit_code) = try_run_demo_harness() {
        return exit_code;
    }

    omnifrons_shell::run();
    std::process::ExitCode::SUCCESS
}
