//! `switchyard export`: run a scenario and write its parcel traces to a file.
//!
//! Wiring (not done here): add `mod export;` to `main.rs`, a
//! `Command::Export(export::ExportArgs)` variant, and call
//! `export::run_export(args)`. The crate needs `switchyard-trace` as a
//! dependency (already added to `crates/cli/Cargo.toml`).
//!
//! ```text
//! switchyard export --scenario small --ticks 2000 --format perfetto --out small.json
//! switchyard export --scenario medium --ticks 7200 --format jsonl --out slow.jsonl --parcels late:p95
//! switchyard export --scenario small --ticks 2000 --format otlp --out t.json --parcels 12,40,41
//! ```
//!
//! Errors are plain strings, like the rest of the CLI.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use clap::{Args, ValueEnum};
use switchyard_core::config::ScenarioConfig;
use switchyard_core::presets;
use switchyard_core::sim::Sim;
use switchyard_trace::export::{
    OtlpOptions, RunId, run_id, write_jsonl_with_run_id, write_otlp, write_perfetto,
};
use switchyard_trace::{ParcelTrace, TraceBuilder};

/// Output format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ExportFormat {
    /// Chrome trace-event JSON; open at <https://ui.perfetto.dev>.
    Perfetto,
    /// OTLP JSON (`resourceSpans`).
    Otlp,
    /// One JSON object per span per line (DuckDB, Python).
    Jsonl,
}

/// Which parcels to keep, to bound the file size.
#[derive(Clone, Debug, PartialEq)]
pub enum ParcelFilter {
    /// Every parcel.
    All,
    /// Parcels whose latency is at or above this percentile (0 to 100) of
    /// the completed parcels' latencies. Parcels still in flight at the end
    /// are judged by their latency so far; if nothing completed, all of
    /// them are kept.
    Late(f64),
    /// Exactly these parcel ids.
    Ids(Vec<u32>),
}

impl FromStr for ParcelFilter {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("all") {
            return Ok(Self::All);
        }
        if let Some(rest) = s.strip_prefix("late:") {
            let num = rest.trim_start_matches(['p', 'P']);
            let p: f64 = num
                .parse()
                .map_err(|_| format!("bad percentile {rest:?} in {s:?} (use late:p95)"))?;
            if !(0.0..=100.0).contains(&p) {
                return Err(format!("percentile {p} is outside 0..=100"));
            }
            return Ok(Self::Late(p));
        }
        let ids = s
            .split(',')
            .map(|part| {
                let part = part.trim().trim_start_matches(['P', 'p']);
                part.parse::<u32>()
                    .map_err(|_| format!("bad parcel id {part:?}; use all, late:p95 or 1,2,3"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::Ids(ids))
    }
}

impl ParcelFilter {
    /// Keep the traces this filter selects, in their original order.
    #[must_use]
    pub fn apply(&self, traces: Vec<ParcelTrace>) -> Vec<ParcelTrace> {
        match self {
            Self::All => traces,
            Self::Ids(ids) => traces
                .into_iter()
                .filter(|t| ids.contains(&t.parcel.0))
                .collect(),
            Self::Late(p) => {
                let mut lat: Vec<u32> = traces
                    .iter()
                    .filter(|t| !t.is_open())
                    .map(ParcelTrace::latency)
                    .collect();
                if lat.is_empty() {
                    return traces;
                }
                lat.sort_unstable();
                // Nearest rank: the smallest latency with at least p% of
                // the parcels at or below it.
                let rank = ((p / 100.0) * lat.len() as f64).ceil() as usize;
                let threshold = lat[rank.clamp(1, lat.len()) - 1];
                traces
                    .into_iter()
                    .filter(|t| t.latency() >= threshold)
                    .collect()
            }
        }
    }
}

/// Arguments of `switchyard export`.
#[derive(Clone, Debug, Args)]
pub struct ExportArgs {
    /// A built-in preset name (small, medium, large) or the path to a
    /// scenario TOML file.
    #[arg(long)]
    pub scenario: PathBuf,
    /// Number of ticks to simulate (1 tick = 0.5 s).
    #[arg(long)]
    pub ticks: u32,
    /// Override the scenario's seed.
    #[arg(long)]
    pub seed: Option<u64>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = ExportFormat::Perfetto)]
    pub format: ExportFormat,
    /// File to write.
    #[arg(long)]
    pub out: PathBuf,
    /// Which parcels to export: `all`, `late:p95` (the slowest 5%), or a
    /// comma-separated id list such as `12,40,41`.
    #[arg(long, default_value = "all")]
    pub parcels: ParcelFilter,
}

