import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  audit,
  asrStatus,
  brollAcceptSuggestion,
  brollAdd,
  brollList,
  brollPreviewCancel,
  brollPreviewStart,
  brollPreviewStatus,
  brollRemove,
  brollUpdate,
  branchCreate,
  branchSwitch,
  configShow,
  cutAuto,
  cutList,
  cutRestore,
  exportSubtitles,
  exportFinalCut,
  finishCheck,
  mergeSubtitles,
  pickBrollFile,
  projectUpdateMeta,
  projectReveal,
  projectShow,
  speakerAssign,
  speakerEvidence,
  speakerMerge,
  speakerReidentifyApply,
  speakerReidentifyCancel,
  speakerReidentifyStart,
  speakerReidentifyStatus,
  speakerRename,
  splitSubtitle,
  styleGet,
  styleSet,
  subtitleList,
  subtitleReplace,
  subtitleSet,
  subtitleVisibility,
  translationSet,
  taskStart,
  taskResume,
  taskStatus,
  transcriptionCancel,
  transcriptionStart,
  transcriptionStatus,
  versionCommit,
  versionList,
  versionRestore,
  videoExportCancel,
  videoExportStart,
  videoExportStatus,
} from "../api";
import type { CutSummary } from "../api";
import {
  AlertIcon,
  CheckIcon,
  PlayIcon,
  TranscriptIcon,
} from "../components/Icons";
import type { Lang } from "../i18n";
import type {
  Doc,
  AsrStatus,
  BrollOverview,
  BrollPlacementInput,
  BrollPreviewJobStatus,
  BrollSuggestion,
  FinishCheckItem,
  SubtitleRow,
  SubtitleStyle,
  ReportSummary,
  SpeakerEvidence,
  SpeakerAnalysisJobStatus,
  SpeakerReidentifyProposal,
  SpeakerInfo,
  SpeakerReidentifyPreview,
  TaskStatus,
  TranscriptionJobStatus,
  VersionHistory,
  VideoExportJobStatus,
} from "../types";
import { StyleWorkspace } from "./editor/StyleWorkspace";
import { EnhancementPanel } from "./editor/EnhancementPanel";
import { PropertiesWorkspace } from "./editor/PropertiesWorkspace";
import { TimelineWorkspace } from "./editor/TimelineWorkspace";
import { TranscriptEditor } from "./editor/TranscriptEditor";
import { TranslationWorkspace } from "./editor/TranslationWorkspace";
import { HistoryWorkspace } from "./editor/HistoryWorkspace";
import { BrollWorkspace } from "./editor/BrollWorkspace";
import { EditorMediaPreview } from "./editor/EditorMediaPreview";
import { EditorTimelineDock } from "./editor/EditorTimelineDock";

interface Props {
  lang: Lang;
  pid: string | null;
  onOpenSettings: () => void;
  onProjectTitleChange: (title: string) => void;
}

type Tab =
  | "setup"
  | "transcript"
  | "speakers"
  | "translate"
  | "style"
  | "properties"
  | "history"
  | "timeline"
  | "broll"
  | "review"
  | "export";
type Feedback = { tone: "success" | "error" | "info"; text: string };

const COPY = {
  zh: {
    setup: "准备",
    transcript: "转写稿",
    style: "样式",
    properties: "属性",
    history: "版本",
    timeline: "时间线",
    broll: "补充画面",
    review: "审查与修复",
    export: "导出",
    imported: "媒体已导入",
    startTitle: "准备开始转写",
    startDescription: "转写在本机运行，首次使用可能需要下载语音模型。",
    start: "开始转写",
    transcribing: "正在转写音频…",
    transcribeHint: "完成后会自动打开转写稿",
    cancelTranscription: "取消转写",
    cancellingTranscription: "正在停止…",
    cancelledTranscription: "转写已取消，原有内容没有被覆盖。",
    media: "媒体",
    duration: "时长",
    language: "语言",
    auto: "自动检测",
    words: "字词",
    paragraphs: "段落",
    speaker: "识别说话人",
    translating: "翻译任务已开始",
    translate: "翻译",
    targetLanguage: "目标语言",
    startTranslate: "开始翻译",
    agentHint: "lumen-cut 会在后台准备翻译环境，无需额外操作。",
    scanCuts: "扫描建议切口",
    runAudit: "运行审查",
    finishCheck: "完成前检查",
    noFindings: "没有发现问题。",
    suggestedCuts: "建议切口",
    restore: "恢复",
    exportSubtitles: "导出字幕",
    exportVideo: "导出带字幕视频",
    exportFcp: "导出 Final Cut 工程",
    exportHint: "先完成交付检查，再选择输出格式。文件会写入当前项目目录。",
    exportCheckTitle: "交付检查",
    exportUnchecked: "尚未检查当前版本",
    exportReady: "当前版本可以交付",
    exportBlocked: "存在阻止正式交付的问题",
    draftOverride: "仍要导出草稿（我了解检查项不会自动修复）",
    revealExports: "在 Finder 中打开项目目录",
    videoExportHint: "视频渲染可能需要数分钟，会在后台运行，编辑窗口不会失去响应。",
    loading: "正在打开项目…",
    noProject: "先从“项目”选择一个媒体文件。",
    emptyTranscript: "还没有转写内容。",
    taskStatus: "后台任务",
    pending: "待处理",
    done: "已完成",
    retry: "重试",
    asrNotReady: "本地转写尚未准备好。请先安装转写引擎并下载模型。",
    openAsrSettings: "前往设置",
  },
  en: {
    setup: "Setup",
    transcript: "Transcript",
    style: "Style",
    properties: "Properties",
    history: "Versions",
    timeline: "Timeline",
    broll: "B-roll",
    review: "Review & Fix",
    export: "Export",
    imported: "Media imported",
    startTitle: "Ready to transcribe",
    startDescription: "Transcription runs locally. The first run may download a speech model.",
    start: "Start transcription",
    transcribing: "Transcribing audio…",
    transcribeHint: "The transcript opens automatically when complete",
    cancelTranscription: "Cancel transcription",
    cancellingTranscription: "Stopping…",
    cancelledTranscription: "Transcription was cancelled. Existing content was not replaced.",
    media: "Media",
    duration: "Duration",
    language: "Language",
    auto: "Auto-detect",
    words: "words",
    paragraphs: "paragraphs",
    speaker: "Identify speakers",
    translating: "Translation started",
    translate: "Translate",
    targetLanguage: "Target language",
    startTranslate: "Start translation",
    agentHint: "lumen-cut prepares translation in the background; no extra setup is needed.",
    scanCuts: "Scan suggested cuts",
    runAudit: "Run review",
    finishCheck: "Pre-export check",
    noFindings: "No issues found.",
    suggestedCuts: "Suggested cuts",
    restore: "Restore",
    exportSubtitles: "Export subtitles",
    exportVideo: "Export subtitled video",
    exportFcp: "Export Final Cut project",
    exportHint: "Run the delivery check, then choose an output. Files are written to the current project folder.",
    exportCheckTitle: "Delivery check",
    exportUnchecked: "The current version has not been checked",
    exportReady: "The current version is ready to deliver",
    exportBlocked: "Issues are blocking a production delivery",
    draftOverride: "Export a draft anyway (I understand checks are not fixed automatically)",
    revealExports: "Open project folder in Finder",
    videoExportHint: "Video rendering can take several minutes. It runs in the background and the editor remains responsive.",
    loading: "Opening project…",
    noProject: "Choose a media file from Projects first.",
    emptyTranscript: "There is no transcript yet.",
    taskStatus: "Background tasks",
    pending: "pending",
    done: "done",
    retry: "Try again",
    asrNotReady: "Local transcription is not ready. Install the runtime and download the models first.",
    openAsrSettings: "Open Settings",
  },
} as const;

function mediaName(path: string) {
  return path.split(/[\\/]/).pop() || path;
}

function friendlyError(error: unknown, lang: Lang) {
  const raw = String(error).replace(/^Error:\s*/i, "");
  if (/sidecar script not found/i.test(raw)) {
    return lang === "zh"
      ? "找不到本地转写组件。请在“设置 → 高级诊断”运行环境检查。"
      : "The local transcription component was not found. Run the environment check in Settings → Advanced diagnostics.";
  }
  if (/ffmpeg/i.test(raw)) {
    return lang === "zh"
      ? "媒体处理失败。请确认文件可以播放，并检查 ffmpeg 环境。"
      : "Media processing failed. Check that the file plays and ffmpeg is available.";
  }
  if (/previous transcription was interrupted when lumen-cut closed/i.test(raw)) {
    return lang === "zh"
      ? "上次转写因 lumen-cut 关闭而中断。点击“重试”可以重新开始，现有项目不会被删除。"
      : "The previous transcription was interrupted when lumen-cut closed. Retry to start again; the existing project is preserved.";
  }
  if (/save the current project as a version before switching branches/i.test(raw)) {
    return lang === "zh"
      ? "当前项目有尚未保存的修改。请先在“版本”中保存当前版本，再切换分支。"
      : "This project has unsaved changes. Save the current version before switching branches.";
  }
  return raw;
}

