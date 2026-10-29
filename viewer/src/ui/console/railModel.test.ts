// @vitest-environment node
/** Pure rail-model tests: no DOM needed. */

import { describe, expect, it } from "vitest";
import { barWidthPct, groupItems, nextId } from "./railModel";
import type { RailItem } from "./types";

function item(overrides: Partial<RailItem> & Pick<RailItem, "id" | "kind">): RailItem {
  return {
    title: overrides.id,
    subtitle: "",
    tone: "muted",
    ...overrides,
  };
}

describe("groupItems", () => {
  it("uses group order and keeps input order inside each group", () => {
    const items = [
      item({ id: "X2", kind: "fix" }),
      item({ id: "inc", kind: "incident" }),
      item({ id: "F1", kind: "cause" }),
      item({ id: "F0", kind: "cause" }),
      item({ id: "X0", kind: "fix" }),
    ];

    const groups = groupItems(items);

    expect(groups.map((g) => g.title)).toEqual(["Incidents", "Causes", "Fixes tested"]);
    expect(groups.map((g) => g.kind)).toEqual(["incident", "cause", "fix"]);
    expect(groups.map((g) => g.items.map((i) => i.id))).toEqual([
      ["inc"],
      ["F1", "F0"],
      ["X2", "X0"],
    ]);
  });

  it("omits empty groups", () => {
    expect(groupItems([])).toEqual([]);

    const groups = groupItems([item({ id: "F0", kind: "cause" })]);
    expect(groups).toHaveLength(1);
    expect(groups[0]?.title).toBe("Causes");
    expect(groups[0]?.items.map((i) => i.id)).toEqual(["F0"]);
  });
});

describe("barWidthPct", () => {
  it("returns 0 when the share is absent or not finite", () => {
    expect(barWidthPct(item({ id: "a", kind: "cause" }))).toBe(0);
    expect(barWidthPct(item({ id: "a", kind: "cause", sharePct: Number.NaN }))).toBe(0);
  });

  it("clamps to 0..100", () => {
    expect(barWidthPct(item({ id: "a", kind: "cause", sharePct: -5 }))).toBe(0);
    expect(barWidthPct(item({ id: "a", kind: "cause", sharePct: 0 }))).toBe(0);
    expect(barWidthPct(item({ id: "a", kind: "cause", sharePct: 47.5 }))).toBe(47.5);
    expect(barWidthPct(item({ id: "a", kind: "cause", sharePct: 150 }))).toBe(100);
  });

  it("applies a 2 % floor to small positive shares", () => {
    expect(barWidthPct(item({ id: "a", kind: "cause", sharePct: 0.1 }))).toBe(2);
    expect(barWidthPct(item({ id: "a", kind: "cause", sharePct: 1.9 }))).toBe(2);
    expect(barWidthPct(item({ id: "a", kind: "cause", sharePct: 2 }))).toBe(2);
    expect(barWidthPct(item({ id: "a", kind: "cause", sharePct: 2.1 }))).toBe(2.1);
  });
});

describe("nextId", () => {
  const items = [
    item({ id: "X0", kind: "fix" }),
    item({ id: "F0", kind: "cause" }),
    item({ id: "inc", kind: "incident" }),
    item({ id: "F1", kind: "cause" }),
  ];
  // Display order is grouped: incident, cause, cause, fix.
  const display = ["inc", "F0", "F1", "X0"];

  it("returns null for an empty rail", () => {
    expect(nextId([], null, 1)).toBeNull();
    expect(nextId([], "inc", -1)).toBeNull();
  });

  it("starts at the first row for +1 and the last row for -1", () => {
    expect(nextId(items, null, 1)).toBe(display[0]);
    expect(nextId(items, null, -1)).toBe(display[display.length - 1]);
  });

  it("moves in display order, not raw input order", () => {
    expect(nextId(items, "inc", 1)).toBe("F0");
    expect(nextId(items, "F0", 1)).toBe("F1");
    expect(nextId(items, "F1", 1)).toBe("X0");
    expect(nextId(items, "X0", -1)).toBe("F1");
  });

  it("wraps at both ends", () => {
    expect(nextId(items, "X0", 1)).toBe("inc");
    expect(nextId(items, "inc", -1)).toBe("X0");
  });

  it("treats an unknown current like no selection", () => {
    expect(nextId(items, "missing", 1)).toBe("inc");
    expect(nextId(items, "missing", -1)).toBe("X0");
  });
});
