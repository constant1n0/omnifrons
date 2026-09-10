//! `FsWrongRootScanner`: the real, filesystem-backed
//! `omnifrons_app::wrong_root::WrongRootScanner` (spike slice 5d,
//! HAP-001-R32, R43, R44, D16 at the post-run-scan half of its default).
//!
//! It walks the active workspace root **depth-first**. Every file is opened
//! once without following a link, exactly as `FsCandidateProber` opens an
//! outbox entry.
//!
//! **How far "no link can redirect the walk" reaches, per platform.** On
//! unix every descent is an `openat` with `O_DIRECTORY | O_NOFOLLOW`
//! relative to the handle above it, so no path component is ever resolved
//! twice and a link placed anywhere in the tree can never redirect the walk
//! out of the project. **On Windows it reaches the final component only.**
//! `std` there offers no directory open relative to a held handle, so the
//! descent re-resolves the whole path from the root with
//! `FILE_FLAG_OPEN_REPARSE_POINT`, which refuses a reparse point at the
//! name being opened and follows one at every *ancestor* of it. A reparse
//! point planted at an ancestor between two descents is therefore
//! traversed, and the walk can leave the project: a finding may name a file
//! outside it, and the publish remedy -- which resolves the finding's name
//! the same way -- would then copy an outside file into the outbox as this
//! project's own unattributed output. The quarantine remedy's deletion is
//! not reachable there (`FsOutboxEntryOps` reports `Unverifiable` on
//! Windows, which keeps the original), so the exposure is copy and
//! disclosure, not loss. Closing it needs a relative open `std` does not
//! have and this crate adds no Windows API dependency for; nothing in this
//! repository can execute Windows, which is exactly why the claim is
//! narrowed here rather than asserted (spike slice 5d, R1-014, and
//! `docs/spike-log.md` § Slice 5d).
//!
//! **What it holds open, and why depth-first.** No handle survives the
//! call, and at any instant during the walk the open directory handles are
//! the ones on the path the walk is *on* -- at most [`MAX_SCAN_DEPTH`] + 1
//! of them -- never one per directory still to visit. A breadth-first walk
//! queues an open handle per pending directory, so a worktree with a few
//! hundred sibling directories (a dependency tree, a build tree) exhausts a
//! 256-descriptor process before it has looked at a single file, and every
//! finding under the directories it can then no longer open is lost. The
//! 32-handle cap HAP-001 D22 sets is shared with the approval surface, and
//! a scan whose descriptor use grew with the tree could starve it.
//!
//! **Every fact from a handle, the listing included.** What an entry *is*
//! -- and therefore whether it is examined at all -- is read relative to
//! the directory handle the walk holds (`fstatat` with
//! `AT_SYMLINK_NOFOLLOW`), not by re-resolving the entry's path. So are the
//! entry *names*: the listing is `fdopendir` over a duplicate of that same
//! handle (unix), never a `read_dir` of the directory's path. The
//! distinction is not decoration -- a listing taken by path decides what
//! *exists*, and a same-user process that swapped any ancestor for an empty
//! directory in the window between the descent and the listing would make
//! the whole subtree contribute nothing, its entries counted in none of
//! `scanned`, `excluded` or `unreadable` while `truncated` still read
//! `false` (spike slice 5d, R1-013). Windows keeps the path-based listing,
//! under the same disclosure the descent above carries.
//!
//! **Two reads, not one.** The head of each file is read first and the type
//! detected from it; the whole file is digested only when the policy
//! classifies it as a class this contract calls misplaced. A worktree
//! carries dependency and build trees whose bytes there is no reason to
//! hash, and a run-end scan has to stay bounded in time.
//!
//! **Bounded in three directions, and in memory as much as in time.**
//! [`MAX_SCANNED_ENTRIES`] entries examined -- directories as much as
//! files, or a tree of empty directories would walk without bound while the
//! cap meant to stop it never fired -- [`MAX_SCAN_DEPTH`] levels deep, and
//! [`MAX_DIRECTORY_NAMES`] names from any one directory. The third is a
//! separate bound because the first is not one: entries *examined* does not
//! bound entries *listed*, and one directory of a few million entries --
//! which the producer this scan polices creates as easily as any other --
//! would be collected and sorted whole before the entry cap could fire once
//! (spike slice 5d, R1-015). Past any of the three, and whenever the walk
//! runs out of file descriptors, the report says `truncated` rather than
//! folding what it could not see into `unreadable` and calling itself
//! complete.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::publication::CandidateSource;
use omnifrons_app::run_outbox::DirectoryHandle;
use omnifrons_app::wrong_root::{
    GIT_DIR, GIT_HEAD, GIT_OBJECTS, MAX_DIRECTORY_NAMES, MAX_SCAN_DEPTH, MAX_SCANNED_ENTRIES,
    OpenMisplacedError, ScanError, ScanReport, ScanRequest, ScannedFile, Verdict, WrongRootScanner,
    finding_for, is_excluded, is_git_metadata_dir,
};
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::outbox::DetectedType;
use sha2::{Digest as _, Sha256};

