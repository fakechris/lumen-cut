import { useEffect, useMemo, useState } from "react";
import type { Lang } from "../../i18n";
import type { Doc, SpeakerInfo } from "../../types";

interface Props {
  busy: boolean;
  doc: Doc;
  lang: Lang;
  speakers: SpeakerInfo[];
  onIdentify: () => Promise<void>;
  onMerge: (from: string, into: string) => Promise<void>;
  onRename: (from: string, to: string) => Promise<void>;
  onSaveMeta: (
    title: string,
    description: string,
    language: string | null,
  ) => Promise<void>;
}

const LANGUAGE_OPTIONS = [
  ["zh", "中文", "Chinese"],
  ["en", "英语", "English"],
  ["ja", "日语", "Japanese"],
  ["ko", "韩语", "Korean"],
  ["es", "西班牙语", "Spanish"],
  ["fr", "法语", "French"],
  ["de", "德语", "German"],
] as const;

export function PropertiesWorkspace({
  busy,
  doc,
  lang,
  speakers,
  onIdentify,
  onMerge,
  onRename,
  onSaveMeta,
}: Props) {
  const [title, setTitle] = useState(doc.meta.title);
  const [description, setDescription] = useState(doc.meta.description);
  const [language, setLanguage] = useState(doc.meta.language || "");
  const [names, setNames] = useState<Record<string, string>>({});
  const [mergeTargets, setMergeTargets] = useState<Record<string, string>>({});
  const [working, setWorking] = useState<string | null>(null);

  useEffect(() => {
    setTitle(doc.meta.title);
    setDescription(doc.meta.description);
    setLanguage(doc.meta.language || "");
  }, [doc.id, doc.meta.description, doc.meta.language, doc.meta.title]);

  useEffect(() => {
    setNames(Object.fromEntries(speakers.map((speaker) => [speaker.id, speaker.id])));
    setMergeTargets((current) =>
      Object.fromEntries(
        speakers.map((speaker) => [
          speaker.id,
          speakers.some((candidate) => candidate.id === current[speaker.id])
            ? current[speaker.id]
            : "",
        ]),
      ),
    );
  }, [speakers]);

  const unlabelled = useMemo(
    () => doc.paragraphs.filter((paragraph) => !paragraph.speaker).length,
    [doc.paragraphs],
  );
  const metaDirty =
    title.trim() !== doc.meta.title ||
    description.trim() !== doc.meta.description ||
    language !== (doc.meta.language || "");
  const currentLanguageKnown =
    !language || LANGUAGE_OPTIONS.some(([value]) => value === language);

  const saveMeta = async () => {
    if (!title.trim() || !metaDirty) return;
    setWorking("meta");
    try {
      await onSaveMeta(title.trim(), description.trim(), language || null);
    } finally {
      setWorking(null);
    }
  };

  const rename = async (speaker: SpeakerInfo) => {
    const next = (names[speaker.id] || "").trim();
    if (!next || next === speaker.id) return;
    setWorking(`rename-${speaker.id}`);
    try {
      await onRename(speaker.id, next);
    } finally {
      setWorking(null);
    }
  };

  const merge = async (speaker: SpeakerInfo) => {
    const target = mergeTargets[speaker.id];
    if (!target) return;
    setWorking(`merge-${speaker.id}`);
    try {
      await onMerge(speaker.id, target);
    } finally {
      setWorking(null);
    }
  };

  return (
    <div className="properties-workspace">
      <section className="property-section project-properties">
        <header>
          <div>
            <p className="eyebrow">{lang === "zh" ? "项目信息" : "Project information"}</p>
            <h2>{lang === "zh" ? "作品属性" : "Project details"}</h2>
          </div>
          <button
            className={metaDirty ? "button-primary" : "button-quiet"}
            disabled={busy || working !== null || !metaDirty || !title.trim()}
            onClick={saveMeta}
          >
            {working === "meta"
              ? lang === "zh" ? "保存中…" : "Saving…"
              : lang === "zh" ? "保存属性" : "Save details"}
          </button>
        </header>

        <div className="property-form">
          <label>
            <span>{lang === "zh" ? "标题" : "Title"}</span>
            <input
              value={title}
              onChange={(event) => setTitle(event.target.value)}
            />
          </label>
          <label>
            <span>{lang === "zh" ? "内容语言" : "Content language"}</span>
            <select
              value={language}
              onChange={(event) => setLanguage(event.target.value)}
            >
              <option value="">{lang === "zh" ? "自动检测" : "Auto-detect"}</option>
              {!currentLanguageKnown && <option value={language}>{language}</option>}
              {LANGUAGE_OPTIONS.map(([value, zh, en]) => (
                <option key={value} value={value}>
                  {lang === "zh" ? zh : en}
                </option>
              ))}
            </select>
          </label>
          <label className="property-description">
            <span>{lang === "zh" ? "说明" : "Description"}</span>
            <textarea
              placeholder={
                lang === "zh"
                  ? "可选：记录主题、交付要求或审阅说明"
                  : "Optional: topic, delivery notes, or review context"
              }
              rows={3}
              value={description}
              onChange={(event) => setDescription(event.target.value)}
            />
          </label>
        </div>
      </section>

      <section className="property-section speaker-properties">
        <header>
          <div>
            <p className="eyebrow">{lang === "zh" ? "说话人" : "Speakers"}</p>
            <h2>
              {speakers.length} {lang === "zh" ? "位说话人" : speakers.length === 1 ? "speaker" : "speakers"}
            </h2>
          </div>
          <button
            className={speakers.length ? "button-quiet" : "button-primary"}
            disabled={busy || working !== null || doc.paragraphs.length === 0}
            onClick={async () => {
              setWorking("identify");
              try {
                await onIdentify();
              } finally {
                setWorking(null);
              }
            }}
          >
            {working === "identify"
              ? lang === "zh" ? "识别中…" : "Identifying…"
              : speakers.length
                ? lang === "zh" ? "重新识别" : "Identify again"
                : lang === "zh" ? "识别说话人" : "Identify speakers"}
          </button>
        </header>

        {unlabelled > 0 && (
          <p className="speaker-coverage" role="status">
            {lang === "zh"
              ? `还有 ${unlabelled} 个段落未标记说话人。`
              : `${unlabelled} paragraph${unlabelled === 1 ? "" : "s"} still need a speaker label.`}
          </p>
        )}

        {speakers.length === 0 ? (
          <div className="speaker-empty">
            <strong>
              {lang === "zh" ? "还没有说话人标签" : "No speaker labels yet"}
            </strong>
            <p>
              {lang === "zh"
                ? "识别后可以把 SPEAKER_00 这样的标签改成真实姓名。"
                : "After identification, replace labels such as SPEAKER_00 with real names."}
            </p>
          </div>
        ) : (
          <div className="speaker-list">
            {speakers.map((speaker, index) => {
              const nextName = names[speaker.id] ?? speaker.id;
              const renameDirty = nextName.trim() !== speaker.id;
              const otherSpeakers = speakers.filter((candidate) => candidate.id !== speaker.id);
              return (
                <article className="speaker-row" key={speaker.id}>
                  <span
                    aria-hidden="true"
                    className={`speaker-swatch speaker-swatch-${index % 6}`}
                  />
                  <div className="speaker-identity">
                    <label htmlFor={`speaker-${speaker.id}`}>
                      {lang === "zh" ? "显示名称" : "Display name"}
                    </label>
                    <div>
                      <input
                        id={`speaker-${speaker.id}`}
                        value={nextName}
                        onChange={(event) =>
                          setNames((current) => ({
                            ...current,
                            [speaker.id]: event.target.value,
                          }))
                        }
                        onKeyDown={(event) => {
                          if (event.key === "Enter") {
                            event.preventDefault();
                            void rename(speaker);
                          }
                        }}
                      />
                      <button
                        className={renameDirty ? "button-primary" : "button-quiet"}
                        disabled={busy || working !== null || !renameDirty || !nextName.trim()}
                        onClick={() => rename(speaker)}
                      >
                        {working === `rename-${speaker.id}`
                          ? lang === "zh" ? "保存中…" : "Saving…"
                          : lang === "zh" ? "保存" : "Save"}
                      </button>
                    </div>
                    <small>
                      {speaker.paragraph_count}{" "}
                      {lang === "zh"
                        ? "个段落"
                        : speaker.paragraph_count === 1 ? "paragraph" : "paragraphs"}
                    </small>
                  </div>
                  <div className="speaker-merge">
                    <label htmlFor={`merge-${speaker.id}`}>
                      {lang === "zh" ? "合并到" : "Merge into"}
                    </label>
                    <div>
                      <select
                        disabled={otherSpeakers.length === 0}
                        id={`merge-${speaker.id}`}
                        value={mergeTargets[speaker.id] || ""}
                        onChange={(event) =>
                          setMergeTargets((current) => ({
                            ...current,
                            [speaker.id]: event.target.value,
                          }))
                        }
                      >
                        <option value="">
                          {otherSpeakers.length
                            ? lang === "zh" ? "选择说话人…" : "Choose speaker…"
                            : lang === "zh" ? "没有其他说话人" : "No other speaker"}
                        </option>
                        {otherSpeakers.map((candidate) => (
                          <option key={candidate.id} value={candidate.id}>
                            {candidate.id}
                          </option>
                        ))}
                      </select>
                      <button
                        className="button-quiet"
                        disabled={
                          busy ||
                          working !== null ||
                          !mergeTargets[speaker.id]
                        }
                        onClick={() => merge(speaker)}
                      >
                        {working === `merge-${speaker.id}`
                          ? lang === "zh" ? "合并中…" : "Merging…"
                          : lang === "zh" ? "合并" : "Merge"}
                      </button>
                    </div>
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </section>
    </div>
  );
}
