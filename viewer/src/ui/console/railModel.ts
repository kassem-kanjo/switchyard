/** Pure rail logic: grouping, bar widths and keyboard order. No DOM, so it is cheap to test. */

import type { ObjectId, ObjectKind, RailItem } from "./types";

/** Display order of the rail groups. */
export const RAIL_GROUPS: { kind: ObjectKind; title: string }[] = [
  { kind: "incident", title: "Incidents" },
  { kind: "cause", title: "Causes" },
  { kind: "fix", title: "Fixes tested" },
];

/** Groups in RAIL_GROUPS order, preserving input order inside a group; empty groups are dropped. */
export function groupItems(
  items: RailItem[],
): { title: string; kind: ObjectKind; items: RailItem[] }[] {
  // Walk the groups (not the input) so the display order is fixed regardless of input order.
  return RAIL_GROUPS.map((group) => ({
    ...group,
    items: items.filter((item) => item.kind === group.kind),
  })).filter((group) => group.items.length > 0);
}

/** Share bar width: clamped to 0..100, 0 when absent, and a 2 % floor so tiny shares stay visible. */
export function barWidthPct(item: RailItem): number {
  const raw = item.sharePct;
  if (raw === undefined || !Number.isFinite(raw)) return 0;
  const pct = Math.min(100, Math.max(0, raw));
  // A sub-2 % bar would round away to nothing; keep it as a visible hairline instead.
  return pct > 0 && pct < 2 ? 2 : pct;
}

/** Next id in display order (grouped, wrapping). Null for an empty rail; an unknown current acts like null. */
export function nextId(
  items: RailItem[],
  current: ObjectId | null,
  delta: 1 | -1,
): ObjectId | null {
  const flat = groupItems(items).flatMap((group) => group.items);
  if (flat.length === 0) return null;

  const index = current === null ? -1 : flat.findIndex((item) => item.id === current);
  // No current (or it vanished): +1 starts at the top, -1 at the bottom.
  if (index === -1) return (delta === 1 ? flat[0] : flat[flat.length - 1])?.id ?? null;

  const next = (index + delta + flat.length) % flat.length;
  return flat[next]?.id ?? null;
}
