//! The tick loop.
//!
//! # Tick order
//!
//! Every call to [`Sim::step`] runs these phases in this fixed order
//! (ARCHITECTURE §3). Within a phase, entities are processed in ascending id
//! order, so the outcome never depends on container iteration order
//! (ADR 0002 §4). Events are appended as each phase runs, so the stream
//! follows this order too.
//!
//! 1. Apply faults that end, then faults that start, at this tick
//!    ([`crate::faults`]); a fault that closes cells rebuilds the distance
//!    tables.
//! 2. Generate arrivals: a Poisson count per station, then a Zipf
//!    destination per parcel, each station on its own streams; then the
//!    extra arrivals of active volume surges, from the surges' own streams.
//! 3. Advance timers: bag swaps finish; loads finish (the scanner reads the
//!    parcel and the robot heads for its chute); drops finish (the parcel is
//!    delivered, a full bag starts a swap).
//! 4. Assign work: each station that is up (no induction outage), in id
//!    order, hands its oldest unclaimed
//!    parcels to the nearest idle robots (by guided distance, ties by id),
//!    up to `fleet.max_inbound_per_station`. Idle robots left without work head
//!    for the parking lane (throttling, see below).
//! 5. Plan moves with the [`Planner`] (guided PIBT by default).
//! 6. Apply moves; carried parcels switch between `Travelling` and
//!    `Blocked`.
//! 7. Resolve arrivals at goals: start loads, and start drops at chutes that
//!    can take a parcel.
//! 8. Attribute waits: every robot's direct cause goes into the tick's
//!    wait-for record ([`Sim::wait_for`]); the record is walked to each
//!    robot's root ([`crate::causality`]); robots with work (heading to a
//!    station or a chute) get a `RobotWaited { direct, root }`, and a
//!    `RobotStalled` when they have waited `planner.watchdog_ticks` ticks in
//!    a row; a wait-for cycle that has persisted `planner.deadlock_ticks`
//!    ticks is reported with `DeadlockDetected`.
//! 9. Watchdog: if robots have work but none has progressed for
//!    `planner.watchdog_ticks`, emit `Stalled`.
//! 10. Advance the tick. The state hash is computed on demand from the
//!     canonical state ([`Sim::state_hash`]).
//!
//! # Throttling
//! Admission control is twofold, both deliberately simple (ADR 0003;
//! research §1 "agent disabling"): a station accepts at most
//! `fleet.max_inbound_per_station` robots (see its docs for the sizing),
//! which caps how many robots crowd the
//! induction area; and every idle robot that phase 4 could not give work to
//! is surplus to the floor's current demand, so instead of loitering in the
//! sort field (where PIBT would keep pushing it around) it drives to the
//! perimeter parking lane ([`crate::layout`]) while there is a free parking
//! spot. It is reassigned from wherever it is as soon as work appears.

use std::collections::VecDeque;
use std::sync::Arc;

mod assign;
mod attribution;
mod faults;
mod mapping;
mod replay;
mod snapshot;
mod state_hash;

pub use snapshot::Snapshot;

use crate::arrivals::{PoissonTable, ZipfTable};
use crate::causality::{DeadlockWatch, RootWalker};
use crate::config::{ConfigError, RobotAddition, ScenarioConfig};
use crate::events::{ChuteState, Event, WaitCause};
use crate::grid::CellIdx;
use crate::guidance::{DistanceTables, UNREACHABLE};
use crate::ids::{ChuteId, DestinationId, FaultId, ParcelId, RobotId, StationId};
use crate::layout::{self, Layout};
use crate::pibt::PibtPlanner;
use crate::planner::{PlanInput, PlanOutput, Planner};
use crate::rng::{Stream, StreamId};
use crate::world::{Goal, Phase, RobotState, World};
use crate::{TICKS_PER_HOUR, Tick};

/// Aggregate counters of a run.
///
/// Conservation holds at every tick: `arrived = queued + in_flight +
/// delivered`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Ticks simulated so far.
    pub tick: Tick,
    /// Parcels that have arrived at any station.
    pub arrived: u64,
    /// Parcels waiting at a station (phases `Queued`, `AwaitingRobot`).
    pub queued: u64,
    /// Parcels on a robot (phases `Loading` through `ChuteWait`), including
    /// parcels stuck on a failed robot.
    pub in_flight: u64,
    /// Parcels dropped into a chute (mis-sorted ones included).
    pub delivered: u64,
    /// Parcels dropped into a chute that does not serve their destination.
    pub mis_sorted: u64,
}

