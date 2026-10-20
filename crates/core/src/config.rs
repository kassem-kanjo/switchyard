//! Scenario configuration, loaded from TOML.
//!
//! Every field is an integer so that a scenario file maps to sim state
//! without any float rounding (ADR 0002 §1). Rates are expressed per hour and
//! converted to per-tick probabilities with integer arithmetic.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::TICKS_PER_HOUR;

/// A complete, self-contained description of one simulation scenario.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioConfig {
    /// Human-readable scenario name (for reports and the viewer).
    pub name: String,
    /// Master seed. Every random stream is derived from `(seed, stream_id)`.
    pub seed: u64,
    /// Floor geometry.
    pub floor: FloorConfig,
    /// Robot fleet.
    pub fleet: FleetConfig,
    /// Parcel arrival process.
    pub arrivals: ArrivalConfig,
    /// Durations of fixed operations.
    pub timing: TimingConfig,
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
}

/// Parcel arrival process.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArrivalConfig {
    /// Mean arrivals per hour at each station. V0 draws at most one arrival
    /// per station per tick (a Bernoulli process), so this must not exceed
    /// [`TICKS_PER_HOUR`]. CP1 replaces it with a Poisson process.
    pub parcels_per_hour_per_station: u32,
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
    /// The arrival rate exceeds one parcel per station per tick.
    ArrivalRateTooHigh {
        /// Requested rate.
        per_hour: u32,
    },
    /// A timing value is zero (every operation takes at least one tick).
    ZeroDuration(&'static str),
    /// The generated layout broke an invariant (a station or chute is
    /// unreachable). This is a generator bug or a degenerate floor.
    Unreachable(String),
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
            Self::ZeroDuration(field) => write!(f, "timing.{field} must be at least 1 tick"),
            Self::Unreachable(msg) => write!(f, "layout is not fully connected: {msg}"),
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
        if self.timing.load_ticks == 0 {
            return Err(ConfigError::ZeroDuration("load_ticks"));
        }
        if self.timing.drop_ticks == 0 {
            return Err(ConfigError::ZeroDuration("drop_ticks"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::SMALL_TOML;

    #[test]
    fn small_preset_parses_and_round_trips() {
        let cfg = ScenarioConfig::from_toml_str(SMALL_TOML).unwrap();
        assert_eq!(cfg.name, "small");
        let again = ScenarioConfig::from_toml_str(&cfg.to_toml_string().unwrap()).unwrap();
        assert_eq!(cfg, again);
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
            Err(ConfigError::ZeroDuration("drop_ticks"))
        );
    }
}
