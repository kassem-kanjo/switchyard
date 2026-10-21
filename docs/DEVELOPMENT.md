# Development setup

## Requirements
| Tool | Version | Used for |
|---|---|---|
| Rust (rustup) | stable (see `rust-toolchain.toml`) + `wasm32-unknown-unknown` | core, CLI, WASM |
| wasm-bindgen-cli | **must match** the `wasm-bindgen` version in `Cargo.lock` (currently 0.2.129) | JS bindings for the viewer |
| Node.js | 22 LTS | viewer (Vite + TypeScript) |

## macOS / Linux
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked
# Node 22: use your package manager or nvm
```

## Windows (recommended: MSVC)
1. Install the Visual Studio Build Tools with the C++ workload (needs admin, once):
   ```powershell
   winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
   ```
2. Install rustup (`winget install Rustlang.Rustup`, or rustup-init.exe), then `rustup target add wasm32-unknown-unknown`.
3. `cargo install wasm-bindgen-cli --version 0.2.129 --locked`
4. Node 22 LTS.

This matches CI (GitHub's `windows-latest` runners use MSVC).

### Fallback without admin rights (GNU toolchain)
If you can't install the Build Tools: install rustup with `--default-host x86_64-pc-windows-gnu`. Some crates (`windows-sys` ≥ 0.61) then need `dlltool`: extract [w64devkit](https://github.com/skeeto/w64devkit) and put **only** `dlltool.exe` and `as.exe` on PATH. Its `gcc` must not be on PATH, because it replaces Rust's bundled linker and fails with `-lgcc_eh`. For crates that compile C (e.g. `ring`), set `CC_x86_64_pc_windows_gnu` and `AR_x86_64_pc_windows_gnu` to w64devkit's `gcc.exe` and `ar.exe` for that command only.

The product's own crates need no C compiler; keep it that way (ADR 0001).

## Everyday commands
```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p switchyard-cli -- run --scenario scenarios/small.toml --ticks 2000
cargo run --release -p switchyard-cli -- run --scenario medium --ticks 14400   # preset by name

cd viewer && npm install && npm run dev      # viewer
```

## Determinism check (CI)
ADR 0002 promises that one scenario and seed give a bit-identical state hash everywhere. CI proves it: `determinism-native` (Linux, Windows, macOS; release CLI) and `determinism-wasm` (Node 22, `wasm-bindgen --target nodejs`) compare against `scripts/determinism/expected.txt` (`<scenario> <ticks> <seed> <hash>`; small, medium, large and the `two-dead-robots-and-a-jam` chaos scenario, which `run` supports because faults live in the scenario TOML).

- Locally: `scripts/determinism/check.sh` (after `cargo build --release -p switchyard-cli`).
- A hash change must be deliberate: run `scripts/determinism/update.sh`, commit `expected.txt`, and explain in the PR why the simulation's behaviour changed. An unexplained change is a determinism bug.