impl Stats {
    /// Delivered parcels per simulated hour, scaled by 1,000 to stay in
    /// integers (divide by 1,000.0 for display).
    #[must_use]
    pub fn throughput_per_hour_milli(&self) -> u64 {
        if self.tick == 0 {
            return 0;
        }
        self.delivered * u64::from(TICKS_PER_HOUR) * 1_000 / u64::from(self.tick)
    }
}

/// A running simulation.
///
/// `Clone` makes a checkpoint: stepping a clone reproduces the original run
/// bit-for-bit (see [`replay`] for checkpoints and counterfactual replays).
/// The layout and the distance tables are shared between clones.
#[derive(Clone, Debug)]
pub struct Sim {
    config: ScenarioConfig,
    layout: Arc<Layout>,
    /// Guided distances in force: `healthy`, or tables with the cells of
    /// active faults closed ([`crate::faults`]).
    distances: Arc<DistanceTables>,
    /// Guided distances of the healthy floor.
    healthy: Arc<DistanceTables>,
    world: World,
    tick: Tick,
    /// Current arrival rate of each station, per hour (changed by
    /// [`Sim::set_arrival_rate`]), and its sampling table.
    arrival_rates: Vec<u32>,
    arrival_tables: Vec<PoissonTable>,
    arrival_streams: Vec<Stream>,
    zipf: ZipfTable,
    destination_streams: Vec<Stream>,
    scanner_streams: Vec<Stream>,
    /// Current scanner misread rate (changed by
    /// [`Sim::set_scanner_error_rate`]).
    scanner_ppm: u32,
    planner: Box<dyn Planner>,
    /// The fault schedule and the state of active faults.
    faults: faults::Faults,
    /// Inbound cap of each station (`fleet.max_inbound_per_station`, with
    /// `fleet.station_caps` and runtime interventions applied).
    station_cap: Vec<u16>,
    /// Robot additions still to come.
    additions: Vec<RobotAddition>,
    // Counters, kept incrementally (Stats checks them against the world).
    arrived: u64,
    delivered: u64,
    mis_sorted: u64,
    /// Last tick on which a robot with work made progress (watchdog).
    last_progress: Tick,
    /// Last tick the watchdog fired.
    last_stall: Tick,
    /// The direct wait cause of each robot in the last tick (its out-edge in
    /// the tick's wait-for graph).
    waits: Vec<Option<WaitCause>>,
    /// Root-cause walk over `waits` (scratch, rebuilt every tick).
    walker: RootWalker,
    /// Persistence tracking of wait-for cycles. Observer state: it decides
    /// only when `DeadlockDetected` is emitted, never what robots do, so it
    /// is not part of the state hash (like `waits`).
    deadlocks: DeadlockWatch,
    /// Each robot's reported root cause in the last tick (scratch).
    roots: Vec<Option<WaitCause>>,
    /// `DeadlockDetected` events of the tick, emitted after the waits.
    deadlock_events: Vec<Event>,
    // Per-tick scratch buffers, reused to avoid allocation in the loop.
    plan_goal: Vec<Option<Goal>>,
    plan_movable: Vec<bool>,
    plan_priority: Vec<u32>,
    plan_out: PlanOutput,
    /// Robots that started the tick at their chute, unable to drop.
    at_chute: Vec<bool>,
    /// Robots whose goal a fault has cut off this tick, and the fault blamed.
    cut_by: Vec<Option<FaultId>>,
    /// Robots on a detour this tick (closer to their goal only on the
    /// faulted floor), with the cell they left.
    detour: Vec<Option<CellIdx>>,
    idle: Vec<u32>,
}

impl Sim {
    /// Build a simulation from a scenario, using guided PIBT.
    ///
    /// # Errors
    /// Any [`ConfigError`] from validation or layout generation, or
    /// [`ConfigError::BadRobotCount`] if the robots do not fit.
    pub fn new(config: &ScenarioConfig) -> Result<Self, ConfigError> {
        Self::with_planner(config, Box::new(PibtPlanner::new()))
    }

