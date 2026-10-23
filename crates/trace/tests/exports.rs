//! Exporter structure: parse each format back and check what the consuming
//! tools require. One small export is snapshotted with `insta`.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use switchyard_core::Tick;
use switchyard_core::config::ScenarioConfig;
use switchyard_core::events::{Event, WaitCause};
use switchyard_core::ids::{ChuteId, DestinationId, ParcelId, RobotId, StationId};
use switchyard_core::presets::SMALL_TOML;
use switchyard_core::sim::Sim;
use switchyard_core::world::Phase;
use switchyard_trace::export::{OtlpOptions, write_jsonl, write_otlp, write_perfetto};
use switchyard_trace::{ParcelTrace, TraceBuilder};

// ---- fixtures ----------------------------------------------------------

fn arrive(tick: Tick, p: u32, station: u16) -> [Event; 2] {
    [
        Event::ParcelArrived {
            tick,
            parcel: ParcelId(p),
            station: StationId(station),
            destination: DestinationId(2),
        },
        Event::ParcelPhase {
            tick,
            parcel: ParcelId(p),
            phase: Phase::Queued,
        },
    ]
}

fn phase(tick: Tick, p: u32, phase: Phase) -> Event {
    Event::ParcelPhase {
        tick,
        parcel: ParcelId(p),
        phase,
    }
}

fn assign(tick: Tick, p: u32, r: u32, station: u16) -> Event {
    Event::ParcelAssigned {
        tick,
        parcel: ParcelId(p),
        robot: RobotId(r),
        station: StationId(station),
    }
}

fn wait(tick: Tick, r: u32, direct: WaitCause, root: WaitCause) -> Event {
    Event::RobotWaited {
        tick,
        robot: RobotId(r),
        direct,
        root,
    }
}

fn drop_ev(tick: Tick, p: u32, r: u32, c: u16) -> [Event; 2] {
    [
        phase(tick, p, Phase::Dropped),
        Event::ParcelDropped {
            tick,
            parcel: ParcelId(p),
            robot: RobotId(r),
            chute: ChuteId(c),
        },
    ]
}

/// Two parcels: P0 (R0) is blocked by P1's robot R1, which is itself held by
/// a chute swap; P1 is still travelling when the run ends (an open trace).
fn tiny() -> Vec<ParcelTrace> {
    let by = |r| WaitCause::BlockedBy(RobotId(r));
    let swap = WaitCause::ChuteUnavailable(ChuteId(5));
    let mut ev = Vec::new();
    ev.extend(arrive(0, 0, 0));
    ev.extend(arrive(1, 1, 0));
    ev.push(assign(1, 0, 0, 0));
    ev.push(phase(1, 0, Phase::AwaitingRobot));
    ev.push(assign(2, 1, 1, 0));
    ev.push(phase(2, 1, Phase::AwaitingRobot));
    ev.push(phase(3, 0, Phase::Loading));
    ev.push(phase(4, 0, Phase::Travelling));
    ev.push(phase(4, 1, Phase::Loading));
    ev.push(phase(5, 1, Phase::Travelling));
    ev.push(phase(6, 0, Phase::Blocked));
    ev.push(wait(6, 0, by(1), swap));
    ev.push(wait(7, 0, by(1), swap));
    ev.push(phase(8, 0, Phase::Travelling));
    ev.push(phase(9, 0, Phase::ChuteWait));
    ev.extend(drop_ev(11, 0, 0, 2));
    ev.push(Event::ParcelMisSorted {
        tick: 11,
        parcel: ParcelId(0),
        chute: ChuteId(2),
        expected: DestinationId(2),
    });
    TraceBuilder::build_all(&ev, 12)
}

fn sim_traces(ticks: u32) -> Vec<ParcelTrace> {
    let cfg = ScenarioConfig::from_toml_str(SMALL_TOML).expect("small preset");
    let mut sim = Sim::new(&cfg).expect("scenario builds");
    let mut events = Vec::new();
    for _ in 0..ticks {
        sim.step(&mut events);
    }
    TraceBuilder::build_all(&events, sim.tick())
}

fn perfetto_json(traces: &[ParcelTrace]) -> Value {
    let mut buf = Vec::new();
    write_perfetto(&mut buf, traces).unwrap();
    serde_json::from_slice(&buf).expect("valid JSON")
}

// ---- Perfetto ----------------------------------------------------------

