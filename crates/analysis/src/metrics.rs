//! Per-minute metric series derived from the event stream (ARCHITECTURE §7,
//! step 1).
//!
//! A [`MetricsRecorder`] is fed one tick's events at a time, so a run never
//! has to keep its whole event stream in memory. Series cover complete
//! minutes only (1 minute = 120 ticks); a trailing partial minute is
//! dropped so every bucket has the same length.

use switchyard_core::Tick;
use switchyard_core::events::Event;
use switchyard_core::faults::SURGE_KEY_FLAG;
use switchyard_core::ids::StationId;
use switchyard_core::sim::Sim;
use switchyard_core::world::{Phase, RobotState};

/// Ticks per simulated minute (1 tick = 0.5 s).
pub const TICKS_PER_MINUTE: Tick = 120;

/// The first tick of minute `m` (saturating).
#[must_use]
pub fn minute_tick(m: usize) -> Tick {
    Tick::try_from(m)
        .unwrap_or(Tick::MAX)
        .saturating_mul(TICKS_PER_MINUTE)
}

/// One parcel of a run: its cross-run identity and its timing.
///
/// `(station, key)` identifies a parcel across a run and its twin: draw keys
/// are counter-based per station (ADR 0002 §3), so regular parcel `k` at
/// station `s` arrives at the same tick with the same destination in both
/// runs. Parcel ids are *not* stable across runs (a surge interleaves extra
/// ids), so matching must use this pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParcelRecord {
    /// Induction station.
    pub station: StationId,
    /// Draw key (the station ordinal for regular parcels).
    pub key: u64,
    /// Arrival tick.
    pub arrived: Tick,
    /// Drop tick, or `None` if the parcel was not delivered in the run.
    pub dropped: Option<Tick>,
}

impl ParcelRecord {
    /// True for the extra parcels of a volume surge.
    #[must_use]
    pub const fn is_surge(&self) -> bool {
        self.key & SURGE_KEY_FLAG != 0
    }

    /// Latency in ticks, censored at `horizon` (a parcel still on the floor
    /// at the end of the run counts as delivered at `horizon`).
    #[must_use]
    pub fn latency_censored(&self, horizon: Tick) -> Tick {
        let end = self.dropped.map_or(horizon, |d| d.min(horizon));
        end.saturating_sub(self.arrived)
    }

    /// True if the parcel was delivered strictly before `horizon`.
    #[must_use]
    pub fn delivered_by(&self, horizon: Tick) -> bool {
        self.dropped.is_some_and(|d| d < horizon)
    }
}

/// Per-minute metrics of one run. Every vector has one entry per complete
/// minute.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MinuteSeries {
    /// Parcels that arrived in the minute (surge parcels included).
    pub arrived: Vec<u32>,
    /// Parcels dropped into a chute in the minute (surge parcels included).
    pub delivered: Vec<u32>,
    /// Regular parcels dropped in the minute: every parcel except the extra
    /// arrivals of a volume surge. Regular parcels are identical in a run
    /// and its fault-free twin (common random numbers, ADR 0002 §3), so this
    /// is the series a twin comparison measures.
    pub delivered_regular: Vec<u32>,
    /// Parcels arrived but not yet delivered at the end of the minute.
    pub backlog: Vec<u32>,
    /// Mean share of robots with work (driving to a station or a chute,
    /// loading or dropping) over the minute's ticks, in `[0, 1]`.
    pub utilisation: Vec<f64>,
}

impl MinuteSeries {
    /// Number of complete minutes.
    #[must_use]
    pub fn minutes(&self) -> usize {
        self.delivered.len()
    }