    /// Build a simulation with a specific planner (tests can plug in a
    /// scripted one).
    ///
    /// # Errors
    /// As [`Sim::new`].
    pub fn with_planner(
        config: &ScenarioConfig,
        planner: Box<dyn Planner>,
    ) -> Result<Self, ConfigError> {
        config.validate()?;
        let layout = layout::generate(&config.floor)?;
        let distances = Arc::new(DistanceTables::build(
            &layout,
            config.guidance.against_flow_cost,
        ));

        let capacity = layout.spawn_capacity();
        let robots = config.fleet.robots;
        let max = u32::try_from(capacity).unwrap_or(u32::MAX);
        let spawn = (robots > 0)
            .then(|| layout.spawn_cells(robots as usize))
            .flatten()
            .ok_or(ConfigError::BadRobotCount { robots, max })?;

        let mut world = World::default();
        let n = spawn.len();
        world.robots.pos = spawn;
        world.robots.state = vec![RobotState::Idle; n];
        world.robots.parcel = vec![None; n];
        world.robots.goal = vec![None; n];
        world.robots.timer = vec![0; n];
        world.robots.waited = vec![0; n];
        world.robots.goal_since = vec![0; n];

        let chutes = layout.chutes.len();
        world.chutes.cell.clone_from(&layout.chutes);
        world.chutes.fill = vec![0; chutes];
        world.chutes.state = vec![ChuteState::Open; chutes];
        world.chutes.timer = vec![0; chutes];
        world.chutes.incoming = vec![0; chutes];

        let destinations = config
            .arrivals
            .destinations
            .unwrap_or_else(|| u16::try_from(chutes).unwrap_or(u16::MAX));
        world.destination_chute = mapping::map_destinations(
            usize::from(destinations),
            config.chutes.mapping,
            &layout,
            &distances,
        );

        let stations = layout.stations.len();
        let mut station_cap = vec![config.fleet.max_inbound_per_station; stations];
        world.stations.cell.clone_from(&layout.stations);
        world.stations.queue = vec![VecDeque::new(); stations];
        world.stations.inbound = vec![0; stations];
        world.stations.arrived = vec![0; stations];

        let station_ids = 0..u16::try_from(stations).unwrap_or(u16::MAX);
        let streams = |id: fn(u16) -> StreamId| {
            station_ids
                .clone()
                .map(|s| Stream::new(config.seed, id(s)))
                .collect::<Vec<_>>()
        };
        let rate = config.arrivals.parcels_per_hour_per_station;
        let mut faults = faults::Faults::new(n, chutes, stations);
        for (i, spec) in config.faults.iter().enumerate() {
            spec.validate_for(&layout, n)?;
            faults.specs.push(spec.clone());
            faults
                .status
                .push(if config.fault_enabled(faults::fault_id(i)) {
                    faults::Status::Pending
                } else {
                    faults::Status::Disabled
                });
        }

        for c in &config.fleet.station_caps {
            replay::check_cap(c, stations)?;
            station_cap[c.station.index()] = c.cap;
        }
        for r in &config.chutes.remap {
            replay::check_remap(r, world.destination_chute.len(), chutes)?;
            world.destination_chute[r.destination.index()] = r.chute;
        }

        let mut sim = Self {
            config: config.clone(),
            layout: Arc::new(layout),
            healthy: Arc::clone(&distances),
            distances,
            world,
            tick: 0,
            arrival_rates: vec![rate; stations],
            arrival_tables: vec![PoissonTable::new(rate); stations],
            arrival_streams: streams(StreamId::arrivals),
            zipf: ZipfTable::new(destinations, config.arrivals.zipf_s_milli),
            destination_streams: streams(StreamId::destinations),
            scanner_streams: streams(StreamId::scanner),
            scanner_ppm: config.scanner.error_rate_ppm,
            planner,
            faults,
            station_cap,
            additions: config.fleet.additions.clone(),
            arrived: 0,
            delivered: 0,
            mis_sorted: 0,
            last_progress: 0,
            last_stall: 0,
            waits: Vec::new(),
            walker: RootWalker::new(),
            deadlocks: DeadlockWatch::new(config.planner.deadlock_ticks),
            roots: Vec::new(),
            deadlock_events: Vec::new(),
            plan_goal: Vec::with_capacity(n),
            plan_movable: Vec::with_capacity(n),
            plan_priority: Vec::with_capacity(n),
            plan_out: PlanOutput::default(),
            at_chute: Vec::new(),
            cut_by: Vec::new(),
            detour: Vec::new(),
            idle: Vec::with_capacity(n),
        };
        sim.size_per_robot();
        Ok(sim)
    }

