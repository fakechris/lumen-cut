// Per-shot framing — preview-side math for the six treatment presets.
// The treatment model (presets, 0–100 size mapping, transform vars) is
// adapted from pireel (AGPL-3.0), `packages/studio-engine/src/
// composition-core.ts` (`ShotTreatment`, `treatScale`, `shotTransformVars`,
// `TREAT_SIZE_DEFAULT`). https://github.com/fakechris/pireel
//
// The Rust export side (`src-tauri/src/export/video.rs::framing_filter_chain`)
// implements the same geometry in ffmpeg filters; keep the two in sync.

import type { ShotFraming, ShotTreatment } from "../../types";
import type { TimelineCutInterval } from "./timelineCuts";

/** Per-treatment default size (0–100), ported from pireel's TREAT_SIZE_DEFAULT. */
export const TREAT_SIZE_DEFAULT: Record<ShotTreatment, number> = {
  full: 0,
  "punch-in": 18,
  "corner-br": 35,
  "corner-tl": 35,
  "split-l": 50,
  "split-r": 50,
};

/** Framing size 0–100 → frame scale: punch-in 1.05–2.0, corner 0.2–0.6, split 0.3–0.7. */
export function treatScale(treatment: ShotTreatment, size?: number | null): number {
  const v = Math.max(0, Math.min(100, size ?? TREAT_SIZE_DEFAULT[treatment])) / 100;
  if (treatment === "punch-in") return 1.05 + v * 0.95;
  if (treatment === "corner-br" || treatment === "corner-tl") return 0.2 + v * 0.4;
  if (treatment === "split-l" || treatment === "split-r") return 0.3 + v * 0.4;
  return 1;
}

export interface ShotTransformVars {
  scale: number;
  xPercent: number;
  yPercent: number;
}

/**
 * Framing → CSS transform variables, ported from pireel's shotTransformVars
 * (borderRadius is dropped: the ffmpeg export has no rounded corners, so the
 * preview stays honest). Scale applies about the frame center, then the
 * frame translates by a percent of the canvas. Corners keep a 2% margin,
 * half-splits hug their edge.
 */
export function shotTransformVars(
  treatment: ShotTreatment,
  size?: number | null,
): ShotTransformVars {
  const s = treatScale(treatment, size);
  const r3 = (x: number) => Math.round(x * 1000) / 1000;
  const edge = r3(((1 - s) / 2) * 100);
  const corner = r3(((1 - s) / 2 - 0.02) * 100);
  switch (treatment) {
    case "punch-in":
      return { scale: r3(s), xPercent: 0, yPercent: 0 };
    case "corner-br":
      return { scale: r3(s), xPercent: corner, yPercent: corner };
    case "corner-tl":
      return { scale: r3(s), xPercent: -corner, yPercent: -corner };
    case "split-l":
      return { scale: r3(s), xPercent: -edge, yPercent: 0 };
    case "split-r":
      return { scale: r3(s), xPercent: edge, yPercent: 0 };
    default:
      return { scale: 1, xPercent: 0, yPercent: 0 };
  }
}

/** CSS transform for a shot (`none` for full). */
export function shotTransformCss(
  treatment: ShotTreatment,
  size?: number | null,
): string {
  if (treatment === "full") return "none";
  const vars = shotTransformVars(treatment, size);
  return `translate(${vars.xPercent}%, ${vars.yPercent}%) scale(${vars.scale})`;
}

/** Kept source-time segments = the complement of the cut intervals. */
export function keptIntervals(
  sourceDuration: number,
  cuts: TimelineCutInterval[],
): TimelineCutInterval[] {
  const kept: TimelineCutInterval[] = [];
  let cursor = 0;
  for (const cut of cuts) {
    const start = Math.max(0, Math.min(sourceDuration, cut.start));
    const end = Math.max(0, Math.min(sourceDuration, cut.end));
    if (start > cursor) kept.push({ start: cursor, end: start });
    cursor = Math.max(cursor, end);
  }
  if (cursor < sourceDuration) kept.push({ start: cursor, end: sourceDuration });
  return kept;
}

/** The framing entry governing source instant `time` (entry.start ≤ t < entry.end). */
export function framingAtTime(
  framings: ShotFraming[],
  time: number,
): ShotFraming | undefined {
  return framings.find(
    (entry) => entry.start <= time && time < entry.end
      && entry.treatment !== "full",
  );
}

/** The framing entry governing the kept segment [start, end), by midpoint. */
export function framingForSegment(
  framings: ShotFraming[],
  start: number,
  end: number,
): ShotFraming | undefined {
  return framingAtTime(framings, (start + end) / 2);
}
