//! End-to-end invariants of the simulation.
//!
//! - Determinism (ADR 0002): same seed, same hash and same event stream;
//!   different seed, different hash; stations draw arrivals independently.
//! - No two robots ever share a cell, and no two robots swap cells, on the
//!   presets and on random small floors (proptest).
//! - Parcel conservation: arrived = queued + in-flight + delivered, checked
//!   against an independent count of parcel phases and the event stream.
//! - Parcel phases follow ARCHITECTURE §3 exactly.
//! - Causes: every `RobotWaited` matches the sim's wait-for record; bag
//!   swaps produce `ChuteUnavailable`; the watchdog reports stalls.
//! - Operations: scanner misreads, chute mapping strategies.
//! - Calibration: the medium preset sorts 25-50 parcels/h per robot.

use proptest::prelude::*;
use switchyard_core::config::{ChuteMapping, FloorConfig, ScenarioConfig};
use switchyard_core::events::{ChuteState, Event, WaitCause};
use switchyard_core::grid::Cell;
use switchyard_core::hash::Fnv1a;
use switchyard_core::ids::RobotId;
use switchyard_core::ids::{ChuteId, StationId};
use switchyard_core::planner::{PlanInput, PlanOutput, Planner};
use switchyard_core::presets::{MEDIUM_TOML, SMALL_TOML};
use switchyard_core::sim::Sim;
use switchyard_core::world::{Goal, Phase, RobotState};
use switchyard_core::{TICKS_PER_HOUR, Tick};

const TICKS: u32 = 2_000;

fn small(seed: u64) -> ScenarioConfig {
    let mut cfg = ScenarioConfig::from_toml_str(SMALL_TOML).expect("small preset parses");
    cfg.seed = seed;
    cfg
}

fn run_cfg(cfg: &ScenarioConfig, ticks: u32) -> (Sim, Vec<Event>) {
    let mut sim = Sim::new(cfg).expect("scenario builds");
    let mut events = Vec::new();
    for _ in 0..ticks {
        sim.step(&mut events);
    }
    (sim, events)
}

fn run(seed: u64, ticks: u32) -> (u64, Vec<Event>) {
    let (sim, events) = run_cfg(&small(seed), ticks);
    (sim.state_hash(), events)
}

#[test]
fn same_seed_gives_same_hash_and_events() {
    let (h1, e1) = run(42, TICKS);
    let (h2, e2) = run(42, TICKS);
    assert_eq!(h1, h2);
    assert_eq!(e1, e2);
}

#[test]
fn different_seed_gives_different_hash() {
    let (h1, _) = run(1, TICKS);
    let (h2, _) = run(2, TICKS);
    assert_ne!(h1, h2);
}

#[test]
fn hash_changes_as_time_advances() {
    let mut sim = Sim::new(&small(1)).unwrap();
    let mut events = Vec::new();
    let h0 = sim.state_hash();
    sim.step(&mut events);
    assert_ne!(h0, sim.state_hash());
}

/// Station 1's arrivals as `(tick, destination)`.
fn station_arrivals(events: &[Event], station: StationId) -> Vec<(Tick, u16)> {
    events
        .iter()
        .filter_map(|e| match *e {
            Event::ParcelArrived {
                tick,
                station: s,
                destination,
                ..
            } if s == station => Some((tick, destination.0)),
            _ => None,
        })
        .collect()
}

#[test]
fn removing_one_stations_arrivals_leaves_the_others_unchanged() {
    let cfg = small(11);
    let (_, base) = run_cfg(&cfg, TICKS);

    let mut sim = Sim::new(&cfg).unwrap();
    sim.set_arrival_rate(StationId(0), 0).unwrap();
    let mut events = Vec::new();
    for _ in 0..TICKS {
        sim.step(&mut events);
    }
    assert!(station_arrivals(&events, StationId(0)).is_empty());
    for s in 1..4 {
        let expected = station_arrivals(&base, StationId(s));
        assert!(!expected.is_empty());
        assert_eq!(station_arrivals(&events, StationId(s)), expected, "S{s}");
    }
}

