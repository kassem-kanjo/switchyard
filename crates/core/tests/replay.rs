//! Checkpoints, counterfactual replays, interventions and re-convergence
//! (ADR 0005; `switchyard_core::sim::replay` docs).
//!
//! The `#[ignore]`d `measure_*` tests print costs for the CP3 design; run
//! them with `cargo test --release -p switchyard-core --test replay --
//! --ignored --nocapture`.

use std::time::Instant;

use switchyard_core::Tick;
use switchyard_core::config::ScenarioConfig;
use switchyard_core::events::Event;
use switchyard_core::faults::{FaultKind, FaultSpec};
use switchyard_core::guidance::DistanceTables;
use switchyard_core::ids::{ChuteId, DestinationId, FaultId, RobotId, StationId};
use switchyard_core::interventions::Intervention;
use switchyard_core::presets::{self, LARGE_TOML, MEDIUM_TOML, SMALL_TOML};
use switchyard_core::sim::Sim;
use switchyard_core::world::{Phase, RobotState};

fn small(seed: u64) -> ScenarioConfig {
    let mut cfg = ScenarioConfig::from_toml_str(SMALL_TOML).unwrap();
    cfg.seed = seed;
    cfg
}

fn fault(at: Tick, duration: Tick, kind: FaultKind) -> FaultSpec {
    FaultSpec { at, duration, kind }
}

fn run_to(sim: &mut Sim, end: Tick) -> Vec<Event> {
    let mut events = Vec::new();
    while sim.tick() < end {
        sim.step(&mut events);
    }
    events
}

fn run(cfg: &ScenarioConfig, end: Tick) -> (Sim, Vec<Event>) {
    let mut sim = Sim::new(cfg).unwrap();
    let events = run_to(&mut sim, end);
    (sim, events)
}

#[test]
fn a_checkpoint_resumes_bit_identically() {
    let mut cfg = small(5);
    cfg.faults.push(fault(
        300,
        400,
        FaultKind::RobotFailure { robot: RobotId(3) },
    ));
    cfg.faults
        .push(fault(500, 200, FaultKind::ChuteJam { chute: ChuteId(2) }));
    let (fresh, fresh_events) = run(&cfg, 1_500);
    for t in [0, 299, 300, 450, 1_000] {
        let mut a = Sim::new(&cfg).unwrap();
        let mut before = run_to(&mut a, t);
        let mut checkpoint = a.clone();
        before.extend(run_to(&mut checkpoint, 1_500));
        assert_eq!(
            checkpoint.state_hash(),
            fresh.state_hash(),
            "checkpoint at {t}"
        );
        assert_eq!(before, fresh_events, "events from checkpoint at {t}");
        // The original is untouched by its clone and continues identically.
        run_to(&mut a, 1_500);
        assert_eq!(a.state_hash(), fresh.state_hash());
    }
}

#[test]
fn disabling_a_fault_at_onset_equals_never_having_it_on_every_chaos_preset() {
    for (name, text) in presets::CHAOS {
        let cfg = ScenarioConfig::from_toml_str(text).unwrap();
        let onset = cfg.faults.iter().map(|f| f.at).min().unwrap();
        let end = onset + 600;
        let mut actual = Sim::new(&cfg).unwrap();
        run_to(&mut actual, onset);
        for i in 0..cfg.faults.len() {
            let id = FaultId(u32::try_from(i).unwrap());
            let mut replay = actual.clone();
            let mut events = Vec::new();
            replay.disable_fault(id, &mut events).unwrap();
            assert!(events.is_empty(), "a pending fault ends silently");
            run_to(&mut replay, end);
            let (never, _) = run(&cfg.without_fault(id), end);
            assert_eq!(replay.state_hash(), never.state_hash(), "{name}: {id}");
            assert_eq!(replay.config(), never.config(), "{name}: {id}");
        }
    }
}

