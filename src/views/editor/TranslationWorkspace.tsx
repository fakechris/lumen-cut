import { useEffect, useMemo, useState } from "react";
import type { Lang } from "../../i18n";
import type { Doc, TaskStatus } from "../../types";

interface Props {
  configured: boolean;
  doc: Doc;
  lang: Lang;
  status: TaskStatus | null;
  busy: boolean;
  onOpenSettings: () => void;
  onStart: (language: string) => Promise<void>;
}

const LANGUAGE_NAMES: Record<string, string> = {
  en: "English",
  zh: "中文",
  ja: "日本語",
  ko: "한국어",
  fr: "Français",
  es: "Español",
};

export function TranslationWorkspace({
  configured,
  doc,
  lang,
  status,
  busy,
  onOpenSettings,
  onStart,
}: Props) {
  const languages = Object.keys(doc.translations);
  const activeTaskLanguage = status?.kinds.find(
    (item) => item.kind === "translate" && item.lang,
  )?.lang;
  const [selected, setSelected] = useState(languages[0] || activeTaskLanguage || "en");

  useEffect(() => {
    if (activeTaskLanguage) {
      setSelected(activeTaskLanguage);
    } else if (languages.length > 0 && !languages.includes(selected)) {
      setSelected(languages[0]);
    }
  }, [languages.join("|"), activeTaskLanguage]);

  const sentences = useMemo(
    () => doc.paragraphs.flatMap((paragraph) =>
      paragraph.sentences.map((sentence) => ({
        ...sentence,
        speaker: paragraph.speaker,
      })),
    ),
    [doc],
  );
  const track = doc.translations[selected] || {};
  const completed = sentences.filter((sentence) => track[sentence.id]?.text).length;
  const translateTask = status?.kinds.find(
    (item) => item.kind === "translate" && (!item.lang || item.lang === selected),
  );
  const batchTotal = translateTask
    ? translateTask.calls ?? translateTask.pending + translateTask.done + translateTask.failed
    : 0;
  const taskStopped = translateTask?.state === "paused" || translateTask?.state === "failed";

  return (
    <div className="translation-workspace">
      <aside className="translation-sidebar">
        <p className="eyebrow">{lang === "zh" ? "目标语言" : "Target language"}</p>
        <select value={selected} onChange={(event) => setSelected(event.target.value)}>
          {Object.entries(LANGUAGE_NAMES).map(([code, name]) => (
            <option key={code} value={code}>{name}</option>
          ))}
        </select>
        <div className="translation-progress">
          <span>
            {lang === "zh" ? "完成度" : "Coverage"}
            <strong>{completed}/{sentences.length}</strong>
          </span>
          <progress max={Math.max(sentences.length, 1)} value={completed} />
        </div>
        {translateTask && (
          <div className="task-detail">
            <strong>{translateTask.pending > 0
              ? taskStopped
                ? (lang === "zh" ? "翻译已暂停" : "Translation paused")
                : (lang === "zh" ? "正在后台翻译" : "Translating in background")
              : translateTask.failed > 0
                ? (lang === "zh" ? "部分任务失败" : "Some calls failed")
                : (lang === "zh" ? "最近翻译完成" : "Latest translation complete")}
            </strong>
            <span>
              {lang === "zh"
                ? `已完成 ${translateTask.done} / ${batchTotal} 批`
                : `${translateTask.done} / ${batchTotal} batches completed`}
              {translateTask.failed > 0 && ` · ${translateTask.failed} ${lang === "zh" ? "失败" : "failed"}`}
            </span>
            <progress
              aria-label={lang === "zh" ? "翻译进度" : "Translation progress"}
              max={Math.max(batchTotal, 1)}
              value={translateTask.done}
            />
            {translateTask.lastError && (
              <p className="task-error-detail">{translateTask.lastError}</p>
            )}
          </div>
        )}
        {configured ? (
          <>
            <button
              className="button-primary"
              disabled={busy || Boolean(translateTask?.pending && !taskStopped)}
              onClick={() => onStart(selected)}
            >
              {translateTask?.pending && taskStopped
                ? (lang === "zh" ? "继续翻译" : "Resume translation")
                : translateTask?.pending
                ? (lang === "zh" ? "翻译进行中…" : "Translation running…")
                : completed > 0
                  ? (lang === "zh" ? "更新翻译" : "Update translation")
                  : (lang === "zh" ? "开始翻译" : "Start translation")}
            </button>
            <small>
              {lang === "zh"
                ? "后台服务会自动启动。源文修改后可再次更新。"
                : "The background service starts automatically. Run again after source edits."}
            </small>
          </>
        ) : (
          <div className="agent-setup-callout">
            <strong>{lang === "zh" ? "先连接 AI Agent" : "Connect an AI Agent first"}</strong>
            <p>
              {lang === "zh"
                ? "本地转写不需要 Agent；翻译需要在设置中填写服务地址和模型。"
                : "Local transcription does not need an Agent. Translation needs an endpoint and model in Settings."}
            </p>
            <button className="button-quiet" onClick={onOpenSettings}>
              {lang === "zh" ? "打开设置" : "Open settings"}
            </button>
          </div>
        )}
      </aside>

      <section className="translation-document">
        <header>
          <span>{lang === "zh" ? "原文" : "Source"}</span>
          <span>{LANGUAGE_NAMES[selected] || selected}</span>
        </header>
        {sentences.map((sentence) => (
          <div className="translation-row" key={sentence.id}>
            <div>
              {sentence.speaker && <small>{sentence.speaker}</small>}
              <p>{sentence.text}</p>
            </div>
            <div className={track[sentence.id]?.text ? "" : "translation-missing"}>
              <p>
                {track[sentence.id]?.text ||
                  (lang === "zh" ? "等待翻译" : "Waiting for translation")}
              </p>
            </div>
          </div>
        ))}
      </section>
    </div>
  );
}
