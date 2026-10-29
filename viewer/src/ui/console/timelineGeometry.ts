/** Pure scaling and layout maths for the timeline pane (no DOM, so it is unit-testable). */
import type { TimelineModel, TimelineSpan } from "./types";

/** viewBox of the timeline svg. */
export const TL_W = 1400;
export const TL_H = 100;

/** Plot area: 6px breathing room at the top, 16px label strip at the bottom. */
export const PLOT_TOP = 6;
export const PLOT_BOTTOM = 82;
/** The tallest value is drawn at y=10, leaving a little headroom inside the plot. */
const VALUE_TOP = 10;

const clamp = (v: number, lo: number, hi: number): number => Math.min(hi, Math.max(lo, v));
const r2 = (v: number): number => Math.round(v * 100) / 100;

/** Minutes from shift start to svg x. Minutes are already shift-relative (0 is the axis origin). */
export function xOfMin(model: TimelineModel, min: number): number {
  if (!(model.totalMin > 0)) return 0;
  return clamp((min / model.totalMin) * TL_W, 0, TL_W);
}

/** Inverse of xOfMin, clamped to the shift. */
export function minOfX(model: TimelineModel, x: number): number {
  if (!(model.totalMin > 0)) return 0;
  return clamp((x / TL_W) * model.totalMin, 0, model.totalMin);
}

/** Tallest value either series reaches, so twin and actual share one scale. */
function yMaxOf(model: TimelineModel): number {
  let max = 0;
  for (const v of model.actual) if (v > max) max = v;
  for (const v of model.twin ?? []) if (v > max) max = v;
  return max > 0 ? max : 1;
}

/** Value to svg y: the tallest value sits at y=10, zero at y=82. */
export function yOfValue(model: TimelineModel, v: number): number {
  const max = yMaxOf(model);
  return PLOT_BOTTOM - (clamp(v, 0, max) / max) * (PLOT_BOTTOM - VALUE_TOP);
}

/** SVG points string for a series; point i is the value at minute `startMin + i`. */
export function linePoints(model: TimelineModel, series: number[]): string {
  return series
    .map((v, i) => `${r2(xOfMin(model, model.startMin + i))},${r2(yOfValue(model, v))}`)
    .join(" ");
}

/**
 * Shaded shortfall between twin and actual, clipped to the incident window:
 * forward along the twin, then back along actual. Empty when there is nothing to shade.
 */
export function lossPolygonPoints(model: TimelineModel): string {
  const { incident, twin } = model;
  if (!incident || !twin || twin.length === 0 || model.actual.length === 0) return "";

  const i0 = Math.max(0, Math.ceil(incident.startMin - model.startMin));
  const i1 = Math.min(
    model.actual.length - 1,
    twin.length - 1,
    Math.floor(incident.endMin - model.startMin),
  );
  if (i1 < i0) return "";

  const pts: string[] = [];
  for (let i = i0; i <= i1; i++) {
    const t = twin[i];
    if (t === undefined) return "";
    pts.push(`${r2(xOfMin(model, model.startMin + i))},${r2(yOfValue(model, t))}`);
  }
  for (let i = i1; i >= i0; i--) {
    const a = model.actual[i];
    if (a === undefined) return "";
    pts.push(`${r2(xOfMin(model, model.startMin + i))},${r2(yOfValue(model, a))}`);
  }
  return pts.join(" ");
}

export interface SpanRow {
  span: TimelineSpan;
  x: number;
  w: number;
  row: number;
}

/**
 * One bar per span. Rows are greedy interval colouring: a span reuses the first row whose last
 * span has already ended, so only overlapping spans are stacked.
 */
export function spanRows(model: TimelineModel): SpanRow[] {
  const sorted = [...model.spans].sort((a, b) => a.startMin - b.startMin);
  const rowEnds: number[] = [];

  return sorted.map((span) => {
    let row = rowEnds.findIndex((end) => end <= span.startMin);
    if (row === -1) {
      row = rowEnds.length;
      rowEnds.push(span.endMin);
    } else {
      rowEnds[row] = Math.max(rowEnds[row] ?? span.endMin, span.endMin);
    }

    const x = xOfMin(model, span.startMin);
    const w = Math.max(3, xOfMin(model, span.endMin) - x); // always visible, even for a zero-length span
    return { span, x, w, row };
  });
}

const pad2 = (n: number): string => String(n).padStart(2, "0");

/** Whole shift minutes as H:MM, e.g. 30 -> "00:30", 90 -> "01:30". */
export function formatMin(min: number): string {
  const whole = Math.max(0, Math.floor(min));
  return `${pad2(Math.floor(whole / 60))}:${pad2(whole % 60)}`;
}
