//! `switchyard postmortem`: detect an incident, prove its causes by replay
//! and write a Google-SRE-format post-mortem (ADR 0005, ADR 0006). The
//! engine is `switchyard_analysis::postmortem`; this module owns the I/O.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;
use switchyard_analysis::postmortem::{self, Options, report};
use switchyard_core::config::ScenarioConfig;

/// Arguments of `switchyard postmortem`.
#[derive(Debug, Args)]
pub struct PostmortemArgs {
    /// A preset name (for example `two-dead-robots-and-a-jam`) or the path
    /// to a scenario TOML file with `[[faults]]`.
    #[arg(long)]
    scenario: PathBuf,
    /// Ticks to simulate (1 tick = 0.5 s; 7200 = 1 hour).
    #[arg(long)]
    ticks: u32,
    /// Override the scenario's seed.
    #[arg(long)]
    seed: Option<u64>,
    /// Histories to replay on: the real one plus dithered copies (1 = no
    /// dither; ADR 0006 §6).
    #[arg(long, default_value_t = 8)]
    dither: usize,
    /// Write the full Markdown report here (otherwise it goes to stdout).
    #[arg(long)]
    out: Option<PathBuf>,
}

/// Run the subcommand; errors go to stderr.
pub fn main(args: &PostmortemArgs) -> ExitCode {
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &PostmortemArgs) -> Result<(), String> {
    let text = crate::scenario_text(&args.scenario)?;
    let mut cfg = ScenarioConfig::from_toml_str(&text).map_err(|e| e.to_string())?;
    if let Some(seed) = args.seed {
        cfg.seed = seed;
    }
    if args.dither == 0 {
        return Err("--dither must be at least 1".into());
    }
    let options = Options {
        dither: args.dither,
        ..Options::default()
    };
    let started = std::time::Instant::now();
    let pm = postmortem::run(&cfg, args.ticks, &options).map_err(|e| e.to_string())?;
    let wall = started.elapsed().as_secs_f64();
    let md = report::render(&pm);

    // Stdout: the summary, then where the full report went.
    match &args.out {
        Some(path) => {
            std::fs::write(path, &md).map_err(|e| format!("{}: {e}", path.display()))?;
            println!("{}", report::summary(&pm));
            println!("\nfull report: {}", path.display());
        }
        None => println!("{md}"),
    }
    let histories = pm.histories();
    let changes = pm.replays.len();
    let replays = changes * histories;
    eprintln!(
        "{replays} {} ({changes} {} x {histories} {}) in {wall:.1} s on {} threads",
        plural(replays, "replay", "replays"),
        plural(changes, "change", "changes"),
        plural(histories, "history", "histories"),
        rayon::current_num_threads()
    );
    Ok(())
}

/// `one` or `many` by count.
const fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 { one } else { many }
}
