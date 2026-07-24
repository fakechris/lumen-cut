import { describe, expect, test } from "vitest";
import { resolveTimelineCollapsed } from "./TranscriptView";

describe("responsive timeline preference", () => {
  test("uses the compact viewport recommendation until the user overrides it", () => {
    expect(resolveTimelineCollapsed("auto", true)).toBe(true);
    expect(resolveTimelineCollapsed("auto", false)).toBe(false);
    expect(resolveTimelineCollapsed("expanded", true)).toBe(false);
    expect(resolveTimelineCollapsed("collapsed", false)).toBe(true);
  });
});
