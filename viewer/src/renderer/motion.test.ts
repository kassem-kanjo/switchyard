import { describe, expect, it } from "vitest";
import { MotionTracker, TRAIL_CELLS } from "./motion";

const xy = (...coords: number[]) => Uint16Array.from(coords);

describe("MotionTracker.position", () => {
  it("interpolates halfway between the previous and current cell", () => {
    const m = new MotionTracker();
    m.update(1, xy(0, 0), 0);
    m.update(2, xy(2, 2), 500);

    const p = m.position(0, 750);

    expect(p.x).toBeCloseTo(1, 10);
    expect(p.y).toBeCloseTo(1, 10);
  });

  it("clamps t at 1 once the interval has elapsed", () => {
    const m = new MotionTracker();
    m.update(1, xy(0, 0), 0);
    m.update(2, xy(2, 2), 500);

    const p = m.position(0, 5000);

    expect(p).toEqual({ x: 2, y: 2 });
  });

  it("uses the current cell before any previous snapshot exists", () => {
    const m = new MotionTracker();
    m.update(1, xy(4, 7), 0);

    expect(m.position(0, 0)).toEqual({ x: 4, y: 7 });
  });

  it("estimates the tick interval with an EMA of 0.3", () => {
    const m = new MotionTracker();
    m.update(1, xy(0, 0), 0);
    m.update(2, xy(2, 0), 100); // interval = 500 + 0.3 * (100 - 500) = 380

    expect(m.position(0, 290).x).toBeCloseTo(1, 10);
  });

  it("clamps the estimated interval to 16..2000 ms", () => {
    const fast = new MotionTracker();
    fast.update(0, xy(0, 0), 0);
    // Converge the EMA towards a 1 ms tick, which is below the 16 ms floor.
    for (let step = 1; step <= 60; step++) fast.update(step, xy(step % 2, 0), step);
    expect(fast.position(0, 68).x).toBeCloseTo(0.5, 10); // t = 8 / 16

    const slow = new MotionTracker();
    slow.update(1, xy(0, 0), 0);
    slow.update(2, xy(2, 0), 10_000);
    expect(slow.position(0, 11_000).x).toBeCloseTo(1, 10); // t = 1000 / 2000
  });

  it("snaps to the current cell on a teleport", () => {
    const m = new MotionTracker();
    m.update(1, xy(0, 0), 0);
    m.update(2, xy(10, 0), 500);

    const p = m.position(0, 750);

    expect(p).toEqual({ x: 10, y: 0 });
  });

  it("returns the current cell when instant is true", () => {
    const m = new MotionTracker();
    m.update(1, xy(0, 0), 0);
    m.update(2, xy(2, 2), 500);

    const p = m.position(0, 750, true);

    expect(p).toEqual({ x: 2, y: 2 });
  });

  it("copies the source arrays", () => {
    const source = xy(0, 0);
    const m = new MotionTracker();
    m.update(1, source, 0);
    source[0] = 9;
    source[1] = 9;

    expect(m.position(0, 0)).toEqual({ x: 0, y: 0 });
  });
});

describe("MotionTracker.trail", () => {
  it("caps the trail length and keeps cells oldest first", () => {
    const m = new MotionTracker();
    m.update(0, xy(0, 0), 0);
    for (let step = 1; step <= 7; step++) m.update(step, xy(step, 0), step * 500);

    const trail = m.trail(0);

    expect(trail).toHaveLength(TRAIL_CELLS);
    expect(trail.map((c) => c.x)).toEqual([2, 3, 4, 5, 6]);
  });

  it("does not include the current position", () => {
    const m = new MotionTracker();
    m.update(1, xy(0, 0), 0);
    m.update(2, xy(1, 0), 500);

    expect(m.trail(0)).toEqual([{ x: 0, y: 0 }]);
  });

  it("adds nothing while the robot stands still", () => {
    const m = new MotionTracker();
    m.update(1, xy(3, 3), 0);
    m.update(2, xy(3, 3), 500);
    m.update(3, xy(3, 3), 1000);

    expect(m.trail(0)).toEqual([]);
  });

  it("resets when the robot count changes", () => {
    const m = new MotionTracker();
    m.update(1, xy(0, 0, 5, 5), 0);
    m.update(2, xy(1, 0, 6, 5), 500);
    expect(m.trail(0)).toHaveLength(1);

    m.update(3, xy(9, 1), 1000);

    expect(m.count()).toBe(1);
    expect(m.trail(0)).toEqual([]);
    expect(m.position(0, 1250)).toEqual({ x: 9, y: 1 });
  });

  it("returns an empty trail for an unknown robot", () => {
    const m = new MotionTracker();

    expect(m.trail(7)).toEqual([]);
  });
});
