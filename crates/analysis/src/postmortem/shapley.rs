//! Exact Shapley attribution over a full lattice of replays (ADR 0006 §4;
//! research 2026-10-20 §3.2).
//!
//! The game: `n` candidate causes, and for every mask `S` (bit `i` set =
//! cause `i` removed) a value `v(S)`, the loss recovered by removing `S`
//! (parcel-ticks of delay saved). `v(∅)` is subtracted, so the game is
//! normalised whatever the caller passes.
//!
//! - **Harsanyi dividends** `m(T) = Σ_{U⊆T} (-1)^{|T|-|U|} v(U)`, by the
//!   in-place subset Möbius transform (`n·2ⁿ` steps).
//! - **Shapley value** `φᵢ = Σ_{T∋i} m(T) / |T|` (each dividend split
//!   equally among its members), equal to the classical
//!   `Σ_S |S|!(n-|S|-1)!/n! · (v(S∪i) - v(S))`.
//! - **Pair interaction** (Grabisch–Roubens) `I_ij = Σ_{T⊇{i,j}} m(T) /
//!   (|T|-1)`. In this *recovery* game, `I_ij > 0` means removing both
//!   recovers more than the two removals apart: the causes are
//!   **redundant** (overdetermination: either one alone does most of the
//!   damage, so removing just one helps little). `I_ij < 0` means removing
//!   either one recovers much of the same loss: the causes **compound**
//!   (together they did more damage than the sum of each alone).
//!   [`Interplay::of`] names the sign.
//!
//! Everything is held as integers scaled by `n!` (`i128`), so efficiency
//! (`Σ φᵢ = v(N) - v(∅)`) holds *exactly*; division happens only when a
//! value is rendered. Values may be negative (removing a cause can make
//! things worse); nothing here assumes monotonicity.

/// What the sign of a pair interaction means (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interplay {
    /// `I_ij > 0`: either cause alone does most of the damage.
    Redundant,
    /// `I_ij < 0`: together they did more damage than apart.
    Compound,
}

impl Interplay {
    /// The interplay of an interaction value (`None` for 0).
    #[must_use]
    pub fn of(interaction: f64) -> Option<Self> {
        if interaction > 0.0 {
            Some(Self::Redundant)
        } else if interaction < 0.0 {
            Some(Self::Compound)
        } else {
            None
        }
    }
}

/// Outcome of comparing a noisy value with a threshold.
///
/// Hard yes/no thresholds ignore replay noise: a set recovering 89.6 % when
/// `tau` is 90 % and the band is ±3 % is *within noise*, not a *no*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The value clears the threshold even at the bottom of the band.
    Yes,
    /// The value misses the threshold even at the top of the band.
    No,
    /// The band straddles the threshold: replay noise could go either way.
    WithinNoise,
}

impl Verdict {
    /// Yes if `value - band >= threshold`; No if `value + band < threshold`; else WithinNoise.
    /// With band 0 this is exactly `value >= threshold` (Yes) / else (No).
    #[must_use]
    pub fn of(value: f64, threshold: f64, band: f64) -> Self {
        if value - band >= threshold {
            Self::Yes
        } else if value + band < threshold {
            Self::No
        } else {
            Self::WithinNoise
        }
    }
}

/// Minimal sufficient sets with the noise band applied.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MinimalSets {
    /// Smallest masks that are a definite Yes (ties in mask order).
    pub sure: Vec<usize>,
    /// Masks of size <= the size of `sure` (or, if `sure` is empty, of the smallest size having any) whose verdict is WithinNoise, in (size, mask) order.
    pub near: Vec<usize>,
}

/// Responsibility with the band: smallest contingency size that is definitely pivotal, and smallest that is possibly pivotal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Contingency {
    /// Fewest other causes that must also be removed before removing `i`
    /// certainly tips the recovery over the threshold.
    pub sure: Option<u32>,
    /// Fewest other causes after which removing `i` *may* tip the recovery
    /// (its value sits within the noise band); ≤ `sure`.
    pub possible: Option<u32>,
}

