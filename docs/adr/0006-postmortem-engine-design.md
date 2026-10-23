# 0006. Post-mortem engine: detection, attribution and cost controls

**Status:** accepted · **Date:** 2026-10-20 · **Refines:** ADR 0005 · **Research:** [post-mortem engine](../research/2026-10-20-postmortem-engine.md)

## Context
ADR 0005 chose root-cause analysis by counterfactual replay. CP2's review showed that loss measured at a run-dependent recovery time is unstable, and that direct blocking chains miss most of a fault's damage, which spreads as congestion. CP3 needs concrete, defensible choices for detection, the loss measure, attribution and cost.

## Decision
1. **Detection:**
   - one-sided CUSUM on the per-minute delivery deficit (twin − actual), scaled by the twin's Poisson noise, not by pre-onset spread, which is exactly 0 because the twin shares the seed;
   - start with k = 0.5σ and h = 5σ, and calibrate h on fault-free runs with different seeds;
   - the deficit is not clipped, because backlog drain makes it negative.
2. **Incident window:**
   - it opens at the last zero of the CUSUM before the alarm and closes after 5 minutes at 0;
   - **loss is measured at a fixed horizon** (window end + 10 min) in every replay, as parcel-minutes of delay, so all replays compare like with like.
3. **Candidates:** all faults active in or before the window, plus labelled capacity hotspots from blame. Hotspots answer a different counterfactual ("relax capacity" rather than "remove fault") and are reported separately. Cap at n ≤ 6.
4. **Attribution:**
   - run all 2ⁿ masks (≤ 63 replays);
   - compute exact Shapley values and pairwise interactions via the Möbius transform (Harsanyi dividends), in `i128` so shares sum exactly to the explained loss;
   - report negative contributions honestly, and show unexplained loss as its own line;
   - find the minimal sufficient set by a lattice scan of the same replays (ddmin only beyond n = 6).
5. **"What would have helped":** candidate fixes (remap a destination, add robots, change a station cap, reroute) are `Intervention`s applied to a checkpoint and scored by parcel-minutes recovered.
6. **Noise: dithered replays.** Common random numbers are already exact; residual noise comes from planner trajectories diverging after any change to a robot's moves. It is small on a calm floor (±32 parcel-min net) but large under load (sd 4–6 % for jams and failures, up to 27 % for aisle blocks). So each post-mortem runs the actual history and every intervention under about 8 causally irrelevant 1-tick perturbations ("dither"). It reports the **median ρ with its spread**, and treats two candidates as a **tie** when their ρ differ by less than twice the dither band. When results are pooled across seeds, it stratifies by mode (e.g. whether a failed robot landed on an induction cell) instead of adding seeds blindly. Evidence: `docs/checkpoints/v1-cp2.md` (investigation).
7. **Cost:**
   - checkpoints of the actual run every 5 simulated minutes; each replay resumes from the latest checkpoint before the earliest fault it removes and stops at the horizon;
   - rayon over masks, collected in mask order, so output is deterministic;
   - a world-only hash (excluding the fault schedule) for optional early stopping.
8. **Where it runs:** full post-mortems are native (CLI). The browser shows precomputed reports or runs small cases (n ≤ 4) in a worker, because multi-threaded WASM needs headers static hosting can't set.

## Alternatives considered
- **Leave-one-out only:** misattributes interacting faults (overdetermination).
- **Sampling Shapley:** unnecessary at n ≤ 6, and it would add noise.
- **Loss at the recovery time:** run-dependent truncation, which was CP2 finding 6.

## Consequences
- Every number in a report is traceable to a specific replay or mask.
- The engine depends on CP3 core work: `Sim: Clone`, `disable_fault` on a running sim, a world hash and incremental distance rebuilds.
