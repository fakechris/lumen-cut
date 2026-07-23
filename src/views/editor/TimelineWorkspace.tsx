import { memo, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { CutSummary } from "../../api";
import { VirtualList } from "../../components/VirtualList";
import type { Lang } from "../../i18n";
import type { Doc } from "../../types";

interface Props {
  busy: boolean;
  currentTime: number;
  cuts: CutSummary[];
  doc: Doc;
  lang: Lang;
  onRestoreCut: (cutId: string) => Promise<void>;
  onSeek: (seconds: number, autoplay?: boolean) => void;
}

function clock(seconds: number) {
  const minutes = Math.floor(seconds / 60);
  const rest = seconds - minutes * 60;
  return `${minutes}:${rest.toFixed(1).padStart(4, "0")}`;
}

interface TimelineSentence {
  end: number;
  id: string;
  speaker?: string | null;
  start: number;
  text: string;
}

interface TimelineRegion {
  end: number;
  id: string;
  start: number;
}

const timelineSentenceKey = (sentence: TimelineSentence) => sentence.id;

const TimelineOverviewRaster = memo(function TimelineOverviewRaster({
  cuts,
  duration,
  sentences,
}: {
  cuts: TimelineRegion[];
  duration: number;
  sentences: TimelineSentence[];
}) {
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
      const styles = window.getComputedStyle(canvas);
      const safeDuration = Math.max(duration, 0.001);
      context.clearRect(0, 0, width, height);
      context.fillStyle = styles.getPropertyValue("--accent").trim() || "#487fbd";
      context.globalAlpha = 0.72;
      for (const sentence of sentences) {
        const left = (sentence.start / safeDuration) * width;
        const right = (sentence.end / safeDuration) * width;
        context.fillRect(left, 18 * ratio, Math.max(1, right - left), 24 * ratio);
      }
      context.fillStyle = styles.getPropertyValue("--danger").trim() || "#b04545";
      context.globalAlpha = 1;
      for (const cut of cuts) {
        const left = (cut.start / safeDuration) * width;
        const right = (cut.end / safeDuration) * width;
        context.fillRect(left, height - 18 * ratio, Math.max(1, right - left), 8 * ratio);
      }
    };

    draw();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(draw);
    observer.observe(canvas);
    return () => observer.disconnect();
  }, [cuts, duration, sentences]);

  return <canvas aria-hidden="true" className="timeline-overview-raster" ref={canvasRef} />;
});

