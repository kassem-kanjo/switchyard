import type { LayoutView, SnapshotView } from "../renderer/Renderer";
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

/** SimSource backed by the Rust sim running in a Web Worker. */
export class WasmSource implements SimSource {
  private snap: SnapshotView = { tick: 0, robotXY: new Uint16Array(0), robotState: new Uint8Array(0) };
  private lastStats: SimStats = ZERO_STATS;
  private pending: { resolve: (m: WorkerResponse) => void; reject: (e: Error) => void } | null = null;

  private constructor(
    private readonly worker: Worker,
    private readonly lay: LayoutView,
  ) {
    worker.onmessage = (e: MessageEvent<WorkerResponse>) => this.settle(e.data);
    worker.onerror = (e) => this.fail(new Error(e.message || "worker error"));
  }

  /** Start the worker and load the WASM; rejects if either fails. */
  static async create(preset = "small"): Promise<WasmSource> {
    const worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
    let first: WorkerResponse;
    try {
      first = await new Promise<WorkerResponse>((resolve, reject) => {
        worker.onmessage = (e: MessageEvent<WorkerResponse>) => resolve(e.data);
        worker.onerror = (e) => reject(new Error(e.message || "worker failed to start"));
        const req: WorkerRequest = { type: "init", preset };
        worker.postMessage(req);
      });
    } catch (err) {
      worker.terminate();
      throw err;
    }
    if (first.type !== "layout") {
      worker.terminate();
      throw new Error(first.type === "error" ? first.message : "unexpected worker reply");
    }
    const src = new WasmSource(worker, { width: first.width, height: first.height, cells: first.cells });
    await src.step(0); // prime the first snapshot
    return src;
  }

  private settle(m: WorkerResponse): void {
    const p = this.pending;
    this.pending = null;
    if (!p) return;
    if (m.type === "error") p.reject(new Error(m.message));
    else p.resolve(m);
  }

  private fail(err: Error): void {
    const p = this.pending;
    this.pending = null;
    p?.reject(err);
  }

  private request(req: WorkerRequest): Promise<WorkerResponse> {
    if (this.pending) return Promise.reject(new Error("a request is already in flight"));
    return new Promise((resolve, reject) => {
      this.pending = { resolve, reject };
      this.worker.postMessage(req);
    });
  }

  layout(): LayoutView {
    return this.lay;
  }

  async step(n: number): Promise<void> {
    const m = await this.request({ type: "step", n });
    if (m.type !== "snapshot") throw new Error("unexpected worker reply");
    this.snap = { tick: m.tick, robotXY: m.robotXY, robotState: m.robotState };
    this.lastStats = m.stats;
  }

  snapshot(): SnapshotView {
    return this.snap;
  }

  stats(): SimStats {
    return this.lastStats;
  }

  dispose(): void {
    this.worker.terminate();
  }
}
