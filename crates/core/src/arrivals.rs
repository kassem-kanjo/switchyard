//! Parcel arrival sampling: Poisson counts per tick and Zipf destinations.
//!
//! Both samplers are inverse-CDF tables in Q32.32 fixed point, built once
//! with integer math ([`crate::fixed`]), so they are bit-identical on every
//! platform (ADR 0002 §1). Every station has its own streams, so the
//! arrivals at one station never depend on another station (ADR 0002 §3):
//! - the Poisson count takes exactly one `u64` per station per tick, so the
//!   arrival stream's position is a function of the tick alone;
//! - a destination is a *keyed* draw ([`Stream::keyed_below`]) on the
//!   parcel's ordinal at its station, so parcel `k` at station `s` gets the
//!   same destination however many parcels arrived before it.

use crate::TICKS_PER_HOUR;
use crate::fixed::{ONE, exp_neg, pow_neg_milli};
use crate::rng::Stream;

/// Poisson-distributed arrivals per tick for a rate given per hour.
///
/// A Poisson process with rate `λ` per tick has a Poisson(`λ`) count in each
/// tick, independently, so drawing one count per tick is exact (no
/// sub-stepping needed). The table holds the CDF `P(N <= k)` in Q32.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoissonTable {
    per_hour: u32,
    cdf: Vec<u64>,
}

impl PoissonTable {
    /// Largest count the table represents; the tail beyond it has
    /// probability below 2^-32 for any rate the config allows.
    const MAX_K: u32 = 24;

    /// The table for `per_hour` arrivals per hour (`λ = per_hour / 7,200`
    /// per tick).
    #[must_use]
    pub fn new(per_hour: u32) -> Self {
        let lambda = u128::from(per_hour) * u128::from(ONE) / u128::from(TICKS_PER_HOUR);
        let mut pmf = u128::from(exp_neg(u64::try_from(lambda).unwrap_or(u64::MAX)));
        let mut acc = pmf;
        let mut cdf = vec![u64::try_from(acc).unwrap_or(ONE)];
        for k in 1..=Self::MAX_K {
            // P(k) = P(k - 1) * λ / k.
            pmf = pmf * lambda / u128::from(ONE) / u128::from(k);
            if pmf == 0 {
                break;
            }
            acc += pmf;
            cdf.push(u64::try_from(acc).unwrap_or(ONE));
        }
        Self { per_hour, cdf }
    }

    /// The rate the table was built for, per hour.
    #[must_use]
    pub const fn per_hour(&self) -> u32 {
        self.per_hour
    }

    /// Count for a uniform draw `u`.
    #[must_use]
    pub fn sample_with(&self, u: u64) -> u32 {
        let x = u >> 32;
        let k = self.cdf.partition_point(|&c| c <= x);
        // Truncation can leave the last entry a hair below 1; a draw past it
        // belongs to the largest count kept.
        u32::try_from(k.min(self.cdf.len() - 1)).unwrap_or(0)
    }

    /// Draw the number of arrivals in one tick (always one draw).
    pub fn sample(&self, stream: &mut Stream) -> u32 {
        self.sample_with(stream.next_u64())
    }
}

/// Zipf-distributed destinations: destination `d` (0-based) has weight
/// `(d + 1)^-s`, with `s = s_milli / 1000`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZipfTable {
    /// Cumulative weights, Q32 each.
    cdf: Vec<u64>,
}

impl ZipfTable {
    /// The table for `n >= 1` destinations and exponent `s_milli / 1000`.
    #[must_use]
    pub fn new(n: u16, s_milli: u32) -> Self {
        let mut acc = 0u64;
        let cdf = (1..=u64::from(n.max(1)))
            .map(|rank| {
                // Weights are at least 1 so every destination stays possible.
                acc += pow_neg_milli(rank, s_milli).max(1);
                acc
            })
            .collect();
        Self { cdf }
    }

    /// Number of destinations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cdf.len()
    }

    /// True if the table has no destinations (never, by construction).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cdf.is_empty()
    }

    /// The destination of the parcel with ordinal `index` on `stream` (a
    /// keyed draw: independent of every other parcel).
    pub fn sample_keyed(&self, stream: &mut Stream, index: u64) -> u16 {
        let total = self.cdf.last().copied().unwrap_or(1);
        let x = stream.keyed_below(index, total);
        let d = self.cdf.partition_point(|&c| c <= x);
        u16::try_from(d).unwrap_or(u16::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::StreamId;

    #[test]
    fn zero_rate_never_arrives() {
        let t = PoissonTable::new(0);
        let mut s = Stream::new(1, StreamId::arrivals(0));
        assert!((0..10_000).all(|_| t.sample(&mut s) == 0));
    }

    #[test]
    fn poisson_mean_and_zero_probability_match_lambda() {
        // λ = 1 per tick: P(0) = e^-1, mean 1.
        let t = PoissonTable::new(TICKS_PER_HOUR);
        let mut s = Stream::new(3, StreamId::arrivals(0));
        let n = 200_000u64;
        let (mut total, mut zeros) = (0u64, 0u64);
        for _ in 0..n {
            let k = u64::from(t.sample(&mut s));
            total += k;
            zeros += u64::from(k == 0);
        }
        // Mean 1 +- 1 %; P(0) = 0.3679 +- 0.005.
        assert!((n * 99 / 100..=n * 101 / 100).contains(&total), "{total}");
        assert!((72_580..=74_580).contains(&zeros), "{zeros}");

        // λ = 800/7200 per tick, the medium preset: mean within 2 %.
        let t = PoissonTable::new(800);
        let expect = n * 800 / u64::from(TICKS_PER_HOUR);
        let total: u64 = (0..n).map(|_| u64::from(t.sample(&mut s))).sum();
        assert!(
            (expect * 98 / 100..=expect * 102 / 100).contains(&total),
            "{total} vs {expect}"
        );
    }

    #[test]
    fn poisson_table_is_a_cdf() {
        for rate in [1, 400, 800, TICKS_PER_HOUR] {
            let t = PoissonTable::new(rate);
            assert!(t.cdf.windows(2).all(|w| w[0] < w[1]), "{rate}");
            assert!(*t.cdf.last().unwrap() <= ONE);
            assert!(ONE - t.cdf.last().unwrap() < 16, "{rate}: tail too heavy");
            assert_eq!(
                t.sample_with(u64::MAX),
                u32::try_from(t.cdf.len() - 1).unwrap()
            );
            assert_eq!(t.sample_with(0), 0);
        }
    }

    #[test]
    fn zipf_uniform_and_skewed() {
        let mut s = Stream::new(9, StreamId::destinations(0));
        let uniform = ZipfTable::new(4, 0);
        let mut counts = [0u32; 4];
        for i in 0..40_000 {
            counts[usize::from(uniform.sample_keyed(&mut s, i))] += 1;
        }
        assert!(
            counts.iter().all(|&c| (9_500..=10_500).contains(&c)),
            "{counts:?}"
        );

        // s = 1: destination 0 is drawn twice as often as destination 1.
        let skewed = ZipfTable::new(100, 1_000);
        let mut counts = vec![0u32; 100];
        for i in 0..100_000 {
            counts[usize::from(skewed.sample_keyed(&mut s, i))] += 1;
        }
        let ratio_milli = counts[0] * 1_000 / counts[1];
        assert!((1_900..=2_100).contains(&ratio_milli), "{ratio_milli}");
        assert!(counts[99] > 0);
        assert_eq!(skewed.len(), 100);
    }
}
