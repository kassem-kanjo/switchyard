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
            replay.disable_fault(id).unwrap();
            run_to(&mut replay, end);
            let (never, _) = run(&cfg.without_fault(id), end);
            assert_eq!(replay.state_hash(), never.state_hash(), "{name}: {id}");
            assert_eq!(replay.config(), never.config(), "{name}: {id}");
        }
    }
}

#[test]
fn disabling_an_active_fault_ends_it_cleanly_at_the_next_step() {
    let mut cfg = small(6);
    cfg.faults.push(fault(
        300,
        600,
        FaultKind::RobotFailure { robot: RobotId(4) },
    ));
    cfg.faults
        .push(fault(300, 600, FaultKind::ChuteJam { chute: ChuteId(1) }));
    let mut sim = Sim::new(&cfg).unwrap();
    run_to(&mut sim, 500);
    assert_eq!(sim.world().robots.state[4], RobotState::Failed);
    sim.disable_fault(FaultId(0)).unwrap();
    sim.disable_fault(FaultId(1)).unwrap();
    // Recorded as a truncation, so the config reproduces the replay.
    assert_eq!(sim.config().faults[0].duration, 200);
    assert_eq!(sim.config().faults[1].duration, 200);
    let mut events = Vec::new();
    sim.step(&mut events);
    assert!(events.contains(&Event::FaultEnded {
        tick: 500,
        fault: FaultId(0)
    }));
    assert!(events.contains(&Event::FaultEnded {
        tick: 500,
        fault: FaultId(1)
    }));
    assert_ne!(sim.world().robots.state[4], RobotState::Failed, "repaired");
    assert_ne!(
        sim.world().chutes.state[1],
        switchyard_core::events::ChuteState::Jammed
    );
    let later = run_to(&mut sim, 1_200);
    assert!(!later.iter().any(|e| matches!(e, Event::FaultEnded { .. })));
    let s = sim.stats();
    assert_eq!(s.arrived, s.queued + s.in_flight + s.delivered);
    // Ended or unknown faults cannot be disabled.
    assert!(sim.disable_fault(FaultId(0)).is_err());
    assert!(sim.disable_fault(FaultId(9)).is_err());
}

