import type { Theme } from "../theme";
import { goalOf, type RobotLabel } from "./labels";
import { RobotState, type SnapshotView } from "./Renderer";
import type { FitTransform } from "./fit";

/** Opacity of everything outside the focus set. */
export const DIM_ALPHA = 0.25;

/** 1 when nothing is focused or `i` is in the set, else DIM_ALPHA. */
export function focusAlpha(focus: ReadonlySet<number> | null, i: number): number {
  return focus === null || focus.size === 0 || focus.has(i) ? 1 : DIM_ALPHA;
}

const carrying = (s: number): boolean => s === RobotState.ToChute || s === RobotState.Dropping;

/**
 * Faint dashed line from each carrying robot to its destination chute.
 * `centre(i)` gives the robot's (interpolated) centre in canvas pixels.
 */
export function drawDestinationLines(
  ctx: CanvasRenderingContext2D,
  snap: SnapshotView,
  fit: FitTransform,
  th: Theme,
  centre: (i: number) => { x: number; y: number },
  focus: ReadonlySet<number> | null,
): void {
  ctx.save();
  ctx.setLineDash([4, 4]);
  ctx.lineWidth = 1;
  ctx.strokeStyle = th.robotState[RobotState.ToChute] ?? "#fff";
  for (let i = 0; i < snap.robotState.length; i++) {
    if (snap.robotState[i] !== RobotState.ToChute) continue;
    const g = goalOf(snap, i);
    if (!g) continue;
    const c = centre(i);
    ctx.globalAlpha = 0.28 * focusAlpha(focus, i);
    ctx.beginPath();
    ctx.moveTo(c.x, c.y);
    ctx.lineTo(fit.offsetX + (g.x + 0.5) * fit.scale, fit.offsetY + (g.y + 0.5) * fit.scale);
    ctx.stroke();
  }
  ctx.restore();
}

/** The parcel: a dot in the chute colour inside every robot that carries one. */
export function drawParcelDots(
  ctx: CanvasRenderingContext2D,
  snap: SnapshotView,
  fit: FitTransform,
  th: Theme,
  centre: (i: number) => { x: number; y: number },
  focus: ReadonlySet<number> | null,
): void {
  ctx.save();
  ctx.fillStyle = th.chute;
  for (let i = 0; i < snap.robotState.length; i++) {
    if (!carrying(snap.robotState[i]!)) continue;
    const c = centre(i);
    ctx.globalAlpha = focusAlpha(focus, i);
    ctx.beginPath();
    ctx.arc(c.x, c.y, Math.max(1.5, fit.scale * th.robotScale * 0.2), 0, Math.PI * 2);
    ctx.fill();
  }
  ctx.restore();
}

/** Hover card next to a robot: id, what it is doing, where it is going, how long it waited. */
export function drawHoverCard(
  ctx: CanvasRenderingContext2D,
  label: RobotLabel,
  anchor: { x: number; y: number },
  bounds: { w: number; h: number },
  th: Theme,
): void {
  const lines = [label.title, label.doing, ...(label.going ? [`Going to ${label.going}`] : []), ...(label.waiting ? [label.waiting] : [])];
  const fs = th.ui.fsCaption;
  const pad = 6;
  ctx.save();
  ctx.font = `${fs}px ${th.ui.font}`;
  const w = Math.max(...lines.map((l) => ctx.measureText(l).width)) + 2 * pad;
  const h = lines.length * (fs + 3) + 2 * pad - 3;
  let x = anchor.x + 14;
  let y = anchor.y - h - 8;
  if (x + w > bounds.w) x = anchor.x - w - 14;
  if (y < 0) y = anchor.y + 14;
  x = Math.max(0, x);
  ctx.fillStyle = th.ui.panelRaised;
  ctx.strokeStyle = th.ui.borderStrong;
  ctx.lineWidth = 1;
  ctx.fillRect(x, y, w, h);
  ctx.strokeRect(x + 0.5, y + 0.5, w - 1, h - 1);
  ctx.textBaseline = "top";
  lines.forEach((l, k) => {
    ctx.fillStyle = k === 0 ? th.ui.text : th.ui.textMuted;
    ctx.font = `${k === 0 ? "600 " : ""}${fs}px ${th.ui.font}`;
    ctx.fillText(l, x + pad, y + pad + k * (fs + 3));
  });
  ctx.restore();
}
