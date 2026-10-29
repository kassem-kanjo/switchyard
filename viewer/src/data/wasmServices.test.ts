import { describe, expect, it } from "vitest";
import { createMockServices } from "./mock";
import fixture from "./fixtures/wasm-shapes.json";
import { createWasmServices, type RpcClient } from "./wasmServices";
import { WorkerClient, type WorkerLike } from "../sim/client";
import type { RpcMethod, WorkerRequest, WorkerResponse } from "../sim/protocol";
import type { FaultKindInfo, HeadlineCause, HeadlineFix, MetricsSeries, PostmortemHeadline } from "./types";

const PHASES = ["queued", "awaiting_robot", "loading", "travelling", "blocked", "chute_wait", "drop"];
const num = (v: unknown) => typeof v === "number";
const nums = (v: unknown) => Array.isArray(v) && v.every(num);

/** Runtime check of the shared fixture against the hand-written TS types, key by key. */
describe("shared fixture matches types.ts", () => {
  it("metrics", () => {
    const m = fixture.metrics;
    expect(num(m.ticksPerMinute) && num(m.startMinute)).toBe(true);
    expect(nums(m.delivered) && nums(m.twin)).toBe(true);
    for (const f of m.faults) expect([typeof f.kind, typeof f.startTick, typeof f.endTick]).toEqual(["string", "number", "number"]);
  });

  it("trace", () => {
    const t = fixture.trace;
    expect(num(t.parcelId) && num(t.robot)).toBe(true);
    for (const s of t.spans) {
      expect(PHASES).toContain(s.phase);
      expect(num(s.startTick) && num(s.endTick)).toBe(true);
      expect(s.directBlocker === null || num(s.directBlocker)).toBe(true);
      expect(s.rootCause === null || typeof s.rootCause === "string").toBe(true);
    }
  });

  it("fault kinds and active faults", () => {
    for (const k of fixture.faultKinds) {
      expect([typeof k.id, typeof k.label, typeof k.description]).toEqual(["string", "string", "string"]);
      for (const p of k.params) {
        expect(["string", "string"]).toEqual([typeof p.key, typeof p.label]);
        expect([p.min, p.max, p.step, p.default].every(num) && typeof p.integer === "boolean").toBe(true);
      }
    }
    const a = fixture.activeFault;
    expect([a.id, a.atTick, a.durationTicks, a.endTick].every(num)).toBe(true);
    expect(Object.values(a.params).every(num)).toBe(true);
  });

  it("headline", () => {
    const h: PostmortemHeadline = fixture.headline;
    expect([h.startMin, h.endMin, h.maxDipPct, h.lossParcelMin, h.lateParcels, h.replays, h.histories].every(num)).toBe(true);
    expect(typeof h.settled).toBe("boolean");
    expect(h.tieWith === null || typeof h.tieWith === "string").toBe(true);
    if (!h.cause || !h.fix) throw new Error("the fixture headline has a cause and a fix");
    const c: HeadlineCause = h.cause;
    expect([typeof c.label, typeof c.fault]).toEqual(["string", "string"]);
    expect(num(c.sharePct) && num(c.bandPct)).toBe(true);
    const f: HeadlineFix = h.fix;
    expect(typeof f.label).toBe("string");
    expect(num(f.recoveredParcelMin) && num(f.recoveryPct)).toBe(true);
    expect(typeof f.withinNoise).toBe("boolean");
  });
});

function fakeRpc(replies: Partial<Record<RpcMethod, string>>, calls: Array<[RpcMethod, unknown]> = []): RpcClient {
  return {
    rpc: (method, arg) => {
      calls.push([method, arg]);
      return Promise.resolve(replies[method] ?? "null");
    },
    series: () => fixture.metrics as MetricsSeries,
  };
}

