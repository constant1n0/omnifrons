//! Tauri 2 desktop shell for Omnifrons: the outer adapter at the renderer
//! boundary (adr/0002-desktop-technology-stack.md § Privilege and IPC
//! boundary; docs/target-architecture.md § Components and trust
//! boundaries: `untrusted renderer -> typed IPC -> application
//! coordinators`). Nothing depends on this crate
//! (docs/repository-layout.md § Crate map).

mod adapter_state;
mod executable_state;
mod health;
mod ipc;
mod outbox_state;
mod publication_state;
mod wrong_root_state;

use adapter_state::AdapterState;
use executable_state::ExecutableState;
use health::ShellHealth;
use ipc::catalog_repair::{catalog_repair, catalog_repair_preview};
use ipc::commands::{
    adapters_list, approvals_list, candidates_list, executable_approve, executable_pick_and_probe,
    executable_revoke, harness_observe, harness_spawn, harness_stop, outbox_status,
    workspace_current, workspace_pick,
};
use ipc::guidance::{
    guidance_apply, guidance_pin, guidance_preview, guidance_remove, guidance_restore,
    guidance_snapshots, guidance_status,
};
use ipc::publication::{artifact_approve, artifact_publish, publications_list};
use ipc::recovery::{recovery_approve, recovery_list};
use ipc::wrong_root::{misplaced_list, misplaced_remedy, wrongroot_scan, wrongroot_status};
use omnifrons_supervisor::TokioProcessSupervisor;
use outbox_state::OutboxState;
use publication_state::PublicationState;
use tauri::Manager as _;
use wrong_root_state::WrongRootState;

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
            // The publication surface's device paths -- the product work
            // area and the asset roots -- sit under the same directory
            // (HAP-001 D1, D8; `docs/spike-log.md` § Slice 5b) and are
            // re-checked at every use, never opened here.
            app.manage(PublicationState::under(&store_dir));
            app.manage(ExecutableState::open(store_dir)?);

            // The closed, built-in adapter catalog and the single active-
            // workspace slot (`docs/spike-log.md` § Slice 3).
            app.manage(AdapterState::new());

            // The outbox surface: policy store, run-subdirectory preparer,
            // inventory, and the bounded per-run table
            // (`docs/spike-log.md` § Slice 5).
            app.manage(OutboxState::new());

            // The wrong-root surface: the findings of the most recent
            // scan for the active workspace, recomputed and never
            // persisted (`docs/spike-log.md` § Slice 5d).
            app.manage(std::sync::Arc::new(WrongRootState::new()));
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
            workspace_pick,
            workspace_current,
            adapters_list,
            outbox_status,
            candidates_list,
            artifact_approve,
            artifact_publish,
            publications_list,
            guidance_status,
            guidance_preview,
            guidance_apply,
            guidance_remove,
            guidance_snapshots,
            guidance_pin,
            guidance_restore,
            wrongroot_status,
            wrongroot_scan,
            misplaced_list,
            misplaced_remedy,
            catalog_repair_preview,
            catalog_repair,
            recovery_list,
            recovery_approve,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Omnifrons Tauri application");
}
