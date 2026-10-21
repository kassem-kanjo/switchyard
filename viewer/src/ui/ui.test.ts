import { theme } from "../theme";
import { describe, expect, it } from "vitest";
import { formatReadout, hoverReadout, indexAt, makeScale, niceCeil, tickToIndex, yTicks } from "./chartMath";
import { blameChain, describeSpan, formatDuration, parseParcelId, summarize } from "./traceFormat";
import { DURATION_KEY, remainingFraction, validateFault } from "./faultValidation";
import { parseInline, parseMarkdown } from "./markdown";
import { pickRobot } from "./pick";
import { MOCK_FAULT_KINDS, MockWorld, createMockServices } from "../data/mock";
import type { ParcelTrace, TraceSpan } from "../data/types";

const rect = { left: 10, top: 5, width: 100, height: 50 };

describe("chart scaling", () => {
  it("rounds the axis ceiling to tidy numbers", () => {
    expect(niceCeil(117)).toBe(200);
    expect(niceCeil(100)).toBe(100);
    expect(niceCeil(2.2)).toBe(2.5);
    expect(niceCeil(0)).toBe(1);
  });

  it("maps indices and values into the plot rect", () => {
    const s = makeScale(5, 117, rect);
    expect(s.yMax).toBe(200);
    expect(s.x(0)).toBe(10);
    expect(s.x(4)).toBe(110);
    expect(s.y(0)).toBe(55);
    expect(s.y(200)).toBe(5);
    expect(s.y(100)).toBe(30);
    expect(s.y(999)).toBe(5); // clamped
    expect(makeScale(1, 5, rect).x(0)).toBe(60);
  });

  it("yields evenly spaced axis ticks", () => {
    expect(yTicks(200, 2)).toEqual([0, 100, 200]);
  });

  it("converts ticks to fractional indices", () => {
    expect(tickToIndex(1500, 20, 60)).toBe(5);
  });
});

describe("hover maths", () => {
  const s = makeScale(5, 100, rect);
  it("snaps to the nearest point and ignores pointers outside the plot", () => {
    expect(indexAt(10, s)).toBe(0);
    expect(indexAt(34, s)).toBe(1);
    expect(indexAt(110, s)).toBe(4);
    expect(indexAt(5, s)).toBeNull();
    expect(indexAt(500, s)).toBeNull();
    expect(indexAt(50, makeScale(0, 1, rect))).toBeNull();
  });

  it("builds a readout with the twin delta", () => {
    const r = hoverReadout(1, 20, [90, 80], [117, 118]);
    expect(r).toEqual({ minute: 21, delivered: 80, twin: 118, delta: -38 });
    expect(formatReadout(r!)).toBe("min 21 · 80 delivered · twin 118 (−38)");
    expect(formatReadout(hoverReadout(0, 0, [5])!)).toBe("min 0 · 5 delivered");
    expect(hoverReadout(3, 0, [1])).toBeNull();
  });
});

