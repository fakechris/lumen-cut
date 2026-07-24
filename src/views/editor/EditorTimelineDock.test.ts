import { describe, expect, test } from "vitest";
import type { Doc } from "../../types";
import {
  buildSplitTargetIndex,
  musicTrackLaneLayout,
  nearestSnap,
  resolveCueDrag,
  resolveMusicDrag,
  splitTargetAtPlayhead,
} from "./EditorTimelineDock";

describe("background music lane layout", () => {
  const track = (id: string, start: number, end: number) => ({
    id,
    path: `/tmp/${id}.mp3`,
    start,
    end,
    sourceStart: 0,
    volume: 0.25,
    fadeIn: 0,
    fadeOut: 0,
    ducking: true,
  });

  test("reuses a lane for sequential clips and separates every overlap", () => {
    const layout = musicTrackLaneLayout([
      track("first", 0, 4),
      track("overlap", 2, 6),
      track("nested", 3, 5),
      track("later", 6, 8),
    ]);

    expect(layout.count).toBe(3);
    expect(layout.lanes.get("first")).toBe(0);
    expect(layout.lanes.get("overlap")).toBe(1);
    expect(layout.lanes.get("nested")).toBe(2);
    expect(layout.lanes.get("later")).toBe(0);
  });
});

describe("timeline snapping", () => {
  test("uses the nearest edit boundary inside the pixel-derived threshold", () => {
    const result = nearestSnap(4.92, [0, 5, 8], 0.1);
    expect(result.value).toBe(5);
    expect(result.snap).toBe(5);
    expect(result.distance).toBeCloseTo(0.08);
  });

  test("does not move an edit when every boundary is outside the threshold", () => {
    expect(nearestSnap(4.8, [0, 5, 8], 0.1)).toEqual({
      value: 4.8,
      distance: Number.POSITIVE_INFINITY,
    });
  });

  test("chooses the closer boundary when several are eligible", () => {
    expect(nearestSnap(5.06, [5, 5.1], 0.1).value).toBe(5.1);
  });
});

describe("caption timing drag", () => {
  const drag = {
    end: 4,
    id: "cue-1",
    mode: "move" as const,
    originEnd: 4,
    originStart: 2,
    originX: 100,
    start: 2,
  };

  test("moves a cue as a fixed span and clamps it between neighbouring cues", () => {
    expect(resolveCueDrag(drag, 10, 1, 7, [], 0)).toMatchObject({
      start: 5,
      end: 7,
    });
    expect(resolveCueDrag(drag, -10, 1, 7, [], 0)).toMatchObject({
      start: 1,
      end: 3,
    });
  });

  test("snaps either edge while preserving the cue duration", () => {
    expect(resolveCueDrag(drag, 0.92, 1, 7, [5], 0.1)).toMatchObject({
      start: 3,
      end: 5,
      snap: 5,
    });
  });

  test("never lets a snapped trim cross the minimum duration or neighbour", () => {
    const startTrim = { ...drag, mode: "start" as const };
    const endTrim = { ...drag, mode: "end" as const };
    expect(resolveCueDrag(startTrim, 1.95, 1, 7, [4.5], 1)).toMatchObject({
      start: 3.9,
      end: 4,
    });
    expect(resolveCueDrag(endTrim, -1.95, 1, 7, [1.5], 1)).toMatchObject({
      start: 2,
      end: 2.1,
    });
  });
});

describe("background music timeline drag", () => {
  const drag = {
    end: 6,
    id: "music-a",
    mode: "move" as const,
    originEnd: 6,
    originPointerProgram: 3,
    originSourceStart: 1,
    originStart: 2,
    sourceStart: 1,
    start: 2,
  };

  test("moves the program clip while preserving its source offset", () => {
    expect(resolveMusicDrag(drag, 10, 10, [], 0)).toMatchObject({
      start: 6,
      end: 10,
      sourceStart: 1,
    });
    expect(resolveMusicDrag(drag, -10, 10, [], 0)).toMatchObject({
      start: 0,
      end: 4,
      sourceStart: 1,
    });
  });

  test("trimming the start advances the source offset and cannot expose negative source time", () => {
    expect(resolveMusicDrag(
      { ...drag, mode: "start" as const },
      1,
      10,
      [],
      0,
    )).toMatchObject({
      start: 3,
      end: 6,
      sourceStart: 2,
    });
    expect(resolveMusicDrag(
      { ...drag, mode: "start" as const },
      -10,
      10,
      [],
      0,
    )).toMatchObject({
      start: 1,
      end: 6,
      sourceStart: 0,
    });
  });

  test("snaps either edge in edited program time", () => {
    expect(resolveMusicDrag(drag, 0.92, 10, [7], 0.1)).toMatchObject({
      start: 3,
      end: 7,
      snap: 7,
    });
    expect(resolveMusicDrag(
      { ...drag, mode: "end" as const },
      0.95,
      10,
      [7],
      0.1,
    )).toMatchObject({
      end: 7,
      snap: 7,
    });
  });
});

describe("playhead split lookup", () => {
  const doc: Doc = {
    id: "project",
    schema: 1,
    media: { path: "/tmp/video.mp4", durationSeconds: 20 },
    meta: {
      title: "Long project",
      description: "",
      createdAt: "2026-01-01T00:00:00Z",
      updatedAt: "2026-01-01T00:00:00Z",
    },
    paragraphs: [{
      id: 1,
      sentences: [
        {
          id: "cue-1",
          text: "one two three",
          words: [
            { id: "w1", text: "one", start: 1, end: 1.4 },
            { id: "w2", text: "two", start: 1.6, end: 2 },
            { id: "w3", text: "three", start: 2.4, end: 3 },
          ],
        },
        {
          id: "cue-2",
          text: "four five",
          words: [
            { id: "w4", text: "four", start: 10, end: 10.4 },
            { id: "w5", text: "five", start: 10.8, end: 11.4 },
          ],
        },
      ],
    }],
    translations: {},
  };

  test("indexes word boundaries once and finds the nearest boundary in the active cue", () => {
    const index = buildSplitTargetIndex(doc);

    expect(index).toHaveLength(2);
    expect(splitTargetAtPlayhead(index, 2.25)).toEqual({
      id: "cue-1",
      at: 2,
      time: 2.2,
    });
    const second = splitTargetAtPlayhead(index, 10.2);
    expect(second).toMatchObject({ id: "cue-2", at: 1 });
    expect(second?.time).toBeCloseTo(10.6);
  });

  test("does not borrow a split point from a neighbouring cue while the playhead is in a gap", () => {
    const index = buildSplitTargetIndex(doc);
    expect(splitTargetAtPlayhead(index, 6)).toBeNull();
    expect(splitTargetAtPlayhead(index, 0.5)).toBeNull();
  });
});
