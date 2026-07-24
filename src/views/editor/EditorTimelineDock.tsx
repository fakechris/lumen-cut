import { convertFileSrc } from "@tauri-apps/api/core";
import {
  type CSSProperties,
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  pickAudioFile,
  timelineVisuals,
  type CutSummary,
  type EditHistoryStatus,
} from "../../api";
import { PlayIcon } from "../../components/Icons";
import type { Lang } from "../../i18n";
import type {
  BrollOverview,
  AudioMix,
  BrollPlacement,
  BrollPlacementInput,
  ChapterRow,
  Doc,
  MusicTrack,
  SubtitleRow,
  TitleClip,
  TitleClipInput,
} from "../../types";
import {
  editedToSourceTime,
  editedTimelineDuration,
  resolveTimelineCuts,
  sourceToEditedTime,
} from "./timelineCuts";

interface Props {
  autoCollapsed: boolean;
  broll: BrollOverview;
  chapters: ChapterRow[];
  currentTime: number;
  cuts: CutSummary[];
  doc: Doc;
  isPlaying: boolean;
  lang: Lang;
  pid: string;
  rows: SubtitleRow[];
  titles: TitleClip[];
  busy: boolean;
  collapsed: boolean;
  history: EditHistoryStatus;
  onOpenBroll: () => void;
  onAddTitle: (input: TitleClipInput) => Promise<void>;
  onUpdateAudioMix: (mix: AudioMix) => Promise<void>;
  onRedo: () => Promise<void>;
  onRemoveCues: (ids: string[]) => Promise<void>;
  onSeek: (seconds: number, autoplay?: boolean) => void;
  onSplit: (id: string, at: number) => Promise<void>;
  onUpdateCueTiming: (id: string, start: number, end: number) => Promise<void>;
  onTogglePlayback: () => void;
  onTogglePreviewCuts: () => void;
  onToggleCollapsed: () => void;
  onUndo: () => Promise<void>;
  onUpdateBroll: (id: string, input: BrollPlacementInput) => Promise<void>;
  onUpdateTitle: (id: string, input: TitleClipInput) => Promise<void>;
  onRemoveTitle: (id: string) => Promise<void>;
  previewCuts: boolean;
  audioMix: AudioMix;
}

const TIMELINE_DRAFTS_KEY_PREFIX = "lumen-cut.timelineDrafts.";

export function musicTrackLaneLayout(tracks: MusicTrack[]) {
  const lanes = new Map<string, number>();
  const laneEnds: number[] = [];
  tracks
    .map((track, index) => ({ track, index }))
    .sort((left, right) =>
      left.track.start - right.track.start
      || left.track.end - right.track.end
      || left.index - right.index)
    .forEach(({ track }) => {
      let lane = laneEnds.findIndex((end) => end <= track.start + 0.000_001);
      if (lane < 0) lane = laneEnds.length;
      laneEnds[lane] = track.end;
      lanes.set(track.id, lane);
    });
  return { count: laneEnds.length, lanes };
}

interface PersistedTimelineDrafts {
  audioDraft: AudioMix;
  audioSource: AudioMix;
  audioPanelOpen: boolean;
  selectedTitleId: string | null;
  titleDraft: TitleClipInput | null;
  titlePanelOpen: boolean;
  titleSource: TitleClipInput | null;
}

function parseAudioMix(value: unknown): AudioMix | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const mix = value as Record<string, unknown>;
  if (!(Number.isFinite(mix.volume)
    && typeof mix.muted === "boolean"
    && Number.isFinite(mix.fadeIn)
    && Number.isFinite(mix.fadeOut))) return null;
  if (mix.voiceEnhance !== undefined && typeof mix.voiceEnhance !== "boolean") return null;
  if (mix.normalizeLoudness !== undefined && typeof mix.normalizeLoudness !== "boolean") return null;
  if (mix.loudnessTarget !== undefined && !Number.isFinite(mix.loudnessTarget)) return null;
  const rawMusic = mix.music === undefined || mix.music === null
    ? []
    : Array.isArray(mix.music)
      ? mix.music
      : [mix.music];
  const music = rawMusic.map((value, index) => {
    if (!value || typeof value !== "object" || Array.isArray(value)) return null;
    const track = value as Record<string, unknown>;
    if (!(typeof track.path === "string"
      && Number.isFinite(track.start)
      && Number.isFinite(track.end)
      && Number.isFinite(track.sourceStart)
      && Number.isFinite(track.volume)
      && Number.isFinite(track.fadeIn)
      && Number.isFinite(track.fadeOut)
      && typeof track.ducking === "boolean")) return null;
    return {
      id: typeof track.id === "string" && track.id.trim()
        ? track.id
        : `music-${index + 1}`,
      path: track.path,
      start: track.start as number,
      end: track.end as number,
      sourceStart: track.sourceStart as number,
      volume: track.volume as number,
      fadeIn: track.fadeIn as number,
      fadeOut: track.fadeOut as number,
      ducking: track.ducking,
    };
  });
  if (music.some((track) => track === null)) return null;
  return {
    volume: mix.volume as number,
    muted: mix.muted,
    fadeIn: mix.fadeIn as number,
    fadeOut: mix.fadeOut as number,
    voiceEnhance: mix.voiceEnhance === true,
    normalizeLoudness: mix.normalizeLoudness === true,
    loudnessTarget: mix.loudnessTarget === undefined ? -16 : mix.loudnessTarget as number,
    music: music as MusicTrack[],
  };
}

function isTitleInput(value: unknown): value is TitleClipInput {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const title = value as Record<string, unknown>;
  return typeof title.text === "string"
    && Number.isFinite(title.start)
    && Number.isFinite(title.end)
    && Number.isFinite(title.x)
    && Number.isFinite(title.y)
    && Number.isFinite(title.fontSize)
    && typeof title.color === "string"
    && typeof title.background === "string"
    && Number.isFinite(title.fadeIn)
    && Number.isFinite(title.fadeOut);
}

function initialTimelineDrafts(pid: string, audioMix: AudioMix): PersistedTimelineDrafts {
  const fallback: PersistedTimelineDrafts = {
    audioDraft: audioMix,
    audioSource: audioMix,
    audioPanelOpen: false,
    selectedTitleId: null,
    titleDraft: null,
    titlePanelOpen: false,
    titleSource: null,
  };
  try {
    const parsed = JSON.parse(
      localStorage.getItem(`${TIMELINE_DRAFTS_KEY_PREFIX}${pid}`) || "null",
    ) as Record<string, unknown> | null;
    if (!parsed) return fallback;
    const titleDraft = parsed.titleDraft === null ? null : parsed.titleDraft;
    const titleSource = parsed.titleSource === null ? null : parsed.titleSource;
    const audioDraft = parseAudioMix(parsed.audioDraft);
    const audioSource = parseAudioMix(parsed.audioSource);
    if (!audioDraft
      || !audioSource
      || (titleDraft !== null && !isTitleInput(titleDraft))
      || (titleSource !== null && !isTitleInput(titleSource))
      || (parsed.selectedTitleId !== null && typeof parsed.selectedTitleId !== "string")) {
      return fallback;
    }
    return {
      audioDraft,
      audioSource,
      audioPanelOpen: parsed.audioPanelOpen === true,
      selectedTitleId: parsed.selectedTitleId,
      titleDraft,
      titlePanelOpen: parsed.titlePanelOpen === true && titleDraft !== null,
      titleSource,
    };
  } catch {
    return fallback;
  }
}

// Lumen categorical palette (matches .speaker-swatch-* order in styles.css)
const SPEAKER_COLORS = ["#9f4f24", "#2563bb", "#2f7d52", "#6c4dab", "#b04545", "#b8862e"];
const CAPTION_COLOR = "#7196ce";
const HIDDEN_CAPTION_COLOR = "#b8c1cf";
const SPEECH_COLOR = "#487fbd";

interface RasterSegment {
  color: string;
  end: number;
  id: string;
  muted?: boolean;
  selected?: boolean;
  start: number;
}

interface RasterLayerProps {
  className?: string;
  duration: number;
  segments: RasterSegment[];
}

const TimelineRasterLayer = memo(function TimelineRasterLayer({
  className = "",
  duration,
  segments,
}: RasterLayerProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useLayoutEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const draw = () => {
      const bounds = canvas.getBoundingClientRect();
      if (bounds.width <= 0 || bounds.height <= 0) return;
      const ratio = Math.min(window.devicePixelRatio || 1, 2);
      const width = Math.max(1, Math.round(bounds.width * ratio));
      const height = Math.max(1, Math.round(bounds.height * ratio));
      if (canvas.width !== width || canvas.height !== height) {
        canvas.width = width;
        canvas.height = height;
      }
      const context = canvas.getContext("2d");
      if (!context) return;
      context.clearRect(0, 0, width, height);
      const safeDuration = Math.max(duration, 0.001);

      for (const segment of segments) {
        const left = Math.max(0, (segment.start / safeDuration) * width);
        const right = Math.min(width, (segment.end / safeDuration) * width);
        const segmentWidth = Math.max(1, right - left);
        context.globalAlpha = segment.muted ? 0.28 : 0.82;
        context.fillStyle = segment.color;
        context.fillRect(left, 3 * ratio, segmentWidth, height - 6 * ratio);
        if (segment.selected) {
          context.globalAlpha = 1;
          context.lineWidth = 2 * ratio;
          context.strokeStyle = "#2f73d9";
          context.strokeRect(
            left + ratio,
            2 * ratio,
            Math.max(ratio, segmentWidth - 2 * ratio),
            height - 4 * ratio,
          );
        }
      }
      context.globalAlpha = 1;
    };

    draw();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(draw);
    observer.observe(canvas);
    return () => observer.disconnect();
  }, [duration, segments]);

  return <canvas aria-hidden="true" className={`timeline-raster-layer ${className}`} ref={canvasRef} />;
});

function clock(seconds: number) {
  const safe = Math.max(0, seconds);
  const minutes = Math.floor(safe / 60);
  const rest = Math.floor(safe % 60);
  return `${String(minutes).padStart(2, "0")}:${String(rest).padStart(2, "0")}`;
}

interface TrackLayerProps {
  broll: BrollOverview;
  chapters: ChapterRow[];
  cuts: CutSummary[];
  doc: Doc;
  duration: number;
  lang: Lang;
  music: MusicTrack[];
  rows: SubtitleRow[];
  titles: TitleClip[];
  selectedBrollId: string | null;
  selectedCueId: string | null;
  selectedCueIds: string[];
  selectedMusicId: string | null;
  selectedTitleId: string | null;
  contactSheet: string | null;
  waveform: string | null;
  onSelectCue: (id: string, mode: "replace" | "range" | "toggle") => void;
  onCuePointerDown: (
    event: React.PointerEvent<HTMLButtonElement>,
    cue: SubtitleRow,
  ) => void;
  onCuePointerMove: (event: React.PointerEvent<HTMLButtonElement>) => void;
  onCuePointerUp: (event: React.PointerEvent<HTMLButtonElement>) => void;
  onCuePointerCancel: (event: React.PointerEvent<HTMLButtonElement>) => void;
  onBrollPointerDown: (
    event: React.PointerEvent<HTMLButtonElement>,
    placement: BrollPlacement,
  ) => void;
  onBrollPointerMove: (event: React.PointerEvent<HTMLButtonElement>) => void;
  onBrollPointerUp: (event: React.PointerEvent<HTMLButtonElement>) => void;
  onMusicPointerCancel: (event: React.PointerEvent<HTMLButtonElement>) => void;
  onMusicPointerDown: (
    event: React.PointerEvent<HTMLButtonElement>,
    track: MusicTrack,
  ) => void;
  onMusicPointerMove: (event: React.PointerEvent<HTMLButtonElement>) => void;
  onMusicPointerUp: (event: React.PointerEvent<HTMLButtonElement>) => void;
  onSelectMusic: (track: MusicTrack) => void;
  onSelectTitle: (title: TitleClip) => void;
  onTitlePointerDown: (
    event: React.PointerEvent<HTMLButtonElement>,
    title: TitleClip,
  ) => void;
  onTitlePointerMove: (event: React.PointerEvent<HTMLButtonElement>) => void;
  onTitlePointerUp: (event: React.PointerEvent<HTMLButtonElement>) => void;
  onSeek: (seconds: number, autoplay?: boolean) => void;
}

