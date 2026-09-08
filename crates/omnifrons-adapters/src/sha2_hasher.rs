//! `Sha2Hasher`: the real `omnifrons_app::content_hasher::ContentHasher`
//! over `sha2` (spike slice 5b) -- the one place SHA-256 is actually
//! computed for a publication identity, an artifact approval id, a project
//! identity, or a published copy's re-verification. `omnifrons-app`
//! defines the preimages; this adapter only supplies the function.

use std::io::Read;

use omnifrons_app::content_hasher::ContentHasher;
use omnifrons_domain::executable::Sha256Digest;
use sha2::{Digest, Sha256};

/// The buffer size used to stream-hash a reader.
const HASH_BUFFER_BYTES: usize = 64 * 1024;

/// The real SHA-256 [`ContentHasher`]. Stateless.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sha2Hasher;

impl Sha2Hasher {
    /// Build a hasher.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ContentHasher for Sha2Hasher {
    fn sha256(&self, input: &[u8]) -> Sha256Digest {
        let digest: [u8; 32] = Sha256::digest(input).into();
        Sha256Digest(digest)
    }

    fn digest_reader(&self, reader: &mut dyn Read) -> std::io::Result<(Sha256Digest, u64)> {
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; HASH_BUFFER_BYTES];
        let mut size: u64 = 0;
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            size = size.saturating_add(read as u64);
        }
        let digest: [u8; 32] = hasher.finalize().into();
        Ok((Sha256Digest(digest), size))
    }
}
