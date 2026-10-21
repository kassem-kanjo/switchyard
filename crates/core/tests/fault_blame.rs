//! Fault blame (CP2 review): every parcel tick a fault costs is attributed
//! to that fault, and to the right one.
//!
//! - A failed robot holding a parcel reports `Fault(f)` every tick it is
//!   down, and its parcel is delayed by `f` in whatever phase it is in.
//! - A dead robot on a station cell cuts the station off: claims are given
//!   back, nobody is dispatched there, and its queue is blamed on the
//!   failure.
//! - A robot cut off by closed cells is blamed on the fault whose removal
//!   reconnects it, not on the lowest active one.
//! - Jams and outages resolve to their fault; queues behind surge parcels
//!   are blamed on the surge.
//! - The chaos presets: every fault receives delay, and every cut-off robot
//!   is blamed on the fault that cut it off (checked independently with one
//!   Dijkstra per goal).

use std::collections::BTreeMap;

use switchyard_core::Tick;
use switchyard_core::config::ScenarioConfig;
use switchyard_core::events::{Event, WaitCause};
use switchyard_core::faults::{FaultKind, FaultSpec};
use switchyard_core::grid::{CellIdx, Pos};
use switchyard_core::guidance::{DistanceTables, UNREACHABLE};
use switchyard_core::ids::{FaultId, ParcelId, RobotId, StationId};
use switchyard_core::presets::{self, SMALL_TOML};
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

/// The first robot in `state` (holding a parcel) at the start of tick `t`.
fn robot_in(cfg: &ScenarioConfig, t: Tick, state: RobotState) -> usize {
    let mut sim = Sim::new(cfg).unwrap();
    let mut events = Vec::new();
    for _ in 0..t {
        sim.step(&mut events);
    }
    let w = sim.world();
    (0..w.robots.len())
        .find(|&r| w.robots.state[r] == state && w.robots.parcel[r].is_some())
        .unwrap_or_else(|| panic!("no robot {state:?} at tick {t}"))
}

/// Delays of one tick by parcel; at most one per parcel, always naming a
/// fault.
fn delays(events: &[Event]) -> BTreeMap<ParcelId, (WaitCause, WaitCause)> {
    let mut m = BTreeMap::new();
    for e in events {
        if let Event::ParcelDelayed {
            parcel,
            cause,
            root,
            ..
        } = *e
        {
            assert!(root.fault().is_some(), "{parcel}: root {root:?}");
            assert!(m.insert(parcel, (cause, root)).is_none(), "{parcel} twice");
        }
    }
    m
}

#[test]
fn a_failed_robot_holding_a_parcel_reports_its_fault_every_tick() {
    let cfg0 = small(3);
    let r = robot_in(&cfg0, 400, RobotState::ToChute);
    let mut cfg = cfg0.clone();
    cfg.faults.push(fault(
        400,
        300,
        FaultKind::RobotFailure {
            robot: RobotId(u32::try_from(r).unwrap()),
        },
    ));
    let mut sim = Sim::new(&cfg).unwrap();
    let mut events = Vec::new();
    let f = WaitCause::Fault(FaultId(0));
    let mut down = 0;
    for _ in 0..1_000 {
        events.clear();
        sim.step(&mut events);
        let t = sim.tick() - 1;
        let waited = events.iter().find_map(|e| match *e {
            Event::RobotWaited {
                robot,
                direct,
                root,
                ..
            } if robot.index() == r => Some((direct, root)),
            _ => None,
        });
        if (400..700).contains(&t) {
            assert_eq!(waited, Some((f, f)), "tick {t}");
            let parcel = sim.world().robots.parcel[r].unwrap();
            assert_eq!(delays(&events).get(&parcel), Some(&(f, f)), "tick {t}");
            down += 1;
        } else {
            assert_ne!(waited.map(|w| w.0), Some(f), "tick {t}");
        }
    }
    assert_eq!(down, 300);
}

