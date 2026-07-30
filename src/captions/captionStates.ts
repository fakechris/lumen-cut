/**
 * Caption-state segmentation for the burn-in export. A "state" is one
 * discrete caption look: karaoke emphasis only changes at word boundaries, so
 * a cue needs one PNG per word boundary (plus gaps between cues). Identical
 * visual content (e.g. every empty gap, or repeated words) shares one frame
 * via the dedupe key, and the in-memory PNG cache makes repeat exports of an
 * unchanged project skip rendering entirely.
 */
import type { CaptionPreset } from "../vendor/pireel/caption-presets";
import type {
  CaptionCue,
  CaptionDrawData,
} from "./captionRender";
import { resolveCaptionDraw } from "./captionRender";

export interface CaptionState {
  /** Dedupe key — identical visual content shares one PNG. */
  key: string;
  data: CaptionDrawData;
}

export interface CaptionSegment {
  start: number;
  end: number;
  key: string;
}

export interface CaptionTimeline {
  segments: CaptionSegment[];
  states: CaptionState[];
}

export const EMPTY_CAPTION_KEY = "__empty__";

const EMPTY_DATA: CaptionDrawData = { kind: "plain", lines: [] };

/** Cheap structural hash for dedupe (FNV-1a over the serialized draw data). */
function captionStateKey(cue: CaptionCue, data: CaptionDrawData): string {
  const serialized = data.kind === "karaoke"
    ? `${cue.id}|k|${data.active.join(",")}`
    : `${cue.id}|p|${data.lines.join("\n")}`;
  let hash = 0x811c9dc5;
  for (let i = 0; i < serialized.length; i++) {
    hash ^= serialized.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return `s${(hash >>> 0).toString(16)}`;
}

/**
 * Cut [0, windowSeconds] into caption states. Boundaries are cue start/end
 * and every word start; each segment samples the draw data at its start.
 * Cues arrive retimed onto the export timeline (Rust spec), so segment times
 * map 1:1 onto output frames.
 */
export function buildCaptionTimeline(
  cues: CaptionCue[],
  preset: CaptionPreset | null,
  windowSeconds: number,
): CaptionTimeline {
  const visible = cues
    .filter((cue) => cue.end > 0 && cue.start < windowSeconds && cue.sourceText.trim().length > 0)
    .map((cue) => ({
      ...cue,
      start: Math.max(0, cue.start),
      end: Math.min(windowSeconds, cue.end),
    }))
    .filter((cue) => cue.end > cue.start)
    .sort((a, b) => a.start - b.start);

  const boundaries = new Set<number>([0, windowSeconds]);
  for (const cue of visible) {
    boundaries.add(cue.start);
    boundaries.add(cue.end);
    // Karaoke steps land on word starts (main + approximated sub alike).
    const probe = resolveCaptionDraw(cue, preset, cue.start + 1e-6);
    if (probe?.kind === "karaoke") {
      for (const line of probe.lines) {
        for (const word of line.words) {
          if (word.start > cue.start + 0.002 && word.start < cue.end - 0.002) {
            boundaries.add(word.start);
          }
        }
      }
    }
  }
  const points = [...boundaries].sort((a, b) => a - b);

  const statesByKey = new Map<string, CaptionState>();
  const segments: CaptionSegment[] = [];
  for (let i = 0; i < points.length - 1; i++) {
    const start = points[i]!;
    const end = points[i + 1]!;
    if (end - start < 0.004) continue; // sub-frame slivers carry no visible state
    const cue: CaptionCue | undefined = visible.find((candidate) => candidate.start <= start && start < candidate.end);
    let state: CaptionState;
    if (!cue) {
      state = { key: EMPTY_CAPTION_KEY, data: EMPTY_DATA };
    } else {
      const data = resolveCaptionDraw(cue, preset, start + 1e-6) ?? EMPTY_DATA;
      state = { key: data === EMPTY_DATA ? EMPTY_CAPTION_KEY : captionStateKey(cue, data), data };
    }
    if (!statesByKey.has(state.key)) statesByKey.set(state.key, state);
    const last = segments[segments.length - 1];
    if (last && last.key === state.key) last.end = end;
    else segments.push({ start, end, key: state.key });
  }
  return { segments, states: [...statesByKey.values()] };
}
