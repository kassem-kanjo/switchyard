import { CellKind, RobotState, type LayoutView, type SnapshotView } from "../renderer/Renderer";

/** Narrow seam; a WASM/Worker-backed source will implement this later. */
export interface SimSource {
  layout(): LayoutView;
  step(n: number): void;
  snapshot(): SnapshotView;
}

/** Small deterministic PRNG (mulberry32). */
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export interface MockOptions {
  width?: number;
  height?: number;
  robots?: number;
  seed?: number;
}

export class MockSource implements SimSource {
  private readonly lay: LayoutView;
  private readonly xy: Uint16Array;
  private readonly state: Uint8Array;
  private readonly occ: Uint8Array;
  private readonly rand: () => number;
  private tick = 0;

  constructor(opts: MockOptions = {}) {
    const width = opts.width ?? 40;
    const height = opts.height ?? 28;
    this.rand = rng(opts.seed ?? 1);
    const cells = new Uint8Array(width * height);
    for (let y = 0; y < height; y++) {
      for (let x = 0; x < width; x++) {
        const border = x === 0 || y === 0 || x === width - 1 || y === height - 1;
        let k: number = border ? CellKind.Wall : CellKind.Free;
        if (!border && x % 3 === 0 && y % 3 === 0) k = CellKind.Chute;
        if (y === height - 1 && x % 4 === 2 && x > 0 && x < width - 1) k = CellKind.Induction;
        cells[y * width + x] = k;
      }
    }
    this.lay = { width, height, cells };

    const free: number[] = [];
    cells.forEach((k, i) => {
      if (k === CellKind.Free) free.push(i);
    });
    const n = Math.min(opts.robots ?? 60, free.length);
    for (let i = free.length - 1; i > 0; i--) {
      const j = Math.floor(this.rand() * (i + 1));
      [free[i], free[j]] = [free[j]!, free[i]!];
    }
    this.xy = new Uint16Array(2 * n);
    this.state = new Uint8Array(n);
    this.occ = new Uint8Array(width * height);
    for (let i = 0; i < n; i++) {
      const c = free[i]!;
      this.xy[2 * i] = c % width;
      this.xy[2 * i + 1] = Math.floor(c / width);
      this.state[i] = i % RobotState.Failed; // 0..4, cosmetic only
      this.occ[c] = 1;
    }
  }

  layout(): LayoutView {
    return this.lay;
  }

  step(n: number): void {
    const { width, height, cells } = this.lay;
    const count = this.state.length;
    const dx = [1, -1, 0, 0, 0];
    const dy = [0, 0, 1, -1, 0];
    for (let t = 0; t < n; t++) {
      // Sequential moves against live occupancy: no overlaps, no swaps.
      for (let i = 0; i < count; i++) {
        const d = Math.floor(this.rand() * 5);
        const x = this.xy[2 * i]!;
        const y = this.xy[2 * i + 1]!;
        const nx = x + dx[d]!;
        const ny = y + dy[d]!;
        if (nx < 0 || ny < 0 || nx >= width || ny >= height) continue;
        const ni = ny * width + nx;
        if (cells[ni] !== CellKind.Free || this.occ[ni]) continue;
        this.occ[y * width + x] = 0;
        this.occ[ni] = 1;
        this.xy[2 * i] = nx;
        this.xy[2 * i + 1] = ny;
        if (this.rand() < 0.02) this.state[i] = Math.floor(this.rand() * RobotState.Failed);
      }
      this.tick++;
    }
  }

  snapshot(): SnapshotView {
    return { tick: this.tick, robotXY: this.xy, robotState: this.state };
  }
}
