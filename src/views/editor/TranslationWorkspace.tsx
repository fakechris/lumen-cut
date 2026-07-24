import { useEffect, useMemo, useState } from "react";
import { VirtualList } from "../../components/VirtualList";
import type { Lang } from "../../i18n";
import type { Doc, TaskStatus } from "../../types";

interface Props {
  configured: boolean;
  currentTime: number;
  doc: Doc;
  lang: Lang;
  status: TaskStatus | null;
  busy: boolean;
  drafts: Record<string, Record<string, TranslationDraft>>;
  onDraftsChange: (
    language: string,
    update: (
      current: Record<string, TranslationDraft>,
    ) => Record<string, TranslationDraft>,
  ) => void;
  onOpenSettings: () => void;
  onPause: () => Promise<void>;
  onLanguageChange: (language: string) => void;
  onSave: (language: string, id: string, text: string) => Promise<void>;
  onSaveMany: (
    language: string,
    updates: Array<{ id: string; text: string }>,
  ) => Promise<void>;
  onSeek: (seconds: number, autoplay?: boolean) => void;
  onStart: (language: string, staleOnly: boolean) => Promise<void>;
}

export type TranslationDraft = {
  text: string;
  savedText: string;
};

const LANGUAGE_NAMES: Record<string, string> = {
  en: "English",
  zh: "中文",
  ja: "日本語",
  ko: "한국어",
  fr: "Français",
  es: "Español",
  de: "Deutsch",
  pt: "Português",
  it: "Italiano",
  ru: "Русский",
  ar: "العربية",
  hi: "हिन्दी",
  th: "ไทย",
  vi: "Tiếng Việt",
  id: "Bahasa Indonesia",
  tr: "Türkçe",
};

const translationRowKey = (
  sentence: Doc["paragraphs"][number]["sentences"][number],
) => sentence.id;
const EMPTY_TRACK: Record<string, { text: string }> = {};

export function translationActivityLabel(
  task: TaskStatus["kinds"][number] | undefined,
  lang: Lang,
) {
  if (!task || (task.queued === undefined && task.inFlight === undefined)) return null;
  const parts: string[] = [];
  if ((task.inFlight ?? 0) > 0) {
    parts.push(lang === "zh" ? `${task.inFlight} 个请求正在等待模型返回` : `${task.inFlight} request(s) awaiting the model`);
  }
  if ((task.queued ?? 0) > 0) {
    parts.push(lang === "zh" ? `${task.queued} 个批次排队中` : `${task.queued} batch(es) queued`);
  }
  if ((task.retrying ?? 0) > 0 && task.attempt && task.maxAttempts) {
    parts.push(
      lang === "zh"
        ? `重试第 ${task.attempt}/${task.maxAttempts} 次`
        : `retry attempt ${task.attempt}/${task.maxAttempts}`,
    );
  }
  if (parts.length === 0 && task.pending > 0) {
    return lang === "zh" ? "正在校验并保存模型结果" : "Validating and saving model results";
  }
  return parts.join(" · ");
}

