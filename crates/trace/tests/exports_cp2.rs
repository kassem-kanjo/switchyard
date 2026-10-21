//! CP2 review fixes in the exporters: OTLP run identity and unique keys,
//! Perfetto colours, lane packing and fault tracks.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use switchyard_core::Tick;
use switchyard_core::config::ScenarioConfig;
use switchyard_core::events::{Event, WaitCause};
use switchyard_core::faults::{FaultKind, FaultSpec};
use switchyard_core::ids::{ChuteId, DestinationId, ParcelId, RobotId, StationId};
use switchyard_core::presets::SMALL_TOML;
use switchyard_core::sim::Sim;
use switchyard_core::world::Phase;
use switchyard_trace::export::{
    OtlpOptions, fault_windows, run_id, write_otlp, write_perfetto, write_perfetto_with_faults,
};
use switchyard_trace::{ParcelTrace, TraceBuilder};

fn phase(tick: Tick, p: u32, phase: Phase) -> Event {
    Event::ParcelPhase {
        tick,
        parcel: ParcelId(p),
        phase,
    }
}

/// Two parcels: P0 (R0) blocked by R1 for two ticks, P1 (R1) still going.
fn tiny() -> Vec<ParcelTrace> {
    let arrive = |tick, p| Event::ParcelArrived {
        tick,
        parcel: ParcelId(p),
        station: StationId(0),
        destination: DestinationId(2),
    };
    let assign = |tick, p, r| Event::ParcelAssigned {
        tick,
        parcel: ParcelId(p),
        robot: RobotId(r),
        station: StationId(0),
    };
    let wait = |tick| Event::RobotWaited {
        tick,
        robot: RobotId(0),
        direct: WaitCause::BlockedBy(RobotId(1)),
        root: WaitCause::ChuteUnavailable(ChuteId(5)),
    };
    let ev = vec![
        arrive(0, 0),
        phase(0, 0, Phase::Queued),
        arrive(1, 1),
        phase(1, 1, Phase::Queued),
        assign(1, 0, 0),
        phase(1, 0, Phase::AwaitingRobot),
        assign(2, 1, 1),
        phase(2, 1, Phase::AwaitingRobot),
        phase(3, 0, Phase::Loading),
        phase(4, 0, Phase::Travelling),
        phase(4, 1, Phase::Loading),
        phase(5, 1, Phase::Travelling),
        phase(6, 0, Phase::Blocked),
        wait(6),
        wait(7),
        phase(8, 0, Phase::Travelling),
        phase(9, 0, Phase::ChuteWait),
        phase(11, 0, Phase::Dropped),
        Event::ParcelDropped {
            tick: 11,
            parcel: ParcelId(0),
            robot: RobotId(0),
            chute: ChuteId(2),
        },
    ];
    TraceBuilder::build_all(&ev, 12)
}

fn sim_traces(ticks: u32) -> Vec<ParcelTrace> {
    let cfg = ScenarioConfig::from_toml_str(SMALL_TOML).unwrap();
    let mut sim = Sim::new(&cfg).unwrap();
    let mut events = Vec::new();
    for _ in 0..ticks {
        sim.step(&mut events);
    }
    TraceBuilder::build_all(&events, sim.tick())
}

fn perfetto_json(traces: &[ParcelTrace]) -> Value {
    let mut buf = Vec::new();
    write_perfetto(&mut buf, traces).unwrap();
    serde_json::from_slice(&buf).unwrap()
}

fn otlp_with(traces: &[ParcelTrace], run: [u8; 8]) -> Value {
    let mut buf = Vec::new();
    let opts = OtlpOptions {
        run_id: run,
        ..OtlpOptions::default()
    };
    write_otlp(&mut buf, traces, &opts).unwrap();
    serde_json::from_slice(&buf).unwrap()
}

fn spans(doc: &Value) -> &Vec<Value> {
    doc["resourceSpans"][0]["scopeSpans"][0]["spans"]
        .as_array()
        .unwrap()
}

