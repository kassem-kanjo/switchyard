//! Automatic post-mortems: detect → shortlist → replay → attribute →
//! remediate → report (ADR 0005, ADR 0006; research 2026-10-20).
//!
//! 1. The scenario runs with its faults (the **actual** history) and as its
//!    fault-free **twin**; [`detect`] finds the incident on the per-minute
//!    deficit and fixes the scoring **horizon** (window end + 10 min).
//! 2. [`candidates`] shortlists the faults to remove (n ≤ 6), capacity
//!    hotspots and candidate fixes.
//! 3. Every subset of candidates is removed in a **replay** that resumes
//!    from the latest checkpoint of the history before the earliest removed
//!    fault and runs to the horizon; every fix likewise. The value of a
//!    replay is the net parcel-minutes of delay against the twin at the
//!    horizon ([`crate::resilience::parcel_delay`]).
//! 4. **Dither** (ADR 0006 §6): the planner's trajectories diverge after any
//!    change to a robot's moves, so the whole lattice is replayed on
//!    `dither` histories: the real one and `dither - 1` copies in which one
//!    moving robot, uninvolved in any candidate, pauses for a single tick at
//!    the first fault's onset. Each mask's value is the median over the histories; the
//!    spread across histories is the noise band printed next to every share.
//! 5. [`shapley`] attributes the median game exactly; [`report`] writes the
//!    Google-SRE-format Markdown, citing a replay id for every number.

pub mod candidates;
pub mod detect;
pub mod report;
pub mod shapley;

use rayon::prelude::*;
use switchyard_core::Tick;
use switchyard_core::config::{ConfigError, ScenarioConfig};
use switchyard_core::events::Event;
use switchyard_core::faults::{FaultKind, FaultSpec};
use switchyard_core::ids::{FaultId, RobotId};
use switchyard_core::sim::Sim;
use switchyard_core::world::RobotState;

use crate::metrics::{MetricsRecorder, MinuteSeries, minute_tick, parcels_of};
use crate::resilience::{self, Params, ParcelDelay, ResilienceScore, Run, TwinIndex, Window};
use crate::twin::{self, FailedRobot};
use candidates::{Blame, Candidate, Fix, Hotspot};
use detect::{CusumParams, Incident};
use shapley::Attribution;

/// Post-mortem settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// Histories to replay on: the real one plus `dither - 1` dithered
    /// copies (1 = no dither).
    pub dither: usize,
    /// Detection and horizon.
    pub cusum: CusumParams,
    /// Ticks between checkpoints of a history (600 = 5 min).
    pub checkpoint_every: Tick,
    /// Share of the recoverable loss a "sufficient" set must recover, in %.
    pub tau_pct: i64,
    /// Late = more than this many extra minutes.
    pub sla_extra_minutes: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            dither: 8,
            cusum: CusumParams::default(),
            checkpoint_every: 600,
            tau_pct: 90,
            sla_extra_minutes: 10,
        }
    }
}

/// What a replay changes on a history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// Remove the candidates in the mask (bit `i` = candidate `i`); mask 0
    /// is the history itself.
    Remove(usize),
    /// Apply fix `i`.
    Fix(usize),
}

/// One replay, run on every history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replay {
    /// Id cited in the report: `R<mask>` or `X<n>`.
    pub id: String,
    /// The change.
    pub change: Change,
    /// Tick it resumed from (the checkpoint), on the real history.
    pub from_tick: Tick,
    /// Net delay at the horizon on each history (parcel-ticks).
    pub delay: Vec<i64>,
    /// Late parcels at the horizon on each history.
    pub late: Vec<u32>,
    /// World hash at the horizon on the real history.
    pub live_hash: u64,
}

impl Replay {
    /// Median net delay over the histories (parcel-ticks).
    #[must_use]
    pub fn median_delay(&self) -> i64 {
        median(&self.delay)
    }