describe("createWasmServices", () => {
  it("parses what the worker returns", async () => {
    const calls: Array<[RpcMethod, unknown]> = [];
    const s = createWasmServices(
      fakeRpc(
        {
          trace_json: JSON.stringify(fixture.trace),
          parcel_of_robot: "41",
          fault_kinds_json: JSON.stringify(fixture.faultKinds),
          inject_fault: JSON.stringify(fixture.activeFault),
          active_faults_json: JSON.stringify([fixture.activeFault]),
          postmortem_markdown: "# Post-mortem (placeholder)",
          postmortem_headline: JSON.stringify(fixture.headline),
        },
        calls,
      ),
    );
    expect(await s.traces.parcelOfRobot(7)).toBe(41);
    expect((await s.traces.getTrace(41))?.spans.length).toBe(3);
    expect(((await s.faults.kinds()) as FaultKindInfo[])[0]?.id).toBe("volume_surge");
    const req = { kind: "robot_failure", params: { count: 2 }, atTick: 120, durationTicks: 300 };
    expect((await s.faults.inject(req)).endTick).toBe(420);
    expect(await s.faults.active(130)).toHaveLength(1);
    expect(await s.postmortem.report()).toMatch(/placeholder/);
    expect(await s.postmortem.headline()).toEqual(fixture.headline);
    expect(s.metrics.series().delivered).toEqual([110, 118, 95]);
    expect(calls).toContainEqual(["parcel_of_robot", 7]);
    expect(calls).toContainEqual(["inject_fault", JSON.stringify(req)]);
    expect(calls).toContainEqual(["active_faults_json", 130]);
    expect(calls).toContainEqual(["postmortem_headline", undefined]);
  });

  it("maps a missing headline to null", async () => {
    const s = createWasmServices(fakeRpc({ postmortem_headline: "null" }));
    expect(await s.postmortem.headline()).toBeNull();
  });

  it("maps a missing parcel and a bad id to null", async () => {
    const s = createWasmServices(fakeRpc({}));
    expect(await s.traces.parcelOfRobot(3)).toBeNull();
    expect(await s.traces.getTrace(5)).toBeNull();
    expect(await s.traces.getTrace(-1)).toBeNull();
    expect(await s.traces.getTrace(1.5)).toBeNull();
  });

  const reply = (ok: boolean, body: string) => ({ ok, text: () => Promise.resolve(body) });

  it("precomputed returns the report when ok and it starts with a markdown H1", async () => {
    const urls: string[] = [];
    const s = createWasmServices(fakeRpc({}), (url) => {
      urls.push(url);
      return Promise.resolve(reply(true, "# Two dead robots and a jam\n\nbody"));
    });
    expect(await s.postmortem.precomputed("two-dead-robots-and-a-jam")).toBe("# Two dead robots and a jam\n\nbody");
    expect(urls).toEqual(["reports/two-dead-robots-and-a-jam.md"]);
  });

  it("precomputed returns null when the response is not ok", async () => {
    const s = createWasmServices(fakeRpc({}), () => Promise.resolve(reply(false, "# Report")));
    expect(await s.postmortem.precomputed("a-preset")).toBeNull();
  });

  it("precomputed returns null for an HTML body (dev server fallback)", async () => {
    const s = createWasmServices(fakeRpc({}), () => Promise.resolve(reply(true, "<!doctype html><html>")));
    expect(await s.postmortem.precomputed("a-preset")).toBeNull();
  });

  it("precomputed returns null when fetch rejects", async () => {
    const s = createWasmServices(fakeRpc({}), () => Promise.reject(new Error("offline")));
    expect(await s.postmortem.precomputed("a-preset")).toBeNull();
  });

  it("precomputed never builds a URL from an odd preset and returns null", async () => {
    const calls: string[] = [];
    const s = createWasmServices(fakeRpc({}), (url) => {
      calls.push(url);
      return Promise.resolve(reply(true, "# x"));
    });
    expect(await s.postmortem.precomputed("../x")).toBeNull();
    expect(calls).toEqual([]);
  });

  it("mock postmortem has no precomputed report", async () => {
    const svc = createMockServices(() => 0);
    expect(await svc.postmortem.precomputed("two-dead-robots-and-a-jam")).toBeNull();
  });
});

class FakeWorker implements WorkerLike {
  onmessage: ((e: MessageEvent<WorkerResponse>) => void) | null = null;
  onerror: ((e: ErrorEvent) => void) | null = null;
  sent: WorkerRequest[] = [];
  terminated = false;
  postMessage(msg: WorkerRequest): void {
    this.sent.push(msg);
  }
  terminate(): void {
    this.terminated = true;
  }
  reply(m: WorkerResponse): void {
    this.onmessage?.({ data: m } as MessageEvent<WorkerResponse>);
  }
}

const snapshot = (series?: string): WorkerResponse => ({
  type: "snapshot",
  tick: 10,
  robotXY: new Uint16Array(0),
  robotState: new Uint8Array(0),
  stats: { tick: 10, arrived: 0, queued: 0, in_flight: 0, delivered: 0, mis_sorted: 0, throughput_per_hour_milli: 0 },
  ...(series === undefined ? {} : { series }),
});

describe("WorkerClient", () => {
  it("answers requests in order, so a step and an rpc can be in flight together", async () => {
    const w = new FakeWorker();
    const c = new WorkerClient(w);
    const step = c.request({ type: "step", n: 1 });
    const call = c.rpc("trace_json", 5);
    expect(w.sent).toHaveLength(2);
    w.reply(snapshot());
    w.reply({ type: "rpc", result: "{}" });
    expect((await step).type).toBe("snapshot");
    expect(await call).toBe("{}");
  });

  it("caches the series pushed with snapshots and keeps it on a bad push", async () => {
    const w = new FakeWorker();
    const c = new WorkerClient(w);
    expect(c.series().delivered).toEqual([]);
    expect(c.series().ticksPerMinute).toBe(120);
    const p1 = c.request({ type: "step", n: 1 });
    w.reply(snapshot(JSON.stringify(fixture.metrics)));
    await p1;
    expect(c.series().delivered).toEqual([110, 118, 95]);
    const p2 = c.request({ type: "step", n: 1 });
    w.reply(snapshot("not json"));
    await p2;
    expect(c.series().delivered).toEqual([110, 118, 95]);
  });

  it("rejects only the request an error answers", async () => {
    const w = new FakeWorker();
    const c = new WorkerClient(w);
    const bad = c.rpc("inject_fault", "{}");
    const good = c.rpc("postmortem_markdown");
    w.reply({ type: "error", message: "unknown fault kind" });
    w.reply({ type: "rpc", result: "ok" });
    await expect(bad).rejects.toThrow("unknown fault kind");
    expect(await good).toBe("ok");
  });

  it("fails everything waiting when the worker dies or is disposed", async () => {
    const w = new FakeWorker();
    const c = new WorkerClient(w);
    const a = c.rpc("metrics_json");
    w.onerror?.({ message: "boom" } as ErrorEvent);
    await expect(a).rejects.toThrow("boom");
    const b = c.rpc("metrics_json");
    c.dispose();
    await expect(b).rejects.toThrow("disposed");
    expect(w.terminated).toBe(true);
  });
});
