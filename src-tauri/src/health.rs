//! Pure, Tauri-independent implementation behind the `shell_health` typed
//! IPC command (see `crate::shell_health`), so its payload shape is
//! unit-tested without spinning up a Tauri runtime or a `WebView`.

use serde::Serialize;

/// The `shell_health` command's response payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShellHealth {
    /// This shell crate's name, always `"omnifrons-shell"`.
    pub shell: String,
    /// This shell crate's version, always `env!("CARGO_PKG_VERSION")`.
    pub version: String,
    /// Process-supervision containment status.
    ///
    /// Always `"unproven"` today: containment is platform-specific and
    /// "must be proven in the planned desktop verification plan"
    /// (adr/0002-desktop-technology-stack.md § Process supervision) --
    /// Windows Job Object policy, Linux cgroup/process-group/watchdog
    /// behavior, and macOS process-group/watchdog behavior are all still
    /// unverified (VP-001). Reporting anything but `"unproven"` before
    /// that verification exists would assert a guarantee this shell
    /// cannot yet back up.
    pub containment: String,
}

/// Build the current [`ShellHealth`] snapshot.
#[must_use]
pub fn health() -> ShellHealth {
    ShellHealth {
        shell: "omnifrons-shell".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        containment: "unproven".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_reports_unproven_containment_and_crate_version() {
        let status = health();

        assert_eq!(status.shell, "omnifrons-shell");
        assert_eq!(status.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(status.containment, "unproven");
    }
}