#[test]
fn disabling_an_active_fault_ends_it_cleanly_now() {
    let mut cfg = small(6);
    cfg.faults.push(fault(
        300,
        600,
        FaultKind::RobotFailure { robot: RobotId(4) },
    ));
    cfg.faults
        .push(fault(300, 600, FaultKind::ChuteJam { chute: ChuteId(1) }));
    let mut sim = Sim::new(&cfg).unwrap();
    let before_fail = {
        let mut probe = Sim::new(&cfg).unwrap();
        run_to(&mut probe, 300);
        probe.world().robots.state[4]
    };
    run_to(&mut sim, 500);
    assert_eq!(sim.world().robots.state[4], RobotState::Failed);
    let mut events = Vec::new();
    sim.disable_fault(FaultId(0), &mut events).unwrap();
    sim.disable_fault(FaultId(1), &mut events).unwrap();
    assert!(events.contains(&Event::FaultEnded {
        tick: 500,
        fault: FaultId(0)
    }));
    assert!(events.contains(&Event::FaultEnded {
        tick: 500,
        fault: FaultId(1)
    }));
    assert_eq!(sim.world().robots.state[4], before_fail, "repaired now");
    assert_ne!(
        sim.world().chutes.state[1],
        switchyard_core::events::ChuteState::Jammed
    );
    // The scheduled end never comes, and the sim keeps conserving parcels.
    let later = run_to(&mut sim, 1_200);
    assert!(!later.iter().any(|e| matches!(e, Event::FaultEnded { .. })));
    let s = sim.stats();
    assert_eq!(s.arrived, s.queued + s.in_flight + s.delivered);
    // Ended, disabled or unknown faults cannot be disabled.
    assert!(sim.disable_fault(FaultId(0), &mut events).is_err());
    assert!(sim.disable_fault(FaultId(9), &mut events).is_err());
}

#[test]
fn interventions_applied_at_tick_zero_equal_the_transformed_scenario() {
    let interventions = [
        Intervention::RemapDestination {
            destination: DestinationId(0),
            chute: ChuteId(5),
        },
        Intervention::AddRobots { robots: 6, at: 400 },
        Intervention::SetStationCap {
            station: Some(StationId(2)),
            cap: 2,
        },
        Intervention::SetStationCap {
            station: None,
            cap: 3,
        },
    ];
    for iv in &interventions {
        let cfg = small(7);
        let (transformed, te) = run(&cfg.apply(iv).unwrap(), 1_200);
        let mut live = Sim::new(&cfg).unwrap();
        let mut events = Vec::new();
        live.apply_intervention(iv, &mut events).unwrap();
        events.extend(run_to(&mut live, 1_200));
        assert_eq!(live.state_hash(), transformed.state_hash(), "{iv:?}");
        assert_eq!(events, te, "{iv:?}");
        assert_ne!(
            transformed.state_hash(),
            run(&cfg, 1_200).0.state_hash(),
            "{iv:?} changed nothing"
        );
    }
    // Robots added from a later checkpoint, before they are due: identical.
    let cfg = small(7);
    let add = Intervention::AddRobots { robots: 6, at: 400 };
    let mut checkpoint = Sim::new(&cfg).unwrap();
    run_to(&mut checkpoint, 250);
    checkpoint
        .apply_intervention(&add, &mut Vec::new())
        .unwrap();
    run_to(&mut checkpoint, 1_200);
    assert_eq!(
        checkpoint.state_hash(),
        run(&cfg.apply(&add).unwrap(), 1_200).0.state_hash()
    );
    // ...but not in the past.
    assert!(
        checkpoint
            .apply_intervention(
                &Intervention::AddRobots { robots: 1, at: 3 },
                &mut Vec::new()
            )
            .is_err()
    );
}

#[test]
fn interventions_have_their_documented_effects() {
    let cfg = small(9);
    // Added robots appear idle at their tick and take work.
    let (sim, events) = run(
        &cfg.apply(&Intervention::AddRobots { robots: 6, at: 400 })
            .unwrap(),
        1_200,
    );
    assert_eq!(sim.world().robots.len(), 46);
    let joined: Vec<_> = events
        .iter()
        .filter_map(|e| match *e {
            Event::RobotState {
                tick,
                robot,
                state: RobotState::Idle,
            } if robot.index() >= 40 && tick == 400 => Some((tick, robot)),
            _ => None,
        })
        .collect();
    assert_eq!(joined.len(), 6, "six robots join, idle, at tick 400");
    assert!(events.iter().any(|e| matches!(*e,
        Event::ParcelAssigned { robot, .. } if robot.index() >= 40)));
    // A remapped destination's parcels go to the new chute.
    let remap = Intervention::RemapDestination {
        destination: DestinationId(0),
        chute: ChuteId(5),
    };
    let (sim, _) = run(&cfg.apply(&remap).unwrap(), 1_200);
    let p = &sim.world().parcels;
    let dropped: Vec<_> = (0..p.len())
        .filter(|&i| p.destination[i] == DestinationId(0) && p.phase[i] == Phase::Dropped)
        .map(|i| p.chute[i])
        .collect();
    assert!(!dropped.is_empty() && dropped.iter().all(|&c| c == Some(ChuteId(5))));
    // A station cap is never exceeded.
    let cap = Intervention::SetStationCap {
        station: Some(StationId(0)),
        cap: 1,
    };
    let mut sim = Sim::new(&cfg.apply(&cap).unwrap()).unwrap();
    let mut events = Vec::new();
    for _ in 0..1_200 {
        sim.step(&mut events);
        assert!(sim.world().stations.inbound[0] <= 1);
    }
    // A remap to a chute the floor does not have is rejected.
    let bad = Intervention::RemapDestination {
        destination: DestinationId(0),
        chute: ChuteId(999),
    };
    assert!(Sim::new(&cfg.apply(&bad).unwrap()).is_err());
}

