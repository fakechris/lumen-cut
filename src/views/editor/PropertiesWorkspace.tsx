import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef, useState } from "react";
import { allowProjectMedia } from "../../api";
import type { Lang } from "../../i18n";
import type {
  Doc,
  SpeakerEvidence,
  SpeakerInfo,
  SpeakerReidentifyPreview,
  SpeakerReidentifyProposal,
} from "../../types";

interface Props {
  busy: boolean;
  diarizeReady: boolean;
  doc: Doc;
  evidence: SpeakerEvidence;
  lang: Lang;
  preview: SpeakerReidentifyPreview | null;
  speakers: SpeakerInfo[];
  onApplyPreview: (proposals: SpeakerReidentifyProposal[]) => Promise<void>;
  onAssign: (paragraphId: number, speaker: string | null) => Promise<void>;
  onMerge: (from: string, into: string) => Promise<void>;
  onOpenSettings: () => void;
  onPreview: () => Promise<void>;
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
  diarizeReady,
  doc,
  evidence,
  lang,
  preview,
  speakers,
  onApplyPreview,
  onAssign,
  onMerge,
  onOpenSettings,
  onPreview,
  onRename,
  onSaveMeta,
}: Props) {
  const [title, setTitle] = useState(doc.meta.title);
  const [description, setDescription] = useState(doc.meta.description);
  const [language, setLanguage] = useState(doc.meta.language || "");
  const [names, setNames] = useState<Record<string, string>>({});
  const [mergeTargets, setMergeTargets] = useState<Record<string, string>>({});
  const [assignmentDrafts, setAssignmentDrafts] = useState<Record<number, string>>({});
  const [turnQuery, setTurnQuery] = useState("");
  const [mediaSource, setMediaSource] = useState<string | null>(null);
  const [mediaError, setMediaError] = useState<string | null>(null);
  const [working, setWorking] = useState<string | null>(null);
  const [selectedProposalIds, setSelectedProposalIds] = useState<Set<number>>(new Set());
  const [proposalLimit, setProposalLimit] = useState(80);
  const [turnLimit, setTurnLimit] = useState(100);
  const playerRef = useRef<HTMLMediaElement | null>(null);
  const playbackEndRef = useRef<number | null>(null);

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

  useEffect(() => {
    let cancelled = false;
    setMediaSource(null);
    setMediaError(null);
    void allowProjectMedia(doc.id)
      .then((path) => {
        if (!cancelled) setMediaSource(convertFileSrc(path));
      })
      .catch((error) => {
        if (!cancelled) setMediaError(String(error).replace(/^Error:\s*/i, ""));
      });
    return () => {
      cancelled = true;
      playerRef.current?.pause();
      playbackEndRef.current = null;
    };
  }, [doc.id]);

  useEffect(() => {
    setSelectedProposalIds(new Set());
    setProposalLimit(80);
  }, [preview]);

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
  const isAudio = /\.(aac|aif|aiff|flac|m4a|mp3|ogg|opus|wav)$/i.test(doc.media.path);
  const changedProposals = useMemo(
    () => preview?.proposals.filter(
      (proposal) => proposal.current !== proposal.proposed,
    ) ?? [],
    [preview],
  );
  const selectedProposals = changedProposals.filter((proposal) =>
    selectedProposalIds.has(proposal.paragraphId),
  );
  const filteredTurns = useMemo(() => {
    const query = turnQuery.trim().toLowerCase();
    return evidence.turns
      .filter((turn) => !query || turn.text.toLowerCase().includes(query)
        || (turn.speaker || "").toLowerCase().includes(query));
  }, [evidence.turns, turnQuery]);
  const visibleTurns = filteredTurns.slice(0, turnLimit);

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
    const confirmed = window.confirm(
      lang === "zh"
        ? `将把 ${speaker.paragraph_count} 个“${speaker.id}”段落合并到“${target}”。合并前状态可从“版本”恢复。是否继续？`
        : `Merge ${speaker.paragraph_count} “${speaker.id}” paragraphs into “${target}”? The prior state remains recoverable from Versions.`,
    );
    if (!confirmed) return;
    setWorking(`merge-${speaker.id}`);
    try {
      await onMerge(speaker.id, target);
    } finally {
      setWorking(null);
    }
  };

  const playTurn = (start: number, end: number) => {
    if (!playerRef.current) return;
    playerRef.current.currentTime = Math.max(0, start);
    playbackEndRef.current = end;
    void playerRef.current.play().catch(() => undefined);
  };

  const stopAtTurnEnd = () => {
    const player = playerRef.current;
    const end = playbackEndRef.current;
    if (!player || end === null || player.currentTime < end) return;
    player.pause();
    playbackEndRef.current = null;
  };

  const assign = async (paragraphId: number, current: string | null) => {
    const value = (assignmentDrafts[paragraphId] ?? current ?? "").trim();
    setWorking(`assign-${paragraphId}`);
    try {
      await onAssign(paragraphId, value || null);
      setAssignmentDrafts((drafts) => {
        const next = { ...drafts };
        delete next[paragraphId];
        return next;
      });
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
            disabled={busy || working !== null || doc.paragraphs.length === 0 || !diarizeReady}
            onClick={async () => {
              setWorking("preview");
              try {
                await onPreview();
              } finally {
                setWorking(null);
              }
            }}
          >
            {working === "preview"
              ? lang === "zh" ? "正在分析，不会覆盖现有标签…" : "Analyzing without changing labels…"
              : speakers.length
                ? lang === "zh" ? "预览重新识别" : "Preview identification"
                : lang === "zh" ? "分析说话人" : "Analyze speakers"}
          </button>
        </header>

        <p className="speaker-safety-note">
          {lang === "zh"
            ? "分析只生成提案，不会改动项目。长视频可能需要数分钟，任务在后台运行，期间仍可切换页面；检查逐段证据后再决定是否应用。"
            : "Analysis creates a proposal without changing the project. Long media can take several minutes in the background; you can keep navigating, then review turn evidence before applying."}
        </p>

        {!diarizeReady && (
          <div className="notice error-notice speaker-readiness" role="alert">
            <span>
              {lang === "zh"
                ? "说话人识别尚未准备好。请在设置中检查 pyannote 运行时、模型缓存和 Hugging Face 授权。"
                : "Speaker identification is not ready. Check the pyannote runtime, model cache, and Hugging Face access in Settings."}
            </span>
            <button className="button-quiet" onClick={onOpenSettings}>
              {lang === "zh" ? "打开设置" : "Open Settings"}
            </button>
          </div>
        )}

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

        {preview && (
          <section className="speaker-proposal" aria-labelledby="speaker-proposal-title">
            <header>
              <div>
                <p className="eyebrow">{lang === "zh" ? "非破坏性提案" : "Non-destructive proposal"}</p>
                <h3 id="speaker-proposal-title">
                  {lang === "zh"
                    ? `${preview.changed} 个段落标签将改变`
                    : `${preview.changed} paragraph label${preview.changed === 1 ? "" : "s"} will change`}
                </h3>
                <small>
                  {lang === "zh"
                    ? `${preview.segments} 个语音片段 · ${preview.unassigned} 个段落没有可靠匹配`
                    : `${preview.segments} voice segments · ${preview.unassigned} paragraphs without a reliable match`}
                </small>
              </div>
              <button
                className="button-primary"
                disabled={busy || working !== null || selectedProposals.length === 0}
                onClick={async () => {
                  setWorking("apply-preview");
                  try {
                    await onApplyPreview(selectedProposals);
                  } finally {
                    setWorking(null);
                  }
                }}
              >
                {working === "apply-preview"
                  ? lang === "zh" ? "应用中…" : "Applying…"
                  : preview.changed === 0
                    ? lang === "zh" ? "无需更改" : "No changes needed"
                    : selectedProposals.length === 0
                      ? lang === "zh" ? "请先勾选" : "Select changes"
                      : lang === "zh" ? `应用 ${selectedProposals.length} 项` : `Apply ${selectedProposals.length}`}
              </button>
            </header>
            {changedProposals.length > 0 && (
              <div className="speaker-proposal-actions">
                <button
                  className="button-quiet"
                  type="button"
                  onClick={() => setSelectedProposalIds(
                    selectedProposalIds.size === changedProposals.length
                      ? new Set()
                      : new Set(changedProposals.map((proposal) => proposal.paragraphId)),
                  )}
                >
                  {selectedProposalIds.size === changedProposals.length
                    ? lang === "zh" ? "取消全选" : "Clear selection"
                    : lang === "zh" ? `明确选择全部 ${changedProposals.length} 项` : `Explicitly select all ${changedProposals.length}`}
                </button>
                <small>
                  {lang === "zh"
                    ? `仅会应用已勾选的 ${selectedProposals.length} 项；未显示或未勾选的内容不会修改。`
                    : `Only ${selectedProposals.length} checked changes will be applied. Hidden or unchecked items stay unchanged.`}
                </small>
              </div>
            )}
            <div className="speaker-proposal-list">
              {changedProposals
                .slice(0, proposalLimit)
                .map((proposal) => (
                  <article key={proposal.paragraphId}>
                    <input
                      aria-label={lang === "zh" ? `选择段落 ${proposal.paragraphId}` : `Select paragraph ${proposal.paragraphId}`}
                      checked={selectedProposalIds.has(proposal.paragraphId)}
                      type="checkbox"
                      onChange={(event) => {
                        const next = new Set(selectedProposalIds);
                        if (event.target.checked) next.add(proposal.paragraphId);
                        else next.delete(proposal.paragraphId);
                        setSelectedProposalIds(next);
                      }}
                    />
                    <div>
                      <strong>{proposal.current || (lang === "zh" ? "未标记" : "Unlabelled")} → {proposal.proposed}</strong>
                      <small>{proposal.start.toFixed(1)}–{proposal.end.toFixed(1)}s · {proposal.cluster} · {Math.round(proposal.coverage * 100)}% {lang === "zh" ? "语音覆盖" : "speech coverage"} · {Math.round(proposal.margin * 100)}% {lang === "zh" ? "领先" : "margin"}</small>
                    </div>
                    <p>{proposal.text}</p>
                  </article>
                ))}
            </div>
            {proposalLimit < changedProposals.length && (
              <button
                className="button-quiet speaker-proposal-more"
                type="button"
                onClick={() => setProposalLimit((current) => current + 80)}
              >
                {lang === "zh"
                  ? `继续查看（剩余 ${changedProposals.length - proposalLimit} 项）`
                  : `Show more (${changedProposals.length - proposalLimit} remaining)`}
              </button>
            )}
          </section>
        )}

        <section className="speaker-evidence" aria-labelledby="speaker-evidence-title">
          <header>
            <div>
              <p className="eyebrow">{lang === "zh" ? "逐段证据" : "Turn evidence"}</p>
              <h3 id="speaker-evidence-title">
                {evidence.turns.length} {lang === "zh" ? "个说话片段" : "reviewable turns"}
              </h3>
            </div>
            <input
              aria-label={lang === "zh" ? "筛选说话片段" : "Filter speaker turns"}
              placeholder={lang === "zh" ? "搜索姓名或文本…" : "Search name or text…"}
              value={turnQuery}
              onChange={(event) => {
                setTurnQuery(event.target.value);
                setTurnLimit(100);
              }}
            />
          </header>

          <div className="speaker-media-player">
            {mediaSource ? (
              isAudio ? (
                <audio controls onTimeUpdate={stopAtTurnEnd} ref={(element) => { playerRef.current = element; }} src={mediaSource} />
              ) : (
                <video controls onTimeUpdate={stopAtTurnEnd} playsInline ref={(element) => { playerRef.current = element; }} src={mediaSource} />
              )
            ) : mediaError ? (
              <p role="alert">{lang === "zh" ? `无法打开媒体：${mediaError}` : `Could not open media: ${mediaError}`}</p>
            ) : (
              <p role="status">{lang === "zh" ? "正在打开媒体…" : "Opening media…"}</p>
            )}
          </div>

          <datalist id="speaker-name-options">
            {speakers.map((speaker) => <option key={speaker.id} value={speaker.id} />)}
          </datalist>
          <div className="speaker-turn-list">
            {visibleTurns.map((turn) => {
              const draft = assignmentDrafts[turn.paragraphId] ?? turn.speaker ?? "";
              const dirty = draft.trim() !== (turn.speaker || "");
              return (
                <article key={turn.paragraphId}>
                  <button className="speaker-play" onClick={() => playTurn(turn.start, turn.end)}>
                    ▶ {turn.start.toFixed(1)}–{turn.end.toFixed(1)}s
                  </button>
                  <p>{turn.text}</p>
                  <div>
                    <input
                      aria-label={lang === "zh" ? "说话人名称" : "Speaker name"}
                      list="speaker-name-options"
                      placeholder={lang === "zh" ? "未标记" : "Unlabelled"}
                      value={draft}
                      onChange={(event) => setAssignmentDrafts((current) => ({
                        ...current,
                        [turn.paragraphId]: event.target.value,
                      }))}
                    />
                    <button
                      className={dirty ? "button-primary" : "button-quiet"}
                      disabled={busy || working !== null || !dirty}
                      onClick={() => assign(turn.paragraphId, turn.speaker)}
                    >
                      {working === `assign-${turn.paragraphId}`
                        ? lang === "zh" ? "保存中…" : "Saving…"
                        : lang === "zh" ? "保存此段" : "Save turn"}
                    </button>
                  </div>
                </article>
              );
            })}
          </div>
          {visibleTurns.length < filteredTurns.length && (
            <button
              className="button-quiet speaker-proposal-more"
              type="button"
              onClick={() => setTurnLimit((current) => current + 100)}
            >
              {lang === "zh"
                ? `继续查看（剩余 ${filteredTurns.length - visibleTurns.length} 条）`
                : `Show more (${filteredTurns.length - visibleTurns.length} remaining)`}
            </button>
          )}
        </section>
      </section>
    </div>
  );
}