    /// Median late parcels over the histories.
    #[must_use]
    pub fn median_late(&self) -> i64 {
        median(&self.late.iter().map(|&x| i64::from(x)).collect::<Vec<_>>())
    }
}

/// Dither copies tried per wanted copy before giving up (most moving
/// robots diverge; a few pauses are absorbed).
pub const DITHER_TRIES: usize = 3;

/// Histories needed for the dither spread to be trusted as a noise band
/// (#14). With fewer, every band is widened to at least
/// [`NOISE_FLOOR_PCT`] % of the loss.
pub const MIN_HISTORIES: usize = 5;

/// The band floor when the dither is too thin, in % of the loss: the CP2
/// investigation measured a 4–6 % spread under load for jams, failures and
/// surges.
pub const NOISE_FLOOR_PCT: f64 = 5.0;

/// Deliveries an incident's window lost against the twin (net of the backlog
/// drain inside it): the size used to pick which incident to analyse.
fn incident_loss(twin: &[u32], actual: &[u32], inc: &Incident) -> i64 {
    let end = inc.end.min(twin.len()).min(actual.len());
    (inc.start..end)
        .map(|t| i64::from(twin[t]) - i64::from(actual[t]))
        .sum()
}

/// Robots to try as dither copies, in order: `extra` evenly spread moving
/// robots first, then the rest of `moving`, at most `extra · DITHER_TRIES`.
/// Each robot appears once: with fewer moving robots than wanted copies the
/// spread picks would repeat, and pausing the same robot twice gives the same
/// history twice (#14).
fn dither_pool(moving: &[RobotId], extra: usize) -> Vec<RobotId> {
    let spread = (1..=extra).filter_map(|d| moving.get(d * moving.len() / (extra + 1)));
    let mut pool: Vec<RobotId> = Vec::new();
    for &r in spread.chain(moving) {
        if pool.len() == extra * DITHER_TRIES {
            break;
        }
        if !pool.contains(&r) {
            pool.push(r);
        }
    }
    pool
}

/// Index of the incident that lost most deliveries; the earliest wins a tie
/// (`max_by_key` keeps the last maximum, hence the reversed index).
fn largest_incident(twin: &[u32], actual: &[u32], incidents: &[Incident]) -> Option<usize> {
    incidents
        .iter()
        .enumerate()
        .max_by_key(|(i, inc)| (incident_loss(twin, actual, inc), std::cmp::Reverse(*i)))
        .map(|(i, _)| i)
}

/// A difference counts only beyond this many dither bands (ADR 0006 §6:
/// "a tie when their ρ differ by less than twice the dither band").
pub const NOISE_K: f64 = 2.0;

/// Is `value` distinguishable from noise of half-range `band`?
#[must_use]
pub fn significant(value: f64, band: f64) -> bool {
    value.abs() > NOISE_K * band
}

/// A finished post-mortem: everything the report renders.
#[derive(Clone, Debug)]
pub struct PostMortem {
    /// The scenario as run (faults included).
    pub config: ScenarioConfig,
    /// Ticks simulated.
    pub ticks: Tick,
    /// Settings.
    pub options: Options,
    /// The incident, if the CUSUM alarmed.
    pub incident: Option<Incident>,
    /// Alarm ticks of the other incidents of the run, in time order. Only the
    /// largest incident is analysed; the report lists the rest (#15).
    pub other_incidents: Vec<Tick>,
    /// Largest CUSUM value over the whole run (σ units).
    pub cusum_peak: f64,
    /// Scoring horizon (tick).
    pub horizon: Tick,
    /// Per-minute series of the twin and the actual run.
    pub twin_series: MinuteSeries,
    /// The actual run's series.
    pub actual_series: MinuteSeries,
    /// Whole-run resilience score of the actual run against the twin.
    pub score: ResilienceScore,
    /// Fault starts and ends, robot clear-offs: `(tick, text)`.
    pub timeline: Vec<(Tick, String)>,
    /// Candidate causes (Shapley players, in this order).
    pub candidates: Vec<Candidate>,
    /// Faults left out by the cap.
    pub dropped: Vec<Candidate>,
    /// Capacity hotspots.
    pub hotspots: Vec<Hotspot>,
    /// Candidate fixes.
    pub fixes: Vec<Fix>,
    /// Dither robots, one per extra history (only copies that diverged).
    pub dither: Vec<RobotId>,
    /// Dither copies tried, diverging or not.
    pub dither_tried: usize,
    /// World hash at the horizon of each history (index 0 = the real one).
    pub history_hashes: Vec<u64>,
    /// Tick of the dither failures.
    pub dither_tick: Tick,
    /// Every replay: masks `0..2ⁿ` first, then fixes.
    pub replays: Vec<Replay>,
    /// Attribution of the median game (values in parcel-ticks).
    pub attribution: Attribution,
    /// Attribution on each history alone (for the noise band).
    pub per_history: Vec<Attribution>,
}