/// Most causes the lattice supports (2¹⁶ masks); the post-mortem uses ≤ 6.
pub const MAX_PLAYERS: usize = 16;

/// The attribution of one game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribution {
    /// Number of players `n`.
    pub n: usize,
    /// `v(S) - v(∅)` for every mask.
    pub v: Vec<i64>,
    /// Harsanyi dividend of every mask.
    pub dividends: Vec<i128>,
    /// `n! · φᵢ`.
    pub shapley_scaled: Vec<i128>,
    /// `n! · I_ij` (symmetric; the diagonal is 0).
    pub interaction_scaled: Vec<Vec<i128>>,
    /// The common scale `n!`.
    pub scale: i128,
}

fn factorial(n: usize) -> i128 {
    (1..=n).map(|k| k as i128).product()
}

impl Attribution {
    /// Attribute the game `v` (indexed by mask, `2ⁿ` entries).
    ///
    /// # Panics
    /// If `v.len()` is not a power of two up to `2^MAX_PLAYERS`.
    #[must_use]
    pub fn new(v: &[i64]) -> Self {
        assert!(
            v.len().is_power_of_two() && v.len() <= 1 << MAX_PLAYERS,
            "one value per mask"
        );
        let n = v.len().trailing_zeros() as usize;
        let base = v[0];
        let v: Vec<i64> = v.iter().map(|&x| x - base).collect();
        let mut m: Vec<i128> = v.iter().map(|&x| i128::from(x)).collect();
        for i in 0..n {
            let bit = 1 << i;
            for mask in 0..m.len() {
                if mask & bit != 0 {
                    m[mask] -= m[mask ^ bit];
                }
            }
        }
        let scale = factorial(n);
        let size = |mask: usize| mask.count_ones() as i128;
        let mut shapley_scaled = vec![0i128; n];
        let mut interaction_scaled = vec![vec![0i128; n]; n];
        for (mask, &d) in m.iter().enumerate().skip(1) {
            let t = size(mask);
            for i in (0..n).filter(|i| mask & (1 << i) != 0) {
                shapley_scaled[i] += d * (scale / t);
                if t >= 2 {
                    for j in (0..n).filter(|&j| j != i && mask & (1 << j) != 0) {
                        interaction_scaled[i][j] += d * (scale / (t - 1));
                    }
                }
            }
        }
        Self {
            n,
            v,
            dividends: m,
            shapley_scaled,
            interaction_scaled,
            scale,
        }
    }

    /// The mask with every player.
    #[must_use]
    pub const fn all(&self) -> usize {
        (1 << self.n) - 1
    }

    /// `v(N) - v(∅)`: the loss recovered by removing every cause.
    #[must_use]
    pub fn total(&self) -> i64 {
        self.v[self.all()]
    }

