//! Outbox domain types (spike slice 5, HAP-001 launch side).
//!
//! Framework-independent (this module lives in `omnifrons-domain`, which
//! depends on `std` and `thiserror` only): the project-relative outbox
//! path a policy declares (HAP-001-R8), the run id that names a run
//! subdirectory (D20), the candidate entry and its attribution
//! (HAP-001-R11), the closed candidate-state and launch-failure tokens the
//! shell puts on the wire (HAP-001-R26), the artifact classes the
//! classification policy assigns (HAP-001-R1 through R4), the content-type
//! detection by magic bytes the policy matches on, and the typed publish
//! proposal a harness surfaces (HAP-001-R12).
//!
//! Nothing here touches the filesystem: validating that a declared path
//! canonicalizes inside a project, opening an entry, or hashing it is the
//! application and adapter layers' work (`omnifrons-app`'s `run_outbox`
//! ports and `omnifrons-adapters`' `Fs*` implementations).

use std::fmt;
use std::path::{Component, Path, PathBuf};

use crate::publication::is_bidi_control;

pub use crate::executable::Sha256Digest;

/// The default outbox path when a project's policy declares none
/// (HAP-001 D17): inside the reserved `.omnifrons/` namespace, beside the
/// policy that would declare it.
pub const DEFAULT_OUTBOX_PATH: &str = ".omnifrons/outbox";

/// The tool-call name a harness uses to propose publication of entries it
/// wrote into its run subdirectory (HAP-001-R12, D5): a proposal only,
/// never executed by anything in this repository.
pub const PUBLISH_PROPOSAL_TOOL_NAME: &str = "artifact.publish";

/// A content digest: HAP-001 D13 selects SHA-256, the same digest the
/// executable prober already computes, so the type is reused rather than
/// duplicated.
pub type ContentDigest = Sha256Digest;

/// Why [`OutboxPath::new`] rejected a declared path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxPathError {
    /// The declared path is empty.
    Empty,
    /// The declared path is absolute, has a root, or carries a Windows
    /// drive prefix (refused on every platform, since a policy roams).
    Absolute,
    /// The declared path contains a `..` component.
    ParentTraversal,
    /// The declared path contains a backslash: a roaming policy names the
    /// path with `/` so it means the same thing on every device.
    Backslash,
    /// The declared path contains a control character, one of the two
    /// Unicode line separators, or a bidirectional override, embedding or
    /// isolate character. The declaration is untrusted content
    /// (HAP-001-R40) that this product substitutes into the managed blocks
    /// it writes into a project's text files, so a line break in it would
    /// forge or break a sentinel and an override would make the block
    /// render in an order it is not stored in; all three are refused here
    /// rather than escaped later. One variant, not three: they are one
    /// class of character -- the ones that make a managed block read as
    /// something other than what it is -- and no caller can act
    /// differently on them, which is why `ManagedFileNameError::Control`
    /// already folds controls and overrides together for the same files.
    Control,
    /// The declared path, or one of its components, is longer than the caps
    /// [`MAX_PROJECT_RELATIVE_BYTES`] and
    /// [`MAX_PROJECT_RELATIVE_COMPONENT_BYTES`] fix.
    TooLong,
}

impl fmt::Display for OutboxPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "the outbox path is empty",
            Self::Absolute => "the outbox path must be relative to the project root",
            Self::ParentTraversal => "the outbox path must not contain a parent (..) component",
            Self::Backslash => "the outbox path must use / as its separator",
            Self::Control => {
                "the outbox path must not contain control, line-separator, or bidirectional \
                 override characters"
            }
            Self::TooLong => "the outbox path is too long",
        };
        f.write_str(message)
    }
}

impl std::error::Error for OutboxPathError {}

/// A validated, project-relative outbox path (HAP-001-R8): never empty,
/// never absolute, never escaping through `..`, always `/`-separated.
///
/// Constructible only through [`Self::new`] or [`Self::default_path`], so
/// every live value has passed those checks. This is the *declared* path;
/// whether it canonicalizes inside a given project root and is not a link
/// is decided by the application layer against a real filesystem.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OutboxPath(PathBuf);

