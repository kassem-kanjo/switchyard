/** Hub presets, then the chaos presets (the medium floor with a fault schedule; each has a precomputed post-mortem). */
export const PRESETS = ["small", "medium", "large", "aisle-7-blocked", "peak-surge", "two-dead-robots-and-a-jam"] as const;
export type Preset = (typeof PRESETS)[number];

/** Parse the `preset` URL parameter; unknown values fall back to "small" with a warning. */
export function parsePreset(search: string, warn: (msg: string) => void = console.warn): Preset {
  const raw = new URLSearchParams(search).get("preset");
  if (raw === null || raw === "") return "small";
  if ((PRESETS as readonly string[]).includes(raw)) return raw as Preset;
  warn(`Unknown preset "${raw}", falling back to "small" (valid: ${PRESETS.join(", ")})`);
  return "small";
}

/** True when the URL names a preset: with none, the viewer shows the first-run empty state instead of a run. */
export function hasPreset(search: string): boolean {
  const raw = new URLSearchParams(search).get("preset");
  return raw !== null && raw !== "";
}

/** The preset the first-run button loads, and the URL flag that asks for the guided sequence. */
export const FIRST_RUN_PRESET: Preset = "two-dead-robots-and-a-jam";
export const isGuided = (search: string): boolean => new URLSearchParams(search).get("guided") === "1";