function taskLabel(kind: string, lang: Lang) {
  const labels: Record<string, [string, string]> = {
    translate: ["翻译", "Translation"],
    align: ["字幕对齐", "Alignment"],
    polish: ["润色", "Polish"],
    segment: ["重新分段", "Segmentation"],
    repunct: ["标点修复", "Punctuation"],
    chapters: ["章节", "Chapters"],
    cleanup: ["口播清理", "Speech cleanup"],
    broll: ["B-roll 建议", "B-roll"],
  };
  return labels[kind]?.[lang === "zh" ? 0 : 1] || kind;
}

function transcriptionPhaseLabel(phase: TranscriptionJobStatus["phase"], lang: Lang) {
  const labels: Record<TranscriptionJobStatus["phase"], [string, string]> = {
    waiting: ["正在等待计算资源", "Waiting for compute capacity"],
    preparing: ["正在准备项目", "Preparing the project"],
    downloading: ["正在下载媒体", "Downloading media"],
    extracting: ["正在提取音频", "Extracting audio"],
    analyzing: ["正在分析媒体", "Analyzing media"],
    transcribing: ["正在分段识别语音", "Recognizing speech in chunks"],
    aligning: ["正在生成词级时码", "Generating word-level timing"],
    saving: ["正在整理转写稿", "Building the transcript"],
    exporting: ["正在生成字幕文件", "Creating subtitle files"],
    completed: ["转写完成", "Transcription complete"],
    cancelling: ["正在安全停止", "Stopping safely"],
    cancelled: ["已取消", "Cancelled"],
    failed: ["转写失败", "Transcription failed"],
  };
  return labels[phase][lang === "zh" ? 0 : 1];
}

