//! Exporters: write [`ParcelTrace`]s to any [`std::io::Write`].
//!
//! | Format | Function | Opens in |
//! |---|---|---|
//! | Chrome trace-event JSON | [`write_perfetto`] | <https://ui.perfetto.dev>, `chrome://tracing` |
//! | OTLP JSON (`resourceSpans`) | [`write_otlp`] | OpenTelemetry collectors, Jaeger |
//! | JSONL, one span per line | [`write_jsonl`] | DuckDB `read_json_auto`, Python |
//!
//! All three are pure functions of the traces: the same input gives the same
//! bytes. Large runs produce large files; filter the traces first (for
//! example only the slowest parcels) rather than exporting everything.

mod faults;
mod jsonl;
mod otlp;
mod perfetto;

use std::io;

pub use faults::{FaultWindow, fault_windows};
pub use jsonl::{write_jsonl, write_jsonl_with_run_id};
pub use otlp::{OtlpOptions, write_otlp};
pub use perfetto::{write_perfetto, write_perfetto_with_faults};

use serde_json::Value;
use switchyard_core::hash::Fnv1a;

/// Which exporter to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Chrome trace-event JSON for Perfetto.
    Perfetto,
    /// OTLP JSON.
    Otlp,
    /// One JSON object per span per line.
    Jsonl,
}

pub(crate) fn json_err(e: serde_json::Error) -> io::Error {
    io::Error::from(e)
}

/// Write `value` followed by nothing: a thin wrapper that maps the error.
pub(crate) fn write_value<W: io::Write>(w: &mut W, value: &Value) -> io::Result<()> {
    serde_json::to_writer(w, value).map_err(json_err)
}

/// SplitMix64 finaliser: a bijective mix, so distinct inputs give distinct
/// ids, with no hashing dependency and no randomness.
const fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// A run's identity: 8 bytes mixed into every OTLP trace and span id, so
/// that runs of different scenarios, seeds or variants never collide in one
/// collector. Use [`run_id`] to derive it.
pub type RunId = [u8; 8];

/// Derive a [`RunId`] from the scenario text, the seed and a variant label
/// (for example `"baseline"` or `"fault-F1-disabled"`).
///
/// Deterministic: FNV-1a over `scenario_toml`, a separator, the seed and the
/// variant (length-prefixed so the parts cannot run into each other), then
/// one SplitMix64 round. The same inputs give the same id on every machine.
#[must_use]
pub fn run_id(scenario_toml: &str, seed: u64, variant: &str) -> RunId {
    let mut h = Fnv1a::new();
    h.write_u64(scenario_toml.len() as u64);
    h.write(scenario_toml.as_bytes());
    h.write_u64(seed);
    h.write_u64(variant.len() as u64);
    h.write(variant.as_bytes());
    mix(h.finish()).to_be_bytes()
}

/// The 128-bit OTLP trace id of a parcel as 32 lowercase hex digits.
/// Deterministic: derived from the run id and the parcel id. For a fixed
/// run the parcel-to-id map is injective (`mix` is a bijection); across runs
/// ids differ except for a 2^-64 chance per pair.
pub(crate) fn trace_id(run: RunId, parcel: switchyard_core::ids::ParcelId) -> String {
    let r = u64::from_be_bytes(run);
    let p = u64::from(parcel.0);
    let hi = mix(p ^ mix(r ^ 0x5357_4954_4348_5941)); // "SWITCHYA"
    let lo = mix(p ^ mix(r ^ 0x5244_5f54_5241_4345)); // "RD_TRACE"
    format!("{hi:016x}{lo:016x}")
}

/// The 64-bit span id as 16 hex digits. `index` 0 is the parcel's root span;
/// phase spans are numbered from 1. Never all zeros (an invalid OTLP id).
/// Mixes in the run id like [`trace_id`], so span ids are also unique across
/// runs that share a collector.
pub(crate) fn span_id(run: RunId, parcel: switchyard_core::ids::ParcelId, index: usize) -> String {
    let r = u64::from_be_bytes(run);
    let key = (u64::from(parcel.0) << 32) | (index as u64 & 0xFFFF_FFFF);
    format!("{:016x}", mix(key ^ mix(r ^ 0x7370_616e_5f69_6421)).max(1))
}