describe("trace formatting", () => {
  const blocked: TraceSpan = {
    phase: "blocked",
    startTick: 100,
    endTick: 141,
    directBlocker: 112,
    blockChain: [87],
    rootCause: "dead robot R031",
  };

  it("formats durations", () => {
    expect(formatDuration(41)).toBe("41 s");
    expect(formatDuration(125)).toBe("2 min 5 s");
    expect(formatDuration(180)).toBe("3 min");
    expect(formatDuration(120, 2)).toBe("1 min");
  });

  it("writes the blame chain in plain language", () => {
    expect(blameChain(blocked)).toBe("R112 → R87 → root: dead robot R031");
    expect(describeSpan(blocked)).toBe("blocked 41 s · by R112 → R87 → root: dead robot R031");
    expect(describeSpan({ ...blocked, blockChain: undefined, rootCause: null })).toBe("blocked 41 s · by R112");
    expect(describeSpan({ ...blocked, directBlocker: null, blockChain: undefined, rootCause: null })).toBe("blocked 41 s");
    expect(describeSpan({ phase: "chute_wait", startTick: 0, endTick: 7, directBlocker: null, rootCause: null })).toBe(
      "waiting at chute 7 s",
    );
  });

  it("summarises blocked time", () => {
    const trace: ParcelTrace = {
      parcelId: 1,
      robot: 2,
      spans: [
        { phase: "travelling", startTick: 0, endTick: 30, directBlocker: null, rootCause: null },
        blocked,
        { ...blocked, startTick: 141, endTick: 150 },
      ],
    };
    const s = summarize(trace);
    expect(s.totalTicks).toBe(30 + 41 + 9);
    expect(s.blockedTicks).toBe(50);
    expect(s.worstBlocked).toBe(blocked);
  });

  it("parses the parcel search box", () => {
    expect(parseParcelId(" 1041 ")).toBe(1041);
    expect(parseParcelId("P77")).toBe(77);
    expect(parseParcelId("#5")).toBe(5);
    expect(parseParcelId("abc")).toBeNull();
    expect(parseParcelId("12x")).toBeNull();
    expect(parseParcelId("")).toBeNull();
  });
});

describe("fault validation", () => {
  const robot = MOCK_FAULT_KINDS.find((k) => k.id === "robot_failure")!;
  const surge = MOCK_FAULT_KINDS.find((k) => k.id === "volume_surge")!;

  it("builds a request from valid input", () => {
    const r = validateFault(robot, { count: "3" }, "120", 500, 1);
    expect(r).toEqual({ ok: true, request: { kind: "robot_failure", params: { count: 3 }, atTick: 500, durationTicks: 120 } });
  });

  it("converts the duration with the tick rate", () => {
    const r = validateFault(robot, { count: 1 }, 30, 0, 10);
    expect(r.ok && r.request.durationTicks).toBe(300);
  });

  it("rejects out-of-range, fractional and missing values with per-field messages", () => {
    const bad = validateFault(robot, { count: "2.5" }, "5", 0, 1);
    expect(bad.ok).toBe(false);
    if (!bad.ok) {
      expect(bad.errors.count).toMatch(/whole number/);
      expect(bad.errors[DURATION_KEY]).toMatch(/10 to 3600/);
    }
    const hi = validateFault(robot, { count: "99" }, "60", 0, 1);
    expect(!hi.ok && hi.errors.count).toBe("Use 1 to 20.");
    const empty = validateFault(surge, {}, "", 0, 1);
    expect(!empty.ok && empty.errors.factor).toBe("Enter a number.");
    expect(validateFault(surge, { factor: "1.5" }, "60", 0, 1).ok).toBe(true);
    expect(validateFault(surge, { factor: "NaN" }, "60", 0, 1).ok).toBe(false);
  });

  it("computes the remaining fraction of a fault window", () => {
    const f = { atTick: 100, endTick: 200 };
    expect(remainingFraction(f, 100)).toBe(1);
    expect(remainingFraction(f, 150)).toBe(0.5);
    expect(remainingFraction(f, 400)).toBe(0);
    expect(remainingFraction({ atTick: 5, endTick: 5 }, 5)).toBe(0);
  });
});

