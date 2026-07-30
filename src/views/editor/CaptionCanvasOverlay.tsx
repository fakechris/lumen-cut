/**
 * Program-monitor caption layer: a transparent <canvas> over the video stage,
 * drawn by the shared caption renderer (src/captions/captionRender.ts) — the
 * same code that renders the burn-in export frames, so the preview IS the
 * export. Replaces the old DOM caption layer.
 *
 * Redraws on every timeupdate (~4Hz from the media element, enough for
 * word-step karaoke) and whenever the stage box or devicePixelRatio changes;
 * drawing coordinates stay in export-canvas pixels regardless of stage size.
 */
import { useEffect, useRef } from "react";
import type { Doc, SubtitleRow, SubtitleStyle } from "../../types";
import {
  drawCaptionLayout,
  layoutCaption,
  canvasTextMeasurer,
  resolveCaptionDraw,
  type CaptionCanvasSize,
  type CaptionCue,
} from "../../captions/captionRender";
import { captionPresetById } from "./captionPresets";

interface Props {
  canvasSize: CaptionCanvasSize;
  currentTime: number;
  doc: Doc;
  rows: SubtitleRow[];
  subtitleStyle: SubtitleStyle;
}

/** Preview row + transcript sentence → renderer cue (mirrors the export spec's
 *  source/translation split: translation-only rows carry no word timing). */
export function captionCueForRow(
  row: SubtitleRow,
  doc: Doc,
): CaptionCue {
  const sentence = doc.paragraphs
    .flatMap((paragraph) => paragraph.sentences)
    .find((candidate) => candidate.id === row.id);
  const source = sentence?.text.trim();
  if (sentence && source === row.text) {
    return {
      id: row.id,
      sourceText: row.text,
      translationText: null,
      start: row.start,
      end: row.end,
      words: sentence.words.map((word) => ({ text: word.text, start: word.start, end: word.end })),
    };
  }
  if (sentence && source && row.text.startsWith(`${source}\n`)) {
    return {
      id: row.id,
      sourceText: source,
      translationText: row.text.slice(source.length + 1),
      start: row.start,
      end: row.end,
      words: sentence.words.map((word) => ({ text: word.text, start: word.start, end: word.end })),
    };
  }
  // Translation-only row (or the source sentence is unavailable): approximate
  // word timing over the whole row text, as the old preview did.
  return {
    id: row.id,
    sourceText: row.text,
    translationText: null,
    start: row.start,
    end: row.end,
    words: [],
  };
}

function activeCaptionRow(rows: SubtitleRow[], currentTime: number): SubtitleRow | undefined {
  let low = 0;
  let high = rows.length - 1;
  let candidate = -1;
  while (low <= high) {
    const middle = Math.floor((low + high) / 2);
    if (rows[middle]!.start <= currentTime) {
      candidate = middle;
      low = middle + 1;
    } else {
      high = middle - 1;
    }
  }
  const cue = candidate >= 0 ? rows[candidate] : undefined;
  return cue && !cue.hidden && currentTime < cue.end ? cue : undefined;
}

export function CaptionCanvasOverlay({
  canvasSize,
  currentTime,
  doc,
  rows,
  subtitleStyle,
}: Props) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const draw = () => {
      const bounds = canvas.getBoundingClientRect();
      if (bounds.width <= 0 || bounds.height <= 0) return;
      const dpr = window.devicePixelRatio || 1;
      const backingWidth = Math.round(bounds.width * dpr);
      const backingHeight = Math.round(bounds.height * dpr);
      if (canvas.width !== backingWidth) canvas.width = backingWidth;
      if (canvas.height !== backingHeight) canvas.height = backingHeight;
      ctx.setTransform(backingWidth / canvasSize.width, 0, 0, backingHeight / canvasSize.height, 0, 0);
      ctx.clearRect(0, 0, canvasSize.width, canvasSize.height);
      const row = activeCaptionRow(rows, currentTime);
      if (!row) return;
      const preset = captionPresetById(subtitleStyle.captionPreset);
      const cue = captionCueForRow(row, doc);
      const data = resolveCaptionDraw(cue, preset, currentTime);
      if (!data) return;
      const layout = layoutCaption(data, subtitleStyle, preset, canvasSize, canvasTextMeasurer(ctx));
      if (!layout) return;
      drawCaptionLayout(ctx, layout, data, subtitleStyle, preset);
    };
    draw();
    const observer = new ResizeObserver(draw);
    observer.observe(canvas);
    return () => observer.disconnect();
  }, [canvasSize, currentTime, doc, rows, subtitleStyle]);

  return (
    <canvas
      aria-hidden="true"
      className="program-caption-canvas"
      ref={canvasRef}
      style={{ position: "absolute", inset: 0, width: "100%", height: "100%", pointerEvents: "none", zIndex: 4 }}
    />
  );
}
