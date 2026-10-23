import type { PostmortemProvider } from "../data/types";
import { h } from "./dom";
import { renderMarkdown } from "./markdown";

/** Runs the post-mortem on demand and renders the Markdown report as safe DOM. */
export class PostmortemPanel {
  private readonly out = h("div", { class: "report" });
  private readonly button = h("button", { type: "button", class: "primary" }, "Run post-mortem");

  constructor(root: HTMLElement, private readonly provider: PostmortemProvider) {
    this.button.addEventListener("click", () => void this.run());
    root.append(
      h("p", { class: "note" }, "Finds the incident, replays each candidate cause with it removed, and ranks them by lost parcels recovered."),
      this.button,
      this.out,
    );
  }

  private async run(): Promise<void> {
    this.button.disabled = true;
    this.out.replaceChildren(h("p", { class: "note" }, "Replaying candidate causes..."));
    try {
      renderMarkdown(await this.provider.report(), this.out);
    } catch (err) {
      this.out.replaceChildren(h("p", { class: "note error" }, `Post-mortem failed: ${String(err)}`));
    } finally {
      this.button.disabled = false;
      this.button.textContent = "Run again";
    }
  }
}
