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
#[cfg_attr(not(unix), derive(Default))]
#[derive(Debug, PartialEq, Eq)]
pub struct CleanupReport {
    pub inspected: usize,
    pub retained: usize,
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
            retained: 0,
            removed: 0,
            truncated: false,
            supported: false,
            failures: 0,
        }
    }
}

#[cfg(unix)]
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

/// Initial handle-derived evidence for one local staging candidate.
///
/// This is deliberately not a deletion authority. The future bounded walker
/// must still revalidate the held root and candidate immediately before any
/// mutation.
#[cfg(unix)]
// Batch 2a establishes evidence before the bounded walker consumes it.
#[allow(dead_code)]
#[derive(Debug)]
struct StagingRootEvidence {
    handle: std::fs::File,
    dev: u64,
    ino: u64,
    uid: u32,
    mode: u32,
}

#[cfg(unix)]
// Batch 2a establishes evidence before the bounded walker consumes it.
#[allow(dead_code)]
#[derive(Debug)]
struct StagingCandidateEvidence {
    root: StagingRootEvidence,
    handle: std::fs::File,
    dev: u64,
    ino: u64,
    uid: u32,
    mode: u32,
    link_count: u64,
    size: u64,
    modified_at: SystemTime,
}

#[cfg(unix)]
#[derive(Debug)]
struct StagingCandidateFacts {
    pid: u32,
    handle: std::fs::File,
    dev: u64,
    ino: u64,
    uid: u32,
    mode: u32,
    link_count: u64,
    size: u64,
    modified_at: SystemTime,
}

/// Open a root and one exact candidate without following links or reading data.
///
/// Every failure is retained by returning `None`. This read-only primitive
/// intentionally does not enumerate, delete, rename, or invoke staging.
#[cfg(unix)]
// Batch 2a establishes evidence before the bounded walker consumes it.
#[allow(dead_code)]
fn open_staging_evidence(
    root_path: &std::path::Path,
    name: &OsStr,
) -> Option<StagingCandidateEvidence> {
    let root = admit_staging_root(root_path)?;
    let candidate = open_staging_candidate(&root, name).ok()??;
    Some(StagingCandidateEvidence {
        root,
        handle: candidate.handle,
        dev: candidate.dev,
        ino: candidate.ino,
        uid: candidate.uid,
        mode: candidate.mode,
        link_count: candidate.link_count,
        size: candidate.size,
        modified_at: candidate.modified_at,
    })
}

#[cfg(unix)]
fn admit_staging_root(root_path: &std::path::Path) -> Option<StagingRootEvidence> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    use nix::unistd::Uid;

    let root = crate::fs_run_outbox_preparer::open_directory_no_follow(root_path).ok()?;
    let root_metadata = root.metadata().ok()?;
    let owner = Uid::effective().as_raw();
    let root_mode = root_metadata.permissions().mode() & 0o777;
    if root_metadata.uid() != owner || root_mode != 0o700 {
        return None;
    }
    Some(StagingRootEvidence {
        dev: root_metadata.dev(),
        ino: root_metadata.ino(),
        uid: root_metadata.uid(),
        mode: root_mode,
        handle: root,
    })
}

#[cfg(unix)]
fn open_staging_candidate(
    root: &StagingRootEvidence,
    name: &OsStr,
) -> Result<Option<StagingCandidateFacts>, ()> {
    use std::fs::File;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    use nix::fcntl::{OFlag, openat};
    use nix::sys::stat::Mode;
    use nix::unistd::Uid;

    let Some(parsed) = parse_staging_basename(name) else {
        return Ok(None);
    };
    let owner = Uid::effective().as_raw();

    let candidate = openat(
        &root.handle,
        std::path::Path::new(name),
        OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| ())?;
    let metadata = candidate.metadata().map_err(|_| ())?;
    let mode = metadata.permissions().mode() & 0o777;
    if !metadata.file_type().is_file()
        || metadata.uid() != owner
        || mode != 0o600
        || metadata.nlink() != 1
    {
        return Ok(None);
    }

    Ok(Some(StagingCandidateFacts {
        pid: parsed.pid,
        handle: candidate,
        dev: metadata.dev(),
        ino: metadata.ino(),
        uid: metadata.uid(),
        mode,
        link_count: metadata.nlink(),
        size: metadata.size(),
        modified_at: metadata.modified().map_err(|_| ())?,
    }))
}

