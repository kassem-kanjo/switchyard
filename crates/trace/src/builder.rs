//! The streaming trace builder.
//!
//! Feed it the event stream one tick (or several whole ticks) at a time.
//! It keeps state only for parcels that are in flight, so memory is bounded
//! by the number of parcels on the floor, not by the length of the run.
//!
//! How a trace is cut from the stream:
//!
//! - `ParcelArrived` opens a trace in `Queued`; each `ParcelPhase` closes
//!   the current span and opens the next (the sim emits one only on a real
//!   change, so consecutive events never repeat a phase).
//! - `ParcelPhase { Dropped }` completes the trace. `done` is that tick:
//!   `Dropped` is an instant, not a span.
//! - A `Blocked` phase is cut into sub-spans by the carrying robot's
//!   `RobotWaited` events: consecutive ticks with the same
//!   `(direct, root)` merge into one span. A `Blocked` tick with no
//!   `RobotWaited` becomes a span with no blocker (unattributed).
//! - A robot that fails mid-delivery changes nothing special: the parcel
//!   stays in whatever phase the stream says, and a later `ParcelAssigned`
//!   adds the replacement robot to the trace.
//!
//! Events of one tick are processed in two passes (state changes, then
//! waits), so the result does not depend on the order of events within a
//! tick. Pass whole ticks to [`TraceBuilder::ingest`].

use std::collections::BTreeMap;

use switchyard_core::Tick;
use switchyard_core::events::{Event, WaitCause};
use switchyard_core::ids::{ChuteId, DestinationId, ParcelId, RobotId, StationId};
use switchyard_core::world::Phase;

use crate::model::{ParcelTrace, Span, SpanAttrs, TraceStatus};

type Key = (WaitCause, WaitCause);

/// The `Blocked` sub-span being accumulated.
#[derive(Clone, Copy, Debug)]
struct Run {
    start: Tick,
    /// One past the last tick accounted to this run.
    end: Tick,
    key: Option<Key>,
}

/// State of a parcel that has not been dropped yet.
#[derive(Clone, Debug)]
struct InFlight {
    parcel: ParcelId,
    station: StationId,
    destination: DestinationId,
    arrival: Tick,
    robot: Option<RobotId>,
    robots: Vec<RobotId>,
    phase: Phase,
    phase_start: Tick,
    /// Robot at the start of the current phase.
    phase_robot: Option<RobotId>,
    /// While `phase == Blocked`: the next tick not yet accounted, and the
    /// sub-span in progress.
    next_tick: Tick,
    run: Option<Run>,
    spans: Vec<Span>,
    chute: Option<ChuteId>,
    mis_sorted: bool,
    dropped_at: Option<Tick>,
}

impl InFlight {
    fn push_run(&mut self, run: Run) {
        self.spans.push(Span {
            phase: Phase::Blocked,
            start: run.start,
            end: run.end,
            attrs: SpanAttrs {
                robot: self.phase_robot,
                direct_blocker: run.key.map(|k| k.0),
                root_cause: run.key.map(|k| k.1),
            },
        });
    }

    /// Account ticks `[self.next_tick, to)` to `key`, extending the open run
    /// when the key matches.
    fn account(&mut self, to: Tick, key: Option<Key>) {
        if to <= self.next_tick {
            return;
        }
        match &mut self.run {
            Some(run) if run.key == key && run.end == self.next_tick => run.end = to,
            _ => {
                if let Some(run) = self.run.take() {
                    self.push_run(run);
                }
                self.run = Some(Run {
                    start: self.next_tick,
                    end: to,
                    key,
                });
            }
        }
        self.next_tick = to;
    }

    /// Close the current phase at tick `t`, pushing its span(s).
    fn close_phase(&mut self, t: Tick) {
        if self.phase == Phase::Blocked {
            self.account(t, None);
            if let Some(run) = self.run.take() {
                self.push_run(run);
            }
        } else if t > self.phase_start {
            self.spans.push(Span {
                phase: self.phase,
                start: self.phase_start,
                end: t,
                attrs: SpanAttrs {
                    robot: self.phase_robot,
                    ..SpanAttrs::default()
                },
            });
        }
    }

