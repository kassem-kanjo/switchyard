import { describe, expect, it } from "vitest";
import { SPEEDS, TICKS_PER_SECOND_AT_1X, TickPacer } from "./pacer";

/** Simulate `seconds` of 60 fps animation frames, including the initial call. */
function run(speed: number, seconds = 1, fps = 60): number {
  const pacer = new TickPacer();
  let ticks = 0;
  ticks += pacer.take(0, speed);
  const frames = seconds * fps;
  for (let i = 1; i <= frames; i++) ticks += pacer.take((i * 1000) / fps, speed);
  return ticks;
}

describe("TickPacer", () => {
  it("defines 1x as two ticks per second", () => {
    expect(TICKS_PER_SECOND_AT_1X).toBe(2);
    expect(SPEEDS).toEqual([1, 4, 16, 64]);
  });

  it("runs 1x in real time over one second of frames", () => {
    expect(run(1)).toBe(2);
  });

  it("runs 4x at eight ticks per second", () => {
    expect(run(4)).toBe(8);
  });

  it("runs 16x at thirty-two ticks per second", () => {
    expect(run(16)).toBe(32);
  });

  it("runs 64x at one hundred and twenty-eight ticks per second", () => {
    expect(run(64)).toBe(128);
  });

  it("carries the fractional remainder across calls", () => {
    const pacer = new TickPacer();
    expect(pacer.take(0, 1)).toBe(0);
    expect(pacer.take(250, 1)).toBe(0); // 0.5 ticks accrued
    expect(pacer.take(500, 1)).toBe(1); // 1.0 ticks, remainder 0
    expect(pacer.take(750, 1)).toBe(0); // 0.5 ticks accrued
    expect(pacer.take(1000, 1)).toBe(1);
  });

  it("returns 0 on the first call and only records the time", () => {
    const pacer = new TickPacer();
    expect(pacer.take(5000, 64)).toBe(0);
    expect(pacer.take(5500, 64)).toBe(64);
  });

  it("forgets time and remainder on reset", () => {
    const pacer = new TickPacer();
    expect(pacer.take(0, 1)).toBe(0);
    expect(pacer.take(500, 1)).toBe(1);
    pacer.reset();
    expect(pacer.take(900, 1)).toBe(0); // first call after reset
    expect(pacer.take(1400, 1)).toBe(1);
  });

  it("returns 0 when time moves backwards without moving its clock back", () => {
    const pacer = new TickPacer();
    expect(pacer.take(0, 1)).toBe(0);
    expect(pacer.take(1000, 1)).toBe(2);
    expect(pacer.take(400, 1)).toBe(0);
    expect(pacer.take(1500, 1)).toBe(1); // counted from 1000, not 400
  });

  it("caps a burst after a long stall and drops the excess", () => {
    const pacer = new TickPacer();
    expect(pacer.take(0, 64)).toBe(0);
    expect(pacer.take(10_000, 64)).toBe(256); // 1280 due, capped at the default
    expect(pacer.take(10_000, 64)).toBe(0); // excess was dropped
  });

  it("honours a custom maxBurst", () => {
    const pacer = new TickPacer(5);
    expect(pacer.take(0, 1)).toBe(0);
    expect(pacer.take(10_000, 1)).toBe(5);
    expect(pacer.take(10_000, 1)).toBe(0);
  });

  it("returns 0 for speed 0 or NaN", () => {
    const pacer = new TickPacer();
    expect(pacer.take(0, 0)).toBe(0);
    expect(pacer.take(1000, 0)).toBe(0);
    expect(pacer.take(2000, Number.NaN)).toBe(0);
  });
});
