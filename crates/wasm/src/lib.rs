//! # switchyard-wasm
//!
//! The wasm-bindgen facade the browser viewer loads in a Web Worker
//! (ADR 0001, ARCHITECTURE §8). It is deliberately thin: it builds a
//! [`switchyard_core::Sim`], steps it and copies flat buffers out. The only
//! logic is the mapping of core enums to the viewer's byte codes.
//!
//! # Byte codes (must match `viewer/src/renderer/Renderer.ts`)
//!
//! | Cell kind | code | Robot state   | code |
//! |-----------|------|---------------|------|
//! | free      | 0    | idle          | 0    |
//! | wall      | 1    | to induction  | 1    |
//! | chute     | 2    | loading       | 2    |
//! | induction | 3    | to chute      | 3    |
//! |           |      | dropping      | 4    |
//! |           |      | failed        | 5    |

//! # Session state
//!
//! Besides the sim, [`WasmSim`] keeps what the viewer's sidebar needs, all
//! derived from the event stream (ADR 0004): a [`switchyard_trace::TraceBuilder`]
//! fed by every step, a bounded store of completed traces ([`store`]),
//! delivered parcels per simulated minute, and the fault vocabulary of the
//! Faults tab (`faultmap`).
//!
//! # The fault-free twin
//! The chart compares throughput with a fault-free run of the same seed.
//! `Sim` is `Clone` (a clone is a checkpoint, `sim/replay.rs` in core), so the
//! twin is created **at the moment the first fault is injected**: a clone of
//! the sim taken just before the fault is scheduled, stepped in lockstep from
//! then on. Its per-minute history before that moment is a copy of the main
//! run's, which is identical by determinism. If the scenario itself carries
//! faults (a chaos preset), the twin is built from the scenario with its
//! faults removed, from tick 0.

// wasm-bindgen's generated glue contains `unsafe` blocks; the workspace
// denies unsafe code everywhere else.
#![allow(unsafe_code)]

mod faultmap;
mod json;
pub mod store;

use serde::Serialize;
use switchyard_analysis::postmortem::{self, PostMortem, report};
use switchyard_core::TICKS_PER_HOUR;
use switchyard_core::config::ScenarioConfig;
use switchyard_core::events::Event;
use switchyard_core::faults::FaultSpec;
use switchyard_core::grid::Cell;
use switchyard_core::ids::FaultId;
use switchyard_core::presets;
use switchyard_core::sim::{Sim, Snapshot};
use switchyard_core::world::{Phase, RobotState};
use switchyard_trace::{ParcelTrace, TraceBuilder};
use wasm_bindgen::prelude::{JsError, wasm_bindgen};

use json::{HeadlineOut, MetricsOut, WindowOut};
use store::TraceStore;

/// Ticks in a simulated minute (1 tick = 0.5 s): 120.
pub const TICKS_PER_MINUTE: u32 = TICKS_PER_HOUR / 60;

/// Minutes of history in the chart series.
const WINDOW_MINUTES: usize = 60;

/// Most scheduled faults the in-browser post-mortem accepts (ADR 0006 §8).
/// Replays grow as 2^n and the browser has one thread, so bigger cases belong
/// to the native CLI, which runs them in parallel.
pub const BROWSER_MAX_FAULTS: usize = 4;

/// Longest run (ticks, 3 simulated hours) the in-browser post-mortem takes.
/// Replays restart from the run start, so cost grows with the run: about 5 s
/// at 2.4 h on one thread, a pause the panel warns about; beyond this the
/// pause is too long for a page and the native CLI is the right tool.
pub const BROWSER_MAX_TICKS: u32 = 3 * TICKS_PER_HOUR;

/// Histories the in-browser post-mortem replays on. One means no dither: the
/// report says its bands are thin (ADR 0006 §6); the CLI default is 8.
const BROWSER_DITHER: usize = 1;

/// Report shown when the run has no fault to explain.
const NO_FAULTS_REPORT: &str = "# Post-mortem

> **Nothing to analyse yet.** This run has no scheduled fault, so there is no incident to explain. Inject a fault from the Faults tab, let the run go on for a few simulated minutes, then run the post-mortem again.
";

/// Markdown for a run with more faults than the browser takes on.
fn too_many_faults_report(n: usize, ticks: u32) -> String {
    format!(
        "# Post-mortem

> **Too large for the browser.** This run has {n} scheduled faults; the in-browser post-mortem takes at most {BROWSER_MAX_FAULTS} (replays grow as 2^n and the browser has one thread). Run the full analysis natively:

- `switchyard postmortem --scenario <name> --ticks {ticks}`
"
    )
}

