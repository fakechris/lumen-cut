import { describe, expect, it } from "vitest";
import {
  CAPTION_PRESETS,
  getCaptionPreset,
} from "../../vendor/pireel/caption-presets";
import {
  chunkWordsBalanced,
  groupAsrWords,
  joinWords,
  latinJoin,
  segmentTokens,
  wordsFromText,
} from "../../vendor/pireel/caption-fx";
import {
  CAPTION_PRESET_GROUPS,
  CAPTION_SUB_LINE_SCALE,
  captionPresetAssFont,
  captionPresetById,
  captionPresetFontFamily,
  captionPresetLineCss,
  captionPresetName,
  captionPresetSubLineCss,
  captionPresetWordCss,
  captionPresetsForMode,
} from "./captionPresets";

describe("pireel caption preset table", () => {
  it("ports all 18 presets in two modes", () => {
    expect(CAPTION_PRESETS).toHaveLength(18);
    expect(CAPTION_PRESETS.filter((p) => p.mode === "emphasis")).toHaveLength(10);
    expect(CAPTION_PRESETS.filter((p) => p.mode === "line")).toHaveLength(8);
  });

  it("falls back to pireel's default on unknown ids", () => {
    expect(getCaptionPreset("em-yellow").id).toBe("em-yellow");
    expect(getCaptionPreset("bogus").id).toBe("em-yellow");
    expect(getCaptionPreset(undefined).id).toBe("em-yellow");
  });
});

describe("captionPresetById", () => {
  it("treats null/undefined/unknown ids as no preset (lumen-cut default)", () => {
    expect(captionPresetById(null)).toBeNull();
    expect(captionPresetById(undefined)).toBeNull();
    expect(captionPresetById("")).toBeNull();
    expect(captionPresetById("bogus")).toBeNull();
    expect(captionPresetById("ln-clean")?.id).toBe("ln-clean");
  });
});

describe("preset grouping and names", () => {
  it("groups emphasis before line, mirroring pireel's panel", () => {
    expect(CAPTION_PRESET_GROUPS.map((g) => g.mode)).toEqual(["emphasis", "line"]);
    expect(captionPresetsForMode("emphasis")).toHaveLength(10);
    expect(captionPresetsForMode("line")).toHaveLength(8);
  });

  it("has bilingual display names for every preset", () => {
    for (const preset of CAPTION_PRESETS) {
      expect(captionPresetName(preset, "zh")).not.toBe(preset.id);
      expect(captionPresetName(preset, "en")).not.toBe(preset.id);
    }
    expect(captionPresetName(getCaptionPreset("em-yellow"), "zh")).toBe("白字黄词");
    expect(captionPresetName(getCaptionPreset("em-yellow"), "en")).toBe("Yellow pop");
  });
});

describe("captionPresetLineCss (preset → CSS)", () => {
  it("maps bare presets to text color without a backing", () => {
    const css = captionPresetLineCss(getCaptionPreset("em-yellow"));
    expect(css.color).toBe("#ffffff");
    expect(css.background).toBeUndefined();
  });

  it("maps backed presets to a clone-decorated pill without a drop shadow", () => {
    const css = captionPresetLineCss(getCaptionPreset("ln-black"));
    expect(css.background).toBe("rgba(0,0,0,0.85)");
    expect(css.borderRadius).toBe("0.3em");
    expect(css.boxDecorationBreak).toBe("clone");
    // Backed text gets no shadow (pireel's bare-vs-backed rule).
    expect(css.textShadow).toBe("none");
  });

  it("never owns font size or weight — those stay on the user's SubtitleStyle", () => {
    for (const preset of CAPTION_PRESETS) {
      const css = captionPresetLineCss(preset);
      expect(css.fontSize).toBeUndefined();
      expect(css.fontWeight).toBeUndefined();
    }
  });

  it("maps italic and fonts", () => {
    expect(captionPresetLineCss(getCaptionPreset("ln-white")).fontStyle).toBe("italic");
    expect(captionPresetFontFamily(getCaptionPreset("ln-navy"))).toContain("Songti SC");
    expect(captionPresetFontFamily(getCaptionPreset("ln-red"))).toContain("IBM Plex Mono");
    expect(captionPresetFontFamily(getCaptionPreset("ln-clean"))).toBeUndefined();
    expect(captionPresetAssFont(getCaptionPreset("em-gold-serif"))).toBe("Noto Serif SC");
    expect(captionPresetAssFont(getCaptionPreset("ln-clean"))).toBeUndefined();
  });
});

