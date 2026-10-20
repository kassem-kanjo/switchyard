import { describe, expect, it } from "vitest";
import { CellKind } from "../renderer/Renderer";
import { MockSource } from "./source";

describe("MockSource", () => {
  it("builds a lattice with walls, chutes and south inductions", () => {
    const { width, height, cells } = new MockSource().layout();
    expect(cells.length).toBe(width * height);
    expect(cells[0]).toBe(CellKind.Wall);
    expect(cells[3 * width + 3]).toBe(CellKind.Chute);
    expect(cells[(height - 1) * width + 2]).toBe(CellKind.Induction);
  });

  it("keeps robots in bounds, on free cells, and never overlapping", () => {
    const s = new MockSource({ robots: 80, seed: 7 });
    const { width, height, cells } = s.layout();
    for (let t = 0; t < 300; t++) {
      s.step(1);
      const snap = s.snapshot();
      const seen = new Set<number>();
      for (let i = 0; i < snap.robotState.length; i++) {
        const x = snap.robotXY[2 * i]!;
        const y = snap.robotXY[2 * i + 1]!;
        expect(x).toBeGreaterThanOrEqual(0);
        expect(x).toBeLessThan(width);
        expect(y).toBeGreaterThanOrEqual(0);
        expect(y).toBeLessThan(height);
        expect(cells[y * width + x]).toBe(CellKind.Free);
        const key = y * width + x;
        expect(seen.has(key)).toBe(false);
        seen.add(key);
      }
    }
  });

  it("advances the tick counter and is deterministic per seed", () => {
    const a = new MockSource({ seed: 3 });
    const b = new MockSource({ seed: 3 });
    a.step(10);
    b.step(10);
    expect(a.snapshot().tick).toBe(10);
    expect(Array.from(a.snapshot().robotXY)).toEqual(Array.from(b.snapshot().robotXY));
  });
});
