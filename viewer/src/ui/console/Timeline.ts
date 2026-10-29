/** Timeline pane: throughput against the fault-free twin, with incident band, span bars and scrub cursor. */
import type { TimelineModel } from "./types";
import {
  PLOT_BOTTOM,
  PLOT_TOP,
  TL_H,
  TL_W,
  formatMin,
  linePoints,
  lossPolygonPoints,
  minOfX,
  spanRows,
  xOfMin,
} from "./timelineGeometry";

const SVG_NS = "http://www.w3.org/2000/svg";

export interface TimelineOptions {
  onScrub?: (min: number) => void;
  onSelect?: (id: string) => void;
}

type Attrs = Record<string, string | number>;

function svgNode<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Attrs): SVGElementTagNameMap[K] {
  const el = document.createElementNS(SVG_NS, tag) as SVGElementTagNameMap[K];
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, String(v));
  return el;
}

export class Timeline {
  private readonly svg: SVGSVGElement;
  private readonly hint: HTMLSpanElement;
  /** Labels are HTML over the svg: the svg stretches to any width, and text in it would stretch too. */
  private readonly labels: HTMLDivElement;
  private readonly onScrub: ((min: number) => void) | undefined;
  private readonly onSelect: ((id: string) => void) | undefined;
  private model: TimelineModel | null = null;
  private pressed = false;

  constructor(root: HTMLElement, opts: TimelineOptions) {
    this.onScrub = opts.onScrub;
    this.onSelect = opts.onSelect;

    const head = document.createElement("div");
    head.className = "tl-head";
    const title = document.createElement("span");
    title.className = "lbl";
    title.textContent = "Timeline · throughput vs normal";
    const hint = document.createElement("span");
    hint.className = "lbl";
    hint.textContent = Timeline.HINT;
    this.hint = hint;
    head.append(title, hint);

    this.svg = svgNode("svg", {
      viewBox: `0 0 ${TL_W} ${TL_H}`,
      preserveAspectRatio: "none",
      width: "100%",
      height: "100%",
      class: "tl-svg",
    });

    // Pointer maths happens in viewBox units, so the svg can stretch to any width.
    this.svg.addEventListener("pointerdown", (ev) => {
      if (ev.button !== 0 || !this.model) return;
      this.pressed = true;
      this.svg.setPointerCapture?.(ev.pointerId);
      this.emitScrub(ev);
    });
    this.svg.addEventListener("pointermove", (ev) => {
      if (!this.pressed || !this.model) return;
      this.emitScrub(ev);
    });
    const release = (ev: PointerEvent): void => {
      this.pressed = false;
      if (this.svg.hasPointerCapture?.(ev.pointerId)) this.svg.releasePointerCapture(ev.pointerId);
    };
    this.svg.addEventListener("pointerleave", () => this.onScrub?.(NaN));
    this.svg.addEventListener("pointerup", release);
    this.svg.addEventListener("pointercancel", release);

    this.labels = document.createElement("div");
    this.labels.className = "tl-labels";
    const plot = document.createElement("div");
    plot.className = "tl-plot";
    plot.append(this.svg, this.labels);
    root.append(head, plot);
  }

  static readonly HINT = "drag to read any minute";

  /** Text for the right of the header (the minute under the pointer); empty restores the hint. */
  setReadout(text: string): void {
    this.hint.textContent = text || Timeline.HINT;
  }

  update(model: TimelineModel): void {
    this.model = model;
    const svg = this.svg;
    svg.replaceChildren();
    this.labels.replaceChildren();

    const bandX0 = model.incident ? xOfMin(model, model.incident.startMin) : 0;
    const bandX1 = model.incident ? xOfMin(model, model.incident.endMin) : 0;

    if (model.incident) {
      svg.append(
        svgNode("rect", {
          class: "tl-band",
          x: bandX0,
          y: PLOT_TOP,
          width: Math.max(0, bandX1 - bandX0),
          height: PLOT_BOTTOM - PLOT_TOP,
          fill: "#CD4246",
          opacity: 0.12,
        }),
      );
    }

    const loss = lossPolygonPoints(model);
    if (loss) {
      svg.append(
        svgNode("polygon", {
          class: "tl-loss",
          points: loss,
          fill: "#CD4246",
          opacity: 0.3,
        }),
      );
    }

    if (model.twin && model.twin.length > 0) {
      svg.append(
        svgNode("polyline", {
          class: "tl-twin",
          points: linePoints(model, model.twin),
          fill: "none",
          stroke: "#ABB3BF",
          "stroke-width": 1.5,
          "stroke-dasharray": "5 4",
        }),
      );
    }

    if (model.actual.length > 0) {
      svg.append(
        svgNode("polyline", {
          class: "tl-actual",
          points: linePoints(model, model.actual),
          fill: "none",
          stroke: "#2D72D2",
          "stroke-width": 2,
        }),
      );
    }

    // Span strip 84..92; rows are 3px apart, so at most three 2px rows fit.
    for (const { span, x, w, row } of spanRows(model)) {
      const y = 82 + row * 3;
      if (y + 2 > 91) continue;
      const highlighted = model.highlight.includes(span.id);
      const rect = svgNode("rect", {
        class: "tl-span",
        x,
        y,
        width: w,
        height: 2,
        fill: "#CD4246",
        opacity: highlighted ? 1 : 0.55,
      });
      if (highlighted) {
        rect.setAttribute("stroke", "#F6F7F9");
        rect.setAttribute("stroke-width", "1");
      }
      rect.addEventListener("pointerdown", (ev) => {
        ev.stopPropagation(); // selecting a bar is not a scrub
        this.onSelect?.(span.id);
      });
      svg.append(rect);
    }

    // Axis ends plus the incident edges, all in the baseline strip.
    this.label(2, "start", formatMin(0));
    if (model.totalMin > 0) this.label(TL_W - 2, "end", formatMin(model.totalMin));
    if (model.incident) {
      this.label(bandX0 + 2, "start", formatMin(model.incident.startMin));
      this.label(bandX1 - 2, "end", formatMin(model.incident.endMin));
      if (model.incidentCaption) {
        this.label((bandX0 + bandX1) / 2, "middle", model.incidentCaption, "caption");
      }
    }

    if (model.cursorMin !== null) {
      const x = xOfMin(model, model.cursorMin);
      svg.append(
        svgNode("line", {
          class: "tl-cursor",
          x1: x,
          x2: x,
          y1: PLOT_TOP,
          y2: PLOT_BOTTOM,
          stroke: "#F6F7F9",
          "stroke-width": 1.5,
        }),
      );
    }
  }

  /** A mono label at svg x (viewBox units): along the bottom edge, or a coloured caption near the top. */
  private label(x: number, anchor: "start" | "middle" | "end", text: string, kind: "axis" | "caption" = "axis"): void {
    const el = document.createElement("span");
    el.className = `tl-label ${kind} ${anchor}`;
    el.style.left = `${(x / TL_W) * 100}%`;
    el.textContent = text;
    this.labels.append(el);
  }

  private emitScrub(ev: PointerEvent): void {
    if (!this.model) return;
    const rect = this.svg.getBoundingClientRect();
    // preserveAspectRatio="none" means a straight linear map from client pixels to viewBox units.
    const vbX = rect.width > 0 ? ((ev.clientX - rect.left) / rect.width) * TL_W : 0;
    this.onScrub?.(minOfX(this.model, vbX));
  }
}