describe("captionPresetSubLineCss (translation line)", () => {
  it("scales relative to the main line in em so it follows the user's font size", () => {
    // 0.85 (not pireel's 0.7): CJK glyphs read far smaller than Latin per em.
    expect(CAPTION_SUB_LINE_SCALE).toBe(0.85);
    expect(captionPresetSubLineCss().fontSize).toBe(`${CAPTION_SUB_LINE_SCALE}em`);
    // No color/font of its own: the sub-line inherits the preset's look.
    expect(captionPresetSubLineCss().color).toBeUndefined();
  });
});

describe("approximated translation word timing (wordsFromText)", () => {
  it("partitions the cue window linearly by token length", () => {
    const words = wordsFromText("你好世界", 2, 4);
    expect(words).toHaveLength(2);
    // Contiguous coverage of [2, 4): each token starts where the last ended.
    expect(words[0].start).toBe(2);
    expect(words[1].start).toBe(words[0].end);
    expect(words[1].end).toBeCloseTo(4, 3);
    expect(words[1].end - words[1].start).toBeCloseTo(
      words[0].end - words[0].start,
      3,
    );
  });

  it("allocates Latin words proportionally and keeps their spaces out of timing", () => {
    const words = wordsFromText("hello there", 0, 1);
    expect(words.map((w) => w.text)).toEqual(["hello", "there"]);
    expect(words[0].end - words[0].start).toBeCloseTo(0.5, 3);
  });

  it("returns no tokens for empty text (caller falls back to whole-line)", () => {
    expect(wordsFromText("   ", 0, 1)).toEqual([]);
  });
});

describe("captionPresetWordCss (emphasis word → CSS)", () => {
  it("recolors the spoken word when the preset has an emphasis color", () => {
    expect(captionPresetWordCss(getCaptionPreset("em-yellow")).color).toBe("#ffe34f");
  });

  it("renders underline decorations with the deco color", () => {
    const css = captionPresetWordCss(getCaptionPreset("em-blue-line"));
    expect(css.textDecoration).toBe("underline");
    expect(css.textDecorationColor).toBe("#0059ff");
  });

  it("renders highlight decorations as a box behind the word", () => {
    const css = captionPresetWordCss(getCaptionPreset("em-box-blue"));
    expect(css.background).toBe("#000000");
    expect(css.borderRadius).toBe("0.16em");
  });

  it("does nothing for presets without emphasis or deco", () => {
    expect(captionPresetWordCss(getCaptionPreset("ln-mint"))).toEqual({});
  });
});

describe("caption-fx pure logic (ported)", () => {
  it("segments CJK words and glues punctuation", () => {
    // Exact CJK boundaries depend on the runtime ICU dictionary; the stable
    // contract is: tokens rebuild the text and punctuation never stands alone.
    const tokens = segmentTokens("科学家发现了新大陆。");
    expect(tokens.join("")).toBe("科学家发现了新大陆。");
    expect(tokens[tokens.length - 1]).toMatch(/。$/);
    expect(segmentTokens("Hello, world")).toEqual(["Hello,", "world"]);
  });

  it("joins Latin words with spaces and CJK without", () => {
    expect(latinJoin("Hello", "world")).toBe(true);
    expect(latinJoin("你", "好")).toBe(false);
    expect(joinWords(["Hello", "world", "你好", "世界"])).toBe("Hello world你好世界");
  });

  it("allocates word timing linearly by character length", () => {
    const words = wordsFromText("ab cd", 1, 3);
    expect(words).toHaveLength(2);
    expect(words[0]).toMatchObject({ text: "ab", start: 1 });
    expect(words[1].end).toBeCloseTo(3, 3);
  });

  it("merges per-char ASR tokens into ICU words", () => {
    const asr = [
      { text: "你", start: 0, end: 0.2 },
      { text: "好", start: 0.2, end: 0.4 },
      { text: "世", start: 0.4, end: 0.7 },
      { text: "界", start: 0.7, end: 1 },
    ];
    expect(groupAsrWords("你好世界", asr)).toEqual([
      { text: "你好", start: 0, end: 0.4 },
      { text: "世界", start: 0.4, end: 1 },
    ]);
  });

  it("keeps raw ASR tokens when they cannot cover the sentence", () => {
    const asr = [{ text: "hello", start: 0, end: 1 }];
    expect(groupAsrWords("你好世界", asr)).toEqual([{ text: "hello", start: 0, end: 1 }]);
  });

  it("breaks lines near equal width without an orphan tail", () => {
    const words = wordsFromText("一二三四五六七八九十甲乙丙丁戊己", 0, 10);
    const chunks = chunkWordsBalanced(words, 10, (w) => w.text.length);
    expect(chunks.length).toBeGreaterThan(1);
    const sizes = chunks.map((c) => c.reduce((n, w) => n + w.text.length, 0));
    expect(Math.max(...sizes) - Math.min(...sizes)).toBeLessThanOrEqual(3);
  });
});
