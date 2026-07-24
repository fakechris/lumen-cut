import { useState } from "react";
import type { Lang } from "../../i18n";
import type { Doc, TaskStatus } from "../../types";

interface Props {
  busy: boolean;
  configured: boolean;
  doc: Doc;
  lang: Lang;
  status: TaskStatus | null;
  onOpenSettings: () => void;
  onStart: (kind: string, language: string | null) => Promise<void>;
}

const TASKS = [
  {
    kind: "polish",
    zh: ["轻度润色", "尽量保持原意，修正明显口误、重复和不自然表达。"],
    en: ["Light polish", "Preserve meaning while fixing slips, repetition, and awkward phrasing."],
  },
  {
    kind: "repunct",
    zh: ["修复标点与断句", "只调整标点和句子边界，不改写内容。"],
    en: ["Repair punctuation", "Adjust punctuation and sentence boundaries without rewriting."],
  },
  {
    kind: "cleanup",
    zh: ["清理口播", "识别填充词、重录片段和过长停顿，生成可恢复的建议切口。"],
    en: ["Clean up speech", "Find fillers, retakes, and long pauses as reversible suggested cuts."],
  },
  {
    kind: "chapters",
    zh: ["生成章节", "按主题为较长内容生成章节标题和起点。"],
    en: ["Generate chapters", "Create chapter titles and starting points for longer content."],
  },
  {
    kind: "broll",
    zh: ["B-roll 建议", "标记适合补充画面的片段；不会自动添加素材。"],
    en: ["Suggest B-roll", "Mark moments that could use supporting visuals; no media is added automatically."],
  },
  {
    kind: "align",
    zh: ["优化翻译排版", "只处理字幕过长的翻译句，使其更适合单行显示。"],
    en: ["Fit translated subtitles", "Review only translated cues that are too long for a single line."],
  },
] as const;

export function EnhancementPanel({
  busy,
  configured,
  doc,
  lang,
  status,
  onOpenSettings,
  onStart,
}: Props) {
  const translationLanguage = Object.keys(doc.translations)[0] || null;
  const [confirmKind, setConfirmKind] = useState<string | null>(null);

  const start = (kind: string, language: string | null) => {
    setConfirmKind(null);
    void onStart(kind, language);
  };

  return (
    <section className="enhancement-panel" aria-labelledby="enhancement-title">
      <header>
        <div>
          <p className="eyebrow">{lang === "zh" ? "可选步骤" : "Optional steps"}</p>
          <h2 id="enhancement-title">
            {lang === "zh" ? "增强转写与成片结构" : "Enhance transcript and structure"}
          </h2>
        </div>
        {!configured && (
          <button className="button-quiet" onClick={onOpenSettings}>
            {lang === "zh" ? "配置 AI 功能" : "Configure AI features"}
          </button>
        )}
      </header>

      {!configured && (
        <p className="enhancement-note">
          {lang === "zh"
            ? "这些可选功能需要先在设置中填写服务地址和模型；API Key 仅在服务要求时填写。基础编辑与导出不受影响。"
            : "These optional features need an endpoint and model in Settings. Add an API key only when the service requires one. Core editing and export remain available."}
        </p>
      )}

      <div className="enhancement-list">
        {TASKS.map((task) => {
          const taskState = status?.kinds.find(
            (candidate) =>
              candidate.kind === task.kind &&
              (task.kind !== "align" || candidate.lang === translationLanguage),
          );
          const running = (taskState?.pending ?? 0) > 0;
          const failed = (taskState?.failed ?? 0) > 0;
          const completed = (taskState?.done ?? 0) > 0 && !running && !failed;
          const alignUnavailable = task.kind === "align" && !translationLanguage;
          const copy = task[lang];
          return (
            <article className="enhancement-row" key={task.kind}>
              <div>
                <strong>{copy[0]}</strong>
                <p>{copy[1]}</p>
                {alignUnavailable && (
                  <small>
                    {lang === "zh"
                      ? "完成至少一种翻译后可用。"
                      : "Available after at least one translation."}
                  </small>
                )}
                {failed && taskState?.lastError && (
                  <small className="task-inline-error">{taskState.lastError}</small>
                )}
              </div>
              <span
                className={
                  running
                    ? "enhancement-state running"
                    : failed
                      ? "enhancement-state failed"
                      : completed
                        ? "enhancement-state done"
                        : "enhancement-state"
                }
              >
                {running
                  ? lang === "zh" ? "处理中" : "Running"
                  : failed
                    ? lang === "zh" ? "失败" : "Failed"
                    : completed
                      ? lang === "zh" ? "已完成" : "Completed"
                      : lang === "zh" ? "未运行" : "Not run"}
              </span>
              {completed && confirmKind === task.kind ? (
                <div className="enhancement-rerun-confirm" role="alert">
                  <span>
                    {lang === "zh"
                      ? "再次运行可能替换这一步的现有结果。"
                      : "Running again may replace the existing result from this step."}
                  </span>
                  <button
                    className="button-quiet"
                    disabled={busy}
                    onClick={() => setConfirmKind(null)}
                  >
                    {lang === "zh" ? "取消" : "Cancel"}
                  </button>
                  <button
                    className="button-danger"
                    disabled={busy}
                    onClick={() => start(
                      task.kind,
                      task.kind === "align" ? translationLanguage : null,
                    )}
                  >
                    {lang === "zh" ? "确认再次运行" : "Confirm rerun"}
                  </button>
                </div>
              ) : (
                <button
                  className="button-quiet"
                  disabled={busy || running || !configured || alignUnavailable}
                  onClick={() => {
                    if (completed) {
                      setConfirmKind(task.kind);
                    } else {
                      start(
                        task.kind,
                        task.kind === "align" ? translationLanguage : null,
                      );
                    }
                  }}
                >
                  {running
                    ? <span className="spinner" aria-hidden="true" />
                    : null}
                  {failed || completed
                    ? lang === "zh" ? "再次运行" : "Run again"
                    : lang === "zh" ? "开始" : "Start"}
                </button>
              )}
            </article>
          );
        })}
      </div>
    </section>
  );
}
