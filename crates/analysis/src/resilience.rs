//! Resilience scoring of a faulted run against its fault-free twin
//! (ARCHITECTURE §7, step 3; reliability research §1(b) and §3).
//!
//! The twin is the same scenario and seed without the faults. Because every
//! random draw is keyed per station and per parcel (ADR 0002 §3), the twin
//! sees exactly the same regular parcels, so the comparison is noise-free
//! common random numbers: any difference is caused by the faults
//! (research §3, sources [20][21]). A regular parcel is identified across
//! the two runs by `(station, key)` ([`ParcelRecord`]); a volume surge's
//! extra parcels exist only in the faulted run.
//!
//! Notation: `T` is the run length in ticks (both runs), `a(p)` the arrival
//! tick of parcel `p`, `d(p)` its drop tick in the faulted run and `d'(p)`
//! in the twin, both censored at `T` (a parcel still on the floor at the end
//! counts as dropped at `T`). `t0` is the minute the first fault starts and
//! `te` the minute after the last one ends. `R` is the set of regular
//! parcels.
//!
//! # Parcel-level scores (what the fault cost)
//! - **Delay, parcel-minutes** (the headline loss): for every regular
//!   parcel, its extra latency `x(p) = d(p) - d'(p)` (arrivals are equal),
//!   summed: `D = Σ_{p∈R} x(p) / 120`. Equivalently `D = Σ_{t<T} (N'(t) -
//!   N(t)) / 120`, the area between the twin's and the faulted run's
//!   cumulative regular deliveries `N'`, `N`: the throughput form of
//!   Bruneau's resilience loss `∫ (PB(t) - P(t)) dt` [15][16] taken over the
//!   whole run instead of up to a recovery time `t1`. That makes it
//!   independent of `t1` (once every delayed parcel is delivered, running
//!   longer adds 0), additive over parcels, and exact in integer ticks.
//!   Parcels can occasionally be *faster* in the faulted run (the fault
//!   reshuffles robot assignments), so `D` is reported as a net value next
//!   to its parts: gross `Σ max(x, 0)` and faster `Σ max(-x, 0)`,
//!   `net = gross - faster`. CP3's counterfactual ranking uses the net
//!   value: `ρ(c) = (D_actual - D_do(c)) / D_actual` (research §1(c) step 3,
//!   with `D` in place of the truncated area `PL`).
//! - **Late parcels**: regular parcels with `x(p) > sla_extra`
//!   (default 10 minutes). With an absolute cutoff `L`, also the parcels of
//!   each run (surge parcels included) whose latency `d(p) - a(p)` exceeds
//!   `L`.
//! - **Peak backlog**: the largest end-of-minute count of parcels arrived
//!   but not delivered (surge parcels included), next to the twin's.
//! - **Undelivered at end**: regular parcels the twin delivered by `T` that
//!   the faulted run did not, plus surge parcels still on the floor at `T`.
//!   This is the only "lost" figure: the parcels the fault left unsorted
//!   when the run ended.
//! - **Served / offered**: deliveries over arrivals of *all* parcels (surge
//!   parcels included) in minutes `t0..te`, next to the twin's. This is the
//!   throughput question for a surge; the delay above deliberately counts
//!   regular parcels only, which is the right comparison for *delay* but
//!   would hide a surge's extra work.
//!
//! # Throughput-curve scores
//! On `PB(t)`, the twin's regular deliveries in minute `t`, and `P(t)`, the
//! faulted run's:
//! - **In band:** minute `t` is within 5 % of the twin when
//!   `P(t) >= 0.95 * PB(t)` (one-sided: catching up above the twin is fine).
//! - **Time to recover** (recovery rapidity `RR` [16]): from `t0` to the
//!   start `t1` of the first run of 5 consecutive in-band minutes after the
//!   first out-of-band minute; `None` if the run never gets back. A run that
//!   never leaves the band recovers in 0 minutes.
//! - **Maximum dip** (disruption tolerance `DT` [16]), relative:
//!   `max over t >= t0 of 100 * (PB(t) - P(t)) / PB(t)`, at least 0.
//! - **Throughput vs capacity** (the graceful-degradation ratio of research
//!   §3): `(Q_fault / Q_base) / C` over minutes `t0..te`, with `Q` the
//!   regular deliveries and `C` the share of capacity the faults left (see
//!   `twin::capacity_retained`). Above 1: the floor kept more throughput
//!   than capacity; below 1: congestion amplified the fault (blocking in
//!   robot fleets is nonlinear [26]). Undefined (`None`) when the faults
//!   removed no capacity (a surge, scanner drift, an aisle block that cuts
//!   nothing off): there the ratio would only restate the delay.
//! - **Baseline utilisation** `ρ`: the twin's mean share of robots with work.
//!   Kingman's `ρ / (1 - ρ)` term [18] means the same fault hurts far more at
//!   high utilisation, so every score is reported next to it.
//!
//! Source numbers refer to `docs/research/2026-10-19-reliability-methods.md`
//! §7.