/// Check vertex, swap and cell-kind invariants between two ticks.
fn check_moves(sim: &Sim, prev: &[u32]) {
    let world = sim.world();
    let layout = sim.layout();
    let cells = layout.grid.len();
    let pos = &world.robots.pos;
    let mut owner = vec![u32::MAX; cells];
    for (r, &c) in pos.iter().enumerate() {
        assert_eq!(
            owner[c as usize],
            u32::MAX,
            "tick {}: robots {} and {r} share cell {c}",
            sim.tick(),
            owner[c as usize]
        );
        owner[c as usize] = u32::try_from(r).unwrap();
        match layout.grid.cell(c) {
            Cell::Free => {}
            Cell::Induction(s) => assert!(
                matches!(
                    world.robots.state[r],
                    RobotState::ToInduction | RobotState::Loading | RobotState::ToChute
                ) && (world.robots.goal[r] == Some(Goal::Station(s))
                    || world.robots.state[r] == RobotState::ToChute),
                "tick {}: robot {r} on station {s} in state {:?}",
                sim.tick(),
                world.robots.state[r]
            ),
            other => panic!("tick {}: robot {r} inside {other:?}", sim.tick()),
        }
    }
    let mut was_at = vec![u32::MAX; cells];
    for (r, &c) in prev.iter().enumerate() {
        was_at[c as usize] = u32::try_from(r).unwrap();
    }
    for (r, (&a, &b)) in prev.iter().zip(pos).enumerate() {
        let other = was_at[b as usize];
        if a != b && other != u32::MAX && other as usize != r {
            assert_ne!(pos[other as usize], a, "robots {r} and {other} swapped");
        }
    }
}

/// Check parcel conservation against the world and the tick's events.
fn check_conservation(sim: &Sim, arrived_events: u64, dropped_events: u64) {
    let s = sim.stats();
    assert_eq!(
        s.arrived,
        s.queued + s.in_flight + s.delivered,
        "conservation broken at tick {}: {s:?}",
        s.tick
    );
    let phases = &sim.world().parcels.phase;
    let count = |f: fn(Phase) -> bool| phases.iter().filter(|&&p| f(p)).count() as u64;
    assert_eq!(s.arrived, phases.len() as u64);
    assert_eq!(
        s.queued,
        count(|p| matches!(p, Phase::Queued | Phase::AwaitingRobot))
    );
    assert_eq!(
        s.in_flight,
        count(|p| matches!(
            p,
            Phase::Loading | Phase::Travelling | Phase::Blocked | Phase::ChuteWait
        ))
    );
    assert_eq!(s.delivered, count(|p| p == Phase::Dropped));
    assert_eq!(s.arrived, arrived_events);
    assert_eq!(s.delivered, dropped_events);
}

fn run_checked(cfg: &ScenarioConfig, ticks: u32) -> Sim {
    let mut sim = Sim::new(cfg).unwrap();
    let mut events = Vec::new();
    let mut prev = sim.world().robots.pos.clone();
    let (mut arrived, mut dropped) = (0u64, 0u64);
    for _ in 0..ticks {
        events.clear();
        sim.step(&mut events);
        for e in &events {
            match e {
                Event::ParcelArrived { .. } => arrived += 1,
                Event::ParcelDropped { .. } => dropped += 1,
                _ => {}
            }
        }
        check_moves(&sim, &prev);
        check_conservation(&sim, arrived, dropped);
        prev.clone_from(&sim.world().robots.pos);
    }
    sim
}

#[test]
fn small_preset_keeps_every_invariant() {
    let sim = run_checked(&small(7), TICKS);
    assert!(sim.stats().delivered > 0, "nothing was delivered");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]
    /// Random seeds and small floors: no vertex or swap conflicts, parcels
    /// conserved, every tick.
    #[test]
    fn random_small_floors_keep_every_invariant(
        seed in any::<u64>(),
        width in 12u16..26,
        height in 12u16..26,
        stations in 1u16..4,
        south_margin in 2u16..5,
        robots in 1u32..40,
        rate in 100u32..1_500,
        k in 1u32..5,
    ) {
        let mut cfg = small(seed);
        cfg.floor = FloorConfig { width, height, stations, south_margin };
        cfg.fleet.robots = robots;
        cfg.arrivals.parcels_per_hour_per_station = rate;
        cfg.arrivals.destinations = None;
        cfg.chutes.bag_capacity = 5;
        cfg.chutes.bag_swap_ticks = 20;
        cfg.guidance.against_flow_cost = k;
        // Some combinations do not fit the floor; those are config errors,
        // not invariant violations.
        if Sim::new(&cfg).is_ok() {
            run_checked(&cfg, 400);
        }
    }
}

