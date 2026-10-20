//! Guided PIBT: Priority Inheritance with Backtracking (ADR 0003).
//!
//! Okumura, Machida, Défago and Tamura, "Priority Inheritance with
//! Backtracking for Iterative Multi-agent Path Finding" (IJCAI-19 / AIJ 2022,
//! arXiv 1901.11282). This is a port in spirit of the reference
//! implementation in Kei18/pibt2 (C++17, MIT licence, (c) Keisuke Okumura;
//! `src/pibt.cpp`), adapted to this crate's integer guidance graph, fixed
//! robots and causal records.
//!
//! # The algorithm, per tick
//! 1. Robots are sorted by priority: robots with a task first, then robots
//!    heading to parking, then robots with nothing to do; within a class, the
//!    longer since the robot's goal was issued or reached, the higher
//!    (Okumura's "elapsed" priority, which is what guarantees every robot
//!    eventually reaches its goal on suitable graphs); ties break by id.
//! 2. Each undecided robot, in that order, calls `pibt(a, parent)`: it ranks
//!    its candidate cells (four neighbours and staying put) by guided cost to
//!    its goal and takes the best one not reserved for the next tick. If a
//!    robot `b` stands in that cell and is undecided, `b` *inherits* `a`'s
//!    priority and plans next, forbidden to take `a`'s current cell (that
//!    would be a swap). If `b` cannot move anywhere, it reports failure and
//!    `a` *backtracks* to its next candidate.
//!
//! # Guidance
//! A candidate move to `c` is ranked by `edge_cost(here, c) + dist(goal, c)`,
//! the true guided cost of a path through `c` ([`crate::guidance`]); staying
//! ranks just behind the best move (`dist(goal, here) + 1`), and at the goal
//! staying ranks first. Equal ranks break by a fixed per-robot rotation of
//! the candidate order that also varies with the tick: a deterministic hash
//! of `(robot, tick)`, with no random stream to advance, so the result
//! depends only on the input (ADR 0002).
//!
//! # Causal records
//! The inheritance tree is the explanation (research §7): a robot that did
//! not get closer to its goal looks at its best candidate cell `u`:
//! - a robot that stays in `u` holds it: `BlockedBy(that robot)`;
//! - a robot that moved into `u` won it on priority: `Yielded(that robot)`;
//! - otherwise the robot gave way to the robot that pushed it (its parent in
//!   the inheritance tree), or `u` is that parent's cell: `Yielded(parent)`.

use crate::events::WaitCause;
use crate::grid::CellIdx;
use crate::guidance::{UNREACHABLE, step};
use crate::hash::Fnv1a;
use crate::ids::RobotId;
use crate::planner::{PlanInput, PlanOutput, Planner, enterable};
use crate::world::Goal;

const NONE: u32 = u32::MAX;

/// Slot of the "stay" candidate in the tie-break rotation (after the four
/// [`crate::grid::DIRS`] directions).
const STAY: u32 = 4;

/// Guided PIBT. See the module docs.
///
/// The planner keeps no state between ticks (only reusable scratch buffers),
/// so it adds nothing to the state hash beyond a version tag.
#[derive(Clone, Debug, Default)]
pub struct PibtPlanner {
    /// Robots in planning order.
    order: Vec<u32>,
    /// Robot standing in each cell now.
    occ_now: Vec<u32>,
    /// Robot that reserved each cell for the next tick.
    occ_next: Vec<u32>,
    /// Robot that pushed each robot this tick (its parent in the inheritance
    /// tree).
    parent: Vec<u32>,
}

/// A ranked candidate cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Candidate {
    key: u32,
    tie: u32,
    cell: CellIdx,
}

impl PibtPlanner {
    /// A planner with empty scratch buffers.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Candidates of robot `r` in rank order; returns how many there are.
    fn candidates(input: &PlanInput<'_>, r: usize, out: &mut [Candidate; 5]) -> usize {
        let here = input.pos[r];
        let goal = input.goal[r];
        let d = input.distances;
        // Per-robot rotation of the slot order, varying with the tick.
        let rot = mix(r as u64, u64::from(input.tick)) % 5;
        let tie = |slot: u32| (slot + 5 - u32::try_from(rot).unwrap_or(0)) % 5;
        let d_here = goal.map(|g| d.get(g, here));
        let mut n = 0;
        for dir in 0..4 {
            let cost = d.edge_cost(here, dir);
            if cost == 0 {
                continue;
            }
            let Some(to) = step(&input.layout.grid, here, dir) else {
                continue;
            };
            if !enterable(input.layout, goal, to) {
                continue;
            }
            let key = match goal {
                Some(g) => cost.saturating_add(d.get(g, to)),
                None => 1,
            };
            // dir < 4.
            #[allow(clippy::cast_possible_truncation)]
            let slot = dir as u32;
            out[n] = Candidate {
                key,
                tie: tie(slot),
                cell: to,
            };
            n += 1;
        }
        let stay_key = match d_here {
            None | Some(0) => 0,
            Some(UNREACHABLE) => UNREACHABLE,
            Some(dh) => dh + 1,
        };
        out[n] = Candidate {
            key: stay_key,
            tie: tie(STAY),
            cell: here,
        };
        n += 1;
        out[..n].sort_unstable();
        n
    }

