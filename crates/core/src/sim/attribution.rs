//! Phase 8 of the tick: why robots and parcels waited (ARCHITECTURE §4).
//!
//! 1. Every robot's direct cause goes into the wait-for record
//!    ([`Sim::wait_for`]): a failed robot waits on its failure
//!    (`Fault(f)`); a robot whose goal closed cells cut off waits on the
//!    fault that cut it off (`CellBlocked(f)`, see [`crate::faults`] for
//!    the blame rule); otherwise the planner's robot-to-robot cause, refined
//!    with what only the sim knows about stations and chutes.
//! 2. The record is walked to each robot's root ([`crate::causality`]); a
//!    jammed chute's `ChuteUnavailable` resolves to `Fault(jam)`, and a
//!    cycle is reported as `Cycle` only once it is a persisting deadlock.
//! 3. Robots with work get `RobotWaited` (and `RobotStalled` after
//!    `planner.watchdog_ticks` waits in a row, except failed robots, whose
//!    stop is already explained by their fault).
//! 4. Every parcel tick a fault costs gets one `ParcelDelayed`.
//! 5. A wait-for cycle that has persisted `planner.deadlock_ticks` ticks is
//!    reported with `DeadlockDetected`.

use super::{Sim, robot_id, station_id};
use crate::Tick;
use crate::events::{Event, WaitCause};
use crate::faults::SURGE_KEY_FLAG;
use crate::ids::FaultId;
use crate::world::{Goal, RobotState, Robots, Stations};

impl Sim {
    /// Phase 8 (see the module docs).
    pub(super) fn attribute_waits(&mut self, t: Tick, out: &mut Vec<Event>) {
        for r in 0..self.world.robots.len() {
            let goal = self.world.robots.goal[r];
            // Still at its chute (not pushed aside this tick) and unable to drop.
            let waiting = self.at_chute[r]
                && self.world.robots.state[r] == RobotState::ToChute
                && self.plan_out.next[r] == self.world.robots.pos[r]
                && goal.is_some_and(|g| self.distances.get(g, self.world.robots.pos[r]) == 0);
            // Every robot's edge is kept, so chains can be walked through
            // idle and parked robots (`wait_for`).
            self.waits[r] = if let Some(f) = self.faults.failed_by[r] {
                Some(WaitCause::Fault(f))
            } else if let Some(f) = self.cut_by[r] {
                Some(WaitCause::CellBlocked(f))
            } else {
                refine_cause(
                    goal,
                    self.plan_out.cause[r],
                    waiting,
                    &self.world.robots,
                    &self.world.stations,
                )
            };
        }
        let jammed = &self.faults.jammed_by;
        self.walker
            .walk(&self.waits, &self.faults.failed_by, |cause| match cause {
                WaitCause::ChuteUnavailable(c) => jammed[c.index()].map_or(cause, WaitCause::Fault),
                other => other,
            });
        self.deadlock_events.clear();
        self.deadlocks
            .update(t, self.walker.cycles(), &mut self.deadlock_events);

        for r in 0..self.world.robots.len() {
            let goal = self.world.robots.goal[r];
            let waited = &mut self.world.robots.waited[r];
            let Some(direct) = self.waits[r] else {
                *waited = 0;
                self.roots[r] = None;
                continue;
            };
            *waited = waited.saturating_add(1);
            let waited = *waited;
            let root = self.deadlocks.label(self.walker.root(r).unwrap_or(direct));
            self.roots[r] = Some(root);
            // Events only for robots with work: idle and parking robots carry
            // no parcel and delay nothing on their own.
            if !matches!(goal, Some(Goal::Station(_) | Goal::Chute(_))) {
                continue;
            }
            if waited == self.config.planner.watchdog_ticks
                && !matches!(direct, WaitCause::Fault(_))
            {
                out.push(Event::RobotStalled {
                    tick: t,
                    robot: robot_id(r),
                    waited,
                });
            }
            out.push(Event::RobotWaited {
                tick: t,
                robot: robot_id(r),
                direct,
                root,
            });
        }
        if !self.faults.is_empty() {
            self.parcel_delays(t, out);
        }
        out.append(&mut self.deadlock_events);
    }

