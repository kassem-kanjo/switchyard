//! The event stream: the core's only output (ADR 0004).
//!
//! Traces, metrics, the viewer and post-mortems are all pure functions of
//! this stream, so a new analysis never needs a change to the sim. Every
//! event carries the tick it happened on. Within a tick, events appear in the
//! fixed phase order documented in [`crate::sim`], and within a phase in id
//! order, so the stream itself is deterministic (ADR 0002).
//!

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
    /// Cells closed by a fault are in the way. As a direct cause: the fault's
    /// closed cells (an aisle block, or a dead robot's cell) cut the robot
    /// off from its goal; the id is the fault that did it (see
    /// [`crate::faults`] for how a robot cut off by several faults is
    /// blamed). As a root: the robot's wait-for chain ends at a robot that
    /// failure `f` stopped. In [`Event::ParcelDelayed`] it also covers detour
    /// ticks and parcels queued at a station that fault cut off.
    CellBlocked(FaultId),
    /// The robot is at its chute but the chute is swapping bags or jammed.
    /// When the chute is jammed, the root of the chain is `Fault(jam)`; a
    /// bag swap is its own root.
    ChuteUnavailable(ChuteId),
    /// The station the robot is heading to is busy loading another robot.
    StationBusy(StationId),
    /// Only ever a *root*: the robot's wait-for chain ends in a deadlock, a
    /// cycle of `BlockedBy` edges (robots that all stayed put, each waiting
    /// on the next) that has persisted for `planner.deadlock_ticks`
    /// consecutive ticks ([`Event::DeadlockDetected`]). The id is the
    /// cycle's canonical member, its lowest robot id. A younger cycle is a
    /// transient head-on meeting and its chains are rooted at
    /// `BlockedBy(canonical member)` instead.
    Cycle(RobotId),
    /// A fault holds the robot or parcel directly: the robot has failed
    /// (direct and root of a failed robot's `RobotWaited`), its chute is
    /// jammed (root of `ChuteUnavailable`), its station is in an induction
    /// outage, or its parcel is queued behind a volume surge's parcels
    /// ([`Event::ParcelDelayed`]).
    Fault(FaultId),
}

