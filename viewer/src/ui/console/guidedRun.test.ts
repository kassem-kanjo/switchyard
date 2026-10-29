import { describe, expect, it } from "vitest";
import { deepestMinute, speedAt } from "./guidedRun";
import type { MetricsSeries } from "../../data/types";

const T = 120;
const series = (delivered: number[], twin: number[], startMinute = 28): MetricsSeries => ({ ticksPerMinute: T, startMinute, delivered, twin, faults: [] });

describe("speedAt", () => {
  it("fast-forwards, then 4x, then 1x at the start of the incident, then 4x", () => {
    expect(speedAt(0, 30, T)).toBe(64);
    expect(speedAt(27 * T, 30, T)).toBe(4);
    expect(speedAt(30 * T + 10, 30, T)).toBe(1);
    expect(speedAt(31 * T, 30, T)).toBe(4);
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