/// Markdown for a run longer than the browser takes on.
fn too_long_report(ticks: u32) -> String {
    format!(
        "# Post-mortem

> **Too long for the browser.** This run is {} simulated minutes; the in-browser post-mortem takes at most {} (every replay restarts from the run start, on one thread). Run the full analysis natively:

- `switchyard postmortem --scenario <name> --ticks {ticks}`
",
        ticks / TICKS_PER_MINUTE,
        BROWSER_MAX_TICKS / TICKS_PER_MINUTE
    )
}

/// The version of the simulation core compiled into this module.
#[wasm_bindgen]
#[must_use]
pub fn version() -> String {
    switchyard_core::VERSION.to_owned()
}

/// Viewer `CellKind` code for a core cell.
#[must_use]
pub const fn cell_code(cell: Cell) -> u8 {
    match cell {
        Cell::Free => 0,
        Cell::Wall => 1,
        Cell::Chute(_) => 2,
        Cell::Induction(_) => 3,
    }
}

/// Viewer `RobotState` code for a core robot state.
#[must_use]
pub const fn robot_state_code(state: RobotState) -> u8 {
    match state {
        RobotState::Idle => 0,
        RobotState::ToInduction => 1,
        RobotState::Loading => 2,
        RobotState::ToChute => 3,
        RobotState::Dropping => 4,
        RobotState::Failed => 5,
    }
}

fn build(toml: &str) -> Result<Sim, String> {
    let cfg = ScenarioConfig::from_toml_str(toml).map_err(|e| e.to_string())?;
    Sim::new(&cfg).map_err(|e| e.to_string())
}

fn preset_toml(name: &str) -> Result<&'static str, String> {
    presets::by_name(name).ok_or_else(|| format!("unknown preset `{name}`"))
}

/// Add the drops in `events` to the per-minute counts.
fn count_drops(events: &[Event], per_minute: &mut Vec<u32>) {
    for e in events {
        if let Event::ParcelDropped { tick, .. } = e {
            let m = (*tick / TICKS_PER_MINUTE) as usize;
            if per_minute.len() <= m {
                per_minute.resize(m + 1, 0);
            }
            per_minute[m] += 1;
        }
    }
}

fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned())
}

/// `sim` with every spec scheduled, plus the first new fault's id; `sim`
/// itself is untouched, so an error midway leaves nothing behind.
fn schedule_all(sim: &Sim, specs: Vec<FaultSpec>) -> Result<(Sim, FaultId), String> {
    let mut next = sim.clone();
    let mut first = None;
    for spec in specs {
        let id = next.schedule_fault(spec).map_err(|e| e.to_string())?;
        first.get_or_insert(id);
    }
    Ok((next, first.ok_or("no fault was scheduled")?))
}

/// The fault-free run the chart compares against.
struct Twin {
    sim: Sim,
    events: Vec<Event>,
    per_minute: Vec<u32>,
}

impl Twin {
    fn step(&mut self) {
        self.events.clear();
        self.sim.step(&mut self.events);
        count_drops(&self.events, &mut self.per_minute);
    }
}

/// A running simulation owned by the viewer's worker.
#[wasm_bindgen]
pub struct WasmSim {
    sim: Sim,
    events: Vec<Event>,
    snap: Snapshot,
    builder: TraceBuilder,
    /// Scratch for traces completed in one tick.
    done: Vec<ParcelTrace>,
    store: TraceStore,
    /// Parcels delivered in each simulated minute.
    per_minute: Vec<u32>,
    twin: Option<Twin>,
}

impl WasmSim {
    /// # Errors
    /// The fault-free twin of a scenario that carries faults could not be
    /// built. Failing loudly beats a chart that silently lacks its baseline.
    fn from_sim(sim: Sim) -> Result<Self, String> {
        // A scenario that carries faults gets its twin from tick 0.
        let twin = if sim.faults().is_empty() {
            None
        } else {
            let mut cfg = sim.config().clone();
            cfg.faults.clear();
            cfg.disabled_faults.clear();
            let twin = Sim::new(&cfg).map_err(|e| format!("building the fault-free twin: {e}"))?;
            Some(Twin {
                sim: twin,
                events: Vec::new(),
                per_minute: Vec::new(),
            })
        };
        let mut s = Self {
            sim,
            events: Vec::new(),
            snap: Snapshot::default(),
            builder: TraceBuilder::new(),
            done: Vec::new(),
            store: TraceStore::default(),
            per_minute: Vec::new(),
            twin,
        };
        s.sim.snapshot(&mut s.snap);
        Ok(s)
    }

