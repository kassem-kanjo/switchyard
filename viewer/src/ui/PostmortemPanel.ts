import type { PostmortemProvider } from "../data/types";
import { h } from "./dom";
import { renderMarkdown } from "./markdown";

/**
 * The post-mortem tab (ADR 0006 section 8). Two sources, always labelled so
 * the reader knows which they are looking at:
 * - the precomputed report of the preset, made by the native CLI with the full
 *   replay budget (shown on load when the preset has one);
 * - a live analysis of the current run, computed in the worker for small
 *   cases (up to 4 faults). It blocks the worker, so the floor pauses meanwhile.
 * Report text is untrusted and only ever rendered through `renderMarkdown`.
 */
export class PostmortemPanel {
  private readonly source = h("p", { class: "source note" });
  private readonly out = h("div", { class: "report" });
  private readonly button = h("button", { type: "button", class: "primary" }, "Analyse this run now");

  constructor(
    root: HTMLElement,
    private readonly provider: PostmortemProvider,
    private readonly preset: string,
  ) {
    this.button.addEventListener("click", () => void this.run());
    root.append(
      h(
        "p",
        { class: "note" },
        "Finds the incident, replays each candidate cause with it removed, and ranks them by lost parcels recovered. " +
          "Presets with faults ship a full report; for your own run, inject up to 4 faults and analyse it here.",
      ),
      this.button,
      this.source,
      this.out,
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
    renderMarkdown(md, this.out);
  }

  private async run(): Promise<void> {
    this.button.disabled = true;
    this.source.textContent = "Live analysis of the current run (in the browser, thin noise bands).";
    this.out.replaceChildren(h("p", { class: "note" }, "Replaying candidate causes... the floor pauses until this finishes."));
    try {
      renderMarkdown(await this.provider.report(), this.out);
    } catch (err) {
      this.out.replaceChildren(h("p", { class: "note error" }, `Post-mortem failed: ${String(err)}`));
    } finally {
      this.button.disabled = false;
      this.button.textContent = "Analyse again";
    }
  }
}
