//! Independent ground truth for the planted-hard suite: run the simulator
//! directly instead of going through [`switchyard_analysis::postmortem::run`].
//!
//! Each history is measured against its own dithered twin, so the dither's
//! effect cancels and only the two faults under test remain.

use rayon::prelude::*;
use switchyard_analysis::metrics::{ParcelRecord, parcels_of};
use switchyard_analysis::resilience::parcel_delay;
use switchyard_core::Tick;
use switchyard_core::config::ScenarioConfig;
use switchyard_core::faults::{FaultKind, FaultSpec};
use switchyard_core::ids::{FaultId, RobotId};
use switchyard_core::sim::Sim;

/// Per-history losses in parcel-ticks, all relative to the background run
/// (the scenario with only the faults other than `a` and `b`).
pub struct Truth {
    /// Loss of background plus `a` over background, one per history.
    pub alone_a: Vec<i64>,
    /// Loss of background plus `b` over background, one per history.
    pub alone_b: Vec<i64>,
    /// Loss of background plus `a` and `b` over background, one per history.
    pub joint: Vec<i64>,
}

impl Truth {
    /// `joint - alone_a - alone_b` on medians: positive means compound,
    /// negative means redundant.
    pub fn interaction(&self) -> i64 {
        median(&self.joint) - median(&self.alone_a) - median(&self.alone_b)
    }

    /// Half the spread of the per-history interaction values: the noise band
    /// that a measured interaction must clear to be meaningful.
    pub fn interaction_band(&self) -> f64 {
        let xs: Vec<i64> = self
            .joint
            .iter()
            .zip(&self.alone_a)
            .zip(&self.alone_b)
            .map(|((&joint, &alone_a), &alone_b)| joint - alone_a - alone_b)
            .collect();
        half_range(&xs)
    }
}

/// Median of `xs` (mean of the two middle values, rounded down, for an even
/// count; 0 when empty).
pub fn median(xs: &[i64]) -> i64 {
    let mut v = xs.to_vec();
    v.sort_unstable();
    match v.len() {
        0 => 0,
        n if n % 2 == 1 => v[n / 2],
        n => (v[n / 2 - 1] + v[n / 2]).div_euclid(2),
    }
}

/// Half the range of `xs` (0 when empty), for a symmetric noise band.
pub fn half_range(xs: &[i64]) -> f64 {
    let (Some(&lo), Some(&hi)) = (xs.iter().min(), xs.iter().max()) else {
        return 0.0;
    };
    (hi - lo) as f64 / 2.0
}

/// Measure `a` and `b` of `cfg` (`FaultId(k)` is the k-th entry of
/// `cfg.faults`) over `ticks` ticks on `histories` histories.
///
/// History 0 has no dither; the others get one 1-tick robot failure at a per-history tick before minute 20.
pub fn measure(
    cfg: &ScenarioConfig,
    a: FaultId,
    b: FaultId,
    ticks: Tick,
    histories: usize,
) -> Truth {
    let ia = a.index();
    let ib = b.index();
    assert!(
        ia < cfg.faults.len() && ib < cfg.faults.len(),
        "fault id out of range"
    );
    let rows: Vec<(i64, i64, i64)> = (0..histories)
        .into_par_iter()
        .map(|h| measure_history(cfg, ia, ib, ticks, h, histories))
        .collect();
    let mut alone_a = Vec::with_capacity(rows.len());
    let mut alone_b = Vec::with_capacity(rows.len());
    let mut joint = Vec::with_capacity(rows.len());
    for (a, b, ab) in rows {
        alone_a.push(a);
        alone_b.push(b);
        joint.push(ab);
    }
    Truth {
        alone_a,
        alone_b,
        joint,
    }
}