impl PostMortem {
    /// The mask replays (index = mask).
    #[must_use]
    pub fn masks(&self) -> &[Replay] {
        &self.replays[..1 << self.candidates.len()]
    }

    /// The fix replays (index = fix).
    #[must_use]
    pub fn fix_replays(&self) -> &[Replay] {
        &self.replays[1 << self.candidates.len()..]
    }

    /// Histories every replay ran on (the real one plus diverging copies).
    #[must_use]
    pub fn histories(&self) -> usize {
        self.dither.len() + 1
    }

    /// True when too few histories diverged to trust their spread; every
    /// band is then widened (see [`MIN_HISTORIES`]).
    #[must_use]
    pub fn thin_dither(&self) -> bool {
        self.histories() < MIN_HISTORIES
    }

    /// `band`, widened to the noise floor when the dither is thin.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn widen(&self, band: f64) -> f64 {
        if self.thin_dither() {
            band.max(NOISE_FLOOR_PCT / 100.0 * self.loss().abs() as f64)
        } else {
            band
        }
    }

    /// Median delay of the actual shift at the horizon (parcel-ticks).
    #[must_use]
    pub fn loss(&self) -> i64 {
        self.masks()[0].median_delay()
    }

    /// Fix `i`'s median gain and its noise band (parcel-ticks).
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn fix_stats(&self, i: usize) -> (i64, f64) {
        let g = self.fix_gain(i);
        (median(&g), self.widen(band(g.iter().map(|&x| x as f64))))
    }

    /// Loss recovered by fix `i` on each history (parcel-ticks).
    #[must_use]
    pub fn fix_gain(&self, i: usize) -> Vec<i64> {
        let base = &self.masks()[0].delay;
        self.fix_replays()[i]
            .delay
            .iter()
            .zip(base)
            .map(|(f, b)| b - f)
            .collect()
    }

    /// Half the range of candidate `i`'s Shapley value across histories
    /// (parcel-ticks): the noise band.
    #[must_use]
    pub fn share_band(&self, i: usize) -> f64 {
        self.widen(band(self.per_history.iter().map(|a| a.shapley(i))))
    }

    /// Half the range of the actual history's delay across histories.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn loss_band(&self) -> f64 {
        self.widen(band(self.masks()[0].delay.iter().map(|&d| d as f64)))
    }
}

/// Half the range of `xs` (0 when empty).
#[must_use]
pub fn band(xs: impl Iterator<Item = f64>) -> f64 {
    let (lo, hi) = xs.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
        (lo.min(x), hi.max(x))
    });
    if lo.is_finite() { (hi - lo) / 2.0 } else { 0.0 }
}

/// Median of `xs` (the mean of the two middle values, rounded down, for an
/// even count; 0 when empty).
#[must_use]
pub fn median(xs: &[i64]) -> i64 {
    let mut v = xs.to_vec();
    v.sort_unstable();
    match v.len() {
        0 => 0,
        n if n % 2 == 1 => v[n / 2],
        n => (v[n / 2 - 1] + v[n / 2]).div_euclid(2),
    }
}