    /// One `ParcelDelayed` for every parcel a fault held up this tick (see
    /// `Event::ParcelDelayed` for the cases). Robots in id order, then
    /// station queues in id and queue order; a parcel is either on a robot
    /// or in a queue unclaimed, so it gets at most one event.
    fn parcel_delays(&mut self, t: Tick, out: &mut Vec<Event>) {
        for r in 0..self.world.robots.len() {
            let Some(parcel) = self.world.robots.parcel[r] else {
                continue;
            };
            let goal = self.world.robots.goal[r];
            // 1. The robot's own wait, if its root is a fault.
            let waited = match (self.waits[r], self.roots[r]) {
                (Some(cause), Some(root)) => root.fault().map(|_| (cause, root)),
                _ => None,
            };
            // 2. Its chute is jammed: the parcel cannot be delivered until the
            //    jam clears, whatever its robot does meanwhile (robots waiting
            //    at a chute are pushed aside and come back, so their motion
            //    says nothing about the parcel's delay).
            let jammed = match goal {
                Some(Goal::Chute(c)) => self.faults.jammed_by[c.index()]
                    .map(|f| (WaitCause::ChuteUnavailable(c), WaitCause::Fault(f))),
                _ => None,
            };
            // 3. It moved on a detour around closed cells (blamed only when
            //    nothing above already explains the tick).
            let delay = waited.or(jammed).or_else(|| match (self.detour[r], goal) {
                (Some(from), Some(goal)) => {
                    let current = self.distances.get(goal, from);
                    let f = self.topology_blame(goal, from, current);
                    Some((WaitCause::CellBlocked(f), WaitCause::CellBlocked(f)))
                }
                _ => None,
            });
            if let Some((cause, root)) = delay {
                out.push(Event::ParcelDelayed {
                    tick: t,
                    parcel,
                    cause,
                    root,
                });
            }
        }

        for s in 0..self.world.stations.cell.len() {
            // Outage; a failed robot on the induction cell (StationBusy with
            // the failure as root: "the station is blocked by a dead
            // loader", distinct from the failure's own parcel and from
            // routing cut-offs); or cells that cut the station off.
            let same = |c: WaitCause| (c, c);
            let station_delay = self.faults.outage_by[s]
                .map(|f| same(WaitCause::Fault(f)))
                .or_else(|| {
                    self.station_blocker(s).map(|f| {
                        (
                            WaitCause::StationBusy(station_id(s)),
                            WaitCause::CellBlocked(f),
                        )
                    })
                })
                .or_else(|| self.faults.station_cut[s].map(|f| same(WaitCause::CellBlocked(f))));
            let mut surge_ahead: Option<FaultId> = None;
            for &parcel in &self.world.stations.queue[s] {
                let p = parcel.index();
                let key = self.world.parcels.key[p];
                let surge = key & SURGE_KEY_FLAG != 0;
                let claimed = self.world.parcels.robot[p].is_some();
                let delay = if claimed {
                    None
                } else if station_delay.is_some() {
                    station_delay
                } else if surge {
                    None
                } else {
                    surge_ahead.map(|f| (WaitCause::Fault(f), WaitCause::Fault(f)))
                };
                if surge && surge_ahead.is_none() {
                    surge_ahead = self.faults.surge_of(key);
                }
                if let Some((cause, root)) = delay {
                    out.push(Event::ParcelDelayed {
                        tick: t,
                        parcel,
                        cause,
                        root,
                    });
                }
            }
        }
    }
}

/// Refine a planner's robot-to-robot cause with what only the sim knows
/// (ARCHITECTURE §4): a robot waiting at a chute that cannot take its parcel
/// waits on the chute; a robot blocked by the robot *loading* on its target
/// station's cell waits on the station. A robot stuck on that cell after
/// loading is just another robot: the edge stays `BlockedBy`, so the walk
/// continues to whatever blocks it.
fn refine_cause(
    goal: Option<Goal>,
    planner: Option<WaitCause>,
    waiting_at_chute: bool,
    robots: &Robots,
    stations: &Stations,
) -> Option<WaitCause> {
    match (goal, planner) {
        (Some(Goal::Chute(c)), _) if waiting_at_chute => Some(WaitCause::ChuteUnavailable(c)),
        (Some(Goal::Station(s)), Some(WaitCause::BlockedBy(b)))
            if robots.pos[b.index()] == stations.cell[s.index()]
                && robots.state[b.index()] == RobotState::Loading =>
        {
            Some(WaitCause::StationBusy(s))
        }
        (_, cause) => cause,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ChuteId, RobotId, StationId};

    /// Robot 1 stands on station 0's cell (cell 7); robot 0 heads there.
    fn floor(blocker: RobotState) -> (Robots, Stations) {
        let robots = Robots {
            pos: vec![8, 7],
            state: vec![RobotState::ToInduction, blocker],
            ..Robots::default()
        };
        let stations = Stations {
            cell: vec![7],
            ..Stations::default()
        };
        (robots, stations)
    }

    #[test]
    fn station_busy_only_while_the_blocker_is_loading() {
        let goal = Some(Goal::Station(StationId(0)));
        let blocked = Some(WaitCause::BlockedBy(RobotId(1)));
        let (robots, stations) = floor(RobotState::Loading);
        assert_eq!(
            refine_cause(goal, blocked, false, &robots, &stations),
            Some(WaitCause::StationBusy(StationId(0)))
        );
        // Loaded and itself stuck on the station cell: the real blocker is
        // further down the chain, so keep the robot edge.
        let (robots, stations) = floor(RobotState::ToChute);
        assert_eq!(
            refine_cause(goal, blocked, false, &robots, &stations),
            blocked
        );
    }

    #[test]
    fn waiting_at_an_unavailable_chute_is_chute_unavailable() {
        let (robots, stations) = floor(RobotState::Loading);
        let goal = Some(Goal::Chute(ChuteId(3)));
        assert_eq!(
            refine_cause(goal, None, true, &robots, &stations),
            Some(WaitCause::ChuteUnavailable(ChuteId(3)))
        );
        // Pushed off the chute while waiting: the robot edge is the cause.
        let yielded = Some(WaitCause::Yielded(RobotId(1)));
        assert_eq!(
            refine_cause(goal, yielded, false, &robots, &stations),
            yielded
        );
    }
}
