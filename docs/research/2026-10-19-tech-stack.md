# Switchyard - Tech Stack Research (2026-10-19)

Status: research input, not a decision record. Versions were read from the crates.io API on 2026-10-19.
"Unverified" marks claims I could not confirm from a source; verify those with a spike before committing.

## 1. Recommendation (read this first)

**Hybrid of B + C, with Rust as the single source of truth. Not Bevy for V1.**

- **Core (`switchyard-core`)**: pure Rust library, no I/O, no threads inside a single sim. One sim = one deterministic
  single-threaded state machine (fixed tick, integer/fixed-step arithmetic, seeded PCG/ChaCha RNG, `BTreeMap`/`IndexMap`/Vec only).
  Parallelism is *across* sims (counterfactual replays) using rayon - which keeps each run bit-reproducible.
- **Native CLI (`switchyard-cli`)**: batch runs, RCA counterfactual fan-out (rayon), trace export (Parquet + Chrome-trace JSON + OTLP JSON).
- **Web viewer (`switchyard-wasm` + TypeScript/Vite + PixiJS v8 or raw Canvas2D/WebGL)**: core compiled to WASM via wasm-bindgen, runs
  in a Web Worker, posts typed-array snapshots to the renderer. Deployed on GitHub Pages = the "10-second recruiter" demo + video.
  Renderer sits behind a small `Renderer` interface fed by a flat per-frame snapshot buffer (swappable: Canvas -> PixiJS -> three.js/deck.gl/Bevy later).
- **Analysis**: DuckDB (CLI + Python) over Parquet traces for RCA queries; Python/Jupyter notebook (Polars + DuckDB, optional PyO3 later) for
  the report. Keep Python as a *consumer of files*, not a hard dependency of the core, so "download and run" stays one `cargo run`.
- **Trace export**: Parquet is canonical (columnar, millions of spans). Derived exports: Chrome trace-event JSON (opens in Perfetto, zero setup)
  and OTLP (to Jaeger via collector endpoint). Don't force OTel's data model into the store; map at the edge.

Why: A web demo is the single highest-leverage thing for the portfolio goal; Bevy WASM bundles are large, load slowly, and gate WebGPU/WebGL2
quirks. A native Bevy viewer can be added later as a second consumer of the same snapshot stream (visual swappability by design).

## 2. Comparison table

Scores 1-5 (5 best). My judgment, not measured data.

| Criterion | A: Rust + native Bevy (+Bevy WASM) | B: Rust->WASM + TS web UI + native CLI | C: Rust + Python (PyO3/Polars/Jupyter) | Recommended (B+C) |
|---|---|---|---|---|
| Dev speed (solo) | 3 (Bevy churns each ~3-4 months; 0.20 released 2026-10-08) | 4 (Vite HMR, mature UI libs) | 4 for analysis, 2 for visuals | 4 |
| Visual swappability | 3 (ECS renderer tied to Bevy) | 5 (snapshot buffer -> any renderer) | 1 (matplotlib only) | 5 |
| Demo-ability (10 s) | 3 (WASM build heavy, long first load) | 5 (static site, link in README) | 2 (needs Python env) | 5 |
| Raw performance | 5 | 4 (WASM ~1.2-2x slower, single thread unless SharedArrayBuffer headers) | 5 core / 3 glue | 5 native batch, 4 web |
| Testability | 4 | 5 (core is pure lib; Playwright for UI) | 4 | 5 |
| Determinism risk | Medium (Bevy schedule/parallel systems must be pinned) | Low (sim isolated from UI) | Low | Low |
| Impressive to logistics/big-tech | 4 (Rust+ECS) | 4 (full-stack, shows web+systems) | 4 (data/ML-adjacent) | 5 (breadth: sim, observability, data, web) |
| Windows+macOS | Good (wgpu) | Excellent (browser) | Fair (Python envs, wheels) | Excellent |

Note: do not use Bevy ECS *inside* the sim core. Keep a plain struct-of-arrays world; Bevy would add schedule non-determinism and a heavy dependency.

## 3. Sourced findings

