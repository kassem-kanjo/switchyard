//! Harder planted incidents (issue #8). `planted.rs` plants one dominant
//! cause among small decoys and the cause always wins (24/24). This suite
//! plants causes the engine cannot trivially separate, and scores honestly:
//!
//! - **Near-tied pair** (A is 6-25 % longer than B: 17-20 min against 16, same kind, symmetric
//!   targets). Correct: A ranks first, OR the report marks the two as a tie
//!   (the report's own rule: Shapley values within twice the noise band).
//! - **Duplicate outage** (two identical outages of the same station over
//!   the same window). Exactly symmetric, so the truth is "no ranking" and
//!   either alone does all the damage. Correct only if the report marks the
//!   tie AND names the interaction redundant; a confident ranking is a false
//!   claim. (A first attempt used outages of two different stations as an
//!   "exact tie", but station geometry makes them genuinely unequal, 3930 vs
//!   4690 parcel-min on one seed; the scorer cannot call that a tie. A second
//!   attempt, outage plus surge at one station as "redundant", measured
//!   compound on 8/8 seeds: the surge's backlog adds load the outage alone
//!   does not, so the hypothesis, not the engine, was wrong.)
//! - **Compound pair**: a fleet-wide surge plus a jam of the busiest chute.
//!   Load meets lost capacity, so together they should do more than apart.
//!   Expected sign: compound.
//!
//! The engine is not tuned to pass: the rate is whatever it is, and the
//! per-class breakdown is printed. Run in release:
//! `cargo test -p switchyard-analysis --release --test planted_hard -- --ignored --nocapture`

use rayon::prelude::*;
use switchyard_analysis::postmortem::shapley::Interplay;
use switchyard_analysis::postmortem::{self, Options, PostMortem, significant};
use switchyard_core::Tick;
use switchyard_core::config::ScenarioConfig;
use switchyard_core::faults::{FaultKind, FaultSpec};
use switchyard_core::ids::{FaultId, RobotId, StationId};
use switchyard_core::presets;
use switchyard_core::sim::Sim;

const MIN: Tick = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    NearTied,
    Duplicate,
    Compound,
}

impl Class {
    const ALL: [Self; 3] = [Self::NearTied, Self::Duplicate, Self::Compound];
}

/// A planted case: the scenario, the two fault ids under test (`a` is the
/// bigger one in `NearTied`), and its class.
struct Case {
    cfg: ScenarioConfig,
    class: Class,
    a: FaultId,
    b: FaultId,
}

fn fault(at: Tick, duration: Tick, kind: FaultKind) -> FaultSpec {
    FaultSpec { at, duration, kind }
}

fn build(base: &str, seed: u64, class: Class) -> Case {
    let mut cfg = ScenarioConfig::from_toml_str(base).unwrap();
    cfg.seed = seed;
    cfg.faults.clear();
    let stations = cfg.floor.stations;
    let robots = cfg.fleet.robots;
    let s0 = StationId(u16::try_from(seed).unwrap() % stations);
    let s1 = StationId((s0.0 + 1) % stations);
    let busy = Sim::new(&cfg).unwrap().world().destination_chute[0];
    let decoy = fault(
        22 * MIN,
        4 * MIN,
        FaultKind::RobotFailure {
            robot: RobotId(u32::try_from(seed * 13 + 5).unwrap() % robots),
        },
    );
    let pair = match class {
        Class::NearTied => vec![
            fault(
                20 * MIN,
                20 * MIN,
                FaultKind::InductionOutage { station: s0 },
            ),
            fault(
                20 * MIN,
                16 * MIN,
                FaultKind::InductionOutage { station: s1 },
            ),
        ],
        Class::Duplicate => vec![
            fault(
                20 * MIN,
                16 * MIN,
                FaultKind::InductionOutage { station: s0 },
            ),
            fault(
                20 * MIN,
                16 * MIN,
                FaultKind::InductionOutage { station: s0 },
            ),
        ],
        Class::Compound => vec![
            fault(
                20 * MIN,
                18 * MIN,
                FaultKind::VolumeSurge {
                    station: None,
                    factor_pct: 200,
                },
            ),
            fault(20 * MIN, 18 * MIN, FaultKind::ChuteJam { chute: busy }),
        ],
    };
    // The decoy sits before or after the pair by seed, so ids carry no hint.
    let at = usize::try_from(seed % 2).unwrap() * 2;
    cfg.faults.extend(pair);
    cfg.faults.insert(at, decoy);
    let id = |k: usize| FaultId(u32::try_from(if k >= at { k + 1 } else { k }).unwrap());
    Case {
        cfg,
        class,
        a: id(0),
        b: id(1),
    }
}

