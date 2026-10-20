//! `switchyard`: command-line front end for the Switchyard simulator.
//!
//! The CLI owns all I/O (reading scenario files, printing results), which
//! keeps the core pure (ADR 0002).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use switchyard_core::config::ScenarioConfig;
use switchyard_core::sim::Sim;

/// Observability, chaos engineering and post-mortems for robotic parcel
/// sortation.
#[derive(Debug, Parser)]
#[command(name = "switchyard", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run a scenario and print throughput and the state hash.
    Run {
        /// Path to a scenario TOML file (for example scenarios/small.toml).
        #[arg(long)]
        scenario: PathBuf,
        /// Number of ticks to simulate (1 tick = 0.5 s).
        #[arg(long)]
        ticks: u32,
        /// Override the scenario's seed.
        #[arg(long)]
        seed: Option<u64>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Run {
            scenario,
            ticks,
            seed,
        } => match run(&scenario, ticks, seed) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
    }
}

fn run(path: &PathBuf, ticks: u32, seed: Option<u64>) -> Result<(), String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let mut config = ScenarioConfig::from_toml_str(&text).map_err(|e| e.to_string())?;
    if let Some(seed) = seed {
        config.seed = seed;
    }
    let mut sim = Sim::new(&config).map_err(|e| e.to_string())?;

    // Events are drained every tick: the CLI only needs the counters here.
    let mut events = Vec::new();
    for _ in 0..ticks {
        sim.step(&mut events);
        events.clear();
    }

    let stats = sim.stats();
    let layout = sim.layout();
    let robots = config.fleet.robots;
    let per_hour = stats.throughput_per_hour_milli() as f64 / 1_000.0;
    let sim_minutes = f64::from(stats.tick) / 120.0;
    println!(
        "scenario      {} (seed {}, {}x{} floor, {} stations, {} chutes, {} robots)",
        config.name,
        config.seed,
        layout.grid.width(),
        layout.grid.height(),
        layout.stations.len(),
        layout.chutes.len(),
        robots
    );
    println!("simulated     {} ticks ({sim_minutes:.1} min)", stats.tick);
    println!(
        "parcels       arrived {}, queued {}, in flight {}, delivered {}",
        stats.arrived, stats.queued, stats.in_flight, stats.delivered
    );
    println!(
        "throughput    {per_hour:.1} parcels/h ({:.1} per robot/h)",
        per_hour / f64::from(robots.max(1))
    );
    println!("state hash    {:016x}", sim.state_hash());
    Ok(())
}
