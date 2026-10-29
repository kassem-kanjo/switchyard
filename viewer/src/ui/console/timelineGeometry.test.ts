import { describe, expect, it } from "vitest";
import {
  TL_H,
  TL_W,
  formatMin,
  linePoints,
  lossPolygonPoints,
  minOfX,
  spanRows,
  xOfMin,
  yOfValue,
} from "./timelineGeometry";
import type { TimelineModel } from "./types";

const base: TimelineModel = {
  totalMin: 100,
  startMin: 0,
  actual: [10, 20, 15],
  incident: null,
  spans: [],
  highlight: [],
  cursorMin: null,
  incidentCaption: "",
};

const model = (over: Partial<TimelineModel> = {}): TimelineModel => ({ ...base, ...over });
const r2 = (v: number): number => Math.round(v * 100) / 100;

describe("timeline x scale", () => {
  it("round trips minutes through pixels", () => {
    const m = model({ totalMin: 100 });
    expect(xOfMin(m, 25)).toBe(350);
    expect(minOfX(m, 350)).toBe(25);
    expect(minOfX(m, xOfMin(m, 42))).toBeCloseTo(42);
  });

  it("clamps to the shift edges", () => {
    const m = model({ totalMin: 100 });
    expect(xOfMin(m, -10)).toBe(0);
    expect(xOfMin(m, 999)).toBe(TL_W);
    expect(minOfX(m, -50)).toBe(0);
    expect(minOfX(m, TL_W + 50)).toBe(100);
    expect(xOfMin(model({ totalMin: 0 }), 20)).toBe(0);
    expect(minOfX(model({ totalMin: 0 }), 20)).toBe(0);
  });

  it("keeps the viewBox height constant", () => {
    expect(TL_H).toBe(100);
  });
});

describe("timeline y scale", () => {
  it("maps zero to the baseline and the tallest value to the top", () => {
    const m = model({ actual: [10, 20, 15], twin: [12, 18] });
    expect(yOfValue(m, 0)).toBe(82);
    expect(yOfValue(m, 20)).toBe(10);
    expect(yOfValue(m, 10)).toBe(46);
    expect(yOfValue(m, 999)).toBe(10); // clamped to the plot
  });

  it("uses the taller of actual and twin", () => {
    const m = model({ actual: [10], twin: [30] });
    expect(yOfValue(m, 30)).toBe(10);
  });

  it("falls back to a max of 1 when both series are empty", () => {
    const m = model({ actual: [], twin: undefined });
    expect(yOfValue(m, 0)).toBe(82);
    expect(yOfValue(m, 1)).toBe(10);
  });
});

describe("timeline polylines", () => {
  it("places point i at minute startMin + i", () => {
    const m = model({ totalMin: 1400, startMin: 30, actual: [10, 20, 15], twin: [12, 30] });
    const parts = linePoints(m, [10, 20, 15]).split(" ");
    expect(parts).toHaveLength(3);
    expect(parts[0]).toBe(`30,${r2(yOfValue(m, 10))}`);
    expect(parts[2]).toBe(`32,${r2(yOfValue(m, 15))}`);
  });

  it("returns an empty string for an empty series", () => {
    expect(linePoints(model(), [])).toBe("");
  });
});

describe("timeline loss polygon", () => {
  it("is empty without a twin", () => {
    const m = model({ incident: { startMin: 1, endMin: 2 } });
    expect(lossPolygonPoints(m)).toBe("");
  });

  it("is empty outside an incident", () => {
    expect(lossPolygonPoints(model({ twin: [12, 18, 17] }))).toBe("");
    expect(lossPolygonPoints(model({ twin: [12, 18, 17], incident: { startMin: 8, endMin: 9 } }))).toBe("");
  });

  it("walks the twin forward and actual back inside the incident", () => {
    const m = model({ twin: [12, 18, 17], incident: { startMin: 1, endMin: 2 } });
    const pts = lossPolygonPoints(m).split(" ");
    expect(pts).toHaveLength(4); // two minutes forward, two back
    // The first twin point and the last actual point are the same minute.
    expect(pts[0]?.split(",")[0]).toBe(pts[3]?.split(",")[0]);
  });
});

describe("timeline span rows", () => {
  it("stacks overlapping spans on different rows", () => {
    const m = model({
      totalMin: 100,
      spans: [
        { id: "a", label: "a", startMin: 10, endMin: 30 },
        { id: "b", label: "b", startMin: 20, endMin: 40 },
      ],
    });
    expect(spanRows(m).map((r) => r.row)).toEqual([0, 1]);
  });

  it("reuses row 0 for disjoint spans", () => {
    const m = model({
      spans: [
        { id: "a", label: "a", startMin: 0, endMin: 5 },
        { id: "b", label: "b", startMin: 10, endMin: 15 },
      ],
    });
    expect(spanRows(m).map((r) => r.row)).toEqual([0, 0]);
  });

  it("maps minutes to x and keeps bars at least 3px wide", () => {
    const m = model({
      totalMin: 100,
      spans: [
        { id: "a", label: "a", startMin: 50, endMin: 60 },
        { id: "b", label: "b", startMin: 0, endMin: 0 },
      ],
    });
    const rows = spanRows(m);
    const wide = rows.find((r) => r.span.id === "a");
    const tiny = rows.find((r) => r.span.id === "b");
    expect(wide?.x).toBe(700);
    expect(wide?.w).toBe(140);
    expect(tiny?.w).toBe(3);
  });
});

describe("formatMin", () => {
  it("formats whole minutes as H:MM", () => {
    expect(formatMin(0)).toBe("00:00");
    expect(formatMin(30)).toBe("00:30");
    expect(formatMin(90)).toBe("01:30");
    expect(formatMin(600)).toBe("10:00");
  });
});
