/**
 * The single caption renderer: Canvas2D, a pure function of
 * (cue, time, style, preset, canvas size). Used by BOTH the program-monitor
 * preview (CaptionCanvasOverlay) and the burn-in export (captionExport +
 * captionStates), so what you see is literally what gets burned into the
 * video. Replaces the old DOM caption layer and the ASS caption burn for
 * in-app exports (the CLI/MCP path has no webview and still uses ASS — see
 * src-tauri/src/export/caption_frames.rs).
 *
 * Layout mirrors the retired DOM preview (see commit cde5991):
 *  - one "logical line" per Dialogue line (main + optional 0.85× translation),
 *    each wrapped independently with pireel's balanced line breaking
 *    (chunkWordsBalanced, real measureText widths);
 *  - per-line backing pill with absolute-px padding
 *    (captionPresetBoxPaddingPx = fontsize/4, min 4 — same on both lines);
 *  - block bottom-anchored at marginV (or top/middle per numpad alignment);
 *  - emphasis presets recolor the active word (and underline/highlight-box it),
 *    main and sub lines karaoke independently;
 *  - bare presets/no preset keep the user's outline + shadow; backed presets
 *    drop them (pireel's bare-vs-backed rule).
 */
import type { SubtitleStyle } from "../types";
import {
  assColourToHex,
  captionPresetBoxPaddingPx,
  captionPresetFontFamily,
  CAPTION_SUB_LINE_SCALE,
} from "../views/editor/captionPresets";
import type { CaptionPreset } from "../vendor/pireel/caption-presets";
import {
  chunkWordsBalanced,
  latinJoin,
  segmentTokens,
  wordsFromText,
} from "../vendor/pireel/caption-fx";

/** Matches `.program-subtitle > span` line-height in styles.css. */
export const CAPTION_LINE_HEIGHT = 1.35;
/** Gap between the main and sub logical lines (the retired DOM stack gap). */
function stackGapPx(style: SubtitleStyle): number {
  return Math.max(2, Math.round(style.fontsize * CAPTION_SUB_LINE_SCALE * 0.1));
}

export interface CaptionWord {
  text: string;
  start: number;
  end: number;
}

/**
 * One caption cue in display form. `words` carries real ASR timing for
 * `sourceText`; empty means "no word timing — approximate". Bilingual cues set
 * `translationText`; its karaoke timing is always approximated (translations
 * have no word timing — same rule as the old preview and the ASS export).
 */
export interface CaptionCue {
  id: string;
  sourceText: string;
  translationText: string | null;
  start: number;
  end: number;
  words: CaptionWord[];
}

export interface CaptionCanvasSize {
  width: number;
  height: number;
}

/** One logical caption line with per-word karaoke timing. */
export interface CaptionKaraokeLine {
  words: CaptionWord[];
  /** Font scale relative to the user's fontsize (1 = main, 0.85 = translation). */
  scale: number;
}

export type CaptionDrawData =
  | { kind: "karaoke"; lines: CaptionKaraokeLine[]; active: number[] }
  | { kind: "plain"; lines: string[] };

/**
 * What should be on screen for `cue` at `time` — the same decision the old
 * DOM preview made in its captionLines/activeIndex memos. Emphasis presets
 * get per-word lines with an active index per line; everything else renders
 * whole lines. Returns null when the cue has no visible text.
 */
