import { convertFileSrc } from "@tauri-apps/api/core";
import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type MutableRefObject,
} from "react";
import {
  allowBrollAsset,
  allowMusicAsset,
  allowProjectMedia,
  projectMediaStatus,
} from "../../api";
import type { Lang } from "../../i18n";
import type {
  BrollOverview,
  BrollPlacement,
  BrollPlacementInput,
  Doc,
  SubtitleRow,
  SubtitleStyle,
  TitleClip,
  TitleClipInput,
  AudioMix,
  VideoExportSettings,
} from "../../types";
import { audioGainAt, musicGainAt } from "./audioMix";
import { titleOpacityAt } from "./titleAnimation";

interface Props {
  audioMix: AudioMix;
  currentTime: number;
  programDuration: number;
  programTime: number;
  broll: BrollOverview;
  doc: Doc;
  exportSettings: VideoExportSettings;
  expanded: boolean;
  lang: Lang;
  playerRef: MutableRefObject<HTMLMediaElement | null>;
  rows: SubtitleRow[];
  subtitleStyle: SubtitleStyle | null;
  titles: TitleClip[];
  onPlayingChange: (playing: boolean) => void;
  onRelinkMedia: () => Promise<void>;
  onRelinkMediaPath: (path: string) => Promise<void>;
  onTimeChange: (seconds: number) => void;
  onToggleExpanded: () => void;
  onUpdateBroll: (id: string, input: BrollPlacementInput) => Promise<void>;
  onUpdateTitle: (id: string, input: TitleClipInput) => Promise<void>;
}

interface StageDrag {
  mode: "move" | "resize";
  originClientX: number;
  originClientY: number;
  origin: { x: number; y: number; width: number; height: number };
}

interface TitleStageDrag {
  originClientX: number;
  originClientY: number;
  originX: number;
  originY: number;
}

const STAGE_WIDTH = 1920;
const STAGE_HEIGHT = 1080;

