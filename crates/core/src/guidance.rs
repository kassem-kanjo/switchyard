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
use crate::ids::{ChuteId, StationId};
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
        Self::build_with_closed(layout, against_flow_cost, &[])
    }

    /// Build the tables with some cells closed to traffic (an aisle block or
    /// a dead robot, [`crate::faults`]): no edge enters a closed cell, so
    /// every route goes around it, and goals that can only be reached
    /// through closed cells become [`UNREACHABLE`]. Edges *out of* a closed
    /// cell remain, so a robot caught inside can leave. `closed` is indexed
    /// by cell; an empty slice closes nothing (the same tables as
    /// [`DistanceTables::build`]).
    #[must_use]
    pub fn build_with_closed(layout: &Layout, against_flow_cost: u32, closed: &[bool]) -> Self {
        let g = &layout.grid;
        let cells = g.len();
        let edge = closed_edge_costs(layout, against_flow_cost, closed);
        let stations = layout.stations.len();
        let chutes = layout.chutes.len();
        let rows = goal_sources(layout);
        let mut data = vec![UNREACHABLE; cells * rows.len()];
        let mut scratch = Dijkstra::new(cells);
        for (sources, row) in rows.iter().zip(data.chunks_exact_mut(cells)) {
            scratch.run_into(g, &edge, sources, row);
        }
        Self {
            cells,
            stations,
            chutes,
            edge,
            data,
        }
    }

    /// One goal's row of [`DistanceTables::build_with_closed`], computed
    /// alone: the guided cost from every cell to `goal` with `closed` cells
    /// closed. One Dijkstra instead of one per goal, for questions such as
    /// "would this robot reach its goal if only that fault were removed?"
    /// (fault blame, [`crate::faults`]).
    #[must_use]
    pub fn goal_row(
        layout: &Layout,
        against_flow_cost: u32,
        goal: Goal,
        closed: &[bool],
    ) -> Vec<u32> {
        let edge = closed_edge_costs(layout, against_flow_cost, closed);
        let mut row = vec![UNREACHABLE; layout.grid.len()];
        Dijkstra::new(layout.grid.len()).run_into(
            &layout.grid,
            &edge,
            &goal_source(layout, goal),
            &mut row,
        );
        row
    }

    /// The tables for the closed set `new`, derived from `self` (the tables
    /// for the closed set `old`; both masks have one flag per cell) by
    /// repairing each goal row locally instead of recomputing it (dynamic
    /// shortest paths after Ramalingam and Reps): newly closed cells first
    /// (the deletion repair, on the graph with both closed sets closed),
    /// then newly opened cells (the insertion repair). A row the change
    /// cannot affect costs a few neighbour checks. The result equals
    /// [`DistanceTables::build_with_closed`] for `new` exactly (property
    /// tests check it). Returns the tables and the number of rows that
    /// changed.
    ///
    /// # Panics
    /// If a mask's length is not the number of cells.
    #[must_use]
    pub fn with_closed_changed(
        &self,
        layout: &Layout,
        against_flow_cost: u32,
        old: &[bool],
        new: &[bool],
    ) -> (Self, usize) {
        let g = &layout.grid;
        assert!(
            old.len() == self.cells && new.len() == self.cells,
            "one flag per cell"
        );
        let edge = closed_edge_costs(layout, against_flow_cost, new);
        let closed = changed(old, new, true);
        let opened = changed(old, new, false);
        // The deletion repair runs on the graph with both closed sets
        // closed; without openings, that graph is the new one.
        let union_edge = (!opened.is_empty()).then(|| {
            let union: Vec<bool> = old.iter().zip(new).map(|(&o, &n)| o || n).collect();
            closed_edge_costs(layout, against_flow_cost, &union)
        });
        let mut data = self.data.clone();
        let mut scratch = Dijkstra::new(self.cells);
        let mut changed_rows = 0;
        for (sources, row) in goal_sources(layout)
            .iter()
            .zip(data.chunks_exact_mut(self.cells))
        {
            let mut changed = false;
            if !closed.is_empty() {
                let e = union_edge.as_deref().unwrap_or(&edge);
                changed |= scratch.repair_increase(g, e, sources, &closed, row);
            }
            if !opened.is_empty() {
                changed |= scratch.repair_decrease(g, &edge, sources, &opened, row);
            }
            changed_rows += usize::from(changed);
        }
        let tables = Self {
            cells: self.cells,
            stations: self.stations,
            chutes: self.chutes,
            edge,
            data,
        };
        (tables, changed_rows)
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

/// The cells whose flag becomes `to` between `old` and `new`.
fn changed(old: &[bool], new: &[bool], to: bool) -> Vec<CellIdx> {
    old.iter()
        .zip(new)
        .enumerate()
        .filter(|&(_, (&o, &n))| o != to && n == to)
        .map(|(c, _)| u32::try_from(c).unwrap_or(u32::MAX))
        .collect()
}

/// The source cells of every goal row, in table order: stations, chutes,
/// parking.
fn goal_sources(layout: &Layout) -> Vec<Vec<CellIdx>> {
    let stations = (0..layout.stations.len()).map(|s| {
        // Station counts are u16 in the config.
        Goal::Station(StationId(u16::try_from(s).unwrap_or(u16::MAX)))
    });
    let chutes = (0..layout.chutes.len()).map(|c| {
        // Chute counts are bounded by u16 in the generator.
        Goal::Chute(ChuteId(u16::try_from(c).unwrap_or(u16::MAX)))
    });
    stations
        .chain(chutes)
        .chain([Goal::Park])
        .map(|goal| goal_source(layout, goal))
        .collect()
}

/// The source cells of one goal: a station's induction cell, a chute's
/// access cells, or the parking lane.
fn goal_source(layout: &Layout, goal: Goal) -> Vec<CellIdx> {
    match goal {
        Goal::Station(s) => vec![layout.stations[s.index()]],
        Goal::Chute(c) => layout.chute_access(c),
        Goal::Park => layout.parking.clone(),
    }
}

/// [`edge_costs`] with every edge into a `closed` cell removed (an empty
/// slice closes nothing).
fn closed_edge_costs(layout: &Layout, k: u32, closed: &[bool]) -> Vec<[u32; 4]> {
    let g = &layout.grid;
    let mut edge = edge_costs(layout, k);
    if !closed.is_empty() {
        for (from, costs) in edge.iter_mut().enumerate() {
            let from = u32::try_from(from).unwrap_or(u32::MAX);
            for (dir, cost) in costs.iter_mut().enumerate() {
                if step(g, from, dir).is_some_and(|to| closed[to as usize]) {
                    *cost = 0;
                }
            }
        }
    }
    edge
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
    /// Cells a repair found affected (cleared after each repair).
    mark: Vec<bool>,
    heap: BinaryHeap<Reverse<(u32, CellIdx)>>,
}

impl Dijkstra {
    fn new(cells: usize) -> Self {
        Self {
            dist: vec![UNREACHABLE; cells],
            source: vec![false; cells],
            mark: vec![false; cells],
            heap: BinaryHeap::new(),
        }
    }

    /// Repair `d`, a row that was exact before edges into `closed` were
    /// removed, so it is exact for `edge` (the deletion case of dynamic
    /// shortest paths after Ramalingam and Reps): find the cells that lost
    /// every tight edge (transitively), clear them, and re-run Dijkstra over
    /// them alone, seeded from their unaffected neighbours. Returns whether
    /// any distance changed.
    fn repair_increase(
        &mut self,
        g: &Grid,
        edge: &[[u32; 4]],
        sources: &[CellIdx],
        closed: &[CellIdx],
        d: &mut [u32],
    ) -> bool {
        self.mark_sources(sources, true);
        let mut affected: Vec<CellIdx> = Vec::new();
        let mut work: Vec<CellIdx> = closed.to_vec();
        while let Some(x) = work.pop() {
            for dir in 0..4 {
                let Some(u) = step(g, x, dir) else {
                    continue;
                };
                let du = d[u as usize];
                if du == UNREACHABLE || self.source[u as usize] || self.mark[u as usize] {
                    continue;
                }
                let valid = (0..4).any(|k| {
                    let w = edge[u as usize][k];
                    w != 0
                        && step(g, u, k).is_some_and(|v| {
                            !self.mark[v as usize]
                                && self.expandable(g, v)
                                && d[v as usize] != UNREACHABLE
                                && du == w.saturating_add(d[v as usize])
                        })
                });
                if !valid {
                    self.mark[u as usize] = true;
                    affected.push(u);
                    work.push(u);
                }
            }
        }
        for &u in &affected {
            d[u as usize] = UNREACHABLE;
        }
        for &u in &affected {
            let best = (0..4)
                .filter_map(|k| {
                    let w = edge[u as usize][k];
                    let v = step(g, u, k)?;
                    (w != 0
                        && !self.mark[v as usize]
                        && self.expandable(g, v)
                        && d[v as usize] != UNREACHABLE)
                        .then(|| w.saturating_add(d[v as usize]))
                })
                .min();
            if let Some(b) = best {
                d[u as usize] = b;
                self.heap.push(Reverse((b, u)));
            }
        }
        self.propagate(g, edge, d, |s, u| s.mark[u as usize]);
        for &u in &affected {
            self.mark[u as usize] = false;
        }
        self.mark_sources(sources, false);
        !affected.is_empty()
    }

    /// Repair `d`, a row that was exact before edges into `opened` were
    /// added, so it is exact for `edge` (the insertion case): re-propagate
    /// from each opened cell, which relaxes the predecessors the new edges
    /// improve and spreads the decrease. Returns whether any distance
    /// changed.
    fn repair_decrease(
        &mut self,
        g: &Grid,
        edge: &[[u32; 4]],
        sources: &[CellIdx],
        opened: &[CellIdx],
        d: &mut [u32],
    ) -> bool {
        self.mark_sources(sources, true);
        for &c in opened {
            let dc = d[c as usize];
            if dc != UNREACHABLE {
                self.heap.push(Reverse((dc, c)));
            }
        }
        let changed = self.propagate(g, edge, d, |_, _| true);
        self.mark_sources(sources, false);
        changed
    }

    /// Dijkstra from the heap's contents over `d`, relaxing predecessors that
    /// `relax` admits; cells that are not expandable are not relaxed through.
    /// Returns whether any distance changed.
    fn propagate(
        &mut self,
        g: &Grid,
        edge: &[[u32; 4]],
        d: &mut [u32],
        relax: impl Fn(&Self, CellIdx) -> bool,
    ) -> bool {
        let mut changed = false;
        while let Some(Reverse((dv, v))) = self.heap.pop() {
            if dv > d[v as usize] || !self.expandable(g, v) {
                continue;
            }
            for dir in 0..4 {
                let Some(u) = step(g, v, dir) else {
                    continue;
                };
                let w = edge[u as usize][(dir + 2) % 4];
                let nd = dv.saturating_add(w);
                if w != 0 && nd < d[u as usize] && relax(self, u) {
                    d[u as usize] = nd;
                    changed = true;
                    self.heap.push(Reverse((nd, u)));
                }
            }
        }
        changed
    }

    /// Can a path pass through `v`? Open floor, or a source of the row.
    fn expandable(&self, g: &Grid, v: CellIdx) -> bool {
        g.is_free(v) || self.source[v as usize]
    }

    fn mark_sources(&mut self, sources: &[CellIdx], on: bool) {
        for &s in sources {
            self.source[s as usize] = on;
        }
    }

    /// Backwards Dijkstra from `sources`, writing one row into `out` (one
    /// entry per cell).
    ///
    /// `dist[u]` is the cheapest cost of a path *from* `u` to a source, so
    /// we relax the edge `u -> v` (cost `edge[u][dir]`) when popping `v`.
    /// Paths never pass *through* an induction cell (only the robot assigned
    /// to a station may enter it), so a non-source induction cell gets a
    /// distance, as a robot standing there must know its way out, but is
    /// never expanded. Ties in the heap break by cell index, though the
    /// distances themselves do not depend on pop order.
    fn run_into(&mut self, g: &Grid, edge: &[[u32; 4]], sources: &[CellIdx], out: &mut [u32]) {
        self.search(g, edge, sources);
        out.copy_from_slice(&self.dist);
    }

    /// Backwards Dijkstra from `sources` into `self.dist`.
    fn search(&mut self, g: &Grid, edge: &[[u32; 4]], sources: &[CellIdx]) {
        self.dist.fill(UNREACHABLE);
        // `source` is all false here: every user clears its flags after.
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
        for &src in sources {
            self.source[src as usize] = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FloorConfig;
    use crate::grid::{Cell, Pos};
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
    fn closed_cells_are_routed_around_and_can_cut_a_goal_off() {
        let l = layout();
        let g = &l.grid;
        let open = DistanceTables::build(&l, 3);
        assert_eq!(DistanceTables::build_with_closed(&l, 3, &[]), open);

        // Close one cell on a shortest route: nobody may step into it, but
        // its own way out stays, and the goal is still reachable around it.
        let goal = Goal::Chute(ChuteId(0));
        let cell = g.idx(Pos::new(3, 2));
        let mut closed = vec![false; g.len()];
        closed[cell as usize] = true;
        let d = DistanceTables::build_with_closed(&l, 3, &closed);
        for dir in 0..4 {
            if let Some(from) = step(g, cell, dir) {
                assert_eq!(d.edge_cost(from, (dir + 2) % 4), 0, "edge into the block");
            }
        }
        assert_eq!(d.get(goal, cell), 0, "a robot inside can still drop");
        let above = g.idx(Pos::new(3, 1));
        assert!(d.get(goal, above) > open.get(goal, above));
        assert_ne!(d.get(goal, above), UNREACHABLE);

        // Close every access cell of the chute: it is cut off.
        let mut closed = vec![false; g.len()];
        for c in l.chute_access(ChuteId(0)) {
            closed[c as usize] = true;
        }
        let d = DistanceTables::build_with_closed(&l, 3, &closed);
        assert_eq!(d.get(goal, above), UNREACHABLE);

        // A single goal row agrees with the full tables.
        let row = DistanceTables::goal_row(&l, 3, goal, &closed);
        let goals = (0..l.stations.len())
            .map(|s| Goal::Station(StationId(u16::try_from(s).unwrap())))
            .chain([goal, Goal::Park]);
        for gl in goals {
            let row_g = DistanceTables::goal_row(&l, 3, gl, &closed);
            for (c, &dist) in row_g.iter().enumerate() {
                let cell = u32::try_from(c).unwrap();
                assert_eq!(dist, d.get(gl, cell), "{gl:?} at {c}");
            }
        }
        assert_eq!(row[above as usize], UNREACHABLE);
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]
        /// Incremental rebuilds equal full rebuilds, through a sequence of
        /// closed sets (cells closing and reopening).
        #[test]
        fn incremental_rebuilds_equal_full_builds(
            steps in proptest::collection::vec(
                proptest::collection::vec(0usize..22 * 24, 0..12),
                1..5,
            )
        ) {
            let l = layout();
            let n = l.grid.len();
            let mut old = vec![false; n];
            let mut tables = DistanceTables::build(&l, 3);
            for cells in steps {
                let mut new = vec![false; n];
                for c in cells {
                    new[c] = l.grid.is_free(u32::try_from(c).unwrap());
                }
                let (inc, _) = tables.with_closed_changed(&l, 3, &old, &new);
                proptest::prop_assert_eq!(&inc, &DistanceTables::build_with_closed(&l, 3, &new));
                tables = inc;
                old = new;
            }
        }
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(48))]
        /// One-way changes take the repair paths: close cells one at a time
        /// (deletion repair), then reopen them in another order (insertion
        /// repair); every step equals a full build.
        #[test]
        fn repairs_equal_full_builds(
            cells in proptest::collection::vec(0usize..22 * 24, 1..10),
            reopen_shift in 0usize..10,
        ) {
            let l = layout();
            let n = l.grid.len();
            let cells: Vec<usize> = cells
                .into_iter()
                .filter(|&c| l.grid.is_free(u32::try_from(c).unwrap()))
                .collect();
            let mut mask = vec![false; n];
            let mut tables = DistanceTables::build(&l, 3);
            let mut order = cells.clone();
            for &c in &cells {
                let mut next = mask.clone();
                next[c] = true;
                let (inc, _) = tables.with_closed_changed(&l, 3, &mask, &next);
                proptest::prop_assert_eq!(&inc, &DistanceTables::build_with_closed(&l, 3, &next));
                tables = inc;
                mask = next;
            }
            if !order.is_empty() {
                let k = reopen_shift % order.len();
                order.rotate_left(k);
            }
            for c in order {
                let mut next = mask.clone();
                next[c] = false;
                let (inc, _) = tables.with_closed_changed(&l, 3, &mask, &next);
                proptest::prop_assert_eq!(&inc, &DistanceTables::build_with_closed(&l, 3, &next));
                tables = inc;
                mask = next;
            }
        }
    }

    /// Two floor sizes for the repair property tests.
    fn floor(big: bool) -> Layout {
        let (width, height, stations) = if big { (34, 31, 5) } else { (22, 24, 3) };
        generate(&FloorConfig {
            width,
            height,
            stations,
            south_margin: 4,
        })
        .unwrap()
    }

    /// The cells of a `w x h` rectangle at `(x, y)` that can be closed:
    /// open floor and induction cells (a dead robot can stand on a
    /// station).
    fn cluster(l: &Layout, x: u16, y: u16, w: u16, h: u16) -> Vec<usize> {
        let g = &l.grid;
        let mut cells = Vec::new();
        for yy in y..(y + h).min(g.height()) {
            for xx in x..(x + w).min(g.width()) {
                let c = g.idx(Pos::new(xx, yy));
                if matches!(g.cell(c), Cell::Free | Cell::Induction(_)) {
                    cells.push(c as usize);
                }
            }
        }
        cells
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]
        /// Clustered closures (aisle blocks), induction cells included, both
        /// guidance settings (`k = 1` disables lanes), two floor sizes:
        /// close clusters one by one (deletion repairs), then open them in
        /// another order (insertion repairs), then swap one cluster for
        /// another in one step (mixed change). Every step equals a full
        /// build.
        #[test]
        fn clustered_repairs_equal_full_builds(
            big in proptest::bool::ANY,
            guided in proptest::bool::ANY,
            rects in proptest::collection::vec((0u16..34, 0u16..31, 1u16..4, 1u16..9), 1..4),
            station_hit in proptest::bool::ANY,
        ) {
            let l = floor(big);
            let k = if guided { 3 } else { 1 };
            let n = l.grid.len();
            let mut clusters: Vec<Vec<usize>> =
                rects.iter().map(|&(x, y, w, h)| cluster(&l, x, y, w, h)).collect();
            if station_hit {
                // Always cover an induction cell in one test case of two.
                clusters.push(vec![l.stations[0] as usize]);
            }
            let check = |tables: &DistanceTables, mask: &[bool]| {
                proptest::prop_assert_eq!(tables, &DistanceTables::build_with_closed(&l, k, mask));
                Ok(())
            };
            let mut mask = vec![false; n];
            let mut tables = DistanceTables::build(&l, k);
            for cl in &clusters {
                let mut next = mask.clone();
                for &c in cl {
                    next[c] = true;
                }
                tables = tables.with_closed_changed(&l, k, &mask, &next).0;
                check(&tables, &next)?;
                mask = next;
            }
            // Mixed: reopen the first cluster and close a fresh one at once.
            let mut next = mask.clone();
            for &c in &clusters[0] {
                next[c] = false;
            }
            for c in cluster(&l, 1, 1, 6, 2) {
                next[c] = true;
            }
            tables = tables.with_closed_changed(&l, k, &mask, &next).0;
            check(&tables, &next)?;
            mask = next;
            for cl in clusters.iter().rev() {
                let mut next = mask.clone();
                for &c in cl {
                    next[c] = false;
                }
                tables = tables.with_closed_changed(&l, k, &mask, &next).0;
                check(&tables, &next)?;
                mask = next;
            }
        }
    }

    #[test]
    fn a_far_closure_recomputes_few_rows() {
        let l = layout();
        let n = l.grid.len();
        let tables = DistanceTables::build(&l, 3);
        let mut new = vec![false; n];
        new[l.grid.idx(Pos::new(1, 12)) as usize] = true;
        let (inc, rows) = tables.with_closed_changed(&l, 3, &vec![false; n], &new);
        assert_eq!(inc, DistanceTables::build_with_closed(&l, 3, &new));
        let total = l.stations.len() + l.chutes.len() + 1;
        assert!(rows < total, "{rows} of {total} rows recomputed");
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