    /// Size every per-robot buffer outside the world's robot columns to the
    /// fleet (at construction, and when robots join).
    fn size_per_robot(&mut self) {
        let n = self.world.robots.len();
        self.waits.resize(n, None);
        self.roots.resize(n, None);
        self.at_chute.resize(n, false);
        self.cut_by.resize(n, None);
        self.detour.resize(n, None);
        self.faults.failed_by.resize(n, None);
        self.faults.pre_fail.resize(n, RobotState::Idle);
        self.faults.failed_since.resize(n, 0);
    }

    /// The scenario this sim was built from.
    #[must_use]
    pub fn config(&self) -> &ScenarioConfig {
        &self.config
    }

    /// The generated floor.
    #[must_use]
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// The guided distance tables the planner uses (rebuilt around closed
    /// cells while an aisle block or a robot failure is active).
    #[must_use]
    pub fn distances(&self) -> &DistanceTables {
        &self.distances
    }

    /// Read-only access to all entity state (for tests and inspection; the
    /// supported output is the event stream).
    #[must_use]
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Ticks simulated so far. The next [`Sim::step`] simulates this tick.
    #[must_use]
    pub fn tick(&self) -> Tick {
        self.tick
    }

    /// The direct wait cause of every robot in the last simulated tick
    /// (`None` for robots that progressed or had nothing to do). These are
    /// the tick's complete wait-for edges, idle and parking robots included,
    /// so every chain can be walked to its end; the tick's `RobotWaited`
    /// events are the subset for robots with work.
    #[must_use]
    pub fn wait_for(&self) -> &[Option<WaitCause>] {
        &self.waits
    }

    /// Change one station's arrival rate from the next tick on. Only that
    /// station's process changes: its stream keeps its position, and every
    /// other station draws from its own streams, so their arrivals are
    /// unaffected (ADR 0002 §3). This is the hook for induction outages and
    /// volume surges (CP2).
    ///
    /// # Errors
    /// [`ConfigError::OutOfRange`] if `station` does not exist, or
    /// [`ConfigError::ArrivalRateTooHigh`] above one parcel per tick; the
    /// sim is unchanged.
    pub fn set_arrival_rate(
        &mut self,
        station: StationId,
        per_hour: u32,
    ) -> Result<(), ConfigError> {
        let stations = self.arrival_rates.len();
        if station.index() >= stations {
            return Err(ConfigError::OutOfRange {
                field: "station",
                value: u64::from(station.0),
                min: 0,
                max: stations.saturating_sub(1) as u64,
            });
        }
        if per_hour > TICKS_PER_HOUR {
            return Err(ConfigError::ArrivalRateTooHigh { per_hour });
        }
        self.arrival_rates[station.index()] = per_hour;
        self.arrival_tables[station.index()] = PoissonTable::new(per_hour);
        Ok(())
    }

    /// Change the scanner misread rate from the next scan on (the hook for
    /// scanner-drift faults, CP2). Each parcel's scan draws are keyed on the
    /// parcel, so the change affects only the parcels scanned while it holds.
    ///
    /// # Errors
    /// [`ConfigError::OutOfRange`] above 1,000,000 ppm; the sim is unchanged.
    pub fn set_scanner_error_rate(&mut self, ppm: u32) -> Result<(), ConfigError> {
        if ppm > 1_000_000 {
            return Err(ConfigError::OutOfRange {
                field: "scanner.error_rate_ppm",
                value: u64::from(ppm),
                min: 0,
                max: 1_000_000,
            });
        }
        self.scanner_ppm = ppm;
        Ok(())
    }

    /// Advance one tick, appending the tick's events to `out` (which is not
    /// cleared, so a caller can accumulate a whole run).
    pub fn step(&mut self, out: &mut Vec<Event>) {
        let t = self.tick;
        self.apply_additions(t, out);
        self.apply_faults(t, out);
        self.generate_arrivals(t, out);
        self.generate_surge_arrivals(t, out);
        self.advance_timers(t, out);
        self.assign_work(t, out);
        self.plan_and_move(t, out);
        self.resolve_goals(t, out);
        self.attribute_waits(t, out);
        self.watchdog(t, out);
        self.tick = t + 1;
    }

