//! # switchyard-analysis
//!
//! Metrics time series, CUSUM incident detection, resilience scoring against
//! a fault-free twin, counterfactual root-cause analysis and post-mortem
//! rendering (ARCHITECTURE §7, ADR 0005). Everything here is a pure function
//! of the event stream; floats are allowed on outputs only (ADR 0002).
//!
//! CP2 delivers the metric series ([`metrics`]), twin runs ([`twin`]) and
//! the resilience score ([`resilience`]); detection and counterfactual RCA
//! land in CP3.

pub use switchyard_trace as trace;

pub mod metrics;
pub mod postmortem;
pub mod resilience;
pub mod twin;

/// Version of the analysis crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
