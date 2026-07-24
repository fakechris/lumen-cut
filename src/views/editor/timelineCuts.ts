import type { CutSummary } from "../../api";
import type { Doc } from "../../types";

export interface TimelineCutInterval {
  end: number;
  start: number;
}

export function resolveTimelineCuts(doc: Doc, cuts: CutSummary[]): TimelineCutInterval[] {
  const words = new Map(
    doc.paragraphs.flatMap((paragraph) =>
      paragraph.sentences.flatMap((sentence) => sentence.words),
    ).map((word) => [word.id, word]),
  );
  const intervals = cuts.flatMap((cut) => {
    const left = words.get(cut.a_word);
    const right = words.get(cut.b_word);
    if (!left || !right) return [];
    const end = cut.kind === "silence" ? right.start : right.end;
    const start = cut.kind === "silence"
      ? Math.max(0, end - cut.duration)
      : left.start;
    return Number.isFinite(start) && Number.isFinite(end) && end > start
      ? [{ start, end }]
      : [];
  }).sort((left, right) => left.start - right.start);

  return intervals.reduce<TimelineCutInterval[]>((merged, interval) => {
    const previous = merged[merged.length - 1];
    if (previous && interval.start <= previous.end) {
      previous.end = Math.max(previous.end, interval.end);
    } else {
      merged.push({ ...interval });
    }
    return merged;
  }, []);
}

export function nextPlayableTime(
  sourceTime: number,
  cuts: TimelineCutInterval[],
): number {
  const interval = cuts.find(
    (cut) => sourceTime >= cut.start && sourceTime < cut.end,
  );
  return interval ? interval.end : sourceTime;
}

export function editedTimelineDuration(
  sourceDuration: number,
  cuts: TimelineCutInterval[],
): number {
  const removed = cuts.reduce((total, cut) => total + cut.end - cut.start, 0);
  return Math.max(0, sourceDuration - removed);
}

export function sourceToEditedTime(
  sourceTime: number,
  cuts: TimelineCutInterval[],
): number {
  const safeTime = Math.max(0, sourceTime);
  const removed = cuts.reduce((total, cut) => {
    if (safeTime <= cut.start) return total;
    return total + Math.min(safeTime, cut.end) - cut.start;
  }, 0);
  return Math.max(0, safeTime - removed);
}

export function editedToSourceTime(
  editedTime: number,
  cuts: TimelineCutInterval[],
  sourceDuration: number,
): number {
  let sourceTime = Math.max(0, editedTime);
  for (const cut of cuts) {
    if (sourceTime < cut.start) break;
    sourceTime += cut.end - cut.start;
  }
  return Math.min(Math.max(0, sourceDuration), sourceTime);
}
