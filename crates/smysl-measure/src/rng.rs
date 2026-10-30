//! The bootstrap generator (SMYSL-2.6 §3.5).
//!
//! A bootstrap interval is part of a payload, so the replicates that produced it are part of a
//! uid, and so the generator is part of the format. Two things are fixed here and nowhere else:
//!
//! - **The seed.** ChaCha20, keyed by `hash_bytes("smysl/measure-rng/1\0" ‖ recipe hash ‖
//!   seed)`, the seed as eight little-endian bytes. A recipe that changes anything — a scope, a
//!   replicate count — draws a different stream, and the same recipe draws the same one.
//! - **An index.** `index(n)` is `(x · n) >> 64` for the next 64-bit draw `x`, written out here
//!   rather than taken from `rand`'s `gen_range`, whose algorithm is a crate's choice and not a
//!   promise. The bias is below 2⁻⁴⁰ for `n < 2²⁴`, which is every sample a measure resamples.
//!
//! The stream is RFC 8439's ChaCha20 keystream under that key, block counter 0 and a zero nonce,
//! read as successive little-endian 64-bit words: `rand_chacha`'s 64-bit counter and stream id
//! coincide with RFC 8439's 32-bit counter and nonce while the stream id is 0 and fewer than 2³²
//! blocks are drawn, which no measure approaches. So a second implementation needs a standard
//! ChaCha20 and not this crate's layout (OQ-57, decided); `tests/payload.rs` pins the first
//! draws, and `nodejs/test/measure_rng.test.js` reproduces them with OpenSSL's ChaCha20.

use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};

/// The domain the generator's key is derived under.
pub const DOMAIN: &[u8] = b"smysl/measure-rng/1\0";

/// A seeded generator. No other kind exists in this crate: nothing can draw an unseeded number.
pub struct Rng(ChaCha20Rng);

impl Rng {
    /// The generator for a recipe and a seed.
    pub fn new(recipe_hash: &[u8; 32], seed: u64) -> Rng {
        let mut key = Vec::with_capacity(DOMAIN.len() + 32 + 8);
        key.extend_from_slice(DOMAIN);
        key.extend_from_slice(recipe_hash);
        key.extend_from_slice(&seed.to_le_bytes());
        Rng(ChaCha20Rng::from_seed(smysl_core::hash_bytes(&key)))
    }

    /// The next 64-bit draw. The only way a value leaves the generator.
    pub fn next_u64(&mut self) -> u64 {
        self.0.next_u64()
    }

    /// An index in `[0, n)`: `(x · n) >> 64`. `n = 0` has no index and returns 0, which no
    /// caller reaches — a resample of nothing is refused before it is drawn.
    pub fn index(&mut self, n: usize) -> usize {
        let x = self.next_u64();
        ((u128::from(x) * n as u128) >> 64) as usize
    }
}