/// What an export wrote, for the summary line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExportSummary {
    /// Traces written.
    pub parcels: usize,
    /// Phase spans written.
    pub spans: usize,
    /// Of which, traces still in flight at the end of the run.
    pub open: usize,
}

fn scenario_text(arg: &Path) -> Result<String, String> {
    if !arg.exists()
        && let Some(text) = arg.to_str().and_then(presets::by_name)
    {
        return Ok(text.to_owned());
    }
    std::fs::read_to_string(arg).map_err(|e| format!("reading {}: {e}", arg.display()))
}

/// Variant label mixed into the run id of CLI exports.
const RUN_VARIANT: &str = "cli";

/// Run the scenario and return every parcel trace (complete and open), plus
/// the run's identity: a hash of the scenario text and the effective seed,
/// so two seeds never share trace ids.
///
/// # Errors
/// An unreadable or invalid scenario.
pub fn collect_run(
    scenario: &Path,
    ticks: u32,
    seed: Option<u64>,
) -> Result<(Vec<ParcelTrace>, RunId), String> {
    let text = scenario_text(scenario)?;
    let mut config = ScenarioConfig::from_toml_str(&text).map_err(|e| e.to_string())?;
    if let Some(seed) = seed {
        config.seed = seed;
    }
    let mut sim = Sim::new(&config).map_err(|e| e.to_string())?;
    let mut builder = TraceBuilder::new();
    let (mut events, mut traces) = (Vec::new(), Vec::new());
    for _ in 0..ticks {
        sim.step(&mut events);
        builder.ingest(&events, &mut traces);
        events.clear();
    }
    traces.extend(builder.finish(sim.tick()));
    traces.sort_by_key(|t| t.parcel);
    Ok((traces, run_id(&text, config.seed, RUN_VARIANT)))
}