#[test]
fn live_hashes_reconverge_when_a_fault_leaves_no_trace() {
    // A scanner drift to the base rate (0 ppm) changes nothing but its own
    // presence: the replay without it matches the original from the tick
    // the drift ends on.
    let mut cfg = small(10);
    cfg.faults.push(fault(
        50,
        100,
        FaultKind::ScannerDrift {
            station: StationId(0),
            error_rate_ppm: 0,
        },
    ));
    let mut original = Sim::new(&cfg).unwrap();
    let mut replay = Sim::new(&cfg.without_fault(FaultId(0))).unwrap();
    assert_ne!(original.live_hash(), replay.live_hash(), "pending fault");
    // The drift ends in phase 1 of tick 150, so the states match after it.
    assert_eq!(replay.reconverge_with(&mut original, 1_000), Some(151));
    // Full state hashes never match again: the schedules differ.
    assert_ne!(original.state_hash(), replay.state_hash());

    // The same answer against a recording of the original's live hashes.
    let mut rec_sim = Sim::new(&cfg).unwrap();
    let mut recorded = vec![rec_sim.live_hash()];
    let mut events = Vec::new();
    for _ in 0..1_000 {
        rec_sim.step(&mut events);
        events.clear();
        recorded.push(rec_sim.live_hash());
    }
    let mut replay = Sim::new(&cfg.without_fault(FaultId(0))).unwrap();
    assert_eq!(replay.reconverge_against(&recorded), Some(151));

    // A fault that moves robots: no re-convergence on this horizon is a
    // valid answer, and the call stops at the horizon.
    let mut cfg = small(10);
    cfg.faults.push(fault(
        50,
        200,
        FaultKind::RobotFailure { robot: RobotId(1) },
    ));
    let mut original = Sim::new(&cfg).unwrap();
    let mut replay = Sim::new(&cfg.without_fault(FaultId(0))).unwrap();
    let met = replay.reconverge_with(&mut original, 800);
    assert!(met.is_none_or(|t| t > 250), "{met:?}");
    assert!(replay.tick() <= 800);
}

/// Full and incremental rebuild cost when a fault starts on the large floor.
#[test]
#[ignore = "measurement; run in release with --ignored --nocapture"]
fn measure_rebuild_cost_on_large() {
    let cfg = ScenarioConfig::from_toml_str(LARGE_TOML).unwrap();
    let sim = Sim::new(&cfg).unwrap();
    let l = sim.layout();
    let n = l.grid.len();
    let healthy = DistanceTables::build(l, 3);
    let none = vec![false; n];
    // A dead robot (one cell) and a 2 x 30 aisle block.
    let g = &l.grid;
    let dead = vec![g.idx(switchyard_core::grid::Pos::new(22, 20))];
    let aisle: Vec<u32> = (3..33)
        .flat_map(|y| [22, 23].map(|x| g.idx(switchyard_core::grid::Pos::new(x, y))))
        .collect();
    for (what, cells) in [("dead robot", dead), ("aisle block", aisle)] {
        let mut mask = none.clone();
        for c in cells {
            mask[c as usize] = g.is_free(c);
        }
        let t = Instant::now();
        let full = DistanceTables::build_with_closed(l, 3, &mask);
        let full_ms = t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        let (inc, rows) = healthy.with_closed_changed(l, 3, &none, &mask);
        let inc_ms = t.elapsed().as_secs_f64() * 1e3;
        assert_eq!(inc, full);
        let t = Instant::now();
        let (back, back_rows) = inc.with_closed_changed(l, 3, &mask, &none);
        let back_ms = t.elapsed().as_secs_f64() * 1e3;
        assert_eq!(back, healthy);
        eprintln!(
            "large {what}: full {full_ms:.1} ms; incremental start {inc_ms:.1} ms ({rows} of {} rows), end {back_ms:.1} ms ({back_rows} rows)",
            l.stations.len() + l.chutes.len() + 1
        );
    }
}