    /// `pibt(a, parent)` from Okumura et al. (Algorithm 1). Returns true if
    /// `a` secured a cell other than a forced stay.
    fn pibt(&mut self, input: &PlanInput<'_>, out: &mut PlanOutput, a: usize, parent: u32) -> bool {
        let mut cands = [Candidate {
            key: 0,
            tie: 0,
            cell: 0,
        }; 5];
        let n = Self::candidates(input, a, &mut cands);
        let parent_cell = (parent != NONE).then(|| input.pos[parent as usize]);
        // a < robot count, which fits in u32.
        #[allow(clippy::cast_possible_truncation)]
        let id = a as u32;
        for c in &cands[..n] {
            let u = c.cell;
            if self.occ_next[u as usize] != NONE || parent_cell == Some(u) {
                continue;
            }
            self.occ_next[u as usize] = id;
            out.next[a] = u;
            let k = self.occ_now[u as usize];
            if k != NONE && k != id && out.next[k as usize] == NONE {
                self.parent[k as usize] = id;
                if !self.pibt(input, out, k as usize, id) {
                    // k stays in u (it re-reserved u for itself): backtrack.
                    continue;
                }
            }
            return true;
        }
        let here = input.pos[a];
        self.occ_next[here as usize] = id;
        out.next[a] = here;
        false
    }

    /// The cause for robot `r`, which did not get closer to its goal (see
    /// the module docs).
    fn cause(&self, input: &PlanInput<'_>, r: usize) -> Option<WaitCause> {
        let mut cands = [Candidate {
            key: 0,
            tie: 0,
            cell: 0,
        }; 5];
        let n = Self::candidates(input, r, &mut cands);
        let best = cands[..n].first()?.cell;
        let parent = self.parent[r];
        let holder = self.occ_next[best as usize];
        // r < robot count, which fits in u32.
        #[allow(clippy::cast_possible_truncation)]
        let id = r as u32;
        if holder != NONE && holder != id && holder != parent {
            let h = RobotId(holder);
            return Some(if input.pos[holder as usize] == best {
                WaitCause::BlockedBy(h)
            } else {
                WaitCause::Yielded(h)
            });
        }
        if parent != NONE {
            return Some(WaitCause::Yielded(RobotId(parent)));
        }
        debug_assert!(
            holder != NONE,
            "robot {r} made no progress with its best cell free"
        );
        (holder != NONE).then_some(WaitCause::BlockedBy(RobotId(holder)))
    }
}

impl Planner for PibtPlanner {
    fn plan(&mut self, input: &PlanInput<'_>, out: &mut PlanOutput) {
        let n = input.pos.len();
        let cells = input.layout.grid.len();
        self.occ_now.clear();
        self.occ_now.resize(cells, NONE);
        self.occ_next.clear();
        self.occ_next.resize(cells, NONE);
        self.parent.clear();
        self.parent.resize(n, NONE);
        out.next.clear();
        out.next.resize(n, NONE);
        out.cause.clear();
        out.cause.resize(n, None);

        for (r, &p) in input.pos.iter().enumerate() {
            // r < robot count, which fits in u32.
            #[allow(clippy::cast_possible_truncation)]
            let id = r as u32;
            self.occ_now[p as usize] = id;
            if !input.movable[r] {
                // Fixed robots are decided before anyone plans: their cell is
                // reserved, so no one can push into it.
                self.occ_next[p as usize] = id;
                out.next[r] = p;
            }
        }

        // Planning order: (class, older first, lower id first).
        self.order.clear();
        #[allow(clippy::cast_possible_truncation)]
        self.order
            .extend((0..n as u32).filter(|&r| input.movable[r as usize]));
        self.order.sort_unstable_by_key(|&r| {
            let class: u8 = match input.goal[r as usize] {
                Some(Goal::Station(_) | Goal::Chute(_)) => 0,
                Some(Goal::Park) => 1,
                None => 2,
            };
            (class, std::cmp::Reverse(input.priority[r as usize]), r)
        });

        for i in 0..self.order.len() {
            let r = self.order[i] as usize;
            if out.next[r] == NONE {
                self.pibt(input, out, r, NONE);
            }
        }

        for r in 0..n {
            let Some(goal) = input.goal[r] else {
                continue;
            };
            if !input.movable[r] {
                continue;
            }
            let d = input.distances;
            let d_here = d.get(goal, input.pos[r]);
            if d_here == 0 || d_here == UNREACHABLE {
                continue;
            }
            if d.get(goal, out.next[r]) < d_here {
                continue;
            }
            out.cause[r] = self.cause(input, r);
        }
    }

