# Switchyard: architecture

**Status:** V1 design · **Last updated:** 2026-10-19 · Decisions are in [`adr/`](adr/).

## 1. Shape of the system

```
                  scenarios/*.toml
                        │
                        ▼
┌───────────────────────────────────────────────┐
│ switchyard-core   (pure, deterministic, no I/O)│
│  layout ─ world ─ assign ─ planner (PIBT)      │
│  faults ─ causality ─ sim::step ─ hash         │
│          │ emits                               │
│          ▼                                     │
│     Event stream (the core's only output)      │
└──────────┬────────────────────────────────────┘
           │
   ┌───────┴────────────┐
   ▼                    ▼
switchyard-trace    switchyard-analysis
spans, blame,       metrics series, CUSUM,
exporters           resilience, counterfactual
(Perfetto, OTLP)    RCA, Shapley, post-mortem
   │                    │
   └────┬───────────────┘
        ▼
┌──────────────┐      ┌───────────────────────────────┐
│ switchyard   │      │ switchyard-wasm → viewer/ (TS) │
│ CLI (rayon   │      │ Web Worker runs the sim;       │
│ fan-out)     │      │ Renderer interface draws it    │
└──────────────┘      └───────────────────────────────┘
```

## 2. Components

| Crate / dir | Responsibility | Depends on |
|---|---|---|
| `crates/core` (`switchyard-core`) | Floor model, layout generation, robots, parcels, chutes, task assignment, guided-PIBT planner, fault schedule, causal wait attribution, tick loop, state hash. **No I/O, no threads, no wall-clock, no floats in sim state.** | `rand_chacha`, `serde` |
| `crates/trace` (`switchyard-trace`) | Builds per-parcel traces from events; blame aggregation; exporters: Chrome trace JSON (Perfetto), OTLP JSON, JSONL | core |
| `crates/analysis` (`switchyard-analysis`) | Metric time series, CUSUM incident detection, resilience scoring against a twin, counterfactual root-cause analysis, Shapley attribution, post-mortem rendering (Markdown) | core, trace |
| `crates/cli` (`switchyard` binary) | `run`, `chaos`, `postmortem`, `export`; parallel replays with rayon | all crates |
| `crates/wasm` (`switchyard-wasm`) | wasm-bindgen facade: create sim, step, flat snapshot buffers, fault injection, trace and metrics queries | core, trace, analysis |
| `viewer/` | TypeScript + Vite web app. The sim runs in a Web Worker; the UI draws through a `Renderer` interface (Canvas 2D in V1) | wasm package |
| `scenarios/` | TOML scenario presets (`small`, `medium`, `large`), calibrated to industry ranges | — |

## 3. Domain model (core)

**Time and space.** Discrete ticks; **1 tick = 0.5 s**. The floor is a 4-connected grid of 1 m cells. A robot moves at most one cell per tick (2 m/s). There is no rotation in V1.

**Cells.** `Free`, `Chute(id)` (a hole robots drop into from an adjacent cell; not traversable), `Induction(id)` (a loading cell on the floor edge), `Wall`.

**Guidance.** Each directed edge between free cells has an integer cost. The default layout alternates one-way-preferred lanes (preferred direction cost 1, against-flow cost `k`, default 3), like Manhattan streets. Distances to every goal are precomputed with Dijkstra on this weighted graph; PIBT ranks candidate moves by them. See ADR 0003.

**Entities** are stored as struct-of-arrays and indexed by dense ids:
- `Robot`: position, state (`Idle | ToInduction | Loading | ToChute | Dropping | Failed`), carried parcel, current goal.
- `Parcel`: id, destination, arrival tick, station, assigned robot, chute, timestamps per phase.
- `Chute`: id, cell, destination(s), bag fill and capacity, bag-swap timer.
- `Station`: id, cell, queue of waiting parcels, loading timer.

**Parcel lifecycle (phases):**
```
Queued ─▶ AwaitingRobot ─▶ Loading ─▶ Travelling ⇄ Blocked ─▶ ChuteWait ─▶ Dropped
```
These phases tile the parcel's lifetime exactly. The trace builder enforces this invariant and a property test checks it.