/// Recheck the admitted root and basename through their original handles.
///
/// This is deliberately non-mutating. A later deletion slice may use this
/// proof immediately before `unlinkat`, while still disclosing that another
/// same-user process can race the final lookup and unlink.
#[cfg(unix)]
#[allow(dead_code)] // Batch3a supplies proof; a later deletion slice consumes it before unlinkat.
fn final_identity_matches(
    root_path: &std::path::Path,
    name: &OsStr,
    evidence: &StagingCandidateEvidence,
) -> bool {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    use nix::fcntl::AtFlags;
    use nix::sys::stat::fstatat;

    let Ok(current_root) = std::fs::symlink_metadata(root_path) else {
        return false;
    };
    let root_mode = current_root.permissions().mode() & 0o777;
    if !current_root.file_type().is_dir()
        || current_root.dev() != evidence.root.dev
        || current_root.ino() != evidence.root.ino
        || current_root.uid() != evidence.root.uid
        || root_mode != evidence.root.mode
    {
        return false;
    }
    let Ok(held_root) = evidence.root.handle.metadata() else {
        return false;
    };
    if held_root.dev() != evidence.root.dev
        || held_root.ino() != evidence.root.ino
        || held_root.uid() != evidence.root.uid
        || (held_root.permissions().mode() & 0o777) != evidence.root.mode
    {
        return false;
    }
    let Ok(held_candidate) = evidence.handle.metadata() else {
        return false;
    };
    if !held_candidate.file_type().is_file()
        || held_candidate.dev() != evidence.dev
        || held_candidate.ino() != evidence.ino
        || held_candidate.uid() != evidence.uid
        || (held_candidate.permissions().mode() & 0o777) != evidence.mode
        || held_candidate.nlink() != evidence.link_count
        || held_candidate.len() != evidence.size
        || held_candidate.modified().ok() != Some(evidence.modified_at)
    {
        return false;
    }
    let Ok(current) = fstatat(
        &evidence.root.handle,
        std::path::Path::new(name),
        AtFlags::AT_SYMLINK_NOFOLLOW,
    ) else {
        return false;
    };
    let Some(seconds) = u64::try_from(current.st_mtime).ok() else {
        return false;
    };
    let Some(nanoseconds) = u64::try_from(current.st_mtime_nsec).ok() else {
        return false;
    };
    stat_field_matches(current.st_dev, evidence.dev)
        && stat_field_matches(current.st_ino, evidence.ino)
        && stat_field_matches(current.st_uid, u64::from(evidence.uid))
        && stat_field_matches(current.st_mode & 0o777, u64::from(evidence.mode))
        && stat_field_matches(current.st_nlink, evidence.link_count)
        && stat_field_matches(current.st_size, evidence.size)
        && SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(seconds))
            .and_then(|time| time.checked_add(Duration::from_nanos(nanoseconds)))
            == Some(evidence.modified_at)
}

/// Fallibly normalize a native stat field before comparison with captured evidence.
#[allow(dead_code)] // Batch3a supplies proof; a later deletion slice consumes it before unlinkat.
fn stat_field_matches<T>(field: T, evidence: u64) -> bool
where
    u64: TryFrom<T>,
{
    u64::try_from(field).ok() == Some(evidence)
}

/// Inspect at most [`MAX_INSPECTED_ENTRIES`] names through one admitted root.
///
/// This is retain-only production behavior: candidates that satisfy every
/// read-only predicate still remain in place. `removed` therefore stays zero;
/// a later phase owns final rechecks and native removal.
#[must_use]
pub fn inspect_staging(root_path: &std::path::Path) -> CleanupReport {
    scan_staging_with(root_path, SystemTime::now(), pid_state)
}

#[cfg(unix)]
fn scan_staging_with(
    root_path: &std::path::Path,
    now: SystemTime,
    pid_state: impl FnMut(u32) -> PidState,
) -> CleanupReport {
    scan_staging_with_simulated_action(root_path, now, pid_state, |_| false)
}

