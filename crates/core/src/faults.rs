//! The fault catalog (plan row 2.4; ARCHITECTURE §3, ADR 0005).
//!
//! A fault is an explicit schedule entry, never a random draw (ADR 0002 §3):
//! a [`FaultKind`] that starts at tick `at` and holds for `duration` ticks,
//! i.e. over `[at, at + duration)`. Faults come from a scenario's
//! `[[faults]]` array or from [`crate::sim::Sim::schedule_fault`] at run time.
//! They take effect in phase 1 of the tick ([`crate::sim`]), emit
//! `FaultStarted` / `FaultEnded`, and are part of the state hash.
//!
//! # Healthy counterparts (the CP3 intervention)
//! Counterfactual root-cause analysis replays a run with one cause made
//! healthy, `do(c := healthy)` (ADR 0005). For a fault, the intervention is
//! **removing its [`FaultSpec`] from the schedule**
//! ([`crate::config::ScenarioConfig::without_fault`]); nothing else changes.
//! Each kind's healthy counterpart is therefore:
//!
//! | Fault | While active | Healthy counterpart |
//! |---|---|---|
//! | `robot-failure` | The robot stops where it is (`Failed`) and is an obstacle; its parcel is stuck on it until repair, when it resumes what it was doing | The robot keeps working |
//! | `aisle-block` | No robot may enter the cells; robots inside may leave | The cells stay open |
//! | `chute-jam` | The chute accepts nothing; a bag swap in progress pauses | Only the normal bag-swap cycle |
//! | `induction-outage` | The station dispatches no new robots; robots already on their way still load (the queue drains no further); parcels keep arriving | The station keeps dispatching |
//! | `volume-surge` | Extra arrivals at `factor_pct / 100` times the station's rate, from the surge's own random streams | The base arrival process only |
//! | `scanner-drift` | The station's scanner misreads at `error_rate_ppm` | The scenario's base misread rate |
//!
//! Removing a fault never shifts unrelated random draws: the base arrival,
//! destination and scanner draws are keyed per station and per parcel, a
//! surge draws its arrival counts from its own streams (keyed on the fault's
//! content, not its position in the schedule) and gives its parcels keys in
//! a range of their own, so the regular parcels of a surged run are
//! bit-identical to those of the run without the surge.
//!
//! # Topology
//! A robot failure or an aisle block changes which cells robots can drive
//! through. The sim then rebuilds its guided distance tables with those
//! cells closed (no edge may enter them; robots route around), and every
//! robot whose goal has become unreachable records `CellBlocked(fault)` and
//! waits, instead of wandering.

use serde::{Deserialize, Serialize};

use crate::Tick;
use crate::config::ConfigError;
use crate::grid::{Cell, Pos};
use crate::hash::Fnv1a;
use crate::ids::{ChuteId, RobotId, StationId};
use crate::layout::Layout;

/// Largest volume-surge factor, in percent (ten times the base rate).
pub const MAX_SURGE_PCT: u32 = 1_000;

/// One scheduled fault.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "FaultToml", into = "FaultToml")]
pub struct FaultSpec {
    /// First tick on which the fault is active.
    pub at: Tick,
    /// Ticks it stays active (at least 1).
    pub duration: Tick,
    /// What breaks.
    pub kind: FaultKind,
}

/// What a fault breaks. See the module docs for each kind's effect and its
/// healthy counterpart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FaultKind {
    /// A robot breaks down where it stands.
    RobotFailure {
        /// The robot.
        robot: RobotId,
    },
    /// Cells are closed to traffic (an aisle blocked by a spill or a person).
    AisleBlock {
        /// The closed cells (free floor only).
        cells: Vec<Pos>,
    },
    /// A chute stops accepting parcels.
    ChuteJam {
        /// The chute.
        chute: ChuteId,
    },
    /// An induction station stops dispatching work.
    InductionOutage {
        /// The station.
        station: StationId,
    },
    /// Extra parcels arrive at one station, or at all of them.
    VolumeSurge {
        /// The station, or `None` for every station.
        station: Option<StationId>,
        /// Total arrival rate during the surge as a percentage of the base
        /// rate (150 = 1.5x), 101 to [`MAX_SURGE_PCT`].
        factor_pct: u32,
    },
    /// A station's scanner misreads more often.
    ScannerDrift {
        /// The station.
        station: StationId,
        /// Misread probability while the fault holds, parts per million.
        error_rate_ppm: u32,
    },
}

