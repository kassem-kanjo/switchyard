import { theme, type Theme } from "../theme";
import { CellKind, type LayoutView, type Renderer, type SnapshotView } from "./Renderer";
import { fitToCanvas, type FitTransform } from "./fit";

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

  constructor(private readonly th: Theme = theme) {}

  init(canvas: HTMLCanvasElement, layout: LayoutView): void {
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("2D canvas context unavailable");
    this.canvas = canvas;
    this.ctx = ctx;
    this.layout = layout;
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

  draw(snap: SnapshotView): void {
    this.last = snap;
    const { ctx, th, fit, dpr } = this;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = th.background;
    ctx.fillRect(0, 0, this.cssW, this.cssH);
    if (this.grid) ctx.drawImage(this.grid, 0, 0, this.cssW, this.cssH);
    const size = fit.scale * th.robotScale;
    const inset = (fit.scale - size) / 2;
    const n = snap.robotState.length;
    for (let i = 0; i < n; i++) {
      const x = fit.offsetX + snap.robotXY[2 * i]! * fit.scale + inset;
      const y = fit.offsetY + snap.robotXY[2 * i + 1]! * fit.scale + inset;
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