export function TranscriptView({
  lang,
  pid,
  onOpenSettings,
  onProjectTitleChange,
}: Props) {
  const c = COPY[lang];
  const [doc, setDoc] = useState<Doc | null>(null);
  const [activeTab, setActiveTab] = useState<Tab>("setup");
  const [operation, setOperation] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<Feedback | null>(null);
  const [auditReport, setAuditReport] = useState<ReportSummary | null>(null);
  const [finishItems, setFinishItems] = useState<FinishCheckItem[] | null>(null);
  const [allowDraftExport, setAllowDraftExport] = useState(false);
  const [taskState, setTaskState] = useState<TaskStatus | null>(null);
  const [cuts, setCuts] = useState<CutSummary[]>([]);
  const [subtitleRows, setSubtitleRows] = useState<SubtitleRow[]>([]);
  const [subtitleStyle, setSubtitleStyle] = useState<SubtitleStyle | null>(null);
  const [speakers, setSpeakers] = useState<SpeakerInfo[]>([]);
  const [speakerEvidenceState, setSpeakerEvidenceState] = useState<SpeakerEvidence>({
    speakers: [],
    turns: [],
    identified: false,
    unlabelled: 0,
  });
  const [speakerPreview, setSpeakerPreview] = useState<SpeakerReidentifyPreview | null>(null);
  const [speakerAnalysisJob, setSpeakerAnalysisJob] =
    useState<SpeakerAnalysisJobStatus | null>(null);
  const [transcriptionJob, setTranscriptionJob] =
    useState<TranscriptionJobStatus | null>(null);
  const [transcriptionFailure, setTranscriptionFailure] = useState<string | null>(null);
  const [videoExportJob, setVideoExportJob] = useState<VideoExportJobStatus | null>(null);
  const [videoExportMode, setVideoExportMode] = useState<VideoExportJobStatus["mode"]>("fast");
  const [agentConfigured, setAgentConfigured] = useState(false);
  const [asrReadiness, setAsrReadiness] = useState<AsrStatus | null>(null);
  const [versionHistory, setVersionHistory] = useState<VersionHistory | null>(null);
  const [brollOverview, setBrollOverview] = useState<BrollOverview>({
    suggestions: [],
    accepted: [],
    errors: [],
  });
  const [brollPreviewJob, setBrollPreviewJob] = useState<BrollPreviewJobStatus | null>(null);
  const [brollPreviewPaths, setBrollPreviewPaths] = useState<string[]>([]);
  const workbenchPlayerRef = useRef<HTMLMediaElement | null>(null);
  const [workbenchTime, setWorkbenchTime] = useState(0);
  const [workbenchPlaying, setWorkbenchPlaying] = useState(false);
  const [previewTranslationLanguage, setPreviewTranslationLanguage] = useState<string | null>(null);
  const previousPending = useRef(0);
  const activeProject = useRef(pid);
  activeProject.current = pid;
  const previewRows = useMemo(() => {
    if (!doc || activeTab !== "translate" || !previewTranslationLanguage) {
      return subtitleRows;
    }
    const track = doc.translations[previewTranslationLanguage];
    if (!track) return subtitleRows;
    return subtitleRows.map((row) => ({
      ...row,
      text: track[row.id]?.text || row.text,
    }));
  }, [activeTab, doc, previewTranslationLanguage, subtitleRows]);
  const { wordsByCue, nextCueById } = useMemo(() => {
    const words: Record<string, string[]> = {};
    const nextCues: Record<string, string> = {};
    for (const paragraph of doc?.paragraphs ?? []) {
      paragraph.sentences.forEach((sentence, index) => {
        words[sentence.id] = sentence.words.map((word) => word.text);
        const next = paragraph.sentences[index + 1];
        if (next) nextCues[sentence.id] = next.id;
      });
    }
    return { wordsByCue: words, nextCueById: nextCues };
  }, [doc?.paragraphs]);

  const seekWorkbench = useCallback((seconds: number, autoplay = false) => {
    if (!doc) return;
    const next = Math.max(0, Math.min(seconds, doc.media.durationSeconds));
    const player = workbenchPlayerRef.current;
    setWorkbenchTime(next);
    if (!player) return;
    player.currentTime = next;
    if (autoplay) {
      void player.play().catch(() => setWorkbenchPlaying(false));
    }
  }, [doc]);

  const toggleWorkbenchPlayback = useCallback(() => {
    const player = workbenchPlayerRef.current;
    if (!player) return;
    if (player.paused) {
      void player.play().catch(() => setWorkbenchPlaying(false));
    } else {
      player.pause();
    }
  }, []);

  const reload = async (projectId: string, resetTab = true) => {
    setFinishItems(null);
    setAllowDraftExport(false);
    const [nextDoc, nextRows, nextStyle, nextEvidence, nextBroll] = await Promise.all([
      projectShow(projectId),
      subtitleList(projectId),
      styleGet(projectId),
      speakerEvidence(projectId),
      brollList(projectId).catch((error) => {
        setFeedback({
          tone: "error",
          text: lang === "zh"
            ? `B-roll 数据无法加载，转写稿仍可继续编辑：${friendlyError(error, lang)}`
            : `B-roll data could not be loaded; transcript editing is still available: ${friendlyError(error, lang)}`,
        });
        return { suggestions: [], accepted: [], errors: [friendlyError(error, lang)] };
      }),
    ]);
    if (activeProject.current !== projectId) return;
    setDoc(nextDoc);
    setSubtitleRows(nextRows);
    setSubtitleStyle(nextStyle);
    setSpeakers(nextEvidence.speakers);
    setSpeakerEvidenceState(nextEvidence);
    setBrollOverview(nextBroll);
    if (resetTab) {
      setActiveTab(nextDoc.paragraphs.length > 0 ? "transcript" : "setup");
    }
    try {
      const nextCuts = await cutList(projectId);
      if (activeProject.current === projectId) setCuts(nextCuts);
    } catch {
      setCuts([]);
    }
  };

  useEffect(() => {
    setDoc(null);
    setFeedback(null);
    setWorkbenchTime(0);
    setWorkbenchPlaying(false);
    workbenchPlayerRef.current?.pause();
    setAuditReport(null);
    setFinishItems(null);
    setAllowDraftExport(false);
    setTaskState(null);
    setOperation(null);
    setTranscriptionJob(null);
    setTranscriptionFailure(null);
    setVideoExportJob(null);
    setVersionHistory(null);
    setSpeakerEvidenceState({ speakers: [], turns: [], identified: false, unlabelled: 0 });
    setSpeakerPreview(null);
    setSpeakerAnalysisJob(null);
    setBrollOverview({ suggestions: [], accepted: [], errors: [] });
    setBrollPreviewJob(null);
    setBrollPreviewPaths([]);
    if (!pid) return;
    void Promise.all([
      reload(pid),
      taskStatus(pid).then(async (status) => {
        if (activeProject.current !== pid) return;
        setTaskState(status);
        if (status.kinds.some(
          (task) => task.pending > 0 && task.state !== "paused" && task.state !== "failed",
        )) {
          const recovery = await taskResume(pid);
          if (activeProject.current !== pid) return;
          if (recovery.resumed > 0 || recovery.recoveredSubmissions > 0) {
            setFeedback({
              tone: "info",
              text: lang === "zh"
                ? `已恢复 ${recovery.resumed} 个未完成任务，其中 ${recovery.recoveredSubmissions} 个模型结果无需重算。`
                : `Resumed ${recovery.resumed} unfinished tasks; ${recovery.recoveredSubmissions} model results did not need recomputation.`,
            });
          }
        }
      }),
      configShow().then((config) =>
        setAgentConfigured(
          Boolean(config.llmEndpoint.trim() && config.llmModel.trim()),
        ),
      ),
      asrStatus().then(setAsrReadiness),
      versionList(pid).then(setVersionHistory),
      transcriptionStatus(pid)
        .then((status) => {
          if (status.state === "running" || status.state === "cancelling") {
            setTranscriptionJob(status);
            setOperation("transcribe");
          } else if (status.state === "failed") {
            const failure = friendlyError(status.error || "Transcription failed", lang);
            setTranscriptionFailure(failure);
            setFeedback({ tone: "error", text: failure });
          }
        })
        .catch(() => undefined),
      speakerReidentifyStatus(pid)
        .then((status) => {
          if (status.state === "running" || status.state === "cancelling") {
            setSpeakerAnalysisJob(status);
            setOperation("speakers-preview");
          } else if (status.state === "completed" && status.preview) {
            setSpeakerPreview(status.preview);
            setFeedback({
              tone: "info",
              text: lang === "zh"
                ? `已恢复上次说话人分析提案：${status.preview.changed} 个段落标签待确认。`
                : `Restored the previous speaker proposal: ${status.preview.changed} paragraph labels await review.`,
            });
          } else if (status.state === "failed") {
            setFeedback({
              tone: "error",
              text: friendlyError(status.error || "Speaker analysis failed", lang),
            });
          }
        })
        .catch(() => undefined),
      videoExportStatus(pid)
        .then((status) => {
          if (status.state === "running" || status.state === "cancelling") {
            setVideoExportJob(status);
          } else if (status.state === "completed" && status.path) {
            setVideoExportJob(status);
            setFeedback({
              tone: "success",
              text: lang === "zh"
                ? `已恢复上次视频导出记录：${status.path}`
                : `Restored the previous video export: ${status.path}`,
            });
          } else if (status.state === "failed") {
            setFeedback({
              tone: "error",
              text: friendlyError(status.error || "Video export failed", lang),
            });
          }
        })
        .catch(() => undefined),
      brollPreviewStatus(pid)
        .then((status) => {
          if (status.state === "running" || status.state === "cancelling") {
            setBrollPreviewJob(status);
          } else if (status.state === "completed") {
            setBrollPreviewJob(status);
            setBrollPreviewPaths(status.paths);
          }
        })
        .catch(() => undefined),
    ]).catch((error) => {
        setFeedback({ tone: "error", text: friendlyError(error, lang) });
      });
  }, [pid]);

  useEffect(() => {
    if (
      !pid
      || !taskState
      || taskState.pending < 1
      || !taskState.kinds.some((task) => task.pending > 0 && task.state !== "paused" && task.state !== "failed")
    ) return;
    let disposed = false;
    let timer: number | undefined;
    let keepPolling = true;
    let completedBatches = taskState.done;
    const watchedTasks = new Set(
      taskState.kinds
        .filter((task) => task.pending > 0)
        .map((task) => `${task.kind}:${task.lang || ""}`),
    );
    const poll = async () => {
      try {
        const status = await taskStatus(pid);
        if (disposed || activeProject.current !== pid) return;
        setTaskState(status);
        if (status.done > completedBatches) {
          completedBatches = status.done;
          await reload(pid, false);
          if (disposed || activeProject.current !== pid) return;
        }
        const running = status.kinds.some(
          (task) => task.pending > 0 && task.state !== "paused" && task.state !== "failed",
        );
        if (running) {
          await taskResume(pid);
        } else if (status.pending === 0) {
          keepPolling = false;
          await reload(pid, false);
          if (disposed || activeProject.current !== pid) return;
          const translate = status.kinds.find(
            (task) => task.kind === "translate" && watchedTasks.has(`${task.kind}:${task.lang || ""}`),
          );
          if (translate) {
            setFeedback({
              tone: translate.failed > 0 || translate.state === "failed" ? "error" : "success",
              text: translate.failed > 0 || translate.state === "failed"
                ? lang === "zh"
                  ? `翻译已停止：${translate.failed} 个批次失败，已完成的结果已经保存。`
                  : `Translation stopped: ${translate.failed} batches failed. Completed results were saved.`
                : lang === "zh"
                  ? "翻译完成，结果已保存。"
                  : "Translation complete. Results were saved.",
            });
          }
        } else {
          keepPolling = false;
          const stopped = status.kinds.find(
            (task) => watchedTasks.has(`${task.kind}:${task.lang || ""}`)
              && (task.state === "paused" || task.state === "failed"),
          );
          if (stopped) {
            setFeedback({
              tone: "error",
              text: lang === "zh"
                ? `${taskLabel(stopped.kind, lang)}已暂停：${stopped.lastError || "请在对应页面确认后继续。"}`
                : `${taskLabel(stopped.kind, lang)} paused: ${stopped.lastError || "Review it before resuming."}`,
            });
          }
        }
      } catch (error) {
        if (keepPolling && !disposed && activeProject.current === pid) {
          setFeedback({
            tone: "error",
            text: lang === "zh"
              ? `暂时无法读取后台进度，将自动重试：${friendlyError(error, lang)}`
              : `Could not read background progress; retrying automatically: ${friendlyError(error, lang)}`,
          });
        }
      } finally {
        if (!disposed && activeProject.current === pid) {
          timer = window.setTimeout(poll, 2500);
        }
      }
    };
    timer = window.setTimeout(poll, 2500);
    return () => {
      disposed = true;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [pid, taskState?.pending, taskState?.kinds.map((task) => task.state).join("|")]);

  useEffect(() => {
    if (
      !pid ||
      !transcriptionJob ||
      !["running", "cancelling"].includes(transcriptionJob.state)
    ) {
      return;
    }
    let disposed = false;
    let timer: number | undefined;
    const poll = async () => {
      try {
        const status = await transcriptionStatus(pid);
        if (disposed) return;
        if (status.state === "completed") {
          await reload(pid);
          if (disposed) return;
          setFeedback({
            tone: "success",
            text:
              lang === "zh"
                ? "转写完成，可以开始审阅。"
                : "Transcription is ready to review.",
          });
          setOperation(null);
          setTranscriptionJob(null);
          setTranscriptionFailure(null);
          return;
        }
        if (status.state === "cancelled") {
          setFeedback({ tone: "info", text: c.cancelledTranscription });
          setOperation(null);
          setTranscriptionJob(null);
          setTranscriptionFailure(null);
          return;
        }
        if (status.state === "failed") {
          const failure = friendlyError(status.error || "Transcription failed", lang);
          setFeedback({
            tone: "error",
            text: failure,
          });
          setOperation(null);
          setTranscriptionJob(null);
          setTranscriptionFailure(failure);
          return;
        }
        setTranscriptionJob(status);
        timer = window.setTimeout(poll, 500);
      } catch (error) {
        if (disposed) return;
        const failure = friendlyError(error, lang);
        setFeedback({ tone: "error", text: failure });
        setOperation(null);
        setTranscriptionJob(null);
        setTranscriptionFailure(failure);
      }
    };
    timer = window.setTimeout(poll, 350);
    return () => {
      disposed = true;
      if (timer) window.clearTimeout(timer);
    };
  }, [pid, transcriptionJob?.state, lang]);

  useEffect(() => {
    if (
      !pid ||
      !videoExportJob ||
      !["running", "cancelling"].includes(videoExportJob.state)
    ) {
      return;
    }
    let disposed = false;
    let timer: number | undefined;
    const poll = async () => {
      try {
        const status = await videoExportStatus(pid);
        if (disposed) return;
        if (status.state === "completed") {
          setVideoExportJob(status);
          setFeedback({
            tone: "success",
            text: lang === "zh"
              ? `视频已导出：${status.path}`
              : `Video exported: ${status.path}`,
          });
          return;
        }
        if (status.state === "cancelled") {
          setVideoExportJob(status);
          setFeedback({
            tone: "info",
            text: lang === "zh" ? "视频导出已取消，原有导出文件未被覆盖。" : "Video export cancelled; the previous export was preserved.",
          });
          return;
        }
        if (status.state === "failed") {
          setVideoExportJob(status);
          setFeedback({
            tone: "error",
            text: friendlyError(status.error || "Video export failed", lang),
          });
          return;
        }
        setVideoExportJob(status);
        timer = window.setTimeout(poll, 500);
      } catch (error) {
        if (disposed) return;
        setFeedback({ tone: "error", text: friendlyError(error, lang) });
        setVideoExportJob(null);
      }
    };
    timer = window.setTimeout(poll, 350);
    return () => {
      disposed = true;
      if (timer) window.clearTimeout(timer);
    };
  }, [pid, videoExportJob?.state, lang]);

  useEffect(() => {
    if (
      !pid ||
      !brollPreviewJob ||
      !["running", "cancelling"].includes(brollPreviewJob.state)
    ) {
      return;
    }
    let disposed = false;
    let timer: number | undefined;
    const poll = async () => {
      try {
        const status = await brollPreviewStatus(pid);
        if (disposed) return;
        if (status.state === "completed") {
          setBrollPreviewJob(status);
          setBrollPreviewPaths(status.paths);
          setFeedback({
            tone: "success",
            text: lang === "zh" ? "B-roll 画面预览已生成。" : "B-roll frame previews are ready.",
          });
          return;
        }
        if (status.state === "cancelled") {
          setBrollPreviewJob(status);
          setFeedback({ tone: "info", text: lang === "zh" ? "B-roll 预览已取消。" : "B-roll preview cancelled." });
          return;
        }
        if (status.state === "failed") {
          setBrollPreviewJob(status);
          setFeedback({ tone: "error", text: friendlyError(status.error || "B-roll preview failed", lang) });
          return;
        }
        setBrollPreviewJob(status);
        timer = window.setTimeout(poll, 500);
      } catch (error) {
        if (!disposed) {
          setFeedback({ tone: "error", text: friendlyError(error, lang) });
          setBrollPreviewJob(null);
        }
      }
    };
    timer = window.setTimeout(poll, 350);
    return () => {
      disposed = true;
      if (timer) window.clearTimeout(timer);
    };
  }, [pid, brollPreviewJob?.state, lang]);

  useEffect(() => {
    if (
      !pid ||
      !speakerAnalysisJob ||
      !["running", "cancelling"].includes(speakerAnalysisJob.state)
    ) {
      return;
    }
    let disposed = false;
    let timer: number | undefined;
    const poll = async () => {
      try {
        const status = await speakerReidentifyStatus(pid);
        if (disposed) return;
        if (status.state === "completed") {
          if (!status.preview) throw new Error("Speaker analysis completed without a preview");
          setSpeakerPreview(status.preview);
          setActiveTab("speakers");
          setFeedback({
            tone: "info",
            text: lang === "zh"
              ? `分析完成：${status.preview.changed} 个段落标签可能改变。项目尚未被修改。`
              : `Analysis complete: ${status.preview.changed} paragraph labels may change. The project is unchanged.`,
          });
          setOperation(null);
          setSpeakerAnalysisJob(null);
          return;
        }
        if (status.state === "cancelled") {
          setFeedback({
            tone: "info",
            text: lang === "zh" ? "说话人分析已取消，项目没有被修改。" : "Speaker analysis was cancelled. The project was not changed.",
          });
          setOperation(null);
          setSpeakerAnalysisJob(null);
          return;
        }
        if (status.state === "failed") {
          setFeedback({
            tone: "error",
            text: friendlyError(status.error || "Speaker analysis failed", lang),
          });
          setOperation(null);
          setSpeakerAnalysisJob(null);
          return;
        }
        setSpeakerAnalysisJob(status);
        timer = window.setTimeout(poll, 500);
      } catch (error) {
        if (disposed) return;
        setFeedback({ tone: "error", text: friendlyError(error, lang) });
        setOperation(null);
        setSpeakerAnalysisJob(null);
      }
    };
    timer = window.setTimeout(poll, 350);
    return () => {
      disposed = true;
      if (timer) window.clearTimeout(timer);
    };
  }, [pid, speakerAnalysisJob?.state, lang]);

  useEffect(() => {
    const pending = taskState?.pending ?? 0;
    if (pid && previousPending.current > 0 && pending === 0) {
      void reload(pid, false);
    }
    previousPending.current = pending;
  }, [pid, taskState?.pending]);

  useEffect(() => {
    if (!feedback || feedback.tone === "error") return;
    const timer = window.setTimeout(() => setFeedback(null), 4500);
    return () => window.clearTimeout(timer);
  }, [feedback]);

  if (!pid) {
    return (
      <section className="editor-empty">
        <TranscriptIcon />
        <p>{c.noProject}</p>
      </section>
    );
  }

  if (!doc) {
    return (
      <section className="editor-loading" role="status">
        <span className="spinner" aria-hidden="true" />
        <p>{c.loading}</p>
        {feedback?.tone === "error" && (
          <div className="notice error-notice">
            <span>{feedback.text}</span>
          </div>
        )}
      </section>
    );
  }

  const hasTranscript = doc.paragraphs.length > 0;
  const failedFinishItems = finishItems?.filter((item) => !item.pass) ?? [];
  const exportReady = finishItems !== null && failedFinishItems.length === 0;
  const exportAllowed = exportReady || allowDraftExport;
  const isVideoExporting = videoExportJob !== null
    && ["running", "cancelling"].includes(videoExportJob.state);
  const failedTasks = taskState?.kinds.reduce((sum, task) => sum + task.failed, 0) ?? 0;
  const stoppedTasks = taskState?.kinds.filter(
    (task) => task.state === "paused" || task.state === "failed",
  ).length ?? 0;
  const perform = async (name: string, action: () => Promise<void>) => {
    setOperation(name);
    setFeedback(null);
    try {
      await action();
    } catch (error) {
      setFeedback({ tone: "error", text: friendlyError(error, lang) });
    } finally {
      setOperation(null);
    }
  };

  const performRecoverable = async <T,>(name: string, action: () => Promise<T>): Promise<T> => {
    setOperation(name);
    setFeedback(null);
    try {
      return await action();
    } catch (error) {
      setFeedback({ tone: "error", text: friendlyError(error, lang) });
      throw error;
    } finally {
      setOperation(null);
    }
  };

  const startTranscription = async () => {
    setOperation("transcribe");
    setFeedback(null);
    setTranscriptionFailure(null);
    try {
      const readiness = await asrStatus();
      setAsrReadiness(readiness);
      if (!readiness.ready) {
        setFeedback({ tone: "error", text: c.asrNotReady });
        setOperation(null);
        return;
      }
      setTranscriptionJob(await transcriptionStart(
        doc.media.path,
        doc.meta.language ?? null,
        doc.meta.title,
        null,
        pid,
      ));
    } catch (error) {
      const failure = friendlyError(error, lang);
      setFeedback({ tone: "error", text: failure });
      setOperation(null);
      setTranscriptionJob(null);
      setTranscriptionFailure(failure);
    }
  };

  const cancelTranscription = async () => {
    try {
      setTranscriptionJob(await transcriptionCancel(pid));
    } catch (error) {
      setFeedback({ tone: "error", text: friendlyError(error, lang) });
    }
  };

  const startTranslation = (language: string) =>
    perform("translate", async () => {
      const result = await taskStart("translate", pid, language);
      setTaskState(await taskStatus(pid));
      setFeedback({
        tone: "info",
        text: `${c.translating} · ${result.pending} ${c.pending}`,
      });
    });

  const startEnhancement = (kind: string, language: string | null) =>
    perform(`enhance-${kind}`, async () => {
      const result = await taskStart(kind, pid, language);
      setTaskState(await taskStatus(pid));
      setFeedback({
        tone: "info",
        text:
          result.pending > 0
            ? lang === "zh"
              ? `${taskLabel(kind, lang)}已开始，可继续编辑其他内容。`
              : `${taskLabel(kind, lang)} started. You can keep editing.`
            : lang === "zh"
              ? `${taskLabel(kind, lang)}没有发现需要处理的内容。`
              : `${taskLabel(kind, lang)} found nothing to process.`,
      });
    });

  const saveSubtitle = async (id: string, text: string) => {
    await perform(`subtitle-${id}`, async () => {
      const changed = await subtitleSet(pid, id, text);
      if (!changed) throw new Error(`subtitle ${id} was not found`);
      await reload(pid, false);
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "这句转写已保存。" : "This transcript line was saved.",
      });
    });
  };

  const saveTranslation = async (language: string, id: string, text: string) => {
    await perform(`translation-${id}`, async () => {
      const changed = await translationSet(pid, language, id, text);
      if (!changed) throw new Error(`subtitle ${id} was not found`);
      setDoc((current) => current ? {
        ...current,
        translations: {
          ...current.translations,
          [language]: {
            ...(current.translations[language] || {}),
            [id]: { text },
          },
        },
      } : current);
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "这句译文已保存。" : "This translation was saved.",
      });
    });
  };

  const replaceSubtitles = async (query: string, replacement: string) => {
    setOperation("replace");
    setFeedback(null);
    try {
      const count = await subtitleReplace(pid, query, replacement);
      await reload(pid, false);
      setFeedback({
        tone: "success",
        text: lang === "zh" ? `已替换 ${count} 处。` : `Replaced ${count} occurrence${count === 1 ? "" : "s"}.`,
      });
      return count;
    } catch (error) {
      setFeedback({ tone: "error", text: friendlyError(error, lang) });
      throw error;
    } finally {
      setOperation(null);
    }
  };

  const changeSubtitleVisibility = async (id: string, hidden: boolean) => {
    await perform(`visibility-${id}`, async () => {
      await subtitleVisibility(pid, id, hidden);
      await reload(pid, false);
    });
  };

  const splitSubtitleLine = async (id: string, at: number) => {
    await perform(`split-${id}`, async () => {
      const changed = await splitSubtitle(pid, id, at);
      if (!changed) {
        throw new Error(
          lang === "zh"
            ? "无法在这个位置拆分，请刷新后重试。"
            : "This cue could not be split at that position. Refresh and try again.",
        );
      }
      await reload(pid, false);
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "已拆分为两句字幕。" : "Split into two cues.",
      });
    });
  };

  const mergeSubtitleLines = async (id1: string, id2: string) => {
    await perform(`merge-${id1}`, async () => {
      const changed = await mergeSubtitles(pid, id1, id2);
      if (!changed) {
        throw new Error(
          lang === "zh"
            ? "这两句无法合并，请刷新后重试。"
            : "These cues could not be merged. Refresh and try again.",
        );
      }
      await reload(pid, false);
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "已与下一句合并。" : "Merged with the next cue.",
      });
    });
  };

  const saveStyle = async (style: SubtitleStyle) => {
    setOperation("style");
    setFeedback(null);
    try {
      await styleSet(pid, style);
      setSubtitleStyle(await styleGet(pid));
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "字幕样式已保存。" : "Subtitle style saved.",
      });
    } catch (error) {
      setFeedback({ tone: "error", text: friendlyError(error, lang) });
      throw error;
    } finally {
      setOperation(null);
    }
  };

  const previewSpeakers = async () => {
    setOperation("speakers-preview");
    setFeedback(null);
    setSpeakerPreview(null);
    setActiveTab("speakers");
    try {
      setSpeakerAnalysisJob(await speakerReidentifyStart(pid));
    } catch (error) {
      setFeedback({ tone: "error", text: friendlyError(error, lang) });
      setOperation(null);
      throw error;
    }
  };

  const cancelSpeakerAnalysis = async () => {
    try {
      setSpeakerAnalysisJob(await speakerReidentifyCancel(pid));
    } catch (error) {
      setFeedback({ tone: "error", text: friendlyError(error, lang) });
    }
  };

  const applySpeakerPreview = async (proposals: SpeakerReidentifyProposal[]) => {
    if (!speakerPreview || proposals.length === 0) return;
    await performRecoverable("speakers-apply", async () => {
      const changed = await speakerReidentifyApply(pid, proposals);
      setSpeakerPreview(null);
      await Promise.all([reload(pid, false), refreshVersionHistory()]);
      setFeedback({
        tone: "success",
        text: lang === "zh"
          ? `已应用 ${changed} 个说话人标签；应用前状态可在“版本”中恢复。`
          : `Applied ${changed} speaker labels. The prior state is recoverable from Versions.`,
      });
    });
  };

  const assignSpeaker = async (paragraphId: number, speaker: string | null) => {
    await performRecoverable(`speaker-assign-${paragraphId}`, async () => {
      await speakerAssign(pid, paragraphId, speaker);
      await reload(pid, false);
      setSpeakerPreview(null);
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "此段说话人已保存。" : "Saved the speaker for this turn.",
      });
    });
  };

  const saveProjectMeta = async (
    title: string,
    description: string,
    language: string | null,
  ) => {
    setOperation("project-meta");
    setFeedback(null);
    try {
      await projectUpdateMeta(pid, title, description, language);
      await reload(pid, false);
      onProjectTitleChange(title);
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "项目属性已保存。" : "Project details saved.",
      });
    } catch (error) {
      setFeedback({ tone: "error", text: friendlyError(error, lang) });
      throw error;
    } finally {
      setOperation(null);
    }
  };

  const refreshVersionHistory = async () => {
    setVersionHistory(await versionList(pid));
  };

  const commitVersion = async (name: string, note: string) => {
    await performRecoverable("version", async () => {
      await versionCommit(pid, name, note);
      await refreshVersionHistory();
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "当前项目已保存为可恢复版本。" : "Saved a recoverable project version.",
      });
    });
  };

  const restoreVersion = async (id: string) => {
    await performRecoverable("version", async () => {
      await versionRestore(pid, id);
      await Promise.all([reload(pid, false), refreshVersionHistory()]);
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "已恢复所选版本。" : "Restored the selected version.",
      });
    });
  };

  const createBranch = async (name: string) => {
    await performRecoverable("version", async () => {
      await branchCreate(pid, name);
      await refreshVersionHistory();
      setFeedback({
        tone: "success",
        text: lang === "zh" ? `已创建并切换到分支“${name}”。` : `Created and switched to “${name}”.`,
      });
    });
  };

  const switchBranch = async (id: string) => {
    await performRecoverable("version", async () => {
      await branchSwitch(pid, id);
      await Promise.all([reload(pid, false), refreshVersionHistory()]);
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "已切换分支。" : "Switched branch.",
      });
    });
  };

  const refreshBroll = async () => {
    setBrollOverview(await brollList(pid));
  };

  const pickBrollAsset = () => performRecoverable("broll-pick", pickBrollFile);

  const acceptBrollSuggestion = async (suggestion: BrollSuggestion) => {
    const file = await pickBrollAsset();
    if (!file) return false;
    await performRecoverable("broll-add", async () => {
      await brollAcceptSuggestion(pid, suggestion, file);
      await refreshBroll();
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "素材已按建议时段加入 B-roll 轨道。" : "Added the asset at the suggested B-roll range.",
      });
    });
    return true;
  };

  const addBroll = async (input: BrollPlacementInput) => {
    await performRecoverable("broll-add", async () => {
      await brollAdd(pid, input);
      await refreshBroll();
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "素材已加入 B-roll 轨道。" : "Added the asset to the B-roll track.",
      });
    });
  };

  const updateBroll = async (id: string, input: BrollPlacementInput) => {
    await performRecoverable("broll-update", async () => {
      await brollUpdate(pid, id, input);
      await refreshBroll();
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "B-roll 调整已保存。" : "Saved the B-roll changes.",
      });
    });
  };

  const removeBroll = async (id: string) => {
    await performRecoverable("broll-remove", async () => {
      if (!await brollRemove(pid, id)) throw new Error(`B-roll ${id} was not found`);
      await refreshBroll();
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "已从成片中移除这段素材。" : "Removed the asset from the edit.",
      });
    });
  };

  const previewBroll = async () => {
    try {
      setFeedback(null);
      setBrollPreviewPaths([]);
      setBrollPreviewJob(await brollPreviewStart(pid));
    } catch (error) {
      setFeedback({ tone: "error", text: friendlyError(error, lang) });
      throw error;
    }
  };

  const cancelBrollPreview = async () => {
    try {
      setBrollPreviewJob(await brollPreviewCancel(pid));
    } catch (error) {
      setFeedback({ tone: "error", text: friendlyError(error, lang) });
    }
  };

  const renameSpeaker = async (from: string, to: string) => {
    setOperation(`speaker-rename-${from}`);
    setFeedback(null);
    try {
      const changed = await speakerRename(pid, from, to);
      if (changed < 1) throw new Error(`speaker ${from} was not found`);
      await reload(pid, false);
      setSpeakerPreview(null);
      setFeedback({
        tone: "success",
        text:
          lang === "zh"
            ? `已将“${from}”改为“${to}”。`
            : `Renamed “${from}” to “${to}”.`,
      });
    } catch (error) {
      setFeedback({ tone: "error", text: friendlyError(error, lang) });
      throw error;
    } finally {
      setOperation(null);
    }
  };

  const mergeSpeaker = async (from: string, into: string) => {
    setOperation(`speaker-merge-${from}`);
    setFeedback(null);
    try {
      const changed = await speakerMerge(pid, from, into);
      if (changed < 1) throw new Error(`speaker ${from} was not found`);
      await reload(pid, false);
      setSpeakerPreview(null);
      setFeedback({
        tone: "success",
        text:
          lang === "zh"
            ? `已将“${from}”合并到“${into}”。`
            : `Merged “${from}” into “${into}”.`,
      });
    } catch (error) {
      setFeedback({ tone: "error", text: friendlyError(error, lang) });
      throw error;
    } finally {
      setOperation(null);
    }
  };

  const scanCuts = () =>
    perform("cuts", async () => {
      const added = await cutAuto(pid);
      setCuts(await cutList(pid));
      setFeedback({
        tone: "success",
        text: lang === "zh" ? `新增 ${added} 个建议切口。` : `Added ${added} suggested cuts.`,
      });
    });

  const runReview = () =>
    perform("audit", async () => {
      setAuditReport(await audit(pid));
    });

  const runFinishCheck = () =>
    perform("finish", async () => {
      const items = await finishCheck(pid);
      setFinishItems(items);
      setAllowDraftExport(false);
    });

  const restoreCut = (cutId: string) =>
    perform(`restore-${cutId}`, async () => {
      await cutRestore(pid, cutId);
      setCuts(await cutList(pid));
    });

  const runSubtitleExport = () =>
    perform("export-subtitles", async () => {
      const paths = await exportSubtitles(pid);
      setFeedback({
        tone: "success",
        text:
          lang === "zh"
            ? `字幕已导出：${paths.join("、")}`
            : `Subtitles exported: ${paths.join(", ")}`,
      });
    });

  const runVideoExport = async () => {
    try {
      setFeedback(null);
      setVideoExportJob(await videoExportStart(pid, videoExportMode));
    } catch (error) {
      setFeedback({ tone: "error", text: friendlyError(error, lang) });
    }
  };

  const cancelVideoExport = async () => {
    try {
      setVideoExportJob(await videoExportCancel(pid));
    } catch (error) {
      setFeedback({ tone: "error", text: friendlyError(error, lang) });
    }
  };

  const runFinalCutExport = () =>
    perform("export-fcp", async () => {
      const path = await exportFinalCut(pid);
      setFeedback({
        tone: "success",
        text: lang === "zh" ? `Final Cut 工程已导出：${path}` : `Final Cut project exported: ${path}`,
      });
    });

  const revealExportFolder = () =>
    perform("reveal-export", async () => {
      await projectReveal(pid);
    });

  const tabs: Array<{ id: Tab; label: string; disabled?: boolean }> = [
    { id: "setup", label: c.setup },
    { id: "transcript", label: c.transcript, disabled: !hasTranscript },
    { id: "speakers", label: lang === "zh" ? "说话人" : "Speakers", disabled: !hasTranscript },
    { id: "translate", label: c.translate, disabled: !hasTranscript },
    { id: "style", label: c.style, disabled: !hasTranscript },
    { id: "properties", label: c.properties },
    { id: "history", label: c.history },
    { id: "timeline", label: c.timeline, disabled: !hasTranscript },
    { id: "broll", label: c.broll, disabled: !hasTranscript },
    { id: "review", label: c.review, disabled: !hasTranscript },
    { id: "export", label: c.export, disabled: !hasTranscript },
  ];

  return (
    <section className="editor-view">
      <header className="editor-header">
        <div>
          <p className="eyebrow">{mediaName(doc.media.path)}</p>
          <h1>{doc.meta.title}</h1>
          <p className="editor-meta">
            {doc.media.durationSeconds.toFixed(1)}s
            {hasTranscript && ` · ${doc.paragraphs.length} ${c.paragraphs} · ${doc.paragraphs.flatMap((p) => p.sentences.flatMap((s) => s.words)).length} ${c.words}`}
          </p>
        </div>
        <div className="editor-header-actions">
          <button
            className={activeTab === "history" ? "active" : ""}
            onClick={() => setActiveTab("history")}
          >
            {lang === "zh" ? "历史" : "History"}
          </button>
          <button
            className={activeTab === "review" ? "active" : ""}
            disabled={!hasTranscript}
            onClick={() => setActiveTab("review")}
          >
            {lang === "zh" ? "检查" : "Review"}
          </button>
          <button
            aria-label={lang === "zh" ? "导出作品" : "Export project"}
            className="editor-export-button"
            disabled={!hasTranscript}
            onClick={() => setActiveTab("export")}
          >
            {lang === "zh" ? "导出" : "Export"}
          </button>
        {taskState && taskState.kinds.length > 0 && (
          <details className="task-activity">
            <summary className="task-pill">
              <span className={stoppedTasks > 0 ? "failed-dot" : taskState.pending > 0 ? "pulse-dot" : failedTasks > 0 ? "failed-dot" : "done-dot"} />
              {c.taskStatus}: {stoppedTasks > 0
                ? (lang === "zh" ? "需要处理" : "Needs attention")
                : taskState.pending > 0
                ? `${taskState.pending} ${c.pending}`
                : failedTasks > 0
                  ? `${failedTasks} ${lang === "zh" ? "失败" : "failed"}`
                  : `${taskState.done} ${c.done}`}
            </summary>
            <div className="task-popover">
              {taskState.kinds.map((task) => (
                <div key={`${task.kind}-${task.lang || ""}`}>
                  <span>
                    <strong>{taskLabel(task.kind, lang)}</strong>
                    {task.lang && <small>{task.lang.toUpperCase()}</small>}
                  </span>
                  <span className={task.failed > 0 || task.state === "paused" || task.state === "failed" ? "task-failed" : task.pending > 0 ? "task-running" : "task-done"}>
                    {task.state === "paused"
                      ? (lang === "zh" ? "已暂停" : "paused")
                      : task.state === "failed" && task.failed === 0
                        ? (lang === "zh" ? "需要重试" : "retry needed")
                        : task.failed > 0
                      ? `${task.failed} ${lang === "zh" ? "失败" : "failed"}`
                      : task.pending > 0
                        ? `${task.pending} ${c.pending}`
                        : c.done}
                  </span>
                </div>
              ))}
            </div>
          </details>
        )}
        </div>
      </header>

      <EditorMediaPreview
            currentTime={workbenchTime}
            doc={doc}
            lang={lang}
        playerRef={workbenchPlayerRef}
        rows={previewRows}
        subtitleStyle={subtitleStyle}
        onPlayingChange={setWorkbenchPlaying}
        onTimeChange={setWorkbenchTime}
      />

      <nav className="editor-tabs" aria-label={lang === "zh" ? "编辑步骤" : "Editor sections"}>
        {tabs.map((tab) => (
          <button
            aria-current={activeTab === tab.id ? "page" : undefined}
            className={activeTab === tab.id ? "active" : ""}
            disabled={tab.disabled}
            key={tab.id}
            onClick={() => {
              setFeedback(null);
              setActiveTab(tab.id);
            }}
          >
            {tab.label}
          </button>
        ))}
      </nav>

      {feedback && (
        <div className={`notice ${feedback.tone}-notice`} role={feedback.tone === "error" ? "alert" : "status"}>
          {feedback.tone === "success" ? <CheckIcon /> : feedback.tone === "error" ? <AlertIcon /> : null}
          <span>{feedback.text}</span>
        </div>
      )}

      {activeTab === "setup" && (
        <div className="setup-layout">
          <section className="setup-primary">
            <div className="setup-status-icon">
              {hasTranscript ? <CheckIcon /> : <PlayIcon />}
            </div>
            <p className="eyebrow">{c.imported}</p>
            <h2>{hasTranscript ? c.transcript : c.startTitle}</h2>
            <p>{hasTranscript ? c.transcribeHint : c.startDescription}</p>
            {asrReadiness && !asrReadiness.ready && (
              <div className="notice error-notice setup-readiness" role="alert">
                <AlertIcon />
                <span>{c.asrNotReady}</span>
                <button className="button-quiet" onClick={onOpenSettings}>
                  {c.openAsrSettings}
                </button>
              </div>
            )}
            <button
              className="button-primary button-large"
              disabled={operation !== null}
              onClick={startTranscription}
            >
              {operation === "transcribe" ? (
                <>
                  <span className="spinner" aria-hidden="true" />
                  {transcriptionJob
                    ? transcriptionPhaseLabel(transcriptionJob.phase, lang)
                    : c.transcribing}
                </>
              ) : (
                <>
                  <PlayIcon />
                  {transcriptionFailure
                    ? c.retry
                    : hasTranscript
                      ? (lang === "zh" ? "重新转写" : "Transcribe again")
                      : c.start}
                </>
              )}
            </button>
            {transcriptionJob ? (
              <div className="transcription-progress" aria-live="polite">
                <div>
                  <strong>{transcriptionPhaseLabel(transcriptionJob.phase, lang)}</strong>
                  <span>{transcriptionJob.progress}%</span>
                </div>
                <progress max={100} value={transcriptionJob.progress} />
                {transcriptionJob.device && (
                  <small className="pipeline-resources">
                    MLX · Metal
                    {transcriptionJob.elapsedSeconds !== null ? ` · ${Math.round(transcriptionJob.elapsedSeconds)}s` : ""}
                    {transcriptionJob.cpuPercent !== null ? ` · CPU ${transcriptionJob.cpuPercent}%` : ""}
                    {transcriptionJob.peakMemoryMb !== null
                      ? ` · ${lang === "zh" ? "峰值内存" : "Peak memory"} ${(transcriptionJob.peakMemoryMb / 1024).toFixed(1)} GB`
                      : ""}
                    {transcriptionJob.memoryLimitMb !== null
                      ? ` / ${(transcriptionJob.memoryLimitMb / 1024).toFixed(1)} GB`
                      : ""}
                    {transcriptionJob.current !== null && transcriptionJob.total !== null
                      ? ` · ${transcriptionJob.current}/${transcriptionJob.total}`
                      : ""}
                  </small>
                )}
                <button
                  className="button-quiet"
                  disabled={transcriptionJob.state === "cancelling"}
                  onClick={cancelTranscription}
                >
                  {transcriptionJob.state === "cancelling"
                    ? c.cancellingTranscription
                    : c.cancelTranscription}
                </button>
              </div>
            ) : (
              <small>{c.transcribeHint}</small>
            )}
          </section>

          <dl className="media-facts">
            <div>
              <dt>{c.media}</dt>
              <dd title={doc.media.path}>{mediaName(doc.media.path)}</dd>
            </div>
            <div>
              <dt>{c.duration}</dt>
              <dd>{doc.media.durationSeconds.toFixed(1)}s</dd>
            </div>
            <div>
              <dt>{c.language}</dt>
              <dd>{doc.meta.language || c.auto}</dd>
            </div>
          </dl>
        </div>
      )}

      {activeTab === "transcript" && (
        <div className="transcript-layout">
          <aside className="editor-actions">
            <section>
              <h2>{lang === "zh" ? "增强转写" : "Enhance transcript"}</h2>
              <button
                onClick={() => setActiveTab("speakers")}
              >
                {operation === "speakers-preview" ? <span className="spinner" /> : null}
                {operation === "speakers-preview"
                  ? lang === "zh" ? "查看识别进度" : "View identification progress"
                  : speakerEvidenceState.identified || speakers.length > 0
                    ? lang === "zh" ? "管理说话人" : "Manage speakers"
                    : c.speaker}
              </button>
              <p className="editor-action-status">
                {operation === "speakers-preview"
                  ? lang === "zh" ? "识别正在后台运行" : "Identification is running in the background"
                  : speakerEvidenceState.identified
                    ? lang === "zh" ? `${speakers.length} 位说话人 · 结果已保存` : `${speakers.length} speakers · result saved`
                    : lang === "zh" ? "打开工作区后再决定是否开始识别" : "Open the workspace before starting identification"}
              </p>
            </section>
            <section className="editor-help">
              <h2>{lang === "zh" ? "编辑提示" : "Editing tip"}</h2>
              <p>
                {lang === "zh"
                  ? "逐句修改后按 ⌘↵ 保存。修改会重新绑定词级时码。"
                  : "Edit a cue and press ⌘↵ to save. Word timing is rebound automatically."}
              </p>
            </section>
          </aside>
          <TranscriptEditor
            busy={operation !== null}
            currentTime={workbenchTime}
            isPlaying={workbenchPlaying}
            lang={lang}
            nextCueById={nextCueById}
            rows={subtitleRows}
            wordsByCue={wordsByCue}
            onMerge={mergeSubtitleLines}
            onReplace={replaceSubtitles}
            onSave={saveSubtitle}
            onSeek={seekWorkbench}
            onSplit={splitSubtitleLine}
            onVisibility={changeSubtitleVisibility}
          />
        </div>
      )}

      {activeTab === "translate" && (
        <TranslationWorkspace
          busy={operation !== null}
          configured={agentConfigured}
          currentTime={workbenchTime}
          doc={doc}
          lang={lang}
          status={taskState}
          onOpenSettings={onOpenSettings}
          onLanguageChange={setPreviewTranslationLanguage}
          onSave={saveTranslation}
          onSeek={seekWorkbench}
          onStart={startTranslation}
        />
      )}

      {activeTab === "style" && subtitleStyle && (
        <StyleWorkspace
          busy={operation === "style"}
          lang={lang}
          style={subtitleStyle}
          onSave={saveStyle}
        />
      )}

      {(activeTab === "speakers" || activeTab === "properties") && (
        <PropertiesWorkspace
          analysis={speakerAnalysisJob}
          busy={operation !== null}
          diarizeReady={asrReadiness?.diarizeReady ?? false}
          doc={doc}
          evidence={speakerEvidenceState}
          lang={lang}
          preview={speakerPreview}
          section={activeTab === "speakers" ? "speakers" : "project"}
          speakers={speakers}
          onApplyPreview={applySpeakerPreview}
          onAssign={assignSpeaker}
          onCancelAnalysis={cancelSpeakerAnalysis}
          onMerge={mergeSpeaker}
          onOpenSettings={onOpenSettings}
          onPreview={previewSpeakers}
          onRename={renameSpeaker}
          onSaveMeta={saveProjectMeta}
        />
      )}

      {activeTab === "history" && versionHistory && (
        <HistoryWorkspace
          busy={operation !== null}
          history={versionHistory}
          lang={lang}
          onCommit={commitVersion}
          onCreateBranch={createBranch}
          onRestore={restoreVersion}
          onSwitchBranch={switchBranch}
        />
      )}

      {activeTab === "timeline" && (
        <TimelineWorkspace
          busy={operation !== null}
          currentTime={workbenchTime}
          cuts={cuts}
          doc={doc}
          lang={lang}
          onRestoreCut={restoreCut}
          onSeek={seekWorkbench}
        />
      )}

      {activeTab === "broll" && (
        <BrollWorkspace
          busy={operation !== null}
          doc={doc}
          lang={lang}
          overview={brollOverview}
          previewJob={brollPreviewJob}
          previewPaths={brollPreviewPaths}
          onAcceptSuggestion={acceptBrollSuggestion}
          onAdd={addBroll}
          onCancelPreview={cancelBrollPreview}
          onPickFile={pickBrollAsset}
          onPreview={previewBroll}
          onRefresh={() => performRecoverable("broll-load", refreshBroll)}
          onRemove={removeBroll}
          onUpdate={updateBroll}
        />
      )}

      {activeTab === "review" && (
        <div className="review-layout">
          <EnhancementPanel
            busy={operation !== null}
            configured={agentConfigured}
            doc={doc}
            lang={lang}
            status={taskState}
            onOpenSettings={onOpenSettings}
            onStart={startEnhancement}
          />
          <div className="review-toolbar">
            <button disabled={operation !== null} onClick={runReview}>{c.runAudit}</button>
            <button disabled={operation !== null} onClick={runFinishCheck}>{c.finishCheck}</button>
            <button disabled={operation !== null} onClick={scanCuts}>{c.scanCuts}</button>
          </div>

          {auditReport && (
            <section className="review-section">
              <h2>{c.runAudit}</h2>
              {auditReport.findings.length === 0 ? (
                <p className="clean-result"><CheckIcon />{c.noFindings}</p>
              ) : (
                <ul className="finding-list">
                  {auditReport.findings.map((finding, index) => (
                    <li key={`${finding.code}-${index}`}>
                      <span className={`severity ${finding.severity}`}>{finding.severity}</span>
                      <div>
                        <strong>{finding.message}</strong>
                        <small>{finding.location} · {finding.code}</small>
                      </div>
                    </li>
                  ))}
                </ul>
              )}
            </section>
          )}

          {finishItems && (
            <section className="review-section">
              <h2>{c.finishCheck}</h2>
              <ul className="check-list">
                {finishItems.map((item) => (
                  <li key={item.ordinal}>
                    <span className={item.pass ? "check-pass" : "check-fail"}>
                      {item.pass ? <CheckIcon /> : <AlertIcon />}
                    </span>
                    <div>
                      <strong>{item.code}</strong>
                      {item.blockers.map((blocker, index) => (
                        <small key={`${item.code}-${index}`}>{blocker}</small>
                      ))}
                    </div>
                  </li>
                ))}
              </ul>
            </section>
          )}

          {cuts.length > 0 && (
            <section className="review-section">
              <h2>{c.suggestedCuts} <span>{cuts.length}</span></h2>
              <div className="cut-rows">
                {cuts.map((cut) => (
                  <div className="cut-row" key={cut.id}>
                    <div>
                      <strong>{cut.a_word} → {cut.b_word}</strong>
                      <small>{cut.kind} · {cut.duration.toFixed(2)}s {cut.note ? `· ${cut.note}` : ""}</small>
                    </div>
                    <button
                      className="button-quiet"
                      disabled={operation !== null}
                      onClick={() => restoreCut(cut.id)}
                    >
                      {c.restore}
                    </button>
                  </div>
                ))}
              </div>
            </section>
          )}
        </div>
      )}

      {activeTab === "export" && (
        <div className="export-layout">
          <div className="export-intro">
            <p className="eyebrow">{c.export}</p>
            <h2>{lang === "zh" ? "交付你的作品" : "Deliver your work"}</h2>
            <p>{c.exportHint}</p>
          </div>
          <section className={`export-preflight ${exportReady ? "ready" : failedFinishItems.length > 0 ? "blocked" : "unchecked"}`}>
            <header>
              <div>
                <p className="eyebrow">{c.exportCheckTitle}</p>
                <h3>
                  {finishItems === null
                    ? c.exportUnchecked
                    : exportReady
                      ? c.exportReady
                      : c.exportBlocked}
                </h3>
              </div>
              <button
                className={finishItems === null ? "button-primary" : "button-quiet"}
                disabled={operation !== null}
                onClick={runFinishCheck}
              >
                {operation === "finish" ? <span className="spinner" /> : null}
                {finishItems === null
                  ? lang === "zh" ? "开始检查" : "Run check"
                  : lang === "zh" ? "重新检查" : "Check again"}
              </button>
            </header>
            {failedFinishItems.length > 0 && (
              <ul>
                {failedFinishItems.map((item) => (
                  <li key={item.ordinal}>
                    <AlertIcon />
                    <span>
                      <strong>{item.code}</strong>
                      {item.blockers.map((blocker, index) => (
                        <small key={`${item.ordinal}-${index}`}>{blocker}</small>
                      ))}
                    </span>
                  </li>
                ))}
              </ul>
            )}
            {!exportReady && finishItems !== null && (
              <label className="draft-export-override">
                <input
                  type="checkbox"
                  checked={allowDraftExport}
                  onChange={(event) => setAllowDraftExport(event.target.checked)}
                />
                <span>{c.draftOverride}</span>
              </label>
            )}
          </section>
          <div className="export-actions">
            <label className="video-export-mode">
              <span>{lang === "zh" ? "视频编码模式" : "Video encoding mode"}</span>
              <select
                disabled={isVideoExporting || operation !== null}
                value={videoExportMode}
                onChange={(event) => setVideoExportMode(event.target.value as VideoExportJobStatus["mode"])}
              >
                <option value="fast">
                  {lang === "zh" ? "硬件低负载 · 低 CPU / 较大文件" : "Hardware low-load · low CPU / larger file"}
                </option>
                <option value="quality">
                  {lang === "zh" ? "高压缩质量 · 高 CPU / 较小文件" : "Compression quality · high CPU / smaller file"}
                </option>
              </select>
            </label>
            <button
              className="export-action"
              disabled={operation !== null || !exportAllowed}
              onClick={runSubtitleExport}
            >
              {operation === "export-subtitles" ? <span className="spinner" /> : <TranscriptIcon />}
              <span>
                <strong>{c.exportSubtitles}</strong>
                <small>SRT · VTT · ASS · Markdown</small>
              </span>
            </button>
            <button
              className="export-action"
              disabled={operation !== null || !exportAllowed || isVideoExporting}
              onClick={runVideoExport}
            >
              {isVideoExporting ? <span className="spinner" /> : <PlayIcon />}
              <span>
                <strong>{c.exportVideo}</strong>
                <small>
                  {videoExportMode === "fast"
                    ? "MP4 · VideoToolbox · low CPU"
                    : "MP4 · libx264 · smaller file"}
                </small>
              </span>
            </button>
            <button
              className="export-action"
              disabled={operation !== null || !exportAllowed}
              onClick={runFinalCutExport}
            >
              {operation === "export-fcp" ? <span className="spinner" /> : <PlayIcon />}
              <span>
                <strong>{c.exportFcp}</strong>
                <small>FCPXML · editable timeline · B-roll</small>
              </span>
            </button>
          </div>
          {videoExportJob && isVideoExporting && (
            <div className="video-export-progress" role="status" aria-live="polite">
              <div>
                <strong>
                  {videoExportJob.phase === "preparing"
                    ? lang === "zh" ? "正在准备视频导出" : "Preparing video export"
                    : videoExportJob.phase === "waiting"
                      ? lang === "zh" ? "正在等待计算资源" : "Waiting for compute capacity"
                    : videoExportJob.state === "cancelling"
                      ? lang === "zh" ? "正在停止导出" : "Stopping export"
                      : lang === "zh" ? "正在硬件编码" : "Hardware encoding"}
                </strong>
                <span>{videoExportJob.progress}%</span>
              </div>
              <progress
                aria-label={lang === "zh" ? "视频导出进度" : "Video export progress"}
                max={100}
                value={videoExportJob.progress}
              />
              <small>
                {videoExportJob.encoder === "h264_videotoolbox"
                  ? "VideoToolbox · Apple Media Engine"
                  : videoExportJob.encoder === "libx264"
                    ? "libx264 · CPU"
                    : lang === "zh" ? "正在选择编码后端…" : "Selecting encoder…"}
                {videoExportJob.currentSeconds !== null && videoExportJob.totalSeconds !== null
                  ? ` · ${Math.round(videoExportJob.currentSeconds)}s / ${Math.round(videoExportJob.totalSeconds)}s`
                  : ""}
              </small>
              <button
                className="button-quiet"
                disabled={videoExportJob.state === "cancelling"}
                onClick={() => void cancelVideoExport()}
              >
                {videoExportJob.state === "cancelling"
                  ? lang === "zh" ? "正在停止…" : "Stopping…"
                  : lang === "zh" ? "取消导出" : "Cancel export"}
              </button>
            </div>
          )}
          <div className="export-footer">
            <small>{c.videoExportHint}</small>
            <button
              className="button-quiet"
              disabled={operation !== null}
              onClick={revealExportFolder}
            >
              {c.revealExports}
            </button>
          </div>
        </div>
      )}

      <EditorTimelineDock
        broll={brollOverview}
        currentTime={workbenchTime}
        cuts={cuts}
        doc={doc}
        isPlaying={workbenchPlaying}
        lang={lang}
        rows={subtitleRows}
        onOpenBroll={() => setActiveTab("broll")}
        onSeek={seekWorkbench}
        onTogglePlayback={toggleWorkbenchPlayback}
      />
    </section>
  );
}
