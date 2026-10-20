// Builds the Rust WASM facade and generates the JS glue into src/wasm/pkg.
// Cross-platform: needs cargo, the wasm32-unknown-unknown target and a
// wasm-bindgen CLI whose version matches Cargo.lock (see docs/DEVELOPMENT.md).
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const viewerDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(viewerDir, "..");
const outDir = join(viewerDir, "src", "wasm", "pkg");
const targetDir = process.env.CARGO_TARGET_DIR
  ? resolve(process.env.CARGO_TARGET_DIR)
  : join(repoRoot, "target");

function run(cmd, args) {
  console.log(`> ${cmd} ${args.join(" ")}`);
  const r = spawnSync(cmd, args, { cwd: repoRoot, stdio: "inherit" });
  if (r.error) {
    console.error(`failed to run ${cmd}: ${r.error.message}`);
    process.exit(1);
  }
  if (r.status !== 0) process.exit(r.status ?? 1);
}

run("cargo", ["build", "-p", "switchyard-wasm", "--target", "wasm32-unknown-unknown", "--release"]);

const wasm = join(targetDir, "wasm32-unknown-unknown", "release", "switchyard_wasm.wasm");
if (!existsSync(wasm)) {
  console.error(`expected ${wasm} after cargo build`);
  process.exit(1);
}
mkdirSync(outDir, { recursive: true });
run("wasm-bindgen", ["--target", "web", "--out-dir", outDir, wasm]);
console.log(`WASM package written to ${outDir}`);
