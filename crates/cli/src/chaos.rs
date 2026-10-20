//! `switchyard chaos`: paired-replay chaos experiments (ARCHITECTURE §7;
//! research §1(b)).
//!
//! For every seed, the scenario runs twice: once with the faults and once
//! as its fault-free twin on the same seed. Each pair is scored with
//! `switchyard_analysis::resilience`, one row per seed plus the spread
//! across seeds. All runs are independent sims, so they run in parallel
//! with rayon (parallelism only across sims, ADR 0002 §4).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;
use rayon::prelude::*;
use switchyard_analysis::resilience::ResilienceScore;
use switchyard_analysis::twin::{self, RunSummary};
use switchyard_core::Tick;
use switchyard_core::config::ScenarioConfig;
use switchyard_core::faults::{FaultKind, FaultSpec};
use switchyard_core::grid::{Cell, Pos};
use switchyard_core::ids::{ChuteId, RobotId, StationId};
use switchyard_core::layout;

/// Arguments of `switchyard chaos`.
#[derive(Debug, Args)]
pub struct ChaosArgs {
    /// A preset name (a hub such as `medium`, or a chaos preset such as
    /// `peak-surge`) or the path to a scenario TOML file. The scenario's own
    /// `[[faults]]` are applied, then every `--fault`.
    #[arg(long)]
    scenario: PathBuf,
    /// Ticks to simulate per run (1 tick = 0.5 s; 7200 = 1 hour).
    #[arg(long)]
    ticks: u32,
    /// A fault to inject, as `kind:key=value,...`, repeatable. Times are in
    /// ticks, or with a unit: `90s`, `30m`, `1h`. Kinds:
    /// `robot-failure:robot=31,at=3600,for=1800`;
    /// `aisle-block:x=22..23,y=3..45,at=30m,for=30m` (x and y take a value or
    /// an inclusive range; the open-floor cells of that rectangle close);
    /// `chute-jam:chute=12,...`;
    /// `induction-outage:station=3,...`;
    /// `volume-surge:station=all,factor=2.5,...` (or `factor_pct=250`);
    /// `scanner-drift:station=1,ppm=50000,...`.
    #[arg(long = "fault", value_name = "SPEC")]
    faults: Vec<String>,
    /// Seeds to run: `1..5` (inclusive), `1,4,9`, or one seed. Defaults to
    /// the scenario's seed.
    #[arg(long)]
    seeds: Option<String>,
}

/// Run the subcommand; errors go to stderr.
pub fn main(args: &ChaosArgs) -> ExitCode {
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &ChaosArgs) -> Result<(), String> {
    let text = crate::scenario_text(&args.scenario)?;
    let base = ScenarioConfig::from_toml_str(&text).map_err(|e| e.to_string())?;
    let mut extra = args
        .faults
        .iter()
        .map(|s| parse_fault(s))
        .collect::<Result<Vec<_>, _>>()?;
    // An aisle block given as a rectangle closes its open-floor cells
    // (chute holes, walls and stations inside it are not floor to close).
    let layout = layout::generate(&base.floor).map_err(|e| e.to_string())?;
    for f in &mut extra {
        if let FaultKind::AisleBlock { cells } = &mut f.kind {
            cells.retain(|&c| {
                c.x < layout.grid.width()
                    && c.y < layout.grid.height()
                    && layout.grid.at(c) == Cell::Free
            });
        }
    }
    let seeds = match &args.seeds {
        Some(s) => parse_seeds(s)?,
        None => vec![base.seed],
    };
    let (_, faulted) = twin::twin_configs(&base, &extra);
    if faulted.faults.is_empty() {
        return Err("no faults: pass --fault or a scenario with [[faults]]".into());
    }

    // Every (seed, twin) pair is an independent sim: run all in parallel.
    let started = std::time::Instant::now();
    let jobs: Vec<(u64, bool)> = seeds
        .iter()
        .flat_map(|&s| [(s, false), (s, true)])
        .collect();
    let runs = jobs
        .par_iter()
        .map(|&(seed, with_faults)| {
            let mut cfg = base.clone();
            cfg.seed = seed;
            let (b, f) = twin::twin_configs(&cfg, &extra);
            twin::run(if with_faults { &f } else { &b }, args.ticks)
        })
        .collect::<Result<Vec<RunSummary>, _>>()
        .map_err(|e| e.to_string())?;
    let wall = started.elapsed().as_secs_f64();

    let rows: Vec<(u64, &RunSummary, &RunSummary, ResilienceScore)> = runs
        .as_chunks::<2>()
        .0
        .iter()
        .zip(&seeds)
        .map(|([baseline, faulted], &seed)| {
            let score = twin::resilience(baseline, faulted);
            (seed, baseline, faulted, score)
        })
        .collect();

    print_header(&faulted, args.ticks, seeds.len(), &runs[0]);
    print_table(&rows);
    println!(
        "wall          {wall:.2} s for {} runs (rayon, {} threads)",
        runs.len(),
        rayon::current_num_threads()
    );
    Ok(())
}

