//! `FsDefinitionTree`: the real, filesystem-backed
//! `omnifrons_app::definition_tree::DefinitionTree` (ADR-0005 sub-slice
//! 1f), one read-only pass over a real directory tree translating the
//! `base_omnifrons` prototype's `walk_workspace` and `find_ancestors`
//! into the port's own, slightly widened [`TreeSnapshot`] shape -- see
//! `omnifrons_app::definition_tree`'s own module documentation for
//! every rule this adapter must honor.
//!
//! **Why plain `std::fs`, not an `openat`-based, no-follow walk.** Other
//! adapters in this crate (`crate::fs_wrong_root_scanner`,
//! `crate::fs_run_outbox_preparer`) harden every open against a
//! same-user adversary racing the walk, because they police a
//! harness's own untrusted output. This port instead mirrors a
//! read-only inventory of a workspace's own agent-identity files --
//! the prototype's own `scan` command -- where the question is "what
//! does this tree currently say", not "can a concurrent writer fool
//! this walk". `std::fs::symlink_metadata` classifies every entry
//! without following its final component, exactly like the prototype's
//! own `lstat`-based checks, and a plain `std::fs::read` follows that
//! classification; the narrow benign race this leaves open (a file
//! replaced between the classification and the read) is the same one
//! the prototype itself accepts, since it is read-only there too.
//!
//! **No bounded depth or entry count.** Unlike the wrong-root scanner,
//! [`omnifrons_app::definition_tree::DefinitionTreeError`] has no
//! "truncated" variant: this walk is meant to be complete, exactly as
//! the prototype's own unbounded `os.walk` is. A pathological tree can
//! still exhaust the call stack, since the walk recurses one directory
//! level at a time; the prototype carries the same unbounded-recursion
//! shape in spirit (`os.walk`'s own generator chases every directory
//! depth-first), so this is a disclosed, matching tradeoff rather than
//! a new one.
//!
//! **`RootUnreadable` versus `WalkFailed`.** Every filesystem operation
//! this module performs *for the scan root's own directory* that can
//! still fail at all -- an identity file's intermediate path segments
//! and final content, `SKILL.md`, and the root's own listing -- reports
//! [`omnifrons_app::definition_tree::DefinitionTreeError::RootUnreadable`]
//! on any unexpected failure; the identical failure anywhere *below*
//! the root reports
//! [`omnifrons_app::definition_tree::DefinitionTreeError::WalkFailed`]
//! instead -- except a below-root directory *listing* failure, which
//! the next paragraph carves out. Host extras, generated-state
//! directories, and every `lexists`-shaped presence check the next
//! section below describes are outside this split entirely: each of
//! them only ever asks "is anything there", so a permission error there
//! is never `RootUnreadable` or `WalkFailed` -- it is simply "absent",
//! at the scan root exactly like everywhere else. A root directory a
//! caller cannot search at all (unix mode `0o000`) still fails every
//! operation that remains part of this split the same way, for the
//! same underlying reason, so that subset reports it uniformly as "the
//! scan root cannot be read" regardless of which specific check
//! happens to hit the permission error first.
//!
//! **Lenient, not strict, a `lexists`-shaped presence check.** The
//! prototype's own presence checks are not one uniform rule. A handful
//! of them -- `find_host_extras`, `has_identity_files` (`find_ancestors`'s
//! own filter, read back here through `identity_file_present`), and
//! the generated-state check inside its warnings composition -- ask a
//! blanket `os.path.lexists`/`os.path.exists` whether anything at all
//! sits at a path, and both of those stdlib calls catch *every*
//! `OSError` a stat can raise, a permission error included, and simply
//! answer "not there". `read_identity`'s own walk down an identity
//! kind's intermediate path segments (`.claude` before
//! `.claude/CLAUDE.md`, for instance) is not one of those calls: it
//! checks each segment with its own, unwrapped directory check instead,
//! so this module mirrors that split by call site rather than
//! flattening every presence check to one rule. `find_host_extras`,
//! `find_generated_state_dirs`, and `identity_file_present` all answer
//! the blanket-`lexists` question, so every one of their own checks
//! goes through `lexists_or_absent` below, never failing the whole
//! scan over a permission error alone -- and neither does
//! `find_ancestors`, which calls `identity_file_present` and
//! `find_host_extras` for exactly this reason.
//! `read_identity_file`'s own intermediate-segment walk is the other
//! shape: it keeps going through `symlink_metadata_or_absent` and
//! still aborts on an unexpected error there, exactly as before. Only
//! the one check that comes *after* that walk -- the full identity
//! path's own existence, which the prototype's `read_identity` always
//! asks through `os.path.lexists` once every intermediate segment is
//! confirmed to be a real directory -- goes through
//! `lexists_or_absent` too, both in `read_identity_file`'s own
//! symlinked-intermediate branch and in `read_identity_at`.
//! `record_skill` is not a fifth lenient site: the prototype's own
//! walk asks `SKILL_FILE in filenames` against a listing already in
//! hand before ever calling its own `lstat`, so this module gates
//! `record_skill` on `list_subdirectories`'s own listing the same way
//! and then reads the file's metadata through
//! `symlink_metadata_or_absent`, the strict primitive, failing the
//! whole scan on anything other than the entry vanishing in between --
//! see `record_skill`'s own doc comment for the full rule.
//!
//! **A below-root listing failure is skipped, not fatal.** When this
//! module's own recursive walk cannot even list a directory's own
//! entries -- `std::fs::read_dir` itself failing, or failing partway
//! through iterating what it returned -- *below* the scan root, this
//! mirrors the prototype's own `os.walk(root, followlinks=False)`
//! under its default `onerror=None`: that directory's own tuple is never
//! produced, so the directory is neither recorded as a
//! [`DirectoryEntry`] nor inspected for its own `SKILL.md`, the walk
//! never descends into it, and the walk continues with its siblings
//! rather than failing outright. A directory removed between being
//! listed by its parent and being visited itself falls into this same
//! case, indistinguishable from any other listing failure. This is
//! narrower than classifying one already-listed entry's own file
//! type: a failed `FileType` check on a single entry (the prototype's
//! own `entry.is_dir()` raising) only drops that one entry from
//! consideration, exactly as if it were not a directory, and never
//! skips the rest of the listing. None of this touches a *content*
//! read -- an identity file or `SKILL.md` that fails to read still
//! reports `WalkFailed`, per the paragraph above -- and the scan
//! root's own listing failure is unaffected by any of this: it still
//! reports `RootUnreadable`, like every other root-level check above.
//!
//! **Refusing, not lossy, a *reported* non-UTF-8 name.** A directory
//! this walk only passes through on the way to somewhere else never
//! needs a `String` built from its own name at all, so a non-UTF-8
//! name costs nothing there -- it is walked through silently, exactly
//! like one holding nothing of interest. The moment a path must be
//! *reported* instead -- a [`DirectoryEntry::path`], a
//! [`SkillFile::path`], or an
//! [`omnifrons_domain::nesting::Ancestor::path`] -- and some component
//! of it is not valid Unicode, this adapter refuses the whole snapshot
//! with
//! [`omnifrons_app::definition_tree::DefinitionTreeError::PathNotUnicode`]
//! rather than folding two distinct names onto the same lossy
//! replacement text, which could silently misattribute one directory's
//! files to another's displayed path. The prototype takes the
//! opposite, always-succeeds choice: it escapes an undecodable byte
//! (Python's own surrogate-escape handling of `os.fsdecode`) rather
//! than ever refusing over encoding alone.

