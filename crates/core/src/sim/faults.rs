//! Phase 1 of the tick: faults that start or end (see [`crate::faults`]).

use super::{Sim, chute_id, robot_id, station_id};
use crate::TICKS_PER_HOUR;
use crate::Tick;
use crate::arrivals::PoissonTable;
use crate::config::ConfigError;
use crate::events::{ChuteState, Event};
use crate::faults::{FaultKind, FaultSpec};
use crate::guidance::DistanceTables;
use crate::hash::Fnv1a;
use crate::ids::{FaultId, StationId};
use crate::rng::{Stream, StreamId};
use crate::world::{Phase, RobotState};

/// Where a scheduled fault is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum Status {
    Pending = 0,
    Active = 1,
    Ended = 2,
}

/// The extra arrival process of one volume surge at one station.
#[derive(Clone, Debug)]
pub(super) struct SurgeRun {
    pub fault: FaultId,
    pub station: StationId,
    pub table: PoissonTable,
    pub stream: Stream,
    /// The surge's content key: its parcels' draw keys live in a range
    /// derived from it.
    pub key: u32,
    /// Surge parcels generated so far at this station.
    pub count: u32,
}

/// Draw keys of surge parcels have bit 62 set, the fault's content key in
/// bits 28..60 and the parcel's count in bits 0..28: disjoint from the
/// regular parcels' keys (their ordinals, below 2^32) and from other
/// surges, and small enough that the scanner's `2 * key + 1` fits in a u64.
pub(super) fn surge_key(fault_key: u32, count: u32) -> u64 {
    (1 << 62) | (u64::from(fault_key) << 28) | u64::from(count & 0x0fff_ffff)
}

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
    /// Per chute: active jams, and the state the first jam interrupted.
    pub jams: Vec<u16>,
    pub jam_prior: Vec<ChuteState>,
    /// Per station: active induction outages.
    pub outages: Vec<u16>,
    /// Per station: the misread rate imposed by active scanner drifts.
    pub drift_ppm: Vec<Option<u32>>,
    /// Active surge processes, by (fault, station).
    pub surges: Vec<SurgeRun>,
    /// Lowest active fault that closes cells, if any: the cause recorded
    /// by robots whose goal it cut off.
    pub topology: Option<FaultId>,
}

impl Faults {
    pub fn new(robots: usize, chutes: usize, stations: usize) -> Self {
        Self {
            failed_by: vec![None; robots],
            pre_fail: vec![RobotState::Idle; robots],
            jams: vec![0; chutes],
            jam_prior: vec![ChuteState::Open; chutes],
            outages: vec![0; stations],
            drift_ppm: vec![None; stations],
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    fn active(&self) -> impl Iterator<Item = (usize, &FaultSpec)> {
        self.specs
            .iter()
            .enumerate()
            .filter(|&(i, _)| self.status[i] == Status::Active)
    }
}

fn fault_id(i: usize) -> FaultId {
    // Schedules are far shorter than u32::MAX entries.
    #[allow(clippy::cast_possible_truncation)]
    FaultId(i as u32)
}

impl Sim {
    /// Add a fault to the schedule at run time. It takes effect in phase 1
    /// of tick `spec.at` (so `at == self.tick()` starts it on the next
    /// [`Sim::step`]). Removing it again is the counterfactual intervention:
    /// re-run without it (see [`crate::faults`]).
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

    /// The fault schedule, indexed by [`FaultId`].
    #[must_use]
    pub fn faults(&self) -> &[FaultSpec] {
        &self.faults.specs
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
                self.faults.jams[c] += 1;
                if self.faults.jams[c] == 1 {
                    self.faults.jam_prior[c] = self.world.chutes.state[c];
                    self.world.chutes.state[c] = ChuteState::Jammed;
                    out.push(Event::ChuteChanged {
                        tick: t,
                        chute,
                        fill: self.world.chutes.fill[c],
                        state: ChuteState::Jammed,
                    });
                }
            }
            FaultKind::InductionOutage { station } => {
                self.faults.outages[station.index()] += 1;
            }
            FaultKind::VolumeSurge {
                station,
                factor_pct,
            } => {
                let key = self.faults.specs[i].content_key();
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

    fn end_fault(&mut self, t: Tick, i: usize, out: &mut Vec<Event>) {
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
                let c = chute.index();
                self.faults.jams[c] -= 1;
                if self.faults.jams[c] == 0 {
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
            FaultKind::InductionOutage { station } => {
                self.faults.outages[station.index()] -= 1;
            }
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
            .map(|(i, _)| fault_id(i));
        self.faults.failed_by[r] = holder;
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

    /// Close the cells of active aisle blocks and dead robots, and rebuild
    /// the guided distance tables around them (healthy tables when nothing
    /// is closed). Robots whose goal is cut off then record
    /// `CellBlocked(topology fault)` in phase 8.
    fn rebuild_topology(&mut self) {
        let g = &self.layout.grid;
        let mut closed = vec![false; g.len()];
        let mut any = None::<FaultId>;
        for (i, f) in self.faults.active() {
            match &f.kind {
                FaultKind::AisleBlock { cells } => {
                    for &c in cells {
                        closed[g.idx(c) as usize] = true;
                    }
                }
                FaultKind::RobotFailure { robot } => {
                    closed[self.world.robots.pos[robot.index()] as usize] = true;
                }
                _ => continue,
            }
            any = any.or(Some(fault_id(i)));
        }
        self.faults.topology = any;
        let closed: &[bool] = if any.is_some() { &closed } else { &[] };
        self.distances = DistanceTables::build_with_closed(
            &self.layout,
            self.config.guidance.against_flow_cost,
            closed,
        );
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
    /// hashes exactly as before faults existed.
    pub(super) fn hash_faults(&self, h: &mut Fnv1a) {
        let f = &self.faults;
        h.write(b"faults-v1");
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
        for (jams, prior) in f.jams.iter().zip(&f.jam_prior) {
            h.write_u16(*jams);
            h.write_u8(*prior as u8);
        }
        for (outages, drift) in f.outages.iter().zip(&f.drift_ppm) {
            h.write_u16(*outages);
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
        self.faults.outages.get(s).is_none_or(|&n| n == 0)
    }
}