#[test]
fn a_dead_robot_on_a_station_cuts_it_off_and_takes_the_blame() {
    let cfg0 = small(4);
    let t0 = 600;
    let r = robot_in(&cfg0, t0, RobotState::Loading);
    let mut probe = Sim::new(&cfg0).unwrap();
    let mut ev = Vec::new();
    for _ in 0..t0 {
        probe.step(&mut ev);
    }
    let w = probe.world();
    let station = (0..w.stations.cell.len())
        .find(|&s| w.stations.cell[s] == w.robots.pos[r])
        .expect("a loading robot stands on a station");
    let mut cfg = cfg0.clone();
    cfg.faults.push(fault(
        t0,
        900,
        FaultKind::RobotFailure {
            robot: RobotId(u32::try_from(r).unwrap()),
        },
    ));
    let mut sim = Sim::new(&cfg).unwrap();
    let mut events = Vec::new();
    let (mut queued_blamed, mut released) = (0, 0);
    for _ in 0..t0 + 900 {
        events.clear();
        let before: Vec<_> = sim.world().parcels.phase.clone();
        sim.step(&mut events);
        let t = sim.tick() - 1;
        if t < t0 {
            continue;
        }
        let w = sim.world();
        let d = delays(&events);
        for e in &events {
            match *e {
                Event::ParcelAssigned { station: s, .. } => {
                    assert_ne!(
                        s.index(),
                        station,
                        "tick {t}: dispatched to a cut-off station"
                    );
                }
                Event::ParcelPhase {
                    parcel,
                    phase: Phase::Queued,
                    ..
                } if before.get(parcel.index()) == Some(&Phase::AwaitingRobot) => released += 1,
                _ => {}
            }
        }
        for &p in &w.stations.queue[station] {
            if w.parcels.robot[p.index()].is_none() {
                let blame = WaitCause::CellBlocked(FaultId(0));
                assert_eq!(d.get(&p), Some(&(blame, blame)), "tick {t}: {p}");
                queued_blamed += 1;
            }
        }
    }
    assert!(released > 0, "no claim was given back");
    assert!(queued_blamed > 100, "{queued_blamed}");
}

/// Free cells of the small floor's column `x` from row 2 to row 12.
fn column(sim: &Sim, x: u16) -> Vec<Pos> {
    let g = &sim.layout().grid;
    (2..=12)
        .map(|y| Pos::new(x, y))
        .filter(|&p| g.at(p) == switchyard_core::grid::Cell::Free)
        .collect()
}

#[test]
fn a_cut_off_robot_is_blamed_on_the_fault_that_cut_it_off() {
    let cfg0 = small(7);
    let probe = Sim::new(&cfg0).unwrap();
    let chute = probe.world().destination_chute[0];
    let g = &probe.layout().grid;
    let access: Vec<Pos> = probe
        .layout()
        .chute_access(chute)
        .into_iter()
        .map(|c| g.pos(c))
        .collect();
    // F0: an aisle far from that chute (lower id, active, harmless to it);
    // F1: closes every access cell of the chute.
    let far = column(
        &probe,
        if g.pos(probe.layout().chutes[chute.index()]).x > 15 {
            4
        } else {
            23
        },
    );
    let mut cfg = cfg0.clone();
    cfg.faults
        .push(fault(300, 600, FaultKind::AisleBlock { cells: far }));
    cfg.faults
        .push(fault(300, 600, FaultKind::AisleBlock { cells: access }));
    let mut sim = Sim::new(&cfg).unwrap();
    let mut events = Vec::new();
    let mut cut = 0;
    for _ in 0..1_000 {
        events.clear();
        sim.step(&mut events);
        for e in &events {
            if let Event::RobotWaited {
                direct: WaitCause::CellBlocked(f),
                ..
            } = *e
            {
                assert_eq!(f, FaultId(1), "blamed on the wrong fault");
                cut += 1;
            }
        }
    }
    assert!(cut > 0, "no robot was cut off");
}

