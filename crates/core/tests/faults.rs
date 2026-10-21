//! The fault catalog end to end (plan row 2.4, `crate::faults`).
//!
//! - Faults are opt-in: the healthy medium hash is pinned.
//! - Every kind starts and ends on schedule, emits its events, and restores
//!   its healthy counterpart; parcels are conserved throughout.
//! - Topology faults: robots never enter closed cells, cut-off robots record
//!   `CellBlocked`, chains ending at a dead robot are rooted at its fault.
//! - Counterfactual independence: removing a fault never shifts the random
//!   draws of parcels it does not touch, and a surge's own draws do not
//!   depend on its position in the schedule.
//! - Faults participate in the state hash; runtime scheduling is checked.
//! - A persisting wait-for cycle is reported as a deadlock exactly once.

use switchyard_core::Tick;
use switchyard_core::config::{ConfigError, ScenarioConfig};
use switchyard_core::events::{ChuteState, Event, WaitCause};
use switchyard_core::faults::{FaultKind, FaultSpec};
use switchyard_core::grid::{Cell, Pos};
use switchyard_core::hash::Fnv1a;
use switchyard_core::ids::{ChuteId, FaultId, RobotId, StationId};
use switchyard_core::planner::{PlanInput, PlanOutput, Planner};
use switchyard_core::presets::{self, MEDIUM_TOML, SMALL_TOML};
use switchyard_core::sim::Sim;
use switchyard_core::world::{Goal, Phase, RobotState};

fn small(seed: u64) -> ScenarioConfig {
    let mut cfg = ScenarioConfig::from_toml_str(SMALL_TOML).unwrap();
    cfg.seed = seed;
    cfg
}

fn fault(at: Tick, duration: Tick, kind: FaultKind) -> FaultSpec {
    FaultSpec { at, duration, kind }
}

fn run(cfg: &ScenarioConfig, ticks: u32) -> (Sim, Vec<Event>) {
    let mut sim = Sim::new(cfg).unwrap();
    let mut events = Vec::new();
    for _ in 0..ticks {
        sim.step(&mut events);
    }
    (sim, events)
}

/// Conservation (arrived = queued + in flight + delivered) after every tick.
fn run_conserving(cfg: &ScenarioConfig, ticks: u32, mut check: impl FnMut(&Sim, &[Event])) {
    let mut sim = Sim::new(cfg).unwrap();
    let mut events = Vec::new();
    for _ in 0..ticks {
        events.clear();
        sim.step(&mut events);
        let s = sim.stats();
        assert_eq!(s.arrived, s.queued + s.in_flight + s.delivered, "{s:?}");
        let phases = &sim.world().parcels.phase;
        let on_robot = phases
            .iter()
            .filter(|p| {
                matches!(
                    p,
                    Phase::Loading | Phase::Travelling | Phase::Blocked | Phase::ChuteWait
                )
            })
            .count() as u64;
        assert_eq!(s.in_flight, on_robot, "tick {}", s.tick);
        check(&sim, &events);
    }
}

fn fault_events(events: &[Event]) -> Vec<(Tick, bool, FaultId)> {
    events
        .iter()
        .filter_map(|e| match *e {
            Event::FaultStarted { tick, fault } => Some((tick, true, fault)),
            Event::FaultEnded { tick, fault } => Some((tick, false, fault)),
            _ => None,
        })
        .collect()
}

#[test]
fn healthy_medium_hash_is_unchanged_by_the_fault_machinery() {
    // The CP1 reference (docs/checkpoints/v1-cp1.md): faults are opt-in.
    let cfg = ScenarioConfig::from_toml_str(MEDIUM_TOML).unwrap();
    let (sim, _) = run(&cfg, 14_400);
    assert_eq!(format!("{:016x}", sim.state_hash()), "747248e3e7c20e9b");
}