export function TranslationWorkspace({
  configured,
  currentTime,
  doc,
  lang,
  status,
  busy,
  drafts: storedDrafts,
  onDraftsChange,
  onOpenSettings,
  onPause,
  onLanguageChange,
  onSave,
  onSaveMany,
  onSeek,
  onStart,
}: Props) {
  const languages = Object.keys(doc.translations);
  const activeTaskLanguage = status?.kinds.find(
    (item) => item.kind === "translate" && item.lang,
  )?.lang;
  const [selected, setSelected] = useState(languages[0] || activeTaskLanguage || "en");
  const [query, setQuery] = useState("");
  const [customLanguage, setCustomLanguage] = useState("");
  const [saveErrors, setSaveErrors] = useState<Record<string, string>>({});
  const [savingId, setSavingId] = useState<string | null>(null);
  const [savingAll, setSavingAll] = useState(false);
  const [pendingLanguage, setPendingLanguage] = useState<string | null>(null);
  const [confirmRetranslate, setConfirmRetranslate] = useState(false);

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
  const sentenceIds = useMemo(
    () => new Set(sentences.map((sentence) => sentence.id)),
    [sentences],
  );
  const track = doc.translations[selected] || EMPTY_TRACK;
  const languageDrafts = storedDrafts[selected] || {};
  const dirtyIds = useMemo(
    () => new Set(Object.entries(languageDrafts)
      .filter(([id, draft]) =>
        sentenceIds.has(id)
        && draft.text !== (track[id]?.text || ""))
      .map(([id]) => id)),
    [languageDrafts, sentenceIds, track],
  );
  const translationCounts = useMemo(() => sentences.reduce(
    (counts, sentence) => {
      const translation = track[sentence.id];
      if (translation?.text) counts.completed += 1;
      if (!translation?.text || translation.sourceText !== sentence.text) {
        counts.needsUpdate += 1;
      }
      return counts;
    },
    { completed: 0, needsUpdate: 0 },
  ), [sentences, track]);
  const { completed, needsUpdate } = translationCounts;
  const translateTask = status?.kinds.find(
    (item) => item.kind === "translate" && (!item.lang || item.lang === selected),
  );
  const otherTranslateTask = status?.kinds.find(
    (item) => item.kind === "translate"
      && item.lang
      && item.lang !== selected
      && item.pending > 0,
  );
  const batchTotal = translateTask
    ? translateTask.calls ?? translateTask.pending + translateTask.done + translateTask.failed
    : 0;
  const taskStopped = translateTask?.state === "paused" || translateTask?.state === "failed";
  const liveActivity = translationActivityLabel(translateTask, lang);
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

  const clock = (seconds: number) => {
    const minutes = Math.floor(seconds / 60);
    const rest = Math.floor(seconds % 60);
    return `${String(minutes).padStart(2, "0")}:${String(rest).padStart(2, "0")}`;
  };

  const errorMessage = (error: unknown) =>
    error instanceof Error ? error.message : String(error);

  const saveDraft = async (id: string) => {
    if (!dirtyIds.has(id)) return;
    setSavingId(id);
    setSaveErrors((current) => {
      const next = { ...current };
      delete next[id];
      return next;
    });
    try {
      await onSave(selected, id, languageDrafts[id]?.text ?? track[id]?.text ?? "");
      onDraftsChange(selected, (current) => {
        const next = { ...current };
        delete next[id];
        return next;
      });
    } catch (error) {
      setSaveErrors((current) => ({ ...current, [id]: errorMessage(error) }));
      throw error;
    } finally {
      setSavingId(null);
    }
  };

  const saveAllDrafts = async () => {
    const updates = sentences
      .filter((sentence) => dirtyIds.has(sentence.id))
      .map((sentence) => ({
        id: sentence.id,
        text: languageDrafts[sentence.id]?.text ?? track[sentence.id]?.text ?? "",
      }));
    if (updates.length === 0) return;
    setSavingAll(true);
    setSaveErrors({});
    try {
      await onSaveMany(selected, updates);
      onDraftsChange(selected, (current) => {
        const next = { ...current };
        for (const update of updates) delete next[update.id];
        return next;
      });
    } catch (error) {
      const message = errorMessage(error);
      setSaveErrors(Object.fromEntries(updates.map((update) => [update.id, message])));
      throw error;
    } finally {
      setSavingAll(false);
    }
  };

  const switchLanguage = (language: string) => {
    setPendingLanguage(null);
    setSaveErrors({});
    setSelected(language);
  };

  const discardAndSwitchLanguage = (language: string) => {
    onDraftsChange(selected, () => ({}));
    switchLanguage(language);
  };

  const requestLanguageChange = (language: string) => {
    if (language === selected) return;
    if (dirtyIds.size > 0) {
      setPendingLanguage(language);
    } else {
      setSelected(language);
    }
  };

  const normalizedCustomLanguage = customLanguage.trim();
  const validCustomLanguage = /^[A-Za-z]{2,3}(?:-[A-Za-z0-9]{2,8})*$/
    .test(normalizedCustomLanguage);

  const languageOptions = Array.from(new Set([
    ...Object.keys(LANGUAGE_NAMES),
    ...languages,
    selected,
  ]));

  const primaryActionLabel = otherTranslateTask
    ? (lang === "zh" ? "其他语言待处理" : "Other language pending")
    : translateTask?.pending && taskStopped
      ? (lang === "zh" ? "继续翻译" : "Resume")
      : translateTask?.pending
        ? (lang === "zh" ? "翻译中…" : "Running…")
        : completed > 0
          ? needsUpdate > 0
            ? (lang === "zh" ? `更新 ${needsUpdate}` : `Update ${needsUpdate}`)
            : (lang === "zh" ? "已是最新" : "Up to date")
          : (lang === "zh" ? "开始翻译" : "Start");
  const primaryActionTitle = lang === "zh"
    ? "按上下文批量请求；更新只发送缺失或源文有变化的字幕。"
    : "Context-aware batches. Update sends only missing or changed lines.";
  const taskStatusLabel = translateTask
    ? translateTask.pending > 0
      ? taskStopped
        ? (lang === "zh" ? "已暂停" : "Paused")
        : (lang === "zh" ? "后台翻译中" : "Translating")
      : translateTask.failed > 0
        ? (lang === "zh" ? "部分失败" : "Partial failure")
        : (lang === "zh" ? "最近完成" : "Complete")
    : null;

  return (
    <div className="translation-workspace">
      <aside className="translation-sidebar">
        <div className="translation-toolbar">
          <select
            aria-label={lang === "zh" ? "目标语言" : "Target language"}
            value={selected}
            onChange={(event) => {
              requestLanguageChange(event.target.value);
            }}
          >
            {languageOptions.map((code) => (
              <option key={code} value={code}>{LANGUAGE_NAMES[code] || code}</option>
            ))}
          </select>
          <div className="translation-custom-language">
            <input
              aria-label={lang === "zh" ? "自定义目标语言代码" : "Custom target language code"}
              placeholder={lang === "zh" ? "其他语言，如 de-CH" : "Other, e.g. de-CH"}
              value={customLanguage}
              onChange={(event) => setCustomLanguage(event.target.value)}
            />
            <button
              className="button-quiet"
              disabled={!validCustomLanguage}
              onClick={() => {
                requestLanguageChange(normalizedCustomLanguage);
                setCustomLanguage("");
              }}
            >
              {lang === "zh" ? "使用" : "Use"}
            </button>
          </div>
          <label className="translation-search">
            <span className="sr-only">{lang === "zh" ? "搜索翻译" : "Search translation"}</span>
            <input
              placeholder={lang === "zh" ? "搜索原文或译文…" : "Search source or translation…"}
              type="search"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
          </label>
          {configured ? (
            <div className="translation-actions">
              <button
                className="button-primary"
                disabled={busy
                  || Boolean(otherTranslateTask)
                  || Boolean(translateTask?.pending && !taskStopped)
                  || (completed > 0 && needsUpdate === 0 && !taskStopped)}
                title={primaryActionTitle}
                onClick={() => onStart(selected, completed > 0)}
              >
                {primaryActionLabel}
              </button>
              {completed > 0 && !translateTask?.pending && !confirmRetranslate && (
                <button
                  className="translation-retranslate button-quiet"
                  disabled={busy}
                  onClick={() => setConfirmRetranslate(true)}
                >
                  {lang === "zh" ? "全部重译" : "Retranslate"}
                </button>
              )}
            </div>
          ) : (
            <button className="button-quiet" onClick={onOpenSettings}>
              {lang === "zh" ? "连接 Agent" : "Connect Agent"}
            </button>
          )}
        </div>

        <div className="translation-status">
          <div className="translation-progress">
            <span>
              {lang === "zh" ? "完成度" : "Coverage"}
              <strong>{completed}/{sentences.length}</strong>
            </span>
            <progress max={Math.max(sentences.length, 1)} value={completed} />
          </div>
          {translateTask && (
            <div className="task-detail">
              <strong>{taskStatusLabel}</strong>
              <span>
                {lang === "zh"
                  ? `${translateTask.done}/${batchTotal} 批`
                  : `${translateTask.done}/${batchTotal} batches`}
                {translateTask.failed > 0 && ` · ${translateTask.failed} ${lang === "zh" ? "失败" : "failed"}`}
              </span>
              {liveActivity && <small className="task-live-activity">{liveActivity}</small>}
              <progress
                aria-label={lang === "zh" ? "翻译进度" : "Translation progress"}
                max={Math.max(batchTotal, 1)}
                value={translateTask.done}
              />
              {translateTask.pending > 0 && !taskStopped && (
                <button
                  className="button-quiet"
                  disabled={busy}
                  onClick={() => void onPause().catch(() => undefined)}
                >
                  {lang === "zh" ? "暂停" : "Pause"}
                </button>
              )}
              {translateTask.lastError && (
                <p className="task-error-detail">{translateTask.lastError}</p>
              )}
            </div>
          )}
        </div>

        {confirmRetranslate && (
          <div className="translation-retranslate-confirm" role="alert">
            <span>
              {lang === "zh"
                ? `确认覆盖并重新翻译全部 ${sentences.length} 条？`
                : `Replace and retranslate all ${sentences.length} lines?`}
            </span>
            <button
              className="button-quiet"
              disabled={busy}
              onClick={() => setConfirmRetranslate(false)}
            >
              {lang === "zh" ? "取消" : "Cancel"}
            </button>
            <button
              className="button-danger"
              disabled={busy}
              onClick={() => {
                setConfirmRetranslate(false);
                void onStart(selected, false);
              }}
            >
              {lang === "zh" ? "确认重译" : "Confirm"}
            </button>
          </div>
        )}

        {dirtyIds.size > 0 && (
          <div className="translation-draft-actions" role="status">
            <span>
              {lang === "zh"
                ? `${dirtyIds.size} 条修改尚未保存`
                : `${dirtyIds.size} unsaved change${dirtyIds.size === 1 ? "" : "s"}`}
            </span>
            <button
              className="button-primary"
              disabled={busy || savingAll}
              onClick={() => void saveAllDrafts().catch(() => undefined)}
            >
              {savingAll
                ? (lang === "zh" ? "保存中…" : "Saving…")
                : (lang === "zh" ? "保存全部" : "Save all")}
            </button>
          </div>
        )}

        {pendingLanguage && (
          <div className="translation-language-confirm" role="alert">
            <span>
              {lang === "zh"
                ? `切换到 ${LANGUAGE_NAMES[pendingLanguage] || pendingLanguage} 前，如何处理未保存修改？`
                : `Handle unsaved changes before switching to ${LANGUAGE_NAMES[pendingLanguage] || pendingLanguage}.`}
            </span>
            <button
              className="button-quiet"
              disabled={savingAll}
              onClick={() => setPendingLanguage(null)}
            >
              {lang === "zh" ? "取消" : "Cancel"}
            </button>
            <button
              className="button-quiet"
              disabled={savingAll}
              onClick={() => discardAndSwitchLanguage(pendingLanguage)}
            >
              {lang === "zh" ? "放弃" : "Discard"}
            </button>
            <button
              className="button-primary"
              disabled={busy || savingAll}
              onClick={() => {
                const target = pendingLanguage;
                void saveAllDrafts()
                  .then(() => switchLanguage(target))
                  .catch(() => undefined);
              }}
            >
              {lang === "zh" ? "保存并切换" : "Save & switch"}
            </button>
          </div>
        )}

        {otherTranslateTask && (
          <div className="translation-task-conflict" role="alert">
            <span>
              {lang === "zh"
                ? `${(otherTranslateTask.lang || "").toUpperCase()} 仍有 ${otherTranslateTask.pending} 批未完成，完成后才能开始 ${selected.toUpperCase()}。`
                : `${(otherTranslateTask.lang || "").toUpperCase()} still has ${otherTranslateTask.pending} batches. Finish it before ${selected.toUpperCase()}.`}
            </span>
            <button
              className="button-quiet"
              onClick={() => requestLanguageChange(otherTranslateTask.lang || selected)}
            >
              {lang === "zh" ? "切回" : "Open"}
            </button>
          </div>
        )}

        {!configured && (
          <div className="agent-setup-callout">
            <strong>{lang === "zh" ? "先连接 AI Agent" : "Connect an AI Agent first"}</strong>
            <p>
              {lang === "zh"
                ? "本地转写不需要 Agent；翻译需要在设置中填写服务地址和模型。"
                : "Local transcription does not need an Agent. Translation needs an endpoint and model in Settings."}
            </p>
          </div>
        )}
      </aside>

      <section className="translation-document">
        <header>
          <span>{lang === "zh" ? "原文" : "Source"}</span>
          <span>{LANGUAGE_NAMES[selected] || selected}</span>
        </header>
        <VirtualList
          activeKey={activeSentence?.id}
          className="translation-virtual-list"
          estimateHeight={96}
          followActive={!query}
          itemKey={translationRowKey}
          items={visibleSentences}
          renderItem={(sentence) => {
          const duration = Math.max(0.1, sentence.end - sentence.start);
          const translated = languageDrafts[sentence.id]?.text ?? track[sentence.id]?.text ?? "";
          const targetCps = translated ? translated.replace(/\s/g, "").length / duration : 0;
          const active = activeSentence?.id === sentence.id;
          return (
          <div
            className={`translation-row${active ? " active" : ""}`}
            key={sentence.id}
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
                  onDraftsChange(selected, (current) => {
                    const next = { ...current };
                    if (text === (track[sentence.id]?.text || "")) {
                      delete next[sentence.id];
                    } else {
                      next[sentence.id] = {
                        text,
                        savedText: track[sentence.id]?.text || "",
                      };
                    }
                    return next;
                  });
                  setSaveErrors((current) => {
                    const next = { ...current };
                    delete next[sentence.id];
                    return next;
                  });
                }}
                onKeyDown={(event) => {
                  if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
                    event.preventDefault();
                    void saveDraft(sentence.id).catch(() => undefined);
                  }
                }}
                onBlur={() => void saveDraft(sentence.id).catch(() => undefined)}
              />
              {savingId === sentence.id && (
                <small className="translation-saving" role="status">
                  {lang === "zh" ? "正在保存…" : "Saving…"}
                </small>
              )}
              {saveErrors[sentence.id] && (
                <small className="translation-save-error" role="alert">
                  {lang === "zh" ? "保存失败，草稿仍保留。" : "Save failed. Draft preserved."}
                </small>
              )}
            </div>
          </div>
        )}}
        />
      </section>
    </div>
  );
}
