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
use switchyard_core::events::Event;
use switchyard_core::faults::{FaultKind, FaultSpec};
use switchyard_core::grid::{Cell, CellIdx};
use switchyard_core::ids::RobotId;
use switchyard_core::layout::Layout;
use switchyard_core::sim::{Sim, Stats};
use switchyard_core::world::RobotState;

use crate::metrics::{MetricsRecorder, MinuteSeries, ParcelRecord, TICKS_PER_MINUTE};
use crate::resilience::{self, Params, ResilienceScore, Run, Window};

/// The floor's size: robots, chutes, stations and open floor.
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

/// A robot that stood failed on a cell during a run, read from the run's
/// own events: where it stopped decides what it blocks (a failed robot on
/// a loading cell takes the whole station out; on a chute's only access
/// cell, the chute).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FailedRobot {
    /// The robot.
    pub robot: RobotId,
    /// The cell it stood on while failed.
    pub cell: CellIdx,
    /// First tick failed.
    pub from: Tick,
    /// First tick working again (the run length if it never recovered).
    pub to: Tick,
}

impl FailedRobot {
    const fn is_active_at(&self, t: Tick) -> bool {
        self.from <= t && t < self.to
    }
}

/// The outcome of one run.
#[derive(Clone, Debug, PartialEq)]
pub struct RunSummary {
    /// The scenario that ran (its faults included).
    pub config: ScenarioConfig,
    /// Per-minute metrics.
    pub series: MinuteSeries,
    /// Every parcel that arrived, with its cross-run identity.
    pub parcels: Vec<ParcelRecord>,
    /// Ticks simulated.
    pub ticks: Tick,
    /// The floor (for reachability under aisle blocks).
    pub layout: Layout,
    /// Every robot failure, with the cell the robot stood on.
    pub failures: Vec<FailedRobot>,
    /// Final counters.
    pub stats: Stats,
    /// Final state hash.
    pub hash: u64,
    /// The floor's capacity.
    pub capacity: Capacity,
}

impl RunSummary {
    /// The run's data for [`resilience::score`].
    #[must_use]
    pub fn as_run(&self) -> Run<'_> {
        Run {
            series: &self.series,
            parcels: &self.parcels,
            ticks: self.ticks,
        }
    }
}

