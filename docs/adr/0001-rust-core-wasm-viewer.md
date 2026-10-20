# 0001. Rust core, native CLI and a WASM-powered web viewer

**Status:** accepted · **Date:** 2026-10-19 · **Research:** [tech stack](../research/2026-10-19-tech-stack.md)

## Context
We need a deterministic, fast simulation of up to 1,000 robots, parallel counterfactual replays, trace export, and a demo a recruiter can open in seconds. The visuals must be easy to replace later. The development machine runs Windows without MSVC; the product must also run on macOS and Linux.

## Decision
- **Rust** for all simulation and analysis logic, split into crates: `core`, `trace`, `analysis`, `cli`, `wasm`.
- **Web viewer** in TypeScript + Vite. The core is compiled to WASM and runs in a Web Worker; the viewer draws through a `Renderer` interface (Canvas 2D in V1).
- **CLI** for heavy batch work (chaos suites, post-mortems with rayon fan-out, exports).
- Python and DuckDB may consume exported files for analysis notebooks; they are never required to run the product.
- On the Windows dev box we use the `x86_64-pc-windows-gnu` toolchain and avoid C-dependent crates.

## Alternatives considered
- **Bevy native app:** strong visuals, but large WASM bundles, frequent breaking releases (0.20 shipped 2026-10-08) and scheduling non-determinism if it touches the sim. It can be added later as another consumer of the snapshot stream.
- **C++:** most MAPF research code is C++, but the build story on Windows, WASM and testing tools favour Rust. We port algorithms rather than link them.
- **Python core:** too slow for 1,000 robots × 64 replays.

## Consequences
- A single `cargo run` gives the full product; `npm run dev` gives the viewer.
- Two languages (Rust and TypeScript) with a narrow boundary: typed arrays plus JSON.
- No multi-threaded WASM (GitHub Pages can't set the cross-origin isolation headers), so heavy replay fan-out stays in the CLI, and in-browser post-mortems use small candidate sets.