function placementInput(placement: BrollPlacement): BrollPlacementInput {
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
  audioMix,
  broll,
  currentTime,
  programDuration,
  programTime,
  doc,
  exportSettings,
  expanded,
  lang,
  playerRef,
  rows,
  subtitleStyle,
  titles,
  onPlayingChange,
  onRelinkMedia,
  onRelinkMediaPath,
  onTimeChange,
  onToggleExpanded,
  onUpdateBroll,
  onUpdateTitle,
}: Props) {
  const [mediaSource, setMediaSource] = useState<string | null>(null);
  const [mediaError, setMediaError] = useState<string | null>(null);
  const [relinking, setRelinking] = useState(false);
  const [suggestedMediaPath, setSuggestedMediaPath] = useState<string | null>(null);
  const [brollSource, setBrollSource] = useState<string | null>(null);
  const [musicSources, setMusicSources] = useState<Record<string, string>>({});
  const [draftRect, setDraftRect] = useState<BrollPlacement["rect"]>(null);
  const [stageDrag, setStageDrag] = useState<StageDrag | null>(null);
  const [titlePosition, setTitlePosition] = useState<{ x: number; y: number } | null>(null);
  const [titleStageDrag, setTitleStageDrag] = useState<TitleStageDrag | null>(null);
  const [sourceDimensions, setSourceDimensions] = useState({ width: 1920, height: 1080 });
  const [stageSize, setStageSize] = useState<{ width: number; height: number } | null>(null);
  const musicAssetKey = JSON.stringify(
    audioMix.music.map((track) => [track.id, track.path]),
  );
  const frameRef = useRef<HTMLDivElement | null>(null);
  const brollVideoRef = useRef<HTMLVideoElement | null>(null);
  const musicRefs = useRef(new Map<string, HTMLAudioElement>());
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
  const activeBroll = useMemo(
    () => broll.accepted.find(
      (placement) => placement.start <= currentTime && currentTime < placement.end,
    ),
    [broll.accepted, currentTime],
  );
  const activeBrollId = activeBroll?.id;
  const activeTitle = useMemo(
    () => titles.find((title) => title.start <= currentTime && currentTime < title.end),
    [currentTime, titles],
  );
  const canvasDimensions = useMemo(
    () => resolveCanvasDimensions(exportSettings, sourceDimensions),
    [exportSettings, sourceDimensions],
  );
  const stageAspect = canvasDimensions.width / canvasDimensions.height;

  useEffect(() => {
    let cancelled = false;
    setMediaSource(null);
    setMediaError(null);
    setSuggestedMediaPath(null);
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
          void projectMediaStatus(doc.id)
            .then((status) => {
              if (!cancelled) setSuggestedMediaPath(status.suggestedPath);
            })
            .catch(() => undefined);
        }
      });
    return () => {
      cancelled = true;
      playerRef.current?.pause();
    };
  }, [doc.id, doc.media.path, lang, playerRef]);

  useEffect(() => {
    let cancelled = false;
    setBrollSource(null);
    setDraftRect(activeBroll?.rect ?? null);
    setStageDrag(null);
    if (!activeBrollId) return;
    void allowBrollAsset(doc.id, activeBrollId)
      .then((path) => {
        if (!cancelled) setBrollSource(convertFileSrc(path));
      })
      .catch(() => {
        if (!cancelled) setBrollSource(null);
      });
    return () => {
      cancelled = true;
    };
  }, [activeBroll?.rect, activeBrollId, doc.id]);

  useEffect(() => {
    let cancelled = false;
    setMusicSources({});
    const assets = JSON.parse(musicAssetKey) as [string, string][];
    void Promise.all(assets.map(async ([id]) => {
      try {
        const path = await allowMusicAsset(doc.id, id);
        return [id, convertFileSrc(path)] as const;
      } catch {
        return null;
      }
    })).then((entries) => {
      if (cancelled) return;
      setMusicSources(Object.fromEntries(
        entries.filter((entry): entry is readonly [string, string] => entry !== null),
      ));
    });
    return () => {
      cancelled = true;
      for (const music of musicRefs.current.values()) music.pause();
    };
  }, [doc.id, musicAssetKey]);

  useEffect(() => {
    if (!activeBroll || !brollVideoRef.current) return;
    const expected = activeBroll.sourceStart + Math.max(0, currentTime - activeBroll.start);
    if (Math.abs(brollVideoRef.current.currentTime - expected) > 0.2) {
      brollVideoRef.current.currentTime = expected;
    }
  }, [activeBroll, currentTime]);

  useEffect(() => {
    setTitlePosition(activeTitle ? { x: activeTitle.x, y: activeTitle.y } : null);
    setTitleStageDrag(null);
  }, [activeTitle?.id, activeTitle?.x, activeTitle?.y]);

  const bindPlayer = (element: HTMLMediaElement | null) => {
    playerRef.current = element;
  };

  useEffect(() => {
    const player = playerRef.current;
    if (!player) return;
    player.muted = audioMix.muted;
    player.volume = audioGainAt(audioMix, programTime, programDuration);
  }, [audioMix, mediaSource, playerRef, programDuration, programTime]);

  useEffect(() => {
    const player = playerRef.current;
    if (!player) return;
    const sourceGain = audioGainAt(audioMix, programTime, programDuration);
    for (const track of audioMix.music) {
      const music = musicRefs.current.get(track.id);
      if (!music || !musicSources[track.id]) continue;
      const gain = musicGainAt(track, programTime, sourceGain > 0 && activeCue !== undefined);
      music.volume = gain;
      if (gain <= 0) {
        music.pause();
        continue;
      }
      const rawTime = track.sourceStart + Math.max(0, programTime - track.start);
      const expected = Number.isFinite(music.duration) && music.duration > 0
        ? rawTime % music.duration
        : rawTime;
      if (Math.abs(music.currentTime - expected) > 0.25) {
        music.currentTime = expected;
      }
      if (player.paused) {
        music.pause();
      } else {
        void music.play().catch(() => undefined);
      }
    }
  }, [activeCue, audioMix, musicSources, playerRef, programDuration, programTime]);

  useLayoutEffect(() => {
    if (isAudio) return;
    const frame = frameRef.current;
    if (!frame) return;
    const update = () => {
      const bounds = frame.getBoundingClientRect();
      if (bounds.width <= 0 || bounds.height <= 0) return;
      const containerAspect = bounds.width / bounds.height;
      if (containerAspect > stageAspect) {
        setStageSize({ width: bounds.height * stageAspect, height: bounds.height });
      } else {
        setStageSize({ width: bounds.width, height: bounds.width / stageAspect });
      }
    };
    update();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", update);
      return () => window.removeEventListener("resize", update);
    }
    const observer = new ResizeObserver(update);
    observer.observe(frame);
    return () => observer.disconnect();
  }, [isAudio, stageAspect]);

  return (
    <section className="workbench-preview" aria-label={lang === "zh" ? "媒体预览" : "Media preview"}>
      <header className="workbench-preview-header">
        <div>
          <span className="preview-status-dot" aria-hidden="true" />
          <strong>{lang === "zh" ? "节目监看" : "Program monitor"}</strong>
        </div>
        <div className="preview-header-actions">
          <span>{clock(programTime)} / {clock(programDuration)}</span>
          <button
            aria-label={expanded
              ? lang === "zh" ? "退出放大监看" : "Exit expanded monitor"
              : lang === "zh" ? "放大节目监看" : "Expand program monitor"}
            onClick={onToggleExpanded}
            title={expanded
              ? lang === "zh" ? "恢复编辑窗格" : "Restore editing panes"
              : lang === "zh" ? "放大视频并隐藏检查器" : "Expand video and hide the inspector"}
            type="button"
          >
            {expanded ? "↙" : "↗"}
          </button>
        </div>
      </header>
      <div
        className={`workbench-preview-frame${isAudio ? " audio" : ""}`}
        ref={frameRef}
      >
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
            <div
              className="program-stage"
              style={stageSize ?? undefined}
            >
              <video
                controls
                playsInline
                ref={bindPlayer}
                src={mediaSource}
                onEnded={() => onPlayingChange(false)}
                onLoadedMetadata={(event) => {
                  const { videoHeight, videoWidth } = event.currentTarget;
                  if (videoHeight > 0 && videoWidth > 0) {
                    setSourceDimensions({ width: videoWidth, height: videoHeight });
                  }
                }}
                onPause={() => onPlayingChange(false)}
                onPlay={() => onPlayingChange(true)}
                onTimeUpdate={(event) => onTimeChange(event.currentTarget.currentTime)}
                style={{ objectFit: exportSettings.canvasFit }}
              />
              {activeBroll && brollSource && (
                <div
                  aria-label={lang === "zh" ? "当前 B-roll 画面；拖动可调整位置" : "Current B-roll; drag to position"}
                  className={`program-broll ${activeBroll.mode}`}
                  onPointerDown={(event) => {
                    if (activeBroll.mode !== "pip" || !draftRect) return;
                    event.currentTarget.setPointerCapture(event.pointerId);
                    const handle = (event.target as HTMLElement).dataset.handle;
                    setStageDrag({
                      mode: handle === "resize" ? "resize" : "move",
                      originClientX: event.clientX,
                      originClientY: event.clientY,
                      origin: draftRect,
                    });
                  }}
                  onPointerMove={(event) => {
                    if (!stageDrag || !event.currentTarget.hasPointerCapture(event.pointerId)) return;
                    const bounds = event.currentTarget.parentElement?.getBoundingClientRect();
                    if (!bounds || bounds.width <= 0 || bounds.height <= 0) return;
                    const dx = ((event.clientX - stageDrag.originClientX) / bounds.width) * STAGE_WIDTH;
                    const dy = ((event.clientY - stageDrag.originClientY) / bounds.height) * STAGE_HEIGHT;
                    if (stageDrag.mode === "resize") {
                      setDraftRect({
                        ...stageDrag.origin,
                        width: Math.round(Math.min(
                          STAGE_WIDTH - stageDrag.origin.x,
                          Math.max(120, stageDrag.origin.width + dx),
                        )),
                        height: Math.round(Math.min(
                          STAGE_HEIGHT - stageDrag.origin.y,
                          Math.max(68, stageDrag.origin.height + dy),
                        )),
                      });
                    } else {
                      setDraftRect({
                        ...stageDrag.origin,
                        x: Math.round(Math.min(
                          STAGE_WIDTH - stageDrag.origin.width,
                          Math.max(0, stageDrag.origin.x + dx),
                        )),
                        y: Math.round(Math.min(
                          STAGE_HEIGHT - stageDrag.origin.height,
                          Math.max(0, stageDrag.origin.y + dy),
                        )),
                      });
                    }
                  }}
                  onPointerUp={(event) => {
                    if (!stageDrag || !activeBroll || !draftRect) return;
                    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
                      event.currentTarget.releasePointerCapture(event.pointerId);
                    }
                    setStageDrag(null);
                    if (activeBroll.rect
                      && activeBroll.rect.x === draftRect.x
                      && activeBroll.rect.y === draftRect.y
                      && activeBroll.rect.width === draftRect.width
                      && activeBroll.rect.height === draftRect.height) return;
                    void onUpdateBroll(activeBroll.id, {
                      ...placementInput(activeBroll),
                      rect: draftRect,
                    }).catch(() => undefined);
                  }}
                  role="group"
                  style={activeBroll.mode === "pip" && draftRect ? {
                    background: activeBroll.background === "black" ? "#000" : undefined,
                    borderRadius: `${activeBroll.radius / 19.2}cqw`,
                    height: `${(draftRect.height / STAGE_HEIGHT) * 100}%`,
                    left: `${(draftRect.x / STAGE_WIDTH) * 100}%`,
                    top: `${(draftRect.y / STAGE_HEIGHT) * 100}%`,
                    width: `${(draftRect.width / STAGE_WIDTH) * 100}%`,
                  } : undefined}
                >
                  {/\.(png|jpe?g|webp|gif)$/i.test(activeBroll.file) ? (
                    <img
                      alt={activeBroll.name || ""}
                      src={brollSource}
                      style={{ objectFit: activeBroll.fit }}
                    />
                  ) : (
                    <video
                      muted
                      playsInline
                      ref={brollVideoRef}
                      src={brollSource}
                      style={{ objectFit: activeBroll.fit }}
                    />
                  )}
                  {activeBroll.mode === "pip" && (
                    <span aria-hidden="true" className="program-broll-resize" data-handle="resize" />
                  )}
                </div>
              )}
              {activeTitle && titlePosition && (
                <div
                  aria-label={lang === "zh" ? "当前标题；拖动可调整位置" : "Current title; drag to position"}
                  className="program-title"
                  onPointerDown={(event) => {
                    event.currentTarget.setPointerCapture(event.pointerId);
                    setTitleStageDrag({
                      originClientX: event.clientX,
                      originClientY: event.clientY,
                      originX: titlePosition.x,
                      originY: titlePosition.y,
                    });
                  }}
                  onPointerMove={(event) => {
                    if (!titleStageDrag || !event.currentTarget.hasPointerCapture(event.pointerId)) return;
                    const bounds = event.currentTarget.parentElement?.getBoundingClientRect();
                    if (!bounds || bounds.width <= 0 || bounds.height <= 0) return;
                    setTitlePosition({
                      x: Math.min(1, Math.max(0, titleStageDrag.originX
                        + (event.clientX - titleStageDrag.originClientX) / bounds.width)),
                      y: Math.min(1, Math.max(0, titleStageDrag.originY
                        + (event.clientY - titleStageDrag.originClientY) / bounds.height)),
                    });
                  }}
                  onPointerUp={(event) => {
                    if (!titleStageDrag) return;
                    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
                      event.currentTarget.releasePointerCapture(event.pointerId);
                    }
                    setTitleStageDrag(null);
                    if (Math.abs(activeTitle.x - titlePosition.x) < 0.0001
                      && Math.abs(activeTitle.y - titlePosition.y) < 0.0001) return;
                    const { id: _id, ...input } = activeTitle;
                    void onUpdateTitle(activeTitle.id, {
                      ...input,
                      x: titlePosition.x,
                      y: titlePosition.y,
                    }).catch(() => undefined);
                  }}
                  role="group"
                  style={{
                    background: activeTitle.background,
                    color: activeTitle.color,
                    fontSize: `clamp(12px, ${(activeTitle.fontSize / canvasDimensions.width) * 100}cqw, ${activeTitle.fontSize}px)`,
                    left: `${titlePosition.x * 100}%`,
                    opacity: titleOpacityAt(activeTitle, currentTime),
                    top: `${titlePosition.y * 100}%`,
                  }}
                >
                  {activeTitle.text}
                </div>
              )}
              {activeCue && (
                <div
                  className="program-subtitle"
                  aria-live="off"
                  style={subtitleStyle
                    ? subtitlePosition(
                      subtitleStyle,
                      canvasDimensions.width,
                      canvasDimensions.height,
                    )
                    : undefined}
                >
                  <span
                    style={subtitleStyle ? {
                      color: assToHex(subtitleStyle.primaryColour),
                      fontFamily: subtitleStyle.fontname,
                      fontSize: `clamp(12px, ${(subtitleStyle.fontsize / canvasDimensions.width) * 100}cqw, ${subtitleStyle.fontsize}px)`,
                      fontStyle: subtitleStyle.italic ? "italic" : "normal",
                      fontWeight: subtitleStyle.bold ? 700 : 400,
                      textDecoration: `${subtitleStyle.underline ? "underline " : ""}${subtitleStyle.strikeOut ? "line-through" : ""}`.trim() || "none",
                      WebkitTextStroke: `${(Math.max(0, subtitleStyle.outline) / canvasDimensions.width) * 100}cqw ${assToHex(subtitleStyle.outlineColour)}`,
                      textShadow: subtitleStyle.shadow > 0
                        ? `${(subtitleStyle.shadow / canvasDimensions.width) * 100}cqw ${(subtitleStyle.shadow / canvasDimensions.width) * 100}cqw ${assToHex(subtitleStyle.outlineColour)}`
                        : undefined,
                    } : undefined}
                  >
                    {activeCue.text}
                  </span>
                </div>
              )}
            </div>
          )
        ) : mediaError ? (
          <div className="workbench-preview-message error media-recovery" role="alert">
            <strong>{lang === "zh" ? "项目媒体已断开" : "Project media is offline"}</strong>
            <span>{mediaError}</span>
            <small title={doc.media.path}>{doc.media.path}</small>
            <button
              disabled={relinking}
              onClick={() => {
                setRelinking(true);
                void onRelinkMedia()
                  .catch((error) => {
                    setMediaError(String(error).replace(/^Error:\s*/i, ""));
                  })
                  .finally(() => setRelinking(false));
              }}
              type="button"
            >
              {relinking
                ? (lang === "zh" ? "正在验证…" : "Validating…")
                : (lang === "zh" ? "重新定位媒体…" : "Locate media…")}
            </button>
            {suggestedMediaPath && (
              <button
                className="button-primary"
                disabled={relinking}
                onClick={() => {
                  setRelinking(true);
                  void onRelinkMediaPath(suggestedMediaPath)
                    .catch((error) => {
                      setMediaError(String(error).replace(/^Error:\s*/i, ""));
                    })
                    .finally(() => setRelinking(false));
                }}
                title={suggestedMediaPath}
                type="button"
              >
                {lang === "zh"
                  ? `连接找到的文件：${suggestedMediaPath.split(/[\\/]/).pop()}`
                  : `Use found file: ${suggestedMediaPath.split(/[\\/]/).pop()}`}
              </button>
            )}
            <em>
              {lang === "zh"
                ? "转写稿和所有编辑仍保存在项目中；重新选择原文件或等长副本即可恢复预览。"
                : "Your transcript and edits are still safe. Choose the original file or an equivalent copy to restore preview."}
            </em>
          </div>
        ) : (
          <div className="workbench-preview-message" role="status">
            <span className="spinner" aria-hidden="true" />
            {lang === "zh" ? "正在准备预览…" : "Preparing preview…"}
          </div>
        )}
      </div>
      {audioMix.music.map((track) => musicSources[track.id] && (
        <audio
          aria-hidden="true"
          hidden
          key={track.id}
          loop
          preload="auto"
          ref={(element) => {
            if (element) musicRefs.current.set(track.id, element);
            else musicRefs.current.delete(track.id);
          }}
          src={musicSources[track.id]}
        />
      ))}
      <footer className="workbench-preview-footer">
        <span>
          {lang === "zh" ? "画布" : "Canvas"}{" "}
          {exportSettings.aspectRatio === "source"
            ? (lang === "zh" ? "源比例" : "source")
            : exportSettings.aspectRatio}
          {" · "}
          {canvasDimensions.width}×{canvasDimensions.height}
          {exportSettings.aspectRatio !== "source"
            ? ` · ${exportSettings.canvasFit === "cover"
              ? (lang === "zh" ? "填满" : "fill")
              : (lang === "zh" ? "适应" : "fit")}`
            : ""}
        </span>
        <span>{activeCue ? `${activeCue.speaker || (lang === "zh" ? "未标记" : "Unlabelled")} · ${activeCue.id}` : (lang === "zh" ? "等待播放" : "Waiting for playback")}</span>
      </footer>
    </section>
  );
}

