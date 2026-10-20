//! OTLP JSON export (`ExportTraceServiceRequest`, protobuf JSON mapping).
//!
//! Mapping, chosen so the file reads naturally in Jaeger-like tools:
//!
//! - One **trace per parcel**. The trace id is derived from the parcel id
//!   alone and the span ids from `(parcel id, span index)`, so exporting the
//!   same run twice gives identical bytes.
//! - A root span `parcel` covers `[arrival, done)`; each phase span is its
//!   child. Phase spans tile the root exactly.
//! - A `Blocked` span carries **links** to the work of the robot that held
//!   it up (`link.type = blocked_by`) and, when different, of the robot at
//!   the root of the wait chain (`link.type = root_cause`). A link points at
//!   the blocker's span (the phase span of whichever parcel that robot
//!   carried at that tick); an idle blocker has no span, so no link, but its
//!   id is still in the span attributes.
//! - Attributes use the `switchyard.*` namespace; the resource carries
//!   `service.name`. Times are the simulated clock added to a fixed epoch
//!   ([`OtlpOptions::epoch_unix_nanos`]): never wall-clock.
//!
//! 64-bit integers are strings, per the protobuf JSON mapping.

use std::collections::BTreeMap;
use std::io::{self, Write};

use serde_json::{Value, json};
use switchyard_core::Tick;
use switchyard_core::events::WaitCause;
use switchyard_core::ids::{ParcelId, RobotId};
use switchyard_core::world::Phase;

use crate::model::{ParcelTrace, Span, cause_label, cause_parts, phase_name};

use super::{span_id, trace_id, write_value};

/// Nanoseconds per tick (1 tick = 0.5 s).
const NANOS_PER_TICK: u64 = 500_000_000;
/// `SPAN_KIND_INTERNAL`.
const KIND_INTERNAL: u32 = 1;
/// `STATUS_CODE_ERROR`.
const STATUS_ERROR: u32 = 2;

/// Options of the OTLP exporter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OtlpOptions {
    /// Unix time of tick 0, in nanoseconds. The default is
    /// 2026-01-01T00:00:00Z, so tools that dislike 1970 timestamps still
    /// render the trace; set it to any fixed value you like.
    pub epoch_unix_nanos: u64,
}

impl Default for OtlpOptions {
    fn default() -> Self {
        Self {
            epoch_unix_nanos: 1_767_225_600 * 1_000_000_000,
        }
    }
}

impl OtlpOptions {
    fn nanos(&self, tick: Tick) -> String {
        (self.epoch_unix_nanos + u64::from(tick) * NANOS_PER_TICK).to_string()
    }
}

fn s_attr(key: &str, value: impl Into<String>) -> Value {
    json!({"key": key, "value": {"stringValue": value.into()}})
}

fn i_attr(key: &str, value: i64) -> Value {
    // int64 is a JSON string in the protobuf mapping.
    json!({"key": key, "value": {"intValue": value.to_string()}})
}

fn b_attr(key: &str, value: bool) -> Value {
    json!({"key": key, "value": {"boolValue": value}})
}

/// One work interval of a robot, for resolving blocker links.
#[derive(Clone, Copy)]
pub(super) struct Work {
    pub(super) start: Tick,
    pub(super) end: Tick,
    pub(super) parcel: ParcelId,
    pub(super) span_index: usize,
}

pub(super) fn robot_work(traces: &[ParcelTrace]) -> BTreeMap<RobotId, Vec<Work>> {
    let mut map: BTreeMap<RobotId, Vec<Work>> = BTreeMap::new();
    for trace in traces {
        for (i, span) in trace.spans.iter().enumerate() {
            if let Some(robot) = span.attrs.robot {
                map.entry(robot).or_default().push(Work {
                    start: span.start,
                    end: span.end,
                    parcel: trace.parcel,
                    span_index: i + 1,
                });
            }
        }
    }
    for list in map.values_mut() {
        list.sort_by_key(|w| (w.start, w.end, w.parcel));
    }
    map
}

/// The work of `robot` that covers `tick`, if it had any.
pub(super) fn work_at(
    map: &BTreeMap<RobotId, Vec<Work>>,
    robot: RobotId,
    tick: Tick,
) -> Option<Work> {
    let list = map.get(&robot)?;
    let i = list.partition_point(|w| w.start <= tick);
    list[..i].iter().rev().find(|w| tick < w.end).copied()
}

fn link(work: Work, kind: &str, cause: WaitCause) -> Value {
    json!({
        "traceId": trace_id(work.parcel),
        "spanId": span_id(work.parcel, work.span_index),
        "attributes": [
            s_attr("link.type", kind),
            s_attr("switchyard.cause", cause_label(cause)),
        ],
    })
}

