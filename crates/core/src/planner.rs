//! Motion planning: deciding every robot's next cell each tick.
//!
//! The planner sits behind the [`Planner`] trait so CP1 can swap the V0
//! [`GreedyPlanner`] for guided PIBT (ADR 0003) without touching the tick
//! loop. The contract every planner must keep:
//!
//! - **No vertex conflicts:** after the step, no two robots share a cell.
//! - **No swaps:** two robots never exchange cells in one tick.
//! - **One move per tick:** `next[r]` is `pos[r]` or a 4-neighbour of it.
//! - **Induction cells** are entered only by a robot whose goal is that
//!   station, so stations never become through-traffic choke points.
//! - **Determinism:** the result depends only on the input and the planner's
//!   own seeded stream (ADR 0002).
//!
//! [`crate::sim::Sim`] checks the first three in debug builds.

use std::fmt;

use crate::grid::{Cell, CellIdx};
use crate::hash::Fnv1a;
use crate::ids::RobotId;
use crate::layout::{Layout, UNREACHABLE};
use crate::rng::{Stream, StreamId};
use crate::world::Goal;

/// Precomputed move distances from every cell to every goal.
///
/// Station goals measure distance to the induction cell; chute goals measure
/// distance to the nearest free cell beside the hole (distance 0 means the
/// robot can drop). V0 uses unweighted BFS; CP1 replaces it with Dijkstra
/// over lane-weighted edges (ADR 0003).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DistanceTables {
    cells: usize,
    stations: usize,
    data: Vec<u16>,
}

impl DistanceTables {
    /// Build the tables for `layout`: one field per station, then one per
    /// chute.
    #[must_use]
    pub fn build(layout: &Layout) -> Self {
        let cells = layout.grid.len();
        let stations = layout.stations.len();
        let mut data = Vec::with_capacity(cells * (stations + layout.chutes.len()));
        for &s in &layout.stations {
            data.extend(layout.distance_field(&[s]));
        }
        for c in 0..layout.chutes.len() {
            // Chute counts are bounded by u16 in the generator.
            #[allow(clippy::cast_possible_truncation)]
            let access = layout.chute_access(crate::ids::ChuteId(c as u16));
            data.extend(layout.distance_field(&access));
        }
        Self {
            cells,
            stations,
            data,
        }
    }

    /// Moves from `cell` to `goal`, or [`UNREACHABLE`].
    #[must_use]
    pub fn get(&self, goal: Goal, cell: CellIdx) -> u16 {
        let field = match goal {
            Goal::Station(s) => s.index(),
            Goal::Chute(c) => self.stations + c.index(),
        };
        self.data[field * self.cells + cell as usize]
    }
}

/// Everything a planner may read when planning one tick.
#[derive(Clone, Copy, Debug)]
pub struct PlanInput<'a> {
    /// The floor.
    pub layout: &'a Layout,
    /// Distances to every goal.
    pub distances: &'a DistanceTables,
    /// Current cell of each robot.
    pub pos: &'a [CellIdx],
    /// Goal of each robot that wants to move this tick (`None` for robots
    /// that are idle, loading or dropping).
    pub goal: &'a [Option<Goal>],
    /// Consecutive ticks each robot has failed to move.
    pub waited: &'a [u16],
    /// Robots with no task. Only these may be asked to step aside; a robot
    /// that is loading or dropping must stay put.
    pub idle: &'a [bool],
}

/// A planner's decision for one tick.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanOutput {
    /// Next cell of each robot.
    pub next: Vec<CellIdx>,
    /// For each robot that wanted to move but did not: the robot holding the
    /// cell it wanted. Input to causal wait attribution (ARCHITECTURE §4).
    pub blocked_by: Vec<Option<RobotId>>,
}

/// A multi-robot motion planner. See the module docs for the contract.
pub trait Planner: Send + fmt::Debug {
    /// Plan one tick. `out` is overwritten.
    fn plan(&mut self, input: &PlanInput<'_>, out: &mut PlanOutput);

    /// Feed any planner state that affects future decisions into the state
    /// hash, so the hash covers the whole sim (ADR 0002 §5).
    fn hash_state(&self, h: &mut Fnv1a);
}

const NO_ROBOT: u32 = u32::MAX;

/// V0 planner: each robot, in id order, takes the first free neighbour that
/// brings it closer to its goal; a robot stuck for `patience` ticks makes a
/// random escape move. There is no lookahead and no priority inheritance, so
/// congestion is handled poorly; CP1 replaces this with guided PIBT
/// (ADR 0003).
///
/// Collision freedom comes from a single occupancy table updated as each
/// robot commits: a robot may enter a cell only if no robot is in it *now*
/// (after earlier robots' moves). A robot can follow one that already moved
/// out this tick, but swaps and rotations are impossible because the first
/// robot of any cycle sees its target occupied.
#[derive(Clone, Debug)]
pub struct GreedyPlanner {
    rng: Stream,
    patience: u16,
    /// Idle robots asked to step aside because they blocked someone.
    nudge: Vec<bool>,
    occ: Vec<u32>,
    scratch: Vec<CellIdx>,
}

impl GreedyPlanner {
    /// Default ticks a robot waits before an escape move.
    pub const DEFAULT_PATIENCE: u16 = 3;

