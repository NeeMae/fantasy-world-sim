//! Deterministic random number streams.
//!
//! Systems never share one RNG: that would make results depend on iteration
//! and thread order. Instead each consumer derives its own stream from
//! `(world seed, tick, entity, purpose)`, so parallel code is reproducible.

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

pub type SimRng = ChaCha8Rng;

/// Stable identifiers for what a random stream is used for, so two systems
/// rolling for the same entity on the same tick get independent numbers.
pub mod purpose {
    pub const WORLDGEN: u64 = 1;
    pub const NAMES: u64 = 2;
}

/// A random stream unique to `(seed, tick, entity, purpose)`.
pub fn stream(seed: u64, tick: u64, entity: u64, purpose: u64) -> SimRng {
    SimRng::seed_from_u64(mix(seed, tick, entity, purpose))
}

/// Combines inputs into one well-distributed 64-bit value.
pub fn mix(seed: u64, tick: u64, entity: u64, purpose: u64) -> u64 {
    let mut h = splitmix64(seed);
    for v in [tick, entity, purpose] {
        h = splitmix64(h ^ v);
    }
    h
}

fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngExt;

    #[test]
    fn streams_are_reproducible_and_independent() {
        let a: u64 = stream(42, 7, 3, purpose::WORLDGEN).random();
        let b: u64 = stream(42, 7, 3, purpose::WORLDGEN).random();
        assert_eq!(a, b);
        assert_ne!(a, stream(42, 7, 4, purpose::WORLDGEN).random::<u64>());
        assert_ne!(a, stream(42, 8, 3, purpose::WORLDGEN).random::<u64>());
        assert_ne!(a, stream(43, 7, 3, purpose::WORLDGEN).random::<u64>());
    }
}