#[test]
fn disabling_an_active_fault_equals_truncating_it_for_every_kind() {
    let kinds = [
        FaultKind::RobotFailure { robot: RobotId(4) },
        FaultKind::AisleBlock {
            cells: (2..=12)
                .map(|y| switchyard_core::grid::Pos::new(5, y))
                .collect(),
        },
        FaultKind::ChuteJam { chute: ChuteId(1) },
        FaultKind::InductionOutage {
            station: StationId(1),
        },
        FaultKind::VolumeSurge {
            station: None,
            factor_pct: 250,
        },
        FaultKind::ScannerDrift {
            station: StationId(0),
            error_rate_ppm: 300_000,
        },
    ];
    for kind in kinds {
        let mut cfg = small(11);
        // Keep the aisle to open floor on this layout.
        let kind = match kind {
            FaultKind::AisleBlock { cells } => {
                let probe = Sim::new(&cfg).unwrap();
                let g = &probe.layout().grid;
                FaultKind::AisleBlock {
                    cells: cells
                        .into_iter()
                        .filter(|&c| g.at(c) == switchyard_core::grid::Cell::Free)
                        .collect(),
                }
            }
            other => other,
        };
        cfg.faults.push(fault(300, 900, kind.clone()));
        let mut replay = Sim::new(&cfg).unwrap();
        run_to(&mut replay, 650);
        replay.disable_fault(FaultId(0)).unwrap();
        run_to(&mut replay, 1_850);
        let mut truncated = cfg.clone();
        truncated.faults[0].duration = 350;
        let (fresh, _) = run(&truncated, 1_850);
        assert_eq!(replay.state_hash(), fresh.state_hash(), "{}", kind.name());
        assert_eq!(replay.config(), &truncated, "{}", kind.name());
    }
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
        live.apply_intervention(iv).unwrap();
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
    checkpoint.apply_intervention(&add).unwrap();
    run_to(&mut checkpoint, 1_200);
    assert_eq!(
        checkpoint.state_hash(),
        run(&cfg.apply(&add).unwrap(), 1_200).0.state_hash()
    );
    // ...but not in the past.
    assert!(
        checkpoint
            .apply_intervention(&Intervention::AddRobots { robots: 1, at: 3 },)
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
            replay.disable_fault(id).unwrap();
            let mut reference = actual.clone();
            let at = replay.reconverge_with(&mut reference, horizon);
            pairs += 1;
            met += usize::from(at.is_some());
            eprintln!("{name} without {id}: re-converged at {at:?} (horizon {horizon})");
        }
        // And the twin: no faults at all.
        let mut twin = actual.clone();
        for i in 0..cfg.faults.len() {
            twin.disable_fault(FaultId(u32::try_from(i).unwrap()))
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

#[test]
fn a_parcel_waiting_out_a_jam_is_blamed_on_every_tick_in_one_span() {
    // two-dead-robots-and-a-jam: C289 jams for ticks 3600..5400 (F0).
    let (_, text) = presets::CHAOS
        .iter()
        .find(|(n, _)| *n == "two-dead-robots-and-a-jam")
        .unwrap();
    let cfg = ScenarioConfig::from_toml_str(text).unwrap();
    let jam = cfg.faults[0].clone();
    let chute = match jam.kind {
        FaultKind::ChuteJam { chute } => chute,
        _ => unreachable!("F0 is the jam"),
    };
    let mut sim = Sim::new(&cfg).unwrap();
    let mut events = Vec::new();
    run_to(&mut sim, jam.at + 200);
    // A parcel that is already waiting at the jammed chute.
    let w = sim.world();
    let parcel = (0..w.parcels.len())
        .find(|&i| w.parcels.phase[i] == Phase::ChuteWait && w.parcels.target[i] == Some(chute))
        .expect("a parcel waits at the jammed chute");
    let (mut blamed, mut phase_changes) = (0, 0);
    while sim.tick() < jam.end() {
        events.clear();
        sim.step(&mut events);
        for e in &events {
            match *e {
                Event::ParcelDelayed {
                    parcel: p,
                    cause,
                    root,
                    ..
                } if p.index() == parcel => {
                    assert_eq!(
                        cause,
                        switchyard_core::events::WaitCause::ChuteUnavailable(chute)
                    );
                    assert_eq!(root, switchyard_core::events::WaitCause::Fault(FaultId(0)));
                    blamed += 1;
                }
                Event::ParcelPhase { parcel: p, .. } if p.index() == parcel => phase_changes += 1,
                _ => {}
            }
        }
    }
    let window = jam.end() - (jam.at + 200);
    assert_eq!(blamed, window, "every tick of the jam is blamed on it");
    assert_eq!(phase_changes, 0, "one ChuteWait span through the jam");
}

#[test]
fn clear_after_is_an_intervention_and_clears_sooner() {
    let cfg = small(4);
    let t0 = 600;
    // A robot loading at a station when it fails.
    let mut probe = Sim::new(&cfg).unwrap();
    run_to(&mut probe, t0);
    let r = (0..probe.world().robots.len())
        .find(|&r| probe.world().robots.state[r] == RobotState::Loading)
        .unwrap();
    let mut faulted = cfg.clone();
    faulted.faults.push(fault(
        t0,
        1_200,
        FaultKind::RobotFailure {
            robot: RobotId(u32::try_from(r).unwrap()),
        },
    ));
    let cleared = |cfg: &ScenarioConfig| {
        let (_, events) = run(cfg, t0 + 1_000);
        events.iter().find_map(|e| match *e {
            Event::RobotCleared { tick, .. } => Some(tick),
            _ => None,
        })
    };
    assert_eq!(cleared(&faulted), Some(t0 + 600), "default: 5 minutes");
    let sooner = faulted
        .apply(&Intervention::ClearAfter { ticks: 120 })
        .unwrap();
    assert_eq!(sooner.recovery.clear_after_ticks, 120);
    assert_eq!(cleared(&sooner), Some(t0 + 120));
    // At runtime, a robot already overdue is cleared at the next step.
    let mut live = Sim::new(&faulted).unwrap();
    run_to(&mut live, t0 + 200);
    live.apply_intervention(&Intervention::ClearAfter { ticks: 120 })
        .unwrap();
    let events = run_to(&mut live, t0 + 201);
    assert!(events.iter().any(|e| matches!(*e,
        Event::RobotCleared { tick, .. } if tick == t0 + 200)));
}

#[test]
fn added_robots_avoid_closed_cells_and_report_a_shortfall() {
    let mut cfg = small(5);
    // Close a stretch of the parking lane, where new robots are placed first.
    let probe = Sim::new(&cfg).unwrap();
    let g = &probe.layout().grid;
    let parking: Vec<_> = probe
        .layout()
        .parking
        .iter()
        .take(10)
        .map(|&c| g.pos(c))
        .collect();
    let closed: Vec<u32> = parking.iter().map(|&p| g.idx(p)).collect();
    cfg.faults
        .push(fault(10, 2_000, FaultKind::AisleBlock { cells: parking }));
    let cfg = cfg
        .apply(&Intervention::AddRobots {
            robots: 100_000,
            at: 100,
        })
        .unwrap();
    let (sim, events) = run(&cfg, 101);
    let added = events
        .iter()
        .find_map(|e| match *e {
            Event::RobotsAdded {
                tick: 100,
                requested,
                added,
            } => Some((requested, added)),
            _ => None,
        })
        .expect("a RobotsAdded event");
    assert_eq!(added.0, 100_000);
    assert!(added.1 < 100_000 && added.1 > 0, "{added:?}");
    assert_eq!(sim.world().robots.len(), 40 + added.1 as usize);
    // None placed on a closed cell.
    for &p in &sim.world().robots.pos[40..] {
        assert!(!closed.contains(&p));
    }
}

#[test]
fn a_mid_run_remap_does_not_mis_sort_parcels_scanned_before_it() {
    let cfg = small(6);
    let mut sim = Sim::new(&cfg).unwrap();
    run_to(&mut sim, 400);
    let old = sim.world().destination_chute[0];
    sim.apply_intervention(&Intervention::RemapDestination {
        destination: DestinationId(0),
        chute: ChuteId(if old.0 == 5 { 6 } else { 5 }),
    })
    .unwrap();
    let events = run_to(&mut sim, 1_500);
    // The scanner never misreads here (0 ppm), so nothing is mis-sorted,
    // even parcels for destination 0 scanned before the remap.
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::ParcelMisSorted { .. }))
    );
    let p = &sim.world().parcels;
    let to_old = (0..p.len())
        .filter(|&i| p.destination[i] == DestinationId(0) && p.chute[i] == Some(old))
        .count();
    assert!(to_old > 0, "no parcel was scanned before the remap");
}