fn id_set(doc: &Value) -> BTreeSet<(String, String)> {
    spans(doc)
        .iter()
        .map(|s| {
            (
                s["traceId"].as_str().unwrap().to_owned(),
                s["spanId"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn run_id_is_deterministic_and_separates_scenario_seed_and_variant() {
    let a = run_id("[floor]\n", 1, "baseline");
    assert_eq!(a, run_id("[floor]\n", 1, "baseline"));
    for other in [
        run_id("[floor]\n ", 1, "baseline"),
        run_id("[floor]\n", 2, "baseline"),
        run_id("[floor]\n", 1, "fault-F0-disabled"),
    ] {
        assert_ne!(a, other);
    }
}

#[test]
fn otlp_ids_differ_across_runs_and_are_stable_within_one() {
    let traces = tiny();
    let (r1, r2) = (run_id("s", 1, "v"), run_id("s", 2, "v"));
    let (a, a2, b) = (
        otlp_with(&traces, r1),
        otlp_with(&traces, r1),
        otlp_with(&traces, r2),
    );
    assert_eq!(a, a2, "same run, same bytes");
    let (ia, ib) = (id_set(&a), id_set(&b));
    assert_eq!(ia.len(), spans(&a).len());
    assert!(ia.is_disjoint(&ib), "runs share no (trace, span) id");
    let trace_ids =
        |s: &BTreeSet<(String, String)>| s.iter().map(|x| x.0.clone()).collect::<BTreeSet<_>>();
    assert!(trace_ids(&ia).is_disjoint(&trace_ids(&ib)));
}

#[test]
fn otlp_attribute_keys_are_unique_per_span() {
    let mut traces = tiny();
    // A parcel with two robots used to repeat `switchyard.robot.id`.
    traces[0].robots.push(RobotId(7));
    for doc in [
        otlp_with(&traces, [0; 8]),
        otlp_with(&sim_traces(1_000), [0; 8]),
    ] {
        for s in spans(&doc) {
            let keys: Vec<&str> = s["attributes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a["key"].as_str().unwrap())
                .collect();
            let unique: BTreeSet<&str> = keys.iter().copied().collect();
            assert_eq!(keys.len(), unique.len(), "duplicate keys in {keys:?}");
        }
    }
    let doc = otlp_with(&traces, [0; 8]);
    let robots = spans(&doc)[0]["attributes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["key"] == "switchyard.robot.id")
        .unwrap();
    let values = robots["value"]["arrayValue"]["values"].as_array().unwrap();
    assert_eq!(values.len(), 2);
}

/// Colour names Chrome trace-viewer reserves (`reserved_color_scheme`).
const RESERVED_CNAMES: &[&str] = &[
    "thread_state_uninterruptible",
    "thread_state_iowait",
    "thread_state_running",
    "thread_state_runnable",
    "thread_state_sleeping",
    "thread_state_unknown",
    "background_memory_dump",
    "light_memory_dump",
    "detailed_memory_dump",
    "vsync_highlight_color",
    "generic_work",
    "good",
    "bad",
    "terrible",
    "black",
    "grey",
    "white",
    "yellow",
    "olive",
    "rail_response",
    "rail_animation",
    "rail_idle",
    "rail_load",
    "startup",
    "heap_dump_stack_frame",
    "heap_dump_object_type",
    "heap_dump_child_node_arrow",
    "cq_build_running",
    "cq_build_passed",
    "cq_build_failed",
    "cq_build_attempt_running",
    "cq_build_attempt_passed",
    "cq_build_attempt_failed",
];

#[test]
fn perfetto_phase_slices_have_valid_colours_and_matching_robot_names() {
    let doc = perfetto_json(&sim_traces(1_500));
    let events = doc["traceEvents"].as_array().unwrap();
    let mut by_cat: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut robot_cats = BTreeSet::new();
    for e in events.iter().filter(|e| e["ph"] == "X") {
        let cat = e["cat"].as_str().unwrap();
        if cat == "parcel" {
            continue;
        }
        let cname = e["cname"].as_str().expect("phase slice has cname");
        assert!(RESERVED_CNAMES.contains(&cname), "invalid cname {cname}");
        by_cat.entry(cat).or_default().insert(cname);
        if e["pid"] == 2 {
            // Same name as on the parcel track; the parcel id is in args.
            assert_eq!(e["name"], e["cat"]);
            assert!(e["args"]["parcel"].is_string());
            robot_cats.insert(cat);
        }
    }
    assert!(
        by_cat.values().all(|c| c.len() == 1),
        "one colour per phase"
    );
    assert_eq!(by_cat["Blocked"], BTreeSet::from(["terrible"]));
    assert_eq!(by_cat["Travelling"], BTreeSet::from(["good"]));
    assert_eq!(by_cat["Queued"], by_cat["AwaitingRobot"]);
    assert!(robot_cats.contains("Travelling"));
}

/// No drop instant may fall inside a parcel slice of its lane, including the
/// start edge of the next parcel.
fn assert_no_instant_in_a_slice(doc: &Value) {
    let events = doc["traceEvents"].as_array().unwrap();
    let mut slices: BTreeMap<u64, Vec<(u64, u64)>> = BTreeMap::new();
    for e in events
        .iter()
        .filter(|e| e["ph"] == "X" && e["cat"] == "parcel")
    {
        let ts = e["ts"].as_u64().unwrap();
        slices
            .entry(e["tid"].as_u64().unwrap())
            .or_default()
            .push((ts, ts + e["dur"].as_u64().unwrap()));
    }
    let mut instants = 0;
    for e in events.iter().filter(|e| e["ph"] == "i" && e["s"] == "t") {
        instants += 1;
        let (tid, ts) = (e["tid"].as_u64().unwrap(), e["ts"].as_u64().unwrap());
        for &(s, end) in &slices[&tid] {
            assert!(
                !(s <= ts && ts < end),
                "instant {ts} inside [{s},{end}) on lane {tid}"
            );
        }
    }
    assert!(instants > 0);
}

#[test]
fn perfetto_drop_instants_never_land_inside_another_slice() {
    assert_no_instant_in_a_slice(&perfetto_json(&sim_traces(1_500)));
    assert_no_instant_in_a_slice(&perfetto_json(&tiny()));
}

#[test]
fn perfetto_lane_count_is_at_most_peak_concurrency_for_filtered_exports() {
    let mut traces = sim_traces(2_500);
    traces.retain(|t| !t.is_open());
    traces.sort_by_key(|t| std::cmp::Reverse(t.latency()));
    traces.truncate(60);
    let doc = perfetto_json(&traces);
    let lanes = doc["traceEvents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["ph"] == "M" && e["name"] == "thread_name" && e["pid"] == 1)
        .count();
    // Peak concurrency per station, a parcel occupying [arrival, done].
    let mut peak = 0;
    let stations: BTreeSet<_> = traces.iter().map(|t| t.station).collect();
    for s in stations {
        let mut points: Vec<(u32, i32)> = Vec::new();
        for t in traces.iter().filter(|t| t.station == s) {
            points.push((t.arrival, 1));
            points.push((t.done + 1, -1));
        }
        points.sort_unstable();
        let (mut cur, mut max) = (0, 0);
        for (_, d) in points {
            cur += d;
            max = max.max(cur);
        }
        peak += max;
    }
    assert!(traces.len() > 20 && lanes > 0);
    assert!(
        i32::try_from(lanes).unwrap() <= peak,
        "{lanes} lanes for peak concurrency {peak} over {} parcels",
        traces.len()
    );
    assert!(lanes < traces.len(), "lanes are shared, not one per parcel");
}

#[test]
fn perfetto_fault_tracks_are_present_when_faults_are_scheduled() {
    let mut cfg = ScenarioConfig::from_toml_str(SMALL_TOML).unwrap();
    cfg.faults.push(FaultSpec {
        at: 100,
        duration: 80,
        kind: FaultKind::RobotFailure { robot: RobotId(1) },
    });
    cfg.faults.push(FaultSpec {
        at: 200,
        duration: 50,
        kind: FaultKind::ChuteJam { chute: ChuteId(0) },
    });
    let mut sim = Sim::new(&cfg).unwrap();
    let mut events = Vec::new();
    for _ in 0..400 {
        sim.step(&mut events);
    }
    let traces = TraceBuilder::build_all(&events, sim.tick());
    let windows = fault_windows(&cfg.faults, &events, sim.tick());
    assert_eq!(windows.len(), 2);
    assert_eq!((windows[0].start, windows[0].end), (100, 180));
    let mut buf = Vec::new();
    write_perfetto_with_faults(&mut buf, &traces, &windows).unwrap();
    let doc: Value = serde_json::from_slice(&buf).unwrap();
    let names: Vec<&str> = doc["traceEvents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["ph"] == "M" && e["name"] == "thread_name" && e["pid"] == 3)
        .map(|e| e["args"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["robot-failure R1", "chute-jam C0"]);
    // And no Faults process without faults.
    let plain = perfetto_json(&traces);
    assert!(
        !plain["traceEvents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["pid"] == 3)
    );
}
