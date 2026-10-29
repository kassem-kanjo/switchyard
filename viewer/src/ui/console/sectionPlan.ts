/**
 * Which report sections the inspector shows, in the mockup's order: Causes (open),
 * Fixes tested, Impact, How the causes interacted, Method. The Summary repeats the card
 * in report jargon, so it is dropped; the remaining sections follow, collapsed.
 */
import type { ReportSection } from "../reportSections";

const find = (secs: ReportSection[], prefix: string): ReportSection | undefined =>
  secs.find((s) => s.title.toLowerCase().startsWith(prefix));

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

export function inspectorSections(all: ReportSection[]): ReportSection[] {
  const causes = find(all, "root causes");
  const withoutInteractions = causes
    ? { ...causes, title: "Causes", open: true, body: causes.body.split("\n").filter((l) => !l.startsWith("Interaction:")).join("\n") }
    : undefined;
  const fixes = find(all, "what would have helped");
  const impact = find(all, "impact");
  const method = find(all, "method");
  const used = new Set([causes, fixes, impact, method, find(all, "summary")]);
  const planned = [
    withoutInteractions,
    fixes && { ...fixes, title: "Fixes tested", open: false },
    impact && { ...impact, open: false },
    interactions(causes) ?? undefined,
    method && { ...method, open: false },
  ].filter((s): s is ReportSection => s !== undefined);
  return [...planned, ...all.filter((s) => !used.has(s)).map((s) => ({ ...s, open: false }))];
}
