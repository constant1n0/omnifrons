//! The `ContentHasher` port and the derivations over it (spike slice 5b):
//! the project identity (a spike default: the digest of the workspace
//! root's canonical path), the publication identity (HAP-001-R23), and
//! the artifact approval id -- each exactly the hash of the preimage the
//! domain spells out, so a real hasher and the fake agree on structure.
//!
//! Only compiled with `--features contract-tests`: the fake hasher lives
//! behind that feature.

#![cfg(feature = "contract-tests")]

use omnifrons_app::WorkspaceRoot;
use omnifrons_app::content_hasher::{
    ContentHasher, PROJECT_IDENTITY_DOMAIN, derive_artifact_approval_id, derive_project_identity,
    derive_publication_identity,
};
use omnifrons_app::contract::publication::FakeHasher;
use omnifrons_domain::executable::Sha256Digest;
use omnifrons_domain::publication::{
    ProjectIdentity, PublicationIdentity, artifact_approval_id_preimage,
    publication_identity_preimage,
};
use std::time::{Duration, SystemTime};

#[test]
fn publication_identity_is_the_hash_of_the_domain_preimage() {
    let hasher = FakeHasher;
    let project = ProjectIdentity(Sha256Digest([1; 32]));
    let digest = Sha256Digest([2; 32]);
    let derived = derive_publication_identity(&hasher, &project, &digest);
    assert_eq!(
        derived,
        PublicationIdentity(hasher.sha256(&publication_identity_preimage(&project, &digest)))
    );
    assert_ne!(
        derived,
        derive_publication_identity(&hasher, &project, &Sha256Digest([3; 32])),
        "a different digest is a different publication"
    );
    assert_ne!(
        derived,
        derive_publication_identity(&hasher, &ProjectIdentity(Sha256Digest([9; 32])), &digest),
        "identical bytes in two projects are two publications (HAP-001-R23)"
    );
}

/// The spike default for the project identity: `sha256(domain ‖ canonical
/// workspace path bytes)`, disclosed as standing in for the registered
/// scope identity context-orb.md owns.
#[test]
fn project_identity_is_the_hash_of_the_canonical_workspace_path() {
    let hasher = FakeHasher;
    let dir = std::env::temp_dir();
    let workspace = WorkspaceRoot::new(&dir).expect("the temp dir is a valid workspace");
    let derived = derive_project_identity(&hasher, &workspace);
    let mut preimage = PROJECT_IDENTITY_DOMAIN.to_vec();
    preimage.extend_from_slice(workspace.path().to_string_lossy().as_bytes());
    assert_eq!(derived, ProjectIdentity(hasher.sha256(&preimage)));
}

#[test]
fn artifact_approval_id_is_the_first_eight_bytes_of_the_preimage_hash() {
    let hasher = FakeHasher;
    let publication = PublicationIdentity(Sha256Digest([4; 32]));
    let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_725_782_401);
    let id = derive_artifact_approval_id(&hasher, &publication, at);
    let full = hasher.sha256(&artifact_approval_id_preimage(&publication, at));
    let mut expected = [0u8; 8];
    expected.copy_from_slice(&full.0[..8]);
    assert_eq!(id.0, u64::from_be_bytes(expected));
    assert_ne!(
        id,
        derive_artifact_approval_id(&hasher, &publication, at + Duration::from_nanos(1)),
        "a later instant is a different approval id"
    );
}

/// `digest_reader` and `sha256` agree on the same bytes, and the reader
/// form reports how many bytes it consumed.
#[test]
fn digest_reader_matches_sha256_over_the_same_bytes() {
    let hasher = FakeHasher;
    let bytes = b"the same bytes either way";
    let (digest, size) = hasher
        .digest_reader(&mut std::io::Cursor::new(bytes))
        .expect("an in-memory reader never fails");
    assert_eq!(digest, hasher.sha256(bytes));
    assert_eq!(size, bytes.len() as u64);
}
