//! The one-glance summary of a post-mortem: the incident window, the loss,
//! the cause that stands out of the noise and the best fix.

use switchyard_core::faults::FaultKind;

use super::report::thousands;
use super::{PostMortem, significant};
use crate::metrics::TICKS_PER_MINUTE;
use crate::resilience::ticks_to_minutes;

/// The one-glance summary of a post-mortem. Every number is the engine's.
#[derive(Clone, Debug, PartialEq)]
pub struct Headline {
    /// Incident window start (minute of the run).
    pub start_min: usize,
    /// Window end (minute of the run).
    pub end_min: usize,
    /// False when the incident was still open when the run ended.
    pub settled: bool,
    /// The worst throughput dip against the twin, in %.
    pub max_dip_pct: f64,
    /// The main cause, if the loss is attributable and a cause stands out of
    /// the noise.
    pub cause: Option<CauseHeadline>,
    /// Short label of the second cause when it ties the first.
    pub tie_with: Option<String>,
    /// Real history's loss, whole parcel-minutes.
    pub loss_parcel_min: i64,
    /// Parcels more than the SLA late at the horizon, real history.
    pub late_parcels: u32,
    /// The best non-hotspot fix with a gain above zero.
    pub fix: Option<FixHeadline>,
    /// Replays behind the post-mortem.
    pub replays: usize,
    /// Histories every replay ran on.
    pub histories: usize,
}

/// The main cause in a few words.
#[derive(Clone, Debug, PartialEq)]
pub struct CauseHeadline {
    /// Short label, e.g. `C289 jam`.
    pub label: String,
    /// The fault's id, e.g. `F0`.
    pub fault: String,
    /// Share of the median loss explained, in %.
    pub share_pct: f64,
    /// The share's noise band, in %.
    pub band_pct: f64,
}

/// The best fix in a few words.
#[derive(Clone, Debug, PartialEq)]
pub struct FixHeadline {
    /// The fix's label, verbatim.
    pub label: String,
    /// Recovered delay, whole parcel-minutes.
    pub recovered_parcel_min: i64,
    /// Share of the median loss recovered, in %.
    pub recovery_pct: f64,
    /// True when the gain is within its noise band.
    pub within_noise: bool,
}

/// A fault kind in a short label.
#[must_use]
pub fn short_label(kind: &FaultKind) -> String {
    match kind {
        FaultKind::RobotFailure { robot } => format!("{robot} failure"),
        FaultKind::ChuteJam { chute } => format!("{chute} jam"),
        FaultKind::AisleBlock { .. } => "aisle blockage".to_owned(),
        FaultKind::InductionOutage { station } => format!("{station} outage"),
        FaultKind::VolumeSurge { .. } => "volume surge".to_owned(),
        FaultKind::ScannerDrift { station, .. } => format!("{station} scanner drift"),
    }
}

/// Parcel-ticks as whole parcel-minutes.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn parcel_minutes(parcel_ticks: i64) -> i64 {
    ticks_to_minutes(parcel_ticks as f64).round() as i64
}

impl PostMortem {
    /// The one-glance summary; `None` when no incident was detected.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn headline(&self) -> Option<Headline> {
        let inc = self.incident?;
        let att = &self.attribution;
        let total = att.total();
        // Same floor and ranking as `render()` in report.rs.
        let loss = self.loss();
        let floor = (loss / 20).max(10 * i64::from(TICKS_PER_MINUTE));
        let attributable = total > floor;

        let mut order: Vec<usize> = (0..self.candidates.len()).collect();
        order.sort_by(|&a, &b| {
            att.shapley_scaled[b]
                .cmp(&att.shapley_scaled[a])
                .then(a.cmp(&b))
        });
        let ranked: Vec<usize> = order
            .into_iter()
            .filter(|&i| att.shapley(i).abs() > self.share_band(i))
            .collect();

        let cause = if attributable {
            ranked.first().map(|&i| {
                let c = &self.candidates[i];
                CauseHeadline {
                    label: short_label(&c.spec.kind),
                    fault: c.fault.to_string(),
                    share_pct: 100.0 * att.shapley(i) / total as f64,
                    band_pct: 100.0 * self.share_band(i) / total as f64,
                }
            })
        } else {
            None
        };
        let tie_with = cause.as_ref().and_then(|_| {
            let first = *ranked.first()?;
            let second = *ranked.get(1)?;
            let diff = att.shapley(first) - att.shapley(second);
            let band = self.share_band(first).max(self.share_band(second));
            (!significant(diff, band)).then(|| short_label(&self.candidates[second].spec.kind))
        });

