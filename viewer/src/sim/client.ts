import { emptySeries } from "../data/series";
import type { MetricsSeries } from "../data/types";
import type { RpcMethod, WorkerRequest, WorkerResponse } from "./protocol";

/** The slice of `Worker` the client uses (a fake in tests). */
export interface WorkerLike {
  postMessage(msg: WorkerRequest): void;
  terminate(): void;
  onmessage: ((e: MessageEvent<WorkerResponse>) => void) | null;
  onerror: ((e: ErrorEvent) => void) | null;
}

interface Waiter {
  resolve: (m: WorkerResponse) => void;
  reject: (e: Error) => void;
}

/**
 * Request/response channel to the sim worker. The worker answers every
 * request exactly once, in order, so a FIFO queue of waiters is enough and
 * simulation steps and sidebar calls can be in flight together without ids.
 * Also caches the latest chart series the worker attaches to snapshots, so
 * `MetricsFeed.series()` can stay synchronous.
 */
export class WorkerClient {
  private readonly queue: Waiter[] = [];
  private cached: MetricsSeries = emptySeries();

  constructor(private readonly worker: WorkerLike) {
    worker.onmessage = (e) => this.settle(e.data);
    worker.onerror = (e) => this.failAll(new Error(e.message || "worker error"));
  }

  /** Send a request; resolves with its response (an `error` response rejects). */
  request(req: WorkerRequest): Promise<WorkerResponse> {
    return new Promise((resolve, reject) => {
      this.queue.push({ resolve, reject });
      this.worker.postMessage(req);
    });
  }

  /** Call a sidebar method on the sim; resolves with its text result. */
  async rpc(method: RpcMethod, arg?: string | number): Promise<string> {
    const m = await this.request(arg === undefined ? { type: "rpc", method } : { type: "rpc", method, arg });
    if (m.type !== "rpc") throw new Error("unexpected worker reply");
    return m.result;
  }

  /** The latest chart series pushed by the worker. */
  series(): MetricsSeries {
    return this.cached;
  }

  dispose(): void {
    this.worker.terminate();
    this.failAll(new Error("worker disposed"));
  }

  private settle(m: WorkerResponse): void {
    const w = this.queue.shift();
    if (!w) return;
    if (m.type === "error") {
      w.reject(new Error(m.message));
      return;
    }
    if (m.type === "snapshot" && m.series !== undefined) {
      try {
        this.cached = JSON.parse(m.series) as MetricsSeries;
      } catch {
        // Keep the previous series; a bad push must not break stepping.
      }
    }
    w.resolve(m);
  }

  private failAll(err: Error): void {
    for (const w of this.queue.splice(0)) w.reject(err);
  }
}
