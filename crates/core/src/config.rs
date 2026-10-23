//! Scenario configuration, loaded from TOML.
//!
//! Every field is an integer so that a scenario file maps to sim state
//! without any float rounding (ADR 0002 §1). Rates are expressed per hour and
//! converted to per-tick sampling tables with integer arithmetic
//! ([`crate::arrivals`]).
//!
//! Sections added after V0 (`chutes`, `scanner`, `guidance`, `planner`, and
//! the new `arrivals` fields) have documented defaults, so older scenario
//! files keep loading.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::TICKS_PER_HOUR;
use crate::Tick;
use crate::faults::FaultSpec;
use crate::ids::{ChuteId, DestinationId, FaultId, StationId};

/// A complete, self-contained description of one simulation scenario.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioConfig {
    /// Human-readable scenario name (for reports and the viewer).
    pub name: String,
    /// Master seed. Every random stream is derived from `(seed, stream_id)`.
    pub seed: u64,
    /// Faults of the schedule that are switched off: the counterfactual
    /// intervention `do(fault := healthy)` ([`ScenarioConfig::without_fault`]).
    /// A disabled fault keeps its place, so every [`FaultId`] (the index in
    /// `faults`) means the same fault in the original run and in every
    /// replay. Ids beyond the schedule are ignored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_faults: Vec<FaultId>,
    /// Floor geometry.
    pub floor: FloorConfig,
    /// Robot fleet.
    pub fleet: FleetConfig,
    /// Parcel arrival process.
    pub arrivals: ArrivalConfig,
    /// Durations of fixed operations.
    pub timing: TimingConfig,
    /// Chute bags and the destination-to-chute mapping.
    #[serde(default)]
    pub chutes: ChuteConfig,
    /// The induction scanner.
    #[serde(default)]
    pub scanner: ScannerConfig,
    /// Lane guidance for the planner (ADR 0003).
    #[serde(default)]
    pub guidance: GuidanceConfig,
    /// Planner safety nets.
    #[serde(default)]
    pub planner: PlannerConfig,
    /// How the floor responds to faults (`[recovery]`).
    #[serde(default)]
    pub recovery: RecoveryConfig,
    /// The fault schedule (`[[faults]]`), empty for a healthy run. Faults are
    /// opt-in: a scenario without them simulates exactly as before. See
    /// [`crate::faults`] for the kinds and their healthy counterparts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub faults: Vec<FaultSpec>,
}

/// Floor geometry, consumed by [`crate::layout::generate`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FloorConfig {
    /// Outer width in cells, walls included.
    pub width: u16,
    /// Outer height in cells, walls included.
    pub height: u16,
    /// Number of induction stations along the south edge.
    pub stations: u16,
    /// Free rows between the station row and the lowest chute row.
    pub south_margin: u16,
}

/// Robot fleet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetConfig {
    /// Number of robots.
    pub robots: u32,
    /// Robots that may be heading to (or loading at) one station at once.
    ///
    /// A station's throughput is at most `inbound / (trip + load)` parcels
    /// per tick, and a trip to the station is 30-60 ticks on the medium and
    /// large floors, so keeping a station busy at 800 parcels/h needs about
    /// five robots in its pipeline (Little's law). The default, 6, keeps every
    /// preset level with demand while capping the crowd around the induction
    /// cell (measured at CP1: 2 left medium at 20 parcels/h per robot with a
    /// growing backlog, 4 left a backlog on `large`, 8 gained nothing).
    #[serde(default = "default_max_inbound")]
    pub max_inbound_per_station: u16,
    /// Per-station overrides of `max_inbound_per_station`
    /// (`[[fleet.station_caps]]`; an intervention, see
    /// [`crate::interventions`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub station_caps: Vec<StationCap>,
    /// Robots added to the fleet during the run (`[[fleet.additions]]`; an
    /// intervention). They appear at the start of tick `at`, idle, on the
    /// first free parking cells (then the first free floor cells), in
    /// row-major order; if the floor is full, fewer are added.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additions: Vec<RobotAddition>,
}

/// A per-station inbound cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StationCap {
    /// The station.
    pub station: StationId,
    /// Robots that may head to (or load at) it at once (at least 1).
    pub cap: u16,
}

/// Robots joining the fleet at a tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RobotAddition {
    /// The tick at whose start they appear.
    pub at: Tick,
    /// How many (at least 1).
    pub robots: u32,
}

