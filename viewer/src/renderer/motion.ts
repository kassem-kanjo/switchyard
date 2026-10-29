/** Interpolates robot positions between simulation snapshots and records short trails. */
export const TRAIL_CELLS = 5;

const DEFAULT_INTERVAL_MS = 500;
const MIN_INTERVAL_MS = 16;
const MAX_INTERVAL_MS = 2000;
const INTERVAL_EMA = 0.3;
/** A jump further than this many cells is a teleport/fast-forward, so snap instead of gliding. */
const TELEPORT_CELLS = 3;

interface Cell {
  x: number;
  y: number;
}

interface Snapshot {
  tick: number;
  cells: Cell[];
}

function clamp(value: number, min: number, max: number): number {
  return value < min ? min : value > max ? max : value;
}

export class MotionTracker {
  private previous: Snapshot | null = null;
  private current: Snapshot | null = null;
  private arrivedMs = 0;
  private interval = DEFAULT_INTERVAL_MS;
  private trails: Array<Cell[] | undefined> = [];

  update(tick: number, xy: ArrayLike<number>, nowMs: number): void {
    const count = Math.floor(xy.length / 2);
    // draw() runs every animation frame; the same snapshot must not restart the animation.
    if (this.current && this.current.tick === tick && this.current.cells.length === count) return;

    const cells: Cell[] = [];
    for (let i = 0; i < count; i++) cells.push({ x: xy[2 * i]!, y: xy[2 * i + 1]! });

    const sameCount = this.current !== null && this.current.cells.length === count;
    if (this.current && sameCount) {
      for (let i = 0; i < count; i++) {
        const prev = this.current.cells[i]!;
        const next = cells[i]!;
        // Standing still (or revisiting the same cell) leaves no trail.
        if (prev.x === next.x && prev.y === next.y) continue;
        const trail = (this.trails[i] ??= []);
        const last = trail[trail.length - 1];
        if (last && last.x === prev.x && last.y === prev.y) continue;
        trail.push(prev);
        if (trail.length > TRAIL_CELLS) trail.shift();
      }
      const measured = nowMs - this.arrivedMs;
      this.interval = clamp(this.interval + INTERVAL_EMA * (measured - this.interval), MIN_INTERVAL_MS, MAX_INTERVAL_MS);
      this.previous = this.current;
    } else {
      this.previous = null;
      this.trails = [];
    }

    this.trails.length = count;
    this.current = { tick, cells };
    this.arrivedMs = nowMs;
  }

  position(i: number, nowMs: number, instant = false): { x: number; y: number } {
    const current = this.current;
    const cell = current?.cells[i];
    if (!cell) return { x: 0, y: 0 };

    const previous = this.previous?.cells[i];
    if (instant || !previous) return { x: cell.x, y: cell.y };
    if (Math.hypot(previous.x - cell.x, previous.y - cell.y) > TELEPORT_CELLS) return { x: cell.x, y: cell.y };

    const t = clamp((nowMs - this.arrivedMs) / this.interval, 0, 1);
    return {
      x: previous.x + (cell.x - previous.x) * t,
      y: previous.y + (cell.y - previous.y) * t,
    };
  }

  trail(i: number): ReadonlyArray<{ x: number; y: number }> {
    return this.trails[i] ?? [];
  }

  count(): number {
    return this.current?.cells.length ?? 0;
  }
}
