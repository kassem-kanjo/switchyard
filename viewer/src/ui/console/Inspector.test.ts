import { describe, expect, it } from "vitest";
import { causeCard, fixCard } from "./Inspector";

// The cards are built with the DOM helper, which needs a document: these tests cover the
// pure parts via the data the cards are fed (reportData.test.ts) and a smoke check of the text.
const fakeDoc = (): void => {
  const mk = (tag: string): Record<string, unknown> => {
    const kids: unknown[] = [];
    return {
      tag, kids, className: "",
      setAttribute() {},
      append(...c: unknown[]) { kids.push(...c); },
      get textContent(): string { return kids.map((k) => (typeof k === "string" ? k : (k as { textContent: string }).textContent)).join(""); },
    };
  };
  (globalThis as unknown as { document: unknown }).document = { createElement: mk };
};

describe("object cards", () => {
  fakeDoc();
  const cause = { id: "F0", label: "chute C289 jams", sharePct: 70, bandPct: 3, parcelMin: 4329, aloneMin: 5076, tied: false, startMin: 30, endMin: 45 };
  it("states the share, the window and the saved-alone number", () => {
    const text = (causeCard(cause, [cause]) as unknown as { textContent: string }).textContent;
    expect(text).toContain("Chute C289 jams");
    expect(text).toContain("From 00:30 to 00:45 (15 min).");
    expect(text).toContain("70 ± 3 % of the delay");
    expect(text).toContain("5,076");
  });
  it("says 'no measurable effect' for a fix inside the noise", () => {
    const fix = { id: "X4", label: "release 10 spare robots", recoveredMin: 118, bandMin: 446, recoveryPct: 2, withinNoise: true, cost: "10 robots" };
    expect((fixCard(fix) as unknown as { textContent: string }).textContent).toContain("No measurable effect: 118 ± 446");
  });
  it("shows the saving and cost for a real fix", () => {
    const fix = { id: "X1", label: "reroute", recoveredMin: 5162, bandMin: 372, recoveryPct: 83, withinNoise: false, cost: "sort-plan change" };
    const text = (fixCard(fix) as unknown as { textContent: string }).textContent;
    expect(text).toContain("Saves 83 %");
    expect(text).toContain("Cost: sort-plan change");
  });
});