impl WaitCause {
    /// The fault this cause blames, if any (`Fault` or `CellBlocked`).
    #[must_use]
    pub const fn fault(self) -> Option<FaultId> {
        match self {
            Self::Fault(f) | Self::CellBlocked(f) => Some(f),
            _ => None,
        }
    }
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
    /// Jammed by a fault: accepts nothing, and a bag swap in progress is
    /// paused until the jam clears.
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
    /// A robot was assigned to collect a parcel. A claim can be given back:
    /// when closed cells cut the robot off from the station, the parcel
    /// returns to `Queued` (a `ParcelPhase` event) and is later assigned
    /// again, so a parcel can have several `ParcelAssigned` events.
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
    /// A robot with work (heading to a station or a chute) did not get
    /// closer to its goal this tick. `direct` is the immediate cause; `root`
    /// is the end of the wait-for chain, walked over the tick's complete
    /// graph (`Sim::wait_for()`, idle and parking robots included) by
    /// [`crate::causality`] (ARCHITECTURE §4). The events themselves are the
    /// graph *restricted to robots with work*: a `BlockedBy(b)` may point at
    /// an idle or parking robot that has no event of its own.
    ///
    /// Roots: `BlockedBy(b)` for a robot `b` with no cause of its own
    /// (loading, dropping, at its goal, or idle), or the canonical member of
    /// a transient cycle; `Yielded(b)` (`b` won the cell and moved on);
    /// `CellBlocked(f)` (closed cells cut the robot off, or the chain ends
    /// at a robot that failure `f` stopped); `Fault(f)` (the robot failed, or
    /// its chute is jammed by `f`); `ChuteUnavailable` (a bag swap);
    /// `StationBusy`; or `Cycle(m)` for a chain that ends in a deadlock.
    ///
    /// A failed robot that holds a parcel (claimed or carried) emits
    /// `RobotWaited { direct: Fault(f), root: Fault(f) }` every tick it is
    /// down.
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
    /// A fault cost this parcel this tick (one event per parcel per tick at
    /// most, after the tick's `RobotWaited` events). The invariant: every
    /// tick of a parcel's life that a fault delayed carries exactly one
    /// `ParcelDelayed` whose `root` names that fault (`root.fault()` is
    /// `Some`), whatever the parcel's phase. Emitted when:
    /// - the parcel's robot (assigned or carrying) waited this tick and its
    ///   root is a fault (`Fault` or `CellBlocked`): `cause` and `root` are
    ///   the robot's `RobotWaited` direct and root (a failed robot, a jammed
    ///   chute, a cut-off goal, a chain ending at a dead robot), in any phase
    ///   from `AwaitingRobot` to `ChuteWait`;
    /// - the parcel's chute is jammed: every tick of the jam, whatever its
    ///   robot does (robots waiting at a chute are pushed aside and come
    ///   back): `cause: ChuteUnavailable(c)`, `root: Fault(jam)`;
    /// - the parcel's robot moved but on a detour: it got closer to its goal
    ///   only on the faulted floor, not by the healthy distances, because a
    ///   fault's closed cells lengthen its route (`CellBlocked(f)`);
    /// - the parcel is `Queued` at a station in an induction outage
    ///   (`Fault(outage)`), at a station whose induction cell a failed robot
    ///   blocks (`cause: StationBusy(s)`, `root: CellBlocked(failure)`, until
    ///   operators clear the robot, `recovery.clear_after_ticks`), at a
    ///   station closed cells cut off from the fleet (`CellBlocked(f)`), or
    ///   behind a volume surge's parcels in the
    ///   station queue (`Fault(surge)`, which can outlast the surge itself
    ///   while its backlog drains).
    ///
    /// `Blocked` ticks also have the robot's `RobotWaited`; for fault blame
    /// this event is the parcel's single source.
    ParcelDelayed {
        /// When.
        tick: Tick,
        /// The delayed parcel.
        parcel: ParcelId,
        /// Immediate cause.
        cause: WaitCause,
        /// Root cause; always names a fault.
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
    /// The per-robot watchdog fired: a robot with work recorded a wait cause
    /// on `waited` consecutive ticks (`planner.watchdog_ticks`). Emitted
    /// once per episode: the robot must progress before it can fire again.
    /// This catches local deadlocks and livelocks that the floor-wide
    /// `Stalled` misses while the rest of the floor keeps sorting.
    RobotStalled {
        /// When.
        tick: Tick,
        /// The stuck robot.
        robot: RobotId,
        /// Consecutive ticks it has waited.
        waited: u32,
    },
    /// A wait-for cycle persisted for `planner.deadlock_ticks` consecutive
    /// ticks: a deadlock (research §2.3: a wait-for cycle iff deadlock).
    /// Emitted once per member robot, in cycle order starting from the
    /// canonical (lowest-id) member, and once per episode: the cycle must
    /// break before it can be reported again. Group the events of one tick
    /// by `cycle` to get the member set. (One event per member keeps
    /// [`Event`] `Copy`.)
    DeadlockDetected {
        /// When the persistence threshold was reached.
        tick: Tick,
        /// A member of the cycle.
        robot: RobotId,
        /// The cycle's canonical member (its lowest robot id).
        cycle: RobotId,
        /// The first tick of the episode on which the cycle was present.
        since: Tick,
    },
    /// Operators pushed a robot that failed on a station's induction cell to
    /// a free cell (`recovery.clear_after_ticks` after its failure), so the
    /// station can work again. The robot stays failed and keeps its parcel.
    RobotCleared {
        /// When.
        tick: Tick,
        /// The failed robot.
        robot: RobotId,
        /// The station it was blocking.
        station: StationId,
    },
    /// Robots joined the fleet (`fleet.additions`): `added` new idle robots
    /// with the next ids. Fewer than `requested` when the floor had no more
    /// free cells (open floor, not closed by a fault, not occupied). The
    /// shortfall is deterministic: a replay of the same scenario reproduces
    /// it.
    RobotsAdded {
        /// When.
        tick: Tick,
        /// Robots the addition asked for.
        requested: u32,
        /// Robots placed.
        added: u32,
    },
    /// A scheduled fault became active. Its effects (a robot entering
    /// `Failed`, a chute `Jammed`) follow in the same tick.
    FaultStarted {
        /// When.
        tick: Tick,
        /// The schedule entry.
        fault: FaultId,
    },
    /// A scheduled fault was cleared: its healthy counterpart is restored
    /// (see [`crate::faults`]).
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
            | Self::ParcelDelayed { tick, .. }
            | Self::ParcelDropped { tick, .. }
            | Self::ParcelMisSorted { tick, .. }
            | Self::ChuteChanged { tick, .. }
            | Self::Stalled { tick, .. }
            | Self::RobotStalled { tick, .. }
            | Self::DeadlockDetected { tick, .. }
            | Self::RobotCleared { tick, .. }
            | Self::RobotsAdded { tick, .. }
            | Self::FaultStarted { tick, .. }
            | Self::FaultEnded { tick, .. } => tick,
        }
    }
}
