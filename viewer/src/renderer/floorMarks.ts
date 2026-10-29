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

/** Cells of the incident: every failed robot and every jammed chute, as [x, y] pairs. */
export function incidentCells(layout: LayoutView, snap: SnapshotView, jammed: readonly number[]): [number, number][] {
  const out: [number, number][] = [];
  for (let i = 0; i < snap.robotState.length; i++) {
    if (snap.robotState[i] === RobotState.Failed) out.push([snap.robotXY[2 * i]!, snap.robotXY[2 * i + 1]!]);
  }
  for (const c of jammed) out.push([c % layout.width, Math.floor(c / layout.width)]);
  return out;
}

export function floorMarks(jammedCells: number[]): FloorMarks {
  return { jammedCells };
}

export interface Frame {
  /** Zoom factor, 1 = whole floor. */
  k: number;
  /** CSS translation in px, applied with transform-origin 0 0. */
  tx: number;
  ty: number;
}

/** Zoom and pan that centre the incident cells in the canvas, never leaving the canvas edges; null if there are none. */
export function incidentFrame(
  cells: readonly (readonly [number, number])[],
  layout: LayoutView,
  w: number,
  h: number,
  padding: number,
  maxZoom = 1.8,
  marginCells = 3,
): Frame | null {
  if (cells.length === 0) return null;
  let x0 = Infinity, y0 = Infinity, x1 = -1, y1 = -1;
  for (const [x, y] of cells) {
    x0 = Math.min(x0, x - marginCells); y0 = Math.min(y0, y - marginCells);
    x1 = Math.max(x1, x + 1 + marginCells); y1 = Math.max(y1, y + 1 + marginCells);
  }
  x0 = Math.max(0, x0); y0 = Math.max(0, y0); x1 = Math.min(layout.width, x1); y1 = Math.min(layout.height, y1);
  const fit = fitToCanvas(layout.width, layout.height, w, h, padding);
  const bw = (x1 - x0) * fit.scale;
  const bh = (y1 - y0) * fit.scale;
  const k = Math.max(1, Math.min(maxZoom, (w * 0.85) / bw, (h * 0.85) / bh));
  const cx = fit.offsetX + ((x0 + x1) / 2) * fit.scale;
  const cy = fit.offsetY + ((y0 + y1) / 2) * fit.scale;
  const clamp = (v: number, size: number) => Math.min(0, Math.max(size * (1 - k), v));
  return { k, tx: clamp(w / 2 - k * cx, w), ty: clamp(h / 2 - k * cy, h) };
}
