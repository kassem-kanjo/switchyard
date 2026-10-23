//! Blame aggregation: who held up how many blocked parcel-ticks.
//!
//! Each `Blocked` tick of a parcel's trace is charged to the direct blocker
//! and to the root cause recorded on it (ARCHITECTURE §4). A robot carries
//! one parcel, so a blocked parcel-tick is a blocked robot-tick. Direct
//! blame says who was in the way; root blame says who started it. Both sum
//! to the total blocked time, so nothing is double counted within one view.
//!
//! A second view counts **fault delay in every phase**: each span annotated
//! with a `ParcelDelayed` (see [`crate::SpanAttrs::delay_root`]) charges its ticks
//! to the fault the root names, and to the phase the parcel was in. That view
//! also sums to its own total: the sum over faults, the sum over phases and
//! the sum over (fault, phase) cells all equal
//! [`BlameIndex::total_fault_delay_ticks`]. The two views overlap on blocked
//! ticks a fault caused (those are in both), so do not add them.

use std::collections::BTreeMap;

use switchyard_core::events::WaitCause;
use switchyard_core::ids::FaultId;
use switchyard_core::world::Phase;

use crate::model::{ParcelTrace, Span};

/// One row of a blame ranking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Blame {
    /// The cause.
    pub cause: WaitCause,
    /// Blocked robot-ticks charged to it.
    pub ticks: u64,
}

/// Blocked robot-ticks per root cause and per direct blocker.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlameIndex {
    root: BTreeMap<WaitCause, u64>,
    direct: BTreeMap<WaitCause, u64>,
    unattributed: u64,
    total: u64,
    /// Fault-delayed parcel-ticks by (fault, phase).
    fault_phase: BTreeMap<(FaultId, Phase), u64>,
}

impl BlameIndex {
    /// An empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Charge one span. Spans that are not `Blocked` are ignored.
    pub fn add_span(&mut self, span: &Span) {
        if let Some(fault) = span.attrs.fault() {
            *self.fault_phase.entry((fault, span.phase)).or_default() += u64::from(span.ticks());
        }
        if span.phase != Phase::Blocked {
            return;
        }
        let ticks = u64::from(span.ticks());
        self.total += ticks;
        match (span.attrs.direct_blocker, span.attrs.root_cause) {
            (Some(direct), Some(root)) => {
                *self.direct.entry(direct).or_default() += ticks;
                *self.root.entry(root).or_default() += ticks;
            }
            _ => self.unattributed += ticks,
        }
    }

    /// Charge every `Blocked` span of a trace (complete or open).
    pub fn add_trace(&mut self, trace: &ParcelTrace) {
        for span in &trace.spans {
            self.add_span(span);
        }
    }

    /// Parcel-ticks that `fault` delayed, in any phase.
    #[must_use]
    pub fn fault_delay(&self, fault: FaultId) -> u64 {
        self.fault_phase
            .range((fault, Phase::Queued)..)
            .take_while(|((f, _), _)| *f == fault)
            .map(|(_, &t)| t)
            .sum()
    }

    /// Parcel-ticks that `fault` delayed while parcels were in `phase`.
    #[must_use]
    pub fn fault_delay_in(&self, fault: FaultId, phase: Phase) -> u64 {
        self.fault_phase.get(&(fault, phase)).copied().unwrap_or(0)
    }

    /// Parcel-ticks any fault delayed while parcels were in `phase`.
    #[must_use]
    pub fn fault_delay_by_phase(&self, phase: Phase) -> u64 {
        self.fault_phase
            .iter()
            .filter(|((_, p), _)| *p == phase)
            .map(|(_, &t)| t)
            .sum()
    }

    /// Parcel-ticks any fault delayed, in any phase.
    #[must_use]
    pub fn total_fault_delay_ticks(&self) -> u64 {
        self.fault_phase.values().sum()
    }

    /// Every fault with delay, with its total, in fault order.
    #[must_use]
    pub fn fault_delays(&self) -> Vec<(FaultId, u64)> {
        let mut out: Vec<(FaultId, u64)> = Vec::new();
        for (&(f, _), &t) in &self.fault_phase {
            match out.last_mut() {
                Some((last, sum)) if *last == f => *sum += t,
                _ => out.push((f, t)),
            }
        }
        out
    }

    /// Total blocked robot-ticks seen.
    #[must_use]
    pub const fn total_blocked_ticks(&self) -> u64 {
        self.total
    }

