import { theme, type Theme } from "../theme";
import type { MetricsSeries } from "../data/types";
import { deepestDip, formatReadout, hoverReadout, indexAt, makeScale, noiseHalfWidth, tickToIndex, yTicks, type Scale } from "./chartMath";

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
    // Axis labels at tidy ticks; hairlines only at 0 and the twin's mean (design/v2.md, chart).
    const twinMean = s?.twin && s.twin.length ? s.twin.reduce((a, b) => a + b, 0) / s.twin.length : null;
    for (const v of yTicks(scale.yMax, 2)) {
      ctx.fillStyle = c.axisText;
      ctx.fillText(String(Math.round(v)), rect.left - 6, Math.round(scale.y(v)));
    }
    const hair = (v: number) => {
      const y = Math.round(scale.y(v)) + 0.5;
      ctx.strokeStyle = c.grid;
      ctx.beginPath();
      ctx.moveTo(rect.left, y);
      ctx.lineTo(rect.left + rect.width, y);
      ctx.stroke();
    };
    hair(0);
    if (twinMean !== null) hair(twinMean);
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

    const idx = (tick: number) => tickToIndex(tick, s.startMinute, s.ticksPerMinute);
    const firstFault = s.faults.length ? Math.min(...s.faults.map((f) => idx(f.startTick))) : n;
    const clipPlot = () => {
      ctx.beginPath();
      ctx.rect(rect.left, rect.top, rect.width, rect.height);
      ctx.clip();
    };
    // The twin's noise band: where the real line may sit by chance alone.
    if (s.twin && s.twin.length > 1) {
      const hw = noiseHalfWidth(s.delivered, s.twin, firstFault);
      ctx.save();
      clipPlot();
      ctx.fillStyle = c.noiseShade;
      ctx.beginPath();
      s.twin.forEach((v, i) => (i === 0 ? ctx.moveTo(scale.x(i), scale.y(v + hw)) : ctx.lineTo(scale.x(i), scale.y(v + hw))));
      for (let i = s.twin.length - 1; i >= 0; i--) ctx.lineTo(scale.x(i), scale.y(Math.max(0, s.twin[i]! - hw)));
      ctx.closePath();
      ctx.fill();
      ctx.restore();
    }

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
      // Loss: the area between the twin and the actual line inside the window.
      if (s.twin) {
        const i0 = Math.max(0, Math.ceil(idx(f.startTick)));
        const i1 = Math.min(n - 1, Math.floor(idx(f.endTick)));
        if (i1 > i0) {
          ctx.fillStyle = c.lossShade;
          ctx.beginPath();
          for (let i = i0; i <= i1; i++) {
            const y = scale.y(Math.max(s.twin[i] ?? 0, s.delivered[i] ?? 0));
            i === i0 ? ctx.moveTo(scale.x(i), y) : ctx.lineTo(scale.x(i), y);
          }
          for (let i = i1; i >= i0; i--) ctx.lineTo(scale.x(i), scale.y(s.delivered[i] ?? 0));
          ctx.closePath();
          ctx.fill();
        }
      }
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

    // Annotate the deepest point inside the incident window.
    if (s.twin && s.faults.length) {
      const lo = Math.min(...s.faults.map((f) => idx(f.startTick)));
      const hi = Math.max(...s.faults.map((f) => idx(f.endTick)));
      const dip = deepestDip(s.delivered, s.twin, lo, hi);
      if (dip) {
        const px = scale.x(dip.index);
        const py = scale.y(s.delivered[dip.index]!);
        ctx.fillStyle = c.annotation;
        ctx.beginPath();
        ctx.arc(px, py, 3, 0, Math.PI * 2);
        ctx.fill();
        const label = `−${dip.pct} %`;
        ctx.font = `600 ${c.font}`;
        const lw = ctx.measureText(label).width;
        ctx.textAlign = "left";
        ctx.textBaseline = "alphabetic";
        const lx = Math.min(px + 7, rect.left + rect.width - lw);
        ctx.fillText(label, lx, Math.min(py - 6, rect.top + rect.height - 14));
        ctx.font = c.font;
      }
    }

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