/// A post-mortem failure.
#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<ConfigError> for Error {
    fn from(e: ConfigError) -> Self {
        Self(e.to_string())
    }
}

/// A recorded history: checkpoints every `every` ticks from tick 0.
struct History {
    checkpoints: Vec<Sim>,
    every: Tick,
}

impl History {
    /// The latest checkpoint at or before `tick`.
    fn at(&self, tick: Tick) -> &Sim {
        let i = (tick / self.every) as usize;
        &self.checkpoints[i.min(self.checkpoints.len() - 1)]
    }
}

/// Run `cfg` to `until`, checkpointing, and hand every tick's events to
/// `observe`; returns the history and the sim at `until`.
fn record(
    cfg: &ScenarioConfig,
    until: Tick,
    every: Tick,
    mut observe: impl FnMut(&Sim, &[Event]),
) -> Result<(History, Sim), Error> {
    let mut sim = Sim::new(cfg)?;
    let mut checkpoints = Vec::new();
    let mut events = Vec::new();
    while sim.tick() < until {
        if sim.tick() % every == 0 {
            checkpoints.push(sim.clone());
        }
        events.clear();
        sim.step(&mut events);
        observe(&sim, &events);
    }
    if sim.tick() % every == 0 {
        checkpoints.push(sim.clone());
    }
    Ok((History { checkpoints, every }, sim))
}

/// Run a replay of `change` on `history` to `horizon`.
fn replay(
    history: &History,
    candidates: &[Candidate],
    fixes: &[Fix],
    change: &Change,
    horizon: Tick,
) -> Result<(Tick, Sim), Error> {
    let start = match *change {
        Change::Remove(0) => horizon,
        Change::Remove(mask) => candidates
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, c)| c.spec.at)
            .min()
            .unwrap_or(horizon)
            .min(horizon),
        Change::Fix(i) => fixes[i].at.min(horizon),
    };
    let mut sim = history.at(start).clone();
    let from = sim.tick();
    let mut events = Vec::new();
    match *change {
        Change::Remove(mask) => {
            for (i, c) in candidates.iter().enumerate() {
                if mask & (1 << i) != 0 {
                    sim.disable_fault(c.fault)?;
                }
            }
        }
        Change::Fix(i) => {
            while sim.tick() < start {
                events.clear();
                sim.step(&mut events);
            }
            for iv in &fixes[i].interventions {
                sim.apply_intervention(iv)?;
            }
        }
    }
    while sim.tick() < horizon {
        events.clear();
        sim.step(&mut events);
    }
    Ok((from, sim))
}