#[test]
fn chaos_presets_are_the_medium_floor_plus_a_fault_schedule() {
    let medium = ScenarioConfig::from_toml_str(MEDIUM_TOML).unwrap();
    for (name, text) in presets::CHAOS {
        let cfg = ScenarioConfig::from_toml_str(text).unwrap();
        assert_eq!(cfg.name, *name);
        assert!(!cfg.faults.is_empty(), "{name} has no faults");
        // Only the name and the schedule differ from the medium hub.
        let mut healthy = cfg.clone();
        healthy.faults.clear();
        healthy.name = medium.name.clone();
        assert_eq!(healthy, medium, "{name}");
        let again = ScenarioConfig::from_toml_str(&cfg.to_toml_string().unwrap()).unwrap();
        assert_eq!(again, cfg, "{name} round trip");
        assert!(Sim::new(&cfg).is_ok(), "{name} builds");
        assert_eq!(presets::by_name(name), Some(*text));
    }
}

#[test]
fn a_robot_failure_stops_the_robot_and_its_parcel_until_repair() {
    let mut cfg = small(3);
    // Robot 5 fails at tick 400 for 300 ticks.
    cfg.faults.push(fault(
        400,
        300,
        FaultKind::RobotFailure { robot: RobotId(5) },
    ));
    let mut pos = None;
    let mut parcel = None;
    run_conserving(&cfg, 1_200, |sim, events| {
        let t = sim.tick() - 1;
        let w = sim.world();
        let state = w.robots.state[5];
        if (400..700).contains(&t) {
            assert_eq!(state, RobotState::Failed, "tick {t}");
            let here = w.robots.pos[5];
            assert_eq!(*pos.get_or_insert(here), here, "a dead robot moved");
            let p = *parcel.get_or_insert(w.robots.parcel[5]);
            assert_eq!(w.robots.parcel[5], p, "the parcel left the dead robot");
            if let Some(p) = p {
                assert_ne!(w.parcels.phase[p.index()], Phase::Dropped);
            }
        } else {
            assert_ne!(state, RobotState::Failed, "tick {t}");
        }
        for &(tick, started, f) in &fault_events(events) {
            assert_eq!(f, FaultId(0));
            assert_eq!(tick, if started { 400 } else { 700 });
        }
    });
}

#[test]
fn a_repaired_robot_resumes_its_previous_state() {
    let mut cfg = small(3);
    cfg.faults.push(fault(
        400,
        300,
        FaultKind::RobotFailure { robot: RobotId(5) },
    ));
    let mut sim = Sim::new(&cfg).unwrap();
    let mut events = Vec::new();
    for _ in 0..400 {
        sim.step(&mut events);
    }
    let before = sim.world().robots.state[5];
    for _ in 400..700 {
        sim.step(&mut events);
    }
    assert_eq!(sim.world().robots.state[5], RobotState::Failed);
    events.clear();
    sim.step(&mut events);
    assert!(events.contains(&Event::FaultEnded {
        tick: 700,
        fault: FaultId(0)
    }));
    // Restored at the start of tick 700, before that tick's own transitions.
    let restored = events.iter().find_map(|e| match *e {
        Event::RobotState {
            robot: RobotId(5),
            state,
            ..
        } => Some(state),
        _ => None,
    });
    assert_eq!(restored, Some(before));
}

