/** Pure logic for the first-run guided sequence: how fast to play, and when the incident is at its deepest. */
import type { MetricsSeries } from "../../data/types";

export type GuideSpeed = 1 | 4 | 64;

/** Ticks of lead-in the run is fast-forwarded to before the incident starts. */
export const LEAD_IN_TICKS = 30;

/** The tick the guided run jumps to without rendering: just before the incident, so the viewer sees it begin. */
export function jumpTarget(incidentStartMin: number, ticksPerMinute: number): number {
  return incidentStartMin * ticksPerMinute - LEAD_IN_TICKS;
}

/**
 * Speed by sim tick, after the jump. 4x into the incident, 1x for its first 10 ticks (5 s) so the jam and
 * the failures are seen happening, then 64x down to the deepest point (real time would take minutes).
 */
export function speedAt(tick: number, incidentStartMin: number, ticksPerMinute: number): GuideSpeed {
  const rel = tick - incidentStartMin * ticksPerMinute;
  if (rel < 0) return 4;
  if (rel < 10) return 1;
  return 64;
}

/**
 * True once the latest finished minute is at least as far below the fault-free twin as the report's
 * worst minute (`dipPct`, whole %), so the top-bar chip and the incident card quote the same figure.
 */
export function reachedDip(series: MetricsSeries, dipPct: number): boolean {
  const i = series.delivered.length - 2; // the last entry is the minute still in progress
  const twin = series.twin?.[i];
  const actual = series.delivered[i];
  if (i < 0 || twin === undefined || actual === undefined || twin <= 0) return false;
  return Math.round((1 - actual / twin) * 100) >= dipPct;
}
