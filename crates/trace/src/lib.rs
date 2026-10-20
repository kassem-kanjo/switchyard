//! # switchyard-trace
//!
//! Builds per-parcel traces from the core's event stream (ADR 0004): one
//! trace per parcel whose phase spans tile its lifetime, plus blame
//! aggregation and exporters (Chrome trace JSON for Perfetto, OTLP JSON,
//! JSONL).
//!
//! V0 skeleton: the crate exists and is wired to the core; the trace builder
//! lands in CP2 (task 2.2).

pub use switchyard_core as core;

/// Version of the trace crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
