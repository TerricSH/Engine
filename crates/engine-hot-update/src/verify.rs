use engine_serialize::HashDigest;
use sha2::{Digest, Sha256};

/// Compute the SHA-256 hash of a byte slice, returning a [`HashDigest`]
/// (`[u8; 32]`).
pub fn compute_hash(data: &[u8]) -> HashDigest {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&result);
    hash
}
