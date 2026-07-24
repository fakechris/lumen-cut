import { useEffect, useMemo, useState } from "react";
import type { Lang } from "../../i18n";
import type {
  ChapterInput,
  ChapterRow,
  SubtitleRow,
  TaskStatus,
} from "../../types";

export interface ChapterDraft {
  sourceTitle: string;
  title: string;
}

interface Props {
  busy: boolean;
  chapters: ChapterRow[];
  configured: boolean;
  currentTime: number;
  drafts: Record<string, ChapterDraft>;
  lang: Lang;
  rows: SubtitleRow[];
  status: TaskStatus["kinds"][number] | null;
  onDraftsChange: (update: (
    current: Record<string, ChapterDraft>,
  ) => Record<string, ChapterDraft>) => void;
  onGenerate: () => Promise<void>;
  onOpenSettings: () => void;
  onSave: (chapters: ChapterInput[]) => Promise<void>;
  onSeek: (seconds: number, autoplay?: boolean) => void;
}

function clock(seconds: number) {
  const total = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const rest = total % 60;
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${String(rest).padStart(2, "0")}`
    : `${String(minutes).padStart(2, "0")}:${String(rest).padStart(2, "0")}`;
}

export function ChapterWorkspace({
  busy,
  chapters,
  configured,
  currentTime,
  drafts,
  lang,
  rows,
  status,
  onDraftsChange,
  onGenerate,
  onOpenSettings,
  onSave,
  onSeek,
}: Props) {
  const zh = lang === "zh";
  const [newTitle, setNewTitle] = useState("");
  const [working, setWorking] = useState(false);
  const [confirmRegenerate, setConfirmRegenerate] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);
  const dirty = chapters.filter((chapter) => {
    const draft = drafts[chapter.startSeg];
    return draft !== undefined && draft.title.trim() !== chapter.title;
  });
  const materialized = () => chapters.map((chapter) => ({
    startSeg: chapter.startSeg,
    title: drafts[chapter.startSeg]?.title.trim() || chapter.title,
  }));
  const cueOrder = useMemo(
    () => new Map(rows.map((row, index) => [row.id, index])),
    [rows],
  );
  const activeCue = useMemo(() => {
    let low = 0;
    let high = rows.length - 1;
    let candidate = 0;
    while (low <= high) {
      const middle = Math.floor((low + high) / 2);
      if (rows[middle].start <= currentTime) {
        candidate = middle;
        low = middle + 1;
      } else {
        high = middle - 1;
      }
    }
    return rows[candidate];
  }, [currentTime, rows]);
  const generating = status?.pending ? status.pending > 0 : false;

  useEffect(() => {
    const byId = new Map(chapters.map((chapter) => [chapter.startSeg, chapter]));
    onDraftsChange((current) => Object.fromEntries(
      Object.entries(current).filter(([id, draft]) => {
        const chapter = byId.get(id);
        return chapter && draft.title.trim() !== chapter.title;
      }),
    ));
  }, [chapters, onDraftsChange]);

  const run = async (action: () => Promise<void>) => {
    setWorking(true);
    setLocalError(null);
    try {
      await action();
    } finally {
      setWorking(false);
    }
  };

  const saveTitles = () => run(async () => {
    await onSave(materialized());
    onDraftsChange(() => ({}));
  });

  const addAtPlayhead = () => run(async () => {
    const title = newTitle.trim();
    const startSeg = chapters.length === 0 ? rows[0]?.id : activeCue?.id;
    if (!title || !startSeg) return;
    if (chapters.some((chapter) => chapter.startSeg === startSeg)) {
      setLocalError(zh
        ? "播放头所在字幕已经是一个章节起点。"
        : "The cue at the playhead is already a chapter start.");
      return;
    }
    const next = [...materialized(), { title, startSeg }].sort(
      (left, right) =>
        (cueOrder.get(left.startSeg) ?? Number.MAX_SAFE_INTEGER)
        - (cueOrder.get(right.startSeg) ?? Number.MAX_SAFE_INTEGER),
    );
    await onSave(next);
    onDraftsChange(() => ({}));
    setNewTitle("");
  });

  const removeChapter = (startSeg: string) => run(async () => {
    const next = materialized().filter((chapter) => chapter.startSeg !== startSeg);
    if (next.length > 0 && next[0].startSeg !== rows[0]?.id) {
      setLocalError(zh
        ? "第一个章节必须从第一条字幕开始。要删除它，请先清空全部章节再重新生成。"
        : "The first chapter must start at the first cue. Clear all chapters before rebuilding it.");
      return;
    }
    await onSave(next);
    onDraftsChange((current) => Object.fromEntries(
      Object.entries(current).filter(([id]) => id !== startSeg),
    ));
  });

  return (
    <div className="chapter-workspace">
      <header className="chapter-header">
        <div>
          <p className="eyebrow">{zh ? "内容结构" : "Content structure"}</p>
          <h2>{zh ? "章节" : "Chapters"}</h2>
          <p>
            {zh
              ? "章节起点绑定到字幕时码，会随剪辑一起进入 Markdown 导出。可在播放头位置补充章节。"
              : "Chapter starts are anchored to cue timing and included in Markdown exports. Add one at the playhead."}
          </p>
        </div>
        <div className="chapter-generation">
          {!configured ? (
            <button className="button-primary" onClick={onOpenSettings}>
              {zh ? "先配置 AI" : "Configure AI"}
            </button>
          ) : chapters.length > 0 && !confirmRegenerate ? (
            <button
              className="button-quiet"
              disabled={busy || generating}
              onClick={() => setConfirmRegenerate(true)}
            >
              {zh ? "重新生成" : "Regenerate"}
            </button>
          ) : (
            <button
              className={chapters.length > 0 ? "button-danger" : "button-primary"}
              disabled={busy || generating}
              onClick={() => {
                setConfirmRegenerate(false);
                void onGenerate().catch(() => undefined);
              }}
            >
              {generating
                ? zh ? `正在生成 · ${status?.done ?? 0}/${status?.calls ?? "—"}` : `Generating · ${status?.done ?? 0}/${status?.calls ?? "—"}`
                : chapters.length > 0
                  ? zh ? "确认重新生成" : "Confirm regenerate"
                  : zh ? "生成章节" : "Generate chapters"}
            </button>
          )}
        </div>
      </header>

      {status && (generating || status.failed > 0 || status.state === "failed") && (
        <div className={`chapter-task-status${status.failed > 0 || status.state === "failed" ? " error" : ""}`} role="status">
          <strong>
            {generating
              ? zh ? "正在后台分析主题结构" : "Analyzing topic structure in the background"
              : zh ? "章节生成需要处理" : "Chapter generation needs attention"}
          </strong>
          <span>{status.done}/{status.calls ?? status.done + status.pending + status.failed}</span>
          {status.lastError && <small>{status.lastError}</small>}
        </div>
      )}

      <form
        className="chapter-add"
        onSubmit={(event) => {
          event.preventDefault();
          void addAtPlayhead().catch(() => undefined);
        }}
      >
        <label>
          <span>{zh ? "新章节标题" : "New chapter title"}</span>
          <input
            maxLength={200}
            placeholder={zh ? "例如：核心方案" : "e.g. Core approach"}
            value={newTitle}
            onChange={(event) => setNewTitle(event.target.value)}
          />
        </label>
        <small>
          {chapters.length === 0
            ? zh ? "第一个章节会从第一条字幕开始" : "The first chapter starts at the first cue"
            : `${zh ? "播放头" : "Playhead"} ${clock(currentTime)} · ${activeCue?.text || "—"}`}
        </small>
        <button className="button-primary" disabled={busy || working || !newTitle.trim()}>
          {zh ? "在播放头添加" : "Add at playhead"}
        </button>
      </form>

      {localError && <p className="chapter-local-error" role="alert">{localError}</p>}

      <section className="chapter-list" aria-label={zh ? "章节列表" : "Chapter list"}>
        {chapters.length === 0 ? (
          <div className="chapter-empty">
            <strong>{zh ? "还没有章节" : "No chapters yet"}</strong>
            <p>{zh ? "可由 AI 生成，也可从播放头手动添加。" : "Generate them with AI or add one at the playhead."}</p>
          </div>
        ) : chapters.map((chapter, index) => {
          const title = drafts[chapter.startSeg]?.title ?? chapter.title;
          const changed = title.trim() !== chapter.title;
          return (
            <article key={chapter.startSeg}>
              <button
                className="chapter-time"
                onClick={() => onSeek(chapter.start, false)}
                title={zh ? "跳到章节起点" : "Seek to chapter start"}
              >
                <span>{String(index + 1).padStart(2, "0")}</span>
                <strong>{clock(chapter.start)}</strong>
                <small>{clock(chapter.end - chapter.start)}</small>
              </button>
              <div>
                <input
                  aria-label={`${zh ? "章节标题" : "Chapter title"} ${index + 1}`}
                  maxLength={200}
                  value={title}
                  onChange={(event) => onDraftsChange((current) => ({
                    ...current,
                    [chapter.startSeg]: {
                      sourceTitle: current[chapter.startSeg]?.sourceTitle ?? chapter.title,
                      title: event.target.value,
                    },
                  }))}
                />
                <p>{chapter.preview}</p>
              </div>
              <button
                className="button-quiet"
                disabled={busy || working || (index === 0 && chapters.length > 1)}
                onClick={() => void removeChapter(chapter.startSeg).catch(() => undefined)}
                title={index === 0 && chapters.length > 1
                  ? zh ? "第一个章节不能单独删除" : "The first chapter cannot be removed by itself"
                  : undefined}
              >
                {zh ? "删除" : "Delete"}
              </button>
              {changed && <span className="chapter-dirty">{zh ? "未保存" : "Unsaved"}</span>}
            </article>
          );
        })}
      </section>

      {dirty.length > 0 && (
        <footer className="chapter-save">
          <span>{zh ? `${dirty.length} 个标题未保存` : `${dirty.length} unsaved title${dirty.length === 1 ? "" : "s"}`}</span>
          <button
            className="button-primary"
            disabled={busy || working || dirty.some((chapter) => !drafts[chapter.startSeg]?.title.trim())}
            onClick={() => void saveTitles().catch(() => undefined)}
          >
            {working ? zh ? "正在保存…" : "Saving…" : zh ? "保存章节" : "Save chapters"}
          </button>
        </footer>
      )}
    </div>
  );
}