function assToHex(value: string) {
  const match = value.match(/&H[0-9A-Fa-f]{2}([0-9A-Fa-f]{2})([0-9A-Fa-f]{2})([0-9A-Fa-f]{2})/);
  return match ? `#${match[3]}${match[2]}${match[1]}` : "#ffffff";
}

function subtitlePosition(style: SubtitleStyle, canvasWidth: number, canvasHeight: number) {
  const vertical = Math.ceil(style.alignment / 3);
  const horizontal = (style.alignment - 1) % 3;
  const offset = `${(Math.max(0, style.marginV) / canvasHeight) * 100}%`;
  return {
    alignItems: vertical === 2 ? "center" : undefined,
    bottom: vertical === 1 ? offset : vertical === 2 ? "0" : "auto",
    justifyContent: horizontal === 0 ? "flex-start" : horizontal === 2 ? "flex-end" : "center",
    left: `${(Math.max(0, style.marginL) / canvasWidth) * 100}%`,
    right: `${(Math.max(0, style.marginR) / canvasWidth) * 100}%`,
    top: vertical === 3 ? offset : vertical === 2 ? "0" : "auto",
  };
}

function resolveCanvasDimensions(
  settings: VideoExportSettings,
  source: { width: number; height: number },
) {
  if (settings.resolution === "source" && settings.aspectRatio === "source") {
    return source;
  }
  const shortEdge = settings.resolution === "720p"
    ? 720
    : settings.resolution === "1080p"
      ? 1080
      : settings.resolution === "4k"
        ? 2160
        : Math.min(source.width, source.height);
  const [ratioWidth, ratioHeight] = settings.aspectRatio === "16:9"
    ? [16, 9]
    : settings.aspectRatio === "9:16"
      ? [9, 16]
      : settings.aspectRatio === "1:1"
        ? [1, 1]
        : settings.aspectRatio === "4:5"
          ? [4, 5]
          : [source.width, source.height];
  if (ratioWidth >= ratioHeight) {
    return {
      width: roundEven(shortEdge * ratioWidth / ratioHeight),
      height: roundEven(shortEdge),
    };
  }
  return {
    width: roundEven(shortEdge),
    height: roundEven(shortEdge * ratioHeight / ratioWidth),
  };
}

function roundEven(value: number) {
  const rounded = Math.max(2, Math.round(value));
  return rounded % 2 === 0 ? rounded : rounded + 1;
}
