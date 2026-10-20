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
//! 1. Apply faults that start or end at this tick *(CP2; none yet)*.
//! 2. Generate arrivals: a Poisson count per station, then a Zipf
//!    destination per parcel, each station on its own streams.
//! 3. Advance timers: bag swaps finish; loads finish (the scanner reads the
//!    parcel and the robot heads for its chute); drops finish (the parcel is
//!    delivered, a full bag starts a swap).
//! 4. Assign work: each station, in id order, hands its oldest unclaimed
//!    parcels to the nearest idle robots (by guided distance, ties by id),
//!    up to `fleet.max_inbound_per_station`. Idle robots left without work head
//!    for the parking lane (throttling, see below).
//! 5. Plan moves with the [`Planner`] (guided PIBT by default).
//! 6. Apply moves; carried parcels switch between `Travelling` and
//!    `Blocked`.
//! 7. Resolve arrivals at goals: start loads, and start drops at chutes that
//!    can take a parcel.
//! 8. Attribute waits: every robot's direct cause goes into the tick's
//!    wait-for record ([`Sim::wait_for`]); robots with work (heading to a
//!    station or a chute) also get a `RobotWaited` (CP1 sets
//!    `root = direct`; CP2 walks the wait-for graph), and a `RobotStalled`
//!    when they have waited `planner.watchdog_ticks` ticks in a row.
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

mod mapping;
mod snapshot;
mod state_hash;

pub use snapshot::Snapshot;

use crate::arrivals::{PoissonTable, ZipfTable};
use crate::config::{ConfigError, ScenarioConfig};
use crate::events::{ChuteState, Event, WaitCause};
use crate::guidance::DistanceTables;
use crate::ids::{ChuteId, DestinationId, ParcelId, RobotId, StationId};
use crate::layout::{self, Layout};
use crate::pibt::PibtPlanner;
use crate::planner::{PlanInput, PlanOutput, Planner};
use crate::rng::{Stream, StreamId};
use crate::world::{Goal, Phase, RobotState, Robots, Stations, World};
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
    /// Parcels on a robot (phases `Loading` through `ChuteWait`).
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
#[derive(Debug)]
pub struct Sim {
    config: ScenarioConfig,
    layout: Layout,
    distances: DistanceTables,
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
    // Per-tick scratch buffers, reused to avoid allocation in the loop.
    plan_goal: Vec<Option<Goal>>,
    plan_movable: Vec<bool>,
    plan_priority: Vec<u32>,
    plan_out: PlanOutput,
    /// Robots that started the tick at their chute, unable to drop.
    at_chute: Vec<bool>,
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
        let distances = DistanceTables::build(&layout, config.guidance.against_flow_cost);

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

        Ok(Self {
            config: config.clone(),
            layout,
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
            arrived: 0,
            delivered: 0,
            mis_sorted: 0,
            last_progress: 0,
            last_stall: 0,
            waits: vec![None; n],
            plan_goal: Vec::with_capacity(n),
            plan_movable: Vec::with_capacity(n),
            plan_priority: Vec::with_capacity(n),
            plan_out: PlanOutput::default(),
            at_chute: vec![false; n],
            idle: Vec::with_capacity(n),
        })
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