#[test]
fn a_chute_jam_refuses_drops_and_robots_wait_on_the_chute() {
    let mut cfg = small(4);
    // Chute 0 serves the most popular destination (popularity mapping or
    // round robin, destination 0 maps to a busy chute either way).
    let (sim, _) = run(&cfg, 1);
    let chute = sim.world().destination_chute[0];
    cfg.faults
        .push(fault(200, 600, FaultKind::ChuteJam { chute }));
    let c = chute.index();
    let mut fill = None;
    let mut waits = 0;
    let mut changes = Vec::new();
    run_conserving(&cfg, 1_200, |sim, events| {
        let t = sim.tick() - 1;
        let ch = &sim.world().chutes;
        if (200..800).contains(&t) {
            assert_eq!(ch.state[c], ChuteState::Jammed, "tick {t}");
            // Nothing new goes in (robots already dropping finish).
            if t >= 210 {
                assert_eq!(*fill.get_or_insert(ch.fill[c]), ch.fill[c], "tick {t}");
            }
        } else {
            assert_ne!(ch.state[c], ChuteState::Jammed, "tick {t}");
        }
        for e in events {
            match *e {
                Event::RobotWaited {
                    direct: WaitCause::ChuteUnavailable(x),
                    ..
                } if x == chute && (200..800).contains(&t) => waits += 1,
                Event::ChuteChanged {
                    tick,
                    chute: x,
                    state,
                    ..
                } if x == chute && (tick == 200 || tick == 800) => changes.push((tick, state)),
                _ => {}
            }
        }
    });
    assert!(waits > 50, "robots waited {waits} robot-ticks on the jam");
    assert_eq!(changes[0], (200, ChuteState::Jammed));
    assert!(matches!(
        changes[1],
        (800, ChuteState::Open | ChuteState::Swapping)
    ));
}

#[test]
fn an_induction_outage_stops_dispatch_but_not_arrivals() {
    let mut cfg = small(5);
    let station = StationId(1);
    cfg.faults
        .push(fault(300, 900, FaultKind::InductionOutage { station }));
    let (healthy, healthy_events) = run(&small(5), 1_500);
    let (sim, events) = run(&cfg, 1_500);
    let arrivals = |ev: &[Event]| {
        ev.iter()
            .filter(|e| matches!(e, Event::ParcelArrived { station: s, .. } if *s == station))
            .count()
    };
    assert_eq!(arrivals(&events), arrivals(&healthy_events));
    // No robot is assigned at the station during the outage.
    let assigned = events.iter().any(|e| {
        matches!(*e, Event::ParcelAssigned { tick, station: s, .. }
            if s == station && (300..1_200).contains(&tick))
    });
    assert!(!assigned, "the station dispatched during its outage");
    // Its queue backs up, and is worked again afterwards.
    assert!(
        sim.stats().delivered < healthy.stats().delivered,
        "{} vs {}",
        sim.stats().delivered,
        healthy.stats().delivered
    );
    assert!(events.iter().any(|e| {
        matches!(*e, Event::ParcelAssigned { tick, station: s, .. } if s == station && tick >= 1_200)
    }));
}

/// Free cells of one aisle column of the small floor (x = 5, rows 2..=12).
fn aisle(sim: &Sim) -> Vec<Pos> {
    let g = &sim.layout().grid;
    (2..=12)
        .map(|y| Pos::new(5, y))
        .filter(|&p| g.at(p) == Cell::Free)
        .collect()
}

#[test]
fn robots_never_enter_a_blocked_aisle() {
    let cfg0 = small(6);
    let (probe, _) = run(&cfg0, 1);
    let cells = aisle(&probe);
    assert!(cells.len() >= 8, "{cells:?}");
    let idx: Vec<u32> = cells.iter().map(|&p| probe.layout().grid.idx(p)).collect();
    let mut cfg = cfg0.clone();
    cfg.faults.push(fault(
        200,
        800,
        FaultKind::AisleBlock {
            cells: cells.clone(),
        },
    ));
    let mut prev = probe.world().robots.pos.clone();
    let mut entered_before = 0;
    run_conserving(&cfg, 1_400, |sim, _| {
        let t = sim.tick() - 1;
        let pos = &sim.world().robots.pos;
        for (r, (&a, &b)) in prev.iter().zip(pos).enumerate() {
            if a != b && idx.contains(&b) {
                assert!(
                    !(200..1_000).contains(&t),
                    "tick {t}: R{r} entered blocked cell {b}"
                );
                entered_before += 1;
            }
        }
        prev.clone_from(pos);
    });
    assert!(entered_before > 0, "the aisle is used when open");
}

