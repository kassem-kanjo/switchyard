//! Traces built from the real simulator.
//!
//! - Spans tile `[arrival, done)` exactly for every parcel, on the presets
//!   and on random small floors (proptest), complete and open.
//! - Streaming (tick by tick) equals batch.
//! - Conservation: Blocked span ticks equal the blame totals; complete
//!   traces match the event stream's drops.

use proptest::prelude::*;
use switchyard_core::Tick;
use switchyard_core::config::{FloorConfig, ScenarioConfig};
use switchyard_core::events::Event;
use switchyard_core::presets::SMALL_TOML;
use switchyard_core::sim::Sim;
use switchyard_core::world::Phase;
use switchyard_trace::{BlameIndex, ParcelTrace, TraceBuilder};

fn small(seed: u64) -> ScenarioConfig {
    let mut cfg = ScenarioConfig::from_toml_str(SMALL_TOML).expect("small preset parses");
    cfg.seed = seed;
    cfg
}

/// Run the sim and the builder together, tick by tick.
fn run(cfg: &ScenarioConfig, ticks: u32) -> (Vec<Event>, Vec<ParcelTrace>, Vec<ParcelTrace>, Tick) {
    let mut sim = Sim::new(cfg).expect("scenario builds");
    let mut builder = TraceBuilder::new();
    let (mut buf, mut all, mut done) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..ticks {
        sim.step(&mut buf);
        builder.ingest(&buf, &mut done);
        all.extend_from_slice(&buf);
        buf.clear();
    }
    let open = builder.open_traces(sim.tick());
    (all, done, open, sim.tick())
}

fn check_all(events: &[Event], done: &[ParcelTrace], open: &[ParcelTrace], end: Tick) {
    for t in done.iter().chain(open) {
        if let Err(e) = t.check_tiling() {
            panic!("{e}");
        }
        assert!(t.done <= end);
    }
    // Complete traces are exactly the dropped parcels.
    let dropped = events
        .iter()
        .filter(|e| matches!(e, Event::ParcelDropped { .. }))
        .count();
    assert_eq!(done.len(), dropped);
    assert!(done.iter().all(|t| !t.is_open() && t.chute.is_some()));
    assert!(open.iter().all(ParcelTrace::is_open));
    // Every arrival has exactly one trace.
    let arrived = events
        .iter()
        .filter(|e| matches!(e, Event::ParcelArrived { .. }))
        .count();
    assert_eq!(done.len() + open.len(), arrived);
    // Streaming equals batch.
    let mut batch = TraceBuilder::build_all(events, end);
    let mut streamed: Vec<ParcelTrace> = done.iter().chain(open).cloned().collect();
    batch.sort_by_key(|t| t.parcel);
    streamed.sort_by_key(|t| t.parcel);
    assert_eq!(batch, streamed);
}

#[test]
fn small_preset_traces_tile_and_conserve_blame() {
    let (events, done, open, end) = run(&small(42), 2_000);
    assert!(!done.is_empty(), "nothing was delivered");
    check_all(&events, &done, &open, end);

    let mut blame = BlameIndex::new();
    let mut blocked_ticks = 0u64;
    for t in done.iter().chain(&open) {
        blame.add_trace(t);
        blocked_ticks += t
            .spans
            .iter()
            .filter(|s| s.phase == Phase::Blocked)
            .map(|s| u64::from(s.ticks()))
            .sum::<u64>();
    }
    assert_eq!(blame.total_blocked_ticks(), blocked_ticks);
    let attributed: u64 = blame.roots().map(|b| b.ticks).sum();
    assert_eq!(attributed + blame.unattributed_ticks(), blocked_ticks);
    // Every Blocked tick of a parcel on a working robot has a RobotWaited.
    assert_eq!(
        blame.unattributed_ticks(),
        0,
        "blocked ticks without a wait event"
    );
    let top = blame.top_roots(5);
    assert!(top.windows(2).all(|w| w[0].ticks >= w[1].ticks));
}

/// The same total from the raw events: each `RobotWaited` of a robot whose
/// parcel is `Blocked` is one blocked tick.
#[test]
fn blocked_ticks_match_phase_durations_from_raw_events() {
    let (events, done, open, _) = run(&small(7), 1_500);
    let mut from_traces = 0u64;
    for t in done.iter().chain(&open) {
        from_traces += t
            .spans
            .iter()
            .filter(|s| s.phase == Phase::Blocked)
            .map(|s| u64::from(s.ticks()))
            .sum::<u64>();
    }
    // Independent count: phase events give the Blocked interval of each
    // parcel; open intervals end at the last tick.
    let mut since: std::collections::BTreeMap<u32, (Phase, Tick)> = Default::default();
    let mut blocked = 0u64;
    for e in &events {
        if let Event::ParcelPhase {
            tick,
            parcel,
            phase,
        } = *e
            && let Some((Phase::Blocked, start)) = since.insert(parcel.0, (phase, tick))
        {
            blocked += u64::from(tick - start);
        }
    }
    let end = 1_500;
    for (phase, start) in since.values() {
        if *phase == Phase::Blocked {
            blocked += u64::from(end - start);
        }
    }
    assert_eq!(from_traces, blocked);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]
    /// Random seeds and small floors: every trace tiles its lifetime.
    #[test]
    fn spans_tile_the_lifetime_on_random_small_floors(
        seed in any::<u64>(),
        width in 12u16..26,
        height in 12u16..26,
        stations in 1u16..4,
        south_margin in 2u16..5,
        robots in 1u32..40,
        rate in 100u32..1_500,
        capacity in 2u32..8,
    ) {
        let mut cfg = small(seed);
        cfg.floor = FloorConfig { width, height, stations, south_margin };
        cfg.fleet.robots = robots;
        cfg.arrivals.parcels_per_hour_per_station = rate;
        cfg.arrivals.destinations = None;
        cfg.chutes.bag_capacity = capacity;
        cfg.chutes.bag_swap_ticks = 20;
        if Sim::new(&cfg).is_ok() {
            let (events, done, open, end) = run(&cfg, 500);
            check_all(&events, &done, &open, end);
        }
    }
}