use std::collections::BTreeMap;

use switchyard_core::Tick;

use crate::metrics::{MinuteSeries, ParcelRecord, TICKS_PER_MINUTE};

/// Scoring parameters (ARCHITECTURE §7: 5 % for 5 consecutive minutes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Width of the recovery band below the twin, in percent.
    pub band_pct: f64,
    /// Consecutive in-band minutes that count as recovered.
    pub hold_minutes: usize,
    /// A regular parcel is late when the fault added more than this many
    /// minutes to its latency.
    pub sla_extra_minutes: u32,
    /// Optional absolute latency cutoff in minutes (e.g. the time to the
    /// truck): parcels slower than this are counted in each run.
    pub cutoff_minutes: Option<u32>,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            band_pct: 5.0,
            hold_minutes: 5,
            sla_extra_minutes: 10,
            cutoff_minutes: None,
        }
    }
}

/// One run's data for scoring.
#[derive(Clone, Copy, Debug)]
pub struct Run<'a> {
    /// Per-minute series.
    pub series: &'a MinuteSeries,
    /// Every parcel that arrived.
    pub parcels: &'a [ParcelRecord],
    /// Run length in ticks (`T`).
    pub ticks: Tick,
}

/// The fault window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    /// Minute the first fault starts (`t0`).
    pub onset: usize,
    /// Minute after the last fault ends (`te`, exclusive).
    pub end: usize,
    /// Mean share of capacity that survived during the window, in `[0, 1]`.
    pub capacity_retained: f64,
    /// Most stations out of action at once during the window.
    pub stations_lost: u32,
    /// Most chutes out of action at once during the window.
    pub chutes_lost: u32,
}

/// Extra latency of the regular parcels (see the module docs). Sums are in
/// parcel-ticks so they stay exact and additive; the `*_minutes` methods
/// convert.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParcelDelay {
    /// `Σ x(p)` over regular parcels.
    pub net_parcel_ticks: i64,
    /// `Σ max(x(p), 0)`.
    pub gross_parcel_ticks: u64,
    /// `Σ max(-x(p), 0)`: time saved by parcels that were faster with the
    /// fault.
    pub faster_parcel_ticks: u64,
    /// Regular parcels with `x(p) > 0`.
    pub delayed_parcels: u32,
    /// Regular parcels with `x(p) < 0`.
    pub faster_parcels: u32,
    /// Regular parcels with `x(p) > sla_extra`.
    pub late_parcels: u32,
    /// Regular parcels of the faulted run with no twin (0 when common random
    /// numbers hold; a sanity check).
    pub unmatched: u32,
}

/// Ticks (or parcel-ticks) as minutes (parcel-minutes).
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub const fn ticks_to_minutes(t: f64) -> f64 {
    t / TICKS_PER_MINUTE as f64
}