    fn enter_phase(&mut self, t: Tick, phase: Phase) {
        if phase == self.phase {
            return;
        }
        self.close_phase(t);
        self.phase = phase;
        self.phase_start = t;
        self.phase_robot = self.robot;
        self.next_tick = t;
        self.run = None;
    }

    /// Close at `t` and produce the trace.
    fn into_trace(mut self, t: Tick, status: TraceStatus) -> ParcelTrace {
        // A snapshot can be asked for a tick before the last event only by
        // mistake; never let `done` precede the last span.
        let t = t.max(self.phase_start);
        self.close_phase(t);
        let trace = ParcelTrace {
            parcel: self.parcel,
            station: self.station,
            destination: self.destination,
            arrival: self.arrival,
            done: t,
            status,
            chute: self.chute,
            robots: self.robots,
            mis_sorted: self.mis_sorted,
            spans: self.spans,
        };
        debug_assert!(
            trace.check_tiling().is_ok(),
            "trace does not tile: {:?}",
            trace.check_tiling()
        );
        trace
    }
}

/// Builds [`ParcelTrace`]s from the event stream.
#[derive(Clone, Debug, Default)]
pub struct TraceBuilder {
    in_flight: BTreeMap<ParcelId, InFlight>,
    /// The parcel each robot is currently assigned.
    robot_parcel: BTreeMap<RobotId, ParcelId>,
}

impl TraceBuilder {
    /// An empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parcels that have arrived and not yet been dropped.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    /// Consume events (whole ticks, in tick order). Traces completed by
    /// these events are appended to `done`, in the order they completed.
    pub fn ingest(&mut self, events: &[Event], done: &mut Vec<ParcelTrace>) {
        let mut rest = events;
        while let Some(first) = rest.first() {
            let tick = first.tick();
            let len = rest.iter().take_while(|e| e.tick() == tick).count();
            let (run, tail) = rest.split_at(len);
            self.ingest_tick(tick, run, done);
            rest = tail;
        }
    }

    fn ingest_tick(&mut self, tick: Tick, events: &[Event], done: &mut Vec<ParcelTrace>) {
        let mut finished: Vec<ParcelId> = Vec::new();
        // Pass 1: arrivals, assignments, phase changes, drops.
        for event in events {
            match *event {
                Event::ParcelArrived {
                    parcel,
                    station,
                    destination,
                    ..
                } => {
                    self.in_flight.entry(parcel).or_insert_with(|| InFlight {
                        parcel,
                        station,
                        destination,
                        arrival: tick,
                        robot: None,
                        robots: Vec::new(),
                        phase: Phase::Queued,
                        phase_start: tick,
                        phase_robot: None,
                        next_tick: tick,
                        run: None,
                        spans: Vec::new(),
                        chute: None,
                        mis_sorted: false,
                        dropped_at: None,
                    });
                }
                Event::ParcelAssigned { parcel, robot, .. } => {
                    if let Some(p) = self.in_flight.get_mut(&parcel) {
                        if let Some(old) = p.robot
                            && old != robot
                            && self.robot_parcel.get(&old) == Some(&parcel)
                        {
                            self.robot_parcel.remove(&old);
                        }
                        p.robot = Some(robot);
                        if !p.robots.contains(&robot) {
                            p.robots.push(robot);
                        }
                        self.robot_parcel.insert(robot, parcel);
                    }
                }
                Event::ParcelPhase { parcel, phase, .. } => {
                    if let Some(p) = self.in_flight.get_mut(&parcel) {
                        if phase == Phase::Dropped {
                            if p.dropped_at.is_none() {
                                p.close_phase(tick);
                                p.dropped_at = Some(tick);
                                finished.push(parcel);
                            }
                        } else if p.dropped_at.is_none() {
                            p.enter_phase(tick, phase);
                        }
                    }
                }
                Event::ParcelDropped { parcel, chute, .. } => {
                    if let Some(p) = self.in_flight.get_mut(&parcel) {
                        p.chute = Some(chute);
                        if p.dropped_at.is_none() {
                            p.close_phase(tick);
                            p.dropped_at = Some(tick);
                            finished.push(parcel);
                        }
                    }
                }
                Event::ParcelMisSorted { parcel, chute, .. } => {
                    if let Some(p) = self.in_flight.get_mut(&parcel) {
                        p.mis_sorted = true;
                        p.chute = Some(chute);
                    }
                }
                // Everything else is irrelevant to parcel spans (or handled in
                // pass 2); new event kinds are ignored until they matter.
                _ => {}
            }
        }
        // Pass 2: waits attribute the carrying parcel's blocked ticks.
        for event in events {
            if let Event::RobotWaited {
                robot,
                direct,
                root,
                ..
            } = *event
                && let Some(parcel) = self.robot_parcel.get(&robot)
                && let Some(p) = self.in_flight.get_mut(parcel)
                && p.phase == Phase::Blocked
                && p.dropped_at.is_none()
            {
                p.account(tick, None);
                p.account(tick + 1, Some((direct, root)));
            }
        }
        for parcel in finished {
            if let Some(p) = self.in_flight.remove(&parcel) {
                if let Some(r) = p.robot
                    && self.robot_parcel.get(&r) == Some(&parcel)
                {
                    self.robot_parcel.remove(&r);
                }
                let at = p.dropped_at.unwrap_or(tick);
                // `close_phase` already ran at the drop; build the trace
                // without closing a second time.
                done.push(Self::complete(p, at));
            }
        }
    }