    /// Aggregate counters. `queued` and `in_flight` are counted from the
    /// world, not tracked, so the conservation identity is a real check.
    #[must_use]
    pub fn stats(&self) -> Stats {
        let queued = self
            .world
            .stations
            .queue
            .iter()
            .map(|q| q.len() as u64)
            .sum();
        // A robot's parcel is on board from `Loading` on (a robot still
        // driving to the station has only claimed it), whatever the robot's
        // state: a failed robot can be carrying one.
        let phases = &self.world.parcels.phase;
        let in_flight = self
            .world
            .robots
            .parcel
            .iter()
            .flatten()
            .filter(|p| !matches!(phases[p.index()], Phase::Queued | Phase::AwaitingRobot))
            .count() as u64;
        Stats {
            tick: self.tick,
            arrived: self.arrived,
            queued,
            in_flight,
            delivered: self.delivered,
            mis_sorted: self.mis_sorted,
        }
    }

    // ---- tick phases -----------------------------------------------------

    /// Phase 2: a Poisson count per station, then one Zipf destination per
    /// parcel, all from that station's own streams so stations never perturb
    /// each other. Exactly one arrival draw per station per tick keeps the
    /// arrival stream's position a function of the tick alone, and each
    /// destination is keyed on the parcel's ordinal at its station.
    fn generate_arrivals(&mut self, t: Tick, out: &mut Vec<Event>) {
        for s in 0..self.world.stations.cell.len() {
            let count = self.arrival_tables[s].sample(&mut self.arrival_streams[s]);
            for _ in 0..count {
                let ordinal = self.world.stations.arrived[s];
                self.world.stations.arrived[s] += 1;
                self.push_parcel(t, station_id(s), ordinal, u64::from(ordinal), out);
            }
        }
    }

    /// A parcel arrives at `station`: draw its destination at `key` on the
    /// station's destination stream and queue it.
    fn push_parcel(
        &mut self,
        t: Tick,
        station: StationId,
        ordinal: u32,
        key: u64,
        out: &mut Vec<Event>,
    ) {
        let s = station.index();
        let destination = DestinationId(
            self.zipf
                .sample_keyed(&mut self.destination_streams[s], key),
        );
        let parcel = ParcelId(u32::try_from(self.world.parcels.len()).unwrap_or(u32::MAX));
        let p = &mut self.world.parcels;
        p.station.push(station);
        p.ordinal.push(ordinal);
        p.key.push(key);
        p.destination.push(destination);
        p.arrived.push(t);
        p.phase.push(Phase::Queued);
        p.phase_since.push(t);
        p.robot.push(None);
        p.target.push(None);
        p.chute.push(None);
        p.misread.push(false);
        self.world.stations.queue[s].push_back(parcel);
        self.arrived += 1;
        out.push(Event::ParcelArrived {
            tick: t,
            parcel,
            station,
            destination,
        });
        out.push(Event::ParcelPhase {
            tick: t,
            parcel,
            phase: Phase::Queued,
        });
    }

    /// Phase 3: bag swaps, then loading and dropping robots.
    fn advance_timers(&mut self, t: Tick, out: &mut Vec<Event>) {
        let chutes = &mut self.world.chutes;
        for c in 0..chutes.cell.len() {
            if chutes.state[c] != ChuteState::Swapping {
                continue;
            }
            chutes.timer[c] -= 1;
            if chutes.timer[c] == 0 {
                chutes.fill[c] = 0;
                chutes.state[c] = ChuteState::Open;
                out.push(Event::ChuteChanged {
                    tick: t,
                    chute: chute_id(c),
                    fill: 0,
                    state: ChuteState::Open,
                });
            }
        }

        for r in 0..self.world.robots.len() {
            let state = self.world.robots.state[r];
            if !matches!(state, RobotState::Loading | RobotState::Dropping) {
                continue;
            }
            let timer = &mut self.world.robots.timer[r];
            *timer = timer.saturating_sub(1);
            if *timer > 0 {
                continue;
            }
            let Some(parcel) = self.world.robots.parcel[r] else {
                debug_assert!(false, "{} is {state:?} without a parcel", robot_id(r));
                continue;
            };
            self.last_progress = t;
            if state == RobotState::Loading {
                self.finish_loading(t, r, parcel, out);
            } else {
                self.finish_drop(t, r, parcel, out);
            }
        }
    }

    /// Loaded: the scanner reads the parcel and the robot heads for the
    /// chute it names. The parcel stays in `Loading` until phase 6 decides
    /// between `Travelling` and `Blocked`, so no zero-length span appears.
    fn finish_loading(&mut self, t: Tick, r: usize, parcel: ParcelId, out: &mut Vec<Event>) {
        let station = self.world.parcels.station[parcel.index()];
        self.world.stations.inbound[station.index()] -= 1;
        let chute = self.scan(parcel);
        self.world.parcels.target[parcel.index()] = Some(chute);
        self.world.robots.goal[r] = Some(Goal::Chute(chute));
        self.world.robots.goal_since[r] = t;
        self.set_robot_state(t, r, RobotState::ToChute, out);
    }

