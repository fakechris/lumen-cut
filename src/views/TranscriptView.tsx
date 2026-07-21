import { useEffect, useRef, useState } from "react";
import {
  audit,
  asrStatus,
  branchCreate,
  branchSwitch,
  configShow,
  cutAuto,
  cutList,
  cutRestore,
  diarize,
  exportSubtitles,
  exportVideo,
  finishCheck,
  mergeSubtitles,
  projectUpdateMeta,
  projectShow,
  speakerMerge,
  speakerRename,
  speakersList,
  splitSubtitle,
  styleGet,
  styleSet,
  subtitleList,
  subtitleReplace,
  subtitleSet,
  subtitleVisibility,
  taskStart,
  taskStatus,
  transcriptionCancel,
  transcriptionStart,
  transcriptionStatus,
  versionCommit,
  versionList,
  versionRestore,
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
  FinishCheckItem,
  SubtitleRow,
  SubtitleStyle,
  ReportSummary,
  SpeakerInfo,
  TaskStatus,
  TranscriptionJobStatus,
  VersionHistory,
} from "../types";
import { StyleWorkspace } from "./editor/StyleWorkspace";
import { EnhancementPanel } from "./editor/EnhancementPanel";
import { PropertiesWorkspace } from "./editor/PropertiesWorkspace";
import { TimelineWorkspace } from "./editor/TimelineWorkspace";
import { TranscriptEditor } from "./editor/TranscriptEditor";
import { TranslationWorkspace } from "./editor/TranslationWorkspace";
import { HistoryWorkspace } from "./editor/HistoryWorkspace";

interface Props {
  lang: Lang;
  pid: string | null;
  onOpenSettings: () => void;
  onProjectTitleChange: (title: string) => void;
}

