import { useEffect, useMemo, useRef, useState } from "react";
import type { Lang } from "../../i18n";
import type { Doc, TaskStatus } from "../../types";

interface Props {
  configured: boolean;
  currentTime: number;
  doc: Doc;
  lang: Lang;
  status: TaskStatus | null;
  busy: boolean;
  onOpenSettings: () => void;
  onLanguageChange: (language: string) => void;
  onSave: (language: string, id: string, text: string) => Promise<void>;
  onSeek: (seconds: number, autoplay?: boolean) => void;
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
const EMPTY_TRACK: Record<string, { text: string }> = {};

export function TranslationWorkspace({
  configured,
  currentTime,
  doc,
  lang,
  status,
  busy,
  onOpenSettings,
  onLanguageChange,
  onSave,
  onSeek,
  onStart,
}: Props) {
  const languages = Object.keys(doc.translations);
  const activeTaskLanguage = status?.kinds.find(
    (item) => item.kind === "translate" && item.lang,
  )?.lang;
  const [selected, setSelected] = useState(languages[0] || activeTaskLanguage || "en");
  const [query, setQuery] = useState("");
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [savingId, setSavingId] = useState<string | null>(null);
  const rowRefs = useRef(new Map<string, HTMLDivElement>());

  useEffect(() => {
    if (activeTaskLanguage) {
      setSelected(activeTaskLanguage);
    } else if (languages.length > 0 && !languages.includes(selected)) {
      setSelected(languages[0]);
    }
  }, [languages.join("|"), activeTaskLanguage]);

  useEffect(() => {
    onLanguageChange(selected);
  }, [onLanguageChange, selected]);

  const sentences = useMemo(
    () => doc.paragraphs.flatMap((paragraph) =>
      paragraph.sentences.map((sentence) => ({
        ...sentence,
        speaker: paragraph.speaker,
        start: sentence.words[0]?.start ?? 0,
        end: sentence.words[sentence.words.length - 1]?.end ?? 0,
      })),
    ),
    [doc],
  );
  const track = doc.translations[selected] || EMPTY_TRACK;

  useEffect(() => {
    setDrafts(Object.fromEntries(
      sentences.map((sentence) => [sentence.id, track[sentence.id]?.text || ""]),
    ));
  }, [selected, sentences, track]);
  const completed = sentences.filter((sentence) => track[sentence.id]?.text).length;
  const translateTask = status?.kinds.find(
    (item) => item.kind === "translate" && (!item.lang || item.lang === selected),
  );
  const batchTotal = translateTask
    ? translateTask.calls ?? translateTask.pending + translateTask.done + translateTask.failed
    : 0;
  const taskStopped = translateTask?.state === "paused" || translateTask?.state === "failed";
  const visibleSentences = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    if (!needle) return sentences;
    return sentences.filter((sentence) =>
      sentence.text.toLocaleLowerCase().includes(needle)
      || track[sentence.id]?.text?.toLocaleLowerCase().includes(needle),
    );
  }, [query, sentences, track]);
  const activeSentence = useMemo(() => {
    let low = 0;
    let high = sentences.length - 1;
    let candidate = -1;
    while (low <= high) {
      const middle = Math.floor((low + high) / 2);
      if (sentences[middle].start <= currentTime) {
        candidate = middle;
        low = middle + 1;
      } else {
        high = middle - 1;
      }
    }
    const sentence = candidate >= 0 ? sentences[candidate] : undefined;
    return sentence && currentTime < sentence.end ? sentence : undefined;
  }, [currentTime, sentences]);

  useEffect(() => {
    if (!activeSentence || query) return;
    const row = rowRefs.current.get(activeSentence.id);
    if (!row || typeof row.scrollIntoView !== "function") return;
    row.scrollIntoView({
      behavior: window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth",
      block: "center",
    });
  }, [activeSentence?.id, query]);

  const clock = (seconds: number) => {
    const minutes = Math.floor(seconds / 60);
    const rest = Math.floor(seconds % 60);
    return `${String(minutes).padStart(2, "0")}:${String(rest).padStart(2, "0")}`;
  };

  return (
    <div className="translation-workspace">
      <aside className="translation-sidebar">
        <p className="eyebrow">{lang === "zh" ? "目标语言" : "Target language"}</p>
        <select value={selected} onChange={(event) => setSelected(event.target.value)}>
          {Object.entries(LANGUAGE_NAMES).map(([code, name]) => (
            <option key={code} value={code}>{name}</option>
          ))}
        </select>
        <label className="translation-search">
          <span className="sr-only">{lang === "zh" ? "搜索翻译" : "Search translation"}</span>
          <input
            placeholder={lang === "zh" ? "搜索原文或译文…" : "Search source or translation…"}
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </label>
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
        {visibleSentences.map((sentence) => {
          const duration = Math.max(0.1, sentence.end - sentence.start);
          const translated = drafts[sentence.id] ?? track[sentence.id]?.text ?? "";
          const targetCps = translated ? translated.replace(/\s/g, "").length / duration : 0;
          const active = activeSentence?.id === sentence.id;
          return (
          <div
            className={`translation-row${active ? " active" : ""}`}
            key={sentence.id}
            ref={(element) => {
              if (element) rowRefs.current.set(sentence.id, element);
              else rowRefs.current.delete(sentence.id);
            }}
          >
            <div>
              <header className="translation-cue-meta">
                <button
                  aria-label={`${lang === "zh" ? "播放字幕" : "Play cue"} ${clock(sentence.start)}`}
                  onClick={() => onSeek(sentence.start, true)}
                >
                  ▶
                </button>
                <small>{sentence.speaker || (lang === "zh" ? "未标记" : "Unlabelled")}</small>
                <span>{clock(sentence.start)}–{clock(sentence.end)}</span>
              </header>
              <p>{sentence.text}</p>
            </div>
            <div className={translated ? "" : "translation-missing"}>
              {translated && (
                <span className={`translation-cps${targetCps > 17 ? " warning" : ""}`}>
                  {targetCps.toFixed(1)} CPS
                </span>
              )}
              <textarea
                aria-label={`${lang === "zh" ? "编辑译文" : "Edit translation"} ${clock(sentence.start)}`}
                disabled={busy && savingId !== sentence.id}
                placeholder={lang === "zh" ? "等待翻译，可直接输入译文" : "Waiting for translation, or enter text"}
                rows={2}
                value={translated}
                onChange={(event) => {
                  const text = event.target.value;
                  setDrafts((current) => ({ ...current, [sentence.id]: text }));
                }}
                onBlur={async () => {
                  const saved = track[sentence.id]?.text || "";
                  if (translated === saved) return;
                  setSavingId(sentence.id);
                  try {
                    await onSave(selected, sentence.id, translated);
                  } finally {
                    setSavingId(null);
                  }
                }}
              />
              {savingId === sentence.id && (
                <small className="translation-saving" role="status">
                  {lang === "zh" ? "正在保存…" : "Saving…"}
                </small>
              )}
            </div>
          </div>
        )})}
      </section>
    </div>
  );
}