    /// `φᵢ`.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn shapley(&self, i: usize) -> f64 {
        self.shapley_scaled[i] as f64 / self.scale as f64
    }

    /// `I_ij`.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn interaction(&self, i: usize, j: usize) -> f64 {
        self.interaction_scaled[i][j] as f64 / self.scale as f64
    }

    /// `v({i})`: removing `i` alone.
    #[must_use]
    pub fn alone(&self, i: usize) -> i64 {
        self.v[1 << i]
    }

    /// Leave-one-out `v(N) - v(N \ {i})`: what keeping only `i` costs once
    /// everything else is removed (but-for, given the others are gone).
    #[must_use]
    pub fn leave_one_out(&self, i: usize) -> i64 {
        self.total() - self.v[self.all() ^ (1 << i)]
    }

    /// Does recovering `x` reach `tau_pct` % of `v(N)`?
    fn reaches(&self, x: i64, tau_pct: i64) -> bool {
        i128::from(x) * 100 >= i128::from(self.total()) * i128::from(tau_pct)
    }

    /// Smallest masks that recover at least `tau_pct` % of `v(N)`, all of
    /// the minimal size (ties listed in mask order). Empty if `v(N) <= 0`.
    #[must_use]
    pub fn minimal_sufficient(&self, tau_pct: i64) -> Vec<usize> {
        let goal = |x: i64| self.reaches(x, tau_pct);
        if self.total() <= 0 {
            return Vec::new();
        }
        (1..=self.n)
            .map(|k| {
                (1..=self.all())
                    .filter(|&s| s.count_ones() as usize == k && goal(self.v[s]))
                    .collect::<Vec<_>>()
            })
            .find(|sets| !sets.is_empty())
            .unwrap_or_default()
    }

    /// Chockler–Halpern degree of responsibility as the smallest contingency
    /// size `k`: the fewest other causes that must also be removed before
    /// removing `i` tips the recovery over `tau_pct` % of `v(N)`
    /// (responsibility `1/(1+k)`). `None` if `i` is never pivotal.
    #[must_use]
    pub fn contingency(&self, i: usize, tau_pct: i64) -> Option<u32> {
        if self.total() <= 0 {
            return None;
        }
        let goal = |x: i64| self.reaches(x, tau_pct);
        let bit = 1 << i;
        (0..=self.all())
            .filter(|&t| t & bit == 0 && !goal(self.v[t]) && goal(self.v[t | bit]))
            .map(|t| t.count_ones())
            .min()
    }

    /// Verdict for "does recovering `x` reach `tau_pct` % of v(N)?" given band (parcel-ticks, same unit as v).
    ///
    /// The threshold is `v(N) · tau_pct / 100` in `f64`; with band 0 this
    /// decides exactly like the integer test in the private `reaches`
    /// (the product stays exact in `f64` for the magnitudes at hand, and a
    /// non-integral threshold is at least 0.01 away from any mask value,
    /// so rounding cannot flip a comparison).
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // threshold is only compared, never accumulated
    pub fn verdict(&self, x: i64, tau_pct: i64, band: f64) -> Verdict {
        let threshold = self.total() as f64 * tau_pct as f64 / 100.0;
        Verdict::of(x as f64, threshold, band)
    }

    /// Verdict for the mask `s`. The full set `N` recovers all of `v(N)` by
    /// definition, so it is always `Yes`: the noise band cannot make "remove
    /// every cause" insufficient to explain the loss it is defined as.
    fn verdict_of_mask(&self, s: usize, tau_pct: i64, bands: &[f64]) -> Verdict {
        if s == self.all() {
            Verdict::Yes
        } else {
            self.verdict(self.v[s], tau_pct, bands[s])
        }
    }

    /// Minimal sufficient sets with the noise band applied: `sure` holds the
    /// smallest masks that *definitely* reach `tau_pct` %, `near` the
    /// WithinNoise masks that might with less replay noise. Empty (default)
    /// if `v(N) <= 0`.
    #[must_use]
    pub fn minimal_sufficient_within(&self, tau_pct: i64, band: f64) -> MinimalSets {
        self.minimal_sufficient_banded(tau_pct, &vec![band; self.v.len()])
    }

    /// [`Self::minimal_sufficient_within`] with a band per mask (`bands[s]`
    /// is the noise of `v(s)`, see [`mask_bands`]; #18).
    ///
    /// # Panics
    /// If `bands` has fewer entries than there are masks.
    #[must_use]
    pub fn minimal_sufficient_banded(&self, tau_pct: i64, bands: &[f64]) -> MinimalSets {
        if self.total() <= 0 {
            return MinimalSets::default();
        }
        // Walk the sizes upward, remembering each size's WithinNoise masks
        // until we know how far "near" may reach. A verdict is never both
        // Yes and WithinNoise, so `sure` and `near` are disjoint for free.
        let mut noise_sizes: Vec<Vec<usize>> = Vec::new();
        for k in 1..=self.n {
            let (mut yes, mut noise) = (Vec::new(), Vec::new());
            for s in 1..=self.all() {
                if s.count_ones() as usize == k {
                    match self.verdict_of_mask(s, tau_pct, bands) {
                        Verdict::Yes => yes.push(s),
                        Verdict::WithinNoise => noise.push(s),
                        Verdict::No => {}
                    }
                }
            }
            if !yes.is_empty() {
                // First definite Yes: everything WithinNoise at or below this
                // size might recover enough with less noise.
                noise_sizes.push(noise);
                let near = noise_sizes.into_iter().flatten().collect();
                return MinimalSets { sure: yes, near };
            }
            noise_sizes.push(noise);
        }
        // Nothing is a definite Yes: near is only the smallest size that
        // could still reach the threshold inside the band.
        let near = noise_sizes
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or_default();
        MinimalSets {
            sure: Vec::new(),
            near,
        }
    }

    /// Chockler–Halpern responsibility with the noise band: `sure` is the
    /// smallest contingency after which removing `i` *definitely* tips the
    /// recovery over `tau_pct` %, `possible` the smallest after which it
    /// *may* (its value sits inside the band, so noise could tip it either
    /// way). Fields are `None` when no such contingency exists or
    /// `v(N) <= 0`.
    #[must_use]
    pub fn contingency_within(&self, i: usize, tau_pct: i64, band: f64) -> Contingency {
        self.contingency_banded(i, tau_pct, &vec![band; self.v.len()])
    }

    /// [`Self::contingency_within`] with a band per mask (#18).
    ///
    /// # Panics
    /// If `bands` has fewer entries than there are masks.
    #[must_use]
    pub fn contingency_banded(&self, i: usize, tau_pct: i64, bands: &[f64]) -> Contingency {
        if self.total() <= 0 {
            return Contingency {
                sure: None,
                possible: None,
            };
        }
        let verdict = |s: usize| self.verdict_of_mask(s, tau_pct, bands);
        let bit = 1 << i;
        // Sure pivot: without `i` the contingency is a definite No and with
        // `i` a definite Yes.
        let sure = (0..=self.all())
            .filter(|&t| {
                t & bit == 0 && verdict(t) == Verdict::No && verdict(t | bit) == Verdict::Yes
            })
            .map(|t| t.count_ones())
            .min();
        // Possible pivot: the without-`i` value is not already a Yes and the
        // with-`i` value is not already a No, i.e. the band straddles the
        // threshold somewhere in between.
        let possible = (0..=self.all())
            .filter(|&t| {
                t & bit == 0 && verdict(t) != Verdict::Yes && verdict(t | bit) != Verdict::No
            })
            .map(|t| t.count_ones())
            .min();
        Contingency { sure, possible }
    }
}

