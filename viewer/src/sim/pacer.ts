export const TICKS_PER_SECOND_AT_1X = 2;
export const SPEEDS = [1, 4, 16, 64] as const;

const DEFAULT_MAX_BURST = 256;
// Per-frame deltas sum to slightly less than a whole second (60 frames of
// 1000/60 ms), so floor() alone would drop a tick. Absorb that drift.
const EPSILON = 1e-9;

export class TickPacer {
  private readonly maxBurst: number;
  private lastMs: number | null = null;
  private acc = 0;

  /** maxBurst caps ticks returned by one call (default 256) so a stalled tab does not fast-forward. */
  constructor(maxBurst: number = DEFAULT_MAX_BURST) {
    this.maxBurst = Math.max(0, Math.floor(maxBurst));
  }

  /** Whole ticks due since the previous call, at `speed` x real time. Keeps the fractional remainder for the next call. The first call (and the first call after reset()) returns 0 and just records nowMs. A nowMs earlier than the last one returns 0. */
  take(nowMs: number, speed: number): number {
    if (this.lastMs === null) {
      this.lastMs = nowMs;
      return 0;
    }
    // Also rejects NaN, and leaves the clock untouched so the next valid call
    // is measured from the last good timestamp.
    if (!(nowMs >= this.lastMs)) return 0;

    const dtMs = nowMs - this.lastMs;
    this.lastMs = nowMs;

    if (!(speed > 0)) return 0; // zero, negative or NaN

    this.acc += (dtMs / 1000) * TICKS_PER_SECOND_AT_1X * speed;

    const ticks = Math.floor(this.acc + EPSILON);
    if (ticks <= 0) {
      if (this.acc < 0) this.acc = 0;
      return 0;
    }
    if (ticks > this.maxBurst) {
      this.acc = 0; // drop the excess rather than paying it back later
      return this.maxBurst;
    }

    this.acc -= ticks;
    if (Math.abs(this.acc) < EPSILON) this.acc = 0;
    return ticks;
  }

  /** Forget time and remainder (call after pause/resume). */
  reset(): void {
    this.lastMs = null;
    this.acc = 0;
  }
}