### 3.1 Language: Rust vs C++ vs others
- MAPF research code is overwhelmingly **C++**: PIBT/PIBT2 and LaCAM by Okumura et al. (project page kei18.github.io/pibt2 per the PIBT paper);
  the League of Robot Runners (Amazon Robotics-sponsored lifelong-MAPF competition, 1 s planning limit per step, large agent counts and maps)
  starter kit and winners are C++ as well. Python is used for benchmarks/plots. (arxiv.org/pdf/1901.11282, arxiv.org/html/2404.16162v1.)
  I did not verify the language of the competition starter kit from a fetched page; treat as widely-known, not sourced.
- Implication: algorithms to implement in Rust: PIBT (priority inheritance, lifelong-capable, scales to 1000+ agents), LaCAM/LaCAM*, windowed
  MAPF (WHCA*/rolling horizon). Port from the C++ reference for correctness cross-checking.
- Rust beats C++ here on: cargo workflow on Windows (no CMake/vcpkg pain), rayon, first-class WASM target (wasm32-unknown-unknown + wasm-bindgen),
  `Send/Sync` safety for the parallel counterfactual runner, proptest/insta ecosystem. C++ wins only on "matches the research code".
  Zig/Go/C#: weaker WASM or parallel-determinism story; not recommended.
- **Determinism rules (Rust)**: std `HashMap/HashSet` iteration order is randomized per run (reproducible-builds.org/docs/stable-outputs).
  Large Rust codebases ban them for this reason and use `BTreeMap/BTreeSet` (Materialize `mz_ore`); a simulator audit (scx_simulator) fixed
  nondeterminism by sorting keys / moving to BTreeMap. Enforce with clippy `disallowed_types` (HashMap, HashSet) in `clippy.toml`.
  Also: no `f32/f64` reductions across threads inside one sim (float add is non-associative; parallel reduce order varies) - use integer
  ticks/fixed-point or keep each sim single-threaded; derive per-agent RNG streams from `(seed, agent_id)` rather than sharing one RNG;
  no wall-clock, no `Instant`, no pointer-address ordering; pin `rand` algorithm explicitly (`rand_chacha::ChaCha8Rng` / `rand_pcg::Pcg64Mcg`)
  because `StdRng` is not guaranteed stable across versions. Cross-platform float drift (Windows vs macOS, WASM vs native): avoid libm-dependent
  transcendental functions in the sim path; golden-hash the state each N ticks in CI on both OSes.

### 3.2 Trace formats
- **Chrome trace-event JSON -> Perfetto: confirmed.** Perfetto's docs list Chrome JSON as directly openable in ui.perfetto.dev. Format = JSON array of
  events with `pid, tid, ts (microseconds), ph, name, cat, args`. Supported: `B/E` and `X` slices, `I` instants, `C` counters, `M` metadata
  (process/thread names), `s/t/f` flow arrows. Constraint: duration events on one track **must nest**; overlapping non-nested events go to an
  overflow track and are "out of spec". Parsed into Perfetto SQL tables (`slice, track, process, thread, counter, args`).
  Source: perfetto.dev/docs/getting-started/other-formats. The page does not mention OTLP/OpenTelemetry - Perfetto does not natively ingest OTLP (as far as sourced).
- Mapping for Switchyard: parcel = one "process", each parcel lifecycle phase (induct, travel, wait-at-chute, sort, deliver) = `X` slices on that
  parcel's track; robot = a thread; blocking/deadlock cause = flow arrows (`s/f`) between slices. Parcel phases should nest or sit sequentially
  on a single track to avoid the overflow-track problem; for overlaps use separate tracks (tid per concurrent lane).
  Millions of events: Perfetto UI loads large JSON slowly (unverified for exact limits; the docs state no numeric limits). Export a *sampled/selected*
  subset (one parcel, one failure window) for demos; keep the full set in Parquet.
