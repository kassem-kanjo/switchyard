/** Pure logic for the first-run guided sequence: how fast to play, and when the incident is at its deepest. */
import type { MetricsSeries } from "../../data/types";

export type GuideSpeed = 1 | 4 | 64;

/**
 * Speed by sim tick. Real time is slow (a shift minute takes 60 s at 1x), so the run fast-forwards to
 * three minutes before the dip, plays at 4x into it, drops to 1x for the first 30 s of the incident so the
 * viewer sees the jam and the failures happen, then 4x down to the deepest point.
 */
export function speedAt(tick: number, incidentStartMin: number, ticksPerMinute: number): GuideSpeed {
  const start = incidentStartMin * ticksPerMinute;
  if (tick < start - 3 * ticksPerMinute) return 64;
  if (tick < start) return 4;
  if (tick < start + ticksPerMinute / 2) return 1;
  return 4;
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