    /// Blocked ticks that carried no cause (the stream had no wait event).
    #[must_use]
    pub const fn unattributed_ticks(&self) -> u64 {
        self.unattributed
    }

    /// Ticks charged to `cause` as a root cause.
    #[must_use]
    pub fn root_ticks(&self, cause: WaitCause) -> u64 {
        self.root.get(&cause).copied().unwrap_or(0)
    }

    /// Ticks charged to `cause` as a direct blocker.
    #[must_use]
    pub fn direct_ticks(&self, cause: WaitCause) -> u64 {
        self.direct.get(&cause).copied().unwrap_or(0)
    }

    /// The `n` root causes with the most blocked ticks, largest first
    /// (ties by cause order, so the ranking is deterministic).
    #[must_use]
    pub fn top_roots(&self, n: usize) -> Vec<Blame> {
        top(&self.root, n)
    }

    /// The `n` direct blockers with the most blocked ticks, largest first.
    #[must_use]
    pub fn top_direct(&self, n: usize) -> Vec<Blame> {
        top(&self.direct, n)
    }

    /// Every root cause with its ticks, in cause order.
    pub fn roots(&self) -> impl Iterator<Item = Blame> + '_ {
        self.root
            .iter()
            .map(|(&cause, &ticks)| Blame { cause, ticks })
    }

    /// Every direct blocker with its ticks, in cause order.
    pub fn directs(&self) -> impl Iterator<Item = Blame> + '_ {
        self.direct
            .iter()
            .map(|(&cause, &ticks)| Blame { cause, ticks })
    }
}

fn top(map: &BTreeMap<WaitCause, u64>, n: usize) -> Vec<Blame> {
    let mut rows: Vec<Blame> = map
        .iter()
        .map(|(&cause, &ticks)| Blame { cause, ticks })
        .collect();
    rows.sort_by(|a, b| b.ticks.cmp(&a.ticks).then(a.cause.cmp(&b.cause)));
    rows.truncate(n);
    rows
}

#[cfg(test)]
mod tests {
    use switchyard_core::ids::{ChuteId, RobotId};

    use super::*;
    use crate::model::SpanAttrs;

    fn blocked(start: u32, end: u32, direct: WaitCause, root: WaitCause) -> Span {
        Span {
            phase: Phase::Blocked,
            start,
            end,
            attrs: SpanAttrs {
                robot: None,
                direct_blocker: Some(direct),
                root_cause: Some(root),
                ..SpanAttrs::default()
            },
        }
    }

    #[test]
    fn ranks_by_ticks_then_cause_and_conserves_the_total() {
        let r = |n| WaitCause::BlockedBy(RobotId(n));
        let chute = WaitCause::ChuteUnavailable(ChuteId(4));
        let mut idx = BlameIndex::new();
        idx.add_span(&blocked(0, 5, r(1), chute));
        idx.add_span(&blocked(5, 7, r(2), chute));
        idx.add_span(&blocked(7, 9, r(2), r(9)));
        idx.add_span(&blocked(9, 11, r(3), r(8)));
        idx.add_span(&Span {
            phase: Phase::Blocked,
            start: 11,
            end: 12,
            attrs: SpanAttrs::default(),
        });
        idx.add_span(&Span {
            phase: Phase::Travelling,
            start: 12,
            end: 30,
            attrs: SpanAttrs::default(),
        });
        assert_eq!(idx.total_blocked_ticks(), 12);
        assert_eq!(idx.unattributed_ticks(), 1);
        assert_eq!(
            idx.top_roots(2),
            vec![
                Blame {
                    cause: chute,
                    ticks: 7
                },
                Blame {
                    cause: r(8),
                    ticks: 2
                }
            ]
        );
        assert_eq!(
            idx.top_direct(1),
            vec![Blame {
                cause: r(1),
                ticks: 5
            }]
        );
        let root_sum: u64 = idx.roots().map(|b| b.ticks).sum();
        let direct_sum: u64 = idx.directs().map(|b| b.ticks).sum();
        assert_eq!(
            root_sum + idx.unattributed_ticks(),
            idx.total_blocked_ticks()
        );
        assert_eq!(direct_sum, root_sum);
        assert_eq!(idx.root_ticks(chute), 7);
        assert_eq!(idx.root_ticks(r(77)), 0);
    }
}
