//! Root causes: walking the tick's wait-for graph (ARCHITECTURE §4).
//!
//! Each tick every robot that did not progress has one out-edge, its
//! *direct* cause ([`crate::sim::Sim::wait_for`]). Following `BlockedBy`
//! edges from robot to robot ends at a **root** (research §2.3, "blame
//! chains"; `root(A) = A`'s cause if it is terminal, else `root(next(A))`):
//!
//! | The walk reaches | Root |
//! |---|---|
//! | a robot with no cause of its own: loading, dropping, at its goal, idle | `BlockedBy(that robot)` |
//! | a robot stopped by a failure fault `f` | `CellBlocked(f)` |
//! | a `Yielded(b)` edge (`b` won the cell and moved on) | `Yielded(b)` |
//! | a `ChuteUnavailable`, `StationBusy`, `CellBlocked` or `Fault` edge | that cause, made fault-aware (a jammed chute's `ChuteUnavailable` becomes `Fault(jam)`) |
//! | a robot already on the current path: a cycle | `Cycle(lowest member id)` |
//!
//! Only `BlockedBy` edges are followed, so a cycle consists of robots that
//! all stayed put, each waiting on the next: a deadlock for as long as it
//! lasts (a wait-for cycle exists iff the robots are deadlocked; research
//! source [6]). PIBT resolves most head-on meetings within a tick or two, so
//! [`DeadlockWatch`] calls a cycle a deadlock only once the *same* cycle has
//! persisted for `planner.deadlock_ticks` consecutive ticks. Until then the
//! sim reports its chains rooted at `BlockedBy(canonical member)` (see
//! [`DeadlockWatch::label`]): `Cycle` in an event always means a deadlock.
//!
//! # Cost and determinism
//! [`RootWalker::walk`] visits every robot at most once per tick: a robot's
//! root is memoised as soon as the walk through it finishes, and later walks
//! that reach it stop there (path compression). Robots are started in id
//! order and a cycle's canonical member is its lowest id, so the result
//! depends only on the graph (ADR 0002).

use crate::Tick;
use crate::events::{Event, WaitCause};
use crate::ids::{FaultId, RobotId};

const UNSEEN: u8 = 0;
const ON_PATH: u8 = 1;
const DONE: u8 = 2;

/// Computes every robot's root cause from one tick's wait-for edges.
/// Holds only scratch buffers, reused from tick to tick.
#[derive(Clone, Debug, Default)]
pub struct RootWalker {
    mark: Vec<u8>,
    root: Vec<Option<WaitCause>>,
    path: Vec<u32>,
    /// Members of the cycles found by the last walk, one cycle after
    /// another, each starting at its canonical (lowest-id) member.
    members: Vec<RobotId>,
    /// End offset of each cycle in `members`.
    ends: Vec<usize>,
}

impl RootWalker {
    /// An empty walker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Walk every chain of `waits` (each robot's direct cause, `None` if it
    /// waited on nothing) to its root. `failed_by[r]` names the failure fault
    /// that stopped robot `r`, if any. `resolve` maps a terminal cause to its
    /// fault-aware root (for example a jammed chute's `ChuteUnavailable` to
    /// `Fault(jam)`); pass `|c| c` when there are no faults.
    ///
    /// # Panics
    /// If `waits` and `failed_by` differ in length, or an edge names a robot
    /// outside them.
    pub fn walk(
        &mut self,
        waits: &[Option<WaitCause>],
        failed_by: &[Option<FaultId>],
        resolve: impl Fn(WaitCause) -> WaitCause,
    ) {
        assert_eq!(waits.len(), failed_by.len(), "one entry per robot");
        let n = waits.len();
        self.mark.clear();
        self.mark.resize(n, UNSEEN);
        self.root.clear();
        self.root.resize(n, None);
        self.members.clear();
        self.ends.clear();
        for start in 0..n {
            if self.mark[start] != UNSEEN || waits[start].is_none() {
                continue;
            }
            self.path.clear();
            let mut r = start;
            let root = loop {
                match self.mark[r] {
                    DONE => break self.root[r],
                    ON_PATH => break Some(self.close_cycle(r)),
                    _ => {}
                }
                self.mark[r] = ON_PATH;
                // r < n, which fits in u32 (robot counts are u32).
                #[allow(clippy::cast_possible_truncation)]
                self.path.push(r as u32);
                match waits[r] {
                    Some(WaitCause::BlockedBy(b)) => {
                        let bi = b.index();
                        if let Some(f) = failed_by[bi] {
                            break Some(WaitCause::CellBlocked(f));
                        }
                        if waits[bi].is_none() {
                            break Some(WaitCause::BlockedBy(b));
                        }
                        r = bi;
                    }
                    // Yielded is terminal (the winner moved on), and the
                    // other causes are not robots.
                    terminal => break terminal.map(&resolve),
                }
            };
            for &p in &self.path {
                self.mark[p as usize] = DONE;
                self.root[p as usize] = root;
            }
        }
    }

