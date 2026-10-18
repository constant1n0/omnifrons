//! Nested agent-root relations (ADR-0005 sub-slice 1d).
//!
//! `docs/adr/0005-agent-identity-and-portable-definition.md` decides
//! that nested agents are nested project roots: when one agent root
//! sits inside another, the inner root is adapted only with its own
//! consent, never folded into its enclosing root's adaptation. This
//! module is the pure relation layer behind that decision -- given the
//! roots a port has already discovered inside one scan, plus the
//! ancestor roots a port has already found above it, it works out which
//! root is whose parent, which roots nest under which, and the warning
//! chain a port walks to flag exposure.
//!
//! ## Boundary
//!
//! This module never touches a filesystem and never decides which
//! directories count as roots in the first place. Walking the
//! directory tree, pruning skip and host-config directories (this
//! crate's [`crate::hosts`] already holds `SKIP_DIRS`,
//! `HOST_CONFIG_DIRS`, and `GENERATED_STATE_DIRS` for that),
//! attributing a host-config directory's identity files to its parent
//! root, and stopping the ancestor walk at the user's home directory
//! all belong to the `DefinitionTree` port and its adapter (sub-slices
//! 1e/1f). Two of the prototype's own tests stay there for exactly that
//! reason: its `test_scan_prunes_ignored_and_host_config_dirs` and
//! `test_home_user_memory_is_not_an_ancestor` assert on the walk and the
//! home boundary, not on the relation this module computes.
//!
//! ## Model
//!
//! Every path handled here is POSIX-style and relative to one scan
//! root: `"."` names the scan root itself, and a well-formed path is
//! never empty, never carries a leading `/`, a `..` component, or a
//! bare `.` component, and never doubles or trails its `/`
//! separators. This module normalizes nothing -- a port hands in
//! already-clean paths -- and [`relate`], its one fallible entry
//! point, rejects a malformed root through [`NestingError`] rather
//! than guessing at an answer.
//!
//! The functions below mirror the prototype's by name: [`nearest_root`]
//! is its `nearest_root`; [`relate`] is the parent/nested assignment
//! from its `scan()`; [`ancestor_chain`] is its `ancestor_chain`; and
//! [`warning_chain`] composes that chain with the outside ancestors the
//! way its warnings walk does.

/// Why a [`relate`] argument was rejected: a root/path must be a
/// clean, scan-root-relative POSIX path -- never empty, never
/// absolute, never carrying a `..` or a bare `.` component, never
/// doubled- or trailing-slashed, and never prefixed with a redundant
/// `./` (the scan root is spelled exactly `.`, never as a prefix of
/// another path). Every entry of `roots` must also be distinct from
/// every other -- `relate` rejects a root repeated anywhere in the
/// slice. An ancestor's own path (see [`Ancestor`]) is outside this
/// precondition entirely, since it names a location outside the scan
/// root and so cannot be written as a clean relative path without a
/// leading `..` of its own.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NestingError {
    /// The path starts with `/`, so it cannot be relative to the scan
    /// root.
    #[error("{0} is an absolute path")]
    AbsolutePath(String),
    /// One of the path's components is `..`, which this module's
    /// callers never need: a port hands in paths already resolved
    /// against the scan root.
    #[error("{0} contains a `..` component")]
    ParentComponent(String),
    /// The path is empty: there is nothing to resolve against the
    /// scan root.
    #[error("the root path is empty")]
    EmptyPath,
    /// The path starts with a redundant `./`: the scan root is spelled
    /// exactly `.`, never as a prefix of another path.
    #[error("{0} starts with a redundant `./`; the scan root is spelled `.`")]
    DotSlashPrefix(String),
    /// The path ends with a `/`, which a clean relative path never
    /// needs.
    #[error("{0} ends with a trailing `/`")]
    TrailingSlash(String),
    /// Two of the path's components are separated by an empty one --
    /// a doubled `/`.
    #[error("{0} contains an empty component (a doubled `/`)")]
    DoubledSlash(String),
    /// One of the path's components is a bare `.`, which is only
    /// meaningful as the whole path (the scan root itself); a port
    /// hands in paths already resolved against it.
    #[error("{0} contains a `.` component")]
    DotComponent(String),
    /// The same root appears more than once in `roots`: a repeat can
    /// never be assigned a single, unambiguous parent and nested list.
    #[error("{0} is a duplicate root")]
    DuplicateRoot(String),
}

