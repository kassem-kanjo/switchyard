//! # switchyard-analysis
//!
//! Metrics time series, CUSUM incident detection, resilience scoring against
//! a fault-free twin, counterfactual root-cause analysis and post-mortem
//! rendering (ARCHITECTURE §7, ADR 0005). Everything here is a pure function
//! of the event stream; floats are allowed on outputs only (ADR 0002).
//!
//! V0 skeleton: the crate exists and is wired to the core and trace crates;
//! the analyses land in CP2 and CP3.

pub use switchyard_trace as trace;

/// Version of the analysis crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
