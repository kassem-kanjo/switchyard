//! Lane guidance and guided distance tables (ADR 0003).
//!
//! # Why guidance
//! Plain PIBT ranks moves by shortest-path distance, so robots heading in
//! opposite directions meet head-on in the same aisle lane and spend their
//! time yielding. Giving every directed edge a cost that prefers one
//! direction per lane turns the floor into a grid of one-way-preferred
//! streets, Manhattan style: opposing flows use different lanes and rarely
//! collide. On the Sortation benchmark a hand-made guidance graph roughly
//! tripled PIBT's throughput (10.20 to 29.99; Jiang et al. 2024,
//! arXiv 2404.16162, cited in `docs/research/2026-10-19-robot-coordination.md`).
//!
//! # The lane pattern
//! Lanes alternate by row and by column, one cell wide:
//! - a horizontal move in an **even** row prefers **east**, in an odd row
//!   **west**;
//! - a vertical move in an **even** column prefers **south**, in an odd
//!   column **north**.
//!
//! The layout's aisles are two cells wide (see [`crate::layout`]), so every
//! aisle carries one lane each way. A move with the lane costs 1; against it,
//! `against_flow_cost` (`k`, default 3). Against-flow moves stay legal: the
//! costs only steer, they never forbid, so the graph stays strongly
//! connected and PIBT's reachability argument still holds.
//!
//! Entering a parking cell ([`Layout::parking`]) also costs `k`, so through
//! traffic keeps off the parking lane and parked robots are rarely disturbed.
//!
//! # Distance tables
//! For every goal (each station's induction cell, each chute's set of access
//! cells, and the parking cells as one goal) we run Dijkstra backwards from
//! the goal over the directed costs, once, in [`DistanceTables::build`]. The
//! result is one dense row of `u32` per goal in a single flat array, so a
//! lookup is one multiply and one index. All arithmetic is integer and the
//! distances are unique, so the tables are identical on every platform
//! (ADR 0002).

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::grid::{Cell, CellIdx, DIRS, Grid};
use crate::ids::ChuteId;
use crate::layout::Layout;
use crate::world::Goal;

/// Distance value for cells that cannot reach a goal.
pub const UNREACHABLE: u32 = u32::MAX;

/// Index of each direction in [`DIRS`].
const NORTH: usize = 0;
const EAST: usize = 1;
const SOUTH: usize = 2;
const WEST: usize = 3;

/// The direction a lane prefers for a move leaving `(x, y)` along `dir`
/// (an index into [`DIRS`]): true if the move goes with the lane.
#[must_use]
pub const fn with_flow(x: u16, y: u16, dir: usize) -> bool {
    match dir {
        EAST => y.is_multiple_of(2),
        WEST => y % 2 == 1,
        SOUTH => x.is_multiple_of(2),
        NORTH => x % 2 == 1,
        _ => false,
    }
}

/// Precomputed guided distances from every cell to every goal, plus the
/// directed edge costs they were computed from.
///
/// Station goals measure the cost to reach the induction cell; chute goals
/// the cost to reach the nearest free cell beside the hole (0 means the robot
/// can drop); the parking goal the cost to reach the nearest parking cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DistanceTables {
    cells: usize,
    stations: usize,
    chutes: usize,
    /// Cost of the move out of each cell in each [`DIRS`] direction; 0 when
    /// the move is impossible (wall, chute or off the grid).
    edge: Vec<[u32; 4]>,
    /// One row of `cells` distances per goal: stations, then chutes, then
    /// parking.
    data: Vec<u32>,
}

impl DistanceTables {
    /// Build the tables for `layout` with the given against-flow cost
    /// (1 disables guidance; see the module docs).
    #[must_use]
    pub fn build(layout: &Layout, against_flow_cost: u32) -> Self {
        let g = &layout.grid;
        let cells = g.len();
        let edge = edge_costs(layout, against_flow_cost);
        let stations = layout.stations.len();
        let chutes = layout.chutes.len();
        let goals = stations + chutes + 1;
        let mut data = Vec::with_capacity(cells * goals);
        let mut scratch = Dijkstra::new(cells);
        for &s in &layout.stations {
            scratch.run(g, &edge, &[s], &mut data);
        }
        for c in 0..chutes {
            // Chute counts are bounded by u16 in the generator.
            let id = ChuteId(u16::try_from(c).unwrap_or(u16::MAX));
            scratch.run(g, &edge, &layout.chute_access(id), &mut data);
        }
        scratch.run(g, &edge, &layout.parking, &mut data);
        Self {
            cells,
            stations,
            chutes,
            edge,
            data,
        }
    }

