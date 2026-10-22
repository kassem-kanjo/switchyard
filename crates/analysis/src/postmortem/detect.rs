//! Incident detection: one-sided CUSUM on the per-minute delivery deficit
//! against the fault-free twin (ADR 0006 §1–2; research 2026-10-20 §3.1).
//!
//! `x_t = (twin_t - actual_t) / σ_t` with `σ_t = √max(twin_t, 1)`, the
//! Poisson noise of the twin's minute count. The pre-onset spread cannot
//! scale the chart: twin and actual share the seed, so the deficit is
//! exactly 0 before the first fault. The deficit is not clipped (backlog
//! drain makes it negative); only the recursion has the one-sided floor:
//! `S_t = max(0, S_{t-1} + x_t - k)`, alarm when `S_t > h`.
//!
//! - **Window:** opens at the minute after the last `S = 0` before the alarm
//!   (Page's change-point estimate) and closes at the first minute after the
//!   alarm that starts `settle` consecutive minutes at `S = 0`.
//! - **Horizon:** every replay is scored at `window end + margin` minutes.
//!
//! # Calibration of `h`
//! ADR 0006 starts from h = 5 and asks for calibration on fault-free runs.
//! Two nulls were measured (`calibrate_h_on_fault_free_pairs`, ignored by
//! default; run it in release with `--ignored --nocapture`), each 40 pairs
//! of 2 h on the small and the medium floor (160 sim-hours in all):
//!
//! | Null | Floor | Peak CUSUM, median | Max | Pairs over h = 3 / 5 |
//! |---|---|---|---|---|
//! | different seeds (twin `s`, actual `s + 1000`) | small | 5.24 | 8.90 | 37 / 20 |
//! | different seeds | medium | 6.33 | 15.53 | 38 / 24 |
//! | same seed, 1-tick failures of 3 robots | small | 0.00 | 0.35 | 0 / 0 |
//! | same seed, 1-tick failures of 3 robots | medium | 0.45 | 1.00 | 0 / 0 |
//!
//! Per-minute throughput is strongly over-dispersed against Poisson (the
//! different-seed null crosses h = 5 in half the pairs), but that null
//! carries *arrival* noise that a run and its twin never have: they share
//! every arrival. The null that matches the comparison is the same seed
//! with trajectories perturbed, where the largest peak is 1.0. So
//! **h = 3**: three times the largest null peak in 160 sim-hours, and low
//! enough to catch a surge on the small floor that costs about 700
//! parcel-minutes (peak 3.1; h = 5 missed it).

/// CUSUM parameters (in σ units) and the window rules.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CusumParams {
    /// Allowance `k`: half the shift to detect.
    pub k: f64,
    /// Decision threshold `h`.
    pub h: f64,
    /// Consecutive minutes at `S = 0` that close the window.
    pub settle_minutes: usize,
    /// Minutes after the window's end at which loss is measured.
    pub margin_minutes: usize,
}

impl Default for CusumParams {
    fn default() -> Self {
        Self {
            k: 0.5,
            h: 3.0,
            settle_minutes: 5,
            margin_minutes: 10,
        }
    }
}

/// A detected incident, in minutes from the start of the run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Incident {
    /// First minute of the window.
    pub start: usize,
    /// Minute the CUSUM crossed `h`.
    pub alarm: usize,
    /// First minute after the window (exclusive end).
    pub end: usize,
    /// False if the run ended before the CUSUM settled.
    pub settled: bool,
    /// Largest CUSUM value in the window (σ units).
    pub peak: f64,
}

impl Incident {
    /// The scoring horizon, `end + margin`, in minutes (callers clamp it to
    /// the run).
    #[must_use]
    pub const fn horizon(&self, params: &CusumParams) -> usize {
        self.end + params.margin_minutes
    }
}

/// The CUSUM path `S_t` of `actual` against `twin` (equal lengths).
#[must_use]
pub fn cusum(twin: &[u32], actual: &[u32], k: f64) -> Vec<f64> {
    let mut s = 0.0f64;
    twin.iter()
        .zip(actual)
        .map(|(&b, &a)| {
            let sigma = f64::from(b.max(1)).sqrt();
            s = (s + (f64::from(b) - f64::from(a)) / sigma - k).max(0.0);
            s
        })
        .collect()
}