/// Half the replay-noise band of every lattice value, per mask.
///
/// `per_history[h][mask]` is the value `v(mask)` replayed on history `h`.
/// Each mask has its own replay noise: the dither histories diverge the
/// planner, so `v(S)` varies from history to history independently of
/// `v(∅)`. A mask's verdict must therefore be judged with its own band, not
/// the loss band (#18): the loss band describes the empty mask's delay, not
/// the value whose threshold is being tested.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn mask_bands(per_history: &[Vec<i64>]) -> Vec<f64> {
    let Some(first) = per_history.first() else {
        return Vec::new();
    };
    (0..first.len())
        .map(|mask| super::band(per_history.iter().map(|v| v[mask] as f64)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Classical Shapley `n! · φᵢ = Σ_S |S|!(n-|S|-1)! (v(S∪i) - v(S))`.
    fn brute(v: &[i64], i: usize) -> i128 {
        let n = v.len().trailing_zeros() as usize;
        (0..v.len())
            .filter(|s| s & (1 << i) == 0)
            .map(|s| {
                let k = s.count_ones() as usize;
                factorial(k)
                    * factorial(n - k - 1)
                    * (i128::from(v[s | (1 << i)]) - i128::from(v[s]))
            })
            .sum()
    }

    fn game() -> impl Strategy<Value = Vec<i64>> {
        (1usize..=6).prop_flat_map(|n| {
            prop::collection::vec(-1_000_000_000i64..1_000_000_000, 1 << n).prop_map(|mut v| {
                v[0] = 0;
                v
            })
        })
    }

    #[test]
    fn compounding_causes_split_the_credit_and_interact_negatively() {
        // Removing either cause alone recovers 80 of 100: alone, each would
        // have done only 20, together 100. They compound.
        let a = Attribution::new(&[0, 80, 80, 100]);
        assert_eq!(a.shapley_scaled, vec![100, 100]); // 50 each, n! = 2
        assert!((a.shapley(0) - 50.0).abs() < 1e-12);
        assert_eq!(a.dividends, vec![0, 80, 80, -60]);
        assert!((a.interaction(0, 1) + 60.0).abs() < 1e-12);
        assert_eq!(
            Interplay::of(a.interaction(0, 1)),
            Some(Interplay::Compound)
        );
        assert_eq!((a.alone(0), a.leave_one_out(0)), (80, 20));
        assert_eq!(a.minimal_sufficient(90), vec![3]);
        assert_eq!(a.minimal_sufficient(80), vec![1, 2]);
        // Pivotal only once the other is gone: responsibility 1/2.
        assert_eq!(a.contingency(0, 90), Some(1));
    }

    #[test]
    fn overdetermined_causes_are_redundant() {
        // Two identical jams of one chute: removing either alone recovers
        // nothing (the other still jams it); removing both recovers all.
        let l = 1_000;
        let a = Attribution::new(&[0, 0, 0, l]);
        assert_eq!(a.shapley_scaled, vec![l.into(), l.into()]); // L/2 each, n! = 2
        assert!((a.interaction(0, 1) - 1_000.0).abs() < 1e-12);
        assert_eq!(
            Interplay::of(a.interaction(0, 1)),
            Some(Interplay::Redundant)
        );
        // Each is pivotal only once the other is gone: responsibility 1/2.
        assert_eq!(a.contingency(0, 90), Some(1));
        assert_eq!(Interplay::of(0.0), None);
    }

    #[test]
    fn a_harmful_removal_gets_an_honest_negative_share() {
        // Removing cause 1 alone makes things worse by 30 (it was masking
        // cause 0's damage); removing both recovers 100.
        let a = Attribution::new(&[0, 120, -30, 100]);
        assert_eq!(a.shapley_scaled.iter().sum::<i128>(), 2 * 100);
        assert!(a.shapley(1) < 0.0);
        assert_eq!(a.contingency(0, 90), Some(0));
        assert_eq!(a.contingency(1, 90), None);
    }

    #[test]
    fn the_base_value_is_subtracted() {
        let a = Attribution::new(&[7, 17]);
        assert_eq!((a.total(), a.shapley_scaled[0]), (10, 10));
        assert!(
            Attribution::new(&[0, 0, 0, -5])
                .minimal_sufficient(90)
                .is_empty()
        );
    }

    #[test]
    fn verdict_boundaries() {
        // Band 2 around a threshold of 10: the Yes edge is inclusive
        // (12 - 2 = 10 counts), the No edge is exclusive
        // (8 + 2 = 10 is still straddling the threshold).
        assert_eq!(Verdict::of(12.0, 10.0, 2.0), Verdict::Yes);
        assert_eq!(Verdict::of(11.9, 10.0, 2.0), Verdict::WithinNoise);
        assert_eq!(Verdict::of(8.0, 10.0, 2.0), Verdict::WithinNoise);
        assert_eq!(Verdict::of(7.9, 10.0, 2.0), Verdict::No);
        // Band 0 degenerates to the hard comparison.
        assert_eq!(Verdict::of(10.0, 10.0, 0.0), Verdict::Yes);
        assert_eq!(Verdict::of(9.99, 10.0, 0.0), Verdict::No);
    }

    #[test]
    fn the_full_set_is_always_sufficient_whatever_the_noise() {
        // One cause, v(N) = 100, band 25: 100 - 25 < the 90 threshold, so the
        // plain value test says WithinNoise. But removing every cause recovers
        // v(N) by definition (audit #19: sole cause showed responsibility 0).
        let a = Attribution::new(&[0, 100]);
        let m = a.minimal_sufficient_within(90, 25.0);
        assert_eq!(m.sure, vec![1]);
        assert!(m.near.is_empty());
        assert_eq!(
            a.contingency_within(0, 90, 25.0),
            Contingency {
                sure: Some(0),
                possible: Some(0),
            }
        );
    }

    #[test]
    fn zero_band_matches_the_hard_thresholds() {
        let a = Attribution::new(&[0, 80, 80, 100]);
        assert_eq!(
            a.minimal_sufficient_within(90, 0.0),
            MinimalSets {
                sure: vec![3],
                near: vec![],
            }
        );
        assert_eq!(a.minimal_sufficient_within(80, 0.0).sure, vec![1, 2]);
        assert_eq!(
            a.contingency_within(0, 90, 0.0),
            Contingency {
                sure: Some(1),
                possible: Some(1),
            }
        );
    }

    #[test]
    fn near_miss_sets_are_listed() {
        // 88 of 100 with band 3: 88 + 3 = 91 >= 90 so WithinNoise, and
        // 88 - 3 = 85 < 90 so it is not a Yes; removal of both is.
        let a = Attribution::new(&[0, 88, 60, 100]);
        assert_eq!(
            a.minimal_sufficient_within(90, 3.0),
            MinimalSets {
                sure: vec![3],
                near: vec![1],
            }
        );
    }

    #[test]
    fn each_mask_is_judged_against_its_own_band() {
        // Game [0, 88, 60, 100], tau 90: mask 1 (88) is the only near miss.
        // A uniform band of 3 puts it within noise; a quiet mask 1 (band 0)
        // makes it a clear No, a noisy one (band 3) keeps it a near miss, and
        // a big band on mask 2 makes that mask the near miss instead.
        let a = Attribution::new(&[0, 88, 60, 100]);
        assert_eq!(
            a.minimal_sufficient_banded(90, &[0.0, 3.0, 0.0, 0.0]).near,
            vec![1]
        );
        assert_eq!(
            a.minimal_sufficient_banded(90, &[0.0, 0.0, 50.0, 0.0]).near,
            vec![2]
        );
        assert_eq!(
            a.minimal_sufficient_banded(90, &[3.0; 4]),
            a.minimal_sufficient_within(90, 3.0)
        );
        assert_eq!(
            a.contingency_banded(0, 90, &[3.0; 4]),
            a.contingency_within(0, 90, 3.0)
        );
    }

    #[test]
    fn contingency_flags_noise() {
        // Game [0, 88, 60, 100], tau 90, band 3, cause 0 (bit 1). Hand
        // derivation over the contingencies t without bit 0:
        // - t = ∅: v[∅] = 0 → No; v[{0}] = 88 → WithinNoise (88 + 3 ≥ 90).
        //   Not a sure pivot (need Yes on top), but a possible one.
        // - t = {1}: v[{1}] = 60 → No; v[{0,1}] = 100 → Yes. A sure pivot.
        // Hence sure = Some(1) (t = {1}) yet possible = Some(0) (t = ∅):
        // the band says removing cause 0 *may* already tip the recovery.
        let a = Attribution::new(&[0, 88, 60, 100]);
        assert_eq!(
            a.contingency_within(0, 90, 3.0),
            Contingency {
                sure: Some(1),
                possible: Some(0),
            }
        );
    }

    #[test]
    fn mask_bands_use_each_masks_own_spread() {
        let bands = mask_bands(&[vec![0, 10, 20, 30], vec![0, 14, 20, 26]]);
        assert_eq!(bands, vec![0.0, 2.0, 0.0, 2.0]);
    }

    #[test]
    fn a_single_history_has_zero_band() {
        assert_eq!(mask_bands(&[vec![0, 10, 20, 30]]), vec![0.0; 4]);
    }

    #[test]
    fn no_histories_means_no_bands() {
        assert!(mask_bands(&[]).is_empty());
    }

    #[test]
    fn a_negative_value_widens_its_band() {
        let bands = mask_bands(&[vec![0, -4], vec![0, 6], vec![0, 2]]);
        assert_eq!(bands, vec![0.0, 5.0]);
    }

    proptest! {
        #[test]
        fn within_with_zero_band_equals_hard(v in game()) {
            let a = Attribution::new(&v);
            for tau in [50, 90] {
                let within = a.minimal_sufficient_within(tau, 0.0);
                prop_assert_eq!(within.sure, a.minimal_sufficient(tau));
                prop_assert!(within.near.is_empty());
            }
        }

        #[test]
        fn shapley_is_efficient_and_matches_the_classical_formula(v in game()) {
            let a = Attribution::new(&v);
            prop_assert_eq!(
                a.shapley_scaled.iter().sum::<i128>(),
                a.scale * i128::from(a.total())
            );
            for i in 0..a.n {
                prop_assert_eq!(a.shapley_scaled[i], brute(&v, i));
            }
        }

        #[test]
        fn symmetric_players_get_equal_shares(v in game()) {
            prop_assume!(v.len() >= 4);
            // Symmetrise players 0 and 1: w(S) = v(S) + v(swap(S)).
            let swap = |s: usize| (s & !3) | ((s & 1) << 1) | ((s >> 1) & 1);
            let w: Vec<i64> = (0..v.len()).map(|s| v[s] + v[swap(s)]).collect();
            let a = Attribution::new(&w);
            prop_assert_eq!(a.shapley_scaled[0], a.shapley_scaled[1]);
            prop_assert_eq!(a.interaction_scaled[0][1], a.interaction_scaled[1][0]);
        }

        #[test]
        fn a_dummy_player_gets_exactly_its_own_value(v in game(), c in -1_000_000i64..1_000_000) {
            // Add a player 0 whose removal always recovers exactly c.
            let w: Vec<i64> = (0..2 * v.len())
                .map(|s| v[s >> 1] + if s & 1 == 1 { c } else { 0 })
                .collect();
            let a = Attribution::new(&w);
            prop_assert_eq!(a.shapley_scaled[0], a.scale * i128::from(c));
            for j in 1..a.n {
                prop_assert_eq!(a.interaction_scaled[0][j], 0);
            }
            // A null player (c = 0) gets nothing.
            let null: Vec<i64> = (0..2 * v.len()).map(|s| v[s >> 1]).collect();
            prop_assert_eq!(Attribution::new(&null).shapley_scaled[0], 0);
        }

        #[test]
        fn shapley_is_additive((v, w) in game().prop_flat_map(|v| {
            let len = v.len();
            (Just(v), prop::collection::vec(-1_000_000_000i64..1_000_000_000, len))
        })) {
            let sum: Vec<i64> = v.iter().zip(&w).map(|(a, b)| a + b).collect();
            let (a, b, c) = (Attribution::new(&v), Attribution::new(&w), Attribution::new(&sum));
            for i in 0..a.n {
                prop_assert_eq!(c.shapley_scaled[i], a.shapley_scaled[i] + b.shapley_scaled[i]);
            }
        }

        #[test]
        fn dividends_invert_to_the_game(v in game()) {
            // v(S) = Σ_{T⊆S} m(T) (zeta transform).
            let a = Attribution::new(&v);
            for s in 0..v.len() {
                let back: i128 = (0..v.len()).filter(|t| t & !s == 0).map(|t| a.dividends[t]).sum();
                prop_assert_eq!(back, i128::from(v[s]));
            }
        }
    }
}
