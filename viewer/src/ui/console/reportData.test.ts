import { describe, expect, it } from "vitest";
import { clockOf, faultSpans, parseReport, parseTable, railItems } from "./reportData";

const reports = import.meta.glob<string>("../../../public/reports/two-dead-robots-and-a-jam.md", { query: "?raw", import: "default", eager: true });
const md = Object.values(reports)[0]!;

describe("parseTable", () => {
  it("keys cells by lower-case header and skips the separator", () => {
    expect(parseTable("| A | B |\n|---|---|\n| 1 | **2** |")).toEqual([{ a: "1", b: "2" }]);
  });
  it("returns nothing for prose", () => {
    expect(parseTable("no table here")).toEqual([]);
  });
});

describe("parseReport on the shipped two-dead-robots report", () => {
  const d = parseReport(md);
  it("reads the incident window and dip from the headline eyebrow", () => {
    expect(d.incident).toEqual({ startMin: 30, endMin: 52, dipPct: 47 });
  });
  it("reads every cause with share, band and its fault window", () => {
    expect(d.causes.map((c) => c.id)).toEqual(["F0", "F2", "F1"]);
    expect(d.causes[0]).toMatchObject({ sharePct: 70, bandPct: 3, parcelMin: 4329, aloneMin: 5076, startMin: 30, endMin: 45, tied: false });
    expect(d.causes[2]!.tied).toBe(true);
  });
  it("reads the fixes with gains and the within-noise flag", () => {
    expect(d.fixes).toHaveLength(4);
    expect(d.fixes[0]).toMatchObject({ recoveredMin: 5162, bandMin: 372, recoveryPct: 83, withinNoise: false });
    expect(d.fixes[3]).toMatchObject({ recoveredMin: 118, withinNoise: true });
  });
  it("gives fixes plain short phrases", () => {
    expect(d.fixes.map((f) => f.short)).toEqual(["Reroute C289 to C269", "Clear dead robots in 1 min", "Allow 8 robots per station", "Add 10 spare robots"]);
  });
  it("builds rail rows in incident, cause, fix order", () => {
    const items = railItems(d);
    expect(items.map((i) => i.kind)).toEqual(["incident", "cause", "cause", "cause", "fix", "fix", "fix", "fix"]);
    expect(items[0]!.subtitle).toContain("00:30–00:52");
    expect(items[0]!.subtitle).toContain("110 parcels late");
    expect(items.at(-1)).toMatchObject({ tone: "muted", subtitle: "no measurable effect" });
  });
  it("makes one timeline span per fault", () => {
    expect(faultSpans(d).map((s) => [s.id, s.startMin, s.endMin])).toEqual([["F0", 30, 45], ["F2", 30, 50], ["F1", 30, 50]]);
  });
});

describe("parseReport edge cases", () => {
  it("gives no rail for a report without an incident", () => {
    expect(railItems(parseReport("# Post-mortem\n\nNo incident was found.\n"))).toEqual([]);
  });
  it("formats the clock", () => {
    expect(clockOf(90)).toBe("01:30");
  });
});