/// The first incident of `actual` against `twin`, if the CUSUM alarms.
#[must_use]
pub fn detect(twin: &[u32], actual: &[u32], params: &CusumParams) -> Option<Incident> {
    let s = cusum(twin, actual, params.k);
    let alarm = s.iter().position(|&x| x > params.h)?;
    let start = s[..alarm]
        .iter()
        .rposition(|&x| x == 0.0)
        .map_or(0, |t| t + 1);
    let settle = params.settle_minutes.max(1);
    let end = (alarm + 1..s.len())
        .find(|&t| t + settle <= s.len() && s[t..t + settle].iter().all(|&x| x == 0.0));
    let close = end.unwrap_or(s.len());
    Some(Incident {
        start,
        alarm,
        end: close,
        settled: end.is_some(),
        peak: s[start..close].iter().copied().fold(0.0, f64::max),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dip_is_detected_with_its_window() {
        // Twin 100/min (σ = 10). Minutes 3..7 lose 30 (x = 3 each), then a
        // 2-minute drain above the twin, then flat.
        let twin = [100u32; 20];
        let mut actual = [100u32; 20];
        actual[3..7].fill(70);
        actual[7..9].fill(120);
        let p = CusumParams::default();
        let s = cusum(&twin, &actual, p.k);
        // S: 2.5, 5.0 (alarm, > 3), 7.5, 10.0, then 10 - 2.5 per drain
        // minute (x = -2 - 0.5) = 7.5, 5.0, then -0.5 per flat minute.
        assert_eq!(s[..3], [0.0; 3]);
        assert!((s[5] - 7.5).abs() < 1e-12 && (s[8] - 5.0).abs() < 1e-12);
        let inc = detect(&twin, &actual, &p).unwrap();
        assert_eq!((inc.start, inc.alarm), (3, 4));
        // Flat minutes from 9 drain 5.0 by 0.5 each: zero at minute 18, and
        // only 2 minutes remain, so the window never settles.
        assert!(!inc.settled);
        assert_eq!(inc.end, 20);
        assert!((inc.peak - 10.0).abs() < 1e-12);
    }

    #[test]
    fn the_window_closes_after_five_quiet_minutes() {
        let twin = [100u32; 30];
        let mut actual = [100u32; 30];
        actual[2..5].fill(40); // x = 6 each
        actual[5..8].fill(160); // x = -6 each: drains the CUSUM to 0 at 7
        let p = CusumParams::default();
        let inc = detect(&twin, &actual, &p).unwrap();
        assert_eq!((inc.start, inc.alarm, inc.end), (2, 2, 7));
        assert!(inc.settled);
        assert_eq!(inc.horizon(&p), 17);
    }

    #[test]
    fn noise_below_the_threshold_does_not_alarm() {
        let twin = [100u32; 60];
        // Alternating ±1.5σ: the CUSUM never builds up.
        let actual: Vec<u32> = (0..60).map(|t| if t % 2 == 0 { 85 } else { 115 }).collect();
        assert_eq!(detect(&twin, &actual, &CusumParams::default()), None);
        assert_eq!(detect(&[0; 5], &[0; 5], &CusumParams::default()), None);
    }

    /// Calibration of `h` (see the module docs). Slow: run in release with
    /// `cargo test -p switchyard-analysis --release -- --ignored --nocapture`.
    #[test]
    #[ignore = "calibration measurement, minutes in debug"]
    fn calibrate_h_on_fault_free_pairs() {
        use rayon::prelude::*;
        use switchyard_core::config::ScenarioConfig;
        use switchyard_core::presets::{MEDIUM_TOML, SMALL_TOML};
        for (name, toml) in [("small", SMALL_TOML), ("medium", MEDIUM_TOML)] {
            let peaks: Vec<f64> = (1..=40u64)
                .into_par_iter()
                .map(|s| {
                    let series = |seed| {
                        let mut c = ScenarioConfig::from_toml_str(toml).unwrap();
                        c.seed = seed;
                        crate::twin::run(&c, 14_400)
                            .unwrap()
                            .series
                            .delivered_regular
                    };
                    let (twin, actual) = (series(s), series(s + 1000));
                    cusum(&twin, &actual, 0.5).into_iter().fold(0.0, f64::max)
                })
                .collect();
            // The null that matters: same seed, a 1-tick failure of one
            // robot at minute 20, 40 and 60 (trajectory divergence only).
            let dithered: Vec<f64> = (1..=40u64)
                .into_par_iter()
                .map(|s| {
                    let mut c = ScenarioConfig::from_toml_str(toml).unwrap();
                    c.seed = s;
                    let twin = crate::twin::run(&c, 14_400)
                        .unwrap()
                        .series
                        .delivered_regular;
                    let robots = u64::from(c.fleet.robots);
                    for (k, at) in [2_400u32, 4_800, 7_200].into_iter().enumerate() {
                        let r = u32::try_from((s * 37 + 11 + 7 * k as u64) % robots).unwrap();
                        c.faults.push(switchyard_core::faults::FaultSpec {
                            at,
                            duration: 1,
                            kind: switchyard_core::faults::FaultKind::RobotFailure {
                                robot: switchyard_core::ids::RobotId(r),
                            },
                        });
                    }
                    let actual = crate::twin::run(&c, 14_400)
                        .unwrap()
                        .series
                        .delivered_regular;
                    cusum(&twin, &actual, 0.5).into_iter().fold(0.0, f64::max)
                })
                .collect();
            let mut d = dithered.clone();
            d.sort_by(f64::total_cmp);
            println!(
                "{name} dithered: CUSUM peak median {:.2}, max {:.2}; over 3: {}, over 4: {}, over 5: {}",
                d[20],
                d[39],
                d.iter().filter(|&&x| x > 3.0).count(),
                d.iter().filter(|&&x| x > 4.0).count(),
                d.iter().filter(|&&x| x > 5.0).count()
            );
            let mut sorted = peaks.clone();
            sorted.sort_by(f64::total_cmp);
            let over = |h: f64| peaks.iter().filter(|&&x| x > h).count();
            println!(
                "{name}: 40 pairs x 2 h; CUSUM peak median {:.2}, max {:.2}; pairs over 3: {}, over 4: {}, over 5: {}",
                sorted[20],
                sorted[39],
                over(3.0),
                over(4.0),
                over(5.0)
            );
        }
    }
}