    /// The parcel-count series of a run of `ticks` ticks (utilisation is left
    /// at 0; [`MetricsRecorder::finish`] fills it in).
    #[must_use]
    pub fn from_parcels(parcels: &[ParcelRecord], ticks: Tick) -> Self {
        let minutes = (ticks / TICKS_PER_MINUTE) as usize;
        let mut s = Self {
            arrived: vec![0; minutes],
            delivered: vec![0; minutes],
            delivered_regular: vec![0; minutes],
            backlog: vec![0; minutes],
            utilisation: vec![0.0; minutes],
        };
        let bucket = |t: Tick| (t / TICKS_PER_MINUTE) as usize;
        for p in parcels {
            if let Some(a) = s.arrived.get_mut(bucket(p.arrived)) {
                *a += 1;
            }
            let Some(d) = p.dropped else {
                continue;
            };
            let m = bucket(d);
            if m < minutes {
                s.delivered[m] += 1;
                if !p.is_surge() {
                    s.delivered_regular[m] += 1;
                }
            }
        }
        let (mut arrived, mut delivered) = (0u32, 0u32);
        for m in 0..minutes {
            arrived += s.arrived[m];
            delivered += s.delivered[m];
            s.backlog[m] = arrived - delivered;
        }
        s
    }

    /// Mean utilisation over the whole run (0 for an empty series).
    #[must_use]
    pub fn mean_utilisation(&self) -> f64 {
        if self.utilisation.is_empty() {
            return 0.0;
        }
        self.utilisation.iter().sum::<f64>() / self.utilisation.len() as f64
    }
}

const fn has_work(state: RobotState) -> bool {
    matches!(
        state,
        RobotState::ToInduction | RobotState::Loading | RobotState::ToChute | RobotState::Dropping
    )
}

/// Every parcel of `sim` read from its current state, without the event
/// stream: a delivered parcel's drop tick is the start of its `Dropped`
/// phase. Replays that resume from a checkpoint use this (they have no
/// events before the checkpoint); it equals [`MetricsRecorder::parcels`]
/// for a run recorded from tick 0.
#[must_use]
pub fn parcels_of(sim: &Sim) -> Vec<ParcelRecord> {
    let p = &sim.world().parcels;
    (0..p.len())
        .map(|i| ParcelRecord {
            station: p.station[i],
            key: p.key[i],
            arrived: p.arrived[i],
            dropped: (p.phase[i] == Phase::Dropped).then_some(p.phase_since[i]),
        })
        .collect()
}

/// Builds a [`MinuteSeries`] from a run's events, one tick at a time.
#[derive(Clone, Debug)]
pub struct MetricsRecorder {
    robots: usize,
    working: Vec<bool>,
    busy: u64,
    tick: Tick,
    /// Arrival tick and drop tick of every parcel, by parcel id.
    arrived_at: Vec<Tick>,
    dropped_at: Vec<Option<Tick>>,
    /// Robot-ticks with work, per minute.
    busy_ticks: Vec<u64>,
}

impl MetricsRecorder {
    /// A recorder for a fleet of `robots` robots, all idle at tick 0.
    #[must_use]
    pub fn new(robots: usize) -> Self {
        Self {
            robots,
            working: vec![false; robots],
            busy: 0,
            tick: 0,
            arrived_at: Vec::new(),
            dropped_at: Vec::new(),
            busy_ticks: Vec::new(),
        }
    }

    /// Record the events of the next tick (exactly one tick's worth, as
    /// [`Sim::step`] appends them).
    pub fn record_tick(&mut self, events: &[Event]) {
        for e in events {
            match *e {
                Event::ParcelArrived { tick, parcel, .. } => {
                    let i = parcel.index();
                    if self.arrived_at.len() <= i {
                        self.arrived_at.resize(i + 1, 0);
                        self.dropped_at.resize(i + 1, None);
                    }
                    self.arrived_at[i] = tick;
                }
                Event::ParcelDropped { tick, parcel, .. } => {
                    if let Some(d) = self.dropped_at.get_mut(parcel.index()) {
                        *d = Some(tick);
                    }
                }
                Event::RobotState { robot, state, .. } => {
                    let r = robot.index();
                    let now = has_work(state);
                    if r < self.robots && self.working[r] != now {
                        self.working[r] = now;
                        if now {
                            self.busy += 1;
                        } else {
                            self.busy -= 1;
                        }
                    }
                }
                _ => {}
            }
        }
        let minute = (self.tick / TICKS_PER_MINUTE) as usize;
        if self.busy_ticks.len() <= minute {
            self.busy_ticks.resize(minute + 1, 0);
        }
        self.busy_ticks[minute] += self.busy;
        self.tick += 1;
    }

