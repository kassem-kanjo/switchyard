import { describe, expect, it } from "vitest";
import { CellKind, RobotState, type LayoutView, type SnapshotView } from "./Renderer";
import { chuteCells, floorMarks, incidentCells, incidentFrame, jammedCellsOf } from "./floorMarks";

const layout: LayoutView = { width: 10, height: 8, cells: new Uint8Array(80) };
layout.cells[3 * 10 + 3] = CellKind.Chute;
layout.cells[3 * 10 + 6] = CellKind.Chute;
const snap = (failedAt: [number, number] | null): SnapshotView => ({
  tick: 0,
  robotXY: new Uint16Array(failedAt ?? [0, 0]),
  robotState: new Uint8Array([failedAt ? RobotState.Failed : RobotState.Idle]),
});

describe("floor marks", () => {
  it("numbers chutes row-major and maps jam faults to cells", () => {
    const chutes = chuteCells(layout);
    expect(chutes).toEqual([33, 36]);
    expect(jammedCellsOf([{ kind: "chute_jam", params: { chute: 1 } }, { kind: "robot_failure", params: {} }], chutes)).toEqual([36]);
    expect(jammedCellsOf([{ kind: "chute_jam", params: { chute: 9 } }], chutes)).toEqual([]);
  });
  it("lists failed robots and jammed chutes as the incident, and nothing on a healthy floor", () => {
    expect(incidentCells(layout, snap([5, 4]), [36])).toEqual([[5, 4], [6, 3]]);
    expect(incidentCells(layout, snap(null), [])).toEqual([]);
    expect(floorMarks([36])).toEqual({ jammedCells: [36] });
  });
  it("has no frame on a healthy floor", () => {
    expect(incidentFrame([], layout, 800, 600, 12)).toBeNull();
  });
  it("frames the incident without leaving the canvas", () => {
    const f = incidentFrame(incidentCells(layout, snap([8, 6]), [36]), layout, 800, 600, 12)!;
    expect(f.k).toBeGreaterThanOrEqual(1);
    expect(f.tx).toBeLessThanOrEqual(0);
    expect(f.tx).toBeGreaterThanOrEqual(800 * (1 - f.k));
    expect(f.ty).toBeGreaterThanOrEqual(600 * (1 - f.k));
  });
});
