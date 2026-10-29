//! Headline builder on the chaos presets: the numbers the one-glance summary
//! states, checked against the post-mortem they come from. Slow: run in
//! release with
//! `cargo test -p switchyard-analysis --release --test headline -- --ignored --nocapture`.

use switchyard_analysis::postmortem::{self, Options};
use switchyard_core::Tick;
use switchyard_core::config::ScenarioConfig;
use switchyard_core::presets;

/// Ticks per minute (1 tick = 0.5 s).
const MIN: Tick = 120;

fn preset(name: &str) -> ScenarioConfig {
    ScenarioConfig::from_toml_str(presets::by_name(name).unwrap()).unwrap()
}

#[test]
#[ignore = "slow: run with --release -- --ignored"]
fn two_dead_robots_and_a_jam_headline() {
    let pm = postmortem::run(
        &preset("two-dead-robots-and-a-jam"),
        120 * MIN,
        &Options::default(),
    )
    .unwrap();
    let h = pm.headline().expect("an incident");
    println!("{h:#?}");

    assert_eq!(h.start_min, 30);
    assert_eq!(h.end_min, 52);
    assert!(h.settled);
    let cause = h.cause.as_ref().expect("a main cause");
    assert_eq!(cause.label, "C289 jam");
    assert!(
        (65.0..=75.0).contains(&cause.share_pct),
        "share {}",
        cause.share_pct
    );
    assert!(h.tie_with.is_none(), "tie with {:?}", h.tie_with);
    assert_eq!(h.late_parcels, 110);
    let fix = h.fix.as_ref().expect("a fix");
    assert!(fix.label.contains("C269"), "fix label {:?}", fix.label);
    assert!(
        (78.0..=88.0).contains(&fix.recovery_pct),
        "recovery {}",
        fix.recovery_pct
    );
    assert!(!fix.within_noise, "best fix is within noise");
}

#[test]
#[ignore = "slow: run with --release -- --ignored"]
fn aisle_7_blocked_headline() {
    let pm = postmortem::run(&preset("aisle-7-blocked"), 120 * MIN, &Options::default()).unwrap();
    let h = pm.headline().expect("an incident");
    println!("{h:#?}");

    let cause = h.cause.as_ref().expect("a main cause");
    assert_eq!(cause.label, "aisle blockage");
    assert!(cause.share_pct > 50.0, "share {}", cause.share_pct);
    assert!(h.fix.is_some(), "no fix");
}
