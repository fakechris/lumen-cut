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
    zh: ["让口播更顺", "改口误和重复，尽量不改原意。"],
    en: ["Smooth the speech", "Fix slips and repetition without changing meaning."],
  },
  {
    kind: "repunct",
    zh: ["修好断句标点", "只动标点和句子边界。"],
    en: ["Fix punctuation", "Boundaries only — no rewrites."],
  },
  {
    kind: "cleanup",
    zh: ["找出可剪的口癖", "填充词、重录、长停顿 → 可恢复切口。"],
    en: ["Find speech to cut", "Fillers, retakes, long pauses as reversible cuts."],
  },
  {
    kind: "chapters",
    zh: ["加章节标题", "按主题标出段落起点。"],
    en: ["Add chapters", "Title the main sections by topic."],
  },
  {
    kind: "broll",
    zh: ["建议补画面", "标出适合 B-roll 的位置，不自动塞素材。"],
    en: ["Suggest B-roll spots", "Mark moments only — nothing is inserted automatically."],
  },
  {
    kind: "align",
    zh: ["拆开过长译文", "只处理超长翻译行，不重翻。"],
    en: ["Split long translations", "Fit over-long lines only — no re-translation."],
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
                {running && taskState?.inFlight !== undefined && (
                  <small className="task-live-activity">
                    {lang === "zh"
                      ? `${taskState.inFlight} 个请求在途 · ${taskState.queued ?? 0} 个等待`
                      : `${taskState.inFlight} in flight · ${taskState.queued ?? 0} queued`}
                    {(taskState.retrying ?? 0) > 0 && taskState.attempt && taskState.maxAttempts
                      ? lang === "zh"
                        ? ` · 重试 ${taskState.attempt}/${taskState.maxAttempts}`
                        : ` · retry ${taskState.attempt}/${taskState.maxAttempts}`
                      : ""}
                  </small>
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
