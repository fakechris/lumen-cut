import { Fragment, useEffect, useMemo, useState } from "react";
import type { Lang } from "../../i18n";
import type { SubtitleRow } from "../../types";
import { CheckIcon } from "../../components/Icons";
import { VirtualList } from "../../components/VirtualList";

export interface TranscriptDraft {
  sourceText: string;
  text: string;
}

interface Props {
  busy: boolean;
  currentTime: number;
  drafts: Record<string, TranscriptDraft>;
  duration: number;
  isPlaying: boolean;
  lang: Lang;
  mode: "subtitle" | "transcript";
  nextCueById: Record<string, string>;
  rows: SubtitleRow[];
  wordsByCue: Record<string, string[]>;
  onMerge: (id1: string, id2: string) => Promise<void>;
  onDraftsChange: (update: (
    current: Record<string, TranscriptDraft>,
  ) => Record<string, TranscriptDraft>) => void;
  onReplace: (query: string, replacement: string) => Promise<number>;
  onSave: (id: string, text: string) => Promise<void>;
  onSaveMany: (updates: Array<{ id: string; text: string }>) => Promise<void>;
  onSeek: (seconds: number, autoplay?: boolean) => void;
  onSplit: (id: string, at: number) => Promise<void>;
  onTiming: (id: string, start: number, end: number) => Promise<void>;
  onVisibility: (id: string, hidden: boolean) => Promise<void>;
}

function timecode(seconds: number) {
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds - minutes * 60;
  return `${String(minutes).padStart(2, "0")}:${remainder.toFixed(1).padStart(4, "0")}`;
}

const subtitleRowKey = (row: SubtitleRow) => row.id;

function captionMetrics(row: SubtitleRow, text: string) {
  const duration = Math.max(0, row.end - row.start);
  const characters = Array.from(text.replace(/\s/g, "")).length;
  const cps = duration > 0 ? characters / duration : Number.POSITIVE_INFINITY;
  const issue = duration < 0.5
    ? "short"
    : duration > 7
      ? "long"
      : cps > 20
        ? "fast"
        : null;
  return { cps, duration, issue };
}

