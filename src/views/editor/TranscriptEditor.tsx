import { Fragment, useEffect, useMemo, useState } from "react";
import type { Lang } from "../../i18n";
import type { SubtitleRow } from "../../types";
import { CheckIcon } from "../../components/Icons";

interface Props {
  busy: boolean;
  lang: Lang;
  nextCueById: Record<string, string>;
  rows: SubtitleRow[];
  wordsByCue: Record<string, string[]>;
  onMerge: (id1: string, id2: string) => Promise<void>;
  onReplace: (query: string, replacement: string) => Promise<number>;
  onSave: (id: string, text: string) => Promise<void>;
  onSplit: (id: string, at: number) => Promise<void>;
  onVisibility: (id: string, hidden: boolean) => Promise<void>;
}

function timecode(seconds: number) {
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds - minutes * 60;
  return `${String(minutes).padStart(2, "0")}:${remainder.toFixed(1).padStart(4, "0")}`;
}

export function TranscriptEditor({
  busy,
  lang,
  nextCueById,
  rows,
  wordsByCue,
  onMerge,
  onReplace,
  onSave,
  onSplit,
  onVisibility,
}: Props) {
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [query, setQuery] = useState("");
  const [replacement, setReplacement] = useState("");
  const [showReplace, setShowReplace] = useState(false);
  const [savingId, setSavingId] = useState<string | null>(null);
  const [replaceResult, setReplaceResult] = useState<number | null>(null);
  const [structureId, setStructureId] = useState<string | null>(null);

  useEffect(() => {
    setDrafts(Object.fromEntries(rows.map((row) => [row.id, row.text])));
    setStructureId((current) =>
      current && rows.some((row) => row.id === current) ? current : null,
    );
  }, [rows]);

  const visibleRows = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    if (!needle) return rows;
    return rows.filter((row) =>
      (drafts[row.id] ?? row.text).toLocaleLowerCase().includes(needle),
    );
  }, [drafts, query, rows]);

  const save = async (row: SubtitleRow) => {
    const text = (drafts[row.id] ?? row.text).trim();
    if (!text || text === row.text) return;
    setSavingId(row.id);
    try {
      await onSave(row.id, text);
    } finally {
      setSavingId(null);
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
            {lang === "zh" ? "在转写稿中查找" : "Find in transcript"}
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
      </header>

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

      <div className="cue-edit-list">
        {visibleRows.map((row, index) => {
          const draft = drafts[row.id] ?? row.text;
          const dirty = draft.trim() !== row.text;
          const words = wordsByCue[row.id] ?? [];
          const nextCueId = nextCueById[row.id];
          const structureOpen = structureId === row.id;
          return (
            <article className={`cue-editor${row.hidden ? " hidden-cue" : ""}`} key={row.id}>
              <div className="cue-ordinal">{String(index + 1).padStart(2, "0")}</div>
              <div className="cue-time">
                <span>{timecode(row.start)}</span>
                <span>{timecode(row.end)}</span>
              </div>
              <div className="cue-copy">
                <div className="cue-speaker">
                  {row.speaker || (lang === "zh" ? "未标记说话人" : "Unlabelled speaker")}
                  {row.hidden && (
                    <span>{lang === "zh" ? "导出时隐藏" : "Hidden from export"}</span>
                  )}
                </div>
                <textarea
                  aria-label={`${lang === "zh" ? "字幕" : "Subtitle"} ${index + 1}`}
                  rows={Math.max(2, Math.ceil(draft.length / 36))}
                  value={draft}
                  onChange={(event) =>
                    setDrafts((previous) => ({
                      ...previous,
                      [row.id]: event.target.value,
                    }))
                  }
                  onKeyDown={(event) => {
                    if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
                      event.preventDefault();
                      void save(row);
                    }
                  }}
                />
                {structureOpen && (
                  <div className="cue-structure">
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
                  disabled={busy || row.hidden}
                  onClick={() =>
                    setStructureId((current) => (current === row.id ? null : row.id))
                  }
                >
                  {lang === "zh" ? "拆分 / 合并" : "Split / merge"}
                </button>
                <button
                  className={dirty ? "button-primary" : "button-quiet"}
                  disabled={busy || !dirty || !draft.trim()}
                  onClick={() => save(row)}
                >
                  {savingId === row.id
                    ? (lang === "zh" ? "保存中…" : "Saving…")
                    : (lang === "zh" ? "保存" : "Save")}
                </button>
              </div>
            </article>
          );
        })}
      </div>
    </div>
  );
}