/// A directory above the scan root holding an identity file or a host
/// extra, outside every root [`relate`] was given -- the prototype's
/// `Ancestor`. An extras-only ancestor (empty
/// [`Self::identity_files`]) is never an agent, exactly as an
/// extras-only directory inside the scan is never a root for
/// [`relate`]'s own purposes; see [`ChainLink::Extras`] and
/// [`warning_chain`]'s own doc comment for the inside-the-scan
/// counterpart. Unlike `roots`/`path` arguments, `path` here is not
/// constrained to a clean, scan-root-relative form: the prototype
/// stores and serializes it as a plain `str(Path)` (an absolute
/// filesystem path in practice), so this module carries it the same
/// way, opaquely, and places no shape requirement on it -- it may be
/// absolute or `../`-relative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ancestor {
    /// The ancestor's own location, opaque to this module (see above).
    pub path: String,
    /// The kinds of identity file found there, in file-name form, in
    /// the port's order. The prototype keys its own `files` mapping by
    /// the file-name form (`"AGENTS.md"`, `"CLAUDE.md"`, ...), so a
    /// value written that way is this field's canonical,
    /// prototype-shaped form; a consumer may also accept the identity
    /// kind's token form (for example `"agents"`) for the same entry --
    /// `crate::definition`'s own warnings composition, in the
    /// `omnifrons-app` crate, does both. This module itself never
    /// parses or validates either form -- it carries whatever a port
    /// supplies opaquely, exactly like `path` above.
    pub identity_files: Vec<String>,
    /// The host-specific extra file names relevant to a host's upward
    /// or fallback walk, found at this ancestor (for example
    /// `CLAUDE.local.md`, one of Claude Code's own
    /// `HostLoader::files`, or `.claude/AGENTS.md`, one of its
    /// `HostLoader::fallback_files`) -- never an identity file itself,
    /// since neither is one of `IdentityKind`'s own kinds.
    /// `crate::definition`'s own warnings composition matches a host's
    /// kinds against this field and [`Self::identity_files`] together,
    /// the union a real ancestor's relevant files may be split across.
    /// This module itself never parses, validates, or reads this
    /// field's entries -- it carries whatever a port supplies opaquely,
    /// exactly like [`Self::path`] and [`Self::identity_files`] above;
    /// ADR-0005 sub-slice 1f's filesystem adapter fills both fields.
    pub host_extras: Vec<String>,
}

/// Which kind of path became a root's parent in a [`RootRelation`]: one
/// of the same scan's own roots, or an ancestor outside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parent {
    /// A root [`relate`] also discovered, named by its relative path
    /// (one entry of its `roots` argument).
    Root(String),
    /// An ancestor above the scan root and outside every discovered
    /// root, named by its own path (the nearest entry of [`relate`]'s
    /// `outer` argument; see [`relate`]'s documentation for why nearest
    /// and not outermost).
    Outside(String),
}

/// One discovered root's relation to the others [`relate`] was given:
/// its parent, if any, and the roots nested directly under it.
/// Corresponds to the prototype's `AgentReport.parent`/`.nested`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootRelation {
    /// This root's own relative path (one entry of [`relate`]'s `roots`
    /// argument).
    pub root: String,
    /// This root's parent, or `None` when it has none inside the scan
    /// or outside it.
    pub parent: Option<Parent>,
    /// The roots whose nearest enclosing root is this one, in the order
    /// [`relate`] was given them.
    pub nested: Vec<String>,
}

