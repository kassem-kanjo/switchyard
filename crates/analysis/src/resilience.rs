//! Resilience scoring of a faulted run against its fault-free twin
//! (ARCHITECTURE §7, step 3; reliability research §1(b) and §3).
//!
//! The twin is the same scenario and seed without the faults. Because every
//! random draw is keyed per station and per parcel (ADR 0002 §3), the twin
//! sees exactly the same regular parcels, so the comparison is noise-free
//! common random numbers: any difference is caused by the faults
//! (research §3, sources [20][21]).
//!
//! # The resilience curve
//! Let `PB(t)` be the twin's deliveries in minute `t` (the benchmark) and
//! `P(t)` the faulted run's deliveries of the same regular parcels (a volume
//! surge's extra parcels are extra *work*, not output: counting them would
//! let a surge look like a gain). `t0` is the minute the first fault starts
//! and `te` the minute after the last one ends.
//!
//! - **In band:** minute `t` is within 5 % of the twin when
//!   `P(t) >= 0.95 * PB(t)` (one-sided: catching up above the twin is fine).
//! - **Time to recover** (recovery rapidity, `RR = te - ts` in [16]): from
//!   `t0` to the start `t1` of the first run of 5 consecutive in-band minutes
//!   after the first out-of-band minute; `None` if the run never gets back.
//!   A run that never leaves the band recovers in 0 minutes.
//! - **Lost parcels** (Bruneau's resilience loss [15], `R = ∫ (100 - Q(t)) dt`
//!   from the event `t0` to full recovery `t1`, in the throughput form
//!   `PL = ∫ (PB(t) - P(t)) dt` of [16]): the signed sum `Σ (PB(t) - P(t))`
//!   over `t0..t1` (to the end of the run if it never recovers; over the
//!   fault window `t0..te` if it never left the band, so small in-band losses
//!   still count). Parcels the twin delivered that the faulted run had not
//!   yet; an integer because both series are counts. Catching up after `t1`
//!   is recovery, not a refund: it is not subtracted.
//! - **Maximum dip** (disruption tolerance, `DT = max (PB(t) - P(t))` [16]),
//!   relative: `max over t >= t0 of 100 * (PB(t) - P(t)) / PB(t)`, at least 0.
//! - **Graceful-degradation ratio** (research §3, Proposal):
//!   `GDR = (Q_fault / Q_base) / (C_fault / C_base)` over the fault window,
//!   with `Q` the deliveries and `C` the surviving capacity (live robots,
//!   open chutes, stations up, open floor: the scarcest of them, averaged
//!   over the window). `GDR >= 1`: the floor loses less than its capacity
//!   share; `GDR < 1`: congestion amplifies the fault (the nonlinearity of
//!   blocking in robot fleets, [26]).
//! - **Baseline utilisation** `ρ`: the twin's mean share of robots with work.
//!   Kingman's `ρ / (1 - ρ)` term [18] means the same fault hurts far more at
//!   high utilisation, so every score is reported next to it.
//!
//! Source numbers refer to `docs/research/2026-10-19-reliability-methods.md`
//! §7.

/// Scoring parameters (ARCHITECTURE §7: 5 % for 5 consecutive minutes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Width of the recovery band below the twin, in percent.
    pub band_pct: f64,
    /// Consecutive in-band minutes that count as recovered.
    pub hold_minutes: usize,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            band_pct: 5.0,
            hold_minutes: 5,
        }
    }
}

/// The resilience of one faulted run against its twin (see the module docs
/// for the formulas).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResilienceScore {
    /// Parcels lost: the Bruneau area between the twin and the faulted run.
    pub lost_parcels: i64,
    /// Deepest per-minute dip below the twin, in percent of the twin.
    pub max_dip_pct: f64,
    /// Minutes from the first fault to sustained recovery; `None` if the run
    /// never recovered.
    pub time_to_recover_min: Option<u32>,
    /// Throughput retained over capacity retained during the faults; `None`
    /// when the twin delivered nothing in the window.
    pub degradation_ratio: Option<f64>,
    /// The twin's mean robot utilisation, in `[0, 1]`.
    pub baseline_utilisation: f64,
}