    fn complete(p: InFlight, at: Tick) -> ParcelTrace {
        let trace = ParcelTrace {
            parcel: p.parcel,
            station: p.station,
            destination: p.destination,
            arrival: p.arrival,
            done: at,
            status: TraceStatus::Complete,
            chute: p.chute,
            robots: p.robots,
            mis_sorted: p.mis_sorted,
            spans: p.spans,
        };
        debug_assert!(
            trace.check_tiling().is_ok(),
            "trace does not tile: {:?}",
            trace.check_tiling()
        );
        trace
    }

    /// Open traces for the parcels still in flight, closed at `end` (the
    /// tick the run stopped at, for example `sim.tick()`). The builder is
    /// not changed, so ingestion can continue. Sorted by parcel id.
    #[must_use]
    pub fn open_traces(&self, end: Tick) -> Vec<ParcelTrace> {
        let mut out: Vec<ParcelTrace> = self
            .in_flight
            .values()
            .map(|p| p.clone().into_trace(end, TraceStatus::Open))
            .collect();
        out.sort_by_key(|t| t.parcel);
        out
    }

    /// End the run: the open traces of every parcel still in flight.
    #[must_use]
    pub fn finish(self, end: Tick) -> Vec<ParcelTrace> {
        self.open_traces(end)
    }

    /// All traces of an event stream, complete and open, sorted by parcel id.
    #[must_use]
    pub fn build_all(events: &[Event], end: Tick) -> Vec<ParcelTrace> {
        let mut builder = Self::new();
        let mut all = Vec::new();
        builder.ingest(events, &mut all);
        all.extend(builder.finish(end));
        all.sort_by_key(|t| t.parcel);
        all
    }
}

#[cfg(test)]
mod tests {
    use switchyard_core::ids::FaultId;

    use super::*;

    fn arrive(tick: Tick, p: u32) -> [Event; 2] {
        [
            Event::ParcelArrived {
                tick,
                parcel: ParcelId(p),
                station: StationId(0),
                destination: DestinationId(3),
            },
            Event::ParcelPhase {
                tick,
                parcel: ParcelId(p),
                phase: Phase::Queued,
            },
        ]
    }

    fn phase(tick: Tick, p: u32, phase: Phase) -> Event {
        Event::ParcelPhase {
            tick,
            parcel: ParcelId(p),
            phase,
        }
    }

    fn assign(tick: Tick, p: u32, r: u32) -> Event {
        Event::ParcelAssigned {
            tick,
            parcel: ParcelId(p),
            robot: RobotId(r),
            station: StationId(0),
        }
    }

    fn waited(tick: Tick, r: u32, direct: WaitCause, root: WaitCause) -> Event {
        Event::RobotWaited {
            tick,
            robot: RobotId(r),
            direct,
            root,
        }
    }

    fn shape(t: &ParcelTrace) -> Vec<(Phase, Tick, Tick)> {
        t.spans.iter().map(|s| (s.phase, s.start, s.end)).collect()
    }