use std::ffi::OsString;
use std::fs::Metadata;
use std::path::{Path, PathBuf};

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::definition_tree::{
    DefinitionTree, DefinitionTreeError, DirectoryEntry, MAX_DEFINITION_ENTRY_BYTES, PathProbe,
    SkillFile, TargetLocation, TreeSnapshot,
};
use omnifrons_domain::definition::{FileShape, IdentityFile, IdentityKind};
use omnifrons_domain::hosts::{EXTRA_READERS, GENERATED_STATE_DIRS, HOST_CONFIG_DIRS, SKIP_DIRS};
use omnifrons_domain::nesting::Ancestor;

/// The basename every discovered skill file carries (the prototype's own
/// `SKILL_FILE`).
const SKILL_FILE_NAME: &str = "SKILL.md";

/// [`IdentityKind::ALL`] minus [`IdentityKind::ClaudeDir`]
/// (`.claude/CLAUDE.md`): the kinds considered at the directory equal to
/// home, per the prototype's own `identity_kinds_for`.
const ALL_BUT_CLAUDE_DIR: [IdentityKind; 5] = [
    IdentityKind::Agents,
    IdentityKind::Claude,
    IdentityKind::Gemini,
    IdentityKind::Qwen,
    IdentityKind::AgentsOverride,
];

/// The real, OS-backed `DefinitionTree`.
///
/// Carries only the resolved home directory (the prototype's own
/// `user_home()`), used by `identity_kinds_for` to drop
/// `.claude/CLAUDE.md` from consideration at exactly that one
/// directory -- never to stop the walk or the ancestor search there;
/// see this module's own documentation and
/// `omnifrons_app::definition_tree`'s corrected one for why.
#[derive(Debug, Clone)]
pub struct FsDefinitionTree {
    home: Option<PathBuf>,
}

impl FsDefinitionTree {
    /// Builds a tree with `home` as the user's home directory, or no
    /// home rule at all when `home` is `None`. `home`, if given, is
    /// canonicalized so it compares equal to the canonical directories
    /// this adapter's own walk and ancestor search produce from a
    /// canonical [`WorkspaceRoot`]; a `home` that cannot be
    /// canonicalized (for example, a path that does not exist) is
    /// treated the same as `None`, since a home directory this process
    /// cannot even resolve can never equal a real directory on the
    /// walk.
    #[must_use]
    pub fn new(home: Option<PathBuf>) -> Self {
        Self {
            home: home.and_then(|path| std::fs::canonicalize(path).ok()),
        }
    }

    /// Builds a tree whose home directory is read from the process
    /// environment: `HOME` on unix, `USERPROFILE` on windows -- the
    /// same platform split `omnifrons_app::harness_adapter`'s own
    /// `base_env_keys` already documents for this crate's launch-time
    /// allowlist. Absent or unresolvable, this is exactly
    /// [`Self::new`]'s own `None` case: no home rule at all.
    #[must_use]
    pub fn from_environment() -> Self {
        let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        Self::new(std::env::var_os(key).map(PathBuf::from))
    }
}

