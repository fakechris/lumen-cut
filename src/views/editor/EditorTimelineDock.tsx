import { memo, useEffect, useMemo, useRef, useState } from "react";
import type { CutSummary } from "../../api";
import { PlayIcon } from "../../components/Icons";
import type { Lang } from "../../i18n";
import type { BrollOverview, Doc, SubtitleRow } from "../../types";

interface Props {
  broll: BrollOverview;
  currentTime: number;
  cuts: CutSummary[];
  doc: Doc;
  isPlaying: boolean;
  lang: Lang;
  rows: SubtitleRow[];
  onOpenBroll: () => void;
  onSeek: (seconds: number, autoplay?: boolean) => void;
  onTogglePlayback: () => void;
}

// Lumen categorical palette (matches .speaker-swatch-* order in styles.css)
const SPEAKER_COLORS = ["#9f4f24", "#2563bb", "#2f7d52", "#6c4dab", "#b04545", "#b8862e"];

function clock(seconds: number) {
  const safe = Math.max(0, seconds);
  const minutes = Math.floor(safe / 60);
  const rest = Math.floor(safe % 60);
  return `${String(minutes).padStart(2, "0")}:${String(rest).padStart(2, "0")}`;
}

interface TrackLayerProps {
  broll: BrollOverview;
  cuts: CutSummary[];
  doc: Doc;
  duration: number;
  lang: Lang;
  rows: SubtitleRow[];
  onSeek: (seconds: number, autoplay?: boolean) => void;
}

const TimelineTrackLayers = memo(function TimelineTrackLayers({
  broll,
  cuts,
  doc,
  duration,
  lang,
  rows,
  onSeek,
}: TrackLayerProps) {
  const speakerColors = useMemo(() => {
    const speakers = [...new Set(rows.map((row) => row.speaker || "unlabelled"))];
    return new Map(speakers.map((speaker, index) => [
      speaker,
      SPEAKER_COLORS[index % SPEAKER_COLORS.length],
    ]));
  }, [rows]);
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

  return (
    <>
      <div className="dock-ruler">
        {ticks.map((tick) => (
          <span key={tick} style={{ left: `${(tick / duration) * 100}%` }}>
            {clock(tick)}
          </span>
        ))}
      </div>
      <div className="dock-track media-track">
        <div className="media-clip">
          <span>{doc.media.path.split(/[\\/]/).pop()}</span>
          <div className="speech-activity" aria-hidden="true">
            {rows.map((row) => (
              <i
                key={row.id}
                style={{
                  left: `${(row.start / duration) * 100}%`,
                  width: `${Math.max(((row.end - row.start) / duration) * 100, 0.18)}%`,
                }}
              />
            ))}
          </div>
        </div>
      </div>
      <div className="dock-track speaker-track">
        {rows.map((row) => (
          <button
            aria-label={`${row.speaker || (lang === "zh" ? "未标记说话人" : "Unlabelled speaker")} ${clock(row.start)}`}
            key={row.id}
            onClick={(event) => {
              event.stopPropagation();
              onSeek(row.start, true);
            }}
            style={{
              background: speakerColors.get(row.speaker || "unlabelled"),
              left: `${(row.start / duration) * 100}%`,
              width: `${Math.max(((row.end - row.start) / duration) * 100, 0.24)}%`,
            }}
            title={`${row.speaker || (lang === "zh" ? "未标记说话人" : "Unlabelled speaker")} · ${row.text}`}
          />
        ))}
      </div>
      <div className="dock-track caption-track">
        {rows.map((row) => (
          <button
            className={row.hidden ? "hidden" : ""}
            key={row.id}
            onClick={(event) => {
              event.stopPropagation();
              onSeek(row.start, true);
            }}
            style={{
              left: `${(row.start / duration) * 100}%`,
              width: `${Math.max(((row.end - row.start) / duration) * 100, 0.24)}%`,
            }}
            title={`${clock(row.start)} · ${row.text}`}
          />
        ))}
      </div>
      <div className="dock-track broll-track">
        {broll.accepted.map((placement) => (
          <button
            key={placement.id}
            onClick={(event) => {
              event.stopPropagation();
              onSeek(placement.start, true);
            }}
            style={{
              left: `${(placement.start / duration) * 100}%`,
              width: `${Math.max(((placement.end - placement.start) / duration) * 100, 0.4)}%`,
            }}
            title={placement.name || placement.file}
          />
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

export function EditorTimelineDock({
  broll,
  currentTime,
  cuts,
  doc,
  isPlaying,
  lang,
  rows,
  onOpenBroll,
  onSeek,
  onTogglePlayback,
}: Props) {
  const [zoom, setZoom] = useState(1);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const duration = Math.max(doc.media.durationSeconds, 0.001);

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

  const seekFromPointer = (event: React.MouseEvent<HTMLDivElement>) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    onSeek(((event.clientX - bounds.left) / bounds.width) * duration, false);
  };

  return (
    <section className="workbench-timeline" aria-label={lang === "zh" ? "编辑时间线" : "Editing timeline"}>
      <header className="timeline-dock-toolbar">
        <div className="timeline-edit-actions">
          <button className="timeline-icon-button" disabled title={lang === "zh" ? "撤销即将提供" : "Undo coming soon"}>↶</button>
          <button className="timeline-icon-button" disabled title={lang === "zh" ? "重做即将提供" : "Redo coming soon"}>↷</button>
          <span className="timeline-toolbar-divider" />
          <button
            className="timeline-tool-button"
            disabled
            title={lang === "zh" ? "标题轨道将在后续版本提供" : "Title tracks are coming in a later version"}
            type="button"
          >
            + {lang === "zh" ? "标题" : "Title"}
          </button>
          <button className="timeline-tool-button" onClick={onOpenBroll} type="button">
            + B-roll
          </button>
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
        </div>
        <div className="timeline-zoom">
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
        </div>
      </header>

      <div className="timeline-dock-body">
        <div className="timeline-track-labels" aria-hidden="true">
          <span>{lang === "zh" ? "媒体" : "Media"}</span>
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
              broll={broll}
              cuts={cuts}
              doc={doc}
              duration={duration}
              lang={lang}
              rows={rows}
              onSeek={onSeek}
            />
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
