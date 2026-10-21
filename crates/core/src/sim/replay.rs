//! Checkpoints, counterfactual replays and re-convergence (ADR 0005).
//!
//! # Checkpoint and resume
//! A [`Sim`] is `Clone`: a clone taken between two [`Sim::step`] calls is a
//! checkpoint, and stepping it reproduces the original run bit-for-bit
//! (same events, same [`Sim::state_hash`]). The layout, the guided distance
//! tables and the fault-blame rows are shared (`Arc`), so a checkpoint
//! copies the entity state and per-robot buffers
//! ([`Sim::checkpoint_bytes`] estimates it).
//!
//! A replay resumes from a checkpoint with a modified schedule or another
//! intervention ([`Sim::apply_intervention`], [`Sim::disable_fault`]):
//! - disabling a fault that has **not started** (`at >= tick`) is exactly
//!   the run with that fault disabled from tick 0
//!   ([`crate::config::ScenarioConfig::without_fault`]): bit-identical state
//!   hash, since nothing before `at` depended on it;
//! - disabling an **active** fault ends it now, before the next step, as if
//!   its window had closed at this tick: the robot is repaired, the chute
//!   unjammed, the cells reopened, the station resumes, the surge stops, the
//!   scanner returns to its base rate. A `FaultEnded` event is emitted with
//!   the current tick. What the fault already did (a backlog, a stuck
//!   parcel's lost time) stays: the replay is "the fault was fixed at this
//!   tick", not "it never happened";
//! - a fault that has already ended (or is already disabled) cannot be
//!   disabled: replay from an earlier checkpoint instead.
//!
//! # Re-convergence
//! [`Sim::state_hash`] covers history (every delivered parcel's timings and
//! the fault schedule), so a replay never hashes equal to the original once
//! anything differed. [`Sim::live_hash`] covers only the state that decides
//! the future: robots, parcels not yet delivered, queues, chutes, the
//! sequential random streams, and pending or active faults. Two sims with
//! equal live hashes at the same tick produce the same future (identical
//! moves, deliveries and parcel ids), so a replay can stop as soon as its
//! live hash matches the original's ([`Sim::reconverge_with`],
//! [`Sim::reconverge_against`]). Exact re-convergence is rare: once robots
//! have been shuffled, every robot must return to the same cell with the
//! same task at the same tick.

use super::faults::{Status, fault_id};
use super::state_hash::hash_goal;
use super::{Sim, robot_id};
use crate::Tick;
use crate::config::{ConfigError, DestinationRemap, RobotAddition, StationCap};
use crate::events::Event;
use crate::hash::Fnv1a;
use crate::ids::FaultId;
use crate::interventions::Intervention;
use crate::world::RobotState;

impl Sim {
    /// Switch fault `id` off from now on (see the module docs for pending
    /// and active faults). Events of the change (`FaultEnded`, a repaired
    /// robot's `RobotState`, an unjammed chute's `ChuteChanged`) are
    /// appended to `out` with the current tick.
    ///
    /// # Errors
    /// [`ConfigError::BadFault`] if `id` is not in the schedule or has
    /// already ended or been disabled; the sim is unchanged.
    pub fn disable_fault(&mut self, id: FaultId, out: &mut Vec<Event>) -> Result<(), ConfigError> {
        let i = id.index();
        let status = self.faults.status.get(i).copied();
        match status {
            Some(Status::Pending) => self.faults.status[i] = Status::Disabled,
            Some(Status::Active) => {
                let t = self.tick;
                self.faults.status[i] = Status::Disabled;
                out.push(Event::FaultEnded { tick: t, fault: id });
                self.end_fault(t, i, out);
                if self.faults.specs[i].kind.changes_topology() {
                    self.rebuild_topology();
                }
            }
            _ => {
                return Err(ConfigError::BadFault(format!(
                    "{id} cannot be disabled at tick {}: {}",
                    self.tick,
                    if status.is_none() {
                        "not in the schedule"
                    } else {
                        "it has already ended or been disabled"
                    }
                )));
            }
        }
        self.config = self.config.without_fault(id);
        Ok(())
    }

