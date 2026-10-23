// WASM determinism check (ADR 0002): runs each line of expected.txt through
// the WasmSim facade and compares state_hash_hex(). Exit 0 = all match.
// Usage: node wasm-hash.mjs <wasm-bindgen --target nodejs output dir>
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "../..");
const pkgDir = resolve(process.argv[2] ?? "");
if (!process.argv[2]) {
  console.error("usage: node wasm-hash.mjs <wasm-bindgen-out-dir>");
  process.exit(2);
}
const { WasmSim } = createRequire(import.meta.url)(join(pkgDir, "switchyard_wasm.js"));

let failed = false;
const lines = readFileSync(join(here, "expected.txt"), "utf8").split(/\r?\n/);
for (const line of lines) {
  if (!line.trim() || line.startsWith("#")) continue;
  const [scenario, ticks, seed, expected] = line.trim().split(/\s+/);
  // The facade has no seed argument: override the top-level `seed` like the
  // CLI's --seed does.
  const toml = readFileSync(join(root, scenario), "utf8").replace(
    /^seed\s*=\s*\d+/m,
    `seed = ${seed}`,
  );
  const sim = new WasmSim(toml);
  sim.step(Number(ticks));
  const actual = sim.state_hash_hex();
  sim.free();
  if (actual === expected) {
    console.log(`ok       ${scenario} ${ticks} ${seed} ${actual}`);
  } else {
    console.log(`MISMATCH ${scenario} ${ticks} ${seed}: expected ${expected}, actual ${actual}`);
    failed = true;
  }
}
process.exit(failed ? 1 : 0);
