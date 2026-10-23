// Precomputes the post-mortem report of each chaos preset with the native CLI
// (ADR 0006 §8: full post-mortems are native; the browser serves the result).
// Output: viewer/public/reports/<preset>.md, committed so a static host needs
// no toolchain. Re-run after any change to the engine or the presets:
//   node scripts/precompute-reports.mjs
import { spawnSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const viewerDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(viewerDir, "..");
const outDir = join(viewerDir, "public", "reports");

/** Presets with a fault schedule, and the ticks to simulate (14400 = 2 h). */
const PRESETS = ["aisle-7-blocked", "peak-surge", "two-dead-robots-and-a-jam"];
const TICKS = 14400;

mkdirSync(outDir, { recursive: true });
for (const name of PRESETS) {
  const out = join(outDir, `${name}.md`);
  const args = ["run", "--release", "-q", "-p", "switchyard-cli", "--", "postmortem",
    "--scenario", name, "--ticks", String(TICKS), "--out", out];
  console.log(`> cargo ${args.join(" ")}`);
  const r = spawnSync("cargo", args, { cwd: repoRoot, stdio: "inherit" });
  if (r.error || r.status !== 0) {
    console.error(`post-mortem of ${name} failed`);
    process.exit(r.status ?? 1);
  }
}
