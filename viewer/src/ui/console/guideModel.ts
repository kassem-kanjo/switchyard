/** Guide strip logic: the three steps, dismissal persistence, and step labelling. */

export const GUIDE_STEPS = ["Incident", "Trace a late parcel", "Read the post-mortem"] as const;

export const GUIDE_KEY = "switchyard.guide.dismissed";

export interface KV {
  getItem(k: string): string | null;
  setItem(k: string, v: string): void;
}

const CIRCLED = ["①", "②", "③"] as const;

/**
 * The store to use: the injected one, else the browser's localStorage.
 * Reading globalThis.localStorage can itself throw (privacy modes), so it stays inside try/catch.
 */
function resolveStore(store?: KV): KV | null {
  if (store) return store;
  try {
    return (globalThis as { localStorage?: KV }).localStorage ?? null;
  } catch {
    return null;
  }
}

/** True only when the guide was explicitly dismissed; any failure counts as "not dismissed". */
export function isDismissed(store?: KV): boolean {
  const kv = resolveStore(store);
  if (!kv) return false;
  try {
    return kv.getItem(GUIDE_KEY) === "1";
  } catch {
    return false;
  }
}

/** Persist the dismissal. Never throws, whatever the store does. */
export function dismiss(store?: KV): void {
  const kv = resolveStore(store);
  if (!kv) return;
  try {
    kv.setItem(GUIDE_KEY, "1");
  } catch {
    // Storage may be unavailable or full; dismissal is best effort.
  }
}

/** Label parts for step `i` while `current` is active (both 0-based). */
export function stepLabel(i: number, current: number): { n: string; text: string; bold: boolean } {
  return {
    n: CIRCLED[i] ?? String(i + 1),
    text: GUIDE_STEPS[i] ?? "",
    bold: i === current,
  };
}