/// Run a full post-mortem of `cfg` over `ticks` ticks.
///
/// # Errors
/// An invalid scenario or intervention.
#[allow(clippy::too_many_lines)]
pub fn run(cfg: &ScenarioConfig, ticks: Tick, options: &Options) -> Result<PostMortem, Error> {
    let every = options.checkpoint_every.max(1);
    let (baseline, actual_cfg) = twin::twin_configs(cfg, &[]);

    // 1. Twin and actual, in parallel.
    let (twin_run, actual) = rayon::join(
        || twin::run(&baseline, ticks),
        || -> Result<_, Error> {
            let mut recorder = None::<MetricsRecorder>;
            let mut blame = Blame::default();
            let mut failures: Vec<FailedRobot> = Vec::new();
            let mut timeline = Vec::new();
            let (history, end) = record(&actual_cfg, ticks, every, |sim, events| {
                let rec =
                    recorder.get_or_insert_with(|| MetricsRecorder::new(sim.world().robots.len()));
                rec.record_tick(events);
                blame.record(events);
                twin::record_failures(&mut failures, events, sim);
                for e in events {
                    match *e {
                        Event::FaultStarted { tick, fault } => {
                            timeline
                                .push((tick, format!("{fault} starts: {}", describe(sim, fault))));
                        }
                        Event::FaultEnded { tick, fault } => {
                            timeline.push((tick, format!("{fault} ends")));
                        }
                        Event::RobotCleared {
                            tick,
                            robot,
                            station,
                        } => timeline.push((
                            tick,
                            format!("{robot} pushed off {station}'s loading cell by operators"),
                        )),
                        _ => {}
                    }
                }
            })?;
            let rec = recorder.unwrap_or_else(|| MetricsRecorder::new(end.world().robots.len()));
            for f in &mut failures {
                f.to = f.to.min(ticks);
            }
            Ok((
                history,
                rec.finish(&end),
                rec.parcels(&end),
                blame,
                failures,
                timeline,
            ))
        },
    );
    let twin_run = twin_run?;
    let (history, actual_series, actual_parcels, blame, failures, timeline) = actual?;

    // 2. Detection, window, horizon.
    let incidents = detect::detect_all(
        &twin_run.series.delivered_regular,
        &actual_series.delivered_regular,
        &options.cusum,
    );
    let main = largest_incident(
        &twin_run.series.delivered_regular,
        &actual_series.delivered_regular,
        &incidents,
    );
    let incident = main.map(|i| incidents[i]);
    let cusum_peak = detect::cusum(
        &twin_run.series.delivered_regular,
        &actual_series.delivered_regular,
        options.cusum.k,
    )
    .into_iter()
    .fold(0.0, f64::max);
    let minute = minute_tick;
    let other_incidents: Vec<Tick> = incidents
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != main)
        .map(|(_, inc)| minute(inc.alarm))
        .collect();
    let (window_start, window_end, horizon) = match &incident {
        Some(inc) => (
            minute(inc.start),
            minute(inc.end),
            minute(inc.horizon(&options.cusum)).min(ticks),
        ),
        None => (0, ticks, ticks),
    };
    let faults: Vec<FaultSpec> = actual_cfg
        .enabled_faults()
        .map(|(_, f)| f.clone())
        .collect();
    let loss = twin::capacity_loss(
        history.at(0).layout(),
        twin_run.capacity.robots,
        &faults,
        &failures,
        window_start,
        window_end,
    );
    let score = resilience::score(
        Run {
            series: &twin_run.series,
            parcels: &twin_run.parcels,
            ticks,
        },
        Run {
            series: &actual_series,
            parcels: &actual_parcels,
            ticks,
        },
        Window {
            onset: incident.map_or(0, |i| i.start),
            end: incident.map_or(0, |i| i.end),
            capacity_retained: loss.retained,
            stations_lost: loss.stations_lost,
            chutes_lost: loss.chutes_lost,
        },
        twin_run.series.mean_utilisation(),
        Params {
            sla_extra_minutes: options.sla_extra_minutes,
            ..Params::default()
        },
    );

    // 3. Candidates, fixes, hotspots.
    let (candidates, dropped) = if incident.is_some() {
        candidates::candidates(&actual_cfg, window_end, &blame)
    } else {
        (Vec::new(), Vec::new())
    };
    let onset = candidates
        .iter()
        .map(|c| c.spec.at)
        .min()
        .unwrap_or(window_start)
        .min(horizon);
    let mut at_onset = history.at(onset).clone();
    let mut ev = Vec::new();
    while at_onset.tick() < onset {
        ev.clear();
        at_onset.step(&mut ev);
    }
    let (fixes, hotspots) = if incident.is_some() {
        candidates::fixes_and_hotspots(&at_onset, onset, window_start, &candidates, &blame)
    } else {
        (Vec::new(), Vec::new())
    };

    // 4. Dithered histories: one moving robot (driving to a station or a
    // chute), not a candidate's, pauses for 1 tick at the onset. An idle
    // parked robot would change nothing (CP2 investigation): the dither must
    // touch a trajectory to sample the divergence.
    let busy: Vec<RobotId> = candidates
        .iter()
        .filter_map(|c| match c.spec.kind {
            FaultKind::RobotFailure { robot } => Some(robot),
            _ => None,
        })
        .collect();
    let stations = &at_onset.layout().stations;
    let moving: Vec<RobotId> = (0..at_onset.world().robots.len())
        .filter(|&r| {
            matches!(
                at_onset.world().robots.state[r],
                RobotState::ToInduction | RobotState::ToChute
            ) && !stations.contains(&at_onset.world().robots.pos[r])
        })
        .filter_map(|r| u32::try_from(r).ok().map(RobotId))
        .filter(|r| !busy.contains(r))
        .collect();
    // A copy counts only if it actually diverges: its world hash at the
    // horizon must differ from the real history's (a pause can be absorbed,
    // e.g. by a robot about to wait anyway). Otherwise another robot is
    // drawn, up to `DITHER_TRIES` per wanted copy (#14).
    let real_end = {
        let mut sim = history.at(horizon).clone();
        while sim.tick() < horizon {
            ev.clear();
            sim.step(&mut ev);
        }
        sim.live_hash()
    };
    let extra = options.dither.max(1) - 1;
    let pool = dither_pool(&moving, extra);
    let mut dither: Vec<RobotId> = Vec::new();
    let mut dithered: Vec<History> = Vec::new();
    let mut history_hashes = vec![real_end];
    let mut next = 0;
    while dither.len() < extra && next < pool.len() {
        let batch = &pool[next..(next + extra - dither.len()).min(pool.len())];
        next += batch.len();
        let tried: Vec<(RobotId, History, u64)> = batch
            .par_iter()
            .map(|&robot| {
                let mut c = actual_cfg.clone();
                c.faults.push(FaultSpec {
                    at: onset,
                    duration: 1,
                    kind: FaultKind::RobotFailure { robot },
                });
                record(&c, horizon, every, |_, _| {}).map(|(h, end)| (robot, h, end.live_hash()))
            })
            .collect::<Result<_, _>>()?;
        for (robot, h, end) in tried {
            // A copy identical to one already kept (two pauses that lead to
            // the same world) is not an independent sample either.
            if !history_hashes.contains(&end) {
                dither.push(robot);
                dithered.push(h);
                history_hashes.push(end);
            }
        }
    }
    let dither_tried = next;
    let histories: Vec<&History> = std::iter::once(&history).chain(&dithered).collect();

    // 5. Every (change, history) replay, in parallel, collected in order.
    let changes: Vec<Change> = (0..1usize << candidates.len())
        .map(Change::Remove)
        .chain((0..fixes.len()).map(Change::Fix))
        .collect();
    let jobs: Vec<(usize, usize)> = (0..changes.len())
        .flat_map(|c| (0..histories.len()).map(move |h| (c, h)))
        .collect();
    let twin_index = TwinIndex::new(&twin_run.parcels);
    let results = jobs
        .par_iter()
        .map(|&(c, h)| {
            let (from, sim) = replay(histories[h], &candidates, &fixes, &changes[c], horizon)?;
            let d = resilience::parcel_delay_indexed(
                &twin_index,
                &parcels_of(&sim),
                horizon,
                options.sla_extra_minutes,
            );
            Ok((from, d, sim.live_hash()))
        })
        .collect::<Result<Vec<(Tick, ParcelDelay, u64)>, Error>>()?;
    let replays: Vec<Replay> = changes
        .iter()
        .enumerate()
        .map(|(c, change)| {
            let rows = &results[c * histories.len()..(c + 1) * histories.len()];
            Replay {
                id: match *change {
                    Change::Remove(m) => format!("R{m}"),
                    Change::Fix(i) => format!("X{}", i + 1),
                },
                change: change.clone(),
                from_tick: rows[0].0,
                delay: rows.iter().map(|r| r.1.net_parcel_ticks).collect(),
                late: rows.iter().map(|r| r.1.late_parcels).collect(),
                live_hash: rows[0].2,
            }
        })
        .collect();

    // 6. Attribution: v(S) = L(∅) - L(S), per history and on the median.
    let masks = 1usize << candidates.len();
    let v = |m: usize, h: usize| replays[0].delay[h] - replays[m].delay[h];
    let per_history = (0..histories.len())
        .map(|h| Attribution::new(&(0..masks).map(|m| v(m, h)).collect::<Vec<_>>()))
        .collect();
    // The median game: the median over histories of each v(S).
    let median_game: Vec<i64> = (0..masks)
        .map(|m| median(&(0..histories.len()).map(|h| v(m, h)).collect::<Vec<_>>()))
        .collect();

    Ok(PostMortem {
        config: actual_cfg,
        ticks,
        options: *options,
        incident,
        other_incidents,
        cusum_peak,
        horizon,
        twin_series: twin_run.series,
        actual_series,
        score,
        timeline,
        candidates,
        dropped,
        hotspots,
        fixes,
        dither,
        dither_tried,
        history_hashes,
        dither_tick: onset,
        replays,
        attribution: Attribution::new(&median_game),
        per_history,
    })
}

