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

export type WorkerRequest =
  | { type: "init"; preset: string }
  | { type: "step"; n: number }
  | { type: "stats" };

export type WorkerResponse =
  | { type: "layout"; width: number; height: number; cells: Uint8Array }
  | {
      type: "snapshot";
      tick: number;
      robotXY: Uint16Array;
      robotState: Uint8Array;
      stats: SimStats;
      hash: string;
    }
  | { type: "stats"; stats: SimStats }
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
}
