//! Regression tests from the CP3 Auditor's adversarial cases A–E (issues
//! #13–#16). Each builds a medium-floor incident designed to trip one
//! failure mode of the post-mortem and checks the report gets it right.

use switchyard_analysis::postmortem::{self, Options, PostMortem, report};
use switchyard_core::Tick;
use switchyard_core::config::ScenarioConfig;
use switchyard_core::faults::{FaultKind, FaultSpec};
use switchyard_core::ids::{ChuteId, StationId};
use switchyard_core::presets;
use switchyard_core::sim::Sim;

const MIN: Tick = 120;

fn f(at: Tick, duration: Tick, kind: FaultKind) -> FaultSpec {
    FaultSpec { at, duration, kind }
}

fn base(seed: u64) -> ScenarioConfig {
    let mut c = ScenarioConfig::from_toml_str(presets::MEDIUM_TOML).unwrap();
    c.seed = seed;
    c.faults.clear();
    c
}

fn chute_of_dest(cfg: &ScenarioConfig, d: usize) -> ChuteId {
    Sim::new(cfg).unwrap().world().destination_chute[d]
}

fn run(cfg: &ScenarioConfig, ticks: Tick, dither: usize) -> (PostMortem, String) {
    let pm = postmortem::run(
        cfg,
        ticks,
        &Options {
            dither,
            ..Options::default()
        },
    )
    .unwrap();
    let md = report::render(&pm);
    (pm, md)
}

/// B: two identical jams of the same chute are overdetermined: removing
/// either alone recovers nothing. The report must call them redundant, never
/// compound (#13).
#[test]
fn b_identical_jams_are_reported_as_redundant() {
    let mut c = base(6);
    let a = chute_of_dest(&c, 0);
    c.faults
        .push(f(20 * MIN, 15 * MIN, FaultKind::ChuteJam { chute: a }));
    c.faults
        .push(f(20 * MIN, 15 * MIN, FaultKind::ChuteJam { chute: a }));
    c.faults.push(f(
        21 * MIN,
        5 * MIN,
        FaultKind::ScannerDrift {
            station: StationId(1),
            error_rate_ppm: 0,
        },
    ));
    let (pm, md) = run(&c, 60 * MIN, 1);
    let att = &pm.attribution;
    assert!(att.interaction(0, 1) > 0.0, "{md}");
    let line = md
        .lines()
        .find(|l| l.starts_with("Interaction:") && l.contains("(F0)") && l.contains("(F1)"))
        .unwrap_or_else(|| panic!("no F0/F1 interaction line:\n{md}"));
    assert!(
        line.contains("redundant") && !line.contains("compound"),
        "{line}"
    );
}

/// Dither copies that do not diverge are no-ops that shrink the noise band:
/// only diverging copies count, and the report gives the effective count
/// (#14).
#[test]
fn dither_counts_only_copies_that_diverge() {
    let mut c = base(2);
    let a = chute_of_dest(&c, 0);
    c.faults
        .push(f(20 * MIN, 15 * MIN, FaultKind::ChuteJam { chute: a }));
    let (pm, md) = run(&c, 45 * MIN, 4);
    assert_eq!(pm.history_hashes.len(), pm.histories());
    assert!(pm.histories() > 1, "{md}");
    assert!(
        pm.history_hashes[1..]
            .iter()
            .all(|&h| h != pm.history_hashes[0]),
        "{:?}",
        pm.history_hashes
    );
    assert!(pm.dither_tried >= pm.dither.len());
    assert!(
        md.contains(&format!("(of {} tried)", pm.dither_tried)),
        "{md}"
    );
    // Fewer than 5 histories: the bands are widened, and the report says so.
    assert!(pm.thin_dither());
    assert!(md.contains("widened to at least 5 % of the loss"), "{md}");
    #[allow(clippy::cast_precision_loss)]
    let floor = 0.05 * pm.loss() as f64;
    assert!(pm.loss_band() >= floor - 1e-9);
}
