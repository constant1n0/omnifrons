//! The `ExecutableProber` port: turn a candidate filesystem path into an
//! executable identity plus the exact open handle it was hashed from, or a
//! reason it could not be probed.
//!
//! `std::fs::File` is a `std` type, not a domain one -- [`ExecHandle`] and
//! [`ProbedExecutable`] live here, in `omnifrons-app`, rather than
//! `omnifrons-domain` (`docs/repository-layout.md` § Crate map:
//! `omnifrons-domain` is `std` and `thiserror` only in its own right, but a
//! live OS handle has no business in a framework-independent domain type
//! that must stay comparable, cloneable, and free of process-local state).

use std::path::Path;

pub use omnifrons_domain::executable::ExecutableIdentity;

/// The open file description a probe hashed, carried alongside the
/// [`ExecutableIdentity`] it produced so a caller can launch *this exact*
/// content -- never a fresh, uncoordinated re-open by path, which would
/// reopen a TOCTOU window a probe's own hashing had just closed.
#[derive(Debug)]
pub enum ExecHandle {
    /// The plain, already-open file the candidate was hashed from. Used
    /// on every platform when no stronger guarantee is available (macOS,
    /// Windows, or a Linux probe that fell back rather than sealing).
    File(std::fs::File),
    /// Linux only: a sealed (`memfd_create` + `F_SEAL_SHRINK`/`GROW`/
    /// `WRITE`/`SEAL`) anonymous memory file whose content is exactly the
    /// bytes that were hashed -- immutable from this point on, including
    /// against the probing process itself.
    #[cfg(target_os = "linux")]
    SealedMemory(std::fs::File),
}

impl ExecHandle {
    /// `fstat` the handle's underlying file description, regardless of
    /// which variant this is.
    ///
    /// Exists so a test can prove the handle refers to the exact same
    /// file description the rest of a probe used to decide
    /// [`ExecutableIdentity`] -- comparing `(dev, ino)` between this and
    /// an independent `stat` of the probed path is the structural
    /// evidence that no swap happened *inside* one probe call, standing
    /// in for a test that would otherwise have to force a real race.
    ///
    /// # Errors
    ///
    /// Returns any `io::Error` the underlying `fstat`/`GetFileInformationByHandle`
    /// call can produce.
    pub fn metadata(&self) -> std::io::Result<std::fs::Metadata> {
        match self {
            Self::File(file) => file.metadata(),
            #[cfg(target_os = "linux")]
            Self::SealedMemory(file) => file.metadata(),
        }
    }
}

/// A probed executable's identity, plus the exact handle it was hashed
/// from.
///
/// Carrying both together -- rather than an [`ExecutableIdentity`] alone,
/// re-opened by path later -- is what lets a caller launch precisely the
/// bytes a [`crate::launch_gate::LaunchGate`] decision just vetted,
/// instead of trusting that a second, independent open by path still
/// resolves to the same content.
#[derive(Debug)]
pub struct ProbedExecutable {
    /// The identity this probe determined: canonical path, size, digest,
    /// and informational platform evidence.
    pub identity: ExecutableIdentity,
    /// The open handle the content was hashed from.
    pub handle: ExecHandle,
}

impl ProbedExecutable {
    /// `fstat` [`Self::handle`] -- see [`ExecHandle::metadata`].
    ///
    /// # Errors
    ///
    /// Returns any `io::Error` the underlying `fstat` call can produce.
    pub fn handle_metadata(&self) -> std::io::Result<std::fs::Metadata> {
        self.handle.metadata()
    }
}

/// The outcome of probing a candidate path for its executable identity.
///
/// Deliberately not [`PartialEq`]/[`Clone`] (unlike
/// `omnifrons_domain::executable::ProbeOutcome`, which this mirrors
/// variant-for-variant apart from `Identity`'s payload): [`ExecHandle`]
/// wraps a live `std::fs::File`, which is neither. [`Self::as_domain_failure`]
/// is the bridge back to the domain-level outcome for the non-`Identity`
/// cases, which carry no handle and so lose nothing in that direction.
#[derive(Debug)]
pub enum ProbeOutcome {
    /// The candidate resolved to a regular, executable file within the
    /// size cap, and was hashed successfully.
    Identity(ProbedExecutable),
    /// The canonicalized candidate is not a regular file (e.g. a
    /// directory, a device, a FIFO).
    NotRegularFile,
    /// The candidate is a regular file but does not appear executable on
    /// this platform.
    NotExecutable,
    /// The candidate's metadata or content could not be read.
    Unreadable,
    /// The candidate exceeds the probe's size cap.
    TooLarge,
}

impl ProbeOutcome {
    /// Map every non-`Identity` variant to its
    /// `omnifrons_domain::executable::ProbeOutcome` equivalent, or `None`
    /// for `Identity` (the success case, which carries a live handle the
    /// domain layer must never hold).
    ///
    /// Used by [`crate::launch_gate::LaunchGate::decide`] to build a
    /// domain-safe `DenialReason::ProbeFailed` from a failed re-probe.
    #[must_use]
    pub fn as_domain_failure(&self) -> Option<omnifrons_domain::executable::ProbeOutcome> {
        use omnifrons_domain::executable::ProbeOutcome as Domain;
        match self {
            Self::Identity(_) => None,
            Self::NotRegularFile => Some(Domain::NotRegularFile),
            Self::NotExecutable => Some(Domain::NotExecutable),
            Self::Unreadable => Some(Domain::Unreadable),
            Self::TooLarge => Some(Domain::TooLarge),
        }
    }
}

/// A port for probing a candidate filesystem path into a
/// [`ProbedExecutable`] (wrapped in [`ProbeOutcome::Identity`]), or the
/// specific reason probing failed.
///
/// `&self`, not `&mut self`: probing is read-only and side-effect-free
/// from the caller's perspective, so [`crate::launch_gate::LaunchGate`]
/// can hold one probe and re-probe freely on every
/// [`crate::launch_gate::LaunchGate::decide`] call.
pub trait ExecutableProber {
    /// Probe `candidate`: canonicalize it, open the canonical path exactly
    /// once, verify it is a regular, executable file within the size cap,
    /// and hash its content from that same open handle -- returning that
    /// handle alongside the identity it produced.
    fn probe(&self, candidate: &Path) -> ProbeOutcome;
}
