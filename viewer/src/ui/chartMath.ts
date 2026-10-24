/** Pure scaling and hover maths for the throughput chart (no DOM, so it is unit-testable). */

export interface PlotRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface Scale {
  /** Number of data points. */
  n: number;
  yMax: number;
  rect: PlotRect;
  /** x for a (possibly fractional) point index. A single point sits mid-plot. */
  x(index: number): number;
  /** y for a value; 0 is the plot bottom, yMax the top. */
  y(value: number): number;
}

/** Round up to 1, 2, 2.5, 5 or 10 times a power of ten, so axis ticks land on tidy numbers. */
export function niceCeil(v: number): number {
  if (!(v > 0)) return 1;
  const mag = 10 ** Math.floor(Math.log10(v));
  for (const m of [1, 2, 2.5, 5, 10]) {
    if (v <= m * mag + 1e-9) return m * mag;
  }
  return 10 * mag;
}

export function makeScale(n: number, maxValue: number, rect: PlotRect): Scale {
  const yMax = niceCeil(maxValue);
  return {
    n,
    yMax,
    rect,
    x: (i) => rect.left + (n > 1 ? i / (n - 1) : 0.5) * rect.width,
    y: (v) => rect.top + rect.height - (Math.min(Math.max(v, 0), yMax) / yMax) * rect.height,
  };
}

/** Nearest point index to a pixel x, or null when the pointer is outside the plot or there are no points. */
export function indexAt(px: number, scale: Scale): number | null {
  const { rect, n } = scale;
  if (n <= 0 || px < rect.left || px > rect.left + rect.width) return null;
  if (n === 1) return 0;
  const f = ((px - rect.left) / rect.width) * (n - 1);
  return Math.min(n - 1, Math.max(0, Math.round(f)));
}

/** Fractional point index of a tick, for placing fault windows. */
export function tickToIndex(tick: number, startMinute: number, ticksPerMinute: number): number {
  return tick / ticksPerMinute - startMinute;
}

/** Evenly spaced y-axis tick values from 0 to yMax. */
export function yTicks(yMax: number, count = 3): number[] {
  return Array.from({ length: count + 1 }, (_, i) => (yMax * i) / count);
}

export interface HoverReadout {
  minute: number;
  delivered: number;
  twin: number | null;
  /** delivered - twin, when a twin exists. */
  delta: number | null;
}

export function hoverReadout(
  index: number,
  startMinute: number,
  delivered: readonly number[],
  twin?: readonly number[],
): HoverReadout | null {
  const d = delivered[index];
  if (d === undefined) return null;
  const t = twin?.[index];
  return { minute: startMinute + index, delivered: d, twin: t ?? null, delta: t === undefined ? null : d - t };
}

export function formatReadout(r: HoverReadout): string {
  const parts = [`min ${r.minute}`, `${r.delivered} delivered`];
  if (r.twin !== null && r.delta !== null) {
    const sign = r.delta > 0 ? "+" : r.delta < 0 ? "−" : "";
    parts.push(`twin ${r.twin} (${sign}${Math.abs(r.delta)})`);
  }
  return parts.join(" · ");
}

/** The deepest relative shortfall against the twin inside [from, to] (inclusive point indices), or null. */
export function deepestDip(
  delivered: readonly number[],
  twin: readonly number[] | undefined,
  from: number,
  to: number,
): { index: number; pct: number } | null {
  if (!twin) return null;
  let best: { index: number; pct: number } | null = null;
  for (let i = Math.max(0, Math.ceil(from)); i <= Math.min(to, delivered.length - 1); i++) {
    const t = twin[i];
    const d = delivered[i];
    if (t === undefined || d === undefined || t <= 0) continue;
    const pct = Math.round(((t - d) / t) * 100);
    if (pct > 0 && (best === null || pct > best.pct)) best = { index: i, pct };
  }
  return best;
}

/**
 * Half-width of the twin's noise band: the standard deviation of (delivered - twin) over the points before
 * the first fault, where the two lines differ only by chance. Falls back to the twin's own spread.
 */
export function noiseHalfWidth(delivered: readonly number[], twin: readonly number[], firstFaultIndex: number): number {
  const end = Math.max(0, Math.min(delivered.length, Math.floor(firstFaultIndex)));
  const diffs: number[] = [];
  for (let i = 0; i < end; i++) diffs.push((delivered[i] ?? 0) - (twin[i] ?? 0));
  const sample = diffs.length >= 3 ? diffs : twin.slice(0, Math.max(end, 3));
  if (sample.length < 2) return 0;
  const mean = sample.reduce((a, b) => a + b, 0) / sample.length;
  return Math.sqrt(sample.reduce((a, b) => a + (b - mean) ** 2, 0) / sample.length);
}
