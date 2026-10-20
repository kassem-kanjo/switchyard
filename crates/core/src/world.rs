//! Entity storage: robots, parcels, chutes and stations as struct-of-arrays.
//!
//! ARCHITECTURE §3: entities are indexed by dense ids ([`crate::ids`]) and
//! stored column-wise. That keeps the tick loop cache-friendly at 500 robots,
//! gives a natural deterministic iteration order (by id, ADR 0002 §4) and
//! makes the canonical state hash a straight walk over the columns.

use std::collections::VecDeque;

use crate::Tick;
use crate::events::ChuteState;
use crate::grid::CellIdx;
use crate::ids::{ChuteId, DestinationId, ParcelId, RobotId, StationId};

/// What a robot is doing (ARCHITECTURE §3). The `u8` value is part of the
/// viewer snapshot format, so it is stable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum RobotState {
    /// No task.
    Idle = 0,
    /// Driving empty to an induction station to collect a claimed parcel.
    ToInduction = 1,
    /// On the induction cell while the parcel is placed on it.
    Loading = 2,
    /// Carrying a parcel to its chute.
    ToChute = 3,
    /// Beside the chute, tipping the parcel in.
    Dropping = 4,
    /// Broken down by a `robot-failure` fault: it holds its cell (and any
    /// parcel) until repaired, then resumes its previous state.
    Failed = 5,
}

/// A phase of a parcel's life (ARCHITECTURE §3). The phases tile the
/// parcel's lifetime exactly; the trace builder turns them into spans
/// (ADR 0004).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Phase {
    /// Waiting at the station with no robot assigned.
    Queued = 0,
    /// A robot is assigned and driving to the station.
    AwaitingRobot = 1,
    /// Being placed on the robot.
    Loading = 2,
    /// On a robot that is making progress towards the chute.
    Travelling = 3,
    /// On a robot that could not move this tick.
    Blocked = 4,
    /// At the chute: the drop itself, plus any wait for the chute to become
    /// available (bag swaps and jams arrive in CP1/CP2).
    ChuteWait = 5,
    /// In the chute. Terminal.
    Dropped = 6,
}

/// Where a robot is heading.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Goal {
    /// The induction cell of a station.
    Station(StationId),
    /// Any free cell adjacent to a chute.
    Chute(ChuteId),
    /// Any parking cell (an idle robot getting out of the way).
    Park,
}

/// Robot columns, indexed by [`RobotId`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Robots {
    /// Current cell.
    pub pos: Vec<CellIdx>,
    /// Current state.
    pub state: Vec<RobotState>,
    /// Parcel claimed or carried, if any.
    pub parcel: Vec<Option<ParcelId>>,
    /// Current navigation goal, if any.
    pub goal: Vec<Option<Goal>>,
    /// Ticks left in a timed state (`Loading`, `Dropping`).
    pub timer: Vec<u16>,
    /// Consecutive ticks the robot recorded a wait cause.
    pub waited: Vec<u32>,
    /// Tick the robot's current goal was issued or last reached. PIBT's
    /// priority is the time since (Okumura et al.).
    pub goal_since: Vec<Tick>,
}

impl Robots {
    /// Number of robots.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pos.len()
    }

    /// True if there are no robots.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pos.is_empty()
    }

    /// All robot ids in ascending order.
    pub fn ids(&self) -> impl Iterator<Item = RobotId> + use<> {
        // Robot counts are validated to fit in u32 by Sim::new.
        #[allow(clippy::cast_possible_truncation)]
        (0..self.len() as u32).map(RobotId)
    }
}

/// Parcel columns, indexed by [`ParcelId`]. Parcels are never removed, so a
/// parcel id stays valid for the whole run (traces refer to it).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parcels {
    /// Station where the parcel was inducted.
    pub station: Vec<StationId>,
    /// Arrival order of the parcel at its station (0 for the station's
    /// first parcel). For a volume-surge parcel, its order among that
    /// surge's parcels at the station.
    pub ordinal: Vec<u32>,
    /// Key of the parcel's random draws (destination, scanner), taken from
    /// its station's streams at this key: the ordinal for regular arrivals,
    /// a key in the surge's own range for surge arrivals. A parcel's draws
    /// therefore never depend on other parcels or on other faults.
    pub key: Vec<u64>,
    /// Sort destination.
    pub destination: Vec<DestinationId>,
    /// Tick of arrival at the station.
    pub arrived: Vec<Tick>,
    /// Current phase.
    pub phase: Vec<Phase>,
    /// Tick the current phase started.
    pub phase_since: Vec<Tick>,
    /// Robot assigned to the parcel, if any.
    pub robot: Vec<Option<RobotId>>,
    /// Chute the scanner sent it to, once loaded (wrong after a misread).
    pub target: Vec<Option<ChuteId>>,
    /// Chute it was dropped into, once dropped.
    pub chute: Vec<Option<ChuteId>>,
}

impl Parcels {
    /// Number of parcels that have ever arrived.
    #[must_use]
    pub fn len(&self) -> usize {
        self.phase.len()
    }

    /// True if no parcel has arrived yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.phase.is_empty()
    }
}

/// Chute columns, indexed by [`ChuteId`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chutes {
    /// Hole cell.
    pub cell: Vec<CellIdx>,
    /// Parcels in the bag.
    pub fill: Vec<u32>,
    /// Open, swapping or jammed.
    pub state: Vec<ChuteState>,
    /// Ticks left in a bag swap.
    pub timer: Vec<u32>,
    /// Robots dropping into the chute right now. A drop starts only if
    /// `fill + incoming < capacity`, so a bag never overflows.
    pub incoming: Vec<u16>,
}

/// Station columns, indexed by [`StationId`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stations {
    /// Induction cell.
    pub cell: Vec<CellIdx>,
    /// Parcels waiting at the station (not yet on a robot), oldest first.
    pub queue: Vec<VecDeque<ParcelId>>,
    /// Robots assigned to this station and not yet finished loading.
    pub inbound: Vec<u16>,
    /// Regular (non-surge) parcels that have arrived at this station so far:
    /// the next regular parcel's ordinal.
    pub arrived: Vec<u32>,
}

/// All entity state of a running simulation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct World {
    /// Robots.
    pub robots: Robots,
    /// Parcels.
    pub parcels: Parcels,
    /// Chutes.
    pub chutes: Chutes,
    /// Induction stations.
    pub stations: Stations,
    /// The chute serving each destination, indexed by [`DestinationId`]
    /// (fixed at start by the configured mapping strategy).
    pub destination_chute: Vec<ChuteId>,
}
