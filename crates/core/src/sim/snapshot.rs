//! The viewer snapshot: flat buffers copied out of the sim each frame.

use super::Sim;
use crate::Tick;

/// A flat, allocation-reusing view of the sim for the viewer (ARCHITECTURE
/// §8). The buffers map directly onto typed arrays in the WASM facade.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// The tick this snapshot was taken at.
    pub tick: Tick,
    /// Interleaved robot coordinates: `[x0, y0, x1, y1, ...]`.
    pub robot_xy: Vec<u16>,
    /// Robot states as [`crate::world::RobotState`] `u8` codes, one per robot.
    pub robot_state: Vec<u8>,
    /// Parcels in each chute's bag, saturated at `u16::MAX`.
    pub chute_fill: Vec<u16>,
    /// Parcels waiting at each station, saturated at `u16::MAX`.
    pub station_queue: Vec<u16>,
}

impl Sim {
    /// Fill `snap` with the current state, reusing its buffers.
    pub fn snapshot(&self, snap: &mut Snapshot) {
        let g = &self.layout.grid;
        snap.tick = self.tick;
        snap.robot_xy.clear();
        for &c in &self.world.robots.pos {
            let p = g.pos(c);
            snap.robot_xy.push(p.x);
            snap.robot_xy.push(p.y);
        }
        snap.robot_state.clear();
        snap.robot_state
            .extend(self.world.robots.state.iter().map(|&s| s as u8));
        snap.chute_fill.clear();
        snap.chute_fill.extend(
            self.world
                .chutes
                .fill
                .iter()
                .map(|&f| u16::try_from(f).unwrap_or(u16::MAX)),
        );
        snap.station_queue.clear();
        snap.station_queue.extend(
            self.world
                .stations
                .queue
                .iter()
                .map(|q| u16::try_from(q.len()).unwrap_or(u16::MAX)),
        );
    }
}
