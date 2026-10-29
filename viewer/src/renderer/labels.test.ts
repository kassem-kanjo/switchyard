import { describe, expect, it } from "vitest";
import { describeRobot, goalOf, NO_GOAL } from "./labels";
import { CellKind, RobotState, type LayoutView, type SnapshotView } from "./Renderer";

const layout: LayoutView = { width: 4, height: 2, cells: new Uint8Array([0, 0, CellKind.Chute, CellKind.Induction, 0, 0, 0, 0]) };
const snap = (state: number, goal: [number, number], waited: number): SnapshotView => ({
  tick: 1,
  robotXY: new Uint16Array([0, 0]),
  robotState: new Uint8Array([state]),
  robotGoalXY: new Uint16Array(goal),
  robotWaited: new Uint32Array([waited]),
});

describe("describeRobot", () => {
  it("carrying robot names its chute and how long it waited", () => {
    const l = describeRobot(snap(RobotState.ToChute, [2, 0], 7), layout, 0);
    expect(l.title).toBe("R0");
    expect(l.doing).toBe("Carrying a parcel to its chute");
    expect(l.going).toBe("chute at column 2, row 0");
    expect(l.waiting).toBe("waiting 4 s");
  });
  it("heading to pick up names the station", () => {
    expect(describeRobot(snap(RobotState.ToInduction, [3, 0], 0), layout, 0).going).toBe("pick-up station at column 3, row 0");
  });
  it("parked robot goes nowhere; broken robot always shows its wait", () => {
    expect(describeRobot(snap(RobotState.Idle, [NO_GOAL, NO_GOAL], 0), layout, 0).going).toBeNull();
    expect(describeRobot(snap(RobotState.Failed, [NO_GOAL, NO_GOAL], 0), layout, 0).waiting).toBe("waiting 0 s");
  });
  it("goalOf is null without goal data (mock source)", () => {
    expect(goalOf({ tick: 0, robotXY: new Uint16Array(2), robotState: new Uint8Array(1) }, 0)).toBeNull();
  });
});