/// This walk's invariant context, threaded through every recursive step
/// without changing.
struct WalkContext<'a> {
    home: Option<&'a Path>,
}

/// The two lists a walk accumulates, bundled so the recursive helpers
/// below stay under a handful of parameters each.
struct Collected {
    directories: Vec<DirectoryEntry>,
    skills: Vec<SkillFile>,
}

impl DefinitionTree for FsDefinitionTree {
    fn snapshot(&self, root: &WorkspaceRoot) -> Result<TreeSnapshot, DefinitionTreeError> {
        let root_path = root.path();
        let ctx = WalkContext {
            home: self.home.as_deref(),
        };
        let mut collected = Collected {
            directories: Vec::new(),
            skills: Vec::new(),
        };
        visit(root_path, ".", false, true, &ctx, &mut collected)?;
        let ancestors = find_ancestors(root_path, &ctx)?;
        Ok(TreeSnapshot {
            directories: collected.directories,
            skills: collected.skills,
            ancestors,
        })
    }

    /// The lenient `lexists` and `is_symlink` of the prototype: a path
    /// that cannot be read at all is absent, never an error.
    fn probe(
        &self,
        root: &WorkspaceRoot,
        relative: &str,
    ) -> Result<PathProbe, DefinitionTreeError> {
        let path = root.path().join(relative);
        let Some(metadata) = lexists_or_absent(&path) else {
            return Ok(PathProbe::Absent);
        };
        if metadata.file_type().is_symlink() {
            let link = read_link_text(&path, false)?;
            return Ok(PathProbe::Symlink { link });
        }
        Ok(PathProbe::Present)
    }

    /// The canonical path of the directory, as hosts see it (they start
    /// from the physical working directory), and whether it is the home
    /// directory given to [`FsDefinitionTree::new`].
    fn locate(
        &self,
        root: &WorkspaceRoot,
        relative: &str,
    ) -> Result<TargetLocation, DefinitionTreeError> {
        let physical = std::fs::canonicalize(root.path().join(relative))
            .map_err(|_| DefinitionTreeError::RootUnreadable)?;
        let mut components = Vec::new();
        for part in physical.components() {
            if let std::path::Component::Normal(name) = part {
                let name = name.to_str().ok_or(DefinitionTreeError::PathNotUnicode)?;
                components.push(name.to_string());
            }
        }
        let home = self.home.as_deref() == Some(physical.as_path());
        Ok(TargetLocation { components, home })
    }
}

/// Maps an unexpected I/O failure to the error this port reports for it:
/// [`DefinitionTreeError::RootUnreadable`] for the scan root's own
/// checks, [`DefinitionTreeError::WalkFailed`] for everything below it
/// -- see this module's own documentation for why the split is drawn
/// this way.
const fn io_error(is_root: bool) -> DefinitionTreeError {
    if is_root {
        DefinitionTreeError::RootUnreadable
    } else {
        DefinitionTreeError::WalkFailed
    }
}

/// `path`'s metadata without following a symlink at its final
/// component, or `None` when nothing is there at all -- the *strict*
/// of this module's two presence primitives, for a call site that
/// still must fail the scan on anything other than not-found: "absent"
/// (an `io::ErrorKind::NotFound`) and "present but unreadable for some
/// other reason" are never confused here, the first always `None` and
/// the second always [`io_error`]. See [`lexists_or_absent`] for the
/// lenient counterpart a `lexists`-shaped call site needs instead, and
/// this module's own documentation for which sites need which.
fn symlink_metadata_or_absent(
    path: &Path,
    is_root: bool,
) -> Result<Option<Metadata>, DefinitionTreeError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(io_error(is_root)),
    }
}

/// `path`'s metadata without following a symlink at its final
/// component, or `None` when *anything at all* keeps it from being
/// read -- not just an `io::ErrorKind::NotFound`, but a permission
/// error or any other unexpected failure too. The *lenient* of this
/// module's two presence primitives: it never fails the scan, mirroring
/// the prototype's own `os.path.lexists`/`os.path.exists`, which both
/// catch every `OSError` a stat can raise and simply answer "not
/// there". This module's own documentation names exactly which call
/// sites need this lenient check instead of
/// [`symlink_metadata_or_absent`]'s stricter one.
fn lexists_or_absent(path: &Path) -> Option<Metadata> {
    std::fs::symlink_metadata(path).ok()
}