impl OutboxPath {
    /// Validate `declared` as a project-relative outbox path.
    ///
    /// # Errors
    ///
    /// Returns [`OutboxPathError`] if `declared` is empty, carries a
    /// control, line-separator or bidirectional override character, is
    /// absolute (or drive-prefixed), contains a `..` component, or
    /// contains a backslash.
    pub fn new(declared: impl AsRef<str>) -> Result<Self, OutboxPathError> {
        let declared = declared.as_ref();
        validate_project_relative(declared)?;
        Ok(Self(PathBuf::from(declared)))
    }

    /// The default outbox path, [`DEFAULT_OUTBOX_PATH`].
    #[must_use]
    pub fn default_path() -> Self {
        Self(PathBuf::from(DEFAULT_OUTBOX_PATH))
    }

    /// The declared, project-relative path.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl fmt::Display for OutboxPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.to_string_lossy())
    }
}

/// The most bytes one component of a project-relative path may carry.
///
/// Exactly [`crate::publication::DisplayName::MAX_BYTES`], the cap
/// `DisplayName::sanitize` already applies to every single name this
/// product puts on a device or on a surface: a project-relative path is a
/// sequence of such names and each one takes the same bound, so a name that
/// would be truncated the moment it became a file name is refused before it
/// becomes a finding instead.
pub const MAX_PROJECT_RELATIVE_COMPONENT_BYTES: usize = crate::publication::DisplayName::MAX_BYTES;

/// The most bytes a whole project-relative path may carry: four components'
/// worth of [`MAX_PROJECT_RELATIVE_COMPONENT_BYTES`].
///
/// A bound rather than "whatever the file system allowed", which is what a
/// producer choosing its own directory names would otherwise decide for
/// every surface this value reaches.
pub const MAX_PROJECT_RELATIVE_BYTES: usize = 4 * MAX_PROJECT_RELATIVE_COMPONENT_BYTES;

/// Validate `declared` as a project-relative path: never empty, never
/// longer than [`MAX_PROJECT_RELATIVE_BYTES`] (nor any component longer
/// than [`MAX_PROJECT_RELATIVE_COMPONENT_BYTES`]), never absolute (or
/// drive-prefixed), never escaping through `..`, always `/`-separated, and
/// never carrying a control, line-separator, or bidirectional character.
///
/// Shared by [`OutboxPath::new`], which validates a *declared* outbox
/// path, and by `wrong_root::MisplacedFinding::new` (spike slice 5d),
/// which validates the name a scan found a file at -- one rule, so a
/// finding can no more carry a device path than a declaration can.
///
/// # Errors
///
/// Returns the [`OutboxPathError`] naming the rule that failed.
#[allow(clippy::missing_panics_doc)]
pub fn validate_project_relative(declared: &str) -> Result<(), OutboxPathError> {
    if declared.is_empty() {
        return Err(OutboxPathError::Empty);
    }
    // Bounded before anything else looks at it. Nothing downstream bounds
    // this value: a *declared* outbox path is a policy file's, and a
    // finding's name is chosen by whatever wrote the file the scan found,
    // so without a cap here the value crosses IPC and reaches a surface at
    // whatever length the file system happened to allow (spike slice 5d,
    // R1-012).
    if declared.len() > MAX_PROJECT_RELATIVE_BYTES {
        return Err(OutboxPathError::TooLong);
    }
    if declared
        .split('/')
        .any(|component| component.len() > MAX_PROJECT_RELATIVE_COMPONENT_BYTES)
    {
        return Err(OutboxPathError::TooLong);
    }
    // Before any path shape: a declaration is untrusted content
    // (HAP-001-R40) that `guidance` substitutes into the managed blocks
    // this product writes into a project's text files, and a line break
    // there would forge or break a sentinel. `char::is_control` covers
    // only the Cc category, so the bidirectional override, embedding
    // and isolate range is refused through the same helper the managed
    // file names and the display names of this data flow use (R1-004).
    if declared
        .chars()
        .any(|c| c.is_control() || is_bidi_control(c) || matches!(c, '\u{2028}' | '\u{2029}'))
    {
        return Err(OutboxPathError::Control);
    }
    if declared.contains('\\') {
        // A `C:\x` form is a drive prefix first: report it as absolute
        // rather than as a separator problem, since fixing the separator
        // alone would not make it relative.
        if has_drive_prefix(declared) {
            return Err(OutboxPathError::Absolute);
        }
        return Err(OutboxPathError::Backslash);
    }
    if has_drive_prefix(declared) {
        return Err(OutboxPathError::Absolute);
    }
    let path = Path::new(declared);
    if path.is_absolute() || path.has_root() {
        return Err(OutboxPathError::Absolute);
    }
    for component in path.components() {
        match component {
            Component::ParentDir => return Err(OutboxPathError::ParentTraversal),
            Component::RootDir | Component::Prefix(_) => return Err(OutboxPathError::Absolute),
            Component::Normal(_) | Component::CurDir => {}
        }
    }
    Ok(())
}

