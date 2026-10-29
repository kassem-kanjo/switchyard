import { describe, expect, it } from "vitest";
import { jumpTarget, reachedDip, speedAt } from "./guidedRun";
import type { MetricsSeries } from "../../data/types";

const T = 120;
const series = (delivered: number[], twin: number[], startMinute = 28): MetricsSeries => ({ ticksPerMinute: T, startMinute, delivered, twin, faults: [] });

describe("speedAt and jumpTarget", () => {
  it("jumps to just before the incident, then 4x, 1x at the jam, 64x", () => {
    expect(jumpTarget(30, T)).toBe(30 * T - 30);
    expect(speedAt(30 * T - 30, 30, T)).toBe(4);
    expect(speedAt(30 * T + 5, 30, T)).toBe(1);
    expect(speedAt(30 * T + 10, 30, T)).toBe(64);
  });
});

describe("reachedDip", () => {
  it("is false without a twin and until the latest finished minute is as deep as the report's dip", () => {
    expect(reachedDip({ ...series([100, 90, 5], [100, 100, 100]), twin: undefined }, 47)).toBe(false);
    expect(reachedDip(series([100, 71, 5], [100, 100, 100]), 47)).toBe(false);
    expect(reachedDip(series([100, 53, 5], [100, 100, 100]), 47)).toBe(true);
    expect(reachedDip(series([100, 40, 5], [100, 100, 100]), 47)).toBe(true);
  });
});
