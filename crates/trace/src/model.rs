//! The trace data model: a parcel's lifetime as a list of phase spans.

use switchyard_core::Tick;
use switchyard_core::events::WaitCause;
use switchyard_core::ids::{ChuteId, DestinationId, ParcelId, RobotId, StationId};
use switchyard_core::world::Phase;

/// Extra facts about a span.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpanAttrs {
    /// The robot the parcel was assigned to when the span began (none while
    /// the parcel is `Queued`).
    pub robot: Option<RobotId>,
    /// `Blocked` spans: the immediate cause of the wait. `None` on other
    /// phases, and on a `Blocked` tick for which the stream carried no
    /// `RobotWaited` event (reported as unattributed).
    pub direct_blocker: Option<WaitCause>,
    /// `Blocked` spans: the end of the wait-for chain.
    pub root_cause: Option<WaitCause>,
}

/// One phase of a parcel's life: the half-open tick range `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    /// The phase.
    pub phase: Phase,
    /// First tick of the span.
    pub start: Tick,
    /// One past the last tick of the span (the next span starts here).
    pub end: Tick,
    /// Robot and blocker details.
    pub attrs: SpanAttrs,
}

impl Span {
    /// Length in ticks.
    #[must_use]
    pub const fn ticks(&self) -> u32 {
        self.end - self.start
    }
}

/// Whether a trace ends in a drop or at the end of the observed run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceStatus {
    /// The parcel was dropped into a chute; `done` is the drop tick.
    Complete,
    /// The parcel was still in flight; `done` is the end of observation.
    Open,
}

/// The trace of one parcel.
///
/// `spans` tile `[arrival, done)` exactly: the first span starts at
/// `arrival`, each span starts where the previous one ended, and the last
/// ends at `done`. The terminal `Dropped` phase is an instant, not a span:
/// `done` *is* the tick the parcel fell into the chute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParcelTrace {
    /// The parcel.
    pub parcel: ParcelId,
    /// The station it arrived at.
    pub station: StationId,
    /// Where it should go.
    pub destination: DestinationId,
    /// Arrival tick (start of the first span).
    pub arrival: Tick,
    /// Drop tick, or the end of observation for an open trace.
    pub done: Tick,
    /// Complete or still in flight.
    pub status: TraceStatus,
    /// The chute it fell into (complete traces).
    pub chute: Option<ChuteId>,
    /// Every robot assigned to it, in order (several if a robot failed).
    pub robots: Vec<RobotId>,
    /// It went into a chute that does not serve its destination.
    pub mis_sorted: bool,
    /// The phase spans, in time order.
    pub spans: Vec<Span>,
}

impl ParcelTrace {
    /// Total latency in ticks (`done - arrival`).
    #[must_use]
    pub const fn latency(&self) -> u32 {
        self.done - self.arrival
    }

    /// True while the parcel was still in flight at the end of observation.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self.status, TraceStatus::Open)
    }

    /// Check the tiling invariant: no gaps, no overlaps, no empty spans, and
    /// the spans cover exactly `[arrival, done)`.
    ///
    /// # Errors
    /// A description of the first violation.
    pub fn check_tiling(&self) -> Result<(), String> {
        let mut at = self.arrival;
        for (i, s) in self.spans.iter().enumerate() {
            if s.start != at {
                return Err(format!(
                    "{}: span {i} ({}) starts at {} but the previous ended at {at}",
                    self.parcel,
                    phase_name(s.phase),
                    s.start
                ));
            }
            if s.end <= s.start {
                return Err(format!(
                    "{}: span {i} ({}) is empty or reversed [{}, {})",
                    self.parcel,
                    phase_name(s.phase),
                    s.start,
                    s.end
                ));
            }
            at = s.end;
        }
        if at != self.done {
            return Err(format!(
                "{}: spans end at {at} but the trace ends at {}",
                self.parcel, self.done
            ));
        }
        Ok(())
    }
}

/// The name of a phase, as used in every export.
#[must_use]
pub const fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Queued => "Queued",
        Phase::AwaitingRobot => "AwaitingRobot",
        Phase::Loading => "Loading",
        Phase::Travelling => "Travelling",
        Phase::Blocked => "Blocked",
        Phase::ChuteWait => "ChuteWait",
        Phase::Dropped => "Dropped",
    }
}

/// A [`WaitCause`] split into a stable kind label and the id it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CauseParts {
    /// Snake-case label: `blocked_by`, `yielded`, `cell_blocked`,
    /// `chute_unavailable`, `station_busy` or `cycle` (a deadlock cycle,
    /// named by its lowest robot id).
    pub kind: &'static str,
    /// The id: robot, fault, chute or station.
    pub id: u32,
    /// The robot the cause names, for the robot-to-robot kinds.
    pub robot: Option<RobotId>,
}

/// Split a cause into its label and id.
#[must_use]
pub fn cause_parts(cause: WaitCause) -> CauseParts {
    match cause {
        WaitCause::BlockedBy(r) => CauseParts {
            kind: "blocked_by",
            id: r.0,
            robot: Some(r),
        },
        WaitCause::Yielded(r) => CauseParts {
            kind: "yielded",
            id: r.0,
            robot: Some(r),
        },
        WaitCause::CellBlocked(f) => CauseParts {
            kind: "cell_blocked",
            id: f.0,
            robot: None,
        },
        WaitCause::ChuteUnavailable(c) => CauseParts {
            kind: "chute_unavailable",
            id: u32::from(c.0),
            robot: None,
        },
        WaitCause::StationBusy(s) => CauseParts {
            kind: "station_busy",
            id: u32::from(s.0),
            robot: None,
        },
        WaitCause::Cycle(r) => CauseParts {
            kind: "cycle",
            id: r.0,
            robot: Some(r),
        },
    }
}

/// `kind:Id` text for a cause, for example `blocked_by:R12`.
pub(crate) fn cause_label(cause: WaitCause) -> String {
    match cause {
        WaitCause::BlockedBy(r) => format!("blocked_by:{r}"),
        WaitCause::Yielded(r) => format!("yielded:{r}"),
        WaitCause::CellBlocked(f) => format!("cell_blocked:{f}"),
        WaitCause::ChuteUnavailable(c) => format!("chute_unavailable:{c}"),
        WaitCause::StationBusy(s) => format!("station_busy:{s}"),
        WaitCause::Cycle(r) => format!("cycle:{r}"),
    }
}