/// Checkpoint sizes, re-convergence on the chaos presets, and the cost of 64
/// one-hour replays from a checkpoint on medium.
#[test]
#[ignore = "measurement; run in release with --ignored --nocapture"]
fn measure_replays() {
    for (name, toml, ticks) in [("medium", MEDIUM_TOML, 7_200), ("large", LARGE_TOML, 7_200)] {
        let cfg = ScenarioConfig::from_toml_str(toml).unwrap();
        let mut sim = Sim::new(&cfg).unwrap();
        run_to(&mut sim, ticks);
        let t = Instant::now();
        let copies: Vec<Sim> = (0..10).map(|_| sim.clone()).collect();
        let clone_ms = t.elapsed().as_secs_f64() * 1e3 / 10.0;
        drop(copies);
        eprintln!(
            "{name} checkpoint at 1 h: ~{} KiB, clone {clone_ms:.2} ms",
            sim.checkpoint_bytes() / 1024
        );
    }

    // Re-convergence: each preset fault disabled at onset, against the
    // actual (faulted) run, up to 1 h after the last fault ends.
    let (mut pairs, mut met) = (0, 0);
    for (name, text) in presets::CHAOS {
        let cfg = ScenarioConfig::from_toml_str(text).unwrap();
        let onset = cfg.faults.iter().map(|f| f.at).min().unwrap();
        let horizon = cfg.faults.iter().map(FaultSpec::end).max().unwrap() + 7_200;
        let mut actual = Sim::new(&cfg).unwrap();
        run_to(&mut actual, onset);
        for i in 0..cfg.faults.len() {
            let id = FaultId(u32::try_from(i).unwrap());
            let mut replay = actual.clone();
            replay.disable_fault(id, &mut Vec::new()).unwrap();
            let mut reference = actual.clone();
            let at = replay.reconverge_with(&mut reference, horizon);
            pairs += 1;
            met += usize::from(at.is_some());
            eprintln!("{name} without {id}: re-converged at {at:?} (horizon {horizon})");
        }
        // And the twin: no faults at all.
        let mut twin = actual.clone();
        for i in 0..cfg.faults.len() {
            twin.disable_fault(FaultId(u32::try_from(i).unwrap()), &mut Vec::new())
                .unwrap();
        }
        let mut reference = actual.clone();
        let at = twin.reconverge_with(&mut reference, horizon);
        pairs += 1;
        met += usize::from(at.is_some());
        eprintln!("{name} without any fault: re-converged at {at:?}");
    }
    eprintln!("re-converged {met} of {pairs} replays");

    // 64 replays of a 1 h window from a checkpoint at minute 30 on medium.
    let cfg = ScenarioConfig::from_toml_str(MEDIUM_TOML).unwrap();
    let mut checkpoint = Sim::new(&cfg).unwrap();
    run_to(&mut checkpoint, 3_600);
    let replay = |c: &Sim| {
        let mut s = c.clone();
        run_to(&mut s, 3_600 + 7_200);
        s.state_hash()
    };
    let t = Instant::now();
    let hashes: Vec<u64> = (0..64).map(|_| replay(&checkpoint)).collect();
    let serial = t.elapsed().as_secs_f64();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let t = Instant::now();
    let parallel: Vec<u64> = std::thread::scope(|scope| {
        let chunks: Vec<_> = (0..threads)
            .map(|k| {
                let c = &checkpoint;
                scope.spawn(move || {
                    (k..64)
                        .step_by(threads)
                        .map(|_| replay(c))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        chunks.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    let par = t.elapsed().as_secs_f64();
    assert!(hashes.iter().chain(&parallel).all(|&h| h == hashes[0]));
    eprintln!(
        "64 x 1 h replays on medium: {serial:.2} s serial ({:.0} ms each), {par:.2} s on {threads} threads",
        serial * 1e3 / 64.0
    );
}