#[test]
fn jams_and_outages_are_blamed_on_their_faults() {
    let cfg0 = small(5);
    let chute = Sim::new(&cfg0).unwrap().world().destination_chute[0];
    let mut cfg = cfg0.clone();
    cfg.faults
        .push(fault(200, 600, FaultKind::ChuteJam { chute }));
    cfg.faults.push(fault(
        300,
        600,
        FaultKind::InductionOutage {
            station: StationId(1),
        },
    ));
    let mut sim = Sim::new(&cfg).unwrap();
    let mut events = Vec::new();
    let (mut jam, mut outage) = (0, 0);
    for _ in 0..1_200 {
        events.clear();
        sim.step(&mut events);
        let t = sim.tick() - 1;
        let d = delays(&events);
        for e in &events {
            if let Event::RobotWaited {
                direct: WaitCause::ChuteUnavailable(c),
                root,
                ..
            } = *e
                && c == chute
                && (200..800).contains(&t)
            {
                assert_eq!(root, WaitCause::Fault(FaultId(0)), "tick {t}");
                jam += 1;
            }
        }
        let w = sim.world();
        for &p in &w.stations.queue[1] {
            if w.parcels.robot[p.index()].is_none() {
                let got = d.get(&p).map(|x| x.1);
                if (300..900).contains(&t) {
                    assert_eq!(got, Some(WaitCause::Fault(FaultId(1))), "tick {t}");
                    outage += 1;
                } else {
                    assert_ne!(got, Some(WaitCause::Fault(FaultId(1))), "tick {t}");
                }
            }
        }
    }
    assert!(jam > 50 && outage > 100, "jam {jam}, outage {outage}");
}

#[test]
fn queues_behind_surge_parcels_are_blamed_on_the_surge() {
    let mut cfg = small(8);
    cfg.faults.push(fault(
        300,
        900,
        FaultKind::VolumeSurge {
            station: Some(StationId(0)),
            factor_pct: 400,
        },
    ));
    let mut sim = Sim::new(&cfg).unwrap();
    let mut events = Vec::new();
    let mut blamed = 0;
    for _ in 0..1_500 {
        events.clear();
        sim.step(&mut events);
        let w = sim.world();
        let d = delays(&events);
        let mut surge_ahead = false;
        for &p in &w.stations.queue[0] {
            let surge = w.parcels.key[p.index()] & switchyard_core::faults::SURGE_KEY_FLAG != 0;
            let unclaimed = w.parcels.robot[p.index()].is_none();
            let expected =
                (unclaimed && !surge && surge_ahead).then_some(WaitCause::Fault(FaultId(0)));
            assert_eq!(d.get(&p).map(|x| x.1), expected, "{p}");
            blamed += usize::from(expected.is_some());
            surge_ahead |= surge;
        }
    }
    assert!(blamed > 100, "{blamed}");
}

/// The closed cells of each active topology fault, from the sim's state.
fn closed_cells(sim: &Sim, t: Tick) -> Vec<(FaultId, Vec<CellIdx>)> {
    let g = &sim.layout().grid;
    sim.faults()
        .iter()
        .enumerate()
        .filter(|(i, f)| {
            sim.fault_enabled(FaultId(u32::try_from(*i).unwrap())) && f.is_active_at(t)
        })
        .filter_map(|(i, f)| {
            let cells = match &f.kind {
                FaultKind::AisleBlock { cells } => cells.iter().map(|&c| g.idx(c)).collect(),
                FaultKind::RobotFailure { robot } => vec![sim.world().robots.pos[robot.index()]],
                _ => return None,
            };
            Some((FaultId(u32::try_from(i).unwrap()), cells))
        })
        .collect()
}

/// The fault that cut `from` off from `goal`, by definition: the lowest-id
/// closure whose reopening alone reconnects it, else the lowest-id closure.
fn cutter(sim: &Sim, closed: &[(FaultId, Vec<CellIdx>)], goal: Goal, from: CellIdx) -> FaultId {
    for (id, _) in closed {
        let mut mask = vec![false; sim.layout().grid.len()];
        for (other, cells) in closed {
            if other != id {
                for &c in cells {
                    mask[c as usize] = true;
                }
            }
        }
        if DistanceTables::goal_row(sim.layout(), 3, goal, &mask)[from as usize] != UNREACHABLE {
            return *id;
        }
    }
    closed[0].0
}