    /// The scanner at the parcel's station reads it: usually the chute of
    /// its destination, but with probability `ppm` (the station's rate in
    /// force; see [`crate::faults`] for scanner drift) a uniformly chosen
    /// other chute. Both draws (misread? which chute?) are keyed on the
    /// parcel's draw key and always taken, so a parcel's scan outcome depends
    /// only on the parcel and the rate in force when it is scanned: a
    /// scanner-drift fault changes the parcels scanned during it and no
    /// others. A misread is recorded in `parcels.misread`, which is what
    /// makes the drop a mis-sort.
    fn scan(&mut self, parcel: ParcelId) -> ChuteId {
        let i = parcel.index();
        let station = self.world.parcels.station[i];
        let ppm = self.scanner_ppm_at(station);
        let p = &self.world.parcels;
        let correct = self.world.destination_chute[p.destination[i].index()];
        let key = p.key[i] * 2;
        let stream = &mut self.scanner_streams[station.index()];
        let chutes = self.world.chutes.cell.len() as u64;
        let roll = stream.keyed_below(key, 1_000_000);
        let offset = 1 + stream.keyed_below(key + 1, chutes.saturating_sub(1).max(1));
        if roll >= u64::from(ppm) || chutes < 2 {
            return correct;
        }
        self.world.parcels.misread[i] = true;
        chute_id(usize::try_from((u64::from(correct.0) + offset) % chutes).unwrap_or(0))
    }

    /// Dropped: the parcel is delivered; a full bag starts a swap.
    fn finish_drop(&mut self, t: Tick, r: usize, parcel: ParcelId, out: &mut Vec<Event>) {
        let Some(Goal::Chute(chute)) = self.world.robots.goal[r] else {
            debug_assert!(false, "{} dropped with no chute goal", robot_id(r));
            return;
        };
        let robot = robot_id(r);
        let c = chute.index();
        let chutes = &mut self.world.chutes;
        chutes.fill[c] += 1;
        chutes.incoming[c] -= 1;
        if chutes.fill[c] >= self.config.chutes.bag_capacity {
            chutes.timer[c] = self.config.chutes.bag_swap_ticks;
            // A jammed chute stays jammed; its swap starts when it clears.
            if chutes.state[c] == ChuteState::Jammed {
                self.faults.jam_prior[c] = ChuteState::Swapping;
            } else {
                chutes.state[c] = ChuteState::Swapping;
            }
        }
        let (fill, state) = (chutes.fill[c], chutes.state[c]);
        self.world.parcels.chute[parcel.index()] = Some(chute);
        self.set_phase(t, parcel, Phase::Dropped, out);
        self.delivered += 1;
        out.push(Event::ParcelDropped {
            tick: t,
            parcel,
            robot,
            chute,
        });
        let expected = self.world.parcels.destination[parcel.index()];
        if self.world.parcels.misread[parcel.index()] {
            self.mis_sorted += 1;
            out.push(Event::ParcelMisSorted {
                tick: t,
                parcel,
                chute,
                expected,
            });
        }
        out.push(Event::ChuteChanged {
            tick: t,
            chute,
            fill,
            state,
        });
        self.world.robots.parcel[r] = None;
        self.world.robots.goal[r] = None;
        self.world.robots.goal_since[r] = t;
        self.set_robot_state(t, r, RobotState::Idle, out);
    }