/// Run `switchyard export`.
///
/// # Errors
/// A bad scenario, or a file that cannot be written.
pub fn run_export(args: ExportArgs) -> Result<(), String> {
    let (traces, run) = collect_run(&args.scenario, args.ticks, args.seed)?;
    let total = traces.len();
    let traces = args.parcels.apply(traces);
    let file =
        File::create(&args.out).map_err(|e| format!("creating {}: {e}", args.out.display()))?;
    let mut w = BufWriter::new(file);
    let res = match args.format {
        ExportFormat::Perfetto => write_perfetto(&mut w, &traces),
        ExportFormat::Otlp => write_otlp(
            &mut w,
            &traces,
            &OtlpOptions {
                run_id: run,
                ..OtlpOptions::default()
            },
        ),
        ExportFormat::Jsonl => write_jsonl_with_run_id(&mut w, &traces, run),
    };
    res.and_then(|()| w.flush())
        .map_err(|e| format!("writing {}: {e}", args.out.display()))?;
    let summary = ExportSummary {
        parcels: traces.len(),
        spans: traces.iter().map(|t| t.spans.len()).sum(),
        open: traces.iter().filter(|t| t.is_open()).count(),
    };
    let bytes = std::fs::metadata(&args.out).map_or(0, |m| m.len());
    println!(
        "exported {} of {total} parcels ({} spans, {} still in flight) to {} ({bytes} bytes)",
        summary.parcels,
        summary.spans,
        summary.open,
        args.out.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_traces(s: &Path, ticks: u32, seed: Option<u64>) -> Result<Vec<ParcelTrace>, String> {
        collect_run(s, ticks, seed).map(|(t, _)| t)
    }

    #[test]
    fn filters_parse() {
        assert_eq!("all".parse(), Ok(ParcelFilter::All));
        assert_eq!("late:p95".parse(), Ok(ParcelFilter::Late(95.0)));
        assert_eq!("late:99.5".parse(), Ok(ParcelFilter::Late(99.5)));
        assert_eq!("3, 1,P7".parse(), Ok(ParcelFilter::Ids(vec![3, 1, 7])));
        assert!("late:p101".parse::<ParcelFilter>().is_err());
        assert!("late:x".parse::<ParcelFilter>().is_err());
        assert!("a,b".parse::<ParcelFilter>().is_err());
    }

    #[test]
    fn filters_select_by_latency_and_id() {
        let all = collect_traces(Path::new("small"), 1_500, Some(3)).unwrap();
        assert!(all.len() > 50);
        let late = ParcelFilter::Late(95.0).apply(all.clone());
        assert!(
            !late.is_empty() && late.len() < all.len() / 4,
            "{}",
            late.len()
        );
        let min_late = late.iter().map(ParcelTrace::latency).min().unwrap();
        assert!(
            all.iter()
                .filter(|t| !t.is_open() && t.latency() > min_late)
                .all(|t| late.iter().any(|l| l.parcel == t.parcel))
        );
        let picked = ParcelFilter::Ids(vec![2, 5, 99_999]).apply(all);
        assert_eq!(
            picked.iter().map(|t| t.parcel.0).collect::<Vec<_>>(),
            [2, 5]
        );
    }

    fn otlp_trace_ids(seed: u64) -> Vec<String> {
        let dir =
            std::env::temp_dir().join(format!("switchyard-runid-{}-{seed}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("o.json");
        run_export(ExportArgs {
            scenario: PathBuf::from("small"),
            ticks: 400,
            seed: Some(seed),
            format: ExportFormat::Otlp,
            out: out.clone(),
            parcels: ParcelFilter::All,
        })
        .unwrap();
        let text = std::fs::read_to_string(&out).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        text.split("\"traceId\":\"")
            .skip(1)
            .map(|r| r[..32].to_owned())
            .collect()
    }

    #[test]
    fn trace_ids_depend_on_the_seed() {
        let a = otlp_trace_ids(1);
        assert!(!a.is_empty());
        assert_eq!(a, otlp_trace_ids(1), "same seed, same ids");
        assert_ne!(a, otlp_trace_ids(2), "different seed, different ids");
        assert!(a.iter().all(|id| id != &"0".repeat(32)));
    }

    #[test]
    fn writes_every_format() {
        let dir = std::env::temp_dir().join(format!("switchyard-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (format, name) in [
            (ExportFormat::Perfetto, "p.json"),
            (ExportFormat::Otlp, "o.json"),
            (ExportFormat::Jsonl, "s.jsonl"),
        ] {
            let out = dir.join(name);
            run_export(ExportArgs {
                scenario: PathBuf::from("small"),
                ticks: 400,
                seed: Some(1),
                format,
                out: out.clone(),
                parcels: ParcelFilter::All,
            })
            .unwrap();
            let text = std::fs::read_to_string(&out).unwrap();
            assert!(text.len() > 100, "{name} is empty");
            if format != ExportFormat::Jsonl {
                serde_json_check(&text);
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A cheap well-formedness check without a JSON dependency in the CLI.
    fn serde_json_check(text: &str) {
        assert!(text.starts_with('{') && text.trim_end().ends_with('}'));
        let (mut depth, mut in_str, mut esc) = (0i64, false, false);
        for c in text.chars() {
            if in_str {
                match (esc, c) {
                    (true, _) => esc = false,
                    (false, '\\') => esc = true,
                    (false, '"') => in_str = false,
                    _ => {}
                }
            } else {
                match c {
                    '"' => in_str = true,
                    '{' | '[' => depth += 1,
                    '}' | ']' => depth -= 1,
                    _ => {}
                }
            }
            assert!(depth >= 0);
        }
        assert_eq!(depth, 0);
    }
}
