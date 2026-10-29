import { CellKind, RobotState, type LayoutView, type SnapshotView } from "./Renderer";

/** Sentinel the wasm facade uses for "no goal". */
export const NO_GOAL = 65535;
const SECONDS_PER_TICK = 0.5;

export interface RobotLabel {
  /** "R12" */
  title: string;
  /** What it is doing, in plain words. */
  doing: string;
  /** Where it is going, or null when it is not going anywhere. */
  going: string | null;
  /** How long it has been waiting, or null when it is moving. */
  waiting: string | null;
}

/** Destination cell of robot i, or null (no goal exposed, parked, or sentinel). */
export function goalOf(snap: SnapshotView, i: number): { x: number; y: number } | null {
  const g = snap.robotGoalXY;
  if (!g) return null;
  const x = g[2 * i];
  const y = g[2 * i + 1];
  if (x === undefined || y === undefined || x === NO_GOAL || y === NO_GOAL) return null;
  return { x, y };
}

/** Plain-words label for the hover card; no internal state names. */
export function describeRobot(snap: SnapshotView, layout: LayoutView, i: number): RobotLabel {
  const state = snap.robotState[i] ?? RobotState.Idle;
  const goal = goalOf(snap, i);
  const kind = goal ? layout.cells[goal.y * layout.width + goal.x] : undefined;
  const target = goal ? `${kind === CellKind.Chute ? "chute" : "pick-up station"} at column ${goal.x}, row ${goal.y}` : null;
  const doing: Record<number, string> = {
    [RobotState.Idle]: "Parked, waiting for work",
    [RobotState.ToInduction]: "Heading to pick up a parcel",
    [RobotState.Loading]: "Loading a parcel at the station",
    [RobotState.ToChute]: "Carrying a parcel to its chute",
    [RobotState.Dropping]: "Dropping a parcel into the chute",
    [RobotState.Failed]: "Broken down, waiting for repair",
  };
  const ticks = snap.robotWaited?.[i] ?? 0;
  const secs = Math.round(ticks * SECONDS_PER_TICK);
  const moving = state === RobotState.ToInduction || state === RobotState.ToChute;
  return {
    title: `R${i}`,
    doing: doing[state] ?? "Unknown",
    going: moving ? target : null,
    waiting: state === RobotState.Failed || (moving && secs >= 1) ? `waiting ${secs} s` : null,
  };
}