impl RootRelation {
    /// Whether adapting this root needs its parent's consent: the
    /// prototype derives `consent_required` from `self.parent is not
    /// None` in `AgentReport.to_json`, true whenever there is any
    /// parent at all -- inside the scan or outside it.
    #[must_use]
    pub fn consent_required(&self) -> bool {
        self.parent.is_some()
    }
}

/// One link in a [`warning_chain`]: a root the same scan also
/// discovered, a directory inside the scan that is never a root, or an
/// ancestor outside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainLink {
    /// A root inside the scan, named by its relative path.
    Root(String),
    /// A directory inside the scan with no identity file of its own --
    /// so [`relate`] never makes it a root, a nesting parent, or
    /// anything needing consent -- but that still holds a host-specific
    /// extra file relevant to some host's upward or fallback walk (for
    /// example `.claude/AGENTS.md`). Named by its relative path exactly
    /// like [`Self::Root`]. An extension beyond the prototype, which
    /// has no concept of a chain member that is not a root at all; see
    /// [`warning_chain`]'s own doc comment.
    Extras(String),
    /// An ancestor outside the scan, with its identity files.
    Outside(Ancestor),
}

/// Splits a well-formed relative path into its components, with `"."`
/// (the scan root) represented as zero components.
fn components(path: &str) -> Vec<&str> {
    if path == "." {
        Vec::new()
    } else {
        path.split('/').collect()
    }
}

/// Whether `prefix`'s components are a prefix of (or equal to)
/// `whole`'s -- the prototype's `is_within`, read the other way round:
/// a root "contains" a path when the root equals the path or the root
/// is one of the path's ancestor directories. An empty `prefix` (the
/// scan root, `"."`) is a prefix of every `whole`, so the scan root
/// contains everything.
fn is_prefix(prefix: &[&str], whole: &[&str]) -> bool {
    whole.starts_with(prefix)
}

/// The parent of a well-formed relative path, in this module's own
/// convention: the parent of a top-level entry is `"."`, and `"."`
/// itself (the scan root) has no parent.
fn parent_of(path: &str) -> Option<&str> {
    if path == "." {
        return None;
    }
    path.rfind('/').map_or(Some("."), |index| path.get(..index))
}

/// The deepest root in `roots` enclosing `root`'s own parent directory,
/// if any. Shared by [`relate`] and [`ancestor_chain`] so both derive
/// the same single enclosing-root step from `parent_of` and
/// [`nearest_root`].
fn inner_parent<'a>(roots: &[&'a str], root: &str) -> Option<&'a str> {
    parent_of(root).and_then(|parent_path| nearest_root(roots, parent_path))
}

/// Rejects a root/path argument that is not a clean, scan-root-relative
/// POSIX path: empty, absolute, carrying a `..` or a bare `.`
/// component, doubled- or trailing-slashed, or prefixed with a
/// redundant `./`. Every other shape is accepted unexamined, exactly as
/// this module's precondition promises -- a port hands in already-clean
/// paths.
fn validate_relative(path: &str) -> Result<(), NestingError> {
    if path.starts_with('/') {
        return Err(NestingError::AbsolutePath(path.to_string()));
    }
    if path.is_empty() {
        return Err(NestingError::EmptyPath);
    }
    if path == "." {
        return Ok(());
    }
    if path.starts_with("./") {
        return Err(NestingError::DotSlashPrefix(path.to_string()));
    }
    if path.ends_with('/') {
        return Err(NestingError::TrailingSlash(path.to_string()));
    }
    for part in path.split('/') {
        match part {
            "" => return Err(NestingError::DoubledSlash(path.to_string())),
            ".." => return Err(NestingError::ParentComponent(path.to_string())),
            "." => return Err(NestingError::DotComponent(path.to_string())),
            _ => {}
        }
    }
    Ok(())
}