fn phase_span(
    trace: &ParcelTrace,
    index: usize,
    span: &Span,
    work: &BTreeMap<RobotId, Vec<Work>>,
    opts: &OtlpOptions,
) -> Value {
    let mut attrs = vec![
        s_attr("switchyard.phase", phase_name(span.phase)),
        i_attr("switchyard.parcel.id", i64::from(trace.parcel.0)),
        i_attr("switchyard.ticks", i64::from(span.ticks())),
    ];
    if let Some(r) = span.attrs.robot {
        attrs.push(i_attr("switchyard.robot.id", i64::from(r.0)));
    }
    let mut links = Vec::new();
    if span.phase == Phase::Blocked {
        for (prefix, kind, cause) in [
            (
                "switchyard.blocked.direct",
                "blocked_by",
                span.attrs.direct_blocker,
            ),
            (
                "switchyard.blocked.root",
                "root_cause",
                span.attrs.root_cause,
            ),
        ] {
            let Some(cause) = cause else { continue };
            let parts = cause_parts(cause);
            attrs.push(s_attr(&format!("{prefix}.kind"), parts.kind));
            attrs.push(i_attr(&format!("{prefix}.id"), i64::from(parts.id)));
            if let Some(robot) = parts.robot
                && let Some(w) = work_at(work, robot, span.start)
            {
                links.push(link(w, kind, cause));
            }
        }
        // The root often equals the direct blocker: one link is enough.
        links.dedup_by(|b, a| a["traceId"] == b["traceId"] && a["spanId"] == b["spanId"]);
    }
    let mut v = json!({
        "traceId": trace_id(trace.parcel),
        "spanId": span_id(trace.parcel, index),
        "parentSpanId": span_id(trace.parcel, 0),
        "name": phase_name(span.phase),
        "kind": KIND_INTERNAL,
        "startTimeUnixNano": opts.nanos(span.start),
        "endTimeUnixNano": opts.nanos(span.end),
        "attributes": attrs,
    });
    if !links.is_empty() {
        v["links"] = Value::Array(links);
    }
    v
}

fn root_span(trace: &ParcelTrace, opts: &OtlpOptions) -> Value {
    let mut attrs = vec![
        i_attr("switchyard.parcel.id", i64::from(trace.parcel.0)),
        i_attr("switchyard.station.id", i64::from(trace.station.0)),
        i_attr("switchyard.destination.id", i64::from(trace.destination.0)),
        b_attr("switchyard.parcel.mis_sorted", trace.mis_sorted),
        b_attr("switchyard.trace.open", trace.is_open()),
        i_attr("switchyard.latency.ticks", i64::from(trace.latency())),
    ];
    if let Some(c) = trace.chute {
        attrs.push(i_attr("switchyard.chute.id", i64::from(c.0)));
    }
    for r in &trace.robots {
        attrs.push(i_attr("switchyard.robot.id", i64::from(r.0)));
    }
    let mut v = json!({
        "traceId": trace_id(trace.parcel),
        "spanId": span_id(trace.parcel, 0),
        "name": "parcel",
        "kind": KIND_INTERNAL,
        "startTimeUnixNano": opts.nanos(trace.arrival),
        "endTimeUnixNano": opts.nanos(trace.done),
        "attributes": attrs,
    });
    if trace.mis_sorted {
        v["status"] = json!({"code": STATUS_ERROR, "message": "mis-sorted"});
    }
    v
}

/// Write the traces as one OTLP/JSON `ExportTraceServiceRequest`.
///
/// # Errors
/// Any error of the writer.
pub fn write_otlp<W: Write>(
    mut w: W,
    traces: &[ParcelTrace],
    opts: &OtlpOptions,
) -> io::Result<()> {
    let resource = json!({"attributes": [
        s_attr("service.name", "switchyard"),
        s_attr("service.version", crate::VERSION),
        s_attr("telemetry.sdk.name", "switchyard-trace"),
    ]});
    let scope = json!({"name": "switchyard-trace", "version": crate::VERSION});
    w.write_all(b"{\"resourceSpans\":[{\"resource\":")?;
    write_value(&mut w, &resource)?;
    w.write_all(b",\"scopeSpans\":[{\"scope\":")?;
    write_value(&mut w, &scope)?;
    w.write_all(b",\"spans\":[")?;
    let work = robot_work(traces);
    let mut first = true;
    let mut emit = |w: &mut W, v: &Value| -> io::Result<()> {
        if !first {
            w.write_all(b",")?;
        }
        first = false;
        write_value(w, v)
    };
    for trace in traces {
        emit(&mut w, &root_span(trace, opts))?;
        for (i, span) in trace.spans.iter().enumerate() {
            emit(&mut w, &phase_span(trace, i + 1, span, &work, opts))?;
        }
    }
    w.write_all(b"]}]}]}\n")?;
    w.flush()
}
