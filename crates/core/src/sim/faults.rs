//! Phase 1 of the tick: faults that start or end, and the fault state the
//! other phases consult (see [`crate::faults`]).

use std::sync::Arc;

use super::{Sim, chute_id, robot_id, station_id};
use crate::TICKS_PER_HOUR;
use crate::Tick;
use crate::arrivals::PoissonTable;
use crate::config::ConfigError;
use crate::events::{ChuteState, Event};
use crate::faults::{FaultKind, FaultSpec, SURGE_KEY_FLAG};
use crate::grid::CellIdx;
use crate::guidance::{DistanceTables, UNREACHABLE};
use crate::hash::Fnv1a;
use crate::ids::{ChuteId, FaultId, StationId};
use crate::rng::{Stream, StreamId};
use crate::world::{Goal, Phase, RobotState};

/// Where a scheduled fault is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum Status {
    Pending = 0,
    Active = 1,
    Ended = 2,
    /// Switched off in the scenario (`disabled_faults`): never starts.
    Disabled = 3,
}

/// The extra arrival process of one volume surge at one station.
#[derive(Clone, Debug)]
pub(super) struct SurgeRun {
    pub fault: FaultId,
    pub station: StationId,
    pub table: PoissonTable,
    pub stream: Stream,
    /// The surge's stream key: its parcels' draw keys live in a range
    /// derived from it.
    pub key: u32,
    /// Surge parcels generated so far at this station.
    pub count: u32,
}

/// Draw keys of surge parcels have [`SURGE_KEY_FLAG`] (bit 62) set, the
/// fault's stream key in bits 28..60 and the parcel's count in bits 0..28:
/// disjoint from the regular parcels' keys (their ordinals, below 2^32) and
/// from other surges, and small enough that the scanner's `2 * key + 1` fits
/// in a u64.
pub(super) fn surge_key(fault_key: u32, count: u32) -> u64 {
    SURGE_KEY_FLAG | (u64::from(fault_key) << 28) | u64::from(count & 0x0fff_ffff)
}

/// The stream key inside a surge parcel's draw key.
fn surge_key_fault(key: u64) -> u32 {
    // Bits 28..60 hold a u32.
    #[allow(clippy::cast_possible_truncation)]
    let k = (key >> 28) as u32;
    k
}

/// Distance tables kept for reuse, keyed by their closed-cell set.
const TABLE_CACHE: usize = 8;

/// Fault schedule and the state of active faults. Empty (and absent from
/// the state hash) for a healthy run.
#[derive(Clone, Debug, Default)]
pub(super) struct Faults {
    pub specs: Vec<FaultSpec>,
    pub status: Vec<Status>,
    /// Per robot: the active failure that stopped it (lowest id if several).
    pub failed_by: Vec<Option<FaultId>>,
    /// Per robot: its state when it failed, restored on repair.
    pub pre_fail: Vec<RobotState>,
    /// Per chute: the active jam holding it (lowest id), and the state the
    /// jam interrupted.
    pub jammed_by: Vec<Option<FaultId>>,
    pub jam_prior: Vec<ChuteState>,
    /// Per station: the active induction outage (lowest id).
    pub outage_by: Vec<Option<FaultId>>,
    /// Per station: the misread rate imposed by active scanner drifts.
    pub drift_ppm: Vec<Option<u32>>,
    /// Active surge processes, by (fault, station).
    pub surges: Vec<SurgeRun>,
    /// Stream key and id of every surge that has started (its parcels can
    /// outlive it in the queues).
    pub surge_keys: Vec<(u32, FaultId)>,
    /// Active faults that close cells, in id order, with their cells.
    pub closed_by: Vec<(FaultId, Vec<CellIdx>)>,
    /// Per station: the fault whose closed cells cut it off from the fleet.
    pub station_cut: Vec<Option<FaultId>>,
    /// Single-goal rows with one topology fault reopened, for blame; valid
    /// until the closed cells change.
    row_cache: Vec<(Goal, FaultId, Arc<[u32]>)>,
    /// Full tables by closed-cell set (sorted), most recent last.
    table_cache: Vec<(Vec<CellIdx>, Arc<DistanceTables>)>,
}

