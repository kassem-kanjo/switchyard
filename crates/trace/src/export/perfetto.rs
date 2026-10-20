//! Chrome trace-event JSON export, for <https://ui.perfetto.dev>.
//!
//! Layout (`1 tick = 0.5 s = 500,000 us`):
//!
//! - **Process 1 "Parcels"**: one track per *station lane*, not one per
//!   parcel (a busy run has thousands of parcels and Perfetto's track list
//!   does not scale to that). Parcels of a station are packed greedily onto
//!   the lowest lane that is free at their arrival, so a lane holds one
//!   parcel at a time. Each parcel is a parent slice `P<id>` over
//!   `[arrival, done)` with its phase spans as **child slices** nested
//!   inside it. Nesting is exact because spans tile the lifetime; that is
//!   what keeps Perfetto from pushing slices onto overflow tracks. Lanes are
//!   named `S<station> lane <k>`; a drop is an instant event.
//! - **Process 2 "Robots"**: one track per robot with a slice for each span
//!   the robot carried (`AwaitingRobot` to `ChuteWait`), named
//!   `<phase> P<id>`. A robot works one parcel at a time, so they never
//!   overlap.
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
use switchyard_core::ids::{RobotId, StationId};
use switchyard_core::world::Phase;

use crate::MICROS_PER_TICK;
use crate::model::{ParcelTrace, cause_label, cause_parts, phase_name};

use super::otlp::{robot_work, work_at};
use super::write_value;

const PID_PARCELS: u32 = 1;
const PID_ROBOTS: u32 = 2;

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
            .position(|&end| end <= t.arrival)
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
pub fn write_perfetto<W: Write>(mut w: W, traces: &[ParcelTrace]) -> io::Result<()> {
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
            let name = phase_name(span.phase);
            let mut args = if span.phase == Phase::Blocked {
                blocked_args(span.attrs.direct_blocker, span.attrs.root_cause)
            } else {
                json!({})
            };
            args["ticks"] = json!(span.ticks());
            if let Some(r) = span.attrs.robot {
                args["robot"] = json!(r.to_string());
            }
            events.push(Ev {
                ts,
                pid: PID_PARCELS,
                tid,
                dur,
                value: json!({
                    "ph": "X", "name": name, "cat": name,
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
                        "ph": "X", "name": format!("{name} {}", trace.parcel), "cat": name,
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