- **Jaeger**: v2 is current (2.22 "latest" on jaegertracing.io), built on the OpenTelemetry Collector framework and OTLP-native (OTLP receiver
  enabled by default since v1.46, per release notes). UI file upload historically expects **Jaeger's own JSON format** (`{"data":[{traceID,spans,processes}]}`),
  and had bugs in old versions (jaeger-ui issue #381). **Whether the current UI accepts raw OTLP JSON upload is UNVERIFIED** - I could not confirm it.
  Reliable path: run Jaeger all-in-one (Docker or the single binary), then push OTLP (HTTP/JSON on 4318 or gRPC on 4317) from the CLI
  (`switchyard export --otlp http://localhost:4318`). Fallback path: also emit Jaeger-JSON for UI upload. Spike this in week 1.
  Jaeger demo caveats: it needs Docker or a downloaded binary -> good for the engineer demo, not the 10-second recruiter demo (use a recorded GIF/video of it).
- **Parquet / Arrow / DuckDB**: Parquet is the right at-rest format for millions of spans (columnar, compressed, dictionary-encodes
  repeated names). DuckDB queries Parquet in place (`SELECT ... FROM 'spans.parquet'`) and has a Rust crate (`duckdb`, bundled build is slow to compile - see risks).
  In-memory: a hand-rolled columnar struct-of-Vecs (`trace_id: Vec<u64>, start: Vec<u32>, dur: Vec<u32>, kind: Vec<u8>, parent: Vec<u32>`) during the
  run is faster and more deterministic than building Arrow arrays per span; convert to Arrow RecordBatch only at flush. Interned span names.
- **OTel mapping**: trace_id = 128-bit derived deterministically from `(seed, parcel_id)`; span_id 64-bit from counter; timestamps in
  *simulated* ns (unix_nano field) - note this puts traces at epoch-near dates; offset to a fixed base time so Jaeger's time-range search finds them.
  Use `opentelemetry-proto` types + `prost`/`serde_json` for hand-built OTLP JSON; the full OTel SDK (`tracing-opentelemetry`) is the wrong tool
  for replaying simulated timestamps.

### 3.3 Bevy status
- Bevy 0.20 released 2026-10-08 (bevy.org/news; crates.io). Previous: 0.19 (2026-06-19), 0.18 (2026-01-13) - i.e. a breaking release roughly every 3-4 months.
  Bevy's WebGPU/WebGL2-in-browser status for 0.20 could not be confirmed from fetched pages (the only WebGPU news item found was from 2023). Unverified.
  Implication: a Bevy viewer has recurring upgrade cost; acceptable for a later "3D viewer" add-on, not for the V1 critical path.

### 3.4 CI / testing (from established practice; not re-fetched this session)
- GitHub Actions: `dtolnay/rust-toolchain@stable` (with `components: clippy, rustfmt`), `Swatinem/rust-cache@v2`, matrix `ubuntu-latest, windows-latest, macos-latest`.
  Jobs: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace` (or `cargo nextest run`), `cargo build --target wasm32-unknown-unknown -p switchyard-wasm`,
  determinism job: run the same seed on all three OSes and compare state hash (artifact diff). Pin the toolchain in `rust-toolchain.toml`. Deploy web build to Pages via `actions/deploy-pages`.
- proptest: invariants (no two robots on one cell, no swap conflicts, every parcel eventually delivered when no failures, replay(seed) == replay(seed)).
- insta: snapshot the trace of a tiny scenario and RCA report text/JSON; `cargo insta review` locally.
- criterion: bench ticks/sec at 200/500/1000 robots; run on CI only as non-gating trend (shared runners are noisy); keep a regression gate via a local baseline or `iai-callgrind`-style instruction counts (optional, unverified fit).

## 4. Crates and libraries (versions from crates.io API, 2026-10-19; use `cargo add` at start and pin via Cargo.lock)

| Purpose | Crate | Latest stable seen |
|---|---|---|
| Parallel batch / counterfactuals | rayon | 1.12.0 |
| RNG (pin algorithm) | rand_chacha / rand_pcg | 0.10.0 / 0.10.2 |
| Ordered map | indexmap (or std BTreeMap) | 2.14.2 |
| Property tests | proptest | 1.12.0 |
| Snapshot tests | insta | 1.49.0 |
| Benchmarks | criterion | 0.8.2 |
| Columnar / Parquet | arrow, parquet | 60.0.0 / 60.0.0 |
| DataFrames (Rust side, optional) | polars | 0.55.2 |
| SQL on traces | duckdb (Rust crate; or call DuckDB CLI/Python) | 1.10506.1 |
| OTLP types | opentelemetry-proto, prost | 0.33.1, 0.14.4 |
| JSON | serde_json | 1.0.154 |
| WASM bindings | wasm-bindgen, wasm-pack | 0.2.129, 0.15.0 |
| Python bindings (optional) | pyo3 | 0.30.1 |
| Native viewer (later) | bevy (bevy_ecs) | 0.20.0 |
| Small ECS if wanted in core | hecs | 0.11.2 |
| Web | Vite, TypeScript, PixiJS v8 (versions not checked) | - |

Caveat: polars/arrow/parquet move fast and break APIs; arrow-rs and polars-arrow are different implementations. Prefer `arrow`+`parquet` for writing, DuckDB/Python Polars for reading.

## 5. Dev machine check (nothing installed)

| Tool | Result |
|---|---|
| rustc | NOT FOUND |
| cargo | NOT FOUND |
| rustup | NOT FOUND |
| wasm-pack | NOT FOUND |
| node | v16.14.2 (too old: Vite 5+/PixiJS tooling needs Node 18+/20+; upgrade needed) |
| python | 3.11.9 (fine for Polars/DuckDB/Jupyter) |
| git | 2.43.0.windows.1 |
| duckdb CLI | NOT FOUND |
Windows 10 Home 10.0.19045. Rust on Windows needs the MSVC build tools (Visual Studio Build Tools, "Desktop development with C++") or the GNU target - not checked.
Docker was not checked (relevant for Jaeger demo).

Setup needed (user to run, not done here): rustup (stable, MSVC), `rustup target add wasm32-unknown-unknown`, wasm-pack, Node 20 LTS, DuckDB CLI, Docker or Jaeger binary.

## 6. Risks

1. **Toolchain absent**: Rust not installed; Node 16 is EOL. First day is setup; confirm MSVC build tools present.
2. **Determinism drift across OS/WASM**: float behavior, hash iteration, rayon ordering. Mitigate: integer/fixed-point sim state, banned HashMap, state-hash CI on win/mac/linux + wasm-vs-native test.
3. **Jaeger import not verified**: OTLP-JSON file upload unconfirmed; the OTLP-to-collector path is solid but needs Docker/binary. Spike early; keep Chrome-trace/Perfetto as the zero-install fallback.
4. **Perfetto nesting rule**: non-nested overlapping slices land on overflow tracks; trace schema must be designed for nesting (per-lane tracks) from the start.
5. **Trace volume**: millions of spans in browser JSON will choke Perfetto/Jaeger UIs and WASM memory (4 GB cap, 2 GB practical). Export selective subsets; keep Parquet canonical; sample.
6. **WASM performance and threading**: multi-threaded WASM needs SharedArrayBuffer (COOP/COEP headers; GitHub Pages cannot set them without a service-worker shim). Plan: web demo runs one sim in one worker; heavy RCA fan-out is CLI-only, or precomputed results shipped as static files.
7. **Scope creep**: 1000+ robots + RCA + tracing + viewer + failure injection is large. Cut line: V1 = core + PIBT + traces + Perfetto export + web viewer + 3 injected failures + one RCA with counterfactual replay.
8. **Dependency churn**: Bevy (3-4 month breaking cycle), arrow/polars major versions. Pin with Cargo.lock; keep them out of the core crate.
9. **DuckDB Rust crate** compiles bundled C++ (slow, needs compiler on Windows); prefer the DuckDB CLI/Python for queries, not a core dependency.
10. **Algorithm correctness**: lifelong MAPF deadlock/livelock subtleties; cross-check PIBT/LaCAM against the C++ reference on small instances (gaps: I did not locate/verify the exact repos).
11. **Search limits**: several facts above (Bevy WebGPU status, Jaeger OTLP upload, CI action versions, bench noise) were not confirmed from fetched sources; they are labeled unverified.

## 7. Sources
- Perfetto, other formats: https://perfetto.dev/docs/getting-started/other-formats
- Jaeger docs (v2.22, OTel Collector base): https://www.jaegertracing.io/docs/latest/ ; v1.46 release (OTLP receiver default): https://github.com/jaegertracing/jaeger/releases/tag/v1.46.0
- jaeger-ui upload issue: https://github.com/jaegertracing/jaeger-ui/issues/381
- Bevy news: https://bevy.org/news/
- Reproducible HashMap order: https://reproducible-builds.org/docs/stable-outputs ; Materialize hash policy: https://dev.materialize.com/api/rust/src/mz_ore/collections/hash.rs.html ; scx_simulator audit: https://gitea.hillion.co.uk/JakeHillion/scx-upstream/raw/branch/simulator.v3/rust/scx_simulator/ai_docs/HASHMAP_AUDIT.md
- PIBT: https://arxiv.org/pdf/1901.11282 ; League of Robot Runners paper: https://arxiv.org/html/2404.16162v1
- Crate versions: https://crates.io/api/v1/crates/<name>
