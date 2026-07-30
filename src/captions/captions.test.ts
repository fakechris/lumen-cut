import { describe, expect, it } from "vitest";
import type { SubtitleStyle } from "../types";
import { captionPresetById } from "../views/editor/captionPresets";
import {
  layoutCaption,
  resolveCaptionDraw,
  type CaptionCue,
  type CaptionTextMeasurer,
} from "./captionRender";
import { buildCaptionTimeline, EMPTY_CAPTION_KEY } from "./captionStates";

const style: SubtitleStyle = {
  name: "Test",
  fontname: "PingFang SC",
  fontsize: 64,
  primaryColour: "&H00FFFFFF",
  outlineColour: "&H00141414",
  bold: true,
  italic: false,
  underline: false,
  strikeOut: false,
  alignment: 2,
  outline: 3,
  shadow: 1,
  marginL: 60,
  marginR: 60,
  marginV: 90,
  captionPreset: "em-yellow",
};

const canvas = { width: 1920, height: 1080 };

/** Deterministic fake metrics: CJK/full-width = 1em, Latin ≈ 0.5em. */
const fakeMeasure: CaptionTextMeasurer = (text, font) => {
  const size = Number(font.match(/(\d+)px/)?.[1] ?? 16);
  const ems = [...text].reduce((n, ch) => n + (ch.codePointAt(0)! > 0x2e7f ? 1 : 0.5), 0);
  return { width: ems * size, ascent: size * 0.8, descent: size * 0.2 };
};

const emYellow = captionPresetById("em-yellow")!;

function bilingualCue(): CaptionCue {
  return {
    id: "s1",
    sourceText: "你好世界",
    translationText: "Hello world",
    start: 1,
    end: 3,
    words: [
      { text: "你好", start: 1.0, end: 1.5 },
      { text: "世界", start: 1.5, end: 2.0 },
    ],
  };
}

describe("resolveCaptionDraw", () => {
  it("emphasis preset: real word timing for main, approximated sub, independent actives", () => {
    const data = resolveCaptionDraw(bilingualCue(), emYellow, 1.6);
    expect(data?.kind).toBe("karaoke");
    if (data?.kind !== "karaoke") return;
    expect(data.lines[0]!.words.map((w) => w.text)).toEqual(["你好", "世界"]);
    expect(data.lines[1]!.scale).toBeCloseTo(0.85, 5);
    expect(data.active[0]).toBe(1); // 1.6 > 1.5 (世界)
    // Sub line is approximated over the cue window; must stay inside it.
    for (const word of data.lines[1]!.words) {
      expect(word.start).toBeGreaterThanOrEqual(1);
      expect(word.end).toBeLessThanOrEqual(3);
    }
  });

  it("translation-only cue: approximates over the whole text (no real words)", () => {
    const cue: CaptionCue = { ...bilingualCue(), sourceText: "你好世界", translationText: null, words: [] };
    const data = resolveCaptionDraw(cue, emYellow, 1.2);
    expect(data?.kind).toBe("karaoke");
    if (data?.kind !== "karaoke") return;
    expect(data.lines).toHaveLength(1);
    expect(data.lines[0]!.words.length).toBeGreaterThan(0);
  });

  it("line preset and no preset: plain whole-line text, bilingual keeps both lines", () => {
    const line = resolveCaptionDraw(bilingualCue(), captionPresetById("ln-black"), 1.2);
    expect(line).toEqual({ kind: "plain", lines: ["你好世界", "Hello world"] });
    const bare = resolveCaptionDraw(bilingualCue(), null, 1.2);
    expect(bare).toEqual({ kind: "plain", lines: ["你好世界", "Hello world"] });
  });

  it("empty text renders nothing", () => {
    const cue: CaptionCue = { ...bilingualCue(), sourceText: "  ", translationText: null };
    expect(resolveCaptionDraw(cue, null, 1.2)).toBeNull();
  });
});

