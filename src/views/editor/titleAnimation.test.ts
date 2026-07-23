import { describe, expect, test } from "vitest";
import type { TitleClip } from "../../types";
import { titleOpacityAt } from "./titleAnimation";

const title = {
  start: 2,
  end: 6,
  fadeIn: 1,
  fadeOut: 2,
} as TitleClip;

describe("title animation", () => {
  test("interpolates fade keyframes at both title edges", () => {
    expect(titleOpacityAt(title, 2)).toBe(0);
    expect(titleOpacityAt(title, 2.5)).toBe(0.5);
    expect(titleOpacityAt(title, 4)).toBe(1);
    expect(titleOpacityAt(title, 5)).toBe(0.5);
    expect(titleOpacityAt(title, 6)).toBe(0);
  });

  test("old titles without animation remain fully opaque", () => {
    expect(titleOpacityAt({ ...title, fadeIn: 0, fadeOut: 0 }, 3)).toBe(1);
  });
});
