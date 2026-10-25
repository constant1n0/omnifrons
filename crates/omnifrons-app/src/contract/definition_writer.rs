//! An in-memory `DefinitionWriter` (ADR-0005 sub-slice 2c) for the apply
//! service's tests: files live in a map, every accepted write is
//! recorded in order, and the same path and expected-fingerprint rules
//! as the filesystem adapter apply, so a test sees the refusals a real
//! disk would give. Digests use `FakeHasher`.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use omnifrons_domain::definition_plan::InputFingerprint;
use omnifrons_domain::executable::Sha256Digest;

use crate::content_hasher::ContentHasher;
use crate::contract::publication::FakeHasher;
use crate::definition_writer::{
    DefinitionWriteError, DefinitionWriter, ModePolicy, WriteRequest, validate_write_path,
};
use crate::harness_adapter::WorkspaceRoot;

/// What the double holds at one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeEntry {
    /// A regular file.
    File(Vec<u8>),
    /// A symbolic link with this target text.
    Symlink(String),
    /// Something that is neither (a directory, a socket, ...).
    Irregular,
}

/// One accepted write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRecord {
    /// The workspace-relative path written.
    pub path: String,
    /// The bytes published.
    pub bytes: Vec<u8>,
    /// What the plan recorded there.
    pub expected: InputFingerprint,
    /// The permission rule the caller asked for.
    pub mode: ModePolicy,
}

#[derive(Debug, Default)]
struct State {
    entries: BTreeMap<String, FakeEntry>,
    directories: Vec<String>,
    writes: Vec<WriteRecord>,
}

impl State {
    /// Whether `dir` is a seeded directory, or the ancestor of one or
    /// of any entry.
    fn is_directory(&self, dir: &str) -> bool {
        let below = format!("{dir}/");
        self.directories
            .iter()
            .chain(self.entries.keys())
            .any(|path| path == dir || path.starts_with(&below))
    }
}

/// The in-memory writer. One workspace: the `root` argument is ignored.
///
/// Parents follow the filesystem adapter's rule: a parent directory must
/// be seeded ([`Self::with_directory`]) or be an ancestor of a seeded or
/// written entry, otherwise the write refuses with `parent-missing`; a
/// parent that exists as a link or any other entry refuses with
/// `not-regular`. The workspace root itself always exists.
#[derive(Debug, Default)]
pub struct InMemoryDefinitionWriter {
    state: Mutex<State>,
}

impl InMemoryDefinitionWriter {
    /// An empty writer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Seeds a regular file at `path`.
    #[must_use]
    pub fn with_file(self, path: &str, bytes: &[u8]) -> Self {
        self.put(path, FakeEntry::File(bytes.to_vec()));
        self
    }

    /// Seeds a symbolic link at `path`.
    #[must_use]
    pub fn with_symlink(self, path: &str, target: &str) -> Self {
        self.put(path, FakeEntry::Symlink(target.to_string()));
        self
    }

    /// Seeds an irregular entry (for example a directory) at `path`.
    #[must_use]
    pub fn with_irregular(self, path: &str) -> Self {
        self.put(path, FakeEntry::Irregular);
        self
    }

    /// Seeds an empty directory at `path` (its ancestors too). A write
    /// needs its parent to exist: a seeded directory, or an ancestor of
    /// a seeded or written entry; any other parent refuses with
    /// `parent-missing`, as the filesystem adapter does.
    #[must_use]
    pub fn with_directory(self, path: &str) -> Self {
        self.lock().directories.push(path.to_string());
        self
    }

    /// Replaces whatever is at `path`, as a concurrent editor would,
    /// without recording a write.
    pub fn put(&self, path: &str, entry: FakeEntry) {
        self.lock().entries.insert(path.to_string(), entry);
    }

    /// The bytes of the regular file at `path`, if there is one.
    #[must_use]
    pub fn bytes(&self, path: &str) -> Option<Vec<u8>> {
        match self.lock().entries.get(path) {
            Some(FakeEntry::File(bytes)) => Some(bytes.clone()),
            _ => None,
        }
    }

