# Checkpoint v1-cp3: replay-proven post-mortems

**Date:** 2026-10-22 · **PR:** #5 · **Issues:** #6 (gates), audit findings #13–#17

## Delivered
The post-mortem engine (ADR 0006): CUSUM detection; dithered checkpoint replays; exact Shapley attribution (Möbius dividends, i128); "what would have helped" fixes; SRE-format report; `switchyard postmortem`. 24/24 planted incidents (medium); 96 replays in 2.1–2.3 s.

## Gate: independent Auditor (Opus), with fixes
- **Verified sound:**
  - Shapley maths (efficiency, symmetry, dummy, null and additivity, all property-tested);
  - replay fidelity, with 12/12 masks bit-identical to fresh runs;
  - no false alarms (0/12);
  - both performance claims reproduce.
- **To fix before merge:**
  - interaction labels are inverted (critical, #13);
  - dither no-ops make the noise band too narrow (#14);
  - later incidents are dropped (#15);
  - citations and wording overclaim (#16).
- **Can follow in CP3.1:** noise-aware thresholds (#16) and the ADR 0006 amendment for h = 3 (#17).

## Still to run
`/code-review high`, the visual check of the rendered report, CI.

## Harder planted incidents (issue #8)
`planted.rs` plants one dominant cause among small decoys; it hit 24/24 because the planted cause dominates. `crates/analysis/tests/planted_hard.rs` (`#[ignore]`, release, about 2 min) adds medium-floor cases where ranking and tie-marking can fail. Run: `cargo test -p switchyard-analysis --release --test planted_hard -- --ignored --nocapture`.

**Classes and scoring** (the report's own tie rule: Shapley values within twice the noise band):
- **Near-tied pair**: two induction outages of different stations, A 17-20 min against B 16 min. Correct if A ranks first or the pair is marked a tie.
- **Duplicate outage**: two identical outages of one station (exactly symmetric). Correct only if marked a tie AND the interaction is redundant.
- **Compound pair**: fleet-wide surge (200 %) plus a jam of the busiest chute. Correct if the interaction is compound.

**Result, final suite: 48/48 (16 seeds per class, 8 dither histories, 136 s).** Near-tied A first 16/16 (never needed the tie rule; the gap was 20-100 % of A even at 6 % longer duration), duplicates tied and redundant 16/16 (interaction sign positive), compound 16/16.

**Honest account of how the suite got here.** The first design scored **20/32 (62.5 %)**: near-tied 8/8, compound 8/8, but "exact tie" 4/8 and "redundant" 0/8. Both failures were defects in my planted truth, not engine findings, so I replaced the classes (and the engine is untouched):
- "Exact tie" used outages of two different stations at equal duration. Geometry makes them genuinely unequal (3930 against 4690 parcel-min on one seed), so the engine was right not to call a tie.
- "Redundant" was an outage plus a surge at the same station. The engine said compound on 8/8 seeds; the surge's backlog adds load the outage alone does not, so my hypothesis was wrong.

**What this does and does not show.** The engine ranked correctly and signed interactions correctly on every case, but the replacements are easier than the originals, so 100 % is not evidence of robustness. Tie-marking was only exercised on perfectly symmetric duplicates (trivially tied). No case produced a real near-tie the engine had to hedge on: even a 6 % longer outage separated clearly from a 16 min outage on another station. A case where truth is a genuine, unequal-but-close tie needs an independent ground truth (for example each fault replayed alone against the twin) and is still open.
