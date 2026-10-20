//! Per-minute metric series derived from the event stream (ARCHITECTURE §7,
//! step 1).
//!
//! A [`MetricsRecorder`] is fed one tick's events at a time, so a run never
//! has to keep its whole event stream in memory. Series cover complete
//! minutes only (1 minute = 120 ticks); a trailing partial minute is
//! dropped so every bucket has the same length.

use switchyard_core::Tick;
use switchyard_core::events::Event;
use switchyard_core::sim::Sim;
use switchyard_core::world::RobotState;

/// Ticks per simulated minute (1 tick = 0.5 s).
pub const TICKS_PER_MINUTE: Tick = 120;

/// Draw keys at or above this value belong to volume-surge parcels
/// (`switchyard_core::world::Parcels::key`).
const SURGE_KEY: u64 = 1 << 62;

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

    /// The series. `sim` is the run's final state, used to tell surge
    /// parcels from regular ones.
    #[must_use]
    pub fn finish(&self, sim: &Sim) -> MinuteSeries {
        let keys = &sim.world().parcels.key;
        let minutes = (self.tick / TICKS_PER_MINUTE) as usize;
        let mut s = MinuteSeries {
            arrived: vec![0; minutes],
            delivered: vec![0; minutes],
            delivered_regular: vec![0; minutes],
            backlog: vec![0; minutes],
            utilisation: vec![0.0; minutes],
        };
        let bucket = |t: Tick| (t / TICKS_PER_MINUTE) as usize;
        for (i, &at) in self.arrived_at.iter().enumerate() {
            if let Some(a) = s.arrived.get_mut(bucket(at)) {
                *a += 1;
            }
            let Some(d) = self.dropped_at[i] else {
                continue;
            };
            let m = bucket(d);
            if m < minutes {
                s.delivered[m] += 1;
                if keys.get(i).is_none_or(|&k| k < SURGE_KEY) {
                    s.delivered_regular[m] += 1;
                }
            }
        }
        let (mut arrived, mut delivered) = (0u32, 0u32);
        for m in 0..minutes {
            arrived += s.arrived[m];
            delivered += s.delivered[m];
            s.backlog[m] = arrived - delivered;
            let robot_ticks = self.robots as f64 * f64::from(TICKS_PER_MINUTE);
            s.utilisation[m] = if robot_ticks > 0.0 {
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
}