    /// Guided cost from `cell` to `goal`, or [`UNREACHABLE`].
    #[inline]
    #[must_use]
    pub fn get(&self, goal: Goal, cell: CellIdx) -> u32 {
        let row = match goal {
            Goal::Station(s) => s.index(),
            Goal::Chute(c) => self.stations + c.index(),
            Goal::Park => self.stations + self.chutes,
        };
        self.data[row * self.cells + cell as usize]
    }

    /// Cost of moving out of `cell` in direction `dir` (an index into
    /// [`DIRS`]), or 0 if that move is impossible.
    #[inline]
    #[must_use]
    pub fn edge_cost(&self, cell: CellIdx, dir: usize) -> u32 {
        self.edge[cell as usize][dir]
    }
}

/// Directed edge costs for every cell (see the module docs). A move is
/// possible between two cells a robot can stand on (free or induction);
/// whether a particular robot may enter an induction cell is the planner's
/// rule, not the graph's.
fn edge_costs(layout: &Layout, k: u32) -> Vec<[u32; 4]> {
    let g = &layout.grid;
    let mut parking = vec![false; g.len()];
    for &p in &layout.parking {
        parking[p as usize] = true;
    }
    let standable = |c: CellIdx| matches!(g.cell(c), Cell::Free | Cell::Induction(_));
    (0..g.len())
        .map(|i| {
            let from = u32::try_from(i).unwrap_or(u32::MAX);
            let mut costs = [0u32; 4];
            if !standable(from) {
                return costs;
            }
            let p = g.pos(from);
            for (dir, cost) in costs.iter_mut().enumerate() {
                let Some(to) = step(g, from, dir) else {
                    continue;
                };
                if !standable(to) {
                    continue;
                }
                *cost = if with_flow(p.x, p.y, dir) && !parking[to as usize] {
                    1
                } else {
                    k
                };
            }
            costs
        })
        .collect()
}

/// The neighbour of `from` in direction `dir`, if it is on the grid.
#[must_use]
pub fn step(g: &Grid, from: CellIdx, dir: usize) -> Option<CellIdx> {
    let p = g.pos(from);
    let (dx, dy) = DIRS[dir];
    let x = u16::try_from(i32::from(p.x) + dx).ok()?;
    let y = u16::try_from(i32::from(p.y) + dy).ok()?;
    (x < g.width() && y < g.height()).then(|| g.idx(crate::grid::Pos::new(x, y)))
}

/// Reusable Dijkstra scratch space (one allocation for all goals).
struct Dijkstra {
    dist: Vec<u32>,
    source: Vec<bool>,
    heap: BinaryHeap<Reverse<(u32, CellIdx)>>,
}

impl Dijkstra {
    fn new(cells: usize) -> Self {
        Self {
            dist: vec![UNREACHABLE; cells],
            source: vec![false; cells],
            heap: BinaryHeap::new(),
        }
    }

