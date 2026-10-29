import { describe, expect, it } from "vitest";
import { deepestMinute, jumpTarget, speedAt } from "./guidedRun";
import type { MetricsSeries } from "../../data/types";

const T = 120;
const series = (delivered: number[], twin: number[], startMinute = 28): MetricsSeries => ({ ticksPerMinute: T, startMinute, delivered, twin, faults: [] });

describe("speedAt and jumpTarget", () => {
  it("jumps to just before the incident, then 4x, 1x at the jam, 16x", () => {
    expect(jumpTarget(30, T)).toBe(30 * T - 30);
    expect(speedAt(30 * T - 30, 30, T)).toBe(4);
    expect(speedAt(30 * T + 5, 30, T)).toBe(1);
    expect(speedAt(30 * T + 10, 30, T)).toBe(16);
  });
});

describe("deepestMinute", () => {
  it("is null without a twin or before the loss eases", () => {
    expect(deepestMinute({ ...series([100, 100, 90, 60, 10], [100, 100, 100, 100, 100]), twin: undefined }, 30)).toBeNull();
    expect(deepestMinute(series([100, 100, 90, 60, 10], [100, 100, 100, 100, 100]), 30)).toBeNull();
  });
  it("finds the worst finished minute once a later minute is better", () => {
    // minutes 28..33; 33 is still in progress
    expect(deepestMinute(series([100, 100, 90, 50, 70, 5], [100, 100, 100, 100, 100, 100]), 30)).toBe(31);
  });
  it("ignores minutes before the incident and a run with no loss", () => {
    expect(deepestMinute(series([50, 60, 100, 100, 100], [100, 100, 100, 100, 100]), 30)).toBeNull();
  });
});
