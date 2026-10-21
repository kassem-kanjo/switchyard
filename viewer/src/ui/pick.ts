import type { SnapshotView } from "../renderer/Renderer";
import type { FitTransform } from "../renderer/fit";

/**
 * Robot under a click in canvas CSS pixels, or null. Robots occupy distinct
 * cells, so we match on the cell. Only states in `accept` count (e.g. those
 * carrying a parcel); pass null to accept any.
 */
export function pickRobot(
  snap: SnapshotView,
  fit: FitTransform,
  px: number,
  py: number,
  accept: readonly number[] | null,
): number | null {
  if (fit.scale <= 0) return null;
  const cx = Math.floor((px - fit.offsetX) / fit.scale);
  const cy = Math.floor((py - fit.offsetY) / fit.scale);
  for (let i = 0; i < snap.robotState.length; i++) {
    if (snap.robotXY[2 * i] === cx && snap.robotXY[2 * i + 1] === cy) {
      return accept === null || accept.includes(snap.robotState[i]!) ? i : null;
    }
  }
  return null;
}