use crate::fs_candidate_prober::{Opened, open_entry};
use crate::fs_run_outbox_preparer::open_directory_no_follow;
use omnifrons_app::run_outbox::CandidateProbe;

/// The buffer size used to stream-hash a misplaced file's content.
const HASH_BUFFER_BYTES: usize = 64 * 1024;

/// The real, OS-backed [`WrongRootScanner`]. Stateless.
#[derive(Debug, Clone, Copy, Default)]
pub struct FsWrongRootScanner;

impl FsWrongRootScanner {
    /// Build a scanner.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// One directory the walk is *inside*: its open handle, its device path
/// (for the platforms with no relative open), its project-relative path,
/// how deep below the root it sits, and the names it has left to examine.
///
/// The stack of these is the walk, and its height -- never the width of the
/// tree -- is what bounds the open directory handles.
struct Frame {
    handle: DirectoryHandle,
    path: PathBuf,
    relative: PathBuf,
    depth: usize,
    names: std::vec::IntoIter<OsString>,
}

/// What one directory entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Directory,
    RegularFile,
    Link,
    /// A device node, a socket, a FIFO: never opened, never a finding.
    Other,
}

/// One directory's names, and whether that is the whole of them.
struct Listing {
    names: Vec<OsString>,
    /// `false` when the directory holds more than [`MAX_DIRECTORY_NAMES`]
    /// entries and the listing stopped there; the walk then reports
    /// `truncated` rather than calling a partial listing a complete one.
    complete: bool,
}

/// The names of the directory `dir` holds open, sorted, so a scan is
/// deterministic, and at most [`MAX_DIRECTORY_NAMES`] of them.
///
/// **Listed relative to the held handle** (`fdopendir`), never by
/// re-resolving the directory's path: a listing taken by path is a decision
/// about what exists, and a same-user process that swaps any ancestor
/// directory for an empty one between the walk's `openat` and its listing
/// would otherwise make the whole subtree contribute nothing at all -- the
/// frame popping with its entries counted in none of `scanned`, `excluded`
/// or `unreadable` while `truncated` still read `false` (spike slice 5d,
/// R1-013).
///
/// `fdopendir` takes ownership of the descriptor it is handed, so the
/// walk's own handle is duplicated rather than given away. `try_clone` is
/// `F_DUPFD_CLOEXEC`: the duplicate names the very same open directory
/// description -- the same directory the walk descended into, with no name
/// resolved a second time -- and closing it leaves the walk's handle
/// untouched. Reading the duplicate advances that shared description's
/// directory offset, which nothing else uses: the walk's handle is only
/// ever a `dirfd` for `openat` and `fstatat`, and neither reads an offset.
#[cfg(unix)]
fn sorted_names(dir: &DirectoryHandle, _dir_path: &Path) -> std::io::Result<Listing> {
    use nix::dir::Dir;
    use std::os::unix::ffi::OsStrExt as _;

    let duplicate = dir.as_file().try_clone()?;
    let mut listing = Dir::from_fd(std::os::fd::OwnedFd::from(duplicate))?;
    let mut names: Vec<OsString> = Vec::new();
    let mut complete = true;
    for entry in listing.iter() {
        let bytes = entry?.file_name().to_bytes().to_vec();
        // `readdir` yields the directory itself and its parent; descending
        // into either is how a walk never returns.
        if bytes == b"." || bytes == b".." {
            continue;
        }
        if names.len() >= MAX_DIRECTORY_NAMES {
            complete = false;
            break;
        }
        names.push(OsStr::from_bytes(&bytes).to_os_string());
    }
    names.sort();
    Ok(Listing { names, complete })
}