/// The deepest root in `roots` that contains `path` (or is `path`),
/// exactly as the prototype's `nearest_root`: `"."` contains every
/// path, and when more than one root contains `path`, the one with the
/// most components wins.
///
/// `path` carries the same scan-root-relative precondition as
/// [`relate`]'s `roots` entries, but this function is non-fallible and
/// never checks it -- a port hands in already-clean paths. Given an
/// absolute `path` instead, `"."`'s own empty component list is still
/// a prefix of anything, so a `roots` slice that includes `"."`
/// reports it as the containing root rather than rejecting the input;
/// a `roots` slice without `"."` reports `None`, since no other
/// root's components can prefix a leading empty one. Callers that
/// cannot guarantee the precondition should validate through
/// [`relate`] first.
#[must_use]
pub fn nearest_root<'a>(roots: &[&'a str], path: &str) -> Option<&'a str> {
    let path_components = components(path);
    let mut best: Option<(&'a str, usize)> = None;
    for &root in roots {
        let root_components = components(root);
        if !is_prefix(&root_components, &path_components) {
            continue;
        }
        let depth = root_components.len();
        if best.is_none_or(|(_, best_depth)| depth > best_depth) {
            best = Some((root, depth));
        }
    }
    best.map(|(root, _)| root)
}

/// Relates every root in `roots` to its parent, if any, and its nested
/// children -- the parent/nested assignment from the prototype's
/// `scan()`.
///
/// Every root's parent, including `"."` (the scan root itself), is
/// the deepest *other* root in `roots` enclosing its own parent
/// directory, if one exists ("inner"); otherwise the parent is the
/// *nearest* ancestor outside the scan -- `outer`'s first entry. The
/// prototype computes this the same way: `outer_parent` is the first of
/// its nearest-first `ancestors`, and a root's parent falls back to
/// `outer_parent` only when it has no inner one. Because
/// `find_ancestors` returns its ancestors nearest first, the nearest
/// outside ancestor becomes the parent -- never the outermost one.
///
/// `"."` never has an inner parent of its own -- `parent_of` returns
/// `None` for `"."`, so its only possible parent is the
/// `outer_parent` fallback above. That means `"."` gets `parent:
/// None` only when `outer` is empty; given a non-empty `outer`,
/// scanning a nested root alone (`roots = ["."]`) reports `"."`
/// itself as needing that nearest outside ancestor's consent, exactly
/// like any other root with no inner parent.
///
/// `nested` lists, for each root, every other root whose own inner
/// parent (as just defined) is this one, in the order `roots` was
/// given -- the same membership the prototype's own nested-list append
/// builds, walked in `roots`' own order rather than a directory walk's.
///
/// # Errors
///
/// Returns [`NestingError`] if any entry of `roots` is not a clean,
/// scan-root-relative POSIX path, or if the same root appears more
/// than once in `roots`. `outer`'s ancestor paths are never checked
/// this way; see [`Ancestor`].
pub fn relate(roots: &[&str], outer: &[Ancestor]) -> Result<Vec<RootRelation>, NestingError> {
    for &root in roots {
        validate_relative(root)?;
    }
    for (index, &root) in roots.iter().enumerate() {
        if roots[..index].contains(&root) {
            return Err(NestingError::DuplicateRoot(root.to_string()));
        }
    }

    let outer_parent = outer.first().map(|ancestor| ancestor.path.as_str());
    let parents: Vec<Option<Parent>> = roots
        .iter()
        .map(|&root| {
            inner_parent(roots, root)
                .map(|inner| Parent::Root(inner.to_string()))
                .or_else(|| outer_parent.map(|path| Parent::Outside(path.to_string())))
        })
        .collect();

    Ok(roots
        .iter()
        .zip(parents.iter())
        .map(|(&root, parent)| {
            let nested = roots
                .iter()
                .zip(parents.iter())
                .filter_map(|(&candidate, candidate_parent)| {
                    let is_child = matches!(
                        candidate_parent,
                        Some(Parent::Root(inner)) if inner.as_str() == root
                    );
                    is_child.then(|| candidate.to_string())
                })
                .collect();
            RootRelation {
                root: root.to_string(),
                parent: parent.clone(),
                nested,
            }
        })
        .collect())
}

