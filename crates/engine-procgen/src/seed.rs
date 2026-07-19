//! Versioned deterministic hashing and seed derivation.
//!
//! # Algorithm (PROCGEN-SEED-v1)
//!
//! [`derive_seed`] is byte-wise **FNV-1a 64** over the concatenation
//!
//! ```text
//! parent_seed (8 bytes, little-endian) || "PROCGEN-SEED-v1\0" || key (UTF-8)
//! ```
//!
//! followed by a **splitmix64 finalizer** (`x ^= x >> 30; x *= C1; x ^= x >>
//! 27; x *= C2; x ^= x >> 31`). Both steps are pure wrapping `u64`
//! arithmetic — no data-dependent endianness, no platform intrinsics — so the
//! result is identical on every platform and in the C# port (`unchecked`
//! `ulong` ops over `Encoding.UTF8` bytes).
//!
//! The domain separator means seeds derived here never collide with ad-hoc
//! FNV usage elsewhere, and bumping the `PROCGEN-SEED-v*` tag is the explicit
//! mechanism for future algorithm changes.

use serde::{Deserialize, Serialize};

/// FNV-1a 64-bit offset basis.
pub(crate) const FNV1A64_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
pub(crate) const FNV1A64_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Domain separator absorbed by [`derive_seed`] after the parent seed bytes.
pub(crate) const SEED_DOMAIN: &[u8] = b"PROCGEN-SEED-v1\0";

/// A deterministic 64-bit seed.
///
/// `Seed` is a plain `u64` newtype so recipes and save files can store it as
/// an integer; serde serialization is transparent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Seed(pub u64);

impl Seed {
    /// The conventional unnamed root seed (`0`).
    pub const ROOT: Seed = Seed(0);

    /// Wrap a raw 64-bit value.
    pub const fn new(value: u64) -> Seed {
        Seed(value)
    }

    /// Derive a child seed; see [`derive_seed`].
    pub fn derive(self, key: &str) -> Seed {
        derive_seed(self, key)
    }
}

/// splitmix64-style finalizer shared by seed derivation and lattice hashing.
///
/// Pure wrapping `u64` ops; identical in Rust and the C# port.
pub(crate) fn splitmix64_finalize(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    value
}

/// Derive a child seed from a parent seed and a string key.
///
/// Stable across platforms, endianness, and processes for a given
/// [`crate::PROCGEN_SCHEMA`] version. The same `(parent, key)` pair always
/// yields the same child; different keys avalanche to unrelated children.
///
/// Typical usage namespaces children without global coordination:
///
/// ```
/// use engine_procgen::Seed;
/// let world = Seed::ROOT.derive("my-game/world");
/// let terrain = world.derive("terrain");
/// let trees = world.derive("placement/trees");
/// assert_ne!(terrain, trees);
/// ```
pub fn derive_seed(parent: Seed, key: &str) -> Seed {
    let mut hash = FNV1A64_OFFSET;
    for byte in parent.0.to_le_bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(FNV1A64_PRIME);
    }
    for byte in SEED_DOMAIN {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(FNV1A64_PRIME);
    }
    for byte in key.as_bytes() {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(FNV1A64_PRIME);
    }
    Seed(splitmix64_finalize(hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_seed_is_stable_across_calls() {
        assert_eq!(
            derive_seed(Seed(42), "terrain"),
            derive_seed(Seed(42), "terrain")
        );
        assert_eq!(Seed::ROOT.derive("a"), Seed::ROOT.derive("a"));
    }

    #[test]
    fn derive_seed_depends_on_parent_and_key() {
        let base = derive_seed(Seed(1), "terrain");
        assert_ne!(base, derive_seed(Seed(2), "terrain"));
        assert_ne!(base, derive_seed(Seed(1), "water"));
        assert_ne!(base, derive_seed(Seed(1), ""));
    }

    #[test]
    fn derive_seed_avalanche_sanity() {
        // Not a statistical proof: single-character key changes must flip a
        // reasonable share of the 64 output bits every time.
        for index in 0..64u32 {
            let a = derive_seed(Seed(0xDEAD_BEEF), &format!("key/{index}"));
            let b = derive_seed(Seed(0xDEAD_BEEF), &format!("key/{}", index + 1000));
            let flipped = (a.0 ^ b.0).count_ones();
            assert!(
                (12..=52).contains(&flipped),
                "expected an avalanche-ish flip count, got {flipped} for index {index}"
            );
        }
    }

    #[test]
    fn derive_seed_handles_unicode_and_empty_keys() {
        let _ = derive_seed(Seed::ROOT, "");
        let unicode = derive_seed(Seed::ROOT, "地形/🌲");
        assert_eq!(unicode, derive_seed(Seed::ROOT, "地形/🌲"));
        assert_ne!(unicode, derive_seed(Seed::ROOT, "地形/🌳"));
    }

    #[test]
    fn seed_serde_is_transparent_u64() {
        let json = serde_json::to_string(&Seed(42)).unwrap();
        assert_eq!(json, "42");
        let parsed: Seed = serde_json::from_str("42").unwrap();
        assert_eq!(parsed, Seed(42));
    }
}
