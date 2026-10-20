# 0002. Determinism contract

**Status:** accepted · **Date:** 2026-10-19

## Context
The product's central claim is that root causes are *proven* by replay. That only holds if a run is a pure function of `(scenario, seed, fault schedule)`, bit-identical across operating systems and between native and WASM.

## Decision
1. Sim state is integers only: positions, ticks, counters and fixed-point where needed. Floats are allowed only in analysis outputs.
2. No `std::collections::HashMap` or `HashSet` in `core`, enforced with clippy `disallowed-types`. Use `Vec` with dense ids, or `BTreeMap`.
3. Randomness comes from `rand_chacha::ChaCha8Rng` streams seeded by `hash(seed, stream_id)`, one stream per concern. Faults are explicit schedule entries, never random draws from shared streams.
4. The sim is single-threaded, with no wall-clock reads and no iteration over unordered containers. Tie-breaks are by id.
5. A canonical FNV-1a state hash is produced at the end of each run (and optionally every N ticks).
6. CI compares the hash of reference scenarios across Linux, Windows, macOS and `wasm32`.

## Alternatives considered
- **Floats with careful ordering:** cross-platform float drift is a well-known risk (fused multiply-add and libm differences), and it isn't worth it for a grid sim.
- **One global RNG stream:** removing a fault would shift every later draw and confound counterfactuals.

## Consequences
- Counterfactual replays differ from the original *only* through causal effects of the change. Downstream divergence (different assignments after the fault is removed) is a real consequence of the fault, not noise.
- The rules need some discipline, enforced by clippy, tests and the CI hash job.