    /// Apply an intervention from the current tick on (see
    /// [`crate::interventions`] for each kind's semantics); events of the
    /// change go to `out`. The sim's [`Sim::config`] records it.
    ///
    /// # Errors
    /// [`ConfigError`] if the intervention is invalid for this floor or this
    /// tick (an unknown chute, station or destination, a zero cap, robots
    /// added in the past, a fault that cannot be disabled); the sim is
    /// unchanged.
    pub fn apply_intervention(
        &mut self,
        intervention: &Intervention,
        out: &mut Vec<Event>,
    ) -> Result<(), ConfigError> {
        if let Intervention::DisableFault(f) = *intervention {
            return self.disable_fault(f, out);
        }
        let updated = self.config.apply(intervention)?;
        match *intervention {
            Intervention::DisableFault(_) => {}
            Intervention::RemapDestination { destination, chute } => {
                check_remap(
                    &DestinationRemap { destination, chute },
                    self.world.destination_chute.len(),
                    self.world.chutes.cell.len(),
                )?;
                self.world.destination_chute[destination.index()] = chute;
            }
            Intervention::AddRobots { robots, at } => {
                if at < self.tick {
                    return Err(ConfigError::OutOfRange {
                        field: "fleet.additions.at",
                        value: u64::from(at),
                        min: u64::from(self.tick),
                        max: u64::from(Tick::MAX),
                    });
                }
                self.additions.push(RobotAddition { at, robots });
            }
            Intervention::SetStationCap { station, cap } => {
                if let Some(s) = station {
                    check_cap(
                        &StationCap { station: s, cap },
                        self.world.stations.cell.len(),
                    )?;
                    self.station_cap[s.index()] = cap;
                } else {
                    self.station_cap.fill(cap);
                }
            }
        }
        self.config = updated;
        Ok(())
    }

    /// Phase 1, first: robots that join the fleet this tick (scheduled by
    /// `fleet.additions` or [`Intervention::AddRobots`]) appear idle on the
    /// first free parking cells, then the first free floor cells, in
    /// row-major order.
    pub(super) fn apply_additions(&mut self, t: Tick, out: &mut Vec<Event>) {
        if self.additions.is_empty() {
            return;
        }
        let due: u32 = self
            .additions
            .iter()
            .filter(|a| a.at == t)
            .map(|a| a.robots)
            .sum();
        self.additions.retain(|a| a.at != t);
        if due == 0 {
            return;
        }
        let g = &self.layout.grid;
        let mut taken = vec![false; g.len()];
        for &p in &self.world.robots.pos {
            taken[p as usize] = true;
        }
        let free = self
            .layout
            .parking
            .iter()
            .copied()
            .chain((0..u32::try_from(g.len()).unwrap_or(u32::MAX)).filter(|&c| g.is_free(c)));
        let mut cells = Vec::new();
        for c in free {
            if cells.len() as u64 == u64::from(due) {
                break;
            }
            if !taken[c as usize] {
                taken[c as usize] = true;
                cells.push(c);
            }
        }
        for c in cells {
            let r = self.world.robots.len();
            let robots = &mut self.world.robots;
            robots.pos.push(c);
            robots.state.push(RobotState::Idle);
            robots.parcel.push(None);
            robots.goal.push(None);
            robots.timer.push(0);
            robots.waited.push(0);
            robots.goal_since.push(t);
            out.push(Event::RobotState {
                tick: t,
                robot: robot_id(r),
                state: RobotState::Idle,
            });
        }
        self.size_per_robot();
    }
}

/// Does the remap name an existing destination and chute?
pub(super) fn check_remap(
    r: &DestinationRemap,
    destinations: usize,
    chutes: usize,
) -> Result<(), ConfigError> {
    if r.destination.index() >= destinations || r.chute.index() >= chutes {
        return Err(ConfigError::OutOfRange {
            field: "chutes.remap",
            value: u64::from(r.destination.0.max(r.chute.0)),
            min: 0,
            max: (destinations.min(chutes) as u64).saturating_sub(1),
        });
    }
    Ok(())
}