    /// A planner drawing escape moves from the `(seed, PLANNER)` stream.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            rng: Stream::new(seed, StreamId::PLANNER),
            patience: Self::DEFAULT_PATIENCE,
            nudge: Vec::new(),
            occ: Vec::new(),
            scratch: Vec::new(),
        }
    }

    /// May robot with `goal` enter `cell`?
    fn enterable(layout: &Layout, goal: Option<Goal>, cell: CellIdx) -> bool {
        match layout.grid.cell(cell) {
            Cell::Free => true,
            Cell::Induction(s) => goal == Some(Goal::Station(s)),
            Cell::Wall | Cell::Chute(_) => false,
        }
    }

    /// Commit robot `r` moving from `from` to `to`.
    fn commit(&mut self, out: &mut PlanOutput, r: usize, from: CellIdx, to: CellIdx) {
        self.occ[from as usize] = NO_ROBOT;
        // r < robot count, which fits in u32.
        #[allow(clippy::cast_possible_truncation)]
        let id = r as u32;
        self.occ[to as usize] = id;
        out.next[r] = to;
    }

    /// Pick a uniformly random free, enterable neighbour of `here`.
    fn random_free_neighbour(
        &mut self,
        layout: &Layout,
        goal: Option<Goal>,
        here: CellIdx,
    ) -> Option<CellIdx> {
        self.scratch.clear();
        for n in layout.grid.neighbours(here) {
            if Self::enterable(layout, goal, n) && self.occ[n as usize] == NO_ROBOT {
                self.scratch.push(n);
            }
        }
        if self.scratch.is_empty() {
            return None;
        }
        let k = self.rng.below(self.scratch.len() as u64);
        // k < scratch.len().
        #[allow(clippy::cast_possible_truncation)]
        Some(self.scratch[k as usize])
    }
}

impl Planner for GreedyPlanner {
    fn plan(&mut self, input: &PlanInput<'_>, out: &mut PlanOutput) {
        let n = input.pos.len();
        let layout = input.layout;
        self.occ.clear();
        self.occ.resize(layout.grid.len(), NO_ROBOT);
        self.nudge.resize(n, false);
        for (r, &p) in input.pos.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            let id = r as u32;
            self.occ[p as usize] = id;
        }
        out.next.clear();
        out.next.extend_from_slice(input.pos);
        out.blocked_by.clear();
        out.blocked_by.resize(n, None);
        let mut next_nudge = vec![false; n];

        for r in 0..n {
            let here = input.pos[r];
            let Some(goal) = input.goal[r] else {
                // Idle robots step aside only when they blocked someone.
                if self.nudge[r]
                    && input.idle[r]
                    && let Some(to) = self.random_free_neighbour(layout, None, here)
                {
                    self.commit(out, r, here, to);
                }
                continue;
            };
            let d_here = input.distances.get(goal, here);
            if d_here == 0 || d_here == UNREACHABLE {
                continue;
            }
            let mut blocker = None;
            let mut moved = false;
            for c in layout.grid.neighbours(here) {
                if !Self::enterable(layout, Some(goal), c) || input.distances.get(goal, c) >= d_here
                {
                    continue;
                }
                let occupant = self.occ[c as usize];
                if occupant == NO_ROBOT {
                    self.commit(out, r, here, c);
                    moved = true;
                    break;
                }
                blocker.get_or_insert(RobotId(occupant));
            }
            if !moved
                && input.waited[r] >= self.patience
                && let Some(to) = self.random_free_neighbour(layout, Some(goal), here)
            {
                self.commit(out, r, here, to);
                moved = true;
            }
            if !moved {
                out.blocked_by[r] = blocker;
                if let Some(b) = blocker
                    && input.idle[b.index()]
                {
                    next_nudge[b.index()] = true;
                }
            }
        }
        self.nudge = next_nudge;
    }

    fn hash_state(&self, h: &mut Fnv1a) {
        h.write_u128(self.rng.word_pos());
        h.write_u16(self.patience);
        for &b in &self.nudge {
            h.write_u8(u8::from(b));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FloorConfig;
    use crate::layout::generate;

    fn layout() -> Layout {
        generate(&FloorConfig {
            width: 16,
            height: 16,
            stations: 2,
            south_margin: 3,
        })
        .unwrap()
    }

    #[test]
    fn robots_in_a_line_follow_without_collision() {
        let l = layout();
        let d = DistanceTables::build(&l);
        let g = &l.grid;
        use crate::grid::Pos;
        // Three robots in a row on y = 1 heading west towards chute 0's
        // access cells; the leader moves and the others follow.
        let pos = vec![
            g.idx(Pos::new(5, 1)),
            g.idx(Pos::new(6, 1)),
            g.idx(Pos::new(7, 1)),
        ];
        let goal = vec![Some(Goal::Chute(crate::ids::ChuteId(0))); 3];
        let waited = vec![0; 3];
        let idle = vec![false; 3];
        let mut p = GreedyPlanner::new(1);
        let mut out = PlanOutput::default();
        p.plan(
            &PlanInput {
                layout: &l,
                distances: &d,
                pos: &pos,
                goal: &goal,
                waited: &waited,
                idle: &idle,
            },
            &mut out,
        );
        let mut cells = out.next.clone();
        cells.sort_unstable();
        cells.dedup();
        assert_eq!(cells.len(), 3, "vertex conflict: {:?}", out.next);
    }
}
