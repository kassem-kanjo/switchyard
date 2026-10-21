//! The viewer's fault vocabulary, mapped onto core [`FaultSpec`]s.
//!
//! The Faults tab speaks in six friendly kinds with a few numeric knobs
//! (`viewer/src/data/types.ts`: `FaultKindInfo`, `FaultRequest`,
//! `ActiveFault`). This module turns a request into core fault specs and a
//! core spec back into the viewer's kind id and parameters. Ids shown to the
//! user are the core's (`R12`, `C3`, `S1`, 0-based); aisles are numbered from
//! 1 from the west wall, like the `aisle-7-blocked` chaos preset.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use switchyard_core::Tick;
use switchyard_core::faults::{FaultKind, FaultSpec, MAX_SURGE_PCT};
use switchyard_core::grid::{Cell, Pos};
use switchyard_core::ids::{ChuteId, RobotId, StationId};
use switchyard_core::sim::Sim;
use switchyard_core::world::RobotState;

/// Most robots one "Robot failure" request may kill.
const MAX_ROBOTS: u32 = 20;

/// The floor facts the fault parameters depend on.
pub struct Floor {
    robots: u32,
    chutes: u32,
    stations: u32,
    /// `(first_x, last_x)` of each vertical two-cell aisle between chute
    /// columns, west to east.
    aisles: Vec<(u16, u16)>,
    /// Rows an aisle block covers: first chute row down to the row above the
    /// induction stations.
    rows: (u16, u16),
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

impl Floor {
    /// Read the floor off a sim.
    #[must_use]
    pub fn of(sim: &Sim) -> Self {
        let layout = sim.layout();
        let pos: Vec<Pos> = layout.chutes.iter().map(|&c| layout.grid.pos(c)).collect();
        let mut xs: Vec<u16> = pos.iter().map(|p| p.x).collect();
        xs.sort_unstable();
        xs.dedup();
        let aisles = xs
            .windows(2)
            .filter(|w| w[1] > w[0] + 1)
            .map(|w| (w[0] + 1, w[1] - 1))
            .collect();
        let first_row = pos.iter().map(|p| p.y).min().unwrap_or(1);
        let last_row = layout.grid.height().saturating_sub(3).max(first_row);
        Self {
            robots: count(sim.world().robots.len()),
            chutes: count(layout.chutes.len()),
            stations: count(layout.stations.len()),
            aisles,
            rows: (first_row, last_row),
        }
    }

    fn aisle_count(&self) -> u32 {
        count(self.aisles.len())
    }
}

/// One numeric parameter of a fault kind (`FaultParamSpec`).
#[derive(Serialize)]
pub struct ParamSpec {
    key: &'static str,
    label: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    unit: Option<&'static str>,
    min: f64,
    max: f64,
    step: f64,
    default: f64,
    integer: bool,
}

/// One injectable fault kind (`FaultKindInfo`).
#[derive(Serialize)]
pub struct KindInfo {
    id: &'static str,
    label: &'static str,
    description: &'static str,
    params: Vec<ParamSpec>,
}

fn int(key: &'static str, label: &'static str, min: u32, max: u32, default: u32) -> ParamSpec {
    let max = max.max(min);
    ParamSpec {
        key,
        label,
        unit: None,
        min: f64::from(min),
        max: f64::from(max),
        step: 1.0,
        default: f64::from(default.clamp(min, max)),
        integer: true,
    }
}

/// The fault kinds this floor supports, with ranges taken from the floor.
#[must_use]
pub fn kinds(floor: &Floor) -> Vec<KindInfo> {
    let station_max = floor.stations.saturating_sub(1);
    let station = || int("station", "Station", 0, station_max, 1);
    vec![
        KindInfo {
            id: "robot_failure",
            label: "Robot failure",
            description: "Robots stop where they are and block their cells. Robots carrying parcels fail first.",
            params: vec![int("count", "Robots", 1, MAX_ROBOTS.min(floor.robots), 2)],
        },
        KindInfo {
            id: "aisle_block",
            label: "Aisle block",
            description: "One aisle is closed to traffic. Aisles are numbered from the west wall.",
            params: vec![int("aisle", "Aisle", 1, floor.aisle_count(), 7)],
        },
        KindInfo {
            id: "chute_jam",
            label: "Chute jam",
            description: "A chute stops accepting parcels.",
            params: vec![int("chute", "Chute", 0, floor.chutes.saturating_sub(1), 12)],
        },
        KindInfo {
            id: "induction_outage",
            label: "Induction outage",
            description: "An induction station stops loading.",
            params: vec![station()],
        },
        KindInfo {
            id: "volume_surge",
            label: "Volume surge",
            description: "Parcel arrivals jump above the planned rate.",
            params: vec![ParamSpec {
                key: "factor",
                label: "Arrival rate",
                unit: Some("x"),
                min: 1.1,
                max: f64::from(MAX_SURGE_PCT) / 100.0,
                step: 0.1,
                default: 1.5,
                integer: false,
            }],
        },
        KindInfo {
            id: "scanner_drift",
            label: "Scanner drift",
            description: "A station's scanner misreads destinations, so parcels go to the wrong chute.",
            params: vec![
                station(),
                ParamSpec {
                    key: "rate",
                    label: "Misread rate",
                    unit: Some("%"),
                    min: 1.0,
                    max: 50.0,
                    step: 1.0,
                    default: 10.0,
                    integer: false,
                },
            ],
        },
    ]
}

/// A request from the Faults tab (`FaultRequest`).
#[derive(Deserialize)]
pub struct Request {
    kind: String,
    #[serde(default)]
    params: BTreeMap<String, f64>,
    #[serde(rename = "atTick")]
    at_tick: Tick,
    #[serde(rename = "durationTicks")]
    duration_ticks: Tick,
}

impl Request {
    fn param(&self, key: &str) -> Result<f64, String> {
        self.params
            .get(key)
            .copied()
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("missing number `{key}`"))
    }

