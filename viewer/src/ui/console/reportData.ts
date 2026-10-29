/**
 * Structured console data read from the post-mortem report the engine writes
 * (the same Markdown the panel shows). The report is the single source: the
 * rail, inspector pages and timeline spans all come from its tables, so no
 * number is typed in here. Text is only ever read, never rendered as HTML.
 */
import { splitReport, type HeadlineCard, type ReportSection } from "../reportSections";
import type { RailItem, TimelineSpan } from "./types";

export interface Cause {
  /** Fault id from the report, e.g. "F0". */
  id: string;
  /** The report's wording, e.g. "chute C289 jams". */
  label: string;
  sharePct: number;
  bandPct: number;
  /** Shapley share of the loss, parcel-minutes. */
  parcelMin: number;
  /** Parcel-minutes recovered by removing only this cause. */
  aloneMin: number;
  /** True when the report calls it a tie with the row above (within noise). */
  tied: boolean;
  startMin: number | null;
  endMin: number | null;
}

export interface Fix {
  /** "X1".. in the report's table order (best first). */
  id: string;
  label: string;
  /** Plain short phrase for the rail; the full label stays in the fix card. */
  short: string;
  recoveredMin: number;
  bandMin: number;
  recoveryPct: number;
  withinNoise: boolean;
  cost: string;
}

export interface ConsoleData {
  card: HeadlineCard | null;
  sections: ReportSection[];
  incident: { startMin: number; endMin: number; dipPct: number | null } | null;
  causes: Cause[];
  fixes: Fix[];
}

const clock = (s: string): number | null => {
  const m = /(\d+):(\d{2})/.exec(s);
  return m ? Number(m[1]) * 60 + Number(m[2]) : null;
};
const num = (s: string | undefined): number => {
  const m = /-?\d[\d,]*(?:\.\d+)?/.exec(s ?? "");
  return m ? Number(m[0].replace(/,/g, "")) : 0;
};
const plusMinus = (s: string | undefined): { v: number; band: number } => {
  const m = /(-?\d[\d,]*(?:\.\d+)?)(?:\s*%)?(?:\s*±\s*(\d[\d,]*(?:\.\d+)?))?/.exec(s ?? "");
  return { v: m ? Number(m[1]!.replace(/,/g, "")) : 0, band: m?.[2] ? Number(m[2].replace(/,/g, "")) : 0 };
};

/** Markdown table -> rows of cells keyed by lower-case header. */
export function parseTable(md: string): Record<string, string>[] {
  const lines = md.split("\n").map((l) => l.trim()).filter((l) => l.startsWith("|"));
  if (lines.length < 3) return [];
  const cells = (l: string) => l.replace(/^\||\|$/g, "").split("|").map((c) => c.trim().replace(/\*+/g, ""));
  const head = cells(lines[0]!).map((c) => c.toLowerCase());
  return lines.slice(2).map((l) => {
    const row: Record<string, string> = {};
    cells(l).forEach((c, i) => (row[head[i] ?? String(i)] = c));
    return row;
  });
}

const section = (sections: ReportSection[], prefix: string) => sections.find((s) => s.title.toLowerCase().startsWith(prefix));

/** Fault start and end minutes from the report's Timeline table ("F0 starts: ..." / "F0 ends"). */
function faultTimes(sections: ReportSection[]): Map<string, { start?: number; end?: number }> {
  const out = new Map<string, { start?: number; end?: number }>();
  const body = section(sections, "timeline")?.body ?? "";
  for (const row of parseTable(body)) {
    const m = /^(F\d+)\s+(starts|ends)/.exec(row["event"] ?? "");
    if (!m) continue;
    const t = out.get(m[1]!) ?? {};
    t[m[2] === "starts" ? "start" : "end"] = clock(row["time"] ?? "") ?? undefined;
    out.set(m[1]!, t);
  }
  return out;
}

/** The short plain phrase for a fix, from the report's wording; falls back to the full label. */
export function shortFix(label: string): string {
  let m = /send chute (\w+)'s destination to (\w+)/i.exec(label);
  if (m) return `Reroute ${m[1]} to ${m[2]}`;
  m = /clear failed robots.*?within (\d+) min/i.exec(label);
  if (m) return `Clear dead robots in ${m[1]} min`;
  m = /allow (\d+) robots inbound per station/i.exec(label);
  if (m) return `Allow ${m[1]} robots per station`;
  m = /release (\d+) spare robots/i.exec(label);
  if (m) return `Add ${m[1]} spare robots`;
  return capitalise(label);
}

