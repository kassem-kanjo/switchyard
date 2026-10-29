/** Shared shapes for the operations console panes (Rail, Inspector, Timeline). Presentation only: no sim or engine imports. */

/** What a rail item or an inspector page represents. */
export type ObjectKind = "incident" | "cause" | "fix";

/** Object ids: "inc" for the incident, the fault id ("F0") for a cause, the fix id ("X2") for a fix. */
export type ObjectId = string;

export type Tone = "danger" | "ok" | "muted";

/** One selectable row in the left rail. */
export interface RailItem {
  id: ObjectId;
  kind: ObjectKind;
  /** Small mono tag before the title, e.g. "F0". */
  tag?: string;
  title: string;
  /** Second line, plain language, e.g. "00:30 · 70 % of the delay". */
  subtitle: string;
  /** 0-100 for a share bar under the row; omit for no bar. */
  sharePct?: number;
  tone: Tone;
}

/** A time span drawn under the throughput line (one per fault). Minutes of shift time. */
export interface TimelineSpan {
  id: ObjectId;
  label: string;
  startMin: number;
  endMin: number;
}

/** Everything the timeline draws. Minutes are shift minutes; `actual[i]` is minute `startMin + i`. */
export interface TimelineModel {
  /** Right edge of the x axis, in minutes. */
  totalMin: number;
  startMin: number;
  /** Delivered per minute, real run. */
  actual: number[];
  /** Delivered per minute, fault-free twin ("normal"); may be shorter than `actual` or absent. */
  twin?: number[];
  /** Incident window, or null. */
  incident: { startMin: number; endMin: number } | null;
  spans: TimelineSpan[];
  /** Span ids to emphasise (the selected object); empty for none. */
  highlight: ObjectId[];
  /** Current shift time in minutes (the scrub line); null hides it. */
  cursorMin: number | null;
  /** Short caption drawn on the incident band, e.g. "-47 %"; empty for none. */
  incidentCaption: string;
}