/// Test-only policy seam that models an acknowledged action without a native
/// filesystem mutation. Production always supplies the retain closure above.
#[cfg(unix)]
fn scan_staging_with_simulated_action(
    root_path: &std::path::Path,
    now: SystemTime,
    mut pid_state: impl FnMut(u32) -> PidState,
    mut action: impl FnMut(&StagingCandidateFacts) -> bool,
) -> CleanupReport {
    use nix::dir::Dir;
    use std::os::unix::ffi::OsStrExt as _;

    let mut report = CleanupReport::default();
    let Some(root) = admit_staging_root(root_path) else {
        report.failures = 1;
        return report;
    };
    let Ok(duplicate) = root.handle.try_clone() else {
        report.failures = 1;
        return report;
    };
    let Ok(mut listing) = Dir::from_fd(std::os::fd::OwnedFd::from(duplicate)) else {
        report.failures = 1;
        return report;
    };

    for entry in listing.iter() {
        let Ok(entry) = entry else {
            report.failures += 1;
            break;
        };
        let name = OsStr::from_bytes(entry.file_name().to_bytes());
        if matches!(name.as_bytes(), b"." | b"..") {
            continue;
        }
        if report.inspected == MAX_INSPECTED_ENTRIES {
            report.truncated = true;
            break;
        }
        report.inspected += 1;
        match open_staging_candidate(&root, name) {
            Ok(Some(candidate)) => {
                let eligible = is_old_enough(now, candidate.modified_at)
                    && pid_state(candidate.pid) == PidState::NotLive;
                if eligible && report.removed < MAX_REMOVED_ENTRIES && action(&candidate) {
                    report.removed += 1;
                } else {
                    report.retained += 1;
                }
            }
            Ok(None) => report.retained += 1,
            Err(()) => {
                report.retained += 1;
                report.failures += 1;
            }
        }
    }
    report
}

