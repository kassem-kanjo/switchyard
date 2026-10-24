import { CellKind, RobotState, type FloorMarks, type LayoutView, type SnapshotView } from "./Renderer";
import { fitToCanvas } from "./fit";

/** Cell index of every chute, in the engine's order (row-major), so "chute N" is `chuteCells(layout)[N]`. */
export function chuteCells(layout: LayoutView): number[] {
  const out: number[] = [];
  layout.cells.forEach((k, i) => {
    if (k === CellKind.Chute) out.push(i);
  });
  return out;
}

/** Cells of the chutes named by the active `chute_jam` faults. */
export function jammedCellsOf(
  active: readonly { kind: string; params: Record<string, number> }[],
  chutes: readonly number[],
): number[] {
  const out: number[] = [];
  for (const f of active) {
    const c = f.kind === "chute_jam" ? chutes[f.params.chute ?? -1] : undefined;
    if (c !== undefined) out.push(c);
  }
  return out;
}

/**
 * A soft glow over the area the incident touches: full strength on each failed robot and jammed chute,
 * fading to nothing `radius` cells away. Row-major, 0..1.
 */
export function buildGlow(layout: LayoutView, snap: SnapshotView, jammed: readonly number[], radius = 4): Float32Array {
  const { width: w, height: h } = layout;
  const glow = new Float32Array(w * h);
  const stamp = (cx: number, cy: number) => {
    for (let y = Math.max(0, cy - radius); y <= Math.min(h - 1, cy + radius); y++) {
      for (let x = Math.max(0, cx - radius); x <= Math.min(w - 1, cx + radius); x++) {
        const v = 1 - Math.hypot(x - cx, y - cy) / (radius + 1);
        if (v > glow[y * w + x]!) glow[y * w + x] = v;
      }
    }
  };
  for (let i = 0; i < snap.robotState.length; i++) {
    if (snap.robotState[i] === RobotState.Failed) stamp(snap.robotXY[2 * i]!, snap.robotXY[2 * i + 1]!);
  }
  for (const c of jammed) stamp(c % w, Math.floor(c / w));
  return glow;
}

export function floorMarks(layout: LayoutView, snap: SnapshotView, jammedCells: number[]): FloorMarks {
  const glow = buildGlow(layout, snap, jammedCells);
  return { jammedCells, glow: glow.some((v) => v > 0) ? glow : null };
}

export interface Frame {
  /** Zoom factor, 1 = whole floor. */
  k: number;
  /** CSS translation in px, applied with transform-origin 0 0. */
  tx: number;
  ty: number;
}

/** Zoom and pan that centre the glowing area in the canvas, never leaving the canvas edges; null if no glow. */
export function incidentFrame(
  glow: Float32Array | null,
  layout: LayoutView,
  w: number,
  h: number,
  padding: number,
  maxZoom = 1.8,
): Frame | null {
  if (!glow) return null;
  let x0 = Infinity, y0 = Infinity, x1 = -1, y1 = -1;
  glow.forEach((v, i) => {
    if (v < 0.2) return;
    const x = i % layout.width;
    const y = Math.floor(i / layout.width);
    x0 = Math.min(x0, x); y0 = Math.min(y0, y); x1 = Math.max(x1, x + 1); y1 = Math.max(y1, y + 1);
  });
  if (x1 < 0) return null;
  const fit = fitToCanvas(layout.width, layout.height, w, h, padding);
  const bw = (x1 - x0) * fit.scale;
  const bh = (y1 - y0) * fit.scale;
  const k = Math.max(1, Math.min(maxZoom, (w * 0.85) / bw, (h * 0.85) / bh));
  const cx = fit.offsetX + ((x0 + x1) / 2) * fit.scale;
  const cy = fit.offsetY + ((y0 + y1) / 2) * fit.scale;
  const clamp = (v: number, size: number) => Math.min(0, Math.max(size * (1 - k), v));
  return { k, tx: clamp(w / 2 - k * cx, w), ty: clamp(h / 2 - k * cy, h) };
}