describe("markdown", () => {
  it("never interprets markup: angle brackets stay literal text", () => {
    const blocks = parseMarkdown("# Title <img src=x onerror=alert(1)>\n\nHello <script>alert(1)</script> world");
    const h = blocks[0]!;
    expect(h.t === "heading" && h.inline).toEqual([{ t: "text", v: "Title <img src=x onerror=alert(1)>" }]);
    const p = blocks[1]!;
    expect(p.t === "p" && p.inline).toEqual([{ t: "text", v: "Hello <script>alert(1)</script> world" }]);
  });

  it("does not turn links or images into anything", () => {
    const [b] = parseMarkdown("[x](javascript:alert(1)) ![i](http://e/x.png)");
    expect(b!.t === "p" && b!.inline.every((i) => i.t === "text")).toBe(true);
  });

  it("parses inline styles", () => {
    expect(parseInline("a **b** *c* `d<e>`")).toEqual([
      { t: "text", v: "a " },
      { t: "bold", v: "b" },
      { t: "text", v: " " },
      { t: "italic", v: "c" },
      { t: "text", v: " " },
      { t: "code", v: "d<e>" },
    ]);
  });

  it("parses headings, lists and paragraphs", () => {
    const blocks = parseMarkdown("# A\nline one\nline two\n\n- x\n- y\n1. p\n2. q\n");
    expect(blocks.map((b) => b.t)).toEqual(["heading", "p", "ul", "ol"]);
    const ul = blocks[2]!;
    expect(ul.t === "ul" && ul.items.length).toBe(2);
    const p = blocks[1]!;
    expect(p.t === "p" && p.inline).toEqual([{ t: "text", v: "line one line two" }]);
  });
});

describe("robot picking", () => {
  const snap = { tick: 0, robotXY: new Uint16Array([2, 3, 4, 3]), robotState: new Uint8Array([3, 0]) };
  const fit = { scale: 10, offsetX: 5, offsetY: 5 };
  it("finds the robot in the clicked cell and filters by state", () => {
    expect(pickRobot(snap, fit, 27, 38, null)).toBe(0);
    expect(pickRobot(snap, fit, 47, 38, null)).toBe(1);
    expect(pickRobot(snap, fit, 47, 38, [3, 4])).toBeNull();
    expect(pickRobot(snap, fit, 300, 300, null)).toBeNull();
    expect(pickRobot(snap, { scale: 0, offsetX: 0, offsetY: 0 }, 1, 1, null)).toBeNull();
  });
});

describe("mock data", () => {
  it("shows a dip below the twin during the fault and a recovery after", () => {
    let now = 60 * theme.ticksPerMinute; // one simulated hour
    const svc = createMockServices(() => now);
    const s = svc.metrics.series();
    const rel = (min: number) => min - s.startMinute;
    const base = s.twin![rel(15)]!;
    expect(s.delivered[rel(15)]!).toBeGreaterThan(base * 0.9);
    expect(s.delivered[rel(26)]!).toBeLessThan(base * 0.8);
    expect(s.delivered[rel(50)]!).toBeGreaterThan(base * 0.9);
    expect(s.faults.length).toBe(2);
    now = 0;
    expect(svc.metrics.series().delivered).toEqual([]);
  });

  it("injected faults appear in active() only inside their window and deepen the dip", async () => {
    let now = 3000;
    const svc = createMockServices(() => now);
    const f = await svc.faults.inject({ kind: "aisle_block", params: { aisle: 7 }, atTick: 3000, durationTicks: 300 });
    expect((await svc.faults.active(3100)).map((a) => a.id)).toContain(f.id);
    expect((await svc.faults.active(3300)).map((a) => a.id)).not.toContain(f.id);
    now = 3300;
    const s = svc.metrics.series();
    expect(s.faults.some((w) => w.kind === "aisle_block")).toBe(true);
  });

  it("traces tile contiguously and carry a blame chain", async () => {
    const svc = createMockServices(() => 0);
    const t = (await svc.traces.getTrace(1000))!;
    for (let i = 1; i < t.spans.length; i++) expect(t.spans[i]!.startTick).toBe(t.spans[i - 1]!.endTick);
    expect(t.spans.some((s) => s.phase === "blocked" && s.directBlocker !== null && s.rootCause)).toBe(true);
    expect(await svc.traces.getTrace(-1)).toBeNull();
    expect(await svc.postmortem.report()).toContain("Post-mortem");
  });

  it("MockWorld without the seeded incident has no loss", () => {
    expect(new MockWorld(() => 0, false).lossAt(10)).toBe(0);
  });
});
