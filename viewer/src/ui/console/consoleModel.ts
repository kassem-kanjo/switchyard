/** Pure glue between the sim feeds, the report data and the console panes (no DOM, unit-tested). */
import type { MetricsSeries } from "../../data/types";
import { faultSpans, type ConsoleData } from "./reportData";
import type { ObjectId, TimelineModel, TimelineSpan } from "./types";

/** `HH:MM:SS` of shift time for a sim tick. */
export function shiftClock(tick: number, ticksPerMinute: number): string {
  const s = Math.floor((tick * 60) / ticksPerMinute);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(Math.floor(s / 3600))}:${p(Math.floor((s % 3600) / 60))}:${p(s % 60)}`;
}

/** Latest finished minute against the fault-free twin, in whole %, or null when there is no twin to compare with. */
export function dipVsTwin(series: MetricsSeries): number | null {
  const i = series.delivered.length - 2; // the last entry is the minute still in progress
  const twin = series.twin?.[i];
  const actual = series.delivered[i];
  if (i < 0 || twin === undefined || actual === undefined || twin <= 0) return null;
  return Math.round((actual / twin - 1) * 100);
}

/** Spans for the timeline: the report's faults when there is a report, else the faults that ran in this session. */
export function spansFor(series: MetricsSeries, data: ConsoleData | null): TimelineSpan[] {
  const fromReport = data ? faultSpans(data) : [];
  if (fromReport.length > 0) return fromReport;
  return series.faults.map((f, i) => ({
    id: `live${i}`,
    label: f.kind,
    startMin: f.startTick / series.ticksPerMinute,
    endMin: f.endTick / series.ticksPerMinute,
  }));
}

/** Which spans light up for the selected object: all faults for the incident, the one fault for a cause, none for a fix. */
export function highlightFor(selected: ObjectId | null, spans: readonly TimelineSpan[], data: ConsoleData | null): ObjectId[] {
  if (selected === "inc") return spans.map((s) => s.id);
  return data?.causes.some((c) => c.id === selected) ? [selected as ObjectId] : [];
}

/** X axis ends on a half-hour so the scale does not creep while the shift runs. */
export function axisEnd(seriesEndMin: number, incidentEndMin: number | null): number {
  return Math.ceil(Math.max(seriesEndMin + 1, (incidentEndMin ?? 0) + 20, 30) / 30) * 30;
}

export function consoleTimeline(series: MetricsSeries, data: ConsoleData | null, selected: ObjectId | null, nowTick: number): TimelineModel {
  const spans = spansFor(series, data);
  const incident = data?.incident ?? null;
  return {
    totalMin: axisEnd(series.startMinute + series.delivered.length, incident?.endMin ?? null),
    startMin: series.startMinute,
    actual: series.delivered,
    twin: series.twin,
    incident: incident ? { startMin: incident.startMin, endMin: incident.endMin } : null,
    spans,
    highlight: highlightFor(selected, spans, data),
    cursorMin: nowTick / series.ticksPerMinute,
    incidentCaption: incident?.dipPct != null ? `−${incident.dipPct} %` : "",
  };
}
