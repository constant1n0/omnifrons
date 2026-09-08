//! The `ContentHasher` port (spike slice 5b): SHA-256 over bytes and over
//! a stream, abstracted because this crate depends on `omnifrons-domain`
//! and `thiserror` only -- the real hasher (`sha2`) lives in
//! `omnifrons-adapters`, and the tests drive a deterministic fake.
//!
//! The three derivations below are the only places an identity is
//! computed: each hashes exactly the preimage `omnifrons-domain`'s
//! `publication` module spells out, so the shape of every identity is
//! decided in one place and the hasher only supplies the function.

use std::io::Read;
use std::time::SystemTime;

use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::outbox::ContentDigest;
use omnifrons_domain::publication::{
    ArtifactApprovalId, ProjectIdentity, PublicationIdentity, artifact_approval_id_preimage,
    publication_identity_preimage,
};

use crate::harness_adapter::WorkspaceRoot;

/// The domain tag mixed into a project identity's preimage. The project
/// identity itself is a spike default (`docs/spike-log.md` § Slice 5b):
/// the digest of the workspace root's canonical path stands in for the
/// registered scope identity context-orb.md and RSP-001-R13 own, which
/// this repository does not yet have.
pub const PROJECT_IDENTITY_DOMAIN: &[u8] = b"omnifrons-project-v1";

/// A source of SHA-256 digests.
pub trait ContentHasher {
    /// The SHA-256 of `input`.
    fn sha256(&self, input: &[u8]) -> Sha256Digest;

    /// The SHA-256 of everything `reader` yields until EOF, and how many
    /// bytes that was.
    ///
    /// # Errors
    ///
    /// Returns any `io::Error` the reader produces.
    fn digest_reader(&self, reader: &mut dyn Read) -> std::io::Result<(Sha256Digest, u64)>;
}

/// The project identity of `workspace`: `sha256(PROJECT_IDENTITY_DOMAIN ‖
/// canonical path bytes)`. A spike default, never a path itself.
#[must_use]
pub fn derive_project_identity(
    hasher: &dyn ContentHasher,
    workspace: &WorkspaceRoot,
) -> ProjectIdentity {
    let mut preimage = PROJECT_IDENTITY_DOMAIN.to_vec();
    preimage.extend_from_slice(workspace.path().to_string_lossy().as_bytes());
    ProjectIdentity(hasher.sha256(&preimage))
}

/// The publication identity of `digest` within `project` (HAP-001-R23).
#[must_use]
pub fn derive_publication_identity(
    hasher: &dyn ContentHasher,
    project: &ProjectIdentity,
    digest: &ContentDigest,
) -> PublicationIdentity {
    PublicationIdentity(hasher.sha256(&publication_identity_preimage(project, digest)))
}

/// The artifact approval id for `publication` approved at `approved_at`:
/// the first 8 bytes of the preimage's SHA-256, big-endian.
#[must_use]
pub fn derive_artifact_approval_id(
    hasher: &dyn ContentHasher,
    publication: &PublicationIdentity,
    approved_at: SystemTime,
) -> ArtifactApprovalId {
    let digest = hasher.sha256(&artifact_approval_id_preimage(publication, approved_at));
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest.0[..8]);
    ArtifactApprovalId(u64::from_be_bytes(bytes))
}