#[test]
fn a_cut_off_goal_records_cell_blocked_instead_of_wandering() {
    let cfg0 = small(7);
    let (probe, _) = run(&cfg0, 1);
    // Close every access cell of the chute serving destination 0.
    let chute = probe.world().destination_chute[0];
    let g = &probe.layout().grid;
    let cells: Vec<Pos> = probe
        .layout()
        .chute_access(chute)
        .into_iter()
        .map(|c| g.pos(c))
        .collect();
    let mut cfg = cfg0.clone();
    cfg.faults
        .push(fault(300, 600, FaultKind::AisleBlock { cells }));
    let mut cut_off = 0;
    run_conserving(&cfg, 1_200, |sim, events| {
        for e in events {
            if let Event::RobotWaited {
                tick,
                robot,
                direct: WaitCause::CellBlocked(f),
                root,
            } = *e
            {
                assert_eq!(f, FaultId(0));
                assert_eq!(root, WaitCause::CellBlocked(f));
                assert!((300..900).contains(&tick));
                let w = sim.world();
                assert_eq!(w.robots.goal[robot.index()], Some(Goal::Chute(chute)));
                if let Some(p) = w.robots.parcel[robot.index()] {
                    assert_eq!(w.parcels.phase[p.index()], Phase::Blocked);
                }
                cut_off += 1;
            }
        }
    });
    assert!(cut_off > 0, "no robot was cut off");
}

/// The root of `r`'s chain by definition, walked naively; chains ending at
/// a failed robot are rooted at its failure.
fn reference_root(
    waits: &[Option<WaitCause>],
    failed: &[Option<FaultId>],
    r: usize,
) -> Option<WaitCause> {
    let mut path = vec![r];
    let mut cause = waits[r]?;
    loop {
        let WaitCause::BlockedBy(b) = cause else {
            return Some(cause);
        };
        if let Some(f) = failed[b.index()] {
            return Some(WaitCause::CellBlocked(f));
        }
        let Some(next) = waits[b.index()] else {
            return Some(cause);
        };
        if let Some(i) = path.iter().position(|&p| p == b.index()) {
            let lowest = path[i..].iter().min().copied().unwrap();
            return Some(WaitCause::Cycle(RobotId(u32::try_from(lowest).unwrap())));
        }
        path.push(b.index());
        cause = next;
    }
}

#[test]
fn chains_ending_at_dead_robots_are_rooted_at_their_failure() {
    let mut cfg = ScenarioConfig::from_toml_str(MEDIUM_TOML).unwrap();
    // Five robots fail together at tick 300.
    for (i, r) in [3, 40, 77, 120, 160].into_iter().enumerate() {
        cfg.faults.push(fault(
            300,
            600 + 10 * u32::try_from(i).unwrap(),
            FaultKind::RobotFailure { robot: RobotId(r) },
        ));
    }
    let mut sim = Sim::new(&cfg).unwrap();
    let mut events = Vec::new();
    let mut at_dead = 0;
    for _ in 0..1_000 {
        events.clear();
        sim.step(&mut events);
        let failed: Vec<Option<FaultId>> = (0..sim.world().robots.len())
            .map(|r| {
                (sim.world().robots.state[r] == RobotState::Failed).then(|| {
                    let i = sim
                        .faults()
                        .iter()
                        .position(|f| {
                            f.kind
                                == FaultKind::RobotFailure {
                                    robot: RobotId(u32::try_from(r).unwrap()),
                                }
                        })
                        .unwrap();
                    FaultId(u32::try_from(i).unwrap())
                })
            })
            .collect();
        for e in &events {
            if let Event::RobotWaited { robot, root, .. } = *e {
                let expected = reference_root(sim.wait_for(), &failed, robot.index());
                let young = match expected {
                    Some(WaitCause::Cycle(m)) => Some(WaitCause::BlockedBy(m)),
                    other => other,
                };
                assert!(
                    Some(root) == expected || Some(root) == young,
                    "tick {}: root of {robot}: {root:?}, expected {expected:?}",
                    sim.tick() - 1
                );
                if root.fault().is_some() {
                    at_dead += 1;
                }
            }
        }
    }
    // Dead robots' cells are closed, so live robots mostly route around
    // them; the unit tests in `causality` cover chains that end at one.
    assert!(at_dead > 0, "no wait was ever blamed on a failure");
}

