/** All colours and sizes for the viewer live here so visuals are swappable. */
export const theme = {
  background: "#1C2127",
  floor: "#1C2127",
  gridLine: "#252A31",
  wall: "#11141A",
  chute: "#D1980B",
  induction: "#238551",
  /** Colour per robot state: idle, to-induction, loading, to-chute, dropping, failed. */
  robotState: ["#686C73", "#0090ff", "#8e4ec6", "#F6F7F9", "#f76b15", "#CD4246"],
  highlight: "#F6F7F9",
  /** Outline drawn around parked/idle robots so they separate from the floor. */
  idleOutline: "#ABB3BF",
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
    panel: "#252A31",
    panelRaised: "#2F343C",
    border: "#383E47",
    borderStrong: "#404854",
    text: "#F6F7F9",
    textMuted: "#ABB3BF",
    accent: "#2D72D2",
    danger: "#CD4246",
    warn: "#C87619",
    ok: "#238551",
    badgeBg: "#3a2410",
    badgeText: "#EC9A3C",
    sidebarWidth: 380,
    font: 'system-ui, -apple-system, "Segoe UI", Roboto, sans-serif',
    fontMono: 'ui-monospace, Consolas, "SF Mono", monospace',
    fsCaption: 12,
    fsBody: 13,
    fsLabel: 11,
    fsTitle: 15,
    fsHeadline: 20,
    fsNumber: 32,
    s1: 4,
    s2: 8,
    s3: 12,
    s4: 16,
    s5: 24,
    s6: 32,
    radiusCard: 2,
    radiusControl: 2,
  },

  /** Below this viewport width the sidebar drops under the floor (mirrored in app.css). */
  narrowBreakpoint: 820,

  chart: {
    height: 150,
    line: "#2D72D2",
    lineWidth: 2,
    twin: "#ABB3BF",
    twinDash: [5, 4] as number[],
    grid: "#383E47",
    axisText: "#ABB3BF",
    faultShade: "rgba(205,66,70,0.12)",
    faultEdge: "rgba(205,66,70,0.55)",
    lossShade: "rgba(205,66,70,0.20)",
    noiseShade: "rgba(171,179,191,0.08)",
    annotation: "#CD4246",
    hoverLine: "#F6F7F9",
    tooltipBg: "#11141A",
    margin: { left: 34, right: 8, top: 8, bottom: 18 },
    font: '12px system-ui, -apple-system, "Segoe UI", Roboto, sans-serif',
  },

  /** Parcel phase colours, in lifecycle order (see PRODUCT.md capability 2). */
  phase: {
    queued: "#5F6B7C",
    awaiting_robot: "#8F99A8",
    loading: "#8e4ec6",
    travelling: "#2D72D2",
    blocked: "#CD4246",
    chute_wait: "#C87619",
    drop: "#238551",
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