**Tick order** (fixed, documented in `sim.rs`):
1. Apply faults that start or end at this tick.
2. Generate arrivals.
3. Advance station and chute timers.
4. Assign idle robots.
5. Plan moves (PIBT).
6. Apply moves.
7. Resolve loads and drops.
8. Attribute causes of waits.
9. Emit events.
10. Update the state hash.

## 4. Causality (what makes the traces explainable)

When PIBT leaves a robot short of progress, the planner records **why**:
- `BlockedBy(robot)`: another robot holds the next cell.
- `Yielded(robot)`: it lost a priority conflict.
- `CellBlocked(fault)`: an aisle block or a dead robot is in the way.
- `ChuteUnavailable(chute)`: the chute is full, swapping bags or jammed.
- `StationBusy(station)`

Each tick, these edges form a **wait-for graph**. `causality.rs` walks each chain to its **root** (a fault, an unavailable chute, a busy station, or a cycle) and emits `RobotWaited { robot, direct, root }`. Blocked time on a parcel's trace is charged to both. This is the robot-floor equivalent of lock-wait attribution in databases. See [reliability research §a](research/2026-10-19-reliability-methods.md).

## 5. Events: the core's only output

`sim.step()` appends `Event`s to a caller-provided buffer. Everything downstream (traces, metrics, viewer, post-mortems) is a pure function of `(scenario, seed, fault schedule)` → event stream. Consequences:
- **Replays are exact.** A counterfactual is a re-run with a modified fault schedule (ADR 0005).
- **Consumers are decoupled.** New analyses never touch the sim.

## 6. Determinism contract (ADR 0002)

- **Integer state only.** No `f32`/`f64` inside the sim (analysis may use floats on outputs).
- **No `HashMap` or `HashSet` in core** (enforced by clippy `disallowed-types`). Use `Vec`, `BTreeMap` or dense ids.
- **RNG:** `ChaCha8Rng` streams derived from `(seed, stream_id)`, one per concern (arrivals, destinations, per-station). Removing a fault therefore never shifts unrelated random draws.
- **Single-threaded sim.** Parallelism only *across* sims.
- **State hash** (FNV-1a over a canonical encoding) is emitted per run. CI compares hashes across Linux, Windows and macOS, and native vs WASM.

## 7. Analysis pipeline

1. **Metrics:** per-minute throughput, backlog, robot utilisation and chute occupancy, derived from events.
2. **Twin:** every chaos run is paired with a fault-free run on the same seed.
3. **Resilience score:**
   - lost parcels (the area between the twin's and the faulted run's throughput)
   - maximum dip
   - time to recover (back within 5 % of the twin for 5 consecutive minutes)
   - graceful-degradation ratio (throughput retained divided by capacity retained)
   - all reported next to baseline utilisation
4. **Detection:** one-sided CUSUM on the throughput deficit versus the twin's baseline.
5. **Candidates:** active faults plus the top root-blame hotspots (for example, a chute that is often full).
6. **Counterfactual ranking:** for each candidate, re-run with that cause removed or relaxed; recovered share = (lost − lost′) / lost.
7. **Interactions:** exact Shapley values over ≤ 6 candidates (≤ 64 replays, run in parallel with rayon).
8. **Report:** the Google-SRE post-mortem template (summary, impact, timeline, root causes with replay evidence, what would have helped, action items).

## 8. Presentation (swappable by design)

The WASM facade exposes **flat typed-array snapshots** (robot x, y, state; chute fill) plus JSON queries (trace, metrics, report). The viewer depends only on a small TypeScript `Renderer` interface (`init(layout)`, `draw(snapshot)`, `highlight(ids)`). V1 ships a Canvas 2D renderer; PixiJS, WebGL or a native Bevy app can replace it later without touching the core. Colours and sizes live in one theme object.

## 9. Toolchain and CI

- Rust stable. On the Windows dev box: the `x86_64-pc-windows-gnu` toolchain (no MSVC installed). Avoid crates that need a C compiler.
- Node 22 LTS for the viewer.
- GitHub Actions:
  - `fmt` and `clippy -D warnings`
  - tests on Ubuntu, Windows and macOS
  - a `wasm32` build
  - the determinism-hash comparison job
  - viewer type-check and build
- Testing: unit tests; `proptest` invariants (no collisions, spans tile lifetimes, conservation of parcels); `insta` snapshots for reports; `criterion` benchmarks for the tick loop.