const TimelineTrackLayers = memo(function TimelineTrackLayers({
  broll,
  chapters,
  cuts,
  doc,
  duration,
  lang,
  music,
  rows,
  titles,
  selectedBrollId,
  selectedCueId,
  selectedCueIds,
  selectedMusicId,
  selectedTitleId,
  contactSheet,
  waveform,
  onSelectCue,
  onCuePointerDown,
  onCuePointerMove,
  onCuePointerUp,
  onCuePointerCancel,
  onBrollPointerDown,
  onBrollPointerMove,
  onBrollPointerUp,
  onMusicPointerCancel,
  onMusicPointerDown,
  onMusicPointerMove,
  onMusicPointerUp,
  onSelectMusic,
  onSelectTitle,
  onTitlePointerDown,
  onTitlePointerMove,
  onTitlePointerUp,
  onSeek,
}: TrackLayerProps) {
  const timelineCuts = useMemo(
    () => resolveTimelineCuts(doc, cuts),
    [cuts, doc],
  );
  const musicSourceRanges = useMemo(() => new Map(music.map((track) => [
    track.id,
    {
      end: editedToSourceTime(track.end, timelineCuts, duration),
      start: editedToSourceTime(track.start, timelineCuts, duration),
    },
  ])), [duration, music, timelineCuts]);
  const musicLanes = useMemo(() => musicTrackLaneLayout(music).lanes, [music]);
  const speakerColors = useMemo(() => {
    const speakers = [...new Set(rows.map((row) => row.speaker || "unlabelled"))];
    return new Map(speakers.map((speaker, index) => [
      speaker,
      SPEAKER_COLORS[index % SPEAKER_COLORS.length],
    ]));
  }, [rows]);
  const speakerSegments = useMemo(() => rows.reduce<Array<{
    id: string;
    speaker: string;
    start: number;
    end: number;
    text: string;
  }>>((segments, row) => {
    const speaker = row.speaker || "unlabelled";
    const previous = segments[segments.length - 1];
    if (previous && previous.speaker === speaker && row.start - previous.end <= 0.35) {
      previous.end = Math.max(previous.end, row.end);
      previous.text = `${previous.text} ${row.text}`.trim();
    } else {
      segments.push({
        id: row.id,
        speaker,
        start: row.start,
        end: row.end,
        text: row.text,
      });
    }
    return segments;
  }, []), [rows]);
  const ticks = useMemo(
    () => Array.from({ length: 9 }, (_, index) => (duration * index) / 8),
    [duration],
  );
  const cutRegions = useMemo(() => {
    const wordTimes = new Map(
      doc.paragraphs.flatMap((paragraph) =>
        paragraph.sentences.flatMap((sentence) => sentence.words),
      ).map((word) => [word.id, word]),
    );
    return cuts.flatMap((cut) => {
      const left = wordTimes.get(cut.a_word);
      const right = wordTimes.get(cut.b_word);
      if (!left || !right) return [];
      const end = cut.kind === "silence" ? right.start : right.end;
      const start = cut.kind === "silence" ? Math.max(0, end - cut.duration) : left.start;
      return [{ ...cut, start, end }];
    });
  }, [cuts, doc.paragraphs]);
  const speechSegments = useMemo<RasterSegment[]>(() => rows.map((row) => ({
    color: SPEECH_COLOR,
    end: row.end,
    id: row.id,
    start: row.start,
  })), [rows]);
  const speakerRasterSegments = useMemo<RasterSegment[]>(() => speakerSegments.map((segment) => ({
    color: speakerColors.get(segment.speaker) ?? SPEAKER_COLORS[0],
    end: segment.end,
    id: segment.id,
    start: segment.start,
  })), [speakerColors, speakerSegments]);
  const captionRasterSegments = useMemo<RasterSegment[]>(() => {
    const selectedIds = new Set(selectedCueIds);
    return rows.map((row) => ({
      color: row.hidden ? HIDDEN_CAPTION_COLOR : CAPTION_COLOR,
      end: row.end,
      id: row.id,
      muted: row.hidden,
      selected: selectedIds.has(row.id),
      start: row.start,
    }));
  }, [rows, selectedCueIds]);
  const cueAtTime = (time: number) => {
    let low = 0;
    let high = rows.length - 1;
    let nearest = -1;
    while (low <= high) {
      const middle = Math.floor((low + high) / 2);
      if (rows[middle].start <= time) {
        nearest = middle;
        low = middle + 1;
      } else {
        high = middle - 1;
      }
    }
    if (nearest >= 0 && time <= rows[nearest].end) return rows[nearest];
    const next = rows[nearest + 1];
    const previous = rows[nearest];
    if (!previous) return next ?? null;
    if (!next) return previous;
    return time - previous.end <= next.start - time ? previous : next;
  };
  const timeFromPointer = (event: React.MouseEvent<HTMLElement>) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    return Math.min(duration, Math.max(0, ((event.clientX - bounds.left) / bounds.width) * duration));
  };

  return (
    <>
      <div className="dock-ruler">
        {ticks.map((tick) => (
          <span key={tick} style={{ left: `${(tick / duration) * 100}%` }}>
            {clock(tick)}
          </span>
        ))}
        {chapters.map((chapter, index) => (
          <button
            aria-label={`${lang === "zh" ? "跳到章节" : "Seek to chapter"} ${index + 1}: ${chapter.title}`}
            className="dock-chapter-marker"
            key={chapter.startSeg}
            onClick={(event) => {
              event.stopPropagation();
              onSeek(chapter.start, false);
            }}
            style={{ left: `${(chapter.start / duration) * 100}%` }}
            title={`${clock(chapter.start)} · ${chapter.title}`}
          >
            {index + 1}
          </button>
        ))}
      </div>
      <div className="dock-track title-track">
        {titles.map((title) => (
          <button
            aria-label={`${title.text} ${clock(title.start)}–${clock(title.end)}`}
            className={selectedTitleId === title.id ? "selected" : ""}
            key={title.id}
            onClick={(event) => {
              event.stopPropagation();
              onSelectTitle(title);
              onSeek(title.start, false);
            }}
            onPointerDown={(event) => onTitlePointerDown(event, title)}
            onPointerMove={onTitlePointerMove}
            onPointerUp={onTitlePointerUp}
            style={{
              left: `${(title.start / duration) * 100}%`,
              width: `${Math.max(((title.end - title.start) / duration) * 100, 0.5)}%`,
            }}
            title={title.text}
          >
            <span aria-hidden="true" className="title-trim-handle start" data-handle="start" />
            <span className="title-track-label">{title.text}</span>
            <span aria-hidden="true" className="title-trim-handle end" data-handle="end" />
          </button>
        ))}
      </div>
      <div className="dock-track media-track">
        <div className="media-clip">
          {contactSheet && <img alt="" className="timeline-contact-sheet" src={contactSheet} />}
          {waveform && <img alt="" className="timeline-waveform" src={waveform} />}
          <span>{doc.media.path.split(/[\\/]/).pop()}</span>
          {!waveform && (
            <TimelineRasterLayer
              className="speech-activity"
              duration={duration}
              segments={speechSegments}
            />
          )}
        </div>
        {music.map((track) => {
          const sourceRange = musicSourceRanges.get(track.id);
          if (!sourceRange) return null;
          const selected = selectedMusicId === track.id;
          const lane = musicLanes.get(track.id) ?? 0;
          return (
          <button
            aria-label={`${lang === "zh" ? "背景音乐" : "Background music"}: ${
              track.path.split(/[\\/]/).pop()
            } ${clock(track.start)}–${clock(track.end)}`}
            aria-pressed={selected}
            className={`timeline-music-clip${selected ? " selected" : ""}`}
            key={track.id}
            onClick={(event) => {
              event.stopPropagation();
              onSelectMusic(track);
              onSeek(sourceRange.start, false);
            }}
            onPointerCancel={onMusicPointerCancel}
            onPointerDown={(event) => onMusicPointerDown(event, track)}
            onPointerMove={onMusicPointerMove}
            onPointerUp={onMusicPointerUp}
            style={{
              bottom: `${1 + lane * 10}px`,
              left: `${(sourceRange.start / duration) * 100}%`,
              width: `${Math.max(
                ((sourceRange.end - sourceRange.start) / duration) * 100,
                0.5,
              )}%`,
            }}
            title={lang === "zh"
              ? "拖动音乐片段或两侧手柄；时间按剪辑后的成片计算"
              : "Drag the music clip or either trim handle; timing follows the edited program"}
            type="button"
          >
            <span aria-hidden="true" className="music-trim-handle start" data-handle="start" />
            <span className="timeline-music-clip-label">
              ♫ {track.path.split(/[\\/]/).pop()}
            </span>
            <span aria-hidden="true" className="music-trim-handle end" data-handle="end" />
          </button>
          );
        })}
      </div>
      <div
        aria-label={lang === "zh" ? "说话人轨道" : "Speaker track"}
        className="dock-track speaker-track"
        onClick={(event) => {
          event.stopPropagation();
          onSeek(timeFromPointer(event), false);
        }}
        role="group"
      >
        <TimelineRasterLayer duration={duration} segments={speakerRasterSegments} />
      </div>
      <div
        aria-activedescendant={selectedCueId ? `timeline-cue-${selectedCueId}` : undefined}
        aria-label={lang === "zh" ? "字幕轨道；左右方向键切换字幕" : "Caption track; use arrow keys to change cue"}
        className="dock-track caption-track"
        onClick={(event) => {
          event.stopPropagation();
          const row = cueAtTime(timeFromPointer(event));
          if (!row) return;
          onSelectCue(
            row.id,
            event.shiftKey ? "range" : event.metaKey || event.ctrlKey ? "toggle" : "replace",
          );
          onSeek(row.start, false);
        }}
        onKeyDown={(event) => {
          if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
          event.preventDefault();
          const current = rows.findIndex((row) => row.id === selectedCueId);
          const direction = event.key === "ArrowLeft" ? -1 : 1;
          const next = Math.min(rows.length - 1, Math.max(0, current < 0 ? 0 : current + direction));
          const row = rows[next];
          if (!row) return;
          onSelectCue(row.id, event.shiftKey ? "range" : "replace");
          onSeek(row.start, false);
        }}
        role="listbox"
        tabIndex={0}
      >
        <TimelineRasterLayer duration={duration} segments={captionRasterSegments} />
        {selectedCueId && rows.find((row) => row.id === selectedCueId) && (() => {
          const cue = rows.find((row) => row.id === selectedCueId)!;
          return (
            <button
              aria-label={`${lang === "zh" ? "调整字幕时码" : "Adjust cue timing"}: ${cue.text}`}
              className="caption-trim-overlay"
              onClick={(event) => {
                event.stopPropagation();
                onSelectCue(cue.id, "replace");
                onSeek(cue.start, false);
              }}
              onPointerDown={(event) => onCuePointerDown(event, cue)}
              onPointerMove={onCuePointerMove}
              onPointerUp={onCuePointerUp}
              onPointerCancel={onCuePointerCancel}
              style={{
                left: `${(cue.start / duration) * 100}%`,
                width: `${Math.max(((cue.end - cue.start) / duration) * 100, 0.5)}%`,
              }}
              title={lang === "zh" ? "拖动字幕或两侧手柄精修时码" : "Drag the cue or either trim handle"}
            >
              <span aria-hidden="true" className="caption-trim-handle start" data-handle="start" />
              <span aria-hidden="true" className="caption-trim-handle end" data-handle="end" />
            </button>
          );
        })()}
        {selectedCueId && (
          <span className="timeline-cue-announcement" id={`timeline-cue-${selectedCueId}`} role="option">
            {selectedCueIds.length > 1
              ? `${selectedCueIds.length} ${lang === "zh" ? "段已选择" : "cues selected"}`
              : rows.find((row) => row.id === selectedCueId)?.text}
          </span>
        )}
      </div>
      <div className="dock-track broll-track">
        {broll.accepted.map((placement) => (
          <button
            aria-label={`${placement.name || placement.file} ${clock(placement.start)}–${clock(placement.end)}`}
            className={selectedBrollId === placement.id ? "selected" : ""}
            key={placement.id}
            onClick={(event) => {
              event.stopPropagation();
              onSeek(placement.start, false);
            }}
            onPointerDown={(event) => onBrollPointerDown(event, placement)}
            onPointerMove={onBrollPointerMove}
            onPointerUp={onBrollPointerUp}
            style={{
              left: `${(placement.start / duration) * 100}%`,
              width: `${Math.max(((placement.end - placement.start) / duration) * 100, 0.4)}%`,
            }}
            title={placement.name || placement.file}
          >
            <span aria-hidden="true" className="broll-trim-handle start" data-handle="start" />
            <span aria-hidden="true" className="broll-trim-handle end" data-handle="end" />
          </button>
        ))}
      </div>
      {cutRegions.map((cut) => (
        <span
          aria-hidden="true"
          className="dock-cut-marker"
          key={cut.id}
          style={{
            left: `${(cut.start / duration) * 100}%`,
            width: `${Math.max(((cut.end - cut.start) / duration) * 100, 0.18)}%`,
          }}
        />
      ))}
    </>
  );
});

