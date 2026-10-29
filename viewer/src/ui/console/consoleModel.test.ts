import { describe, expect, it } from "vitest";
import type { MetricsSeries } from "../../data/types";
import { axisEnd, consoleTimeline, dipVsTwin, highlightFor, shiftClock, spansFor } from "./consoleModel";
import { focusRect } from "./floorFocus";
import type { ConsoleData } from "./reportData";

const series = (over: Partial<MetricsSeries> = {}): MetricsSeries => ({
  ticksPerMinute: 120, startMinute: 0, delivered: [100, 100, 50, 40], twin: [100, 100, 100, 100], faults: [], ...over,
});
const data = {
  card: null, sections: [], incident: { startMin: 30, endMin: 52, dipPct: 47 }, fixes: [],
  causes: [{ id: "F0", label: "chute", sharePct: 70, bandPct: 3, parcelMin: 1, aloneMin: 1, tied: false, startMin: 30, endMin: 45 }],
} as ConsoleData;

describe("console model", () => {
  it("formats shift time from ticks", () => {
    expect(shiftClock(0, 120)).toBe("00:00:00");
    expect(shiftClock(4944, 120)).toBe("00:41:12");
  });
  it("measures the last finished minute against the twin", () => {
    expect(dipVsTwin(series())).toBe(-50);
    expect(dipVsTwin(series({ twin: undefined }))).toBeNull();
    expect(dipVsTwin(series({ delivered: [5] }))).toBeNull();
  });
  it("prefers the report's spans, else the session's faults", () => {
    expect(spansFor(series(), data).map((s) => s.id)).toEqual(["F0"]);
    const live = spansFor(series({ faults: [{ kind: "chute_jam", startTick: 120, endTick: 240 }] }), null);
    expect(live[0]).toMatchObject({ id: "live0", startMin: 1, endMin: 2 });
  });
  it("highlights all faults for the incident, one for a cause, none for a fix", () => {
    const spans = spansFor(series(), data);
    expect(highlightFor("inc", spans, data)).toEqual(["F0"]);
    expect(highlightFor("F0", spans, data)).toEqual(["F0"]);
    expect(highlightFor("X1", spans, data)).toEqual([]);
  });
  it("keeps the axis on half hours", () => {
    expect(axisEnd(10, null)).toBe(30);
    expect(axisEnd(41, 52)).toBe(90);
  });
  it("builds the timeline model with the incident caption and cursor", () => {
    const m = consoleTimeline(series(), data, "inc", 4800);
    expect(m.incidentCaption).toBe("−47 %");
    expect(m.cursorMin).toBe(40);
    expect(m.incident).toEqual({ startMin: 30, endMin: 52 });
  });
  it("centres the focus ring on the cell", () => {
    const r = focusRect({ scale: 10, offsetX: 5, offsetY: 5 }, 2, 3);
    expect(r.left + r.size / 2).toBe(30);
    expect(r.top + r.size / 2).toBe(40);
  });
});