impl FaultKind {
    /// The kind's name in scenario files and on the command line.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::RobotFailure { .. } => "robot-failure",
            Self::AisleBlock { .. } => "aisle-block",
            Self::ChuteJam { .. } => "chute-jam",
            Self::InductionOutage { .. } => "induction-outage",
            Self::VolumeSurge { .. } => "volume-surge",
            Self::ScannerDrift { .. } => "scanner-drift",
        }
    }

    /// True for kinds that close cells to traffic (the sim rebuilds its
    /// distance tables when one starts or ends).
    #[must_use]
    pub const fn changes_topology(&self) -> bool {
        matches!(self, Self::RobotFailure { .. } | Self::AisleBlock { .. })
    }
}

impl FaultSpec {
    /// The first tick on which the fault is no longer active.
    #[must_use]
    pub const fn end(&self) -> Tick {
        self.at.saturating_add(self.duration)
    }

    /// Is the fault active on tick `t`?
    #[must_use]
    pub const fn is_active_at(&self, t: Tick) -> bool {
        self.at <= t && t < self.end()
    }

    /// Feed a canonical encoding of the spec into `h` (state hash, and the
    /// surge stream keys).
    pub fn hash_into(&self, h: &mut Fnv1a) {
        h.write_u32(self.at);
        h.write_u32(self.duration);
        match &self.kind {
            FaultKind::RobotFailure { robot } => {
                h.write_u8(0);
                h.write_u32(robot.0);
            }
            FaultKind::AisleBlock { cells } => {
                h.write_u8(1);
                h.write_u64(cells.len() as u64);
                for c in cells {
                    h.write_u16(c.x);
                    h.write_u16(c.y);
                }
            }
            FaultKind::ChuteJam { chute } => {
                h.write_u8(2);
                h.write_u16(chute.0);
            }
            FaultKind::InductionOutage { station } => {
                h.write_u8(3);
                h.write_u16(station.0);
            }
            FaultKind::VolumeSurge {
                station,
                factor_pct,
            } => {
                h.write_u8(4);
                h.write_opt_u32(station.map(|s| u32::from(s.0)));
                h.write_u32(*factor_pct);
            }
            FaultKind::ScannerDrift {
                station,
                error_rate_ppm,
            } => {
                h.write_u8(5);
                h.write_u16(station.0);
                h.write_u32(*error_rate_ppm);
            }
        }
    }

    /// A 32-bit key derived from the spec's content: the same fault gets the
    /// same random streams wherever it sits in the schedule, so removing
    /// another fault never changes its draws.
    #[must_use]
    pub fn content_key(&self) -> u32 {
        let mut h = Fnv1a::new();
        h.write(b"fault");
        self.hash_into(&mut h);
        let x = h.finish();
        // Fold the 64-bit hash to 32 bits.
        #[allow(clippy::cast_possible_truncation)]
        let folded = (x ^ (x >> 32)) as u32;
        folded
    }