/// The phase transitions allowed by ARCHITECTURE §3:
/// `Queued -> AwaitingRobot -> Loading -> Travelling <-> Blocked ->
/// ChuteWait -> Dropped` (a parcel may leave `Loading` blocked).
fn allowed(from: Phase, to: Phase) -> bool {
    use Phase::{AwaitingRobot, Blocked, ChuteWait, Dropped, Loading, Queued, Travelling};
    matches!(
        (from, to),
        (Queued, AwaitingRobot)
            | (AwaitingRobot, Loading)
            | (Loading | Blocked, Travelling)
            | (Loading | Travelling, Blocked)
            | (Travelling | Blocked, ChuteWait)
            | (ChuteWait, Dropped)
    )
}

#[test]
fn every_parcel_follows_the_lifecycle_exactly() {
    let mut cfg = small(5);
    cfg.chutes.bag_capacity = 3;
    let (_, events) = run_cfg(&cfg, TICKS);
    let mut last: Vec<Option<(u32, Phase)>> = Vec::new();
    let mut dropped = 0;
    for e in &events {
        if let Event::ParcelPhase {
            tick,
            parcel,
            phase,
        } = *e
        {
            if last.len() <= parcel.index() {
                last.resize(parcel.index() + 1, None);
            }
            match last[parcel.index()] {
                None => assert_eq!(phase, Phase::Queued, "{parcel} did not start Queued"),
                Some((t0, p0)) => {
                    assert!(allowed(p0, phase), "{parcel}: illegal {p0:?} -> {phase:?}");
                    // Only the queue can be empty (a robot was free at once);
                    // every other phase lasts at least one tick.
                    if p0 == Phase::Queued {
                        assert!(tick >= t0);
                    } else {
                        assert!(tick > t0, "{parcel}: empty {p0:?} span at {tick}");
                    }
                }
            }
            dropped += usize::from(phase == Phase::Dropped);
            last[parcel.index()] = Some((tick, phase));
        }
    }
    assert!(dropped > 0);
}

#[test]
fn wait_events_match_the_wait_for_record() {
    let mut sim = Sim::new(&small(9)).unwrap();
    let mut events = Vec::new();
    let mut waits = 0;
    for _ in 0..TICKS {
        events.clear();
        sim.step(&mut events);
        let mut from_events = vec![None; sim.world().robots.len()];
        for e in &events {
            if let Event::RobotWaited {
                robot,
                direct,
                root,
                ..
            } = *e
            {
                assert_eq!(direct, root, "CP1 sets root = direct");
                assert!(
                    from_events[robot.index()].is_none(),
                    "two waits for {robot}"
                );
                if let WaitCause::BlockedBy(b) | WaitCause::Yielded(b) = direct {
                    assert_ne!(b, robot, "{robot} waits for itself");
                    assert!(b.index() < sim.world().robots.len());
                }
                // Only robots with work record waits.
                assert!(matches!(
                    sim.world().robots.goal[robot.index()],
                    Some(Goal::Station(_) | Goal::Chute(_))
                ));
                from_events[robot.index()] = Some(direct);
                waits += 1;
            }
        }
        // Events are the wait-for record restricted to robots with work.
        let working: Vec<_> = sim
            .wait_for()
            .iter()
            .zip(&sim.world().robots.goal)
            .map(|(&c, g)| c.filter(|_| matches!(g, Some(Goal::Station(_) | Goal::Chute(_)))))
            .collect();
        assert_eq!(from_events, working, "tick {}", sim.tick() - 1);
    }
    assert!(waits > 0, "a busy floor should produce some waits");
}