    /// The guided distance tables the planner uses.
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
        // 1. Faults: none until CP2.
        self.generate_arrivals(t, out);
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
        let in_flight = self
            .world
            .robots
            .state
            .iter()
            .zip(&self.world.robots.parcel)
            .filter(|(s, p)| {
                p.is_some()
                    && matches!(
                        s,
                        RobotState::Loading | RobotState::ToChute | RobotState::Dropping
                    )
            })
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
                let destination = DestinationId(
                    self.zipf
                        .sample_keyed(&mut self.destination_streams[s], u64::from(ordinal)),
                );
                let station = station_id(s);
                let parcel = ParcelId(u32::try_from(self.world.parcels.len()).unwrap_or(u32::MAX));
                let p = &mut self.world.parcels;
                p.station.push(station);
                p.ordinal.push(ordinal);
                p.destination.push(destination);
                p.arrived.push(t);
                p.phase.push(Phase::Queued);
                p.phase_since.push(t);
                p.robot.push(None);
                p.target.push(None);
                p.chute.push(None);
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
        }
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
    /// its destination, but with probability `scanner_ppm` a uniformly chosen
    /// other chute. Both draws (misread? which chute?) are keyed on the
    /// parcel's ordinal at its station and always taken, so a parcel's scan
    /// outcome depends only on the parcel and the rate in force when it is
    /// scanned: a scanner-drift fault changes the parcels scanned during it
    /// and no others.
    fn scan(&mut self, parcel: ParcelId) -> ChuteId {
        let i = parcel.index();
        let p = &self.world.parcels;
        let correct = self.world.destination_chute[p.destination[i].index()];
        let key = u64::from(p.ordinal[i]) * 2;
        let stream = &mut self.scanner_streams[p.station[i].index()];
        let chutes = self.world.chutes.cell.len() as u64;
        let roll = stream.keyed_below(key, 1_000_000);
        let offset = 1 + stream.keyed_below(key + 1, chutes.saturating_sub(1).max(1));
        if roll >= u64::from(self.scanner_ppm) || chutes < 2 {
            return correct;
        }
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
            chutes.state[c] = ChuteState::Swapping;
            chutes.timer[c] = self.config.chutes.bag_swap_ticks;
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
        if self.world.destination_chute[expected.index()] != chute {
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

    /// Phase 4: stations hand out work, then surplus idle robots park.
    fn assign_work(&mut self, t: Tick, out: &mut Vec<Event>) {
        self.idle.clear();
        for r in self.world.robots.ids() {
            if self.world.robots.state[r.index()] == RobotState::Idle {
                self.idle.push(r.0);
            }
        }

        for s in 0..self.world.stations.cell.len() {
            let station = station_id(s);
            while self.world.stations.inbound[s] < self.config.fleet.max_inbound_per_station {
                let Some(&parcel) = self.world.stations.queue[s]
                    .iter()
                    .find(|p| self.world.parcels.robot[p.index()].is_none())
                else {
                    break;
                };
                let goal = Goal::Station(station);
                let Some(i) = (0..self.idle.len()).min_by_key(|&i| {
                    let r = self.idle[i];
                    (
                        self.distances.get(goal, self.world.robots.pos[r as usize]),
                        r,
                    )
                }) else {
                    break;
                };
                let r = self.idle.swap_remove(i) as usize;
                let robot = robot_id(r);
                self.world.stations.inbound[s] += 1;
                self.world.parcels.robot[parcel.index()] = Some(robot);
                self.world.robots.parcel[r] = Some(parcel);
                self.world.robots.goal[r] = Some(goal);
                self.world.robots.goal_since[r] = t;
                out.push(Event::ParcelAssigned {
                    tick: t,
                    parcel,
                    robot,
                    station,
                });
                self.set_phase(t, parcel, Phase::AwaitingRobot, out);
                self.set_robot_state(t, r, RobotState::ToInduction, out);
            }
        }

        // Throttling: surplus idle robots park while spots remain.
        let robots = &mut self.world.robots;
        let mut parking = robots
            .goal
            .iter()
            .filter(|&&g| g == Some(Goal::Park))
            .count();
        for r in 0..robots.len() {
            if parking >= self.layout.parking.len() {
                break;
            }
            if robots.state[r] == RobotState::Idle && robots.goal[r].is_none() {
                robots.goal[r] = Some(Goal::Park);
                robots.goal_since[r] = t;
                parking += 1;
            }
        }
    }

    /// Phases 5 and 6: plan, then apply moves and update parcel phases.
    fn plan_and_move(&mut self, t: Tick, out: &mut Vec<Event>) {
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
            self.plan_movable.push(movable);
            self.plan_goal.push(if movable { goal } else { None });
            self.plan_priority.push(if at_chute {
                0
            } else {
                t.saturating_sub(robots.goal_since[r])
            });
        }
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
            let Some(goal) = self.plan_goal[r] else {
                continue;
            };
            let d_to = self.distances.get(goal, to);
            if goal != Goal::Park && d_to < self.distances.get(goal, from) {
                self.last_progress = t;
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

    /// Phase 8: record every robot's direct wait cause and emit
    /// `RobotWaited`. The planner supplies robot-to-robot causes; the sim
    /// refines them with what it knows about stations and chutes.
    fn attribute_waits(&mut self, t: Tick, out: &mut Vec<Event>) {
        for r in 0..self.world.robots.len() {
            let goal = self.world.robots.goal[r];
            // Still at its chute (not pushed aside this tick) and unable to drop.
            let waiting = self.at_chute[r]
                && self.world.robots.state[r] == RobotState::ToChute
                && self.plan_out.next[r] == self.world.robots.pos[r]
                && goal.is_some_and(|g| self.distances.get(g, self.world.robots.pos[r]) == 0);
            let cause = refine_cause(
                goal,
                self.plan_out.cause[r],
                waiting,
                &self.world.robots,
                &self.world.stations,
            );
            // Every robot's edge is kept, so chains can be walked through
            // idle and parked robots (`wait_for`).
            self.waits[r] = cause;
            let waited = &mut self.world.robots.waited[r];
            let Some(direct) = cause else {
                *waited = 0;
                continue;
            };
            *waited = waited.saturating_add(1);
            let waited = *waited;
            // Events only for robots with work: idle and parking robots carry
            // no parcel and delay nothing on their own.
            if !matches!(goal, Some(Goal::Station(_) | Goal::Chute(_))) {
                continue;
            }
            if waited == self.config.planner.watchdog_ticks {
                out.push(Event::RobotStalled {
                    tick: t,
                    robot: robot_id(r),
                    waited,
                });
            }
            out.push(Event::RobotWaited {
                tick: t,
                robot: robot_id(r),
                direct,
                // CP2 walks the wait-for graph to the root.
                root: direct,
            });
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

/// Refine a planner's robot-to-robot cause with what only the sim knows
/// (ARCHITECTURE §4): a robot waiting at a chute that cannot take its parcel
/// waits on the chute; a robot blocked by the robot *loading* on its target
/// station's cell waits on the station. A robot stuck on that cell after
/// loading is just another robot: the edge stays `BlockedBy`, so the walk
/// continues to whatever blocks it.
fn refine_cause(
    goal: Option<Goal>,
    planner: Option<WaitCause>,
    waiting_at_chute: bool,
    robots: &Robots,
    stations: &Stations,
) -> Option<WaitCause> {
    match (goal, planner) {
        (Some(Goal::Chute(c)), _) if waiting_at_chute => Some(WaitCause::ChuteUnavailable(c)),
        (Some(Goal::Station(s)), Some(WaitCause::BlockedBy(b)))
            if robots.pos[b.index()] == stations.cell[s.index()]
                && robots.state[b.index()] == RobotState::Loading =>
        {
            Some(WaitCause::StationBusy(s))
        }
        (_, cause) => cause,
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
fn check_plan(layout: &Layout, pos: &[crate::grid::CellIdx], next: &[crate::grid::CellIdx]) {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Robot 1 stands on station 0's cell (cell 7); robot 0 heads there.
    fn floor(blocker: RobotState) -> (Robots, Stations) {
        let robots = Robots {
            pos: vec![8, 7],
            state: vec![RobotState::ToInduction, blocker],
            ..Robots::default()
        };
        let stations = Stations {
            cell: vec![7],
            ..Stations::default()
        };
        (robots, stations)
    }

    #[test]
    fn station_busy_only_while_the_blocker_is_loading() {
        let goal = Some(Goal::Station(StationId(0)));
        let blocked = Some(WaitCause::BlockedBy(RobotId(1)));
        let (robots, stations) = floor(RobotState::Loading);
        assert_eq!(
            refine_cause(goal, blocked, false, &robots, &stations),
            Some(WaitCause::StationBusy(StationId(0)))
        );
        // Loaded and itself stuck on the station cell: the real blocker is
        // further down the chain, so keep the robot edge.
        let (robots, stations) = floor(RobotState::ToChute);
        assert_eq!(
            refine_cause(goal, blocked, false, &robots, &stations),
            blocked
        );
    }

    #[test]
    fn waiting_at_an_unavailable_chute_is_chute_unavailable() {
        let (robots, stations) = floor(RobotState::Loading);
        let goal = Some(Goal::Chute(ChuteId(3)));
        assert_eq!(
            refine_cause(goal, None, true, &robots, &stations),
            Some(WaitCause::ChuteUnavailable(ChuteId(3)))
        );
        // Pushed off the chute while waiting: the robot edge is the cause.
        let yielded = Some(WaitCause::Yielded(RobotId(1)));
        assert_eq!(
            refine_cause(goal, yielded, false, &robots, &stations),
            yielded
        );
    }
}
