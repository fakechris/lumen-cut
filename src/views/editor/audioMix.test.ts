import { describe, expect, test } from "vitest";
import { audioGainAt, DEFAULT_AUDIO_MIX, musicGainAt } from "./audioMix";

describe("audio mix preview", () => {
  test("combines project gain with edge fades", () => {
    const mix = {
      ...DEFAULT_AUDIO_MIX,
      volume: 0.8,
      fadeIn: 2,
      fadeOut: 1,
    };
    expect(audioGainAt(mix, 0, 10)).toBe(0);
    expect(audioGainAt(mix, 1, 10)).toBe(0.4);
    expect(audioGainAt(mix, 4, 10)).toBe(0.8);
    expect(audioGainAt(mix, 9.5, 10)).toBe(0.4);
  });

  test("mute overrides gain and fades", () => {
    expect(audioGainAt(
      { ...DEFAULT_AUDIO_MIX, muted: true },
      5,
      10,
    )).toBe(0);
  });
});

test("previews music timing, fades, and dialogue ducking", () => {
  const track = {
    id: "music-a",
    path: "/tmp/music.wav",
    start: 2,
    end: 12,
    sourceStart: 1,
    volume: 0.5,
    fadeIn: 2,
    fadeOut: 2,
    ducking: true,
  };
  expect(musicGainAt(track, 1, true)).toBe(0);
  expect(musicGainAt(track, 3, false)).toBeCloseTo(0.25);
  expect(musicGainAt(track, 7, true)).toBeCloseTo(0.175);
  expect(musicGainAt(track, 11, false)).toBeCloseTo(0.25);
  expect(musicGainAt(track, 12, false)).toBe(0);
});