    fn index(&self, key: &str, min: u32, max: u32) -> Result<u32, String> {
        let v = self.param(key)?;
        if v.fract() != 0.0 || v < f64::from(min) || v > f64::from(max) {
            return Err(format!(
                "`{key}` must be a whole number from {min} to {max}"
            ));
        }
        // Checked above: a whole number inside the u32 range.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(v as u32)
    }
}

/// Core fault specs for a request, validated against the floor but not yet
/// scheduled. `now` is the sim's tick: a request in the past starts now.
///
/// # Errors
/// A message for an unknown kind or an out-of-range parameter.
pub fn to_specs(req: &Request, sim: &Sim, now: Tick) -> Result<Vec<FaultSpec>, String> {
    let floor = Floor::of(sim);
    let at = req.at_tick.max(now);
    let duration = req.duration_ticks;
    let one = |kind| Ok(vec![FaultSpec { at, duration, kind }]);
    let station_of = || {
        req.index("station", 0, floor.stations.saturating_sub(1))
            .map(|s| StationId(u16::try_from(s).unwrap_or(0)))
    };
    match req.kind.as_str() {
        "robot_failure" => {
            let n = req.index("count", 1, MAX_ROBOTS.min(floor.robots))? as usize;
            let robots = pick_robots(sim, n);
            if robots.len() < n {
                return Err(format!("only {} robots are still working", robots.len()));
            }
            Ok(robots
                .into_iter()
                .map(|robot| FaultSpec {
                    at,
                    duration,
                    kind: FaultKind::RobotFailure { robot },
                })
                .collect())
        }
        "aisle_block" => {
            let n = req.index("aisle", 1, floor.aisle_count())? as usize;
            let (x0, x1) = floor.aisles[n - 1];
            let grid = &sim.layout().grid;
            let cells: Vec<Pos> = (floor.rows.0..=floor.rows.1)
                .flat_map(|y| (x0..=x1).map(move |x| Pos::new(x, y)))
                .filter(|&p| grid.at(p) == Cell::Free)
                .collect();
            one(FaultKind::AisleBlock { cells })
        }
        "chute_jam" => {
            let c = req.index("chute", 0, floor.chutes.saturating_sub(1))?;
            one(FaultKind::ChuteJam {
                chute: ChuteId(u16::try_from(c).unwrap_or(0)),
            })
        }
        "induction_outage" => one(FaultKind::InductionOutage {
            station: station_of()?,
        }),
        "volume_surge" => {
            let pct = (req.param("factor")? * 100.0).round();
            if !(101.0..=f64::from(MAX_SURGE_PCT)).contains(&pct) {
                return Err(format!(
                    "`factor` must be from 1.01 to {}",
                    MAX_SURGE_PCT / 100
                ));
            }
            // Range checked just above.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let factor_pct = pct as u32;
            one(FaultKind::VolumeSurge {
                station: None,
                factor_pct,
            })
        }
        "scanner_drift" => {
            let rate = req.param("rate")?;
            if !(0.0..=100.0).contains(&rate) {
                return Err("`rate` must be a percentage from 0 to 100".into());
            }
            // Range checked just above: at most 1,000,000.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let error_rate_ppm = (rate * 10_000.0).round() as u32;
            one(FaultKind::ScannerDrift {
                station: station_of()?,
                error_rate_ppm,
            })
        }
        other => Err(format!("unknown fault kind `{other}`")),
    }
}