fn print_header(cfg: &ScenarioConfig, ticks: Tick, seeds: usize, any: &RunSummary) {
    let c = any.capacity;
    println!(
        "scenario      {} ({} robots, {} stations, {} chutes), {} ticks ({:.1} min), {} seed{}",
        cfg.name,
        c.robots,
        c.stations,
        c.chutes,
        ticks,
        f64::from(ticks) / 120.0,
        seeds,
        if seeds == 1 { "" } else { "s" }
    );
    for (i, f) in cfg.faults.iter().enumerate() {
        println!(
            "fault F{i:<9} {:<38} ticks {}..{} (min {:.1}-{:.1})",
            describe(&f.kind),
            f.at,
            f.end(),
            f64::from(f.at) / 120.0,
            f64::from(f.end()) / 120.0
        );
    }
    println!(
        "              twin = same seed, no faults; lost = Bruneau area (parcels), \
         recovery = within 5 % of the twin for 5 min"
    );
}

fn describe(kind: &FaultKind) -> String {
    match kind {
        FaultKind::RobotFailure { robot } => format!("robot-failure {robot}"),
        FaultKind::AisleBlock { cells } => format!("aisle-block {} cells", cells.len()),
        FaultKind::ChuteJam { chute } => format!("chute-jam {chute}"),
        FaultKind::InductionOutage { station } => format!("induction-outage {station}"),
        FaultKind::VolumeSurge {
            station,
            factor_pct,
        } => format!(
            "volume-surge {} x{:.2}",
            station.map_or_else(|| "all stations".to_owned(), |s| s.to_string()),
            f64::from(*factor_pct) / 100.0
        ),
        FaultKind::ScannerDrift {
            station,
            error_rate_ppm,
        } => format!("scanner-drift {station} {error_rate_ppm} ppm"),
    }
}

fn print_table(rows: &[(u64, &RunSummary, &RunSummary, ResilienceScore)]) {
    println!(
        "{:>6} {:>10} {:>10} {:>8} {:>9} {:>9} {:>6} {:>6}",
        "seed", "twin", "faulted", "lost", "max dip", "recover", "GDR", "util"
    );
    for (seed, b, f, s) in rows {
        println!(
            "{seed:>6} {:>10} {:>10} {:>8} {:>8.1}% {:>9} {:>6} {:>5.0}%",
            b.stats.delivered,
            f.stats.delivered,
            s.lost_parcels,
            s.max_dip_pct,
            s.time_to_recover_min
                .map_or_else(|| "never".to_owned(), |m| format!("{m} min")),
            s.degradation_ratio
                .map_or_else(|| "-".to_owned(), |g| format!("{g:.2}")),
            100.0 * s.baseline_utilisation
        );
    }
    if rows.len() < 2 {
        return;
    }
    let col = |f: &dyn Fn(&ResilienceScore) -> Option<f64>| -> Vec<f64> {
        rows.iter().filter_map(|r| f(&r.3)).collect()
    };
    #[allow(clippy::cast_precision_loss)]
    let lost = col(&|s| Some(s.lost_parcels as f64));
    let dip = col(&|s| Some(s.max_dip_pct));
    let ttr = col(&|s| s.time_to_recover_min.map(f64::from));
    let gdr = col(&|s| s.degradation_ratio);
    let never = rows
        .iter()
        .filter(|r| r.3.time_to_recover_min.is_none())
        .count();
    println!("spread over {} seeds (mean ± sd, min..max):", rows.len());
    println!("  lost        {}", spread(&lost, 0, ""));
    println!("  max dip     {}", spread(&dip, 1, " %"));
    println!(
        "  recover     {}{}",
        spread(&ttr, 1, " min"),
        if never > 0 {
            format!(" ({never} never recovered)")
        } else {
            String::new()
        }
    );
    println!("  GDR         {}", spread(&gdr, 2, ""));
}

