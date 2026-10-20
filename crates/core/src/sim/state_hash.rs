//! The canonical state hash (ADR 0002 §5).

use super::Sim;
use crate::hash::Fnv1a;
use crate::world::Goal;

impl Sim {
    /// FNV-1a hash of a canonical encoding of the complete sim state: tick,
    /// every entity column in id order, counters and the position of every
    /// random stream (ADR 0002 §5). Equal hashes across platforms and between
    /// native and WASM are the determinism check.
    #[must_use]
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv1a::new();
        h.write(b"switchyard-state-v2");
        h.write_u32(self.tick);
        h.write_u64(self.arrived);
        h.write_u64(self.delivered);
        h.write_u64(self.mis_sorted);
        h.write_u32(self.last_progress);
        h.write_u32(self.last_stall);

        let r = &self.world.robots;
        h.write_u64(r.len() as u64);
        for i in 0..r.len() {
            h.write_u32(r.pos[i]);
            h.write_u8(r.state[i] as u8);
            h.write_opt_u32(r.parcel[i].map(|p| p.0));
            match r.goal[i] {
                None => h.write_u8(0),
                Some(Goal::Station(s)) => {
                    h.write_u8(1);
                    h.write_u16(s.0);
                }
                Some(Goal::Chute(c)) => {
                    h.write_u8(2);
                    h.write_u16(c.0);
                }
                Some(Goal::Park) => h.write_u8(3),
            }
            h.write_u16(r.timer[i]);
            h.write_u32(r.waited[i]);
            h.write_u32(r.goal_since[i]);
        }

        let p = &self.world.parcels;
        h.write_u64(p.len() as u64);
        for i in 0..p.len() {
            h.write_u16(p.station[i].0);
            h.write_u32(p.ordinal[i]);
            h.write_u16(p.destination[i].0);
            h.write_u32(p.arrived[i]);
            h.write_u8(p.phase[i] as u8);
            h.write_u32(p.phase_since[i]);
            h.write_opt_u32(p.robot[i].map(|r| r.0));
            h.write_opt_u32(p.target[i].map(|c| u32::from(c.0)));
            h.write_opt_u32(p.chute[i].map(|c| u32::from(c.0)));
        }

        let s = &self.world.stations;
        for i in 0..s.cell.len() {
            h.write_u32(s.cell[i]);
            h.write_u16(s.inbound[i]);
            h.write_u32(s.arrived[i]);
            h.write_u32(self.arrival_rates[i]);
            h.write_u64(s.queue[i].len() as u64);
            for q in &s.queue[i] {
                h.write_u32(q.0);
            }
        }

        let c = &self.world.chutes;
        for i in 0..c.cell.len() {
            h.write_u32(c.cell[i]);
            h.write_u32(c.fill[i]);
            h.write_u8(c.state[i] as u8);
            h.write_u32(c.timer[i]);
            h.write_u16(c.incoming[i]);
        }
        h.write_u64(self.world.destination_chute.len() as u64);
        for d in &self.world.destination_chute {
            h.write_u16(d.0);
        }

        for st in self
            .arrival_streams
            .iter()
            .chain(&self.destination_streams)
            .chain(&self.scanner_streams)
        {
            h.write_u128(st.word_pos());
        }
        h.write_u32(self.scanner_ppm);
        self.planner.hash_state(&mut h);
        h.finish()
    }
}