/// Windows: `std` offers no listing relative to a directory handle and this
/// crate adds no Windows API dependency, so the names come from the
/// directory's own path -- the same path-based residual `open_entry` and
/// `entry_kind` already disclose on this platform (`docs/spike-log.md`
/// § Slice 5d), not a claim. The per-directory bound applies here as it
/// does on unix.
///
/// **An entry error fails the whole listing, exactly as the unix arm's `?`
/// does** (spike slice 5d, R1-024). Skipping the erroring entry instead
/// would drop that name from the walk while it was counted in none of
/// `scanned`, `excluded` or `unreadable` and `complete` still read `true`,
/// so `truncated` stayed `false` -- silent loss under a report calling
/// itself complete, which is the shape R1-013 closed on unix. Failing here
/// makes the directory `unreadable` instead, which is a counted outcome.
#[cfg(not(unix))]
fn sorted_names(_dir: &DirectoryHandle, dir_path: &Path) -> std::io::Result<Listing> {
    let mut names: Vec<OsString> = Vec::new();
    let mut complete = true;
    for entry in std::fs::read_dir(dir_path)? {
        let entry = entry?;
        if names.len() >= MAX_DIRECTORY_NAMES {
            complete = false;
            break;
        }
        names.push(entry.file_name());
    }
    names.sort();
    Ok(Listing { names, complete })
}

/// Whether `error` says the process or the system ran out of file
/// descriptors, rather than anything about the entry it was raised on.
///
/// The distinction is the whole difference between a report that is partial
/// and says so and one that folds what it could not see into `unreadable`
/// and calls itself complete (spike slice 5d, R1-002).
#[cfg(unix)]
fn is_descriptor_exhaustion(error: &std::io::Error) -> bool {
    use nix::errno::Errno;
    matches!(
        error.raw_os_error(),
        Some(code) if code == Errno::EMFILE as i32 || code == Errno::ENFILE as i32
    )
}

/// Windows: `ERROR_TOO_MANY_OPEN_FILES`.
#[cfg(not(unix))]
fn is_descriptor_exhaustion(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(4)
}

/// What the entry `name` inside `dir` is, taken **relative to the directory
/// handle the walk holds** and never by re-resolving the entry's path.
///
/// Whether an entry is examined at all is a decision, and a decision taken
/// by path can be answered by a directory that is no longer the one the
/// walk descended into: a same-user process that presents an entry as a
/// link for exactly that instant gets it counted excluded and never opened,
/// and the finding is lost while `truncated` still reads `false`
/// (spike slice 5d, R1-010).
#[cfg(unix)]
fn entry_kind(dir: &DirectoryHandle, _dir_path: &Path, name: &OsStr) -> Option<EntryKind> {
    use nix::fcntl::AtFlags;
    use nix::sys::stat::{SFlag, fstatat};
    let stat = fstatat(dir.as_file(), Path::new(name), AtFlags::AT_SYMLINK_NOFOLLOW).ok()?;
    let masked = stat.st_mode & SFlag::S_IFMT.bits();
    Some(if masked == SFlag::S_IFDIR.bits() {
        EntryKind::Directory
    } else if masked == SFlag::S_IFLNK.bits() {
        EntryKind::Link
    } else if masked == SFlag::S_IFREG.bits() {
        EntryKind::RegularFile
    } else {
        EntryKind::Other
    })
}

