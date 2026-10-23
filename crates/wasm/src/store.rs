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

/// Total spans held across all traces (about 50 bytes each, so roughly
/// 100 MB at the cap).
pub const SPAN_CAP: usize = 2_000_000;

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
    /// Which push created this entry; deque slots from older pushes of the
    /// same parcel id are stale and skipped (lazy deletion).
    generation: u64,
    in_recent: bool,
    late: bool,
}

/// Recent plus late completed traces, by parcel id, bounded by trace count
/// and by the total number of spans held (late traces are long: a trace can
/// have over a thousand spans).
pub struct TraceStore {
    map: BTreeMap<u32, Entry>,
    /// `(parcel, generation)` in push order; may hold stale slots.
    recent: VecDeque<(u32, u64)>,
    late: VecDeque<(u32, u64)>,
    live_recent: usize,
    live_late: usize,
    spans: usize,
    next_generation: u64,
    recent_cap: usize,
    late_cap: usize,
    span_cap: usize,
}

impl Default for TraceStore {
    fn default() -> Self {
        Self::with_caps(RECENT_CAP, LATE_CAP)
    }
}

impl TraceStore {
    /// A store with custom trace-count caps (tests use small ones) and the
    /// default span cap.
    #[must_use]
    pub fn with_caps(recent_cap: usize, late_cap: usize) -> Self {
        Self {
            map: BTreeMap::new(),
            recent: VecDeque::new(),
            late: VecDeque::new(),
            live_recent: 0,
            live_late: 0,
            spans: 0,
            next_generation: 0,
            recent_cap,
            late_cap,
            span_cap: SPAN_CAP,
        }
    }

    /// Replace the cap on total spans held.
    #[must_use]
    pub fn with_span_cap(mut self, span_cap: usize) -> Self {
        self.span_cap = span_cap;
        self
    }

    /// Add a completed trace, evicting the oldest as needed. Pushing a
    /// parcel id that is already held replaces the old trace.
    pub fn push(&mut self, trace: ParcelTrace) {
        let id = trace.parcel.0;
        if let Some(old) = self.map.remove(&id) {
            // Its deque slots are now stale (different generation).
            self.spans -= old.trace.spans.len();
            self.live_recent -= usize::from(old.in_recent);
            self.live_late -= usize::from(old.late);
        }
        let late = is_late(&trace);
        let generation = self.next_generation;
        self.next_generation += 1;
        self.spans += trace.spans.len();
        self.map.insert(
            id,
            Entry {
                trace,
                generation,
                in_recent: true,
                late,
            },
        );
        self.recent.push_back((id, generation));
        self.live_recent += 1;
        if late {
            self.late.push_back((id, generation));
            self.live_late += 1;
        }
        while self.live_recent > self.recent_cap && self.evict_recent() {}
        while self.live_late > self.late_cap && self.evict_late() {}
        // Over the span budget: shed the oldest recent traces first, then the
        // oldest late ones, but never the trace just pushed.
        while self.spans > self.span_cap
            && self.map.len() > 1
            && (self.evict_recent() || self.evict_late())
        {}
    }

    /// Drop the oldest live recent slot; false when none is left.
    fn evict_recent(&mut self) -> bool {
        while let Some((id, generation)) = self.recent.pop_front() {
            if self.is_live(id, generation) {
                self.release(id, |e| e.in_recent = false);
                self.live_recent -= 1;
                return true;
            }
        }
        false
    }

    /// Drop the oldest live late slot; false when none is left.
    fn evict_late(&mut self) -> bool {
        while let Some((id, generation)) = self.late.pop_front() {
            if self.is_live(id, generation) {
                self.release(id, |e| e.late = false);
                self.live_late -= 1;
                return true;
            }
        }
        false
    }

    fn is_live(&self, id: u32, generation: u64) -> bool {
        self.map
            .get(&id)
            .is_some_and(|e| e.generation == generation)
    }

    fn release(&mut self, id: u32, clear: impl FnOnce(&mut Entry)) {
        if let Some(e) = self.map.get_mut(&id) {
            clear(e);
            if !e.in_recent
                && !e.late
                && let Some(gone) = self.map.remove(&id)
            {
                self.spans -= gone.trace.spans.len();
            }
        }
    }

    /// The stored trace of a parcel, if it is still held.
    #[must_use]
    pub fn get(&self, parcel: u32) -> Option<&ParcelTrace> {
        self.map.get(&parcel).map(|e| &e.trace)
    }

    /// Spans held across all traces.
    #[must_use]
    pub fn span_count(&self) -> usize {
        self.spans
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
    fn span_cap_sheds_the_oldest_traces_first() {
        // Each trace has 2 spans when late (blocked > 0), 1 otherwise.
        let mut s = TraceStore::with_caps(100, 100).with_span_cap(5);
        for i in 0..10 {
            s.push(trace(i, 30)); // 2 spans each, all late
            assert!(s.span_count() <= 5, "{}", s.span_count());
        }
        assert!(s.get(9).is_some(), "the newest survives");
        assert!(s.get(0).is_none());
        assert_eq!(s.span_count(), s.len() * 2);
    }

    #[test]
    fn one_oversized_trace_is_still_kept() {
        let mut s = TraceStore::with_caps(10, 10).with_span_cap(1);
        s.push(trace(1, 30));
        assert!(s.get(1).is_some());
    }

    #[test]
    fn re_pushing_a_parcel_replaces_it_without_corrupting_eviction() {
        let mut s = TraceStore::with_caps(2, 2);
        s.push(trace(1, 0));
        s.push(trace(2, 0));
        s.push(trace(1, 30)); // replaces; parcel 1 is now the newest
        assert_eq!(s.len(), 2);
        assert!(s.get(1).unwrap().spans.len() == 2, "fresh trace held");
        s.push(trace(3, 0)); // evicts the oldest live: parcel 2, not 1
        assert!(s.get(2).is_none());
        assert!(s.get(1).is_some(), "the stale slot must not evict it");
        assert!(s.get(3).is_some());
        assert_eq!(s.span_count(), 2 + 1);
    }

    #[test]
    fn short_blocks_are_not_late() {
        assert!(!is_late(&trace(0, LATE_BLOCKED_TICKS - 1)));
        assert!(is_late(&trace(0, LATE_BLOCKED_TICKS)));
    }
}
