//! Twin runs: a faulted run and its fault-free twin on the same seed
//! (ARCHITECTURE §7, step 2; research §1(b): "the control group is the
//! same seed with no fault, a noise-free twin").
//!
//! The runs are independent, so callers with threads can run them in
//! parallel: build both scenarios with [`twin_configs`] and [`run`] each
//! (the CLI does this with rayon). [`run_twin`] is the sequential
//! convenience.

use switchyard_core::Tick;
use switchyard_core::config::{ConfigError, ScenarioConfig};
use switchyard_core::faults::{FaultKind, FaultSpec};
use switchyard_core::grid::Cell;
use switchyard_core::sim::{Sim, Stats};

use crate::metrics::{MetricsRecorder, MinuteSeries, TICKS_PER_MINUTE};
use crate::resilience::{self, Params, ResilienceScore, Window};

/// The floor's capacity, for the graceful-degradation ratio.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capacity {
    /// Robots in the fleet.
    pub robots: usize,
    /// Chutes on the floor.
    pub chutes: usize,
    /// Induction stations.
    pub stations: usize,
    /// Cells of open floor.
    pub free_cells: usize,
}

/// The outcome of one run.
#[derive(Clone, Debug, PartialEq)]
pub struct RunSummary {
    /// The scenario that ran (its faults included).
    pub config: ScenarioConfig,
    /// Per-minute metrics.
    pub series: MinuteSeries,
    /// Final counters.
    pub stats: Stats,
    /// Final state hash.
    pub hash: u64,
    /// The floor's capacity.
    pub capacity: Capacity,
}

/// Run `cfg` for `ticks` ticks and summarise it.
///
/// # Errors
/// Any [`ConfigError`] from building the sim (an invalid fault included).
pub fn run(cfg: &ScenarioConfig, ticks: Tick) -> Result<RunSummary, ConfigError> {
    let mut sim = Sim::new(cfg)?;
    let mut recorder = MetricsRecorder::new(sim.world().robots.len());
    let mut events = Vec::new();
    for _ in 0..ticks {
        events.clear();
        sim.step(&mut events);
        recorder.record_tick(&events);
    }
    let layout = sim.layout();
    let capacity = Capacity {
        robots: sim.world().robots.len(),
        chutes: layout.chutes.len(),
        stations: layout.stations.len(),
        free_cells: layout
            .grid
            .cells()
            .iter()
            .filter(|&&c| c == Cell::Free)
            .count(),
    };
    Ok(RunSummary {
        config: cfg.clone(),
        series: recorder.finish(&sim),
        stats: sim.stats(),
        hash: sim.state_hash(),
        capacity,
    })
}

/// The twin pair for `cfg` plus `faults`: the baseline has no faults at all
/// (the scenario's own schedule removed too); the faulted run has the
/// scenario's faults followed by `faults`. Same seed for both.
#[must_use]
pub fn twin_configs(
    cfg: &ScenarioConfig,
    faults: &[FaultSpec],
) -> (ScenarioConfig, ScenarioConfig) {
    let mut baseline = cfg.clone();
    baseline.faults.clear();
    let mut faulted = cfg.clone();
    faulted.faults.extend_from_slice(faults);
    (baseline, faulted)
}

/// Run the twin pair of [`twin_configs`] for `ticks` ticks, one after the
/// other: `(baseline, faulted)`.
///
/// # Errors
/// Any [`ConfigError`] from building either sim.
pub fn run_twin(
    cfg: &ScenarioConfig,
    faults: &[FaultSpec],
    ticks: Tick,
) -> Result<(RunSummary, RunSummary), ConfigError> {
    let (baseline, faulted) = twin_configs(cfg, faults);
    Ok((run(&baseline, ticks)?, run(&faulted, ticks)?))
}

/// Score the faulted run of a twin pair (see [`crate::resilience`]). The
/// fault window is the faulted scenario's schedule.
#[must_use]
pub fn resilience(baseline: &RunSummary, faulted: &RunSummary) -> ResilienceScore {
    let faults = &faulted.config.faults;
    let minutes = baseline.series.minutes();
    let start = faults.iter().map(|f| f.at).min().unwrap_or(0);
    let end = faults.iter().map(FaultSpec::end).max().unwrap_or(0);
    let window = Window {
        onset: (start / TICKS_PER_MINUTE) as usize,
        end: end.div_ceil(TICKS_PER_MINUTE) as usize,
        capacity_retained: capacity_retained(faulted.capacity, faults, start, end),
    };
    resilience::score(
        &baseline.series.delivered,
        &faulted.series.delivered_regular[..minutes.min(faulted.series.minutes())],
        window,
        baseline.series.mean_utilisation(),
        Params::default(),
    )
}

