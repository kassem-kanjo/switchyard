//! What a post-mortem replays: candidate causes, capacity hotspots and
//! candidate fixes (ADR 0006 §3 and §5).
//!
//! - **Candidates** are causes to *remove*: every enabled fault that
//!   started before the incident window closed and had not already ended
//!   before it opened (a fault that was over cannot have caused it, #21).
//!   Beyond [`MAX_CANDIDATES`],
//!   the ones with the most fault blame (parcel-ticks of `ParcelDelayed`
//!   rooted at the fault) are kept.
//! - **Hotspots** are capacity limits the floor hit without any fault
//!   (robots queuing at a busy station, chutes swapping bags), ranked by
//!   robot-ticks of root blame. They answer a different counterfactual,
//!   "relax this capacity" rather than "remove this fault", so they are
//!   reported separately, never mixed into the Shapley shares.
//! - **Fixes** are [`Intervention`]s applied to the real (faulted) history
//!   at a given tick: "what would have helped".

use std::collections::BTreeMap;

use switchyard_core::Tick;
use switchyard_core::config::ScenarioConfig;
use switchyard_core::events::{Event, WaitCause};
use switchyard_core::faults::{FaultKind, FaultSpec};
use switchyard_core::ids::{ChuteId, DestinationId, FaultId, StationId};
use switchyard_core::interventions::Intervention;
use switchyard_core::sim::Sim;

use crate::metrics::TICKS_PER_MINUTE;

/// Most candidate causes (2⁶ = 64 replays per dither).
pub const MAX_CANDIDATES: usize = 6;

/// Hotspots with less root blame than this (robot-ticks; 10 robot-minutes)
/// are not reported: too small to matter next to a fault.
pub const MIN_HOTSPOT_BLAME: u64 = 1_200;

/// A fault considered as a cause.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// Its stable id in the schedule.
    pub fault: FaultId,
    /// The fault.
    pub spec: FaultSpec,
    /// Parcel-ticks of delay the trace blames on it (direct and root).
    pub blame: u64,
}

/// A capacity limit the floor hit without a fault.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hotspot {
    /// Plain-language label.
    pub label: String,
    /// Robot-ticks of waiting rooted at it.
    pub blame: u64,
    /// The relax-capacity fix that tests it, if one is modelled.
    pub fix: Option<usize>,
}

/// A candidate fix: interventions applied to the real history at `at`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    /// Plain-language description.
    pub label: String,
    /// What it costs, in plain words.
    pub cost: String,
    /// Tick from which it applies.
    pub at: Tick,
    /// The interventions.
    pub interventions: Vec<Intervention>,
    /// True for a hotspot's relax-capacity test.
    pub hotspot: bool,
    /// For a remap fix, the chute whose destinations it moves (the headline
    /// names it; two jams can start on the same tick, so it is not inferable).
    pub from_chute: Option<ChuteId>,
}

/// Blame read from the actual run's events, counted per event as the run
/// streams (`ParcelDelayed` parcel-ticks per root fault, `RobotWaited`
/// robot-ticks per capacity root). `switchyard_trace::blame::BlameIndex`
/// holds the same roots per span, but needs every parcel's trace built;
/// candidate ranking only needs these totals, so they are counted without
/// building traces.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Blame {
    /// Parcel-ticks of delay rooted at each fault.
    pub fault: BTreeMap<FaultId, u64>,
    /// Robot-ticks waiting on a busy station (no fault).
    pub station_busy: BTreeMap<StationId, u64>,
    /// Robot-ticks waiting on a chute swapping bags (no fault).
    pub chute_busy: BTreeMap<ChuteId, u64>,
}