impl ParcelDelay {
    /// Net delay in parcel-minutes (the headline; `ρ` in CP3 uses this).
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub const fn net_minutes(&self) -> f64 {
        ticks_to_minutes(self.net_parcel_ticks as f64)
    }

    /// Gross delay in parcel-minutes.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub const fn gross_minutes(&self) -> f64 {
        ticks_to_minutes(self.gross_parcel_ticks as f64)
    }

    /// Parcel-minutes saved by parcels that were faster with the fault.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub const fn faster_minutes(&self) -> f64 {
        ticks_to_minutes(self.faster_parcel_ticks as f64)
    }
}

/// Parcels past an absolute latency cutoff, in each run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CutoffCount {
    /// The cutoff, in minutes.
    pub minutes: u32,
    /// Parcels of the faulted run slower than the cutoff.
    pub faulted: u32,
    /// Parcels of the twin slower than the cutoff.
    pub twin: u32,
}

/// The resilience of one faulted run against its twin (see the module docs
/// for the formulas).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResilienceScore {
    /// Extra latency of the regular parcels, late parcels included.
    pub delay: ParcelDelay,
    /// Parcels past the absolute cutoff, if one is configured.
    pub past_cutoff: Option<CutoffCount>,
    /// Largest end-of-minute backlog of the faulted run, in parcels.
    pub peak_backlog: u32,
    /// The twin's largest end-of-minute backlog.
    pub twin_peak_backlog: u32,
    /// Parcels the fault left undelivered at the end of the run.
    pub undelivered: u32,
    /// Deepest per-minute dip below the twin, in percent of the twin.
    pub max_dip_pct: f64,
    /// Minutes from the first fault to sustained recovery; `None` if the run
    /// never recovered.
    pub time_to_recover_min: Option<u32>,
    /// Deliveries over arrivals (all parcels) during the fault window, in
    /// the faulted run; `None` if nothing arrived.
    pub served_offered: Option<f64>,
    /// The same for the twin.
    pub twin_served_offered: Option<f64>,
    /// Throughput retained over capacity retained during the faults;
    /// `None` when the faults removed no capacity or the twin delivered
    /// nothing in the window.
    pub throughput_vs_capacity: Option<f64>,
    /// Mean share of capacity the faults left, in `[0, 1]`.
    pub capacity_retained: f64,
    /// Most stations out of action at once (an outage, a failed robot on
    /// the loading cell, or cut off from every chute).
    pub stations_lost: u32,
    /// Most chutes out of action at once (jammed, or every access cell
    /// closed or held by a failed robot, or cut off from every station).
    pub chutes_lost: u32,
    /// The twin's mean robot utilisation, in `[0, 1]`.
    pub baseline_utilisation: f64,
}

/// The twin's regular parcels by cross-run identity `(station, key)`.
/// Build it once when many runs are scored against the same twin (a
/// post-mortem scores hundreds of replays).
#[derive(Clone, Debug, Default)]
pub struct TwinIndex<'a>(BTreeMap<(u16, u64), &'a ParcelRecord>);

impl<'a> TwinIndex<'a> {
    /// Index the regular parcels of `twin`.
    #[must_use]
    pub fn new(twin: &'a [ParcelRecord]) -> Self {
        Self(
            twin.iter()
                .filter(|p| !p.is_surge())
                .map(|p| ((p.station.0, p.key), p))
                .collect(),
        )
    }
}

/// Extra latency of the faulted run's regular parcels over their twins,
/// both censored at `horizon` (see the module docs).
#[must_use]
pub fn parcel_delay(
    twin: &[ParcelRecord],
    faulted: &[ParcelRecord],
    horizon: Tick,
    sla_extra_minutes: u32,
) -> ParcelDelay {
    parcel_delay_indexed(&TwinIndex::new(twin), faulted, horizon, sla_extra_minutes)
}