/// Robots to fail: working robots carrying a parcel first (they make the
/// best story: the parcel is stuck), then the rest, in id order. Robots that
/// already have a failure scheduled are skipped.
fn pick_robots(sim: &Sim, n: usize) -> Vec<RobotId> {
    let taken: Vec<RobotId> = sim
        .faults()
        .iter()
        .filter_map(|f| match f.kind {
            FaultKind::RobotFailure { robot } => Some(robot),
            _ => None,
        })
        .collect();
    let robots = &sim.world().robots;
    let (mut carrying, mut other) = (Vec::new(), Vec::new());
    for id in robots.ids().filter(|r| !taken.contains(r)) {
        match robots.state[id.index()] {
            RobotState::ToChute | RobotState::Dropping => carrying.push(id),
            RobotState::Failed => {}
            _ => other.push(id),
        }
    }
    carrying.extend(other);
    carrying.truncate(n);
    carrying
}

/// The viewer's kind id for a core fault (`robot-failure` -> `robot_failure`).
#[must_use]
pub fn ui_kind(kind: &FaultKind) -> String {
    kind.name().replace('-', "_")
}

/// A fault in the viewer's shape (`ActiveFault`).
#[derive(Serialize)]
pub struct ActiveOut {
    id: u32,
    kind: String,
    params: BTreeMap<&'static str, f64>,
    #[serde(rename = "atTick")]
    at_tick: Tick,
    #[serde(rename = "durationTicks")]
    duration_ticks: Tick,
    #[serde(rename = "endTick")]
    end_tick: Tick,
}

/// The viewer-side parameters of a core fault (the inverse of [`to_specs`]).
fn params_of(spec: &FaultSpec, floor: &Floor) -> BTreeMap<&'static str, f64> {
    let mut p = BTreeMap::new();
    match &spec.kind {
        FaultKind::RobotFailure { robot } => {
            p.insert("robot", f64::from(robot.0));
        }
        FaultKind::AisleBlock { cells } => {
            let x = cells.iter().map(|c| c.x).min().unwrap_or(0);
            let aisle = floor
                .aisles
                .iter()
                .position(|&(x0, x1)| (x0..=x1).contains(&x));
            p.insert("aisle", aisle.map_or(0.0, |i| f64::from(count(i + 1))));
        }
        FaultKind::ChuteJam { chute } => {
            p.insert("chute", f64::from(chute.0));
        }
        FaultKind::InductionOutage { station } => {
            p.insert("station", f64::from(station.0));
        }
        FaultKind::VolumeSurge { factor_pct, .. } => {
            p.insert("factor", f64::from(*factor_pct) / 100.0);
        }
        FaultKind::ScannerDrift {
            station,
            error_rate_ppm,
        } => {
            p.insert("station", f64::from(station.0));
            p.insert("rate", f64::from(*error_rate_ppm) / 10_000.0);
        }
    }
    p
}

/// The viewer shape of fault `id` of the sim's schedule.
#[must_use]
pub fn active_out(id: u32, spec: &FaultSpec, floor: &Floor) -> ActiveOut {
    ActiveOut {
        id,
        kind: ui_kind(&spec.kind),
        params: params_of(spec, floor),
        at_tick: spec.at,
        duration_ticks: spec.duration,
        end_tick: spec.end(),
    }
}