    /// Every parcel that arrived, by parcel id. `sim` is the run's final
    /// state, which holds each parcel's station and draw key.
    #[must_use]
    pub fn parcels(&self, sim: &Sim) -> Vec<ParcelRecord> {
        let p = &sim.world().parcels;
        self.arrived_at
            .iter()
            .zip(&self.dropped_at)
            .enumerate()
            .map(|(i, (&arrived, &dropped))| ParcelRecord {
                station: p.station[i],
                key: p.key[i],
                arrived,
                dropped,
            })
            .collect()
    }

    /// Ticks recorded so far.
    #[must_use]
    pub const fn ticks(&self) -> Tick {
        self.tick
    }

    /// The series. `sim` is the run's final state, used to tell surge
    /// parcels from regular ones.
    #[must_use]
    pub fn finish(&self, sim: &Sim) -> MinuteSeries {
        let mut s = MinuteSeries::from_parcels(&self.parcels(sim), self.tick);
        let robot_ticks = self.robots as f64 * f64::from(TICKS_PER_MINUTE);
        for (m, u) in s.utilisation.iter_mut().enumerate() {
            *u = if robot_ticks > 0.0 {
                self.busy_ticks[m] as f64 / robot_ticks
            } else {
                0.0
            };
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use switchyard_core::config::ScenarioConfig;
    use switchyard_core::presets::SMALL_TOML;

    #[test]
    fn series_agree_with_the_sim_counters() {
        let cfg = ScenarioConfig::from_toml_str(SMALL_TOML).unwrap();
        let mut sim = Sim::new(&cfg).unwrap();
        let mut rec = MetricsRecorder::new(cfg.fleet.robots as usize);
        let mut events = Vec::new();
        for _ in 0..(10 * TICKS_PER_MINUTE + 30) {
            events.clear();
            sim.step(&mut events);
            rec.record_tick(&events);
        }
        let s = rec.finish(&sim);
        assert_eq!(s.minutes(), 10, "the partial 11th minute is dropped");
        // Up to the end of minute 10 (tick 1,200), counts match a run cut
        // there.
        let mut cut = Sim::new(&cfg).unwrap();
        events.clear();
        for _ in 0..(10 * TICKS_PER_MINUTE) {
            cut.step(&mut events);
        }
        let st = cut.stats();
        assert_eq!(u64::from(s.arrived.iter().sum::<u32>()), st.arrived);
        assert_eq!(u64::from(s.delivered.iter().sum::<u32>()), st.delivered);
        assert_eq!(s.delivered, s.delivered_regular, "no surge, no difference");
        assert_eq!(u64::from(s.backlog[9]), st.queued + st.in_flight);
        assert!(s.utilisation.iter().all(|&u| (0.0..=1.0).contains(&u)));
        assert!(s.mean_utilisation() > 0.0);
    }

    #[test]
    fn parcels_read_from_the_state_match_the_recorded_ones() {
        let mut cfg = ScenarioConfig::from_toml_str(SMALL_TOML).unwrap();
        cfg.faults.push(switchyard_core::faults::FaultSpec {
            at: 300,
            duration: 600,
            kind: switchyard_core::faults::FaultKind::VolumeSurge {
                station: None,
                factor_pct: 200,
            },
        });
        let mut sim = Sim::new(&cfg).unwrap();
        let mut rec = MetricsRecorder::new(cfg.fleet.robots as usize);
        let mut events = Vec::new();
        for _ in 0..1_800 {
            events.clear();
            sim.step(&mut events);
            rec.record_tick(&events);
        }
        let from_state = parcels_of(&sim);
        assert_eq!(from_state, rec.parcels(&sim));
        assert!(from_state.iter().any(ParcelRecord::is_surge));
    }
}
