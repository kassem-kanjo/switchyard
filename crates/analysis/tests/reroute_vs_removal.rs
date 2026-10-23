//! Reroute vs removal (measurement): how much of the chute jam's loss a
//! sort-plan remap recovers, compared with simply removing the jam, on every
//! dither history. This is an ignored measurement, not a regression test:
//! run it in release with
//! `cargo test -p switchyard-analysis --release --test reroute_vs_removal -- --ignored --nocapture`.

use switchyard_analysis::postmortem::{self, Options};
use switchyard_core::Tick;
use switchyard_core::config::ScenarioConfig;
use switchyard_core::faults::FaultKind;
use switchyard_core::ids::FaultId;
use switchyard_core::presets;

/// Ticks per minute (1 tick = 0.5 s).
const MIN: Tick = 120;

/// Parcel-ticks to parcel-minutes.
#[allow(clippy::cast_precision_loss)]
fn parcel_minutes(ticks: i64) -> f64 {
    ticks as f64 / MIN as f64
}

/// Mean of `xs`, in parcel-minutes.
#[allow(clippy::cast_precision_loss)]
fn mean_minutes(xs: &[i64]) -> f64 {
    xs.iter().map(|&x| parcel_minutes(x)).sum::<f64>() / xs.len() as f64
}

#[test]
#[ignore = "measurement; run in release with --ignored --nocapture"]
fn reroute_vs_removal() {
    let cfg = ScenarioConfig::from_toml_str(presets::by_name("two-dead-robots-and-a-jam").unwrap())
        .unwrap();
    let pm = postmortem::run(
        &cfg,
        90 * MIN,
        &Options {
            dither: 8,
            ..Options::default()
        },
    )
    .unwrap();

    // The jam is the first fault of the preset (F0); find it by id and kind
    // rather than by position.
    let jam = pm
        .candidates
        .iter()
        .position(|c| c.fault == FaultId(0) && matches!(c.spec.kind, FaultKind::ChuteJam { .. }));
    // Its remap fix is labelled "... from the jam on (never undone)"; the
    // hotspots' relax fixes do not carry that suffix.
    let remap = pm
        .fixes
        .iter()
        .position(|f| f.label.contains("send chute") && f.label.contains("from the jam on"));

    assert!(
        pm.histories() >= 2,
        "need at least two histories, got {}",
        pm.histories()
    );
    assert!(jam.is_some(), "the chute jam (F0) is not a candidate");
    assert!(remap.is_some(), "the jam's remap fix is missing");
    let jam = jam.unwrap();
    let remap = remap.unwrap();

    let mask = 1usize << jam;
    let removal: Vec<i64> = pm.masks()[0]
        .delay
        .iter()
        .zip(&pm.masks()[mask].delay)
        .map(|(real, without)| real - without)
        .collect();
    let remapped = pm.fix_gain(remap);
    let difference: Vec<i64> = remapped
        .iter()
        .zip(&removal)
        .map(|(fix, without)| fix - without)
        .collect();

    println!(
        "two-dead-robots-and-a-jam: {} histories, horizon {} ticks",
        pm.histories(),
        pm.horizon
    );
    println!(
        "real loss {} {:.1} parcel-min; median loss {:.1} parcel-min",
        pm.masks()[0].id,
        parcel_minutes(pm.real_loss()),
        parcel_minutes(pm.loss())
    );
    println!(
        "candidate {} is {} ({}); mask {mask} -> replay {}",
        jam,
        pm.candidates[jam].fault,
        postmortem::describe_kind(&pm.candidates[jam].spec.kind),
        pm.masks()[mask].id
    );

    println!("fixes:");
    for (i, fix) in pm.fixes.iter().enumerate() {
        println!(
            "  {} @ {}: {} [{}]",
            pm.fix_replays()[i].id,
            fix.at,
            fix.label,
            fix.cost
        );
    }
    println!(
        "replays: baseline {}, removal {}, remap {}",
        pm.masks()[0].id,
        pm.masks()[mask].id,
        pm.fix_replays()[remap].id
    );

    println!(
        "{:<10} {:>12} {:>12} {:>14}",
        "history", "removal", "remap", "fix-removal"
    );
    for h in 0..pm.histories() {
        let name = if h == 0 {
            "0 (real)".to_owned()
        } else {
            h.to_string()
        };
        println!(
            "{name:<10} {:>12.1} {:>12.1} {:>14.1}",
            parcel_minutes(removal[h]),
            parcel_minutes(remapped[h]),
            parcel_minutes(difference[h])
        );
    }

    println!(
        "real history alone: removal {:.1}, remap {:.1}, fix minus removal {:.1} parcel-min",
        parcel_minutes(removal[0]),
        parcel_minutes(remapped[0]),
        parcel_minutes(difference[0])
    );
    println!(
        "median: removal {:.1}, remap {:.1}, fix minus removal {:.1} parcel-min",
        parcel_minutes(postmortem::median(&removal)),
        parcel_minutes(postmortem::median(&remapped)),
        parcel_minutes(postmortem::median(&difference))
    );
    println!(
        "mean:   removal {:.1}, remap {:.1}, fix minus removal {:.1} parcel-min",
        mean_minutes(&removal),
        mean_minutes(&remapped),
        mean_minutes(&difference)
    );
    let wins = difference.iter().filter(|&&d| d > 0).count();
    println!("fix beats removal on {wins}/{} histories", pm.histories());
}