    /// Every accepted write, in order.
    #[must_use]
    pub fn writes(&self) -> Vec<WriteRecord> {
        self.lock().writes.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn fingerprint(entry: Option<&FakeEntry>) -> InputFingerprint {
    match entry {
        None => InputFingerprint::Absent,
        Some(FakeEntry::File(bytes)) => InputFingerprint::Sha256(FakeHasher.sha256(bytes)),
        Some(FakeEntry::Symlink(target)) => InputFingerprint::Symlink(target.clone()),
        Some(FakeEntry::Irregular) => InputFingerprint::Irregular,
    }
}

impl DefinitionWriter for InMemoryDefinitionWriter {
    fn write(
        &self,
        _root: &WorkspaceRoot,
        request: &WriteRequest<'_>,
    ) -> Result<Sha256Digest, DefinitionWriteError> {
        let components = validate_write_path(request.path)?;
        let path = request.path.to_string();
        if matches!(
            request.expected,
            InputFingerprint::Symlink(_) | InputFingerprint::Irregular
        ) {
            return Err(DefinitionWriteError::NotWritable { path });
        }
        let mut state = self.lock();
        // Every parent prefix must be a real directory: seeded as one, or
        // the ancestor of a seeded or written entry.
        let mut prefix = String::new();
        for component in &components[..components.len() - 1] {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            match state.entries.get(&prefix) {
                Some(_) => return Err(DefinitionWriteError::NotRegular { path }),
                None if state.is_directory(&prefix) => {}
                None => return Err(DefinitionWriteError::ParentMissing { path }),
            }
        }
        let current = state.entries.get(request.path);
        if matches!(request.expected, InputFingerprint::Sha256(_))
            && matches!(current, Some(FakeEntry::Symlink(_) | FakeEntry::Irregular))
        {
            return Err(DefinitionWriteError::NotRegular { path });
        }
        let found = fingerprint(current);
        if &found != request.expected {
            return Err(DefinitionWriteError::Stale {
                path,
                expected: request.expected.clone(),
                found,
            });
        }
        if let ModePolicy::NoWiderThan { source } = request.mode {
            validate_write_path(source).map_err(|_| DefinitionWriteError::SourceUnavailable {
                path: source.clone(),
            })?;
            if !matches!(state.entries.get(source), Some(FakeEntry::File(_))) {
                return Err(DefinitionWriteError::SourceUnavailable {
                    path: source.clone(),
                });
            }
        }
        state
            .entries
            .insert(path.clone(), FakeEntry::File(request.bytes.to_vec()));
        state.writes.push(WriteRecord {
            path,
            bytes: request.bytes.to_vec(),
            expected: request.expected.clone(),
            mode: request.mode.clone(),
        });
        Ok(FakeHasher.sha256(request.bytes))
    }
}

/// A fault-injecting [`DefinitionWriter`] over any other writer: it
/// forwards each write, counts the ones it performed, and from write
/// number `crash_before` on returns an error that stands for a process
/// death, leaving the target untouched.
///
/// An error before write N models the process dying between write N-1
/// and write N: the earlier writes are on disk, the apply service has
/// stored the snapshots it took before them, and it never reaches the
/// point where it stores a record. The wrapper does not stop the apply
/// the way a real death would; it relies on the service ending its run at
/// the first write error, so no later write is attempted. Should one be,
/// it fails the same way.
///
/// The crash is [`DefinitionWriteError::Io`] carrying the path of the
/// write that did not happen, the one variant that says "could not be
/// completed, nothing published". The apply maps it to
/// `DefinitionError::Write` like any other write error. No fixture a
/// crash test builds can produce a genuine `Io`, so a test tells the
/// simulated crash from a refusal by its `io` reason and by
/// [`Self::crashed`].
#[derive(Debug)]
pub struct CrashingWriter<W> {
    inner: W,
    crash_before: Option<usize>,
    attempted: AtomicUsize,
    performed: AtomicUsize,
}

impl<W: DefinitionWriter> CrashingWriter<W> {
    /// A writer that crashes before the `crash_before`th write
    /// (1-based). `crash_before` of 1 crashes before any file changes.
    #[must_use]
    pub fn crashing_before(inner: W, crash_before: usize) -> Self {
        Self {
            inner,
            crash_before: Some(crash_before),
            attempted: AtomicUsize::new(0),
            performed: AtomicUsize::new(0),
        }
    }

    /// A writer that never crashes and only counts, to learn how many
    /// writes a run makes.
    #[must_use]
    pub fn counting(inner: W) -> Self {
        Self {
            inner,
            crash_before: None,
            attempted: AtomicUsize::new(0),
            performed: AtomicUsize::new(0),
        }
    }

    /// How many writes reached the inner writer and succeeded.
    #[must_use]
    pub fn writes_performed(&self) -> usize {
        self.performed.load(Ordering::SeqCst)
    }

    /// Whether the simulated crash fired.
    #[must_use]
    pub fn crashed(&self) -> bool {
        self.crash_before
            .is_some_and(|n| self.attempted.load(Ordering::SeqCst) >= n)
    }

    /// The wrapped writer.
    #[must_use]
    pub fn inner(&self) -> &W {
        &self.inner
    }
}

impl<W: DefinitionWriter> DefinitionWriter for CrashingWriter<W> {
    fn write(
        &self,
        root: &WorkspaceRoot,
        request: &WriteRequest<'_>,
    ) -> Result<Sha256Digest, DefinitionWriteError> {
        let number = self.attempted.fetch_add(1, Ordering::SeqCst) + 1;
        if self.crash_before.is_some_and(|n| number >= n) {
            return Err(DefinitionWriteError::Io {
                path: request.path.to_string(),
            });
        }
        let digest = self.inner.write(root, request)?;
        self.performed.fetch_add(1, Ordering::SeqCst);
        Ok(digest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> WorkspaceRoot {
        WorkspaceRoot::new(std::env::temp_dir()).expect("a workspace")
    }

    fn write(
        writer: &InMemoryDefinitionWriter,
        path: &str,
        bytes: &[u8],
        expected: &InputFingerprint,
    ) -> Result<Sha256Digest, DefinitionWriteError> {
        writer.write(
            &root(),
            &WriteRequest {
                path,
                bytes,
                expected,
                mode: &ModePolicy::Keep,
            },
        )
    }

    fn sha(bytes: &[u8]) -> InputFingerprint {
        InputFingerprint::Sha256(FakeHasher.sha256(bytes))
    }

    #[test]
    fn the_crashing_writer_forwards_until_its_write_and_then_leaves_the_target_alone() {
        let inner = InMemoryDefinitionWriter::new().with_file("CLAUDE.md", b"old\n");
        let writer = CrashingWriter::crashing_before(inner, 2);
        let request = |path, bytes, expected| WriteRequest {
            path,
            bytes,
            expected,
            mode: &ModePolicy::Keep,
        };
        writer
            .write(
                &root(),
                &request("AGENTS.md", b"a\n", &InputFingerprint::Absent),
            )
            .expect("write 1 goes through");
        assert!(!writer.crashed());
        let error = writer
            .write(&root(), &request("CLAUDE.md", b"w\n", &sha(b"old\n")))
            .expect_err("write 2 is the crash");
        assert_eq!(error.reason(), "io");
        assert!(writer.crashed());
        assert_eq!(writer.writes_performed(), 1);
        assert_eq!(
            writer.inner().bytes("CLAUDE.md").as_deref(),
            Some(&b"old\n"[..])
        );
    }

    #[test]
    fn accepted_writes_are_recorded_in_order_and_readable() {
        let writer = InMemoryDefinitionWriter::new().with_file("CLAUDE.md", b"old\r\n");
        let created = write(&writer, "AGENTS.md", b"a\n", &InputFingerprint::Absent).expect("ok");
        assert_eq!(created, FakeHasher.sha256(b"a\n"));
        write(&writer, "CLAUDE.md", b"@AGENTS.md\n", &sha(b"old\r\n")).expect("ok");
        let paths: Vec<String> = writer.writes().into_iter().map(|w| w.path).collect();
        assert_eq!(paths, vec!["AGENTS.md", "CLAUDE.md"]);
        assert_eq!(
            writer.bytes("CLAUDE.md").as_deref(),
            Some(&b"@AGENTS.md\n"[..])
        );
    }

    #[test]
    fn a_stale_or_appeared_file_refuses_and_is_untouched() {
        let writer = InMemoryDefinitionWriter::new().with_file("AGENTS.md", b"theirs\n");
        let appeared = write(&writer, "AGENTS.md", b"mine\n", &InputFingerprint::Absent);
        assert_eq!(appeared.expect_err("refused").reason(), "stale");
        let stale = write(&writer, "AGENTS.md", b"mine\n", &sha(b"planned\n"));
        assert_eq!(stale.expect_err("refused").reason(), "stale");
        assert_eq!(writer.bytes("AGENTS.md").as_deref(), Some(&b"theirs\n"[..]));
        assert!(writer.writes().is_empty());
    }

    #[test]
    fn links_irregular_entries_and_bad_paths_refuse() {
        let writer = InMemoryDefinitionWriter::new()
            .with_symlink("AGENTS.md", "x")
            .with_irregular("CLAUDE.md")
            .with_symlink("linked", "elsewhere");
        let code = |path: &str, expected: &InputFingerprint| {
            write(&writer, path, b"n", expected)
                .expect_err(path)
                .reason()
        };
        assert_eq!(code("AGENTS.md", &sha(b"x")), "not-regular");
        assert_eq!(code("CLAUDE.md", &sha(b"x")), "not-regular");
        assert_eq!(
            code("linked/AGENTS.md", &InputFingerprint::Absent),
            "not-regular"
        );
        assert_eq!(
            code("AGENTS.md", &InputFingerprint::Irregular),
            "not-writable"
        );
        assert_eq!(code("../x", &InputFingerprint::Absent), "invalid-path");
        assert!(writer.writes().is_empty());
    }

    #[test]
    fn only_a_seeded_directory_or_a_seeded_files_ancestors_are_parents() {
        let writer = InMemoryDefinitionWriter::new()
            .with_file("seen/inner/a.md", b"a")
            .with_directory("empty");
        let code = |path: &str| {
            write(&writer, path, b"n", &InputFingerprint::Absent)
                .err()
                .map(|error| error.reason())
        };
        assert_eq!(code("missing/AGENTS.md"), Some("parent-missing"));
        assert_eq!(code("seen/missing/AGENTS.md"), Some("parent-missing"));
        assert_eq!(code("AGENTS.md"), None);
        assert_eq!(code("seen/AGENTS.md"), None);
        assert_eq!(code("seen/inner/AGENTS.md"), None);
        assert_eq!(code("empty/AGENTS.md"), None);
        // A written file makes its own directory real for later writes.
        assert_eq!(code("empty/sub/AGENTS.md"), Some("parent-missing"));
        assert_eq!(code("seen/AGENTS.md"), Some("stale"));
    }

    #[test]
    fn the_permission_source_must_be_a_regular_file() {
        let writer = InMemoryDefinitionWriter::new().with_file("CLAUDE.md", b"s");
        let request = |source: &str| {
            writer.write(
                &root(),
                &WriteRequest {
                    path: "AGENTS.md",
                    bytes: b"s",
                    expected: &InputFingerprint::Absent,
                    mode: &ModePolicy::NoWiderThan {
                        source: source.to_string(),
                    },
                },
            )
        };
        assert_eq!(
            request("MISSING.md").expect_err("refused").reason(),
            "source-unavailable"
        );
        request("CLAUDE.md").expect("written");
    }
}
