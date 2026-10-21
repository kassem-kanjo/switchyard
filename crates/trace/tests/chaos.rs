//! The chaos presets through the trace builder: fault delay is attributed in
//! every phase, spans still tile, blame is conserved and agrees with the raw
//! `ParcelDelayed` events, and the Perfetto export shows the faults.
//!
//! Events are streamed tick by tick (peak-surge emits ~657k `ParcelDelayed`),
//! so memory stays bounded by the parcels in flight.

use std::collections::BTreeMap;

use serde_json::Value;
use switchyard_core::config::ScenarioConfig;
use switchyard_core::events::Event;
use switchyard_core::faults::FaultSpec;
use switchyard_core::ids::FaultId;
use switchyard_core::presets;
use switchyard_core::sim::Sim;
use switchyard_core::world::Phase;
use switchyard_trace::export::{fault_windows, write_perfetto_with_faults};
use switchyard_trace::{BlameIndex, ParcelTrace, TraceBuilder};

struct Run {
    cfg: ScenarioConfig,
    end: u32,
    traces: Vec<ParcelTrace>,
    /// Raw `ParcelDelayed` events per root fault.
    delayed: BTreeMap<FaultId, u64>,
    /// Fault events only (a handful).
    fault_events: Vec<Event>,
}

fn run_preset(name: &str) -> Run {
    let text = presets::by_name(name).expect("chaos preset");
    let cfg = ScenarioConfig::from_toml_str(text).unwrap();
    let end = cfg.faults.iter().map(FaultSpec::end).max().unwrap() + 600;
    let mut sim = Sim::new(&cfg).unwrap();
    let mut builder = TraceBuilder::new();
    let (mut events, mut traces) = (Vec::new(), Vec::new());
    let mut delayed: BTreeMap<FaultId, u64> = BTreeMap::new();
    let mut fault_events = Vec::new();
    for _ in 0..end {
        events.clear();
        sim.step(&mut events);
        for e in &events {
            match e {
                Event::ParcelDelayed { root, .. } => {
                    *delayed
                        .entry(root.fault().expect("root names a fault"))
                        .or_default() += 1;
                }
                Event::FaultStarted { .. } | Event::FaultEnded { .. } => fault_events.push(*e),
                _ => {}
            }
        }
        builder.ingest(&events, &mut traces);
    }
    traces.extend(builder.open_traces(sim.tick()));
    Run {
        cfg,
        end: sim.tick(),
        traces,
        delayed,
        fault_events,
    }
}

fn blame(traces: &[ParcelTrace]) -> BlameIndex {
    let mut idx = BlameIndex::new();
    for t in traces {
        idx.add_trace(t);
    }
    idx
}

fn check(name: &str) -> Run {
    let run = run_preset(name);
    for t in &run.traces {
        if let Err(e) = t.check_tiling() {
            panic!("{name}: {e}");
        }
    }
    let idx = blame(&run.traces);
    // Every fault's span ticks equal its raw ParcelDelayed events.
    for i in 0..run.cfg.faults.len() {
        let f = FaultId(u32::try_from(i).unwrap());
        let events = run.delayed.get(&f).copied().unwrap_or(0);
        assert!(events > 0, "{name}: {f} delayed nothing");
        assert_eq!(idx.fault_delay(f), events, "{name}: {f}");
    }
    // Conservation: faults, phases and (fault, phase) cells all total the same.
    let total = idx.total_fault_delay_ticks();
    assert_eq!(total, run.delayed.values().sum::<u64>(), "{name}");
    assert_eq!(
        total,
        idx.fault_delays().iter().map(|&(_, t)| t).sum::<u64>()
    );
    let by_phase: u64 = [
        Phase::Queued,
        Phase::AwaitingRobot,
        Phase::Loading,
        Phase::Travelling,
        Phase::Blocked,
        Phase::ChuteWait,
        Phase::Dropped,
    ]
    .iter()
    .map(|&p| idx.fault_delay_by_phase(p))
    .sum();
    assert_eq!(total, by_phase, "{name}");
    assert_eq!(idx.fault_delay_by_phase(Phase::Dropped), 0);
    run
}

#[test]
fn two_dead_robots_and_a_jam_matches_the_core_numbers() {
    let run = check("two-dead-robots-and-a-jam");
    let idx = blame(&run.traces);
    let got: Vec<u64> = (0..3).map(|i| idx.fault_delay(FaultId(i))).collect();
    assert_eq!(got, [53_383, 311_249, 340_696]);
}

#[test]
fn aisle_7_blocked_attributes_delay() {
    check("aisle-7-blocked");
}

#[test]
fn peak_surge_attributes_delay_in_the_queue() {
    let run = check("peak-surge");
    let idx = blame(&run.traces);
    assert!(idx.fault_delay_by_phase(Phase::Queued) > 0);
}

#[test]
fn perfetto_shows_one_track_per_fault_with_markers_and_labels() {
    let run = run_preset("two-dead-robots-and-a-jam");
    let windows = fault_windows(&run.cfg.faults, &run.fault_events, run.end);
    assert_eq!(windows.len(), run.cfg.faults.len());
    // Keep the document small: a slice of the traces that includes delayed ones.
    let sample: Vec<ParcelTrace> = run
        .traces
        .iter()
        .filter(|t| t.spans.iter().any(|s| s.attrs.fault().is_some()))
        .take(40)
        .cloned()
        .collect();
    assert!(!sample.is_empty());
    let mut buf = Vec::new();
    write_perfetto_with_faults(&mut buf, &sample, &windows).unwrap();
    let doc: Value = serde_json::from_slice(&buf).unwrap();
    let events = doc["traceEvents"].as_array().unwrap();

    let fault_tracks: Vec<&Value> = events
        .iter()
        .filter(|e| e["ph"] == "X" && e["pid"] == 3)
        .collect();
    assert_eq!(fault_tracks.len(), windows.len());
    for (w, e) in windows.iter().zip(&fault_tracks) {
        assert_eq!(e["name"].as_str().unwrap(), w.name);
        assert_eq!(e["ts"].as_u64().unwrap(), u64::from(w.start) * 500_000);
        assert_eq!(
            e["dur"].as_u64().unwrap(),
            u64::from(w.end - w.start) * 500_000
        );
    }
    assert!(
        windows
            .iter()
            .any(|w| w.name.starts_with("robot-failure R"))
    );
    let markers = events
        .iter()
        .filter(|e| e["ph"] == "i" && e["s"] == "g")
        .count();
    assert_eq!(markers, windows.len());
    // Delayed sub-spans carry the fault in their name.
    assert!(events.iter().any(|e| {
        e["ph"] == "X"
            && e["pid"] == 1
            && e["name"]
                .as_str()
                .is_some_and(|n| n.contains(" \u{b7} fault F"))
    }));
}