/// Windows: `std` offers no stat relative to a directory handle, so the
/// entry's type comes from its own path through `symlink_metadata`, which
/// at least never follows a link at the final component. The same
/// path-based residual `open_entry` already discloses on this platform
/// (`docs/spike-log.md` § Slice 5d), not a claim.
#[cfg(not(unix))]
fn entry_kind(_dir: &DirectoryHandle, dir_path: &Path, name: &OsStr) -> Option<EntryKind> {
    let file_type = std::fs::symlink_metadata(dir_path.join(name))
        .ok()?
        .file_type();
    Some(if file_type.is_symlink() {
        EntryKind::Link
    } else if file_type.is_dir() {
        EntryKind::Directory
    } else if file_type.is_file() {
        EntryKind::RegularFile
    } else {
        EntryKind::Other
    })
}

/// Whether the directory `dir` names is a real repository's metadata
/// directory -- it holds `HEAD` and an `objects` directory -- both read
/// relative to the handle just opened and never by re-resolving the path.
///
/// See `omnifrons_app::wrong_root::is_git_metadata_dir` for why the name
/// alone is not the test (spike slice 5d, R1-006).
fn holds_git_metadata(dir: &DirectoryHandle, path: &Path) -> bool {
    let holds_head = matches!(open_entry(dir, path, OsStr::new(GIT_HEAD)), Opened::File(_));
    let holds_objects = crate::fs_run_outbox_preparer::open_child_directory_no_follow(
        dir.as_file(),
        path,
        OsStr::new(GIT_OBJECTS),
    )
    .is_ok();
    is_git_metadata_dir(holds_head, holds_objects)
}

/// Read the first [`DetectedType::SNIFF_BYTES`] of `file`, then rewind it.
fn read_head(file: &mut File) -> std::io::Result<Vec<u8>> {
    let mut head = vec![0u8; DetectedType::SNIFF_BYTES];
    let mut filled = 0usize;
    while filled < head.len() {
        match file.read(&mut head[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    head.truncate(filled);
    file.seek(SeekFrom::Start(0))?;
    Ok(head)
}

/// Stream-hash `file` from its start, returning the digest and the byte
/// count. The handle is dropped by the caller immediately afterwards.
fn digest_whole(file: &mut File) -> std::io::Result<(Sha256Digest, u64)> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_BUFFER_BYTES];
    let mut size: u64 = 0;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size = size.saturating_add(read as u64);
    }
    let digest: [u8; 32] = hasher.finalize().into();
    Ok((Sha256Digest(digest), size))
}

/// The project-relative name of `relative`, `/`-separated on every
/// platform, or `None` when it is not valid UTF-8 (a name a finding could
/// not carry as text anyway).
fn relative_name(relative: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for component in relative.components() {
        parts.push(component.as_os_str().to_str()?);
    }
    Some(parts.join("/"))
}

impl WrongRootScanner for FsWrongRootScanner {
    fn scan(&self, request: &ScanRequest<'_>) -> Result<ScanReport, ScanError> {
        let root_path = request.project.path().to_path_buf();
        let root = DirectoryHandle::new(
            open_directory_no_follow(&root_path).map_err(|_| ScanError::Unreadable)?,
        );
        let root_listing = sorted_names(&root, &root_path).map_err(|_| ScanError::Unreadable)?;
        let mut report = ScanReport {
            truncated: !root_listing.complete,
            ..ScanReport::default()
        };
        // Every entry the walk looks at, whatever it turns out to be. The
        // report's own `scanned` is the narrower "files examined"; this is
        // what the entry cap is a cap on.
        let mut examined: u32 = 0;
        let mut stack = vec![Frame {
            handle: root,
            path: root_path,
            relative: PathBuf::new(),
            depth: 0,
            names: root_listing.names.into_iter(),
        }];

        // Depth-first: the stack holds one open handle per level of the
        // path the walk is currently on, and never one per directory it has
        // yet to visit.
        while !stack.is_empty() {
            let Some(name) = stack.last_mut().and_then(|frame| frame.names.next()) else {
                stack.pop();
                continue;
            };
            let mut descend = None;
            {
                let frame = stack
                    .last()
                    .expect("the stack is non-empty everywhere inside this loop");
                let relative = frame.relative.join(&name);
                if is_excluded(&relative, request.outbox) {
                    report.excluded = report.excluded.saturating_add(1);
                    continue;
                }
                if examined >= MAX_SCANNED_ENTRIES {
                    report.truncated = true;
                    return Ok(report);
                }
                examined = examined.saturating_add(1);

                let Some(kind) = entry_kind(&frame.handle, &frame.path, &name) else {
                    report.unreadable = report.unreadable.saturating_add(1);
                    continue;
                };
                match kind {
                    // Never dereferenced, so it can never redirect the walk
                    // or produce a finding.
                    EntryKind::Link => report.excluded = report.excluded.saturating_add(1),
                    EntryKind::Other => report.unreadable = report.unreadable.saturating_add(1),
                    EntryKind::Directory => {
                        descend = open_child(frame, &name, relative, &mut report);
                    }
                    EntryKind::RegularFile => {
                        examine_file(frame, &name, &relative, request, &mut report);
                    }
                }
            }
            if let Some(frame) = descend {
                stack.push(frame);
            }
        }
        Ok(report)
    }

