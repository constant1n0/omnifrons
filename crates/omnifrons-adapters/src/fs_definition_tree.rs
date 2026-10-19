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
//! this module performs *for the scan root's own directory* -- its own
//! identity files, host extras, generated-state directories, `SKILL.md`,
//! and its own listing -- reports
//! [`omnifrons_app::definition_tree::DefinitionTreeError::RootUnreadable`]
//! on any unexpected failure; the identical failure anywhere *below*
//! the root reports
//! [`omnifrons_app::definition_tree::DefinitionTreeError::WalkFailed`]
//! instead -- except a below-root directory *listing* failure, which
//! the next paragraph carves out. A root directory a caller cannot
//! search at all (unix mode `0o000`) fails every one of those
//! root-level operations the same way, for the same underlying reason,
//! so this split reports it uniformly as "the scan root cannot be
//! read" regardless of which specific check happens to hit the
//! permission error first.
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
//! **Lossy, not refusing, non-UTF-8 names.** `TreeSnapshot`'s paths are
//! plain `String`s; an entry name that is not valid UTF-8 is carried
//! through `to_string_lossy` rather than failing the walk, matching
//! this crate's existing `fs_outbox_inventory` precedent for a
//! best-effort name rather than a hard refusal over encoding alone.

use std::ffi::OsString;
use std::fs::Metadata;
use std::path::{Path, PathBuf};

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::definition_tree::{
    DefinitionTree, DefinitionTreeError, DirectoryEntry, MAX_DEFINITION_ENTRY_BYTES, SkillFile,
    TreeSnapshot,
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
        visit(root_path, ".", true, &ctx, &mut collected)?;
        let ancestors = find_ancestors(root_path, &ctx)?;
        Ok(TreeSnapshot {
            directories: collected.directories,
            skills: collected.skills,
            ancestors,
        })
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
/// component, or `None` when nothing is there at all -- the shared
/// primitive every existence and shape check in this module goes
/// through, so "absent" (an `io::ErrorKind::NotFound`) and "present but
/// unreadable for some other reason" are never confused: the first
/// means "not found" everywhere this adapter's own prototype
/// counterpart would use `os.path.lexists`, and the second always
/// becomes [`io_error`].
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
fn visit(
    absolute: &Path,
    relative_display: &str,
    is_root: bool,
    ctx: &WalkContext<'_>,
    collected: &mut Collected,
) -> Result<(), DefinitionTreeError> {
    let Some(candidates) = list_subdirectories(absolute, is_root)? else {
        return Ok(());
    };

    record_directory(
        absolute,
        relative_display,
        is_root,
        ctx,
        &mut collected.directories,
    )?;
    record_skill(absolute, relative_display, is_root, &mut collected.skills)?;

    for name in candidates {
        let child_absolute = absolute.join(&name);
        let child_relative = join_relative(relative_display, &name.to_string_lossy());
        visit(&child_absolute, &child_relative, false, ctx, collected)?;
    }
    Ok(())
}

/// Lists `absolute`'s immediate real, non-pruned subdirectories eligible
/// for recursion, in sorted name order -- or `None` when the listing
/// itself fails *below* the scan root.
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
/// wider, whole-directory skip.
///
/// At the scan root itself (`is_root`), any such failure instead
/// becomes `Err(DefinitionTreeError::RootUnreadable)`, matching every
/// other root-level check in this module.
fn list_subdirectories(
    absolute: &Path,
    is_root: bool,
) -> Result<Option<Vec<OsString>>, DefinitionTreeError> {
    let Ok(read_dir) = std::fs::read_dir(absolute) else {
        return listing_failed(is_root);
    };
    let mut candidates: Vec<OsString> = Vec::new();
    for entry in read_dir {
        let Ok(entry) = entry else {
            return listing_failed(is_root);
        };
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
        let name = entry.file_name();
        if SKIP_DIRS.contains(&name.to_string_lossy().as_ref()) {
            continue;
        }
        candidates.push(name);
    }
    candidates.sort();
    Ok(Some(candidates))
}

/// [`list_subdirectories`]'s own failure convention: `Err` at the scan
/// root (`RootUnreadable`, matching every other root-level check),
/// `Ok(None)` -- "skip, do not descend" -- everywhere else, so a caller
/// never repeats the `is_root` branch at each of that listing's two
/// failure points (opening the directory, and iterating its entries).
const fn listing_failed(is_root: bool) -> Result<Option<Vec<OsString>>, DefinitionTreeError> {
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
fn record_directory(
    absolute: &Path,
    relative_display: &str,
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
    let host_extras = find_host_extras(absolute, is_root)?;
    if identity_files.is_empty() && host_extras.is_empty() {
        return Ok(());
    }
    let generated_state_dirs = find_generated_state_dirs(absolute, is_root)?;
    out.push(DirectoryEntry {
        path: relative_display.to_string(),
        identity_files,
        host_extras,
        generated_state_dirs,
    });
    Ok(())
}

/// Records `absolute`'s own `SKILL.md` iff one is there as a regular
/// file -- never a symlink, whatever it targets, and never a directory
/// (the prototype's own `lstat`-based `stat.S_ISREG` check on the
/// walk's `filenames` entry).
fn record_skill(
    absolute: &Path,
    relative_display: &str,
    is_root: bool,
    out: &mut Vec<SkillFile>,
) -> Result<(), DefinitionTreeError> {
    let path = absolute.join(SKILL_FILE_NAME);
    let Some(metadata) = symlink_metadata_or_absent(&path, is_root)? else {
        return Ok(());
    };
    if !metadata.is_file() {
        return Ok(());
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
/// resolves through it, absent otherwise.
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
            if symlink_metadata_or_absent(&full_path, is_root)?.is_none() {
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
/// intermediate segment is known to be a real directory.
fn read_identity_at(
    full_path: &Path,
    kind: IdentityKind,
    is_root: bool,
) -> Result<Option<IdentityFile>, DefinitionTreeError> {
    let Some(metadata) = symlink_metadata_or_absent(full_path, is_root)? else {
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
/// order -- existence only (the prototype's own `os.path.lexists`),
/// never read, matching `find_host_extras`'s own documented behaviour.
fn find_host_extras(directory: &Path, is_root: bool) -> Result<Vec<String>, DefinitionTreeError> {
    let mut extras = Vec::new();
    for (key, _readers) in EXTRA_READERS {
        if symlink_metadata_or_absent(&directory.join(key), is_root)?.is_some() {
            extras.push((*key).to_string());
        }
    }
    Ok(extras)
}

/// The [`GENERATED_STATE_DIRS`] entries that exist directly under
/// `directory`, existence only, in their table order.
fn find_generated_state_dirs(
    directory: &Path,
    is_root: bool,
) -> Result<Vec<&'static str>, DefinitionTreeError> {
    let mut found = Vec::new();
    for name in GENERATED_STATE_DIRS {
        if symlink_metadata_or_absent(&directory.join(name), is_root)?.is_some() {
            found.push(*name);
        }
    }
    Ok(found)
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
/// for the same need.
fn posix_absolute(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Every ancestor above `root` holding at least one identity file or
/// host extra, nearest first -- the prototype's own `find_ancestors`
/// walking every one of `root.parents`, all the way to the filesystem
/// root; see this module's corrected documentation in
/// `omnifrons_app::definition_tree` for why there is no home-boundary
/// stop. No ancestor here is ever the scan root itself, so every
/// failure below always reports [`DefinitionTreeError::WalkFailed`],
/// never [`DefinitionTreeError::RootUnreadable`].
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
            if read_identity_file(parent, kind, false)?.is_some() {
                identity_files.push(kind.file_name().to_string());
            }
        }
        let host_extras = find_host_extras(parent, false)?;
        if !identity_files.is_empty() || !host_extras.is_empty() {
            ancestors.push(Ancestor {
                path: posix_absolute(parent),
                identity_files,
                host_extras,
            });
        }
        current = parent;
    }
    Ok(ancestors)
}