interface BrollDrag {
  end: number;
  id: string;
  mode: "move" | "start" | "end";
  originEnd: number;
  originStart: number;
  originX: number;
  snap?: number;
  start: number;
}

interface TitleDrag {
  end: number;
  id: string;
  mode: "move" | "start" | "end";
  originEnd: number;
  originStart: number;
  originX: number;
  snap?: number;
  start: number;
}

export interface MusicDrag {
  end: number;
  id: string;
  mode: "move" | "start" | "end";
  originEnd: number;
  originPointerProgram: number;
  originSourceStart: number;
  originStart: number;
  snap?: number;
  sourceStart: number;
  start: number;
}

export interface CueDrag {
  end: number;
  id: string;
  mode: "move" | "start" | "end";
  originEnd: number;
  originStart: number;
  originX: number;
  snap?: number;
  start: number;
}

export function resolveMusicDrag(
  drag: MusicDrag,
  delta: number,
  programDuration: number,
  candidates: number[],
  threshold: number,
): MusicDrag {
  const upper = Math.max(0, programDuration);
  const withValidSnap = (
    next: MusicDrag,
    snapped: ReturnType<typeof nearestSnap>,
    clamped: number,
  ) => ({
    ...next,
    snap: snapped.snap !== undefined && Math.abs(clamped - snapped.value) < 0.001
      ? snapped.snap
      : undefined,
  });

  if (drag.mode === "start") {
    const lower = Math.max(0, drag.originStart - drag.originSourceStart);
    const maximum = Math.max(lower, Math.min(upper, drag.originEnd - 0.1));
    const raw = Math.min(maximum, Math.max(lower, drag.originStart + delta));
    const snapped = nearestSnap(raw, candidates, threshold);
    const start = Math.min(maximum, Math.max(lower, snapped.value));
    return withValidSnap({
      ...drag,
      sourceStart: Math.max(
        0,
        drag.originSourceStart + start - drag.originStart,
      ),
      start,
    }, snapped, start);
  }
  if (drag.mode === "end") {
    const minimum = Math.min(upper, Math.max(0, drag.originStart + 0.1));
    const raw = Math.max(minimum, Math.min(upper, drag.originEnd + delta));
    const snapped = nearestSnap(raw, candidates, threshold);
    const end = Math.max(minimum, Math.min(upper, snapped.value));
    return withValidSnap({ ...drag, end }, snapped, end);
  }

  const span = Math.min(upper, Math.max(0.1, drag.originEnd - drag.originStart));
  const maximumStart = Math.max(0, upper - span);
  const rawStart = Math.min(maximumStart, Math.max(0, drag.originStart + delta));
  const startSnap = nearestSnap(rawStart, candidates, threshold);
  const endSnap = nearestSnap(rawStart + span, candidates, threshold);
  const snapped = startSnap.distance <= endSnap.distance ? startSnap : endSnap;
  const snappedStart = snapped === endSnap ? snapped.value - span : snapped.value;
  const start = Math.min(maximumStart, Math.max(0, snappedStart));
  return withValidSnap(
    { ...drag, start, end: start + span },
    snapped,
    snapped === endSnap ? start + span : start,
  );
}

export function nearestSnap(
  value: number,
  candidates: number[],
  threshold: number,
): { value: number; snap?: number; distance: number } {
  let best = value;
  let distance = threshold + Number.EPSILON;
  for (const candidate of candidates) {
    const nextDistance = Math.abs(candidate - value);
    if (nextDistance < distance) {
      best = candidate;
      distance = nextDistance;
    }
  }
  return distance <= threshold
    ? { value: best, snap: best, distance }
    : { value, distance: Number.POSITIVE_INFINITY };
}

export function resolveCueDrag(
  drag: CueDrag,
  delta: number,
  earliest: number,
  latest: number,
  candidates: number[],
  threshold: number,
): CueDrag {
  const lower = Math.max(0, earliest);
  const upper = Math.max(lower, latest);
  const withValidSnap = (
    next: CueDrag,
    snapped: ReturnType<typeof nearestSnap>,
    clamped: number,
  ) => ({
    ...next,
    snap: snapped.snap !== undefined && Math.abs(clamped - snapped.value) < 0.001
      ? snapped.snap
      : undefined,
  });

  if (drag.mode === "start") {
    const maximum = Math.max(lower, Math.min(upper, drag.originEnd - 0.1));
    const raw = Math.min(maximum, Math.max(lower, drag.originStart + delta));
    const snapped = nearestSnap(raw, candidates, threshold);
    const start = Math.min(maximum, Math.max(lower, snapped.value));
    return withValidSnap({ ...drag, start }, snapped, start);
  }
  if (drag.mode === "end") {
    const minimum = Math.min(upper, Math.max(lower, drag.originStart + 0.1));
    const raw = Math.max(minimum, Math.min(upper, drag.originEnd + delta));
    const snapped = nearestSnap(raw, candidates, threshold);
    const end = Math.max(minimum, Math.min(upper, snapped.value));
    return withValidSnap({ ...drag, end }, snapped, end);
  }

  const available = Math.max(0, upper - lower);
  const span = Math.min(available, Math.max(0.1, drag.originEnd - drag.originStart));
  const maximumStart = Math.max(lower, upper - span);
  const rawStart = Math.min(maximumStart, Math.max(lower, drag.originStart + delta));
  const startSnap = nearestSnap(rawStart, candidates, threshold);
  const endSnap = nearestSnap(rawStart + span, candidates, threshold);
  const snapped = startSnap.distance <= endSnap.distance ? startSnap : endSnap;
  const snappedStart = snapped === endSnap ? snapped.value - span : snapped.value;
  const start = Math.min(maximumStart, Math.max(lower, snappedStart));
  return withValidSnap(
    { ...drag, start, end: start + span },
    snapped,
    snapped === endSnap ? start + span : start,
  );
}

function brollInput(placement: BrollPlacement): BrollPlacementInput {
  return {
    background: placement.background,
    end: placement.end,
    file: placement.file,
    fit: placement.fit,
    mode: placement.mode,
    name: placement.name ?? null,
    radius: placement.radius,
    rect: placement.rect,
    sourceStart: placement.sourceStart,
    start: placement.start,
  };
}

function titleInput(title: TitleClip): TitleClipInput {
  const duration = Math.max(0, title.end - title.start);
  const fadeIn = Math.min(duration, Math.max(0, title.fadeIn || 0));
  const fadeOut = Math.min(duration - fadeIn, Math.max(0, title.fadeOut || 0));
  return {
    background: title.background,
    color: title.color,
    end: title.end,
    fadeIn,
    fadeOut,
    fontSize: title.fontSize,
    start: title.start,
    text: title.text,
    x: title.x,
    y: title.y,
  };
}

interface SplitTarget {
  boundaries: Array<{ at: number; time: number }>;
  end: number;
  id: string;
  start: number;
}

export function buildSplitTargetIndex(doc: Doc): SplitTarget[] {
  const targets: SplitTarget[] = [];
  for (const paragraph of doc.paragraphs) {
    for (const sentence of paragraph.sentences) {
      const words = sentence.words;
      if (words.length < 2) continue;
      const start = words[0]?.start ?? 0;
      const end = words[words.length - 1]?.end ?? 0;
      targets.push({
        boundaries: words.slice(1).map((word, index) => ({
          at: index + 1,
          time: (words[index].end + word.start) / 2,
        })),
        end,
        id: sentence.id,
        start,
      });
    }
  }
  return targets;
}

export function splitTargetAtPlayhead(
  targets: SplitTarget[],
  currentTime: number,
): { at: number; id: string; time: number } | null {
  let low = 0;
  let high = targets.length - 1;
  let candidate = -1;
  while (low <= high) {
    const middle = Math.floor((low + high) / 2);
    if (targets[middle].start < currentTime) {
      candidate = middle;
      low = middle + 1;
    } else {
      high = middle - 1;
    }
  }

  const sentence = candidate >= 0 ? targets[candidate] : undefined;
  if (!sentence || currentTime >= sentence.end) return null;

  low = 0;
  high = sentence.boundaries.length - 1;
  candidate = -1;
  while (low <= high) {
    const middle = Math.floor((low + high) / 2);
    if (sentence.boundaries[middle].time <= currentTime) {
      candidate = middle;
      low = middle + 1;
    } else {
      high = middle - 1;
    }
  }
  const left = candidate >= 0 ? sentence.boundaries[candidate] : undefined;
  const right = sentence.boundaries[candidate + 1];
  const nearest = !left
    ? right
    : !right
      ? left
      : currentTime - left.time <= right.time - currentTime
        ? left
        : right;
  return nearest ? { id: sentence.id, at: nearest.at, time: nearest.time } : null;
}