    /// A parcel with a mixed blocked stretch:
    /// 0 queued, 2 awaiting, 5 loading, 7 travelling, 9..13 blocked
    /// (9,10 by R1/root R2; 11 by R1/root R3; 12 gap), 13 travelling,
    /// 15 chute wait, 17 dropped.
    fn blocked_stream() -> Vec<Event> {
        let (r1, r2, r3) = (RobotId(1), RobotId(2), RobotId(3));
        let mut ev = Vec::new();
        ev.extend(arrive(0, 7));
        ev.push(assign(2, 7, 0));
        ev.push(phase(2, 7, Phase::AwaitingRobot));
        ev.push(phase(5, 7, Phase::Loading));
        ev.push(phase(7, 7, Phase::Travelling));
        ev.push(phase(9, 7, Phase::Blocked));
        ev.push(waited(
            9,
            0,
            WaitCause::BlockedBy(r1),
            WaitCause::BlockedBy(r2),
        ));
        ev.push(waited(
            10,
            0,
            WaitCause::BlockedBy(r1),
            WaitCause::BlockedBy(r2),
        ));
        ev.push(waited(
            11,
            0,
            WaitCause::BlockedBy(r1),
            WaitCause::BlockedBy(r3),
        ));
        // tick 12: no wait event
        ev.push(phase(13, 7, Phase::Travelling));
        ev.push(phase(15, 7, Phase::ChuteWait));
        ev.push(phase(17, 7, Phase::Dropped));
        ev.push(Event::ParcelDropped {
            tick: 17,
            parcel: ParcelId(7),
            robot: RobotId(0),
            chute: ChuteId(3),
        });
        ev
    }

    #[test]
    fn blocked_ticks_merge_by_cause_and_split_on_change() {
        let traces = TraceBuilder::build_all(&blocked_stream(), 20);
        assert_eq!(traces.len(), 1);
        let t = &traces[0];
        assert_eq!(t.status, TraceStatus::Complete);
        assert_eq!((t.arrival, t.done), (0, 17));
        assert_eq!(t.chute, Some(ChuteId(3)));
        assert_eq!(t.robots, vec![RobotId(0)]);
        assert_eq!(
            shape(t),
            vec![
                (Phase::Queued, 0, 2),
                (Phase::AwaitingRobot, 2, 5),
                (Phase::Loading, 5, 7),
                (Phase::Travelling, 7, 9),
                (Phase::Blocked, 9, 11),
                (Phase::Blocked, 11, 12),
                (Phase::Blocked, 12, 13),
                (Phase::Travelling, 13, 15),
                (Phase::ChuteWait, 15, 17),
            ]
        );
        let (a, b, gap) = (&t.spans[4], &t.spans[5], &t.spans[6]);
        assert_eq!(
            a.attrs.direct_blocker,
            Some(WaitCause::BlockedBy(RobotId(1)))
        );
        assert_eq!(a.attrs.root_cause, Some(WaitCause::BlockedBy(RobotId(2))));
        assert_eq!(b.attrs.root_cause, Some(WaitCause::BlockedBy(RobotId(3))));
        assert_eq!(gap.attrs.direct_blocker, None);
        assert_eq!(t.spans[3].attrs.robot, Some(RobotId(0)));
        assert_eq!(t.spans[0].attrs.robot, None);
        t.check_tiling().unwrap();
    }

    #[test]
    fn same_trace_for_any_slicing_and_event_order_within_a_tick() {
        let stream = blocked_stream();
        let whole = TraceBuilder::build_all(&stream, 20);
        // Tick by tick.
        let mut b = TraceBuilder::new();
        let mut done = Vec::new();
        for t in 0..20 {
            let slice: Vec<Event> = stream.iter().copied().filter(|e| e.tick() == t).collect();
            b.ingest(&slice, &mut done);
        }
        assert_eq!(done, whole);
        assert_eq!(b.in_flight(), 0);
        // Waits before the phase event of the same tick.
        let mut shuffled = stream.clone();
        shuffled.swap(6, 7); // Blocked phase vs first wait at tick 9
        assert!(matches!(shuffled[6], Event::RobotWaited { tick: 9, .. }));
        assert_eq!(TraceBuilder::build_all(&shuffled, 20), whole);
    }

