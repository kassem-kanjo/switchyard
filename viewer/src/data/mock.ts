import type {
  ActiveFault,
  FaultControl,
  FaultKindInfo,
  FaultRequest,
  FaultWindow,
  MetricsFeed,
  MetricsSeries,
  ParcelTrace,
  PhaseName,
  PostmortemProvider,
  Services,
  TraceProvider,
  TraceSpan,
} from "./types";
import { theme } from "../theme";

/** Deterministic hash -> [0,1). */
function hash01(n: number): number {
  let t = (n + 0x6d2b79f5) >>> 0;
  t = Math.imul(t ^ (t >>> 15), t | 1);
  t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}

export const MOCK_FAULT_KINDS: FaultKindInfo[] = [
  {
    id: "robot_failure",
    label: "Robot failure",
    description: "Robots stop where they are and block their cells.",
    params: [{ key: "count", label: "Robots", min: 1, max: 20, step: 1, default: 2, integer: true }],
  },
  {
    id: "aisle_block",
    label: "Aisle block",
    description: "One aisle is closed to traffic.",
    params: [{ key: "aisle", label: "Aisle", min: 1, max: 16, step: 1, default: 7, integer: true }],
  },
  {
    id: "chute_jam",
    label: "Chute jam",
    description: "A chute stops accepting parcels.",
    params: [{ key: "chute", label: "Chute", min: 1, max: 40, step: 1, default: 12, integer: true }],
  },
  {
    id: "induction_outage",
    label: "Induction outage",
    description: "An induction station stops loading.",
    params: [{ key: "station", label: "Station", min: 1, max: 8, step: 1, default: 2, integer: true }],
  },
  {
    id: "volume_surge",
    label: "Volume surge",
    description: "Parcel arrivals jump above the planned rate.",
    params: [{ key: "factor", label: "Arrival rate", unit: "x", min: 1.1, max: 3, step: 0.1, default: 1.5, integer: false }],
  },
  {
    id: "scanner_drift",
    label: "Scanner drift",
    description: "Scanners misread destinations, so parcels go to the wrong chute.",
    params: [{ key: "rate", label: "Misread rate", unit: "%", min: 1, max: 50, step: 1, default: 10, integer: false }],
  },
];

/** How much of the throughput a fault removes at its worst, 0..0.6. Mock-only. */
function severity(f: FaultRequest): number {
  const p = f.params;
  switch (f.kind) {
    case "robot_failure":
      return Math.min(0.6, 0.06 * (p.count ?? 1) + 0.04);
    case "aisle_block":
      return 0.25;
    case "chute_jam":
      return 0.12;
    case "induction_outage":
      return 0.35;
    case "volume_surge":
      return Math.min(0.4, 0.12 * ((p.factor ?? 1.5) - 1));
    case "scanner_drift":
      return Math.min(0.5, ((p.rate ?? 10) / 100) * 1.5);
    default:
      return 0.1;
  }
}

const BASELINE_PER_MINUTE = 117; // ~7,000 parcels/h, the PRODUCT.md demo hub

/**
 * Shared mock world: one list of faults, read by the fault control, the metrics
 * feed and the trace provider so they tell the same story. Seeded with the demo
 * incident (two robots die, a chute jams) so a dip and a recovery always exist.
 */
export class MockWorld {
  readonly faults: ActiveFault[] = [];
  private nextId = 1;

  constructor(readonly clock: () => number, seedIncident = true) {
    if (seedIncident) {
      const tpm = theme.ticksPerMinute;
      this.add({ kind: "robot_failure", params: { count: 2 }, atTick: 22 * tpm, durationTicks: 9 * tpm });
      this.add({ kind: "chute_jam", params: { chute: 12 }, atTick: 22.5 * tpm, durationTicks: 6 * tpm });
    }
  }

  add(req: FaultRequest): ActiveFault {
    const f: ActiveFault = { ...req, id: this.nextId++, endTick: req.atTick + req.durationTicks };
    this.faults.push(f);
    return f;
  }

  /** Fraction of baseline lost in minute `m`: window ramp-up, then an exponential recovery tail. */
  lossAt(m: number): number {
    const tpm = theme.ticksPerMinute;
    let keep = 1;
    for (const f of this.faults) {
      const s = f.atTick / tpm;
      const e = f.endTick / tpm;
      const mid = m + 0.5;
      let dip = 0;
      if (mid >= s && mid <= e) dip = severity(f) * Math.min(1, (mid - s) / 1.5 + 0.35);
      else if (mid > e) dip = severity(f) * Math.exp(-(mid - e) / 3.5);
      keep *= 1 - dip;
    }
    return 1 - keep;
  }
}

class MockMetrics implements MetricsFeed {
  constructor(private readonly world: MockWorld, private readonly windowMinutes = 60) {}