    /// `r` is already on the current path: the path from `r` onwards is a
    /// cycle. Record it, starting at its lowest id, and return its root.
    fn close_cycle(&mut self, r: usize) -> WaitCause {
        // r is on the path, so the search succeeds.
        let from = self
            .path
            .iter()
            .rposition(|&p| p as usize == r)
            .unwrap_or(0);
        let cycle = &self.path[from..];
        let (lowest, &canonical) = cycle
            .iter()
            .enumerate()
            .min_by_key(|&(_, &m)| m)
            .unwrap_or((0, &0));
        let len = cycle.len();
        self.members
            .extend((0..len).map(|i| RobotId(cycle[(lowest + i) % len])));
        self.ends.push(self.members.len());
        WaitCause::Cycle(RobotId(canonical))
    }

    /// The root cause of robot `r` in the last walk (`None` if it waited on
    /// nothing).
    #[must_use]
    pub fn root(&self, r: usize) -> Option<WaitCause> {
        self.root[r]
    }

    /// The cycles found by the last walk, each starting at its canonical
    /// (lowest-id) member and following the wait-for edges; in order of
    /// discovery, which is deterministic.
    pub fn cycles(&self) -> impl Iterator<Item = &[RobotId]> {
        let mut start = 0;
        self.ends.iter().map(move |&end| {
            let c = &self.members[start..end];
            start = end;
            c
        })
    }
}

/// A cycle being watched for persistence.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Watched {
    members: Vec<RobotId>,
    since: Tick,
    reported: bool,
}

/// Turns per-tick wait-for cycles into deadlock reports: a cycle counts as
/// a deadlock once the same members, in the same order, have formed it on
/// `persist` consecutive ticks. Transient head-on meetings that PIBT
/// resolves within a few ticks are never reported.
#[derive(Clone, Debug)]
pub struct DeadlockWatch {
    persist: u32,
    watched: Vec<Watched>,
    next: Vec<Watched>,
    /// The tick of the last update.
    tick: Tick,
}

impl DeadlockWatch {
    /// A watch that reports cycles lasting at least `persist` ticks (at
    /// least 1).
    #[must_use]
    pub fn new(persist: u32) -> Self {
        Self {
            persist: persist.max(1),
            watched: Vec::new(),
            next: Vec::new(),
            tick: 0,
        }
    }

    /// Feed tick `t`'s cycles (as [`RootWalker::cycles`] yields them) and
    /// append a `DeadlockDetected` per member for every cycle that reaches
    /// the persistence threshold on this tick.
    pub fn update<'a>(
        &mut self,
        t: Tick,
        cycles: impl Iterator<Item = &'a [RobotId]>,
        out: &mut Vec<Event>,
    ) {
        self.next.clear();
        for members in cycles {
            let mut w = match self.watched.iter().position(|w| w.members == members) {
                Some(i) => self.watched.swap_remove(i),
                None => Watched {
                    members: members.to_vec(),
                    since: t,
                    reported: false,
                },
            };
            if !w.reported && w.persisted(t, self.persist) {
                w.reported = true;
                let cycle = w.members[0];
                out.extend(w.members.iter().map(|&robot| Event::DeadlockDetected {
                    tick: t,
                    robot,
                    cycle,
                    since: w.since,
                }));
            }
            self.next.push(w);
        }
        // Cycles absent this tick are broken: their episode is over.
        std::mem::swap(&mut self.watched, &mut self.next);
        self.tick = t;
    }

    /// The root to report for a walked root, after [`DeadlockWatch::update`]
    /// for the same tick: `Cycle(c)` stays a `Cycle` only if that cycle is a
    /// deadlock (it has persisted); a transient cycle's chains are rooted at
    /// `BlockedBy(c)`, its canonical member, like any robot that stayed put.
    #[must_use]
    pub fn label(&self, root: WaitCause) -> WaitCause {
        match root {
            WaitCause::Cycle(c)
                if !self
                    .watched
                    .iter()
                    .any(|w| w.members[0] == c && w.persisted(self.tick, self.persist)) =>
            {
                WaitCause::BlockedBy(c)
            }
            other => other,
        }
    }
}