/// Does the cap name an existing station?
pub(super) fn check_cap(c: &StationCap, stations: usize) -> Result<(), ConfigError> {
    if c.station.index() >= stations {
        return Err(ConfigError::OutOfRange {
            field: "fleet.station_caps.station",
            value: u64::from(c.station.0),
            min: 0,
            max: (stations as u64).saturating_sub(1),
        });
    }
    Ok(())
}

impl Sim {
    /// Hash of the state that decides the future (see the module docs):
    /// equal live hashes at the same tick mean identical futures. Excludes
    /// history (delivered parcels, counters of past events, timestamps that
    /// only label spans), the positions of keyed random streams (keyed draws
    /// do not depend on them), observer state (wait records, watchdog and
    /// deadlock tracking) and ended or disabled faults.
    #[must_use]
    pub fn live_hash(&self) -> u64 {
        let mut h = Fnv1a::new();
        h.write(b"switchyard-live-v1");
        h.write_u32(self.tick);
        let r = &self.world.robots;
        h.write_u64(r.len() as u64);
        for i in 0..r.len() {
            h.write_u32(r.pos[i]);
            h.write_u8(r.state[i] as u8);
            h.write_opt_u32(r.parcel[i].map(|p| p.0));
            hash_goal(&mut h, r.goal[i]);
            h.write_u16(r.timer[i]);
            h.write_u32(r.goal_since[i]);
        }
        // Every undelivered parcel is in a station queue or on a robot, so
        // hashing those covers the live parcels in O(live), not O(history).
        let p = &self.world.parcels;
        h.write_u64(p.len() as u64);
        let live = self
            .world
            .stations
            .queue
            .iter()
            .flatten()
            .chain(r.parcel.iter().flatten());
        for parcel in live {
            let i = parcel.index();
            h.write_u32(parcel.0);
            h.write_u16(p.station[i].0);
            h.write_u64(p.key[i]);
            h.write_u16(p.destination[i].0);
            h.write_u8(p.phase[i] as u8);
            h.write_opt_u32(p.robot[i].map(|r| r.0));
            h.write_opt_u32(p.target[i].map(|c| u32::from(c.0)));
        }
        let s = &self.world.stations;
        for i in 0..s.cell.len() {
            h.write_u16(s.inbound[i]);
            h.write_u32(s.arrived[i]);
            h.write_u32(self.arrival_rates[i]);
            h.write_u16(self.station_cap[i]);
            h.write_u64(s.queue[i].len() as u64);
            for q in &s.queue[i] {
                h.write_u32(q.0);
            }
        }
        let c = &self.world.chutes;
        for i in 0..c.cell.len() {
            h.write_u32(c.fill[i]);
            h.write_u8(c.state[i] as u8);
            h.write_u32(c.timer[i]);
            h.write_u16(c.incoming[i]);
        }
        for d in &self.world.destination_chute {
            h.write_u16(d.0);
        }
        for st in &self.arrival_streams {
            h.write_u128(st.word_pos());
        }
        h.write_u32(self.scanner_ppm);
        for a in &self.additions {
            h.write_u32(a.at);
            h.write_u32(a.robots);
        }
        self.planner.hash_state(&mut h);
        self.hash_live_faults(&mut h);
        h.finish()
    }

