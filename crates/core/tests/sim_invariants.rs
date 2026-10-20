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
use switchyard_core::ids::{ChuteId, StationId};
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
    sim.set_arrival_rate(StationId(0), 0);
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
        assert_eq!(from_events, sim.wait_for(), "tick {}", sim.tick() - 1);
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

#[test]
fn watchdog_reports_a_floor_that_stops_sorting() {
    // Every chute takes one parcel and then swaps for longer than the run:
    // soon every robot holds a parcel nobody can take.
    let mut cfg = small(2);
    cfg.chutes.bag_capacity = 1;
    cfg.chutes.bag_swap_ticks = 1_000_000;
    cfg.planner.watchdog_ticks = 100;
    let (_, events) = run_cfg(&cfg, 6_000);
    let stalls: Vec<_> = events
        .iter()
        .filter_map(|e| match *e {
            Event::Stalled { tick, since } => Some((tick, since)),
            _ => None,
        })
        .collect();
    assert!(!stalls.is_empty(), "the watchdog never fired");
    for &(tick, since) in &stalls {
        assert!(tick - since >= 100);
    }
    // A healthy floor never stalls.
    let (_, events) = run_cfg(&small(2), TICKS);
    assert!(!events.iter().any(|e| matches!(e, Event::Stalled { .. })));
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
