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

// wasm-bindgen's generated glue contains `unsafe` blocks; the workspace
// denies unsafe code everywhere else.
#![allow(unsafe_code)]

use switchyard_core::config::ScenarioConfig;
use switchyard_core::events::Event;
use switchyard_core::grid::Cell;
use switchyard_core::presets::SMALL_TOML;
use switchyard_core::sim::{Sim, Snapshot};
use switchyard_core::world::RobotState;
use wasm_bindgen::prelude::{JsError, wasm_bindgen};

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
    match name {
        "small" => Ok(SMALL_TOML),
        other => Err(format!("unknown preset `{other}`")),
    }
}

/// A running simulation owned by the viewer's worker.
#[wasm_bindgen]
pub struct WasmSim {
    sim: Sim,
    events: Vec<Event>,
    snap: Snapshot,
}

impl WasmSim {
    fn from_sim(sim: Sim) -> Self {
        let mut s = Self {
            sim,
            events: Vec::new(),
            snap: Snapshot::default(),
        };
        s.sim.snapshot(&mut s.snap);
        s
    }

    fn refresh(&mut self) {
        self.sim.snapshot(&mut self.snap);
    }
}

#[wasm_bindgen]
impl WasmSim {
    /// Build from scenario TOML text.
    ///
    /// # Errors
    /// A JS error carrying the parse or validation message.
    #[wasm_bindgen(constructor)]
    pub fn new(scenario_toml: &str) -> Result<WasmSim, JsError> {
        build(scenario_toml)
            .map(Self::from_sim)
            .map_err(|e| JsError::new(&e))
    }

    /// Build from a built-in preset (`"small"`).
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

    /// Advance `n` ticks. Events are discarded for now; the buffer is reused.
    pub fn step(&mut self, n: u32) {
        for _ in 0..n {
            self.events.clear();
            self.sim.step(&mut self.events);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use switchyard_core::ids::{ChuteId, StationId};

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
        let mut w = WasmSim::from_sim(build(preset_toml("small").unwrap()).unwrap());
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
    fn bad_inputs_are_errors() {
        assert!(preset_toml("huge").is_err());
        assert!(build("not = [valid").is_err());
    }
}
