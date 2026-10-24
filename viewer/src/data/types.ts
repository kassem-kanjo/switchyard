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

export interface HeadlineCause {
  /** Short label, e.g. "C289 jam". */
  label: string;
  /** The fault's id, e.g. "F0". */
  fault: string;
  /** Share of the median loss explained, in %. */
  sharePct: number;
  /** The share's noise band, in %. */
  bandPct: number;
}

export interface HeadlineFix {
  /** The fix's label, verbatim. */
  label: string;
  /** Recovered delay, whole parcel-minutes. */
  recoveredParcelMin: number;
  /** Share of the median loss recovered, in %. */
  recoveryPct: number;
  /** True when the gain is within its noise band. */
  withinNoise: boolean;
}

/** The one-glance summary of a post-mortem (`Headline`). */
export interface PostmortemHeadline {
  /** Incident window start (minute of the run). */
  startMin: number;
  /** Window end (minute of the run). */
  endMin: number;
  /** False when the incident was still open when the run ended. */
  settled: boolean;
  /** The worst throughput dip against the twin, in %. */
  maxDipPct: number;
  /** The main cause, if the loss is attributable and a cause stands out. */
  cause: HeadlineCause | null;
  /** Short label of the second cause when it ties the first. */
  tieWith: string | null;
  /** Real history's loss, whole parcel-minutes. */
  lossParcelMin: number;
  /** Parcels more than the SLA late at the horizon, real history. */
  lateParcels: number;
  /** The best non-hotspot fix with a gain above zero. */
  fix: HeadlineFix | null;
  /** Replays behind the post-mortem. */
  replays: number;
  /** Histories every replay ran on. */
  histories: number;
}

export interface PostmortemProvider {
  /** Markdown report. Treated as untrusted text by the UI. */
  report(): Promise<string>;
  /** One-glance summary, or null when there is no incident to explain. */
  headline(): Promise<PostmortemHeadline | null>;
  /** Precomputed report for a preset (ADR 0006 section 8), or null when there is none. Untrusted text. */
  precomputed(preset: string): Promise<string | null>;
}

export interface Services {
  metrics: MetricsFeed;
  traces: TraceProvider;
  faults: FaultControl;
  postmortem: PostmortemProvider;
}
