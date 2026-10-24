//! The post-mortem as Google-SRE-format Markdown (research 2026-10-20 §3.5
//! and §7): the numbers first, causes as blameless system conditions, what
//! would have helped, then method and the replay index. Each section says in
//! plain words how its numbers were obtained. The replay-id citations (`[A]`
//! the actual run, `[T]` its fault-free twin, `[R<mask>]` a lattice replay,
//! `[X<n>]` a fix replay) live only in the appendix replay index.

use std::fmt::Write as _;

use switchyard_core::Tick;
use switchyard_core::interventions::Intervention;

use super::candidates::{MAX_CANDIDATES, minutes};
use super::headline::short_label;
use super::shapley::Interplay;
use super::{PostMortem, describe_kind, significant};
use crate::metrics::{TICKS_PER_MINUTE, minute_tick};
use crate::resilience::ticks_to_minutes;

/// Largest CUSUM peak in the fault-free perturbed-path calibration (the
/// "Calibration of `h`" note in detect.rs; ADR 0006 amendment). The report
/// quotes `h` as a multiple of it, derived from the live parameter so the
/// sentence cannot go stale when `h` changes.
const NULL_PEAK_MAX: f64 = 1.0;

/// `hh:mm` of simulated time.
fn clock(tick: Tick) -> String {
    let m = tick / TICKS_PER_MINUTE;
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// `hh:mm` of minute `m`.
fn clock_min(m: usize) -> String {
    clock(minute_tick(m))
}

/// Parcel-ticks as whole parcel-minutes (never "-0").
fn pminf(parcel_ticks: f64) -> String {
    let v = ticks_to_minutes(parcel_ticks).round();
    format!("{:.0}", if v == 0.0 { 0.0 } else { v })
}

/// [`pminf`] for an integer count.
#[allow(clippy::cast_precision_loss)]
fn pmin(parcel_ticks: i64) -> String {
    pminf(parcel_ticks as f64)
}

/// How the scoring horizon relates to the incident window, from the horizon
/// actually used (it is clamped to the run end, so it can be closer than the
/// configured margin) (#21).
fn horizon_note(pm_: &PostMortem, window_end_min: usize) -> String {
    let actual = (pm_.horizon / TICKS_PER_MINUTE) as usize;
    let gap = actual.saturating_sub(window_end_min);
    let margin = pm_.options.cusum.margin_minutes;
    if gap == 0 {
        "the window end, where the run ends".to_owned()
    } else if gap >= margin {
        format!("window end + {margin} min")
    } else {
        format!("window end + {gap} min, where the run ends")
    }
}

/// Start a folded section: GitHub and the viewer's panel both read this
/// `<details>` block. The `## ` heading stays inside it (graders and plain
/// Markdown readers still find it); the row shows the title and a teaser.
fn open_section(o: &mut String, title: &str, teaser: &str) {
    let _ = writeln!(
        o,
        "\n<details>\n<summary><b>{title}</b> — {teaser}</summary>\n"
    );
}

/// End a folded section.
fn close_section(o: &mut String) {
    let _ = writeln!(o, "\n</details>");
}

/// A whole number with thousands separators: `6264` -> `6,264`.
pub(super) fn thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// The first sections (title, headline, header, summary): what the CLI prints.
#[must_use]
pub fn summary(pm_: &PostMortem) -> String {
    let md = render(pm_);
    md.split("\n<details>")
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_owned()
}

/// Render `pm` as Markdown.
#[must_use]
#[allow(clippy::too_many_lines, clippy::cast_precision_loss)]
pub fn render(pm_: &PostMortem) -> String {
    let mut o = String::new();
    let cfg = &pm_.config;
    let histories = pm_.histories();
    let _ = writeln!(o, "# Post-mortem: {} (seed {})\n", cfg.name, cfg.seed);
    let Some(inc) = pm_.incident else {
        let peak = pm_.cusum_peak;
        let _ = writeln!(
            o,
            "**Status:** no incident. Throughput never fell far enough below the fault-free \
             twin for long enough to raise an alarm (CUSUM peak {peak:.1} of h = {}).",
            pm_.options.cusum.h
        );
        return o;
    };

    if let Some(h) = pm_.headline() {
        let _ = writeln!(o, "{}\n", h.markdown());
    }

    let n = pm_.candidates.len();
    let att = &pm_.attribution;
    // Facts (headline, Impact) come from the real history; `loss` is the
    // median game the attribution divides (#20).
    let loss = pm_.loss();
    let real_loss = pm_.real_loss();
    let loss_band = pm_.loss_band();
    let mask_bands = pm_.mask_bands();
    let total = att.total();
    let all = att.all();
    let floor = (loss / 20).max(10 * i64::from(TICKS_PER_MINUTE));
    let attributable = total > floor;
    // A share is always printed with its noise band (#16): a bare percentage
    // reads as more certain than the replays can support.
    let share = |x: f64, band: f64| {
        if attributable {
            format!(
                "{:.0} % ± {:.0} %",
                100.0 * x / total as f64,
                100.0 * band / total as f64
            )
        } else {
            "-".to_owned()
        }
    };
    // Shapley values use every replay of the lattice, so they cite the whole
    // range, not just the single-removal replay R(1<<i) (#16).
    let lattice = format!("R0–R{all}");
    let n_replays = pm_.replays.len() * histories;
    // Provenance for the Method section; the top of the report stays plain.
    let generated = format!(
        "**Generated by** switchyard-analysis {} · {n_replays} replays ({} changes × {histories} \
         {}) · loss scored at {} ({}) · **noise band** ± {} parcel-min",
        crate::VERSION,
        pm_.replays.len(),
        plural(histories, "history", "histories"),
        clock(pm_.horizon),
        horizon_note(pm_, inc.end),
        pminf(loss_band),
    );

    // Order causes by Shapley value.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        att.shapley_scaled[b]
            .cmp(&att.shapley_scaled[a])
            .then(a.cmp(&b))
    });
    // Causes whose share is within one noise band of zero are not ranked:
    // their order would be an artefact of the dither (#16). They are listed
    // once, below the table.
    let (order, in_noise): (Vec<usize>, Vec<usize>) = order
        .into_iter()
        .partition(|&i| att.shapley(i).abs() > pm_.share_band(i));
    let label = |i: usize| {
        let c = &pm_.candidates[i];
        format!("{} ({})", describe_kind(&c.spec.kind), c.fault)
    };
    let fix_rows: Vec<(usize, i64, f64)> = {
        let mut v: Vec<(usize, i64, f64)> = (0..pm_.fixes.len())
            .filter(|&i| !pm_.fixes[i].hotspot)
            .map(|i| {
                let (g, b) = pm_.fix_stats(i);
                (i, g, b)
            })
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    };

    // ---- Summary
    let _ = writeln!(o, "## Summary\n");
    let _ = writeln!(
        o,
        "From {} sorted throughput fell up to {:.0} % below the fault-free twin. By {} the \
         incident had cost **{} parcel-minutes** of delay, and {} parcels were more than {} min \
         late.",
        clock_min(inc.start),
        pm_.score.max_dip_pct,
        clock(pm_.horizon),
        pmin(real_loss),
        pm_.real_late(),
        pm_.options.sla_extra_minutes,
    );
    if n == 0 {
        let _ = writeln!(
            o,
            "\nNo scheduled fault was active before the window closed: the loss is intrinsic congestion."
        );
    } else if attributable {
        let parts: Vec<String> = order
            .iter()
            .take(3)
            .map(|&i| {
                format!(
                    "{} explains {} ({} parcel-min)",
                    label(i),
                    share(att.shapley(i), pm_.share_band(i)),
                    pminf(att.shapley(i)),
                )
            })
            .collect();
        let _ = write!(o, "{}", cause_sentence(&parts));
        // Shares divide the median game, so the remainder is of the median
        // loss; name it when it is not the headline figure, or the reader
        // adds the numbers up and gets a contradiction. A remainder inside
        // the noise band is not a fact, so it is not reported at all.
        let unexplained = loss - total;
        if significant(unexplained as f64, loss_band) {
            let of_median = if loss == real_loss {
                String::new()
            } else {
                format!(
                    " of the {} parcel-min median loss across the {histories} histories (the figure \
                     the shares divide)",
                    pmin(loss)
                )
            };
            if unexplained < 0 {
                let _ = writeln!(
                    o,
                    " In other words, removing these causes recovers {} parcel-min more than the \
                     whole loss.",
                    pmin(unexplained.abs())
                );
            } else {
                let _ = writeln!(
                    o,
                    " {} parcel-min ({}){of_median} is not explained by removing these causes.",
                    pmin(unexplained),
                    if loss > 0 {
                        format!("{:.0} %", 100.0 * unexplained as f64 / loss as f64)
                    } else {
                        "-".to_owned()
                    }
                );
            }
        } else {
            let _ = writeln!(o);
        }
    } else {
        let _ = writeln!(
            o,
            "\nRemoving every candidate cause recovers only {} parcel-min, within the noise: no \
             attributable loss.",
            pmin(total)
        );
    }
    if let Some(&(i, g, b)) = fix_rows.first()
        && g > 0
        && significant(g as f64, b)
    {
        let _ = writeln!(
            o,
            "\nBest fix: **{}** would have recovered {} parcel-min ({:.0} % of the loss).",
            pm_.fixes[i].label,
            pmin(g),
            100.0 * g as f64 / loss.max(1) as f64
        );
    }
    let _ = writeln!(
        o,
        "\nHow we know: the real shift and its fault-free twin, plus replays with each cause \
         removed ({lattice}); see the replay index at the end."
    );

    // ---- Impact
    let s = &pm_.score;
    open_section(
        &mut o,
        "Impact",
        &format!(
            "{} parcels late · peak backlog {} (twin {})",
            pm_.real_late(),
            s.peak_backlog,
            s.twin_peak_backlog
        ),
    );
    let _ = writeln!(o, "## Impact\n");
    let _ = writeln!(o, "| Measure | Value |\n|---|---|");
    let _ = writeln!(
        o,
        "| Delay at {} vs the fault-free twin | {} parcel-min (± {}) |",
        clock(pm_.horizon),
        pmin(real_loss),
        pminf(loss_band)
    );
    // Two late counts, each labelled with its own cut-off: the headline's is
    // at the horizon; the scorecard's covers the whole run, which can go on
    // past the horizon and so count more parcels.
    let _ = writeln!(
        o,
        "| Parcels more than {} min late at {} | {} |",
        pm_.options.sla_extra_minutes,
        clock(pm_.horizon),
        pm_.real_late()
    );
    let _ = writeln!(
        o,
        "| Parcels more than {} min late (whole run, to {}) | {} |",
        pm_.options.sla_extra_minutes,
        clock(pm_.ticks),
        s.delay.late_parcels
    );
    let _ = writeln!(
        o,
        "| Peak backlog | {} parcels (twin {}) |",
        s.peak_backlog, s.twin_peak_backlog
    );
    let _ = writeln!(
        o,
        "| Worst minute / time to recover | {:.0} % below the twin / {} |",
        s.max_dip_pct,
        s.time_to_recover_min
            .map_or_else(|| "not within the run".to_owned(), |m| format!("{m} min"))
    );
    let _ = writeln!(
        o,
        "| Stations / chutes out of action at once | {} / {} |",
        s.stations_lost, s.chutes_lost
    );
    let _ = writeln!(
        o,
        "| Parcels left unsorted at the end of the run | {} |",
        s.undelivered
    );
    let _ = writeln!(
        o,
        "\nHow we know: the real shift compared with its fault-free twin up to {}.",
        clock(pm_.horizon)
    );
    close_section(&mut o);

    // ---- Root causes
    if n > 0 {
        open_section(
            &mut o,
            "Root causes",
            &match order.first() {
                Some(&i) => format!(
                    "{n} {} tested · {} first",
                    plural(n, "cause", "causes"),
                    short_label(&pm_.candidates[i].spec.kind)
                ),
                None => format!(
                    "{n} {} tested · none stands out of the noise",
                    plural(n, "cause", "causes")
                ),
            },
        );
        let _ = writeln!(o, "## Root causes (system conditions)\n");
        let _ = writeln!(
            o,
            "Each cause was removed in every combination and the shift replayed (exact Shapley \
             values over {} subsets). *Alone* = recovered by removing only this cause; \
             *last* = what it adds once every other cause is gone.\n",
            1usize << n
        );
        let _ = writeln!(
            o,
            "| # | Condition | Shapley (parcel-min) | Share | Alone | Last | Needed to explain it? |\n\
             |---|---|---|---|---|---|---|"
        );
        for (rank, &i) in order.iter().enumerate() {
            let phi = att.shapley(i);
            let band_i = pm_.share_band(i);
            let tie = rank > 0 && {
                let prev = order[rank - 1];
                !significant(att.shapley(prev) - phi, band_i.max(pm_.share_band(prev)))
            };
            let c = att.contingency_banded(i, pm_.options.tau_pct, &mask_bands);
            let need = if c.sure == c.possible {
                need_text(c.sure)
            } else {
                format!(
                    "{} (possibly: {})",
                    need_text(c.sure),
                    need_text(c.possible)
                )
            };
            let _ = writeln!(
                o,
                "| {} | {}{} | {} ± {} | {} | {} | {} | {} |",
                rank + 1,
                label(i),
                if tie {
                    " (tie with the row above: within noise)"
                } else {
                    ""
                },
                pminf(phi),
                pminf(band_i),
                share(phi, band_i),
                pmin(att.alone(i)),
                pmin(att.leave_one_out(i)),
                need,
            );
            if phi < 0.0 {
                let _ = writeln!(
                    o,
                    "|  | ↳ removing this cause on its own makes the shift *worse*: it was holding \
                     back another cause's damage |  |  |  |  |  |"
                );
            }
        }
        let _ = writeln!(
            o,
            "\nHow we know: shares use every replay of the lattice ({lattice}); Alone and Last each come from one replay."
        );
        if !in_noise.is_empty() {
            let names: Vec<String> = in_noise
                .iter()
                .map(|&i| {
                    format!(
                        "{} ({} ± {} parcel-min)",
                        label(i),
                        pminf(att.shapley(i)),
                        pminf(pm_.share_band(i))
                    )
                })
                .collect();
            let _ = writeln!(
                o,
                "\nWithin noise (share no larger than its band, so not ranked): {}.",
                names.join("; ")
            );
        }
        // Interactions beyond the noise.
        let noise = (0..n).map(|i| pm_.share_band(i)).fold(0.0, f64::max);
        let mut pairs: Vec<(usize, usize, f64)> = (0..n)
            .flat_map(|i| (i + 1..n).map(move |j| (i, j)))
            .map(|(i, j)| (i, j, att.interaction(i, j)))
            .filter(|&(_, _, x)| significant(x, noise) && x.abs() >= 1.0)
            .collect();
        pairs.sort_by(|a, b| b.2.abs().total_cmp(&a.2.abs()));
        let _ = writeln!(o);
        let any_interactions = !pairs.is_empty();
        if !any_interactions {
            let _ = writeln!(
                o,
                "Interactions: none beyond the noise; the causes add up independently."
            );
        }
        for (i, j, x) in pairs.into_iter().take(3) {
            let kind = interplay_text(Interplay::of(x), &pminf(x.abs()));
            let _ = writeln!(o, "Interaction: {} and {} {kind}.", label(i), label(j));
        }
        if any_interactions {
            let _ = writeln!(
                o,
                "\nHow we know: interactions come from every replay ({lattice}), not just the three named."
            );
        }
        let sets = att.minimal_sufficient_banded(pm_.options.tau_pct, &mask_bands);
        let set_names = |masks: &[usize]| -> String {
            masks
                .iter()
                .map(|&m| {
                    let parts: Vec<String> = (0..n)
                        .filter(|i| m & (1 << i) != 0)
                        .map(|i| pm_.candidates[i].fault.to_string())
                        .collect();
                    format!("{{{}}}", parts.join(", "))
                })
                .collect::<Vec<_>>()
                .join(" or ")
        };
        if !sets.sure.is_empty() {
            let _ = writeln!(
                o,
                "\nSmallest set of causes whose removal recovers at least {} % of the recoverable loss (each judged against its own replay noise): {}.",
                pm_.options.tau_pct,
                set_names(&sets.sure)
            );
        }
        // Sets inside the band may be as good: say so rather than crown one.
        if !sets.near.is_empty() {
            let _ = writeln!(
                o,
                "\nNear misses (within the noise of the {} % threshold, so possibly just as sufficient): {}.",
                pm_.options.tau_pct,
                set_names(&sets.near)
            );
        }
        if !pm_.dropped.is_empty() {
            let ids: Vec<String> = pm_.dropped.iter().map(|c| c.fault.to_string()).collect();
            let _ = writeln!(
                o,
                "\nLeft out (least blamed beyond the {MAX_CANDIDATES}-cause limit): {}.",
                ids.join(", ")
            );
        }
        close_section(&mut o);
    }

    // ---- Hotspots
    if !pm_.hotspots.is_empty() {
        open_section(
            &mut o,
            "Capacity hotspots",
            &format!(
                "{} {} found, a different question from the causes",
                pm_.hotspots.len(),
                plural(pm_.hotspots.len(), "limit", "limits")
            ),
        );
        let _ = writeln!(o, "## Capacity hotspots (a different question)\n");
        let _ = writeln!(
            o,
            "Limits the floor hit with no fault involved. Testing one means relaxing capacity \
             from {}, not removing a fault, so these are not part of the shares above.\n",
            clock_min(inc.start)
        );
        let _ = writeln!(
            o,
            "| Hotspot | Robot-minutes waiting | Relaxed: parcel-min recovered |\n|---|---|---|"
        );
        for h in &pm_.hotspots {
            let val = h.fix.map_or_else(
                || "-".to_owned(),
                |i| {
                    let (g, b) = pm_.fix_stats(i);
                    format!("{} ± {}", pmin(g), pminf(b))
                },
            );
            #[allow(clippy::cast_possible_wrap)]
            let blame = h.blame as i64;
            let _ = writeln!(o, "| {} | {} | {val} |", h.label, pmin(blame));
        }
        let _ = writeln!(
            o,
            "\nHow we know: relaxed values come from a replay with the capacity limit lifted, not \
             from removing a fault."
        );
        close_section(&mut o);
    }

    // ---- Trigger, detection, resolution
    open_section(
        &mut o,
        "Trigger and detection",
        &format!("alarm at {}", clock_min(inc.alarm)),
    );
    let _ = writeln!(o, "## Trigger and detection\n");
    if let Some(c) = pm_.candidates.iter().min_by_key(|c| (c.spec.at, c.fault)) {
        let _ = writeln!(
            o,
            "Trigger: {} at {} ({}).",
            describe_kind(&c.spec.kind),
            clock(c.spec.at),
            c.fault
        );
    }
    let _ = writeln!(
        o,
        "Detection: a one-sided CUSUM on the per-minute shortfall against the twin (k = {}, \
         h = {} in units of the twin's Poisson noise) alarmed at {}; the incident window runs \
         {} to {}{}.",
        pm_.options.cusum.k,
        pm_.options.cusum.h,
        clock_min(inc.alarm),
        clock_min(inc.start),
        clock_min(inc.end),
        if inc.settled {
            ""
        } else {
            " (still open when the run ended)"
        }
    );
    // Only the largest incident is analysed; say so about the others rather
    // than drop them silently (#15). The CLI has no flag to pick an incident
    // yet, so there is no command to suggest.
    for &tick in &pm_.other_incidents {
        let _ = writeln!(
            o,
            "\nAnother alarm at {} was not analysed (this report covers the largest incident \
             of the run).",
            clock(tick)
        );
    }
    let _ = writeln!(
        o,
        "\nHow we know: the trigger is the scenario's earliest fault; detection compares the real \
         run with its fault-free twin."
    );
    close_section(&mut o);
    open_section(
        &mut o,
        "Resolution",
        &match (inc.settled, s.time_to_recover_min) {
            (false, _) => "still open when the run ended".to_owned(),
            (true, Some(m)) => format!("recovered {m} min after the window opened"),
            (true, None) => "not recovered before the run ended".to_owned(),
        },
    );
    let _ = writeln!(o, "## Resolution\n");
    if inc.settled {
        let _ = writeln!(
            o,
            "{}.",
            s.time_to_recover_min.map_or_else(
                || "Throughput did not hold within 5 % of the twin for 5 minutes before the run \
                    ended"
                    .to_owned(),
                |m| format!(
                    "Throughput was back within 5 % of the twin, and held there, {m} min after the \
                     window opened"
                )
            )
        );
    } else {
        let _ = writeln!(
            o,
            "The incident window was still open when the run ended: throughput had not recovered."
        );
    }
    let _ = writeln!(
        o,
        "\nHow we know: throughput is the real run's deliveries per minute compared with its \
         fault-free twin."
    );

    // ---- What would have helped
    close_section(&mut o);
    open_section(
        &mut o,
        "What would have helped",
        &match fix_rows.first() {
            Some(&(_, g, b)) if g > 0 && significant(g as f64, b) => format!(
                "{} {} tested · best {:.0} %",
                fix_rows.len(),
                plural(fix_rows.len(), "fix", "fixes"),
                100.0 * g as f64 / loss.max(1) as f64
            ),
            _ => format!(
                "{} {} tested · none clearly helps",
                fix_rows.len(),
                plural(fix_rows.len(), "fix", "fixes")
            ),
        },
    );
    let _ = writeln!(o, "## What would have helped\n");
    let _ = writeln!(
        o,
        "Each fix was applied to the real shift at the time shown and replayed to {}. Each fix's \
         gain is the median over the histories of each history's own difference.\n",
        clock(pm_.horizon)
    );
    let _ = writeln!(
        o,
        "| Fix | From | Parcel-min recovered | Share of loss | Cost |\n|---|---|---|---|---|"
    );
    for &(i, g, b) in &fix_rows {
        let f = &pm_.fixes[i];
        let verdict = if !significant(g as f64, b) {
            " (within noise)"
        } else {
            ""
        };
        let _ = writeln!(
            o,
            "| {} | {} | {} ± {}{verdict} | {:.0} % | {} |",
            f.label,
            clock(f.at),
            pmin(g),
            pminf(b),
            100.0 * g as f64 / loss.max(1) as f64,
            f.cost
        );
    }
    let _ = writeln!(
        o,
        "\nHow we know: each fix was replayed from the time shown (X1–X{}).",
        pm_.fixes.len()
    );

    // ---- Action items
    close_section(&mut o);
    let n_actions = usize::from(!order.is_empty() && attributable)
        + fix_rows
            .iter()
            .filter(|&&(_, g, b)| g > 0 && significant(g as f64, b))
            .take(2)
            .count();
    open_section(
        &mut o,
        "Action items",
        &format!("{n_actions} {}", plural(n_actions, "action", "actions")),
    );
    let _ = writeln!(o, "## Action items\n");
    let _ = writeln!(
        o,
        "| Action | Type | Expected effect | Owner |\n|---|---|---|---|"
    );
    if let Some(&i) = order.first()
        && attributable
    {
        let _ = writeln!(
            o,
            "| Reduce the frequency or duration of: {} | prevent | up to {} ± {} parcel-min | site ops |",
            describe_kind(&pm_.candidates[i].spec.kind),
            pmin(att.alone(i)),
            pminf(loss_band)
        );
    }
    for &(i, g, b) in fix_rows
        .iter()
        .filter(|&&(_, g, b)| g > 0 && significant(g as f64, b))
        .take(2)
    {
        let _ = writeln!(
            o,
            "| {} | mitigate | {} ± {} parcel-min | site ops |",
            capitalise(&pm_.fixes[i].label),
            pmin(g),
            pminf(b)
        );
    }
    let _ = writeln!(
        o,
        "\nHow we know: the prevent row is the cause's replay, and the mitigate rows are the fix \
         replays."
    );

    // ---- Timeline
    let mut events: Vec<(Tick, String)> =
        pm_.timeline.iter().map(|(t, e)| (*t, e.clone())).collect();
    let at = minute_tick;
    events.push((at(inc.start), "incident window opens".to_owned()));
    events.push((at(inc.alarm), "CUSUM alarm".to_owned()));
    if inc.settled {
        events.push((at(inc.end), "incident window closes".to_owned()));
    }
    events.push((pm_.horizon, "loss scored (horizon)".to_owned()));
    events.sort_by_key(|e| e.0);
    close_section(&mut o);
    open_section(&mut o, "Timeline", &format!("{} events", events.len()));
    let _ = writeln!(o, "## Timeline\n");
    let _ = writeln!(o, "| Time | Tick | Event |\n|---|---|---|");
    for (t, e) in events {
        let _ = writeln!(o, "| {} | {t} | {e} |", clock(t));
    }
    let _ = writeln!(
        o,
        "\nHow we know: fault times come from the scenario; the alarm and window come from \
         detection on the real run and its twin."
    );

    // ---- Method
    close_section(&mut o);
    open_section(
        &mut o,
        "Method and confidence",
        "how the numbers were made and how far to trust them",
    );
    let _ = writeln!(o, "## Method and confidence\n");
    let _ = writeln!(o, "- {generated}");
    let robots: Vec<String> = pm_.dither.iter().map(ToString::to_string).collect();
    let _ = writeln!(
        o,
        "- **Counterfactual replay.** The shift was re-run with each combination of causes \
         removed, resuming from the last {} checkpoint before the earliest removed cause, \
         and scored at {} as net parcel-minutes of delay against the fault-free twin (same \
         seed, same parcels). Shapley values are exact over all subsets, in integer arithmetic, \
         so the shares add up exactly to the recovered loss.",
        minutes(pm_.options.checkpoint_every),
        clock(pm_.horizon)
    );
    let _ = writeln!(
        o,
        "- **Noise.** Robot paths diverge after any small change, so every replay ran on \
         {histories} {}: the real one{}. The headline delay and late count are the real \
         history's; every other value is the median across histories; ± is half their \
         range{}. *Alone*, *Last* and each fix's gain are the median over the histories of \
         each history's own difference. Two values that differ by less than twice that band are \
         reported as a tie.",
        plural(histories, "history", "histories"),
        if robots.is_empty() {
            if pm_.dither_tried > 0 {
                format!(
                    " (no dithered copy diverged: 0 of {} tried)",
                    pm_.dither_tried
                )
            } else {
                String::new()
            }
        } else {
            format!(
                ", and {} {} (of {} tried) in which one moving robot ({}) paused for a single \
                 tick at {} and the shift diverged",
                robots.len(),
                plural(robots.len(), "copy", "copies"),
                pm_.dither_tried,
                robots.join(", "),
                clock(pm_.dither_tick)
            )
        },
        if pm_.thin_dither() {
            format!(
                ", widened to at least {} % of the loss because fewer than {} histories \
                 diverged",
                super::NOISE_FLOOR_PCT,
                super::MIN_HISTORIES
            )
        } else {
            String::new()
        }
    );
    let _ = writeln!(
        o,
        "- **Detection** uses the twin's Poisson noise as the scale. h = {} is {:.1} times the \
         largest CUSUM value ({NULL_PEAK_MAX:.1}) seen in 160 simulated hours of fault-free runs \
         whose robot paths were perturbed.\n",
        pm_.options.cusum.h,
        pm_.options.cusum.h / NULL_PEAK_MAX
    );

    // ---- Replay index
    close_section(&mut o);
    open_section(
        &mut o,
        "Appendix: replay index",
        &format!(
            "{} {} × {histories} {}, one row per change",
            pm_.replays.len(),
            plural(pm_.replays.len(), "change", "changes"),
            plural(histories, "history", "histories")
        ),
    );
    let _ = writeln!(o, "## Appendix: replay index\n");
    let _ = writeln!(
        o,
        "| Replay | Change | From tick | Delay (parcel-min, median) | Range | Late (median) | World hash (real history) |\n\
         |---|---|---|---|---|---|---|"
    );
    for rp in &pm_.replays {
        let change = match rp.change {
            super::Change::Remove(0) => "actual shift".to_owned(),
            super::Change::Remove(m) => {
                let ids: Vec<String> = (0..n)
                    .filter(|i| m & (1 << i) != 0)
                    .map(|i| pm_.candidates[i].fault.to_string())
                    .collect();
                format!("remove {}", ids.join(", "))
            }
            super::Change::Fix(i) => fix_change(&pm_.fixes[i].interventions),
        };
        let lo = rp.delay.iter().copied().min().unwrap_or(0);
        let hi = rp.delay.iter().copied().max().unwrap_or(0);
        let late = rp.median_late();
        let _ = writeln!(
            o,
            "| {} | {change} | {} | {} | {}..{} | {late} | `{:016x}` |",
            rp.id,
            rp.from_tick,
            pmin(rp.median_delay()),
            pmin(lo),
            pmin(hi),
            rp.live_hash
        );
    }
    let _ = writeln!(
        o,
        "\n[A] actual run and [T] fault-free twin, {} ticks ({}).",
        pm_.ticks,
        minutes(pm_.ticks)
    );
    close_section(&mut o);
    o
}

