import type { FaultKindInfo, FaultRequest } from "../data/types";

export type ValidationResult =
  | { ok: true; request: FaultRequest }
  | { ok: false; errors: Record<string, string> };

export const DURATION_KEY = "duration";
export const MIN_DURATION_S = 10;
export const MAX_DURATION_S = 3600;

function parseNum(raw: string | number | undefined): number {
  if (typeof raw === "number") return raw;
  if (raw === undefined || raw.trim() === "") return NaN;
  return Number(raw);
}

/**
 * Check form input against a fault kind's parameter specs and build the request.
 * `values` holds the raw field text; `durationSeconds` is the duration field.
 * Errors are keyed by param key (or DURATION_KEY) so the form can mark the field.
 */
export function validateFault(
  kind: FaultKindInfo,
  values: Record<string, string | number | undefined>,
  durationSeconds: string | number | undefined,
  atTick: number,
  ticksPerSecond: number,
): ValidationResult {
  const errors: Record<string, string> = {};
  const params: Record<string, number> = {};
  for (const spec of kind.params) {
    const v = parseNum(values[spec.key]);
    const unit = spec.unit ? ` ${spec.unit}` : "";
    if (!Number.isFinite(v)) errors[spec.key] = "Enter a number.";
    else if (spec.integer && !Number.isInteger(v)) errors[spec.key] = "Use a whole number.";
    else if (v < spec.min || v > spec.max) errors[spec.key] = `Use ${spec.min} to ${spec.max}${unit}.`;
    else params[spec.key] = v;
  }
  const d = parseNum(durationSeconds);
  if (!Number.isFinite(d)) errors[DURATION_KEY] = "Enter a number of seconds.";
  else if (d < MIN_DURATION_S || d > MAX_DURATION_S) errors[DURATION_KEY] = `Use ${MIN_DURATION_S} to ${MAX_DURATION_S} s.`;
  if (Object.keys(errors).length) return { ok: false, errors };
  return { ok: true, request: { kind: kind.id, params, atTick, durationTicks: Math.round(d * ticksPerSecond) } };
}

/** Remaining fraction (0..1) of a fault's window at `nowTick`. */
export function remainingFraction(f: { atTick: number; endTick: number }, nowTick: number): number {
  const total = f.endTick - f.atTick;
  if (total <= 0) return 0;
  return Math.min(1, Math.max(0, (f.endTick - nowTick) / total));
}