/// `mean ± sd (min..max)` with `digits` decimals.
fn spread(v: &[f64], digits: usize, unit: &str) -> String {
    if v.is_empty() {
        return "-".to_owned();
    }
    #[allow(clippy::cast_precision_loss)]
    let n = v.len() as f64;
    let mean = v.iter().sum::<f64>() / n;
    let sd = (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0)).sqrt();
    let min = v.iter().copied().fold(f64::INFINITY, f64::min);
    let max = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    format!("{mean:.digits$}{unit} ± {sd:.digits$} ({min:.digits$}..{max:.digits$})")
}

/// `1..5` (inclusive), `1..=5`, `1,4,9` or `7`.
fn parse_seeds(s: &str) -> Result<Vec<u64>, String> {
    let bad = || format!("bad --seeds `{s}` (use 1..5, 1,4,9 or 7)");
    if let Some((a, b)) = s.split_once("..") {
        let a: u64 = a.trim().parse().map_err(|_| bad())?;
        let b: u64 = b
            .trim()
            .trim_start_matches('=')
            .parse()
            .map_err(|_| bad())?;
        if b < a || b - a >= 1_000 {
            return Err(bad());
        }
        return Ok((a..=b).collect());
    }
    s.split(',')
        .map(|x| x.trim().parse().map_err(|_| bad()))
        .collect()
}

/// A tick count: plain ticks, or a number with `s`, `m` or `h`.
fn parse_ticks(v: &str) -> Result<Tick, String> {
    let (num, scale) = match v.as_bytes().last() {
        Some(b's') => (&v[..v.len() - 1], 2),
        Some(b'm') => (&v[..v.len() - 1], 120),
        Some(b'h') => (&v[..v.len() - 1], 7_200),
        _ => (v, 1),
    };
    num.trim()
        .parse::<Tick>()
        .ok()
        .and_then(|n| n.checked_mul(scale))
        .ok_or_else(|| format!("bad time `{v}` (ticks, or 90s, 30m, 1h)"))
}

/// `a` or `a..b` (inclusive).
fn parse_range(v: &str) -> Result<std::ops::RangeInclusive<u16>, String> {
    let bad = || format!("bad coordinate `{v}` (a number or a..b)");
    let (a, b) = v.split_once("..").unwrap_or((v, v));
    let a: u16 = a.trim().parse().map_err(|_| bad())?;
    let b: u16 = b
        .trim()
        .trim_start_matches('=')
        .parse()
        .map_err(|_| bad())?;
    if b < a {
        return Err(bad());
    }
    Ok(a..=b)
}