#[test]
fn bag_swaps_make_robots_wait_with_chute_unavailable() {
    let mut cfg = small(4);
    cfg.chutes.bag_capacity = 2;
    cfg.chutes.bag_swap_ticks = 150;
    let mut sim = Sim::new(&cfg).unwrap();
    let mut events = Vec::new();
    let (mut unavailable, mut swaps, mut reopen) = (0, 0, 0);
    for _ in 0..TICKS {
        events.clear();
        sim.step(&mut events);
        for e in &events {
            match *e {
                Event::RobotWaited {
                    robot,
                    direct: WaitCause::ChuteUnavailable(c),
                    ..
                } => {
                    unavailable += 1;
                    let w = sim.world();
                    assert_eq!(w.robots.goal[robot.index()], Some(Goal::Chute(c)));
                    let ch = &w.chutes;
                    let i = c.index();
                    assert!(
                        ch.state[i] != ChuteState::Open
                            || ch.fill[i] + u32::from(ch.incoming[i]) >= 2,
                        "{c} could take a parcel"
                    );
                }
                Event::ChuteChanged {
                    state: ChuteState::Swapping,
                    fill,
                    ..
                } => {
                    assert_eq!(fill, 2);
                    swaps += 1;
                }
                Event::ChuteChanged {
                    state: ChuteState::Open,
                    fill: 0,
                    ..
                } => reopen += 1,
                _ => {}
            }
        }
        for &fill in &sim.world().chutes.fill {
            assert!(fill <= 2, "bag overflowed");
        }
    }
    assert!(swaps > 0 && reopen > 0, "swaps {swaps}, reopened {reopen}");
    assert!(unavailable > 0, "no robot ever waited for a swap");
}

/// A planner that never moves anyone and blames the next robot: every robot
/// with work is stuck, as in a gridlock.
#[derive(Debug)]
struct Frozen;

impl Planner for Frozen {
    fn plan(&mut self, input: &PlanInput<'_>, out: &mut PlanOutput) {
        let n = input.pos.len();
        out.next = input.pos.to_vec();
        out.cause = (0..n)
            .map(|r| {
                let wants = input.movable[r]
                    && input.goal[r].is_some_and(|g| input.distances.get(g, input.pos[r]) > 0);
                let next = RobotId(u32::try_from((r + 1) % n).unwrap());
                wants.then_some(WaitCause::BlockedBy(next))
            })
            .collect();
    }

    fn hash_state(&self, _: &mut Fnv1a) {}
}

fn frozen_run(ticks: u32) -> (Sim, Vec<Event>) {
    let mut cfg = small(2);
    cfg.planner.watchdog_ticks = 100;
    let mut sim = Sim::with_planner(&cfg, Box::new(Frozen)).unwrap();
    let mut events = Vec::new();
    for _ in 0..ticks {
        sim.step(&mut events);
    }
    (sim, events)
}

#[test]
fn watchdog_reports_a_floor_that_stops_sorting() {
    let (_, events) = frozen_run(1_000);
    let stalls: Vec<_> = events
        .iter()
        .filter_map(|e| match *e {
            Event::Stalled { tick, since } => Some((tick, since)),
            _ => None,
        })
        .collect();
    assert!(!stalls.is_empty(), "the floor watchdog never fired");
    for &(tick, since) in &stalls {
        assert!(tick - since >= 100);
    }
    // A healthy floor never stalls.
    let (_, events) = run_cfg(&small(2), TICKS);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::Stalled { .. } | Event::RobotStalled { .. }))
    );
}

#[test]
fn each_stuck_robot_reports_one_stall_per_episode() {
    let (sim, events) = frozen_run(1_000);
    let mut per_robot = vec![0u32; sim.world().robots.len()];
    for e in &events {
        if let Event::RobotStalled { robot, waited, .. } = *e {
            assert_eq!(waited, 100);
            per_robot[robot.index()] += 1;
        }
    }
    // Robots with work never progress, so each reports exactly once.
    assert!(
        per_robot.iter().any(|&n| n > 0),
        "no robot reported a stall"
    );
    assert!(per_robot.iter().all(|&n| n <= 1), "{per_robot:?}");
}

#[test]
fn station_busy_always_names_a_loading_robot() {
    let mut sim = Sim::new(&small(8)).unwrap();
    let mut events = Vec::new();
    let mut busy = 0;
    for _ in 0..TICKS {
        events.clear();
        sim.step(&mut events);
        let w = sim.world();
        for e in &events {
            if let Event::RobotWaited {
                direct: WaitCause::StationBusy(s),
                ..
            } = *e
            {
                busy += 1;
                let cell = w.stations.cell[s.index()];
                let loader = w.robots.pos.iter().position(|&p| p == cell);
                let loader = loader.unwrap_or_else(|| panic!("{s} busy but empty"));
                assert_eq!(w.robots.state[loader], RobotState::Loading, "{s}");
            }
        }
    }
    assert!(busy > 0, "no StationBusy wait on a busy floor");
}

