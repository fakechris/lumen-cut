import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useMemo, useState, type MutableRefObject } from "react";
import { allowProjectMedia } from "../../api";
import type { Lang } from "../../i18n";
import type { Doc, SubtitleRow, SubtitleStyle } from "../../types";

interface Props {
  currentTime: number;
  doc: Doc;
  lang: Lang;
  playerRef: MutableRefObject<HTMLMediaElement | null>;
  rows: SubtitleRow[];
  subtitleStyle: SubtitleStyle | null;
  onPlayingChange: (playing: boolean) => void;
  onTimeChange: (seconds: number) => void;
}

function clock(seconds: number) {
  const safe = Math.max(0, seconds);
  const hours = Math.floor(safe / 3600);
  const minutes = Math.floor((safe % 3600) / 60);
  const rest = Math.floor(safe % 60);
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${String(rest).padStart(2, "0")}`
    : `${minutes}:${String(rest).padStart(2, "0")}`;
}

export function EditorMediaPreview({
  currentTime,
  doc,
  lang,
  playerRef,
  rows,
  subtitleStyle,
  onPlayingChange,
  onTimeChange,
}: Props) {
  const [mediaSource, setMediaSource] = useState<string | null>(null);
  const [mediaError, setMediaError] = useState<string | null>(null);
  const isAudio = /\.(aac|aif|aiff|flac|m4a|mp3|ogg|opus|wav)$/i.test(doc.media.path);
  const activeCue = useMemo(() => {
    let low = 0;
    let high = rows.length - 1;
    let candidate = -1;
    while (low <= high) {
      const middle = Math.floor((low + high) / 2);
      if (rows[middle].start <= currentTime) {
        candidate = middle;
        low = middle + 1;
      } else {
        high = middle - 1;
      }
    }
    const cue = candidate >= 0 ? rows[candidate] : undefined;
    return cue && !cue.hidden && currentTime < cue.end ? cue : undefined;
  }, [currentTime, rows]);

  useEffect(() => {
    let cancelled = false;
    setMediaSource(null);
    setMediaError(null);
    void allowProjectMedia(doc.id)
      .then((path) => {
        if (!cancelled) setMediaSource(convertFileSrc(path));
      })
      .catch((error) => {
        if (!cancelled) {
          const message = String(error).replace(/^Error:\s*/i, "");
          setMediaError(
            lang === "zh" ? `无法打开项目媒体：${message}` : `Could not open project media: ${message}`,
          );
        }
      });
    return () => {
      cancelled = true;
      playerRef.current?.pause();
    };
  }, [doc.id, playerRef]);

  const bindPlayer = (element: HTMLMediaElement | null) => {
    playerRef.current = element;
  };

  return (
    <section className="workbench-preview" aria-label={lang === "zh" ? "媒体预览" : "Media preview"}>
      <header className="workbench-preview-header">
        <div>
          <span className="preview-status-dot" aria-hidden="true" />
          <strong>{lang === "zh" ? "节目监看" : "Program monitor"}</strong>
        </div>
        <span>{clock(currentTime)} / {clock(doc.media.durationSeconds)}</span>
      </header>
      <div className={`workbench-preview-frame${isAudio ? " audio" : ""}`}>
        {mediaSource ? (
          isAudio ? (
            <audio
              controls
              ref={bindPlayer}
              src={mediaSource}
              onEnded={() => onPlayingChange(false)}
              onPause={() => onPlayingChange(false)}
              onPlay={() => onPlayingChange(true)}
              onTimeUpdate={(event) => onTimeChange(event.currentTarget.currentTime)}
            />
          ) : (
            <>
              <video
                controls
                playsInline
                ref={bindPlayer}
                src={mediaSource}
                onEnded={() => onPlayingChange(false)}
                onPause={() => onPlayingChange(false)}
                onPlay={() => onPlayingChange(true)}
                onTimeUpdate={(event) => onTimeChange(event.currentTarget.currentTime)}
              />
              {activeCue && (
                <div className="program-subtitle" aria-live="off">
                  <span
                    style={subtitleStyle ? {
                      color: assToHex(subtitleStyle.primaryColour),
                      fontFamily: subtitleStyle.fontname,
                      fontStyle: subtitleStyle.italic ? "italic" : "normal",
                      fontWeight: subtitleStyle.bold ? 700 : 400,
                      textDecoration: `${subtitleStyle.underline ? "underline " : ""}${subtitleStyle.strikeOut ? "line-through" : ""}`.trim() || "none",
                      WebkitTextStroke: `${Math.max(0, subtitleStyle.outline * 0.35)}px ${assToHex(subtitleStyle.outlineColour)}`,
                      textShadow: subtitleStyle.shadow > 0
                        ? `${subtitleStyle.shadow}px ${subtitleStyle.shadow}px ${assToHex(subtitleStyle.outlineColour)}`
                        : undefined,
                    } : undefined}
                  >
                    {activeCue.text}
                  </span>
                </div>
              )}
            </>
          )
        ) : mediaError ? (
          <div className="workbench-preview-message error" role="alert">{mediaError}</div>
        ) : (
          <div className="workbench-preview-message" role="status">
            <span className="spinner" aria-hidden="true" />
            {lang === "zh" ? "正在准备预览…" : "Preparing preview…"}
          </div>
        )}
      </div>
      <footer className="workbench-preview-footer">
        <span>{lang === "zh" ? "字幕叠加" : "Caption overlay"}</span>
        <span>{activeCue ? `${activeCue.speaker || (lang === "zh" ? "未标记" : "Unlabelled")} · ${activeCue.id}` : (lang === "zh" ? "等待播放" : "Waiting for playback")}</span>
      </footer>
    </section>
  );
}

function assToHex(value: string) {
  const match = value.match(/&H[0-9A-Fa-f]{2}([0-9A-Fa-f]{2})([0-9A-Fa-f]{2})([0-9A-Fa-f]{2})/);
  return match ? `#${match[3]}${match[2]}${match[1]}` : "#ffffff";
}
