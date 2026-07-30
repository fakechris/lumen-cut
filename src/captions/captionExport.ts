/**
 * Burn-in export, frontend half: ask Rust for the caption render spec (the
 * exact retimed cues + style snapshot the export will burn), render every
 * caption state with the shared Canvas2D renderer, stream PNG frames over raw
 * IPC, then seal the manifest. `video_export_start` picks up the sealed
 * manifest and burns it with an ffmpeg overlay instead of ASS.
 *
 * Frames are cached in memory by (spec hash, state key), so a repeat export
 * of an unchanged project skips both rendering and PNG encoding and only
 * re-uploads bytes (~120 MB/s, sub-second even for long videos).
 */
import { invoke } from "@tauri-apps/api/core";
import type { SubtitleStyle } from "../types";
import { captionPresetById } from "../views/editor/captionPresets";
import {
  canvasTextMeasurer,
  drawCaptionLayout,
  layoutCaption,
  type CaptionCue,
} from "./captionRender";
import { buildCaptionTimeline } from "./captionStates";

export interface CaptionSpecCue {
  id: string;
  start: number;
  end: number;
  sourceText: string;
  translationText: string | null;
  words: Array<{ text: string; start: number; end: number }>;
}

export interface CaptionRenderSpec {
  version: number;
  hash: string;
  width: number;
  height: number;
  duration: number;
  style: SubtitleStyle;
  cues: CaptionSpecCue[];
}

export interface CaptionRenderProgress {
  current: number;
  total: number;
}

export class CaptionRenderCancelled extends Error {
  constructor() {
    super("caption render cancelled");
    this.name = "CaptionRenderCancelled";
  }
}

/** spec hash → (state key → PNG bytes). Module-lifetime cache. */
const frameCache = new Map<string, Map<string, Uint8Array>>();

export async function captionExportPrepare(pid: string): Promise<CaptionRenderSpec | null> {
  return invoke<CaptionRenderSpec | null>("caption_export_prepare", { pid });
}

export async function captionFramesAbort(pid: string): Promise<void> {
  await invoke("caption_frames_abort", { pid });
}

/** Frame payload with a tiny name header so one raw invoke carries both.
 *  Top-level Uint8Array arrives as InvokeBody::Raw (no JSON round-trip). */
function packFrame(name: string, bytes: Uint8Array): Uint8Array {
  const encoded = new TextEncoder().encode(name);
  const payload = new Uint8Array(2 + encoded.length + bytes.length);
  payload[0] = encoded.length & 0xff;
  payload[1] = (encoded.length >> 8) & 0xff;
  payload.set(encoded, 2);
  payload.set(bytes, 2 + encoded.length);
  return payload;
}

async function canvasToPng(canvas: HTMLCanvasElement): Promise<Uint8Array> {
  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/png"));
  if (!blob) throw new Error("canvas.toBlob returned null");
  return new Uint8Array(await blob.arrayBuffer());
}

/**
 * Render + upload every caption state for `spec`, then seal the manifest.
 * Calls onProgress after each unique state. Throws CaptionRenderCancelled
 * when shouldCancel flips (and aborts the Rust session).
 */
export async function renderCaptionFrames(
  pid: string,
  spec: CaptionRenderSpec,
  options: {
    onProgress?: (progress: CaptionRenderProgress) => void;
    shouldCancel?: () => boolean;
  } = {},
): Promise<{ uniqueStates: number; segments: number; cached: number }> {
  const preset = captionPresetById(spec.style.captionPreset);
  const timeline = buildCaptionTimeline(
    spec.cues.map((cue): CaptionCue => ({
      id: cue.id,
      sourceText: cue.sourceText,
      translationText: cue.translationText,
      start: cue.start,
      end: cue.end,
      words: cue.words,
    })),
    preset,
    spec.duration,
  );

  const canvas = document.createElement("canvas");
  canvas.width = spec.width;
  canvas.height = spec.height;
  const ctx = canvas.getContext("2d")!;
  const measurer = canvasTextMeasurer(ctx);
  const cache = frameCache.get(spec.hash) ?? new Map<string, Uint8Array>();
  frameCache.set(spec.hash, cache);

  let cached = 0;
  const fileByKey = new Map<string, string>();
  for (let i = 0; i < timeline.states.length; i++) {
    if (options.shouldCancel?.()) {
      await captionFramesAbort(pid).catch(() => undefined);
      throw new CaptionRenderCancelled();
    }
    const state = timeline.states[i]!;
    const file = `f${String(i).padStart(5, "0")}.png`;
    fileByKey.set(state.key, file);
    let png = cache.get(state.key);
    if (png) {
      cached++;
    } else {
      ctx.clearRect(0, 0, spec.width, spec.height);
      if (state.data.kind !== "plain" || state.data.lines.length > 0) {
        const layout = layoutCaption(state.data, spec.style, preset, spec, measurer);
        if (layout) drawCaptionLayout(ctx, layout, state.data, spec.style, preset);
      }
      png = await canvasToPng(canvas);
      cache.set(state.key, png);
    }
    await invoke("caption_frames_push", packFrame(file, png) as unknown as Record<string, never>);
    options.onProgress?.({ current: i + 1, total: timeline.states.length });
  }

  const manifest = {
    version: spec.version,
    hash: spec.hash,
    width: spec.width,
    height: spec.height,
    duration: spec.duration,
    segments: timeline.segments.map((segment) => ({
      file: fileByKey.get(segment.key)!,
      start: segment.start,
      end: segment.end,
    })),
  };
  await invoke("caption_frames_seal", { pid, manifest: JSON.stringify(manifest) });
  return { uniqueStates: timeline.states.length, segments: timeline.segments.length, cached };
}
