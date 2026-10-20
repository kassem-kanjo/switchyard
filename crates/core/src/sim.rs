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
//!    up to [`MAX_INBOUND_PER_STATION`]. Idle robots left without work head
//!    for the parking lane (throttling, see below).
//! 5. Plan moves with the [`Planner`] (guided PIBT by default).
//! 6. Apply moves; carried parcels switch between `Travelling` and
//!    `Blocked`.
//! 7. Resolve arrivals at goals: start loads, and start drops at chutes that
//!    can take a parcel.
//! 8. Attribute waits: every robot with work (heading to a station or a
//!    chute) that did not get closer to its goal gets a `RobotWaited` with
//!    its direct cause (CP1 sets `root = direct`; CP2 walks the wait-for
//!    graph). Idle robots driving to parking are not recorded.
//! 9. Watchdog: if robots have work but none has progressed for
//!    `planner.watchdog_ticks`, emit `Stalled`.
//! 10. Advance the tick. The state hash is computed on demand from the
//!     canonical state ([`Sim::state_hash`]).
//!
//! # Throttling
//! Admission control is twofold, both deliberately simple (ADR 0003;
//! research §1 "agent disabling"): a station accepts at most
//! [`MAX_INBOUND_PER_STATION`] robots, which caps how many robots crowd the
//! induction area; and every idle robot that phase 4 could not give work to
//! is surplus to the floor's current demand, so instead of loitering in the
//! sort field (where PIBT would keep pushing it around) it drives to the
//! perimeter parking lane ([`crate::layout`]) while there is a free parking
//! spot. It is reassigned from wherever it is as soon as work appears.

use std::collections::VecDeque;

use crate::arrivals::{PoissonTable, ZipfTable};
use crate::config::{ChuteMapping, ConfigError, ScenarioConfig};
use crate::events::{ChuteState, Event, WaitCause};
use crate::guidance::DistanceTables;
use crate::hash::Fnv1a;
use crate::ids::{ChuteId, DestinationId, ParcelId, RobotId, StationId};
use crate::layout::{self, Layout};
use crate::pibt::PibtPlanner;
use crate::planner::{PlanInput, PlanOutput, Planner};
use crate::rng::{Stream, StreamId};
use crate::world::{Goal, Phase, RobotState, World};
use crate::{TICKS_PER_HOUR, Tick};

/// Robots that may be heading to (or loading at) one station at once.
///
/// The station's throughput is at most `inbound / (trip + load)` parcels per
/// tick, and a robot's trip to the station is 30-60 ticks on the medium and
/// large floors, so keeping a station busy at 800 parcels/h needs about five
/// robots in its pipeline (Little's law). With 2 (the V0 value) medium
/// delivered 20 parcels/h per robot with a growing backlog; 6 keeps every
/// preset level with demand while capping the crowd around the induction
/// cell (measured: 4 left a backlog on `large`, 8 gained nothing).
pub const MAX_INBOUND_PER_STATION: u16 = 6;

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

/// A flat, allocation-reusing view of the sim for the viewer (ARCHITECTURE
/// §8). The buffers map directly onto typed arrays in the WASM facade.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// The tick this snapshot was taken at.
    pub tick: Tick,
    /// Interleaved robot coordinates: `[x0, y0, x1, y1, ...]`.
    pub robot_xy: Vec<u16>,
    /// Robot states as [`RobotState`] `u8` codes, one per robot.
    pub robot_state: Vec<u8>,
    /// Parcels in each chute's bag, saturated at `u16::MAX`.
    pub chute_fill: Vec<u16>,
    /// Parcels waiting at each station, saturated at `u16::MAX`.
    pub station_queue: Vec<u16>,
}

/// A running simulation.
#[derive(Debug)]
pub struct Sim {
    config: ScenarioConfig,
    layout: Layout,
    distances: DistanceTables,
    world: World,
    tick: Tick,
    arrival_tables: Vec<PoissonTable>,
    arrival_streams: Vec<Stream>,
    zipf: ZipfTable,
    destination_streams: Vec<Stream>,
    scanner_streams: Vec<Stream>,
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
        world.destination_chute = map_destinations(
            usize::from(destinations),
            config.chutes.mapping,
            &layout,
            &distances,
        );