/// A fix's interventions in a few words.
fn fix_change(ivs: &[Intervention]) -> String {
    match ivs {
        [Intervention::ClearAfter { ticks }] => format!("clear after {}", minutes(*ticks)),
        [Intervention::AddRobots { robots, .. }] => format!("add {robots} robots"),
        [Intervention::SetStationCap { cap, .. }] => format!("station cap {cap}"),
        [Intervention::RemapDestination { chute, .. }, ..] => {
            format!(
                "remap {} {} to {chute}",
                ivs.len(),
                plural(ivs.len(), "destination", "destinations")
            )
        }
        _ => format!("{} interventions", ivs.len()),
    }
}

/// Plain words for an interaction's sign and size (see [`Interplay`]).
fn interplay_text(kind: Option<Interplay>, overlap: &str) -> String {
    match kind {
        Some(Interplay::Redundant) => format!(
            "are redundant: either one alone causes most of this loss, so removing just one \
             recovers little ({overlap} parcel-min of overlap)"
        ),
        Some(Interplay::Compound) => format!(
            "compound: together they caused {overlap} parcel-min more damage than each did alone"
        ),
        None => "are independent".to_owned(),
    }
}

/// `one` or `many` by count.
const fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 { one } else { many }
}

/// The summary's cause sentence. Every cause can be within noise while
/// their sum is not (several small causes): say so instead of printing an
/// empty sentence.
fn cause_sentence(parts: &[String]) -> String {
    if parts.is_empty() {
        "No single cause stands out of the noise; together they explain the loss.".to_owned()
    } else {
        format!("{}.", capitalise(&parts.join("; ")))
    }
}

