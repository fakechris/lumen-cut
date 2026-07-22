import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type { Lang } from "../../i18n";
import type {
  BrollOverview,
  BrollPlacement,
  BrollPlacementInput,
  BrollPreviewJobStatus,
  BrollSuggestion,
  Doc,
} from "../../types";

interface Props {
  busy: boolean;
  doc: Doc;
  lang: Lang;
  overview: BrollOverview;
  previewJob: BrollPreviewJobStatus | null;
  previewPaths: string[];
  onAcceptSuggestion: (suggestion: BrollSuggestion) => Promise<boolean>;
  onAdd: (input: BrollPlacementInput) => Promise<void>;
  onPickFile: () => Promise<string | null>;
  onPreview: () => Promise<void>;
  onCancelPreview: () => Promise<void>;
  onRefresh: () => Promise<void>;
  onRemove: (id: string) => Promise<void>;
  onUpdate: (id: string, input: BrollPlacementInput) => Promise<void>;
}

function fileName(path: string) {
  return path.split(/[\\/]/).pop() || path;
}

function inputFromPlacement(placement: BrollPlacement): BrollPlacementInput {
  return {
    file: placement.file,
    start: placement.start,
    end: placement.end,
    mode: placement.mode,
    fit: placement.fit,
    background: placement.background,
    sourceStart: placement.sourceStart,
    radius: placement.radius,
    name: placement.name || "",
  };
}

const EMPTY_INPUT: BrollPlacementInput = {
  file: "",
  start: 3,
  end: 7,
  mode: "pip",
  fit: "cover",
  background: "black",
  sourceStart: 0,
  radius: 12,
  name: "",
};

