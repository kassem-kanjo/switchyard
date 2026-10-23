import type { WorkerClient } from "../sim/client";
import type {
  ActiveFault,
  FaultControl,
  FaultKindInfo,
  FaultRequest,
  MetricsFeed,
  MetricsSeries,
  ParcelTrace,
  PostmortemProvider,
  Services,
  TraceProvider,
} from "./types";

/** The part of `WorkerClient` the services need (a fake in tests). */
export type RpcClient = Pick<WorkerClient, "rpc" | "series">;

function parse<T>(text: string): T {
  return JSON.parse(text) as T;
}

class WasmMetrics implements MetricsFeed {
  constructor(private readonly client: RpcClient) {}
  series(): MetricsSeries {
    return this.client.series();
  }
}

class WasmTraces implements TraceProvider {
  constructor(private readonly client: RpcClient) {}

  async parcelOfRobot(robotId: number): Promise<number | null> {
    return parse<number | null>(await this.client.rpc("parcel_of_robot", robotId));
  }

  async getTrace(parcelId: number): Promise<ParcelTrace | null> {
    if (!Number.isInteger(parcelId) || parcelId < 0) return null;
    return parse<ParcelTrace | null>(await this.client.rpc("trace_json", parcelId));
  }
}

class WasmFaults implements FaultControl {
  constructor(private readonly client: RpcClient) {}

  async kinds(): Promise<FaultKindInfo[]> {
    return parse<FaultKindInfo[]>(await this.client.rpc("fault_kinds_json"));
  }

  async inject(req: FaultRequest): Promise<ActiveFault> {
    return parse<ActiveFault>(await this.client.rpc("inject_fault", JSON.stringify(req)));
  }

  async active(nowTick: number): Promise<ActiveFault[]> {
    return parse<ActiveFault[]>(await this.client.rpc("active_faults_json", nowTick));
  }
}

class WasmPostmortem implements PostmortemProvider {
  constructor(
    private readonly client: RpcClient,
    private readonly fetchText: (url: string) => Promise<{ ok: boolean; text(): Promise<string> }> = (url) => fetch(url),
  ) {}
  report(): Promise<string> {
    return this.client.rpc("postmortem_markdown");
  }
  async precomputed(preset: string): Promise<string | null> {
    if (!/^[a-z0-9-]+$/.test(preset)) return null; // never build a URL from odd input
    try {
      const res = await this.fetchText(`reports/${preset}.md`);
      const text = await res.text();
      // The dev server answers unknown paths with index.html and status 200, which must read as "none".
      return res.ok && text.startsWith("# ") ? text : null;
    } catch {
      return null;
    }
  }
}

/** `Services` backed by the Rust sim in the worker behind `client`. */
export function createWasmServices(
  client: RpcClient,
  fetchText?: (url: string) => Promise<{ ok: boolean; text(): Promise<string> }>,
): Services {
  return {
    metrics: new WasmMetrics(client),
    traces: new WasmTraces(client),
    faults: new WasmFaults(client),
    postmortem: new WasmPostmortem(client, fetchText),
  };
}
