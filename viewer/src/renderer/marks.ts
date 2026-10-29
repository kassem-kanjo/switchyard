/** Pure geometry/colour maths for incident floor marks. No DOM access. */
const TAU = Math.PI * 2;

/** Smooth 0..1 pulse over `periodMs`; 0 at t=0, 1 at half a period. */
export function ringPhase(nowMs: number, periodMs = 1200): number {
  if (!Number.isFinite(nowMs) || !(periodMs > 0)) return 0;
  const wrapped = ((nowMs % periodMs) + periodMs) % periodMs;
  return 0.5 - 0.5 * Math.cos((TAU * wrapped) / periodMs);
}

/** Expanding (or, when motion is reduced, static) ring around a failed robot. */
export function ringRadius(cell: number, phase: number, reducedMotion: boolean): { r: number; alpha: number } {
  if (reducedMotion) return { r: cell * 0.65, alpha: 0.9 };
  return { r: cell * (0.55 + 0.25 * phase), alpha: 0.9 - 0.6 * phase };
}

/**
 * Diagonal 45-degree hatch segments (x1,y1,x2,y2) filling the square
 * [x,x+size]x[y,y+size], spaced every `step` px along the anti-diagonal.
 */
export function hatchSegments(x: number, y: number, size: number, step: number): Array<[number, number, number, number]> {
  const out: Array<[number, number, number, number]> = [];
  if (!(size > 0) || !(step > 0) || !Number.isFinite(x + y + size + step)) return out;
  for (let c = -size; c < size; c += step) {
    if (c < 0) {
      // Enters the top edge and leaves the right edge.
      out.push([x - c, y, x + size, y + size + c]);
    } else {
      // Enters the left edge and leaves the bottom edge.
      out.push([x, y + c, x + size - c, y + size]);
    }
  }
  return out;
}