/// A destination sent to a chute other than the mapping's choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DestinationRemap {
    /// The destination.
    pub destination: DestinationId,
    /// The chute that serves it instead.
    pub chute: ChuteId,
}

const fn default_max_inbound() -> u16 {
    6
}

/// Parcel arrival process.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArrivalConfig {
    /// Mean arrivals per hour at each station: a Poisson process per station
    /// (see [`crate::arrivals`]). At most [`TICKS_PER_HOUR`] (a mean of one
    /// per tick), far above any real induction rate.
    pub parcels_per_hour_per_station: u32,
    /// Number of sort destinations (postcode groups). Omitted: one per chute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destinations: Option<u16>,
    /// Zipf exponent `s` of destination popularity, times 1,000: destination
    /// `d` is drawn with weight `(d + 1)^-s`, so destination 0 is the most
    /// popular. 0 (the default) is uniform; hub volumes are skewed towards a
    /// few big destinations, which `s` near 1 models.
    #[serde(default)]
    pub zipf_s_milli: u32,
}

/// Durations of fixed operations, in ticks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimingConfig {
    /// Ticks a robot spends on the induction cell while a parcel is loaded.
    pub load_ticks: u16,
    /// Ticks a robot spends next to the chute while it drops the parcel.
    pub drop_ticks: u16,
}

/// Chute bags and the destination-to-chute mapping.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ChuteConfig {
    /// Parcels a bag holds before it must be swapped.
    pub bag_capacity: u32,
    /// Ticks a bag swap takes. The chute accepts nothing meanwhile, and
    /// robots waiting at it record `ChuteUnavailable`.
    pub bag_swap_ticks: u32,
    /// How destinations are assigned to chutes.
    pub mapping: ChuteMapping,
    /// Overrides applied after the mapping (`[[chutes.remap]]`; an
    /// intervention, see [`crate::interventions`]). Later entries win.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub remap: Vec<DestinationRemap>,
}

impl Default for ChuteConfig {
    fn default() -> Self {
        // A 50-parcel bag swapped in 30 s, typical of manual bag changes at
        // robotic sort walls (vendor-claimed, range-calibrated, not measured).
        Self {
            bag_capacity: 50,
            bag_swap_ticks: 60,
            mapping: ChuteMapping::RoundRobin,
            remap: Vec::new(),
        }
    }
}

/// Destination-to-chute mapping strategy. The mapping sets the floor's
/// traffic pattern; a good one is worth 8-29 % throughput (Zhang et al.
/// 2025, `docs/research/2026-10-19-robot-coordination.md` §2.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChuteMapping {
    /// Destination `d` goes to chute `d mod chutes`.
    #[default]
    RoundRobin,
    /// The most popular destinations go to the chutes nearest the stations
    /// (by guided distance summed over all stations), so the bulk of the
    /// volume makes short trips.
    PopularityNearInduction,
}

/// The induction scanner.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ScannerConfig {
    /// Probability of a misread, in parts per million. A misread parcel is
    /// sent to a wrong chute and reported as `ParcelMisSorted` when dropped.
    /// Real baselines are 100-1,000 ppm (0.01-0.1 %); the default is 0 so
    /// that mis-sorts appear only when a scenario asks for them.
    pub error_rate_ppm: u32,
}

/// How operators respond to faults (`[recovery]`). Only matters when the
/// scenario has faults.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct RecoveryConfig {
    /// A robot that fails on an induction cell blocks its station. After
    /// this many ticks (default 600 = 5 minutes) operators push it to the
    /// nearest free cell; it stays failed (and keeps its parcel) until its
    /// fault ends. `u32::MAX` never clears it.
    pub clear_after_ticks: u32,
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        Self {
            clear_after_ticks: 600,
        }
    }
}

/// Lane guidance (ADR 0003).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct GuidanceConfig {
    /// Cost of moving one cell against a lane's preferred direction (moving
    /// with it costs 1). 1 disables guidance.
    pub against_flow_cost: u32,
}

impl Default for GuidanceConfig {
    fn default() -> Self {
        Self {
            against_flow_cost: 3,
        }
    }
}

