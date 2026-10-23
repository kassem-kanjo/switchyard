# Switchyard: status

**Updated:** 2026-10-21 · **Phase:** CP2 fixes done → CP2 re-review (double review) → CP3

## Now
- V0 `v0.1.0` (PR #2), CP1 merged (PR #3).
- CP2 (PR #4): causal roots, fault-aware blame in every phase, traces and exporters, chaos with client-correct metrics, plus CP3 groundwork (checkpoints, replays, interventions), the viewer wired to the real sim, and CI determinism jobs. 186 Rust + 45 viewer tests. See `docs/checkpoints/v1-cp2.md`.

## Next
1. CP2 re-review: an independent Opus reviewer plus `/code-review high`; `/simplify`; merge.
2. CP3: post-mortem engine per ADR 0006 (CUSUM, fixed-horizon parcel-minute delay, dithered replays, exact Shapley via Möbius, interventions), `switchyard postmortem`, viewer post-mortem panel.
3. V1: README numbers, demo GIF/video, `/security-review`, tag `v1.0.0`, CEO review.

## Blockers
- None.

## Key findings
- Most fault damage spreads as congestion; replay attribution captures it (README story).
- A robot dying on an induction cell costs ~10× more than one dying in an aisle.
