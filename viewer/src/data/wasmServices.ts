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
  constructor(private readonly client: RpcClient) {}
  report(): Promise<string> {
    return this.client.rpc("postmortem_markdown");
  }
}

/** `Services` backed by the Rust sim in the worker behind `client`. */
export function createWasmServices(client: RpcClient): Services {
  return {
    metrics: new WasmMetrics(client),
    traces: new WasmTraces(client),
    faults: new WasmFaults(client),
    postmortem: new WasmPostmortem(client),
  };
}
