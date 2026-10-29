export const CellKind = { Free: 0, Wall: 1, Chute: 2, Induction: 3 } as const;
export const RobotState = { Idle: 0, ToInduction: 1, Loading: 2, ToChute: 3, Dropping: 4, Failed: 5 } as const;

export interface LayoutView {
  width: number;
  height: number;
  /** Row-major, width*height entries; values from CellKind. */
  cells: Uint8Array;
}

export interface SnapshotView {
  tick: number;
  /** x0,y0,x1,y1,... */
  robotXY: Uint16Array;
  /** One RobotState per robot. */
  robotState: Uint8Array;
}

export interface FloorMarks {
  /** Cell indices (row-major) of jammed chutes. */
  jammedCells: number[];
}

export interface Renderer {
  init(canvas: HTMLCanvasElement, layout: LayoutView): void;
  draw(snap: SnapshotView): void;
  highlight(robotIds: number[]): void;
  resize(w: number, h: number): void;
  setMarks?(marks: FloorMarks): void;
  setReducedMotion?(on: boolean): void;
}