    fn open_misplaced(
        &self,
        project: &WorkspaceRoot,
        relative: &str,
    ) -> Result<CandidateSource, OpenMisplacedError> {
        let path = Path::new(relative);
        omnifrons_domain::outbox::validate_project_relative(relative)
            .map_err(|_| OpenMisplacedError::InvalidName)?;
        let file_name = path
            .file_name()
            .ok_or(OpenMisplacedError::InvalidName)?
            .to_os_string();

        // One no-follow open per directory component, each relative to the
        // handle above it, so no component is re-resolved and a link
        // standing in for one is refused rather than traversed.
        //
        // Unix only, exactly as the walk's own descent is: on Windows
        // `open_child_directory_no_follow` re-resolves the joined path and
        // `FILE_FLAG_OPEN_REPARSE_POINT` protects the final component
        // alone, so a reparse point at an ancestor is followed and this
        // re-open can reach a file outside the project -- which the publish
        // remedy would then copy into the outbox as this project's own
        // output (see this module's own comment, R1-014).
        let mut dir_path = project.path().to_path_buf();
        let mut dir = DirectoryHandle::new(
            open_directory_no_follow(&dir_path).map_err(|_| OpenMisplacedError::NotFound)?,
        );
        if let Some(parent) = path.parent() {
            for component in parent.components() {
                let name = component.as_os_str();
                let opened = crate::fs_run_outbox_preparer::open_child_directory_no_follow(
                    dir.as_file(),
                    &dir_path,
                    name,
                )
                .map_err(|_| OpenMisplacedError::NotFound)?;
                dir_path = dir_path.join(name);
                dir = DirectoryHandle::new(opened);
            }
        }

        let handle = match open_entry(&dir, &dir_path, &file_name) {
            Opened::File(file) => file,
            Opened::Refused(CandidateProbe::Escape(_)) => {
                return Err(OpenMisplacedError::NotRegularFile);
            }
            Opened::Refused(_) => return Err(refused_reason(&dir_path, &file_name)),
            // No descriptor left to open the file with is not a fact about
            // the file: reported as unreadable, never as "nothing is there".
            Opened::Exhausted => return Err(OpenMisplacedError::Unreadable),
        };
        // Every fact from that one handle's own `fstat`, exactly as
        // `FsCandidateProber` reads an outbox entry's: a regular file, and
        // the link count HAP-001-R20 refuses above one -- HAP-001-R32
        // applies R15 through R20 with the misplaced file as the candidate,
        // and a second name for the same bytes makes both remedies mean
        // something other than what the surface says.
        let metadata = handle
            .metadata()
            .map_err(|_| OpenMisplacedError::Unreadable)?;
        if !metadata.file_type().is_file() {
            return Err(OpenMisplacedError::NotRegularFile);
        }
        if let Some(count) = crate::fs_candidate_prober::link_count(&metadata)
            && count > 1
        {
            return Err(OpenMisplacedError::Linked { link_count: count });
        }
        Ok(CandidateSource {
            dir,
            dir_path,
            file_name,
            handle,
        })
    }
}

