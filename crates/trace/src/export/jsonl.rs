//! JSONL export: one JSON object per span, one span per line.
//!
//! Flat columns, so DuckDB loads it with `read_json_auto('spans.jsonl')` and
//! pandas with `read_json(lines=True)`. Cause columns come as a kind and an
//! id (`direct_kind = 'blocked_by'`, `direct_id = 12`) so they can be grouped
//! and joined without string parsing.

use std::io::{self, Write};

use serde_json::json;

use crate::MICROS_PER_TICK;
use crate::model::{ParcelTrace, cause_parts, phase_name};

use super::{span_id, trace_id, write_value};

/// Write one line per span of every trace.
///
/// Columns: `trace_id`, `span_id`, `parcel`, `station`, `destination`,
/// `phase`, `start_tick`, `end_tick`, `ticks`, `start_us`, `end_us`,
/// `robot` (or null), `direct_kind`/`direct_id` and `root_kind`/`root_id`
/// (null unless the span is an attributed `Blocked` span), `complete`,
/// `mis_sorted` and `chute` (or null).
///
/// # Errors
/// Any error of the writer.
pub fn write_jsonl<W: Write>(mut w: W, traces: &[ParcelTrace]) -> io::Result<()> {
    for trace in traces {
        let tid = trace_id(trace.parcel);
        for (i, span) in trace.spans.iter().enumerate() {
            let direct = span.attrs.direct_blocker.map(cause_parts);
            let root = span.attrs.root_cause.map(cause_parts);
            let line = json!({
                "trace_id": tid,
                "span_id": span_id(trace.parcel, i + 1),
                "parcel": trace.parcel.0,
                "station": trace.station.0,
                "destination": trace.destination.0,
                "phase": phase_name(span.phase),
                "start_tick": span.start,
                "end_tick": span.end,
                "ticks": span.ticks(),
                "start_us": u64::from(span.start) * MICROS_PER_TICK,
                "end_us": u64::from(span.end) * MICROS_PER_TICK,
                "robot": span.attrs.robot.map(|r| r.0),
                "direct_kind": direct.map(|d| d.kind),
                "direct_id": direct.map(|d| d.id),
                "root_kind": root.map(|d| d.kind),
                "root_id": root.map(|d| d.id),
                "complete": !trace.is_open(),
                "mis_sorted": trace.mis_sorted,
                "chute": trace.chute.map(|c| c.0),
            });
            write_value(&mut w, &line)?;
            w.write_all(b"\n")?;
        }
    }
    w.flush()
}