    fn refresh(&mut self) {
        self.sim.snapshot(&mut self.snap);
    }

    fn advance(&mut self) {
        self.events.clear();
        self.sim.step(&mut self.events);
        count_drops(&self.events, &mut self.per_minute);
        self.builder.ingest(&self.events, &mut self.done);
        for trace in self.done.drain(..) {
            self.store.push(trace);
        }
        if let Some(twin) = &mut self.twin {
            twin.step();
        }
    }

    /// Schedule the faults of a request; returns the first one as JSON.
    fn inject(&mut self, request: &str) -> Result<String, String> {
        let req: faultmap::Request = serde_json::from_str(request).map_err(|e| e.to_string())?;
        let now = self.sim.tick();
        let specs = faultmap::to_specs(&req, &self.sim, now)?;
        // All or nothing: schedule on a copy and adopt it only if every spec
        // was accepted, so a bad robot in a multi-robot request leaves no
        // earlier fault live (and creates no twin).
        let (next, id) = schedule_all(&self.sim, specs)?;
        // The pre-fault sim becomes the twin: a checkpoint taken just before
        // the first fault, identical to the main run up to here.
        let before = std::mem::replace(&mut self.sim, next);
        if self.twin.is_none() {
            self.twin = Some(Twin {
                sim: before,
                events: Vec::new(),
                per_minute: self.per_minute.clone(),
            });
        }
        self.refresh();
        let floor = faultmap::Floor::of(&self.sim);
        let spec = &self.sim.faults()[id.index()];
        Ok(to_json(&faultmap::active_out(id.0, spec, &floor)))
    }

    fn metrics(&self) -> MetricsOut {
        let done = (self.sim.tick() / TICKS_PER_MINUTE) as usize;
        let start = done.saturating_sub(WINDOW_MINUTES);
        let window = |counts: &[u32]| -> Vec<u32> {
            (start..done)
                .map(|m| counts.get(m).copied().unwrap_or(0))
                .collect()
        };
        let mut faults: Vec<WindowOut> = Vec::new();
        for (i, spec) in self.sim.faults().iter().enumerate() {
            let enabled = u32::try_from(i).is_ok_and(|i| self.sim.fault_enabled(FaultId(i)));
            if !enabled {
                continue;
            }
            let w = WindowOut {
                kind: faultmap::ui_kind(&spec.kind),
                start_tick: spec.at,
                end_tick: spec.end(),
            };
            // A multi-robot request is several core faults with one window.
            if !faults.contains(&w) {
                faults.push(w);
            }
        }
        MetricsOut {
            ticks_per_minute: TICKS_PER_MINUTE,
            start_minute: u32::try_from(start).unwrap_or(0),
            delivered: window(&self.per_minute),
            twin: self.twin.as_ref().map(|t| window(&t.per_minute)),
            faults,
        }
    }

    fn trace(&self, parcel: u32) -> Option<json::TraceOut> {
        let faults = self.sim.faults();
        if let Some(t) = self.store.get(parcel) {
            return Some(json::trace_out(t, faults));
        }
        let phases = &self.sim.world().parcels.phase;
        // Unknown id, or dropped long enough ago to have left the store.
        if phases
            .get(parcel as usize)
            .is_none_or(|&p| p == Phase::Dropped)
        {
            return None;
        }
        self.builder
            .open_traces(self.sim.tick())
            .iter()
            .find(|t| t.parcel.0 == parcel)
            .map(|t| json::trace_out(t, faults))
    }
}

#[wasm_bindgen]
impl WasmSim {
    /// Build from scenario TOML text.
    ///
    /// # Errors
    /// A JS error carrying the parse or validation message, or the reason the
    /// fault-free twin of a faulty scenario could not be built.
    #[wasm_bindgen(constructor)]
    pub fn new(scenario_toml: &str) -> Result<WasmSim, JsError> {
        build(scenario_toml)
            .and_then(Self::from_sim)
            .map_err(|e| JsError::new(&e))
    }

    /// Build from a built-in preset (`"small"`, `"medium"` or `"large"`).
    ///
    /// # Errors
    /// A JS error for an unknown preset name.
    pub fn with_preset(name: &str) -> Result<WasmSim, JsError> {
        let toml = preset_toml(name).map_err(|e| JsError::new(&e))?;
        Self::new(toml)
    }

    /// Grid width in cells.
    #[must_use]
    pub fn width(&self) -> u16 {
        self.sim.layout().grid.width()
    }