/// Regular parcels by `(station, key)`: arrival tick, destination, target.
type Keyed = Vec<((u16, u64), (Tick, u16, Option<u16>))>;

fn keyed(sim: &Sim, surge: bool) -> Keyed {
    let p = &sim.world().parcels;
    let mut v: Keyed = (0..p.len())
        .filter(|&i| (p.key[i] >= 1 << 62) == surge)
        .map(|i| {
            (
                (p.station[i].0, p.key[i]),
                (p.arrived[i], p.destination[i].0, p.target[i].map(|c| c.0)),
            )
        })
        .collect();
    v.sort_unstable();
    v
}

#[test]
fn removing_a_fault_leaves_unrelated_draws_untouched() {
    // ADR 0002 §3 / ADR 0005, extended to the fault catalog: a replay
    // without one fault differs from the original only through that fault.
    let mut cfg = small(21);
    cfg.scanner.error_rate_ppm = 50_000;
    let surge = fault(
        500,
        600,
        FaultKind::VolumeSurge {
            station: Some(StationId(0)),
            factor_pct: 250,
        },
    );
    let drift = fault(
        400,
        400,
        FaultKind::ScannerDrift {
            station: StationId(2),
            error_rate_ppm: 400_000,
        },
    );
    let others = [
        fault(300, 500, FaultKind::RobotFailure { robot: RobotId(7) }),
        fault(600, 300, FaultKind::ChuteJam { chute: ChuteId(1) }),
        fault(
            700,
            300,
            FaultKind::InductionOutage {
                station: StationId(3),
            },
        ),
    ];
    let mut all = cfg.clone();
    all.faults = vec![surge.clone(), drift.clone()];
    all.faults.extend(others.iter().cloned());
    let ticks = 2_000;

    let (full, _) = run(&all, ticks);
    let (healthy, _) = run(&cfg, ticks);
    // Removing every fault: the regular parcels arrive at the same ticks
    // with the same destinations; scans differ only at the drifted station.
    let (a, b) = (keyed(&full, false), keyed(&healthy, false));
    assert_eq!(a.len(), b.len(), "regular arrivals changed");
    let mut scans = 0;
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(x.0, y.0);
        assert_eq!(x.1.0, y.1.0, "arrival of {:?}", x.0);
        assert_eq!(x.1.1, y.1.1, "destination of {:?}", x.0);
        if x.0.0 != 2
            && let (Some(s), Some(t)) = (x.1.2, y.1.2)
        {
            assert_eq!(s, t, "scan of {:?}", x.0);
            scans += 1;
        }
    }
    assert!(scans > 100, "{scans}");

    // Removing the surge leaves every other parcel exactly as it was.
    let (no_surge, no_surge_events) = run(&all.without_fault(FaultId(0)), ticks);
    assert_eq!(
        keyed(&no_surge, false)
            .iter()
            .map(|e| (e.0, e.1.0, e.1.1))
            .collect::<Vec<_>>(),
        a.iter().map(|e| (e.0, e.1.0, e.1.1)).collect::<Vec<_>>()
    );

    // Fault ids are stable under the intervention: the replay without the
    // surge reports the other faults under the same ids at the same ticks.
    let (_, full_events) = run(&all, ticks);
    let others_events: Vec<_> = fault_events(&full_events)
        .into_iter()
        .filter(|e| e.2 != FaultId(0))
        .collect();
    assert_eq!(fault_events(&no_surge_events), others_events);
    assert!(no_surge.fault_enabled(FaultId(1)) && !no_surge.fault_enabled(FaultId(0)));

    // The surge's own parcels do not depend on the other faults.
    let mut alone_cfg = all.clone();
    for i in 1..5 {
        alone_cfg = alone_cfg.without_fault(FaultId(i));
    }
    let (alone, _) = run(&alone_cfg, ticks);
    drop((drift, surge));
    let (s1, s2) = (keyed(&full, true), keyed(&alone, true));
    assert!(s1.len() > 20, "{} surge parcels", s1.len());
    assert_eq!(
        s1.iter().map(|e| (e.0, e.1.0, e.1.1)).collect::<Vec<_>>(),
        s2.iter().map(|e| (e.0, e.1.0, e.1.1)).collect::<Vec<_>>()
    );
}

