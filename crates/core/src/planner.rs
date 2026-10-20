//! Motion planning: deciding every robot's next cell each tick.
//!
//! The planner sits behind the [`Planner`] trait so the tick loop does not
//! depend on the algorithm. The default is guided PIBT
//! ([`crate::pibt::PibtPlanner`], ADR 0003). The contract every planner must
//! keep:
//!
//! - **No vertex conflicts:** after the step, no two robots share a cell.
//! - **No swaps:** two robots never exchange cells in one tick. (Rotations of
//!   three or more robots, each entering a cell vacated in the same tick, are
//!   allowed.)
//! - **One move per tick:** `next[r]` is `pos[r]` or a 4-neighbour of it.
//! - **Fixed robots stay:** a robot with `movable[r] == false` (loading,
//!   dropping, waiting at a chute) keeps its cell.
//! - **Induction cells** are entered only by a robot whose goal is that
//!   station, so stations never become through-traffic choke points.
//! - **Causes:** every movable robot that did not get closer to its goal
//!   (by guided distance; robots already at their goal excepted), or that
//!   another robot tried to push and that could not move, gets a
//!   [`WaitCause`] in `cause[r]`; no other robot does. `BlockedBy(b)` names a
//!   robot that stayed put, `Yielded(b)` one that moved. Recording pushed
//!   robots too keeps every wait-for chain walkable through idle and parked
//!   robots. Planners report only robot-to-robot causes; the sim refines
//!   them with what it knows about stations and chutes (ARCHITECTURE §4).
//! - **Determinism:** the result depends only on the input (ADR 0002).
//!
//! [`crate::sim::Sim`] checks the first three in debug builds.

use std::fmt;

use crate::Tick;
use crate::events::WaitCause;
use crate::grid::{Cell, CellIdx};
use crate::hash::Fnv1a;
use crate::layout::Layout;
use crate::world::Goal;

pub use crate::guidance::DistanceTables;

/// Everything a planner may read when planning one tick.
#[derive(Clone, Copy, Debug)]
pub struct PlanInput<'a> {
    /// The floor.
    pub layout: &'a Layout,
    /// Guided distances to every goal, and the edge costs behind them.
    pub distances: &'a DistanceTables,
    /// The tick being planned (planners may use it for deterministic
    /// tie-break rotation).
    pub tick: Tick,
    /// Current cell of each robot.
    pub pos: &'a [CellIdx],
    /// Where each movable robot wants to go; `None` for a robot with nothing
    /// to do (it stays unless pushed).
    pub goal: &'a [Option<Goal>],
    /// False for robots that must not move this tick (loading, dropping,
    /// waiting at a chute).
    pub movable: &'a [bool],
    /// Urgency of each robot: higher plans first. The sim passes the ticks
    /// since the robot's current goal was issued or last reached.
    pub priority: &'a [u32],
}

/// A planner's decision for one tick.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanOutput {
    /// Next cell of each robot.
    pub next: Vec<CellIdx>,
    /// For each robot that wanted to get closer to its goal but did not, or
    /// was pushed and could not move: why (`BlockedBy` or `Yielded`). These
    /// are the wait-for edges of the tick (ARCHITECTURE §4).
    pub cause: Vec<Option<WaitCause>>,
}

/// A multi-robot motion planner. See the module docs for the contract.
pub trait Planner: Send + fmt::Debug {
    /// Plan one tick. `out` is overwritten.
    fn plan(&mut self, input: &PlanInput<'_>, out: &mut PlanOutput);

    /// Feed any planner state that affects future decisions into the state
    /// hash, so the hash covers the whole sim (ADR 0002 §5).
    fn hash_state(&self, h: &mut Fnv1a);
}

/// May a robot with `goal` stand on `cell`? Open floor, or the induction
/// cell of the station it is heading to.
#[must_use]
pub fn enterable(layout: &Layout, goal: Option<Goal>, cell: CellIdx) -> bool {
    match layout.grid.cell(cell) {
        Cell::Free => true,
        Cell::Induction(s) => goal == Some(Goal::Station(s)),
        Cell::Wall | Cell::Chute(_) => false,
    }
}
