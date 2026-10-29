import { theme, type Theme } from "./theme";

export interface LegendEntry {
  label: string;
  colour: string;
  /** "outline" swatches draw a ring (used for parked robots). */
  kind: "robot" | "outlined-robot" | "cell";
}

/** Legend rows derived from the theme so colours live in one place. */
export function legendEntries(th: Theme = theme): LegendEntry[] {
  const s = th.robotState;
  const l = th.robotStateLabel;
  return [
    { label: l[3], colour: s[3], kind: "robot" },
    { label: l[1], colour: s[1], kind: "robot" },
    { label: l[5], colour: s[5], kind: "robot" },
    { label: l[0], colour: s[0], kind: "outlined-robot" },
    { label: "Chute", colour: th.chute, kind: "cell" },
    { label: "Pick-up station", colour: th.induction, kind: "cell" },
  ];
}

/** WCAG 2.x relative luminance of a #rrggbb colour. */
export function luminance(hex: string): number {
  const n = parseInt(hex.slice(1), 16);
  const ch = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((v) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * ch[0]! + 0.7152 * ch[1]! + 0.0722 * ch[2]!;
}

export function contrastRatio(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi! + 0.05) / (lo! + 0.05);
}

export function activeParked(state: Uint8Array): { active: number; parked: number } {
  let parked = 0;
  for (let i = 0; i < state.length; i++) if (state[i] === 0) parked++;
  return { active: state.length - parked, parked };
}
