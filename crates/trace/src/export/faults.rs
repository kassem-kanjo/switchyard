//! Fault windows for the Perfetto "Faults" process.
//!
//! The core emits `FaultStarted` / `FaultEnded` for each schedule entry; this
//! module pairs them into [`FaultWindow`]s, labelled from the scenario's
//! [`FaultSpec`]s (a [`FaultId`] is an index into that schedule).

use serde_json::{Value, json};
use switchyard_core::Tick;
use switchyard_core::events::Event;
use switchyard_core::faults::{FaultKind, FaultSpec};
use switchyard_core::ids::FaultId;

/// One fault's active interval, ready to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct FaultWindow {
    /// The schedule entry.
    pub fault: FaultId,
    /// Tick of `FaultStarted`.
    pub start: Tick,
    /// Tick of `FaultEnded`, or the end of the observed run if the fault was
    /// still active then.
    pub end: Tick,
    /// Track name, for example `robot-failure R57`.
    pub name: String,
    /// The spec as slice arguments (kind, `at`, `duration`, targets).
    pub args: Value,
}

fn target(kind: &FaultKind) -> String {
    match kind {
        FaultKind::RobotFailure { robot } => robot.to_string(),
        FaultKind::AisleBlock { cells } => format!("{} cells", cells.len()),
        FaultKind::ChuteJam { chute } => chute.to_string(),
        FaultKind::InductionOutage { station } | FaultKind::ScannerDrift { station, .. } => {
            station.to_string()
        }
        FaultKind::VolumeSurge { station, .. } => {
            station.map_or_else(|| "all stations".to_owned(), |s| s.to_string())
        }
    }
}

fn spec_args(spec: &FaultSpec) -> Value {
    let mut v = json!({
        "kind": spec.kind.name(),
        "at": spec.at,
        "duration": spec.duration,
    });
    match &spec.kind {
        FaultKind::RobotFailure { robot } => v["robot"] = json!(robot.to_string()),
        FaultKind::AisleBlock { cells } => {
            v["cells"] = json!(cells.iter().map(|c| [c.x, c.y]).collect::<Vec<_>>());
        }
        FaultKind::ChuteJam { chute } => v["chute"] = json!(chute.to_string()),
        FaultKind::InductionOutage { station } => v["station"] = json!(station.to_string()),
        FaultKind::VolumeSurge {
            station,
            factor_pct,
        } => {
            v["station"] = json!(station.map(|s| s.to_string()));
            v["factor_pct"] = json!(factor_pct);
        }
        FaultKind::ScannerDrift {
            station,
            error_rate_ppm,
        } => {
            v["station"] = json!(station.to_string());
            v["error_rate_ppm"] = json!(error_rate_ppm);
        }
    }
    v
}

/// Pair the fault events of a run into windows, one per fault that started,
/// sorted by `(start, fault)`. `specs` is the scenario's fault schedule (it
/// names and describes each window; an id outside it gets a bare `F<n>`
/// name). `run_end` closes faults still active when observation stopped.
#[must_use]
pub fn fault_windows(specs: &[FaultSpec], events: &[Event], run_end: Tick) -> Vec<FaultWindow> {
    let mut open: Vec<(FaultId, Tick, Option<Tick>)> = Vec::new();
    for e in events {
        match *e {
            Event::FaultStarted { tick, fault } => open.push((fault, tick, None)),
            Event::FaultEnded { tick, fault } => {
                if let Some(w) = open.iter_mut().find(|w| w.0 == fault && w.2.is_none()) {
                    w.2 = Some(tick);
                }
            }
            _ => {}
        }
    }
    let mut out: Vec<FaultWindow> = open
        .into_iter()
        .map(|(fault, start, end)| {
            let spec = specs.get(fault.0 as usize);
            FaultWindow {
                fault,
                start,
                end: end.unwrap_or(run_end).max(start),
                name: spec.map_or_else(
                    || fault.to_string(),
                    |s| format!("{} {}", s.kind.name(), target(&s.kind)),
                ),
                args: spec.map_or_else(|| json!({}), spec_args),
            }
        })
        .collect();
    out.sort_by_key(|w| (w.start, w.fault));
    out
}