export function TimelineWorkspace({
  busy,
  currentTime,
  cuts,
  doc,
  lang,
  onRestoreCut,
  onSeek,
}: Props) {
  const [followPlayback, setFollowPlayback] = useState(true);
  const duration = Math.max(doc.media.durationSeconds, 0.001);
  const words = useMemo(() => doc.paragraphs.flatMap((paragraph) =>
    paragraph.sentences.flatMap((sentence) => sentence.words),
  ), [doc.paragraphs]);
  const wordTimes = useMemo(
    () => new Map(words.map((word) => [word.id, word])),
    [words],
  );
  const sentences = useMemo<TimelineSentence[]>(() => doc.paragraphs.flatMap((paragraph) =>
    paragraph.sentences.map((sentence) => ({
      id: sentence.id,
      speaker: paragraph.speaker,
      start: sentence.words[0]?.start ?? 0,
      end: sentence.words[sentence.words.length - 1]?.end ?? 0,
      text: sentence.text,
    })),
  ), [doc.paragraphs]);
  const cutRegions = useMemo(() => cuts.flatMap((cut) => {
    const left = wordTimes.get(cut.a_word);
    const right = wordTimes.get(cut.b_word);
    if (!left || !right) return [];
    const end = cut.kind === "silence" ? right.start : right.end;
    const start = cut.kind === "silence"
      ? Math.max(0, end - cut.duration)
      : left.start;
    return [{ ...cut, start, end }];
  }), [cuts, wordTimes]);
  const ticks = useMemo(
    () => Array.from({ length: 6 }, (_, index) => (duration * index) / 5),
    [duration],
  );
  const activeSentenceIndex = useMemo(() => {
    let low = 0;
    let high = sentences.length - 1;
    let candidate = -1;
    while (low <= high) {
      const middle = Math.floor((low + high) / 2);
      if (sentences[middle].start <= currentTime) {
        candidate = middle;
        low = middle + 1;
      } else {
        high = middle - 1;
      }
    }
    return candidate >= 0 && currentTime < sentences[candidate].end ? candidate : -1;
  }, [currentTime, sentences]);
  const activeSentence = activeSentenceIndex >= 0 ? sentences[activeSentenceIndex] : undefined;

  const seekTo = (seconds: number, play = true) => {
    onSeek(Math.max(0, Math.min(seconds, duration)), play);
  };

  return (
    <div className="timeline-workspace">
      <div className="timeline-stage">
        <div className="timeline-edit-column">
          <header className="timeline-summary">
            <div>
              <p className="eyebrow">{lang === "zh" ? "时间线" : "Timeline"}</p>
              <h2>{clock(duration)}</h2>
              <span>
                {sentences.length} {lang === "zh" ? "条字幕" : "cues"} · {cuts.length}{" "}
                {lang === "zh" ? "个建议切口" : "suggested cuts"}
              </span>
            </div>
            <div className="timeline-summary-actions">
              <button
                aria-controls="timeline-cue-list"
                aria-pressed={followPlayback}
                className="timeline-follow-button"
                onClick={() => setFollowPlayback((following) => !following)}
                title={lang === "zh" ? "播放时自动显示当前字幕" : "Keep the current cue in view during playback"}
              >
                <span aria-hidden="true" className="follow-indicator" />
                {lang === "zh" ? "跟随播放" : "Follow playback"}
              </button>
              <div className="timeline-legend">
                <span><i className="legend-cue" />{lang === "zh" ? "字幕" : "Cue"}</span>
                <span><i className="legend-cut" />{lang === "zh" ? "移除区间" : "Removed"}</span>
              </div>
            </div>
          </header>

          <section className="timeline-overview">
            <div className="timeline-ruler">
              {ticks.map((tick) => (
                <span key={tick} style={{ left: `${(tick / duration) * 100}%` }}>
                  {clock(tick)}
                </span>
              ))}
            </div>
            <div
              aria-label={lang === "zh" ? "媒体时间线，点击跳转" : "Media timeline, click to seek"}
              className="timeline-track"
              onClick={(event) => {
                const bounds = event.currentTarget.getBoundingClientRect();
                seekTo(((event.clientX - bounds.left) / bounds.width) * duration, false);
              }}
              role="slider"
              aria-valuemin={0}
              aria-valuemax={duration}
              aria-valuenow={currentTime}
              tabIndex={0}
              onKeyDown={(event) => {
                if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
                  event.preventDefault();
                  seekTo(currentTime + (event.key === "ArrowLeft" ? -5 : 5), false);
                }
              }}
            >
              <TimelineOverviewRaster cuts={cutRegions} duration={duration} sentences={sentences} />
              {activeSentence && (
                <span
                  aria-hidden="true"
                  className="cue-region active"
                  style={{
                    left: `${(activeSentence.start / duration) * 100}%`,
                    width: `${Math.max(((activeSentence.end - activeSentence.start) / duration) * 100, 0.25)}%`,
                  }}
                />
              )}
              <span
                aria-hidden="true"
                className="timeline-playhead"
                style={{ left: `${Math.min((currentTime / duration) * 100, 100)}%` }}
              />
            </div>
          </section>

          <section className="timeline-cues" aria-labelledby="timeline-cue-heading">
            <header className="timeline-cue-header">
              <strong id="timeline-cue-heading">{lang === "zh" ? "字幕轨道" : "Subtitle track"}</strong>
              <span>
                {activeSentenceIndex >= 0
                  ? `${activeSentenceIndex + 1} / ${sentences.length}`
                  : (lang === "zh" ? "等待播放" : "Waiting for playback")}
              </span>
            </header>
            <VirtualList
              activeKey={activeSentence?.id}
              className="timeline-cue-scroll"
              estimateHeight={62}
              followActive={followPlayback}
              id="timeline-cue-list"
              itemKey={timelineSentenceKey}
              items={sentences}
              renderItem={(sentence, index) => (
                <article
                  className={activeSentence?.id === sentence.id ? "active" : ""}
                >
                  <button onClick={() => seekTo(sentence.start)}>
                    <span className="timeline-index">{String(index + 1).padStart(2, "0")}</span>
                    <span className="timeline-copy">
                      <strong>{sentence.text}</strong>
                      <small>
                        {sentence.speaker || (lang === "zh" ? "未标记说话人" : "Unlabelled speaker")} ·{" "}
                        {clock(sentence.start)}–{clock(sentence.end)}
                      </small>
                    </span>
                    <span className="duration-mark">
                      <span style={{ width: `${Math.min(((sentence.end - sentence.start) / 8) * 100, 100)}%` }} />
                    </span>
                  </button>
                </article>
              )}
            />
          </section>
        </div>
      </div>

      <section className="timeline-cut-list">
        <header>
          <div>
            <p className="eyebrow">{lang === "zh" ? "剪辑决定" : "Edit decisions"}</p>
            <h3>
              {cutRegions.length > 0
                ? (lang === "zh" ? `${cutRegions.length} 个区间将在成片中移除` : `${cutRegions.length} regions will be removed`)
                : (lang === "zh" ? "没有移除区间" : "No removed regions")}
            </h3>
          </div>
          <small>
            {lang === "zh"
              ? "恢复后，该区间会重新出现在成片中。"
              : "Restoring a region returns it to the exported video."}
          </small>
        </header>
        {cutRegions.map((cut) => (
          <article key={cut.id}>
            <span className="timeline-cut-time">{clock(cut.start)}–{clock(cut.end)}</span>
            <span>
              <strong>{cut.kind === "silence" ? (lang === "zh" ? "静音" : "Silence") : (lang === "zh" ? "内容" : "Content")}</strong>
              <small>{cut.note || `${cut.duration.toFixed(2)}s`}</small>
            </span>
            <button
              className="button-quiet"
              disabled={busy}
              onClick={() => void onRestoreCut(cut.id)}
            >
              {lang === "zh" ? "恢复此区间" : "Restore region"}
            </button>
          </article>
        ))}
      </section>
    </div>
  );
}