export function resolveCaptionDraw(
  cue: CaptionCue,
  preset: CaptionPreset | null,
  time: number,
): CaptionDrawData | null {
  const source = cue.sourceText.trim();
  const translation = cue.translationText?.trim() ?? "";
  if (preset?.mode === "emphasis") {
    const approximate = (text: string) => wordsFromText(text, cue.start, cue.end);
    const main: CaptionWord[] = cue.words.length
      ? cue.words.map((word) => ({ ...word }))
      : approximate(source);
    if (!main.length) return null;
    const sub = translation ? approximate(translation) : [];
    const activeOf = (words: CaptionWord[]) => {
      let index = -1;
      words.forEach((word, i) => {
        if (time >= word.start) index = i;
      });
      return index;
    };
    const lines: CaptionKaraokeLine[] = [{ words: main, scale: 1 }];
    const active = [activeOf(main)];
    if (sub.length) {
      lines.push({ words: sub, scale: CAPTION_SUB_LINE_SCALE });
      active.push(activeOf(sub));
    }
    return { kind: "karaoke", lines, active };
  }
  const lines = [source, translation].filter((line) => line.length > 0);
  return lines.length ? { kind: "plain", lines } : null;
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

export interface CaptionFontMetrics {
  width: number;
  ascent: number;
  descent: number;
}

/** Injected so layout stays testable without a real canvas. */
export type CaptionTextMeasurer = (text: string, font: string) => CaptionFontMetrics;

/** Production measurer backed by CanvasRenderingContext2D.measureText. */
export function canvasTextMeasurer(ctx: CanvasRenderingContext2D): CaptionTextMeasurer {
  return (text, font) => {
    ctx.font = font;
    const metrics = ctx.measureText(text);
    const fallback = fontSizeOf(font);
    return {
      width: metrics.width,
      ascent: metrics.actualBoundingBoxAscent || fallback * 0.8,
      descent: metrics.actualBoundingBoxDescent || fallback * 0.25,
    };
  };
}

function fontSizeOf(font: string): number {
  const match = font.match(/(\d+(?:\.\d+)?)px/);
  return match ? Number(match[1]) : 16;
}

interface ResolvedLook {
  color: string;
  fontFamily: string;
  italic: boolean;
  bold: boolean;
  /** Backing pill color; null = bare text with outline/shadow. */
  bg: string | null;
}

export function resolveCaptionLook(style: SubtitleStyle, preset: CaptionPreset | null): ResolvedLook {
  return {
    color: preset ? preset.text : assColourToHex(style.primaryColour),
    fontFamily: preset
      ? (captionPresetFontFamily(preset) ?? style.fontname)
      : style.fontname,
    italic: style.italic || (preset?.italic ?? false),
    bold: style.bold,
    bg: preset?.bg ?? null,
  };
}

function captionFontString(size: number, look: ResolvedLook): string {
  return `${look.italic ? "italic " : ""}${look.bold ? 700 : 400} ${size}px ${look.fontFamily}`;
}

export interface CaptionLayoutToken {
  text: string;
  /** Word index within its logical line, -1 for plain text. */
  wordIndex: number;
  x: number;
  baseline: number;
  width: number;
}

export interface CaptionLayoutLine {
  tokens: CaptionLayoutToken[];
  font: string;
  fontSize: number;
  /** Logical line this visual line belongs to (main vs sub pill grouping). */
  logicalIndex: number;
  x: number;
  width: number;
  baseline: number;
  ascent: number;
  descent: number;
}

export interface CaptionLayout {
  lines: CaptionLayoutLine[];
}

interface LayoutInputLine {
  tokens: Array<{ text: string; spacer: boolean; wordIndex: number }>;
  fontSize: number;
  font: string;
  logicalIndex: number;
}

/** Anchor math shared by layout: the retired DOM subtitlePosition(). */
function anchorBox(style: SubtitleStyle, canvas: CaptionCanvasSize) {
  const vertical = Math.ceil(style.alignment / 3); // 1 bottom, 2 middle, 3 top
  const horizontal = (style.alignment - 1) % 3; // 0 left, 1 center, 2 right
  return {
    vertical,
    horizontal,
    x0: Math.max(0, style.marginL),
    x1: canvas.width - Math.max(0, style.marginR),
    marginV: Math.max(0, style.marginV),
  };
}

/**
 * Lay out a caption for the given canvas. Wrapping uses pireel's balanced
 * breaking with real measured widths; the wrap limit is the margin box minus
 * the pill's horizontal padding (a backed line must fit INSIDE its pill).
 */
export function layoutCaption(
  data: CaptionDrawData,
  style: SubtitleStyle,
  preset: CaptionPreset | null,
  canvas: CaptionCanvasSize,
  measure: CaptionTextMeasurer,
): CaptionLayout | null {
  const look = resolveCaptionLook(style, preset);
  const box = anchorBox(style, canvas);
  const pillPad = look.bg ? captionPresetBoxPaddingPx(style.fontsize) : 0;
  const wrapLimit = Math.max(40, box.x1 - box.x0 - pillPad * 2);

  const inputLines: LayoutInputLine[] = [];
  if (data.kind === "karaoke") {
    data.lines.forEach((line, logicalIndex) => {
      const fontSize = Math.max(1, Math.round(style.fontsize * line.scale));
      inputLines.push({
        tokens: line.words.map((word, wordIndex) => ({
          text: word.text,
          spacer: wordIndex < line.words.length - 1 && latinJoin(word.text, line.words[wordIndex + 1]!.text),
          wordIndex,
        })),
        fontSize,
        font: captionFontString(fontSize, look),
        logicalIndex,
      });
    });
  } else {
    data.lines.forEach((text, logicalIndex) => {
      const words = segmentTokens(text);
      inputLines.push({
        tokens: words.map((word, wordIndex) => ({
          text: word,
          spacer: wordIndex < words.length - 1 && latinJoin(word, words[wordIndex + 1]!),
          wordIndex: -1,
        })),
        fontSize: style.fontsize,
        font: captionFontString(style.fontsize, look),
        logicalIndex,
      });
    });
  }
  if (!inputLines.length) return null;

  interface WrappedLine {
    tokens: Array<{ text: string; spacer: boolean; wordIndex: number; width: number }>;
    width: number;
    input: LayoutInputLine;
  }
  const wrapped: WrappedLine[] = [];
  for (const input of inputLines) {
    const spaceWidth = (token: { text: string; spacer: boolean }) =>
      measure(token.text, input.font).width
      + (token.spacer ? measure(" ", input.font).width : 0);
    const chunks = chunkWordsBalanced(input.tokens, wrapLimit, spaceWidth);
    for (const chunk of chunks) {
      const tokens = chunk.map((token) => ({ ...token, width: spaceWidth(token) }));
      wrapped.push({
        tokens,
        width: tokens.reduce((total, token) => total + token.width, 0),
        input,
      });
    }
  }

  const gap = stackGapPx(style);
  const slotHeights = wrapped.map((line) => line.input.fontSize * CAPTION_LINE_HEIGHT);
  const blockHeight = slotHeights.reduce((total, height) => total + height, 0)
    + gap * Math.max(0, inputLines.length - 1);
  const blockTop = box.vertical === 3
    ? box.marginV
    : box.vertical === 2
      ? (canvas.height - blockHeight) / 2
      : canvas.height - box.marginV - blockHeight;

  const lines: CaptionLayoutLine[] = [];
  let slotTop = blockTop;
  let previousLogical = -1;
  for (const line of wrapped) {
    if (previousLogical >= 0 && line.input.logicalIndex !== previousLogical) {
      slotTop += gap;
    }
    previousLogical = line.input.logicalIndex;
    const slotHeight = line.input.fontSize * CAPTION_LINE_HEIGHT;
    const reference = line.tokens.map((token) => token.text).join("");
    const metrics = measure(reference || "Ag", line.input.font);
    const x = box.horizontal === 0
      ? box.x0
      : box.horizontal === 2
        ? box.x1 - line.width
        : (box.x0 + box.x1 - line.width) / 2;
    const baseline = slotTop + slotHeight / 2 + (metrics.ascent - metrics.descent) / 2;
    let tokenX = x;
    lines.push({
      tokens: line.tokens.map((token) => {
        const placed = { text: token.text, wordIndex: token.wordIndex, x: tokenX, baseline, width: token.width };
        tokenX += token.width;
        return placed;
      }),
      font: line.input.font,
      fontSize: line.input.fontSize,
      logicalIndex: line.input.logicalIndex,
      x,
      width: line.width,
      baseline,
      ascent: metrics.ascent,
      descent: metrics.descent,
    });
    slotTop += slotHeight;
  }
  return { lines };
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/**
 * Draw one laid-out caption onto `ctx`. The canvas must already be cleared
 * (or fresh) and sized to the export canvas (or scaled so drawing coordinates
 * are in canvas pixels).
 */
export function drawCaptionLayout(
  ctx: CanvasRenderingContext2D,
  layout: CaptionLayout,
  data: CaptionDrawData,
  style: SubtitleStyle,
  preset: CaptionPreset | null,
): void {
  const look = resolveCaptionLook(style, preset);
  const pillPad = look.bg ? captionPresetBoxPaddingPx(style.fontsize) : 0;
  ctx.textBaseline = "alphabetic";
  ctx.lineJoin = "round";

  // Backing pills first (behind every token), one per visual line.
  if (look.bg) {
    ctx.fillStyle = look.bg;
    for (const line of layout.lines) {
      ctx.beginPath();
      ctx.roundRect(
        line.x - pillPad,
        line.baseline - line.ascent - pillPad,
        line.width + pillPad * 2,
        line.ascent + line.descent + pillPad * 2,
        line.fontSize * 0.3,
      );
      ctx.fill();
    }
  }

  // Shadow: backed presets drop it (pireel bare-vs-backed rule); bare text
  // uses the user's shadow, falling back to the old overlay's base CSS shadow.
  if (!look.bg) {
    if (style.shadow > 0) {
      ctx.shadowColor = assColourToHex(style.outlineColour);
      ctx.shadowOffsetX = style.shadow;
      ctx.shadowOffsetY = style.shadow;
      ctx.shadowBlur = 0;
    } else if (!preset) {
      ctx.shadowColor = "#000000";
      ctx.shadowOffsetX = 0;
      ctx.shadowOffsetY = 1;
      ctx.shadowBlur = 2;
    }
  }

  for (const line of layout.lines) {
    ctx.font = line.font;
    for (const token of line.tokens) {
      const isActive = token.wordIndex >= 0
        && data.kind === "karaoke"
        && data.active[line.logicalIndex] === token.wordIndex;
      if (isActive && preset?.deco === "highlight" && preset.decoColor) {
        const pad = line.fontSize * 0.08;
        ctx.save();
        ctx.shadowColor = "transparent";
        ctx.fillStyle = preset.decoColor;
        ctx.beginPath();
        ctx.roundRect(
          token.x - pad,
          line.baseline - line.ascent - pad,
          token.width + pad * 2,
          line.ascent + line.descent + pad * 2,
          line.fontSize * 0.16,
        );
        ctx.fill();
        ctx.restore();
      }
      // text-stroke emulation: stroke first, the fill covers the inner half,
      // leaving an `outline`-px ring (matches -webkit-text-stroke).
      if (!look.bg && style.outline > 0) {
        ctx.strokeStyle = assColourToHex(style.outlineColour);
        ctx.lineWidth = Math.max(1, style.outline * 2);
        ctx.strokeText(token.text, token.x, token.baseline);
      }
      ctx.fillStyle = isActive && preset?.emphasis ? preset.emphasis : look.color;
      ctx.fillText(token.text, token.x, token.baseline);
      const underline = (isActive && preset?.deco === "underline") || (!preset && style.underline);
      const strike = !preset && style.strikeOut;
      if (underline || strike) {
        ctx.save();
        ctx.shadowColor = "transparent";
        ctx.strokeStyle = isActive && preset?.deco === "underline" && preset.decoColor
          ? preset.decoColor
          : ctx.fillStyle;
        ctx.lineWidth = Math.max(1, line.fontSize * 0.1);
        const y = strike
          ? line.baseline - (line.ascent - line.descent) / 2
          : line.baseline + line.fontSize * 0.15;
        ctx.beginPath();
        ctx.moveTo(token.x, y);
        ctx.lineTo(token.x + token.width, y);
        ctx.stroke();
        ctx.restore();
      }
    }
  }
  ctx.shadowColor = "transparent";
  ctx.shadowOffsetX = 0;
  ctx.shadowOffsetY = 0;
  ctx.shadowBlur = 0;
}

/** One-call convenience: clear `ctx` and draw cue-at-time. */
export function drawCaptionCue(
  ctx: CanvasRenderingContext2D,
  cue: CaptionCue,
  time: number,
  style: SubtitleStyle,
  preset: CaptionPreset | null,
  canvas: CaptionCanvasSize,
): void {
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  const data = resolveCaptionDraw(cue, preset, time);
  if (!data) return;
  const layout = layoutCaption(data, style, preset, canvas, canvasTextMeasurer(ctx));
  if (!layout) return;
  drawCaptionLayout(ctx, layout, data, style, preset);
}
