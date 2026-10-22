# Checkpoint v1-cp2: tracing and chaos

**Date:** 2026-10-20 · **PR:** [#4](https://github.com/kassem-kanjo/switchyard/pull/4) · **Lead:** Engagement Lead, Switchyard

## Planned increment
Causal root chains and deadlock detection (2.1); per-parcel traces and blame (2.2); Perfetto/OTLP/JSONL exporters (2.3); fault catalog, fault-free twin and resilience score (2.4).

## Delivered (first pass, `b34f4f9`)
- Root walk over the wait-for graph each tick (O(robots)); persistent-cycle deadlock detection.
- Streaming trace builder (spans tile each parcel's lifetime, property-tested); blame index.
- Exporters plus `switchyard export`; six fault kinds plus `switchyard chaos` (rayon over seeds and twins: 10 sims of 2 h on medium in 0.85 s).
- No-fault behaviour unchanged (medium hash `747248e3e7c20e9b`).

## Gates (first pass)
| Gate | Result | Evidence | Blocking findings |
|---|---|---|---|
| Visual | **Partial** | Exports structurally valid (no nesting violations or lane overlaps; flows point at the correct blocker; 1 tick = 500,000 µs). The Perfetto UI could not load local files from the in-app browser, so the rendering is not yet seen | Phase colours, lane packing in filtered exports, no fault annotations |
| Code review | **Fail** | Independent Opus review of `0e6eb29..b34f4f9` with experiments on the chaos presets | 2 critical, 6 important (below) |
| Tests | **Pass** | All tests in debug and release; hash stable; chaos output byte-identical on rerun; all export formats | None |

## Review findings
| # | Sev. | Finding | Why it matters | Resolution |
|---|---|---|---|---|
| 1 | Critical | A robot failing on a station cuts the station off; assignment kept sending robots; the delay sat in non-`Blocked` phases → **0 root blame for the failures** in the headline preset | The product's first claim fails on its own demo | Assignment skips unreachable stations and releases claims; new `ParcelDelayed` event attributes fault-caused delay in every phase; trace builder splits phases by cause. Test `chaos_presets_blame_every_fault_and_the_right_one`: F0/F1/F2 = 53,383 / 311,249 / 340,696 parcel-ticks (matched exactly by the trace crate) |
| 2 | Critical | A parcel on a robot that failed while carrying got no cause (800 of 827 blocked ticks unattributed) | Same | Failed robots holding a parcel emit `RobotWaited{Fault(f)}` every tick; zero unattributed blocked ticks on fault presets |
| 3 | Important | Cut-off robots blamed on the lowest active fault, not the one that cut them off | Wrong rankings and Shapley values in CP3 | Per-robot attribution to the fault whose removal reconnects it (cached single-goal Dijkstra); checked independently in 647,145 cases |
| 4 | Important | Fault IDs shift under `without_fault` | Runs and replays can't be joined | Stable ids: `disabled_faults`, `without_fault(FaultId)` disables in place |
| 5 | Important | Jams and outages not linked to their faults | Jam indistinguishable from a bag swap | `WaitCause::Fault`; jams and outages root to their fault; a cycle is a root only once persistent |
| 6 | Important | "Lost parcels" was really backlog at a run-dependent recovery time (went negative once) | Misleading to a client; unstable for CP3 ρ | Replaced by parcel-minutes of delay (area between cumulative-delivery curves, t1-independent), peak backlog, late vs SLA, undelivered |
| 7 | Important | Degradation ratio misleading for surges and aisle blocks | Misleading to a client | served/offered for surges; throughput/capacity uses reachable capacity, including stations and chutes lost to failed robots |
| 8 | Important | OTLP trace IDs collide across runs and seeds | Exports merge in one collector | Run id mixed into OTLP ids; unique attribute keys; Perfetto colours, fault tracks, packed lanes |

## Insight for the product story
Even with perfect direct attribution, most of a fault's damage spreads as **congestion** (yields, busy stations) far from the fault itself. Blocking chains can't see that; **counterfactual replay can**. This is the strongest argument for CP3's design (ADR 0005), and it belongs in the README and the CEO review.

## Investigation: outliers and replay noise (2026-10-21)
- **The seed-1 outlier is a genuine mechanism, not a bug.** When a robot fails *while on a station's induction cell*, that station becomes an unannounced outage: no robot can reach it, and its queue grows by about 14 parcels/min (to 269 in 20 min). Across 30 seeds, the number of robots that died on induction cells predicts the outcome:

  | Robots dead on induction | Delay | Late parcels |
  |---|---|---|
  | 2 | 13,327 parcel-min | 632 |
  | 1 | 10,496 | 452 |
  | 0 | 2,438–6,805 | 39–83 |

  Client message: *a robot dying on a loading point costs about ten times more than one dying in an aisle.* A candidate "what would have helped" intervention for CP3: clear the robot off the induction cell, or divert that station's queue.
- **Replay noise:**
  - Common random numbers are exact: causally irrelevant perturbations change nothing on 10 of 10 seeds.
  - Once any robot move changes, about 4,400 parcels reshuffle each way. On a calm floor the net delay stays within ±32 parcel-min.
  - Under load the spread is 4–6 % (jams, failures, surges), and up to 27 % for aisle blocks.
  - This drove ADR 0006 §6: dithered replays, a median with its spread, and ties within the noise band.
- **Metric fix:** throughput/capacity now counts stations and chutes lost to failed robots (previously seed 1 showed a misleading 0.77).

## Re-verification after fixes (lead, 2026-10-21)
fmt, clippy `-D warnings`, **186 Rust tests** (2 ignored measurement tests), 45 viewer tests, wasm32 and viewer builds: all exit 0. No-fault medium hash unchanged (`747248e3e7c20e9b`). Cross-platform determinism: native and WASM hashes match on all 4 reference scenarios (`scripts/determinism/`).

## Also landed on this branch (CP3 groundwork, reviewed with CP2)
- `Sim: Clone` checkpoints (medium ~274 KiB), `disable_fault` on a running sim, world-only `live_hash`, interventions (`DisableFault`, `RemapDestination`, `AddRobots`, `SetStationCap`).
- Incremental distance repair: fault start on large 160 ms → 4.8 ms.
- 64 one-hour replays on medium: 1.8 s on 16 threads. Exact re-convergence observed 0/8, so replays run to a fixed horizon (ADR 0006).
- Viewer wired to the real sim: live twin, fault injection, real traces. CI determinism jobs (Linux, Windows, macOS, WASM).

## Re-review (double review) and final fixes (2026-10-21)
| Gate | Result |
|---|---|
| Code: independent Opus reviewer | **With fixes.** 7/8 original findings verified fixed by reproducing them on `b34f4f9` and showing them gone; #8 was fixed in the library but not wired into the CLI. Its own stress test of the incremental distance repair: 10,800 checks, 0 mismatches |
| Code: `/code-review high` | 7 findings (spawn cells, partial injection, twin errors, store eviction, per-frame hash, Markdown tables, silent AddRobots shortfall) |
| Tests | 200 Rust + 48 viewer tests; determinism native and WASM match on all reference scenarios |

Fixes landed:
- **Replays reproducible from their config.** An active-fault disable is recorded as a truncation, and the surge stream key no longer depends on duration. Pinned replay-path hash `053601a2fbdb2cc8`.
- **The jam gets full blame:** 53k → 90k parcel-ticks.
- **Failed robots are cleared off induction cells after 5 min** (`[recovery] clear_after_ticks`), per the reviewer's ruling. A repaired loader drives back and reloads. Dead-robot blame falls to a realistic 24k / 22k.
- AddRobots respects closed cells and reports any shortfall (`RobotsAdded`); remaps don't create false mis-sorts; live-hash soundness test.
- **The CLI export now uses run ids**; fault injection is all-or-nothing; twin errors are surfaced; the trace store is capped by spans; the per-frame hash is removed; Markdown tables.

**Verdict:** all blocking findings resolved; CP2 passes.