/// Run `cfg` for `ticks` ticks and summarise it.
///
/// # Errors
/// Any [`ConfigError`] from building the sim (an invalid fault included).
pub fn run(cfg: &ScenarioConfig, ticks: Tick) -> Result<RunSummary, ConfigError> {
    let mut sim = Sim::new(cfg)?;
    let mut recorder = MetricsRecorder::new(sim.world().robots.len());
    let mut events = Vec::new();
    let mut failures: Vec<FailedRobot> = Vec::new();
    for _ in 0..ticks {
        events.clear();
        sim.step(&mut events);
        recorder.record_tick(&events);
        for e in &events {
            if let Event::RobotState { tick, robot, state } = *e {
                let open = failures
                    .iter_mut()
                    .rev()
                    .find(|f| f.robot == robot && f.to == Tick::MAX);
                match (state, open) {
                    (RobotState::Failed, None) => failures.push(FailedRobot {
                        robot,
                        // A failed robot does not move: its cell after the
                        // step is the cell it failed on.
                        cell: sim.world().robots.pos[robot.index()],
                        from: tick,
                        to: Tick::MAX,
                    }),
                    (RobotState::Failed, Some(_)) | (_, None) => {}
                    (_, Some(f)) => f.to = tick,
                }
            }
        }
    }
    for f in &mut failures {
        f.to = f.to.min(ticks);
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
        parcels: recorder.parcels(&sim),
        ticks: recorder.ticks(),
        stats: sim.stats(),
        hash: sim.state_hash(),
        capacity,
        layout: layout.clone(),
        failures,
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
    baseline.disabled_faults.clear();
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
pub fn resilience(baseline: &RunSummary, faulted: &RunSummary, params: Params) -> ResilienceScore {
    // Disabled faults (counterfactual replays) never run: they are not part
    // of the window or the capacity.
    let faults: Vec<FaultSpec> = faulted
        .config
        .enabled_faults()
        .map(|(_, f)| f.clone())
        .collect();
    let faults = faults.as_slice();
    let start = faults.iter().map(|f| f.at).min().unwrap_or(0);
    let end = faults.iter().map(FaultSpec::end).max().unwrap_or(0);
    let loss = capacity_loss(
        &faulted.layout,
        faulted.capacity.robots,
        faults,
        &faulted.failures,
        start,
        end,
    );
    let window = Window {
        onset: (start / TICKS_PER_MINUTE) as usize,
        end: end.div_ceil(TICKS_PER_MINUTE) as usize,
        capacity_retained: loss.retained,
        stations_lost: loss.stations_lost,
        chutes_lost: loss.chutes_lost,
    };
    resilience::score(
        baseline.as_run(),
        faulted.as_run(),
        window,
        baseline.series.mean_utilisation(),
        params,
    )
}

/// Capacity the faults left during a window (see [`capacity_loss`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapacityLoss {
    /// Mean over the window of the scarcest surviving share, in `[0, 1]`.
    pub retained: f64,
    /// Most stations out of action at once.
    pub stations_lost: u32,
    /// Most chutes out of action at once.
    pub chutes_lost: u32,
}

/// Capacity over ticks `start..end`: the mean of the scarcest surviving
/// share of the floor's working capacity, plus the most stations and chutes
/// out of action at once. A share counts:
/// - robots not failed;
/// - chutes that are not jammed *and* that a robot can still reach from a
///   working station (some access cell is open and connected, over open
///   floor, to a working station's loading cell);
/// - stations that are up, whose loading cell is not held by a failed
///   robot, *and* that are still connected to at least one usable chute.
///
/// Closed floor is an aisle block's cells plus the cells failed robots
/// stand on (`failures`, from the run itself): the sim closes a failed
/// robot's cell, so a robot that dies on a loading cell silently takes the
/// whole station out, and one on a chute's only access cell, the chute
/// (investigation of the CP2 seed-1 outlier). An aisle block that cuts
/// nothing off costs no capacity: its closed cells are congestion, which
/// shows up in the delay instead. Surges and scanner drift take no
/// capacity away.
///
/// TODO: robots stranded in a region a block cuts off still count as
/// available.
#[must_use]
pub fn capacity_loss(
    layout: &Layout,
    robots: usize,
    faults: &[FaultSpec],
    failures: &[FailedRobot],
    start: Tick,
    end: Tick,
) -> CapacityLoss {
    let mut out = CapacityLoss {
        retained: 1.0,
        stations_lost: 0,
        chutes_lost: 0,
    };
    if end <= start {
        return out;
    }
    // The active set only changes at fault and failure boundaries: score
    // each segment once.
    let mut cuts: Vec<Tick> = faults
        .iter()
        .flat_map(|f| [f.at, f.end()])
        .chain(failures.iter().flat_map(|f| [f.from, f.to]))
        .chain([start, end])
        .filter(|&t| (start..=end).contains(&t))
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut sum = 0.0;
    for seg in cuts.windows(2) {
        let at = capacity_at(layout, robots, faults, failures, seg[0]);
        sum += f64::from(seg[1] - seg[0]) * at.retained;
        out.stations_lost = out.stations_lost.max(at.stations_lost);
        out.chutes_lost = out.chutes_lost.max(at.chutes_lost);
    }
    out.retained = sum / f64::from(end - start);
    out
}

/// [`CapacityLoss`] with the faults and failures active on tick `t`.
#[allow(clippy::cast_precision_loss)]
fn capacity_at(
    layout: &Layout,
    robots: usize,
    faults: &[FaultSpec],
    failures: &[FailedRobot],
    t: Tick,
) -> CapacityLoss {
    let g = &layout.grid;
    let mut failed = Vec::new();
    let mut jammed = vec![false; layout.chutes.len()];
    let mut down = vec![false; layout.stations.len()];
    let mut closed = vec![false; g.len()];
    for f in faults.iter().filter(|f| f.is_active_at(t)) {
        match &f.kind {
            FaultKind::RobotFailure { robot } => failed.push(robot.0),
            FaultKind::ChuteJam { chute } => {
                if let Some(j) = jammed.get_mut(usize::from(chute.0)) {
                    *j = true;
                }
            }
            FaultKind::InductionOutage { station } => {
                if let Some(d) = down.get_mut(usize::from(station.0)) {
                    *d = true;
                }
            }
            FaultKind::AisleBlock { cells } => {
                for &c in cells {
                    if c.x < g.width() && c.y < g.height() {
                        closed[g.idx(c) as usize] = true;
                    }
                }
            }
            FaultKind::VolumeSurge { .. } | FaultKind::ScannerDrift { .. } => {}
        }
    }
    for f in failures.iter().filter(|f| f.is_active_at(t)) {
        if let Some(c) = closed.get_mut(f.cell as usize) {
            *c = true;
        }
        if let Some(s) = layout.stations.iter().position(|&s| s == f.cell) {
            down[s] = true;
        }
    }
    failed.sort_unstable();
    failed.dedup();

    // Label the connected regions of open floor.
    let open = |c: CellIdx| g.is_free(c) && !closed[c as usize];
    let mut region = vec![u32::MAX; g.len()];
    let mut regions = 0u32;
    let mut stack = Vec::new();
    for c in (0..g.len()).filter_map(|c| CellIdx::try_from(c).ok()) {
        if !open(c) || region[c as usize] != u32::MAX {
            continue;
        }
        region[c as usize] = regions;
        stack.push(c);
        while let Some(x) = stack.pop() {
            for y in g.neighbours(x) {
                if open(y) && region[y as usize] == u32::MAX {
                    region[y as usize] = regions;
                    stack.push(y);
                }
            }
        }
        regions += 1;
    }
    // Regions next to each working station / chute (empty if it is down).
    let touching = |cells: &[CellIdx], off: &[bool]| -> Vec<Vec<u32>> {
        cells
            .iter()
            .zip(off)
            .map(|(&cell, &off)| {
                if off {
                    return Vec::new();
                }
                g.neighbours(cell)
                    .filter(|&n| open(n))
                    .map(|n| region[n as usize])
                    .collect()
            })
            .collect()
    };
    let stations = touching(&layout.stations, &down);
    let chutes = touching(&layout.chutes, &jammed);
    let mark = |sets: &[Vec<u32>]| {
        let mut m = vec![false; regions as usize];
        for &r in sets.iter().flatten() {
            m[r as usize] = true;
        }
        m
    };
    let (fed, drained) = (mark(&stations), mark(&chutes));
    let usable = |sets: &[Vec<u32>], other: &[bool]| {
        sets.iter()
            .filter(|rs| rs.iter().any(|&r| other[r as usize]))
            .count()
    };
    let (chutes_ok, stations_ok) = (usable(&chutes, &fed), usable(&stations, &drained));
    let share = |kept: usize, total: usize| {
        if total == 0 {
            1.0
        } else {
            kept.min(total) as f64 / total as f64
        }
    };
    let lost = |ok: usize, total: usize| u32::try_from(total - ok.min(total)).unwrap_or(u32::MAX);
    CapacityLoss {
        retained: [
            share(robots - failed.len().min(robots), robots),
            share(chutes_ok, chutes.len()),
            share(stations_ok, stations.len()),
        ]
        .into_iter()
        .fold(1.0, f64::min),
        stations_lost: lost(stations_ok, stations.len()),
        chutes_lost: lost(chutes_ok, chutes.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use switchyard_core::grid::Pos;
    use switchyard_core::ids::{ChuteId, RobotId, StationId};
    use switchyard_core::layout;
    use switchyard_core::presets::SMALL_TOML;

    fn fault(at: Tick, duration: Tick, kind: FaultKind) -> FaultSpec {
        FaultSpec { at, duration, kind }
    }

    fn small_layout() -> Layout {
        let cfg = ScenarioConfig::from_toml_str(SMALL_TOML).unwrap();
        layout::generate(&cfg.floor).unwrap()
    }

    fn block(cells: Vec<Pos>) -> FaultSpec {
        fault(0, 10, FaultKind::AisleBlock { cells })
    }

    #[test]
    fn capacity_is_the_scarcest_resource_averaged_over_the_window() {
        let l = small_layout();
        let chutes = l.chutes.len() as f64;
        let faults = [
            // Two robots of 100 down for the whole window: 0.98.
            fault(0, 100, FaultKind::RobotFailure { robot: RobotId(1) }),
            fault(0, 100, FaultKind::RobotFailure { robot: RobotId(2) }),
            // One chute jammed for the second half.
            fault(50, 50, FaultKind::ChuteJam { chute: ChuteId(3) }),
        ];
        let c = capacity_loss(&l, 100, &faults, &[], 0, 100).retained;
        let second_half = 0.98_f64.min(1.0 - 1.0 / chutes);
        assert!((c - (0.98 + second_half) / 2.0).abs() < 1e-12, "{c}");
        let outage = [fault(
            0,
            10,
            FaultKind::InductionOutage {
                station: StationId(0),
            },
        )];
        let stations = l.stations.len() as f64;
        let c = capacity_loss(&l, 100, &outage, &[], 0, 10).retained;
        assert!((c - (1.0 - 1.0 / stations)).abs() < 1e-12, "{c}");
        assert!((capacity_loss(&l, 100, &[], &[], 5, 5).retained - 1.0).abs() < 1e-12);
    }

    #[test]
    fn an_aisle_block_costs_capacity_only_when_it_cuts_something_off() {
        let l = small_layout();
        let g = &l.grid;
        // A few closed cells in open floor, next to nothing in particular:
        // full capacity (the old closed-cells share would say < 1).
        let some: Vec<Pos> = (0..g.len())
            .filter_map(|i| u32::try_from(i).ok())
            .filter(|&i| g.is_free(i))
            .map(|i| g.pos(i))
            .filter(|p| p.y == g.height() / 2)
            .take(2)
            .collect();
        let c = capacity_loss(&l, 40, &[block(some)], &[], 0, 10).retained;
        assert!((c - 1.0).abs() < 1e-12, "{c}");

        // Close every access cell of chute 0: that chute is cut off.
        let around_chute: Vec<Pos> = l
            .chute_access(ChuteId(0))
            .into_iter()
            .map(|i| g.pos(i))
            .collect();
        let c = capacity_loss(&l, 40, &[block(around_chute)], &[], 0, 10).retained;
        let chutes = l.chutes.len() as f64;
        assert!((c - (1.0 - 1.0 / chutes)).abs() < 1e-12, "{c}");

        // Close every free cell around station 0: it can feed no chute.
        let around_station: Vec<Pos> = g
            .neighbours(l.stations[0])
            .filter(|&n| g.is_free(n))
            .map(|n| g.pos(n))
            .collect();
        let c = capacity_loss(&l, 40, &[block(around_station)], &[], 0, 10).retained;
        let stations = l.stations.len() as f64;
        assert!((c - (1.0 - 1.0 / stations)).abs() < 1e-12, "{c}");
    }

    fn medium_layout() -> Layout {
        let cfg = ScenarioConfig::from_toml_str(switchyard_core::presets::MEDIUM_TOML).unwrap();
        layout::generate(&cfg.floor).unwrap()
    }

    fn failed_on(robot: u32, cell: CellIdx, from: Tick, to: Tick) -> FailedRobot {
        FailedRobot {
            robot: RobotId(robot),
            cell,
            from,
            to,
        }
    }

    #[test]
    fn robots_failed_on_loading_cells_take_their_stations_out() {
        // The CP2 seed-1 case on the medium floor (200 robots, 8 stations):
        // R57 and R119 die while loading at stations 3 and 2.
        let l = medium_layout();
        assert_eq!(l.stations.len(), 8);
        let faults = [
            fault(0, 100, FaultKind::RobotFailure { robot: RobotId(57) }),
            fault(
                0,
                100,
                FaultKind::RobotFailure {
                    robot: RobotId(119),
                },
            ),
        ];
        let failures = [
            failed_on(57, l.stations[3], 0, 100),
            failed_on(119, l.stations[2], 0, 100),
        ];
        let loss = capacity_loss(&l, 200, &faults, &failures, 0, 100);
        assert!((loss.retained - 0.75).abs() < 1e-12, "{loss:?}");
        assert_eq!((loss.stations_lost, loss.chutes_lost), (2, 0));
        // The same two failures in open floor cost 2 robots of 200 only.
        let free: Vec<CellIdx> = (0..l.grid.len())
            .filter_map(|i| u32::try_from(i).ok())
            .filter(|&i| l.grid.is_free(i) && l.grid.pos(i).y == l.grid.height() / 2)
            .step_by(7)
            .take(2)
            .collect();
        let aisle = [
            failed_on(57, free[0], 0, 100),
            failed_on(119, free[1], 0, 100),
        ];
        let loss = capacity_loss(&l, 200, &faults, &aisle, 0, 100);
        assert!((loss.retained - 0.99).abs() < 1e-12, "{loss:?}");
        assert_eq!((loss.stations_lost, loss.chutes_lost), (0, 0));

        // Throughput kept 75 % of the twin's on 75 % capacity: thru/cap 1.0
        // (the old robots-only capacity, 0.99, gave about 0.76).
        let horizon = 6 * TICKS_PER_MINUTE;
        let mut twin = MinuteSeries::from_parcels(&[], horizon);
        twin.delivered_regular = vec![100; 6];
        let mut hit = twin.clone();
        hit.delivered_regular = vec![100, 100, 75, 75, 100, 100];
        let run = |s| Run {
            series: s,
            parcels: &[],
            ticks: horizon,
        };
        let window = Window {
            onset: 2,
            end: 4,
            capacity_retained: 0.75,
            stations_lost: 2,
            chutes_lost: 0,
        };
        let s = resilience::score(run(&twin), run(&hit), window, 0.3, Params::default());
        assert!((s.throughput_vs_capacity.unwrap() - 1.0).abs() < 1e-12);
        assert_eq!(s.stations_lost, 2);
    }

    #[test]
    fn a_robot_failed_on_a_chutes_access_cells_takes_the_chute_out() {
        let l = small_layout();
        let access = l.chute_access(ChuteId(0));
        let failures: Vec<FailedRobot> = access
            .iter()
            .zip(0..)
            .map(|(&c, r)| failed_on(r, c, 0, 10))
            .collect();
        let loss = capacity_loss(&l, 40, &[], &failures, 0, 10);
        assert_eq!((loss.stations_lost, loss.chutes_lost), (0, 1), "{loss:?}");
        let chutes = l.chutes.len() as f64;
        assert!((loss.retained - (1.0 - 1.0 / chutes)).abs() < 1e-12);
        // Failures outside the window cost nothing.
        let loss = capacity_loss(&l, 40, &[], &failures, 10, 20);
        assert!((loss.retained - 1.0).abs() < 1e-12);
    }

    #[test]
    fn the_seed_1_failures_are_read_from_the_run() {
        let cfg = ScenarioConfig::from_toml_str(
            switchyard_core::presets::by_name("two-dead-robots-and-a-jam").unwrap(),
        )
        .unwrap();
        assert_eq!(cfg.seed, 1);
        let f = run(&cfg, 3_700).unwrap();
        let l = &f.layout;
        let mut cells: Vec<(u32, CellIdx, Tick, Tick)> = f
            .failures
            .iter()
            .map(|x| (x.robot.0, x.cell, x.from, x.to))
            .collect();
        cells.sort_unstable();
        assert_eq!(
            cells,
            vec![
                (57, l.stations[3], 3_600, 3_700),
                (119, l.stations[2], 3_600, 3_700),
            ]
        );
        let faults: Vec<FaultSpec> = cfg.faults.clone();
        let loss = capacity_loss(l, 200, &faults, &f.failures, 3_600, 3_700);
        assert!((loss.retained - 0.75).abs() < 1e-12, "{loss:?}");
        assert_eq!(loss.stations_lost, 2);
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
        assert_eq!(b.ticks, 3_600);
        // The same regular arrivals in both runs (common random numbers),
        // so every regular parcel of the faulted run has a twin.
        assert_eq!(b.series.arrived, f.series.arrived);
        let score = resilience(&b, &f, Params::default());
        assert_eq!(score.delay.unmatched, 0, "{score:?}");
        assert!(score.baseline_utilisation > 0.0);
        assert!(
            score.delay.gross_parcel_ticks >= score.delay.faster_parcel_ticks,
            "{score:?}"
        );
        assert!(score.capacity_retained < 1.0);
    }
}