    fn hash_live_faults(&self, h: &mut Fnv1a) {
        let f = &self.faults;
        for (i, spec) in f.specs.iter().enumerate() {
            if matches!(f.status[i], Status::Pending | Status::Active) {
                h.write_u32(fault_id(i).0);
                h.write_u8(f.status[i] as u8);
                spec.hash_into(h);
            }
        }
        for (r, failed) in f.failed_by.iter().enumerate() {
            if let Some(id) = failed {
                h.write_u64(r as u64);
                h.write_u32(id.0);
                h.write_u8(f.pre_fail[r] as u8);
            }
        }
        for (c, jam) in f.jammed_by.iter().enumerate() {
            if jam.is_some() {
                h.write_u64(c as u64);
                h.write_u8(f.jam_prior[c] as u8);
            }
        }
        for (o, d) in f.outage_by.iter().zip(&f.drift_ppm) {
            h.write_opt_u32(o.map(|x| x.0));
            h.write_opt_u32(*d);
        }
        for s in &f.surges {
            h.write_u32(s.fault.0);
            h.write_u16(s.station.0);
            h.write_u32(s.count);
            h.write_u128(s.stream.word_pos());
        }
    }

    /// Step this sim and `reference` in lockstep (both must be at the same
    /// tick) until their live hashes match or `until` is reached. Returns the
    /// first tick at which they match (`self.tick()` after the step), or
    /// `None`. Events are discarded; a replay that needs them steps itself
    /// and compares [`Sim::live_hash`].
    ///
    /// # Panics
    /// If the two sims are not at the same tick.
    pub fn reconverge_with(&mut self, reference: &mut Self, until: Tick) -> Option<Tick> {
        assert_eq!(self.tick, reference.tick, "lockstep needs equal ticks");
        let mut events = Vec::new();
        while self.tick < until {
            events.clear();
            self.step(&mut events);
            events.clear();
            reference.step(&mut events);
            // Matches are rare: a cheap test first, the hashes only then.
            if self.world.robots.pos == reference.world.robots.pos
                && self.live_hash() == reference.live_hash()
            {
                return Some(self.tick);
            }
        }
        None
    }

    /// As [`Sim::reconverge_with`], against live hashes recorded from the
    /// reference run: `recorded[t]` is its live hash after `t` ticks. Steps
    /// until a match or the end of the recording.
    pub fn reconverge_against(&mut self, recorded: &[u64]) -> Option<Tick> {
        let mut events = Vec::new();
        while (self.tick as usize) + 1 < recorded.len() {
            events.clear();
            self.step(&mut events);
            if self.live_hash() == recorded[self.tick as usize] {
                return Some(self.tick);
            }
        }
        None
    }

    /// Approximate heap size of a checkpoint ([`Clone`]) in bytes: the
    /// entity columns, queues and per-robot buffers. The layout, distance
    /// tables and blame rows are shared between clones and not counted; the
    /// planner's scratch (two words per cell for PIBT) is not counted either.
    #[must_use]
    pub fn checkpoint_bytes(&self) -> usize {
        use std::mem::size_of_val;
        let w = &self.world;
        let r = &w.robots;
        let p = &w.parcels;
        let robots = size_of_val(r.pos.as_slice())
            + size_of_val(r.state.as_slice())
            + size_of_val(r.parcel.as_slice())
            + size_of_val(r.goal.as_slice())
            + size_of_val(r.timer.as_slice())
            + size_of_val(r.waited.as_slice())
            + size_of_val(r.goal_since.as_slice());
        let parcels = size_of_val(p.station.as_slice())
            + size_of_val(p.ordinal.as_slice())
            + size_of_val(p.key.as_slice())
            + size_of_val(p.destination.as_slice())
            + size_of_val(p.arrived.as_slice())
            + size_of_val(p.phase.as_slice())
            + size_of_val(p.phase_since.as_slice())
            + size_of_val(p.robot.as_slice())
            + size_of_val(p.target.as_slice())
            + size_of_val(p.chute.as_slice());
        let queues: usize = w.stations.queue.iter().map(|q| q.len() * 4).sum();
        let chutes = w.chutes.cell.len() * 15 + w.destination_chute.len() * 2;
        // waits, roots, cut_by, detour, progressed, at_chute and the plan
        // buffers: about 40 bytes per robot.
        robots + parcels + queues + chutes + r.len() * 40
    }
}
