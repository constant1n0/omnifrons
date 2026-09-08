//! `FsCandidateProber`: the real, filesystem-backed
//! `omnifrons_app::CandidateProber` (spike slice 5, HAP-001-R15, R16, R20),
//! sharing `FsExecutableProber`'s open-once discipline and extending it
//! with the three additions HAP-001 states: the link-count check, the
//! handle kept for the approval that follows, and (in slice 5b) the
//! published bytes taken from that handle.
//!
//! Unix: the entry is opened *relative to the directory handle* the
//! inventory holds (`openat`) with `O_NOFOLLOW` (a link at the name fails
//! with `ELOOP` and is never dereferenced), `O_NONBLOCK` (a FIFO with no
//! writer opens immediately instead of blocking; it has no effect on a
//! regular file), and `O_CLOEXEC`. Every identity fact then comes from
//! `fstat` on that one handle -- regular file, link count, size -- and
//! the digest and the type-sniffing bytes from one streaming read of the
//! same handle, which is rewound and returned as the held handle.
//!
//! Windows: the entry is opened by its joined path with
//! `FILE_FLAG_OPEN_REPARSE_POINT`, so a symbolic link or junction at the
//! name opens as itself and is refused rather than followed; regular-file
//! and size facts come from the handle. The link count is not read: the
//! by-handle accessor in `std::os::windows::fs::MetadataExt` is unstable
//! and this crate adds no Windows API dependency, so `outbox-linked`
//! cannot be produced there -- a residual disclosed under HAP-001-R19 and
//! `docs/spike-log.md` § Slice 5, not a claim.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::Path;

use omnifrons_app::run_outbox::{
    CandidateProbe, CandidateProber, DirectoryHandle, EscapeReason, RegularCandidate,
};
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::outbox::DetectedType;
use sha2::{Digest, Sha256};

/// The buffer size used to stream-hash an entry's content.
const HASH_BUFFER_BYTES: usize = 64 * 1024;

/// The real, OS-backed [`CandidateProber`]. Stateless.
#[derive(Debug, Clone, Copy, Default)]
pub struct FsCandidateProber;

impl FsCandidateProber {
    /// Build a prober.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// What opening an entry without following links yielded, before any
/// fact is read from it.
enum Opened {
    File(File),
    Refused(CandidateProbe),
}

/// Unix: `openat` relative to the held directory, no-follow, non-blocking.
#[cfg(unix)]
fn open_entry(dir: &DirectoryHandle, _dir_path: &Path, name: &OsStr) -> Opened {
    use nix::errno::Errno;
    use nix::fcntl::{OFlag, openat};
    use nix::sys::stat::Mode;
    match openat(
        dir.as_file(),
        Path::new(name),
        OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC | OFlag::O_NONBLOCK,
        Mode::empty(),
    ) {
        Ok(fd) => Opened::File(File::from(fd)),
        // A symbolic link at the name: refused without dereferencing.
        Err(Errno::ELOOP) => Opened::Refused(CandidateProbe::Escape(EscapeReason::Link)),
        // A socket cannot be opened with `open(2)` at all -- `ENXIO` on
        // Linux, `EOPNOTSUPP` on macOS (open(2): "the named file is a
        // socket"): not a regular file, by the only fact obtainable.
        Err(Errno::ENXIO | Errno::EOPNOTSUPP) => {
            Opened::Refused(CandidateProbe::Escape(EscapeReason::NotRegular))
        }
        Err(_) => Opened::Refused(CandidateProbe::Unreadable),
    }
}

/// Windows: open by the joined path with the reparse point opened as
/// itself; a directory or a reparse point that will not open as a file is
/// classified from the path's own metadata (the disclosed path-based
/// residual on this platform).
#[cfg(not(unix))]
fn open_entry(_dir: &DirectoryHandle, dir_path: &Path, name: &OsStr) -> Opened {
    let path = dir_path.join(name);
    let opened = {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(
                    crate::fs_run_outbox_preparer::win_flags::FILE_FLAG_OPEN_REPARSE_POINT,
                )
                .open(&path)
        }
        #[cfg(not(windows))]
        {
            File::open(&path)
        }
    };
    match opened {
        Ok(file) => Opened::File(file),
        Err(_) => match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Opened::Refused(CandidateProbe::Escape(EscapeReason::Link))
            }
            Ok(metadata) if !metadata.is_file() => {
                Opened::Refused(CandidateProbe::Escape(EscapeReason::NotRegular))
            }
            _ => Opened::Refused(CandidateProbe::Unreadable),
        },
    }
}

/// The link count from the handle's own metadata, where the platform's
/// stable `std` exposes it. The `Option` is the platform seam -- the
/// Windows version below is where `None` comes from -- so the unix version
/// wraps unconditionally on purpose.
#[cfg(unix)]
#[allow(clippy::unnecessary_wraps)]
fn link_count(metadata: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt as _;
    Some(metadata.nlink())
}

#[cfg(not(unix))]
fn link_count(_metadata: &std::fs::Metadata) -> Option<u64> {
    None
}

/// Stream-hash `file` from its start, keeping the first
/// [`DetectedType::SNIFF_BYTES`] for type detection, then rewind it so
/// the held handle reads the same bytes again.
fn hash_from_handle(mut file: File, name: &OsStr) -> CandidateProbe {
    let mut hasher = Sha256::new();
    let mut head: Vec<u8> = Vec::with_capacity(DetectedType::SNIFF_BYTES);
    let mut buffer = vec![0u8; HASH_BUFFER_BYTES];
    let mut size: u64 = 0;
    loop {
        let read = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(_) => return CandidateProbe::Unreadable,
        };
        hasher.update(&buffer[..read]);
        if head.len() < DetectedType::SNIFF_BYTES {
            let take = (DetectedType::SNIFF_BYTES - head.len()).min(read);
            head.extend_from_slice(&buffer[..take]);
        }
        size = size.saturating_add(read as u64);
    }
    if file.seek(SeekFrom::Start(0)).is_err() {
        return CandidateProbe::Unreadable;
    }
    let digest: [u8; 32] = hasher.finalize().into();
    CandidateProbe::Regular(RegularCandidate {
        size,
        digest: Sha256Digest(digest),
        detected_type: DetectedType::detect(&name.to_string_lossy(), &head),
        handle: file,
    })
}

impl CandidateProber for FsCandidateProber {
    fn probe(&self, dir: &DirectoryHandle, dir_path: &Path, name: &OsStr) -> CandidateProbe {
        let file = match open_entry(dir, dir_path, name) {
            Opened::File(file) => file,
            Opened::Refused(probe) => return probe,
        };
        // `fstat` on the handle just opened, never a stat by path: every
        // fact below is about the file description `file` names.
        let Ok(metadata) = file.metadata() else {
            return CandidateProbe::Unreadable;
        };
        if !metadata.file_type().is_file() {
            return CandidateProbe::Escape(EscapeReason::NotRegular);
        }
        if let Some(count) = link_count(&metadata)
            && count > 1
        {
            return CandidateProbe::Linked { link_count: count };
        }
        hash_from_handle(file, name)
    }
}
