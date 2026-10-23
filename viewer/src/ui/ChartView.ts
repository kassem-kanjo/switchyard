import { theme, type Theme } from "../theme";
import type { MetricsSeries } from "../data/types";
import { formatReadout, hoverReadout, indexAt, makeScale, tickToIndex, yTicks, type Scale } from "./chartMath";

/** Canvas line chart: delivered per minute, dashed twin, shaded fault windows, hover readout. */
export class ChartView {
  private readonly ctx: CanvasRenderingContext2D;
  private series: MetricsSeries | null = null;
  private scale: Scale | null = null;
  private hover: number | null = null;
  private cssW = 0;
  private cssH = 0;

  constructor(private readonly canvas: HTMLCanvasElement, private readonly th: Theme = theme) {
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("2D canvas context unavailable");
    this.ctx = ctx;
    canvas.addEventListener("pointermove", (e) => {
      const r = canvas.getBoundingClientRect();
      this.hover = this.scale ? indexAt(e.clientX - r.left, this.scale) : null;
      this.draw();
    });
    canvas.addEventListener("pointerleave", () => {
      this.hover = null;
      this.draw();
    });
    new ResizeObserver(() => this.resize()).observe(canvas);
    this.resize();
  }

  update(series: MetricsSeries): void {
    this.series = series;
    this.draw();
  }

  private resize(): void {
    const dpr = window.devicePixelRatio || 1;
    this.cssW = this.canvas.clientWidth;
    this.cssH = this.canvas.clientHeight;
    this.canvas.width = Math.max(1, Math.round(this.cssW * dpr));
    this.canvas.height = Math.max(1, Math.round(this.cssH * dpr));
    this.draw();
  }

  private draw(): void {
    const { ctx, th, cssW: w, cssH: h } = this;
    const c = th.chart;
    const dpr = window.devicePixelRatio || 1;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const s = this.series;
    const rect = {
      left: c.margin.left,
      top: c.margin.top,
      width: Math.max(1, w - c.margin.left - c.margin.right),
      height: Math.max(1, h - c.margin.top - c.margin.bottom),
    };
    const n = s?.delivered.length ?? 0;
    const maxV = Math.max(1, ...(s?.delivered ?? []), ...(s?.twin ?? []));
    const scale = makeScale(n, maxV, rect);
    this.scale = scale;

    ctx.font = c.font;
    ctx.textBaseline = "middle";
    ctx.textAlign = "right";
    ctx.lineWidth = 1;
    for (const v of yTicks(scale.yMax, 2)) {
      const y = Math.round(scale.y(v)) + 0.5;
      ctx.strokeStyle = c.grid;
      ctx.beginPath();
      ctx.moveTo(rect.left, y);
      ctx.lineTo(rect.left + rect.width, y);
      ctx.stroke();
      ctx.fillStyle = c.axisText;
      ctx.fillText(String(Math.round(v)), rect.left - 6, y);
    }
    if (!s || n === 0) {
      ctx.textAlign = "center";
      ctx.fillStyle = c.axisText;
      ctx.fillText("Collecting the first minute of data", rect.left + rect.width / 2, rect.top + rect.height / 2);
      return;
    }

    ctx.textAlign = "left";
    ctx.textBaseline = "alphabetic";
    ctx.fillStyle = c.axisText;
    ctx.fillText(`min ${s.startMinute}`, rect.left, h - 4);
    ctx.textAlign = "right";
    ctx.fillText(`min ${s.startMinute + n - 1}`, rect.left + rect.width, h - 4);

    // Fault windows, clipped to the plot.
    ctx.save();
    ctx.beginPath();
    ctx.rect(rect.left, rect.top, rect.width, rect.height);
    ctx.clip();
    for (const f of s.faults) {
      const x0 = scale.x(tickToIndex(f.startTick, s.startMinute, s.ticksPerMinute));
      const x1 = scale.x(tickToIndex(f.endTick, s.startMinute, s.ticksPerMinute));
      if (x1 < rect.left || x0 > rect.left + rect.width) continue;
      ctx.fillStyle = c.faultShade;
      ctx.fillRect(x0, rect.top, Math.max(1, x1 - x0), rect.height);
      ctx.strokeStyle = c.faultEdge;
      ctx.beginPath();
      ctx.moveTo(x0 + 0.5, rect.top);
      ctx.lineTo(x0 + 0.5, rect.top + rect.height);
      ctx.stroke();
    }
    ctx.restore();

    const line = (vals: readonly number[], colour: string, width: number, dash: number[]) => {
      ctx.strokeStyle = colour;
      ctx.lineWidth = width;
      ctx.setLineDash(dash);
      ctx.lineJoin = "round";
      ctx.beginPath();
      vals.forEach((v, i) => (i === 0 ? ctx.moveTo(scale.x(i), scale.y(v)) : ctx.lineTo(scale.x(i), scale.y(v))));
      ctx.stroke();
      ctx.setLineDash([]);
    };
    if (s.twin) line(s.twin, c.twin, 1.5, c.twinDash);
    line(s.delivered, c.line, c.lineWidth, []);

    const hi = this.hover;
    const r = hi === null ? null : hoverReadout(hi, s.startMinute, s.delivered, s.twin);
    if (hi !== null && r) {
      const x = scale.x(hi);
      ctx.strokeStyle = c.hoverLine;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(Math.round(x) + 0.5, rect.top);
      ctx.lineTo(Math.round(x) + 0.5, rect.top + rect.height);
      ctx.stroke();
      ctx.fillStyle = c.line;
      ctx.beginPath();
      ctx.arc(x, scale.y(r.delivered), 3.5, 0, Math.PI * 2);
      ctx.fill();

      const text = formatReadout(r);
      ctx.font = c.font;
      const tw = ctx.measureText(text).width + 12;
      const tx = Math.min(Math.max(x - tw / 2, 2), w - tw - 2);
      ctx.fillStyle = c.tooltipBg;
      ctx.fillRect(tx, rect.top, tw, 20);
      ctx.fillStyle = th.ui.text;
      ctx.textAlign = "left";
      ctx.textBaseline = "middle";
      ctx.fillText(text, tx + 6, rect.top + 10);
    }
  }
}
