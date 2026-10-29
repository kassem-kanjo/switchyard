import type { LayoutView, SnapshotView } from "../renderer/Renderer";
import { WorkerClient } from "./client";
import type { SimStats, WorkerRequest, WorkerResponse } from "./protocol";
import type { SimSource } from "./source";

const ZERO_STATS: SimStats = {
  tick: 0,
  arrived: 0,
  queued: 0,
  in_flight: 0,
  delivered: 0,
  mis_sorted: 0,
  throughput_per_hour_milli: 0,
};

/**
 * SimSource backed by the Rust sim running in a Web Worker. The same worker
 * answers the sidebar's calls through `client` (see `data/wasmServices.ts`).
 */
export class WasmSource implements SimSource {
  private snap: SnapshotView = { tick: 0, robotXY: new Uint16Array(0), robotState: new Uint8Array(0) };
  private lastStats: SimStats = ZERO_STATS;

  private constructor(
    readonly client: WorkerClient,
    private readonly lay: LayoutView,
  ) {}

  /** Start the worker and load the WASM; rejects if either fails. */
  static async create(preset = "small"): Promise<WasmSource> {
    const worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
    const client = new WorkerClient(worker);
    let first: WorkerResponse;
    try {
      const init: WorkerRequest = { type: "init", preset };
      first = await client.request(init);
    } catch (err) {
      client.dispose();
      throw err;
    }
    if (first.type !== "layout") {
      client.dispose();
      throw new Error("unexpected worker reply");
    }
    const src = new WasmSource(client, { width: first.width, height: first.height, cells: first.cells });
    await src.step(0); // prime the first snapshot
    return src;
  }

  layout(): LayoutView {
    return this.lay;
  }

  async step(n: number): Promise<void> {
    const m = await this.client.request({ type: "step", n });
    if (m.type !== "snapshot") throw new Error("unexpected worker reply");
    this.snap = {
      tick: m.tick,
      robotXY: m.robotXY,
      robotState: m.robotState,
      robotGoalXY: m.robotGoalXY,
      robotWaited: m.robotWaited,
    };
    this.lastStats = m.stats;
  }

  snapshot(): SnapshotView {
    return this.snap;
  }

  stats(): SimStats {
    return this.lastStats;
  }

  dispose(): void {
    this.client.dispose();
  }
}