        let stations = layout.stations.len();
        world.stations.cell.clone_from(&layout.stations);
        world.stations.queue = vec![VecDeque::new(); stations];
        world.stations.inbound = vec![0; stations];

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
            arrival_tables: vec![PoissonTable::new(rate); stations],
            arrival_streams: streams(StreamId::arrivals),
            zipf: ZipfTable::new(destinations, config.arrivals.zipf_s_milli),
            destination_streams: streams(StreamId::destinations),
            scanner_streams: streams(StreamId::scanner),
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
    /// the tick's wait-for edges; they match its `RobotWaited` events.
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
    /// # Panics
    /// If `station` does not exist.
    pub fn set_arrival_rate(&mut self, station: StationId, per_hour: u32) {
        self.arrival_tables[station.index()] = PoissonTable::new(per_hour.min(TICKS_PER_HOUR));
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

    /// Fill `snap` with the current state, reusing its buffers.
    pub fn snapshot(&self, snap: &mut Snapshot) {
        let g = &self.layout.grid;
        snap.tick = self.tick;
        snap.robot_xy.clear();
        for &c in &self.world.robots.pos {
            let p = g.pos(c);
            snap.robot_xy.push(p.x);
            snap.robot_xy.push(p.y);
        }
        snap.robot_state.clear();
        snap.robot_state
            .extend(self.world.robots.state.iter().map(|&s| s as u8));
        snap.chute_fill.clear();
        snap.chute_fill.extend(
            self.world
                .chutes
                .fill
                .iter()
                .map(|&f| u16::try_from(f).unwrap_or(u16::MAX)),
        );
        snap.station_queue.clear();
        snap.station_queue.extend(
            self.world
                .stations
                .queue
                .iter()
                .map(|q| u16::try_from(q.len()).unwrap_or(u16::MAX)),
        );
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

    /// FNV-1a hash of a canonical encoding of the complete sim state: tick,
    /// every entity column in id order, counters and the position of every
    /// random stream (ADR 0002 §5). Equal hashes across platforms and between
    /// native and WASM are the determinism check.
    #[must_use]
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv1a::new();
        h.write(b"switchyard-state-v1");
        h.write_u32(self.tick);
        h.write_u64(self.arrived);
        h.write_u64(self.delivered);
        h.write_u64(self.mis_sorted);
        h.write_u32(self.last_progress);
        h.write_u32(self.last_stall);

        let r = &self.world.robots;
        h.write_u64(r.len() as u64);
        for i in 0..r.len() {
            h.write_u32(r.pos[i]);
            h.write_u8(r.state[i] as u8);
            h.write_opt_u32(r.parcel[i].map(|p| p.0));
            match r.goal[i] {
                None => h.write_u8(0),
                Some(Goal::Station(s)) => {
                    h.write_u8(1);
                    h.write_u16(s.0);
                }
                Some(Goal::Chute(c)) => {
                    h.write_u8(2);
                    h.write_u16(c.0);
                }
                Some(Goal::Park) => h.write_u8(3),
            }
            h.write_u16(r.timer[i]);
            h.write_u16(r.waited[i]);
            h.write_u32(r.goal_since[i]);
        }

        let p = &self.world.parcels;
        h.write_u64(p.len() as u64);
        for i in 0..p.len() {
            h.write_u16(p.station[i].0);
            h.write_u16(p.destination[i].0);
            h.write_u32(p.arrived[i]);
            h.write_u8(p.phase[i] as u8);
            h.write_u32(p.phase_since[i]);
            h.write_opt_u32(p.robot[i].map(|r| r.0));
            h.write_opt_u32(p.target[i].map(|c| u32::from(c.0)));
            h.write_opt_u32(p.chute[i].map(|c| u32::from(c.0)));
        }

        let s = &self.world.stations;
        for i in 0..s.cell.len() {
            h.write_u32(s.cell[i]);
            h.write_u16(s.inbound[i]);
            h.write_u64(s.queue[i].len() as u64);
            for q in &s.queue[i] {
                h.write_u32(q.0);
            }
        }

        let c = &self.world.chutes;
        for i in 0..c.cell.len() {
            h.write_u32(c.cell[i]);
            h.write_u32(c.fill[i]);
            h.write_u8(c.state[i] as u8);
            h.write_u32(c.timer[i]);
            h.write_u16(c.incoming[i]);
        }
        h.write_u64(self.world.destination_chute.len() as u64);
        for d in &self.world.destination_chute {
            h.write_u16(d.0);
        }

        for st in self
            .arrival_streams
            .iter()
            .chain(&self.destination_streams)
            .chain(&self.scanner_streams)
        {
            h.write_u128(st.word_pos());
        }
        self.planner.hash_state(&mut h);
        h.finish()
    }

    // ---- tick phases -----------------------------------------------------

    /// Phase 2: a Poisson count per station, then one Zipf destination per
    /// parcel, all from that station's own streams so stations never perturb
    /// each other. Exactly one arrival draw per station per tick keeps each
    /// stream's position a function of the tick alone.
    fn generate_arrivals(&mut self, t: Tick, out: &mut Vec<Event>) {
        for s in 0..self.world.stations.cell.len() {
            let count = self.arrival_tables[s].sample(&mut self.arrival_streams[s]);
            for _ in 0..count {
                let destination = DestinationId(self.zipf.sample(&mut self.destination_streams[s]));
                let station = station_id(s);
                let parcel = ParcelId(u32::try_from(self.world.parcels.len()).unwrap_or(u32::MAX));
                let p = &mut self.world.parcels;
                p.station.push(station);
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
        let chute = self.scan(station, self.world.parcels.destination[parcel.index()]);
        self.world.parcels.target[parcel.index()] = Some(chute);
        self.world.robots.goal[r] = Some(Goal::Chute(chute));
        self.world.robots.goal_since[r] = t;
        self.set_robot_state(t, r, RobotState::ToChute, out);
    }

    /// The scanner at `station` reads a parcel for `destination`: usually
    /// the destination's chute, but with probability `error_rate_ppm` a
    /// uniformly chosen other chute. One draw per scan regardless of the
    /// rate, so changing the rate (a scanner-drift fault) does not shift the
    /// stream for later parcels that are read correctly.
    fn scan(&mut self, station: StationId, destination: DestinationId) -> ChuteId {
        let correct = self.world.destination_chute[destination.index()];
        let stream = &mut self.scanner_streams[station.index()];
        let misread = stream.below(1_000_000) < u64::from(self.config.scanner.error_rate_ppm);
        let chutes = self.world.chutes.cell.len() as u64;
        if !misread || chutes < 2 {
            return correct;
        }
        let offset = 1 + stream.below(chutes - 1);
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
            while self.world.stations.inbound[s] < MAX_INBOUND_PER_STATION {
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
            let movable = !at_chute
                && matches!(
                    state,
                    RobotState::Idle | RobotState::ToInduction | RobotState::ToChute
                );
            self.plan_movable.push(movable);
            self.plan_goal.push(if movable { goal } else { None });
            self.plan_priority
                .push(t.saturating_sub(robots.goal_since[r]));
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
            {
                // Arrivals (d_to == 0) are left to phase 7, which moves the
                // parcel to `ChuteWait` without an empty `Travelling` span.
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
            let cause = match (goal, self.plan_out.cause[r]) {
                (Some(Goal::Chute(c)), _)
                    if self.at_chute[r] && self.world.robots.state[r] == RobotState::ToChute =>
                {
                    Some(WaitCause::ChuteUnavailable(c))
                }
                (Some(Goal::Station(s)), Some(WaitCause::BlockedBy(b)))
                    if self.world.robots.pos[b.index()] == self.world.stations.cell[s.index()] =>
                {
                    Some(WaitCause::StationBusy(s))
                }
                // Robots driving to parking have no parcel and delay nothing:
                // their waits are not recorded. A robot with work blocked by
                // one still records `BlockedBy(it)`, so the chain ends there.
                (Some(Goal::Park) | None, _) => None,
                (_, cause) => cause,
            };
            self.waits[r] = cause;
            let waited = &mut self.world.robots.waited[r];
            let Some(direct) = cause else {
                *waited = 0;
                continue;
            };
            *waited = waited.saturating_add(1);
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

/// The chute for each of `n` destinations under `mapping`. Destination ids
/// are popularity ranks (Zipf weight falls with the id), so "most popular"
/// means "lowest id".
fn map_destinations(
    n: usize,
    mapping: ChuteMapping,
    layout: &Layout,
    distances: &DistanceTables,
) -> Vec<ChuteId> {
    let chutes = layout.chutes.len();
    let mut order: Vec<ChuteId> = (0..chutes).map(chute_id).collect();
    if mapping == ChuteMapping::PopularityNearInduction {
        // Sum of guided distances from every induction cell; ties by id.
        order.sort_by_key(|&c| {
            let total: u64 = layout
                .stations
                .iter()
                .map(|&s| u64::from(distances.get(Goal::Chute(c), s)))
                .sum();
            (total, c)
        });
    }
    (0..n).map(|d| order[d % chutes]).collect()
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
