//! End-to-end invariants of the V0 simulation on the `small` preset.
//!
//! - Determinism (ADR 0002): same seed, same hash and same event stream;
//!   different seed, different hash.
//! - No two robots ever share a cell, and no two robots swap cells.
//! - Parcel conservation: arrived = queued + in-flight + delivered, checked
//!   against an independent count of parcel phases and the event stream.

use switchyard_core::config::ScenarioConfig;
use switchyard_core::events::Event;
use switchyard_core::grid::Cell;
use switchyard_core::presets::SMALL_TOML;
use switchyard_core::sim::Sim;
use switchyard_core::world::{Phase, RobotState};

const TICKS: u32 = 2_000;

fn small(seed: u64) -> ScenarioConfig {
    let mut cfg = ScenarioConfig::from_toml_str(SMALL_TOML).expect("small preset parses");
    cfg.seed = seed;
    cfg
}

fn run(seed: u64, ticks: u32) -> (u64, Vec<Event>) {
    let mut sim = Sim::new(&small(seed)).expect("small preset builds");
    let mut events = Vec::new();
    for _ in 0..ticks {
        sim.step(&mut events);
    }
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

#[test]
fn no_two_robots_share_a_cell_or_swap() {
    let mut sim = Sim::new(&small(7)).unwrap();
    let cells = sim.layout().grid.len();
    let mut events = Vec::new();
    let mut prev = sim.world().robots.pos.clone();
    for _ in 0..TICKS {
        sim.step(&mut events);
        let world = sim.world();
        let layout = sim.layout();
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

            // Robots stand only on open floor, or on an induction cell they
            // were sent to.
            match layout.grid.cell(c) {
                Cell::Free => {}
                Cell::Induction(_) => assert!(
                    matches!(
                        world.robots.state[r],
                        RobotState::ToInduction | RobotState::Loading | RobotState::ToChute
                    ),
                    "tick {}: robot {r} parked on a station in state {:?}",
                    sim.tick(),
                    world.robots.state[r]
                ),
                other => panic!("tick {}: robot {r} inside {other:?}", sim.tick()),
            }
        }

        // No swaps: if r moved a -> b, whoever was at b did not move to a.
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
        prev.clone_from(pos);
    }
}

#[test]
fn parcels_are_conserved_every_tick() {
    let mut sim = Sim::new(&small(3)).unwrap();
    let mut events = Vec::new();
    let (mut arrived_events, mut dropped_events) = (0u64, 0u64);
    for _ in 0..TICKS {
        events.clear();
        sim.step(&mut events);
        for e in &events {
            match e {
                Event::ParcelArrived { .. } => arrived_events += 1,
                Event::ParcelDropped { .. } => dropped_events += 1,
                _ => {}
            }
        }

        let s = sim.stats();
        assert_eq!(
            s.arrived,
            s.queued + s.in_flight + s.delivered,
            "conservation broken at tick {}: {s:?}",
            s.tick
        );

        // Independent count from the parcel phase column.
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
    let s = sim.stats();
    assert!(s.delivered > 0, "nothing was delivered: {s:?}");
}

#[test]
fn every_parcel_follows_the_lifecycle_in_order() {
    let (_, events) = run(5, TICKS);
    let mut last: Vec<Option<(u32, Phase)>> = Vec::new();
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
                    assert!(tick >= t0, "{parcel} went back in time");
                    assert_ne!(p0, Phase::Dropped, "{parcel} changed after Dropped");
                    let forward =
                        phase > p0 || (p0 == Phase::Blocked && phase == Phase::Travelling);
                    assert!(forward, "{parcel}: illegal {p0:?} -> {phase:?}");
                }
            }
            last[parcel.index()] = Some((tick, phase));
        }
    }
}

#[test]
fn throughput_is_in_a_plausible_range() {
    // V0 movement is naive, so this is a smoke band, not the CP1
    // calibration test (25-50 parcels/h per robot).
    let mut sim = Sim::new(&small(1)).unwrap();
    let mut events = Vec::new();
    for _ in 0..TICKS {
        sim.step(&mut events);
        events.clear();
    }
    let per_hour = sim.stats().throughput_per_hour_milli() / 1_000;
    assert!((500..=2_500).contains(&per_hour), "{per_hour} parcels/h");
}