#[test]
fn wait_for_chains_are_walkable() {
    // `Yielded(b)` names a robot that moved (it won the cell: a terminal of
    // the walk). `BlockedBy(b)` names a robot that stayed; it either has its
    // own cause (the walk continues) or is settled: loading, dropping, at its
    // goal, or without a goal. A chain is never cut at a robot that was itself
    // trying to get somewhere, idle and parking robots included.
    let mut cfg = ScenarioConfig::from_toml_str(MEDIUM_TOML).unwrap();
    cfg.chutes.bag_capacity = 5;
    let mut sim = Sim::new(&cfg).unwrap();
    let mut events = Vec::new();
    let mut edges = 0;
    for _ in 0..1_000 {
        events.clear();
        let before = sim.world().robots.pos.clone();
        sim.step(&mut events);
        let w = sim.world();
        for (r, cause) in sim.wait_for().iter().enumerate() {
            let b = match *cause {
                Some(WaitCause::Yielded(b)) => {
                    assert_ne!(
                        w.robots.pos[b.index()],
                        before[b.index()],
                        "{b} did not move"
                    );
                    continue;
                }
                Some(WaitCause::BlockedBy(b)) => b,
                _ => continue,
            };
            edges += 1;
            assert_ne!(b.index(), r);
            assert_eq!(w.robots.pos[b.index()], before[b.index()], "{b} moved");
            if sim.wait_for()[b.index()].is_some() {
                continue;
            }
            // Robots with work plan before idle and parking robots (PIBT
            // classes), so an idle or parking robot blocking one was pushed
            // and could not move: it must carry its own cause.
            let working = |g: Option<Goal>| matches!(g, Some(Goal::Station(_) | Goal::Chute(_)));
            assert!(
                !working(w.robots.goal[r]) || working(w.robots.goal[b.index()]) || {
                    matches!(
                        w.robots.state[b.index()],
                        RobotState::Loading | RobotState::Dropping
                    )
                },
                "tick {}: R{r} blocked by idle {b}, which records no cause",
                sim.tick() - 1
            );
            let settled = matches!(
                w.robots.state[b.index()],
                RobotState::Loading | RobotState::Dropping
            ) || w.robots.goal[b.index()]
                .is_none_or(|g| sim.distances().get(g, w.robots.pos[b.index()]) == 0);
            assert!(
                settled,
                "tick {}: chain from R{r} cut at {b} ({:?}, goal {:?})",
                sim.tick() - 1,
                w.robots.state[b.index()],
                w.robots.goal[b.index()]
            );
        }
    }
    assert!(edges > 0);
}

/// Each parcel keyed by `(station, ordinal at that station)`, with its
/// destination, scanned target and scan tick (if loaded).
type Keyed = Vec<((u16, u32), (u16, Option<u16>, Option<Tick>))>;

fn keyed(sim: &Sim, events: &[Event]) -> Keyed {
    let p = &sim.world().parcels;
    let load = u32::from(sim.config().timing.load_ticks);
    let mut scanned = vec![None; p.len()];
    for e in events {
        if let Event::ParcelPhase {
            tick,
            parcel,
            phase: Phase::Loading,
        } = *e
        {
            scanned[parcel.index()] = Some(tick + load);
        }
    }
    let mut v: Keyed = (0..p.len())
        .map(|i| {
            (
                (p.station[i].0, p.ordinal[i]),
                (p.destination[i].0, p.target[i].map(|c| c.0), scanned[i]),
            )
        })
        .collect();
    v.sort_unstable();
    v
}

fn lookup(k: &Keyed, key: (u16, u32)) -> Option<(u16, Option<u16>, Option<Tick>)> {
    k.binary_search_by_key(&key, |e| e.0).ok().map(|i| k[i].1)
}