/// A fault's kind in a few words.
fn describe(sim: &Sim, fault: FaultId) -> String {
    sim.config()
        .faults
        .get(fault.index())
        .map_or_else(String::new, |f| describe_kind(&f.kind))
}

/// A fault kind in a few words.
#[must_use]
pub fn describe_kind(kind: &FaultKind) -> String {
    match kind {
        FaultKind::RobotFailure { robot } => format!("robot {robot} breaks down"),
        FaultKind::AisleBlock { cells } => format!("aisle blocked ({} cells)", cells.len()),
        FaultKind::ChuteJam { chute } => format!("chute {chute} jams"),
        FaultKind::InductionOutage { station } => format!("station {station} stops inducting"),
        FaultKind::VolumeSurge {
            station,
            factor_pct,
        } => format!(
            "volume surge x{:.1} at {}",
            f64::from(*factor_pct) / 100.0,
            station.map_or_else(|| "all stations".to_owned(), |s| format!("station {s}"))
        ),
        FaultKind::ScannerDrift {
            station,
            error_rate_ppm,
        } => format!(
            "scanner at station {station} misreads {:.1} %",
            f64::from(*error_rate_ppm) / 10_000.0
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::detect::{CusumParams, detect_all};
    use super::{DITHER_TRIES, dither_pool, largest_incident};
    use switchyard_core::ids::RobotId;

    #[test]
    fn the_dither_pool_never_repeats_a_robot() {
        let moving: Vec<RobotId> = (0..4).map(RobotId).collect();
        // More copies wanted than moving robots: each robot once.
        let pool = dither_pool(&moving, 7);
        assert_eq!(pool.len(), 4);
        let mut sorted = pool.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), pool.len(), "{pool:?}");
        // Spread picks first, capped at extra * DITHER_TRIES.
        let moving: Vec<RobotId> = (0..20).map(RobotId).collect();
        let pool = dither_pool(&moving, 2);
        assert_eq!(pool.len(), 2 * DITHER_TRIES);
        assert_eq!(&pool[..2], &[RobotId(6), RobotId(13)]);
        assert!(dither_pool(&moving, 0).is_empty());
    }

    #[test]
    fn the_largest_incident_is_analysed_and_a_tie_goes_to_the_earliest() {
        let p = CusumParams::default();
        let twin = [100u32; 120];
        // A short dip, then a deeper and longer one.
        let mut actual = [100u32; 120];
        actual[3..6].fill(60);
        actual[60..66].fill(30);
        let all = detect_all(&twin, &actual, &p);
        assert_eq!(all.len(), 2, "{all:?}");
        assert_eq!(largest_incident(&twin, &actual, &all), Some(1));
        // The same dip twice: equal loss, so the first.
        let mut same = [100u32; 120];
        same[3..6].fill(40);
        same[70..73].fill(40);
        let all = detect_all(&twin, &same, &p);
        assert_eq!(all.len(), 2, "{all:?}");
        assert_eq!(largest_incident(&twin, &same, &all), Some(0));
        assert_eq!(largest_incident(&twin, &twin, &[]), None);
    }
}