/// Whether `declared` starts with a Windows drive prefix such as `C:`.
fn has_drive_prefix(declared: &str) -> bool {
    let bytes = declared.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// Why [`RunId::new`] rejected a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunIdError {
    /// The token is empty.
    Empty,
    /// The token exceeds [`RunId::MAX_CHARS`] characters.
    TooLong,
    /// The token contains a character outside `[A-Za-z0-9_-]`.
    InvalidCharacter,
}

impl fmt::Display for RunIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "the run id is empty",
            Self::TooLong => "the run id exceeds 64 characters",
            Self::InvalidCharacter => "the run id must contain only ASCII letters, digits, - and _",
        };
        f.write_str(message)
    }
}

impl std::error::Error for RunIdError {}

/// A run's identifier, doubling as the name of its run subdirectory under
/// the outbox (`<outbox>/<run id>/`, HAP-001 D20).
///
/// Constructible only through [`Self::new`], which admits a single path
/// component of ASCII letters, digits, `-` and `_` -- never a separator,
/// never `.` or `..`, never anything a filesystem or a wire token could
/// misread -- so a run id can be joined onto the outbox path without a
/// second validation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RunId(String);

impl RunId {
    /// The maximum length of a run id, inclusive.
    pub const MAX_CHARS: usize = 64;

    /// Validate `token` as a run id.
    ///
    /// # Errors
    ///
    /// Returns [`RunIdError`] if `token` is empty, longer than
    /// [`Self::MAX_CHARS`], or contains a character outside
    /// `[A-Za-z0-9_-]`.
    pub fn new(token: impl Into<String>) -> Result<Self, RunIdError> {
        let token = token.into();
        if token.is_empty() {
            return Err(RunIdError::Empty);
        }
        if token.len() > Self::MAX_CHARS {
            return Err(RunIdError::TooLong);
        }
        if !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(RunIdError::InvalidCharacter);
        }
        Ok(Self(token))
    }

    /// This id's token, also its run subdirectory's name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether a candidate entry is attributed to a run (HAP-001-R11): only
/// when it lies under that run's subdirectory *and* the run's own publish
/// proposal names it by a content digest equal to the one computed from
/// the entry's handle. Everything else is unattributed; location alone
/// never attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attribution {
    /// The run whose own proposal named this entry by digest.
    Run(RunId),
    /// No run's proposal named this entry: at the outbox root, or under a
    /// run subdirectory the run did not claim it in.
    Unattributed,
}

impl Attribution {
    /// The attributed run's id, or `None` for an unattributed entry.
    #[must_use]
    pub fn run_id(&self) -> Option<&RunId> {
        match self {
            Self::Run(run_id) => Some(run_id),
            Self::Unattributed => None,
        }
    }
}