export function TranscriptEditor({
  busy,
  currentTime,
  drafts,
  duration,
  isPlaying,
  lang,
  mode,
  nextCueById,
  rows,
  wordsByCue,
  onDraftsChange,
  onMerge,
  onReplace,
  onSave,
  onSaveMany,
  onSeek,
  onSplit,
  onTiming,
  onVisibility,
}: Props) {
  const [query, setQuery] = useState("");
  const [replacement, setReplacement] = useState("");
  const [showReplace, setShowReplace] = useState(false);
  const [captionFilter, setCaptionFilter] =
    useState<"all" | "visible" | "hidden" | "issues">("all");
  const [savingId, setSavingId] = useState<string | null>(null);
  const [savingAll, setSavingAll] = useState(false);
  const [replaceResult, setReplaceResult] = useState<number | null>(null);
  const [structureId, setStructureId] = useState<string | null>(null);
  const [timingDraft, setTimingDraft] = useState<{
    id: string;
    start: number;
    end: number;
  } | null>(null);

  useEffect(() => {
    const rowById = new Map(rows.map((row) => [row.id, row]));
    onDraftsChange((current) => Object.fromEntries(
      Object.entries(current).filter(([id, draft]) => {
        const row = rowById.get(id);
        return row && draft.text !== row.text;
      }),
    ));
    setStructureId((current) =>
      current && rows.some((row) => row.id === current) ? current : null,
    );
    setTimingDraft((current) => {
      if (!current) return null;
      const row = rowById.get(current.id);
      return row ? { id: row.id, start: row.start, end: row.end } : null;
    });
  }, [onDraftsChange, rows]);

  const visibleRows = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    return rows.filter((row) => {
      const text = drafts[row.id]?.text ?? row.text;
      if (needle && !text.toLocaleLowerCase().includes(needle)) return false;
      if (mode !== "subtitle" || captionFilter === "all") return true;
      if (captionFilter === "visible") return !row.hidden;
      if (captionFilter === "hidden") return row.hidden;
      return captionMetrics(row, text).issue !== null;
    });
  }, [captionFilter, drafts, mode, query, rows]);
  const captionSummary = useMemo(() => rows.reduce(
    (summary, row) => {
      const text = drafts[row.id]?.text ?? row.text;
      if (row.hidden) summary.hidden += 1;
      if (captionMetrics(row, text).issue) summary.issues += 1;
      return summary;
    },
    { hidden: 0, issues: 0 },
  ), [drafts, rows]);
  const dirtyRows = useMemo(() => rows.flatMap((row) => {
    const text = (drafts[row.id]?.text ?? row.text).trim();
    return text && text !== row.text ? [{ id: row.id, text }] : [];
  }), [drafts, rows]);
  const activeRow = useMemo(() => {
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
    const row = candidate >= 0 ? rows[candidate] : undefined;
    return row && currentTime < row.end ? row : undefined;
  }, [currentTime, rows]);

  const save = async (row: SubtitleRow) => {
    const text = (drafts[row.id]?.text ?? row.text).trim();
    if (!text || text === row.text) return;
    setSavingId(row.id);
    try {
      await onSave(row.id, text);
      onDraftsChange((current) => Object.fromEntries(
        Object.entries(current).filter(([id]) => id !== row.id),
      ));
    } finally {
      setSavingId(null);
    }
  };

  const saveAll = async () => {
    if (dirtyRows.length === 0) return;
    setSavingAll(true);
    try {
      await onSaveMany(dirtyRows);
      const saved = new Set(dirtyRows.map((update) => update.id));
      onDraftsChange((current) => Object.fromEntries(
        Object.entries(current).filter(([id]) => !saved.has(id)),
      ));
    } finally {
      setSavingAll(false);
    }
  };

  const replaceAll = async () => {
    if (!query.trim()) return;
    try {
      const count = await onReplace(query, replacement);
      setReplaceResult(count);
      setShowReplace(false);
    } catch {
      // The parent displays the actionable error message.
    }
  };

  return (
    <div className="transcript-editor">
      <header className="transcript-tools">
        <div className="find-field">
          <label htmlFor="transcript-find">
            {mode === "subtitle"
              ? lang === "zh" ? "筛选字幕文字" : "Filter captions"
              : lang === "zh" ? "在转写稿中查找" : "Find in transcript"}
          </label>
          <input
            id="transcript-find"
            placeholder={lang === "zh" ? "输入文字…" : "Type to filter…"}
            type="search"
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setReplaceResult(null);
            }}
          />
        </div>
        {mode === "subtitle" && (
          <select
            aria-label={lang === "zh" ? "字幕质量筛选" : "Caption quality filter"}
            value={captionFilter}
            onChange={(event) => setCaptionFilter(event.target.value as typeof captionFilter)}
          >
            <option value="all">{lang === "zh" ? "全部字幕" : "All captions"}</option>
            <option value="visible">{lang === "zh" ? "成片可见" : "Visible"}</option>
            <option value="hidden">{lang === "zh" ? "已隐藏" : "Hidden"}</option>
            <option value="issues">{lang === "zh" ? "需检查" : "Needs review"}</option>
          </select>
        )}
        <button
          aria-expanded={showReplace}
          className="button-quiet"
          disabled={!query.trim()}
          onClick={() => setShowReplace((value) => !value)}
        >
          {lang === "zh" ? "替换" : "Replace"}
        </button>
        <span className="cue-count">
          {visibleRows.length}/{rows.length}
        </span>
        {dirtyRows.length > 0 && (
          <>
            <span className="transcript-unsaved" role="status">
              {lang === "zh"
                ? `${dirtyRows.length} 条修改未保存`
                : `${dirtyRows.length} unsaved edit${dirtyRows.length === 1 ? "" : "s"}`}
            </span>
            <button
              className="button-primary"
              disabled={busy || savingAll}
              onClick={() => void saveAll().catch(() => undefined)}
            >
              {savingAll
                ? lang === "zh" ? "正在保存…" : "Saving…"
                : lang === "zh" ? "全部保存" : "Save all"}
            </button>
          </>
        )}
      </header>

      {mode === "subtitle" && (
        <div className="subtitle-quality-summary" role="status">
          <span>{rows.length - captionSummary.hidden} {lang === "zh" ? "条进入成片" : "visible"}</span>
          <span>{captionSummary.hidden} {lang === "zh" ? "条已隐藏" : "hidden"}</span>
          <span className={captionSummary.issues > 0 ? "warning" : ""}>
            {captionSummary.issues} {lang === "zh" ? "条需检查阅读节奏" : "timing issues"}
          </span>
        </div>
      )}

      {showReplace && (
        <div className="replace-bar">
          <label htmlFor="transcript-replacement">
            {lang === "zh" ? "替换为" : "Replace with"}
          </label>
          <input
            autoFocus
            id="transcript-replacement"
            value={replacement}
            onChange={(event) => setReplacement(event.target.value)}
          />
          <button className="button-primary" disabled={busy} onClick={replaceAll}>
            {lang === "zh" ? "全部替换" : "Replace all"}
          </button>
        </div>
      )}

      {replaceResult !== null && (
        <p className="inline-confirmation" role="status">
          <CheckIcon />
          {lang === "zh"
            ? `已替换 ${replaceResult} 处。`
            : `Replaced ${replaceResult} occurrence${replaceResult === 1 ? "" : "s"}.`}
        </p>
      )}

      <VirtualList
        activeKey={activeRow?.id}
        className="cue-edit-list"
        estimateHeight={126}
        followActive={isPlaying}
        itemKey={subtitleRowKey}
        items={visibleRows}
        renderItem={(row, index) => {
          const draft = drafts[row.id]?.text ?? row.text;
          const dirty = draft.trim() !== row.text;
          const words = wordsByCue[row.id] ?? [];
          const metrics = captionMetrics(row, draft);
          const nextCueId = nextCueById[row.id];
          const structureOpen = structureId === row.id;
          const timing = structureOpen && timingDraft?.id === row.id
            ? timingDraft
            : { id: row.id, start: row.start, end: row.end };
          const rowIndex = rows.findIndex((candidate) => candidate.id === row.id);
          const earliestStart = rowIndex > 0 ? rows[rowIndex - 1].end : 0;
          const latestEnd = rowIndex >= 0 && rowIndex < rows.length - 1
            ? rows[rowIndex + 1].start
            : duration;
          const timingChanged = Math.abs(timing.start - row.start) > 0.000_001
            || Math.abs(timing.end - row.end) > 0.000_001;
          const timingValid = timing.start >= earliestStart
            && timing.end <= latestEnd
            && timing.end - timing.start >= 0.1;
          return (
            <article
              className={`cue-editor${row.hidden ? " hidden-cue" : ""}${activeRow?.id === row.id ? " active-cue" : ""}${mode === "subtitle" && metrics.issue ? " caption-issue" : ""}`}
              key={row.id}
            >
              <button
                aria-label={`${lang === "zh" ? "播放字幕" : "Play cue"} ${index + 1}`}
                className="cue-ordinal cue-play"
                onClick={() => onSeek(row.start, true)}
              >
                {String(index + 1).padStart(2, "0")}
              </button>
              <div className="cue-time">
                <span>{timecode(row.start)}</span>
                <span>{timecode(row.end)}</span>
                {mode === "subtitle" && <small>{metrics.duration.toFixed(1)}s</small>}
              </div>
              <div className="cue-copy">
                <div className="cue-speaker">
                  {mode === "subtitle"
                    ? `${Number.isFinite(metrics.cps) ? metrics.cps.toFixed(1) : "∞"} CPS`
                    : row.speaker || (lang === "zh" ? "未标记说话人" : "Unlabelled speaker")}
                  {mode === "subtitle" && metrics.issue && (
                    <span className="caption-issue-label">
                      {metrics.issue === "fast"
                        ? lang === "zh" ? "阅读速度偏快" : "Reading speed is high"
                        : metrics.issue === "short"
                          ? lang === "zh" ? "显示时间过短" : "Duration is too short"
                          : lang === "zh" ? "显示时间过长" : "Duration is too long"}
                    </span>
                  )}
                  {row.hidden && (
                    <span>{lang === "zh" ? "导出时隐藏" : "Hidden from export"}</span>
                  )}
                </div>
                <textarea
                  aria-label={`${lang === "zh" ? "字幕" : "Subtitle"} ${index + 1}`}
                  rows={Math.max(2, Math.ceil(draft.length / 36))}
                  value={draft}
                  onChange={(event) =>
                    onDraftsChange((previous) => ({
                      ...previous,
                      [row.id]: {
                        sourceText: previous[row.id]?.sourceText ?? row.text,
                        text: event.target.value,
                      },
                    }))
                  }
                  onKeyDown={(event) => {
                    if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
                      event.preventDefault();
                      void save(row).catch(() => undefined);
                    }
                  }}
                />
                {structureOpen && (
                  <div className="cue-structure">
                    {mode === "subtitle" && (
                      <form
                        className="cue-timing-editor"
                        onSubmit={(event) => {
                          event.preventDefault();
                          if (!timingValid || !timingChanged) return;
                          void onTiming(row.id, timing.start, timing.end)
                            .then(() => {
                              setStructureId(null);
                              setTimingDraft(null);
                            })
                            .catch(() => undefined);
                        }}
                      >
                        <div className="cue-structure-heading">
                          <strong>{lang === "zh" ? "精修字幕时码" : "Fine-tune cue timing"}</strong>
                          <small>
                            {lang === "zh"
                              ? `可用窗口 ${timecode(earliestStart)}–${timecode(latestEnd)}；词级时码会按比例保留。`
                              : `Available window ${timecode(earliestStart)}–${timecode(latestEnd)}; word timing is preserved proportionally.`}
                          </small>
                        </div>
                        <div className="cue-timing-fields">
                          <label>
                            <span>{lang === "zh" ? "开始（秒）" : "Start (s)"}</span>
                            <input
                              aria-label={`${lang === "zh" ? "字幕开始" : "Cue start"} ${index + 1}`}
                              max={Math.max(earliestStart, timing.end - 0.1)}
                              min={earliestStart}
                              onChange={(event) => {
                                const value = event.target.valueAsNumber;
                                if (!Number.isFinite(value)) return;
                                setTimingDraft({ ...timing, start: value });
                              }}
                              step={0.05}
                              type="number"
                              value={Number(timing.start.toFixed(3))}
                            />
                          </label>
                          <label>
                            <span>{lang === "zh" ? "结束（秒）" : "End (s)"}</span>
                            <input
                              aria-label={`${lang === "zh" ? "字幕结束" : "Cue end"} ${index + 1}`}
                              max={latestEnd}
                              min={Math.min(latestEnd, timing.start + 0.1)}
                              onChange={(event) => {
                                const value = event.target.valueAsNumber;
                                if (!Number.isFinite(value)) return;
                                setTimingDraft({ ...timing, end: value });
                              }}
                              step={0.05}
                              type="number"
                              value={Number(timing.end.toFixed(3))}
                            />
                          </label>
                          <button
                            className="button-primary"
                            disabled={busy || !timingValid || !timingChanged}
                            type="submit"
                          >
                            {lang === "zh" ? "应用时码" : "Apply timing"}
                          </button>
                        </div>
                      </form>
                    )}
                    {words.length > 1 ? (
                      <>
                        <div className="cue-structure-heading">
                          <strong>{lang === "zh" ? "从词间拆开" : "Split between words"}</strong>
                          <small>
                            {lang === "zh"
                              ? "点击词间的竖线，时码会自动保留。"
                              : "Choose a divider; word timing is preserved."}
                          </small>
                        </div>
                        <div className="split-word-stream">
                          {words.map((word, wordIndex) => (
                            <Fragment key={`${row.id}-${wordIndex}`}>
                              <span>{word}</span>
                              {wordIndex < words.length - 1 && (
                                <button
                                  aria-label={
                                    lang === "zh"
                                      ? `在“${word}”后拆分`
                                      : `Split after “${word}”`
                                  }
                                  disabled={busy}
                                  onClick={async () => {
                                    await onSplit(row.id, wordIndex + 1);
                                    setStructureId(null);
                                  }}
                                  title={
                                    lang === "zh"
                                      ? `在“${word}”后拆分`
                                      : `Split after “${word}”`
                                  }
                                >
                                  <i aria-hidden="true" />
                                </button>
                              )}
                            </Fragment>
                          ))}
                        </div>
                      </>
                    ) : (
                      <p className="structure-unavailable">
                        {lang === "zh"
                          ? "这句没有足够的词级时码，暂时不能拆分。"
                          : "This cue does not have enough timed words to split."}
                      </p>
                    )}
                    {nextCueId && (
                      <div className="merge-next-row">
                        <span>
                          <strong>{lang === "zh" ? "合并下一句" : "Merge next cue"}</strong>
                          <small>
                            {lang === "zh"
                              ? "两句文字和词级时码会连在一起。"
                              : "Joins both texts and their word timing."}
                          </small>
                        </span>
                        <button
                          className="button-quiet"
                          disabled={busy}
                          onClick={async () => {
                            await onMerge(row.id, nextCueId);
                            setStructureId(null);
                          }}
                        >
                          {lang === "zh" ? "合并下句" : "Merge next"}
                        </button>
                      </div>
                    )}
                  </div>
                )}
              </div>
              <div className="cue-actions">
                <button
                  className="button-quiet"
                  disabled={busy}
                  onClick={() => {
                    setStructureId(null);
                    void onVisibility(row.id, !row.hidden);
                  }}
                >
                  {row.hidden
                    ? (lang === "zh" ? "恢复" : "Restore")
                    : (lang === "zh" ? "隐藏" : "Hide")}
                </button>
                <button
                  aria-expanded={structureOpen}
                  className={structureOpen ? "button-selected" : "button-quiet"}
                  disabled={busy || row.hidden || dirty}
                  onClick={() =>
                    setStructureId((current) => {
                      const next = current === row.id ? null : row.id;
                      setTimingDraft(next
                        ? { id: row.id, start: row.start, end: row.end }
                        : null);
                      return next;
                    })
                  }
                >
                  {mode === "subtitle"
                    ? lang === "zh" ? "时码 / 结构" : "Timing / structure"
                    : lang === "zh" ? "拆分 / 合并" : "Split / merge"}
                </button>
                <button
                  className={dirty ? "button-primary" : "button-quiet"}
                  disabled={busy || !dirty || !draft.trim()}
                  onClick={() => void save(row).catch(() => undefined)}
                >
                  {savingId === row.id
                    ? (lang === "zh" ? "保存中…" : "Saving…")
                    : (lang === "zh" ? "保存" : "Save")}
                </button>
              </div>
            </article>
          );
        }}
      />
    </div>
  );
}
