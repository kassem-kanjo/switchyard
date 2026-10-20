//! The tick loop.
//!
//! # Tick order
//!
//! Every call to [`Sim::step`] runs these phases in this fixed order
//! (ARCHITECTURE §3). Within a phase, entities are processed in ascending id
//! order, so the outcome never depends on container iteration order
//! (ADR 0002 §4).
//!
//! 1. Apply faults that start or end at this tick *(CP2; none in V0)*.
//! 2. Generate arrivals: one Bernoulli draw per station from its own stream.
//! 3. Advance station and chute timers: finish loads and drops.
//! 4. Assign idle robots to the oldest unclaimed parcel at the nearest
//!    station with spare inbound capacity.
//! 5. Plan moves (the [`Planner`]; greedy in V0, guided PIBT from CP1).
//! 6. Apply moves; carried parcels switch between `Travelling` and `Blocked`.
//! 7. Resolve arrivals at goals: start loads and drops.
//! 8. Attribute causes of waits (V0: `root = direct`; CP2 walks the
//!    wait-for graph).
//! 9. Emit events (appended as each phase runs, so the stream follows this
//!    order).
//! 10. Advance the tick. The state hash is computed on demand from the
//!     canonical state ([`Sim::state_hash`]).

use std::collections::VecDeque;

use crate::config::{ConfigError, ScenarioConfig};
use crate::events::{ChuteState, Event, WaitCause};
use crate::hash::Fnv1a;
use crate::ids::{ChuteId, DestinationId, ParcelId, RobotId, StationId};
use crate::layout::{self, Layout};
use crate::planner::{DistanceTables, GreedyPlanner, PlanInput, PlanOutput, Planner};
use crate::rng::{Stream, StreamId};
use crate::world::{Goal, Phase, RobotState, World};
use crate::{TICKS_PER_HOUR, Tick};

/// Robots that may be heading to (or loading at) one station at once. More
/// would only crowd the induction cell, whose neighbours are the only way in.
pub const MAX_INBOUND_PER_STATION: u16 = 2;

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
    /// Parcels dropped into a chute.
    pub delivered: u64,
    /// Parcels dropped into the wrong chute (always 0 before CP2).
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
    arrival_streams: Vec<Stream>,
    destination_streams: Vec<Stream>,
    planner: Box<dyn Planner>,
    // Counters, kept incrementally (Stats checks them against the world).
    arrived: u64,
    delivered: u64,
    mis_sorted: u64,
    // Per-tick scratch buffers, reused to avoid allocation in the loop.
    plan_goal: Vec<Option<Goal>>,
    plan_idle: Vec<bool>,
    plan_out: PlanOutput,
}

impl Sim {
    /// Build a simulation from a scenario, using the V0 greedy planner.
    ///
    /// # Errors
    /// Any [`ConfigError`] from validation or layout generation, or
    /// [`ConfigError::BadRobotCount`] if the robots do not fit.
    pub fn new(config: &ScenarioConfig) -> Result<Self, ConfigError> {
        Self::with_planner(config, Box::new(GreedyPlanner::new(config.seed)))
    }