describe("layoutCaption", () => {
  it("bottom-anchors the block at marginV and centers lines horizontally", () => {
    const data = resolveCaptionDraw(bilingualCue(), emYellow, 1.2)!;
    const layout = layoutCaption(data, style, emYellow, canvas, fakeMeasure)!;
    const last = layout.lines[layout.lines.length - 1]!;
    // Block bottom = slot bottom of the last visual line ≈ H - marginV.
    const slotBottom = last.baseline - (last.ascent - last.descent) / 2 + (last.fontSize * 1.35) / 2;
    expect(slotBottom).toBeCloseTo(canvas.height - style.marginV, 0);
    // Centered: line sits inside the margin box, centered.
    const center = last.x + last.width / 2;
    expect(center).toBeCloseTo((style.marginL + (canvas.width - style.marginR)) / 2, 0);
  });

  it("sub line is 0.85× and stacked above the block bottom", () => {
    const data = resolveCaptionDraw(bilingualCue(), emYellow, 1.2)!;
    const layout = layoutCaption(data, style, emYellow, canvas, fakeMeasure)!;
    expect(layout.lines).toHaveLength(2);
    expect(layout.lines[0]!.fontSize).toBe(64);
    expect(layout.lines[1]!.fontSize).toBe(Math.round(64 * 0.85));
    expect(layout.lines[0]!.baseline).toBeLessThan(layout.lines[1]!.baseline);
  });

  it("wraps long lines with balanced breaking inside the margin box", () => {
    const longCue: CaptionCue = {
      id: "s2",
      sourceText: "这是一个非常非常长的句子它需要被平衡地折成两行而不是溢出到安全边距之外去",
      translationText: null,
      start: 0,
      end: 5,
      words: [],
    };
    const data = resolveCaptionDraw(longCue, null, 0.5)!;
    const layout = layoutCaption(data, style, null, canvas, fakeMeasure)!;
    expect(layout.lines.length).toBeGreaterThanOrEqual(2);
    const limit = canvas.width - style.marginL - style.marginR;
    for (const line of layout.lines) {
      expect(line.width).toBeLessThanOrEqual(limit);
    }
    // Balanced: no orphan tail — the two lines are within 35% of each other.
    const widths = layout.lines.map((line) => line.width);
    expect(Math.min(...widths)).toBeGreaterThan(Math.max(...widths) * 0.35);
  });

  it("backed presets reserve pill padding inside the wrap limit", () => {
    const backed = captionPresetById("ln-black")!;
    const longCue: CaptionCue = {
      id: "s3",
      sourceText: "这是一个需要折行的底衬样式长句用来验证药丸内边距被计入可用宽度",
      translationText: null,
      start: 0,
      end: 5,
      words: [],
    };
    const data = resolveCaptionDraw(longCue, backed, 0.5)!;
    const layout = layoutCaption(data, style, backed, canvas, fakeMeasure)!;
    const pad = Math.max(4, Math.round(style.fontsize / 4));
    const limit = canvas.width - style.marginL - style.marginR - pad * 2;
    for (const line of layout.lines) {
      expect(line.width).toBeLessThanOrEqual(limit + 1);
    }
  });
});

describe("buildCaptionTimeline", () => {
  it("cuts states at cue and word boundaries; gaps share the empty state", () => {
    const timeline = buildCaptionTimeline([bilingualCue()], emYellow, 10);
    const empty = timeline.segments.filter((s) => s.key === EMPTY_CAPTION_KEY);
    expect(empty[0]).toMatchObject({ start: 0, end: 1 });
    // Karaoke cue [1,3]: main word boundary 1.5 + sub approx boundaries.
    const cueSegments = timeline.segments.filter((s) => s.start >= 1 && s.end <= 3);
    expect(cueSegments.length).toBeGreaterThanOrEqual(3);
    expect(timeline.segments[timeline.segments.length - 1]).toMatchObject({ end: 10 });
    // Full coverage of the window.
    const covered = timeline.segments.reduce((n, s) => n + (s.end - s.start), 0);
    expect(covered).toBeCloseTo(10, 6);
  });

  it("dedupes identical states by content key", () => {
    const cues = [
      { ...bilingualCue(), id: "a", start: 1, end: 2 },
      { ...bilingualCue(), id: "b", start: 3, end: 4 },
    ];
    const timeline = buildCaptionTimeline(cues, emYellow, 10);
    // One empty state shared across all three gaps.
    const empties = timeline.states.filter((s) => s.key === EMPTY_CAPTION_KEY);
    expect(empties).toHaveLength(1);
    // Every segment references an existing state.
    const keys = new Set(timeline.states.map((s) => s.key));
    for (const segment of timeline.segments) {
      expect(keys.has(segment.key)).toBe(true);
    }
  });

  it("karaoke states advance the active word across a boundary", () => {
    const timeline = buildCaptionTimeline([bilingualCue()], emYellow, 10);
    const states = timeline.states
      .filter((s) => s.key !== EMPTY_CAPTION_KEY)
      .map((s) => s.data);
    const mains = new Set(
      states.map((data) => (data.kind === "karaoke" ? data.active[0] : -99)),
    );
    expect(mains.has(0)).toBe(true);
    expect(mains.has(1)).toBe(true);
  });

  it("plain (non-emphasis) cues produce a single state per cue", () => {
    const timeline = buildCaptionTimeline([bilingualCue()], captionPresetById("ln-black"), 10);
    const content = timeline.states.filter((s) => s.key !== EMPTY_CAPTION_KEY);
    expect(content).toHaveLength(1);
    expect(content[0]!.data).toEqual({ kind: "plain", lines: ["你好世界", "Hello world"] });
  });
});