impl Faults {
    pub fn new(robots: usize, chutes: usize, stations: usize) -> Self {
        Self {
            failed_by: vec![None; robots],
            pre_fail: vec![RobotState::Idle; robots],
            jammed_by: vec![None; chutes],
            jam_prior: vec![ChuteState::Open; chutes],
            outage_by: vec![None; stations],
            drift_ppm: vec![None; stations],
            station_cut: vec![None; stations],
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    /// Is any fault closing cells?
    pub fn topology_active(&self) -> bool {
        !self.closed_by.is_empty()
    }

    fn active(&self) -> impl Iterator<Item = (FaultId, &FaultSpec)> {
        self.specs
            .iter()
            .enumerate()
            .filter(|&(i, _)| self.status[i] == Status::Active)
            .map(|(i, f)| (fault_id(i), f))
    }

    /// The surge fault a surge parcel's draw key belongs to.
    pub fn surge_of(&self, key: u64) -> Option<FaultId> {
        let k = surge_key_fault(key);
        self.surge_keys
            .iter()
            .find(|&&(sk, _)| sk == k)
            .map(|&(_, f)| f)
    }
}

pub(super) fn fault_id(i: usize) -> FaultId {
    // Schedules are far shorter than u32::MAX entries.
    #[allow(clippy::cast_possible_truncation)]
    FaultId(i as u32)
}

impl Sim {
    /// Add a fault to the schedule at run time. It takes effect in phase 1
    /// of tick `spec.at` (so `at == self.tick()` starts it on the next
    /// [`Sim::step`]). Its id is its index in the schedule (after the
    /// scenario's faults), so the same fault gets the same id in a replay
    /// that schedules the same faults.
    ///
    /// # Errors
    /// [`ConfigError::BadFault`] if the fault is invalid for this floor or
    /// starts in the past; the sim is unchanged.
    pub fn schedule_fault(&mut self, spec: FaultSpec) -> Result<FaultId, ConfigError> {
        spec.validate_for(&self.layout, self.world.robots.len())?;
        if spec.at < self.tick {
            return Err(ConfigError::BadFault(format!(
                "{} at tick {} is in the past (now {})",
                spec.kind.name(),
                spec.at,
                self.tick
            )));
        }
        let id = fault_id(self.faults.specs.len());
        self.faults.specs.push(spec);
        self.faults.status.push(Status::Pending);
        Ok(id)
    }

    /// The fault schedule, indexed by [`FaultId`] (switched-off faults
    /// included, see [`Sim::fault_enabled`]).
    #[must_use]
    pub fn faults(&self) -> &[FaultSpec] {
        &self.faults.specs
    }

    /// Is fault `id` in the schedule and switched on?
    #[must_use]
    pub fn fault_enabled(&self, id: FaultId) -> bool {
        self.faults
            .status
            .get(id.index())
            .is_some_and(|&s| s != Status::Disabled)
    }

    /// Phase 1: end faults whose window closes at `t`, then start those
    /// whose window opens, each in id order.
    pub(super) fn apply_faults(&mut self, t: Tick, out: &mut Vec<Event>) {
        if self.faults.is_empty() {
            return;
        }
        let mut topology = false;
        for i in 0..self.faults.specs.len() {
            if self.faults.status[i] == Status::Active && self.faults.specs[i].end() == t {
                self.faults.status[i] = Status::Ended;
                out.push(Event::FaultEnded {
                    tick: t,
                    fault: fault_id(i),
                });
                topology |= self.faults.specs[i].kind.changes_topology();
                self.end_fault(t, i, out);
            }
        }
        for i in 0..self.faults.specs.len() {
            if self.faults.status[i] == Status::Pending && self.faults.specs[i].at == t {
                self.faults.status[i] = Status::Active;
                out.push(Event::FaultStarted {
                    tick: t,
                    fault: fault_id(i),
                });
                topology |= self.faults.specs[i].kind.changes_topology();
                self.start_fault(t, i, out);
            }
        }
        if topology {
            self.rebuild_topology();
        }
    }

    fn start_fault(&mut self, t: Tick, i: usize, out: &mut Vec<Event>) {
        let id = fault_id(i);
        match self.faults.specs[i].kind.clone() {
            FaultKind::RobotFailure { robot } => {
                let r = robot.index();
                if self.faults.failed_by[r].is_none() {
                    self.faults.pre_fail[r] = self.world.robots.state[r];
                    self.set_robot_state(t, r, RobotState::Failed, out);
                    // A parcel on its way to the chute is now stuck (one
                    // being loaded or dropped stays in that phase).
                    if self.faults.pre_fail[r] == RobotState::ToChute
                        && let Some(p) = self.world.robots.parcel[r]
                        && self.world.parcels.phase[p.index()] != Phase::ChuteWait
                    {
                        self.set_phase(t, p, Phase::Blocked, out);
                    }
                }
                self.refresh_failure(r);
            }
            FaultKind::AisleBlock { .. } => {}
            FaultKind::ChuteJam { chute } => {
                let c = chute.index();
                if self.faults.jammed_by[c].is_none() {
                    self.faults.jam_prior[c] = self.world.chutes.state[c];
                    self.world.chutes.state[c] = ChuteState::Jammed;
                    out.push(Event::ChuteChanged {
                        tick: t,
                        chute,
                        fill: self.world.chutes.fill[c],
                        state: ChuteState::Jammed,
                    });
                }
                self.refresh_jam(chute);
            }
            FaultKind::InductionOutage { station } => self.refresh_outage(station),
            FaultKind::VolumeSurge {
                station,
                factor_pct,
            } => {
                let key = self.faults.specs[i].stream_key(id);
                self.faults.surge_keys.push((key, id));
                let stations = self.world.stations.cell.len();
                let targets = station.map_or(0..stations, |s| s.index()..s.index() + 1);
                for s in targets {
                    let base = u64::from(self.arrival_rates[s]);
                    let extra = base * u64::from(factor_pct - 100) / 100;
                    let extra = u32::try_from(extra.min(u64::from(TICKS_PER_HOUR)))
                        .unwrap_or(TICKS_PER_HOUR);
                    let station = station_id(s);
                    self.faults.surges.push(SurgeRun {
                        fault: id,
                        station,
                        table: PoissonTable::new(extra),
                        stream: Stream::new(self.config.seed, StreamId::surge(key, station.0)),
                        key,
                        count: 0,
                    });
                }
            }
            FaultKind::ScannerDrift { station, .. } => self.refresh_drift(station),
        }
    }

    pub(super) fn end_fault(&mut self, t: Tick, i: usize, out: &mut Vec<Event>) {
        let id = fault_id(i);
        match self.faults.specs[i].kind.clone() {
            FaultKind::RobotFailure { robot } => {
                let r = robot.index();
                self.refresh_failure(r);
                if self.faults.failed_by[r].is_none() {
                    // Repaired: resume exactly what it was doing.
                    let state = self.faults.pre_fail[r];
                    self.set_robot_state(t, r, state, out);
                }
            }
            FaultKind::AisleBlock { .. } => {}
            FaultKind::ChuteJam { chute } => {
                self.refresh_jam(chute);
                let c = chute.index();
                if self.faults.jammed_by[c].is_none() {
                    let state = self.faults.jam_prior[c];
                    self.world.chutes.state[c] = state;
                    out.push(Event::ChuteChanged {
                        tick: t,
                        chute: chute_id(c),
                        fill: self.world.chutes.fill[c],
                        state,
                    });
                }
            }
            FaultKind::InductionOutage { station } => self.refresh_outage(station),
            FaultKind::VolumeSurge { .. } => self.faults.surges.retain(|s| s.fault != id),
            FaultKind::ScannerDrift { station, .. } => self.refresh_drift(station),
        }
    }

    /// Recompute which active failure (lowest id) holds robot `r`.
    fn refresh_failure(&mut self, r: usize) {
        let robot = robot_id(r);
        let holder = self
            .faults
            .active()
            .find(|(_, f)| f.kind == FaultKind::RobotFailure { robot })
            .map(|(id, _)| id);
        self.faults.failed_by[r] = holder;
    }

    /// Recompute which active jam (lowest id) holds `chute`.
    fn refresh_jam(&mut self, chute: ChuteId) {
        let holder = self
            .faults
            .active()
            .find(|(_, f)| f.kind == FaultKind::ChuteJam { chute })
            .map(|(id, _)| id);
        self.faults.jammed_by[chute.index()] = holder;
    }

    /// Recompute which active outage (lowest id) holds `station`.
    fn refresh_outage(&mut self, station: StationId) {
        let holder = self
            .faults
            .active()
            .find(|(_, f)| f.kind == FaultKind::InductionOutage { station })
            .map(|(id, _)| id);
        self.faults.outage_by[station.index()] = holder;
    }

    /// Recompute a station's drifted misread rate: the highest rate among
    /// the active drifts on it.
    fn refresh_drift(&mut self, station: StationId) {
        let ppm = self
            .faults
            .active()
            .filter_map(|(_, f)| match f.kind {
                FaultKind::ScannerDrift {
                    station: s,
                    error_rate_ppm,
                } if s == station => Some(error_rate_ppm),
                _ => None,
            })
            .max();
        self.faults.drift_ppm[station.index()] = ppm;
    }

    /// Collect the cells closed by active aisle blocks and dead robots, and
    /// switch to the distance tables for that closed set (the healthy tables
    /// when nothing is closed; cached tables when the set was seen before).
    /// Then find the stations the closed cells cut off from the parking
    /// lane, where idle robots wait for work.
    pub(super) fn rebuild_topology(&mut self) {
        let g = &self.layout.grid;
        let mut closed_by = Vec::new();
        for (id, f) in self.faults.active() {
            let cells = match &f.kind {
                FaultKind::AisleBlock { cells } => cells.iter().map(|&c| g.idx(c)).collect(),
                FaultKind::RobotFailure { robot } => vec![self.world.robots.pos[robot.index()]],
                _ => continue,
            };
            closed_by.push((id, cells));
        }
        let flatten = |by: &[(FaultId, Vec<CellIdx>)]| {
            let mut all: Vec<CellIdx> = by.iter().flat_map(|(_, c)| c.iter().copied()).collect();
            all.sort_unstable();
            all.dedup();
            all
        };
        // Closed cells never move while their fault holds (a dead robot
        // stays put), so the previous closures give the previous closed set.
        let before = flatten(&self.faults.closed_by);
        let all = flatten(&closed_by);
        self.faults.closed_by = closed_by;
        self.faults.row_cache.clear();
        self.distances = if all.is_empty() {
            Arc::clone(&self.healthy)
        } else if let Some((_, t)) = self.faults.table_cache.iter().find(|(c, _)| *c == all) {
            Arc::clone(t)
        } else {
            // Derive from the tables in force, recomputing only the goal
            // rows the change of closed cells can affect.
            let mask = |cells: &[CellIdx]| {
                let mut m = vec![false; g.len()];
                for &c in cells {
                    m[c as usize] = true;
                }
                m
            };
            let (tables, _) = self.distances.with_closed_changed(
                &self.layout,
                self.config.guidance.against_flow_cost,
                &mask(&before),
                &mask(&all),
            );
            let tables = Arc::new(tables);
            if self.faults.table_cache.len() == TABLE_CACHE {
                self.faults.table_cache.remove(0);
            }
            self.faults
                .table_cache
                .push((all.clone(), Arc::clone(&tables)));
            tables
        };

        for s in 0..self.faults.station_cut.len() {
            let goal = Goal::Station(station_id(s));
            let cut = !all.is_empty()
                && self
                    .layout
                    .parking
                    .iter()
                    .all(|&p| self.distances.get(goal, p) == UNREACHABLE);
            self.faults.station_cut[s] = match self.layout.parking.first() {
                Some(&p) if cut => Some(self.topology_blame(goal, p, UNREACHABLE)),
                _ => None,
            };
        }
    }

    /// The topology fault to blame for a robot at `cell` whose guided cost to
    /// `goal` is `current` on the faulted floor (`UNREACHABLE` when cut off,
    /// or a detour's length): the lowest-id active topology fault whose
    /// removal alone would bring the cost below `current`; if none would on
    /// its own (the faults only cut the route together), the lowest-id
    /// active topology fault. Requires an active topology fault.
    pub(super) fn topology_blame(&mut self, goal: Goal, cell: CellIdx, current: u32) -> FaultId {
        for k in 0..self.faults.closed_by.len() {
            let row = self.row_without(goal, k);
            if self.faults.row_cache[row].2[cell as usize] < current {
                return self.faults.closed_by[k].0;
            }
        }
        self.faults.closed_by.first().map_or(FaultId(0), |c| c.0)
    }

    /// Index in the row cache of `goal`'s distances with the `k`-th active
    /// topology fault's cells reopened (the others stay closed).
    fn row_without(&mut self, goal: Goal, k: usize) -> usize {
        let f = self.faults.closed_by[k].0;
        if let Some(i) = self
            .faults
            .row_cache
            .iter()
            .position(|(g, id, _)| *g == goal && *id == f)
        {
            return i;
        }
        let mut mask = vec![false; self.layout.grid.len()];
        for (j, (_, cells)) in self.faults.closed_by.iter().enumerate() {
            if j != k {
                for &c in cells {
                    mask[c as usize] = true;
                }
            }
        }
        let row = DistanceTables::goal_row(
            &self.layout,
            self.config.guidance.against_flow_cost,
            goal,
            &mask,
        );
        self.faults.row_cache.push((goal, f, row.into()));
        self.faults.row_cache.len() - 1
    }

    /// Phase 2, continued: the extra arrivals of active volume surges, one
    /// count draw per surge process per tick from its own stream. Surge
    /// parcels take draw keys from their own range ([`surge_key`]), so the
    /// regular parcels are identical to those of the run without the surge.
    pub(super) fn generate_surge_arrivals(&mut self, t: Tick, out: &mut Vec<Event>) {
        for i in 0..self.faults.surges.len() {
            let run = &mut self.faults.surges[i];
            let count = run.table.sample(&mut run.stream);
            for _ in 0..count {
                let run = &mut self.faults.surges[i];
                let key = surge_key(run.key, run.count);
                let ordinal = run.count;
                run.count += 1;
                let station = run.station;
                self.push_parcel(t, station, ordinal, key, out);
            }
        }
    }

    /// Feed the fault schedule and the state of active faults into the
    /// state hash. Called only when a schedule exists, so a healthy run
    /// hashes exactly as before faults existed. Caches and the blame state
    /// derived from the schedule are left out: they never change what
    /// robots do.
    pub(super) fn hash_faults(&self, h: &mut Fnv1a) {
        let f = &self.faults;
        h.write(b"faults-v2");
        h.write_u64(f.specs.len() as u64);
        for (spec, status) in f.specs.iter().zip(&f.status) {
            spec.hash_into(h);
            h.write_u8(*status as u8);
        }
        for (failed, state) in f.failed_by.iter().zip(&f.pre_fail) {
            h.write_opt_u32(failed.map(|x| x.0));
            if failed.is_some() {
                h.write_u8(*state as u8);
            }
        }
        for (jam, prior) in f.jammed_by.iter().zip(&f.jam_prior) {
            h.write_opt_u32(jam.map(|x| x.0));
            h.write_u8(*prior as u8);
        }
        for (outage, drift) in f.outage_by.iter().zip(&f.drift_ppm) {
            h.write_opt_u32(outage.map(|x| x.0));
            h.write_opt_u32(*drift);
        }
        h.write_u64(f.surges.len() as u64);
        for s in &f.surges {
            h.write_u32(s.fault.0);
            h.write_u16(s.station.0);
            h.write_u32(s.key);
            h.write_u32(s.count);
            h.write_u128(s.stream.word_pos());
        }
        let keys = &self.world.parcels.key;
        h.write_u64(keys.len() as u64);
        for &k in keys {
            h.write_u64(k);
        }
    }

    /// The misread rate in force at station `s`.
    pub(super) fn scanner_ppm_at(&self, s: StationId) -> u32 {
        self.faults
            .drift_ppm
            .get(s.index())
            .copied()
            .flatten()
            .unwrap_or(self.scanner_ppm)
    }

    /// Is station `s` dispatching work?
    pub(super) fn station_up(&self, s: usize) -> bool {
        self.faults.outage_by.get(s).is_none_or(Option::is_none)
    }
}