    /// Backwards Dijkstra from `sources`; appends one row to `out`.
    ///
    /// `dist[u]` is the cheapest cost of a path *from* `u` to a source, so
    /// we relax the edge `u -> v` (cost `edge[u][dir]`) when popping `v`.
    /// Paths never pass *through* an induction cell (only the robot assigned
    /// to a station may enter it), so a non-source induction cell gets a
    /// distance, as a robot standing there must know its way out, but is
    /// never expanded. Ties in the heap break by cell index, though the
    /// distances themselves do not depend on pop order.
    fn run(&mut self, g: &Grid, edge: &[[u32; 4]], sources: &[CellIdx], out: &mut Vec<u32>) {
        self.dist.fill(UNREACHABLE);
        self.source.fill(false);
        for &s in sources {
            self.dist[s as usize] = 0;
            self.source[s as usize] = true;
            self.heap.push(Reverse((0, s)));
        }
        while let Some(Reverse((d, v))) = self.heap.pop() {
            if d > self.dist[v as usize] {
                continue;
            }
            if !self.source[v as usize] && !g.is_free(v) {
                continue;
            }
            for dir in 0..4 {
                let Some(u) = step(g, v, dir) else {
                    continue;
                };
                // The move u -> v goes in the opposite direction.
                let cost = edge[u as usize][(dir + 2) % 4];
                if cost == 0 {
                    continue;
                }
                let nd = d.saturating_add(cost);
                if nd < self.dist[u as usize] {
                    self.dist[u as usize] = nd;
                    self.heap.push(Reverse((nd, u)));
                }
            }
        }
        out.extend_from_slice(&self.dist);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FloorConfig;
    use crate::grid::Pos;
    use crate::ids::StationId;
    use crate::layout::{self, generate};

    fn layout() -> Layout {
        generate(&FloorConfig {
            width: 22,
            height: 24,
            stations: 3,
            south_margin: 4,
        })
        .unwrap()
    }

    fn goals(l: &Layout) -> Vec<Goal> {
        let mut v: Vec<Goal> = (0..l.stations.len())
            .map(|s| Goal::Station(StationId(u16::try_from(s).unwrap())))
            .collect();
        v.extend((0..l.chutes.len()).map(|c| Goal::Chute(ChuteId(u16::try_from(c).unwrap()))));
        v
    }

    #[test]
    fn lanes_alternate_by_row_and_column() {
        let l = layout();
        let d = DistanceTables::build(&l, 3);
        let g = &l.grid;
        // Row 2 (even) runs east; row 1 (odd) runs west.
        let a = g.idx(Pos::new(5, 2));
        assert_eq!(d.edge_cost(a, EAST), 1);
        assert_eq!(d.edge_cost(a, WEST), 3);
        let b = g.idx(Pos::new(5, 4));
        assert_eq!(d.edge_cost(b, EAST), 1);
        let c = g.idx(Pos::new(5, 5));
        assert_eq!(d.edge_cost(c, WEST), 1);
        assert_eq!(d.edge_cost(c, EAST), 3);
        // Column 4 (even) runs south; column 5 (odd) runs north.
        let e = g.idx(Pos::new(4, 7));
        assert_eq!(d.edge_cost(e, SOUTH), 1);
        assert_eq!(d.edge_cost(e, NORTH), 3);
        assert_eq!(d.edge_cost(c, NORTH), 1);
        // No move into a wall or a chute.
        let corner = g.idx(Pos::new(1, 2));
        assert_eq!(d.edge_cost(corner, WEST), 0);
        let above_chute = g.idx(Pos::new(3, 2));
        assert_eq!(d.edge_cost(above_chute, SOUTH), 0);
    }

    #[test]
    fn without_guidance_distances_equal_bfs() {
        let l = layout();
        let d = DistanceTables::build(&l, 1);
        for goal in goals(&l) {
            let sources = match goal {
                Goal::Station(s) => vec![l.stations[s.index()]],
                Goal::Chute(c) => l.chute_access(c),
                Goal::Park => unreachable!(),
            };
            let bfs = l.distance_field(&sources);
            for (c, &hops) in bfs.iter().enumerate() {
                let cell = u32::try_from(c).unwrap();
                if l.grid.is_free(cell) {
                    assert_eq!(u32::from(hops), d.get(goal, cell), "{goal:?} at {c}");
                }
            }
        }
    }

    #[test]
    fn guided_distances_are_bounded_by_bfs_and_satisfy_bellman() {
        let l = layout();
        let k = 3;
        let d = DistanceTables::build(&l, k);
        let g = &l.grid;
        for goal in goals(&l) {
            let sources = match goal {
                Goal::Station(s) => vec![l.stations[s.index()]],
                Goal::Chute(c) => l.chute_access(c),
                Goal::Park => unreachable!(),
            };
            let bfs = l.distance_field(&sources);
            for (c, &hops) in bfs.iter().enumerate() {
                let cell = u32::try_from(c).unwrap();
                if !g.is_free(cell) {
                    continue;
                }
                let dist = d.get(goal, cell);
                let hops = u32::from(hops);
                assert!(hops <= dist && dist <= k * hops, "{goal:?} at {c}");
                if dist == 0 {
                    continue;
                }
                // Bellman: some move achieves the distance exactly and none
                // beats it (moves into other stations' cells excluded).
                let best = (0..4)
                    .filter_map(|dir| {
                        let to = step(g, cell, dir)?;
                        let cost = d.edge_cost(cell, dir);
                        let enter = g.is_free(to) || sources.contains(&to);
                        (cost > 0 && enter).then(|| cost + d.get(goal, to))
                    })
                    .min();
                assert_eq!(best, Some(dist), "{goal:?} at {c}");
            }
        }
    }

    #[test]
    fn parking_goal_is_zero_on_parking_cells_and_entering_costs_k() {
        let l = layout();
        let d = DistanceTables::build(&l, 3);
        assert!(!l.parking.is_empty());
        for &p in &l.parking {
            assert_eq!(d.get(Goal::Park, p), 0);
        }
        // Row 2 runs east, so (w - 3, 2) -> (w - 2, 2) is with the lane, but
        // (w - 2, 2) is a parking cell, so the move costs k.
        let g = &l.grid;
        let inner = g.idx(Pos::new(g.width() - 3, 2));
        assert_eq!(d.edge_cost(inner, EAST), 3);
        assert_eq!(d.get(Goal::Park, inner), 3);
        // Every free cell can reach parking.
        for c in 0..g.len() {
            let cell = u32::try_from(c).unwrap();
            if g.is_free(cell) {
                assert_ne!(d.get(Goal::Park, cell), UNREACHABLE);
            }
        }
        // The layout's BFS still agrees that parking is reachable.
        let bfs = l.distance_field(&l.parking);
        assert!(bfs.iter().any(|&x| x != layout::UNREACHABLE));
    }
}