/// Planner safety nets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PlannerConfig {
    /// If robots have work but none has made progress for this many ticks,
    /// the sim emits `Event::Stalled` (a deadlock or livelock) instead of
    /// silently going quiet (ADR 0003, robustness).
    pub watchdog_ticks: u32,
    /// A wait-for cycle must persist this many consecutive ticks before the
    /// sim reports it as a deadlock (`Event::DeadlockDetected`). PIBT clears
    /// most head-on meetings within a tick or two, so a short persistence
    /// requirement filters out those transient 2-cycles.
    pub deadlock_ticks: u32,
}

impl Default for PlannerConfig {
    fn default() -> Self {
        Self {
            // 5 simulated minutes: longer than any legitimate wait (a bag swap).
            watchdog_ticks: 600,
            // 5 simulated seconds.
            deadlock_ticks: 10,
        }
    }
}

/// Why a scenario could not be loaded or turned into a simulation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// The TOML text did not parse or did not match the schema.
    Parse(String),
    /// The floor is too small to hold walls, a chute lattice and stations.
    FloorTooSmall {
        /// Requested width.
        width: u16,
        /// Requested height.
        height: u16,
    },
    /// More stations than fit along the south edge, or none at all.
    BadStationCount {
        /// Requested stations.
        stations: u16,
        /// Maximum that fits on this floor.
        max: u16,
    },
    /// The chute lattice came out empty.
    NoChutes,
    /// More robots than free parking cells, or none at all.
    BadRobotCount {
        /// Requested robots.
        robots: u32,
        /// Maximum that fits on this floor.
        max: u32,
    },
    /// The arrival rate exceeds a mean of one parcel per station per tick.
    ArrivalRateTooHigh {
        /// Requested rate.
        per_hour: u32,
    },
    /// A duration is zero (every operation takes at least one tick).
    ZeroDuration(&'static str),
    /// A value is outside its allowed range.
    OutOfRange {
        /// The offending field, as `section.field`.
        field: &'static str,
        /// Its value.
        value: u64,
        /// Smallest allowed value.
        min: u64,
        /// Largest allowed value.
        max: u64,
    },
    /// The generated layout broke an invariant (a station or chute is
    /// unreachable). This is a generator bug or a degenerate floor.
    Unreachable(String),
    /// A fault in the schedule is invalid (unknown robot, chute or station,
    /// a blocked cell that is not open floor, a factor out of range, or a
    /// start tick in the past).
    BadFault(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(msg) => write!(f, "invalid scenario TOML: {msg}"),
            Self::FloorTooSmall { width, height } => {
                write!(f, "floor {width}x{height} is too small (minimum 10x10)")
            }
            Self::BadStationCount { stations, max } => {
                write!(
                    f,
                    "{stations} stations requested; 1..={max} fit on this floor"
                )
            }
            Self::NoChutes => write!(f, "the floor has no room for chutes"),
            Self::BadRobotCount { robots, max } => {
                write!(f, "{robots} robots requested; 1..={max} fit on this floor")
            }
            Self::ArrivalRateTooHigh { per_hour } => write!(
                f,
                "{per_hour} parcels/h per station exceeds one per tick ({TICKS_PER_HOUR}/h)"
            ),
            Self::ZeroDuration(field) => write!(f, "{field} must be at least 1 tick"),
            Self::OutOfRange {
                field,
                value,
                min,
                max,
            } => write!(f, "{field} = {value} is outside {min}..={max}"),
            Self::Unreachable(msg) => write!(f, "layout is not fully connected: {msg}"),
            Self::BadFault(msg) => write!(f, "invalid fault: {msg}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl ScenarioConfig {
    /// Parse a scenario from TOML text. Parsing is pure: the caller does the
    /// file I/O, keeping the core free of it (ADR 0002).
    ///
    /// # Errors
    /// [`ConfigError::Parse`] if the text is not valid TOML for this schema,
    /// or any [`ScenarioConfig::validate`] error.
    pub fn from_toml_str(text: &str) -> Result<Self, ConfigError> {
        let cfg: Self = toml::from_str(text).map_err(|e| ConfigError::Parse(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Serialize back to TOML (useful for recording the exact scenario of a
    /// run next to its state hash).
    ///
    /// # Errors
    /// [`ConfigError::Parse`] if serialization fails (it should not).
    pub fn to_toml_string(&self) -> Result<String, ConfigError> {
        toml::to_string(self).map_err(|e| ConfigError::Parse(e.to_string()))
    }

    /// Check the values that can be checked without generating the layout.
    /// Layout-dependent limits (stations, robots) are checked by
    /// [`crate::layout::generate`] and [`crate::sim::Sim::new`].
    ///
    /// # Errors
    /// The first invalid value found.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.floor.width < 10 || self.floor.height < 10 {
            return Err(ConfigError::FloorTooSmall {
                width: self.floor.width,
                height: self.floor.height,
            });
        }
        if self.arrivals.parcels_per_hour_per_station > TICKS_PER_HOUR {
            return Err(ConfigError::ArrivalRateTooHigh {
                per_hour: self.arrivals.parcels_per_hour_per_station,
            });
        }
        if self.chutes.bag_swap_ticks >= self.planner.watchdog_ticks {
            // A robot waiting out a bag swap is not stuck: a swap as long as
            // the watchdog window would raise false `RobotStalled` alarms.
            return Err(ConfigError::OutOfRange {
                field: "chutes.bag_swap_ticks",
                value: u64::from(self.chutes.bag_swap_ticks),
                min: 1,
                max: u64::from(self.planner.watchdog_ticks.saturating_sub(1)),
            });
        }
        let durations = [
            ("timing.load_ticks", u32::from(self.timing.load_ticks)),
            ("timing.drop_ticks", u32::from(self.timing.drop_ticks)),
            ("chutes.bag_swap_ticks", self.chutes.bag_swap_ticks),
            ("planner.watchdog_ticks", self.planner.watchdog_ticks),
            ("planner.deadlock_ticks", self.planner.deadlock_ticks),
        ];
        if let Some((field, _)) = durations.iter().find(|(_, v)| *v == 0) {
            return Err(ConfigError::ZeroDuration(field));
        }
        let ranges: [(&'static str, u64, u64, u64); 6] = [
            (
                "fleet.max_inbound_per_station",
                u64::from(self.fleet.max_inbound_per_station),
                1,
                u64::from(u16::MAX),
            ),
            (
                "arrivals.destinations",
                self.arrivals.destinations.map_or(1, u64::from),
                1,
                u64::from(u16::MAX),
            ),
            (
                "arrivals.zipf_s_milli",
                u64::from(self.arrivals.zipf_s_milli),
                0,
                5_000,
            ),
            (
                "chutes.bag_capacity",
                u64::from(self.chutes.bag_capacity),
                1,
                1_000_000,
            ),
            (
                "scanner.error_rate_ppm",
                u64::from(self.scanner.error_rate_ppm),
                0,
                1_000_000,
            ),
            (
                "guidance.against_flow_cost",
                u64::from(self.guidance.against_flow_cost),
                1,
                100,
            ),
        ];
        for (field, value, min, max) in ranges {
            if !(min..=max).contains(&value) {
                return Err(ConfigError::OutOfRange {
                    field,
                    value,
                    min,
                    max,
                });
            }
        }
        for fault in &self.faults {
            fault.validate()?;
        }
        if let Some(c) = self.fleet.station_caps.iter().find(|c| c.cap == 0) {
            return Err(ConfigError::OutOfRange {
                field: "fleet.station_caps.cap",
                value: u64::from(c.cap),
                min: 1,
                max: u64::from(u16::MAX),
            });
        }
        if self.fleet.additions.iter().any(|a| a.robots == 0) {
            return Err(ConfigError::OutOfRange {
                field: "fleet.additions.robots",
                value: 0,
                min: 1,
                max: u64::from(u32::MAX),
            });
        }
        Ok(())
    }

    /// The same scenario with fault `id` switched off: the counterfactual
    /// intervention `do(fault := healthy)` (ADR 0005; see [`crate::faults`]
    /// for each kind's healthy counterpart). The fault is disabled in place
    /// ([`ScenarioConfig::disabled_faults`]), so the other faults keep their
    /// ids and their random streams, and events and blame join across the
    /// original run and the replay by `FaultId`.
    #[must_use]
    pub fn without_fault(&self, id: FaultId) -> Self {
        let mut cfg = self.clone();
        if !cfg.disabled_faults.contains(&id) {
            cfg.disabled_faults.push(id);
            cfg.disabled_faults.sort_unstable();
        }
        cfg
    }

    /// Is fault `id` in the schedule and switched on?
    #[must_use]
    pub fn fault_enabled(&self, id: FaultId) -> bool {
        id.index() < self.faults.len() && !self.disabled_faults.contains(&id)
    }

    /// The faults that will run, with their stable ids.
    pub fn enabled_faults(&self) -> impl Iterator<Item = (FaultId, &FaultSpec)> {
        self.faults.iter().enumerate().filter_map(|(i, f)| {
            // Schedules are far shorter than u32::MAX entries.
            #[allow(clippy::cast_possible_truncation)]
            let id = FaultId(i as u32);
            self.fault_enabled(id).then_some((id, f))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::{ALL, SMALL_TOML};

    #[test]
    fn every_preset_parses_and_round_trips() {
        for (name, text) in ALL {
            let cfg = ScenarioConfig::from_toml_str(text).unwrap();
            assert_eq!(cfg.name, *name);
            let again = ScenarioConfig::from_toml_str(&cfg.to_toml_string().unwrap()).unwrap();
            assert_eq!(cfg, again);
        }
    }

    #[test]
    fn rejects_unknown_fields_and_floats() {
        let bad = SMALL_TOML.replace("robots = 40", "robots = 40\nspeed = 1");
        assert!(matches!(
            ScenarioConfig::from_toml_str(&bad),
            Err(ConfigError::Parse(_))
        ));
        let float = SMALL_TOML.replace("load_ticks = 6", "load_ticks = 6.5");
        assert!(matches!(
            ScenarioConfig::from_toml_str(&float),
            Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn rejects_out_of_range_values() {
        let rate = SMALL_TOML.replace(
            "parcels_per_hour_per_station = 400",
            "parcels_per_hour_per_station = 9000",
        );
        assert!(matches!(
            ScenarioConfig::from_toml_str(&rate),
            Err(ConfigError::ArrivalRateTooHigh { .. })
        ));
        let zero = SMALL_TOML.replace("drop_ticks = 2", "drop_ticks = 0");
        assert_eq!(
            ScenarioConfig::from_toml_str(&zero),
            Err(ConfigError::ZeroDuration("timing.drop_ticks"))
        );
        let cost = SMALL_TOML.replace("against_flow_cost = 3", "against_flow_cost = 0");
        assert!(matches!(
            ScenarioConfig::from_toml_str(&cost),
            Err(ConfigError::OutOfRange {
                field: "guidance.against_flow_cost",
                ..
            })
        ));
    }

    #[test]
    fn new_sections_default_when_omitted() {
        // A V0-era scenario (no chutes/scanner/guidance/planner sections and
        // no destination mix) still loads, with the documented defaults.
        let v0 = "name = \"v0\"\nseed = 1\n\
                  [floor]\nwidth = 20\nheight = 20\nstations = 2\nsouth_margin = 3\n\
                  [fleet]\nrobots = 4\n\
                  [arrivals]\nparcels_per_hour_per_station = 100\n\
                  [timing]\nload_ticks = 2\ndrop_ticks = 1\n";
        let cfg = ScenarioConfig::from_toml_str(v0).unwrap();
        assert_eq!(cfg.chutes, ChuteConfig::default());
        assert_eq!(cfg.guidance, GuidanceConfig::default());
        assert_eq!(cfg.planner, PlannerConfig::default());
        assert_eq!(cfg.scanner.error_rate_ppm, 0);
        assert_eq!(cfg.arrivals.destinations, None);
        assert_eq!(cfg.arrivals.zipf_s_milli, 0);
        assert_eq!(cfg.fleet.max_inbound_per_station, 6);
    }

    #[test]
    fn rejects_bag_swaps_the_watchdog_would_call_a_stall() {
        // A robot waiting out a bag swap is not stuck; a swap as long as the
        // watchdog window would raise false RobotStalled alarms.
        let mut cfg = ScenarioConfig::from_toml_str(SMALL_TOML).unwrap();
        cfg.chutes.bag_swap_ticks = cfg.planner.watchdog_ticks;
        assert!(matches!(
            cfg.validate(),
            Err(ConfigError::OutOfRange {
                field: "chutes.bag_swap_ticks",
                ..
            })
        ));
        cfg.chutes.bag_swap_ticks = cfg.planner.watchdog_ticks - 1;
        assert_eq!(cfg.validate(), Ok(()));
    }

    #[test]
    fn rejects_a_zero_station_cap() {
        let mut cfg = ScenarioConfig::from_toml_str(SMALL_TOML).unwrap();
        cfg.fleet.max_inbound_per_station = 0;
        assert!(matches!(
            cfg.validate(),
            Err(ConfigError::OutOfRange {
                field: "fleet.max_inbound_per_station",
                ..
            })
        ));
    }
}