    /// Grid height in cells.
    #[must_use]
    pub fn height(&self) -> u16 {
        self.sim.layout().grid.height()
    }

    /// Row-major cell kinds (see the module table).
    #[must_use]
    pub fn cells(&self) -> Vec<u8> {
        self.sim
            .layout()
            .grid
            .cells()
            .iter()
            .map(|&c| cell_code(c))
            .collect()
    }

    /// Advance `n` ticks. Every tick's events feed the trace builder and the
    /// per-minute counts; the event buffer is reused.
    pub fn step(&mut self, n: u32) {
        for _ in 0..n {
            self.advance();
        }
        self.refresh();
    }

    /// Ticks simulated so far.
    #[must_use]
    pub fn tick(&self) -> u32 {
        self.sim.tick()
    }

    /// Interleaved robot coordinates `[x0, y0, x1, y1, ...]`.
    #[must_use]
    pub fn robot_xy(&self) -> Vec<u16> {
        self.snap.robot_xy.clone()
    }

    /// One viewer `RobotState` code per robot.
    #[must_use]
    pub fn robot_state(&self) -> Vec<u8> {
        self.sim
            .world()
            .robots
            .state
            .iter()
            .map(|&s| robot_state_code(s))
            .collect()
    }

    /// Aggregate counters as a JSON object (hand-formatted; all integers).
    #[must_use]
    pub fn stats_json(&self) -> String {
        let s = self.sim.stats();
        format!(
            "{{\"tick\":{},\"arrived\":{},\"queued\":{},\"in_flight\":{},\"delivered\":{},\"mis_sorted\":{},\"throughput_per_hour_milli\":{}}}",
            s.tick,
            s.arrived,
            s.queued,
            s.in_flight,
            s.delivered,
            s.mis_sorted,
            s.throughput_per_hour_milli()
        )
    }

    /// The determinism hash of the full sim state, as 16 hex digits.
    #[must_use]
    pub fn state_hash_hex(&self) -> String {
        format!("{:016x}", self.sim.state_hash())
    }

    /// The chart series as JSON (`MetricsSeries`): delivered parcels per
    /// whole minute over the last hour, the fault-free twin once one exists,
    /// and the fault windows.
    #[must_use]
    pub fn metrics_json(&self) -> String {
        to_json(&self.metrics())
    }

    /// The trace of `parcel` as JSON (`ParcelTrace`), or `null` when the
    /// parcel is unknown or its trace has left the bounded store.
    #[must_use]
    pub fn trace_json(&self, parcel: u32) -> String {
        to_json(&self.trace(parcel))
    }

    /// The parcel a robot is carrying or has claimed, if any.
    #[must_use]
    pub fn parcel_of_robot(&self, robot: u32) -> Option<u32> {
        self.sim
            .world()
            .robots
            .parcel
            .get(robot as usize)
            .copied()
            .flatten()
            .map(|p| p.0)
    }

    /// The fault kinds the Faults tab offers, with ranges for this floor, as
    /// JSON (`FaultKindInfo[]`).
    #[must_use]
    pub fn fault_kinds_json(&self) -> String {
        to_json(&faultmap::kinds(&faultmap::Floor::of(&self.sim)))
    }

    /// Schedule a fault from a JSON `FaultRequest`. It starts at the later of
    /// the request's tick and the sim's current tick. Returns the first
    /// scheduled fault as JSON (`ActiveFault`); a request for several robots
    /// schedules one core fault per robot.
    ///
    /// # Errors
    /// A JS error for malformed JSON, an unknown kind or a bad parameter.
    pub fn inject_fault(&mut self, request: &str) -> Result<String, JsError> {
        self.inject(request).map_err(|e| JsError::new(&e))
    }

    /// Faults active at `tick` (or now, if later) as JSON (`ActiveFault[]`).
    #[must_use]
    pub fn active_faults_json(&self, tick: u32) -> String {
        let t = tick.max(self.sim.tick());
        let floor = faultmap::Floor::of(&self.sim);
        let active: Vec<_> = self
            .sim
            .faults()
            .iter()
            .enumerate()
            .filter_map(|(i, spec)| {
                let id = u32::try_from(i).ok()?;
                (self.sim.fault_enabled(FaultId(id)) && spec.is_active_at(t))
                    .then(|| faultmap::active_out(id, spec, &floor))
            })
            .collect();
        to_json(&active)
    }