/// One history's three relative losses, in the order `(a, b, ab)`.
fn measure_history(
    cfg: &ScenarioConfig,
    ia: usize,
    ib: usize,
    ticks: Tick,
    h: usize,
    histories: usize,
) -> (i64, i64, i64) {
    let dither = (h > 0).then(|| {
        let robots = usize::try_from(cfg.fleet.robots).unwrap();
        let robot = u32::try_from(h * robots / histories).unwrap() % cfg.fleet.robots;
        FaultSpec {
            // Before any planted fault (minute 20) and at a different tick per
            // history, so the pause lands on a moving robot and really perturbs
            // the run (a pause at one fixed tick was often absorbed: band 0).
            at: 10 * 120 + Tick::try_from(h).unwrap() * 53,
            duration: 1,
            kind: FaultKind::RobotFailure {
                robot: RobotId(robot),
            },
        }
    });

    // The twin carries only the dither, so every loss is relative to it.
    let twin_cfg = config_with(cfg, dither.iter().cloned().collect());
    // Keep the original relative order; the dither fault always goes last.
    let bg_cfg = config_with(cfg, faults_without(cfg, &[ia, ib], dither.as_ref()));
    let a_cfg = config_with(cfg, faults_without(cfg, &[ib], dither.as_ref()));
    let b_cfg = config_with(cfg, faults_without(cfg, &[ia], dither.as_ref()));
    let ab_cfg = config_with(cfg, faults_without(cfg, &[], dither.as_ref()));

    let twin = run(&twin_cfg, ticks);
    let loss_bg = loss(&twin, &run(&bg_cfg, ticks), ticks);
    let loss_a = loss(&twin, &run(&a_cfg, ticks), ticks);
    let loss_b = loss(&twin, &run(&b_cfg, ticks), ticks);
    let loss_ab = loss(&twin, &run(&ab_cfg, ticks), ticks);

    (loss_a - loss_bg, loss_b - loss_bg, loss_ab - loss_bg)
}

/// Clone `cfg` and replace its fault schedule.
fn config_with(cfg: &ScenarioConfig, faults: Vec<FaultSpec>) -> ScenarioConfig {
    let mut out = cfg.clone();
    out.faults = faults;
    out
}

/// `cfg`'s faults except `skip`, in their original order, then `dither`.
fn faults_without(
    cfg: &ScenarioConfig,
    skip: &[usize],
    dither: Option<&FaultSpec>,
) -> Vec<FaultSpec> {
    let mut faults: Vec<FaultSpec> = cfg
        .faults
        .iter()
        .enumerate()
        .filter(|(k, _)| !skip.contains(k))
        .map(|(_, f)| f.clone())
        .collect();
    if let Some(dither) = dither {
        faults.push(dither.clone());
    }
    faults
}

/// Run `cfg` to `ticks` and read its parcels from the final state.
fn run(cfg: &ScenarioConfig, ticks: Tick) -> Vec<ParcelRecord> {
    let mut sim = Sim::new(cfg).unwrap();
    let mut events = Vec::new();
    while sim.tick() < ticks {
        events.clear();
        sim.step(&mut events);
    }
    parcels_of(&sim)
}

/// Extra latency of `run`'s regular parcels over the twin's.
fn loss(twin: &[ParcelRecord], run: &[ParcelRecord], ticks: Tick) -> i64 {
    parcel_delay(twin, run, ticks, 0).net_parcel_ticks
}

#[cfg(test)]
mod tests {
    use super::{half_range, median};

    #[test]
    fn median_handles_empty_odd_and_even() {
        assert_eq!(median(&[]), 0);
        assert_eq!(median(&[3]), 3);
        assert_eq!(median(&[4, 1, 3, 2]), 2);
        // Rounded down, not toward zero.
        assert_eq!(median(&[-3, -2]), -3);
    }

    #[test]
    fn half_range_is_half_the_span() {
        assert!((half_range(&[1, 5]) - 2.0).abs() < f64::EPSILON);
        assert!(half_range(&[]).abs() < f64::EPSILON);
        assert!(half_range(&[7, 7]).abs() < f64::EPSILON);
    }
}
