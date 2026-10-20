//! `switchyard`: command-line front end for the Switchyard simulator.
//!
//! The CLI owns all I/O (reading scenario files, printing results), which
//! keeps the core pure (ADR 0002).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use switchyard_core::config::ScenarioConfig;
use switchyard_core::events::{Event, WaitCause};
use switchyard_core::presets;
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
        /// A built-in preset name (small, medium, large) or the path to a
        /// scenario TOML file (for example scenarios/medium.toml).
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

/// The scenario text for `arg`: an existing file wins; otherwise a preset
/// name is looked up among the presets embedded in the core.
fn scenario_text(arg: &PathBuf) -> Result<String, String> {
    if !arg.exists()
        && let Some(text) = arg.to_str().and_then(presets::by_name)
    {
        return Ok(text.to_owned());
    }
    std::fs::read_to_string(arg).map_err(|e| format!("reading {}: {e}", arg.display()))
}

/// Wait events by cause kind, for the summary line.
#[derive(Default)]
struct WaitCounts {
    blocked: u64,
    yielded: u64,
    cell: u64,
    chute: u64,
    station: u64,
}

fn run(path: &PathBuf, ticks: u32, seed: Option<u64>) -> Result<(), String> {
    let text = scenario_text(path)?;
    let mut config = ScenarioConfig::from_toml_str(&text).map_err(|e| e.to_string())?;
    if let Some(seed) = seed {
        config.seed = seed;
    }
    let mut sim = Sim::new(&config).map_err(|e| e.to_string())?;

    // Events are drained every tick: the CLI only needs counters here.
    let mut events = Vec::new();
    let mut waits = WaitCounts::default();
    let mut stalls = 0u64;
    let started = std::time::Instant::now();
    for _ in 0..ticks {
        sim.step(&mut events);
        for e in &events {
            match e {
                Event::RobotWaited { direct, .. } => match direct {
                    WaitCause::BlockedBy(_) => waits.blocked += 1,
                    WaitCause::Yielded(_) => waits.yielded += 1,
                    WaitCause::CellBlocked(_) => waits.cell += 1,
                    WaitCause::ChuteUnavailable(_) => waits.chute += 1,
                    WaitCause::StationBusy(_) => waits.station += 1,
                },
                Event::Stalled { .. } => stalls += 1,
                _ => {}
            }
        }
        events.clear();
    }
    let wall = started.elapsed().as_secs_f64();

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
    println!("mis-sorted    {}", stats.mis_sorted);
    println!(
        "robot waits   blocked {}, yielded {}, cell blocked {}, chute unavailable {}, station busy {} (robot-ticks)",
        waits.blocked, waits.yielded, waits.cell, waits.chute, waits.station
    );
    println!("stalls        {stalls}");
    println!(
        "speed         {:.0} ticks/s wall ({:.2} s)",
        f64::from(stats.tick) / wall.max(1e-9),
        wall
    );
    println!("state hash    {:016x}", sim.state_hash());
    Ok(())
}
