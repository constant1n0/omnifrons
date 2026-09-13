use std::ffi::OsStr;
use std::time::{Duration, SystemTime};

#[derive(Debug, PartialEq, Eq)]
pub struct StagingBasename<'a> {
    pub publication_hex: &'a str,
    pub pid: u32,
    pub sequence: u64,
}

/// The minimum age required before a staging entry may be considered.
pub const MIN_STAGING_AGE: Duration = Duration::from_hours(24);
/// The maximum number of directory entries a future cleanup sweep may inspect.
pub const MAX_INSPECTED_ENTRIES: usize = 256;
/// The maximum number of entries a future cleanup sweep may remove.
pub const MAX_REMOVED_ENTRIES: usize = 32;

/// Path-free aggregate results for one cleanup attempt.
#[derive(Debug, PartialEq, Eq)]
pub struct CleanupReport {
    pub inspected: usize,
    pub removed: usize,
    pub truncated: bool,
    pub supported: bool,
    pub failures: usize,
}

impl CleanupReport {
    /// A report for an inert or unsupported cleanup operation.
    #[must_use]
    pub const fn unsupported() -> Self {
        Self {
            inspected: 0,
            removed: 0,
            truncated: false,
            supported: false,
            failures: 0,
        }
    }
}

impl Default for CleanupReport {
    fn default() -> Self {
        Self {
            supported: cfg!(unix),
            ..Self::unsupported()
        }
    }
}

/// Parse only a canonical local-dir staging basename.
#[must_use]
pub fn parse_staging_basename(name: &OsStr) -> Option<StagingBasename<'_>> {
    let name = name.to_str()?;
    let body = name.strip_prefix('.')?;
    let (publication_hex, process_and_sequence) = body.split_once('.')?;
    let (pid, sequence_and_suffix) = process_and_sequence.split_once('-')?;
    let sequence = sequence_and_suffix.strip_suffix(".part")?;

    if !is_lowercase_publication_hex(publication_hex) {
        return None;
    }

    Some(StagingBasename {
        publication_hex,
        pid: parse_canonical_decimal(pid, 1)?,
        sequence: parse_canonical_decimal(sequence, 0)?,
    })
}

fn is_lowercase_publication_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn parse_canonical_decimal<T>(value: &str, minimum: u8) -> Option<T>
where
    T: std::str::FromStr,
{
    if value.is_empty() || (value.len() > 1 && value.starts_with('0')) {
        return None;
    }
    if value.bytes().any(|byte| !byte.is_ascii_digit()) || value == "0" && minimum != 0 {
        return None;
    }
    value.parse().ok()
}

/// Whether metadata age passes the strict staging retention boundary.
#[must_use]
pub fn is_old_enough(now: SystemTime, modified_at: SystemTime) -> bool {
    now.duration_since(modified_at)
        .is_ok_and(|age| age > MIN_STAGING_AGE)
}

/// A conservative creator-process liveness result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PidState {
    Live,
    NotLive,
    Unknown,
}

/// The result of a platform liveness probe, exposed for deterministic tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PidProbeOutcome {
    Success,
    PermissionDenied,
    Missing,
    Other,
}

/// Apply conservative liveness semantics after validating the probe input.
#[must_use]
pub fn pid_state_with(pid: u32, probe: impl FnOnce(i32) -> PidProbeOutcome) -> PidState {
    let Some(pid) = checked_probe_pid(pid) else {
        return PidState::Unknown;
    };

    match probe(pid) {
        PidProbeOutcome::Success | PidProbeOutcome::PermissionDenied => PidState::Live,
        PidProbeOutcome::Missing => PidState::NotLive,
        PidProbeOutcome::Other => PidState::Unknown,
    }
}

fn checked_probe_pid(pid: u32) -> Option<i32> {
    i32::try_from(pid).ok().filter(|pid| *pid > 0)
}

/// Probe a process without sending it a signal.
#[cfg(unix)]
#[must_use]
pub fn pid_state(pid: u32) -> PidState {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    let Some(raw) = nix::libc::pid_t::try_from(pid).ok().filter(|raw| *raw > 0) else {
        return PidState::Unknown;
    };

    pid_state_with(pid, |_| match kill(Pid::from_raw(raw), None) {
        Ok(()) => PidProbeOutcome::Success,
        Err(Errno::EPERM) => PidProbeOutcome::PermissionDenied,
        Err(Errno::ESRCH) => PidProbeOutcome::Missing,
        Err(_) => PidProbeOutcome::Other,
    })
}