/// Plain words for contingency size `k` (`None`: never pivotal).
fn need_text(k: Option<u32>) -> String {
    match k {
        None => "no".to_owned(),
        Some(0) => "yes, on its own".to_owned(),
        Some(1) => "yes, with 1 other cause".to_owned(),
        Some(k) => format!("yes, with {k} other causes"),
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| {
        f.to_uppercase().collect::<String>() + c.as_str()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::postmortem::{Options, run};
    use switchyard_core::config::ScenarioConfig;
    use switchyard_core::faults::{FaultKind, FaultSpec};
    use switchyard_core::ids::{ChuteId, RobotId};
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
    fn report_of_a_small_incident() {
        let pm = small_incident();
        // Efficiency holds exactly on the median game.
        let att = &pm.attribution;
        assert_eq!(
            att.shapley_scaled.iter().sum::<i128>(),
            att.scale * i128::from(att.total())
        );
        insta::assert_snapshot!(render(&pm));
    }

    /// #16: a cause whose share is within its noise band is not ranked; it is
    /// listed once as within noise, and every ranked share carries a ±.
    #[test]
    fn a_cause_within_noise_is_not_ranked() {
        let mut cfg = ScenarioConfig::from_toml_str(SMALL_TOML).unwrap();
        cfg.name = "small-jam".to_owned();
        let busy = switchyard_core::sim::Sim::new(&cfg)
            .unwrap()
            .world()
            .destination_chute[0];
        cfg.faults = vec![
            FaultSpec {
                at: 1_200,
                duration: 1_800,
                kind: FaultKind::ChuteJam { chute: busy },
            },
            // A scanner drift of 0 ppm changes nothing: a null cause.
            FaultSpec {
                at: 1_320,
                duration: 600,
                kind: FaultKind::ScannerDrift {
                    station: switchyard_core::ids::StationId(1),
                    error_rate_ppm: 0,
                },
            },
        ];
        let pm = run(
            &cfg,
            6_000,
            &Options {
                dither: 2,
                ..Options::default()
            },
        )
        .unwrap();
        let md = render(&pm);
        assert!(md.contains("Within noise (share no larger"), "{md}");
        assert!(
            !md.contains(
                "
| 2 |"
            ),
            "null cause was ranked:
{md}"
        );
        let row = md.lines().find(|l| l.starts_with("| 1 |")).unwrap();
        assert!(row.matches('±').count() >= 2, "share lacks ±: {row}");
    }

    /// #20: the headline and the Impact table state the real history's loss
    /// and late count (index 0); the median over the dither copies is for
    /// attribution only. Both places must agree.
    #[test]
    fn headline_facts_come_from_the_real_history_not_the_median() {
        let mut pm = small_incident();
        pm.dither = vec![RobotId(1), RobotId(2)];
        assert_eq!(pm.histories(), 3);
        // Real history 6 000 000 parcel-ticks / 7 late; the copies are lower,
        // so the median (the middle value) is a different number.
        pm.replays[0].delay = vec![6_000_000, 5_400_000, 5_000_000];
        pm.replays[0].late = vec![7, 2, 3];
        assert_ne!(pm.loss(), pm.real_loss());
        let md = render(&pm);
        let real = pmin(6_000_000);
        assert!(
            md.contains(&format!(
                "cost **{real} parcel-minutes** of delay, and 7 parcels"
            )),
            "{md}"
        );
        assert!(
            md.contains(&format!("| {real} parcel-min (± ")),
            "Impact delay disagrees with the headline: {md}"
        );
        assert!(!md.contains(&format!("cost **{} parcel-minutes**", pmin(pm.loss()))));
        // The late count at the horizon in Impact is the headline's, and the
        // whole-run scorecard count says where it stops.
        assert!(
            md.contains(&format!(
                "| Parcels more than {} min late at {} | 7 |",
                pm.options.sla_extra_minutes,
                clock(pm.horizon)
            )),
            "{md}"
        );
        assert!(
            md.contains(&format!("(whole run, to {})", clock(pm.ticks))),
            "{md}"
        );
    }

    /// Review of #20: the unexplained remainder is `median loss - v(N)`. When
    /// the headline states the (different) real-history loss, the remainder
    /// names the median it is part of, so the numbers never silently disagree.
    #[test]
    fn the_unexplained_remainder_names_the_median_it_is_part_of() {
        let mut pm = small_incident();
        pm.dither = vec![RobotId(1), RobotId(2)];
        let tpm = i64::from(TICKS_PER_MINUTE);
        // A median 300 parcel-min above what the causes recover: the remainder
        // is outside the noise band, so it is reported (and names the median).
        let median = pm.attribution.total() + 300 * tpm;
        // Real history 100 parcel-min above the median, so it stays attributable.
        pm.replays[0].delay = vec![median + 100 * tpm, median, median - 50 * tpm];
        assert_eq!(pm.loss(), median);
        let md = render(&pm);
        assert!(
            md.contains(&format!(
                "of the {} parcel-min median loss across the 3 histories (the figure the shares divide) is not explained",
                pmin(median)
            )),
            "{md}"
        );
        // Same real and median loss: no extra clause.
        pm.replays[0].delay = vec![median, median, median];
        assert!(!render(&pm).contains("median loss across"));
    }

    /// #21(a): the header names the horizon actually used, which is clamped
    /// to the run end.
    #[test]
    fn the_horizon_note_follows_the_clamped_horizon() {
        let mut pm = small_incident();
        let end = pm.incident.unwrap().end;
        let margin = pm.options.cusum.margin_minutes;
        pm.horizon = minute_tick(end + margin);
        assert_eq!(horizon_note(&pm, end), format!("window end + {margin} min"));
        pm.horizon = minute_tick(end + 3);
        assert_eq!(
            horizon_note(&pm, end),
            "window end + 3 min, where the run ends"
        );
        pm.horizon = minute_tick(end);
        assert_eq!(horizon_note(&pm, end), "the window end, where the run ends");
        let md = render(&pm);
        assert!(!md.contains(&format!("window end + {margin} min")), "{md}");
    }

    #[test]
    fn other_alarms_are_listed_as_not_analysed() {
        let mut pm = small_incident();
        assert!(!render(&pm).contains("Another alarm"));
        pm.other_incidents = vec![90 * TICKS_PER_MINUTE];
        let md = render(&pm);
        assert!(
            md.contains("Another alarm at 01:30 was not analysed"),
            "{md}"
        );
    }

    #[test]
    fn the_cause_sentence_is_never_empty() {
        assert_eq!(
            cause_sentence(&[]),
            "No single cause stands out of the noise; together they explain the loss."
        );
        assert_eq!(
            cause_sentence(&["a explains 9 %".to_owned()]),
            "A explains 9 %."
        );
    }

    #[test]
    fn a_run_without_an_incident_says_so() {
        let cfg = ScenarioConfig::from_toml_str(SMALL_TOML).unwrap();
        let pm = run(
            &cfg,
            2_400,
            &Options {
                dither: 1,
                ..Options::default()
            },
        )
        .unwrap();
        assert!(pm.incident.is_none());
        assert!(render(&pm).contains("no incident"));
    }

    #[test]
    fn interaction_wording_follows_the_sign_of_the_recovery_game() {
        use crate::postmortem::shapley::Attribution;
        // Overdetermination: [0, 0, 0, L] -> I > 0 -> redundant.
        let a = Attribution::new(&[0, 0, 0, 600]);
        assert!(interplay_text(Interplay::of(a.interaction(0, 1)), "600").contains("redundant"));
        // Compounding: either removal recovers most -> I < 0 -> compound.
        let b = Attribution::new(&[0, 500, 500, 600]);
        assert!(interplay_text(Interplay::of(b.interaction(0, 1)), "100").contains("compound"));
    }

    /// The headline is the first block under the title, the details are
    /// folded, and the CLI summary stops before the first fold.
    #[test]
    fn the_headline_comes_first_and_the_detail_is_folded() {
        let pm = small_incident();
        let md = render(&pm);
        let head = md.find("**Incident ").expect("headline eyebrow");
        let summary_at = md.find("## Summary").unwrap();
        let first_fold = md.find("<details>").unwrap();
        assert!(head < summary_at && summary_at < first_fold, "{md}");
        let generated = md.find("**Generated by**").unwrap();
        assert!(
            generated > md.find("## Method and confidence").unwrap(),
            "{md}"
        );
        assert_eq!(
            md.matches("<details>").count(),
            md.matches("</details>").count()
        );
        // Every fold names itself on one summary line.
        for line in md.lines().filter(|l| l.starts_with("<summary>")) {
            assert!(
                line.contains("<b>") && line.contains(" \u{2014} "),
                "{line}"
            );
        }
        let s = summary(&pm);
        assert!(s.contains("## Summary") && !s.contains("<details>"), "{s}");
    }

    #[test]
    fn thousands_get_separators() {
        assert_eq!(thousands(6264), "6,264");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(thousands(-1500), "-1,500");
        assert_eq!(thousands(0), "0");
    }

    #[test]
    fn clocks_read_as_hours_and_minutes() {
        assert_eq!(clock(0), "00:00");
        assert_eq!(clock(7_200 + 3_720), "01:31");
    }
}
