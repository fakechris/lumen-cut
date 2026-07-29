import { describe, expect, test } from "vitest";
import type { CutSummary } from "../../api";
import type { Doc } from "../../types";
import {
  cutWordIdsForDoc,
  editedToSourceTime,
  editedTimelineDuration,
  nextPlayableTime,
  resolveTimelineCuts,
  sourceToEditedTime,
} from "./timelineCuts";

const doc = {
  id: "p1",
  media: { durationSeconds: 10 },
  paragraphs: [{
    sentences: [{
      words: [
        { id: "w1", start: 1, end: 2 },
        { id: "w2", start: 3, end: 4 },
        { id: "w3", start: 5, end: 6 },
      ],
    }],
  }],
} as Doc;

function cut(overrides: Partial<CutSummary>): CutSummary {
  return {
    a_word: "w1",
    b_word: "w1",
    duration: 1,
    id: "cut",
    kind: "manual",
    note: null,
    ...overrides,
  };
}

describe("timeline cut preview", () => {
  test("merges overlapping source removals before computing edited duration", () => {
    const intervals = resolveTimelineCuts(doc, [
      cut({ a_word: "w1", b_word: "w2" }),
      cut({ id: "overlap", a_word: "w2", b_word: "w3" }),
    ]);

    expect(intervals).toEqual([{ start: 1, end: 6 }]);
    expect(editedTimelineDuration(10, intervals)).toBe(5);
  });

  test("skips a removed region only while the playhead is inside it", () => {
    const intervals = [{ start: 1, end: 2 }];

    expect(nextPlayableTime(0.9, intervals)).toBe(0.9);
    expect(nextPlayableTime(1.2, intervals)).toBe(2);
    expect(nextPlayableTime(2, intervals)).toBe(2);
  });

  test("uses cut duration to resolve a compressed silence", () => {
    expect(resolveTimelineCuts(doc, [
      cut({
        a_word: "w1",
        b_word: "w2",
        duration: 0.4,
        kind: "silence",
      }),
    ])).toEqual([{ start: 2.6, end: 3 }]);
  });

  test("maps source time to edited program time across cuts", () => {
    const intervals = [
      { start: 1, end: 2 },
      { start: 4, end: 6 },
    ];

    expect(sourceToEditedTime(0.5, intervals)).toBe(0.5);
    expect(sourceToEditedTime(1.5, intervals)).toBe(1);
    expect(sourceToEditedTime(3, intervals)).toBe(2);
    expect(sourceToEditedTime(5, intervals)).toBe(3);
    expect(sourceToEditedTime(8, intervals)).toBe(5);
  });

  test("maps edited program time back to playable source time", () => {
    const intervals = [
      { start: 1, end: 2 },
      { start: 4, end: 6 },
    ];

    expect(editedToSourceTime(0.5, intervals, 10)).toBe(0.5);
    expect(editedToSourceTime(1, intervals, 10)).toBe(2);
    expect(editedToSourceTime(2, intervals, 10)).toBe(3);
    expect(editedToSourceTime(3, intervals, 10)).toBe(6);
    expect(editedToSourceTime(5, intervals, 10)).toBe(8);
  });

  test("round-trips playable boundaries and clamps to the source duration", () => {
    const intervals = [
      { start: 1, end: 2 },
      { start: 4, end: 6 },
    ];

    for (const sourceTime of [0, 0.5, 2, 3, 6, 8, 10]) {
      expect(editedToSourceTime(
        sourceToEditedTime(sourceTime, intervals),
        intervals,
        10,
      )).toBe(sourceTime);
    }
    expect(editedToSourceTime(-1, intervals, 10)).toBe(0);
    expect(editedToSourceTime(100, intervals, 10)).toBe(10);
  });
});

describe("cut word display", () => {
  test("marks every word a word-cut interval covers", () => {
    const removed = cutWordIdsForDoc(doc, [cut({ a_word: "w1", b_word: "w2" })]);

    expect([...removed].sort()).toEqual(["w1", "w2"]);
  });

  test("silence cuts strike through nothing — only the gap is removed", () => {
    const removed = cutWordIdsForDoc(doc, [
      cut({ a_word: "w1", b_word: "w2", duration: 0.4, kind: "silence" }),
    ]);

    expect(removed.size).toBe(0);
  });

  test("a restored (split) cut no longer marks the restored word", () => {
    // Mirrors the Rust `cuts_restore` split: w2 returns to the timeline and
    // the original w1..w3 cut becomes two one-word cuts.
    const restored = [
      cut({ id: "cut~1", a_word: "w1", b_word: "w1" }),
      cut({ id: "cut~2", a_word: "w3", b_word: "w3" }),
    ];

    const removed = cutWordIdsForDoc(doc, restored);
    expect(removed.has("w1")).toBe(true);
    expect(removed.has("w2")).toBe(false);
    expect(removed.has("w3")).toBe(true);

    const intervals = resolveTimelineCuts(doc, restored);
    expect(editedTimelineDuration(10, intervals)).toBe(8);
  });

  test("with no cuts nothing is struck through", () => {
    expect(cutWordIdsForDoc(doc, []).size).toBe(0);
  });
});
