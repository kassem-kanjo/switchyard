/** Pure logic for the first-run guided sequence: how fast to play, and when the incident is at its deepest. */
import type { MetricsSeries } from "../../data/types";

export type GuideSpeed = 1 | 4 | 16;

/** Ticks of lead-in the run is fast-forwarded to before the incident starts. */
export const LEAD_IN_TICKS = 30;

/** The tick the guided run jumps to without rendering: just before the incident, so the viewer sees it begin. */
export function jumpTarget(incidentStartMin: number, ticksPerMinute: number): number {
  return incidentStartMin * ticksPerMinute - LEAD_IN_TICKS;
}

/**
 * Speed by sim tick, after the jump. 4x into the incident, 1x for its first 10 ticks (5 s) so the jam and
 * the failures are seen happening, then 16x down to the deepest point (real time would take minutes).
 */
export function speedAt(tick: number, incidentStartMin: number, ticksPerMinute: number): GuideSpeed {
  const rel = tick - incidentStartMin * ticksPerMinute;
  if (rel < 0) return 4;
  if (rel < 10) return 1;
  return 16;
}

/**
 * The shift minute with the largest loss against the fault-free twin, once the series shows the loss
 * easing again (so it is known to be the deepest), else null. Only finished minutes count; the last entry
 * is still in progress.
 */
export function deepestMinute(series: MetricsSeries, incidentStartMin: number): number | null {
  const twin = series.twin;
  if (!twin) return null;
  let best: { minute: number; delta: number } | null = null;
  let eased = false;
  for (let i = 0; i < series.delivered.length - 1; i++) {
    const minute = series.startMinute + i;
    const a = series.delivered[i];
    const t = twin[i];
    if (minute < incidentStartMin || a === undefined || t === undefined) continue;
    const delta = a - t;
    if (best === null || delta < best.delta) {
      best = { minute, delta };
      eased = false;
    } else if (delta > best.delta) {
      eased = true;
    }
  }
  return best !== null && best.delta < 0 && eased ? best.minute : null;
}
