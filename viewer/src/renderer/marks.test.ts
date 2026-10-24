import { describe, expect, it } from "vitest";
import { glowAlpha, hatchSegments, ringPhase, ringRadius } from "./marks";

describe("ringPhase", () => {
  it("is 0 at t=0", () => {
    expect(ringPhase(0)).toBeCloseTo(0, 10);
  });

  it("is 1 at half a period", () => {
    expect(ringPhase(600)).toBeCloseTo(1, 10);
    expect(ringPhase(1200, 2400)).toBeCloseTo(1, 10);
  });

  it("stays within 0..1 across a period", () => {
    for (let t = 0; t <= 1200; t += 25) {
      const p = ringPhase(t);
      expect(p).toBeGreaterThanOrEqual(0);
      expect(p).toBeLessThanOrEqual(1);
    }
  });
});

describe("ringRadius", () => {
  it("pulses with phase when motion is allowed", () => {
    const start = ringRadius(20, 0, false);
    const mid = ringRadius(20, 1, false);
    expect(start.r).toBeCloseTo(11, 10);
    expect(mid.r).toBeCloseTo(16, 10);
    expect(start.alpha).toBeCloseTo(0.9, 10);
    expect(mid.alpha).toBeCloseTo(0.3, 10);
  });

  it("is static when reducedMotion is true", () => {
    const a = ringRadius(20, 0, true);
    const b = ringRadius(20, 0.75, true);
    expect(a).toEqual(b);
    expect(a.r).toBeCloseTo(13, 10);
    expect(a.alpha).toBeCloseTo(0.9, 10);
  });
});

describe("hatchSegments", () => {
  it("fills the square with segments fully inside it", () => {
    const x = 10;
    const y = 20;
    const size = 40;
    const segments = hatchSegments(x, y, size, 4);
    expect(segments.length).toBeGreaterThan(0);
    for (const [x1, y1, x2, y2] of segments) {
      for (const v of [x1, y1, x2, y2]) expect(Number.isFinite(v)).toBe(true);
      expect(x1).toBeGreaterThanOrEqual(x - 1e-9);
      expect(x1).toBeLessThanOrEqual(x + size + 1e-9);
      expect(x2).toBeGreaterThanOrEqual(x - 1e-9);
      expect(x2).toBeLessThanOrEqual(x + size + 1e-9);
      expect(y1).toBeGreaterThanOrEqual(y - 1e-9);
      expect(y1).toBeLessThanOrEqual(y + size + 1e-9);
      expect(y2).toBeGreaterThanOrEqual(y - 1e-9);
      expect(y2).toBeLessThanOrEqual(y + size + 1e-9);
    }
  });

  it("produces 45-degree segments", () => {
    const segments = hatchSegments(0, 0, 10, 3);
    for (const [x1, y1, x2, y2] of segments) {
      expect(Math.abs(x2 - x1)).toBeCloseTo(Math.abs(y2 - y1), 10);
    }
  });

  it("returns nothing for degenerate sizes", () => {
    expect(hatchSegments(0, 0, 0, 4)).toEqual([]);
    expect(hatchSegments(0, 0, 10, 0)).toEqual([]);
  });
});

describe("glowAlpha", () => {
  it("scales intensity and clamps to 0..1", () => {
    expect(glowAlpha(-1)).toBe(0);
    expect(glowAlpha(0)).toBe(0);
    expect(glowAlpha(0.5)).toBeCloseTo(0.175, 10);
    expect(glowAlpha(2)).toBeCloseTo(0.35, 10);
  });

  it("honours a custom maximum", () => {
    expect(glowAlpha(0.5, 0.4)).toBeCloseTo(0.2, 10);
    expect(glowAlpha(5, 0.4)).toBeCloseTo(0.4, 10);
  });
});
