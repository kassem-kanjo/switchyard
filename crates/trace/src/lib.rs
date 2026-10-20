//! # switchyard-trace
//!
//! Builds per-parcel traces from the core's event stream (ADR 0004) and
//! exports them.
//!
//! - [`TraceBuilder`] consumes the events tick by tick and produces one
//!   [`ParcelTrace`] per parcel. A trace's [`Span`]s tile the parcel's
//!   lifetime `[arrival, done)` exactly. `Blocked` spans carry the direct
//!   blocker and the root cause of the wait.
//! - [`BlameIndex`] totals the blocked ticks per root cause and per direct
//!   blocker, so a post-mortem can shortlist the culprits (CP3).
//! - [`export`] writes traces as Chrome trace-event JSON (opens in Perfetto),
//!   OTLP JSON and JSONL.
//!
//! ```
//! use switchyard_core::{config::ScenarioConfig, presets, sim::Sim};
//! use switchyard_trace::{BlameIndex, TraceBuilder};
//!
//! let cfg = ScenarioConfig::from_toml_str(presets::SMALL_TOML).unwrap();
//! let mut sim = Sim::new(&cfg).unwrap();
//! let mut builder = TraceBuilder::new();
//! let (mut events, mut done) = (Vec::new(), Vec::new());
//! for _ in 0..500 {
//!     sim.step(&mut events);
//!     builder.ingest(&events, &mut done);
//!     events.clear();
//! }
//! let open = builder.open_traces(sim.tick());
//! let mut blame = BlameIndex::default();
//! for trace in done.iter().chain(&open) {
//!     blame.add_trace(trace);
//! }
//! assert!(blame.top_roots(3).len() <= 3);
//! ```

mod blame;
mod builder;
mod model;

pub mod export;

pub use blame::{Blame, BlameIndex};
pub use builder::TraceBuilder;
pub use model::{CauseParts, ParcelTrace, Span, SpanAttrs, TraceStatus, cause_parts, phase_name};
pub use switchyard_core as core;

/// Version of the trace crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Microseconds of simulated time per tick (1 tick = 0.5 s).
pub const MICROS_PER_TICK: u64 = 500_000;