#[test]
fn counterfactual_changes_leave_other_parcels_untouched() {
    // Review focus #4: changing one cause must not shift the random draws
    // of unrelated parcels (ADR 0002 §3, ADR 0005).
    let mut cfg = small(21);
    cfg.scanner.error_rate_ppm = 50_000;
    let base = {
        let (sim, events) = run_cfg(&cfg, TICKS);
        keyed(&sim, &events)
    };

    // (a) A surge at station 0 from tick 500.
    let mut sim = Sim::new(&cfg).unwrap();
    let mut events = Vec::new();
    for t in 0..TICKS {
        if t == 500 {
            sim.set_arrival_rate(StationId(0), 2_000).unwrap();
        }
        sim.step(&mut events);
    }
    let surge = keyed(&sim, &events);
    let mut compared = 0;
    for &(key, (dest, target, _)) in &surge {
        if let Some((d0, t0, _)) = lookup(&base, key) {
            assert_eq!(dest, d0, "destination of {key:?} shifted");
            if let (Some(a), Some(b)) = (target, t0) {
                assert_eq!(a, b, "scan of {key:?} shifted");
                compared += 1;
            }
        }
    }
    assert!(compared > 100, "{compared}");

    // (b) Scanner drift: 30 % misreads during ticks 500..1000 only.
    let mut drift_all = cfg.clone();
    drift_all.scanner.error_rate_ppm = 300_000;
    let all = {
        let (sim, events) = run_cfg(&drift_all, TICKS);
        keyed(&sim, &events)
    };
    let mut sim = Sim::new(&cfg).unwrap();
    let mut events = Vec::new();
    for t in 0..TICKS {
        match t {
            500 => sim.set_scanner_error_rate(300_000).unwrap(),
            1_000 => sim.set_scanner_error_rate(50_000).unwrap(),
            _ => {}
        }
        sim.step(&mut events);
    }
    let window = keyed(&sim, &events);
    let (mut outside, mut inside) = (0, 0);
    for &(key, (dest, target, scanned)) in &window {
        let (Some(target), Some(at)) = (target, scanned) else {
            continue;
        };
        let reference = if (500..1_000).contains(&at) {
            inside += 1;
            lookup(&all, key)
        } else {
            outside += 1;
            lookup(&base, key)
        };
        if let Some((d0, Some(t0), _)) = reference {
            assert_eq!(dest, d0, "destination of {key:?} shifted");
            assert_eq!(target, t0, "scan of {key:?} (at {at}) shifted");
        }
    }
    assert!(inside > 20 && outside > 100, "{inside} / {outside}");
}

