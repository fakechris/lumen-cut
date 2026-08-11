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
import { isWindows } from "../../platform";
import type { SubtitleStyle } from "../../types";
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
 *  falls back to the platform serif (no serif webfont is bundled). The
 *  Windows list leads with SimSun so the WYSIWYG canvas burn-in matches what
 *  libass will draw for the same preset. */
export function captionPresetFontFamily(preset: CaptionPreset): string | undefined {
  if (preset.font === "serif") {
    return isWindows()
      ? `"SimSun","Noto Serif SC",serif`
      : `"Songti SC","Noto Serif SC",serif`;
  }
  if (preset.font === "mono") return `"IBM Plex Mono",ui-monospace,monospace`;
  return undefined;
}

/** ASS fontname for export. Must resolve under libass, so these name fonts
 *  the host platform always ships: serif matches the preview (Songti SC on
 *  macOS, SimSun on Windows); mono is Menlo/Consolas — the preview's bundled
 *  IBM Plex Mono is woff2-only, which freetype cannot load. Keep in sync with
 *  preset_fontname in src-tauri/src/data/caption_presets.rs. */
export function captionPresetAssFont(preset: CaptionPreset): string | undefined {
  if (preset.font === "serif") return isWindows() ? "SimSun" : "Songti SC";
  if (preset.font === "mono") return isWindows() ? "Consolas" : "Menlo";
  return undefined;
}

/** Translation (sub) line size relative to the main line. pireel uses 0.7×, but
 *  that reads far smaller for CJK than for Latin (denser glyphs at the same em
 *  size), so lumen-cut uses 0.85×. The main line itself is always the user's
 *  SubtitleStyle font size (presets never own size). The ASS export applies the
 *  same ratio via \fs on the sub-line — keep CAPTION_SUB_LINE_SCALE in
 *  src-tauri/src/data/caption_presets.rs in sync. */
export const CAPTION_SUB_LINE_SCALE = 0.85;

/** ASS `&HAABBGGRR` → CSS `#rrggbb` (alpha dropped; the preview treats ASS
 *  outline/shadow colours as opaque, as before). */
export function assColourToHex(value: string): string {
  const match = value.match(/&H[0-9A-Fa-f]{2}([0-9A-Fa-f]{2})([0-9A-Fa-f]{2})([0-9A-Fa-f]{2})/);
  return match ? `#${match[3]}${match[2]}${match[1]}` : "#ffffff";
}

/** ASS BorderStyle-3 backing padding in px: the export sets the style's
 *  outline width to fontsize/4 (min 4) for backed presets — the box padding.
 *  (Mirrors ass.rs; used to size the preview pill padding.) */
export function captionPresetBoxPaddingPx(fontsize: number): number {
  return Math.max(4, Math.round(fontsize / 4));
}

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

/**
 * One caption LINE (main or translation) as it should render in the preview —
 * the mirror of one Dialogue event in the export, which emits bilingual preset
 * cues as two events (ass.rs). Each line gets:
 *   - its own backing pill (the export's BorderStyle-3 box is per event, so a
 *     single shared span with box-decoration-break never matched: Chromium
 *     draws one box, WKWebView drops it entirely);
 *   - pill padding equal to the ASS box padding (outline = fontsize/4, min 4,
 *     the same absolute px value on both lines), expressed in cqw like the
 *     font size so it scales with the stage;
 *   - font size = user's SubtitleStyle size × fontScale (the translation line
 *     passes CAPTION_SUB_LINE_SCALE, matching the export's \fs override).
 * Bare presets keep the user's outline + shadow; backed presets drop them
 * (pireel's bare-vs-backed rule, and the export zeroes the shadow).
 */
export function captionPresetLineSpanCss(
  preset: CaptionPreset,
  style: SubtitleStyle,
  canvasWidth: number,
  fontScale = 1,
): CSSProperties {
  const look = captionPresetLineCss(preset);
  const fontSize = Math.max(1, Math.round(style.fontsize * fontScale));
  return {
    ...look,
    fontFamily: look.fontFamily ?? style.fontname,
    fontSize: `clamp(12px, ${(fontSize / canvasWidth) * 100}cqw, ${fontSize}px)`,
    fontStyle: style.italic || preset.italic ? "italic" : "normal",
    fontWeight: style.bold ? 700 : 400,
    textAlign: "center",
    ...(preset.bg
      ? {
        padding: `${(captionPresetBoxPaddingPx(style.fontsize) / canvasWidth) * 100}cqw`,
      }
      : {
        WebkitTextStroke: `${(Math.max(0, style.outline) / canvasWidth) * 100}cqw ${assColourToHex(style.outlineColour)}`,
        textShadow: style.shadow > 0
          ? `${(style.shadow / canvasWidth) * 100}cqw ${(style.shadow / canvasWidth) * 100}cqw ${assColourToHex(style.outlineColour)}`
          : undefined,
      }),
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
