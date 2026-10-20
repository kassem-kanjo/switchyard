//! # switchyard-core
//!
//! The deterministic simulation core of Switchyard: a robotic parcel-sortation
//! floor modelled as a discrete grid (1 tick = 0.5 s, 1 cell = 1 m).
//!
//! The core is a pure function of `(scenario, seed, fault schedule)`:
//! no I/O, no threads, no wall-clock reads and no floats in sim state
//! (ADR 0002). Its only output is the [`events::Event`] stream (ADR 0004);
//! traces, metrics, the viewer and post-mortems are all derived from it.
//!
//! ```
//! use switchyard_core::{config::ScenarioConfig, presets, sim::Sim};
//!
//! let cfg = ScenarioConfig::from_toml_str(presets::SMALL_TOML).unwrap();
//! let mut sim = Sim::new(&cfg).unwrap();
//! let mut events = Vec::new();
//! for _ in 0..100 {
//!     sim.step(&mut events);
//! }
//! assert_eq!(sim.tick(), 100);
//! ```

pub mod config;
pub mod events;
pub mod grid;
pub mod hash;
pub mod ids;
pub mod layout;
pub mod planner;
pub mod presets;
pub mod rng;
pub mod sim;
pub mod world;

/// Version of the core crate, exposed so the CLI and the WASM facade can
/// report which simulation build produced a state hash.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Simulation time in ticks. 1 tick = 0.5 s (ARCHITECTURE §3).
///
/// `u32` covers ~68 years of simulated time, far beyond any run.
pub type Tick = u32;

/// Ticks per simulated hour (3,600 s / 0.5 s).
pub const TICKS_PER_HOUR: u32 = 7_200;