#[test]
fn equal_live_hashes_have_identical_physical_futures() {
    // Two runs that differ (a fault in one), whose live hashes meet: from
    // then on they deliver the same parcels to the same chutes at the same
    // ticks.
    let mut cfg = small(10);
    cfg.faults.push(fault(
        50,
        100,
        FaultKind::ScannerDrift {
            station: StationId(0),
            error_rate_ppm: 0,
        },
    ));
    let mut a = Sim::new(&cfg).unwrap();
    let mut b = Sim::new(&cfg.without_fault(FaultId(0))).unwrap();
    let met = b.reconverge_with(&mut a, 1_000).expect("they meet");
    let deliveries = |sim: &mut Sim| {
        let mut out = Vec::new();
        let events = run_to(sim, met + 800);
        for e in events {
            if let Event::ParcelDropped {
                tick,
                parcel,
                robot,
                chute,
            } = e
            {
                out.push((tick, parcel, robot, chute));
            }
        }
        out
    };
    let (da, db) = (deliveries(&mut a), deliveries(&mut b));
    assert!(da.len() > 50);
    assert_eq!(da, db);
    assert_eq!(a.live_hash(), b.live_hash());
}

/// CP3 relies on checkpoint -> disable_fault -> intervention being
/// reproducible on every platform; CI runs this on Linux, Windows, macOS.
#[test]
fn the_replay_path_hash_is_pinned() {
    let mut cfg = small(13);
    cfg.faults.push(fault(
        300,
        600,
        FaultKind::RobotFailure { robot: RobotId(4) },
    ));
    cfg.faults
        .push(fault(350, 600, FaultKind::ChuteJam { chute: ChuteId(2) }));
    cfg.faults.push(fault(
        400,
        600,
        FaultKind::VolumeSurge {
            station: Some(StationId(1)),
            factor_pct: 200,
        },
    ));
    let mut sim = Sim::new(&cfg).unwrap();
    run_to(&mut sim, 500);
    let mut replay = sim.clone();
    replay.disable_fault(FaultId(1)).unwrap();
    replay
        .apply_intervention(&Intervention::RemapDestination {
            destination: DestinationId(1),
            chute: ChuteId(3),
        })
        .unwrap();
    replay
        .apply_intervention(&Intervention::AddRobots { robots: 3, at: 700 })
        .unwrap();
    run_to(&mut replay, 1_500);
    assert_eq!(format!("{:016x}", replay.state_hash()), "053601a2fbdb2cc8");
}
