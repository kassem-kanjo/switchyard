//! `switchyard chaos`: paired-replay chaos experiments (ARCHITECTURE §7;
//! research §1(b)).
//!
//! For every seed, the scenario runs twice: once with the faults and once
//! as its fault-free twin on the same seed. Each pair is scored with
//! `switchyard_analysis::resilience` (formulas and sources in its module
//! docs; the Bruneau/DT/RR lineage is cited there, not in the output), one
//! row per seed plus the spread across seeds and a one-line legend per
//! column for site operations staff. All runs are independent sims, so they run in parallel
//! with rayon (parallelism only across sims, ADR 0002 §4).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;
use rayon::prelude::*;
use switchyard_analysis::resilience::{Params, ResilienceScore};
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
    /// A parcel counts as late when the faults add more than this many
    /// minutes to its time from arrival to chute.
    #[arg(long, default_value_t = 10)]
    sla_extra_minutes: u32,
    /// Optional absolute cutoff in minutes from arrival to chute (e.g. the
    /// time to the truck): also count parcels slower than this in each run.
    #[arg(long)]
    cutoff_minutes: Option<u32>,
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

    let params = Params {
        sla_extra_minutes: args.sla_extra_minutes,
        cutoff_minutes: args.cutoff_minutes,
        ..Params::default()
    };
    let rows: Vec<Row<'_>> = runs
        .as_chunks::<2>()
        .0
        .iter()
        .zip(&seeds)
        .map(|([baseline, faulted], &seed)| {
            let score = twin::resilience(baseline, faulted, params);
            (seed, baseline, faulted, score)
        })
        .collect();

    print_header(&faulted, args.ticks, seeds.len(), &runs[0]);
    print_table(&rows, params);
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
    for (id, f) in cfg.enabled_faults() {
        println!(
            "fault {:<9} {:<38} ticks {}..{} (min {:.1}-{:.1})",
            id.to_string(),
            describe(&f.kind),
            f.at,
            f.end(),
            f64::from(f.at) / 120.0,
            f64::from(f.end()) / 120.0
        );
    }
    println!(
        "              each seed runs twice: with the faults, and as a fault-free twin \
         (same seed, same parcels)"
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

type Row<'a> = (u64, &'a RunSummary, &'a RunSummary, ResilienceScore);

/// `a vs b` for a faulted value and its twin.
fn versus(a: impl std::fmt::Display, b: impl std::fmt::Display) -> String {
    format!("{a} vs {b}")
}

fn ratio(r: Option<f64>) -> String {
    r.map_or_else(|| "-".to_owned(), |x| format!("{x:.2}"))
}

fn print_table(rows: &[Row<'_>], params: Params) {
    let late = format!("late >{}m", params.sla_extra_minutes);
    let cutoff = params.cutoff_minutes.map(|m| format!("over {m}m"));
    print!(
        "{:>5} {:>18} {:>9} {:>14} {:>11} {:>8} {:>8} {:>14} {:>14} {:>8} {:>5}",
        "seed",
        "delay (parcel-min)",
        late,
        "peak backlog",
        "undelivered",
        "max dip",
        "recover",
        "served/offered",
        "lost stn/chute",
        "thru/cap",
        "util"
    );
    if let Some(c) = &cutoff {
        print!(" {c:>12}");
    }
    println!();
    for (seed, _, _, s) in rows {
        print!(
            "{seed:>5} {:>18.0} {:>9} {:>14} {:>11} {:>7.1}% {:>8} {:>14} {:>14} {:>8} {:>4.0}%",
            s.delay.net_minutes(),
            s.delay.late_parcels,
            versus(s.peak_backlog, s.twin_peak_backlog),
            s.undelivered,
            s.max_dip_pct,
            s.time_to_recover_min
                .map_or_else(|| "never".to_owned(), |m| format!("{m} min")),
            versus(ratio(s.served_offered), ratio(s.twin_served_offered)),
            format!("{} / {}", s.stations_lost, s.chutes_lost),
            ratio(s.throughput_vs_capacity),
            100.0 * s.baseline_utilisation
        );
        if let Some(c) = s.past_cutoff {
            print!(" {:>12}", versus(c.faulted, c.twin));
        }
        println!();
    }
    print_delay_detail(rows);
    print_spread(rows);
    print_legend(params);
}

/// Gross and offsetting parts of the delay, when some parcels were faster
/// with the fault than without it.
fn print_delay_detail(rows: &[Row<'_>]) {
    if rows.iter().all(|r| r.3.delay.faster_parcels == 0) {
        return;
    }
    println!(
        "delay detail (parcel-min): net = extra time of delayed parcels - time saved by faster ones"
    );
    for (seed, _, _, s) in rows {
        let d = s.delay;
        println!(
            "{seed:>5}   net {:>8.0} = {:>8.0} over {:>5} delayed parcels - {:>6.0} over {:>5} faster",
            d.net_minutes(),
            d.gross_minutes(),
            d.delayed_parcels,
            d.faster_minutes(),
            d.faster_parcels
        );
    }
}

fn print_spread(rows: &[Row<'_>]) {
    if rows.len() < 2 {
        return;
    }
    let col = |f: &dyn Fn(&ResilienceScore) -> Option<f64>| -> Vec<f64> {
        rows.iter().filter_map(|r| f(&r.3)).collect()
    };
    let never = rows
        .iter()
        .filter(|r| r.3.time_to_recover_min.is_none())
        .count();
    println!("spread over {} seeds (mean ± sd, min..max):", rows.len());
    let line = |name: &str, v: &[f64], digits: usize, unit: &str| {
        println!("  {name:<18} {}", spread(v, digits, unit));
    };
    line(
        "delay",
        &col(&|s| Some(s.delay.net_minutes())),
        0,
        " parcel-min",
    );
    line(
        "late",
        &col(&|s| Some(f64::from(s.delay.late_parcels))),
        0,
        " parcels",
    );
    line(
        "peak backlog",
        &col(&|s| Some(f64::from(s.peak_backlog))),
        0,
        " parcels",
    );
    line(
        "undelivered",
        &col(&|s| Some(f64::from(s.undelivered))),
        0,
        " parcels",
    );
    line("max dip", &col(&|s| Some(s.max_dip_pct)), 1, " %");
    println!(
        "  {:<18} {}{}",
        "recover",
        spread(&col(&|s| s.time_to_recover_min.map(f64::from)), 1, " min"),
        if never > 0 {
            format!(" ({never} never recovered)")
        } else {
            String::new()
        }
    );
    line("served/offered", &col(&|s| s.served_offered), 2, "");
    line(
        "stations lost",
        &col(&|s| Some(f64::from(s.stations_lost))),
        1,
        "",
    );
    line(
        "chutes lost",
        &col(&|s| Some(f64::from(s.chutes_lost))),
        1,
        "",
    );
    line("thru/cap", &col(&|s| s.throughput_vs_capacity), 2, "");
}

fn print_legend(params: Params) {
    let sla = params.sla_extra_minutes;
    println!("columns (faulted run compared with its fault-free twin):");
    for (name, text) in [
        (
            "delay",
            "total extra time the fault added to regular parcels, in parcel-minutes \
             (60 = one parcel 1 hour late, or 60 parcels 1 minute late)"
                .to_owned(),
        ),
        (
            "late",
            format!("regular parcels the fault delayed by more than {sla} min"),
        ),
        (
            "peak backlog",
            "most parcels waiting or in transit at once, faulted vs twin".to_owned(),
        ),
        (
            "undelivered",
            "parcels the fault left unsorted when the run ended".to_owned(),
        ),
        (
            "max dip",
            "worst minute's drop in sorted parcels below the twin".to_owned(),
        ),
        (
            "recover",
            "minutes from the first fault until throughput holds within 5 % of the twin \
             for 5 minutes"
                .to_owned(),
        ),
        (
            "served/offered",
            "parcels sorted / parcels arriving while the fault lasted (surge parcels \
             included), faulted vs twin"
                .to_owned(),
        ),
        (
            "lost stn/chute",
            "stations lost = stations whose loading cell is blocked by a failed robot \
             (or that are down or cut off); chutes lost = jammed, or every access cell \
             blocked; the most at once"
                .to_owned(),
        ),
        (
            "thru/cap",
            "share of throughput kept / share of capacity kept during the fault; below 1 \
             = congestion made it worse than the lost capacity alone; - = the fault \
             removed no capacity"
                .to_owned(),
        ),
        (
            "util",
            "share of robots busy in the twin (the same fault hurts more on a busier floor)"
                .to_owned(),
        ),
    ] {
        println!("  {name:<15} {text}");
    }
    if let Some(m) = params.cutoff_minutes {
        println!(
            "  {:<15} parcels that took longer than {m} min from arrival to chute, \
             faulted vs twin",
            format!("over {m}m")
        );
    }
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