export function BrollWorkspace({
  busy,
  doc,
  lang,
  overview,
  previewJob,
  previewPaths,
  onAcceptSuggestion,
  onAdd,
  onPickFile,
  onPreview,
  onCancelPreview,
  onRefresh,
  onRemove,
  onUpdate,
}: Props) {
  const zh = lang === "zh";
  const duration = doc.media.durationSeconds;
  const wordTimes = new Map(
    doc.paragraphs.flatMap((paragraph) => paragraph.sentences)
      .flatMap((sentence) => sentence.words)
      .map((word) => [word.id, word] as const),
  );
  const [drafts, setDrafts] = useState<Record<string, BrollPlacementInput>>({});
  const [newPlacement, setNewPlacement] = useState<BrollPlacementInput>(EMPTY_INPUT);
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);
  const run = (action: Promise<unknown>) => {
    void action.catch(() => undefined);
  };

  useEffect(() => {
    setDrafts((current) => Object.fromEntries(
      overview.accepted.map((placement) => [
        placement.id,
        current[placement.id] || inputFromPlacement(placement),
      ]),
    ));
  }, [overview.accepted]);

  const chooseNewAsset = async () => {
    const file = await onPickFile();
    if (file) {
      setNewPlacement((current) => ({
        ...current,
        file,
        name: current.name || fileName(file).replace(/\.[^.]+$/, ""),
      }));
    }
  };

  const add = async () => {
    await onAdd(newPlacement);
    setNewPlacement(EMPTY_INPUT);
  };

  const replaceAsset = async (id: string) => {
    const file = await onPickFile();
    if (file) {
      setDrafts((current) => ({
        ...current,
        [id]: { ...current[id], file },
      }));
    }
  };

  const isRendering = previewJob !== null
    && (previewJob.state === "running" || previewJob.state === "cancelling");

  const validNew = Boolean(newPlacement.file)
    && Number.isFinite(newPlacement.start)
    && Number.isFinite(newPlacement.end)
    && newPlacement.start >= 0
    && newPlacement.end > newPlacement.start
    && (duration <= 0 || newPlacement.end <= duration);

  return (
    <div className="broll-workspace">
      <header className="broll-header">
        <div>
          <p className="eyebrow">{zh ? "补充画面" : "Supporting visuals"}</p>
          <h2>{zh ? "B-roll 素材轨道" : "B-roll track"}</h2>
          <p>
            {zh
              ? "建议只标记适合插入画面的时段。选择本地图片或视频后才会加入成片；预览会在本机完整渲染时间线，长视频需要等待。"
              : "Suggestions only mark useful moments. Choose a local image or video to add it to the edit. Preview renders the full timeline locally, so long videos take time."}
          </p>
        </div>
        <button
          className="button-primary"
          disabled={busy || isRendering || overview.accepted.length === 0}
          onClick={() => run(onPreview())}
        >
          {isRendering ? `${zh ? "正在生成" : "Rendering"} ${previewJob.progress}%` : (zh ? "生成画面预览" : "Render previews")}
        </button>
      </header>

      {isRendering && previewJob && (
        <section className="broll-preview-progress" role="status" aria-live="polite">
          <div>
            <strong>
              {previewJob.phase === "waiting"
                ? zh ? "正在等待计算资源" : "Waiting for compute capacity"
                : previewJob.phase === "preparing"
                  ? zh ? "正在准备时间线" : "Preparing timeline"
                  : previewJob.phase === "frames"
                    ? zh ? "正在提取预览帧" : "Extracting preview frames"
                    : previewJob.state === "cancelling"
                      ? zh ? "正在安全停止" : "Stopping safely"
                      : zh ? "正在硬件渲染时间线" : "Rendering timeline in hardware"}
            </strong>
            <span>{previewJob.progress}%</span>
          </div>
          <progress max={100} value={previewJob.progress} aria-label={zh ? "B-roll 预览进度" : "B-roll preview progress"} />
          <small>
            {previewJob.encoder === "h264_videotoolbox" ? "VideoToolbox · Apple Media Engine" : ""}
            {previewJob.current !== null && previewJob.total !== null
              ? ` · ${Math.round(previewJob.current)} / ${Math.round(previewJob.total)}`
              : ""}
          </small>
          <button
            className="button-quiet"
            disabled={previewJob.state === "cancelling"}
            onClick={() => run(onCancelPreview())}
          >
            {previewJob.state === "cancelling"
              ? zh ? "正在停止…" : "Stopping…"
              : zh ? "取消预览" : "Cancel preview"}
          </button>
        </section>
      )}

      {previewPaths.length > 0 && (
        <section className="broll-previews" aria-label={zh ? "B-roll 画面预览" : "B-roll previews"}>
          {previewPaths.map((path) => (
            <figure key={path}>
              <img alt={zh ? "合成后的 B-roll 预览" : "Composited B-roll preview"} src={convertFileSrc(path)} />
              <figcaption>{fileName(path)}</figcaption>
            </figure>
          ))}
        </section>
      )}

      {overview.errors.length > 0 && (
        <section className="broll-load-error" role="alert">
          <div>
            <strong>{zh ? "部分 B-roll 数据无法加载" : "Some B-roll data could not be loaded"}</strong>
            {overview.errors.map((error) => <small key={error}>{error}</small>)}
          </div>
          <button className="button-quiet" disabled={busy} onClick={() => run(onRefresh())}>
            {zh ? "重新加载" : "Reload"}
          </button>
        </section>
      )}

      <section className="broll-section" aria-labelledby="broll-suggestions-title">
        <header>
          <div>
            <h2 id="broll-suggestions-title">{zh ? "画面建议" : "Visual suggestions"}</h2>
            <p>
              {overview.suggestions.length
                ? (zh ? "选择素材后，建议的词级范围会自动转换为准确时码。" : "Choose an asset and the suggested word range becomes an exact timeline placement.")
                : (zh ? "还没有建议。可在“审查与修复”中运行 B-roll 建议。" : "No suggestions yet. Run Suggest B-roll in Review & Fix.")}
            </p>
          </div>
          <span>{overview.suggestions.length}</span>
        </header>
        <div className="broll-suggestion-grid">
          {overview.suggestions.map((suggestion, index) => {
            const start = wordTimes.get(suggestion.start)?.start;
            const end = wordTimes.get(suggestion.end)?.end;
            const accepted = start !== undefined && end !== undefined
              && overview.accepted.some((placement) => start < placement.end && placement.start < end);
            return (
              <article key={`${suggestion.start}-${suggestion.end}-${index}`}>
                <div className="broll-suggestion-mode">{suggestion.mode === "pip" ? "PIP" : (zh ? "全屏" : "FULL")}</div>
                <strong>{suggestion.query}</strong>
                <p>{suggestion.reason}</p>
                <small>{suggestion.start} → {suggestion.end}</small>
                <button
                  className="button-quiet"
                  disabled={busy || accepted}
                  onClick={() => run(onAcceptSuggestion(suggestion))}
                >
                  {accepted
                    ? (zh ? "已加入此时段" : "Already added")
                    : (zh ? "选择素材并添加" : "Choose asset & add")}
                </button>
              </article>
            );
          })}
        </div>
      </section>

      <section className="broll-section" aria-labelledby="broll-track-title">
        <header>
          <div>
            <h2 id="broll-track-title">{zh ? "已加入成片" : "Accepted placements"}</h2>
            <p>{zh ? "修改时码或显示方式后需要保存；重叠时段会被阻止。" : "Save after changing timing or display settings. Overlapping placements are blocked."}</p>
          </div>
          <span>{overview.accepted.length}</span>
        </header>
        {overview.accepted.length === 0 ? (
          <div className="broll-empty">
            <strong>{zh ? "B-roll 轨道还是空的" : "The B-roll track is empty"}</strong>
            <p>{zh ? "接受上面的建议，或在下方手动添加素材。" : "Accept a suggestion above or add an asset manually below."}</p>
          </div>
        ) : (
          <div className="broll-placement-list">
            {overview.accepted.map((placement) => {
              const draft = drafts[placement.id] || inputFromPlacement(placement);
              return (
                <article key={placement.id}>
                  <div className="broll-placement-title">
                    <div>
                      <strong>{draft.name || fileName(draft.file)}</strong>
                      <small title={draft.file}>{fileName(draft.file)}</small>
                    </div>
                    <button className="button-quiet" disabled={busy} onClick={() => run(replaceAsset(placement.id))}>
                      {zh ? "更换素材" : "Replace asset"}
                    </button>
                  </div>
                  <div className="broll-fields">
                    <label>
                      <span>{zh ? "名称" : "Name"}</span>
                      <input
                        value={draft.name || ""}
                        onChange={(event) => setDrafts((current) => ({
                          ...current,
                          [placement.id]: { ...draft, name: event.target.value },
                        }))}
                      />
                    </label>
                    <label>
                      <span>{zh ? "开始（秒）" : "Start (s)"}</span>
                      <input type="number" min={0} max={duration} step={0.1} value={draft.start} onChange={(event) => setDrafts((current) => ({ ...current, [placement.id]: { ...draft, start: event.target.valueAsNumber } }))} />
                    </label>
                    <label>
                      <span>{zh ? "结束（秒）" : "End (s)"}</span>
                      <input type="number" min={0} max={duration} step={0.1} value={draft.end} onChange={(event) => setDrafts((current) => ({ ...current, [placement.id]: { ...draft, end: event.target.valueAsNumber } }))} />
                    </label>
                    <label>
                      <span>{zh ? "显示" : "Display"}</span>
                      <select value={draft.mode} onChange={(event) => setDrafts((current) => ({ ...current, [placement.id]: { ...draft, mode: event.target.value as BrollPlacementInput["mode"] } }))}>
                        <option value="pip">{zh ? "画中画" : "Picture in picture"}</option>
                        <option value="fullscreen">{zh ? "全屏" : "Fullscreen"}</option>
                      </select>
                    </label>
                    <label>
                      <span>{zh ? "素材起点（秒）" : "Source start (s)"}</span>
                      <input type="number" min={0} step={0.1} value={draft.sourceStart} onChange={(event) => setDrafts((current) => ({ ...current, [placement.id]: { ...draft, sourceStart: event.target.valueAsNumber } }))} />
                    </label>
                    {draft.mode === "pip" && (
                      <>
                        <label>
                          <span>{zh ? "裁切" : "Fit"}</span>
                          <select value={draft.fit} onChange={(event) => setDrafts((current) => ({ ...current, [placement.id]: { ...draft, fit: event.target.value as BrollPlacementInput["fit"] } }))}>
                            <option value="cover">Cover</option>
                            <option value="contain">Contain</option>
                          </select>
                        </label>
                        <label>
                          <span>{zh ? "背景" : "Background"}</span>
                          <select value={draft.background} onChange={(event) => setDrafts((current) => ({ ...current, [placement.id]: { ...draft, background: event.target.value as BrollPlacementInput["background"] } }))}>
                            <option value="black">{zh ? "黑色" : "Black"}</option>
                            <option value="blur">{zh ? "模糊" : "Blur"}</option>
                          </select>
                        </label>
                        <label>
                          <span>{zh ? "圆角" : "Corner radius"}</span>
                          <input type="number" min={0} step={1} value={draft.radius} onChange={(event) => setDrafts((current) => ({ ...current, [placement.id]: { ...draft, radius: event.target.valueAsNumber } }))} />
                        </label>
                      </>
                    )}
                  </div>
                  <div className="broll-placement-actions">
                    {confirmRemove === placement.id ? (
                      <>
                        <span>{zh ? "确认移除这段素材？" : "Remove this placement?"}</span>
                        <button className="button-quiet" disabled={busy} onClick={() => setConfirmRemove(null)}>{zh ? "取消" : "Cancel"}</button>
                        <button className="button-danger" disabled={busy} onClick={() => run(onRemove(placement.id).then(() => setConfirmRemove(null)))}>{zh ? "确认移除" : "Remove"}</button>
                      </>
                    ) : (
                      <button className="button-quiet" disabled={busy} onClick={() => setConfirmRemove(placement.id)}>{zh ? "移除" : "Remove"}</button>
                    )}
                    <button className="button-primary" disabled={busy} onClick={() => run(onUpdate(placement.id, draft))}>{zh ? "保存调整" : "Save changes"}</button>
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </section>

      <section className="broll-section broll-manual" aria-labelledby="broll-manual-title">
        <header>
          <div>
            <h2 id="broll-manual-title">{zh ? "手动添加" : "Add manually"}</h2>
            <p>{zh ? "适合已经知道素材要放在哪个时段的情况。" : "Use this when you already know where the asset belongs."}</p>
          </div>
        </header>
        <div className="broll-manual-form">
          <button className="asset-picker" disabled={busy} onClick={() => run(chooseNewAsset())}>
            <strong>{newPlacement.file ? fileName(newPlacement.file) : (zh ? "选择图片或视频" : "Choose image or video")}</strong>
            <small>{newPlacement.file || (zh ? "支持 PNG、JPEG、WebP、GIF、MP4、MOV 等" : "PNG, JPEG, WebP, GIF, MP4, MOV, and more")}</small>
          </button>
          <label>
            <span>{zh ? "开始（秒）" : "Start (s)"}</span>
            <input type="number" min={0} max={duration} step={0.1} value={newPlacement.start} onChange={(event) => setNewPlacement((current) => ({ ...current, start: event.target.valueAsNumber }))} />
          </label>
          <label>
            <span>{zh ? "结束（秒）" : "End (s)"}</span>
            <input type="number" min={0} max={duration} step={0.1} value={newPlacement.end} onChange={(event) => setNewPlacement((current) => ({ ...current, end: event.target.valueAsNumber }))} />
          </label>
          <label>
            <span>{zh ? "显示" : "Display"}</span>
            <select value={newPlacement.mode} onChange={(event) => setNewPlacement((current) => ({ ...current, mode: event.target.value as BrollPlacementInput["mode"] }))}>
              <option value="pip">{zh ? "画中画" : "Picture in picture"}</option>
              <option value="fullscreen">{zh ? "全屏" : "Fullscreen"}</option>
            </select>
          </label>
          <label>
            <span>{zh ? "素材起点（秒）" : "Source start (s)"}</span>
            <input type="number" min={0} step={0.1} value={newPlacement.sourceStart} onChange={(event) => setNewPlacement((current) => ({ ...current, sourceStart: event.target.valueAsNumber }))} />
          </label>
          {newPlacement.mode === "pip" && (
            <label>
              <span>{zh ? "裁切" : "Fit"}</span>
              <select value={newPlacement.fit} onChange={(event) => setNewPlacement((current) => ({ ...current, fit: event.target.value as BrollPlacementInput["fit"] }))}>
                <option value="cover">Cover</option>
                <option value="contain">Contain</option>
              </select>
            </label>
          )}
          <button className="button-primary" disabled={busy || !validNew} onClick={() => run(add())}>{zh ? "加入 B-roll 轨道" : "Add to B-roll track"}</button>
        </div>
      </section>
    </div>
  );
}
