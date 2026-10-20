import { describe, expect, it } from "vitest";
import { fitToCanvas } from "./fit";

describe("fitToCanvas", () => {
  it("is limited by width and centres vertically", () => {
    const t = fitToCanvas(10, 5, 200, 200);
    expect(t.scale).toBe(20);
    expect(t.offsetX).toBe(0);
    expect(t.offsetY).toBe(50);
  });

  it("is limited by height and centres horizontally", () => {
    const t = fitToCanvas(10, 10, 300, 100);
    expect(t.scale).toBe(10);
    expect(t.offsetX).toBe(100);
    expect(t.offsetY).toBe(0);
  });

  it("respects padding", () => {
    // available 96 x 36 -> scale min(12, 9) = 9
    const t = fitToCanvas(8, 4, 116, 56, 10);
    expect(t.scale).toBe(9);
    expect(t.offsetX).toBe((116 - 72) / 2);
    expect(t.offsetY).toBe(10);
  });

  it("handles degenerate sizes", () => {
    expect(fitToCanvas(10, 10, 5, 5, 10).scale).toBe(0);
    expect(fitToCanvas(0, 0, 100, 100).scale).toBe(0);
  });
});