    /// Checks that need no layout (called from
    /// [`crate::config::ScenarioConfig::validate`]).
    ///
    /// # Errors
    /// [`ConfigError::BadFault`] or [`ConfigError::ZeroDuration`].
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.duration == 0 {
            return Err(ConfigError::ZeroDuration("faults.duration"));
        }
        if self.at.checked_add(self.duration).is_none() {
            return Err(ConfigError::BadFault(format!(
                "{} at {} for {} ticks overflows the tick counter",
                self.kind.name(),
                self.at,
                self.duration
            )));
        }
        match &self.kind {
            FaultKind::AisleBlock { cells } if cells.is_empty() => Err(ConfigError::BadFault(
                "aisle-block needs at least one cell".into(),
            )),
            FaultKind::VolumeSurge { factor_pct, .. }
                if !(101..=MAX_SURGE_PCT).contains(factor_pct) =>
            {
                Err(ConfigError::BadFault(format!(
                    "volume-surge factor_pct = {factor_pct} is outside 101..={MAX_SURGE_PCT}"
                )))
            }
            FaultKind::ScannerDrift { error_rate_ppm, .. } if *error_rate_ppm > 1_000_000 => {
                Err(ConfigError::BadFault(format!(
                    "scanner-drift error_rate_ppm = {error_rate_ppm} is above 1000000"
                )))
            }
            _ => Ok(()),
        }
    }

    /// Checks against the generated floor: the robot, chute, station and
    /// cells must exist, and blocked cells must be open floor.
    ///
    /// # Errors
    /// [`ConfigError::BadFault`] naming the problem, or any
    /// [`FaultSpec::validate`] error.
    pub fn validate_for(&self, layout: &Layout, robots: usize) -> Result<(), ConfigError> {
        self.validate()?;
        let name = self.kind.name();
        let bad = |what: String| Err(ConfigError::BadFault(format!("{name}: {what}")));
        let station_ok = |s: StationId| s.index() < layout.stations.len();
        match &self.kind {
            FaultKind::RobotFailure { robot } if robot.index() >= robots => {
                bad(format!("{robot} does not exist ({robots} robots)"))
            }
            FaultKind::ChuteJam { chute } if chute.index() >= layout.chutes.len() => bad(format!(
                "{chute} does not exist ({} chutes)",
                layout.chutes.len()
            )),
            FaultKind::InductionOutage { station }
            | FaultKind::ScannerDrift { station, .. }
            | FaultKind::VolumeSurge {
                station: Some(station),
                ..
            } if !station_ok(*station) => bad(format!(
                "{station} does not exist ({} stations)",
                layout.stations.len()
            )),
            FaultKind::AisleBlock { cells } => {
                let g = &layout.grid;
                for &c in cells {
                    if c.x >= g.width() || c.y >= g.height() || g.at(c) != Cell::Free {
                        return bad(format!("cell ({}, {}) is not open floor", c.x, c.y));
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// The scenario-file shape of a fault: one flat table with a `kind` tag.
/// A separate type (rather than `#[serde(flatten)]`) keeps
/// `deny_unknown_fields`, so a misspelt key is an error, not a silently
/// different fault.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FaultToml {
    kind: String,
    at: Tick,
    duration: Tick,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    robot: Option<RobotId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cells: Option<Vec<[u16; 2]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chute: Option<ChuteId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    station: Option<StationId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    factor_pct: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_rate_ppm: Option<u32>,
}

impl TryFrom<FaultToml> for FaultSpec {
    type Error = String;

    fn try_from(t: FaultToml) -> Result<Self, String> {
        let mut used = Vec::new();
        let mut need = |field: &'static str, present: bool| {
            used.push(field);
            if present {
                Ok(())
            } else {
                Err(format!("{} needs `{field}`", t.kind))
            }
        };
        let kind = match t.kind.as_str() {
            "robot-failure" => {
                need("robot", t.robot.is_some())?;
                FaultKind::RobotFailure {
                    robot: t.robot.unwrap_or(RobotId(0)),
                }
            }
            "aisle-block" => {
                need("cells", t.cells.is_some())?;
                FaultKind::AisleBlock {
                    cells: t
                        .cells
                        .iter()
                        .flatten()
                        .map(|&[x, y]| Pos::new(x, y))
                        .collect(),
                }
            }
            "chute-jam" => {
                need("chute", t.chute.is_some())?;
                FaultKind::ChuteJam {
                    chute: t.chute.unwrap_or(ChuteId(0)),
                }
            }
            "induction-outage" => {
                need("station", t.station.is_some())?;
                FaultKind::InductionOutage {
                    station: t.station.unwrap_or(StationId(0)),
                }
            }
            "volume-surge" => {
                // `station` is optional here: omitted means every station.
                need("station", true)?;
                need("factor_pct", t.factor_pct.is_some())?;
                FaultKind::VolumeSurge {
                    station: t.station,
                    factor_pct: t.factor_pct.unwrap_or(0),
                }
            }
            "scanner-drift" => {
                need("station", t.station.is_some())?;
                need("error_rate_ppm", t.error_rate_ppm.is_some())?;
                FaultKind::ScannerDrift {
                    station: t.station.unwrap_or(StationId(0)),
                    error_rate_ppm: t.error_rate_ppm.unwrap_or(0),
                }
            }
            other => {
                return Err(format!(
                    "unknown fault kind `{other}` (robot-failure, aisle-block, chute-jam, \
                     induction-outage, volume-surge, scanner-drift)"
                ));
            }
        };
        let present = [
            ("robot", t.robot.is_some()),
            ("cells", t.cells.is_some()),
            ("chute", t.chute.is_some()),
            ("station", t.station.is_some()),
            ("factor_pct", t.factor_pct.is_some()),
            ("error_rate_ppm", t.error_rate_ppm.is_some()),
        ];
        if let Some((field, _)) = present.iter().find(|(f, p)| *p && !used.contains(f)) {
            return Err(format!("{} does not take `{field}`", t.kind));
        }
        Ok(Self {
            at: t.at,
            duration: t.duration,
            kind,
        })
    }
}

impl From<FaultSpec> for FaultToml {
    fn from(f: FaultSpec) -> Self {
        let mut t = Self {
            kind: f.kind.name().to_owned(),
            at: f.at,
            duration: f.duration,
            ..Self::default()
        };
        match f.kind {
            FaultKind::RobotFailure { robot } => t.robot = Some(robot),
            FaultKind::AisleBlock { cells } => {
                t.cells = Some(cells.iter().map(|c| [c.x, c.y]).collect());
            }
            FaultKind::ChuteJam { chute } => t.chute = Some(chute),
            FaultKind::InductionOutage { station } => t.station = Some(station),
            FaultKind::VolumeSurge {
                station,
                factor_pct,
            } => {
                t.station = station;
                t.factor_pct = Some(factor_pct);
            }
            FaultKind::ScannerDrift {
                station,
                error_rate_ppm,
            } => {
                t.station = Some(station);
                t.error_rate_ppm = Some(error_rate_ppm);
            }
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize, Serialize)]
    struct Doc {
        faults: Vec<FaultSpec>,
    }

    fn parse(text: &str) -> Result<Vec<FaultSpec>, String> {
        toml::from_str::<Doc>(text)
            .map(|d| d.faults)
            .map_err(|e| e.to_string())
    }

    #[test]
    fn every_kind_parses_and_round_trips() {
        let text = r#"
            [[faults]]
            kind = "robot-failure"
            robot = 31
            at = 3600
            duration = 1800

            [[faults]]
            kind = "aisle-block"
            cells = [[10, 5], [10, 6]]
            at = 1
            duration = 2

            [[faults]]
            kind = "chute-jam"
            chute = 4
            at = 1
            duration = 2

            [[faults]]
            kind = "induction-outage"
            station = 3
            at = 1
            duration = 2

            [[faults]]
            kind = "volume-surge"
            factor_pct = 200
            at = 1
            duration = 2

            [[faults]]
            kind = "scanner-drift"
            station = 1
            error_rate_ppm = 50000
            at = 1
            duration = 2
        "#;
        let faults = parse(text).unwrap();
        assert_eq!(faults.len(), 6);
        assert_eq!(
            faults[0],
            FaultSpec {
                at: 3600,
                duration: 1800,
                kind: FaultKind::RobotFailure { robot: RobotId(31) },
            }
        );
        assert_eq!(
            faults[1].kind,
            FaultKind::AisleBlock {
                cells: vec![Pos::new(10, 5), Pos::new(10, 6)]
            }
        );
        assert_eq!(
            faults[4].kind,
            FaultKind::VolumeSurge {
                station: None,
                factor_pct: 200
            }
        );
        let again = toml::to_string(&Doc {
            faults: faults.clone(),
        })
        .unwrap();
        assert_eq!(parse(&again).unwrap(), faults);
    }

    #[test]
    fn misspelt_missing_and_foreign_fields_are_errors() {
        let base = "[[faults]]\nkind = \"robot-failure\"\nat = 1\nduration = 2\n";
        assert!(parse(&format!("{base}robot = 1\nduraton = 3\n")).is_err());
        let missing = parse(base).unwrap_err();
        assert!(missing.contains("needs `robot`"), "{missing}");
        let foreign = parse(&format!("{base}robot = 1\nchute = 3\n")).unwrap_err();
        assert!(foreign.contains("does not take `chute`"), "{foreign}");
        let unknown = parse("[[faults]]\nkind = \"meteor\"\nat = 1\nduration = 2\n");
        assert!(unknown.unwrap_err().contains("unknown fault kind"));
    }

    #[test]
    fn validation_rejects_empty_windows_and_bad_factors() {
        let mut f = FaultSpec {
            at: 10,
            duration: 0,
            kind: FaultKind::ChuteJam { chute: ChuteId(0) },
        };
        assert_eq!(
            f.validate(),
            Err(ConfigError::ZeroDuration("faults.duration"))
        );
        f.duration = 5;
        assert_eq!(f.validate(), Ok(()));
        assert!((f.is_active_at(10) && f.is_active_at(14)) && !f.is_active_at(15));
        f.kind = FaultKind::VolumeSurge {
            station: None,
            factor_pct: 100,
        };
        assert!(matches!(f.validate(), Err(ConfigError::BadFault(_))));
    }

    #[test]
    fn content_keys_ignore_schedule_position_but_not_content() {
        let a = FaultSpec {
            at: 100,
            duration: 50,
            kind: FaultKind::VolumeSurge {
                station: Some(StationId(1)),
                factor_pct: 200,
            },
        };
        let mut b = a.clone();
        assert_eq!(a.content_key(), b.content_key());
        b.at = 101;
        assert_ne!(a.content_key(), b.content_key());
    }
}