/// What the score needs besides the two series.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    /// Minute the first fault starts (`t0`).
    pub onset: usize,
    /// Minute after the last fault ends (`te`, exclusive).
    pub end: usize,
    /// Mean share of capacity that survived during the window, in `(0, 1]`.
    pub capacity_retained: f64,
}

/// Score `actual` (`P`) against `benchmark` (`PB`), both per-minute
/// deliveries of equal length.
///
/// # Panics
/// If the series differ in length.
#[must_use]
pub fn score(
    benchmark: &[u32],
    actual: &[u32],
    window: Window,
    baseline_utilisation: f64,
    params: Params,
) -> ResilienceScore {
    assert_eq!(benchmark.len(), actual.len(), "series of equal length");
    let n = benchmark.len();
    let t0 = window.onset.min(n);
    let te = window.end.clamp(t0, n);
    let keep = 1.0 - params.band_pct / 100.0;
    let in_band = |t: usize| f64::from(actual[t]) >= keep * f64::from(benchmark[t]);
    let hold = params.hold_minutes.max(1);

    // `Some(t0)` if the run never left the band.
    let recovered = match (t0..n).find(|&t| !in_band(t)) {
        None => Some(t0),
        Some(out) => (out..n.saturating_sub(hold - 1)).find(|&t| (t..t + hold).all(in_band)),
    };
    let area_end = match recovered {
        None => n,
        Some(t1) if t1 == t0 => te,
        Some(t1) => t1,
    };
    let lost_parcels = (t0..area_end)
        .map(|t| i64::from(benchmark[t]) - i64::from(actual[t]))
        .sum();
    let max_dip_pct = (t0..n)
        .filter(|&t| benchmark[t] > 0)
        .map(|t| 100.0 * (f64::from(benchmark[t]) - f64::from(actual[t])) / f64::from(benchmark[t]))
        .fold(0.0, f64::max);

    let q_base: u64 = benchmark[t0..te].iter().map(|&x| u64::from(x)).sum();
    let q_fault: u64 = actual[t0..te].iter().map(|&x| u64::from(x)).sum();
    let degradation_ratio = (q_base > 0 && window.capacity_retained > 0.0)
        .then(|| (q_fault as f64 / q_base as f64) / window.capacity_retained);

    ResilienceScore {
        lost_parcels,
        max_dip_pct,
        time_to_recover_min: recovered.map(|t1| u32::try_from(t1 - t0).unwrap_or(u32::MAX)),
        degradation_ratio,
        baseline_utilisation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(onset: usize, end: usize, capacity_retained: f64) -> Window {
        Window {
            onset,
            end,
            capacity_retained,
        }
    }

    #[test]
    fn a_dip_and_a_recovery_scored_by_hand() {
        // Twin: a flat 100 per minute. Fault in minutes 2..5 (te = 5).
        let base = [100; 16];
        //            0    1   2   3   4   5    6    7    8    9   10 ...
        let fault = [
            100, 100, 60, 50, 80, 90, 110, 100, 96, 100, 99, 100, 100, 100, 100, 100,
        ];
        let s = score(&base, &fault, window(2, 5, 0.8), 0.6, Params::default());
        // Out of band first at 2; minutes 2..=5 are below 95; from 6 on all
        // are >= 95, so t1 = 6 and the time to recover is 6 - 2 = 4.
        assert_eq!(s.time_to_recover_min, Some(4));
        // Area over t0..t1 = 2..6: 40 + 50 + 20 + 10 = 120.
        assert_eq!(s.lost_parcels, 120);
        // Deepest dip: minute 3, 50 % below the twin.
        assert!((s.max_dip_pct - 50.0).abs() < 1e-9);
        // Window 2..5: Q = (60 + 50 + 80) / 300 = 0.6333; GDR = Q / 0.8.
        let gdr = s.degradation_ratio.unwrap();
        assert!((gdr - (190.0 / 300.0) / 0.8).abs() < 1e-12, "{gdr}");
        assert!((s.baseline_utilisation - 0.6).abs() < 1e-12);
    }

    #[test]
    fn a_short_lull_inside_the_dip_is_not_a_recovery() {
        let base = [100; 14];
        // In band for 3 minutes (5..8) in the middle of the dip, then out
        // again: recovery only starts at 9.
        let fault = [
            100, 50, 50, 50, 50, 100, 100, 100, 70, 100, 100, 100, 100, 100,
        ];
        let s = score(&base, &fault, window(1, 3, 1.0), 0.5, Params::default());
        assert_eq!(s.time_to_recover_min, Some(8));
        // 50 * 4 (minutes 1..5) + 0 * 3 + 30 (minute 8) = 230.
        assert_eq!(s.lost_parcels, 230);
    }

    #[test]
    fn catching_up_after_recovery_is_not_a_refund() {
        let base = [100; 12];
        // Down 40, then 30 over the twin (catch-up), then flat.
        let fault = [100, 60, 130, 100, 100, 100, 100, 100, 100, 100, 100, 100];
        let s = score(&base, &fault, window(1, 4, 1.0), 0.5, Params::default());
        assert_eq!(s.time_to_recover_min, Some(1));
        // The area stops at t1 = 2: only minute 1, so 40.
        assert_eq!(s.lost_parcels, 40);
        // Over-delivery inside the dip does pay back: 40 - 10 + 20.
        let fault = [100, 60, 110, 80, 100, 100, 100, 100, 100, 100, 100, 100];
        let s = score(&base, &fault, window(1, 2, 1.0), 0.5, Params::default());
        assert_eq!(s.time_to_recover_min, Some(3));
        assert_eq!(s.lost_parcels, 40 - 10 + 20);
    }

    #[test]
    fn a_run_that_never_recovers_has_no_recovery_time() {
        let base = [100; 10];
        let fault = [100, 100, 80, 80, 80, 80, 80, 100, 100, 100];
        let s = score(&base, &fault, window(2, 4, 1.0), 0.5, Params::default());
        // Minutes 7..10 are back in band, but only 3 of them before the end.
        assert_eq!(s.time_to_recover_min, None);
        assert_eq!(s.lost_parcels, 5 * 20);
        assert!((s.max_dip_pct - 20.0).abs() < 1e-9);
    }

    #[test]
    fn a_fault_with_no_effect_scores_zero() {
        let base = [100, 90, 110, 100, 100, 100, 100, 100];
        let s = score(&base, &base, window(1, 3, 0.5), 0.4, Params::default());
        assert_eq!(s.time_to_recover_min, Some(0));
        assert_eq!(s.lost_parcels, 0);
        assert!(s.max_dip_pct.abs() < 1e-12);
        // Full throughput on half the capacity: GDR 2.
        assert!((s.degradation_ratio.unwrap() - 2.0).abs() < 1e-12);
        // Within 5 %: 96 against 100 is in band; the in-band shortfall over
        // the fault window 1..3 still counts.
        let near = [100, 86, 105, 96, 100, 100, 100, 100];
        let s = score(&base, &near, window(1, 3, 1.0), 0.4, Params::default());
        assert_eq!(s.time_to_recover_min, Some(0));
        assert_eq!(s.lost_parcels, 4 + 5);
    }

    #[test]
    fn empty_twin_windows_have_no_degradation_ratio() {
        let zeros = [0; 6];
        let s = score(&zeros, &zeros, window(1, 3, 1.0), 0.0, Params::default());
        assert_eq!(s.degradation_ratio, None);
        assert_eq!(s.time_to_recover_min, Some(0));
    }
}