/// `parent`'s own POSIX-relative display path, with `name` appended --
/// `"."` names the scan root, so appending to it never leaves a leading
/// slash.
fn join_relative(parent: &str, name: &str) -> String {
    if parent == "." {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// Visits one directory, pre-order: lists its real, non-pruned
/// subdirectories first ([`list_subdirectories`]). When that listing
/// itself fails anywhere below the scan root, this directory is
/// skipped entirely -- never recorded, never descended into -- the
/// prototype's own `os.walk(root, followlinks=False)` under its
/// default `onerror=None`; see [`list_subdirectories`] for why.
/// Otherwise, it records this directory (`record_directory`), its own
/// `SKILL.md` (`record_skill`), then recurses into every listed
/// subdirectory in sorted name order -- the prototype's own walk plus
/// its `SKIP_DIRS` prune, translated one level of recursion at a time.
///
/// `is_root` is `true` for exactly the call on the scan root itself: a
/// listing failure there is never skipped, and still reports
/// [`DefinitionTreeError::RootUnreadable`]; see [`io_error`] and
/// [`list_subdirectories`] for why every failure at that one level
/// reports differently from a below-root listing failure.
///
/// `relative_display_is_lossy` is `true` once `relative_display` itself
/// -- or any ancestor segment folded into it on the way down from the
/// scan root -- lost information to `to_string_lossy`; `false`
/// for the scan root itself, whose own display path is always the
/// literal `"."`. [`record_directory`] and [`record_skill`] are the
/// only two places this flag ever matters, and only once each has
/// already decided this directory holds something worth reporting; see
/// this module's own documentation for why a directory that is only
/// ever walked through never needs to check it at all.
fn visit(
    absolute: &Path,
    relative_display: &str,
    relative_display_is_lossy: bool,
    is_root: bool,
    ctx: &WalkContext<'_>,
    collected: &mut Collected,
) -> Result<(), DefinitionTreeError> {
    let Some(listing) = list_subdirectories(absolute, is_root)? else {
        return Ok(());
    };

    record_directory(
        absolute,
        relative_display,
        relative_display_is_lossy,
        is_root,
        ctx,
        &mut collected.directories,
    )?;
    record_skill(
        absolute,
        relative_display,
        relative_display_is_lossy,
        listing.skill_listed,
        is_root,
        &mut collected.skills,
    )?;

    for name in listing.subdirectories {
        let child_absolute = absolute.join(&name);
        let child_is_lossy = relative_display_is_lossy || name.to_str().is_none();
        let child_relative = join_relative(relative_display, &name.to_string_lossy());
        visit(
            &child_absolute,
            &child_relative,
            child_is_lossy,
            false,
            ctx,
            collected,
        )?;
    }
    Ok(())
}

/// [`list_subdirectories`]'s own result: `absolute`'s real, non-pruned
/// subdirectory names eligible for recursion, in sorted order,
/// alongside whether [`SKILL_FILE_NAME`] was itself one of this
/// directory's listed entries, by name alone, regardless of that
/// entry's own type. [`record_skill`] reads only
/// [`Self::skill_listed`], reusing the one `read_dir` pass
/// [`list_subdirectories`] already made of this same directory instead
/// of probing the filesystem a second time -- the prototype's own
/// `SKILL_FILE in filenames` membership test against its own walk
/// tuple, translated to this port's two-pass directory/skill recording
/// split.
struct DirectoryListing {
    /// The real, non-pruned subdirectory names, in sorted order.
    subdirectories: Vec<OsString>,
    /// Whether an entry named [`SKILL_FILE_NAME`] was among this
    /// directory's listed entries.
    skill_listed: bool,
}

/// Lists `absolute`'s immediate entries into a [`DirectoryListing`] --
/// or `None` when the listing itself fails *below* the scan root.
///
/// Mirrors `os.walk`'s own default `onerror=None`: a directory this
/// process cannot even open for listing, or whose entry iteration fails
/// partway through (for example because an entry vanished), is simply
/// skipped -- as if it held no entries of interest at all -- rather
/// than aborting the whole walk; see this module's own documentation
/// for the full rule and why it never touches a content read. A failure
/// classifying one specific entry's own file type is narrower still:
/// that one entry alone is treated as "not a directory" (the
/// prototype's own `entry.is_dir()` fallback), which never forces this
/// wider, whole-directory skip, and never keeps that same entry's name
/// from still counting toward [`DirectoryListing::skill_listed`].
///
/// At the scan root itself (`is_root`), any such failure instead
/// becomes `Err(DefinitionTreeError::RootUnreadable)`, matching every
/// other root-level check in this module.
fn list_subdirectories(
    absolute: &Path,
    is_root: bool,
) -> Result<Option<DirectoryListing>, DefinitionTreeError> {
    let Ok(read_dir) = std::fs::read_dir(absolute) else {
        return listing_failed(is_root);
    };
    let mut subdirectories: Vec<OsString> = Vec::new();
    let mut skill_listed = false;
    for entry in read_dir {
        let Ok(entry) = entry else {
            return listing_failed(is_root);
        };
        let name = entry.file_name();
        if name == SKILL_FILE_NAME {
            skill_listed = true;
        }
        // Not following a symlink at this name: a symlink is never a
        // recursion candidate regardless of what it points to, so
        // `FileType::is_dir` here -- which, like `symlink_metadata`,
        // never follows -- already excludes every symlink, whether it
        // targets a file or a directory. A failure classifying this
        // one entry is treated as "not a directory" rather than a
        // reason to abandon the rest of this listing -- see this
        // function's own documentation.
        let is_dir = entry.file_type().is_ok_and(|file_type| file_type.is_dir());
        if !is_dir {
            continue;
        }
        if SKIP_DIRS.contains(&name.to_string_lossy().as_ref()) {
            continue;
        }
        subdirectories.push(name);
    }
    subdirectories.sort();
    Ok(Some(DirectoryListing {
        subdirectories,
        skill_listed,
    }))
}

/// [`list_subdirectories`]'s own failure convention: `Err` at the scan
/// root (`RootUnreadable`, matching every other root-level check),
/// `Ok(None)` -- "skip, do not descend" -- everywhere else, so a caller
/// never repeats the `is_root` branch at each of that listing's two
/// failure points (opening the directory, and iterating its entries).
const fn listing_failed(is_root: bool) -> Result<Option<DirectoryListing>, DefinitionTreeError> {
    if is_root {
        Err(DefinitionTreeError::RootUnreadable)
    } else {
        Ok(None)
    }
}

/// Records `absolute` as a [`DirectoryEntry`] iff it holds at least one
/// identity file or host extra (the port's own widened rule; see
/// `omnifrons_app::definition_tree`'s module documentation) -- never for
/// an empty directory, and never twice for the same path, since each
/// directory is visited exactly once by [`visit`].
///
/// Refuses with [`DefinitionTreeError::PathNotUnicode`] when
/// `relative_display_is_lossy` and this directory turns out to hold
/// something to report: see this module's own "Refusing, not lossy"
/// documentation for why the check happens here, after deciding there
/// is something to report, rather than the moment a non-Unicode name is
/// first seen in [`visit`].
fn record_directory(
    absolute: &Path,
    relative_display: &str,
    relative_display_is_lossy: bool,
    is_root: bool,
    ctx: &WalkContext<'_>,
    out: &mut Vec<DirectoryEntry>,
) -> Result<(), DefinitionTreeError> {
    let kinds = identity_kinds_for(absolute, ctx.home);
    let mut identity_files = Vec::with_capacity(kinds.len());
    for &kind in kinds {
        if let Some(file) = read_identity_file(absolute, kind, is_root)? {
            identity_files.push(file);
        }
    }
    let host_extras = find_host_extras(absolute);
    if identity_files.is_empty() && host_extras.is_empty() {
        return Ok(());
    }
    if relative_display_is_lossy {
        return Err(DefinitionTreeError::PathNotUnicode);
    }
    let generated_state_dirs = find_generated_state_dirs(absolute);
    out.push(DirectoryEntry {
        path: relative_display.to_string(),
        identity_files,
        host_extras,
        generated_state_dirs,
        portable_agent_folder: has_portable_agent_folder(absolute),
    });
    Ok(())
}

/// Records `absolute`'s own `SKILL.md` iff one is there as a regular
/// file -- never a symlink, whatever it targets, and never a directory
/// (the prototype's own `lstat`-based `stat.S_ISREG` check on the
/// walk's `filenames` entry).
///
/// Gated on `skill_listed`, the one `read_dir` pass
/// [`list_subdirectories`] already made of this same directory: the
/// prototype's own walk checks `SKILL_FILE in filenames`, a free test
/// against a listing already in hand, before ever calling its own
/// `lstat`, so this function takes that same listing as its own
/// presence gate instead of probing the filesystem a second time.
/// `skill_listed` false short-circuits here with no filesystem call of
/// this function's own at all -- a directory with no `SKILL.md` is
/// simply absent, needing only the read permission the listing itself
/// already needed. Once `skill_listed` is true, this function reads
/// the entry's metadata through the *strict* [`symlink_metadata_or_absent`],
/// matching the prototype's own `lstat`, which raises on anything but
/// success: an entry the listing just reported but this process cannot
/// then stat -- a permission error on an otherwise
/// listable-but-unsearchable directory, in particular -- fails the
/// whole scan with [`DefinitionTreeError::WalkFailed`] (or
/// [`DefinitionTreeError::RootUnreadable`] at the scan root), never
/// silently folded into "absent". Only the metadata call reporting the
/// entry gone outright -- vanishing in the narrow window between the
/// listing and this stat -- still resolves to absent, the one case
/// [`symlink_metadata_or_absent`] itself treats leniently.
///
/// Refuses with [`DefinitionTreeError::PathNotUnicode`] when
/// `relative_display_is_lossy` and a reportable `SKILL.md` is actually
/// there, for the same reason [`record_directory`] does.
fn record_skill(
    absolute: &Path,
    relative_display: &str,
    relative_display_is_lossy: bool,
    skill_listed: bool,
    is_root: bool,
    out: &mut Vec<SkillFile>,
) -> Result<(), DefinitionTreeError> {
    if !skill_listed {
        return Ok(());
    }
    let path = absolute.join(SKILL_FILE_NAME);
    let Some(metadata) = symlink_metadata_or_absent(&path, is_root)? else {
        return Ok(());
    };
    if !metadata.is_file() {
        return Ok(());
    }
    if relative_display_is_lossy {
        return Err(DefinitionTreeError::PathNotUnicode);
    }
    let bytes = read_capped(&path, &metadata, is_root)?;
    out.push(SkillFile {
        path: join_relative(relative_display, SKILL_FILE_NAME),
        bytes,
    });
    Ok(())
}

/// Reads `path`'s whole content, refusing with
/// [`DefinitionTreeError::EntryTooLarge`] when `metadata`'s reported
/// size exceeds [`MAX_DEFINITION_ENTRY_BYTES`], and again when the bytes
/// actually read do -- the same double bound
/// `crate::fs_project_text_file`'s own `read` applies, so a file grown
/// between the two checks is still refused rather than silently
/// truncated or admitted.
fn read_capped(
    path: &Path,
    metadata: &Metadata,
    is_root: bool,
) -> Result<Vec<u8>, DefinitionTreeError> {
    if metadata.len() > MAX_DEFINITION_ENTRY_BYTES {
        return Err(DefinitionTreeError::EntryTooLarge);
    }
    let bytes = std::fs::read(path).map_err(|_| io_error(is_root))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_DEFINITION_ENTRY_BYTES {
        return Err(DefinitionTreeError::EntryTooLarge);
    }
    Ok(bytes)
}

/// `path`'s symlink target, as its own unresolved text (the prototype's
/// own `os.readlink`); any failure to read it -- the link vanishing
/// between the classifying `symlink_metadata` and this call -- is the
/// same [`io_error`] every other unexpected failure here becomes.
fn read_link_text(path: &Path, is_root: bool) -> Result<String, DefinitionTreeError> {
    std::fs::read_link(path)
        .map(|target| target.to_string_lossy().into_owned())
        .map_err(|_| io_error(is_root))
}

/// Inspects one identity `kind` at `directory`, walking every path
/// segment of [`IdentityKind::file_name`] exactly as the prototype's own
/// `read_identity` does: an intermediate segment that does not exist (or
/// is not a directory) means the kind is absent, and one that is itself
/// a symlink (for example a symlinked `.claude`) reports the kind as
/// [`FileShape::Symlink`] -- carrying that intermediate symlink's own
/// target, never the final file's -- when the full nested path still
/// resolves through it, absent otherwise. "Resolves through it" is
/// deliberately lenient, through [`lexists_or_absent`]: a permission
/// error reaching the full path from a symlinked intermediate -- or,
/// once every intermediate is confirmed a real directory,
/// [`read_identity_at`]'s own first check -- means absent too, never a
/// reason to fail the scan, matching the prototype's own
/// `os.path.lexists` there. An intermediate segment's *own* check stays
/// strict, through [`symlink_metadata_or_absent`]: this module's own
/// documentation explains why the two differ.
fn read_identity_file(
    directory: &Path,
    kind: IdentityKind,
    is_root: bool,
) -> Result<Option<IdentityFile>, DefinitionTreeError> {
    let relative = Path::new(kind.file_name());
    let mut current = directory.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        if components.peek().is_none() {
            // The final component is the file itself; handled below,
            // outside this segment-walking loop.
            break;
        }
        current.push(component);
        let Some(metadata) = symlink_metadata_or_absent(&current, is_root)? else {
            return Ok(None);
        };
        if metadata.file_type().is_symlink() {
            let full_path = directory.join(relative);
            if lexists_or_absent(&full_path).is_none() {
                return Ok(None);
            }
            let target = read_link_text(&current, is_root)?;
            return Ok(Some(IdentityFile {
                kind,
                bytes: Vec::new(),
                shape: FileShape::Symlink { target },
            }));
        }
        if !metadata.is_dir() {
            return Ok(None);
        }
    }
    read_identity_at(&directory.join(relative), kind, is_root)
}

/// Classifies and, when it is a regular file, reads `full_path` -- the
/// final-component half of [`read_identity_file`]'s own walk, once every
/// intermediate segment is known to be a real directory. The initial
/// existence check is deliberately lenient, through
/// [`lexists_or_absent`] rather than [`symlink_metadata_or_absent`]:
/// the prototype's own `read_identity` asks `os.path.lexists` this
/// exact question once its own segment walk reaches this point, so a
/// permission error here -- for example an intermediate directory that
/// exists but cannot be searched -- means absent too, never a reason to
/// fail the scan.
fn read_identity_at(
    full_path: &Path,
    kind: IdentityKind,
    is_root: bool,
) -> Result<Option<IdentityFile>, DefinitionTreeError> {
    let Some(metadata) = lexists_or_absent(full_path) else {
        return Ok(None);
    };
    if metadata.file_type().is_symlink() {
        let target = read_link_text(full_path, is_root)?;
        return Ok(Some(IdentityFile {
            kind,
            bytes: Vec::new(),
            shape: FileShape::Symlink { target },
        }));
    }
    if !metadata.is_file() {
        return Ok(Some(IdentityFile {
            kind,
            bytes: Vec::new(),
            shape: FileShape::Irregular,
        }));
    }
    let bytes = read_capped(full_path, &metadata, is_root)?;
    Ok(Some(IdentityFile {
        kind,
        bytes,
        shape: FileShape::Regular,
    }))
}

/// The [`EXTRA_READERS`] keys that exist at `directory`, in their table
/// order -- existence only (the prototype's own `os.path.lexists`,
/// through [`lexists_or_absent`]), never read, matching
/// `find_host_extras`'s own documented behaviour. A key such as
/// `".claude/AGENTS.md"` joins into a multi-segment path exactly like
/// an identity kind's own [`IdentityKind::file_name`] does, but this
/// function never walks it segment by segment the way
/// [`read_identity_file`] does: the prototype's own `find_host_extras`
/// asks `os.path.lexists` the question in one combined call, so a
/// permission error anywhere along that path -- not just a missing
/// final component -- is this function's own "not there" too, never a
/// reason to fail the scan.
fn find_host_extras(directory: &Path) -> Vec<String> {
    let mut extras = Vec::new();
    for (key, _readers) in EXTRA_READERS {
        if lexists_or_absent(&directory.join(key)).is_some() {
            extras.push((*key).to_string());
        }
    }
    extras
}

/// The prototype's adapter-state folder name (`STATE_DIR`), also one of
/// [`SKIP_DIRS`]' entries.
const PORTABLE_AGENT_DIR: &str = ".portable-agent";

/// The [`GENERATED_STATE_DIRS`] entries that exist directly under
/// `directory`, existence only (through [`lexists_or_absent`], for the
/// same reason [`find_host_extras`] uses it), in their table order.
fn find_generated_state_dirs(directory: &Path) -> Vec<&'static str> {
    let mut found = Vec::new();
    for name in GENERATED_STATE_DIRS {
        if lexists_or_absent(&directory.join(name)).is_some() {
            found.push(*name);
        }
    }
    found
}

