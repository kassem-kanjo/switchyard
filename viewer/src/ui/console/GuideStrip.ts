/** Three-step guide strip with a dismiss button. DOM only; all text goes through text nodes. */

import { h } from "../dom";
import { GUIDE_STEPS, dismiss, isDismissed, stepLabel, type KV } from "./guideModel";

export class GuideStrip {
  constructor(
    private readonly host: HTMLElement,
    private readonly store?: KV,
  ) {
    if (isDismissed(this.store)) {
      this.host.hidden = true;
      return;
    }
    this.setStep(0);
  }

  setStep(i: number): void {
    const nodes: (HTMLElement | string)[] = [];

    GUIDE_STEPS.forEach((_, idx) => {
      if (idx > 0) nodes.push("→");
      const { n, text, bold } = stepLabel(idx, i);
      nodes.push(h("span", { class: bold ? "guide-step on" : "guide-step" }, `${n} ${text}`));
    });

    const close = h("button", { type: "button", class: "guide-x", "aria-label": "Dismiss guide" }, "✕");
    close.addEventListener("click", () => {
      dismiss(this.store);
      this.hide();
    });

    this.host.replaceChildren(...nodes, close);
  }

  hide(): void {
    this.host.hidden = true;
  }
}
