import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import type { CutSummary } from "../../api";
import { allowProjectMedia } from "../../api";
import type { Lang } from "../../i18n";
import type { Doc } from "../../types";

interface Props {
  cuts: CutSummary[];
  doc: Doc;
  lang: Lang;
}

function clock(seconds: number) {
  const minutes = Math.floor(seconds / 60);
  const rest = seconds - minutes * 60;
  return `${minutes}:${rest.toFixed(1).padStart(4, "0")}`;
}

export function TimelineWorkspace({ cuts, doc, lang }: Props) {
  const playerRef = useRef<HTMLMediaElement | null>(null);
  const [mediaSource, setMediaSource] = useState<string | null>(null);
  const [mediaError, setMediaError] = useState<string | null>(null);
  const [currentTime, setCurrentTime] = useState(0);
  const duration = Math.max(doc.media.duration_seconds, 0.001);
  const words = doc.paragraphs.flatMap((paragraph) =>
    paragraph.sentences.flatMap((sentence) => sentence.words),
  );
  const wordTimes = new Map(words.map((word) => [word.id, word]));
  const sentences = doc.paragraphs.flatMap((paragraph) =>
    paragraph.sentences.map((sentence) => ({
      ...sentence,
      speaker: paragraph.speaker,
      start: sentence.words[0]?.start ?? 0,
      end: sentence.words[sentence.words.length - 1]?.end ?? 0,
    })),
  );
  const cutRegions = cuts.flatMap((cut) => {
    const left = wordTimes.get(cut.a_word);
    const right = wordTimes.get(cut.b_word);
    if (!left || !right) return [];
    const end = cut.kind === "silence" ? right.start : right.end;
    const start = cut.kind === "silence"
      ? Math.max(0, end - cut.duration)
      : left.start;
    return [{ ...cut, start, end }];
  });
  const ticks = Array.from({ length: 6 }, (_, index) => (duration * index) / 5);
  const isAudio = /\.(aac|aif|aiff|flac|m4a|mp3|ogg|opus|wav)$/i.test(doc.media.path);
  const activeSentence = sentences.find(
    (sentence) => currentTime >= sentence.start && currentTime < sentence.end,
  );

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

  const seekTo = (seconds: number, play = true) => {
    const player = playerRef.current;
    if (!player) return;
    player.currentTime = Math.max(0, Math.min(seconds, duration));
    setCurrentTime(player.currentTime);
    if (play) void player.play().catch(() => undefined);
  };

  return (
    <div className="timeline-workspace">
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

      <header className="timeline-summary">
        <div>
          <p className="eyebrow">{lang === "zh" ? "时间线" : "Timeline"}</p>
          <h2>{clock(duration)}</h2>
          <span>
            {sentences.length} {lang === "zh" ? "条字幕" : "cues"} · {cuts.length}{" "}
            {lang === "zh" ? "个建议切口" : "suggested cuts"}
          </span>
        </div>
        <div className="timeline-legend">
          <span><i className="legend-cue" />{lang === "zh" ? "字幕" : "Cue"}</span>
          <span><i className="legend-cut" />{lang === "zh" ? "移除区间" : "Removed"}</span>
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

      <section className="timeline-cues">
        {sentences.map((sentence, index) => (
          <article
            className={activeSentence?.id === sentence.id ? "active" : ""}
            key={sentence.id}
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
      </section>
    </div>
  );
}