    /// Phases 5 and 6: plan, then apply moves and update parcel phases.
    fn plan_and_move(&mut self, t: Tick, out: &mut Vec<Event>) {
        let mut cut = Vec::new();
        let robots = &self.world.robots;
        self.plan_goal.clear();
        self.plan_movable.clear();
        self.plan_priority.clear();
        for r in 0..robots.len() {
            let goal = robots.goal[r];
            let state = robots.state[r];
            let at_chute = state == RobotState::ToChute
                && goal.is_some_and(|g| self.distances.get(g, robots.pos[r]) == 0);
            self.at_chute[r] = at_chute;
            // Robots waiting at a chute that cannot take their parcel stay
            // movable, with the lowest priority: through traffic can push
            // them aside and they come back, instead of their queue turning
            // into a wall across the aisle.
            let movable = matches!(
                state,
                RobotState::Idle | RobotState::ToInduction | RobotState::ToChute
            );
            // A goal a fault has cut off: the robot waits where it is
            // (unless pushed) instead of wandering, and records why.
            let cut_off = movable
                && self.faults.topology_active()
                && goal.is_some_and(|g| self.distances.get(g, robots.pos[r]) == UNREACHABLE);
            self.cut_by[r] = None;
            if cut_off {
                // Blamed below (blame needs `&mut self`).
                cut.push(r);
            }
            self.plan_movable.push(movable);
            self.plan_goal
                .push(if movable && !cut_off { goal } else { None });
            self.plan_priority.push(if at_chute {
                0
            } else {
                t.saturating_sub(robots.goal_since[r])
            });
        }
        for r in cut {
            if let Some(goal) = self.world.robots.goal[r] {
                let pos = self.world.robots.pos[r];
                self.cut_by[r] = Some(self.topology_blame(goal, pos, UNREACHABLE));
            }
        }
        let robots = &self.world.robots;
        let input = PlanInput {
            layout: &self.layout,
            distances: &self.distances,
            tick: t,
            pos: &robots.pos,
            goal: &self.plan_goal,
            movable: &self.plan_movable,
            priority: &self.plan_priority,
        };
        self.planner.plan(&input, &mut self.plan_out);
        #[cfg(debug_assertions)]
        check_plan(&self.layout, &self.world.robots.pos, &self.plan_out.next);

        for r in 0..self.world.robots.len() {
            let from = self.world.robots.pos[r];
            let to = self.plan_out.next[r];
            self.world.robots.pos[r] = to;
            self.detour[r] = None;
            let Some(goal) = self.plan_goal[r] else {
                // A carried parcel whose chute is cut off is blocked (one
                // that is waiting at the chute keeps waiting there).
                if self.cut_by[r].is_some()
                    && self.world.robots.state[r] == RobotState::ToChute
                    && let Some(parcel) = self.world.robots.parcel[r]
                    && self.world.parcels.phase[parcel.index()] != Phase::ChuteWait
                {
                    self.set_phase(t, parcel, Phase::Blocked, out);
                }
                continue;
            };
            let d_to = self.distances.get(goal, to);
            if goal != Goal::Park && d_to < self.distances.get(goal, from) {
                self.last_progress = t;
                // Progress on the faulted floor but not on the healthy one:
                // the robot is going round closed cells.
                if self.faults.topology_active()
                    && self.healthy.get(goal, to) >= self.healthy.get(goal, from)
                {
                    self.detour[r] = Some(from);
                }
            }
            if self.world.robots.state[r] == RobotState::ToChute
                && d_to != 0
                && let Some(parcel) = self.world.robots.parcel[r]
                && self.world.parcels.phase[parcel.index()] != Phase::ChuteWait
            {
                // Arrivals (d_to == 0) are left to phase 7, which moves the
                // parcel to `ChuteWait` without an empty `Travelling` span.
                // A parcel already in `ChuteWait` stays there while its
                // robot is pushed aside and returns: it is still waiting for
                // the chute.
                let phase = if self.plan_out.cause[r].is_some() {
                    Phase::Blocked
                } else {
                    Phase::Travelling
                };
                self.set_phase(t, parcel, phase, out);
            }
        }
    }

    /// Phase 7: robots that reached their goal start loading or dropping.
    fn resolve_goals(&mut self, t: Tick, out: &mut Vec<Event>) {
        for r in 0..self.world.robots.len() {
            let state = self.world.robots.state[r];
            let (Some(goal), Some(parcel)) =
                (self.world.robots.goal[r], self.world.robots.parcel[r])
            else {
                continue;
            };
            if self.distances.get(goal, self.world.robots.pos[r]) != 0 {
                continue;
            }
            match (state, goal) {
                (RobotState::ToInduction, Goal::Station(s)) => {
                    let queue = &mut self.world.stations.queue[s.index()];
                    if let Some(i) = queue.iter().position(|&q| q == parcel) {
                        queue.remove(i);
                    }
                    self.world.robots.timer[r] = self.config.timing.load_ticks;
                    self.world.robots.goal_since[r] = t;
                    self.last_progress = t;
                    self.set_phase(t, parcel, Phase::Loading, out);
                    self.set_robot_state(t, r, RobotState::Loading, out);
                }
                (RobotState::ToChute, Goal::Chute(c)) => {
                    self.set_phase(t, parcel, Phase::ChuteWait, out);
                    if self.chute_accepts(c) {
                        self.world.chutes.incoming[c.index()] += 1;
                        self.world.robots.timer[r] = self.config.timing.drop_ticks;
                        self.world.robots.goal_since[r] = t;
                        self.last_progress = t;
                        self.set_robot_state(t, r, RobotState::Dropping, out);
                    }
                }
                _ => {}
            }
        }
    }

