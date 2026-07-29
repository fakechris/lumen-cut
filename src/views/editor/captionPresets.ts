/**
 * lumen-cut glue for the pireel caption presets (src/vendor/pireel/).
 *
 * A preset governs LOOK only (colors / backing / decoration / typeface):
 * font size, bold and position stay on the user's SubtitleStyle, matching
 * pireel's rule that size and weight are never preset-owned. No preset is
 * selected by default — `null` keeps the project's existing caption style.
 */
import type { CSSProperties } from "react";
import type { Lang } from "../../i18n";
import {
  CAPTION_PRESETS,
  type CaptionMode,
  type CaptionPreset,
} from "../../vendor/pireel/caption-presets";

/** Display names, ported from pireel's engine message catalogs (packages/studio-engine/src/messages.ts). */
const CAPTION_PRESET_NAMES: Record<string, { zh: string; en: string }> = {
  "em-yellow": { zh: "白字黄词", en: "Yellow pop" },
  "em-green": { zh: "白字荧绿", en: "Neon green" },
  "em-purple-black": { zh: "黑底紫词", en: "Purple on black" },
  "em-serif-black": { zh: "黑底青词", en: "Mint serif" },
  "em-underline": { zh: "黑底划线", en: "Black underline" },
  "em-blue-line": { zh: "灰底蓝线", en: "Blue underline" },
  "em-box-purple": { zh: "紫底跳块", en: "Purple blocks" },
  "em-box-blue": { zh: "蓝底黑块", en: "Blue blocks" },
  "em-pink": { zh: "粉底提白", en: "Pink pop" },
  "em-gold-serif": { zh: "米底金字", en: "Gold on cream" },
  "ln-clean": { zh: "干净白字", en: "Clean white" },
  "ln-black": { zh: "黑条白字", en: "Black tape" },
  "ln-navy": { zh: "蓝灰衬线", en: "Navy serif" },
  "ln-white": { zh: "白条蓝字", en: "White tape" },
  "ln-orange": { zh: "橙条白字", en: "Orange tape" },
  "ln-yellow": { zh: "黄条黑字", en: "Yellow tape" },
  "ln-red": { zh: "红条等宽", en: "Red mono" },
  "ln-mint": { zh: "青字投影", en: "Mint glow" },
};

/** Look up a preset by id. `null`/`undefined`/unknown ids mean "no preset" —
 *  the project's existing SubtitleStyle renders unchanged (lumen-cut default). */
export function captionPresetById(id: string | null | undefined): CaptionPreset | null {
  if (!id) return null;
  return CAPTION_PRESETS.find((preset) => preset.id === id) ?? null;
}

export function captionPresetName(preset: CaptionPreset, lang: Lang): string {
  const name = CAPTION_PRESET_NAMES[preset.id];
  return name ? name[lang] : preset.id;
}

/** Picker grouping (mirrors pireel's captions-panel SECTIONS): word emphasis first, line by line second. */
export const CAPTION_PRESET_GROUPS: Array<{ mode: CaptionMode; zh: string; en: string }> = [
  { mode: "emphasis", zh: "逐词强调", en: "Word emphasis" },
  { mode: "line", zh: "逐行", en: "Line by line" },
];

export function captionPresetsForMode(mode: CaptionMode): CaptionPreset[] {
  return CAPTION_PRESETS.filter((preset) => preset.mode === mode);
}

/** Preset font → CSS font-family. IBM Plex Mono ships with the app; serif
 *  falls back to the platform serif (no serif webfont is bundled). */
export function captionPresetFontFamily(preset: CaptionPreset): string | undefined {
  if (preset.font === "serif") return `"Songti SC","Noto Serif SC",serif`;
  if (preset.font === "mono") return `"IBM Plex Mono",ui-monospace,monospace`;
  return undefined;
}

/** ASS fontname for export (libass resolves against system fonts). */
export function captionPresetAssFont(preset: CaptionPreset): string | undefined {
  if (preset.font === "serif") return "Noto Serif SC";
  if (preset.font === "mono") return "IBM Plex Mono";
  return undefined;
}

/** Translation (sub) line size relative to the main line — pireel renders its
 *  bilingual sub-line at 0.7× the main line's scale; the main line itself is
 *  always the user's SubtitleStyle font size (presets never own size). */
export const CAPTION_SUB_LINE_SCALE = 0.7;

/** Whole-line preset look as CSS (color / backing pill / typeface / italic).
 *  Callers layer font size, weight and position from the user's SubtitleStyle;
 *  bare presets (no bg) should also keep the user's outline + shadow. */
export function captionPresetLineCss(preset: CaptionPreset): CSSProperties {
  return {
    color: preset.text,
    fontFamily: captionPresetFontFamily(preset),
    fontStyle: preset.italic ? "italic" : undefined,
    ...(preset.bg
      ? {
        background: preset.bg,
        borderRadius: "0.3em",
        padding: "0.12em 0.45em",
        boxDecorationBreak: "clone",
        WebkitBoxDecorationBreak: "clone",
        // Backed text gets no drop shadow (pireel's bare-vs-backed rule); the
        // overlay's base CSS paints one otherwise.
        textShadow: "none",
      }
      : {}),
  };
}

/** Translation-line CSS under a preset: same look as the main line (inherited),
 *  just scaled down — em keeps it proportional to the user's font size. */
export function captionPresetSubLineCss(): CSSProperties {
  return { fontSize: `${CAPTION_SUB_LINE_SCALE}em` };
}

/** Current-word treatment for emphasis presets: accent color and/or the
 *  preset's underline / highlight-box decoration. */
export function captionPresetWordCss(preset: CaptionPreset): CSSProperties {
  return {
    ...(preset.emphasis ? { color: preset.emphasis } : {}),
    ...(preset.deco === "underline"
      ? {
        textDecoration: "underline",
        textDecorationColor: preset.decoColor,
        textDecorationThickness: "0.1em",
        textUnderlineOffset: "0.15em",
      }
      : {}),
    ...(preset.deco === "highlight"
      ? { background: preset.decoColor, borderRadius: "0.16em", padding: "0 0.08em" }
      : {}),
  };
}