export function EditorTimelineDock({
  audioMix,
  autoCollapsed,
  broll,
  chapters,
  busy,
  collapsed,
  currentTime,
  cuts,
  doc,
  history,
  isPlaying,
  lang,
  pid,
  rows,
  titles,
  onAddTitle,
  onUpdateAudioMix,
  onOpenBroll,
  onRedo,
  onRemoveCues,
  onSeek,
  onSplit,
  onUpdateCueTiming,
  onTogglePlayback,
  onTogglePreviewCuts,
  onToggleCollapsed,
  onUndo,
  onUpdateBroll,
  onUpdateTitle,
  onRemoveTitle,
  previewCuts,
}: Props) {
  const initialDrafts = useMemo(
    () => initialTimelineDrafts(pid, audioMix),
    [audioMix, pid],
  );
  const [zoom, setZoom] = useState(1);
  const [snapping, setSnapping] = useState(true);
  const [selectedCueId, setSelectedCueId] = useState<string | null>(null);
  const [selectedCueIds, setSelectedCueIds] = useState<string[]>([]);
  const [cueDrag, setCueDrag] = useState<CueDrag | null>(null);
  const [selectedBrollId, setSelectedBrollId] = useState<string | null>(null);
  const [brollDrag, setBrollDrag] = useState<BrollDrag | null>(null);
  const [selectedTitleId, setSelectedTitleId] =
    useState<string | null>(initialDrafts.selectedTitleId);
  const [titleDrag, setTitleDrag] = useState<TitleDrag | null>(null);
  const [titleDraft, setTitleDraft] =
    useState<TitleClipInput | null>(initialDrafts.titleDraft);
  const [titleDraftSource, setTitleDraftSource] =
    useState<TitleClipInput | null>(initialDrafts.titleSource);
  const [titlePanelOpen, setTitlePanelOpen] = useState(initialDrafts.titlePanelOpen);
  const [audioPanelOpen, setAudioPanelOpen] = useState(initialDrafts.audioPanelOpen);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [audioDraft, setAudioDraft] = useState(initialDrafts.audioDraft);
  const [audioDraftSource, setAudioDraftSource] = useState(initialDrafts.audioSource);
  const [selectedMusicId, setSelectedMusicId] = useState<string | null>(null);
  const [musicDrag, setMusicDrag] = useState<MusicDrag | null>(null);
  const [contactSheet, setContactSheet] = useState<string | null>(null);
  const [waveform, setWaveform] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const shortcutsButtonRef = useRef<HTMLButtonElement | null>(null);
  const shortcutsPanelRef = useRef<HTMLElement | null>(null);
  const duration = Math.max(doc.media.durationSeconds, 0.001);
  const cutIntervals = useMemo(() => resolveTimelineCuts(doc, cuts), [cuts, doc]);
  const programDuration = useMemo(
    () => editedTimelineDuration(duration, cutIntervals),
    [cutIntervals, duration],
  );
  const staticSnapPoints = useMemo(() => [
    0,
    duration,
    ...rows.flatMap((row) => [row.start, row.end]),
    ...broll.accepted.flatMap((placement) => [placement.start, placement.end]),
    ...titles.flatMap((title) => [title.start, title.end]),
  ].filter((value) => Number.isFinite(value)), [
    broll.accepted,
    duration,
    rows,
    titles,
  ]);
  const programSnapPoints = useMemo(() => [
    ...new Set([
      0,
      programDuration,
      ...staticSnapPoints.map((point) => sourceToEditedTime(point, cutIntervals)),
      ...audioDraft.music.flatMap((track) => [track.start, track.end]),
    ]),
  ].filter((value) => Number.isFinite(value)), [
    audioDraft.music,
    cutIntervals,
    programDuration,
    staticSnapPoints,
  ]);
  const displayedBroll = useMemo<BrollOverview>(() => brollDrag ? {
    ...broll,
    accepted: broll.accepted.map((placement) => placement.id === brollDrag.id
      ? { ...placement, start: brollDrag.start, end: brollDrag.end }
      : placement),
  } : broll, [broll, brollDrag]);
  const displayedTitles = useMemo(() => titleDrag
    ? titles.map((title) => title.id === titleDrag.id
      ? { ...title, start: titleDrag.start, end: titleDrag.end }
      : title)
    : titles, [titleDrag, titles]);
  const displayedRows = useMemo(() => cueDrag
    ? rows.map((row) => row.id === cueDrag.id
      ? { ...row, start: cueDrag.start, end: cueDrag.end }
      : row)
    : rows, [cueDrag, rows]);
  const displayedMusic = useMemo(() => {
    if (!musicDrag) return audioDraft.music;
    return audioDraft.music.map((track) => track.id === musicDrag.id ? {
      ...track,
      end: musicDrag.end,
      sourceStart: musicDrag.sourceStart,
      start: musicDrag.start,
    } : track);
  }, [audioDraft.music, musicDrag]);
  const musicLaneCount = useMemo(
    () => musicTrackLaneLayout(displayedMusic).count,
    [displayedMusic],
  );
  const musicTrackHeight = Math.max(24, 12 + Math.max(0, musicLaneCount - 1) * 10);
  const splitTargetIndex = useMemo(() => buildSplitTargetIndex(doc), [doc.paragraphs]);
  const splitTarget = useMemo(
    () => splitTargetAtPlayhead(splitTargetIndex, currentTime),
    [currentTime, splitTargetIndex],
  );
  const snapGuide = titleDrag?.snap ?? brollDrag?.snap ?? cueDrag?.snap;
  const displayedSnapGuide = musicDrag?.snap !== undefined
    ? editedToSourceTime(musicDrag.snap, cutIntervals, duration)
    : snapGuide;
  const selectedTitle = selectedTitleId
    ? titles.find((title) => title.id === selectedTitleId) ?? null
    : null;
  const selectedMusicTrack = selectedMusicId
    ? audioDraft.music.find((track) => track.id === selectedMusicId) ?? null
    : null;
  const selectedTitleInput = selectedTitle ? titleInput(selectedTitle) : null;
  const audioDirty = JSON.stringify(audioDraft) !== JSON.stringify(audioDraftSource);
  const musicValid = audioDraft.music.every((track) =>
    track.id.trim().length > 0
    && track.start >= 0
    && track.end > track.start
    && track.end <= programDuration
    && track.sourceStart >= 0
    && track.volume >= 0
    && track.volume <= 2);
  const titleDirty = titleDraft !== null && (
    selectedTitleId === null
    || titleDraftSource === null
    || JSON.stringify(titleDraft) !== JSON.stringify(titleDraftSource)
  );
  const audioConflict = audioDirty
    && JSON.stringify(audioMix) !== JSON.stringify(audioDraftSource);
  const titleConflict = titleDirty
    && selectedTitleInput !== null
    && titleDraftSource !== null
    && JSON.stringify(selectedTitleInput) !== JSON.stringify(titleDraftSource);

  useEffect(() => {
    if (JSON.stringify(audioMix) === JSON.stringify(audioDraftSource)) return;
    if (JSON.stringify(audioDraft) !== JSON.stringify(audioDraftSource)) return;
    setAudioDraft(audioMix);
    setAudioDraftSource(audioMix);
  }, [audioDraft, audioDraftSource, audioMix]);

  useEffect(() => {
    if (!selectedTitleId) return;
    if (!selectedTitleInput) {
      setSelectedTitleId(null);
      setTitleDraft(null);
      setTitleDraftSource(null);
      setTitlePanelOpen(false);
      return;
    }
    if (!titleDraftSource) {
      setTitleDraft((current) => current ?? selectedTitleInput);
      setTitleDraftSource(selectedTitleInput);
      return;
    }
    if (JSON.stringify(selectedTitleInput) === JSON.stringify(titleDraftSource)) return;
    if (titleDraft && JSON.stringify(titleDraft) !== JSON.stringify(titleDraftSource)) return;
    setTitleDraft(selectedTitleInput);
    setTitleDraftSource(selectedTitleInput);
  }, [selectedTitleId, selectedTitleInput, titleDraft, titleDraftSource]);

  useEffect(() => {
    try {
      const key = `${TIMELINE_DRAFTS_KEY_PREFIX}${pid}`;
      if (!audioDirty && !titleDirty) {
        localStorage.removeItem(key);
        return;
      }
      localStorage.setItem(key, JSON.stringify({
        audioDraft,
        audioSource: audioDraftSource,
        audioPanelOpen,
        selectedTitleId,
        titleDraft,
        titlePanelOpen,
        titleSource: titleDraftSource,
      } satisfies PersistedTimelineDrafts));
    } catch {
      // Keep the draft in memory if browser storage is unavailable.
    }
  }, [
    audioDraft,
    audioDraftSource,
    audioDirty,
    audioPanelOpen,
    pid,
    selectedTitleId,
    titleDraft,
    titleDraftSource,
    titleDirty,
    titlePanelOpen,
  ]);

  useEffect(() => {
    if (!audioDirty && !titleDirty) return;
    const warnOnClose = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", warnOnClose);
    return () => window.removeEventListener("beforeunload", warnOnClose);
  }, [audioDirty, titleDirty]);

  useEffect(() => {
    const validIds = new Set(rows.map((row) => row.id));
    setSelectedCueIds((current) => {
      const valid = current.filter((id) => validIds.has(id));
      return valid.length === current.length ? current : valid;
    });
    if (selectedCueId && !validIds.has(selectedCueId)) setSelectedCueId(null);
  }, [rows, selectedCueId]);

  useEffect(() => {
    if (!selectedMusicId) return;
    if (audioDraft.music.some((track) => track.id === selectedMusicId)) return;
    setSelectedMusicId(null);
    setMusicDrag((current) => current?.id === selectedMusicId ? null : current);
  }, [audioDraft.music, selectedMusicId]);

  const selectCue = useCallback((
    id: string,
    mode: "replace" | "range" | "toggle",
  ) => {
    setSelectedBrollId(null);
    setSelectedMusicId(null);
    if (!titleDirty) {
      setSelectedTitleId(null);
      setTitleDraft(null);
      setTitleDraftSource(null);
    }
    setTitlePanelOpen(false);
    if (mode === "range" && selectedCueId) {
      const anchor = rows.findIndex((row) => row.id === selectedCueId);
      const target = rows.findIndex((row) => row.id === id);
      if (anchor >= 0 && target >= 0) {
        const [start, end] = anchor <= target ? [anchor, target] : [target, anchor];
        setSelectedCueIds(rows.slice(start, end + 1).map((row) => row.id));
        setSelectedCueId(id);
        return;
      }
    }
    if (mode === "toggle") {
      if (selectedCueIds.includes(id)) {
        const next = selectedCueIds.filter((candidate) => candidate !== id);
        setSelectedCueIds(next);
        if (selectedCueId === id) setSelectedCueId(next[next.length - 1] ?? null);
      } else {
        setSelectedCueIds([...selectedCueIds, id]);
        setSelectedCueId(id);
      }
      return;
    }
    setSelectedCueIds([id]);
    setSelectedCueId(id);
  }, [rows, selectedCueId, selectedCueIds, titleDirty]);

  const clearCueSelection = useCallback(() => {
    setSelectedCueId(null);
    setSelectedCueIds([]);
  }, []);

  const beginCueDrag = useCallback((
    event: React.PointerEvent<HTMLButtonElement>,
    cue: SubtitleRow,
  ) => {
    if (busy) return;
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    const handle = (event.target as HTMLElement).dataset.handle;
    selectCue(cue.id, "replace");
    setCueDrag({
      end: cue.end,
      id: cue.id,
      mode: handle === "start" || handle === "end" ? handle : "move",
      originEnd: cue.end,
      originStart: cue.start,
      originX: event.clientX,
      start: cue.start,
    });
  }, [busy, selectCue]);

  const moveCueDrag = useCallback((event: React.PointerEvent<HTMLButtonElement>) => {
    if (!cueDrag || !event.currentTarget.hasPointerCapture(event.pointerId)) return;
    event.stopPropagation();
    const canvas = event.currentTarget.closest(".timeline-canvas");
    const width = canvas?.getBoundingClientRect().width ?? 0;
    if (width <= 0) return;
    const delta = ((event.clientX - cueDrag.originX) / width) * duration;
    const index = rows.findIndex((row) => row.id === cueDrag.id);
    if (index < 0) return;
    const earliest = index > 0 ? rows[index - 1].end : 0;
    const latest = index < rows.length - 1 ? rows[index + 1].start : duration;
    const threshold = snapping
      ? Math.min(0.5, Math.max(0.03, (8 / width) * duration))
      : 0;
    const candidates = staticSnapPoints.filter((point) =>
      Math.abs(point - cueDrag.originStart) > 0.001
      && Math.abs(point - cueDrag.originEnd) > 0.001);
    if (Math.abs(currentTime - cueDrag.originStart) > 0.001
      && Math.abs(currentTime - cueDrag.originEnd) > 0.001) {
      candidates.push(currentTime);
    }
    setCueDrag((current) => current
      ? resolveCueDrag(current, delta, earliest, latest, candidates, threshold)
      : null);
  }, [cueDrag, currentTime, duration, rows, snapping, staticSnapPoints]);

  const cancelCueDrag = useCallback((event: React.PointerEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    setCueDrag(null);
  }, []);

  const finishCueDrag = useCallback((event: React.PointerEvent<HTMLButtonElement>) => {
    if (!cueDrag) return;
    event.stopPropagation();
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    const cue = rows.find((row) => row.id === cueDrag.id);
    const next = cueDrag;
    setCueDrag(null);
    if (!cue
      || (Math.abs(cue.start - next.start) < 0.001
        && Math.abs(cue.end - next.end) < 0.001)) return;
    void onUpdateCueTiming(cue.id, next.start, next.end).catch(() => undefined);
  }, [cueDrag, onUpdateCueTiming, rows]);

  const removeSelectedCues = useCallback(async () => {
    if (selectedCueIds.length === 0) return;
    await onRemoveCues(selectedCueIds);
    clearCueSelection();
  }, [clearCueSelection, onRemoveCues, selectedCueIds]);

  useEffect(() => {
    if (selectedBrollId && !broll.accepted.some((placement) => placement.id === selectedBrollId)) {
      setSelectedBrollId(null);
    }
  }, [broll.accepted, selectedBrollId]);

  const selectTitle = useCallback((title: TitleClip) => {
    const input = titleInput(title);
    setSelectedTitleId(title.id);
    setSelectedBrollId(null);
    setSelectedMusicId(null);
    clearCueSelection();
    setTitleDraft(input);
    setTitleDraftSource(input);
    setAudioPanelOpen(false);
    setTitlePanelOpen(true);
  }, [clearCueSelection]);

  const beginTitleDrag = useCallback((
    event: React.PointerEvent<HTMLButtonElement>,
    title: TitleClip,
  ) => {
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    const handle = (event.target as HTMLElement).dataset.handle;
    selectTitle(title);
    setTitleDrag({
      end: title.end,
      id: title.id,
      mode: handle === "start" || handle === "end" ? handle : "move",
      originEnd: title.end,
      originStart: title.start,
      originX: event.clientX,
      start: title.start,
    });
  }, [selectTitle]);

  const moveTitleDrag = useCallback((event: React.PointerEvent<HTMLButtonElement>) => {
    if (!titleDrag || !event.currentTarget.hasPointerCapture(event.pointerId)) return;
    event.stopPropagation();
    const canvas = event.currentTarget.closest(".timeline-canvas");
    const width = canvas?.getBoundingClientRect().width ?? 0;
    if (width <= 0) return;
    const delta = ((event.clientX - titleDrag.originX) / width) * duration;
    const threshold = snapping
      ? Math.min(0.5, Math.max(0.03, (8 / width) * duration))
      : 0;
    const candidates = staticSnapPoints.filter((point) =>
      Math.abs(point - titleDrag.originStart) > 0.001
      && Math.abs(point - titleDrag.originEnd) > 0.001);
    if (Math.abs(currentTime - titleDrag.originStart) > 0.001
      && Math.abs(currentTime - titleDrag.originEnd) > 0.001) {
      candidates.push(currentTime);
    }
    setTitleDrag((current) => {
      if (!current) return null;
      if (current.mode === "start") {
        const raw = Math.min(current.originEnd - 0.1, Math.max(0, current.originStart + delta));
        const snapped = nearestSnap(raw, candidates, threshold);
        return {
          ...current,
          snap: snapped.snap,
          start: snapped.value,
        };
      }
      if (current.mode === "end") {
        const raw = Math.max(current.originStart + 0.1, Math.min(duration, current.originEnd + delta));
        const snapped = nearestSnap(raw, candidates, threshold);
        return {
          ...current,
          end: snapped.value,
          snap: snapped.snap,
        };
      }
      const span = current.originEnd - current.originStart;
      const rawStart = Math.min(duration - span, Math.max(0, current.originStart + delta));
      const startSnap = nearestSnap(rawStart, candidates, threshold);
      const endSnap = nearestSnap(rawStart + span, candidates, threshold);
      const snapped = startSnap.distance <= endSnap.distance ? startSnap : endSnap;
      const start = Math.min(
        duration - span,
        Math.max(0, snapped === endSnap ? snapped.value - span : snapped.value),
      );
      return { ...current, start, end: start + span, snap: snapped.snap };
    });
  }, [currentTime, duration, snapping, staticSnapPoints, titleDrag]);

  const finishTitleDrag = useCallback((event: React.PointerEvent<HTMLButtonElement>) => {
    if (!titleDrag) return;
    event.stopPropagation();
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    const title = titles.find((candidate) => candidate.id === titleDrag.id);
    const next = titleDrag;
    setTitleDrag(null);
    if (!title
      || (Math.abs(title.start - next.start) < 0.001
        && Math.abs(title.end - next.end) < 0.001)) return;
    const input = titleInput({ ...title, start: next.start, end: next.end });
    setTitleDraft(input);
    void onUpdateTitle(title.id, input).catch(() => undefined);
  }, [onUpdateTitle, titleDrag, titles]);

  const beginBrollDrag = useCallback((
    event: React.PointerEvent<HTMLButtonElement>,
    placement: BrollPlacement,
  ) => {
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    const handle = (event.target as HTMLElement).dataset.handle;
    setSelectedBrollId(placement.id);
    setSelectedMusicId(null);
    clearCueSelection();
    if (!titleDirty) {
      setSelectedTitleId(null);
      setTitleDraft(null);
      setTitleDraftSource(null);
    }
    setTitlePanelOpen(false);
    setBrollDrag({
      end: placement.end,
      id: placement.id,
      mode: handle === "start" || handle === "end" ? handle : "move",
      originEnd: placement.end,
      originStart: placement.start,
      originX: event.clientX,
      start: placement.start,
    });
  }, [clearCueSelection, titleDirty]);

  const moveBrollDrag = useCallback((event: React.PointerEvent<HTMLButtonElement>) => {
    if (!brollDrag || !event.currentTarget.hasPointerCapture(event.pointerId)) return;
    event.stopPropagation();
    const canvas = event.currentTarget.closest(".timeline-canvas");
    const width = canvas?.getBoundingClientRect().width ?? 0;
    if (width <= 0) return;
    const delta = ((event.clientX - brollDrag.originX) / width) * duration;
    const threshold = snapping
      ? Math.min(0.5, Math.max(0.03, (8 / width) * duration))
      : 0;
    const candidates = staticSnapPoints.filter((point) =>
      Math.abs(point - brollDrag.originStart) > 0.001
      && Math.abs(point - brollDrag.originEnd) > 0.001);
    if (Math.abs(currentTime - brollDrag.originStart) > 0.001
      && Math.abs(currentTime - brollDrag.originEnd) > 0.001) {
      candidates.push(currentTime);
    }
    setBrollDrag((current) => {
      if (!current) return null;
      if (current.mode === "start") {
        const raw = Math.min(current.originEnd - 0.1, Math.max(0, current.originStart + delta));
        const snapped = nearestSnap(raw, candidates, threshold);
        return { ...current, start: snapped.value, snap: snapped.snap };
      }
      if (current.mode === "end") {
        const raw = Math.max(current.originStart + 0.1, Math.min(duration, current.originEnd + delta));
        const snapped = nearestSnap(raw, candidates, threshold);
        return { ...current, end: snapped.value, snap: snapped.snap };
      }
      const span = current.originEnd - current.originStart;
      const rawStart = Math.min(duration - span, Math.max(0, current.originStart + delta));
      const startSnap = nearestSnap(rawStart, candidates, threshold);
      const endSnap = nearestSnap(rawStart + span, candidates, threshold);
      const snapped = startSnap.distance <= endSnap.distance ? startSnap : endSnap;
      const start = Math.min(
        duration - span,
        Math.max(0, snapped === endSnap ? snapped.value - span : snapped.value),
      );
      return { ...current, start, end: start + span, snap: snapped.snap };
    });
  }, [brollDrag, currentTime, duration, snapping, staticSnapPoints]);

  const finishBrollDrag = useCallback((event: React.PointerEvent<HTMLButtonElement>) => {
    if (!brollDrag) return;
    event.stopPropagation();
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    const placement = broll.accepted.find((candidate) => candidate.id === brollDrag.id);
    const next = brollDrag;
    setBrollDrag(null);
    if (!placement
      || (Math.abs(placement.start - next.start) < 0.001
        && Math.abs(placement.end - next.end) < 0.001)) return;
    void onUpdateBroll(next.id, brollInput({
      ...placement,
      end: next.end,
      start: next.start,
    })).catch(() => undefined);
  }, [broll.accepted, brollDrag, onUpdateBroll]);

  const selectMusic = useCallback((track: MusicTrack) => {
    setSelectedMusicId(track.id);
    setSelectedBrollId(null);
    clearCueSelection();
    if (!titleDirty) {
      setSelectedTitleId(null);
      setTitleDraft(null);
      setTitleDraftSource(null);
    }
    setTitlePanelOpen(false);
    setAudioPanelOpen(true);
  }, [clearCueSelection, titleDirty]);

  const removeSelectedMusic = useCallback(() => {
    if (!selectedMusicId) return;
    const music = audioDraft.music.filter((track) => track.id !== selectedMusicId);
    if (music.length === audioDraft.music.length) return;
    const nextMix = { ...audioDraft, music };
    const persistImmediately = !audioDirty && !audioConflict;
    setAudioDraft(nextMix);
    setSelectedMusicId(music[0]?.id ?? null);
    if (!persistImmediately) return;
    void onUpdateAudioMix(nextMix)
      .then(() => {
        setAudioDraftSource(nextMix);
        setAudioDraft((latest) =>
          JSON.stringify(latest) === JSON.stringify(nextMix) ? nextMix : latest);
      })
      .catch(() => undefined);
  }, [
    audioConflict,
    audioDirty,
    audioDraft,
    onUpdateAudioMix,
    selectedMusicId,
  ]);

  const beginMusicDrag = useCallback((
    event: React.PointerEvent<HTMLButtonElement>,
    track: MusicTrack,
  ) => {
    if (busy) return;
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    const canvas = event.currentTarget.closest(".timeline-canvas");
    const bounds = canvas?.getBoundingClientRect();
    if (!bounds || bounds.width <= 0) return;
    const sourceTime = Math.min(
      duration,
      Math.max(0, ((event.clientX - bounds.left) / bounds.width) * duration),
    );
    const handle = (event.target as HTMLElement).dataset.handle;
    selectMusic(track);
    setMusicDrag({
      end: track.end,
      id: track.id,
      mode: handle === "start" || handle === "end" ? handle : "move",
      originEnd: track.end,
      originPointerProgram: sourceToEditedTime(sourceTime, cutIntervals),
      originSourceStart: track.sourceStart,
      originStart: track.start,
      sourceStart: track.sourceStart,
      start: track.start,
    });
  }, [busy, cutIntervals, duration, selectMusic]);

  const moveMusicDrag = useCallback((
    event: React.PointerEvent<HTMLButtonElement>,
  ) => {
    if (!musicDrag || !event.currentTarget.hasPointerCapture(event.pointerId)) return;
    event.stopPropagation();
    const canvas = event.currentTarget.closest(".timeline-canvas");
    const bounds = canvas?.getBoundingClientRect();
    if (!bounds || bounds.width <= 0) return;
    const sourceTime = Math.min(
      duration,
      Math.max(0, ((event.clientX - bounds.left) / bounds.width) * duration),
    );
    const pointerProgram = sourceToEditedTime(sourceTime, cutIntervals);
    const delta = pointerProgram - musicDrag.originPointerProgram;
    const threshold = snapping
      ? Math.min(0.5, Math.max(0.03, (8 / bounds.width) * programDuration))
      : 0;
    const candidates = programSnapPoints.filter((point) =>
      Math.abs(point - musicDrag.originStart) > 0.001
      && Math.abs(point - musicDrag.originEnd) > 0.001);
    const playheadProgram = sourceToEditedTime(currentTime, cutIntervals);
    if (Math.abs(playheadProgram - musicDrag.originStart) > 0.001
      && Math.abs(playheadProgram - musicDrag.originEnd) > 0.001) {
      candidates.push(playheadProgram);
    }
    setMusicDrag((current) => current
      ? resolveMusicDrag(current, delta, programDuration, candidates, threshold)
      : null);
  }, [
    currentTime,
    cutIntervals,
    duration,
    musicDrag,
    programDuration,
    programSnapPoints,
    snapping,
  ]);

  const cancelMusicDrag = useCallback((
    event: React.PointerEvent<HTMLButtonElement>,
  ) => {
    event.stopPropagation();
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    setMusicDrag(null);
  }, []);

  const finishMusicDrag = useCallback((
    event: React.PointerEvent<HTMLButtonElement>,
  ) => {
    if (!musicDrag) return;
    event.stopPropagation();
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    const nextDrag = musicDrag;
    const currentMusic = audioDraft.music.find((track) => track.id === musicDrag.id);
    setMusicDrag(null);
    if (!currentMusic
      || (Math.abs(currentMusic.start - nextDrag.start) < 0.001
      && Math.abs(currentMusic.end - nextDrag.end) < 0.001
      && Math.abs(currentMusic.sourceStart - nextDrag.sourceStart) < 0.001)) return;

    const span = nextDrag.end - nextDrag.start;
    const fadeIn = Math.min(span, Math.max(0, currentMusic.fadeIn));
    const music = {
      ...currentMusic,
      end: nextDrag.end,
      fadeIn,
      fadeOut: Math.min(span - fadeIn, Math.max(0, currentMusic.fadeOut)),
      sourceStart: nextDrag.sourceStart,
      start: nextDrag.start,
    };
    const nextMix = {
      ...audioDraft,
      music: audioDraft.music.map((track) => track.id === music.id ? music : track),
    };
    const persistImmediately = !audioDirty && !audioConflict;
    setAudioDraft(nextMix);
    if (!persistImmediately) return;
    void onUpdateAudioMix(nextMix)
      .then(() => {
        setAudioDraftSource(nextMix);
        setAudioDraft((latest) =>
          JSON.stringify(latest) === JSON.stringify(nextMix) ? nextMix : latest);
      })
      .catch(() => undefined);
  }, [
    audioConflict,
    audioDirty,
    audioDraft,
    musicDrag,
    onUpdateAudioMix,
  ]);

  const beginNewTitle = () => {
    const start = Math.min(currentTime, Math.max(0, duration - 0.1));
    const end = Math.min(duration, Math.max(start + 0.1, start + 3));
    setSelectedTitleId(null);
    setTitleDraftSource(null);
    setSelectedBrollId(null);
    setSelectedMusicId(null);
    clearCueSelection();
    setAudioPanelOpen(false);
    setTitleDraft({
      background: "#00000099",
      color: "#FFFFFF",
      end,
      fadeIn: 0.25,
      fadeOut: 0.25,
      fontSize: 64,
      start,
      text: "",
      x: 0.5,
      y: 0.18,
    });
    setTitlePanelOpen(true);
  };

  const updateSelectedMusic = (update: Partial<MusicTrack>) => {
    if (!selectedMusicId) return;
    setAudioDraft((current) => ({
      ...current,
      music: current.music.map((track) => track.id === selectedMusicId
        ? { ...track, ...update }
        : track),
    }));
  };

  const addMusicTrack = () => {
    void pickAudioFile()
      .then((path) => {
        if (!path) return;
        const playhead = Math.min(
          programDuration,
          sourceToEditedTime(currentTime, cutIntervals),
        );
        const start = playhead >= programDuration - 0.1
          ? Math.max(0, programDuration - 10)
          : playhead;
        const end = Math.min(
          programDuration,
          Math.max(start + 0.1, start + 10),
        );
        const id = typeof crypto.randomUUID === "function"
          ? `music-${crypto.randomUUID()}`
          : `music-${Date.now()}`;
        setAudioDraft((current) => ({
          ...current,
          music: [...current.music, {
            id,
            path,
            start,
            end,
            sourceStart: 0,
            volume: 0.25,
            fadeIn: Math.min(1, end - start),
            fadeOut: Math.min(1, Math.max(0, end - start - 1)),
            ducking: true,
          }],
        }));
        setSelectedMusicId(id);
      })
      .catch(() => undefined);
  };

  const submitTitle = async () => {
    if (!titleDraft || !titleDraft.text.trim()) return;
    const duration = titleDraft.end - titleDraft.start;
    const fadeIn = Math.min(duration, Math.max(0, titleDraft.fadeIn));
    const fadeOut = Math.min(duration - fadeIn, Math.max(0, titleDraft.fadeOut));
    const normalized = {
      ...titleDraft,
      fadeIn,
      fadeOut,
      text: titleDraft.text.trim(),
    };
    if (selectedTitleId) {
      await onUpdateTitle(selectedTitleId, normalized);
      setTitleDraft(normalized);
      setTitleDraftSource(normalized);
    } else {
      await onAddTitle(normalized);
      setTitleDraft(null);
      setTitleDraftSource(null);
      setTitlePanelOpen(false);
    }
  };

  const submitAudioMix = async () => {
    const fadeIn = Math.min(programDuration, Math.max(0, audioDraft.fadeIn));
    const fadeOut = Math.min(
      programDuration - fadeIn,
      Math.max(0, audioDraft.fadeOut),
    );
    const music = audioDraft.music.map((track) => {
      const start = Math.min(programDuration, Math.max(0, track.start));
      const end = Math.min(programDuration, Math.max(start, track.end));
      const duration = end - start;
      const fadeIn = Math.min(duration, Math.max(0, track.fadeIn));
      const fadeOut = Math.min(
        duration - fadeIn,
        Math.max(0, track.fadeOut),
      );
      return {
        ...track,
        start,
        end,
        sourceStart: Math.max(0, track.sourceStart),
        volume: Math.min(2, Math.max(0, track.volume)),
        fadeIn,
        fadeOut,
      };
    });
    const normalized = {
      ...audioDraft,
      fadeIn,
      fadeOut,
      volume: Math.min(2, Math.max(0, audioDraft.volume)),
      music,
    };
    await onUpdateAudioMix(normalized);
    setAudioDraft(normalized);
    setAudioDraftSource(normalized);
    setAudioPanelOpen(false);
  };

  useEffect(() => {
    let disposed = false;
    let retry: number | null = null;
    const load = async () => {
      try {
        const result = await timelineVisuals(doc.id);
        if (disposed) return;
        if (result.deferred) {
          retry = window.setTimeout(() => void load(), 3000);
          return;
        }
        setContactSheet(result.contactSheet ? convertFileSrc(result.contactSheet) : null);
        setWaveform(result.waveform ? convertFileSrc(result.waveform) : null);
      } catch {
        if (!disposed) {
          setContactSheet(null);
          setWaveform(null);
        }
      }
    };
    setContactSheet(null);
    setWaveform(null);
    void load();
    return () => {
      disposed = true;
      if (retry !== null) window.clearTimeout(retry);
    };
  }, [doc.id]);

  useEffect(() => {
    if (!isPlaying || zoom <= 1) return;
    const scroll = scrollRef.current;
    if (!scroll) return;
    const playhead = (currentTime / duration) * scroll.scrollWidth;
    const edge = scroll.clientWidth * 0.72;
    const visible = playhead - scroll.scrollLeft;
    if (visible < scroll.clientWidth * 0.18 || visible > edge) {
      scroll.scrollTo({
        behavior: "auto",
        left: Math.max(0, playhead - scroll.clientWidth * 0.35),
      });
    }
  }, [currentTime, duration, isPlaying, zoom]);

  useEffect(() => {
    if (shortcutsOpen) shortcutsPanelRef.current?.focus();
  }, [shortcutsOpen]);

  const seekFromPointer = (event: React.MouseEvent<HTMLDivElement>) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    onSeek(((event.clientX - bounds.left) / bounds.width) * duration, false);
  };

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;
      const target = event.target instanceof Element ? event.target : null;
      const isInteractive = target?.matches(
        "button, a[href], input, textarea, select, summary, [contenteditable='true'], [role='button']",
      );
      const modifier = event.metaKey || event.ctrlKey;
      if (event.key === "Escape" && shortcutsOpen) {
        event.preventDefault();
        setShortcutsOpen(false);
        shortcutsButtonRef.current?.focus();
        return;
      }
      if (modifier && event.key.toLocaleLowerCase() === "z") {
        if (busy || isInteractive) return;
        event.preventDefault();
        void (event.shiftKey ? onRedo() : onUndo());
        return;
      }
      if (modifier && (event.key === "=" || event.key === "+")) {
        if (isInteractive) return;
        event.preventDefault();
        setZoom((value) => Math.min(4, value + 0.5));
        return;
      }
      if (modifier && event.key === "-") {
        if (isInteractive) return;
        event.preventDefault();
        setZoom((value) => Math.max(1, value - 0.5));
        return;
      }
      if (!modifier && !event.altKey && event.code === "Space") {
        if (isInteractive) return;
        event.preventDefault();
        onTogglePlayback();
        return;
      }
      if (!modifier && !event.altKey && event.key === "?" && !isInteractive) {
        event.preventDefault();
        if (shortcutsOpen) {
          setShortcutsOpen(false);
          shortcutsButtonRef.current?.focus();
        } else {
          setShortcutsOpen(true);
        }
        return;
      }
      if (!modifier && !event.altKey && (event.key === "ArrowLeft" || event.key === "ArrowRight")) {
        if (isInteractive) return;
        event.preventDefault();
        const amount = event.shiftKey ? 0.1 : 1;
        onSeek(
          Math.min(
            duration,
            Math.max(0, currentTime + (event.key === "ArrowLeft" ? -amount : amount)),
          ),
          false,
        );
        return;
      }
      if (!modifier && !event.altKey && event.key.toLocaleLowerCase() === "s") {
        if (busy || isInteractive || !splitTarget) return;
        event.preventDefault();
        void onSplit(splitTarget.id, splitTarget.at);
        return;
      }
      if (!modifier && !event.altKey && (event.key === "Backspace" || event.key === "Delete")) {
        if (busy
          || isInteractive
          || (!selectedTitleId && !selectedMusicId && selectedCueIds.length === 0)) return;
        event.preventDefault();
        if (selectedTitleId) {
          void onRemoveTitle(selectedTitleId);
        } else if (selectedMusicId) {
          removeSelectedMusic();
        } else {
          void removeSelectedCues();
        }
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [
    busy,
    currentTime,
    duration,
    onRedo,
    onRemoveTitle,
    onSeek,
    onSplit,
    onTogglePlayback,
    onUndo,
    removeSelectedCues,
    removeSelectedMusic,
    selectedCueIds.length,
    selectedMusicId,
    selectedTitleId,
    shortcutsOpen,
    splitTarget,
  ]);

  return (
    <section className={`workbench-timeline${collapsed ? " collapsed" : ""}`} aria-label={lang === "zh" ? "编辑时间线" : "Editing timeline"}>
      <header className="timeline-dock-toolbar">
        <div className={`timeline-edit-actions${audioPanelOpen || titlePanelOpen ? " popover-open" : ""}`}>
          <button
            aria-label={lang === "zh" ? "撤销" : "Undo"}
            className="timeline-icon-button"
            disabled={busy || !history.canUndo}
            onClick={() => void onUndo()}
            title={history.undoLabel
              ? `${lang === "zh" ? "撤销" : "Undo"}：${history.undoLabel} (⌘Z)`
              : `${lang === "zh" ? "没有可撤销的编辑" : "Nothing to undo"} (⌘Z)`}
          >↶</button>
          <button
            aria-label={lang === "zh" ? "重做" : "Redo"}
            className="timeline-icon-button"
            disabled={busy || !history.canRedo}
            onClick={() => void onRedo()}
            title={history.redoLabel
              ? `${lang === "zh" ? "重做" : "Redo"}：${history.redoLabel} (⇧⌘Z)`
              : `${lang === "zh" ? "没有可重做的编辑" : "Nothing to redo"} (⇧⌘Z)`}
          >↷</button>
          <span className="timeline-toolbar-divider" />
          <button
            className="timeline-tool-button"
            disabled={busy || !splitTarget}
            onClick={() => splitTarget && void onSplit(splitTarget.id, splitTarget.at)}
            title={splitTarget
              ? `${lang === "zh" ? "在播放头附近的词间拆分" : "Split at the word boundary nearest the playhead"} (S)`
              : lang === "zh"
                ? "把播放头放在至少包含两个词的字幕中"
                : "Place the playhead inside a cue with at least two words"}
            type="button"
          >
            {lang === "zh" ? "拆分" : "Split"} · S
          </button>
          <button
            className="timeline-tool-button"
            disabled={busy || selectedCueIds.length === 0}
            onClick={() => void removeSelectedCues()}
            title={selectedCueIds.length > 0
              ? lang === "zh"
                ? `将所选 ${selectedCueIds.length} 段字幕对应的画面和声音作为一次编辑移除`
                : `Remove the picture and sound covered by ${selectedCueIds.length} selected cue(s) as one edit`
              : lang === "zh"
                ? "先在字幕轨道选择一个片段"
                : "Select a cue on the caption track first"}
            type="button"
          >
            {lang === "zh"
              ? `移除${selectedCueIds.length > 1 ? ` ${selectedCueIds.length} 段` : "片段"}`
              : `Remove${selectedCueIds.length > 1 ? ` ${selectedCueIds.length} clips` : " clip"}`}
          </button>
          <button
            className="timeline-tool-button"
            disabled={busy}
            onClick={beginNewTitle}
            title={lang === "zh" ? "在播放头位置添加标题" : "Add a title at the playhead"}
            type="button"
          >
            + {lang === "zh" ? "标题" : "Title"}
          </button>
          <button className="timeline-tool-button" onClick={onOpenBroll} type="button">
            + B-roll
          </button>
          <button
            aria-pressed={snapping}
            className={`timeline-tool-button${snapping ? " active" : ""}`}
            onClick={() => setSnapping((value) => !value)}
            title={lang === "zh"
              ? "拖动标题、音乐和 B-roll 时吸附到播放头、字幕及其他片段边界"
              : "Snap titles, music, and B-roll to the playhead, cues, and other clip edges"}
            type="button"
          >
            {snapping ? "⌁ " : ""}
            {lang === "zh" ? "吸附" : "Snap"}
          </button>
          <button
            aria-pressed={previewCuts}
            className={`timeline-tool-button${previewCuts ? " active" : ""}`}
            disabled={cutIntervals.length === 0}
            onClick={onTogglePreviewCuts}
            title={lang === "zh"
              ? "播放时自动跳过已移除区间，节目监看与最终导出保持一致"
              : "Skip removed regions during playback so the program monitor matches export"}
            type="button"
          >
            {lang === "zh" ? "跳过切口" : "Preview cuts"}
          </button>
          <button
            aria-expanded={audioPanelOpen}
            className={`timeline-tool-button${audioMix.muted ? " active" : ""}`}
            disabled={busy}
            onClick={() => {
              setTitlePanelOpen(false);
              if (!selectedMusicId && audioDraft.music[0]) {
                setSelectedMusicId(audioDraft.music[0].id);
              }
              setAudioPanelOpen((value) => !value);
            }}
            title={lang === "zh"
              ? "调整原始音频音量、静音和成片淡入淡出"
              : "Adjust source volume, mute, and program fades"}
            type="button"
          >
            {audioMix.muted
              ? lang === "zh" ? "音频已静音" : "Audio muted"
              : `${lang === "zh" ? "音频" : "Audio"} ${Math.round(audioMix.volume * 100)}%`}
          </button>
          {audioPanelOpen && (
            <form
              className="timeline-audio-popover"
              noValidate
              onSubmit={(event) => {
                event.preventDefault();
                void submitAudioMix().catch(() => undefined);
              }}
            >
              <header>
                <strong>{lang === "zh" ? "原始音频" : "Source audio"}</strong>
                <button
                  aria-label={lang === "zh" ? "关闭音频设置" : "Close audio settings"}
                  onClick={() => setAudioPanelOpen(false)}
                  type="button"
                >
                  ×
                </button>
              </header>
              {audioConflict && (
                <div className="timeline-draft-conflict" role="alert">
                  <span>
                    {lang === "zh"
                      ? "保存的音频设置已在其他操作中改变。草稿尚未覆盖它。"
                      : "The saved audio settings changed elsewhere. Your draft has not overwritten them."}
                  </span>
                  <button
                    className="button-quiet"
                    onClick={() => {
                      setAudioDraft(audioMix);
                      setAudioDraftSource(audioMix);
                    }}
                    type="button"
                  >
                    {lang === "zh" ? "使用已保存版本" : "Use saved"}
                  </button>
                  <button
                    className="button-quiet"
                    onClick={() => setAudioDraftSource(audioMix)}
                    type="button"
                  >
                    {lang === "zh" ? "保留我的草稿" : "Keep draft"}
                  </button>
                </div>
              )}
              <label className="timeline-audio-mute">
                <input
                  checked={audioDraft.muted}
                  onChange={(event) => setAudioDraft((current) => ({
                    ...current,
                    muted: event.target.checked,
                  }))}
                  type="checkbox"
                />
                <span>{lang === "zh" ? "静音原始音频" : "Mute source audio"}</span>
              </label>
              <fieldset className="timeline-audio-processing">
                <legend>{lang === "zh" ? "对白处理" : "Dialogue processing"}</legend>
                <label className="timeline-audio-mute">
                  <input
                    checked={audioDraft.voiceEnhance}
                    onChange={(event) => setAudioDraft((current) => ({
                      ...current,
                      voiceEnhance: event.target.checked,
                    }))}
                    type="checkbox"
                  />
                  <span>
                    <strong>{lang === "zh" ? "增强对白" : "Enhance dialogue"}</strong>
                    <small>
                      {lang === "zh"
                        ? "轻度降噪、滤除低频轰鸣并压缩动态。"
                        : "Light denoise, rumble filtering, and dynamic compression."}
                    </small>
                  </span>
                </label>
                <label className="timeline-audio-mute">
                  <input
                    checked={audioDraft.normalizeLoudness}
                    onChange={(event) => setAudioDraft((current) => ({
                      ...current,
                      normalizeLoudness: event.target.checked,
                    }))}
                    type="checkbox"
                  />
                  <span>
                    <strong>{lang === "zh" ? "标准化响度" : "Normalize loudness"}</strong>
                    <small>
                      {lang === "zh"
                        ? "按节目响度统一音量，限制真峰值。"
                        : "Match program loudness and constrain true peak."}
                    </small>
                  </span>
                </label>
                {audioDraft.normalizeLoudness && (
                  <label>
                    <span>{lang === "zh" ? "目标响度（LUFS）" : "Target loudness (LUFS)"}</span>
                    <select
                      aria-label={lang === "zh" ? "目标响度" : "Target loudness"}
                      onChange={(event) => setAudioDraft((current) => ({
                        ...current,
                        loudnessTarget: Number(event.target.value),
                      }))}
                      value={audioDraft.loudnessTarget}
                    >
                      <option value={-14}>{lang === "zh" ? "网络视频 · -14" : "Web video · -14"}</option>
                      <option value={-16}>{lang === "zh" ? "对白节目 · -16" : "Spoken program · -16"}</option>
                      <option value={-18}>{lang === "zh" ? "保留动态 · -18" : "More dynamics · -18"}</option>
                      <option value={-23}>{lang === "zh" ? "广播交付 · -23" : "Broadcast · -23"}</option>
                    </select>
                  </label>
                )}
                {(audioDraft.voiceEnhance || audioDraft.normalizeLoudness) && (
                  <small className="timeline-audio-export-note">
                    {lang === "zh"
                      ? "这些处理在导出时由 FFmpeg 完整执行；节目监看仅预览音量、静音和淡入淡出。"
                      : "FFmpeg applies these processors during export; the monitor previews gain, mute, and fades only."}
                  </small>
                )}
              </fieldset>
              <fieldset className="timeline-audio-processing timeline-music-track">
                <legend>
                  {lang === "zh"
                    ? `背景音乐 · ${audioDraft.music.length} 段`
                    : `Background music · ${audioDraft.music.length} clips`}
                </legend>
                <div className="timeline-music-list-header">
                  <small>
                    {lang === "zh"
                      ? "每段音乐可独立定位、裁切、淡入淡出和压低。"
                      : "Each clip has independent timing, trims, fades, and ducking."}
                  </small>
                  <button
                    className="button-quiet"
                    disabled={busy}
                    onClick={addMusicTrack}
                    type="button"
                  >
                    + {lang === "zh" ? "添加音乐" : "Add music"}
                  </button>
                </div>
                {audioDraft.music.length > 0 && (
                  <div
                    aria-label={lang === "zh" ? "音乐片段列表" : "Music clip list"}
                    className="timeline-music-list"
                    role="listbox"
                  >
                    {audioDraft.music.map((track, index) => (
                      <button
                        aria-selected={selectedMusicId === track.id}
                        className={selectedMusicId === track.id ? "selected" : ""}
                        key={track.id}
                        onClick={() => setSelectedMusicId(track.id)}
                        role="option"
                        type="button"
                      >
                        <span>{index + 1}</span>
                        <strong>{track.path.split(/[\\/]/).pop()}</strong>
                        <small>{clock(track.start)}–{clock(track.end)}</small>
                      </button>
                    ))}
                  </div>
                )}
                {selectedMusicTrack ? (
                  <>
                    <div className="timeline-music-file">
                      <span title={selectedMusicTrack.path}>
                        {selectedMusicTrack.path.split(/[\\/]/).pop()}
                      </span>
                      <button
                        className="button-quiet"
                        onClick={removeSelectedMusic}
                        type="button"
                      >
                        {lang === "zh" ? "移除" : "Remove"}
                      </button>
                    </div>
                    <label>
                      <span>
                        {lang === "zh" ? "音乐音量" : "Music volume"} ·{" "}
                        {Math.round(selectedMusicTrack.volume * 100)}%
                      </span>
                      <input
                        max={200}
                        min={0}
                        onChange={(event) => updateSelectedMusic({
                          volume: event.target.valueAsNumber / 100,
                        })}
                        step={1}
                        type="range"
                        value={Math.round(selectedMusicTrack.volume * 100)}
                      />
                    </label>
                    <div className="timeline-music-grid">
                      <label>
                        <span>{lang === "zh" ? "成片开始" : "Program start"}</span>
                        <input
                          max={selectedMusicTrack.end}
                          min={0}
                          onChange={(event) => updateSelectedMusic({
                            start: Math.max(0, event.target.valueAsNumber || 0),
                          })}
                          step={0.05}
                          type="number"
                          value={selectedMusicTrack.start}
                        />
                      </label>
                      <label>
                        <span>{lang === "zh" ? "成片结束" : "Program end"}</span>
                        <input
                          max={programDuration}
                          min={selectedMusicTrack.start}
                          onChange={(event) => updateSelectedMusic({
                            end: Math.max(0, event.target.valueAsNumber || 0),
                          })}
                          step={0.05}
                          type="number"
                          value={selectedMusicTrack.end}
                        />
                      </label>
                      <label>
                        <span>{lang === "zh" ? "素材起点" : "Source offset"}</span>
                        <input
                          min={0}
                          onChange={(event) => updateSelectedMusic({
                            sourceStart: Math.max(0, event.target.valueAsNumber || 0),
                          })}
                          step={0.05}
                          type="number"
                          value={selectedMusicTrack.sourceStart}
                        />
                      </label>
                      <label>
                        <span>{lang === "zh" ? "淡入" : "Fade in"}</span>
                        <input
                          min={0}
                          onChange={(event) => updateSelectedMusic({
                            fadeIn: Math.max(0, event.target.valueAsNumber || 0),
                          })}
                          step={0.05}
                          type="number"
                          value={selectedMusicTrack.fadeIn}
                        />
                      </label>
                      <label>
                        <span>{lang === "zh" ? "淡出" : "Fade out"}</span>
                        <input
                          min={0}
                          onChange={(event) => updateSelectedMusic({
                            fadeOut: Math.max(0, event.target.valueAsNumber || 0),
                          })}
                          step={0.05}
                          type="number"
                          value={selectedMusicTrack.fadeOut}
                        />
                      </label>
                    </div>
                    <label className="timeline-audio-mute">
                      <input
                        checked={selectedMusicTrack.ducking}
                        onChange={(event) => updateSelectedMusic({
                          ducking: event.target.checked,
                        })}
                        type="checkbox"
                      />
                      <span>
                        <strong>{lang === "zh" ? "对白时自动压低音乐" : "Duck under dialogue"}</strong>
                        <small>
                          {lang === "zh"
                            ? "导出时根据对白信号自动降低背景音乐。"
                            : "Automatically lower music while dialogue is present on export."}
                        </small>
                      </span>
                    </label>
                  </>
                ) : (
                  <small className="timeline-music-empty">
                    {audioDraft.music.length === 0
                      ? lang === "zh" ? "还没有音乐片段。" : "No music clips yet."
                      : lang === "zh" ? "在上方选择一段音乐进行编辑。" : "Select a music clip above to edit it."}
                  </small>
                )}
              </fieldset>
              <label>
                <span>{lang === "zh" ? "音量" : "Volume"} · {Math.round(audioDraft.volume * 100)}%</span>
                <input
                  max={200}
                  min={0}
                  onChange={(event) => setAudioDraft((current) => ({
                    ...current,
                    volume: event.target.valueAsNumber / 100,
                  }))}
                  step={1}
                  type="range"
                  value={Math.round(audioDraft.volume * 100)}
                />
              </label>
              <div className="timeline-audio-fades">
                <label>
                  <span>{lang === "zh" ? "淡入（秒）" : "Fade in (s)"}</span>
                  <input
                    max={Math.max(0, programDuration - audioDraft.fadeOut)}
                    min={0}
                    onChange={(event) => setAudioDraft((current) => ({
                      ...current,
                      fadeIn: Math.max(0, event.target.valueAsNumber || 0),
                    }))}
                    step={0.05}
                    type="number"
                    value={audioDraft.fadeIn}
                  />
                </label>
                <label>
                  <span>{lang === "zh" ? "淡出（秒）" : "Fade out (s)"}</span>
                  <input
                    max={Math.max(0, programDuration - audioDraft.fadeIn)}
                    min={0}
                    onChange={(event) => setAudioDraft((current) => ({
                      ...current,
                      fadeOut: Math.max(0, event.target.valueAsNumber || 0),
                    }))}
                    step={0.05}
                    type="number"
                    value={audioDraft.fadeOut}
                  />
                </label>
              </div>
              <footer>
                {audioDirty && (
                  <small className="timeline-unsaved-draft">
                    {lang === "zh" ? "音频修改尚未保存，重启后仍会恢复。" : "Audio changes are unsaved and will survive a restart."}
                  </small>
                )}
                {audioDirty && (
                  <button
                    className="button-quiet"
                    onClick={() => {
                      setAudioDraft(audioMix);
                      setAudioDraftSource(audioMix);
                    }}
                    type="button"
                  >
                    {lang === "zh" ? "放弃修改" : "Discard"}
                  </button>
                )}
                {audioDraft.volume > 1 && (
                  <small>
                    {lang === "zh"
                      ? "超过 100% 的增益会在最终视频导出中生效；系统播放器预览上限为 100%。"
                      : "Gain above 100% is applied to the final video export; system playback previews up to 100%."}
                  </small>
                )}
                {!musicValid && (
                  <small role="alert">
                    {lang === "zh"
                      ? "音乐结束时间必须晚于开始时间，并位于成片时长内。"
                      : "Music must end after it starts and remain inside the program duration."}
                  </small>
                )}
                <button className="button-primary" disabled={busy || !musicValid} type="submit">
                  {lang === "zh" ? "保存音频设置" : "Save audio"}
                </button>
              </footer>
            </form>
          )}
          {titlePanelOpen && titleDraft && (
            <form
              className="timeline-title-popover"
              onSubmit={(event) => {
                event.preventDefault();
                void submitTitle().catch(() => undefined);
              }}
            >
              <header>
                <strong>
                  {selectedTitleId
                    ? lang === "zh" ? "编辑标题" : "Edit title"
                    : lang === "zh" ? "新建标题" : "New title"}
                </strong>
                <button
                  aria-label={lang === "zh" ? "关闭标题编辑" : "Close title editor"}
                  onClick={() => setTitlePanelOpen(false)}
                  type="button"
                >
                  ×
                </button>
              </header>
              {titleConflict && selectedTitleInput && (
                <div className="timeline-draft-conflict" role="alert">
                  <span>
                    {lang === "zh"
                      ? "这个标题的已保存版本发生了变化。请选择要继续编辑的版本。"
                      : "The saved version of this title changed. Choose which version to continue editing."}
                  </span>
                  <button
                    className="button-quiet"
                    onClick={() => {
                      setTitleDraft(selectedTitleInput);
                      setTitleDraftSource(selectedTitleInput);
                    }}
                    type="button"
                  >
                    {lang === "zh" ? "使用已保存版本" : "Use saved"}
                  </button>
                  <button
                    className="button-quiet"
                    onClick={() => setTitleDraftSource(selectedTitleInput)}
                    type="button"
                  >
                    {lang === "zh" ? "保留我的草稿" : "Keep draft"}
                  </button>
                </div>
              )}
              <label>
                <span>{lang === "zh" ? "文字" : "Text"}</span>
                <input
                  autoFocus
                  maxLength={240}
                  onChange={(event) => setTitleDraft((current) => current
                    ? { ...current, text: event.target.value }
                    : current)}
                  placeholder={lang === "zh" ? "输入标题" : "Enter title"}
                  value={titleDraft.text}
                />
              </label>
              <div className="timeline-title-style">
                <label>
                  <span>{lang === "zh" ? "字号" : "Size"}</span>
                  <input
                    max={240}
                    min={12}
                    onChange={(event) => setTitleDraft((current) => current
                      ? { ...current, fontSize: event.target.valueAsNumber }
                      : current)}
                    type="number"
                    value={titleDraft.fontSize}
                  />
                </label>
                <label>
                  <span>{lang === "zh" ? "文字" : "Text"}</span>
                  <input
                    onChange={(event) => setTitleDraft((current) => current
                      ? { ...current, color: event.target.value.toUpperCase() }
                      : current)}
                    type="color"
                    value={titleDraft.color.slice(0, 7)}
                  />
                </label>
                <label>
                  <span>{lang === "zh" ? "背景" : "Background"}</span>
                  <input
                    onChange={(event) => setTitleDraft((current) => current
                      ? { ...current, background: `${event.target.value.toUpperCase()}99` }
                      : current)}
                    type="color"
                    value={titleDraft.background.slice(0, 7)}
                  />
                </label>
              </div>
              <div className="timeline-title-time">
                <label>
                  <span>{lang === "zh" ? "开始（秒）" : "Start (s)"}</span>
                  <input
                    max={Math.max(0, titleDraft.end - 0.1)}
                    min={0}
                    onChange={(event) => {
                      const value = event.target.valueAsNumber;
                      if (!Number.isFinite(value)) return;
                      setTitleDraft((current) => current
                        ? {
                          ...current,
                          start: Math.min(current.end - 0.1, Math.max(0, value)),
                        }
                        : current);
                    }}
                    step={0.05}
                    type="number"
                    value={Number(titleDraft.start.toFixed(3))}
                  />
                </label>
                <label>
                  <span>{lang === "zh" ? "结束（秒）" : "End (s)"}</span>
                  <input
                    max={duration}
                    min={Math.min(duration, titleDraft.start + 0.1)}
                    onChange={(event) => {
                      const value = event.target.valueAsNumber;
                      if (!Number.isFinite(value)) return;
                      setTitleDraft((current) => current
                        ? {
                          ...current,
                          end: Math.max(current.start + 0.1, Math.min(duration, value)),
                        }
                        : current);
                    }}
                    step={0.1}
                    type="number"
                    value={Number(titleDraft.end.toFixed(3))}
                  />
                </label>
                <label>
                  <span>{lang === "zh" ? "淡入（秒）" : "Fade in (s)"}</span>
                  <input
                    max={Math.max(0, titleDraft.end - titleDraft.start - titleDraft.fadeOut)}
                    min={0}
                    onChange={(event) => {
                      const value = event.target.valueAsNumber;
                      if (!Number.isFinite(value)) return;
                      setTitleDraft((current) => current
                        ? {
                          ...current,
                          fadeIn: Math.min(
                            current.end - current.start - current.fadeOut,
                            Math.max(0, value),
                          ),
                        }
                        : current);
                    }}
                    step={0.05}
                    type="number"
                    value={Number(titleDraft.fadeIn.toFixed(3))}
                  />
                </label>
                <label>
                  <span>{lang === "zh" ? "淡出（秒）" : "Fade out (s)"}</span>
                  <input
                    max={Math.max(0, titleDraft.end - titleDraft.start - titleDraft.fadeIn)}
                    min={0}
                    onChange={(event) => {
                      const value = event.target.valueAsNumber;
                      if (!Number.isFinite(value)) return;
                      setTitleDraft((current) => current
                        ? {
                          ...current,
                          fadeOut: Math.min(
                            current.end - current.start - current.fadeIn,
                            Math.max(0, value),
                          ),
                        }
                        : current);
                    }}
                    step={0.05}
                    type="number"
                    value={Number(titleDraft.fadeOut.toFixed(3))}
                  />
                </label>
              </div>
              <footer>
                {titleDirty && (
                  <small className="timeline-unsaved-draft">
                    {lang === "zh" ? "标题修改尚未保存，重启后仍会恢复。" : "Title changes are unsaved and will survive a restart."}
                  </small>
                )}
                {titleDirty && (
                  <button
                    className="button-quiet"
                    onClick={() => {
                      if (selectedTitleInput) {
                        setTitleDraft(selectedTitleInput);
                        setTitleDraftSource(selectedTitleInput);
                      } else {
                        setTitleDraft(null);
                        setTitleDraftSource(null);
                        setTitlePanelOpen(false);
                      }
                    }}
                    type="button"
                  >
                    {lang === "zh" ? "放弃修改" : "Discard"}
                  </button>
                )}
                {selectedTitleId && (
                  <button
                    className="button-danger"
                    disabled={busy}
                    onClick={() => {
                      void onRemoveTitle(selectedTitleId)
                        .then(() => {
                          setSelectedTitleId(null);
                          setTitleDraft(null);
                          setTitleDraftSource(null);
                          setTitlePanelOpen(false);
                        })
                        .catch(() => undefined);
                    }}
                    type="button"
                  >
                    {lang === "zh" ? "删除" : "Delete"}
                  </button>
                )}
                <button
                  className="button-primary"
                  disabled={busy || !titleDraft.text.trim()}
                  type="submit"
                >
                  {selectedTitleId
                    ? lang === "zh" ? "保存" : "Save"
                    : lang === "zh" ? "添加" : "Add"}
                </button>
              </footer>
            </form>
          )}
        </div>
        <div className="timeline-transport">
          <button
            aria-label={isPlaying ? (lang === "zh" ? "暂停" : "Pause") : (lang === "zh" ? "播放" : "Play")}
            className={`timeline-play-button${isPlaying ? " playing" : ""}`}
            onClick={onTogglePlayback}
          >
            {isPlaying ? <span aria-hidden="true">Ⅱ</span> : <PlayIcon />}
          </button>
          <strong>{clock(currentTime)}</strong>
          <span>/ {clock(duration)}</span>
          {cutIntervals.length > 0 && (
            <span className="timeline-program-duration">
              {lang === "zh" ? "成片" : "Program"} {clock(programDuration)}
            </span>
          )}
        </div>
        <div className="timeline-zoom">
          <button
            aria-expanded={shortcutsOpen}
            aria-label={lang === "zh" ? "快捷键" : "Keyboard shortcuts"}
            className="timeline-shortcuts-button"
            onClick={() => setShortcutsOpen((value) => !value)}
            ref={shortcutsButtonRef}
            title={lang === "zh" ? "查看编辑快捷键（?）" : "Show editing shortcuts (?)"}
            type="button"
          >
            ?
          </button>
          {shortcutsOpen && (
            <aside
              aria-labelledby="timeline-shortcuts-title"
              className="timeline-shortcuts-popover"
              ref={shortcutsPanelRef}
              tabIndex={-1}
            >
              <header>
                <strong id="timeline-shortcuts-title">
                  {lang === "zh" ? "编辑快捷键" : "Editing shortcuts"}
                </strong>
                <button
                  aria-label={lang === "zh" ? "关闭快捷键" : "Close shortcuts"}
                  onClick={() => {
                    setShortcutsOpen(false);
                    shortcutsButtonRef.current?.focus();
                  }}
                  type="button"
                >
                  ×
                </button>
              </header>
              <dl>
                <div><dt>Space</dt><dd>{lang === "zh" ? "播放 / 暂停" : "Play / pause"}</dd></div>
                <div><dt>← / →</dt><dd>{lang === "zh" ? "前后移动 1 秒；按住 ⇧ 移动 0.1 秒" : "Move 1s; hold ⇧ for 0.1s"}</dd></div>
                <div><dt>S</dt><dd>{lang === "zh" ? "在播放头附近的词间拆分" : "Split near the playhead"}</dd></div>
                <div><dt>Delete</dt><dd>{lang === "zh" ? "移除所选字幕区间或标题" : "Remove selected cue range or title"}</dd></div>
                <div><dt>⌘ Z / ⇧⌘ Z</dt><dd>{lang === "zh" ? "撤销 / 重做" : "Undo / redo"}</dd></div>
                <div><dt>⌘ + / ⌘ −</dt><dd>{lang === "zh" ? "缩放时间线" : "Zoom timeline"}</dd></div>
                <div><dt>⌘ ↵</dt><dd>{lang === "zh" ? "保存当前转写或翻译" : "Save the current transcript or translation"}</dd></div>
              </dl>
              <small>
                {lang === "zh"
                  ? "输入文字时不会触发时间线快捷键。随时按 ? 再次打开。"
                  : "Timeline shortcuts stay off while typing. Press ? to reopen this list."}
              </small>
            </aside>
          )}
          <button
            aria-label={lang === "zh" ? "缩小时间线" : "Zoom timeline out"}
            disabled={zoom <= 1}
            onClick={() => setZoom((value) => Math.max(1, value - 0.5))}
          >−</button>
          <input
            aria-label={lang === "zh" ? "时间线缩放" : "Timeline zoom"}
            max={4}
            min={1}
            step={0.5}
            type="range"
            value={zoom}
            onChange={(event) => setZoom(Number(event.target.value))}
          />
          <button
            aria-label={lang === "zh" ? "放大时间线" : "Zoom timeline in"}
            disabled={zoom >= 4}
            onClick={() => setZoom((value) => Math.min(4, value + 0.5))}
          >+</button>
          <button className="timeline-fit-button" onClick={() => setZoom(1)}>
            {lang === "zh" ? "适合" : "Fit"}
          </button>
          <button
            aria-label={collapsed
              ? lang === "zh" ? "展开时间线" : "Expand timeline"
              : lang === "zh" ? "收起时间线" : "Collapse timeline"}
            className="timeline-collapse-button"
            onClick={onToggleCollapsed}
            title={collapsed
              ? autoCollapsed
                ? lang === "zh"
                  ? "窗口空间较小时自动收起；点击保持展开"
                  : "Collapsed automatically for this window; click to keep it expanded"
                : lang === "zh" ? "展开时间线" : "Expand timeline"
              : lang === "zh" ? "收起时间线，给视频和字幕更多空间" : "Collapse timeline for more editing space"}
          >
            {collapsed ? "⌃" : "⌄"}
          </button>
        </div>
      </header>

      <div
        className="timeline-dock-body"
        style={{
          "--music-track-height": `${musicTrackHeight}px`,
        } as CSSProperties}
      >
        <div className="timeline-track-labels" aria-hidden="true">
          <span>{lang === "zh" ? "标题" : "Titles"}</span>
          <span>{lang === "zh" ? "媒体 / 音乐" : "Media / music"}</span>
          <span>{lang === "zh" ? "说话人" : "Voices"}</span>
          <span>{lang === "zh" ? "字幕" : "Captions"}</span>
          <span>B-roll</span>
        </div>
        <div className="timeline-scroll" ref={scrollRef}>
          <div
            className="timeline-canvas"
            onClick={seekFromPointer}
            style={{ width: `${zoom * 100}%` }}
          >
            <TimelineTrackLayers
              broll={displayedBroll}
              chapters={chapters}
              cuts={cuts}
              doc={doc}
              duration={duration}
              lang={lang}
              music={displayedMusic}
              rows={displayedRows}
              titles={displayedTitles}
              selectedBrollId={selectedBrollId}
              selectedCueId={selectedCueId}
              selectedCueIds={selectedCueIds}
              selectedMusicId={selectedMusicId}
              selectedTitleId={selectedTitleId}
              contactSheet={contactSheet}
              waveform={waveform}
              onSelectCue={selectCue}
              onCuePointerDown={beginCueDrag}
              onCuePointerMove={moveCueDrag}
              onCuePointerUp={finishCueDrag}
              onCuePointerCancel={cancelCueDrag}
              onBrollPointerDown={beginBrollDrag}
              onBrollPointerMove={moveBrollDrag}
              onBrollPointerUp={finishBrollDrag}
              onMusicPointerCancel={cancelMusicDrag}
              onMusicPointerDown={beginMusicDrag}
              onMusicPointerMove={moveMusicDrag}
              onMusicPointerUp={finishMusicDrag}
              onSelectMusic={selectMusic}
              onSelectTitle={selectTitle}
              onTitlePointerDown={beginTitleDrag}
              onTitlePointerMove={moveTitleDrag}
              onTitlePointerUp={finishTitleDrag}
              onSeek={onSeek}
            />
            {displayedSnapGuide !== undefined && (
              <span
                aria-hidden="true"
                className="timeline-snap-guide"
                style={{
                  left: `${Math.min(
                    100,
                    Math.max(0, (displayedSnapGuide / duration) * 100),
                  )}%`,
                }}
              />
            )}
            <span
              aria-hidden="true"
              className="dock-playhead"
              style={{ left: `${Math.min(100, Math.max(0, (currentTime / duration) * 100))}%` }}
            />
          </div>
        </div>
      </div>
    </section>
  );
}
