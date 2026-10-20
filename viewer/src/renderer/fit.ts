export interface FitTransform {
  /** Pixels per cell. */
  scale: number;
  offsetX: number;
  offsetY: number;
}

/** Fit a cols x rows grid centred in a w x h area (same units) with padding, preserving aspect. */
export function fitToCanvas(cols: number, rows: number, w: number, h: number, padding = 0): FitTransform {
  const availW = Math.max(0, w - 2 * padding);
  const availH = Math.max(0, h - 2 * padding);
  if (cols <= 0 || rows <= 0) return { scale: 0, offsetX: padding, offsetY: padding };
  const scale = Math.min(availW / cols, availH / rows);
  return {
    scale,
    offsetX: (w - cols * scale) / 2,
    offsetY: (h - rows * scale) / 2,
  };
}