/// Why an open that did not classify itself was refused: nothing at the
/// name, something that is not a regular file, or a file that could not be
/// read. Read from the name's own `symlink_metadata`, which never follows
/// a link at the final component.
fn refused_reason(dir_path: &Path, name: &std::ffi::OsStr) -> OpenMisplacedError {
    match std::fs::symlink_metadata(dir_path.join(name)) {
        Err(_) => OpenMisplacedError::NotFound,
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
            OpenMisplacedError::NotRegularFile
        }
        Ok(_) => OpenMisplacedError::Unreadable,
    }
}

/// Descend into the directory `name` inside `frame`, or account for why the
/// walk did not: the depth bound and descriptor exhaustion both make the
/// report `truncated`, a real repository's metadata directory is `excluded`,
/// and anything else that will not open is `unreadable`.
fn open_child(
    frame: &Frame,
    name: &OsStr,
    relative: PathBuf,
    report: &mut ScanReport,
) -> Option<Frame> {
    if frame.depth + 1 > MAX_SCAN_DEPTH {
        report.truncated = true;
        return None;
    }
    let opened = match crate::fs_run_outbox_preparer::open_child_directory_no_follow(
        frame.handle.as_file(),
        &frame.path,
        name,
    ) {
        Ok(opened) => opened,
        Err(error) if is_descriptor_exhaustion(&error) => {
            report.truncated = true;
            return None;
        }
        Err(_) => {
            report.unreadable = report.unreadable.saturating_add(1);
            return None;
        }
    };
    let handle = DirectoryHandle::new(opened);
    let path = frame.path.join(name);
    // HAP-001-R44's sibling exclusion, decided by what the directory holds
    // rather than by what it is called (R1-006).
    if name == OsStr::new(GIT_DIR) && holds_git_metadata(&handle, &path) {
        report.excluded = report.excluded.saturating_add(1);
        return None;
    }
    match sorted_names(&handle, &path) {
        Ok(listing) => {
            // A listing cut short at [`MAX_DIRECTORY_NAMES`] is a partial
            // walk, reported as one: the names it did not collect are
            // neither examined nor counted anywhere else.
            if !listing.complete {
                report.truncated = true;
            }
            Some(Frame {
                handle,
                path,
                relative,
                depth: frame.depth + 1,
                names: listing.names.into_iter(),
            })
        }
        Err(error) if is_descriptor_exhaustion(&error) => {
            report.truncated = true;
            None
        }
        Err(_) => {
            report.unreadable = report.unreadable.saturating_add(1);
            None
        }
    }
}

/// Examine the regular file `name` inside `frame` and account for it in at
/// most one of the report's counters.
///
/// The three are disjoint on purpose: `scanned` is the files a verdict was
/// reached for, `unreadable` the entries that could not be opened, read, or
/// *named*, and `excluded` the ones a rule skipped. A file counted in two
/// of them makes every count downstream of it unusable.
///
/// **At most one, not exactly one** (spike slice 5d, R1-018). A file the
/// walk had no descriptor left to open is accounted for by `truncated`
/// instead: running out of descriptors is a fact about the process and
/// counting it as an entry outcome is what would make the report read
/// complete when it is not. `ScanReport`'s own comment carries the same
/// distinction for the walk as a whole.
fn examine_file(
    frame: &Frame,
    name: &OsStr,
    relative: &Path,
    request: &ScanRequest<'_>,
    report: &mut ScanReport,
) {
    let Some(name_text) = relative_name(relative) else {
        report.unreadable = report.unreadable.saturating_add(1);
        return;
    };
    match examine(frame, name, &name_text, request) {
        Examined::Facts(scanned) => match finding_for(&scanned, request.classifier) {
            Verdict::Misplaced(finding) => {
                report.scanned = report.scanned.saturating_add(1);
                report.findings.push(finding);
            }
            Verdict::KeptWhereItIs => report.scanned = report.scanned.saturating_add(1),
            // Misplaced by class, at a name no finding may carry across
            // IPC. Counted, so the report can never read as a clean scan of
            // a project a file was quietly dropped from (R3-003).
            Verdict::UnreportableName => {
                report.unreadable = report.unreadable.saturating_add(1);
            }
        },
        Examined::NotMisplaced => report.scanned = report.scanned.saturating_add(1),
        Examined::Unreadable => report.unreadable = report.unreadable.saturating_add(1),
        // No descriptor left to open it with says nothing about the file:
        // the report is partial, and says so.
        Examined::Exhausted => report.truncated = true,
    }
}

