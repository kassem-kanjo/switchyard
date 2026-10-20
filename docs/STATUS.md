# Switchyard: status

**Updated:** 2026-10-20 · **Phase:** CP1 done (gates passed) → CP2 next

## Now
- V0 merged and tagged `v0.1.0` (PR #2).
- CP1 (PR #3): guided PIBT with causal wait records, floor operations, calibrated presets (medium 31.4 parcels/h per robot), 66 tests, viewer presets and legend. Gate report: `docs/checkpoints/v1-cp1.md`.

## Next
1. CP2: causality (wait-for chain walk, cycle detection), trace builder, exporters (Perfetto, OTLP, JSONL), fault catalog and resilience score.
2. CP3: post-mortems (CUSUM, counterfactual replays, Shapley) and viewer features.

## Blockers
- None.

## Decisions log
- ADRs 0001–0005. CP1: station cap 6, configurable; waits recorded for all movable robots, events for working robots only.