/// Whether `directory` holds a real `.portable-agent` directory -- the
/// prototype's adapter-state folder (`STATE_DIR`), where its backups
/// live. One lenient probe ([`lexists_or_absent`], like the other
/// existence checks here), mirroring the prototype's `list_backups`,
/// which only reads below it when it is neither a symlink nor anything
/// but a directory: a symlink, a plain file, or an unreadable entry is
/// not the folder. The walk prunes that folder (`SKIP_DIRS`), so this
/// is the only look anyone takes at it and nothing below it is read.
fn has_portable_agent_folder(directory: &Path) -> bool {
    lexists_or_absent(&directory.join(PORTABLE_AGENT_DIR))
        .is_some_and(|metadata| metadata.file_type().is_dir())
}

/// The identity kinds that make `directory` an agent root, exactly like
/// the prototype's own `identity_kinds_for`: none at all for a
/// [`HOST_CONFIG_DIRS`]-named directory (its identity files belong to
/// its parent instead), every kind but
/// [`IdentityKind::ClaudeDir`] at the directory equal to `home`, and
/// every kind everywhere else.
fn identity_kinds_for(directory: &Path, home: Option<&Path>) -> &'static [IdentityKind] {
    let is_host_config_dir = directory
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|name| HOST_CONFIG_DIRS.contains(&name));
    if is_host_config_dir {
        return &[];
    }
    if home.is_some_and(|home_path| home_path == directory) {
        return &ALL_BUT_CLAUDE_DIR;
    }
    &IdentityKind::ALL
}

