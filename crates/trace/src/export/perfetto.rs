//! Chrome trace-event JSON export, for <https://ui.perfetto.dev>.
//!
//! Layout (`1 tick = 0.5 s = 500,000 us`):
//!
//! - **Process 1 "Parcels"**: one track per *station lane*, not one per
//!   parcel (a busy run has thousands of parcels and Perfetto's track list
//!   does not scale to that). Parcels of a station are packed greedily onto
//!   the lowest lane that is free at their arrival, so a lane holds one
//!   parcel at a time, and a parcel dropping at tick `t` frees its lane only
//!   for arrivals after `t` (its drop instant never sits inside another
//!   slice). Filtered exports pack the same way: lanes = peak concurrency. Each parcel is a parent slice `P<id>` over
//!   `[arrival, done)` with its phase spans as **child slices** nested
//!   inside it. Nesting is exact because spans tile the lifetime; that is
//!   what keeps Perfetto from pushing slices onto overflow tracks. Lanes are
//!   named `S<station> lane <k>`; a drop is an instant event.
//! - **Process 2 "Robots"**: one track per robot with a slice for each span
//!   the robot carried (`AwaitingRobot` to `ChuteWait`), named by the phase
//!   exactly like the parcel track, with the parcel id in `args.parcel`. A
//!   robot works one parcel at a time, so they never overlap.
//! - **Process 3 "Faults"** (only via [`write_perfetto_with_faults`]): one
//!   track per fault spanning `FaultStarted..FaultEnded`, named like
//!   `robot-failure R57`, with the spec in `args`; plus a global instant
//!   marker (a vertical line across all tracks) at each fault start.
//! - **Colours**: every phase slice has a `cname` from Chrome trace-viewer's
//!   reserved palette (Blocked red, Travelling green, ChuteWait orange,
//!   Loading blue, Queued/AwaitingRobot grey); see [`phase_cname`].
//! - **Flow arrows** (`ph:"s"`/`"f"`): from each `Blocked` span on the
//!   parcel's lane to the blocking robot's track, when that robot was
//!   carrying something at that moment (an idle blocker has no slice to
//!   bind to, and a dangling flow would be dropped by Perfetto anyway).
//!
//! Events are sorted by `(ts, pid, tid, longest first)` so a parent always
//! precedes its children at equal timestamps. Complete (`"X"`) events only,
//! plus metadata (`"M"`), instants (`"i"`) and flows.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::io::{self, Write};

use serde_json::{Value, json};
use switchyard_core::events::WaitCause;
use switchyard_core::ids::{FaultId, RobotId, StationId};
use switchyard_core::world::Phase;

use crate::MICROS_PER_TICK;
use crate::model::{ParcelTrace, cause_label, cause_parts, phase_name};

use super::faults::FaultWindow;
use super::otlp::{robot_work, work_at};
use super::write_value;

const PID_PARCELS: u32 = 1;
const PID_ROBOTS: u32 = 2;
const PID_FAULTS: u32 = 3;

/// Chrome trace-viewer reserved colour name of a phase slice. Every value is
/// in trace-viewer's `reserved_color_scheme` list.
#[must_use]
pub const fn phase_cname(phase: Phase) -> &'static str {
    match phase {
        Phase::Blocked => "terrible",
        Phase::Travelling => "good",
        Phase::ChuteWait => "rail_idle",
        Phase::Loading => "rail_response",
        Phase::Queued | Phase::AwaitingRobot => "grey",
        Phase::Dropped => "black",
    }
}

struct Ev {
    ts: u64,
    pid: u32,
    tid: u32,
    dur: u64,
    value: Value,
}

fn micros(tick: u32) -> u64 {
    u64::from(tick) * MICROS_PER_TICK
}

fn meta(name: &str, pid: u32, tid: u32, arg: &str, value: Value) -> Ev {
    Ev {
        ts: 0,
        pid,
        tid,
        dur: u64::MAX, // metadata first
        value: json!({"ph": "M", "name": name, "pid": pid, "tid": tid, "args": {arg: value}}),
    }
}

/// Greedy interval packing per station: the lane of each trace (by index).
/// A lane is reusable only when the previous parcel ended strictly before the
/// next arrives, so the drop instant at `done` stays outside the next slice.
fn assign_lanes(traces: &[ParcelTrace]) -> Vec<(StationId, usize)> {
    let mut order: Vec<usize> = (0..traces.len()).collect();
    order.sort_by_key(|&i| (traces[i].arrival, traces[i].parcel));
    let mut free_at: BTreeMap<StationId, Vec<u32>> = BTreeMap::new();
    let mut lanes = vec![(StationId(0), 0); traces.len()];
    for i in order {
        let t = &traces[i];
        let lane_ends = free_at.entry(t.station).or_default();
        let lane = lane_ends
            .iter()
            .position(|&end| end < t.arrival)
            .unwrap_or_else(|| {
                lane_ends.push(0);
                lane_ends.len() - 1
            });
        lane_ends[lane] = t.done;
        lanes[i] = (t.station, lane);
    }
    lanes
}