#[test]
fn chaos_presets_blame_every_fault_and_the_right_one() {
    for (name, text) in presets::CHAOS {
        let cfg = ScenarioConfig::from_toml_str(text).unwrap();
        let end = cfg.faults.iter().map(FaultSpec::end).max().unwrap() + 600;
        let mut sim = Sim::new(&cfg).unwrap();
        let mut events = Vec::new();
        let mut blame = vec![0u64; cfg.faults.len()];
        // (goal, fault reopened, closed-set signature) -> row.
        let mut rows: BTreeMap<(Goal, FaultId, Vec<CellIdx>), Vec<u32>> = BTreeMap::new();
        let mut cut_checked = 0;
        let mut cutters: BTreeMap<(StationId, Vec<CellIdx>), FaultId> = BTreeMap::new();
        for _ in 0..end {
            events.clear();
            let pos = sim.world().robots.pos.clone();
            let t = sim.tick();
            let closed = closed_cells(&sim, t);
            sim.step(&mut events);
            let d = delays(&events);
            for (parcel, (_, root)) in &d {
                blame[root.fault().unwrap().index()] += 1;
                // (ii) A parcel queued at a cut-off station is blamed on the
                // fault that cut the station off from the parking lane.
                let w = sim.world();
                if let WaitCause::CellBlocked(f) = *root
                    && w.parcels.phase[parcel.index()] == Phase::Queued
                {
                    let s = w.parcels.station[parcel.index()];
                    let all: Vec<CellIdx> = closed.iter().flat_map(|c| c.1.clone()).collect();
                    let expected = *cutters.entry((s, all)).or_insert_with(|| {
                        cutter(&sim, &closed, Goal::Station(s), sim.layout().parking[0])
                    });
                    assert_eq!(f, expected, "{name}: {parcel}");
                    cut_checked += 1;
                }
            }
            for e in &events {
                let Event::RobotWaited {
                    robot,
                    direct: WaitCause::CellBlocked(f),
                    ..
                } = *e
                else {
                    continue;
                };
                // (ii) Independent check: the robot's goal is unreachable
                // with every active closure, and reachable with only `f`
                // reopened (or no single reopening suffices and `f` is the
                // lowest active topology fault).
                let goal = sim.world().robots.goal[robot.index()].unwrap();
                let from = pos[robot.index()];
                let all: Vec<CellIdx> = closed.iter().flat_map(|c| c.1.clone()).collect();
                let reach = |rows: &mut BTreeMap<_, Vec<u32>>, without: Option<FaultId>| {
                    let key = (goal, without.unwrap_or(FaultId(u32::MAX)), all.clone());
                    let row = rows.entry(key).or_insert_with(|| {
                        let mut mask = vec![false; sim.layout().grid.len()];
                        for (id, cells) in &closed {
                            if Some(*id) != without {
                                for &c in cells {
                                    mask[c as usize] = true;
                                }
                            }
                        }
                        DistanceTables::goal_row(sim.layout(), 3, goal, &mask)
                    });
                    row[from as usize] != UNREACHABLE
                };
                assert!(!reach(&mut rows, None), "{name}: {robot} is not cut off");
                let reconnects: Vec<FaultId> = closed
                    .iter()
                    .map(|c| c.0)
                    .filter(|&id| reach(&mut rows, Some(id)))
                    .collect();
                let expected = reconnects.first().copied().unwrap_or(closed[0].0);
                assert_eq!(f, expected, "{name}: {robot} blamed on the wrong fault");
                cut_checked += 1;
            }
        }
        assert!(
            cut_checked > 0 || !name.contains("dead"),
            "{name}: no cut-off blame to check"
        );
        // (i) Every fault receives root-attributed delay.
        for (i, &b) in blame.iter().enumerate() {
            assert!(b > 0, "{name}: fault F{i} received no delay");
        }
        eprintln!("{name}: delay by fault {blame:?}, {cut_checked} cut-off waits checked");
    }
}
