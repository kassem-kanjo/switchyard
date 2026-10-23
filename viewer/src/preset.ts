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