/// Parse `kind:key=value,...` (see [`ChaosArgs`]).
fn parse_fault(spec: &str) -> Result<FaultSpec, String> {
    let (kind, rest) = spec
        .split_once(':')
        .ok_or_else(|| format!("bad --fault `{spec}`: expected kind:key=value,..."))?;
    let mut kv = Vec::new();
    for pair in rest.split(',').filter(|p| !p.trim().is_empty()) {
        let (k, v) = pair
            .split_once('=')
            .ok_or_else(|| format!("bad --fault `{spec}`: `{pair}` is not key=value"))?;
        kv.push((k.trim(), v.trim(), false));
    }
    let mut take = |key: &str| -> Option<String> {
        kv.iter_mut().find(|e| e.0 == key && !e.2).map(|e| {
            e.2 = true;
            e.1.to_owned()
        })
    };
    let num = |v: Option<String>, key: &str| -> Result<u64, String> {
        let v = v.ok_or_else(|| format!("{kind} needs `{key}=`"))?;
        v.parse().map_err(|_| format!("bad `{key}={v}`"))
    };
    let small = |n: u64, key: &str| u16::try_from(n).map_err(|_| format!("`{key}` is too large"));

    let at = parse_ticks(&take("at").ok_or_else(|| format!("{kind} needs `at=`"))?)?;
    let duration = parse_ticks(
        &take("for")
            .or_else(|| take("duration"))
            .ok_or_else(|| format!("{kind} needs `for=`"))?,
    )?;
    let kind = match kind {
        "robot-failure" => FaultKind::RobotFailure {
            robot: RobotId(
                u32::try_from(num(take("robot"), "robot")?).map_err(|_| "robot too large")?,
            ),
        },
        "aisle-block" => {
            let xs = parse_range(&take("x").ok_or("aisle-block needs `x=`")?)?;
            let ys = parse_range(&take("y").ok_or("aisle-block needs `y=`")?)?;
            FaultKind::AisleBlock {
                cells: ys
                    .flat_map(|y| xs.clone().map(move |x| Pos::new(x, y)))
                    .collect(),
            }
        }
        "chute-jam" => FaultKind::ChuteJam {
            chute: ChuteId(small(num(take("chute"), "chute")?, "chute")?),
        },
        "induction-outage" => FaultKind::InductionOutage {
            station: StationId(small(num(take("station"), "station")?, "station")?),
        },
        "volume-surge" => {
            let station = match take("station").as_deref() {
                None | Some("all") => None,
                Some(s) => Some(StationId(small(
                    num(Some(s.to_owned()), "station")?,
                    "station",
                )?)),
            };
            let factor_pct = match (take("factor"), take("factor_pct")) {
                (Some(f), None) => {
                    let f: f64 = f.parse().map_err(|_| format!("bad `factor={f}`"))?;
                    // Rounded to a whole percent: the sim takes integers.
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let pct = (f * 100.0).round().clamp(0.0, 1e6) as u32;
                    pct
                }
                (None, Some(p)) => u32::try_from(num(Some(p), "factor_pct")?)
                    .map_err(|_| "factor_pct too large")?,
                _ => return Err("volume-surge needs one of `factor=` or `factor_pct=`".into()),
            };
            FaultKind::VolumeSurge {
                station,
                factor_pct,
            }
        }
        "scanner-drift" => FaultKind::ScannerDrift {
            station: StationId(small(num(take("station"), "station")?, "station")?),
            error_rate_ppm: u32::try_from(num(
                take("ppm").or_else(|| take("error_rate_ppm")),
                "ppm",
            )?)
            .map_err(|_| "ppm too large")?,
        },
        other => {
            return Err(format!(
                "unknown fault kind `{other}` (robot-failure, aisle-block, chute-jam, \
                 induction-outage, volume-surge, scanner-drift)"
            ));
        }
    };
    if let Some(unused) = kv.iter().find(|e| !e.2) {
        return Err(format!("{} does not take `{}`", kind.name(), unused.0));
    }
    let fault = FaultSpec { at, duration, kind };
    fault.validate().map_err(|e| e.to_string())?;
    Ok(fault)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faults_parse_from_the_command_line() {
        assert_eq!(
            parse_fault("robot-failure:robot=31,at=3600,for=1800"),
            Ok(FaultSpec {
                at: 3_600,
                duration: 1_800,
                kind: FaultKind::RobotFailure { robot: RobotId(31) },
            })
        );
        let block = parse_fault("aisle-block:x=22..23,y=3..5,at=30m,for=90s").unwrap();
        assert_eq!((block.at, block.duration), (3_600, 180));
        let FaultKind::AisleBlock { cells } = block.kind else {
            panic!("{block:?}");
        };
        assert_eq!(cells.len(), 6);
        assert_eq!(cells[0], Pos::new(22, 3));
        assert_eq!(
            parse_fault("volume-surge:station=all,factor=2.5,at=1h,for=10m").map(|f| f.kind),
            Ok(FaultKind::VolumeSurge {
                station: None,
                factor_pct: 250
            })
        );
        assert_eq!(
            parse_fault("scanner-drift:station=1,ppm=50000,at=0,for=1").map(|f| f.kind),
            Ok(FaultKind::ScannerDrift {
                station: StationId(1),
                error_rate_ppm: 50_000
            })
        );
    }

    #[test]
    fn bad_faults_say_what_is_wrong() {
        let err = |s: &str| parse_fault(s).unwrap_err();
        assert!(err("robot-failure:robot=1,at=5").contains("needs `for=`"));
        assert!(err("robot-failure:robot=1,at=5,for=2,chute=3").contains("does not take `chute`"));
        assert!(err("meteor:at=1,for=2").contains("unknown fault kind"));
        assert!(err("chute-jam:chute=1,at=5,for=0").contains("at least 1 tick"));
        assert!(err("volume-surge:factor=0.5,at=1,for=2").contains("factor_pct"));
        assert!(err("robot-failure").contains("kind:key=value"));
    }

    #[test]
    fn seeds_parse_as_ranges_lists_or_one() {
        assert_eq!(parse_seeds("1..5"), Ok(vec![1, 2, 3, 4, 5]));
        assert_eq!(parse_seeds("1..=3"), Ok(vec![1, 2, 3]));
        assert_eq!(parse_seeds("4,9"), Ok(vec![4, 9]));
        assert_eq!(parse_seeds("7"), Ok(vec![7]));
        assert!(parse_seeds("5..1").is_err());
        assert!(parse_seeds("x").is_err());
    }
}
