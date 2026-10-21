/** All colours and sizes for the viewer live here so visuals are swappable. */
export const theme = {
  background: "#1d2330",
  floor: "#2b3345",
  gridLine: "#343d52",
  wall: "#0f131c",
  chute: "#d9a441",
  induction: "#4fae8b",
  /** Colour per robot state: idle, to-induction, loading, to-chute, dropping, failed. */
  robotState: ["#b4bfd6", "#5aa9e6", "#8e7cc3", "#e8e8f0", "#f0a04b", "#e0524f"],
  highlight: "#ffffff",
  /** Outline drawn around parked/idle robots so they separate from the floor. */
  idleOutline: "#e6ebf7",
  /** Robot state names, index-aligned with robotState (state 0 is idle/parked). */
  robotStateLabel: ["Parked / idle", "To induction", "Loading", "Carrying", "Dropping", "Failed"],
  /** Robot square size as a fraction of a cell. */
  robotScale: 0.6,
  /** Padding around the floor, in CSS pixels. */
  padding: 12,

  /** Simulation clock. Mirrors the core's tick length; used by every time label. */
  // 1 tick = 0.5 s (docs/ARCHITECTURE.md §3), so 120 ticks per simulated minute.
  ticksPerMinute: 120,

  /** Sidebar chrome. Applied as CSS custom properties by applyThemeVars(). */
  ui: {
    panel: "#262d3d",
    panelRaised: "#2e3649",
    border: "#343d52",
    borderStrong: "#44506b",
    text: "#e6e9ef",
    textMuted: "#9aa6bf",
    accent: "#5aa9e6",
    danger: "#e0524f",
    warn: "#f0a04b",
    badgeBg: "#7a4b12",
    badgeText: "#ffd9a0",
    sidebarWidth: 380,
  },

  /** Below this viewport width the sidebar drops under the floor (mirrored in app.css). */
  narrowBreakpoint: 820,

  chart: {
    height: 150,
    line: "#5aa9e6",
    lineWidth: 2,
    twin: "#9aa6bf",
    twinDash: [5, 4] as number[],
    grid: "#343d52",
    axisText: "#9aa6bf",
    faultShade: "rgba(224, 82, 79, 0.18)",
    faultEdge: "rgba(224, 82, 79, 0.55)",
    hoverLine: "#e6e9ef",
    tooltipBg: "#0f131c",
    margin: { left: 34, right: 8, top: 8, bottom: 18 },
    font: "11px system-ui, sans-serif",
  },

  /** Parcel phase colours, in lifecycle order (see PRODUCT.md capability 2). */
  phase: {
    queued: "#6b7894",
    awaiting_robot: "#8d9ab5",
    loading: "#8e7cc3",
    travelling: "#5aa9e6",
    blocked: "#e0524f",
    chute_wait: "#d9a441",
    drop: "#4fae8b",
  },
} as const;

/** Push theme colours into CSS custom properties so stylesheets never hard-code them. */
export function applyThemeVars(root: HTMLElement = document.documentElement, th: Theme = theme): void {
  const set = (k: string, v: string | number) => root.style.setProperty(`--${k}`, typeof v === "number" ? `${v}px` : v);
  set("bg", th.background);
  set("wall", th.wall);
  set("grid-line", th.gridLine);
  for (const [k, v] of Object.entries(th.ui)) set(k.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`), v);
  for (const [k, v] of Object.entries(th.phase)) set(`phase-${k.replace(/_/g, "-")}`, v);
}

export type Theme = typeof theme;
