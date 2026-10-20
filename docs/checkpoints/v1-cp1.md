# Checkpoint v1-cp1: a floor that sorts

**Date:** 2026-10-20 · **PR:** [#3](https://github.com/kassem-kanjo/switchyard/pull/3) · **Lead:** Engagement Lead, Switchyard

## Planned increment
Guided PIBT with causal wait records; realistic floor operations (Poisson arrivals, Zipf destinations, chute bags and swaps, scanner misreads); calibrated presets (plan rows 1.1–1.3).

## Delivered
| Preset | Robots | Parcels/h | Per robot/h | Simulated 2 h in (release) |
|---|---|---|---|---|
| small | 40 | 1,546 | 38.7 | — |
| medium | 200 | 6,290 | 31.4 | 0.39 s |
| large | 500 | 15,806 | 31.6 | ~1 s |

Industry band: 25–50 parcels/h per robot ([industry research](../research/2026-10-19-industry-and-client.md)).

## Gates
| Gate | Result | Evidence | Blocking findings |
|---|---|---|---|
| Visual | **Pass** | `assets/v1-cp1-tick28.jpg`, `-tick588`, `-tick3232`, `-tick72416`; no overlaps, whole-floor flow, stable parking, no console errors | None (follow-ups: preset selector, legend, contrast) |
| Code review | **Pass with fixes** | Independent Opus review of `5fff6d4..defebb3` | 5 important findings (below) |
| Tests | **Pass** | 54 tests in debug and release; presets reproducible (medium hash `e92f5a27d38e23b5` on two reruns); viewer 7 tests, typecheck | None |

## Review findings and resolution
| # | Finding | Why it matters | Resolution |
|---|---|---|---|
| 1 | `StationBusy` refinement hid the real blocker when the robot on the station had already finished loading | Wrong root causes in traces | Refine to `StationBusy` only while the blocker is `Loading`; tests `station_busy_only_while_the_blocker_is_loading`, `station_busy_always_names_a_loading_robot` |
| 2 | Wait-for chains cut at idle and parking robots | CP2 can't walk chains to their root | Every movable robot that makes no progress records a cause; `wait_for()` keeps all edges; terminal roots documented in ARCHITECTURE §4; tests `wait_for_chains_are_walkable`, `pushed_robot_that_cannot_move_records_its_own_blocker` |
| 3 | Scanner and destination draws coupled to unrelated events | Counterfactual replays (ADR 0005) would be confounded | Counter-keyed draws: destination and scan outcomes depend only on `(station, parcel ordinal)`; test `counterfactual_changes_leave_other_parcels_untouched` |
| 4 | Robots waiting at a swapping chute were immovable; throughput collapsed under overload (9,765 → 6,050 /h) | Chaos results would measure an artefact | Chute waiters are movable (lowest priority). The reported collapse did **not** reproduce on default settings (throughput keeps rising past capacity: 10,131 → 10,773 /h); the artefact is real under heavy bag swaps (8,129 → 10,468 /h after the fix); tests `overload_does_not_collapse_throughput`, `robots_waiting_at_swapping_chutes_do_not_wall_off_the_aisles` |
| 5 | Watchdog detected only whole-floor stalls | Local deadlocks unreported | Per-robot `RobotStalled` once per episode; config rejects `bag_swap_ticks >= watchdog_ticks`; tests `each_stuck_robot_reports_one_stall_per_episode`, `rejects_bag_swaps_the_watchdog_would_call_a_stall` |

## Re-verification after fixes (lead)
fmt, clippy `-D warnings`, **66 tests**, wasm32 build, viewer (13 tests, build): all exit 0. Medium: 6,285.5 /h (31.4 per robot), hash `747248e3e7c20e9b`, identical on two runs. Minor review items were also addressed (BlockedBy(parent), hedged guarantee, `set_arrival_rate` errors plus rates in the hash, configurable station cap, `sim.rs` split).

Viewer follow-ups from visual QA: preset selector (`?preset=`), legend, ≥ 3:1 contrast test, active/parked counts. See `assets/v1-cp1-medium.jpg`.

## Decisions
- Station inbound cap: 6 (Little's-law argument accepted by review), moved into config as a tunable.
- Architecture §4 amended: events are emitted for working robots only, but the full wait-for edge set is kept.

## Carried forward
- Plan row 1.4: criterion benchmark (performance is already 350× the target, so priority is low).
- Per-robot deadlock and cycle detection feeds CP2.1 causality.
