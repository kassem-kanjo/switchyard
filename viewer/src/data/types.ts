/**
 * Narrow data interfaces the viewer UI depends on. The WASM worker will
 * implement these later; `mock.ts` implements them today. Everything that
 * crosses the worker boundary is async, except the metrics snapshot which
 * (like SimSource) reads the latest known state so redraws never block.
 */

export interface FaultWindow {
  kind: string;
  startTick: number;
  endTick: number;
}

export interface MetricsSeries {
  ticksPerMinute: number;
  /** Minute index of `delivered[0]`. */
  startMinute: number;
  /** Parcels delivered in each whole minute. */
  delivered: number[];
  /** Fault-free twin series (same seed), aligned with `delivered`, when available. */
  twin?: number[];
  faults: FaultWindow[];
}

export interface MetricsFeed {
  series(): MetricsSeries;
}

export type PhaseName =
  | "queued"
  | "awaiting_robot"
  | "loading"
  | "travelling"
  | "blocked"
  | "chute_wait"
  | "drop";

export interface TraceSpan {
  phase: PhaseName;
  startTick: number;
  endTick: number;
  /** Robot id directly in the way (blocked spans only). */
  directBlocker: number | null;
  /** Robots between the direct and the root blocker, nearest first. */
  blockChain?: number[];
  /** Plain label of the root cause, e.g. "dead robot R031". */
  rootCause: string | null;
}

export interface ParcelTrace {
  parcelId: number;
  robot: number | null;
  /** Contiguous spans covering the parcel's lifetime. */
  spans: TraceSpan[];
}

export interface TraceProvider {
  /** Parcel currently carried by a robot, if any. */
  parcelOfRobot(robotId: number): Promise<number | null>;
  getTrace(parcelId: number): Promise<ParcelTrace | null>;
}

export interface FaultParamSpec {
  key: string;
  label: string;
  unit?: string;
  min: number;
  max: number;
  step: number;
  default: number;
  integer: boolean;
}

export interface FaultKindInfo {
  id: string;
  label: string;
  description: string;
  params: FaultParamSpec[];
}

export interface FaultRequest {
  kind: string;
  params: Record<string, number>;
  atTick: number;
  durationTicks: number;
}

export interface ActiveFault extends FaultRequest {
  id: number;
  endTick: number;
}

export interface FaultControl {
  kinds(): Promise<FaultKindInfo[]>;
  inject(req: FaultRequest): Promise<ActiveFault>;
  /** Faults whose window contains `nowTick`. */
  active(nowTick: number): Promise<ActiveFault[]>;
}

export interface PostmortemProvider {
  /** Markdown report. Treated as untrusted text by the UI. */
  report(): Promise<string>;
  /** Precomputed report for a preset (ADR 0006 section 8), or null when there is none. Untrusted text. */
  precomputed(preset: string): Promise<string | null>;
}

export interface Services {
  metrics: MetricsFeed;
  traces: TraceProvider;
  faults: FaultControl;
  postmortem: PostmortemProvider;
}
