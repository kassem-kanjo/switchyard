import { describe, expect, it } from "vitest";
import { splitReport } from "../reportSections";
import { inspectorSections } from "./sectionPlan";

const reports = import.meta.glob<string>("../../../public/reports/two-dead-robots-and-a-jam.md", { query: "?raw", import: "default", eager: true });
const secs = inspectorSections(splitReport(Object.values(reports)[0]!).sections);

describe("inspectorSections", () => {
  it("follows the mockup order and drops the Summary", () => {
    expect(secs.slice(0, 5).map((s) => s.title)).toEqual(["Causes", "Fixes tested", "Impact", "How the causes interacted", "Method and confidence"]);
    expect(secs.some((s) => s.title === "Summary")).toBe(false);
  });
  it("opens only Causes and moves the interaction lines out of it", () => {
    expect(secs.filter((s) => s.open).map((s) => s.title)).toEqual(["Causes"]);
    expect(secs[0]!.body).not.toContain("Interaction:");
    expect(secs[3]!.teaser).toBe("2 pairs compound");
    expect(secs[3]!.body).toContain("compound");
  });
  it("shows three plain columns in Causes and moves the detail into Method", () => {
    expect(secs[0]!.body.split("\n")[0]).toBe("| Cause | Share | Parcel-min |");
    expect(secs[0]!.body).toContain("| Chute C289 jams | 70 % ± 3 | 4329 |");
    expect(secs[0]!.body).not.toMatch(/\(F\d\)|Shapley|Alone/);
    expect(secs[4]!.body).toContain("Shapley");
    expect(secs[4]!.body).toContain("Alone");
  });
  it("keeps the other report sections, collapsed, after the five", () => {
    expect(secs.length).toBeGreaterThan(5);
    expect(secs.slice(5).every((s) => !s.open)).toBe(true);
  });
});
