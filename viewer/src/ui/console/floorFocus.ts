/** Where to draw the selection ring on the floor, in canvas CSS pixels (before the incident-frame zoom, which the ring shares). */
import type { FitTransform } from "../../renderer/fit";

export interface Ring {
  left: number;
  top: number;
  size: number;
}

/** A ring centred on grid cell (x, y), a bit larger than the cell so it frames the robot or chute. */
export function focusRect(fit: FitTransform, x: number, y: number): Ring {
  const size = Math.max(14, fit.scale * 1.7);
  return { left: fit.offsetX + (x + 0.5) * fit.scale - size / 2, top: fit.offsetY + (y + 0.5) * fit.scale - size / 2, size };
}