    /// Build a simulation with a specific planner (CP1 plugs guided PIBT in
    /// here; tests can plug in a scripted one).
    ///
    /// # Errors
    /// As [`Sim::new`].
    pub fn with_planner(
        config: &ScenarioConfig,
        planner: Box<dyn Planner>,
    ) -> Result<Self, ConfigError> {
        config.validate()?;
        let layout = layout::generate(&config.floor)?;
        let distances = DistanceTables::build(&layout);

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

        world.chutes.cell.clone_from(&layout.chutes);
        // V0: chute i serves destination i.
        world.chutes.destination = (0..layout.chutes.len())
            .map(|i| DestinationId(u16::try_from(i).unwrap_or(u16::MAX)))
            .collect();
        world.chutes.fill = vec![0; layout.chutes.len()];

        let stations = layout.stations.len();
        world.stations.cell.clone_from(&layout.stations);
        world.stations.queue = vec![VecDeque::new(); stations];
        world.stations.inbound = vec![0; stations];

        let station_ids = 0..u16::try_from(stations).unwrap_or(u16::MAX);
        let arrival_streams = station_ids
            .clone()
            .map(|s| Stream::new(config.seed, StreamId::arrivals(s)))
            .collect();
        let destination_streams = station_ids
            .map(|s| Stream::new(config.seed, StreamId::destinations(s)))
            .collect();

        Ok(Self {
            config: config.clone(),
            layout,
            distances,
            world,
            tick: 0,
            arrival_streams,
            destination_streams,
            planner,
            arrived: 0,
            delivered: 0,
            mis_sorted: 0,
            plan_goal: Vec::with_capacity(n),
            plan_idle: Vec::with_capacity(n),
            plan_out: PlanOutput::default(),
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

    /// Advance one tick, appending the tick's events to `out` (which is not
    /// cleared, so a caller can accumulate a whole run).
    pub fn step(&mut self, out: &mut Vec<Event>) {
        let t = self.tick;
        // 1. Faults: none until CP2.
        self.generate_arrivals(t, out);
        self.advance_timers(t, out);
        self.assign_idle_robots(t, out);
        self.plan_and_move(t, out);
        self.resolve_goals(t, out);
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
        h.write(b"switchyard-state-v0");
        h.write_u32(self.tick);
        h.write_u64(self.arrived);
        h.write_u64(self.delivered);
        h.write_u64(self.mis_sorted);

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
            }
            h.write_u16(r.timer[i]);
            h.write_u16(r.waited[i]);
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
            h.write_u16(c.destination[i].0);
            h.write_u32(c.fill[i]);
        }

        for st in self.arrival_streams.iter().chain(&self.destination_streams) {
            h.write_u128(st.word_pos());
        }
        self.planner.hash_state(&mut h);
        h.finish()
    }

    // ---- tick phases -----------------------------------------------------

    /// Phase 2: at most one arrival per station per tick, drawn from that
    /// station's own streams so stations never perturb each other.
    fn generate_arrivals(&mut self, t: Tick, out: &mut Vec<Event>) {
        let rate = u64::from(self.config.arrivals.parcels_per_hour_per_station);
        let destinations = self.world.chutes.destination.len() as u64;
        for s in 0..self.world.stations.cell.len() {
            if !self.arrival_streams[s].chance(rate, u64::from(TICKS_PER_HOUR)) {
                continue;
            }
            // destinations <= u16::MAX by construction.
            #[allow(clippy::cast_possible_truncation)]
            let destination = DestinationId(self.destination_streams[s].below(destinations) as u16);
            #[allow(clippy::cast_possible_truncation)]
            let station = StationId(s as u16);
            let parcel = ParcelId(u32::try_from(self.world.parcels.len()).unwrap_or(u32::MAX));
            let p = &mut self.world.parcels;
            p.station.push(station);
            p.destination.push(destination);
            p.arrived.push(t);
            p.phase.push(Phase::Queued);
            p.phase_since.push(t);
            p.robot.push(None);
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

    /// Phase 3: count down loading and dropping robots.
    fn advance_timers(&mut self, t: Tick, out: &mut Vec<Event>) {
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
            let robot = robot_id(r);
            let Some(parcel) = self.world.robots.parcel[r] else {
                debug_assert!(false, "{robot} is {state:?} without a parcel");
                continue;
            };
            if state == RobotState::Loading {
                // Loaded: head for the chute serving the parcel's destination.
                // The parcel stays in `Loading` until phase 6 decides between
                // `Travelling` and `Blocked`, so no zero-length span appears.
                let station = self.world.parcels.station[parcel.index()];
                self.world.stations.inbound[station.index()] -= 1;
                let chute = self.chute_for(self.world.parcels.destination[parcel.index()]);
                self.world.robots.goal[r] = Some(Goal::Chute(chute));
                self.set_robot_state(t, r, RobotState::ToChute, out);
            } else {
                let chute = match self.world.robots.goal[r] {
                    Some(Goal::Chute(c)) => c,
                    _ => self.chute_for(self.world.parcels.destination[parcel.index()]),
                };
                self.world.chutes.fill[chute.index()] += 1;
                self.world.parcels.chute[parcel.index()] = Some(chute);
                self.set_phase(t, parcel, Phase::Dropped, out);
                self.delivered += 1;
                out.push(Event::ParcelDropped {
                    tick: t,
                    parcel,
                    robot,
                    chute,
                });
                out.push(Event::ChuteChanged {
                    tick: t,
                    chute,
                    fill: self.world.chutes.fill[chute.index()],
                    state: ChuteState::Open,
                });
                self.world.robots.parcel[r] = None;
                self.world.robots.goal[r] = None;
                self.set_robot_state(t, r, RobotState::Idle, out);
            }
        }
    }

    /// Phase 4: each idle robot, in id order, claims the oldest unclaimed
    /// parcel at the nearest station that has spare inbound capacity (ties go
    /// to the lower station id).
    fn assign_idle_robots(&mut self, t: Tick, out: &mut Vec<Event>) {
        for r in 0..self.world.robots.len() {
            if self.world.robots.state[r] != RobotState::Idle {
                continue;
            }
            let here = self.world.robots.pos[r];
            let mut best: Option<(u16, usize, ParcelId)> = None;
            for s in 0..self.world.stations.cell.len() {
                if self.world.stations.inbound[s] >= MAX_INBOUND_PER_STATION {
                    continue;
                }
                let Some(&parcel) = self.world.stations.queue[s]
                    .iter()
                    .find(|p| self.world.parcels.robot[p.index()].is_none())
                else {
                    continue;
                };
                #[allow(clippy::cast_possible_truncation)]
                let d = self.distances.get(Goal::Station(StationId(s as u16)), here);
                if best.is_none_or(|(bd, _, _)| d < bd) {
                    best = Some((d, s, parcel));
                }
            }
            let Some((_, s, parcel)) = best else {
                continue;
            };
            #[allow(clippy::cast_possible_truncation)]
            let station = StationId(s as u16);
            let robot = robot_id(r);
            self.world.stations.inbound[s] += 1;
            self.world.parcels.robot[parcel.index()] = Some(robot);
            self.world.robots.parcel[r] = Some(parcel);
            self.world.robots.goal[r] = Some(Goal::Station(station));
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

    /// Phases 5, 6 and 8: plan, apply moves, attribute waits.
    fn plan_and_move(&mut self, t: Tick, out: &mut Vec<Event>) {
        let robots = &self.world.robots;
        self.plan_goal.clear();
        self.plan_idle.clear();
        for r in 0..robots.len() {
            let moving = matches!(
                robots.state[r],
                RobotState::ToInduction | RobotState::ToChute
            );
            self.plan_goal
                .push(if moving { robots.goal[r] } else { None });
            self.plan_idle.push(robots.state[r] == RobotState::Idle);
        }
        let input = PlanInput {
            layout: &self.layout,
            distances: &self.distances,
            pos: &robots.pos,
            goal: &self.plan_goal,
            waited: &robots.waited,
            idle: &self.plan_idle,
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
            let at_goal = self.distances.get(goal, from) == 0;
            let moved = from != to;
            if moved || at_goal {
                self.world.robots.waited[r] = 0;
            } else {
                self.world.robots.waited[r] = self.world.robots.waited[r].saturating_add(1);
                if let Some(blocker) = self.plan_out.blocked_by[r] {
                    let direct = self.wait_cause(goal, blocker);
                    out.push(Event::RobotWaited {
                        tick: t,
                        robot: robot_id(r),
                        direct,
                        // CP2 walks the wait-for graph to the root.
                        root: direct,
                    });
                }
            }
            if self.world.robots.state[r] == RobotState::ToChute
                && let Some(parcel) = self.world.robots.parcel[r]
            {
                let phase = if moved || at_goal {
                    Phase::Travelling
                } else {
                    Phase::Blocked
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
            let here = self.world.robots.pos[r];
            if self.distances.get(goal, here) != 0 {
                continue;
            }
            match (state, goal) {
                (RobotState::ToInduction, Goal::Station(s)) => {
                    let queue = &mut self.world.stations.queue[s.index()];
                    if let Some(i) = queue.iter().position(|&q| q == parcel) {
                        queue.remove(i);
                    }
                    self.world.robots.timer[r] = self.config.timing.load_ticks;
                    self.set_phase(t, parcel, Phase::Loading, out);
                    self.set_robot_state(t, r, RobotState::Loading, out);
                }
                (RobotState::ToChute, Goal::Chute(_)) => {
                    self.world.robots.timer[r] = self.config.timing.drop_ticks;
                    self.set_phase(t, parcel, Phase::ChuteWait, out);
                    self.set_robot_state(t, r, RobotState::Dropping, out);
                }
                _ => {}
            }
        }
    }

    // ---- helpers ---------------------------------------------------------

    /// The chute serving `destination` (V0: the chute with the same index).
    fn chute_for(&self, destination: DestinationId) -> ChuteId {
        ChuteId(destination.0)
    }

    /// Classify a wait: a robot blocked by the robot loading at its target
    /// station is waiting on the station, not on traffic.
    fn wait_cause(&self, goal: Goal, blocker: RobotId) -> WaitCause {
        if let Goal::Station(s) = goal
            && self.world.robots.pos[blocker.index()] == self.world.stations.cell[s.index()]
        {
            return WaitCause::StationBusy(s);
        }
        WaitCause::BlockedBy(blocker)
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