fn blocked_args(direct: Option<WaitCause>, root: Option<WaitCause>) -> Value {
    json!({
        "direct_blocker": direct.map_or_else(|| "unattributed".to_owned(), cause_label),
        "root_cause": root.map_or_else(|| "unattributed".to_owned(), cause_label),
    })
}

/// Write the traces as Chrome trace-event JSON.
///
/// # Errors
/// Any error of the writer.
pub fn write_perfetto<W: Write>(w: W, traces: &[ParcelTrace]) -> io::Result<()> {
    write_perfetto_with_faults(w, traces, &[])
}

/// Like [`write_perfetto`], plus a "Faults" process with one track per
/// [`FaultWindow`] (see [`fault_windows`](super::fault_windows)) and a global
/// instant marker at each fault start.
///
/// # Errors
/// Any error of the writer.
pub fn write_perfetto_with_faults<W: Write>(
    mut w: W,
    traces: &[ParcelTrace],
    faults: &[FaultWindow],
) -> io::Result<()> {
    let lanes = assign_lanes(traces);
    // Thread ids for lanes: stations ascending, lanes ascending, from 1.
    let mut lane_count: BTreeMap<StationId, usize> = BTreeMap::new();
    for &(s, l) in &lanes {
        let n = lane_count.entry(s).or_default();
        *n = (*n).max(l + 1);
    }
    let mut lane_tid: BTreeMap<(StationId, usize), u32> = BTreeMap::new();
    let mut next_tid = 1u32;
    for (&s, &n) in &lane_count {
        for l in 0..n {
            lane_tid.insert((s, l), next_tid);
            next_tid += 1;
        }
    }

    let work = robot_work(traces);
    let names: BTreeMap<FaultId, &str> =
        faults.iter().map(|f| (f.fault, f.name.as_str())).collect();
    let mut events: Vec<Ev> = Vec::new();
    events.push(meta(
        "process_name",
        PID_PARCELS,
        0,
        "name",
        json!("Parcels"),
    ));
    events.push(meta(
        "process_sort_index",
        PID_PARCELS,
        0,
        "sort_index",
        json!(1),
    ));
    for (&(s, l), &tid) in &lane_tid {
        events.push(meta(
            "thread_name",
            PID_PARCELS,
            tid,
            "name",
            json!(format!("{s} lane {l}")),
        ));
        events.push(meta(
            "thread_sort_index",
            PID_PARCELS,
            tid,
            "sort_index",
            json!(tid),
        ));
    }
    events.push(meta("process_name", PID_ROBOTS, 0, "name", json!("Robots")));
    events.push(meta(
        "process_sort_index",
        PID_ROBOTS,
        0,
        "sort_index",
        json!(2),
    ));
    let mut robots: Vec<RobotId> = work.keys().copied().collect();
    robots.sort();
    for r in &robots {
        let tid = r.0 + 1;
        events.push(meta(
            "thread_name",
            PID_ROBOTS,
            tid,
            "name",
            json!(r.to_string()),
        ));
        events.push(meta(
            "thread_sort_index",
            PID_ROBOTS,
            tid,
            "sort_index",
            json!(tid),
        ));
    }

    let mut flow_id = 0u64;
    for (trace, &(station, lane)) in traces.iter().zip(&lanes) {
        let tid = lane_tid[&(station, lane)];
        let ts = micros(trace.arrival);
        let dur = micros(trace.done) - ts;
        events.push(Ev {
            ts,
            pid: PID_PARCELS,
            tid,
            dur,
            value: json!({
                "ph": "X", "name": trace.parcel.to_string(), "cat": "parcel",
                "pid": PID_PARCELS, "tid": tid, "ts": ts, "dur": dur,
                "args": {
                    "station": station.to_string(),
                    "destination": trace.destination.to_string(),
                    "robots": trace.robots.iter().map(ToString::to_string).collect::<Vec<_>>(),
                    "chute": trace.chute.map(|c| c.to_string()),
                    "mis_sorted": trace.mis_sorted,
                    "open": trace.is_open(),
                    "latency_ticks": trace.latency(),
                },
            }),
        });
        for span in &trace.spans {
            let ts = micros(span.start);
            let dur = micros(span.end) - ts;
            let phase = phase_name(span.phase);
            let fault = span
                .attrs
                .fault()
                .or_else(|| span.attrs.root_cause.and_then(WaitCause::fault));
            // A fault-delayed sub-span says so in its name; the category stays
            // the bare phase so colours and filters keep working.
            let name = fault.map_or_else(
                || phase.to_owned(),
                |f| {
                    names.get(&f).map_or_else(
                        || format!("{phase} \u{b7} fault {f}"),
                        |n| format!("{phase} \u{b7} fault {f} {n}"),
                    )
                },
            );
            let mut args = if span.phase == Phase::Blocked {
                blocked_args(span.attrs.direct_blocker, span.attrs.root_cause)
            } else {
                json!({})
            };
            args["ticks"] = json!(span.ticks());
            if let (Some(c), Some(r)) = (span.attrs.delay_cause, span.attrs.delay_root) {
                args["delay_cause"] = json!(cause_label(c));
                args["delay_root"] = json!(cause_label(r));
            }
            if let Some(f) = fault {
                args["fault"] = json!(f.to_string());
            }
            if let Some(r) = span.attrs.robot {
                args["robot"] = json!(r.to_string());
            }
            events.push(Ev {
                ts,
                pid: PID_PARCELS,
                tid,
                dur,
                value: json!({
                    "ph": "X", "name": name, "cat": phase, "cname": phase_cname(span.phase),
                    "pid": PID_PARCELS, "tid": tid, "ts": ts, "dur": dur, "args": args,
                }),
            });
            if let Some(r) = span.attrs.robot {
                let rtid = r.0 + 1;
                events.push(Ev {
                    ts,
                    pid: PID_ROBOTS,
                    tid: rtid,
                    dur,
                    value: json!({
                        "ph": "X", "name": name, "cat": phase, "cname": phase_cname(span.phase),
                        "pid": PID_ROBOTS, "tid": rtid, "ts": ts, "dur": dur,
                        "args": {"parcel": trace.parcel.to_string(), "ticks": span.ticks()},
                    }),
                });
            }
            if span.phase == Phase::Blocked
                && let Some(cause) = span.attrs.direct_blocker
                && let Some(blocker) = cause_parts(cause).robot
                && work_at(&work, blocker, span.start).is_some()
            {
                flow_id += 1;
                let btid = blocker.0 + 1;
                for (ph, pid, tid) in [("s", PID_PARCELS, tid), ("f", PID_ROBOTS, btid)] {
                    let mut v = json!({
                        "ph": ph, "id": flow_id, "name": "blocked_by", "cat": "blocked",
                        "pid": pid, "tid": tid, "ts": ts,
                    });
                    if ph == "f" {
                        v["bp"] = json!("e");
                    }
                    events.push(Ev {
                        ts,
                        pid,
                        tid,
                        dur: 0,
                        value: v,
                    });
                }
            }
        }
        if !trace.is_open() {
            let ts = micros(trace.done);
            events.push(Ev {
                ts,
                pid: PID_PARCELS,
                tid,
                dur: 0,
                value: json!({
                    "ph": "i", "s": "t", "name": "Dropped", "cat": "Dropped",
                    "pid": PID_PARCELS, "tid": tid, "ts": ts,
                    "args": {"chute": trace.chute.map(|c| c.to_string()), "mis_sorted": trace.mis_sorted},
                }),
            });
        }
    }

    if !faults.is_empty() {
        push_faults(&mut events, faults);
    }

    // Stable sort: parents are pushed before their children, flows after the
    // slices they bind to.
    events.sort_by_key(|e| (e.ts, e.pid, e.tid, Reverse(e.dur)));

    w.write_all(b"{\"traceEvents\":[")?;
    for (i, e) in events.iter().enumerate() {
        if i > 0 {
            w.write_all(b",\n")?;
        }
        write_value(&mut w, &e.value)?;
    }
    w.write_all(b"],\"displayTimeUnit\":\"ms\",\"metadata\":{\"generator\":\"switchyard-trace\",\"tick_us\":500000}}\n")?;
    w.flush()
}