    /// The post-mortem of the run so far, as Markdown (ADR 0006 §8: small
    /// cases run in the browser, bigger ones are precomputed or native).
    ///
    /// It replays the scenario this session holds, including the faults
    /// injected from the viewer, up to the current tick, so it blocks the
    /// worker for as long as the replays take. A run with no faults, or with
    /// more than [`BROWSER_MAX_FAULTS`], or longer than [`BROWSER_MAX_TICKS`],
    /// gets an explanatory report instead
    /// of an analysis.
    #[must_use]
    pub fn postmortem_markdown(&self) -> String {
        match self.run_postmortem() {
            Ok(pm) => report::render(&pm),
            Err(text) => text,
        }
    }

    /// The one-glance summary of the run's post-mortem, as JSON
    /// (`PostmortemHeadline`), or `null` when no incident was detected or when
    /// the run is out of the browser's range (same guards as
    /// [`Self::postmortem_markdown`]).
    #[must_use]
    pub fn postmortem_headline(&self) -> String {
        let headline = self.run_postmortem().ok().and_then(|pm| pm.headline());
        to_json(&headline.as_ref().map(HeadlineOut::from))
    }

    /// Run the in-browser post-mortem, or return the report text explaining
    /// why this run is out of range.
    fn run_postmortem(&self) -> Result<PostMortem, String> {
        let cfg = self.sim.config();
        let n = cfg.enabled_faults().count();
        let ticks = self.sim.tick();
        if n == 0 {
            return Err(NO_FAULTS_REPORT.to_owned());
        }
        if n > BROWSER_MAX_FAULTS {
            return Err(too_many_faults_report(n, ticks));
        }
        if ticks > BROWSER_MAX_TICKS {
            return Err(too_long_report(ticks));
        }
        let options = postmortem::Options {
            dither: BROWSER_DITHER,
            ..postmortem::Options::default()
        };
        postmortem::run(cfg, ticks, &options).map_err(|e| {
            format!(
                "# Post-mortem

> **Failed.** {e}
"
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use switchyard_core::faults::FaultKind;
    use switchyard_core::ids::{ChuteId, StationId};

    /// A session over `sim`; the tests' scenarios always have a twin.
    fn ws(sim: Sim) -> WasmSim {
        WasmSim::from_sim(sim).unwrap()
    }

    #[test]
    fn version_matches_core() {
        assert_eq!(version(), switchyard_core::VERSION);
    }

    #[test]
    fn cell_codes_match_viewer_and_core() {
        let cells = [
            Cell::Free,
            Cell::Wall,
            Cell::Chute(ChuteId(0)),
            Cell::Induction(StationId(0)),
        ];
        for (i, c) in cells.into_iter().enumerate() {
            assert_eq!(usize::from(cell_code(c)), i);
            assert_eq!(cell_code(c), c.code());
        }
    }

    #[test]
    fn robot_state_codes_match_viewer_and_core() {
        let states = [
            RobotState::Idle,
            RobotState::ToInduction,
            RobotState::Loading,
            RobotState::ToChute,
            RobotState::Dropping,
            RobotState::Failed,
        ];
        for (i, s) in states.into_iter().enumerate() {
            assert_eq!(usize::from(robot_state_code(s)), i);
            assert_eq!(robot_state_code(s), s as u8);
        }
    }

    #[test]
    fn preset_builds_and_steps() {
        let mut w = ws(build(preset_toml("small").unwrap()).unwrap());
        w.step(50);
        assert_eq!(w.tick(), 50);
        assert_eq!(
            w.cells().len(),
            usize::from(w.width()) * usize::from(w.height())
        );
        assert_eq!(w.robot_xy().len(), 2 * w.robot_state().len());
        assert_eq!(w.state_hash_hex().len(), 16);
        assert!(w.stats_json().starts_with("{\"tick\":50,"));
    }

    #[test]
    fn every_preset_builds() {
        for name in ["small", "medium", "large"] {
            assert!(build(preset_toml(name).unwrap()).is_ok(), "{name}");
        }
    }

    #[test]
    fn bad_inputs_are_errors() {
        assert!(preset_toml("huge").is_err());
        assert!(build("not = [valid").is_err());
    }

    // ---- sidebar services -------------------------------------------------

    use serde_json::{Value, json};

    /// The contract shared with `viewer/src/data/types.ts`.
    fn fixture() -> Value {
        let text = include_str!("../../../viewer/src/data/fixtures/wasm-shapes.json");
        serde_json::from_str(text).unwrap()
    }

    /// Keys the viewer treats as optional: they may be absent.
    const OPTIONAL: [&str; 3] = ["twin", "unit", "blockChain"];

    /// Panic unless `actual` has the shape of `shape`: every key of `actual`
    /// is in `shape`, every non-optional key of `shape` is in `actual`, and
    /// scalar kinds agree (`null` matches anything).
    fn conforms(actual: &Value, shape: &Value, path: &str) {
        match (actual, shape) {
            (Value::Null, _) | (_, Value::Null) => {}
            // A fault's parameters are free-form numbers, keyed by kind.
            (Value::Object(a), Value::Object(_)) if path.ends_with(".params") => {
                assert!(a.values().all(Value::is_number), "{path} must be numbers");
            }
            (Value::Object(a), Value::Object(s)) => {
                for (k, v) in a {
                    let sv = s
                        .get(k)
                        .unwrap_or_else(|| panic!("{path}.{k} is not in the fixture"));
                    conforms(v, sv, &format!("{path}.{k}"));
                }
                for k in s.keys().filter(|k| !k.starts_with('_')) {
                    assert!(
                        a.contains_key(k) || OPTIONAL.contains(&k.as_str()),
                        "{path}.{k} is missing"
                    );
                }
            }
            (Value::Array(a), Value::Array(s)) => {
                if let Some(first) = s.first() {
                    for (i, v) in a.iter().enumerate() {
                        conforms(v, first, &format!("{path}[{i}]"));
                    }
                }
            }
            (Value::Number(_), Value::Number(_))
            | (Value::String(_), Value::String(_))
            | (Value::Bool(_), Value::Bool(_)) => {}
            _ => panic!("{path}: {actual} does not match the fixture's {shape}"),
        }
    }

    fn small() -> WasmSim {
        ws(build(preset_toml("small").unwrap()).unwrap())
    }

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    fn request(kind: &str, params: &Value, at: u32, duration: u32) -> String {
        json!({"kind": kind, "params": params, "atTick": at, "durationTicks": duration}).to_string()
    }

    #[test]
    fn ticks_per_minute_is_120() {
        assert_eq!(TICKS_PER_MINUTE, 120);
    }

    #[test]
    fn metrics_follow_the_shared_shape_with_and_without_a_twin() {
        let fx = fixture();
        let mut w = small();
        w.step(600);
        let before = parse(&w.metrics_json());
        conforms(&before, &fx["metrics"], "metrics");
        assert!(before.get("twin").is_none(), "no twin before a fault");
        assert_eq!(before["delivered"].as_array().unwrap().len(), 5);
        assert_eq!(before["ticksPerMinute"], 120);

        w.inject(&request("robot_failure", &json!({"count": 2}), 600, 600))
            .unwrap();
        w.step(1200);
        let after = parse(&w.metrics_json());
        conforms(&after, &fx["metrics"], "metrics");
        let delivered = after["delivered"].as_array().unwrap();
        let twin = after["twin"].as_array().unwrap();
        assert_eq!(delivered.len(), 15);
        assert_eq!(twin.len(), 15);
        // The twin has the main run's history up to the fault.
        assert_eq!(delivered[..5], twin[..5]);
        let faults = after["faults"].as_array().unwrap();
        assert_eq!(faults.len(), 1, "two robots, one window");
        assert_eq!(faults[0]["kind"], "robot_failure");
        assert_eq!(faults[0]["startTick"], 600);
        assert_eq!(faults[0]["endTick"], 1200);
    }

    #[test]
    fn twin_matches_a_plain_fault_free_run() {
        let mut w = small();
        w.step(300);
        w.inject(&request("chute_jam", &json!({"chute": 3}), 300, 300))
            .unwrap();
        w.step(900);

        let mut plain = build(preset_toml("small").unwrap()).unwrap();
        let (mut events, mut counts) = (Vec::new(), Vec::new());
        for _ in 0..1200 {
            events.clear();
            plain.step(&mut events);
            count_drops(&events, &mut counts);
        }
        assert_eq!(w.twin.as_ref().unwrap().per_minute, counts);
        assert_eq!(w.twin.as_ref().unwrap().sim.tick(), w.sim.tick());
    }

    #[test]
    fn a_scenario_with_faults_has_a_twin_from_the_start() {
        let toml = presets::by_name("peak-surge").unwrap();
        let mut w = ws(build(toml).unwrap());
        w.step(240);
        let m = parse(&w.metrics_json());
        assert_eq!(m["twin"].as_array().unwrap().len(), 2);
        assert!(!m["faults"].as_array().unwrap().is_empty());
    }

    #[test]
    fn traces_follow_the_shared_shape_and_name_the_dead_robot() {
        let fx = fixture();
        let mut w = small();
        w.step(1500);
        // A completed parcel and an open one.
        let done = parse(&w.trace_json(0));
        conforms(&done, &fx["trace"], "trace");
        assert_eq!(done["parcelId"], 0);
        assert!(w.trace_json(u32::MAX) == "null");

        let reply = w
            .inject(&request("robot_failure", &json!({"count": 1}), 1500, 3000))
            .unwrap();
        let fault = parse(&reply);
        let dead = fault["params"]["robot"].as_f64().unwrap() as u64;
        assert!(dead < 40);
        w.step(400);
        let parcel = w.parcel_of_robot(u32::try_from(dead).unwrap());
        let parcel = parcel.expect("the dead robot holds its parcel");
        let t = parse(&w.trace_json(parcel));
        conforms(&t, &fx["trace"], "trace");
        let spans = t["spans"].as_array().unwrap();
        let blocked = spans.last().unwrap();
        assert_eq!(blocked["phase"], "blocked");
        assert_eq!(blocked["rootCause"], format!("dead robot R{dead}"));
        assert_eq!(t["robot"].as_u64(), Some(dead));
    }

    #[test]
    fn open_traces_tile_up_to_now() {
        let mut w = small();
        w.step(500);
        let parcel = (0..w.sim.world().robots.len())
            .find_map(|r| w.parcel_of_robot(u32::try_from(r).unwrap()))
            .expect("a robot holds a parcel");
        let t = parse(&w.trace_json(parcel));
        let spans = t["spans"].as_array().unwrap();
        assert_eq!(spans.last().unwrap()["endTick"], 500);
        for pair in spans.windows(2) {
            assert_eq!(pair[0]["endTick"], pair[1]["startTick"]);
        }
    }

    #[test]
    fn fault_kinds_follow_the_shared_shape_and_every_default_injects() {
        let fx = fixture();
        let w = small();
        let kinds = parse(&w.fault_kinds_json());
        conforms(&kinds, &fx["faultKinds"], "faultKinds");
        let kinds = kinds.as_array().unwrap();
        assert_eq!(kinds.len(), 6);
        for kind in kinds {
            for pick in ["default", "min", "max"] {
                let mut w = small();
                w.step(200);
                let params: serde_json::Map<String, Value> = kind["params"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| (p["key"].as_str().unwrap().to_owned(), p[pick].clone()))
                    .collect();
                let req = request(
                    kind["id"].as_str().unwrap(),
                    &Value::Object(params),
                    200,
                    100,
                );
                let reply = w
                    .inject(&req)
                    .unwrap_or_else(|e| panic!("{} {pick}: {e}", kind["id"]));
                conforms(&parse(&reply), &fx["activeFault"], "activeFault");
            }
        }
    }

    #[test]
    fn active_faults_follow_the_shared_shape_and_expire() {
        let fx = fixture();
        let mut w = small();
        w.step(100);
        w.inject(&request("volume_surge", &json!({"factor": 1.5}), 100, 200))
            .unwrap();
        w.inject(&request(
            "scanner_drift",
            &json!({"station": 1, "rate": 10}),
            100,
            400,
        ))
        .unwrap();
        w.step(50);
        let now = parse(&w.active_faults_json(150));
        conforms(&now, &json!([fx["activeFault"]]), "active");
        assert_eq!(now.as_array().unwrap().len(), 2);
        assert_eq!(now[0]["params"]["factor"], 1.5);
        assert_eq!(now[1]["params"]["rate"], 10.0);
        w.step(200); // tick 350: the surge (ends 300) is over
        let later = parse(&w.active_faults_json(350));
        assert_eq!(later.as_array().unwrap().len(), 1);
        assert_eq!(later[0]["kind"], "scanner_drift");
    }

    #[test]
    fn a_request_in_the_past_starts_now() {
        let mut w = small();
        w.step(100);
        let f = parse(
            &w.inject(&request("chute_jam", &json!({"chute": 1}), 5, 50))
                .unwrap(),
        );
        assert_eq!(f["atTick"], 100);
        assert_eq!(f["endTick"], 150);
    }

    #[test]
    fn bad_fault_requests_are_errors_not_panics() {
        let mut w = small();
        for bad in [
            "not json".to_owned(),
            request("meteor", &json!({}), 0, 10),
            request("robot_failure", &json!({"count": 0}), 0, 10),
            request("robot_failure", &json!({"count": 1.5}), 0, 10),
            request("robot_failure", &json!({}), 0, 10),
            request("chute_jam", &json!({"chute": 9999}), 0, 10),
            request("aisle_block", &json!({"aisle": 0}), 0, 10),
            request("volume_surge", &json!({"factor": 99}), 0, 10),
            request("chute_jam", &json!({"chute": 1}), 0, 0),
        ] {
            assert!(w.inject(&bad).is_err(), "{bad}");
        }
        assert!(w.twin.is_none() || w.sim.faults().is_empty());
        assert!(w.sim.faults().is_empty(), "nothing was scheduled");
    }

    #[test]
    fn aisle_block_matches_the_chaos_preset_cells() {
        let w = ws(build(presets::by_name("aisle-7-blocked").unwrap()).unwrap());
        let mut clean = ws(build(presets::MEDIUM_TOML).unwrap());
        clean
            .inject(&request("aisle_block", &json!({"aisle": 7}), 0, 100))
            .unwrap();
        let FaultKind::AisleBlock { cells } = &clean.sim.faults()[0].kind else {
            panic!("not an aisle block");
        };
        let FaultKind::AisleBlock { cells: expected } = &w.sim.faults()[0].kind else {
            panic!("not an aisle block");
        };
        assert_eq!(cells, expected);
    }

    #[test]
    fn a_failed_multi_robot_request_leaves_no_fault_and_no_twin() {
        let mut w = small();
        w.step(100);
        let n = u32::try_from(w.sim.world().robots.len()).unwrap();
        let spec = |robot: u32| FaultSpec {
            at: 100,
            duration: 300,
            kind: FaultKind::RobotFailure {
                robot: switchyard_core::ids::RobotId(robot),
            },
        };
        // Three valid robots, then one that does not exist.
        let specs = vec![spec(0), spec(1), spec(2), spec(n + 5)];
        let before = w.sim.faults().len();
        assert!(schedule_all(&w.sim, specs).is_err());
        assert_eq!(w.sim.faults().len(), before, "no fault was left live");
        assert!(w.twin.is_none(), "no twin was created");
        // The all-valid prefix does schedule, on the copy only.
        let (next, id) = schedule_all(&w.sim, vec![spec(0), spec(1)]).unwrap();
        assert_eq!(next.faults().len(), before + 2);
        assert_eq!(id.index(), before);
        assert_eq!(w.sim.faults().len(), before);
    }

    #[test]
    fn postmortem_without_faults_says_there_is_nothing_to_analyse() {
        let md = small().postmortem_markdown();
        assert!(md.starts_with(
            "# Post-mortem
"
        ));
        assert!(md.contains("Nothing to analyse"));
    }

    #[test]
    fn postmortem_beyond_the_browser_limit_points_to_the_cli() {
        let md = too_many_faults_report(BROWSER_MAX_FAULTS + 1, 7200);
        assert!(md.contains("switchyard postmortem"));
        assert!(md.contains("--ticks 7200"));
    }

    #[test]
    fn postmortem_beyond_the_browser_tick_cap_points_to_the_cli() {
        let mut w = small();
        w.inject(&request("robot_failure", &json!({"count": 1}), 600, 1200))
            .unwrap();
        w.step(BROWSER_MAX_TICKS + 1);
        let md = w.postmortem_markdown();
        assert!(md.contains("Too long for the browser"), "{md}");
        assert!(
            md.contains(&format!("--ticks {}", BROWSER_MAX_TICKS + 1)),
            "{md}"
        );
    }

    #[test]
    fn postmortem_of_an_injected_fault_is_a_real_report() {
        let mut w = small();
        w.step(120);
        w.inject(&request("robot_failure", &json!({"count": 2}), 600, 1200))
            .unwrap();
        w.step(4800);
        let md = w.postmortem_markdown();
        assert!(md.starts_with("# "), "{md}");
        assert!(!md.contains("Nothing to analyse"), "{md}");
        assert!(!md.contains("**Failed.**"), "{md}");
    }

    #[test]
    fn postmortem_headline_follows_the_shared_shape() {
        let fx = fixture();
        let mut w = small();
        w.step(120);
        let busy = w.sim.world().destination_chute[0];
        w.inject(&request("chute_jam", &json!({"chute": busy.0}), 600, 1800))
            .unwrap();
        w.step(4800);
        let h = parse(&w.postmortem_headline());
        conforms(&h, &fx["headline"], "headline");
        assert!(h["cause"].is_object(), "{h}");
    }

    #[test]
    fn postmortem_headline_without_faults_is_null() {
        assert_eq!(small().postmortem_headline(), "null");
    }
}