/// `path`'s absolute form, written with `/` separators on every
/// platform -- [`Ancestor::path`]'s own documented shape, mirroring
/// `omnifrons_app::definition`'s own private `absolute_posix` convention
/// for the same need. Refuses with
/// [`DefinitionTreeError::PathNotUnicode`] when `path` is not valid
/// Unicode, rather than lossily replacing an unrepresentable byte: see
/// this module's own "Refusing, not lossy" documentation.
/// [`find_ancestors`] is this function's only caller, and only once it
/// has already decided `path` is worth reporting as an [`Ancestor`] at
/// all, so a `path` this process only ever walks through, never
/// reports, never reaches this check.
fn posix_absolute(path: &Path) -> Result<String, DefinitionTreeError> {
    let text = path.to_str().ok_or(DefinitionTreeError::PathNotUnicode)?;
    Ok(text.replace('\\', "/"))
}

/// Whether identity `kind` is present at `directory`, checking its
/// presence and shape only -- never reading or size-capping its bytes,
/// and never failing: this function mirrors the prototype's own
/// `has_identity_files`, which asks a single blanket
/// `os.path.lexists(directory / kind)` and so already answers "absent"
/// for a permission error anywhere along that path, never raising.
/// This function reaches the same answer by walking
/// [`IdentityKind::file_name`] one segment at a time instead -- the
/// same shape [`read_identity_file`] walks -- but every one of its own
/// checks goes through [`lexists_or_absent`], so an intermediate
/// segment that cannot be read is absent here too, not a hard failure:
/// see this module's own "Lenient, not strict" documentation for why
/// this function's segment walk still reaches `has_identity_files`'s
/// lenient answer despite looking like [`read_identity_file`]'s
/// stricter one. An intermediate segment that does not exist, or
/// exists but is not a directory, means the kind is absent, and one
/// that is itself a symlink counts the kind present exactly when the
/// full nested path still resolves through it -- matching
/// [`read_identity_file`]'s own intermediate-symlink rule, without
/// reading that symlink's own target text, since no caller of this
/// function needs it. At the final path, a regular file, a symlink, or
/// an irregular entry all count as present, exactly as
/// [`read_identity_at`] treats them at the entry level -- just without
/// ever calling [`read_capped`]. [`find_ancestors`] is this function's
/// only caller; see its own documentation for why an ancestor check
/// never needs an identity file's content.
fn identity_file_present(directory: &Path, kind: IdentityKind) -> bool {
    let relative = Path::new(kind.file_name());
    let mut current = directory.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        if components.peek().is_none() {
            // The final component is the file itself; handled below,
            // outside this segment-walking loop.
            break;
        }
        current.push(component);
        let Some(metadata) = lexists_or_absent(&current) else {
            return false;
        };
        if metadata.file_type().is_symlink() {
            let full_path = directory.join(relative);
            return lexists_or_absent(&full_path).is_some();
        }
        if !metadata.is_dir() {
            return false;
        }
    }
    lexists_or_absent(&directory.join(relative)).is_some()
}

