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

## Amendment (2026-10-22): CUSUM threshold h
**Decision 1 said "start with h = 5σ and calibrate"; the engine ships h = 3σ (k = 0.5σ).** The justification lives in `crates/analysis/src/postmortem/detect.rs` ("Calibration of `h`"); this section records it where decisions are tracked, with the long-shift check the CP3 audit asked for (issue #17).

**Why 3, not 5.**
- The relevant null is a run and its twin sharing every arrival, with only robot trajectories perturbed (1-tick robot failures on the same seed). Its largest CUSUM peak over 160 simulated hours (40 pairs × 2 h on the small and medium floors) was 1.0. The different-seed null (peaks up to 15.5) carries arrival noise a run and its twin never have, so it is the wrong yardstick.
- h = 3 is three times that largest null peak. h = 5 missed a small-floor surge costing about 700 parcel-minutes (peak 3.1).
- Trade-off: a lower h detects smaller incidents sooner but raises the false-alarm rate; a higher h does the reverse. With a null peak of 1.0 there is headroom either way, so detection delay and sensitivity decided it. The window start is Page's change-point estimate, so a lower h does not move the start later.

**Calibration on long shifts (new).** `calibrate_h_on_long_shifts` (`crates/analysis/tests/calibration.rs`, `#[ignore]`, release) repeats the perturbed-path null on the medium floor for 8 h and 10 h shifts, 6 seeds each, with a 1-tick robot failure every 30 simulated minutes (16 and 20 perturbations). Run: `cargo test -p switchyard-analysis --release --test calibration -- --ignored --nocapture` (about 30 s).

| Shift | Seeds | Peak median | Peak max | Alarms at h=2 | h=3 | h=4 |
|---|---|---|---|---|---|---|
| 8 h | 6 | 1.00 | 1.23 | 0 | 0 | 0 |
| 10 h | 6 | 1.00 | 1.23 | 0 | 0 | 0 |

Reading: no false alarms at h = 2, 3 or 4 on either length, and the peak did not grow with shift length (the 10 h peaks equal the 8 h ones, so the maximum is set early, not by accumulation: the one-sided recursion resets to 0 whenever the deficit goes quiet). The largest peak, 1.23, leaves h = 3 a factor of 2.4 of headroom. **h = 3 stands.** Limits: 6 seeds per length and one floor, so it shows headroom rather than bounding the false-alarm rate tightly (0 alarms in 6 is still compatible with a rate near 40 %); a different floor or a much noisier fleet should repeat the run.

**Report wording.** The report no longer hard-codes "three times": it prints `h / 1.0` (the largest null peak) from the live parameter, so changing `h` cannot leave a stale sentence.
