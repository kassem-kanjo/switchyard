//! Random streams, one per concern.
//!
//! ADR 0002 §3: every concern (arrivals at station 3, destinations at
//! station 3, the planner's escape moves, ...) draws from its own
//! `ChaCha8Rng` stream derived from `(seed, stream_id)`. Removing a fault or
//! adding a station therefore never shifts unrelated draws, which keeps
//! counterfactual replays (ADR 0005) free of random confounding.
//!
//! The 256-bit ChaCha key is expanded from the seed with SplitMix64, which we
//! implement here rather than rely on `SeedableRng::seed_from_u64`, so the
//! derivation is pinned by this crate and cannot change under a dependency
//! upgrade. The stream id goes into ChaCha's native 64-bit stream counter.

use rand_chacha::ChaCha8Rng;
use rand_core::{RngCore, SeedableRng};

/// Identifies one random stream. Ids are stable across versions: changing
/// one changes every run that uses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct StreamId(pub u64);

impl StreamId {
    /// Parcel arrivals at one station.
    #[must_use]
    pub const fn arrivals(station: u16) -> Self {
        Self(0x0001_0000 | station as u64)
    }

    /// Parcel destinations at one station.
    #[must_use]
    pub const fn destinations(station: u16) -> Self {
        Self(0x0002_0000 | station as u64)
    }

    /// The V0 planner's escape moves. Unused since guided PIBT (which needs
    /// no randomness), but the id stays reserved so it is never reused.
    pub const PLANNER: Self = Self(0x0003_0000);

    /// Scanner misreads at one station.
    #[must_use]
    pub const fn scanner(station: u16) -> Self {
        Self(0x0004_0000 | station as u64)
    }
}

/// A deterministic random stream (a thin wrapper, so the rest of the core
/// does not depend on `rand_core` traits directly).
#[derive(Clone, Debug)]
pub struct Stream {
    rng: ChaCha8Rng,
}

impl Stream {
    /// The stream for `(seed, id)`.
    #[must_use]
    pub fn new(seed: u64, id: StreamId) -> Self {
        let mut key = [0u8; 32];
        let mut sm = seed;
        for chunk in key.as_chunks_mut::<8>().0 {
            *chunk = splitmix64(&mut sm).to_le_bytes();
        }
        let mut rng = ChaCha8Rng::from_seed(key);
        rng.set_stream(id.0);
        Self { rng }
    }

    /// The next uniformly distributed `u64`.
    pub fn next_u64(&mut self) -> u64 {
        self.rng.next_u64()
    }

    /// A value in `0..n` (`n > 0`). Rejection sampling makes it exactly
    /// uniform, which matters for calibration rather than determinism.
    pub fn below(&mut self, n: u64) -> u64 {
        debug_assert!(n > 0, "below(0) is undefined");
        // Largest multiple of n that fits in u64; draws at or above it retry.
        let zone = u64::MAX - (u64::MAX % n);
        loop {
            let v = self.rng.next_u64();
            if v < zone {
                return v % n;
            }
        }
    }

    /// True with probability `numerator / denominator`, using integers only.
    pub fn chance(&mut self, numerator: u64, denominator: u64) -> bool {
        self.below(denominator) < numerator
    }

    /// The `index`-th `u64` of the stream, by seeking to it: a
    /// counter-based draw. Keying a draw on an entity (for example "parcel
    /// `k` at station `s`") makes its value independent of how many other
    /// draws happened before it, so a surge, an outage or a different scanner
    /// rate never shifts the draws of unrelated parcels (ADR 0002 §3, ADR 0005).
    /// It moves the stream's position, so a stream should be used either
    /// sequentially or keyed, not both.
    pub fn keyed_u64(&mut self, index: u64) -> u64 {
        // One u64 is two 32-bit words.
        self.rng.set_word_pos(u128::from(index) << 1);
        self.rng.next_u64()
    }

    /// A keyed value in `0..n` (`n > 0`), by Lemire's multiply-high
    /// reduction: one draw, no rejection loop (which would consume a
    /// variable number of draws). The bias is below `n / 2^64`.
    pub fn keyed_below(&mut self, index: u64, n: u64) -> u64 {
        let wide = u128::from(self.keyed_u64(index)) * u128::from(n);
        // The high half of a u64 x u64 product fits in u64.
        u64::try_from(wide >> 64).unwrap_or(u64::MAX)
    }

    /// Position of the stream (in 32-bit words). It is part of the state hash,
    /// so two runs that drew a different number of values never look equal.
    #[must_use]
    pub fn word_pos(&self) -> u128 {
        self.rng.get_word_pos()
    }
}

/// SplitMix64 (Steele, Lea and Flood, 2014): a standard way to expand a
/// 64-bit seed into well-mixed key material.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_and_stream_repeat() {
        let mut a = Stream::new(7, StreamId::PLANNER);
        let mut b = Stream::new(7, StreamId::PLANNER);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn streams_are_independent() {
        let mut a = Stream::new(7, StreamId::arrivals(0));
        let mut b = Stream::new(7, StreamId::arrivals(1));
        let mut c = Stream::new(8, StreamId::arrivals(0));
        let (x, y, z) = (a.next_u64(), b.next_u64(), c.next_u64());
        assert_ne!(x, y);
        assert_ne!(x, z);
    }

    #[test]
    fn below_stays_in_range() {
        let mut s = Stream::new(1, StreamId::PLANNER);
        for n in 1..50 {
            assert!(s.below(n) < n);
        }
    }
}