#[test]
fn a_volume_surge_adds_arrivals_at_the_requested_factor() {
    let mut cfg = small(8);
    cfg.faults.push(fault(
        0,
        7_200,
        FaultKind::VolumeSurge {
            station: None,
            factor_pct: 200,
        },
    ));
    let (healthy, _) = run(&small(8), 7_200);
    let (surged, _) = run(&cfg, 7_200);
    let (h, s) = (healthy.stats().arrived, surged.stats().arrived);
    // Double the rate over an hour: 2x within Poisson noise.
    assert!(s * 100 > h * 185 && s * 100 < h * 215, "{h} -> {s}");
}

#[test]
fn scanner_drift_raises_misreads_only_at_its_station() {
    let mut cfg = small(9);
    cfg.faults.push(fault(
        0,
        3_000,
        FaultKind::ScannerDrift {
            station: StationId(0),
            error_rate_ppm: 500_000,
        },
    ));
    let (sim, events) = run(&cfg, 3_000);
    let mut by_station = [0u32; 4];
    for e in &events {
        if let Event::ParcelMisSorted { parcel, .. } = *e {
            by_station[sim.world().parcels.station[parcel.index()].index()] += 1;
        }
    }
    assert!(by_station[0] > 50, "{by_station:?}");
    assert_eq!(by_station[1..], [0, 0, 0], "{by_station:?}");
}

#[test]
fn faults_participate_in_the_state_hash() {
    let healthy = Sim::new(&small(1)).unwrap().state_hash();
    let mut cfg = small(1);
    cfg.faults
        .push(fault(10_000, 5, FaultKind::ChuteJam { chute: ChuteId(2) }));
    // Even a fault that has not started yet is part of the state.
    let pending = Sim::new(&cfg).unwrap().state_hash();
    assert_ne!(healthy, pending);
    let mut other = cfg.clone();
    other.faults[0].duration = 6;
    assert_ne!(pending, Sim::new(&other).unwrap().state_hash());
    // And the same schedule always gives the same run.
    let (a, ea) = run(&cfg, 500);
    let (b, eb) = run(&cfg, 500);
    assert_eq!(a.state_hash(), b.state_hash());
    assert_eq!(ea, eb);
}

#[test]
fn faults_scheduled_at_run_time_match_scenario_faults() {
    let spec = fault(300, 200, FaultKind::RobotFailure { robot: RobotId(2) });
    let mut cfg = small(10);
    cfg.faults.push(spec.clone());
    let (from_config, e1) = run(&cfg, 800);

    let mut sim = Sim::new(&small(10)).unwrap();
    let mut e2 = Vec::new();
    for t in 0..800 {
        if t == 100 {
            assert_eq!(sim.schedule_fault(spec.clone()), Ok(FaultId(0)));
        }
        sim.step(&mut e2);
    }
    assert_eq!(e1, e2);
    assert_eq!(from_config.state_hash(), sim.state_hash());
}