/// The "Faults" process: a track and a start marker per fault.
fn push_faults(events: &mut Vec<Ev>, faults: &[FaultWindow]) {
    events.push(meta("process_name", PID_FAULTS, 0, "name", json!("Faults")));
    events.push(meta(
        "process_sort_index",
        PID_FAULTS,
        0,
        "sort_index",
        json!(0),
    ));
    for f in faults {
        let tid = f.fault.0 + 1;
        let ts = micros(f.start);
        let dur = micros(f.end) - ts;
        events.push(meta("thread_name", PID_FAULTS, tid, "name", json!(f.name)));
        events.push(meta(
            "thread_sort_index",
            PID_FAULTS,
            tid,
            "sort_index",
            json!(tid),
        ));
        events.push(Ev {
            ts,
            pid: PID_FAULTS,
            tid,
            dur,
            value: json!({
                "ph": "X", "name": f.name, "cat": "fault", "cname": "bad",
                "pid": PID_FAULTS, "tid": tid, "ts": ts, "dur": dur, "args": f.args,
            }),
        });
        events.push(Ev {
            ts,
            pid: PID_FAULTS,
            tid,
            dur: 0,
            value: json!({
                "ph": "i", "s": "g", "name": f.name, "cat": "fault", "cname": "bad",
                "pid": PID_FAULTS, "tid": tid, "ts": ts, "args": f.args,
            }),
        });
    }
}