/// [`parcel_delay`] against a prebuilt [`TwinIndex`].
#[must_use]
pub fn parcel_delay_indexed(
    twin: &TwinIndex<'_>,
    faulted: &[ParcelRecord],
    horizon: Tick,
    sla_extra_minutes: u32,
) -> ParcelDelay {
    let by_id = &twin.0;
    let sla = i64::from(sla_extra_minutes) * i64::from(TICKS_PER_MINUTE);
    let mut out = ParcelDelay::default();
    for p in faulted.iter().filter(|p| !p.is_surge()) {
        let Some(t) = by_id.get(&(p.station.0, p.key)) else {
            out.unmatched += 1;
            continue;
        };
        let x = i64::from(p.latency_censored(horizon)) - i64::from(t.latency_censored(horizon));
        out.net_parcel_ticks += x;
        if x > 0 {
            out.gross_parcel_ticks += x.unsigned_abs();
            out.delayed_parcels += 1;
        } else if x < 0 {
            out.faster_parcel_ticks += x.unsigned_abs();
            out.faster_parcels += 1;
        }
        if x > sla {
            out.late_parcels += 1;
        }
    }
    out
}

/// Parcels the fault left undelivered at `horizon`: regular parcels the
/// twin delivered and the faulted run did not, plus surge parcels still on
/// the floor.
#[must_use]
pub fn undelivered(twin: &[ParcelRecord], faulted: &[ParcelRecord], horizon: Tick) -> u32 {
    let twin_done: BTreeMap<(u16, u64), bool> = twin
        .iter()
        .filter(|p| !p.is_surge())
        .map(|p| ((p.station.0, p.key), p.delivered_by(horizon)))
        .collect();
    let n = faulted
        .iter()
        .filter(|p| !p.delivered_by(horizon))
        .filter(|p| p.is_surge() || twin_done.get(&(p.station.0, p.key)) == Some(&true))
        .count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Parcels whose latency (censored at `horizon`) exceeds `minutes`.
#[must_use]
pub fn past_cutoff(parcels: &[ParcelRecord], horizon: Tick, minutes: u32) -> u32 {
    let limit = Tick::from(minutes).saturating_mul(TICKS_PER_MINUTE);
    let n = parcels
        .iter()
        .filter(|p| p.latency_censored(horizon) > limit)
        .count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Maximum dip in percent and time to recover in minutes, on per-minute
/// deliveries `actual` (`P`) against `benchmark` (`PB`) from minute `onset`.
///
/// # Panics
/// If the series differ in length.
#[must_use]
pub fn curve(
    benchmark: &[u32],
    actual: &[u32],
    onset: usize,
    params: Params,
) -> (f64, Option<u32>) {
    assert_eq!(benchmark.len(), actual.len(), "series of equal length");
    let n = benchmark.len();
    let t0 = onset.min(n);
    let keep = 1.0 - params.band_pct / 100.0;
    let in_band = |t: usize| f64::from(actual[t]) >= keep * f64::from(benchmark[t]);
    let hold = params.hold_minutes.max(1);
    // `Some(t0)` if the run never left the band.
    let recovered = match (t0..n).find(|&t| !in_band(t)) {
        None => Some(t0),
        Some(out) => (out..n.saturating_sub(hold - 1)).find(|&t| (t..t + hold).all(in_band)),
    };
    let max_dip_pct = (t0..n)
        .filter(|&t| benchmark[t] > 0)
        .map(|t| 100.0 * (f64::from(benchmark[t]) - f64::from(actual[t])) / f64::from(benchmark[t]))
        .fold(0.0, f64::max);
    (
        max_dip_pct,
        recovered.map(|t1| u32::try_from(t1 - t0).unwrap_or(u32::MAX)),
    )
}

/// Deliveries over arrivals in minutes `t0..te`; `None` if nothing arrived.
fn served_offered(s: &MinuteSeries, t0: usize, te: usize) -> Option<f64> {
    let te = te.min(s.minutes());
    let t0 = t0.min(te);
    let offered: u64 = s.arrived[t0..te].iter().map(|&x| u64::from(x)).sum();
    let served: u64 = s.delivered[t0..te].iter().map(|&x| u64::from(x)).sum();
    #[allow(clippy::cast_precision_loss)]
    (offered > 0).then(|| served as f64 / offered as f64)
}

/// Score `faulted` against `twin` (see the module docs).
///
/// # Panics
/// If the runs differ in length.
#[must_use]
pub fn score(
    twin: Run<'_>,
    faulted: Run<'_>,
    window: Window,
    baseline_utilisation: f64,
    params: Params,
) -> ResilienceScore {
    assert_eq!(twin.ticks, faulted.ticks, "runs of equal length");
    let horizon = twin.ticks;
    let benchmark = &twin.series.delivered_regular;
    let actual = &faulted.series.delivered_regular;
    let n = benchmark.len().min(actual.len());
    let t0 = window.onset.min(n);
    let te = window.end.clamp(t0, n);
    let (max_dip_pct, time_to_recover_min) = curve(&benchmark[..n], &actual[..n], t0, params);

    let q_base: u64 = benchmark[t0..te].iter().map(|&x| u64::from(x)).sum();
    let q_fault: u64 = actual[t0..te].iter().map(|&x| u64::from(x)).sum();
    #[allow(clippy::cast_precision_loss)]
    let throughput_vs_capacity =
        (q_base > 0 && window.capacity_retained > 0.0 && window.capacity_retained < 1.0 - 1e-9)
            .then(|| (q_fault as f64 / q_base as f64) / window.capacity_retained);

    let peak = |s: &MinuteSeries| s.backlog.iter().copied().max().unwrap_or(0);
    ResilienceScore {
        delay: parcel_delay(
            twin.parcels,
            faulted.parcels,
            horizon,
            params.sla_extra_minutes,
        ),
        past_cutoff: params.cutoff_minutes.map(|m| CutoffCount {
            minutes: m,
            faulted: past_cutoff(faulted.parcels, horizon, m),
            twin: past_cutoff(twin.parcels, horizon, m),
        }),
        peak_backlog: peak(faulted.series),
        twin_peak_backlog: peak(twin.series),
        undelivered: undelivered(twin.parcels, faulted.parcels, horizon),
        max_dip_pct,
        time_to_recover_min,
        served_offered: served_offered(faulted.series, t0, te),
        twin_served_offered: served_offered(twin.series, t0, te),
        throughput_vs_capacity,
        capacity_retained: window.capacity_retained,
        stations_lost: window.stations_lost,
        chutes_lost: window.chutes_lost,
        baseline_utilisation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use switchyard_core::faults::SURGE_KEY_FLAG;
    use switchyard_core::ids::StationId;

    const M: Tick = TICKS_PER_MINUTE;

    fn parcel(station: u16, key: u64, arrived: Tick, dropped: Option<Tick>) -> ParcelRecord {
        ParcelRecord {
            station: StationId(station),
            key,
            arrived,
            dropped,
        }
    }

    fn run<'a>(series: &'a MinuteSeries, parcels: &'a [ParcelRecord], ticks: Tick) -> Run<'a> {
        Run {
            series,
            parcels,
            ticks,
        }
    }

    fn window(onset: usize, end: usize, capacity_retained: f64) -> Window {
        Window {
            onset,
            end,
            capacity_retained,
            stations_lost: 0,
            chutes_lost: 0,
        }
    }

    // ---- Throughput curve -------------------------------------------------

    #[test]
    fn a_dip_and_a_recovery_scored_by_hand() {
        let base = [100; 16];
        //            0    1   2   3   4   5    6    7    8    9   10 ...
        let fault = [
            100, 100, 60, 50, 80, 90, 110, 100, 96, 100, 99, 100, 100, 100, 100, 100,
        ];
        let (dip, ttr) = curve(&base, &fault, 2, Params::default());
        // Out of band first at 2; minutes 2..=5 are below 95; from 6 on all
        // are >= 95, so t1 = 6 and the time to recover is 6 - 2 = 4.
        assert_eq!(ttr, Some(4));
        // Deepest dip: minute 3, 50 % below the twin.
        assert!((dip - 50.0).abs() < 1e-9);
    }

    #[test]
    fn a_short_lull_inside_the_dip_is_not_a_recovery() {
        let base = [100; 14];
        let fault = [
            100, 50, 50, 50, 50, 100, 100, 100, 70, 100, 100, 100, 100, 100,
        ];
        assert_eq!(curve(&base, &fault, 1, Params::default()).1, Some(8));
    }

    #[test]
    fn a_run_that_never_recovers_has_no_recovery_time() {
        let base = [100; 10];
        let fault = [100, 100, 80, 80, 80, 80, 80, 100, 100, 100];
        let (dip, ttr) = curve(&base, &fault, 2, Params::default());
        assert_eq!(ttr, None);
        assert!((dip - 20.0).abs() < 1e-9);
    }

    #[test]
    fn a_fault_with_no_effect_has_no_dip() {
        let base = [100, 90, 110, 100, 100, 100, 100, 100];
        assert_eq!(curve(&base, &base, 1, Params::default()), (0.0, Some(0)));
    }

    // ---- Parcel delay -----------------------------------------------------

    /// Four regular parcels at two stations; the twin delivers each 2
    /// minutes after arrival.
    fn twin_parcels() -> Vec<ParcelRecord> {
        vec![
            parcel(0, 0, 0, Some(2 * M)),
            parcel(0, 1, M, Some(3 * M)),
            parcel(1, 0, 0, Some(2 * M)),
            parcel(1, 1, 2 * M, Some(4 * M)),
        ]
    }

    #[test]
    fn a_fault_that_only_delays_costs_parcel_minutes_and_loses_nothing() {
        let twin = twin_parcels();
        // Same parcels, listed in another order (ids differ across runs),
        // delayed by 0, 3, 12 and 1.5 minutes.
        let faulted = vec![
            parcel(1, 1, 2 * M, Some(4 * M + M / 2 * 3)),
            parcel(0, 0, 0, Some(2 * M)),
            parcel(1, 0, 0, Some(14 * M)),
            parcel(0, 1, M, Some(6 * M)),
        ];
        let horizon = 30 * M;
        let d = parcel_delay(&twin, &faulted, horizon, 10);
        assert_eq!(d.net_parcel_ticks, i64::from(3 * M + 12 * M + 3 * M / 2));
        assert!((d.net_minutes() - 16.5).abs() < 1e-12);
        assert_eq!(d.gross_parcel_ticks, d.net_parcel_ticks.unsigned_abs());
        assert_eq!((d.delayed_parcels, d.faster_parcels), (3, 0));
        // Only the 12-minute delay exceeds the 10-minute SLA.
        assert_eq!(d.late_parcels, 1);
        assert_eq!(d.unmatched, 0);
        assert_eq!(undelivered(&twin, &faulted, horizon), 0);

        // The same through `score`, with series built from the parcels.
        let (st, sf) = (
            MinuteSeries::from_parcels(&twin, horizon),
            MinuteSeries::from_parcels(&faulted, horizon),
        );
        let s = score(
            run(&st, &twin, horizon),
            run(&sf, &faulted, horizon),
            window(0, 3, 1.0),
            0.5,
            Params::default(),
        );
        assert_eq!(s.delay, d);
        assert_eq!(s.undelivered, 0);
        // Backlog at the end of minute 1: 3 arrived, none delivered, in both
        // runs. The twin then drains (2, 1, 0); the faulted run holds 3
        // until minute 5 but never exceeds it.
        assert_eq!(s.twin_peak_backlog, 3);
        assert_eq!(s.peak_backlog, 3);
        assert_eq!(s.throughput_vs_capacity, None, "no capacity removed");
    }

    #[test]
    fn faster_parcels_are_reported_gross_and_net() {
        let twin = twin_parcels();
        // (0, 0) is 2 minutes slower, (0, 1) half a minute faster.
        let mut faulted = twin.clone();
        faulted[0].dropped = Some(4 * M);
        faulted[1].dropped = Some(3 * M - M / 2);
        let d = parcel_delay(&twin, &faulted, 30 * M, 10);
        assert!((d.gross_minutes() - 2.0).abs() < 1e-12);
        assert!((d.faster_minutes() - 0.5).abs() < 1e-12);
        assert!((d.net_minutes() - 1.5).abs() < 1e-12);
        assert_eq!((d.delayed_parcels, d.faster_parcels), (1, 1));
    }

    #[test]
    fn a_surge_scores_regular_delay_and_served_over_offered_separately() {
        let horizon = 10 * M;
        let twin = twin_parcels();
        let mut faulted = twin.clone();
        // The surge delays one regular parcel by 1 minute...
        faulted[3].dropped = Some(5 * M);
        // ...and adds four surge parcels in minutes 0..2, two of them
        // delivered (minutes 1 and 6), two still on the floor at the end.
        for (k, at, drop) in [
            (0, 0, Some(M + 5)),
            (1, 10, Some(6 * M)),
            (2, M, None),
            (3, M + 1, None),
        ] {
            faulted.push(parcel(0, SURGE_KEY_FLAG | k, at, drop));
        }
        let (st, sf) = (
            MinuteSeries::from_parcels(&twin, horizon),
            MinuteSeries::from_parcels(&faulted, horizon),
        );
        let s = score(
            run(&st, &twin, horizon),
            run(&sf, &faulted, horizon),
            window(0, 2, 1.0),
            0.5,
            Params::default(),
        );
        // Regular delay only: surge parcels have no twin and are not
        // counted as delayed (nor as unmatched).
        assert_eq!(s.delay.net_parcel_ticks, i64::from(M));
        assert_eq!(s.delay.unmatched, 0);
        // Minutes 0..2: twin offered 3 (arrivals at 0, 0, 1 min) and
        // delivered 0, so 0/3; faulted offered 3 + 4 = 7, delivered 1
        // (the surge parcel at M + 5).
        assert_eq!(s.twin_served_offered, Some(0.0));
        assert!((s.served_offered.unwrap() - 1.0 / 7.0).abs() < 1e-12);
        // Two surge parcels never delivered.
        assert_eq!(s.undelivered, 2);
        assert!(s.peak_backlog > s.twin_peak_backlog);
        // A surge removes no capacity: no throughput-vs-capacity ratio,
        // however the throughput moved.
        assert_eq!(s.throughput_vs_capacity, None);
    }

    #[test]
    fn delay_is_non_negative_where_the_old_area_went_negative() {
        // Fault from minute 2. The twin drops parcel (0, 0) in minute 1,
        // the faulted run half a minute later, in minute 2. All else equal.
        let horizon = 20 * M;
        let twin = vec![parcel(0, 0, 0, Some(M + 80)), parcel(0, 1, 0, Some(5 * M))];
        let mut faulted = twin.clone();
        faulted[0].dropped = Some(2 * M + 20);
        let (st, sf) = (
            MinuteSeries::from_parcels(&twin, horizon),
            MinuteSeries::from_parcels(&faulted, horizon),
        );
        // The old signed area Σ (PB - P) over the window 2..4 is -1: the
        // faulted run "delivered more" in the window.
        let old: i64 = (2..4)
            .map(|t| i64::from(st.delivered_regular[t]) - i64::from(sf.delivered_regular[t]))
            .sum();
        assert_eq!(old, -1);
        let d = parcel_delay(&twin, &faulted, horizon, 10);
        assert_eq!(d.net_parcel_ticks, 60);
        assert!((d.net_minutes() - 0.5).abs() < 1e-12);
    }

    #[test]
    fn delay_does_not_depend_on_where_the_run_is_cut() {
        // Delays settle by minute 15; cutting at 20 or at 60 minutes (with
        // more, undisturbed parcels arriving in between) gives the same D.
        let mut twin = twin_parcels();
        let mut faulted = twin.clone();
        faulted[0].dropped = Some(9 * M);
        faulted[2].dropped = Some(15 * M);
        let short = parcel_delay(&twin, &faulted, 20 * M, 10);
        for k in 2..40u32 {
            let p = parcel(0, u64::from(k), k * M, Some(k * M + 90));
            twin.push(p);
            faulted.push(p);
        }
        let long = parcel_delay(&twin, &faulted, 60 * M, 10);
        assert_eq!(short, long);
        assert_eq!(short.net_parcel_ticks, i64::from(7 * M + 13 * M));
        // Before the delays settle, the censored value is smaller (the area
        // so far), never larger.
        let early = parcel_delay(&twin, &faulted, 10 * M, 10);
        assert!(early.net_parcel_ticks < short.net_parcel_ticks);
    }

    #[test]
    fn delay_equals_the_area_between_cumulative_deliveries() {
        let horizon = 20 * M;
        let twin = twin_parcels();
        let mut faulted = twin.clone();
        faulted[1].dropped = Some(7 * M + 33);
        faulted[3].dropped = None; // still on the floor at the end
        let d = parcel_delay(&twin, &faulted, horizon, 10);
        let delivered_by =
            |ps: &[ParcelRecord], t: Tick| ps.iter().filter(|p| p.delivered_by(t + 1)).count();
        let area: i64 = (0..horizon)
            .map(|t| {
                i64::try_from(delivered_by(&twin, t)).unwrap()
                    - i64::try_from(delivered_by(&faulted, t)).unwrap()
            })
            .sum();
        assert_eq!(d.net_parcel_ticks, area);
        // The censored parcel counts as late (16 extra minutes) and as
        // undelivered.
        assert_eq!(d.late_parcels, 1);
        assert_eq!(undelivered(&twin, &faulted, horizon), 1);
    }

    #[test]
    fn the_absolute_cutoff_counts_each_run() {
        let horizon = 30 * M;
        let twin = twin_parcels();
        let mut faulted = twin.clone();
        faulted[0].dropped = Some(25 * M); // 25 minutes
        faulted[1].dropped = None; // censored: 29 minutes
        assert_eq!(past_cutoff(&twin, horizon, 20), 0);
        assert_eq!(past_cutoff(&faulted, horizon, 20), 2);
        assert_eq!(past_cutoff(&faulted, horizon, 25), 1);
        let (st, sf) = (
            MinuteSeries::from_parcels(&twin, horizon),
            MinuteSeries::from_parcels(&faulted, horizon),
        );
        let params = Params {
            cutoff_minutes: Some(20),
            ..Params::default()
        };
        let s = score(
            run(&st, &twin, horizon),
            run(&sf, &faulted, horizon),
            window(0, 2, 1.0),
            0.5,
            params,
        );
        assert_eq!(
            s.past_cutoff,
            Some(CutoffCount {
                minutes: 20,
                faulted: 2,
                twin: 0
            })
        );
    }

    #[test]
    fn throughput_vs_capacity_by_hand() {
        // 100 regular parcels per minute in the twin; the faulted run
        // delivers 60 in minutes 2..4 on 80 % capacity.
        let horizon = 6 * M;
        let mut st = MinuteSeries::from_parcels(&[], horizon);
        st.delivered_regular = vec![100; 6];
        let mut sf = st.clone();
        sf.delivered_regular = vec![100, 100, 60, 60, 100, 100];
        let s = score(
            run(&st, &[], horizon),
            run(&sf, &[], horizon),
            window(2, 4, 0.8),
            0.5,
            Params::default(),
        );
        assert!((s.throughput_vs_capacity.unwrap() - 0.6 / 0.8).abs() < 1e-12);
        assert!((s.max_dip_pct - 40.0).abs() < 1e-12);
        assert_eq!(s.time_to_recover_min, None, "only 2 in-band minutes");
    }
}