impl Watched {
    fn persisted(&self, t: Tick, persist: u32) -> bool {
        t - self.since + 1 >= persist
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocked(r: u32) -> Option<WaitCause> {
        Some(WaitCause::BlockedBy(RobotId(r)))
    }

    fn walk(waits: &[Option<WaitCause>], failed: &[Option<FaultId>]) -> RootWalker {
        let mut w = RootWalker::new();
        w.walk(waits, failed, |c| c);
        w
    }

    #[test]
    fn a_chain_ends_at_the_robot_that_is_loading() {
        // 0 -> 1 -> 2 -> 3, and robot 3 is loading (no cause of its own).
        let waits = [blocked(1), blocked(2), blocked(3), None];
        let w = walk(&waits, &[None; 4]);
        for r in 0..3 {
            assert_eq!(w.root(r), blocked(3), "robot {r}");
        }
        assert_eq!(w.root(3), None);
        assert_eq!(w.cycles().count(), 0);
    }

    #[test]
    fn memoised_roots_are_shared_by_chains_that_merge() {
        // 3 -> 1 is walked after 0 -> 1 -> 2 finished: it reuses 1's root.
        let waits = [
            blocked(1),
            blocked(2),
            Some(WaitCause::Yielded(RobotId(4))),
            blocked(1),
            None,
        ];
        let w = walk(&waits, &[None; 5]);
        let yielded = Some(WaitCause::Yielded(RobotId(4)));
        for r in 0..4 {
            assert_eq!(w.root(r), yielded, "robot {r}");
        }
    }

    #[test]
    fn terminal_causes_are_their_own_root() {
        let chute = Some(WaitCause::ChuteUnavailable(crate::ids::ChuteId(7)));
        let station = Some(WaitCause::StationBusy(crate::ids::StationId(2)));
        let waits = [blocked(1), chute, blocked(3), station];
        let w = walk(&waits, &[None; 4]);
        assert_eq!(w.root(0), chute);
        assert_eq!(w.root(1), chute);
        assert_eq!(w.root(2), station);
    }

    #[test]
    fn terminal_causes_resolve_to_the_fault_behind_them() {
        // 0 -> 1, and robot 1 waits at a chute that fault 3 jammed.
        let chute = Some(WaitCause::ChuteUnavailable(crate::ids::ChuteId(9)));
        let waits = [blocked(1), chute];
        let mut w = RootWalker::new();
        w.walk(&waits, &[None; 2], |c| match c {
            WaitCause::ChuteUnavailable(_) => WaitCause::Fault(FaultId(3)),
            other => other,
        });
        assert_eq!(w.root(0), Some(WaitCause::Fault(FaultId(3))));
        assert_eq!(w.root(1), Some(WaitCause::Fault(FaultId(3))));
    }

    #[test]
    fn a_young_cycle_is_labelled_blocked_by_until_it_persists() {
        let waits = [blocked(1), blocked(0)];
        let mut walker = RootWalker::new();
        let mut watch = DeadlockWatch::new(3);
        let mut out = Vec::new();
        let mut labels = Vec::new();
        for t in 0..4 {
            walker.walk(&waits, &[None; 2], |c| c);
            watch.update(t, walker.cycles(), &mut out);
            labels.push(watch.label(walker.root(1).unwrap()));
        }
        let young = WaitCause::BlockedBy(RobotId(0));
        let dead = WaitCause::Cycle(RobotId(0));
        assert_eq!(labels, vec![young, young, dead, dead]);
        // Other roots pass through unchanged.
        assert_eq!(watch.label(young), young);
    }

    #[test]
    fn a_dead_robot_at_the_end_of_a_chain_is_cell_blocked() {
        // 0 -> 1 -> 2, and robot 2 was stopped by fault 5.
        let waits = [blocked(1), blocked(2), None];
        let failed = [None, None, Some(FaultId(5))];
        let w = walk(&waits, &failed);
        let root = Some(WaitCause::CellBlocked(FaultId(5)));
        assert_eq!(w.root(0), root);
        assert_eq!(w.root(1), root);
    }

    #[test]
    fn a_cycle_is_rooted_at_its_lowest_member_with_tails_included() {
        // 4 -> 3 -> 2 -> 3 (a 2-cycle {2, 3} with a tail), plus 0 -> 1 idle.
        let waits = [blocked(1), None, blocked(3), blocked(2), blocked(3)];
        let w = walk(&waits, &[None; 5]);
        let cycle = Some(WaitCause::Cycle(RobotId(2)));
        assert_eq!(w.root(2), cycle);
        assert_eq!(w.root(3), cycle);
        assert_eq!(w.root(4), cycle);
        assert_eq!(w.root(0), blocked(1));
        let cycles: Vec<_> = w.cycles().collect();
        assert_eq!(cycles, vec![&[RobotId(2), RobotId(3)][..]]);
    }

    #[test]
    fn cycle_members_start_at_the_canonical_member_in_edge_order() {
        // 5 -> 9 -> 2 -> 5, discovered from 0 -> 9.
        let mut waits = vec![None; 10];
        waits[0] = blocked(9);
        waits[5] = blocked(9);
        waits[9] = blocked(2);
        waits[2] = blocked(5);
        let w = walk(&waits, &[None; 10]);
        let cycles: Vec<_> = w.cycles().collect();
        assert_eq!(cycles, vec![&[RobotId(2), RobotId(5), RobotId(9)][..]]);
        assert_eq!(w.root(0), Some(WaitCause::Cycle(RobotId(2))));
    }

    fn deadlocks(events: &[Event]) -> Vec<(Tick, RobotId, RobotId, Tick)> {
        events
            .iter()
            .filter_map(|e| match *e {
                Event::DeadlockDetected {
                    tick,
                    robot,
                    cycle,
                    since,
                } => Some((tick, robot, cycle, since)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_persisting_two_cycle_is_reported_once_as_a_deadlock() {
        // A <-> B head-on (robots 0 and 1) for 8 ticks; k = 5.
        let waits = [blocked(1), blocked(0)];
        let mut walker = RootWalker::new();
        let mut watch = DeadlockWatch::new(5);
        let mut out = Vec::new();
        for t in 100..108 {
            walker.walk(&waits, &[None; 2], |c| c);
            watch.update(t, walker.cycles(), &mut out);
        }
        assert_eq!(
            deadlocks(&out),
            vec![
                (104, RobotId(0), RobotId(0), 100),
                (104, RobotId(1), RobotId(0), 100),
            ]
        );
    }

    #[test]
    fn a_transient_two_cycle_is_not_a_deadlock() {
        // The cycle lasts k - 1 ticks, breaks, and forms again for k - 1.
        let cycle = [blocked(1), blocked(0)];
        let broken = [blocked(1), None];
        let mut walker = RootWalker::new();
        let mut watch = DeadlockWatch::new(5);
        let mut out = Vec::new();
        for t in 0..12 {
            let waits = if t == 4 || t == 9 { &broken } else { &cycle };
            walker.walk(waits, &[None; 2], |c| c);
            watch.update(t, walker.cycles(), &mut out);
        }
        assert!(deadlocks(&out).is_empty(), "{out:?}");
    }

    #[test]
    fn a_reported_deadlock_is_reported_again_only_after_it_breaks() {
        let cycle = [blocked(1), blocked(0)];
        let broken = [None, None];
        let mut walker = RootWalker::new();
        let mut watch = DeadlockWatch::new(2);
        let mut out = Vec::new();
        for t in 0..10 {
            let waits = if t == 5 { &broken } else { &cycle };
            walker.walk(waits, &[None; 2], |c| c);
            watch.update(t, walker.cycles(), &mut out);
        }
        let ticks: Vec<_> = deadlocks(&out).iter().map(|d| (d.0, d.3)).collect();
        assert_eq!(ticks, vec![(1, 0), (1, 0), (7, 6), (7, 6)]);
    }
}