/// What examining one file yielded.
enum Examined {
    /// The file's facts, digested because its class is one this contract
    /// calls misplaced. The verdict itself is `finding_for`'s.
    Facts(ScannedFile),
    /// The file was examined and is not misplaced: nothing was digested.
    NotMisplaced,
    /// The file could not be opened or read.
    Unreadable,
    /// There was no file descriptor left to open it with.
    Exhausted,
}

/// Open `name` inside `frame` once, without following a link, detect its
/// type from the head of the file, and digest the whole file only when the
/// policy classifies it as misplaced. The handle is dropped before this
/// returns.
fn examine(frame: &Frame, name: &OsStr, name_text: &str, request: &ScanRequest<'_>) -> Examined {
    let mut file = match open_entry(&frame.handle, &frame.path, name) {
        Opened::File(file) => file,
        Opened::Refused(_) => return Examined::Unreadable,
        Opened::Exhausted => return Examined::Exhausted,
    };
    // `fstat` on the handle just opened, never a stat by path.
    let Ok(metadata) = file.metadata() else {
        return Examined::Unreadable;
    };
    if !metadata.file_type().is_file() {
        return Examined::Unreadable;
    }
    let Ok(head) = read_head(&mut file) else {
        return Examined::Unreadable;
    };
    let detected_type = DetectedType::detect(name_text, &head);
    let size = metadata.len();
    let class = request.classifier.classify(name_text, detected_type, size);
    if !omnifrons_domain::wrong_root::is_misplaced_class(class) {
        return Examined::NotMisplaced;
    }
    let Ok((digest, digested_size)) = digest_whole(&mut file) else {
        return Examined::Unreadable;
    };
    Examined::Facts(ScannedFile {
        name: name_text.to_string(),
        size: digested_size,
        digest,
        detected_type,
    })
}

