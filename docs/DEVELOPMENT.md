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

## Windows without Visual Studio (GNU toolchain)
Windows normally needs the MSVC build tools, which require an admin install. The setup below avoids that:
1. Install rustup with the GNU host: download `rustup-init.exe` from `static.rust-lang.org/rustup/dist/x86_64-pc-windows-gnu/` and run `rustup-init.exe -y --default-host x86_64-pc-windows-gnu -t wasm32-unknown-unknown`.
2. Some crates (`windows-sys` ≥ 0.61) need `dlltool`, and a few build-time tools need a C compiler. Extract [w64devkit](https://github.com/skeeto/w64devkit) to `%LOCALAPPDATA%\Programs\w64devkit`. **Do not put its `bin` on PATH**: its `gcc` would replace Rust's bundled linker and fail with `cannot find -lgcc_eh`. Instead:
   - copy `dlltool.exe` and `as.exe` into a folder of their own (e.g. `%LOCALAPPDATA%\Programs\mingw-dlltool\bin`) and add *that* to PATH;
   - when a crate needs a C compiler (e.g. `ring` when installing `wasm-bindgen-cli`), set it for that command only:
     ```bash
     CC_x86_64_pc_windows_gnu=".../w64devkit/bin/gcc.exe" AR_x86_64_pc_windows_gnu=".../w64devkit/bin/ar.exe" \
       cargo install wasm-bindgen-cli --version 0.2.129 --locked
     ```
3. Node 22: the official zip from nodejs.org, extracted to `%LOCALAPPDATA%\Programs` and added to PATH.

The product's own crates need no C compiler; keep it that way (ADR 0001).

## Everyday commands
```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p switchyard-cli -- run --scenario scenarios/small.toml --ticks 2000

cd viewer && npm install && npm run dev      # viewer
```
