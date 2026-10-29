import { h } from "../dom";
import { capitalise, clockOf, type Cause, type ConsoleData, type Fix } from "./reportData";
import type { ObjectId } from "./types";

const kpi = (value: string, caption: string): HTMLElement => h("div", { class: "kpi" }, h("div", { class: "v" }, value), h("div", { class: "k" }, caption));
const fmt = (n: number): string => n.toLocaleString("en-US");
const range = (n: number, band: number): string => (band > 0 ? `${fmt(n)} ± ${fmt(band)}` : fmt(n));

/** The card for one cause: what happened, how much of the delay it explains, what removing it alone would give. */
export function causeCard(c: Cause, all: readonly Cause[]): HTMLElement {
  const when =
    c.startMin !== null && c.endMin !== null
      ? ` From ${clockOf(c.startMin)} to ${clockOf(c.endMin)} (${c.endMin - c.startMin} min).`
      : "";
  const i = all.indexOf(c);
  const tie = c.tied && i > 0 ? ` Tied with ${all[i - 1]!.id}: too close to rank.` : "";
  return h(
    "div",
    { class: "card" },
    h("div", { class: "head" }, h("b", {}, capitalise(c.label))),
    h("div", { class: "sub" }, `It explains ${range(c.sharePct, c.bandPct)} % of the delay: ${fmt(c.parcelMin)} parcel-minutes.${when}${tie}`),
    h("div", { class: "kpis" }, kpi(`${c.sharePct} %`, "share of the delay"), kpi(fmt(c.aloneMin), "parcel-min saved if removed alone")),
    h("div", { class: "how" }, "Share = a fair split of the loss across causes, from re-running the shift with each combination removed."),
  );
}

/** The card for one tested fix: the gain with its error bar, or an honest "no effect" when inside the noise. */
export function fixCard(f: Fix): HTMLElement {
  const sub = f.withinNoise
    ? h("div", { class: "sub" }, `No measurable effect: ${range(f.recoveredMin, f.bandMin)} parcel-minutes is inside the noise.`)
    : h("div", { class: "sub" }, h("b", {}, `Saves ${f.recoveryPct} %`), ` of the delay (${range(f.recoveredMin, f.bandMin)} parcel-minutes).${f.cost ? ` Cost: ${f.cost}.` : ""}`);
  return h(
    "div",
    { class: "card" },
    h("div", { class: "head" }, capitalise(f.label)),
    sub,
    ...(f.withinNoise ? [] : [h("div", { class: "kpis" }, kpi(`${f.recoveryPct} %`, "of the delay recoverable"))]),
    h("div", { class: "how" }, "Tested by applying the fix to the real shift and replaying it."),
  );
}

/**
 * The selection pane. The incident page is hosted by the post-mortem panel
 * (headline card, sections, live analysis); a cause or fix gets its own card.
 */
export class Inspector {
  private readonly title = h("span", { class: "lbl" }, "Inspector");
  private readonly meta = h("span", { class: "mono ih-meta" });
  private readonly cardHost = h("div", { class: "obj-card" });

  constructor(
    root: HTMLElement,
    private readonly incidentHost: HTMLElement,
  ) {
    root.append(h("div", { class: "ih" }, this.title, this.meta), this.incidentHost, this.cardHost);
    this.show("inc", null);
  }

  /** Show `id`. With no data (no precomputed report) the incident page, with its analyse button, is all there is. */
  show(id: ObjectId | null, data: ConsoleData | null): void {
    const cause = data?.causes.find((c) => c.id === id);
    const fix = data?.fixes.find((f) => f.id === id);
    const incident = !cause && !fix;
    this.incidentHost.hidden = !incident;
    this.cardHost.hidden = incident;
    this.title.textContent = cause ? `Inspector · Cause ${cause.id}` : fix ? `Inspector · Fix ${fix.id}` : "Inspector · Incident";
    this.meta.textContent = incident && data?.incident ? `${clockOf(data.incident.startMin)}–${clockOf(data.incident.endMin)}` : "";
    this.cardHost.replaceChildren(...(cause ? [causeCard(cause, data!.causes)] : fix ? [fixCard(fix)] : []));
  }
}
