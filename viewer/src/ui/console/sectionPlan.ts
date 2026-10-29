/**
 * Which report sections the inspector shows, in the mockup's order: Causes (open),
 * Fixes tested, Impact, How the causes interacted, Method. The Summary repeats the card
 * in report jargon, so it is dropped; the remaining sections follow, collapsed.
 */
import type { ReportSection } from "../reportSections";
import { capitalise, parseTable } from "./reportData";

const find = (secs: ReportSection[], prefix: string): ReportSection | undefined =>
  secs.find((s) => s.title.toLowerCase().startsWith(prefix));

/** The causes section's body without its "Interaction:" lines (those get a section of their own). */
const withoutInteractions = (body: string): string =>
  body
    .split("\n")
    .filter((l) => !l.startsWith("Interaction:"))
    .join("\n");

/** "Interaction: ..." lines of the causes section, as a section of their own. */
function interactions(causes: ReportSection | undefined): ReportSection | null {
  const lines = (causes?.body ?? "").split("\n").filter((l) => l.startsWith("Interaction:"));
  if (lines.length === 0) return null;
  return {
    title: "How the causes interacted",
    teaser: `${lines.length} ${lines.length === 1 ? "pair compounds" : "pairs compound"}`,
    open: false,
    body: lines.map((l) => l.replace(/^Interaction:\s*/, "")).join("\n\n"),
  };
}

/** The mockup's three-column causes table: plain name, share with its error bar, parcel-minutes. */
function causesTable(body: string): string {
  const rows = parseTable(body).map((r) => {
    const cond = r["condition"] ?? "";
    const name = capitalise(cond.replace(/\s*\(F\d+\)/, "").replace(/\s*\(tie[^)]*\)/, "").trim());
    const tie = /\(tie/.test(cond) ? " (tie)" : "";
    const share = (r["share"] ?? "").replace(/\s*%\s*±\s*(\d+)\s*%/, " % ± $1");
    const pm = (r["shapley (parcel-min)"] ?? "").replace(/\s*±.*/, "");
    return `| ${name}${tie} | ${share} | ${pm} |`;
  });
  return rows.length === 0 ? body : ["| Cause | Share | Parcel-min |", "|---|---|---|", ...rows].join("\n");
}

export function inspectorSections(all: ReportSection[]): ReportSection[] {
  const causes = find(all, "root causes");
  const fixes = find(all, "what would have helped");
  const impact = find(all, "impact");
  const method = find(all, "method");
  const used = new Set([causes, fixes, impact, method, find(all, "summary")]);
  const planned = [
    causes && { ...causes, title: "Causes", open: true, body: causesTable(causes.body) },
    fixes && { ...fixes, title: "Fixes tested", open: false },
    impact && { ...impact, open: false },
    interactions(causes) ?? undefined,
    // The Shapley explanation and the Alone/Last columns live in Method, where a reader looks for how it was made.
    method && { ...method, open: false, body: `${method.body}\n\n**How the shares were made.**\n\n${withoutInteractions(causes?.body ?? "")}` },
  ].filter((s): s is ReportSection => s !== undefined);
  return [...planned, ...all.filter((s) => !used.has(s)).map((s) => ({ ...s, open: false }))];
}