  series(): MetricsSeries {
    const tpm = theme.ticksPerMinute;
    const done = Math.floor(this.world.clock() / tpm); // whole minutes elapsed
    const start = Math.max(0, done - this.windowMinutes);
    const delivered: number[] = [];
    const twin: number[] = [];
    for (let m = start; m < done; m++) {
      const base = BASELINE_PER_MINUTE + (hash01(m) - 0.5) * 10;
      twin.push(Math.round(base));
      delivered.push(Math.round(base * (1 - this.world.lossAt(m)) + (hash01(m + 9999) - 0.5) * 3));
    }
    const faults: FaultWindow[] = this.world.faults.map((f) => ({ kind: f.kind, startTick: f.atTick, endTick: f.endTick }));
    return { ticksPerMinute: tpm, startMinute: start, delivered, twin, faults };
  }
}

const PHASE_ORDER: PhaseName[] = ["queued", "awaiting_robot", "loading", "travelling", "blocked", "chute_wait", "drop"];

class MockTraces implements TraceProvider {
  parcelOfRobot(robotId: number): Promise<number | null> {
    return Promise.resolve(1000 + robotId);
  }

  getTrace(parcelId: number): Promise<ParcelTrace | null> {
    if (!Number.isInteger(parcelId) || parcelId < 0) return Promise.resolve(null);
    const tps = theme.ticksPerMinute / 60;
    const durations: Record<PhaseName, number> = {
      queued: 8 + hash01(parcelId) * 20,
      awaiting_robot: 4 + hash01(parcelId + 1) * 10,
      loading: 3 + hash01(parcelId + 2) * 3,
      travelling: 25 + hash01(parcelId + 3) * 30,
      blocked: hash01(parcelId + 4) < 0.7 ? 20 + hash01(parcelId + 5) * 40 : 0,
      chute_wait: 2 + hash01(parcelId + 6) * 8,
      drop: 2 + hash01(parcelId + 7) * 2,
    };
    if (parcelId % 3 === 1) durations.blocked = 41; // the PRODUCT.md demo parcel
    let t = Math.round((200 + parcelId) * tps);
    const spans: TraceSpan[] = [];
    for (const phase of PHASE_ORDER) {
      const len = Math.round(durations[phase] * tps);
      if (len <= 0) continue;
      const span: TraceSpan = { phase, startTick: t, endTick: t + len, directBlocker: null, rootCause: null };
      if (phase === "blocked") {
        span.directBlocker = 112;
        span.blockChain = [87];
        span.rootCause = "dead robot R031";
      }
      spans.push(span);
      t += len;
    }
    return Promise.resolve({ parcelId, robot: parcelId >= 1000 ? parcelId - 1000 : parcelId % 60, spans });
  }
}

class MockFaults implements FaultControl {
  constructor(private readonly world: MockWorld) {}
  kinds(): Promise<FaultKindInfo[]> {
    return Promise.resolve(MOCK_FAULT_KINDS);
  }
  inject(req: FaultRequest): Promise<ActiveFault> {
    return Promise.resolve(this.world.add(req));
  }
  active(nowTick: number): Promise<ActiveFault[]> {
    return Promise.resolve(this.world.faults.filter((f) => nowTick >= f.atTick && nowTick < f.endTick));
  }
}

export const MOCK_REPORT = `# Post-mortem: throughput drop at 14:02

## Summary
Throughput fell **31 %** at 14:02 and took 9 minutes to recover. About 420 parcels were lost.

## Impact
- Lost parcels: **420** (area of the dip against the fault-free twin)
- Maximum dip: **31 %** below baseline
- Time to recover: 9 min

## Root causes
Each cause was replayed with it removed. Shares are of the lost parcels.
1. The **R031** failure in aisle 7 explains **78 %** (replay-verified).
2. The **C12** chute jam explains **19 %** (replay-verified).
3. The remaining 3 % is noise.

| # | Condition | Shapley (parcels) | Share | Replay |
|---|:--|--:|--:|:-:|
| 1 | Dead robot R031 | 412 | **78 %** | R:3 |
| 2 | Chute jam C12 | 100 | 19 % | R:5 |
| 3 | Intrinsic congestion | 16 | 3 % | R:1 |

## What would have helped
Rerouting aisle 7 would have recovered 88 % of the loss.

## Action items
- Add a bypass lane around aisle 7.
- Alert when a robot is stationary for more than 30 s.
- Note: <script>alert(1)</script> is shown as text, never run.
`;

class MockPostmortem implements PostmortemProvider {
  report(): Promise<string> {
    return Promise.resolve(MOCK_REPORT);
  }
}

export function createMockServices(clock: () => number): Services {
  const world = new MockWorld(clock);
  return { metrics: new MockMetrics(world), traces: new MockTraces(), faults: new MockFaults(world), postmortem: new MockPostmortem() };
}