/// Unsupported platforms have no process evidence and retain candidates.
#[cfg(not(unix))]
#[must_use]
pub fn pid_state(_pid: u32) -> PidState {
    PidState::Unknown
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::time::{Duration, SystemTime};

    use super::{
        CleanupReport, MIN_STAGING_AGE, PidProbeOutcome, PidState, is_old_enough,
        parse_staging_basename, pid_state_with,
    };

    const PUBLICATION_HEX: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn parses_only_the_canonical_local_staging_basename() {
        let valid = format!(".{PUBLICATION_HEX}.42-7.part");
        let parsed = parse_staging_basename(OsStr::new(&valid)).expect("canonical basename");

        assert_eq!(parsed.pid, 42);
        assert_eq!(parsed.sequence, 7);
        assert_eq!(parsed.publication_hex, PUBLICATION_HEX);
    }

    #[test]
    fn retains_malformed_and_foreign_part_names() {
        let cases = [
            format!(".{PUBLICATION_HEX}.0-7.part"),
            format!(".{PUBLICATION_HEX}.042-7.part"),
            format!(".{PUBLICATION_HEX}.42-007.part"),
            format!(".{PUBLICATION_HEX}.42-7.part.extra"),
            format!(".{PUBLICATION_HEX}.42--7.part"),
            format!(".{PUBLICATION_HEX}.42-7.tmp"),
            format!(".{}.42-7.part", PUBLICATION_HEX.to_uppercase()),
            format!(".{}g.42-7.part", &PUBLICATION_HEX[..63]),
            format!(".{}.42-7.part", &PUBLICATION_HEX[..63]),
            format!(".{PUBLICATION_HEX}.4294967296-7.part"),
            format!(".{PUBLICATION_HEX}.42-18446744073709551616.part"),
            format!("{PUBLICATION_HEX}.part"),
            format!(".{PUBLICATION_HEX}.42-7.part.final"),
            ".outbox.42-7.part".to_string(),
            ".provider-upload.42-7.part".to_string(),
            PUBLICATION_HEX.to_string(),
        ];

        for name in cases {
            assert!(
                parse_staging_basename(OsStr::new(&name)).is_none(),
                "{name} must remain outside local staging cleanup"
            );
        }
    }

    #[test]
    fn parses_canonical_numeric_bounds() {
        let name = format!(".{PUBLICATION_HEX}.4294967295-18446744073709551615.part");
        let parsed = parse_staging_basename(OsStr::new(&name)).expect("maximum bounds");

        assert_eq!(parsed.pid, u32::MAX);
        assert_eq!(parsed.sequence, u64::MAX);
    }

    #[cfg(unix)]
    #[test]
    fn retains_non_utf8_basenames() {
        use std::os::unix::ffi::OsStrExt as _;

        assert!(parse_staging_basename(OsStr::from_bytes(b".\xff.part")).is_none());
    }

    #[test]
    fn requires_an_age_strictly_greater_than_twenty_four_hours() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);

        assert!(is_old_enough(
            now,
            now - MIN_STAGING_AGE - Duration::from_nanos(1)
        ));
        assert!(!is_old_enough(now, now - MIN_STAGING_AGE));
        assert!(!is_old_enough(now, now + Duration::from_secs(1)));
    }

    #[test]
    fn starts_with_a_path_free_empty_cleanup_report() {
        assert_eq!(
            CleanupReport::default(),
            CleanupReport {
                inspected: 0,
                removed: 0,
                truncated: false,
                supported: cfg!(unix),
                failures: 0,
            }
        );
    }

    #[test]
    fn invalid_pid_values_are_unknown_without_calling_the_probe() {
        for pid in [0, u32::try_from(i32::MAX).expect("i32 maximum") + 1] {
            let mut called = false;
            let state = pid_state_with(pid, |_| {
                called = true;
                PidProbeOutcome::Success
            });

            assert_eq!(state, PidState::Unknown);
            assert!(!called, "an invalid pid must not reach the probe");
        }
    }

    #[test]
    fn injected_probe_maps_only_missing_processes_to_not_live() {
        assert_eq!(
            pid_state_with(1, |_| PidProbeOutcome::Success),
            PidState::Live
        );
        assert_eq!(
            pid_state_with(1, |_| PidProbeOutcome::PermissionDenied),
            PidState::Live
        );
        assert_eq!(
            pid_state_with(1, |_| PidProbeOutcome::Missing),
            PidState::NotLive
        );
        assert_eq!(
            pid_state_with(1, |_| PidProbeOutcome::Other),
            PidState::Unknown
        );
    }

    #[cfg(unix)]
    #[test]
    fn probes_an_owned_child_as_live_then_not_live() {
        use std::process::{Child, Command};

        struct ChildGuard(Child);

        impl ChildGuard {
            fn id(&self) -> u32 {
                self.0.id()
            }

            fn stop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        impl Drop for ChildGuard {
            fn drop(&mut self) {
                self.stop();
            }
        }

        let mut child = ChildGuard(
            Command::new("sleep")
                .arg("30")
                .spawn()
                .expect("owned child process"),
        );
        let pid = child.id();

        assert_eq!(super::pid_state(pid), PidState::Live);
        child.stop();
        assert_eq!(super::pid_state(pid), PidState::NotLive);
    }

    #[cfg(windows)]
    #[test]
    fn windows_liveness_is_unknown() {
        assert_eq!(super::pid_state(1), PidState::Unknown);
    }
}