        let mut fix_rows: Vec<(usize, i64, f64)> = (0..self.fixes.len())
            .filter(|&i| !self.fixes[i].hotspot)
            .map(|i| {
                let (g, b) = self.fix_stats(i);
                (i, g, b)
            })
            .collect();
        fix_rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let fix = fix_rows
            .into_iter()
            .find(|&(_, g, _)| g > 0)
            .map(|(i, g, b)| FixHeadline {
                label: self.fixes[i].label.clone(),
                recovered_parcel_min: parcel_minutes(g),
                recovery_pct: 100.0 * g as f64 / loss.max(1) as f64,
                within_noise: !significant(g as f64, b),
            });

        Some(Headline {
            start_min: inc.start,
            end_min: inc.end,
            settled: inc.settled,
            max_dip_pct: self.score.max_dip_pct,
            cause,
            tie_with,
            loss_parcel_min: parcel_minutes(self.real_loss()),
            late_parcels: self.real_late(),
            fix,
            replays: self.replays.len() * self.histories(),
            histories: self.histories(),
        })
    }
}

/// `hh:mm` of minute `m` of the run.
fn hhmm(m: usize) -> String {
    format!("{:02}:{:02}", m / 60, m % 60)
}

impl Headline {
    /// The report's first block, in plain sentences (design/v2.md, component
    /// 1): an eyebrow, the cause and the fix in a sentence each, the three
    /// numbers, and one line saying how we know. The viewer's headline card
    /// is laid out from exactly these four paragraphs, so their order and
    /// shape are a contract (`viewer/src/ui/reportSections.ts`).
    #[must_use]
    pub fn markdown(&self) -> String {
        let open = if self.settled {
            String::new()
        } else {
            ", still open when the run ended".to_owned()
        };
        let eyebrow = format!(
            "**Incident {}\u{2013}{} \u{b7} up to {:.0} % below the fault-free twin{open}**",
            hhmm(self.start_min),
            hhmm(self.end_min),
            self.max_dip_pct,
        );
        let cause = match (&self.cause, &self.tie_with) {
            (Some(c), Some(second)) => format!(
                "The {} and the {second} each caused about {:.0} %; they are too close to rank.",
                c.label, c.share_pct
            ),
            (Some(c), None) => format!("The {} caused {:.0} % of the delay.", c.label, c.share_pct),
            (None, _) => "No single cause stands out of the noise.".to_owned(),
        };
        // A fix inside the noise is not worth a sentence (design: dropped).
        let fix = self.fix.as_ref().filter(|f| !f.within_noise);
        let mut sentences = cause;
        if let Some(f) = fix {
            sentences.push_str(&format!("\n{}", fix_sentence(&f.label, f.recovery_pct)));
        }
        let mut stats = vec![
            format!(
                "**{}** parcel-min of delay",
                thousands(self.loss_parcel_min)
            ),
            format!(
                "**{}** parcels late",
                thousands(i64::from(self.late_parcels))
            ),
        ];
        if let Some(f) = fix {
            stats.push(format!("**{:.0} %** recoverable", f.recovery_pct));
        }
        let proof = if self.histories > 1 {
            format!(
                "*How we know: proven by {} replays of this shift \u{b7} error bars from {} versions of the day.*",
                self.replays, self.histories
            )
        } else {
            format!(
                "*How we know: proven by {} replays of this shift \u{b7} one version of the day, so no error bars.*",
                self.replays
            )
        };
        format!(
            "{eyebrow}\n\n{sentences}\n\n{}\n\n{proof}",
            stats.join(" \u{b7} ")
        )
    }
}