/// The class the classification policy assigned to a candidate entry
/// (HAP-001 § Artifact classes and classification policy, narrowed to the
/// outcomes this slice distinguishes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArtifactClass {
    /// Routed to the asset root: a document, image, recording, dataset, or
    /// archive (`generated-heavy`).
    GeneratedHeavy,
    /// Small portable text a `git-tracked` row claims: never published
    /// through this path (HAP-001-R3).
    GitTracked,
    /// A Markdown note: knowledge-plane content, never an artifact of this
    /// contract (HAP-001-R2).
    PortableText,
    /// Identified as executable: quarantine by default (HAP-001-R4).
    Executable,
    /// No policy row matched: blocked pending the Asset Policy Owner
    /// (HAP-001-R1).
    Unclassified,
}

impl ArtifactClass {
    /// Every class, for exhaustive iteration in tests and policy parsing.
    pub const ALL: [Self; 5] = [
        Self::GeneratedHeavy,
        Self::GitTracked,
        Self::PortableText,
        Self::Executable,
        Self::Unclassified,
    ];

    /// This class's stable token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::GeneratedHeavy => "generated-heavy",
            Self::GitTracked => "git-tracked",
            Self::PortableText => "portable-text",
            Self::Executable => "executable",
            Self::Unclassified => "unclassified",
        }
    }

    /// Parse a token produced by [`Self::as_str`].
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|class| class.as_str() == token)
    }
}

/// The state a candidate entry is rendered in after validation
/// (HAP-001 § Signal mapping): the three tokens the launch side of this
/// slice exposes for an outbox entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CandidateState {
    /// A regular file with a link count of one, digested from its handle,
    /// awaiting approval (never "validated for publication").
    Candidate,
    /// A link found in the outbox (never dereferenced), a non-regular file
    /// (a directory, FIFO, socket, or device), or an entry that resolves
    /// outside the outbox (HAP-001-R14, R15).
    OutboxEscape,
    /// A regular file whose link count, read from the opened handle, is
    /// greater than one (HAP-001-R20).
    OutboxLinked,
}

impl CandidateState {
    /// This state's stable wire token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::OutboxEscape => "outbox-escape",
            Self::OutboxLinked => "outbox-linked",
        }
    }
}

/// The launch-side failure tokens of the outbox (HAP-001 § Signal
/// mapping): a declaration that fails, and an outbox that cannot host a
/// run subdirectory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutboxFailure {
    /// The declared outbox path resolves outside the project root or is a
    /// link (HAP-001-R8).
    OutboxInvalid,
    /// The outbox fails its pre-creation check, the run subdirectory cannot
    /// be created exclusively, or handle verification fails (HAP-001-R10).
    OutboxUnavailable,
}

impl OutboxFailure {
    /// This failure's stable wire token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::OutboxInvalid => "outbox-invalid",
            Self::OutboxUnavailable => "outbox-unavailable",
        }
    }
}

/// One validated candidate entry: its name relative to the outbox, the
/// facts taken from its handle, its attribution, and its class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateEntry {
    /// The entry's name relative to the outbox (`<run id>/<file>` under a
    /// run subdirectory, `<file>` at the outbox root). Producer-supplied
    /// text: a consumer renders it as plain text only.
    pub name: String,
    /// The number of bytes digested from the handle.
    pub size: u64,
    /// The content digest computed from the handle (HAP-001-R16).
    pub digest: ContentDigest,
    /// The content type detected from the same bytes.
    pub detected_type: DetectedType,
    /// Whether a run's own proposal named this entry by digest.
    pub attribution: Attribution,
    /// The class the policy assigned.
    pub class: ArtifactClass,
}

/// One entry an `artifact.publish` proposal names: a name relative to the
/// run subdirectory and the content digest the harness claims for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedEntry {
    /// The proposed entry's name, as the harness stated it.
    pub name: String,
    /// The content digest the harness claims; attribution requires
    /// Omnifrons's own digest from the handle to equal it (HAP-001-R11).
    pub sha256: ContentDigest,
}

