import type { ParcelTrace, PhaseName, TraceSpan } from "../data/types";

export const PHASE_LABEL: Record<PhaseName, string> = {
  queued: "Queued",
  awaiting_robot: "Waiting for a robot",
  loading: "Loading",
  travelling: "Travelling",
  blocked: "Blocked",
  chute_wait: "Waiting at chute",
  drop: "Dropping",
};

/** "41 s", "2 min 5 s", "3 min". Ticks are converted with ticksPerSecond. */
export function formatDuration(ticks: number, ticksPerSecond = 1): string {
  const total = Math.max(0, Math.round(ticks / ticksPerSecond));
  if (total < 60) return `${total} s`;
  const m = Math.floor(total / 60);
  const s = total % 60;
  return s === 0 ? `${m} min` : `${m} min ${s} s`;
}

/** "R112 → R087 → root: dead robot R031". Null when a blocked span names nobody. */
export function blameChain(span: TraceSpan): string | null {
  const robots = [span.directBlocker, ...(span.blockChain ?? [])].filter((r): r is number => r !== null);
  const parts = robots.map((r) => `R${r}`);
  if (span.rootCause) parts.push(`root: ${span.rootCause}`);
  return parts.length ? parts.join(" → ") : null;
}

/** Plain-language line for one span, e.g. "blocked 41 s · by R112 → R087 → root: dead robot R031". */
export function describeSpan(span: TraceSpan, ticksPerSecond = 1): string {
  const dur = formatDuration(span.endTick - span.startTick, ticksPerSecond);
  if (span.phase !== "blocked") return `${PHASE_LABEL[span.phase].toLowerCase()} ${dur}`;
  const chain = blameChain(span);
  return chain ? `blocked ${dur} · by ${chain}` : `blocked ${dur}`;
}

export interface TraceSummary {
  totalTicks: number;
  blockedTicks: number;
  /** Longest blocked span, if any. */
  worstBlocked: TraceSpan | null;
}

export function summarize(trace: ParcelTrace): TraceSummary {
  let totalTicks = 0;
  let blockedTicks = 0;
  let worst: TraceSpan | null = null;
  for (const s of trace.spans) {
    const len = s.endTick - s.startTick;
    totalTicks += len;
    if (s.phase === "blocked") {
      blockedTicks += len;
      if (!worst || len > worst.endTick - worst.startTick) worst = s;
    }
  }
  return { totalTicks, blockedTicks, worstBlocked: worst };
}

/** Parse the parcel search box: digits only (optionally prefixed with P or #). */
export function parseParcelId(input: string): number | null {
  const m = /^\s*[p#]?\s*(\d{1,9})\s*$/i.exec(input);
  return m ? Number(m[1]) : null;
}