#[test]
fn arrival_rate_changes_are_checked_and_hashed() {
    let mut a = Sim::new(&small(3)).unwrap();
    let b = Sim::new(&small(3)).unwrap();
    assert!(
        a.set_arrival_rate(StationId(0), TICKS_PER_HOUR + 1)
            .is_err()
    );
    assert!(a.set_arrival_rate(StationId(99), 100).is_err());
    assert_eq!(a.state_hash(), b.state_hash());
    a.set_arrival_rate(StationId(0), 1_000).unwrap();
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn overload_does_not_collapse_throughput() {
    // Past capacity a well-behaved floor saturates; it must not fall over
    // (robots queueing at swapping chutes used to gridlock the aisles).
    let hour = |rate: u32| {
        let mut cfg = ScenarioConfig::from_toml_str(MEDIUM_TOML).unwrap();
        cfg.arrivals.parcels_per_hour_per_station = rate;
        let (sim, _) = run_cfg(&cfg, TICKS_PER_HOUR);
        sim.stats().delivered
    };
    let at_capacity = hour(1_300);
    let overloaded = hour(1_950);
    eprintln!("medium, 1 h: 1,300/h/station -> {at_capacity}; 1,950/h/station -> {overloaded}");
    assert!(
        overloaded * 10 >= at_capacity * 9,
        "throughput collapsed: {at_capacity} -> {overloaded}"
    );
}

#[test]
fn scanner_misreads_are_mis_sorted_and_reported() {
    let mut cfg = small(6);
    cfg.scanner.error_rate_ppm = 200_000;
    let (sim, events) = run_cfg(&cfg, TICKS);
    let w = sim.world();
    let mut reported = 0u64;
    for (i, e) in events.iter().enumerate() {
        if let Event::ParcelMisSorted {
            parcel,
            chute,
            expected,
            ..
        } = *e
        {
            reported += 1;
            assert_eq!(w.parcels.destination[parcel.index()], expected);
            assert_ne!(w.destination_chute[expected.index()], chute);
            assert_eq!(w.parcels.chute[parcel.index()], Some(chute));
            assert!(
                matches!(events[i - 1], Event::ParcelDropped { parcel: p, .. } if p == parcel),
                "ParcelMisSorted must follow its ParcelDropped"
            );
        }
    }
    let s = sim.stats();
    assert_eq!(s.mis_sorted, reported);
    // About 20 % of deliveries.
    assert!(
        (s.delivered / 10..=s.delivered * 3 / 10).contains(&s.mis_sorted),
        "{s:?}"
    );

    // A perfect scanner never mis-sorts.
    let (sim, _) = run_cfg(&small(6), TICKS);
    assert_eq!(sim.stats().mis_sorted, 0);
}

#[test]
fn chute_mapping_strategies() {
    let mut cfg = small(1);
    cfg.chutes.mapping = ChuteMapping::RoundRobin;
    cfg.arrivals.destinations = Some(200);
    let sim = Sim::new(&cfg).unwrap();
    let chutes = sim.layout().chutes.len();
    for (d, &c) in sim.world().destination_chute.iter().enumerate() {
        assert_eq!(c.index(), d % chutes);
    }

    cfg.chutes.mapping = ChuteMapping::PopularityNearInduction;
    let sim = Sim::new(&cfg).unwrap();
    let l = sim.layout();
    let total = |c: ChuteId| -> u64 {
        l.stations
            .iter()
            .map(|&s| u64::from(sim.distances().get(Goal::Chute(c), s)))
            .sum()
    };
    let map = &sim.world().destination_chute;
    // More popular destinations (lower ids) never sit further away.
    for w in map[..chutes].windows(2) {
        assert!(total(w[0]) <= total(w[1]));
    }
    // Every chute is used once before any is reused.
    let mut first: Vec<_> = map[..chutes].to_vec();
    first.sort_unstable();
    first.dedup();
    assert_eq!(first.len(), chutes);
}

#[test]
fn throughput_is_in_a_plausible_range() {
    // Short smoke band on `small`; the calibration test below is the real
    // check on `medium`.
    let (sim, _) = run_cfg(&small(1), TICKS);
    let per_hour = sim.stats().throughput_per_hour_milli() / 1_000;
    assert!((1_000..=2_200).contains(&per_hour), "{per_hour} parcels/h");
}

/// Calibration (research: industry-and-client §4): the medium hub sorts
/// 25-50 parcels/h per robot over 2 simulated hours, with no stall and no
/// runaway backlog. About 8 s in a debug build, 0.5 s in release.
#[test]
fn medium_preset_is_calibrated() {
    let cfg = ScenarioConfig::from_toml_str(MEDIUM_TOML).unwrap();
    let (sim, events) = run_cfg(&cfg, 2 * TICKS_PER_HOUR);
    let s = sim.stats();
    let per_robot_milli = s.throughput_per_hour_milli() / u64::from(cfg.fleet.robots);
    eprintln!(
        "medium: {:.1} parcels/h, {:.2} per robot/h, {s:?}",
        s.throughput_per_hour_milli() as f64 / 1_000.0,
        per_robot_milli as f64 / 1_000.0
    );
    assert!(
        (25_000..=50_000).contains(&per_robot_milli),
        "{per_robot_milli} milli-parcels/h per robot"
    );
    assert!(!events.iter().any(|e| matches!(e, Event::Stalled { .. })));
    assert!(s.queued < s.arrived / 20, "backlog grew: {s:?}");
}

#[test]
fn robots_waiting_at_swapping_chutes_do_not_wall_off_the_aisles() {
    // Small bags and slow swaps under heavy demand: many robots wait at
    // chutes. Waiting robots stay pushable, so through traffic still flows.
    // Measured: about 10,400/h; with immovable waiters it was about 8,100/h.
    let mut cfg = ScenarioConfig::from_toml_str(MEDIUM_TOML).unwrap();
    cfg.arrivals.parcels_per_hour_per_station = 2_000;
    cfg.chutes.bag_capacity = 20;
    cfg.chutes.bag_swap_ticks = 120;
    let (sim, _) = run_cfg(&cfg, TICKS_PER_HOUR);
    let delivered = sim.stats().delivered;
    assert!(delivered >= 9_500, "{delivered} parcels in 1 h");
}