/// Every ancestor above `root` holding at least one identity file or
/// host extra, nearest first -- the prototype's own `find_ancestors`
/// walking every one of `root.parents`, all the way to the filesystem
/// root; see this module's corrected documentation in
/// `omnifrons_app::definition_tree` for why there is no home-boundary
/// stop. [`identity_file_present`] and [`find_host_extras`] never fail
/// at all -- both are lenient, `lexists`-shaped checks, per this
/// module's own "Lenient, not strict" documentation -- so the only way
/// this function itself can fail is [`posix_absolute`] refusing a
/// non-Unicode ancestor path with
/// [`DefinitionTreeError::PathNotUnicode`]; no ancestor here is ever
/// the scan root itself, but that refusal is not an `is_root` split the
/// way [`DefinitionTreeError::RootUnreadable`] and
/// [`DefinitionTreeError::WalkFailed`] are, since it never depends on
/// which directory is scanned, only on what its own path contains.
///
/// Each kind is checked with [`identity_file_present`] -- presence and
/// shape alone, never [`read_identity_file`] -- because an [`Ancestor`]
/// carries only [`Ancestor::identity_files`] file names, never bytes.
/// The prototype's own `find_ancestors` does read every ancestor
/// identity file's bytes, through its own `read_identity_files`, but
/// only ever keeps the resulting names; this port reads nothing an
/// ancestor check does not need, so an ancestor identity file this
/// process cannot read the content of, or one over
/// [`MAX_DEFINITION_ENTRY_BYTES`], never fails an otherwise-healthy
/// scan.
fn find_ancestors(
    root: &Path,
    ctx: &WalkContext<'_>,
) -> Result<Vec<Ancestor>, DefinitionTreeError> {
    let mut ancestors = Vec::new();
    let mut current = root;
    while let Some(parent) = current.parent() {
        let kinds = identity_kinds_for(parent, ctx.home);
        let mut identity_files = Vec::new();
        for &kind in kinds {
            if identity_file_present(parent, kind) {
                identity_files.push(kind.file_name().to_string());
            }
        }
        let host_extras = find_host_extras(parent);
        if !identity_files.is_empty() || !host_extras.is_empty() {
            ancestors.push(Ancestor {
                path: posix_absolute(parent)?,
                identity_files,
                host_extras,
            });
        }
        current = parent;
    }
    Ok(ancestors)
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use omnifrons_app::definition_tree::DefinitionTreeError;
    use omnifrons_domain::definition::IdentityKind;

    use super::read_identity_file;

    /// A drop-guard temp directory (mirrors
    /// `crate::fs_wrong_root_scanner`'s own inline unit-test helper).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "omnifrons-definition-tree-unit-{}-{label}-{n}",
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

    /// Isolates [`read_identity_file`]'s own split, in a directory this
    /// process can list but not search (unix mode `0o400`, `r--`),
    /// apart from `record_directory`'s unconditional sweep over every
    /// [`IdentityKind`] -- `crates/omnifrons-adapters/tests/definition_tree_fs.rs`'s
    /// own integration test for this same directory shows that combined,
    /// whole-directory outcome (`WalkFailed`, because of
    /// `IdentityKind::ClaudeDir` below) and explains why the two differ.
    /// A single-segment kind (`IdentityKind::Agents`) hits only the
    /// lenient final check this item's fix added, and resolves to
    /// absent (`Ok(None)`); a multi-segment kind
    /// (`IdentityKind::ClaudeDir`) hits its own intermediate segment's
    /// strict check first, and still aborts (`Err(WalkFailed)`),
    /// exactly as before this item's fix.
    ///
    /// Skipped when running as `root`, which bypasses unix permission
    /// bits entirely.
    #[test]
    fn a_single_segment_kind_is_absent_but_a_multi_segment_kind_still_aborts() {
        if nix::unistd::Uid::effective().is_root() {
            eprintln!(
                "skipping a_single_segment_kind_is_absent_but_a_multi_segment_kind_still_aborts: \
                 running as root, which bypasses unix permission bits entirely"
            );
            return;
        }
        let project = TempDir::new("read-identity-file-split");
        let blocked = project.path().join("blocked");
        std::fs::create_dir_all(&blocked).expect("mkdir blocked");
        std::fs::write(blocked.join("AGENTS.md"), b"irrelevant").expect("write AGENTS.md");
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o400))
            .expect("chmod 0o400 (r--: listable, not searchable)");

        let agents_result = read_identity_file(&blocked, IdentityKind::Agents, false);
        let claude_dir_result = read_identity_file(&blocked, IdentityKind::ClaudeDir, false);

        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o700))
            .expect("restore permissions before cleanup");
        assert_eq!(
            agents_result,
            Ok(None),
            "AGENTS.md has no intermediate segment, so it hits only read_identity_at's \
             own lenient final check: absent, never a hard failure"
        );
        assert_eq!(
            claude_dir_result,
            Err(DefinitionTreeError::WalkFailed),
            ".claude/CLAUDE.md's own .claude intermediate segment still goes through \
             the strict symlink_metadata_or_absent, unaffected by this item's fix"
        );
    }
}
