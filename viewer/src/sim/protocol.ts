/** Messages between the main thread and the sim worker. */

export interface SimStats {
  tick: number;
  arrived: number;
  queued: number;
  in_flight: number;
  delivered: number;
  mis_sorted: number;
  /** Delivered parcels per simulated hour, times 1000 (integer). */
  throughput_per_hour_milli: number;
}

/** The sidebar calls the worker can answer; each maps to a `WasmSim` method. */
export type RpcMethod =
  | "metrics_json"
  | "trace_json"
  | "parcel_of_robot"
  | "fault_kinds_json"
  | "inject_fault"
  | "active_faults_json"
  | "postmortem_markdown"
  /** On demand only: hashing walks the whole history, so snapshots do not carry it. */
  | "state_hash_hex";

export type WorkerRequest =
  | { type: "init"; preset: string }
  | { type: "step"; n: number }
  | { type: "stats" }
  | { type: "rpc"; method: RpcMethod; arg?: string | number };

export type WorkerResponse =
  | { type: "layout"; width: number; height: number; cells: Uint8Array }
  | {
      type: "snapshot";
      tick: number;
      robotXY: Uint16Array;
      robotState: Uint8Array;
      stats: SimStats;
      /** `MetricsSeries` JSON, attached every few ticks and after a fault is injected. */
      series?: string;
    }
  | { type: "stats"; stats: SimStats }
  /** Result of an rpc: JSON text (or Markdown for the post-mortem). */
  | { type: "rpc"; result: string }
  | { type: "error"; message: string };

/** The subset of the wasm-bindgen `WasmSim` class the worker uses. */
export interface WasmSimApi {
  width(): number;
  height(): number;
  cells(): Uint8Array;
  step(n: number): void;
  tick(): number;
  robot_xy(): Uint16Array;
  robot_state(): Uint8Array;
  stats_json(): string;
  state_hash_hex(): string;
  metrics_json(): string;
  trace_json(parcel: number): string;
  parcel_of_robot(robot: number): number | undefined;
  fault_kinds_json(): string;
  inject_fault(request: string): string;
  active_faults_json(tick: number): string;
  postmortem_markdown(): string;
}