/// "The best fix is to send ...: it would have saved 83 % of the delay."
fn fix_sentence(label: &str, pct: f64) -> String {
    format!("The best fix is to {label}: it would have saved {pct:.0} % of the delay.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::postmortem::{Options, run};
    use crate::resilience::ticks_to_minutes;
    use switchyard_core::config::ScenarioConfig;
    use switchyard_core::faults::{FaultKind, FaultSpec};
    use switchyard_core::grid::Pos;
    use switchyard_core::ids::{ChuteId, RobotId, StationId};
    use switchyard_core::presets::SMALL_TOML;

    /// A small, deterministic incident: the busiest chute jams for 15 min
    /// while a robot is down for 5; 2 histories.
    fn small_incident() -> PostMortem {
        let mut cfg = ScenarioConfig::from_toml_str(SMALL_TOML).unwrap();
        cfg.name = "small-jam".to_owned();
        let busy = {
            let sim = switchyard_core::sim::Sim::new(&cfg).unwrap();
            sim.world().destination_chute[0]
        };
        cfg.faults = vec![
            FaultSpec {
                at: 1_200,
                duration: 1_800,
                kind: FaultKind::ChuteJam { chute: busy },
            },
            FaultSpec {
                at: 1_320,
                duration: 600,
                kind: FaultKind::RobotFailure { robot: RobotId(7) },
            },
        ];
        assert_ne!(busy, ChuteId(u16::MAX));
        run(
            &cfg,
            6_000,
            &Options {
                dither: 2,
                ..Options::default()
            },
        )
        .unwrap()
    }

    #[test]
    fn short_labels_name_the_broken_thing() {
        let cases = [
            (
                FaultKind::RobotFailure {
                    robot: RobotId(119),
                },
                "R119 failure",
            ),
            (
                FaultKind::ChuteJam {
                    chute: ChuteId(289),
                },
                "C289 jam",
            ),
            (
                FaultKind::AisleBlock {
                    cells: vec![Pos::new(22, 3)],
                },
                "aisle blockage",
            ),
            (
                FaultKind::InductionOutage {
                    station: StationId(3),
                },
                "S3 outage",
            ),
            (
                FaultKind::VolumeSurge {
                    station: Some(StationId(3)),
                    factor_pct: 150,
                },
                "volume surge",
            ),
            (
                FaultKind::VolumeSurge {
                    station: None,
                    factor_pct: 150,
                },
                "volume surge",
            ),
            (
                FaultKind::ScannerDrift {
                    station: StationId(3),
                    error_rate_ppm: 10_000,
                },
                "S3 scanner drift",
            ),
        ];
        for (kind, want) in cases {
            assert_eq!(short_label(&kind), want);
        }
    }

    #[test]
    fn headline_reads_the_incident_and_the_real_history() {
        let pm = small_incident();
        let h = pm.headline().expect("an incident");
        let inc = pm.incident.unwrap();
        assert_eq!(
            (h.start_min, h.end_min, h.settled),
            (inc.start, inc.end, inc.settled)
        );
        assert_eq!(h.max_dip_pct, pm.score.max_dip_pct);
        assert!(h.cause.is_some(), "{h:?}");
        assert_eq!(
            h.loss_parcel_min,
            ticks_to_minutes(pm.real_loss() as f64).round() as i64
        );
        assert_eq!(h.late_parcels, pm.real_late());
        assert_eq!(h.replays, pm.replays.len() * pm.histories());
        assert_eq!(h.histories, pm.histories());
    }

    #[test]
    fn a_run_without_an_incident_has_no_headline() {
        let mut pm = small_incident();
        pm.incident = None;
        assert!(pm.headline().is_none());
    }

    fn sample() -> Headline {
        Headline {
            start_min: 30,
            end_min: 52,
            settled: true,
            max_dip_pct: 47.3,
            cause: Some(CauseHeadline {
                label: "C289 jam".to_owned(),
                fault: "F0".to_owned(),
                share_pct: 69.86,
                band_pct: 3.15,
            }),
            tie_with: None,
            loss_parcel_min: 6264,
            late_parcels: 110,
            fix: Some(FixHeadline {
                label: "send chute C289's destination to C269".to_owned(),
                recovered_parcel_min: 5162,
                recovery_pct: 83.39,
                within_noise: false,
            }),
            replays: 96,
            histories: 8,
        }
    }

    #[test]
    fn the_headline_block_reads_as_plain_sentences() {
        assert_eq!(
            sample().markdown(),
            "**Incident 00:30\u{2013}00:52 \u{b7} up to 47 % below the fault-free twin**\n\n\
             The C289 jam caused 70 % of the delay.\n\
             The best fix is to send chute C289's destination to C269: it would have saved 83 % of the delay.\n\n\
             **6,264** parcel-min of delay \u{b7} **110** parcels late \u{b7} **83 %** recoverable\n\n\
             *How we know: proven by 96 replays of this shift \u{b7} error bars from 8 versions of the day.*"
        );
    }

    #[test]
    fn a_tie_is_said_not_ranked() {
        let mut h = sample();
        h.tie_with = Some("R119 failure".to_owned());
        h.cause.as_mut().unwrap().share_pct = 15.2;
        let md = h.markdown();
        assert!(
            md.contains(
                "The C289 jam and the R119 failure each caused about 15 %; they are too close to rank."
            ),
            "{md}"
        );
    }

    #[test]
    fn a_fix_within_noise_is_not_promised() {
        let mut h = sample();
        h.fix.as_mut().unwrap().within_noise = true;
        let md = h.markdown();
        assert!(
            !md.contains("best fix") && !md.contains("recoverable"),
            "{md}"
        );
        // The card still has its three paragraphs after the eyebrow.
        assert_eq!(md.split("\n\n").count(), 4, "{md}");
    }

    #[test]
    fn no_attributable_cause_and_an_open_window_are_said() {
        let mut h = sample();
        h.cause = None;
        h.settled = false;
        let md = h.markdown();
        assert!(
            md.contains("No single cause stands out of the noise."),
            "{md}"
        );
        assert!(md.contains("still open when the run ended"), "{md}");
    }

    #[test]
    fn one_history_has_no_error_bars() {
        let mut h = sample();
        h.histories = 1;
        assert!(h.markdown().contains("so no error bars"));
    }
}
