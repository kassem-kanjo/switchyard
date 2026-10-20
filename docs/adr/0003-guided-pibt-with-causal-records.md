# 0003. Guided PIBT with causal records for robot coordination

**Status:** accepted · **Date:** 2026-10-19 · **Research:** [robot coordination](../research/2026-10-19-robot-coordination.md)

## Context
200–1,000 robots must be coordinated every tick (0.5 s simulated) and run far faster than real time, deterministically. Every wait must have an explainable cause for tracing.

## Decision
- **Planner:** PIBT (Okumura et al.), ported natively to Rust, with Kei18/pibt2 (MIT) as the reference. It costs about 1–1.5 ms per step at 1,000 agents in the literature, and it is deterministic with id-based tie-breaking.
- **Guidance:** candidate moves are ranked by distance-to-goal on a **weighted guidance graph** (one-way-preferred lanes). Guidance roughly tripled PIBT's throughput on the Sortation benchmark in the literature.
- **Throttling:** if the floor is over-saturated, idle robots park instead of entering.
- **Causal records:** the planner records, for every robot that does not progress, `BlockedBy | Yielded | CellBlocked | ChuteUnavailable | StationBusy`. Priority inheritance gives these edges directly.
- **Assignment:** greedy. Each idle robot takes the oldest parcel at the nearest station with work. A destination-to-chute mapping strategy is configurable (round-robin, or popularity-aware with busy destinations near induction).
- **Robustness:** a watchdog detects wait-for cycles (deadlock or livelock) and reports them as a root cause. After topology-changing faults we check that the free-cell graph is still connected.

## Alternatives considered
- **RHCR:** 2 s per step at 1,000 agents, and a non-permissive licence.
- **LaCAM / Lifelong LaCAM:** stronger completeness, but 30–200 ms per step. Kept as a V2 stuck-recovery option.
- **WPPL / EPIBT with LNS:** competition winners, but their wall-clock-bounded parallel search breaks bit-reproducibility, and their waits have no local cause.
- **Hungarian assignment:** the 2024 competition winner abandoned it in favour of greedy.

## Consequences
- Throughput depends on layout guidance, which becomes a tunable product surface.
- PIBT can stall in dead-end aisles. Layouts avoid them, and the watchdog makes stalls visible rather than silent.