/// A typed `artifact.publish` proposal a harness surfaced (HAP-001-R12):
/// a proposal only -- nothing executes it -- naming entries by digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishProposal {
    /// The entries the harness proposes for publication.
    pub entries: Vec<ProposedEntry>,
}

impl PublishProposal {
    /// Whether any proposed entry claims `digest`.
    #[must_use]
    pub fn names_digest(&self, digest: &ContentDigest) -> bool {
        self.entries.iter().any(|entry| &entry.sha256 == digest)
    }
}

/// The content type detected for a candidate entry from the first bytes
/// read from its handle, with the name's extension consulted only where
/// the bytes alone cannot decide (a ZIP container that is an Office
/// document; Markdown among plain text). Content wins over the name
/// (HAP-001-R1: the row for the detected type wins).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DetectedType {
    Pdf,
    /// A ZIP container named `.docx`/`.xlsx`/`.pptx`/`.odt`/`.ods`/`.odp`.
    OfficeDocument,
    Png,
    Jpeg,
    Gif,
    Webp,
    Mp3,
    Wav,
    Ogg,
    Flac,
    Mp4,
    Matroska,
    Zip,
    Gzip,
    SevenZip,
    Tar,
    ElfExecutable,
    PeExecutable,
    MachOExecutable,
    /// A `#!`-interpreted script: executable-shaped by content.
    Script,
    /// Valid UTF-8 text named `.md`/`.markdown`.
    Markdown,
    /// Valid UTF-8 text with no recognized magic.
    PlainText,
    /// Bytes no rule recognized.
    Unknown,
}

impl DetectedType {
    /// Every detected type, for exhaustive iteration in tests and policy
    /// parsing.
    pub const ALL: [Self; 23] = [
        Self::Pdf,
        Self::OfficeDocument,
        Self::Png,
        Self::Jpeg,
        Self::Gif,
        Self::Webp,
        Self::Mp3,
        Self::Wav,
        Self::Ogg,
        Self::Flac,
        Self::Mp4,
        Self::Matroska,
        Self::Zip,
        Self::Gzip,
        Self::SevenZip,
        Self::Tar,
        Self::ElfExecutable,
        Self::PeExecutable,
        Self::MachOExecutable,
        Self::Script,
        Self::Markdown,
        Self::PlainText,
        Self::Unknown,
    ];

    /// How many leading bytes [`Self::detect`] needs to decide every rule:
    /// the `ustar` marker of a tar header sits at offset 257.
    pub const SNIFF_BYTES: usize = 512;

