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
