//! Checkpoint integrity for the same-device switch (target-architecture.md
//! § Model and harness switching): the body
//! [`SwitchCheckpoint::encode_body`] writes, sealed with the SHA-256 of its
//! bytes, and opened only when that digest still matches.
//!
//! The digest comes through the [`ContentHasher`] port, like every other
//! identity in this crate. A seal detects a changed or truncated file; it
//! is not an authenticity proof, which HTP-001's signed envelope owns and
//! a same-device switch does not need.

use omnifrons_domain::switch_checkpoint::{
    CheckpointError, SwitchCheckpoint, integrity_record, split_sealed,
};

use crate::content_hasher::ContentHasher;

/// Why a sealed checkpoint did not open.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpenError {
    /// The text is not a sealed checkpoint this version reads.
    #[error("the checkpoint is malformed: {0}")]
    Malformed(CheckpointError),
    /// The body no longer digests to the sealed value.
    #[error("the checkpoint's integrity digest does not match its body")]
    IntegrityMismatch,
}

/// `checkpoint`'s body followed by its `integrity` record.
#[must_use]
pub fn seal(checkpoint: &SwitchCheckpoint, hasher: &dyn ContentHasher) -> String {
    let mut sealed = checkpoint.encode_body();
    let digest = hasher.sha256(sealed.as_bytes());
    sealed.push_str(&integrity_record(&digest));
    sealed
}

/// The checkpoint `text` seals, once its integrity digest is checked.
///
/// # Errors
///
/// [`OpenError::IntegrityMismatch`] when the body does not digest to the
/// sealed value; [`OpenError::Malformed`] for anything that is not a
/// sealed checkpoint of a supported major.
pub fn open(text: &str, hasher: &dyn ContentHasher) -> Result<SwitchCheckpoint, OpenError> {
    let (body, claimed) = split_sealed(text).map_err(OpenError::Malformed)?;
    if hasher.sha256(body.as_bytes()) != claimed {
        return Err(OpenError::IntegrityMismatch);
    }
    SwitchCheckpoint::decode_body(body).map_err(OpenError::Malformed)
}