type Tab =
  | "setup"
  | "transcript"
  | "translate"
  | "style"
  | "properties"
  | "history"
  | "timeline"
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
    exportHint: "文件会写入当前项目目录。",
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
    exportHint: "Files are written to the current project folder.",
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
    preparing: ["正在准备项目", "Preparing the project"],
    downloading: ["正在下载媒体", "Downloading media"],
    extracting: ["正在提取音频", "Extracting audio"],
    analyzing: ["正在分析媒体", "Analyzing media"],
    transcribing: ["正在识别语音与时码", "Recognizing speech and timing"],
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
  const [taskState, setTaskState] = useState<TaskStatus | null>(null);
  const [cuts, setCuts] = useState<CutSummary[]>([]);
  const [subtitleRows, setSubtitleRows] = useState<SubtitleRow[]>([]);
  const [subtitleStyle, setSubtitleStyle] = useState<SubtitleStyle | null>(null);
  const [speakers, setSpeakers] = useState<SpeakerInfo[]>([]);
  const [transcriptionJob, setTranscriptionJob] =
    useState<TranscriptionJobStatus | null>(null);
  const [agentConfigured, setAgentConfigured] = useState(false);
  const [asrReadiness, setAsrReadiness] = useState<AsrStatus | null>(null);
  const [versionHistory, setVersionHistory] = useState<VersionHistory | null>(null);
  const previousPending = useRef(0);

  const reload = async (projectId: string, resetTab = true) => {
    const [nextDoc, nextRows, nextStyle, nextSpeakers] = await Promise.all([
      projectShow(projectId),
      subtitleList(projectId),
      styleGet(projectId),
      speakersList(projectId),
    ]);
    setDoc(nextDoc);
    setSubtitleRows(nextRows);
    setSubtitleStyle(nextStyle);
    setSpeakers(nextSpeakers);
    if (resetTab) {
      setActiveTab(nextDoc.paragraphs.length > 0 ? "transcript" : "setup");
    }
    try {
      setCuts(await cutList(projectId));
    } catch {
      setCuts([]);
    }
  };

  useEffect(() => {
    setDoc(null);
    setFeedback(null);
    setAuditReport(null);
    setFinishItems(null);
    setTaskState(null);
    setOperation(null);
    setTranscriptionJob(null);
    setVersionHistory(null);
    if (!pid) return;
    void Promise.all([
      reload(pid),
      taskStatus(pid).then(setTaskState),
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
          }
        })
        .catch(() => undefined),
    ]).catch((error) => {
        setFeedback({ tone: "error", text: friendlyError(error, lang) });
      });
  }, [pid]);

  useEffect(() => {
    if (!pid || !taskState || taskState.pending < 1) return;
    const timer = window.setInterval(() => {
      void taskStatus(pid)
        .then(setTaskState)
        .catch(() => window.clearInterval(timer));
    }, 2500);
    return () => window.clearInterval(timer);
  }, [pid, taskState?.pending]);

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
          return;
        }
        if (status.state === "cancelled") {
          setFeedback({ tone: "info", text: c.cancelledTranscription });
          setOperation(null);
          setTranscriptionJob(null);
          return;
        }
        if (status.state === "failed") {
          setFeedback({
            tone: "error",
            text: friendlyError(status.error || "Transcription failed", lang),
          });
          setOperation(null);
          setTranscriptionJob(null);
          return;
        }
        setTranscriptionJob(status);
        timer = window.setTimeout(poll, 500);
      } catch (error) {
        if (disposed) return;
        setFeedback({ tone: "error", text: friendlyError(error, lang) });
        setOperation(null);
        setTranscriptionJob(null);
      }
    };
    timer = window.setTimeout(poll, 350);
    return () => {
      disposed = true;
      if (timer) window.clearTimeout(timer);
    };
  }, [pid, transcriptionJob?.state, lang]);

  useEffect(() => {
    const pending = taskState?.pending ?? 0;
    if (pid && previousPending.current > 0 && pending === 0) {
      void reload(pid, false);
    }
    previousPending.current = pending;
  }, [pid, taskState?.pending]);

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
  const failedTasks = taskState?.kinds.reduce((sum, task) => sum + task.failed, 0) ?? 0;
  const wordsByCue: Record<string, string[]> = {};
  const nextCueById: Record<string, string> = {};
  for (const paragraph of doc.paragraphs) {
    paragraph.sentences.forEach((sentence, index) => {
      wordsByCue[sentence.id] = sentence.words.map((word) => word.text);
      const next = paragraph.sentences[index + 1];
      if (next) nextCueById[sentence.id] = next.id;
    });
  }

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

  const performRecoverable = async (name: string, action: () => Promise<void>) => {
    setOperation(name);
    setFeedback(null);
    try {
      await action();
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
      setFeedback({ tone: "error", text: friendlyError(error, lang) });
      setOperation(null);
      setTranscriptionJob(null);
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

  const identifySpeakers = () =>
    perform("speakers", async () => {
      const result = await diarize(pid);
      await reload(pid);
      setFeedback({
        tone: "success",
        text:
          lang === "zh"
            ? `已为 ${result.paragraphs_assigned} 个段落识别说话人。`
            : `Identified speakers for ${result.paragraphs_assigned} paragraphs.`,
      });
    });

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

  const renameSpeaker = async (from: string, to: string) => {
    setOperation(`speaker-rename-${from}`);
    setFeedback(null);
    try {
      const changed = await speakerRename(pid, from, to);
      if (changed < 1) throw new Error(`speaker ${from} was not found`);
      await reload(pid, false);
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
      setFinishItems(await finishCheck(pid));
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

  const runVideoExport = () =>
    perform("export-video", async () => {
      const path = await exportVideo(pid);
      setFeedback({
        tone: "success",
        text: lang === "zh" ? `视频已导出：${path}` : `Video exported: ${path}`,
      });
    });

  const tabs: Array<{ id: Tab; label: string; disabled?: boolean }> = [
    { id: "setup", label: c.setup },
    { id: "transcript", label: c.transcript, disabled: !hasTranscript },
    { id: "translate", label: c.translate, disabled: !hasTranscript },
    { id: "style", label: c.style, disabled: !hasTranscript },
    { id: "properties", label: c.properties },
    { id: "history", label: c.history },
    { id: "timeline", label: c.timeline, disabled: !hasTranscript },
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
        {taskState && taskState.kinds.length > 0 && (
          <details className="task-activity">
            <summary className="task-pill">
              <span className={taskState.pending > 0 ? "pulse-dot" : failedTasks > 0 ? "failed-dot" : "done-dot"} />
              {c.taskStatus}: {taskState.pending > 0
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
                  <span className={task.failed > 0 ? "task-failed" : task.pending > 0 ? "task-running" : "task-done"}>
                    {task.failed > 0
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
      </header>

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
                  {hasTranscript ? (lang === "zh" ? "重新转写" : "Transcribe again") : c.start}
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
              <button disabled={operation !== null} onClick={identifySpeakers}>
                {operation === "speakers" ? <span className="spinner" /> : null}
                {c.speaker}
              </button>
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
            lang={lang}
            nextCueById={nextCueById}
            rows={subtitleRows}
            wordsByCue={wordsByCue}
            onMerge={mergeSubtitleLines}
            onReplace={replaceSubtitles}
            onSave={saveSubtitle}
            onSplit={splitSubtitleLine}
            onVisibility={changeSubtitleVisibility}
          />
        </div>
      )}

      {activeTab === "translate" && (
        <TranslationWorkspace
          busy={operation !== null}
          configured={agentConfigured}
          doc={doc}
          lang={lang}
          status={taskState}
          onOpenSettings={onOpenSettings}
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

      {activeTab === "properties" && (
        <PropertiesWorkspace
          busy={operation !== null}
          doc={doc}
          lang={lang}
          speakers={speakers}
          onIdentify={identifySpeakers}
          onMerge={mergeSpeaker}
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
        <TimelineWorkspace cuts={cuts} doc={doc} lang={lang} />
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
          <div>
            <p className="eyebrow">{c.export}</p>
            <h2>{lang === "zh" ? "交付你的作品" : "Deliver your work"}</h2>
            <p>{c.exportHint}</p>
          </div>
          <div className="export-actions">
            <button
              className="export-action"
              disabled={operation !== null}
              onClick={runSubtitleExport}
            >
              <TranscriptIcon />
              <span>
                <strong>{c.exportSubtitles}</strong>
                <small>SRT · VTT · ASS · Markdown</small>
              </span>
            </button>
            <button
              className="export-action"
              disabled={operation !== null}
              onClick={runVideoExport}
            >
              <PlayIcon />
              <span>
                <strong>{c.exportVideo}</strong>
                <small>MP4 · burn-in subtitles</small>
              </span>
            </button>
          </div>
        </div>
      )}
    </section>
  );
}
