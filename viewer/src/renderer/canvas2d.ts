import { theme, type Theme } from "../theme";
import { hatchSegments, ringPhase, ringRadius } from "./marks";
import { CellKind, RobotState, type FloorMarks, type LayoutView, type Renderer, type SnapshotView } from "./Renderer";
import { fitToCanvas, type FitTransform } from "./fit";
import { MotionTracker } from "./motion";
import { describeRobot } from "./labels";
import { drawDestinationLines, drawHoverCard, drawParcelDots, focusAlpha } from "./overlay";

export class Canvas2DRenderer implements Renderer {
  private canvas!: HTMLCanvasElement;
  private ctx!: CanvasRenderingContext2D;
  private layout!: LayoutView;
  private grid: HTMLCanvasElement | null = null;
  private fit: FitTransform = { scale: 0, offsetX: 0, offsetY: 0 };
  private cssW = 0;
  private cssH = 0;
  private dpr = 1;
  private highlighted = new Set<number>();
  private last: SnapshotView | null = null;
  private marks: FloorMarks | null = null;
  private reducedMotion = false;
  private readonly motion = new MotionTracker();
  private focus: ReadonlySet<number> | null = null;
  private hover: number | null = null;

  constructor(private readonly th: Theme = theme) {}

  init(canvas: HTMLCanvasElement, layout: LayoutView): void {
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("2D canvas context unavailable");
    this.canvas = canvas;
    this.ctx = ctx;
    this.layout = layout;
    if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
      this.reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    }
    this.resize(canvas.clientWidth, canvas.clientHeight);
  }

  resize(w: number, h: number): void {
    this.cssW = w;
    this.cssH = h;
    this.dpr = window.devicePixelRatio || 1;
    this.canvas.width = Math.max(1, Math.round(w * this.dpr));
    this.canvas.height = Math.max(1, Math.round(h * this.dpr));
    this.fit = fitToCanvas(this.layout.width, this.layout.height, w, h, this.th.padding);
    this.buildGrid();
    if (this.last) this.draw(this.last);
  }

  highlight(robotIds: number[]): void {
    this.highlighted = new Set(robotIds);
  }

  /** Dim every robot not in `ids` to 25 %; an empty list clears focus. */
  setFocus(ids: number[]): void {
    this.focus = ids.length === 0 ? null : new Set(ids);
  }

  pickAt(px: number, py: number): number | null {
    const snap = this.last;
    if (!snap || this.fit.scale <= 0) return null;
    const now = performance.now();
    const half = (this.fit.scale * this.th.robotScale) / 2 + 2;
    let best: number | null = null;
    let bestD = Infinity;
    for (let i = 0; i < snap.robotState.length; i++) {
      const p = this.motion.position(i, now, this.reducedMotion);
      const dx = Math.abs(this.fit.offsetX + (p.x + 0.5) * this.fit.scale - px);
      const dy = Math.abs(this.fit.offsetY + (p.y + 0.5) * this.fit.scale - py);
      if (dx <= half && dy <= half && dx + dy < bestD) {
        best = i;
        bestD = dx + dy;
      }
    }
    return best;
  }

  setHover(id: number | null): void {
    this.hover = id;
  }

  setMarks(marks: FloorMarks): void {
    this.marks = marks;
  }

  setReducedMotion(on: boolean): void {
    this.reducedMotion = on;
  }

  draw(snap: SnapshotView): void {
    this.last = snap;
    const now = performance.now();
    this.motion.update(snap.tick, snap.robotXY, now);
    const { ctx, th, fit, dpr, layout } = this;
    const centre = (i: number) => {
      const p = this.motion.position(i, now, this.reducedMotion);
      return { x: fit.offsetX + (p.x + 0.5) * fit.scale, y: fit.offsetY + (p.y + 0.5) * fit.scale };
    };
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = th.background;
    ctx.fillRect(0, 0, this.cssW, this.cssH);
    if (this.grid) ctx.drawImage(this.grid, 0, 0, this.cssW, this.cssH);

    const marks = this.marks;
    if (marks) {
      for (const cell of marks.jammedCells) {
        if (!Number.isInteger(cell) || cell < 0 || cell >= layout.width * layout.height) continue;
        const x = fit.offsetX + (cell % layout.width) * fit.scale;
        const y = fit.offsetY + Math.floor(cell / layout.width) * fit.scale;
        ctx.globalAlpha = 0.25;
        ctx.fillStyle = th.ui.warn;
        ctx.fillRect(x, y, fit.scale, fit.scale);
        ctx.globalAlpha = 1;
        ctx.strokeStyle = th.ui.warn;
        ctx.lineWidth = 1;
        ctx.beginPath();
        for (const [x1, y1, x2, y2] of hatchSegments(x, y, fit.scale, 4)) {
          ctx.moveTo(x1, y1);
          ctx.lineTo(x2, y2);
        }
        ctx.stroke();
      }
      ctx.globalAlpha = 1;
    }

    // Faint trails behind moving robots, fading towards the oldest cell.
    if (!this.reducedMotion) {
      ctx.lineCap = "round";
      ctx.lineWidth = Math.max(1, fit.scale * 0.18);
      for (let i = 0; i < snap.robotState.length; i++) {
        if (snap.robotState[i] === RobotState.Idle) continue;
        const trail = this.motion.trail(i);
        if (trail.length === 0) continue;
        const pos = this.motion.position(i, now);
        const points: Array<{ x: number; y: number }> = trail.map((c) => ({
          x: fit.offsetX + (c.x + 0.5) * fit.scale,
          y: fit.offsetY + (c.y + 0.5) * fit.scale,
        }));
        points.push({ x: fit.offsetX + (pos.x + 0.5) * fit.scale, y: fit.offsetY + (pos.y + 0.5) * fit.scale });
        ctx.strokeStyle = th.robotState[snap.robotState[i]!] ?? th.robotState[0];
        const segments = points.length - 1;
        for (let s = 0; s < segments; s++) {
          const a = points[s]!;
          const b = points[s + 1]!;
          ctx.globalAlpha = 0.22 * ((s + 1) / segments) * focusAlpha(this.focus, i);
          ctx.beginPath();
          ctx.moveTo(a.x, a.y);
          ctx.lineTo(b.x, b.y);
          ctx.stroke();
        }
      }
      ctx.globalAlpha = 1;
      ctx.lineCap = "butt";
    }

    const lineFor = new Set<number>(this.focus ?? []);
    if (this.hover !== null) lineFor.add(this.hover);
    drawDestinationLines(ctx, snap, fit, th, centre, lineFor);

    const size = fit.scale * th.robotScale;
    const inset = (fit.scale - size) / 2;
    const n = snap.robotState.length;
    for (let i = 0; i < n; i++) {
      const pos = this.motion.position(i, now, this.reducedMotion);
      const x = fit.offsetX + pos.x * fit.scale + inset;
      const y = fit.offsetY + pos.y * fit.scale + inset;
      ctx.globalAlpha = focusAlpha(this.focus, i);
      ctx.fillStyle = th.robotState[snap.robotState[i]!] ?? th.robotState[0];
      ctx.fillRect(x, y, size, size);
      if (snap.robotState[i] === 0) {
        ctx.strokeStyle = th.idleOutline;
        ctx.lineWidth = 1.5;
        ctx.strokeRect(x - 0.75, y - 0.75, size + 1.5, size + 1.5);
      }
      if (this.highlighted.has(i)) {
        ctx.strokeStyle = th.highlight;
        ctx.lineWidth = 2;
        ctx.strokeRect(x - 1, y - 1, size + 2, size + 2);
      }
    }

    ctx.globalAlpha = 1;
    drawParcelDots(ctx, snap, fit, th, centre, this.focus);

    const { r, alpha } = ringRadius(fit.scale, ringPhase(now), this.reducedMotion);
    for (let i = 0; i < n; i++) {
      if (snap.robotState[i] !== RobotState.Failed) continue;
      const pos = this.motion.position(i, now, this.reducedMotion);
      const cx = fit.offsetX + pos.x * fit.scale + fit.scale / 2;
      const cy = fit.offsetY + pos.y * fit.scale + fit.scale / 2;
      ctx.globalAlpha = alpha * focusAlpha(this.focus, i);
      ctx.strokeStyle = th.ui.danger;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.arc(cx, cy, r, 0, Math.PI * 2);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;

    if (this.hover !== null && this.hover < n) {
      drawHoverCard(ctx, describeRobot(snap, layout, this.hover), centre(this.hover), { w: this.cssW, h: this.cssH }, th);
    }
  }

  /** Static floor, cached on an offscreen canvas at device resolution. */
  private buildGrid(): void {
    const g = document.createElement("canvas");
    g.width = this.canvas.width;
    g.height = this.canvas.height;
    const c = g.getContext("2d");
    if (!c) return;
    const { th, fit, layout, dpr } = this;
    c.setTransform(dpr, 0, 0, dpr, 0, 0);
    const kindColour = [th.floor, th.wall, th.chute, th.induction];
    for (let y = 0; y < layout.height; y++) {
      for (let x = 0; x < layout.width; x++) {
        c.fillStyle = kindColour[layout.cells[y * layout.width + x] ?? CellKind.Free] ?? th.floor;
        c.fillRect(fit.offsetX + x * fit.scale, fit.offsetY + y * fit.scale, fit.scale, fit.scale);
      }
    }
    c.strokeStyle = th.gridLine;
    c.lineWidth = 1 / dpr;
    c.beginPath();
    for (let x = 0; x <= layout.width; x++) {
      c.moveTo(fit.offsetX + x * fit.scale, fit.offsetY);
      c.lineTo(fit.offsetX + x * fit.scale, fit.offsetY + layout.height * fit.scale);
    }
    for (let y = 0; y <= layout.height; y++) {
      c.moveTo(fit.offsetX, fit.offsetY + y * fit.scale);
      c.lineTo(fit.offsetX + layout.width * fit.scale, fit.offsetY + y * fit.scale);
    }
    c.stroke();
    this.grid = g;
  }
}
