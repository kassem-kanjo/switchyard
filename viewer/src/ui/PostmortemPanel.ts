import type { PostmortemProvider } from "../data/types";
import { collapsible } from "./Collapsible";
import { h } from "./dom";
import { renderMarkdown } from "./markdown";
import { splitReport, type HeadlineCard } from "./reportSections";

/** The headline card: the sentence and three numbers, the proof one click away. */
function cardNode(card: HeadlineCard, openMethod: () => void): HTMLElement {
  const [cause, ...rest] = card.sentences;
  const how = h("button", { type: "button", class: "link" }, "How we know ▸");
  how.addEventListener("click", openMethod);
  return h(
    "div",
    { class: "headline-card" },
    h("p", { class: "hc-eyebrow" }, card.eyebrow),
    h(
      "div",
      { class: "hc-sentences" },
      ...(cause ? [h("p", { class: "hc-cause" }, cause)] : []),
      ...rest.map((t) => h("p", { class: "hc-fix" }, t)),
    ),
    h(
      "div",
      { class: "hc-stats" },
      ...card.stats.map((s) => h("div", { class: "hc-stat" }, h("span", { class: "hc-num" }, s.value), h("span", { class: "hc-cap" }, s.caption))),
    ),
    h("p", { class: "hc-proof" }, card.provenance, " · ", how),
  );
}

/** Lay a report out: the headline card, then collapsible sections (Summary open). Falls back to plain Markdown. */
export function renderReport(md: string, into: HTMLElement): void {
  const { card, sections } = splitReport(md);
  if (!card || sections.length === 0) {
    renderMarkdown(md, into);
    return;
  }
  const nodes: Node[] = [];
  let method: HTMLElement | null = null;
  const openMethod = () => {
    const head = method?.querySelector<HTMLButtonElement>(".collapsible-head");
    if (head && head.getAttribute("aria-expanded") !== "true") head.click();
    method?.scrollIntoView?.({ block: "start", behavior: "smooth" });
  };
  nodes.push(cardNode(card, openMethod));
  for (const sec of sections) {
    const body = h("div", { class: "report-body" });
    renderMarkdown(sec.body, body);
    const node = collapsible({ title: sec.title, teaser: sec.teaser, open: sec.open }, [body]);
    if (sec.title.startsWith("Method")) method = node;
    nodes.push(node);
  }
  into.replaceChildren(...nodes);
}

/**
 * The post-mortem tab (ADR 0006 section 8). Two sources, always labelled so
 * the reader knows which they are looking at:
 * - the precomputed report of the preset, made by the native CLI with the full
 *   replay budget (shown on load when the preset has one);
 * - a live analysis of the current run, computed in the worker for small
 *   cases (up to 4 faults). It blocks the worker, so the floor pauses meanwhile.
 * Report text is untrusted and only ever rendered through `renderMarkdown` (the card and section titles are set with textContent).
 */
export class PostmortemPanel {
  private readonly source = h("p", { class: "source note" });
  private readonly out = h("div", { class: "report" });
  private readonly button = h("button", { type: "button", class: "primary" }, "Analyse this run now");

  constructor(
    root: HTMLElement,
    private readonly provider: PostmortemProvider,
    private readonly preset: string,
    /** Called with each report shown, so the console can read its structured tables. */
    private readonly onReport: (md: string) => void = () => {},
  ) {
    this.button.addEventListener("click", () => void this.run());
    root.append(
      h("div", { class: "pm-top" }, this.button, this.source),
      this.out,
      h(
        "p",
        { class: "note" },
        "Finds the incident, replays each candidate cause with it removed, and ranks them by lost parcels recovered. " +
          "Presets with faults ship a full report; for your own run, inject up to 4 faults and analyse it here (runs up to 3 simulated hours).",
      ),
    );
    void this.showPrecomputed();
  }

  private async showPrecomputed(): Promise<void> {
    let md: string | null = null;
    try {
      md = await this.provider.precomputed(this.preset);
    } catch {
      md = null;
    }
    if (md === null) return;
    // A live analysis the reader already started wins over a late fetch.
    if (this.out.childElementCount > 0) return;
    this.source.textContent = `Precomputed for the "${this.preset}" preset (native engine, full replay budget).`;
    renderReport(md, this.out);
    this.onReport(md);
  }

  private async run(): Promise<void> {
    this.button.disabled = true;
    this.source.textContent = "Live analysis of the current run (in the browser, thin noise bands).";
    this.out.replaceChildren(h("p", { class: "note" }, "Replaying candidate causes... this can take several seconds on a long run, and the floor pauses until it finishes."));
    try {
      const md = await this.provider.report();
      renderReport(md, this.out);
      this.onReport(md);
    } catch (err) {
      this.out.replaceChildren(h("p", { class: "note error" }, `Post-mortem failed: ${String(err)}`));
    } finally {
      this.button.disabled = false;
      this.button.textContent = "Analyse again";
    }
  }
}