impl Blame {
    /// Add one tick's events.
    pub fn record(&mut self, events: &[Event]) {
        for e in events {
            match *e {
                Event::ParcelDelayed {
                    root: WaitCause::Fault(f) | WaitCause::CellBlocked(f),
                    ..
                } => *self.fault.entry(f).or_default() += 1,
                Event::RobotWaited { root, .. } => match root {
                    WaitCause::StationBusy(s) => *self.station_busy.entry(s).or_default() += 1,
                    WaitCause::ChuteUnavailable(c) => *self.chute_busy.entry(c).or_default() += 1,
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

/// Enabled faults of `cfg` that started before `window_end` and were still
/// active at or after `window_start` (ticks), the most blamed first if there
/// are more than [`MAX_CANDIDATES`]; returned in fault-id order, plus the
/// ones left out.
#[must_use]
pub fn candidates(
    cfg: &ScenarioConfig,
    window_start: Tick,
    window_end: Tick,
    blame: &Blame,
) -> (Vec<Candidate>, Vec<Candidate>) {
    let mut all: Vec<Candidate> = cfg
        .enabled_faults()
        .filter(|(_, f)| f.at < window_end && f.end() > window_start)
        .map(|(id, f)| Candidate {
            fault: id,
            spec: f.clone(),
            blame: blame.fault.get(&id).copied().unwrap_or(0),
        })
        .collect();
    // Most blamed first, ties by id; keep the top ones; back to id order.
    all.sort_by(|a, b| b.blame.cmp(&a.blame).then(a.fault.cmp(&b.fault)));
    let dropped = all.split_off(all.len().min(MAX_CANDIDATES));
    all.sort_by_key(|c| c.fault);
    (all, dropped)
}

/// The healthy chute nearest to `chute` (Manhattan distance between holes,
/// ties by id), skipping `avoid`.
fn nearest_chute(sim: &Sim, chute: ChuteId, avoid: &[ChuteId]) -> Option<ChuteId> {
    let l = sim.layout();
    let at = l.grid.pos(l.chutes[chute.index()]);
    (0..l.chutes.len())
        .filter_map(|c| u16::try_from(c).ok().map(ChuteId))
        .filter(|c| *c != chute && !avoid.contains(c))
        .min_by_key(|c| (l.grid.pos(l.chutes[c.index()]).manhattan(at), *c))
}

/// Remap every destination `chute` serves (in `sim`'s current mapping) to
/// its nearest healthy chute.
fn remap_all(sim: &Sim, chute: ChuteId, avoid: &[ChuteId]) -> Option<(ChuteId, Vec<Intervention>)> {
    let to = nearest_chute(sim, chute, avoid)?;
    let remaps: Vec<Intervention> = sim
        .world()
        .destination_chute
        .iter()
        .enumerate()
        .filter(|&(_, &c)| c == chute)
        .filter_map(|(d, _)| u16::try_from(d).ok())
        .map(|d| Intervention::RemapDestination {
            destination: DestinationId(d),
            chute: to,
        })
        .collect();
    (!remaps.is_empty()).then_some((to, remaps))
}

/// Candidate fixes for the incident, plus hotspots and their relax tests.
/// `sim` is the actual run at the first candidate's onset (`onset`).
#[must_use]
pub fn fixes_and_hotspots(
    sim: &Sim,
    onset: Tick,
    window_start: Tick,
    candidates: &[Candidate],
    blame: &Blame,
) -> (Vec<Fix>, Vec<Hotspot>) {
    let cfg = sim.config();
    let mut fixes = Vec::new();
    let jammed: Vec<ChuteId> = candidates
        .iter()
        .filter_map(|c| match c.spec.kind {
            FaultKind::ChuteJam { chute } => Some(chute),
            _ => None,
        })
        .collect();

    let failures: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| matches!(c.spec.kind, FaultKind::RobotFailure { .. }))
        .collect();
    let clear = cfg.recovery.clear_after_ticks;
    let target = TICKS_PER_MINUTE;
    if let Some(first) = failures.iter().map(|c| c.spec.at).min()
        && clear > target
    {
        fixes.push(Fix {
            label: format!(
                "clear failed robots off loading points within 1 min (now {})",
                minutes(clear)
            ),
            cost: "floor procedure".to_owned(),
            at: first,
            interventions: vec![Intervention::ClearAfter { ticks: target }],
            hotspot: false,
            from_chute: None,
        });
    }
    for c in candidates {
        if let FaultKind::ChuteJam { chute } = c.spec.kind
            && let Some((to, remaps)) = remap_all(sim, chute, &jammed)
        {
            fixes.push(Fix {
                label: format!(
                    "send chute {chute}'s {} to {to} from the jam on (never undone)",
                    if remaps.len() == 1 {
                        "destination".to_owned()
                    } else {
                        format!("{} destinations", remaps.len())
                    }
                ),
                cost: "sort-plan change".to_owned(),
                at: c.spec.at,
                interventions: remaps,
                hotspot: false,
                from_chute: Some(chute),
            });
        }
    }
    let robots = u32::try_from(sim.world().robots.len()).unwrap_or(u32::MAX);
    let spare = (robots / 20).max(1);
    fixes.push(Fix {
        label: format!("release {spare} spare robots at the first fault"),
        cost: format!("{spare} robots"),
        at: onset,
        interventions: vec![Intervention::AddRobots {
            robots: spare,
            at: onset,
        }],
        hotspot: false,
        from_chute: None,
    });
    let cap = cfg.fleet.max_inbound_per_station;
    fixes.push(Fix {
        label: format!("allow {} robots inbound per station (now {cap})", cap + 2),
        cost: "dispatch setting".to_owned(),
        at: onset,
        interventions: vec![Intervention::SetStationCap {
            station: None,
            cap: cap + 2,
        }],
        hotspot: false,
        from_chute: None,
    });

    let mut hotspots = Vec::new();
    for (b, s) in top(&blame.station_busy) {
        hotspots.push(Hotspot {
            label: format!("robots queuing at busy station {s} (loading time; no fix modelled)"),
            blame: b,
            fix: None,
        });
    }
    for (b, c) in top(&blame.chute_busy) {
        if jammed.contains(&c) {
            continue;
        }
        let fix = remap_all(sim, c, &jammed).map(|(to, remaps)| {
            fixes.push(Fix {
                label: format!("relax chute {c}: send its destinations to {to}"),
                cost: "sort-plan change".to_owned(),
                at: window_start,
                interventions: remaps,
                hotspot: true,
                from_chute: Some(c),
            });
            fixes.len() - 1
        });
        hotspots.push(Hotspot {
            label: format!("robots waiting on chute {c} bag swaps"),
            blame: b,
            fix,
        });
    }
    hotspots.sort_by(|a, b| b.blame.cmp(&a.blame).then(a.label.cmp(&b.label)));
    (fixes, hotspots)
}

/// The two largest entries of `m`, largest first (ties by key).
fn top<K: Copy + Ord>(m: &BTreeMap<K, u64>) -> Vec<(u64, K)> {
    let mut v: Vec<(u64, K)> = m.iter().map(|(k, &b)| (b, *k)).collect();
    v.retain(|&(b, _)| b >= MIN_HOTSPOT_BLAME);
    v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    v.truncate(2);
    v
}

/// `"5 min"`, `"30 s"`.
#[must_use]
pub fn minutes(ticks: Tick) -> String {
    if ticks.is_multiple_of(TICKS_PER_MINUTE) {
        format!("{} min", ticks / TICKS_PER_MINUTE)
    } else {
        format!("{} s", ticks / 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use switchyard_core::ids::RobotId;
    use switchyard_core::presets::SMALL_TOML;

    fn fault(at: Tick, kind: FaultKind) -> FaultSpec {
        FaultSpec {
            at,
            duration: 600,
            kind,
        }
    }

    #[test]
    fn candidates_are_faults_before_the_window_end_capped_by_blame() {
        let mut cfg = ScenarioConfig::from_toml_str(SMALL_TOML).unwrap();
        for r in 0..8 {
            cfg.faults.push(fault(
                100 * r,
                FaultKind::RobotFailure { robot: RobotId(r) },
            ));
        }
        cfg.faults
            .push(fault(5_000, FaultKind::ChuteJam { chute: ChuteId(1) }));
        cfg = cfg.without_fault(FaultId(7));
        let mut blame = Blame::default();
        for (f, b) in [(0, 5), (1, 50), (2, 40), (3, 30), (4, 20), (5, 10), (6, 1)] {
            blame.fault.insert(FaultId(f), b);
        }
        let (kept, dropped) = candidates(&cfg, 0, 1_000, &blame);
        let ids: Vec<u32> = kept.iter().map(|c| c.fault.0).collect();
        // F7 is disabled, F8 starts after the window; F6 (blame 1) is the
        // least blamed of seven.
        assert_eq!(ids, vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].fault, FaultId(6));
    }

    /// #21(c): a fault that ended before the window opened cannot have caused
    /// the incident (edge-two-incidents: a 2-min jam 40 min earlier).
    #[test]
    fn a_fault_over_before_the_window_opened_is_not_a_candidate() {
        let mut cfg = ScenarioConfig::from_toml_str(SMALL_TOML).unwrap();
        // F0 is over at 600; F1 is still active at 1_000 (600..1_200); F2
        // ends exactly when the window opens (end() is the first inactive tick).
        cfg.faults
            .push(fault(0, FaultKind::ChuteJam { chute: ChuteId(1) }));
        cfg.faults
            .push(fault(600, FaultKind::ChuteJam { chute: ChuteId(1) }));
        cfg.faults
            .push(fault(400, FaultKind::ChuteJam { chute: ChuteId(1) }));
        let (kept, dropped) = candidates(&cfg, 1_000, 2_000, &Blame::default());
        let ids: Vec<u32> = kept.iter().map(|c| c.fault.0).collect();
        assert_eq!(ids, vec![1]);
        assert!(dropped.is_empty());
    }

    #[test]
    fn minutes_read_plainly() {
        assert_eq!(minutes(600), "5 min");
        assert_eq!(minutes(61), "30 s");
    }
}