    fn hash_state(&self, h: &mut Fnv1a) {
        // Stateless between ticks; the tag pins the algorithm version.
        h.write(b"pibt-v1");
    }
}

/// SplitMix64-style mixing of a robot id and a tick into a well-spread
/// value, for the tie-break rotation.
const fn mix(a: u64, b: u64) -> u64 {
    let mut z = a
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(b.wrapping_mul(0xbf58_476d_1ce4_e5b9));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FloorConfig;
    use crate::grid::Pos;
    use crate::guidance::DistanceTables;
    use crate::ids::{ChuteId, StationId};
    use crate::layout::{Layout, generate};
    use proptest::prelude::*;

    fn layout() -> Layout {
        generate(&FloorConfig {
            width: 16,
            height: 16,
            stations: 2,
            south_margin: 3,
        })
        .unwrap()
    }

    #[derive(Debug)]
    struct Case {
        pos: Vec<CellIdx>,
        goal: Vec<Option<Goal>>,
        movable: Vec<bool>,
        priority: Vec<u32>,
    }

    fn plan(l: &Layout, d: &DistanceTables, c: &Case, tick: u32) -> PlanOutput {
        let mut out = PlanOutput::default();
        PibtPlanner::new().plan(
            &PlanInput {
                layout: l,
                distances: d,
                tick,
                pos: &c.pos,
                goal: &c.goal,
                movable: &c.movable,
                priority: &c.priority,
            },
            &mut out,
        );
        out
    }

    #[test]
    fn robot_behind_a_fixed_robot_is_blocked_by_it() {
        let l = layout();
        let d = DistanceTables::build(&l, 3);
        let g = &l.grid;
        // Chute 0 is at (3, 3); (3, 2) is its north access cell and the
        // unique best move for a robot at (3, 1). Robot 1 stands there, fixed
        // (it is dropping).
        let a = g.idx(Pos::new(3, 1));
        let b = g.idx(Pos::new(3, 2));
        let case = Case {
            pos: vec![a, b],
            goal: vec![Some(Goal::Chute(ChuteId(0))), None],
            movable: vec![true, false],
            priority: vec![10, 0],
        };
        for tick in 0..10 {
            let out = plan(&l, &d, &case, tick);
            assert_eq!(out.next, vec![a, b], "tick {tick}");
            assert_eq!(out.cause[0], Some(WaitCause::BlockedBy(RobotId(1))));
            assert_eq!(out.cause[1], None);
        }
    }

    #[test]
    fn lower_priority_robot_yields_a_contested_cell() {
        let l = layout();
        let d = DistanceTables::build(&l, 1);
        let g = &l.grid;
        // Both robots head for station 0; its induction cell is the unique
        // best move for both. Robot 1 is older, so it wins.
        let station = l.stations[0];
        let p = g.pos(station);
        let north = g.idx(Pos::new(p.x, p.y - 1));
        let west = g.idx(Pos::new(p.x - 1, p.y));
        let goal = Some(Goal::Station(StationId(0)));
        let case = Case {
            pos: vec![west, north],
            goal: vec![goal, goal],
            movable: vec![true, true],
            priority: vec![1, 9],
        };
        let out = plan(&l, &d, &case, 0);
        assert_eq!(out.next[1], station);
        assert_ne!(out.next[0], station);
        assert_eq!(out.cause[0], Some(WaitCause::Yielded(RobotId(1))));
        assert_eq!(out.cause[1], None);
    }

    #[test]
    fn idle_robot_is_pushed_aside() {
        let l = layout();
        let d = DistanceTables::build(&l, 3);
        let g = &l.grid;
        let a = g.idx(Pos::new(3, 1));
        let b = g.idx(Pos::new(3, 2));
        let case = Case {
            pos: vec![a, b],
            goal: vec![Some(Goal::Chute(ChuteId(0))), None],
            movable: vec![true, true],
            priority: vec![0, 0],
        };
        let out = plan(&l, &d, &case, 0);
        assert_eq!(out.next[0], b, "the busy robot takes the access cell");
        assert_ne!(out.next[1], b);
        assert_ne!(out.next[1], a, "no swap");
        assert_eq!(out.cause, vec![None, None]);
    }

    #[test]
    fn head_on_robots_do_not_swap() {
        let l = layout();
        let d = DistanceTables::build(&l, 3);
        let g = &l.grid;
        // Side by side in row 2: robot 0 wants to go west (chute 0 at
        // (3, 3)), robot 1 east (chute 1 at (6, 3)), so each wants the
        // other's cell. They must get past each other without swapping.
        let a = g.idx(Pos::new(5, 2));
        let b = g.idx(Pos::new(4, 2));
        let case = Case {
            pos: vec![a, b],
            goal: vec![Some(Goal::Chute(ChuteId(0))), Some(Goal::Chute(ChuteId(1)))],
            movable: vec![true, true],
            priority: vec![5, 5],
        };
        for tick in 0..20 {
            let out = plan(&l, &d, &case, tick);
            assert!(!(out.next[0] == b && out.next[1] == a), "tick {tick}: swap");
            assert_ne!(out.next[0], out.next[1]);
        }
    }

    /// Check the planner contract (see `crate::planner`) on one output.
    fn check_contract(l: &Layout, d: &DistanceTables, c: &Case, out: &PlanOutput) {
        let g = &l.grid;
        let n = c.pos.len();
        let mut owner = vec![NONE; g.len()];
        let mut at = vec![NONE; g.len()];
        for (r, &p) in c.pos.iter().enumerate() {
            at[p as usize] = u32::try_from(r).unwrap();
        }
        for r in 0..n {
            let (from, to) = (c.pos[r], out.next[r]);
            assert!(
                from == to || g.neighbours(from).any(|x| x == to),
                "robot {r} jumped"
            );
            assert_eq!(owner[to as usize], NONE, "vertex conflict at {to}");
            owner[to as usize] = u32::try_from(r).unwrap();
            if !c.movable[r] {
                assert_eq!(from, to, "fixed robot {r} moved");
            }
            if from != to {
                assert!(enterable(l, c.goal[r], to), "robot {r} entered {to}");
                let other = at[to as usize];
                if other != NONE {
                    assert_ne!(out.next[other as usize], from, "swap {r}/{other}");
                }
            }
            let wants = c.movable[r]
                && c.goal[r].is_some_and(|goal| {
                    let dh = d.get(goal, from);
                    dh != 0 && dh != UNREACHABLE && d.get(goal, to) >= dh
                });
            assert_eq!(
                out.cause[r].is_some(),
                wants,
                "robot {r}: {:?}",
                out.cause[r]
            );
        }
    }

    fn arb_case(l: &Layout) -> impl Strategy<Value = Case> + use<> {
        let free: Vec<CellIdx> = (0..u32::try_from(l.grid.len()).unwrap())
            .filter(|&c| l.grid.is_free(c))
            .collect();
        let stations = l.stations.len();
        let chutes = l.chutes.len();
        let station_cells = l.stations.clone();
        (1usize..30).prop_flat_map(move |n| {
            let free = free.clone();
            let station_cells = station_cells.clone();
            (
                proptest::sample::subsequence(free, n),
                proptest::collection::vec(
                    (
                        0u8..5,
                        0usize..stations.max(chutes),
                        any::<bool>(),
                        0u32..50,
                    ),
                    n,
                ),
                proptest::option::of(0usize..stations),
            )
                .prop_map(move |(cells, attrs, on_station)| {
                    let mut pos = cells;
                    let mut goal = Vec::new();
                    let mut movable = Vec::new();
                    let mut priority = Vec::new();
                    for (kind, id, fixed, prio) in &attrs {
                        goal.push(match kind {
                            0 => None,
                            1 => Some(Goal::Park),
                            2 => Some(Goal::Station(StationId(
                                u16::try_from(id % stations).unwrap(),
                            ))),
                            _ => Some(Goal::Chute(ChuteId(u16::try_from(id % chutes).unwrap()))),
                        });
                        movable.push(!fixed);
                        priority.push(*prio);
                    }
                    // Optionally put robot 0 on a station cell (loading or
                    // just loaded), as the sim does.
                    if let Some(s) = on_station {
                        pos[0] = station_cells[s];
                        if goal[0] == Some(Goal::Park) || goal[0].is_none() {
                            movable[0] = false;
                        }
                    }
                    Case {
                        pos,
                        goal,
                        movable,
                        priority,
                    }
                })
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]
        #[test]
        fn plans_keep_the_contract(case in arb_case(&layout()), tick in 0u32..1000) {
            let l = layout();
            let d = DistanceTables::build(&l, 3);
            let out = plan(&l, &d, &case, tick);
            check_contract(&l, &d, &case, &out);
            // Determinism: planning the same input again gives the same plan.
            prop_assert_eq!(out, plan(&l, &d, &case, tick));
        }
    }
}