    /// Phase 9: fire `Stalled` if there is work but nothing has moved it on
    /// for `watchdog_ticks`.
    fn watchdog(&mut self, t: Tick, out: &mut Vec<Event>) {
        let busy = self.world.robots.state.iter().any(|s| {
            matches!(
                s,
                RobotState::ToInduction
                    | RobotState::Loading
                    | RobotState::ToChute
                    | RobotState::Dropping
            )
        }) || self.world.stations.queue.iter().any(|q| !q.is_empty());
        if !busy {
            self.last_progress = t;
            return;
        }
        let quiet_since = self.last_progress.max(self.last_stall);
        if t - quiet_since >= self.config.planner.watchdog_ticks {
            self.last_stall = t;
            out.push(Event::Stalled {
                tick: t,
                since: self.last_progress,
            });
        }
    }

    // ---- helpers ---------------------------------------------------------

    /// Can a robot start dropping into chute `c` now?
    fn chute_accepts(&self, c: ChuteId) -> bool {
        let ch = &self.world.chutes;
        let i = c.index();
        ch.state[i] == ChuteState::Open
            && ch.fill[i] + u32::from(ch.incoming[i]) < self.config.chutes.bag_capacity
    }

    /// Move a parcel to `phase`, emitting an event only on a real change so
    /// the phase events tile the lifetime without empty spans.
    fn set_phase(&mut self, t: Tick, parcel: ParcelId, phase: Phase, out: &mut Vec<Event>) {
        let i = parcel.index();
        if self.world.parcels.phase[i] == phase {
            return;
        }
        self.world.parcels.phase[i] = phase;
        self.world.parcels.phase_since[i] = t;
        out.push(Event::ParcelPhase {
            tick: t,
            parcel,
            phase,
        });
    }

    fn set_robot_state(&mut self, t: Tick, r: usize, state: RobotState, out: &mut Vec<Event>) {
        if self.world.robots.state[r] == state {
            return;
        }
        self.world.robots.state[r] = state;
        out.push(Event::RobotState {
            tick: t,
            robot: robot_id(r),
            state,
        });
    }
}

fn robot_id(r: usize) -> RobotId {
    // Robot counts come from a u32 config field.
    #[allow(clippy::cast_possible_truncation)]
    RobotId(r as u32)
}

fn station_id(s: usize) -> StationId {
    // Station counts are u16 in the config.
    #[allow(clippy::cast_possible_truncation)]
    StationId(s as u16)
}

fn chute_id(c: usize) -> ChuteId {
    // Chute counts are bounded by u16 in the layout generator.
    #[allow(clippy::cast_possible_truncation)]
    ChuteId(c as u16)
}

/// Debug-build check of the planner contract (see [`crate::planner`]).
#[cfg(debug_assertions)]
fn check_plan(layout: &Layout, pos: &[CellIdx], next: &[CellIdx]) {
    let mut seen = vec![u32::MAX; layout.grid.len()];
    for (r, (&a, &b)) in pos.iter().zip(next).enumerate() {
        assert!(
            a == b || layout.grid.neighbours(a).any(|n| n == b),
            "robot {r} jumped from {a} to {b}"
        );
        #[allow(clippy::cast_possible_truncation)]
        let id = r as u32;
        assert!(
            seen[b as usize] == u32::MAX,
            "robots {} and {r} both planned into cell {b}",
            seen[b as usize]
        );
        seen[b as usize] = id;
    }
    // Swap check: if r moves a->b, no robot may move b->a.
    let mut at = vec![u32::MAX; layout.grid.len()];
    for (r, &a) in pos.iter().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        let id = r as u32;
        at[a as usize] = id;
    }
    for (r, (&a, &b)) in pos.iter().zip(next).enumerate() {
        let other = at[b as usize];
        if a != b && other != u32::MAX && other as usize != r {
            assert!(
                next[other as usize] != a,
                "robots {r} and {other} swapped cells"
            );
        }
    }
}
