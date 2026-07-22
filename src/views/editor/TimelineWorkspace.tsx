import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef, useState } from "react";
import type { CutSummary } from "../../api";
import { allowProjectMedia } from "../../api";
import type { Lang } from "../../i18n";
import type { Doc } from "../../types";

interface Props {
  busy: boolean;
  cuts: CutSummary[];
  doc: Doc;
  lang: Lang;
  onRestoreCut: (cutId: string) => Promise<void>;
}

function clock(seconds: number) {
  const minutes = Math.floor(seconds / 60);
  const rest = seconds - minutes * 60;
  return `${minutes}:${rest.toFixed(1).padStart(4, "0")}`;
}

export function TimelineWorkspace({ busy, cuts, doc, lang, onRestoreCut }: Props) {
  const playerRef = useRef<HTMLMediaElement | null>(null);
  const cueListRef = useRef<HTMLDivElement | null>(null);
  const cueRefs = useRef(new Map<string, HTMLElement>());
  const [mediaSource, setMediaSource] = useState<string | null>(null);
  const [mediaError, setMediaError] = useState<string | null>(null);
  const [currentTime, setCurrentTime] = useState(0);
  const [followPlayback, setFollowPlayback] = useState(true);
  const duration = Math.max(doc.media.durationSeconds, 0.001);
  const words = useMemo(() => doc.paragraphs.flatMap((paragraph) =>
    paragraph.sentences.flatMap((sentence) => sentence.words),
  ), [doc.paragraphs]);
  const wordTimes = useMemo(
    () => new Map(words.map((word) => [word.id, word])),
    [words],
  );
  const sentences = useMemo(() => doc.paragraphs.flatMap((paragraph) =>
    paragraph.sentences.map((sentence) => ({
      ...sentence,
      speaker: paragraph.speaker,
      start: sentence.words[0]?.start ?? 0,
      end: sentence.words[sentence.words.length - 1]?.end ?? 0,
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
  const isAudio = /\.(aac|aif|aiff|flac|m4a|mp3|ogg|opus|wav)$/i.test(doc.media.path);
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

  useEffect(() => {
    let cancelled = false;
    setMediaSource(null);
    setMediaError(null);
    setCurrentTime(0);
    void allowProjectMedia(doc.id)
      .then((path) => {
        if (!cancelled) setMediaSource(convertFileSrc(path));
      })
      .catch((error) => {
        if (!cancelled) {
          setMediaError(
            lang === "zh"
              ? `无法打开项目媒体：${String(error).replace(/^Error:\s*/i, "")}`
              : `Could not open project media: ${String(error).replace(/^Error:\s*/i, "")}`,
          );
        }
      });
    return () => {
      cancelled = true;
      playerRef.current?.pause();
    };
  }, [doc.id, lang]);

  useEffect(() => {
    if (!followPlayback || !activeSentence) return;
    const list = cueListRef.current;
    const cue = cueRefs.current.get(activeSentence.id);
    if (!list || !cue) return;

    const top = cue.offsetTop - (list.clientHeight - cue.offsetHeight) / 2;
    const reducedMotion = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    const nextTop = Math.max(0, top);
    if (typeof list.scrollTo === "function") {
      list.scrollTo({
        behavior: reducedMotion ? "auto" : "smooth",
        top: nextTop,
      });
    } else {
      list.scrollTop = nextTop;
    }
  }, [activeSentence?.id, followPlayback]);

  const seekTo = (seconds: number, play = true) => {
    const player = playerRef.current;
    if (!player) return;
    player.currentTime = Math.max(0, Math.min(seconds, duration));
    setCurrentTime(player.currentTime);
    if (play) void player.play().catch(() => undefined);
  };

  return (
    <div className="timeline-workspace">
      <div className="timeline-stage">
        <aside className="timeline-media-column">
          <header className="media-preview-header">
            <strong>{lang === "zh" ? "播放预览" : "Playback preview"}</strong>
            <span>{clock(currentTime)} / {clock(duration)}</span>
          </header>
          <section className={`media-preview${isAudio ? " audio-preview" : ""}`}>
            {mediaSource ? (
              isAudio ? (
                <audio
                  controls
                  ref={(element) => {
                    playerRef.current = element;
                  }}
                  src={mediaSource}
                  onTimeUpdate={(event) => setCurrentTime(event.currentTarget.currentTime)}
                />
              ) : (
                <video
                  controls
                  playsInline
                  ref={(element) => {
                    playerRef.current = element;
                  }}
                  src={mediaSource}
                  onTimeUpdate={(event) => setCurrentTime(event.currentTarget.currentTime)}
                />
              )
            ) : mediaError ? (
              <div className="media-preview-error" role="alert">{mediaError}</div>
            ) : (
              <div className="media-preview-loading" role="status">
                <span className="spinner" aria-hidden="true" />
                {lang === "zh" ? "正在打开媒体…" : "Opening media…"}
              </div>
            )}
          </section>
          <p className="media-preview-hint">
            {lang === "zh"
              ? "预览会保持在视野内；点击右侧字幕可立即跳转。"
              : "The preview stays in view; select a cue to jump to it."}
          </p>
        </aside>

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
              {sentences.map((sentence) => (
                <button
                  aria-label={`${clock(sentence.start)} · ${sentence.text}`}
                  className={`cue-region${activeSentence?.id === sentence.id ? " active" : ""}`}
                  key={sentence.id}
                  onClick={(event) => {
                    event.stopPropagation();
                    seekTo(sentence.start);
                  }}
                  style={{
                    left: `${(sentence.start / duration) * 100}%`,
                    width: `${Math.max(((sentence.end - sentence.start) / duration) * 100, 0.25)}%`,
                  }}
                  title={`${clock(sentence.start)}–${clock(sentence.end)} · ${sentence.text}`}
                />
              ))}
              {cutRegions.map((cut) => (
                <span
                  className="cut-region"
                  key={cut.id}
                  style={{
                    left: `${(cut.start / duration) * 100}%`,
                    width: `${Math.max(((cut.end - cut.start) / duration) * 100, 0.3)}%`,
                  }}
                  title={`${cut.kind} · ${cut.duration.toFixed(2)}s`}
                />
              ))}
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
            <div className="timeline-cue-scroll" id="timeline-cue-list" ref={cueListRef}>
              {sentences.map((sentence, index) => (
                <article
                  className={activeSentence?.id === sentence.id ? "active" : ""}
                  key={sentence.id}
                  ref={(element) => {
                    if (element) cueRefs.current.set(sentence.id, element);
                    else cueRefs.current.delete(sentence.id);
                  }}
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
              ))}
            </div>
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