#[cfg(not(unix))]
fn scan_staging_with(
    _root_path: &std::path::Path,
    _now: SystemTime,
    _pid_state: impl FnMut(u32) -> PidState,
) -> CleanupReport {
    CleanupReport::unsupported()
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

    #[cfg(unix)]
    struct TestDir(std::path::PathBuf);

    #[cfg(unix)]
    impl TestDir {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};

            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "omnifrons-staging-cleanup-{}-{label}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).expect("fixture directory");
            Self(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    #[cfg(unix)]
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    fn final_recheck_fixture(
        label: &str,
    ) -> (
        TestDir,
        String,
        std::path::PathBuf,
        super::StagingCandidateEvidence,
    ) {
        use std::os::unix::fs::PermissionsExt as _;

        let root = TestDir::new(label);
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700))
            .expect("owner-only root");
        let name = format!(".{PUBLICATION_HEX}.42-7.part");
        let candidate = root.path().join(&name);
        std::fs::write(&candidate, b"staging").expect("candidate");
        std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o600))
            .expect("owner-only candidate");
        let evidence =
            super::open_staging_evidence(root.path(), OsStr::new(&name)).expect("initial evidence");

        (root, name, candidate, evidence)
    }

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
            now - MIN_STAGING_AGE - Duration::from_millis(1)
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
                retained: 0,
                removed: 0,
                truncated: false,
                supported: cfg!(unix),
                failures: 0,
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn walker_retains_eligible_and_excluded_entries_without_mutating_them() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = TestDir::new("retain-first-walker");
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700))
            .expect("owner-only root");
        let eligible = root.path().join(format!(".{PUBLICATION_HEX}.42-7.part"));
        let excluded = root.path().join(".outbox.42-7.part");
        std::fs::write(&eligible, b"staging").expect("eligible candidate");
        std::fs::write(&excluded, b"outbox candidate").expect("excluded candidate");
        for path in [&eligible, &excluded] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .expect("owner-only candidate");
        }
        let now = eligible
            .metadata()
            .expect("candidate metadata")
            .modified()
            .expect("candidate modification time")
            + MIN_STAGING_AGE
            + Duration::from_millis(1);

        let report = super::scan_staging_with(root.path(), now, |_| PidState::NotLive);

        assert_eq!(report.inspected, 2);
        assert_eq!(report.retained, 2);
        assert_eq!(report.removed, 0);
        assert!(!report.truncated);
        assert_eq!(report.failures, 0);
        assert!(eligible.exists());
        assert!(excluded.exists());
    }

    #[cfg(unix)]
    #[test]
    fn simulated_action_never_exceeds_the_removal_policy_cap() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = TestDir::new("walker-action-cap");
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700))
            .expect("owner-only root");
        let candidates: Vec<_> = (0..=super::MAX_REMOVED_ENTRIES)
            .map(|sequence| {
                let path = root
                    .path()
                    .join(format!(".{PUBLICATION_HEX}.42-{sequence}.part"));
                std::fs::write(&path, b"staging").expect("candidate");
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                    .expect("owner-only candidate");
                path
            })
            .collect();
        let newest = candidates
            .iter()
            .map(|path| {
                path.metadata()
                    .expect("metadata")
                    .modified()
                    .expect("mtime")
            })
            .max()
            .expect("candidate times");

        let report = super::scan_staging_with_simulated_action(
            root.path(),
            newest + MIN_STAGING_AGE + Duration::from_millis(1),
            |_| PidState::NotLive,
            |_| true,
        );

        assert_eq!(report.inspected, super::MAX_REMOVED_ENTRIES + 1);
        assert_eq!(report.removed, super::MAX_REMOVED_ENTRIES);
        assert_eq!(report.retained, 1);
        assert!(candidates.iter().all(|path| path.exists()));
    }

    #[cfg(unix)]
    #[test]
    fn walker_marks_an_uninspected_remainder_as_truncated() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = TestDir::new("walker-inspection-cap");
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700))
            .expect("owner-only root");
        for sequence in 0..=super::MAX_INSPECTED_ENTRIES {
            let path = root.path().join(format!("foreign-{sequence}"));
            std::fs::write(path, b"unrelated").expect("foreign entry");
        }

        let report =
            super::scan_staging_with(root.path(), SystemTime::now(), |_| PidState::Unknown);

        assert_eq!(report.inspected, super::MAX_INSPECTED_ENTRIES);
        assert_eq!(report.retained, super::MAX_INSPECTED_ENTRIES);
        assert_eq!(report.removed, 0);
        assert!(report.truncated);
        assert_eq!(report.failures, 0);
    }

    #[cfg(unix)]
    #[test]
    fn live_and_unknown_pids_remain_even_when_the_action_seam_accepts_them() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = TestDir::new("walker-pid-retention");
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700))
            .expect("owner-only root");
        for pid in [42, 43] {
            let path = root.path().join(format!(".{PUBLICATION_HEX}.{pid}-7.part"));
            std::fs::write(&path, b"staging").expect("candidate");
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .expect("owner-only candidate");
        }
        let now = SystemTime::now() + MIN_STAGING_AGE + Duration::from_millis(1);

        let report = super::scan_staging_with_simulated_action(
            root.path(),
            now,
            |pid| {
                if pid == 42 {
                    PidState::Live
                } else {
                    PidState::Unknown
                }
            },
            |_| true,
        );

        assert_eq!(report.inspected, 2);
        assert_eq!(report.removed, 0);
        assert_eq!(report.retained, 2);
    }

    #[cfg(unix)]
    #[test]
    fn walker_reports_an_unadmitted_root_as_a_failure() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = TestDir::new("walker-root-failure");
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755))
            .expect("shared root");

        let report =
            super::scan_staging_with(root.path(), SystemTime::now(), |_| PidState::Unknown);

        assert_eq!(report.inspected, 0);
        assert_eq!(report.retained, 0);
        assert_eq!(report.removed, 0);
        assert!(!report.truncated);
        assert_eq!(report.failures, 1);
    }

    #[cfg(unix)]
    #[test]
    fn opens_owner_only_root_and_records_regular_single_link_candidate_evidence() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = TestDir::new("candidate-evidence");
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700))
            .expect("owner-only root");
        let name = format!(".{PUBLICATION_HEX}.42-7.part");
        let candidate = root.path().join(&name);
        std::fs::write(&candidate, b"staging").expect("candidate");
        std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o600))
            .expect("owner-only candidate");

        let evidence = super::open_staging_evidence(root.path(), OsStr::new(&name))
            .expect("safe candidate evidence");

        assert_eq!(evidence.size, 7);
        assert_eq!(evidence.link_count, 1);
        assert_eq!(evidence.mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn final_recheck_accepts_unchanged_held_root_and_candidate() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = TestDir::new("final-recheck-stable");
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700))
            .expect("owner-only root");
        let name = format!(".{PUBLICATION_HEX}.42-7.part");
        let candidate = root.path().join(&name);
        std::fs::write(&candidate, b"staging").expect("candidate");
        std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o600))
            .expect("owner-only candidate");

        let evidence =
            super::open_staging_evidence(root.path(), OsStr::new(&name)).expect("initial evidence");

        assert!(super::final_identity_matches(
            root.path(),
            OsStr::new(&name),
            &evidence
        ));
    }

    #[test]
    fn stat_field_comparison_accepts_representable_apple_width_values() {
        assert!(super::stat_field_matches(42_i32, 42));
        assert!(super::stat_field_matches(0o600_u16, 0o600));
        assert!(super::stat_field_matches(1_u16, 1));
    }

    #[test]
    fn stat_field_comparison_rejects_negative_and_overflowing_values() {
        assert!(!super::stat_field_matches(-1_i32, u64::MAX));
        assert!(!super::stat_field_matches(-1_i32, 1));
        assert!(!super::stat_field_matches(
            0o600_u16,
            u64::from(u16::MAX) + 1
        ));
    }

    #[cfg(unix)]
    #[test]
    fn final_recheck_rejects_a_replaced_candidate_basename() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = TestDir::new("final-recheck-replaced");
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700))
            .expect("owner-only root");
        let name = format!(".{PUBLICATION_HEX}.42-7.part");
        let candidate = root.path().join(&name);
        std::fs::write(&candidate, b"staging").expect("candidate");
        std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o600))
            .expect("owner-only candidate");
        let evidence =
            super::open_staging_evidence(root.path(), OsStr::new(&name)).expect("initial evidence");
        std::fs::remove_file(&candidate).expect("replace fixture");
        std::fs::write(&candidate, b"other").expect("replacement");
        std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o600))
            .expect("owner-only replacement");

        assert!(!super::final_identity_matches(
            root.path(),
            OsStr::new(&name),
            &evidence
        ));
    }

    #[cfg(unix)]
    #[test]
    fn final_recheck_rejects_a_root_path_replaced_by_a_symlink() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let fixture = TestDir::new("final-recheck-root-symlink");
        let root = fixture.path().join("admitted-root");
        let held_root_path = fixture.path().join("held-root");
        std::fs::create_dir(&root).expect("admitted root");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .expect("owner-only root");
        let name = format!(".{PUBLICATION_HEX}.42-7.part");
        let candidate = root.join(&name);
        std::fs::write(&candidate, b"staging").expect("candidate");
        std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o600))
            .expect("owner-only candidate");
        let evidence =
            super::open_staging_evidence(&root, OsStr::new(&name)).expect("initial evidence");

        std::fs::rename(&root, &held_root_path).expect("move admitted root");
        symlink(&held_root_path, &root).expect("replace root path with symlink");

        assert!(
            !super::final_identity_matches(&root, OsStr::new(&name), &evidence),
            "a replacement root path must fail closed even when it resolves to the held directory"
        );
    }

    #[cfg(unix)]
    #[test]
    fn final_recheck_rejects_candidate_metadata_changes() {
        use std::os::unix::fs::PermissionsExt as _;

        let (root, name, candidate, evidence) = final_recheck_fixture("final-recheck-metadata");
        std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o640))
            .expect("change candidate mode");

        assert!(!super::final_identity_matches(
            root.path(),
            OsStr::new(&name),
            &evidence
        ));

        std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o600))
            .expect("restore candidate mode");
        std::thread::sleep(Duration::from_millis(1));
        std::fs::write(&candidate, b"changed").expect("change candidate contents");

        assert!(!super::final_identity_matches(
            root.path(),
            OsStr::new(&name),
            &evidence
        ));
    }

    #[cfg(unix)]
    #[test]
    fn final_recheck_rejects_missing_candidate_and_root() {
        let (root, name, candidate, evidence) = final_recheck_fixture("final-recheck-missing");
        std::fs::remove_file(&candidate).expect("remove candidate fixture");

        assert!(!super::final_identity_matches(
            root.path(),
            OsStr::new(&name),
            &evidence
        ));

        let (root, name, _candidate, evidence) =
            final_recheck_fixture("final-recheck-missing-root");
        std::fs::rename(root.path(), root.path().with_extension("missing"))
            .expect("remove root fixture path");

        assert!(!super::final_identity_matches(
            root.path(),
            OsStr::new(&name),
            &evidence
        ));
    }

    #[cfg(unix)]
    #[test]
    fn final_recheck_rejects_linked_symbolic_and_non_regular_replacements() {
        use std::os::unix::fs::symlink;

        let (root, name, candidate, evidence) = final_recheck_fixture("final-recheck-hard-link");
        let replacement = root.path().join("replacement");
        std::fs::write(&replacement, b"replacement").expect("replacement file");
        std::fs::remove_file(&candidate).expect("remove candidate fixture");
        std::fs::hard_link(&replacement, &candidate).expect("hard-link replacement");
        assert!(!super::final_identity_matches(
            root.path(),
            OsStr::new(&name),
            &evidence
        ));

        let (root, name, candidate, evidence) = final_recheck_fixture("final-recheck-symlink");
        std::fs::remove_file(&candidate).expect("remove candidate fixture");
        symlink("replacement", &candidate).expect("symbolic-link replacement");
        assert!(!super::final_identity_matches(
            root.path(),
            OsStr::new(&name),
            &evidence
        ));

        let (root, name, candidate, evidence) = final_recheck_fixture("final-recheck-directory");
        std::fs::remove_file(&candidate).expect("remove candidate fixture");
        std::fs::create_dir(&candidate).expect("directory replacement");
        assert!(!super::final_identity_matches(
            root.path(),
            OsStr::new(&name),
            &evidence
        ));
    }

    #[cfg(unix)]
    #[test]
    fn retains_initial_root_identity_and_handles_after_root_path_replacement() {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        let fixture = TestDir::new("retained-handles");
        let root = fixture.path().join("admitted-root");
        std::fs::create_dir(&root).expect("admitted root");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .expect("owner-only root");
        let name = format!(".{PUBLICATION_HEX}.42-7.part");
        let candidate = root.join(&name);
        std::fs::write(&candidate, b"staging").expect("candidate");
        std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o600))
            .expect("owner-only candidate");
        let initial_root = std::fs::metadata(&root).expect("initial root metadata");
        let initial_candidate = std::fs::metadata(&candidate).expect("initial candidate metadata");

        let evidence = super::open_staging_evidence(&root, OsStr::new(&name))
            .expect("safe candidate evidence");

        std::fs::rename(&root, fixture.path().join("replaced-root")).expect("replace root path");
        std::fs::create_dir(&root).expect("replacement root");

        assert_eq!(evidence.root.dev, initial_root.dev());
        assert_eq!(evidence.root.ino, initial_root.ino());
        assert_eq!(evidence.root.uid, initial_root.uid());
        assert_eq!(
            evidence.root.mode,
            initial_root.permissions().mode() & 0o777
        );
        assert_eq!(
            evidence.root.handle.metadata().expect("held root").ino(),
            initial_root.ino()
        );
        assert_eq!(
            evidence.handle.metadata().expect("held candidate").ino(),
            initial_candidate.ino()
        );
        assert_eq!(evidence.ino, initial_candidate.ino());
    }

    #[cfg(unix)]
    #[test]
    fn retains_excluded_linked_and_non_regular_candidates_without_reading_them() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let root = TestDir::new("unsafe-candidates");
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700))
            .expect("owner-only root");
        let exact = format!(".{PUBLICATION_HEX}.42-7.part");
        let candidate = root.path().join(&exact);
        std::fs::write(&candidate, b"staging").expect("candidate");
        std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o600))
            .expect("owner-only candidate");

        assert!(
            super::open_staging_evidence(root.path(), OsStr::new(".outbox.42-7.part")).is_none(),
            "excluded producers never become candidates"
        );

        let linked = root.path().join(format!(".{PUBLICATION_HEX}.42-8.part"));
        std::fs::hard_link(&candidate, &linked).expect("hard link");
        assert!(
            super::open_staging_evidence(root.path(), linked.file_name().expect("name")).is_none(),
            "a multi-link candidate is retained"
        );

        let linked_path = root.path().join(format!(".{PUBLICATION_HEX}.42-9.part"));
        symlink(&candidate, &linked_path).expect("symbolic link");
        assert!(
            super::open_staging_evidence(root.path(), linked_path.file_name().expect("name"))
                .is_none(),
            "a symbolic link is not followed"
        );

        let directory = root.path().join(format!(".{PUBLICATION_HEX}.42-10.part"));
        std::fs::create_dir(&directory).expect("directory");
        assert!(
            super::open_staging_evidence(root.path(), directory.file_name().expect("name"))
                .is_none(),
            "a directory is retained without reading payload data"
        );
    }

    #[cfg(unix)]
    #[test]
    fn retains_roots_and_candidates_that_are_not_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = TestDir::new("owner-mode");
        let name = format!(".{PUBLICATION_HEX}.42-7.part");
        let candidate = root.path().join(&name);
        std::fs::write(&candidate, b"staging").expect("candidate");
        std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o600))
            .expect("owner-only candidate");
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755))
            .expect("shared root");

        assert!(
            super::open_staging_evidence(root.path(), OsStr::new(&name)).is_none(),
            "a root readable by another user is retained"
        );

        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700))
            .expect("owner-only root");
        std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o640))
            .expect("shared candidate");
        assert!(
            super::open_staging_evidence(root.path(), OsStr::new(&name)).is_none(),
            "a candidate readable by another user is retained"
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
