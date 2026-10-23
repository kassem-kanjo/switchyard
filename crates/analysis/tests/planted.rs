//! Planted incidents (PRODUCT.md §5, "causal correctness"): inject a known
//! dominant cause among smaller decoy faults, run the post-mortem, and check
//! that the planted cause ranks first by Shapley value in at least 90 % of
//! the cases.
//!
//! - `planted_causes_rank_first_on_the_small_floor` runs in every
//!   `cargo test` (small floor, no dither).
//! - `planted_causes_rank_first_on_the_medium_floor` is the headline
//!   measurement: the three chaos presets plus generated variants on the
//!   medium floor, 8 histories each. Run it in release:
//!   `cargo test -p switchyard-analysis --release --test planted -- --ignored --nocapture`.

use rayon::prelude::*;
use switchyard_analysis::postmortem::{self, Options};
use switchyard_core::Tick;
use switchyard_core::config::ScenarioConfig;
use switchyard_core::faults::{FaultKind, FaultSpec};
use switchyard_core::ids::{ChuteId, FaultId, RobotId, StationId};
use switchyard_core::presets;
use switchyard_core::sim::Sim;

const MIN: Tick = 120;

fn fault(at: Tick, duration: Tick, kind: FaultKind) -> FaultSpec {
    FaultSpec { at, duration, kind }
}

/// The chutes serving the most and the least popular destination
/// (destination 0 is the most popular under the Zipf mix).
fn chutes(cfg: &ScenarioConfig) -> (ChuteId, ChuteId) {
    let sim = Sim::new(cfg).unwrap();
    let map = &sim.world().destination_chute;
    (map[0], map[map.len() - 1])
}

/// A scenario with one planted cause (variant 0: a long jam of the busiest
/// chute; 1: an induction outage; 2: a volume surge) at minute 20 and three
/// decoys: a short robot failure, a mild scanner drift and a short jam of
/// the quietest chute. Returns the scenario and the planted fault's id.
fn planted(base: &str, seed: u64, variant: u64) -> (ScenarioConfig, FaultId) {
    let mut cfg = ScenarioConfig::from_toml_str(base).unwrap();
    cfg.seed = seed;
    cfg.faults.clear();
    let (busy, quiet) = chutes(&cfg);
    let stations = cfg.floor.stations;
    let robots = cfg.fleet.robots;
    let station = StationId(u16::try_from(seed).unwrap() % stations);
    let decoys = [
        fault(
            22 * MIN,
            5 * MIN,
            FaultKind::RobotFailure {
                robot: RobotId(u32::try_from(seed * 13 + 5).unwrap() % robots),
            },
        ),
        fault(
            21 * MIN,
            10 * MIN,
            FaultKind::ScannerDrift {
                station: StationId((station.0 + 1) % stations),
                error_rate_ppm: 20_000,
            },
        ),
        fault(24 * MIN, 4 * MIN, FaultKind::ChuteJam { chute: quiet }),
    ];
    let plant = match variant % 3 {
        0 => fault(20 * MIN, 20 * MIN, FaultKind::ChuteJam { chute: busy }),
        1 => fault(20 * MIN, 15 * MIN, FaultKind::InductionOutage { station }),
        _ => fault(
            20 * MIN,
            15 * MIN,
            FaultKind::VolumeSurge {
                station: None,
                factor_pct: 250,
            },
        ),
    };
    // The planted fault sits among the decoys, not always first.
    let at = usize::try_from(seed % 4).unwrap();
    cfg.faults.extend_from_slice(&decoys);
    cfg.faults.insert(at, plant);
    (cfg, FaultId(u32::try_from(at).unwrap()))
}

/// Rank 1 by Shapley value: the planted fault?
fn ranks_first(cfg: &ScenarioConfig, planted: FaultId, ticks: Tick, dither: usize) -> bool {
    let pm = postmortem::run(
        cfg,
        ticks,
        &Options {
            dither,
            ..Options::default()
        },
    )
    .unwrap();
    if pm.incident.is_none() {
        let peak = postmortem::detect::cusum(
            &pm.twin_series.delivered_regular,
            &pm.actual_series.delivered_regular,
            0.5,
        )
        .into_iter()
        .fold(0.0, f64::max);
        println!(
            "{} seed {}: no incident (CUSUM peak {peak:.1}, max dip {:.0} %, delay {:.0} parcel-min)",
            cfg.name,
            cfg.seed,
            pm.score.max_dip_pct,
            pm.score.delay.net_minutes()
        );
    }
    let att = &pm.attribution;
    let top =
        (0..pm.candidates.len()).max_by_key(|&i| (att.shapley_scaled[i], std::cmp::Reverse(i)));
    let ok = top.is_some_and(|i| pm.candidates[i].fault == planted);
    let shares: Vec<String> = (0..pm.candidates.len())
        .map(|i| format!("{}={:.0}", pm.candidates[i].fault, att.shapley(i) / 120.0))
        .collect();
    println!(
        "{} seed {}: planted {planted} {} [{}]",
        cfg.name,
        cfg.seed,
        if ok { "ranked first" } else { "MISSED" },
        shares.join(" ")
    );
    ok
}

fn rate(results: &[bool]) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let r = results.iter().filter(|&&b| b).count() as f64 / results.len() as f64;
    r
}

#[test]
fn planted_causes_rank_first_on_the_small_floor() {
    let cases: Vec<(u64, u64)> = (1..=10).map(|s| (s, s % 3)).collect();
    let results: Vec<bool> = cases
        .par_iter()
        .map(|&(seed, variant)| {
            let (cfg, id) = planted(presets::SMALL_TOML, seed, variant);
            ranks_first(&cfg, id, 45 * MIN, 1)
        })
        .collect();
    let r = rate(&results);
    println!(
        "small floor: planted cause first in {:.0} % of {}",
        100.0 * r,
        results.len()
    );
    assert!(r >= 0.9, "{r}");
}

#[test]
#[ignore = "headline measurement; run in release with --ignored --nocapture"]
fn planted_causes_rank_first_on_the_medium_floor() {
    let started = std::time::Instant::now();
    let mut cases: Vec<(ScenarioConfig, FaultId)> = (1..=21)
        .map(|s| planted(presets::MEDIUM_TOML, s, s % 3))
        .collect();
    // The chaos presets: their dominant fault is known from CP2's replays
    // (the jam in two-dead-robots-and-a-jam; the only fault otherwise).
    for (name, dominant) in [
        ("two-dead-robots-and-a-jam", 0),
        ("aisle-7-blocked", 0),
        ("peak-surge", 0),
    ] {
        let cfg = ScenarioConfig::from_toml_str(presets::by_name(name).unwrap()).unwrap();
        cases.push((cfg, FaultId(dominant)));
    }
    let results: Vec<bool> = cases
        .iter()
        .map(|(cfg, id)| ranks_first(cfg, *id, 90 * MIN, 8))
        .collect();
    let r = rate(&results);
    println!(
        "medium floor: planted cause first in {:.1} % of {} ({:.0} s)",
        100.0 * r,
        results.len(),
        started.elapsed().as_secs_f64()
    );
    assert!(r >= 0.9, "{r}");
}
