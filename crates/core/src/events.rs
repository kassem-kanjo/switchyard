//! The event stream: the core's only output (ADR 0004).
//!
//! Traces, metrics, the viewer and post-mortems are all pure functions of
//! this stream, so a new analysis never needs a change to the sim. Every
//! event carries the tick it happened on. Within a tick, events appear in the
//! fixed phase order documented in [`crate::sim`], and within a phase in id
//! order, so the stream itself is deterministic (ADR 0002).
//!
//! Variants marked *(CP2)* are part of the schema now so that consumers can
//! be written against it, but the sim does not emit them yet.

use crate::Tick;
use crate::ids::{ChuteId, DestinationId, FaultId, ParcelId, RobotId, StationId};
use crate::world::{Phase, RobotState};

/// Why a robot that wanted to move did not (ARCHITECTURE §4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum WaitCause {
    /// Another robot holds the next cell.
    BlockedBy(RobotId),
    /// The robot lost a priority conflict to another robot: it gave way to a
    /// higher-priority robot that pushed it, or that robot took the cell it
    /// wanted (PIBT, see [`crate::pibt`]).
    Yielded(RobotId),
    /// An aisle block or a dead robot is in the way *(CP2)*.
    CellBlocked(FaultId),
    /// The robot is at its chute but the chute is swapping bags (or, from
    /// CP2, jammed).
    ChuteUnavailable(ChuteId),
    /// The station the robot is heading to is busy loading another robot.
    StationBusy(StationId),
}

/// The state of a chute, reported when it changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChuteState {
    /// Accepting parcels.
    Open,
    /// The bag is full and waiting for a swap. Not used yet: a swap starts
    /// as soon as the bag fills, so chutes go straight to `Swapping`.
    Full,
    /// A bag swap is in progress; the chute accepts nothing.
    Swapping,
    /// Jammed by a fault *(CP2)*.
    Jammed,
}

/// One thing that happened in the simulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A parcel arrived at a station and joined its queue (phase `Queued`).
    ParcelArrived {
        /// When.
        tick: Tick,
        /// The new parcel.
        parcel: ParcelId,
        /// Where it arrived.
        station: StationId,
        /// Where it must go.
        destination: DestinationId,
    },
    /// A robot was assigned to collect a parcel.
    ParcelAssigned {
        /// When.
        tick: Tick,
        /// The parcel.
        parcel: ParcelId,
        /// The robot sent to collect it.
        robot: RobotId,
        /// The station it waits at.
        station: StationId,
    },
    /// A parcel entered a new phase. Phases tile the parcel's lifetime, so
    /// consecutive `ParcelPhase` events for one parcel are its trace spans.
    ParcelPhase {
        /// When the new phase started.
        tick: Tick,
        /// The parcel.
        parcel: ParcelId,
        /// The phase it entered.
        phase: Phase,
    },
    /// A robot changed state.
    RobotState {
        /// When.
        tick: Tick,
        /// The robot.
        robot: RobotId,
        /// Its new state.
        state: RobotState,
    },
    /// A robot with a goal did not get closer to it this tick. `direct` is
    /// the immediate cause; `root` is the end of the wait-for chain. CP1 sets
    /// `root = direct`; CP2 walks the chain (ARCHITECTURE §4). The
    /// `RobotWaited` events of one tick are that tick's complete wait-for
    /// graph: each `BlockedBy(b)` / `Yielded(b)` edge points at a robot whose
    /// own `RobotWaited` (if any) is in the same tick, so the chain can be
    /// rebuilt from the stream alone.
    RobotWaited {
        /// When.
        tick: Tick,
        /// The waiting robot.
        robot: RobotId,
        /// Immediate cause.
        direct: WaitCause,
        /// Root cause.
        root: WaitCause,
    },
    /// A parcel fell into a chute (phase `Dropped`).
    ParcelDropped {
        /// When.
        tick: Tick,
        /// The parcel.
        parcel: ParcelId,
        /// The robot that dropped it.
        robot: RobotId,
        /// The chute it fell into.
        chute: ChuteId,
    },
    /// A parcel went into a chute that does not serve its destination
    /// (a scanner misread at induction). Follows its `ParcelDropped`.
    ParcelMisSorted {
        /// When.
        tick: Tick,
        /// The parcel.
        parcel: ParcelId,
        /// The chute it went into.
        chute: ChuteId,
        /// The destination it should have reached.
        expected: DestinationId,
    },
    /// A chute's fill level or state changed.
    ChuteChanged {
        /// When.
        tick: Tick,
        /// The chute.
        chute: ChuteId,
        /// Parcels in the bag.
        fill: u32,
        /// Its state.
        state: ChuteState,
    },
    /// The watchdog fired: robots have work, but none has made progress
    /// since `since` (a deadlock, livelock or a floor starved by unavailable
    /// chutes). Emitted once per `planner.watchdog_ticks` of stall, so the
    /// stall is visible instead of silent (ADR 0003, robustness).
    Stalled {
        /// When.
        tick: Tick,
        /// The last tick on which any robot made progress.
        since: Tick,
    },
    /// A scheduled fault became active *(CP2)*.
    FaultStarted {
        /// When.
        tick: Tick,
        /// The schedule entry.
        fault: FaultId,
    },
    /// A scheduled fault was cleared *(CP2)*.
    FaultEnded {
        /// When.
        tick: Tick,
        /// The schedule entry.
        fault: FaultId,
    },
}

impl Event {
    /// The tick the event happened on.
    #[must_use]
    pub const fn tick(&self) -> Tick {
        match *self {
            Self::ParcelArrived { tick, .. }
            | Self::ParcelAssigned { tick, .. }
            | Self::ParcelPhase { tick, .. }
            | Self::RobotState { tick, .. }
            | Self::RobotWaited { tick, .. }
            | Self::ParcelDropped { tick, .. }
            | Self::ParcelMisSorted { tick, .. }
            | Self::ChuteChanged { tick, .. }
            | Self::Stalled { tick, .. }
            | Self::FaultStarted { tick, .. }
            | Self::FaultEnded { tick, .. } => tick,
        }
    }
}