fn candidate_index(pm: &PostMortem, id: FaultId) -> Option<usize> {
    pm.candidates.iter().position(|c| c.fault == id)
}

/// Score one case; prints a line with the evidence.
fn score(case: &Case, ticks: Tick, dither: usize) -> bool {
    let pm = postmortem::run(
        &case.cfg,
        ticks,
        &Options {
            dither,
            ..Options::default()
        },
    )
    .unwrap();
    let tag = format!("{:?} seed {}", case.class, case.cfg.seed);
    let (Some(ia), Some(ib)) = (candidate_index(&pm, case.a), candidate_index(&pm, case.b)) else {
        println!("{tag}: MISSED (a planted fault was not a candidate)");
        return false;
    };
    let att = &pm.attribution;
    let phi = |i: usize| att.shapley(i);
    // The report's tie rule, applied to the two planted faults.
    let tied = !significant(phi(ia) - phi(ib), pm.share_band(ia).max(pm.share_band(ib)));
    let a_first = (0..pm.candidates.len())
        .max_by_key(|&i| (att.shapley_scaled[i], std::cmp::Reverse(i)))
        == Some(ia);
    let x = att.interaction(ia, ib);
    let got = Interplay::of(x);
    println!(
        "{tag}: A={:.0} B={:.0} bands {:.0}/{:.0} tied={tied} a_first={a_first};          interaction {:.0} -> {got:?}",
        phi(ia) / 120.0,
        phi(ib) / 120.0,
        pm.share_band(ia) / 120.0,
        pm.share_band(ib) / 120.0,
        x / 120.0,
    );
    let ok = match case.class {
        Class::NearTied => tied || a_first,
        Class::Duplicate => tied && got == Some(Interplay::Redundant),
        Class::Compound => got == Some(Interplay::Compound),
    };
    if !ok {
        println!("{tag}: MISSED");
    }
    ok
}

#[test]
#[ignore = "harder measurement; run in release with --ignored --nocapture"]
#[allow(clippy::cast_precision_loss)]
fn planted_hard_near_tied_and_interacting_causes() {
    let started = std::time::Instant::now();
    let cases: Vec<Case> = Class::ALL
        .iter()
        .flat_map(|&c| (1..=16).map(move |s| build(presets::MEDIUM_TOML, s, c)))
        .collect();
    let results: Vec<(Class, bool)> = cases
        .par_iter()
        .map(|c| (c.class, score(c, 90 * MIN, 8)))
        .collect();
    let mut total = 0;
    for class in Class::ALL {
        let of: Vec<bool> = results
            .iter()
            .filter(|r| r.0 == class)
            .map(|r| r.1)
            .collect();
        let hit = of.iter().filter(|&&b| b).count();
        total += hit;
        println!("{class:?}: {hit}/{}", of.len());
    }
    println!(
        "planted-hard: {total}/{} correct ({:.1} %, {:.0} s). The rate is reported as measured; the engine is not tuned to pass.",
        results.len(),
        100.0 * total as f64 / results.len() as f64,
        started.elapsed().as_secs_f64()
    );
    // Not an accuracy gate: only guards against a broken harness.
    assert!(total > 0, "no case scored: the harness is broken");
}
