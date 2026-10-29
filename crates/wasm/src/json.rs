//! JSON shapes handed to the viewer. Each struct mirrors a TypeScript type in
//! `viewer/src/data/types.ts` (named in its doc comment); the shared fixture
//! `viewer/src/data/fixtures/wasm-shapes.json` keeps the two in step.

use serde::Serialize;
use switchyard_analysis::postmortem::headline::Headline;
use switchyard_core::Tick;
use switchyard_core::events::WaitCause;
use switchyard_core::faults::{FaultKind, FaultSpec};
use switchyard_core::world::Phase;
use switchyard_trace::{ParcelTrace, Span, cause_parts};

/// A fault shaded on the chart (`FaultWindow`).
#[derive(Serialize, Clone, PartialEq, Eq)]
pub struct WindowOut {
    pub kind: String,
    #[serde(rename = "startTick")]
    pub start_tick: Tick,
    #[serde(rename = "endTick")]
    pub end_tick: Tick,
}

/// The chart series (`MetricsSeries`).
#[derive(Serialize)]
pub struct MetricsOut {
    #[serde(rename = "ticksPerMinute")]
    pub ticks_per_minute: u32,
    #[serde(rename = "startMinute")]
    pub start_minute: u32,
    pub delivered: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub twin: Option<Vec<u32>>,
    pub faults: Vec<WindowOut>,
}

/// One trace span (`TraceSpan`).
#[derive(Serialize)]
pub struct SpanOut {
    pub phase: &'static str,
    #[serde(rename = "startTick")]
    pub start_tick: Tick,
    #[serde(rename = "endTick")]
    pub end_tick: Tick,
    #[serde(rename = "directBlocker")]
    pub direct_blocker: Option<u32>,
    #[serde(rename = "rootCause")]
    pub root_cause: Option<String>,
}

/// A parcel trace (`ParcelTrace`).
#[derive(Serialize)]
pub struct TraceOut {
    #[serde(rename = "parcelId")]
    pub parcel_id: u32,
    pub robot: Option<u32>,
    pub spans: Vec<SpanOut>,
}

/// The viewer's phase name (`PhaseName`).
const fn phase_id(phase: Phase) -> &'static str {
    match phase {
        Phase::Queued => "queued",
        Phase::AwaitingRobot => "awaiting_robot",
        Phase::Loading => "loading",
        Phase::Travelling => "travelling",
        Phase::Blocked => "blocked",
        Phase::ChuteWait => "chute_wait",
        Phase::Dropped => "drop",
    }
}

/// Plain label of a fault: `dead robot R31`, `jammed chute C12`, ...
fn fault_label(spec: Option<&FaultSpec>) -> String {
    match spec.map(|s| &s.kind) {
        Some(FaultKind::RobotFailure { robot }) => format!("dead robot {robot}"),
        Some(FaultKind::AisleBlock { .. }) => "blocked aisle".into(),
        Some(FaultKind::ChuteJam { chute }) => format!("jammed chute {chute}"),
        Some(FaultKind::InductionOutage { station }) => {
            format!("induction outage at {station}")
        }
        Some(FaultKind::VolumeSurge { .. }) => "volume surge".into(),
        Some(FaultKind::ScannerDrift { station, .. }) => format!("scanner drift at {station}"),
        None => "a fault".into(),
    }
}

/// Plain label of a wait cause, as the root of a blocked or delayed span.
fn cause_label(cause: WaitCause, faults: &[FaultSpec]) -> String {
    if let Some(f) = cause.fault() {
        return fault_label(faults.get(f.index()));
    }
    let parts = cause_parts(cause);
    match cause {
        WaitCause::Cycle(_) => format!("deadlock around R{}", parts.id),
        WaitCause::ChuteUnavailable(c) => format!("chute {c} swapping bags"),
        WaitCause::StationBusy(s) => format!("station {s} busy"),
        WaitCause::BlockedBy(r) | WaitCause::Yielded(r) => format!("robot {r}"),
        WaitCause::CellBlocked(_) | WaitCause::Fault(_) => "a fault".into(),
    }
}

fn span_out(span: &Span, faults: &[FaultSpec]) -> SpanOut {
    let a = span.attrs;
    let direct = a.direct_blocker.and_then(|c| cause_parts(c).robot);
    // A blocked span's root; any other span's fault delay.
    let root = a.root_cause.or(a.delay_root);
    // A plain robot root that is the direct blocker adds nothing to the chain.
    let root = root.filter(|&c| cause_parts(c).robot.is_none() || cause_parts(c).robot != direct);
    SpanOut {
        phase: phase_id(span.phase),
        start_tick: span.start,
        end_tick: span.end,
        direct_blocker: direct.map(|r| r.0),
        root_cause: root.map(|c| cause_label(c, faults)),
    }
}

/// The viewer shape of a trace. `faults` is the sim's fault schedule, used to
/// name root causes.
#[must_use]
pub fn trace_out(trace: &ParcelTrace, faults: &[FaultSpec]) -> TraceOut {
    TraceOut {
        parcel_id: trace.parcel.0,
        robot: trace.robots.last().map(|r| r.0),
        spans: trace.spans.iter().map(|s| span_out(s, faults)).collect(),
    }
}

/// The main cause in the headline (`HeadlineCause`).
#[derive(Serialize)]
pub struct CauseOut {
    pub label: String,
    pub fault: String,
    #[serde(rename = "sharePct")]
    pub share_pct: f64,
    #[serde(rename = "bandPct")]
    pub band_pct: f64,
}

/// The best fix in the headline (`HeadlineFix`).
#[derive(Serialize)]
pub struct FixOut {
    pub label: String,
    #[serde(rename = "recoveredParcelMin")]
    pub recovered_parcel_min: i64,
    #[serde(rename = "recoveryPct")]
    pub recovery_pct: f64,
    #[serde(rename = "withinNoise")]
    pub within_noise: bool,
}

/// The one-glance post-mortem summary (`PostmortemHeadline`).
#[derive(Serialize)]
pub struct HeadlineOut {
    #[serde(rename = "startMin")]
    pub start_min: usize,
    #[serde(rename = "endMin")]
    pub end_min: usize,
    pub settled: bool,
    #[serde(rename = "maxDipPct")]
    pub max_dip_pct: f64,
    pub cause: Option<CauseOut>,
    #[serde(rename = "tieWith")]
    pub tie_with: Option<String>,
    #[serde(rename = "lossParcelMin")]
    pub loss_parcel_min: i64,
    #[serde(rename = "lateParcels")]
    pub late_parcels: u32,
    pub fix: Option<FixOut>,
    pub replays: usize,
    pub histories: usize,
}

impl From<&Headline> for HeadlineOut {
    fn from(h: &Headline) -> Self {
        Self {
            start_min: h.start_min,
            end_min: h.end_min,
            settled: h.settled,
            max_dip_pct: h.max_dip_pct,
            cause: h.cause.as_ref().map(|c| CauseOut {
                label: c.label.clone(),
                fault: c.fault.clone(),
                share_pct: c.share_pct,
                band_pct: c.band_pct,
            }),
            tie_with: h.tie_with.clone(),
            loss_parcel_min: h.loss_parcel_min,
            late_parcels: h.late_parcels,
            fix: h.fix.as_ref().map(|f| FixOut {
                label: f.label.clone(),
                recovered_parcel_min: f.recovered_parcel_min,
                recovery_pct: f.recovery_pct,
                within_noise: f.within_noise,
            }),
            replays: h.replays,
            histories: h.histories,
        }
    }
}