    #[test]
    fn open_trace_is_closed_at_the_end_tick() {
        let stream = blocked_stream();
        let cut: Vec<Event> = stream.into_iter().filter(|e| e.tick() <= 11).collect();
        let traces = TraceBuilder::build_all(&cut, 12);
        let t = &traces[0];
        assert!(t.is_open());
        assert_eq!(t.done, 12);
        assert_eq!(
            t.spans.last().map(|s| (s.phase, s.start, s.end)),
            Some((Phase::Blocked, 11, 12))
        );
        t.check_tiling().unwrap();
        // Snapshots do not disturb the builder.
        let mut b = TraceBuilder::new();
        let mut done = Vec::new();
        b.ingest(&cut, &mut done);
        assert!(done.is_empty());
        assert_eq!(b.open_traces(12), b.open_traces(12));
        assert_eq!(b.in_flight(), 1);
    }

    #[test]
    fn robot_failing_mid_delivery_keeps_the_phase_and_records_both_robots() {
        let mut ev = Vec::new();
        ev.extend(arrive(0, 1));
        ev.push(assign(1, 1, 4));
        ev.push(phase(1, 1, Phase::AwaitingRobot));
        ev.push(phase(3, 1, Phase::Loading));
        ev.push(phase(4, 1, Phase::Travelling));
        ev.push(phase(6, 1, Phase::Blocked));
        // R4 dies; the blockage is a fault; nothing changes for the parcel
        // until a replacement robot is assigned.
        for t in 6..10 {
            ev.push(waited(
                t,
                4,
                WaitCause::CellBlocked(FaultId(2)),
                WaitCause::CellBlocked(FaultId(2)),
            ));
        }
        ev.push(assign(10, 1, 9));
        ev.push(phase(10, 1, Phase::AwaitingRobot));
        ev.push(phase(12, 1, Phase::Loading));
        ev.push(phase(13, 1, Phase::Travelling));
        let t = &TraceBuilder::build_all(&ev, 15)[0];
        assert_eq!(t.robots, vec![RobotId(4), RobotId(9)]);
        assert_eq!(
            shape(t),
            vec![
                (Phase::Queued, 0, 1),
                (Phase::AwaitingRobot, 1, 3),
                (Phase::Loading, 3, 4),
                (Phase::Travelling, 4, 6),
                (Phase::Blocked, 6, 10),
                (Phase::AwaitingRobot, 10, 12),
                (Phase::Loading, 12, 13),
                (Phase::Travelling, 13, 15),
            ]
        );
        assert_eq!(t.spans[4].attrs.robot, Some(RobotId(4)));
        assert_eq!(t.spans[5].attrs.robot, Some(RobotId(9)));
        assert!(t.is_open());
    }

    #[test]
    fn mis_sort_is_flagged_and_zero_length_phases_are_skipped() {
        let mut ev = Vec::new();
        ev.extend(arrive(3, 2));
        ev.push(assign(3, 2, 0));
        ev.push(phase(3, 2, Phase::AwaitingRobot)); // zero-length Queued
        ev.push(phase(5, 2, Phase::Loading));
        ev.push(phase(6, 2, Phase::ChuteWait));
        ev.push(phase(8, 2, Phase::Dropped));
        ev.push(Event::ParcelDropped {
            tick: 8,
            parcel: ParcelId(2),
            robot: RobotId(0),
            chute: ChuteId(1),
        });
        ev.push(Event::ParcelMisSorted {
            tick: 8,
            parcel: ParcelId(2),
            chute: ChuteId(1),
            expected: DestinationId(3),
        });
        let t = &TraceBuilder::build_all(&ev, 9)[0];
        assert!(t.mis_sorted);
        assert_eq!(t.spans.len(), 3);
        assert_eq!(t.spans[0].phase, Phase::AwaitingRobot);
        t.check_tiling().unwrap();
    }

    #[test]
    fn completed_parcels_are_forgotten() {
        let mut b = TraceBuilder::new();
        let mut done = Vec::new();
        b.ingest(&blocked_stream(), &mut done);
        assert_eq!(done.len(), 1);
        assert_eq!(b.in_flight(), 0);
        assert!(b.robot_parcel.is_empty());
    }
}