/// The roots in `roots` enclosing `root`, nearest first -- the
/// prototype's `ancestor_chain`, which walks a root's parent chain
/// while it stays inside the scan. This module recomputes each step
/// with `inner_parent` instead of reading a stored `parent` field, so
/// it never needs a pre-built [`RootRelation`] list; the walk still
/// terminates for the same reason the prototype's does -- each step's
/// root is strictly shallower than the last -- and it stops exactly
/// where the prototype's own inside-the-scan check would turn false.
///
/// `root` inherits [`nearest_root`]'s scan-root-relative precondition:
/// an absolute `root` is never rejected here either, since every step
/// still routes through `nearest_root`.
#[must_use]
pub fn ancestor_chain<'a>(roots: &[&'a str], root: &str) -> Vec<&'a str> {
    let mut chain = Vec::new();
    let mut current = inner_parent(roots, root);
    while let Some(next) = current {
        chain.push(next);
        current = inner_parent(roots, next);
    }
    chain
}

/// The inside-the-scan portion of a [`warning_chain`]: every root in
/// `roots` and every extras-only directory in `extras_only` enclosing
/// `root`, nearest first, each tagged by which set it came from.
/// Walks the combined set with the same `inner_parent` step
/// [`ancestor_chain`] uses, but never calls [`ancestor_chain`] itself:
/// that function's own roots-only contract stays exactly as documented,
/// for [`relate`]'s own parent/nested wiring, which must never treat an
/// extras-only directory as a root. The two sets are assumed disjoint --
/// a directory with any identity file belongs to `roots`, never to
/// `extras_only` -- and both inherit the same scan-root-relative
/// precondition as [`relate`]'s own `roots` argument.
fn inside_chain(roots: &[&str], extras_only: &[&str], root: &str) -> Vec<ChainLink> {
    let mut combined: Vec<&str> = Vec::with_capacity(roots.len() + extras_only.len());
    combined.extend_from_slice(roots);
    combined.extend_from_slice(extras_only);

    let mut chain = Vec::new();
    let mut current = inner_parent(&combined, root);
    while let Some(next) = current {
        chain.push(if roots.contains(&next) {
            ChainLink::Root(next.to_string())
        } else {
            ChainLink::Extras(next.to_string())
        });
        current = inner_parent(&combined, next);
    }
    chain
}

/// The full warning chain for `root`: `inside_chain`'s roots and
/// extras-only directories enclosing it, nearest first, followed by
/// `outer`'s ancestors outside the scan, in `outer`'s own order -- the
/// prototype composes its own warning chain the same way, for the
/// roots-only part alone.
///
/// `extras_only` is this function's own extension beyond the prototype,
/// which has no concept of a chain member that is not a root at all:
/// the prototype's `walk_workspace` only ever discovers a directory as
/// an agent root (`has_identity_files`) or not, and both its
/// `ancestor_chain` and `find_ancestors` walk agent roots exclusively,
/// so a directory holding only a host-specific extra -- never an
/// identity file -- is invisible to it everywhere: not a root, not a
/// parent, not a chain member. `crate::warnings`'s own module
/// documentation already extends Claude Code's fallback decision to
/// match a chain ancestor's host extras the same way its identity files
/// are matched (its own `ancestor_has_any_kind`); `extras_only` is what
/// lets a directory with no identity file reach that same,
/// already-extended check too, instead of staying invisible to it the
/// way the prototype would leave it.
///
/// An outside [`Ancestor`] with no identity files but a host extra
/// already reaches the chain without any help from `extras_only`:
/// `outer`'s entries are appended unconditionally below, never filtered
/// on whether [`Ancestor::identity_files`] is empty.
///
/// `root` inherits the same scan-root-relative precondition
/// transitively, through `inside_chain`.
#[must_use]
pub fn warning_chain(
    roots: &[&str],
    root: &str,
    extras_only: &[&str],
    outer: &[Ancestor],
) -> Vec<ChainLink> {
    let mut chain = inside_chain(roots, extras_only, root);
    chain.extend(outer.iter().cloned().map(ChainLink::Outside));
    chain
}
