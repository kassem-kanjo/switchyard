import { describe, expect, it } from "vitest";
import { DIM_ALPHA, focusAlpha } from "./overlay";

describe("focusAlpha", () => {
  it("is 1 with no focus or an empty set", () => {
    expect(focusAlpha(null, 3)).toBe(1);
    expect(focusAlpha(new Set(), 3)).toBe(1);
  });
  it("dims everything outside the focus set to 25 %", () => {
    const f = new Set([2]);
    expect(focusAlpha(f, 2)).toBe(1);
    expect(focusAlpha(f, 3)).toBe(DIM_ALPHA);
    expect(DIM_ALPHA).toBe(0.25);
  });
});
