//! Tauri 2 desktop shell for Omnifrons: the outer adapter at the renderer
//! boundary (adr/0002-desktop-technology-stack.md § Privilege and IPC
//! boundary; docs/target-architecture.md § Components and trust
//! boundaries: `untrusted renderer -> typed IPC -> application
//! coordinators`). Nothing depends on this crate
//! (docs/repository-layout.md § Crate map).

mod adapter_state;
mod executable_state;
mod harness_log;
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
use ipc::definition::{
    definition_apply, definition_create_apply, definition_create_fields, definition_create_preview,
    definition_hosts, definition_preview, definition_status, definition_switch_check,
    definition_verify,
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
    install_panic_hook();
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
            // The per-run harness logs sit under the same directory
            // (`crate::harness_log`); a failure to create it is reported per
            // run, never fatal.
            app.manage(harness_log::HarnessLogs(harness_log::HarnessLogDir::open(
                &store_dir,
            )));
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
            definition_status,
            definition_verify,
            definition_preview,
            definition_apply,
            definition_create_fields,
            definition_create_preview,
            definition_create_apply,
            definition_switch_check,
            definition_hosts,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Omnifrons Tauri application");
}

/// Installs a process-wide panic hook that logs every panic through
/// `tracing` (thread, location, message) and then calls the previous hook,
/// so the default stderr report stays. Tauri never settles the promise of
/// an async command whose future panics, so without this a panic inside a
/// command left no trace at all (#185).
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        guarded_panic_report(
            || {
                let thread = std::thread::current();
                let panic_message = info
                    .payload()
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("non-string panic payload");
                tracing::error!(
                    thread = thread.name().unwrap_or("unnamed"),
                    location = %info
                        .location()
                        .map_or_else(|| "unknown".to_string(), ToString::to_string),
                    panic_message,
                    "panic"
                );
            },
            || previous(info),
        );
    }));
}

thread_local! {
    /// Set while this thread is inside the panic hook's logging.
    static IN_PANIC_HOOK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `log` and then `previous`, so a failing logger never replaces the
/// panic being reported: a panic inside `log` (a subscriber that panics) is
/// caught, and a panic raised while this thread is already inside the hook
/// skips `log` instead of recursing. A panic escaping a hook aborts the
/// process, which would turn one task's panic into a crash.
fn guarded_panic_report(log: impl FnOnce(), previous: impl FnOnce()) {
    let nested = IN_PANIC_HOOK.with(|flag| flag.replace(true));
    if !nested {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(log));
        IN_PANIC_HOOK.with(|flag| flag.set(false));
    }
    previous();
}

#[cfg(test)]
mod tests {
    use super::guarded_panic_report;
    use std::cell::Cell;

    /// A logger that panics must not stop the previous hook or unwind out.
    #[test]
    fn a_panicking_logger_still_runs_the_previous_hook() {
        let previous_ran = Cell::new(false);
        let logger_panic: fn() = || std::panic::resume_unwind(Box::new("subscriber failed"));
        guarded_panic_report(logger_panic, || previous_ran.set(true));
        assert!(previous_ran.get());
    }

    /// A panic raised inside the logging skips the nested log and still
    /// reaches the previous hook, without recursing.
    #[test]
    fn a_nested_report_skips_the_log_and_runs_the_previous_hook() {
        let nested_logged = Cell::new(false);
        let nested_previous = Cell::new(false);
        guarded_panic_report(
            || {
                guarded_panic_report(|| nested_logged.set(true), || nested_previous.set(true));
            },
            || {},
        );
        assert!(!nested_logged.get());
        assert!(nested_previous.get());
    }

    /// The flag is cleared afterwards: a later report logs again.
    #[test]
    fn the_guard_is_released_after_each_report() {
        let logged = Cell::new(0);
        guarded_panic_report(|| logged.set(logged.get() + 1), || {});
        guarded_panic_report(|| logged.set(logged.get() + 1), || {});
        assert_eq!(logged.get(), 2);
    }
}