#[test]
fn invalid_faults_are_rejected() {
    let mut sim = Sim::new(&small(11)).unwrap();
    let mut events = Vec::new();
    for _ in 0..50 {
        sim.step(&mut events);
    }
    let bad = [
        fault(10, 5, FaultKind::ChuteJam { chute: ChuteId(0) }),
        fault(
            60,
            5,
            FaultKind::RobotFailure {
                robot: RobotId(999),
            },
        ),
        fault(
            60,
            5,
            FaultKind::ChuteJam {
                chute: ChuteId(999),
            },
        ),
        fault(
            60,
            5,
            FaultKind::InductionOutage {
                station: StationId(9),
            },
        ),
        fault(
            60,
            5,
            FaultKind::AisleBlock {
                cells: vec![Pos::new(0, 0)],
            },
        ),
        fault(60, 0, FaultKind::ChuteJam { chute: ChuteId(0) }),
    ];
    let hash = sim.state_hash();
    for f in bad {
        let err = sim.schedule_fault(f.clone());
        assert!(
            matches!(
                err,
                Err(ConfigError::BadFault(_) | ConfigError::ZeroDuration(_))
            ),
            "{f:?}: {err:?}"
        );
    }
    assert_eq!(sim.state_hash(), hash, "a rejected fault changed the sim");
    assert!(sim.faults().is_empty());
    // A scenario with a bad fault does not build.
    let mut cfg = small(11);
    cfg.faults
        .push(fault(5, 5, FaultKind::RobotFailure { robot: RobotId(40) }));
    assert!(matches!(Sim::new(&cfg), Err(ConfigError::BadFault(_))));
}

/// A planner that never moves anyone; robots 0 and 1 block each other
/// (a head-on 2-cycle) whenever both are movable.
#[derive(Clone, Debug)]
struct HeadOn;

impl Planner for HeadOn {
    fn plan(&mut self, input: &PlanInput<'_>, out: &mut PlanOutput) {
        out.next = input.pos.to_vec();
        out.cause = vec![None; input.pos.len()];
        if input.movable[0] && input.movable[1] {
            out.cause[0] = Some(WaitCause::BlockedBy(RobotId(1)));
            out.cause[1] = Some(WaitCause::BlockedBy(RobotId(0)));
        }
    }

    fn hash_state(&self, _: &mut Fnv1a) {}

    fn box_clone(&self) -> Box<dyn Planner> {
        Box::new(self.clone())
    }
}

#[test]
fn a_persisting_cycle_is_reported_once_as_a_deadlock() {
    let mut cfg = small(12);
    cfg.planner.deadlock_ticks = 7;
    let mut sim = Sim::with_planner(&cfg, Box::new(HeadOn)).unwrap();
    let mut events = Vec::new();
    for _ in 0..200 {
        sim.step(&mut events);
    }
    let deadlocks: Vec<_> = events
        .iter()
        .filter_map(|e| match *e {
            Event::DeadlockDetected {
                tick,
                robot,
                cycle,
                since,
            } => Some((tick, robot, cycle, since)),
            _ => None,
        })
        .collect();
    assert_eq!(
        deadlocks,
        vec![
            (6, RobotId(0), RobotId(0), 0),
            (6, RobotId(1), RobotId(0), 0)
        ]
    );
    // Until it has persisted, the meeting is rooted at the canonical member
    // like any robot that stayed put; from then on it is a deadlock.
    let mut deadlocked = 0;
    for e in &events {
        if let Event::RobotWaited {
            tick, robot, root, ..
        } = *e
            && robot.index() < 2
        {
            if tick < 6 {
                assert_eq!(root, WaitCause::BlockedBy(RobotId(0)), "tick {tick}");
            } else {
                assert_eq!(root, WaitCause::Cycle(RobotId(0)), "tick {tick}");
                deadlocked += 1;
            }
        }
    }
    assert!(deadlocked > 0);
}
