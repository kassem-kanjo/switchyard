//! Phase 4 of the tick: stations hand out work; surplus idle robots park.

use super::{Sim, robot_id, station_id};
use crate::Tick;
use crate::events::Event;
use crate::guidance::UNREACHABLE;
use crate::world::{Goal, Phase, RobotState};

impl Sim {
    /// Phase 4. First, robots that a fault has cut off from the station they
    /// were sent to give their claim back. Then each station that is up (no
    /// induction outage), in id order, hands its oldest unclaimed parcels to
    /// the nearest idle robots that can reach it (by guided distance, ties
    /// by id), up to its inbound cap (`fleet.max_inbound_per_station`, or its
    /// `fleet.station_caps` override). Idle robots left
    /// without work head for the parking lane (throttling, see the module
    /// docs of [`crate::sim`]).
    pub(super) fn assign_work(&mut self, t: Tick, out: &mut Vec<Event>) {
        if self.faults.topology_active() {
            self.release_cut_off_claims(t, out);
        }
        self.idle.clear();
        for r in self.world.robots.ids() {
            if self.world.robots.state[r.index()] == RobotState::Idle {
                self.idle.push(r.0);
            }
        }

        for s in 0..self.world.stations.cell.len() {
            if !self.station_up(s) {
                continue;
            }
            let station = station_id(s);
            let goal = Goal::Station(station);
            while self.world.stations.inbound[s] < self.station_cap[s] {
                let Some(&parcel) = self.world.stations.queue[s]
                    .iter()
                    .find(|p| self.world.parcels.robot[p.index()].is_none())
                else {
                    break;
                };
                // Only robots that can reach the station (all of them on a
                // healthy floor).
                let Some((_, i)) = (0..self.idle.len())
                    .map(|i| {
                        let r = self.idle[i];
                        let d = self.distances.get(goal, self.world.robots.pos[r as usize]);
                        ((d, r), i)
                    })
                    .filter(|&((d, _), _)| d != UNREACHABLE)
                    .min()
                else {
                    break;
                };
                let r = self.idle.swap_remove(i) as usize;
                let robot = robot_id(r);
                self.world.stations.inbound[s] += 1;
                self.world.parcels.robot[parcel.index()] = Some(robot);
                self.world.robots.parcel[r] = Some(parcel);
                self.world.robots.goal[r] = Some(goal);
                self.world.robots.goal_since[r] = t;
                out.push(Event::ParcelAssigned {
                    tick: t,
                    parcel,
                    robot,
                    station,
                });
                self.set_phase(t, parcel, Phase::AwaitingRobot, out);
                self.set_robot_state(t, r, RobotState::ToInduction, out);
            }
        }

        // Throttling: surplus idle robots park while spots remain.
        let robots = &mut self.world.robots;
        let mut parking = robots
            .goal
            .iter()
            .filter(|&&g| g == Some(Goal::Park))
            .count();
        for r in 0..robots.len() {
            if parking >= self.layout.parking.len() {
                break;
            }
            if robots.state[r] == RobotState::Idle && robots.goal[r].is_none() {
                robots.goal[r] = Some(Goal::Park);
                robots.goal_since[r] = t;
                parking += 1;
            }
        }
    }

    /// A robot driving to a station that closed cells have cut it off from
    /// gives its claim back: the robot becomes idle (and can take work it
    /// can reach) and the parcel returns to `Queued`, to be assigned again
    /// (a second `ParcelAssigned`). A failed robot keeps its claim: it is
    /// `Failed`, not `ToInduction`.
    fn release_cut_off_claims(&mut self, t: Tick, out: &mut Vec<Event>) {
        for r in 0..self.world.robots.len() {
            let robots = &self.world.robots;
            let (RobotState::ToInduction, Some(goal @ Goal::Station(s)), Some(parcel)) =
                (robots.state[r], robots.goal[r], robots.parcel[r])
            else {
                continue;
            };
            // A loader driving back after being cleared off the station
            // holds a parcel already in `Loading`: it keeps its claim.
            if self.distances.get(goal, robots.pos[r]) != UNREACHABLE
                || self.world.parcels.phase[parcel.index()] != Phase::AwaitingRobot
            {
                continue;
            }
            self.world.robots.parcel[r] = None;
            self.world.robots.goal[r] = None;
            self.world.robots.goal_since[r] = t;
            self.world.parcels.robot[parcel.index()] = None;
            self.world.stations.inbound[s.index()] -= 1;
            self.set_phase(t, parcel, Phase::Queued, out);
            self.set_robot_state(t, r, RobotState::Idle, out);
        }
    }
}
