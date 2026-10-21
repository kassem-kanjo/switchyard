//! Interventions: "what would have helped?" as scenario transforms
//! (ADR 0005, step 5).
//!
//! A counterfactual replay changes one thing and re-runs. Each
//! [`Intervention`] is a pure, deterministic transform of a
//! [`ScenarioConfig`] ([`ScenarioConfig::apply`]), so a replay is fully
//! described by `(scenario, seed, interventions)` and reproducible
//! bit-for-bit. Interventions can also be applied to a running sim
//! ([`crate::sim::Sim::apply_intervention`]); there they take effect from the
//! sim's current tick on, which differs from applying them to the scenario
//! (effect from tick 0) except as noted per kind:
//!
//! | Intervention | On the scenario | On a running sim (from tick `t` on) | Same as the scenario when |
//! |---|---|---|---|
//! | `DisableFault(f)` | `f` never happens (`disabled_faults`; ids stay stable) | a pending `f` never starts; an active `f` ends now, cleanly, as if it had healthily ended (its effects so far remain) | `f` has not started yet |
//! | `RemapDestination` | the destination's chute is replaced after the mapping | parcels scanned from now on go to the new chute; parcels already scanned keep their target | applied at tick 0 |
//! | `AddRobots { at }` | robots join at the start of tick `at` | the same, if `at >= t` | `at >= t` |
//! | `SetStationCap` | a station's (or every station's) inbound cap | the cap changes now; robots already inbound keep their claims | applied at tick 0 |
//!
//! Rerouting around an aisle is not offered: it is a planner policy, not a
//! parameter, and the guidance graph already routes around closed cells.

use crate::Tick;
use crate::config::{ConfigError, DestinationRemap, RobotAddition, ScenarioConfig, StationCap};
use crate::ids::{ChuteId, DestinationId, FaultId, StationId};

/// One change to a scenario (see the module docs for the semantics).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intervention {
    /// Switch a scheduled fault off (its healthy counterpart,
    /// [`crate::faults`]).
    DisableFault(FaultId),
    /// Serve a destination from another chute.
    RemapDestination {
        /// The destination.
        destination: DestinationId,
        /// Its new chute.
        chute: ChuteId,
    },
    /// Add robots to the fleet at a tick.
    AddRobots {
        /// How many (at least 1).
        robots: u32,
        /// The tick at whose start they appear.
        at: Tick,
    },
    /// Change the inbound cap of one station, or of all (`None`).
    SetStationCap {
        /// The station, or every station.
        station: Option<StationId>,
        /// The new cap (at least 1).
        cap: u16,
    },
}

impl ScenarioConfig {
    /// The scenario with `intervention` applied (see [`crate::interventions`]).
    /// Checks that need the floor (does the chute or station exist?) happen
    /// when the sim is built.
    ///
    /// # Errors
    /// [`ConfigError::BadFault`] for an unknown fault id, or any
    /// [`ScenarioConfig::validate`] error of the result (a zero cap or robot
    /// count).
    pub fn apply(&self, intervention: &Intervention) -> Result<Self, ConfigError> {
        let mut cfg = self.clone();
        match *intervention {
            Intervention::DisableFault(f) => {
                if f.index() >= cfg.faults.len() {
                    return Err(ConfigError::BadFault(format!(
                        "{f} is not in the schedule ({} faults)",
                        cfg.faults.len()
                    )));
                }
                cfg = cfg.without_fault(f);
            }
            Intervention::RemapDestination { destination, chute } => {
                cfg.chutes.remap.retain(|r| r.destination != destination);
                cfg.chutes
                    .remap
                    .push(DestinationRemap { destination, chute });
            }
            Intervention::AddRobots { robots, at } => {
                cfg.fleet.additions.push(RobotAddition { at, robots });
            }
            Intervention::SetStationCap { station, cap } => match station {
                Some(station) => {
                    cfg.fleet.station_caps.retain(|c| c.station != station);
                    cfg.fleet.station_caps.push(StationCap { station, cap });
                }
                None => {
                    cfg.fleet.max_inbound_per_station = cap;
                    cfg.fleet.station_caps.clear();
                }
            },
        }
        cfg.validate()?;
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::SMALL_TOML;

    fn small() -> ScenarioConfig {
        ScenarioConfig::from_toml_str(SMALL_TOML).unwrap()
    }

    #[test]
    fn interventions_are_pure_config_transforms_that_round_trip() {
        let cfg = small();
        let steps = [
            Intervention::RemapDestination {
                destination: DestinationId(0),
                chute: ChuteId(7),
            },
            Intervention::RemapDestination {
                destination: DestinationId(0),
                chute: ChuteId(8),
            },
            Intervention::AddRobots { robots: 5, at: 300 },
            Intervention::SetStationCap {
                station: Some(StationId(1)),
                cap: 9,
            },
        ];
        let mut out = cfg.clone();
        for s in &steps {
            out = out.apply(s).unwrap();
        }
        // The later remap of a destination replaces the earlier one.
        assert_eq!(
            out.chutes.remap,
            vec![DestinationRemap {
                destination: DestinationId(0),
                chute: ChuteId(8)
            }]
        );
        assert_eq!(
            out.fleet.additions,
            vec![RobotAddition { at: 300, robots: 5 }]
        );
        assert_eq!(out.fleet.station_caps.len(), 1);
        // Deterministic and serialisable: the TOML round trip is exact.
        let again = ScenarioConfig::from_toml_str(&out.to_toml_string().unwrap()).unwrap();
        assert_eq!(again, out);
        // Setting every station's cap drops the per-station overrides.
        let all = out
            .apply(&Intervention::SetStationCap {
                station: None,
                cap: 4,
            })
            .unwrap();
        assert!(all.fleet.station_caps.is_empty());
        assert_eq!(all.fleet.max_inbound_per_station, 4);
        // The input is never modified.
        assert_eq!(cfg, small());
    }

    #[test]
    fn invalid_interventions_are_rejected() {
        let cfg = small();
        assert!(matches!(
            cfg.apply(&Intervention::DisableFault(FaultId(0))),
            Err(ConfigError::BadFault(_))
        ));
        assert!(
            cfg.apply(&Intervention::SetStationCap {
                station: None,
                cap: 0
            })
            .is_err()
        );
        assert!(
            cfg.apply(&Intervention::AddRobots { robots: 0, at: 1 })
                .is_err()
        );
    }
}