/// Mean over ticks `start..end` of the scarcest surviving share of the
/// floor's resources: live robots, open chutes, stations up and open floor.
/// Surges and scanner drift take no capacity away.
#[must_use]
pub fn capacity_retained(capacity: Capacity, faults: &[FaultSpec], start: Tick, end: Tick) -> f64 {
    if end <= start {
        return 1.0;
    }
    let share = |lost: usize, total: usize| {
        if total == 0 {
            1.0
        } else {
            1.0 - lost.min(total) as f64 / total as f64
        }
    };
    let mut sum = 0.0;
    for t in start..end {
        let (mut robots, mut chutes, mut stations) = (Vec::new(), Vec::new(), Vec::new());
        let mut cells = Vec::new();
        for f in faults.iter().filter(|f| f.is_active_at(t)) {
            match &f.kind {
                FaultKind::RobotFailure { robot } => robots.push(robot.0),
                FaultKind::ChuteJam { chute } => chutes.push(u32::from(chute.0)),
                FaultKind::InductionOutage { station } => stations.push(u32::from(station.0)),
                FaultKind::AisleBlock { cells: c } => cells.extend(c.iter().copied()),
                FaultKind::VolumeSurge { .. } | FaultKind::ScannerDrift { .. } => {}
            }
        }
        for v in [&mut robots, &mut chutes, &mut stations] {
            v.sort_unstable();
            v.dedup();
        }
        cells.sort_unstable();
        cells.dedup();
        sum += [
            share(robots.len(), capacity.robots),
            share(chutes.len(), capacity.chutes),
            share(stations.len(), capacity.stations),
            share(cells.len(), capacity.free_cells),
        ]
        .into_iter()
        .fold(1.0, f64::min);
    }
    sum / f64::from(end - start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use switchyard_core::ids::{ChuteId, RobotId, StationId};
    use switchyard_core::presets::SMALL_TOML;

    fn fault(at: Tick, duration: Tick, kind: FaultKind) -> FaultSpec {
        FaultSpec { at, duration, kind }
    }

    #[test]
    fn capacity_is_the_scarcest_resource_averaged_over_the_window() {
        let cap = Capacity {
            robots: 100,
            chutes: 10,
            stations: 4,
            free_cells: 1_000,
        };
        let faults = [
            // Two robots down for the whole window: 0.98.
            fault(0, 100, FaultKind::RobotFailure { robot: RobotId(1) }),
            fault(0, 100, FaultKind::RobotFailure { robot: RobotId(2) }),
            // One chute of ten jammed for the second half: 0.9 there.
            fault(50, 50, FaultKind::ChuteJam { chute: ChuteId(3) }),
        ];
        let c = capacity_retained(cap, &faults, 0, 100);
        assert!((c - (0.98 + 0.9) / 2.0).abs() < 1e-12, "{c}");
        let outage = [fault(
            0,
            10,
            FaultKind::InductionOutage {
                station: StationId(0),
            },
        )];
        assert!((capacity_retained(cap, &outage, 0, 10) - 0.75).abs() < 1e-12);
        assert!((capacity_retained(cap, &[], 5, 5) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn twins_share_the_seed_and_only_the_faulted_run_has_faults() {
        let mut cfg = ScenarioConfig::from_toml_str(SMALL_TOML).unwrap();
        cfg.faults
            .push(fault(10, 10, FaultKind::ChuteJam { chute: ChuteId(0) }));
        let extra = [fault(
            600,
            1_200,
            FaultKind::RobotFailure { robot: RobotId(3) },
        )];
        let (base, faulted) = twin_configs(&cfg, &extra);
        assert_eq!(base.seed, faulted.seed);
        assert!(base.faults.is_empty());
        assert_eq!(faulted.faults.len(), 2);

        let (b, f) = run_twin(&cfg, &extra, 3_600).unwrap();
        assert_eq!(b.series.minutes(), 30);
        // The same regular arrivals in both runs (common random numbers).
        assert_eq!(b.series.arrived, f.series.arrived);
        let score = resilience(&b, &f);
        assert!(score.baseline_utilisation > 0.0);
        assert!(score.lost_parcels >= -50, "{score:?}");
    }
}
