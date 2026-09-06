//! Tauri 2 desktop shell for Omnifrons: the outer adapter at the renderer
//! boundary (adr/0002-desktop-technology-stack.md § Privilege and IPC
//! boundary; docs/target-architecture.md § Components and trust
//! boundaries: `untrusted renderer -> typed IPC -> application
//! coordinators`). Nothing depends on this crate
//! (docs/repository-layout.md § Crate map).

mod health;
mod ipc;

use health::ShellHealth;
use ipc::commands::{harness_observe, harness_spawn, harness_stop};
use omnifrons_supervisor::TokioProcessSupervisor;
use tauri::Manager as _;

/// Typed IPC command: the renderer's only way to read this shell's
/// name, version, and process-supervision containment status.
///
/// A thin wrapper over the pure [`health::health`] function, kept
/// deliberately free of any Tauri-specific logic so the payload shape is
/// unit-tested without a Tauri runtime.
#[tauri::command]
fn shell_health() -> ShellHealth {
    health::health()
}

/// Build and run the Tauri application.
///
/// `#[cfg_attr(mobile, tauri::mobile_entry_point)]` is inert on desktop
/// builds (this crate declares no mobile crate-type); it is kept only to
/// match Tauri's standard scaffold shape.
///
/// # Panics
///
/// Panics if the Tauri application fails to build or exits with an error
/// (for example, an invalid `tauri.conf.json` or a `WebView` that failed
/// to initialize), or if `tauri::process::current_binary` cannot resolve
/// this process's own path (the demo harness launcher).
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // No program path or argument vector for a harness request
            // ever crosses IPC (docs/spike-log.md § IPC contract): this is
            // the one place a real path lives, resolved once here and
            // fixed for the supervisor's whole lifetime, not carried by
            // any command argument.
            //
            // Managed directly, with no outer `Mutex`: `TokioProcessSupervisor`
            // is itself a cheap `Clone` handle over shared state, and every
            // command clones it rather than locking a single shared value
            // for its whole call -- wrapping it in a `Mutex` here would
            // serialize every command behind whichever one happened to be
            // running a multi-second `stop` (`ipc::commands::with_supervisor`).
            let launcher = tauri::process::current_binary(&app.env())?;
            app.manage(TokioProcessSupervisor::with_demo_launcher(launcher));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            shell_health,
            harness_spawn,
            harness_stop,
            harness_observe,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Omnifrons Tauri application");
}
