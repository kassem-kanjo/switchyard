//! Calibration of the CUSUM threshold `h` on long shifts (issue #17, part 1).
//!
//! The dithered null of `calibrate_h_on_fault_free_pairs`
//! (`postmortem::detect`, 2 h runs) repeated on the shift lengths production
//! actually runs: same seed, twin = fault-free run, actual = the same config
//! plus 1-tick `RobotFailure` faults every 30 simulated minutes, which
//! perturbs the trajectories without a real fault. The peak CUSUM of
//! `actual` against `twin` (k = 0.5) is the noise `h` must clear over a full
//! shift.
//!
//! Measurement only (no assertion on the counts); run it in release:
//! `cargo test -p switchyard-analysis --release --test calibration -- --ignored --nocapture`.

use rayon::prelude::*;
use switchyard_analysis::metrics::TICKS_PER_MINUTE;
use switchyard_analysis::postmortem::detect::cusum;
use switchyard_analysis::twin;
use switchyard_core::Tick;
use switchyard_core::config::ScenarioConfig;
use switchyard_core::faults::{FaultKind, FaultSpec};
use switchyard_core::ids::RobotId;
use switchyard_core::presets::MEDIUM_TOML;

#[test]
#[ignore = "calibration measurement; run in release with --ignored --nocapture"]
fn calibrate_h_on_long_shifts() {
    println!("| Shift | Seeds | Peak median | Peak max | Alarms at h=2 | h=3 | h=4 |");
    println!("|---|---|---|---|---|---|---|");
    for hours in [8u64, 10] {
        let ticks = Tick::try_from(hours * 60).unwrap() * TICKS_PER_MINUTE;
        let minutes = u64::from(ticks / TICKS_PER_MINUTE);
        let peaks: Vec<f64> = (1..=6u64)
            .into_par_iter()
            .map(|seed| {
                let mut c = ScenarioConfig::from_toml_str(MEDIUM_TOML).unwrap();
                c.seed = seed;
                let twin = twin::run(&c, ticks).unwrap().series.delivered_regular;
                let robots = u64::from(c.fleet.robots);
                // 1-tick robot failures every 30 simulated minutes (k = 0,
                // 1, ...): trajectory divergence only, no real outage.
                for (k, at) in (1..=minutes / 30).enumerate() {
                    let r = u32::try_from((seed * 37 + 11 + 7 * k as u64) % robots).unwrap();
                    c.faults.push(FaultSpec {
                        at: Tick::try_from(30 * at).unwrap() * TICKS_PER_MINUTE,
                        duration: 1,
                        kind: FaultKind::RobotFailure { robot: RobotId(r) },
                    });
                }
                let actual = twin::run(&c, ticks).unwrap().series.delivered_regular;
                cusum(&twin, &actual, 0.5).into_iter().fold(0.0, f64::max)
            })
            .collect();
        assert!(peaks.iter().all(|&p| p.is_finite()), "peaks must be finite");
        let mut sorted = peaks.clone();
        sorted.sort_by(f64::total_cmp);
        let alarms = |h: f64| peaks.iter().filter(|&&x| x > h).count();
        println!(
            "| {hours} h | {} | {:.2} | {:.2} | {} | {} | {} |",
            peaks.len(),
            sorted[peaks.len() / 2],
            sorted[peaks.len() - 1],
            alarms(2.0),
            alarms(3.0),
            alarms(4.0)
        );
    }
}
