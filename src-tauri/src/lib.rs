//! Tauri 2 desktop shell for Omnifrons: the outer adapter at the renderer
//! boundary (adr/0002-desktop-technology-stack.md § Privilege and IPC
//! boundary; docs/target-architecture.md § Components and trust
//! boundaries: `untrusted renderer -> typed IPC -> application
//! coordinators`). Nothing depends on this crate
//! (docs/repository-layout.md § Crate map).

mod health;

use health::ShellHealth;

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
/// to initialize).
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![shell_health])
        .run(tauri::generate_context!())
        .expect("error while running the Omnifrons Tauri application");
}
