import { describe, expect, it } from "vitest";
import { activeParked, contrastRatio, legendEntries } from "./legend";
import { theme } from "./theme";
import { parsePreset } from "./preset";

describe("legend", () => {
  it("is generated from theme colours", () => {
    const e = legendEntries();
    expect(e.map((x) => x.colour)).toEqual(
      expect.arrayContaining([theme.chute, theme.induction, theme.robotState[0], theme.robotState[5]]),
    );
    expect(e.map((x) => x.label)).toEqual([
      "Carrying a parcel",
      "Heading to pick up",
      "Broken down",
      "Parked",
      "Chute",
      "Pick-up station",
    ]);
  });
  it("robot fills have >= 3:1 contrast against the floor", () => {
    for (const c of theme.robotState) expect(contrastRatio(c, theme.floor)).toBeGreaterThanOrEqual(3);
  });
  it("counts active vs parked", () => {
    expect(activeParked(new Uint8Array([0, 1, 3, 0, 5]))).toEqual({ active: 3, parked: 2 });
  });
});

describe("parsePreset", () => {
  it("defaults to small", () => expect(parsePreset("")).toBe("small"));
  it("accepts known presets", () => {
    expect(parsePreset("?preset=medium")).toBe("medium");
    expect(parsePreset("?preset=large")).toBe("large");
    expect(parsePreset("?preset=two-dead-robots-and-a-jam")).toBe("two-dead-robots-and-a-jam");
  });
  it("falls back with a warning", () => {
    const w: string[] = [];
    expect(parsePreset("?preset=huge", (m) => w.push(m))).toBe("small");
    expect(w).toHaveLength(1);
  });
});
