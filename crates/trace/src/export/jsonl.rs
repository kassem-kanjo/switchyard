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

use super::{RunId, span_id, trace_id, write_value};

/// Write one line per span of every trace.
///
/// Columns: `trace_id`, `span_id`, `parcel`, `station`, `destination`,
/// `phase`, `start_tick`, `end_tick`, `ticks`, `start_us`, `end_us`,
/// `robot` (or null), `direct_kind`/`direct_id` and `root_kind`/`root_id`
/// (null unless the span is an attributed `Blocked` span),
/// `delay_kind`/`delay_id` and `delay_root_kind`/`delay_root_id` (the fault
/// delay of the span, any phase, else null), `fault` (the delaying fault's
/// id, or null), `complete`,
/// `mis_sorted` and `chute` (or null).
///
/// # Errors
/// Any error of the writer.
pub fn write_jsonl<W: Write>(w: W, traces: &[ParcelTrace]) -> io::Result<()> {
    write_jsonl_with_run_id(w, traces, [0; 8])
}

/// Like [`write_jsonl`], with ids mixed with a run identity
/// ([`run_id`](super::run_id)) so several runs can share one table.
///
/// # Errors
/// Any error of the writer.
pub fn write_jsonl_with_run_id<W: Write>(
    mut w: W,
    traces: &[ParcelTrace],
    run: RunId,
) -> io::Result<()> {
    for trace in traces {
        let tid = trace_id(run, trace.parcel);
        for (i, span) in trace.spans.iter().enumerate() {
            let direct = span.attrs.direct_blocker.map(cause_parts);
            let root = span.attrs.root_cause.map(cause_parts);
            let delay = span.attrs.delay_cause.map(cause_parts);
            let delay_root = span.attrs.delay_root.map(cause_parts);
            let line = json!({
                "trace_id": tid,
                "span_id": span_id(run, trace.parcel, i + 1),
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
                "delay_kind": delay.map(|d| d.kind),
                "delay_id": delay.map(|d| d.id),
                "delay_root_kind": delay_root.map(|d| d.kind),
                "delay_root_id": delay_root.map(|d| d.id),
                "fault": span.attrs.fault().map(|f| f.0),
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
