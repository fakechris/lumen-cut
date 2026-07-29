import { describe, expect, test } from "vitest";
import type { ShotFraming } from "../../types";
import {
  framingAtTime,
  framingForSegment,
  keptIntervals,
  shotTransformCss,
  shotTransformVars,
  treatScale,
} from "./shotFraming";

describe("treatScale", () => {
  test("maps the pireel treatment ranges", () => {
    expect(treatScale("punch-in", 0)).toBeCloseTo(1.05);
    expect(treatScale("punch-in", 100)).toBeCloseTo(2.0);
    expect(treatScale("punch-in")).toBeCloseTo(1.05 + 0.18 * 0.95);
    expect(treatScale("corner-br", 0)).toBeCloseTo(0.2);
    expect(treatScale("corner-tl", 100)).toBeCloseTo(0.6);
    expect(treatScale("corner-br")).toBeCloseTo(0.34);
    expect(treatScale("split-l", 0)).toBeCloseTo(0.3);
    expect(treatScale("split-r", 100)).toBeCloseTo(0.7);
    expect(treatScale("split-l")).toBeCloseTo(0.5);
    expect(treatScale("full", 80)).toBe(1);
  });

  test("clamps out-of-range sizes", () => {
    expect(treatScale("punch-in", 250)).toBeCloseTo(2.0);
    expect(treatScale("corner-br", -5)).toBeCloseTo(0.2);
  });
});

describe("shotTransformVars", () => {
  test("punch-in zooms about the center", () => {
    expect(shotTransformVars("punch-in", 100)).toEqual({
      scale: 2,
      xPercent: 0,
      yPercent: 0,
    });
  });

  test("corners keep a 2% margin", () => {
    // scale 0.34 → free space (1-0.34)/2 = 0.33 → 33% − 2% = 31%.
    expect(shotTransformVars("corner-br")).toEqual({
      scale: 0.34,
      xPercent: 31,
      yPercent: 31,
    });
    expect(shotTransformVars("corner-tl")).toEqual({
      scale: 0.34,
      xPercent: -31,
      yPercent: -31,
    });
  });

  test("splits hug their edge and stay vertically centered", () => {
    // scale 0.5 → shift (1-0.5)/2 = 25%.
    expect(shotTransformVars("split-l")).toEqual({
      scale: 0.5,
      xPercent: -25,
      yPercent: 0,
    });
    expect(shotTransformVars("split-r")).toEqual({
      scale: 0.5,
      xPercent: 25,
      yPercent: 0,
    });
  });

  test("full renders no transform", () => {
    expect(shotTransformVars("full")).toEqual({ scale: 1, xPercent: 0, yPercent: 0 });
    expect(shotTransformCss("full")).toBe("none");
    expect(shotTransformCss("split-l")).toBe("translate(-25%, 0%) scale(0.5)");
  });
});

describe("keptIntervals", () => {
  test("computes the complement of the cut intervals", () => {
    expect(keptIntervals(10, [{ start: 2, end: 4 }, { start: 7, end: 9 }])).toEqual([
      { start: 0, end: 2 },
      { start: 4, end: 7 },
      { start: 9, end: 10 },
    ]);
    expect(keptIntervals(10, [])).toEqual([{ start: 0, end: 10 }]);
    expect(keptIntervals(10, [{ start: 0, end: 10 }])).toEqual([]);
  });
});

describe("framing lookup", () => {
  const entries: ShotFraming[] = [
    { id: "a", start: 4, end: 7, treatment: "punch-in" },
    { id: "b", start: 9, end: 10, treatment: "split-l", size: 60 },
    { id: "c", start: 0, end: 1, treatment: "full" },
  ];

  test("finds the entry containing a source instant", () => {
    expect(framingAtTime(entries, 5)?.id).toBe("a");
    expect(framingAtTime(entries, 9.5)?.id).toBe("b");
    expect(framingAtTime(entries, 7)).toBeUndefined();
    // `full` entries are stored-cleared semantics: never applied.
    expect(framingAtTime(entries, 0.5)).toBeUndefined();
  });

  test("resolves a kept segment by its midpoint", () => {
    expect(framingForSegment(entries, 4, 7)?.id).toBe("a");
    expect(framingForSegment(entries, 7, 9)).toBeUndefined();
  });
});
