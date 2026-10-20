/** All colours and sizes for the viewer live here so visuals are swappable. */
export const theme = {
  background: "#1d2330",
  floor: "#2b3345",
  gridLine: "#343d52",
  wall: "#0f131c",
  chute: "#d9a441",
  induction: "#4fae8b",
  /** Colour per robot state: idle, to-induction, loading, to-chute, dropping, failed. */
  robotState: ["#8c97ad", "#5aa9e6", "#8e7cc3", "#e8e8f0", "#f0a04b", "#e0524f"],
  highlight: "#ffffff",
  /** Robot square size as a fraction of a cell. */
  robotScale: 0.6,
  /** Padding around the floor, in CSS pixels. */
  padding: 12,
} as const;

export type Theme = typeof theme;
