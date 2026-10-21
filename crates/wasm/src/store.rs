//! A bounded store of completed parcel traces.
//!
//! The viewer asks for traces by parcel id, long after the parcel was
//! dropped. Keeping every trace of a long session would grow without bound,
//! so the store keeps the most recent [`RECENT_CAP`] completed traces plus
//! the *late* ones (the interesting ones: delayed by a fault, or blocked for
//! a while), up to [`LATE_CAP`].

use std::collections::{BTreeMap, VecDeque};

use switchyard_core::world::Phase;
use switchyard_trace::ParcelTrace;

/// Most recent completed traces kept.
pub const RECENT_CAP: usize = 5_000;

/// Late traces kept in addition to the recent ones.
pub const LATE_CAP: usize = 20_000;

/// A blocked span at least this long (10 s) makes a trace "late".
pub const LATE_BLOCKED_TICKS: u32 = 20;

/// Is this trace worth keeping beyond the recent window?
#[must_use]
pub fn is_late(trace: &ParcelTrace) -> bool {
    trace.spans.iter().any(|s| {
        s.attrs.fault().is_some() || (s.phase == Phase::Blocked && s.ticks() >= LATE_BLOCKED_TICKS)
    })
}

struct Entry {
    trace: ParcelTrace,
    in_recent: bool,
    late: bool,
}

/// Recent plus late completed traces, by parcel id.
pub struct TraceStore {
    map: BTreeMap<u32, Entry>,
    recent: VecDeque<u32>,
    late: VecDeque<u32>,
    recent_cap: usize,
    late_cap: usize,
}

impl Default for TraceStore {
    fn default() -> Self {
        Self::with_caps(RECENT_CAP, LATE_CAP)
    }
}

impl TraceStore {
    /// A store with custom caps (tests use small ones).
    #[must_use]
    pub fn with_caps(recent_cap: usize, late_cap: usize) -> Self {
        Self {
            map: BTreeMap::new(),
            recent: VecDeque::new(),
            late: VecDeque::new(),
            recent_cap,
            late_cap,
        }
    }

    /// Add a completed trace, evicting the oldest as needed.
    pub fn push(&mut self, trace: ParcelTrace) {
        let id = trace.parcel.0;
        let late = is_late(&trace);
        self.map.insert(
            id,
            Entry {
                trace,
                in_recent: true,
                late,
            },
        );
        self.recent.push_back(id);
        if late {
            self.late.push_back(id);
        }
        while self.recent.len() > self.recent_cap {
            if let Some(old) = self.recent.pop_front() {
                self.release(old, |e| e.in_recent = false);
            }
        }
        while self.late.len() > self.late_cap {
            if let Some(old) = self.late.pop_front() {
                self.release(old, |e| e.late = false);
            }
        }
    }

    fn release(&mut self, id: u32, clear: impl FnOnce(&mut Entry)) {
        if let Some(e) = self.map.get_mut(&id) {
            clear(e);
            if !e.in_recent && !e.late {
                self.map.remove(&id);
            }
        }
    }

    /// The stored trace of a parcel, if it is still held.
    #[must_use]
    pub fn get(&self, parcel: u32) -> Option<&ParcelTrace> {
        self.map.get(&parcel).map(|e| &e.trace)
    }

    /// Traces held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True when nothing is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use switchyard_core::ids::{DestinationId, ParcelId, StationId};
    use switchyard_core::world::Phase;
    use switchyard_trace::{Span, SpanAttrs, TraceStatus};

    fn trace(id: u32, blocked: u32) -> ParcelTrace {
        let mut spans = vec![Span {
            phase: Phase::Travelling,
            start: 0,
            end: 10,
            attrs: SpanAttrs::default(),
        }];
        if blocked > 0 {
            spans.push(Span {
                phase: Phase::Blocked,
                start: 10,
                end: 10 + blocked,
                attrs: SpanAttrs::default(),
            });
        }
        ParcelTrace {
            parcel: ParcelId(id),
            station: StationId(0),
            destination: DestinationId(0),
            arrival: 0,
            done: 10 + blocked,
            status: TraceStatus::Complete,
            chute: None,
            robots: vec![],
            mis_sorted: false,
            spans,
        }
    }

    #[test]
    fn recent_window_is_bounded_and_late_traces_survive() {
        let mut s = TraceStore::with_caps(3, 2);
        s.push(trace(0, 30)); // late
        for i in 1..=6 {
            s.push(trace(i, 0));
        }
        assert!(s.get(0).is_some(), "late trace kept");
        assert!(s.get(1).is_none(), "old ordinary trace evicted");
        assert!(s.get(6).is_some());
        assert_eq!(s.len(), 4); // 3 recent + 1 late
    }

    #[test]
    fn late_cap_evicts_oldest_late() {
        let mut s = TraceStore::with_caps(1, 2);
        for i in 0..4 {
            s.push(trace(i, 30));
        }
        assert!(s.get(0).is_none() && s.get(1).is_none());
        assert!(s.get(2).is_some() && s.get(3).is_some());
    }

    #[test]
    fn short_blocks_are_not_late() {
        assert!(!is_late(&trace(0, LATE_BLOCKED_TICKS - 1)));
        assert!(is_late(&trace(0, LATE_BLOCKED_TICKS)));
    }
}