    /// This type's stable token (also the policy file's spelling).
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::OfficeDocument => "office-document",
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Gif => "gif",
            Self::Webp => "webp",
            Self::Mp3 => "mp3",
            Self::Wav => "wav",
            Self::Ogg => "ogg",
            Self::Flac => "flac",
            Self::Mp4 => "mp4",
            Self::Matroska => "matroska",
            Self::Zip => "zip",
            Self::Gzip => "gzip",
            Self::SevenZip => "7z",
            Self::Tar => "tar",
            Self::ElfExecutable => "elf-executable",
            Self::PeExecutable => "pe-executable",
            Self::MachOExecutable => "mach-o-executable",
            Self::Script => "script",
            Self::Markdown => "markdown",
            Self::PlainText => "plain-text",
            Self::Unknown => "unknown",
        }
    }

    /// Parse a token produced by [`Self::as_str`].
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == token)
    }

    /// Whether this type is executable-shaped by content: a native
    /// executable image or a `#!` script (HAP-001-R4).
    #[must_use]
    pub const fn is_executable(&self) -> bool {
        matches!(
            self,
            Self::ElfExecutable | Self::PeExecutable | Self::MachOExecutable | Self::Script
        )
    }

    /// Detect the type of an entry named `name` from `head`, its leading
    /// bytes (at least [`Self::SNIFF_BYTES`] where the file is that long).
    /// Magic bytes decide first; the extension is consulted only to tell
    /// an Office document from a plain ZIP and Markdown from plain text.
    #[must_use]
    pub fn detect(name: &str, head: &[u8]) -> Self {
        if let Some(by_magic) = Self::by_magic(name, head) {
            return by_magic;
        }
        if std::str::from_utf8(head).is_ok() {
            if has_extension(name, &["md", "markdown"]) {
                return Self::Markdown;
            }
            return Self::PlainText;
        }
        Self::Unknown
    }

    fn by_magic(name: &str, head: &[u8]) -> Option<Self> {
        if head.starts_with(b"%PDF-") {
            return Some(Self::Pdf);
        }
        if head.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
            return Some(Self::Png);
        }
        if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return Some(Self::Jpeg);
        }
        if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
            return Some(Self::Gif);
        }
        if head.starts_with(b"RIFF") && head.len() >= 12 {
            match &head[8..12] {
                b"WEBP" => return Some(Self::Webp),
                b"WAVE" => return Some(Self::Wav),
                _ => {}
            }
        }
        if head.starts_with(b"ID3")
            || (head.len() >= 2 && head[0] == 0xFF && matches!(head[1], 0xFB | 0xF3 | 0xF2))
        {
            return Some(Self::Mp3);
        }
        if head.starts_with(b"OggS") {
            return Some(Self::Ogg);
        }
        if head.starts_with(b"fLaC") {
            return Some(Self::Flac);
        }
        if head.len() >= 8 && &head[4..8] == b"ftyp" {
            return Some(Self::Mp4);
        }
        if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
            return Some(Self::Matroska);
        }
        if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
            if has_extension(name, &["docx", "xlsx", "pptx", "odt", "ods", "odp"]) {
                return Some(Self::OfficeDocument);
            }
            return Some(Self::Zip);
        }
        if head.starts_with(&[0x1F, 0x8B]) {
            return Some(Self::Gzip);
        }
        if head.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
            return Some(Self::SevenZip);
        }
        if head.len() >= 262 && &head[257..262] == b"ustar" {
            return Some(Self::Tar);
        }
        if head.starts_with(&[0x7F, b'E', b'L', b'F']) {
            return Some(Self::ElfExecutable);
        }
        if head.starts_with(b"MZ") {
            return Some(Self::PeExecutable);
        }
        if head.starts_with(&[0xFE, 0xED, 0xFA, 0xCE])
            || head.starts_with(&[0xFE, 0xED, 0xFA, 0xCF])
            || head.starts_with(&[0xCE, 0xFA, 0xED, 0xFE])
            || head.starts_with(&[0xCF, 0xFA, 0xED, 0xFE])
            || head.starts_with(&[0xCA, 0xFE, 0xBA, 0xBE])
        {
            return Some(Self::MachOExecutable);
        }
        if head.starts_with(b"#!") {
            return Some(Self::Script);
        }
        None
    }
}

/// Whether `name`'s extension (case-insensitively, without the dot) is one
/// of `extensions`.
#[must_use]
pub fn has_extension(name: &str, extensions: &[&str]) -> bool {
    let Some((_, extension)) = name.rsplit_once('.') else {
        return false;
    };
    if extension.contains('/') {
        return false;
    }
    let lowered = extension.to_ascii_lowercase();
    extensions.contains(&lowered.as_str())
}

#[cfg(test)]
mod tests {
    use super::{OutboxPath, RunId, has_extension};

    #[test]
    fn has_extension_is_case_insensitive_and_needs_a_dot() {
        assert!(has_extension("A.PDF", &["pdf"]));
        assert!(!has_extension("pdf", &["pdf"]));
        assert!(!has_extension("dir.d/file", &["d"]));
    }

    #[test]
    fn outbox_path_keeps_the_declared_spelling() {
        let path = OutboxPath::new("./out").expect("a ./ prefix is a relative path");
        assert_eq!(path.to_string(), "./out");
    }

    #[test]
    fn run_id_is_a_single_path_component() {
        let id = RunId::new("run_1-a").expect("valid");
        assert_eq!(std::path::Path::new(id.as_str()).components().count(), 1);
    }
}
