import { theme } from "../theme";
import type { MetricsSeries } from "./types";

/** A series with no data yet (before the first minute completes). */
export function emptySeries(): MetricsSeries {
  return { ticksPerMinute: theme.ticksPerMinute, startMinute: 0, delivered: [], faults: [] };
}
