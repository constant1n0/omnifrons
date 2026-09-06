//! Tauri 2 desktop shell for Omnifrons: the outer adapter at the renderer
//! boundary (adr/0002-desktop-technology-stack.md § Privilege and IPC
//! boundary; docs/target-architecture.md § Components and trust
//! boundaries: `untrusted renderer -> typed IPC -> application
//! coordinators`). Nothing depends on this crate
//! (docs/repository-layout.md § Crate map).

mod executable_state;
mod health;
mod ipc;

use executable_state::ExecutableState;
use health::ShellHealth;
use ipc::commands::{
    approvals_list, executable_approve, executable_pick_and_probe, executable_revoke,
    harness_observe, harness_spawn, harness_stop,
};
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
/// to initialize), if `tauri::process::current_binary` cannot resolve this
/// process's own path (the demo harness launcher), if this platform's
/// application-local data directory cannot be resolved, or if the
/// approval store under it could not be opened.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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

            // The approval store's path is logged at debug only -- it is
            // never itself IPC output, and debug-level logs are not the
            // renderer-facing surface `docs/spike-log.md` § IPC contract's
            // no-path-leak rule governs, but there is still no reason to
            // print it any louder than that.
            let store_dir = app.path().app_local_data_dir()?;
            tracing::debug!(store_dir = %store_dir.display(), "opening the approval store");
            app.manage(ExecutableState::open(store_dir)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            shell_health,
            harness_spawn,
            harness_stop,
            harness_observe,
            executable_pick_and_probe,
            executable_approve,
            executable_revoke,
            approvals_list,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Omnifrons Tauri application");
}