#[cfg(test)]
mod tests {
    use super::{DirectoryHandle, MAX_DIRECTORY_NAMES, is_descriptor_exhaustion, sorted_names};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A drop-guard temp directory.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "omnifrons-wrong-root-scanner-unit-{}-{label}-{n}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).expect("fixture dir");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn open_handle(path: &Path) -> DirectoryHandle {
        DirectoryHandle::new(
            crate::fs_run_outbox_preparer::open_directory_no_follow(path)
                .expect("the fixture directory opens"),
        )
    }

    /// R1-013: the walk's *listing* is taken from the directory handle it
    /// holds, exactly as every other fact about an entry already is, and
    /// never by re-resolving the directory's path a second time.
    ///
    /// The swap is arranged deterministically rather than raced: the handle
    /// is opened, the directory that path named is moved aside and an empty
    /// one put in its place, and only then is the listing taken -- which is
    /// precisely the window a same-user process gets between the walk's
    /// `openat` and its listing. A listing that re-resolved the path would
    /// return nothing for a directory whose every entry is still there, and
    /// the frame would pop having counted none of them in `scanned`,
    /// `excluded` or `unreadable` while `truncated` still read `false`:
    /// silent loss under a report that calls itself complete.
    #[cfg(unix)]
    #[test]
    fn the_listing_comes_from_the_held_handle_and_not_from_the_path() {
        let dir = TempDir::new("listing-handle");
        let subject = dir.path().join("subject");
        std::fs::create_dir(&subject).expect("fixture dir");
        std::fs::write(subject.join("report.pdf"), b"x").expect("fixture file");

        let handle = open_handle(&subject);
        // The adversary is the unmediated producer the scan exists to
        // catch: it swaps the directory at the path for an empty one while
        // the walk holds the original open.
        std::fs::rename(&subject, dir.path().join("moved")).expect("the swap aside");
        std::fs::create_dir(&subject).expect("an empty directory at the same path");

        let listing = sorted_names(&handle, &subject).expect("the held handle lists");
        assert_eq!(
            listing.names,
            vec![std::ffi::OsString::from("report.pdf")],
            "the listing must name what the held handle holds, not what the path now resolves to"
        );
        assert!(listing.complete, "a one-entry directory is listed whole");
    }

    /// The Windows counterpart of the assertion above. `std` offers no
    /// listing relative to a directory handle there and this crate adds no
    /// Windows API dependency, so the names come from the directory's own
    /// path and the swap above cannot be refused -- the disclosed
    /// path-based residual on that platform (`docs/spike-log.md` § Slice
    /// 5d), not a claim. The swap itself is not arranged here: a directory
    /// whose handle this process holds open cannot be renamed on Windows,
    /// so the fixture would fail rather than the assertion. What is pinned
    /// is that the path-based listing is the one that runs.
    #[cfg(not(unix))]
    #[test]
    fn a_platform_without_a_handle_relative_listing_lists_by_path() {
        let dir = TempDir::new("listing-path");
        let subject = dir.path().join("subject");
        std::fs::create_dir(&subject).expect("fixture dir");
        std::fs::write(subject.join("report.pdf"), b"x").expect("fixture file");

        let handle = open_handle(&subject);
        let listing = sorted_names(&handle, &subject).expect("the path lists");
        assert_eq!(
            listing.names,
            vec![std::ffi::OsString::from("report.pdf")],
            "the listing names what the directory holds"
        );
        assert!(listing.complete, "a one-entry directory is listed whole");
    }

    /// R1-015: one directory's listing is bounded, and a directory wider
    /// than the bound says so rather than materializing every name it
    /// holds. `MAX_SCANNED_ENTRIES` bounds the entries a walk *examines*,
    /// which is not the same bound at all: a single directory of a few
    /// million entries -- as easy for a producer to create as any other --
    /// is collected and sorted whole before that cap can fire once, and
    /// this scan runs automatically at every run end.
    #[test]
    fn one_directorys_listing_is_bounded_and_says_so_when_it_is_cut_short() {
        let dir = TempDir::new("listing-cap");
        let subject = dir.path().join("wide");
        std::fs::create_dir(&subject).expect("fixture dir");
        for n in 0..=MAX_DIRECTORY_NAMES {
            std::fs::write(subject.join(format!("f{n:07}")), b"").expect("fixture entry");
        }

        let handle = open_handle(&subject);
        let listing = sorted_names(&handle, &subject).expect("list");
        assert_eq!(
            listing.names.len(),
            MAX_DIRECTORY_NAMES,
            "one directory's listing is bounded, whatever the directory holds"
        );
        assert!(
            !listing.complete,
            "a listing cut short must say so, so the report reads truncated rather than clean"
        );
    }

    /// R1-002: running out of descriptors is not a fact about the entry the
    /// open was attempted on, and the walk reports the two differently --
    /// exhaustion makes the report `truncated`, anything else counts
    /// `unreadable`. Pinned here because arranging real descriptor
    /// exhaustion inside a test process starves every other test sharing
    /// it, and this repository adds no `nix` feature to lower a limit.
    #[test]
    fn only_descriptor_exhaustion_is_classified_as_exhaustion() {
        #[cfg(unix)]
        let exhausted = [
            nix::errno::Errno::EMFILE as i32,
            nix::errno::Errno::ENFILE as i32,
        ];
        #[cfg(not(unix))]
        let exhausted = [4];
        for code in exhausted {
            assert!(
                is_descriptor_exhaustion(&std::io::Error::from_raw_os_error(code)),
                "raw OS error {code} is descriptor exhaustion"
            );
        }
        #[cfg(unix)]
        let other = [
            nix::errno::Errno::EACCES as i32,
            nix::errno::Errno::ENOENT as i32,
            nix::errno::Errno::ELOOP as i32,
            nix::errno::Errno::ENOTDIR as i32,
        ];
        #[cfg(not(unix))]
        let other = [5, 2, 3];
        for code in other {
            assert!(
                !is_descriptor_exhaustion(&std::io::Error::from_raw_os_error(code)),
                "raw OS error {code} is a fact about the entry, not about the process"
            );
        }
    }
}
