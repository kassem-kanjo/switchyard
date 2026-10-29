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
  it("keeps the other report sections, collapsed, after the five", () => {
    expect(secs.length).toBeGreaterThan(5);
    expect(secs.slice(5).every((s) => !s.open)).toBe(true);
  });
});