export function parseReport(md: string): ConsoleData {
  const { card, sections } = splitReport(md);
  const eye = card?.eyebrow ?? "";
  const win = /(\d+:\d{2})\s*[–-]\s*(\d+:\d{2})/.exec(eye);
  const dip = /(\d+(?:\.\d+)?)\s*%\s*below/.exec(eye);
  const times = faultTimes(sections);

  const causes: Cause[] = [];
  for (const r of parseTable(section(sections, "root causes")?.body ?? "")) {
    const cond = r["condition"] ?? "";
    const id = /\((F\d+)\)/.exec(cond)?.[1];
    if (!id) continue;
    const share = plusMinus(r["share"]);
    const t = times.get(id);
    causes.push({
      id,
      label: cond.replace(/\s*\(F\d+\)/, "").replace(/\s*\(tie[^)]*\)/, "").trim(),
      sharePct: share.v,
      bandPct: share.band,
      parcelMin: plusMinus(r["shapley (parcel-min)"]).v,
      aloneMin: num(r["alone"]),
      tied: /\(tie/.test(cond),
      startMin: t?.start ?? null,
      endMin: t?.end ?? null,
    });
  }

  const fixes: Fix[] = parseTable(section(sections, "what would have helped")?.body ?? "").map((r, i) => {
    const gain = plusMinus(r["parcel-min recovered"]);
    return {
      id: `X${i + 1}`,
      label: r["fix"] ?? "",
      short: shortFix(r["fix"] ?? ""),
      recoveredMin: gain.v,
      bandMin: gain.band,
      recoveryPct: num(r["share of loss"]),
      withinNoise: /within noise/.test(r["parcel-min recovered"] ?? ""),
      cost: r["cost"] ?? "",
    };
  });

  return {
    card,
    sections,
    incident: win ? { startMin: clock(win[1]!)!, endMin: clock(win[2]!)!, dipPct: dip ? Number(dip[1]) : null } : null,
    causes,
    fixes,
  };
}

export const capitalise = (s: string): string => s.charAt(0).toUpperCase() + s.slice(1);

/** "00:30" for whole minutes, matching the report's clock. */
export const clockOf = (min: number): string => `${String(Math.floor(min / 60)).padStart(2, "0")}:${String(Math.round(min % 60)).padStart(2, "0")}`;

const stat = (card: HeadlineCard | null, caption: string): string | null => card?.stats.find((s) => s.caption.toLowerCase().startsWith(caption))?.value ?? null;

/** Rail rows: the incident, each cause with its share bar, each fix with its recovery bar. */
export function railItems(d: ConsoleData): RailItem[] {
  if (!d.incident) return [];
  const items: RailItem[] = [];
  const late = stat(d.card, "parcels late");
  items.push({
    id: "inc",
    kind: "incident",
    title: "Throughput drop",
    subtitle: [`${clockOf(d.incident.startMin)}–${clockOf(d.incident.endMin)}`, `${d.causes.length} causes`, ...(late ? [`${late} parcels late`] : [])].join(" · "),
    tone: "danger",
  });
  d.causes.forEach((c, i) => {
    const tieNote = c.tied && i > 0 ? ` · tied with ${d.causes[i - 1]!.id}` : "";
    items.push({
      id: c.id,
      kind: "cause",
      tag: c.id,
      title: capitalise(c.label),
      subtitle: `${c.startMin !== null ? clockOf(c.startMin) + " · " : ""}${c.sharePct} % of the delay${tieNote}`,
      sharePct: c.sharePct,
      tone: "danger",
    });
  });
  for (const f of d.fixes) {
    items.push({
      id: f.id,
      kind: "fix",
      title: f.short,
      subtitle: f.withinNoise ? "no measurable effect" : `would save ${f.recoveryPct} %`,
      sharePct: f.withinNoise ? 0 : f.recoveryPct,
      tone: f.withinNoise ? "muted" : "ok",
    });
  }
  return items;
}

/** One span per fault, for the timeline. Faults with no known window are left out. */
export function faultSpans(d: ConsoleData): TimelineSpan[] {
  return d.causes.flatMap((c) =>
    c.startMin !== null && c.endMin !== null ? [{ id: c.id, label: `${c.id} ${c.label}`, startMin: c.startMin, endMin: c.endMin }] : [],
  );
}