#[test]
fn perfetto_has_the_required_fields_and_microsecond_timestamps() {
    let doc = perfetto_json(&tiny());
    let events = doc["traceEvents"].as_array().expect("traceEvents array");
    let mut x = 0;
    for e in events {
        let ph = e["ph"].as_str().expect("ph");
        match ph {
            "X" => {
                x += 1;
                for k in ["name", "pid", "tid", "ts", "dur"] {
                    assert!(!e[k].is_null(), "X event without {k}: {e}");
                }
                assert_eq!(e["ts"].as_u64().unwrap() % 500_000, 0);
                assert_eq!(e["dur"].as_u64().unwrap() % 500_000, 0);
            }
            "M" => assert!(e["args"]["name"].is_string() || e["args"]["sort_index"].is_number()),
            "i" | "s" | "f" => {
                assert!(e["ts"].is_number() && e["pid"].is_number() && e["tid"].is_number());
            }
            other => panic!("unexpected phase {other}"),
        }
    }
    // P0: 1 parent + 6 spans (Queued..ChuteWait, Blocked split into one);
    // P1 (open): 1 parent + 4 spans. Robot tracks repeat all spans that had a
    // robot (everything but Queued).
    assert!(x > 10);
    let names: BTreeSet<&str> = events
        .iter()
        .filter(|e| e["ph"] == "M" && e["name"] == "process_name")
        .map(|e| e["args"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, BTreeSet::from(["Parcels", "Robots"]));
}

/// Perfetto's nesting rule: on every track, slices either nest or are
/// disjoint (otherwise they land on overflow tracks).
fn assert_properly_nested(doc: &Value) {
    let mut tracks: BTreeMap<(u64, u64), Vec<(u64, u64)>> = BTreeMap::new();
    for e in doc["traceEvents"].as_array().unwrap() {
        if e["ph"] == "X" {
            let ts = e["ts"].as_u64().unwrap();
            tracks
                .entry((e["pid"].as_u64().unwrap(), e["tid"].as_u64().unwrap()))
                .or_default()
                .push((ts, ts + e["dur"].as_u64().unwrap()));
        }
    }
    for ((pid, tid), mut slices) in tracks {
        // Same order Perfetto uses: by start, longest first.
        slices.sort_by_key(|&(s, e)| (s, std::cmp::Reverse(e)));
        let mut stack: Vec<(u64, u64)> = Vec::new();
        for (s, e) in slices {
            while stack.last().is_some_and(|&(_, top_end)| top_end <= s) {
                stack.pop();
            }
            if let Some(&(ps, pe)) = stack.last() {
                assert!(
                    ps <= s && e <= pe,
                    "slice [{s},{e}) partially overlaps [{ps},{pe}) on pid {pid} tid {tid}"
                );
            }
            stack.push((s, e));
        }
    }
}

#[test]
fn perfetto_slices_nest_on_every_track_for_a_real_run() {
    let traces = sim_traces(1_500);
    assert!(traces.len() > 20);
    assert_properly_nested(&perfetto_json(&traces));
    assert_properly_nested(&perfetto_json(&tiny()));
}

#[test]
fn perfetto_flows_pair_up_and_end_on_a_robot_track() {
    let doc = perfetto_json(&sim_traces(1_500));
    let mut starts: BTreeMap<u64, &Value> = BTreeMap::new();
    let mut ends: BTreeMap<u64, &Value> = BTreeMap::new();
    for e in doc["traceEvents"].as_array().unwrap() {
        match e["ph"].as_str().unwrap() {
            "s" => assert!(starts.insert(e["id"].as_u64().unwrap(), e).is_none()),
            "f" => assert!(ends.insert(e["id"].as_u64().unwrap(), e).is_none()),
            _ => {}
        }
    }
    assert!(
        !starts.is_empty(),
        "a busy run has blocked spans with robot blockers"
    );
    assert_eq!(
        starts.keys().collect::<Vec<_>>(),
        ends.keys().collect::<Vec<_>>()
    );
    for (id, s) in &starts {
        let f = ends[id];
        assert_eq!(s["pid"], 1);
        assert_eq!(f["pid"], 2);
        assert_eq!(f["bp"], "e");
        assert!(f["ts"].as_u64() >= s["ts"].as_u64());
    }
}

#[test]
fn perfetto_flow_in_tiny_run_points_at_the_blocking_robot() {
    let doc = perfetto_json(&tiny());
    let flows: Vec<&Value> = doc["traceEvents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["ph"] == "f")
        .collect();
    assert_eq!(flows.len(), 1);
    // R1 is tid 2 on the robots process; the block starts at tick 6.
    assert_eq!(
        (flows[0]["pid"].as_u64(), flows[0]["tid"].as_u64()),
        (Some(2), Some(2))
    );
    assert_eq!(flows[0]["ts"].as_u64(), Some(6 * 500_000));
}

#[test]
fn perfetto_snapshot_of_the_tiny_run() {
    let mut buf = Vec::new();
    write_perfetto(&mut buf, &tiny()).unwrap();
    insta::assert_snapshot!(String::from_utf8(buf).unwrap());
}

// ---- OTLP --------------------------------------------------------------

fn otlp_json(traces: &[ParcelTrace]) -> Value {
    let mut buf = Vec::new();
    write_otlp(&mut buf, traces, &OtlpOptions::default()).unwrap();
    serde_json::from_slice(&buf).expect("valid JSON")
}

fn otlp_spans(doc: &Value) -> &Vec<Value> {
    doc["resourceSpans"][0]["scopeSpans"][0]["spans"]
        .as_array()
        .expect("spans array")
}

#[test]
fn otlp_has_the_required_structure_and_resolvable_ids() {
    let traces = sim_traces(1_500);
    let doc = otlp_json(&traces);
    assert_eq!(
        doc["resourceSpans"][0]["resource"]["attributes"][0]["key"],
        "service.name"
    );
    let spans = otlp_spans(&doc);
    let expected: usize = traces.iter().map(|t| 1 + t.spans.len()).sum();
    assert_eq!(spans.len(), expected);

    let hex = |s: &Value, len: usize| {
        let s = s.as_str().unwrap();
        assert_eq!(s.len(), len);
        assert!(
            s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
        assert!(s.bytes().any(|b| b != b'0'));
    };
    let mut ids: BTreeMap<(String, String), &Value> = BTreeMap::new();
    for s in spans {
        hex(&s["traceId"], 32);
        hex(&s["spanId"], 16);
        let key = (
            s["traceId"].as_str().unwrap().to_owned(),
            s["spanId"].as_str().unwrap().to_owned(),
        );
        assert!(ids.insert(key, s).is_none(), "duplicate span id");
        assert!(s["name"].is_string());
        let (a, b) = (
            s["startTimeUnixNano"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .unwrap(),
            s["endTimeUnixNano"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .unwrap(),
        );
        assert!(a <= b);
        assert!(
            s["attributes"]
                .as_array()
                .unwrap()
                .iter()
                .all(|a| { a["key"].as_str().unwrap().starts_with("switchyard.") })
        );
    }
    let mut links = 0;
    for ((trace_id, _), s) in &ids {
        if let Some(parent) = s["parentSpanId"].as_str() {
            let p = ids
                .get(&(trace_id.clone(), parent.to_owned()))
                .expect("parent exists in the same trace");
            assert!(p["parentSpanId"].is_null(), "root has no parent");
            assert!(p["startTimeUnixNano"].as_str() <= s["startTimeUnixNano"].as_str());
        }
        for l in s
            .get("links")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            links += 1;
            assert!(
                ids.contains_key(&(
                    l["traceId"].as_str().unwrap().to_owned(),
                    l["spanId"].as_str().unwrap().to_owned()
                )),
                "link target exists"
            );
            assert_eq!(s["name"], "Blocked");
        }
    }
    assert!(links > 0, "a busy run has linked Blocked spans");
}

#[test]
fn otlp_is_deterministic_and_links_the_blocker() {
    let a = otlp_json(&tiny());
    let b = otlp_json(&tiny());
    assert_eq!(a, b);
    let spans = otlp_spans(&a);
    let blocked: Vec<&Value> = spans.iter().filter(|s| s["name"] == "Blocked").collect();
    assert_eq!(blocked.len(), 1);
    let links = blocked[0]["links"].as_array().unwrap();
    // direct = R1 (carrying P1); root is a chute, which has no span to link.
    assert_eq!(links.len(), 1);
    assert_eq!(
        links[0]["attributes"][0]["value"]["stringValue"],
        "blocked_by"
    );
    let target = spans
        .iter()
        .find(|s| s["spanId"] == links[0]["spanId"])
        .unwrap();
    assert!(
        target["attributes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| { a["key"] == "switchyard.parcel.id" && a["value"]["intValue"] == "1" })
    );
    // The mis-sorted parcel's root span is an error.
    let roots: Vec<&Value> = spans.iter().filter(|s| s["name"] == "parcel").collect();
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0]["status"]["code"], 2);
    assert!(roots[1]["status"].is_null());
}

// ---- JSONL -------------------------------------------------------------

#[test]
fn jsonl_has_one_valid_line_per_span() {
    let traces = sim_traces(1_000);
    let mut buf = Vec::new();
    write_jsonl(&mut buf, &traces).unwrap();
    let text = String::from_utf8(buf).unwrap();
    let rows: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).expect("each line is JSON"))
        .collect();
    assert_eq!(
        rows.len(),
        traces.iter().map(|t| t.spans.len()).sum::<usize>()
    );
    for k in [
        "trace_id",
        "span_id",
        "parcel",
        "phase",
        "start_tick",
        "end_tick",
        "ticks",
        "start_us",
        "end_us",
        "robot",
        "direct_kind",
        "root_kind",
        "complete",
        "mis_sorted",
    ] {
        assert!(
            rows.iter().all(|r| r.get(k).is_some()),
            "missing column {k}"
        );
    }
    // Per-parcel ticks sum to the latency.
    let mut sums: BTreeMap<u64, u64> = BTreeMap::new();
    for r in &rows {
        *sums.entry(r["parcel"].as_u64().unwrap()).or_default() += r["ticks"].as_u64().unwrap();
    }
    for t in &traces {
        if !t.spans.is_empty() {
            assert_eq!(sums[&u64::from(t.parcel.0)], u64::from(t.latency()));
        }
    }
    assert!(
        rows.iter()
            .any(|r| r["phase"] == "Blocked" && r["direct_kind"] == "blocked_by")
    );
}
