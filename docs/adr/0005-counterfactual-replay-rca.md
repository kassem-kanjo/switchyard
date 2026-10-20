# 0005. Root-cause analysis by counterfactual replay

**Status:** accepted · **Date:** 2026-10-19 · **Research:** [reliability methods](../research/2026-10-19-reliability-methods.md)

## Context
Production root-cause tools estimate "what if this cause were absent" from observational data (correlation, learned causal graphs). A 2024 benchmark found many graph-based methods perform close to random. Our twin is deterministic, so we can *intervene* instead of estimating.

## Decision
1. **Detect:** a one-sided CUSUM on the throughput deficit against the fault-free twin raises an incident window.
2. **Shortlist:** take faults active in or before the window, plus the top root-blame hotspots from traces (robust z-score of blame time, BARO-style), capped at 6 candidates.
3. **Intervene:** for each candidate, re-run the identical scenario and seed with that cause removed (a fault) or relaxed (hotspot capacity). Measure the recovered share of lost parcels.
4. **Attribute:** exact Shapley values over the candidate set (≤ 2⁶ = 64 replays, run in parallel) split the blame between interacting causes. Shares sum to the total explained loss, and anything left is reported as "unexplained / intrinsic congestion".
5. **Suggest fixes:** score candidate fixes (reroute around an aisle, remap a destination, add capacity) the same way.
6. **Report:** render a Google-SRE-format post-mortem in which every number cites the replay that produced it.

## Alternatives considered
- **Correlation or graph-based RCA:** no ground truth, and it can't separate interacting causes.
- **Leave-one-out only:** it misattributes when causes interact (two faults that each alone cause no loss); Shapley values fix that.

## Consequences
- Cost: up to 64 extra simulations per incident, which is fine natively (rayon) and limited in the browser.
- "Planted incident" tests become possible: inject a known cause and assert it ranks first. That's our causal-correctness metric.
