/** All colours and sizes for the viewer live here so visuals are swappable. */
export const theme = {
  background: "#111113",
  floor: "#18191b",
  gridLine: "#212225",
  wall: "#0b0b0c",
  chute: "#ffc53d",
  induction: "#30a46c",
  /** Colour per robot state: idle, to-induction, loading, to-chute, dropping, failed. */
  robotState: ["#64686f", "#0090ff", "#8e4ec6", "#edeef0", "#f76b15", "#e5484d"],
  highlight: "#ffffff",
  /** Outline drawn around parked/idle robots so they separate from the floor. */
  idleOutline: "#b0b4ba",
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
    panel: "#18191b",
    panelRaised: "#212225",
    border: "#272a2d",
    borderStrong: "#363a3f",
    text: "#edeef0",
    textMuted: "#b0b4ba",
    accent: "#0090ff",
    danger: "#e5484d",
    warn: "#ffc53d",
    ok: "#30a46c",
    badgeBg: "#3a2b00",
    badgeText: "#ffc53d",
    sidebarWidth: 420,
    font: 'system-ui, -apple-system, "Segoe UI", Roboto, sans-serif',
    fsCaption: 12,
    fsBody: 13,
    fsTitle: 15,
    fsHeadline: 20,
    fsNumber: 32,
    s1: 4,
    s2: 8,
    s3: 12,
    s4: 16,
    s5: 24,
    s6: 32,
    radiusCard: 6,
    radiusControl: 4,
  },

  /** Below this viewport width the sidebar drops under the floor (mirrored in app.css). */
  narrowBreakpoint: 820,

  chart: {
    height: 150,
    line: "#0090ff",
    lineWidth: 2,
    twin: "#b0b4ba",
    twinDash: [5, 4] as number[],
    grid: "#272a2d",
    axisText: "#b0b4ba",
    faultShade: "rgba(229,72,77,0.12)",
    faultEdge: "rgba(229,72,77,0.55)",
    lossShade: "rgba(229,72,77,0.20)",
    noiseShade: "rgba(176,180,186,0.08)",
    annotation: "#e5484d",
    hoverLine: "#edeef0",
    tooltipBg: "#0b0b0c",
    margin: { left: 34, right: 8, top: 8, bottom: 18 },
    font: '12px system-ui, -apple-system, "Segoe UI", Roboto, sans-serif',
  },

  /** Parcel phase colours, in lifecycle order (see PRODUCT.md capability 2). */
  phase: {
    queued: "#5b5f66",
    awaiting_robot: "#8b8d98",
    loading: "#8e4ec6",
    travelling: "#0090ff",
    blocked: "#e5484d",
    chute_wait: "#ffc53d",
    drop: "#30a46c",
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
