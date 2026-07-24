import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { PipelineFreshness } from "../components/PipelineFreshness";
import {
  audit,
  audioMixGet,
  audioMixSet,
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
  chapterList,
  chapterSetMany,
  configShow,
  cutAuto,
  cutList,
  cutManualMany,
  cutRestore,
  editHistoryStatus,
  editRedo,
  editUndo,
  exportSubtitles,
  exportFinalCut,
  exportSettingsGet,
  exportSettingsSet,
  exportPreflight,
  finishCheckForExport,
  translationAutoFit,
  mergeSubtitles,
  pickBrollFile,
  pickMediaFile,
  projectMediaRelink,
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
  setSubtitleTiming,
  splitSubtitle,
  styleGet,
  styleSet,
  subtitleList,
  subtitleReplace,
  subtitleUpdateMany,
  subtitleVisibility,
  translationSet,
  translationSetMany,
  taskStart,
  taskResume,
  taskPause,
  taskStatus,
  titleAdd,
  titleList,
  titleRemove,
  titleUpdate,
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
import type { CutSummary, EditHistoryStatus } from "../api";
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
  AudioMix,
  BrollOverview,
  BrollPlacementInput,
  BrollPreviewJobStatus,
  BrollSuggestion,
  ChapterInput,
  ChapterRow,
  FinishCheckItem,
  ExportPreflightReport,
  SubtitleRow,
  SubtitleStyle,
  ReportSummary,
  SpeakerEvidence,
  SpeakerAnalysisJobStatus,
  SpeakerReidentifyProposal,
  SpeakerInfo,
  SpeakerReidentifyPreview,
  TaskStatus,
  TitleClip,
  TitleClipInput,
  TranscriptionJobStatus,
  VersionHistory,
  VideoExportJobStatus,
  VideoExportSettings,
} from "../types";
import { StyleWorkspace } from "./editor/StyleWorkspace";
import { EnhancementPanel } from "./editor/EnhancementPanel";
import { PropertiesWorkspace } from "./editor/PropertiesWorkspace";
import { TimelineWorkspace } from "./editor/TimelineWorkspace";
import {
  TranscriptEditor,
  type TranscriptDraft,
} from "./editor/TranscriptEditor";
import {
  TranslationWorkspace,
  type TranslationDraft,
} from "./editor/TranslationWorkspace";
import { HistoryWorkspace } from "./editor/HistoryWorkspace";
import { BrollWorkspace, EMPTY_BROLL_INPUT } from "./editor/BrollWorkspace";
import { ReviewFindings } from "./editor/ReviewFindings";
import { EditorMediaPreview } from "./editor/EditorMediaPreview";
import { EditorTimelineDock } from "./editor/EditorTimelineDock";
import {
  ChapterWorkspace,
  type ChapterDraft,
} from "./editor/ChapterWorkspace";
import { DEFAULT_AUDIO_MIX } from "./editor/audioMix";
import {
  editedTimelineDuration,
  nextPlayableTime,
  resolveTimelineCuts,
  sourceToEditedTime,
} from "./editor/timelineCuts";

interface Props {
  active: boolean;
  chapterDrafts: Record<string, ChapterDraft>;
  lang: Lang;
  onChapterDraftsChange: (update: (
    current: Record<string, ChapterDraft>,
  ) => Record<string, ChapterDraft>) => void;
  onTranscriptDraftsChange: (update: (
    current: Record<string, TranscriptDraft>,
  ) => Record<string, TranscriptDraft>) => void;
  onTranslationDraftsChange: (
    language: string,
    update: (
      current: Record<string, TranslationDraft>,
    ) => Record<string, TranslationDraft>,
  ) => void;
  pid: string | null;
  onOpenProjects: () => void;
  onOpenSettings: () => void;
  onProjectTitleChange: (title: string) => void;
  transcriptDrafts: Record<string, TranscriptDraft>;
  translationDrafts: Record<string, Record<string, TranslationDraft>>;
}

type Tab =
  | "setup"
  | "transcript"
  | "subtitle"
  | "chapters"
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
const EMPTY_EDIT_HISTORY: EditHistoryStatus = {
  canUndo: false,
  canRedo: false,
  undoLabel: null,
  redoLabel: null,
};
const DEFAULT_VIDEO_EXPORT_SETTINGS: VideoExportSettings = {
  container: "mp4",
  videoCodec: "h264",
  resolution: "source",
  aspectRatio: "source",
  canvasFit: "contain",
  subtitleMode: "burn",
  subtitleLanguage: null,
  bilingualSubtitles: false,
  audioCodec: "aac",
  encodingSpeed: "fast",
};
const BROLL_DRAFTS_KEY_PREFIX = "lumen-cut.brollDrafts.";
const STYLE_DRAFTS_KEY_PREFIX = "lumen-cut.styleDrafts.";
const INSPECTOR_PERCENT_KEY = "lumen-cut.inspectorPercent";
const COMPACT_TIMELINE_QUERY =
  "(max-height: 760px), (max-width: 860px) and (max-height: 820px)";
type TimelinePreference = "auto" | "expanded" | "collapsed";

export function resolveTimelineCollapsed(
  preference: TimelinePreference,
  compactViewport: boolean,
) {
  if (preference === "collapsed") return true;
  if (preference === "expanded") return false;
  return compactViewport;
}

function normalizeVideoExportSettings(
  settings: Partial<VideoExportSettings> | null | undefined,
): VideoExportSettings {
  return { ...DEFAULT_VIDEO_EXPORT_SETTINGS, ...(settings ?? {}) };
}

function videoCanvasSummary(settings: VideoExportSettings, lang: Lang) {
  const ratio = settings.aspectRatio === "source"
    ? (lang === "zh" ? "跟随源比例" : "source ratio")
    : settings.aspectRatio;
  if (settings.resolution === "source") {
    return `${ratio} · ${lang === "zh" ? "沿用源清晰度" : "source quality"}`;
  }
  if (settings.aspectRatio === "source") {
    return `${ratio} · ${settings.resolution}`;
  }
  const shortEdge = settings.resolution === "720p"
    ? 720
    : settings.resolution === "1080p"
      ? 1080
      : 2160;
  const [ratioWidth, ratioHeight] = settings.aspectRatio.split(":").map(Number);
  const width = ratioWidth >= ratioHeight
    ? Math.round(shortEdge * ratioWidth / ratioHeight)
    : shortEdge;
  const height = ratioWidth >= ratioHeight
    ? shortEdge
    : Math.round(shortEdge * ratioHeight / ratioWidth);
  return `${settings.aspectRatio} · ${width} × ${height}`;
}

type PersistedBrollDrafts = {
  placements: Record<string, BrollPlacementInput>;
  newPlacement: BrollPlacementInput;
};

function isBrollPlacementInput(value: unknown): value is BrollPlacementInput {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const input = value as Record<string, unknown>;
  const rect = input.rect;
  const validRect = rect === undefined
    || rect === null
    || (
      typeof rect === "object"
      && !Array.isArray(rect)
      && ["x", "y", "width", "height"].every(
        (key) => Number.isFinite((rect as Record<string, unknown>)[key]),
      )
    );
  return typeof input.file === "string"
    && Number.isFinite(input.start)
    && Number.isFinite(input.end)
    && (input.mode === "pip" || input.mode === "fullscreen")
    && (input.fit === "cover" || input.fit === "contain")
    && (input.background === "black" || input.background === "blur")
    && Number.isFinite(input.sourceStart)
    && Number.isFinite(input.radius)
    && (typeof input.name === "string" || input.name === null)
    && validRect;
}

function initialBrollDrafts(pid: string | null): PersistedBrollDrafts {
  if (!pid) {
    return { placements: {}, newPlacement: { ...EMPTY_BROLL_INPUT } };
  }
  try {
    const parsed = JSON.parse(
      localStorage.getItem(`${BROLL_DRAFTS_KEY_PREFIX}${pid}`) || "{}",
    ) as Record<string, unknown>;
    const placementsValue = parsed.placements;
    const placements = placementsValue
      && typeof placementsValue === "object"
      && !Array.isArray(placementsValue)
      ? Object.fromEntries(Object.entries(placementsValue)
        .filter((entry): entry is [string, BrollPlacementInput] =>
          isBrollPlacementInput(entry[1])))
      : {};
    return {
      placements,
      newPlacement: isBrollPlacementInput(parsed.newPlacement)
        ? parsed.newPlacement
        : { ...EMPTY_BROLL_INPUT },
    };
  } catch {
    return { placements: {}, newPlacement: { ...EMPTY_BROLL_INPUT } };
  }
}

function isSubtitleStyle(value: unknown): value is SubtitleStyle {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const style = value as Record<string, unknown>;
  return ["name", "fontname", "primaryColour", "outlineColour"]
    .every((key) => typeof style[key] === "string")
    && ["fontsize", "alignment", "outline", "shadow", "marginL", "marginR", "marginV"]
      .every((key) => Number.isFinite(style[key]))
    && ["bold", "italic", "underline", "strikeOut"]
      .every((key) => typeof style[key] === "boolean");
}

function restoredStyleDraft(pid: string, fallback: SubtitleStyle): SubtitleStyle {
  try {
    const parsed = JSON.parse(
      localStorage.getItem(`${STYLE_DRAFTS_KEY_PREFIX}${pid}`) || "null",
    );
    return isSubtitleStyle(parsed) ? parsed : fallback;
  } catch {
    return fallback;
  }
}

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
    checkingAsr: "正在检查本地环境…",
    prepareAsr: "准备转写环境",
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
    paragraphs: "字幕段",
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
    exportHint: "项目模式只负责编辑。交付前请主动运行检查；仅转写完成不会自动导出。",
    exportCheckTitle: "交付检查",
    exportUnchecked: "尚未检查当前版本",
    exportReady: "当前版本可以交付",
    exportBlocked: "存在阻止正式交付的问题",
    projectModeTitle: "项目模式",
    projectModeHint: "转写/翻译完成只表示可以编辑。导出成片前才会做交付检查；不会自动导出。",
    autoFitCaptions: "自动拆开过长字幕",
    autoFitting: "正在拆开…",
    saveVersionContinue: "保存版本并继续",
    savingVersion: "正在保存版本…",
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
    checkingAsr: "Checking local setup…",
    prepareAsr: "Prepare transcription",
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
    paragraphs: "cues",
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
    exportHint: "Project mode is for editing. Run the delivery check before export; transcription alone never auto-exports.",
    exportCheckTitle: "Delivery check",
    exportUnchecked: "The current version has not been checked",
    exportReady: "The current version is ready to deliver",
    exportBlocked: "Issues are blocking a production delivery",
    projectModeTitle: "Project mode",
    projectModeHint: "Transcription/translation only makes the project editable. Delivery checks run when you export — never automatically.",
    autoFitCaptions: "Auto-split long captions",
    autoFitting: "Splitting…",
    saveVersionContinue: "Save version and continue",
    savingVersion: "Saving version…",
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

function finishCheckLabel(code: string, lang: Lang) {
  const labels: Record<string, [string, string]> = {
    "transcribe-complete": ["转写内容", "Transcript"],
    "translations-filled": ["翻译完整性", "Translation coverage"],
    "audit-pass": ["内容与时码审查", "Content and timing"],
    "aligned-with-media": ["媒体时码对齐", "Media alignment"],
    "soft-cuts-sane": ["时间线剪辑", "Timeline edits"],
    "speaker-labels": ["说话人标签", "Speaker labels"],
    "export-ready": ["导出资产", "Export assets"],
    "version-head-committed": ["版本状态", "Version state"],
  };
  return labels[code]?.[lang === "zh" ? 0 : 1] ?? code;
}

function exportPreflightLabel(code: string, lang: Lang) {
  const labels: Record<string, [string, string]> = {
    settings: ["导出规格", "Delivery settings"],
    media: ["源媒体", "Source media"],
    "media-duration": ["媒体时长", "Media duration"],
    timeline: ["成片时间线", "Edited timeline"],
    "timeline-data": ["时间线数据", "Timeline data"],
    captions: ["字幕轨道", "Caption track"],
    "caption-state": ["字幕可见性", "Caption visibility"],
    "hidden-captions": ["隐藏字幕", "Hidden captions"],
    style: ["字幕样式", "Caption style"],
    broll: ["B-roll 素材", "B-roll assets"],
    titles: ["标题图层", "Title layers"],
    audio: ["音频混合", "Audio mix"],
    encoder: ["视频编码器", "Video encoder"],
    "size-estimate": ["文件体积", "File size"],
  };
  return labels[code]?.[lang === "zh" ? 0 : 1] ?? code;
}

function exportPreflightMessage(
  item: ExportPreflightReport["items"][number],
  report: ExportPreflightReport,
  lang: Lang,
) {
  if (lang !== "zh") return item.message;
  if (item.code === "hidden-captions") {
    return `${report.summary.hiddenCaptions} 行隐藏字幕不会进入成片。`;
  }
  if (item.code === "size-estimate") {
    return `预计输出 ${report.summary.estimatedMinMb}–${report.summary.estimatedMaxMb} MB，实际体积会随画面复杂度变化。`;
  }
  const prefixes: Record<string, string> = {
    settings: "当前容器、编码、字幕或音频组合不兼容",
    media: "源媒体无法用于视频导出",
    "media-duration": "项目记录的时长与源媒体不同",
    timeline: "当前剪辑后没有可导出的画面",
    "timeline-data": "时间线编辑数据已损坏或无法读取",
    captions: "所选字幕内容尚未准备好",
    "caption-state": "字幕的显示与隐藏状态已损坏或无法读取",
    style: "字幕样式已损坏或无法读取",
    broll: "有 B-roll 素材无法用于成片",
    titles: "有标题图层无法用于成片",
    audio: "音频设置无法用于成片",
    encoder: "当前设备缺少所选编码器",
  };
  const prefix = prefixes[item.code];
  return prefix ? `${prefix}：${item.message}` : item.message;
}

function exportPreflightFixLabel(code: string, lang: Lang) {
  const labels: Record<string, [string, string]> = {
    media: ["重新定位", "Relink"],
    timeline: ["查看时间线", "Open timeline"],
    "timeline-data": ["查看时间线", "Open timeline"],
    captions: ["前往翻译", "Open translation"],
    "caption-state": ["检查字幕", "Review captions"],
    style: ["检查样式", "Review style"],
    broll: ["检查素材", "Review assets"],
    titles: ["检查标题", "Review titles"],
    audio: ["检查音频", "Review audio"],
    settings: ["调整规格", "Adjust settings"],
    encoder: ["调整编码", "Change encoder"],
  };
  return labels[code]?.[lang === "zh" ? 0 : 1] ?? null;
}

export function TranscriptView({
  active,
  chapterDrafts,
  lang,
  onChapterDraftsChange,
  onTranscriptDraftsChange,
  onTranslationDraftsChange,
  pid,
  onOpenProjects,
  onOpenSettings,
  onProjectTitleChange,
  transcriptDrafts,
  translationDrafts,
}: Props) {
  const c = COPY[lang];
  const [doc, setDoc] = useState<Doc | null>(null);
  const [projectLoadError, setProjectLoadError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<Tab>("setup");
  const [toolsMenuOpen, setToolsMenuOpen] = useState(false);
  const toolsButtonRef = useRef<HTMLButtonElement>(null);
  const toolsMenuRef = useRef<HTMLDivElement>(null);
  const [operation, setOperation] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<Feedback | null>(null);
  const [auditReport, setAuditReport] = useState<ReportSummary | null>(null);
  const [finishItems, setFinishItems] = useState<FinishCheckItem[] | null>(null);
  const [exportPreflightReport, setExportPreflightReport] =
    useState<ExportPreflightReport | null>(null);
  const [exportPreflightUpdating, setExportPreflightUpdating] = useState(false);
  const [allowDraftExport, setAllowDraftExport] = useState(false);
  const [taskState, setTaskState] = useState<TaskStatus | null>(null);
  const [cuts, setCuts] = useState<CutSummary[]>([]);
  const [editHistory, setEditHistory] = useState<EditHistoryStatus>(EMPTY_EDIT_HISTORY);
  const [subtitleRows, setSubtitleRows] = useState<SubtitleRow[]>([]);
  const [chapters, setChapters] = useState<ChapterRow[]>([]);
  const [subtitleStyle, setSubtitleStyle] = useState<SubtitleStyle | null>(null);
  const [savedSubtitleStyle, setSavedSubtitleStyle] = useState<SubtitleStyle | null>(null);
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
  const [confirmRetranscription, setConfirmRetranscription] = useState(false);
  const [videoExportJob, setVideoExportJob] = useState<VideoExportJobStatus | null>(null);
  const [videoExportSettings, setVideoExportSettings] = useState<VideoExportSettings>(
    DEFAULT_VIDEO_EXPORT_SETTINGS,
  );
  const [agentConfigured, setAgentConfigured] = useState(false);
  const [asrReadiness, setAsrReadiness] = useState<AsrStatus | null>(null);
  const selectedAsrReady = asrReadiness
    ? (asrReadiness.selectedReady ?? asrReadiness.ready)
    : false;
  const [versionHistory, setVersionHistory] = useState<VersionHistory | null>(null);
  const [brollOverview, setBrollOverview] = useState<BrollOverview>({
    suggestions: [],
    accepted: [],
    errors: [],
  });
  const initialBroll = useMemo(() => initialBrollDrafts(pid), [pid]);
  const [brollDrafts, setBrollDrafts] =
    useState<Record<string, BrollPlacementInput>>(initialBroll.placements);
  const [newBrollPlacement, setNewBrollPlacement] =
    useState<BrollPlacementInput>(initialBroll.newPlacement);
  const [brollPreviewJob, setBrollPreviewJob] = useState<BrollPreviewJobStatus | null>(null);
  const [brollPreviewPaths, setBrollPreviewPaths] = useState<string[]>([]);
  const [titles, setTitles] = useState<TitleClip[]>([]);
  const [audioMix, setAudioMix] = useState<AudioMix>(DEFAULT_AUDIO_MIX);
  const workbenchPlayerRef = useRef<HTMLMediaElement | null>(null);
  const [workbenchTime, setWorkbenchTime] = useState(0);
  const [workbenchPlaying, setWorkbenchPlaying] = useState(false);
  const [previewCuts, setPreviewCuts] = useState(true);
  const [previewExpanded, setPreviewExpanded] = useState(false);
  const [timelinePreference, setTimelinePreference] =
    useState<TimelinePreference>("auto");
  const [compactTimelineViewport, setCompactTimelineViewport] = useState(
    () => window.matchMedia?.(COMPACT_TIMELINE_QUERY).matches ?? false,
  );
  const [inspectorPercent, setInspectorPercent] = useState(() => {
    try {
      const raw = localStorage.getItem(INSPECTOR_PERCENT_KEY);
      const parsed = raw ? Number(raw) : 50;
      if (!Number.isFinite(parsed)) return 50;
      return Math.min(58, Math.max(32, parsed));
    } catch {
      return 50;
    }
  });
  const [resizingPanes, setResizingPanes] = useState(false);
  const [previewTranslationLanguage, setPreviewTranslationLanguage] = useState<string | null>(null);
  const previousPending = useRef(0);
  const wasActive = useRef(active);
  const activeProject = useRef(pid);
  const exportSettingsProject = useRef<string | null>(null);
  const exportCheckRun = useRef(false);
  activeProject.current = pid;
  const newBrollPlacementDirty = JSON.stringify(newBrollPlacement)
    !== JSON.stringify(EMPTY_BROLL_INPUT);
  const subtitleStyleDirty = subtitleStyle !== null
    && savedSubtitleStyle !== null
    && JSON.stringify(subtitleStyle) !== JSON.stringify(savedSubtitleStyle);
  const timelineCollapsed = resolveTimelineCollapsed(
    timelinePreference,
    compactTimelineViewport,
  );
  const timelineAutoCollapsed = timelinePreference === "auto" && compactTimelineViewport;

  useEffect(() => {
    if (typeof window.matchMedia !== "function") return;
    const query = window.matchMedia(COMPACT_TIMELINE_QUERY);
    const update = () => setCompactTimelineViewport(query.matches);
    update();
    if (typeof query.addEventListener === "function") {
      query.addEventListener("change", update);
      return () => query.removeEventListener("change", update);
    }
    query.addListener?.(update);
    return () => query.removeListener?.(update);
  }, []);

  useEffect(() => {
    try {
      localStorage.setItem(INSPECTOR_PERCENT_KEY, String(Math.round(inspectorPercent)));
    } catch {
      // Split preference is session-only when storage is unavailable.
    }
  }, [inspectorPercent]);

  useEffect(() => {
    if (!toolsMenuOpen) return;
    toolsMenuRef.current
      ?.querySelector<HTMLButtonElement>('[role="menuitem"]')
      ?.focus();
    const closeOnOutsidePointer = (event: PointerEvent) => {
      if (event.target instanceof Node && !toolsMenuRef.current?.parentElement?.contains(event.target)) {
        setToolsMenuOpen(false);
      }
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      setToolsMenuOpen(false);
      toolsButtonRef.current?.focus();
    };
    document.addEventListener("pointerdown", closeOnOutsidePointer);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("pointerdown", closeOnOutsidePointer);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [toolsMenuOpen]);

  useEffect(() => {
    if (!pid || !subtitleStyle || !savedSubtitleStyle) return;
    try {
      const key = `${STYLE_DRAFTS_KEY_PREFIX}${pid}`;
      if (subtitleStyleDirty) {
        localStorage.setItem(key, JSON.stringify(subtitleStyle));
      } else {
        localStorage.removeItem(key);
      }
    } catch {
      // Keep the in-memory preview if browser storage is unavailable.
    }
  }, [pid, savedSubtitleStyle, subtitleStyle, subtitleStyleDirty]);

  useEffect(() => {
    if (!pid) return;
    try {
      const key = `${BROLL_DRAFTS_KEY_PREFIX}${pid}`;
      if (Object.keys(brollDrafts).length === 0 && !newBrollPlacementDirty) {
        localStorage.removeItem(key);
      } else {
        localStorage.setItem(key, JSON.stringify({
          placements: brollDrafts,
          newPlacement: newBrollPlacement,
        } satisfies PersistedBrollDrafts));
      }
    } catch {
      // Keep the in-memory draft if browser storage is unavailable.
    }
  }, [brollDrafts, newBrollPlacement, newBrollPlacementDirty, pid]);

  useEffect(() => {
    if (!doc) return;
    const accepted = new Map(brollOverview.accepted.map((placement) => [
      placement.id,
      {
        file: placement.file,
        start: placement.start,
        end: placement.end,
        mode: placement.mode,
        fit: placement.fit,
        background: placement.background,
        rect: placement.rect,
        sourceStart: placement.sourceStart,
        radius: placement.radius,
        name: placement.name || "",
      } satisfies BrollPlacementInput,
    ]));
    setBrollDrafts((current) => Object.fromEntries(
      Object.entries(current).filter(([id, draft]) => {
        const original = accepted.get(id);
        return original && JSON.stringify(draft) !== JSON.stringify(original);
      }),
    ));
  }, [brollOverview.accepted, doc]);

  useEffect(() => {
    if (
      Object.keys(brollDrafts).length === 0
      && !newBrollPlacementDirty
      && !subtitleStyleDirty
    ) return;
    const warnOnClose = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", warnOnClose);
    return () => window.removeEventListener("beforeunload", warnOnClose);
  }, [brollDrafts, newBrollPlacementDirty, subtitleStyleDirty]);

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
  const previewBrollOverview = useMemo<BrollOverview>(() => {
    if (activeTab !== "broll") return brollOverview;
    return {
      ...brollOverview,
      accepted: brollOverview.accepted.map((placement) => {
        const draft = brollDrafts[placement.id];
        return draft ? { id: placement.id, ...draft } : placement;
      }),
    };
  }, [activeTab, brollDrafts, brollOverview]);
  const translationLanguages = useMemo(
    () => Object.keys(doc?.translations ?? {}).sort((left, right) => left.localeCompare(right)),
    [doc],
  );
  const exportCaptionTrack = videoExportSettings.subtitleLanguage
    ? `${videoExportSettings.bilingualSubtitles ? "bilingual" : "translation"}:${videoExportSettings.subtitleLanguage}`
    : "source";
  const timelineCutIntervals = useMemo(
    () => doc ? resolveTimelineCuts(doc, cuts) : [],
    [cuts, doc],
  );
  const programDuration = useMemo(
    () => doc
      ? editedTimelineDuration(doc.media.durationSeconds, timelineCutIntervals)
      : 0,
    [doc, timelineCutIntervals],
  );
  const programTime = useMemo(
    () => sourceToEditedTime(workbenchTime, timelineCutIntervals),
    [timelineCutIntervals, workbenchTime],
  );
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

  useEffect(() => {
    if (active) return;
    const player = workbenchPlayerRef.current;
    if (player && !player.paused) player.pause();
    setWorkbenchPlaying(false);
  }, [active]);

  useEffect(() => {
    const returningToEditor = active && !wasActive.current;
    wasActive.current = active;
    if (!returningToEditor || !pid) return;
    let disposed = false;
    void Promise.all([asrStatus(), configShow()])
      .then(([readiness, config]) => {
        if (disposed) return;
        setAsrReadiness(readiness);
        setAgentConfigured(Boolean(
          config.llmEndpoint.trim() && config.llmModel.trim(),
        ));
      })
      .catch((error) => {
        if (!disposed) {
          setFeedback({ tone: "error", text: friendlyError(error, lang) });
        }
      });
    return () => {
      disposed = true;
    };
  }, [active, lang, pid]);

  const updateWorkbenchTime = useCallback((seconds: number) => {
    let next = seconds;
    if (doc && previewCuts && workbenchPlaying) {
      const playable = nextPlayableTime(seconds, timelineCutIntervals);
      if (playable > seconds + 0.001) {
        next = Math.min(doc.media.durationSeconds, playable + 0.001);
        if (workbenchPlayerRef.current) {
          workbenchPlayerRef.current.currentTime = next;
        }
      }
    }
    setWorkbenchTime(next);
  }, [doc, previewCuts, timelineCutIntervals, workbenchPlaying]);

  const reload = async (projectId: string, resetTab = true) => {
    exportCheckRun.current = false;
    setFinishItems(null);
    setExportPreflightReport(null);
    setExportPreflightUpdating(false);
    setAllowDraftExport(false);
    const [
      nextDoc,
      nextRows,
      nextStyle,
      nextEvidence,
      nextBroll,
      nextTitles,
      nextEditHistory,
      nextAudioMix,
      nextExportSettings,
      nextChapters,
    ] = await Promise.all([
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
      titleList(projectId).catch(() => []),
      editHistoryStatus(projectId).catch(() => EMPTY_EDIT_HISTORY),
      audioMixGet(projectId).catch(() => DEFAULT_AUDIO_MIX),
      exportSettingsGet(projectId).catch(() => DEFAULT_VIDEO_EXPORT_SETTINGS),
      chapterList(projectId).catch((error) => {
        setFeedback({
          tone: "error",
          text: lang === "zh"
            ? `章节数据无法加载，其他编辑仍可继续：${friendlyError(error, lang)}`
            : `Chapter data could not be loaded; other editing can continue: ${friendlyError(error, lang)}`,
        });
        return [];
      }),
    ]);
    if (activeProject.current !== projectId) return;
    setDoc(nextDoc);
    setSubtitleRows(nextRows);
    setSubtitleStyle(restoredStyleDraft(projectId, nextStyle));
    setSavedSubtitleStyle(nextStyle);
    setSpeakers(nextEvidence.speakers);
    setSpeakerEvidenceState(nextEvidence);
    setBrollOverview(nextBroll);
    setTitles(nextTitles);
    setEditHistory(nextEditHistory);
    setAudioMix(nextAudioMix);
    setVideoExportSettings(normalizeVideoExportSettings(nextExportSettings));
    setChapters(nextChapters);
    exportSettingsProject.current = projectId;
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
    exportCheckRun.current = false;
    setDoc(null);
    setProjectLoadError(null);
    setFeedback(null);
    setWorkbenchTime(0);
    setWorkbenchPlaying(false);
    workbenchPlayerRef.current?.pause();
    setAuditReport(null);
    setFinishItems(null);
    setExportPreflightReport(null);
    setExportPreflightUpdating(false);
    setAllowDraftExport(false);
    setTaskState(null);
    setOperation(null);
    setTranscriptionJob(null);
    setTranscriptionFailure(null);
    setConfirmRetranscription(false);
    setChapters([]);
    setVideoExportJob(null);
    setVersionHistory(null);
    setSpeakerEvidenceState({ speakers: [], turns: [], identified: false, unlabelled: 0 });
    setSpeakerPreview(null);
    setSpeakerAnalysisJob(null);
    setBrollOverview({ suggestions: [], accepted: [], errors: [] });
    setTitles([]);
    setAudioMix(DEFAULT_AUDIO_MIX);
    setEditHistory(EMPTY_EDIT_HISTORY);
    setBrollPreviewJob(null);
    setBrollPreviewPaths([]);
    exportSettingsProject.current = null;
    if (!pid) return;
    void Promise.all([
      reload(pid),
      taskStatus(pid).then(async (status) => {
        if (activeProject.current !== pid) return;
        setTaskState(status);
        if (status.kinds.some(
          (task) => task.pending > 0 && task.state !== "paused" && task.state !== "failed",
        )) {
          try {
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
          } catch (error) {
            if (activeProject.current !== pid) return;
            setFeedback({
              tone: "error",
              text: lang === "zh"
                ? `未完成的 AI 任务尚未恢复：${friendlyError(error, lang)}`
                : `Unfinished AI tasks were not resumed: ${friendlyError(error, lang)}`,
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
                ? `已恢复上次说话人分析提案：${status.preview.changed} 个字幕片段标签待确认。`
                : `Restored the previous speaker proposal: ${status.preview.changed} subtitle labels await review.`,
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
      brollPreviewStatus(pid, true)
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
        const message = friendlyError(error, lang);
        setProjectLoadError(message);
        setFeedback({ tone: "error", text: message });
      });
  }, [pid]);

  useEffect(() => {
    if (!pid || exportSettingsProject.current !== pid) return;
    const shouldRecheck = exportCheckRun.current;
    if (shouldRecheck) {
      setExportPreflightReport(null);
      setExportPreflightUpdating(true);
    }
    let disposed = false;
    const timeout = window.setTimeout(() => {
      void (async () => {
        try {
          await exportSettingsSet(pid, videoExportSettings);
          if (shouldRecheck) {
            const [items, preflight] = await Promise.all([
              finishCheckForExport(pid, videoExportSettings, lang),
              exportPreflight(pid, videoExportSettings),
            ]);
            if (disposed || activeProject.current !== pid) return;
            setFinishItems(items);
            setExportPreflightReport(preflight);
            setAllowDraftExport(false);
          }
        } catch (error) {
          if (disposed || activeProject.current !== pid) return;
          setFeedback({
            tone: "error",
            text:
              lang === "zh"
                ? `导出预设未能保存或重新检查：${friendlyError(error, lang)}`
                : `Export preset could not be saved or rechecked: ${friendlyError(error, lang)}`,
          });
        } finally {
          if (!disposed && activeProject.current === pid) {
            setExportPreflightUpdating(false);
          }
        }
      })();
    }, 350);
    return () => {
      disposed = true;
      window.clearTimeout(timeout);
    };
  }, [lang, pid, videoExportSettings]);

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
          const finished = status.kinds.find(
            (task) => watchedTasks.has(`${task.kind}:${task.lang || ""}`),
          );
          if (finished) {
            const failed = finished.failed > 0 || finished.state === "failed";
            setFeedback({
              tone: failed ? "error" : "success",
              text: failed
                ? lang === "zh"
                  ? `${taskLabel(finished.kind, lang)}已停止：${finished.failed} 个批次失败。${finished.lastError || "已完成的结果已经保存。"}`
                  : `${taskLabel(finished.kind, lang)} stopped: ${finished.failed} batches failed. ${finished.lastError || "Completed results were saved."}`
                : lang === "zh"
                  ? `${taskLabel(finished.kind, lang)}完成，结果已保存。`
                  : `${taskLabel(finished.kind, lang)} completed. Results were saved.`,
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
        const status = await brollPreviewStatus(pid, true);
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
              ? `分析完成：${status.preview.changed} 个字幕片段标签可能改变。项目尚未被修改。`
              : `Analysis complete: ${status.preview.changed} subtitle labels may change. The project is unchanged.`,
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
    if (projectLoadError) {
      return (
        <section className="editor-load-error" role="alert">
          <AlertIcon />
          <p className="eyebrow">{lang === "zh" ? "项目没有打开" : "Project did not open"}</p>
          <h2>{lang === "zh" ? "项目数据暂时无法读取" : "Project data is temporarily unavailable"}</h2>
          <p>{projectLoadError}</p>
          <small>
            {lang === "zh"
              ? "原始媒体和已保存编辑不会因这次加载失败而被删除。"
              : "The source media and saved edits are not deleted by this loading failure."}
          </small>
          <div>
            <button
              className="button-primary"
              onClick={() => {
                if (!pid) return;
                setProjectLoadError(null);
                setFeedback(null);
                void reload(pid).catch((error) => {
                  const message = friendlyError(error, lang);
                  setProjectLoadError(message);
                  setFeedback({ tone: "error", text: message });
                });
              }}
            >
              {lang === "zh" ? "重试打开" : "Try again"}
            </button>
            <button className="button-quiet" onClick={onOpenProjects}>
              {lang === "zh" ? "返回项目" : "Back to projects"}
            </button>
          </div>
        </section>
      );
    }
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
  const exportPreflightBlockers = exportPreflightReport?.items.filter(
    (item) => item.level === "blocker",
  ) ?? [];
  const hasExportCheck = finishItems !== null && exportPreflightReport !== null;
  const workflowReady = finishItems !== null && failedFinishItems.length === 0;
  const exportReady = workflowReady && exportPreflightReport?.ready === true;
  const workflowExportAllowed = workflowReady || allowDraftExport;
  const videoExportAllowed = workflowExportAllowed
    && exportPreflightReport?.ready === true;
  const subtitleExportAllowed = workflowExportAllowed
    && exportPreflightReport !== null
    && !exportPreflightBlockers.some((item) =>
      ["settings", "captions", "caption-state", "style", "timeline-data"].includes(item.code)
    );
  const finalCutExportAllowed = workflowExportAllowed
    && exportPreflightReport !== null
    && !exportPreflightBlockers.some((item) =>
      ["media", "timeline", "timeline-data", "broll", "titles"].includes(item.code)
    );
  const isVideoExporting = videoExportJob !== null
    && ["running", "cancelling"].includes(videoExportJob.state);
  const failedTasks = taskState?.kinds.reduce((sum, task) => sum + task.failed, 0) ?? 0;
  const stoppedTasks = taskState?.kinds.filter(
    (task) => task.state === "paused" || task.state === "failed",
  ).length ?? 0;
  const invalidateDeliveryCheck = () => {
    exportCheckRun.current = false;
    setFinishItems(null);
    setExportPreflightReport(null);
    setExportPreflightUpdating(false);
    setAllowDraftExport(false);
  };
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
    setConfirmRetranscription(false);
    setOperation("transcribe");
    setFeedback(null);
    setTranscriptionFailure(null);
    try {
      const readiness = await asrStatus();
      setAsrReadiness(readiness);
      if (!(readiness.selectedReady ?? readiness.ready)) {
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

  const startTranslation = (language: string, staleOnly: boolean) =>
    perform("translate", async () => {
      const result = await taskStart("translate", pid, language, staleOnly);
      setTaskState(await taskStatus(pid));
      setFeedback({
        tone: "info",
        text: result.pending > 0
          ? `${c.translating} · ${result.pending} ${lang === "zh" ? "个上下文批次" : "context batches"}`
          : lang === "zh"
            ? "没有缺失或过期的译文，当前翻译已是最新。"
            : "No missing or stale translations. This track is up to date.",
      });
    });

  const pauseTranslation = () =>
    perform("translate-pause", async () => {
      const result = await taskPause(pid, "translate");
      setTaskState(await taskStatus(pid));
      setFeedback({
        tone: "info",
        text: lang === "zh"
          ? `翻译已暂停；${result.queuedCalls} 个排队批次已停止，${result.inFlightCalls} 个在途请求完成后会安全保存。`
          : `Translation paused. ${result.queuedCalls} queued batch(es) stopped; ${result.inFlightCalls} in-flight request(s) will be saved safely.`,
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
    await performRecoverable(`subtitle-${id}`, async () => {
      const result = await subtitleUpdateMany(pid, [{ id, text }]);
      const updated = result.sentences[0];
      if (!updated) throw new Error(`subtitle ${id} was not found`);
      setDoc((current) => current ? {
        ...current,
        paragraphs: current.paragraphs.map((paragraph) => ({
          ...paragraph,
          sentences: paragraph.sentences.map((sentence) =>
            sentence.id === id ? updated : sentence
          ),
        })),
      } : current);
      setSubtitleRows((current) => current.map((row) =>
        row.id === id ? {
          ...row,
          text: updated.text,
          start: updated.words[0]?.start ?? row.start,
          end: updated.words[updated.words.length - 1]?.end ?? row.end,
        } : row
      ));
      await refreshEditHistory();
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "这句转写已保存。" : "This transcript line was saved.",
      });
    });
  };

  const saveSubtitles = async (updates: Array<{ id: string; text: string }>) => {
    if (updates.length === 0) return;
    await performRecoverable("subtitles", async () => {
      const result = await subtitleUpdateMany(pid, updates);
      const updated = new Map(result.sentences.map((sentence) => [sentence.id, sentence]));
      setDoc((current) => current ? {
        ...current,
        paragraphs: current.paragraphs.map((paragraph) => ({
          ...paragraph,
          sentences: paragraph.sentences.map((sentence) => {
            return updated.get(sentence.id) ?? sentence;
          }),
        })),
      } : current);
      setSubtitleRows((current) => current.map((row) => {
        const sentence = updated.get(row.id);
        return sentence ? {
          ...row,
          text: sentence.text,
          start: sentence.words[0]?.start ?? row.start,
          end: sentence.words[sentence.words.length - 1]?.end ?? row.end,
        } : row;
      }));
      await refreshEditHistory();
      setFeedback({
        tone: "success",
        text: lang === "zh"
          ? `已原子保存 ${updates.length} 条转写（${result.changed} 条有变化），可一次撤销。`
          : `Saved ${updates.length} transcript lines atomically (${result.changed} changed) in one undo step.`,
      });
    });
  };

  const saveChapters = async (next: ChapterInput[]) => {
    await performRecoverable("chapters", async () => {
      await chapterSetMany(pid, next);
      setChapters(await chapterList(pid));
      await refreshEditHistory();
      setFeedback({
        tone: "success",
        text: lang === "zh"
          ? `已保存 ${next.length} 个章节。`
          : `Saved ${next.length} chapter${next.length === 1 ? "" : "s"}.`,
      });
    });
  };

  const saveTranslation = async (language: string, id: string, text: string) => {
    await performRecoverable(`translation-${id}`, async () => {
      const changed = await translationSet(pid, language, id, text);
      if (!changed) throw new Error(`subtitle ${id} was not found`);
      setDoc((current) => current ? {
        ...current,
        translations: {
          ...current.translations,
          [language]: {
            ...(current.translations[language] || {}),
            [id]: {
              text,
              sourceText: current.paragraphs
                .flatMap((paragraph) => paragraph.sentences)
                .find((sentence) => sentence.id === id)?.text,
            },
          },
        },
      } : current);
      await refreshEditHistory();
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "这句译文已保存。" : "This translation was saved.",
      });
    });
  };

  const saveTranslations = async (
    language: string,
    updates: Array<{ id: string; text: string }>,
  ) => {
    if (updates.length === 0) return;
    await performRecoverable("translations", async () => {
      await translationSetMany(pid, language, updates);
      const updateMap = new Map(updates.map((update) => [update.id, update.text]));
      setDoc((current) => current ? {
        ...current,
        translations: {
          ...current.translations,
          [language]: {
            ...(current.translations[language] || {}),
            ...Object.fromEntries(
              current.paragraphs
                .flatMap((paragraph) => paragraph.sentences)
                .filter((sentence) => updateMap.has(sentence.id))
                .map((sentence) => [
                  sentence.id,
                  {
                    text: updateMap.get(sentence.id) ?? "",
                    sourceText: sentence.text,
                  },
                ]),
            ),
          },
        },
      } : current);
      await refreshEditHistory();
      setFeedback({
        tone: "success",
        text: lang === "zh"
          ? `已保存 ${updates.length} 条译文。`
          : `Saved ${updates.length} translation${updates.length === 1 ? "" : "s"}.`,
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
      const changed = await subtitleVisibility(pid, id, hidden);
      if (!changed) throw new Error(`subtitle ${id} was not found`);
      setSubtitleRows((current) => current.map((row) =>
        row.id === id ? { ...row, hidden } : row
      ));
      await refreshEditHistory();
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

  const updateSubtitleTiming = async (id: string, start: number, end: number) => {
    await perform(`timing-${id}`, async () => {
      const changed = await setSubtitleTiming(pid, id, start, end);
      if (!changed) return;
      await reload(pid, false);
      setFeedback({
        tone: "success",
        text: lang === "zh"
          ? `字幕时码已更新为 ${start.toFixed(2)}s–${end.toFixed(2)}s，可撤销。`
          : `Cue timing updated to ${start.toFixed(2)}s–${end.toFixed(2)}s and can be undone.`,
      });
    });
  };

  const undoEditorEdit = async () => {
    await perform("undo", async () => {
      const action = await editUndo(pid);
      setEditHistory(action.status);
      if (!action.changed) return;
      await reload(pid, false);
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "已撤销上一项编辑。" : "Undid the last edit.",
      });
    });
  };

  const redoEditorEdit = async () => {
    await perform("redo", async () => {
      const action = await editRedo(pid);
      setEditHistory(action.status);
      if (!action.changed) return;
      await reload(pid, false);
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "已重新应用编辑。" : "Redid the edit.",
      });
    });
  };

  const saveStyle = async (style: SubtitleStyle) => {
    setOperation("style");
    setFeedback(null);
    try {
      await styleSet(pid, style);
      const saved = await styleGet(pid);
      setSubtitleStyle(saved);
      setSavedSubtitleStyle(saved);
      await refreshEditHistory();
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

  const resetStylePreview = () => {
    if (savedSubtitleStyle) setSubtitleStyle(savedSubtitleStyle);
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

  const refreshTitles = async () => {
    setTitles(await titleList(pid));
  };

  const refreshEditHistory = async () => {
    invalidateDeliveryCheck();
    try {
      setEditHistory(await editHistoryStatus(pid));
    } catch {
      // A history refresh must never turn a completed, durable edit into a UI failure.
    }
  };

  const relinkProjectMediaAt = async (path: string) => {
    await performRecoverable("media-relink", async () => {
      await projectMediaRelink(pid, path);
      await Promise.all([reload(pid, false), refreshEditHistory()]);
      setFeedback({
        tone: "success",
        text: lang === "zh"
          ? "项目媒体已重新连接；转写稿、说话人和时间线编辑均已保留。"
          : "Project media reconnected. Transcript, speakers, and timeline edits were preserved.",
      });
    });
  };

  const relinkProjectMedia = async () => {
    const path = await pickMediaFile();
    if (!path) return;
    await relinkProjectMediaAt(path);
  };

  const fixExportBlocker = (code: string) => {
    if (code === "media") {
      void relinkProjectMedia();
      return;
    }
    if (code === "captions") {
      setActiveTab("translate");
      return;
    }
    if (code === "caption-state") {
      setActiveTab("subtitle");
      return;
    }
    if (code === "style") {
      setActiveTab("style");
      return;
    }
    if (code === "timeline" || code === "timeline-data" || code === "titles" || code === "audio") {
      setActiveTab("timeline");
      return;
    }
    if (code === "broll") {
      setActiveTab("broll");
      return;
    }
    if (code === "settings" || code === "encoder") {
      window.requestAnimationFrame(() => {
        const field = document.querySelector<HTMLSelectElement>(
          ".video-export-settings select",
        );
        field?.scrollIntoView({ block: "center", behavior: "smooth" });
        field?.focus();
      });
    }
  };

  const pickBrollAsset = () => performRecoverable("broll-pick", pickBrollFile);

  const acceptBrollSuggestion = async (suggestion: BrollSuggestion) => {
    const file = await pickBrollAsset();
    if (!file) return false;
    await performRecoverable("broll-add", async () => {
      await brollAcceptSuggestion(pid, suggestion, file);
      await Promise.all([refreshBroll(), refreshEditHistory()]);
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
      await Promise.all([refreshBroll(), refreshEditHistory()]);
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "素材已加入 B-roll 轨道。" : "Added the asset to the B-roll track.",
      });
    });
  };

  const updateBroll = async (id: string, input: BrollPlacementInput) => {
    await performRecoverable("broll-update", async () => {
      await brollUpdate(pid, id, input);
      await Promise.all([refreshBroll(), refreshEditHistory()]);
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "B-roll 调整已保存。" : "Saved the B-roll changes.",
      });
    });
  };

  const removeBroll = async (id: string) => {
    await performRecoverable("broll-remove", async () => {
      if (!await brollRemove(pid, id)) throw new Error(`B-roll ${id} was not found`);
      await Promise.all([refreshBroll(), refreshEditHistory()]);
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "已从成片中移除这段素材。" : "Removed the asset from the edit.",
      });
    });
  };

  const addTitle = async (input: TitleClipInput) => {
    await performRecoverable("title-add", async () => {
      await titleAdd(pid, input);
      await Promise.all([refreshTitles(), refreshEditHistory()]);
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "标题已加入时间线。" : "Added the title to the timeline.",
      });
    });
  };

  const updateTitle = async (id: string, input: TitleClipInput) => {
    await performRecoverable("title-update", async () => {
      await titleUpdate(pid, id, input);
      await Promise.all([refreshTitles(), refreshEditHistory()]);
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "标题调整已保存。" : "Saved the title changes.",
      });
    });
  };

  const removeTitle = async (id: string) => {
    await performRecoverable("title-remove", async () => {
      if (!await titleRemove(pid, id)) throw new Error(`title ${id} was not found`);
      await Promise.all([refreshTitles(), refreshEditHistory()]);
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "标题已移除。" : "Removed the title.",
      });
    });
  };

  const updateAudioMix = async (mix: AudioMix) => {
    await performRecoverable("audio-mix", async () => {
      const saved = await audioMixSet(pid, mix);
      setAudioMix(saved);
      await refreshEditHistory();
      setFeedback({
        tone: "success",
        text: mix.voiceEnhance || mix.normalizeLoudness
          ? lang === "zh"
            ? "音频设置已保存；音量与淡化用于节目监看，对白增强与响度处理会在导出时执行。"
            : "Audio settings saved. Gain and fades are monitored; dialogue and loudness processing run on export."
          : lang === "zh"
            ? "音频设置已保存，并会应用到节目监看和视频导出。"
            : "Audio settings saved for the program monitor and video export.",
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
      if (added > 0) await refreshEditHistory();
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
      const [items, preflight] = await Promise.all([
        finishCheckForExport(pid, videoExportSettings, lang),
        exportPreflight(pid, videoExportSettings),
      ]);
      setFinishItems(items);
      setExportPreflightReport(preflight);
      exportCheckRun.current = true;
      setAllowDraftExport(false);
    });

  const exportReasonCodes = new Set(
    (finishItems ?? []).flatMap((item) => item.reasonCodes ?? []),
  );
  const needsCaptionFit = exportReasonCodes.has("target-width")
    || exportReasonCodes.has("target-width-aim")
    || (finishItems ?? []).some((item) =>
      item.blockers.some((message) =>
        message.includes("太长")
        || message.includes("too long")
        || message.includes("hard capacity")
        || message.includes("硬上限"),
      ),
    );
  const needsVersionCommit = exportReasonCodes.has("version-uncommitted")
    || (finishItems ?? []).some((item) =>
      item.blockers.some((message) =>
        message.includes("版本")
        || message.includes("version")
        || message.includes("not committed"),
      ),
    );
  const fitLanguage = previewTranslationLanguage
    || (doc
      ? Object.keys(doc.translations).find((key) => Object.keys(doc.translations[key] || {}).length > 0)
      : null)
    || "zh";

  const runAutoFitCaptions = () =>
    perform("auto-fit", async () => {
      const report = await translationAutoFit(pid, fitLanguage, null);
      await reload(pid, false);
      const [items, preflight] = await Promise.all([
        finishCheckForExport(pid, videoExportSettings, lang),
        exportPreflight(pid, videoExportSettings),
      ]);
      setFinishItems(items);
      setExportPreflightReport(preflight);
      exportCheckRun.current = true;
      setFeedback({
        tone: report.remainingHard > 0 ? "error" : "success",
        text: lang === "zh"
          ? report.fixed > 0
            ? `已拆开 ${report.fixed} 行字幕${
              report.remainingHard > 0
                ? `，仍有 ${report.remainingHard} 行需要手动缩短`
                : "，可重新导出"
            }。`
            : report.remainingHard > 0
              ? `仍有 ${report.remainingHard} 行过长，请在翻译页手动缩短。`
              : "字幕长度已符合要求。"
          : report.fixed > 0
            ? `Split ${report.fixed} caption line(s)${
              report.remainingHard > 0
                ? `; ${report.remainingHard} still need a manual shorten`
                : "; ready to export again"
            }.`
            : report.remainingHard > 0
              ? `${report.remainingHard} line(s) still too long — edit them in Translate.`
              : "Captions already fit.",
      });
    });

  const runSaveVersionAndContinue = () =>
    perform("version-commit", async () => {
      const stamp = new Date().toISOString().slice(0, 19).replace("T", " ");
      await versionCommit(
        pid,
        lang === "zh" ? `导出前快照 ${stamp}` : `Pre-export snapshot ${stamp}`,
        lang === "zh" ? "导出检查自动保存" : "Saved from export check",
      );
      setVersionHistory(await versionList(pid));
      const [items, preflight] = await Promise.all([
        finishCheckForExport(pid, videoExportSettings, lang),
        exportPreflight(pid, videoExportSettings),
      ]);
      setFinishItems(items);
      setExportPreflightReport(preflight);
      exportCheckRun.current = true;
      setFeedback({
        tone: "success",
        text: lang === "zh" ? "版本已保存，可继续导出。" : "Version saved — you can continue exporting.",
      });
    });

  const restoreCut = (cutId: string) =>
    perform(`restore-${cutId}`, async () => {
      const changed = await cutRestore(pid, cutId);
      setCuts(await cutList(pid));
      if (changed) await refreshEditHistory();
    });

  const removeTimelineCues = async (cueIds: string[]) => {
    await perform(`cut-${cueIds.join("-")}`, async () => {
      const added = await cutManualMany(pid, cueIds);
      if (added === 0) return;
      setCuts(await cutList(pid));
      await refreshEditHistory();
      setFeedback({
        tone: "success",
        text: lang === "zh"
          ? `已将 ${cueIds.length} 段字幕对应的画面和声音作为一次编辑移除，可一次撤销。`
          : `${cueIds.length} selected cue(s) will be removed as one edit and can be undone once.`,
      });
    });
  };

  const runSubtitleExport = () =>
    perform("export-subtitles", async () => {
      const saved = await exportSettingsSet(pid, videoExportSettings);
      setVideoExportSettings(saved);
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
      const saved = await exportSettingsSet(pid, videoExportSettings);
      setVideoExportSettings(saved);
      setVideoExportJob(await videoExportStart(pid, saved));
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

  const primaryTabs: Array<{ id: Tab; label: string }> = hasTranscript
    ? [
      { id: "transcript", label: c.transcript },
      { id: "subtitle", label: lang === "zh" ? "字幕" : "Subtitle" },
      { id: "translate", label: c.translate },
      { id: "style", label: c.style },
    ]
    : [{ id: "setup", label: c.setup }];
  const toolTabs: Array<{ id: Tab; label: string; description: string }> = [
    {
      id: "setup",
      label: lang === "zh" ? "转写设置" : "Transcription setup",
      description: lang === "zh" ? "检查模型或安全地重新转写" : "Check models or safely transcribe again",
    },
    {
      id: "speakers",
      label: lang === "zh" ? "说话人" : "Speakers",
      description: lang === "zh" ? "识别、校对与合并" : "Identify, review, and merge",
    },
    {
      id: "chapters",
      label: lang === "zh" ? "章节" : "Chapters",
      description: lang === "zh" ? "生成与整理章节" : "Generate and organize chapters",
    },
    {
      id: "timeline",
      label: c.timeline,
      description: lang === "zh" ? "检查剪切与节奏" : "Review cuts and pacing",
    },
    {
      id: "broll",
      label: c.broll,
      description: lang === "zh" ? "添加补充画面" : "Add supporting visuals",
    },
  ];
  const activeTool = toolTabs.find((tab) => tab.id === activeTab);

  return (
    <section
      className={`editor-view${hasTranscript ? " has-transcript" : ""}${previewExpanded ? " preview-expanded" : ""}${timelineCollapsed ? " timeline-collapsed" : ""}${resizingPanes ? " resizing-panes" : ""}`}
      style={{ "--inspector-width": `${inspectorPercent}%` } as CSSProperties}
    >
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
            className={activeTab === "properties" ? "active" : ""}
            onClick={() => setActiveTab("properties")}
          >
            {lang === "zh" ? "项目" : "Project"}
          </button>
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
            audioMix={audioMix}
            broll={previewBrollOverview}
            exportSettings={videoExportSettings}
            currentTime={workbenchTime}
            doc={doc}
            expanded={previewExpanded}
            lang={lang}
            programDuration={programDuration}
            programTime={programTime}
        playerRef={workbenchPlayerRef}
        rows={previewRows}
        subtitleStyle={subtitleStyle}
        titles={titles}
        onPlayingChange={setWorkbenchPlaying}
        onRelinkMedia={relinkProjectMedia}
        onRelinkMediaPath={relinkProjectMediaAt}
        onTimeChange={updateWorkbenchTime}
        onToggleExpanded={() => setPreviewExpanded((value) => !value)}
        onUpdateBroll={updateBroll}
        onUpdateTitle={updateTitle}
      />

      <div
        aria-label={lang === "zh" ? "调整视频和编辑器宽度" : "Resize monitor and editor panes"}
        aria-orientation="vertical"
        aria-valuemax={58}
        aria-valuemin={32}
        aria-valuenow={inspectorPercent}
        className="editor-pane-resizer"
        onKeyDown={(event) => {
          if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
          event.preventDefault();
          setInspectorPercent((value) => Math.min(
            58,
            Math.max(32, value + (event.key === "ArrowLeft" ? 2 : -2)),
          ));
        }}
        onPointerDown={(event) => {
          event.currentTarget.setPointerCapture(event.pointerId);
          setResizingPanes(true);
        }}
        onPointerMove={(event) => {
          if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
          const bounds = event.currentTarget.parentElement?.getBoundingClientRect();
          if (!bounds || bounds.width <= 0) return;
          const next = ((bounds.right - event.clientX) / bounds.width) * 100;
          setInspectorPercent(Math.min(58, Math.max(32, next)));
        }}
        onPointerUp={(event) => {
          if (event.currentTarget.hasPointerCapture(event.pointerId)) {
            event.currentTarget.releasePointerCapture(event.pointerId);
          }
          setResizingPanes(false);
        }}
        role="separator"
        tabIndex={0}
      />

      <nav className="editor-tabs" aria-label={lang === "zh" ? "编辑主流程" : "Primary editor workflow"}>
        {primaryTabs.map((tab) => (
          <button
            aria-current={activeTab === tab.id ? "page" : undefined}
            className={`${activeTab === tab.id ? "active " : ""}editor-tab-${tab.id} editor-tab-primary`}
            key={tab.id}
            onClick={() => {
              setFeedback(null);
              setToolsMenuOpen(false);
              setActiveTab(tab.id);
            }}
          >
            {tab.label}
          </button>
        ))}
        {hasTranscript && (
          <div className="editor-tools-menu">
            <button
              aria-label={activeTool
                ? lang === "zh"
                  ? `当前工具：${activeTool.label}，打开更多编辑工具`
                  : `Current tool: ${activeTool.label}. Open more editing tools`
                : lang === "zh" ? "打开更多编辑工具" : "Open more editing tools"}
              aria-expanded={toolsMenuOpen}
              aria-haspopup="menu"
              className={activeTool ? "active" : ""}
              onKeyDown={(event) => {
                if (event.key !== "ArrowDown") return;
                event.preventDefault();
                setToolsMenuOpen(true);
              }}
              onClick={() => setToolsMenuOpen((open) => !open)}
              ref={toolsButtonRef}
              type="button"
            >
              <span>{activeTool?.label || (lang === "zh" ? "工具" : "Tools")}</span>
              <span aria-hidden="true">⌄</span>
            </button>
            {toolsMenuOpen && <div
              onKeyDown={(event) => {
                if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
                event.preventDefault();
                const items = Array.from(
                  event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'),
                );
                const index = items.indexOf(document.activeElement as HTMLButtonElement);
                const offset = event.key === "ArrowDown" ? 1 : -1;
                items[(index + offset + items.length) % items.length]?.focus();
              }}
              ref={toolsMenuRef}
              role="menu"
            >
              {toolTabs.map((tab) => (
                <button
                  aria-current={activeTab === tab.id ? "page" : undefined}
                  key={tab.id}
                  onClick={() => {
                    setFeedback(null);
                    setActiveTab(tab.id);
                    setToolsMenuOpen(false);
                  }}
                  role="menuitem"
                >
                  <span>{tab.label}</span>
                  <small>{tab.description}</small>
                </button>
              ))}
            </div>}
          </div>
        )}
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
            {asrReadiness && !selectedAsrReady && (
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
              disabled={operation !== null || asrReadiness === null}
              onClick={() => {
                if (!selectedAsrReady) {
                  onOpenSettings();
                } else if (hasTranscript) {
                  setConfirmRetranscription(true);
                } else {
                  void startTranscription();
                }
              }}
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
                  {selectedAsrReady ? <PlayIcon /> : null}
                  {asrReadiness === null
                    ? c.checkingAsr
                    : !selectedAsrReady
                      ? c.prepareAsr
                      : transcriptionFailure
                        ? c.retry
                        : hasTranscript
                          ? (lang === "zh" ? "重新转写" : "Transcribe again")
                          : c.start}
                </>
              )}
            </button>
            {confirmRetranscription && operation === null && (
              <div className="retranscription-confirm" role="alert">
                <strong>
                  {lang === "zh" ? "重新转写会替换当前转写稿" : "Retranscription replaces the current transcript"}
                </strong>
                <p>
                  {lang === "zh"
                    ? "开始前会自动保存完整恢复版本。旧译文、说话人、隐藏字幕、剪切和 AI 分析会随新时码重置；字幕样式、标题、音频设置和按时间放置的 B-roll 会保留。"
                    : "A complete recovery version is saved first. Translations, speakers, hidden cues, cuts, and AI analysis reset with the new timing; style, titles, audio settings, and time-based B-roll remain."}
                </p>
                <div>
                  <button
                    className="button-quiet"
                    onClick={() => setConfirmRetranscription(false)}
                  >
                    {lang === "zh" ? "取消" : "Cancel"}
                  </button>
                  <button
                    className="button-danger"
                    onClick={() => void startTranscription()}
                  >
                    {lang === "zh" ? "保存恢复点并重新转写" : "Save recovery point & retranscribe"}
                  </button>
                </div>
              </div>
            )}
            {transcriptionJob ? (
              <div className="transcription-progress" aria-live="polite">
                <div>
                  <strong>{transcriptionPhaseLabel(transcriptionJob.phase, lang)}</strong>
                  <span>{transcriptionJob.progress}%</span>
                </div>
                <progress max={100} value={transcriptionJob.progress} />
                {transcriptionJob.device && (
                  <small className="pipeline-resources">
                    {transcriptionJob.device === "cloud"
                      ? (lang === "zh" ? "云端转写" : "Cloud transcription")
                      : "MLX · Metal"}
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
                <PipelineFreshness
                  state={transcriptionJob.state}
                  phase={transcriptionJob.phase}
                  updatedAt={transcriptionJob.updatedAt}
                  lang={lang}
                />
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
          <ol className="setup-workflow" aria-label={lang === "zh" ? "项目流程" : "Project workflow"}>
            <li className="done">
              <span>1</span>
              <div>
                <strong>{lang === "zh" ? "媒体已导入" : "Media imported"}</strong>
                <small>{lang === "zh" ? "原文件保持在原位置" : "The source stays in place"}</small>
              </div>
            </li>
            <li className={hasTranscript ? "done" : "current"} aria-current={!hasTranscript ? "step" : undefined}>
              <span>2</span>
              <div>
                <strong>{lang === "zh" ? "生成转写与时码" : "Create transcript & timing"}</strong>
                <small>
                  {hasTranscript
                    ? lang === "zh" ? "已完成，可安全重新转写" : "Complete; safe to run again"
                    : lang === "zh" ? "确认环境后由你开始" : "You start after setup is ready"}
                </small>
              </div>
            </li>
            <li className={hasTranscript ? "current" : ""} aria-current={hasTranscript ? "step" : undefined}>
              <span>3</span>
              <div>
                <strong>{lang === "zh" ? "编辑（项目模式）" : "Edit (project mode)"}</strong>
                <small>
                  {lang === "zh"
                    ? "转写完成后即可编辑；不会自动导出"
                    : "Edit-ready after transcription; export is never automatic"}
                </small>
              </div>
            </li>
            <li>
              <span>4</span>
              <div>
                <strong>{lang === "zh" ? "交付检查并导出" : "Delivery check & export"}</strong>
                <small>
                  {lang === "zh"
                    ? "在导出页主动运行 finish-check"
                    : "Run finish-check on the export tab when ready"}
                </small>
              </div>
            </li>
          </ol>
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
            drafts={transcriptDrafts}
            duration={doc.media.durationSeconds}
            isPlaying={workbenchPlaying}
            lang={lang}
            mode="transcript"
            nextCueById={nextCueById}
            rows={subtitleRows}
            wordsByCue={wordsByCue}
            onDraftsChange={onTranscriptDraftsChange}
            onMerge={mergeSubtitleLines}
            onReplace={replaceSubtitles}
            onSave={saveSubtitle}
            onSaveMany={saveSubtitles}
            onSeek={seekWorkbench}
            onSplit={splitSubtitleLine}
            onTiming={updateSubtitleTiming}
            onVisibility={changeSubtitleVisibility}
          />
        </div>
      )}

      {activeTab === "subtitle" && (
        <div className="subtitle-layout">
          <TranscriptEditor
            busy={operation !== null}
            currentTime={workbenchTime}
            drafts={transcriptDrafts}
            duration={doc.media.durationSeconds}
            isPlaying={workbenchPlaying}
            lang={lang}
            mode="subtitle"
            nextCueById={nextCueById}
            rows={subtitleRows}
            wordsByCue={wordsByCue}
            onDraftsChange={onTranscriptDraftsChange}
            onMerge={mergeSubtitleLines}
            onReplace={replaceSubtitles}
            onSave={saveSubtitle}
            onSaveMany={saveSubtitles}
            onSeek={seekWorkbench}
            onSplit={splitSubtitleLine}
            onTiming={updateSubtitleTiming}
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
          onPause={pauseTranslation}
          onLanguageChange={setPreviewTranslationLanguage}
          onDraftsChange={onTranslationDraftsChange}
          onSave={saveTranslation}
          onSaveMany={saveTranslations}
          onSeek={seekWorkbench}
          onStart={startTranslation}
          drafts={translationDrafts}
        />
      )}

      {activeTab === "chapters" && (
        <ChapterWorkspace
          busy={operation !== null}
          chapters={chapters}
          configured={agentConfigured}
          currentTime={workbenchTime}
          drafts={chapterDrafts}
          lang={lang}
          rows={subtitleRows}
          status={taskState?.kinds.find((task) => task.kind === "chapters") ?? null}
          onDraftsChange={onChapterDraftsChange}
          onGenerate={() => startEnhancement("chapters", null)}
          onOpenSettings={onOpenSettings}
          onSave={saveChapters}
          onSeek={seekWorkbench}
        />
      )}

      {activeTab === "style" && savedSubtitleStyle && subtitleStyle && (
        <StyleWorkspace
          busy={operation === "style"}
          lang={lang}
          savedStyle={savedSubtitleStyle}
          style={subtitleStyle}
          onPreview={setSubtitleStyle}
          onReset={resetStylePreview}
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
          drafts={brollDrafts}
          lang={lang}
          newPlacement={newBrollPlacement}
          overview={brollOverview}
          previewJob={brollPreviewJob}
          previewPaths={brollPreviewPaths}
          onAcceptSuggestion={acceptBrollSuggestion}
          onAdd={addBroll}
          onCancelPreview={cancelBrollPreview}
          onDraftsChange={setBrollDrafts}
          onNewPlacementChange={setNewBrollPlacement}
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
                <ReviewFindings findings={auditReport.findings} lang={lang} />
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
          <section className="export-project-mode" role="note">
            <p className="eyebrow">{c.projectModeTitle}</p>
            <p>{c.projectModeHint}</p>
          </section>
          <section className={`export-preflight ${exportPreflightUpdating ? "checking" : exportReady ? "ready" : failedFinishItems.length > 0 || exportPreflightBlockers.length > 0 ? "blocked" : "unchecked"}`}>
            <header>
              <div>
                <p className="eyebrow">{c.exportCheckTitle}</p>
                <h3>
                  {exportPreflightUpdating
                    ? lang === "zh" ? "正在按新规格重新检查…" : "Rechecking the new settings…"
                    : !hasExportCheck
                    ? c.exportUnchecked
                    : exportReady
                      ? c.exportReady
                      : c.exportBlocked}
                </h3>
              </div>
              <button
                className={!hasExportCheck ? "button-primary" : "button-quiet"}
                disabled={operation !== null || exportPreflightUpdating}
                onClick={runFinishCheck}
              >
                {operation === "finish" || exportPreflightUpdating
                  ? <span className="spinner" />
                  : null}
                {!hasExportCheck
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
                      <strong>{finishCheckLabel(item.code, lang)}</strong>
                      {item.blockers.map((blocker, index) => (
                        <small key={`${item.ordinal}-${index}`}>{blocker}</small>
                      ))}
                    </span>
                  </li>
                ))}
              </ul>
            )}
            {(needsCaptionFit || needsVersionCommit) && (
              <div className="export-fix-actions">
                {needsCaptionFit && (
                  <button
                    className="button-primary"
                    disabled={operation !== null}
                    onClick={runAutoFitCaptions}
                    type="button"
                  >
                    {operation === "auto-fit" ? <span className="spinner" /> : null}
                    {operation === "auto-fit" ? c.autoFitting : c.autoFitCaptions}
                  </button>
                )}
                {needsVersionCommit && (
                  <button
                    className={needsCaptionFit ? "button-quiet" : "button-primary"}
                    disabled={operation !== null}
                    onClick={runSaveVersionAndContinue}
                    type="button"
                  >
                    {operation === "version-commit" ? <span className="spinner" /> : null}
                    {operation === "version-commit" ? c.savingVersion : c.saveVersionContinue}
                  </button>
                )}
              </div>
            )}
            {exportPreflightReport && exportPreflightBlockers.length > 0 && (
              <ul className="export-preflight-specific">
                {exportPreflightBlockers.map((item) => {
                  const fixLabel = exportPreflightFixLabel(item.code, lang);
                  return (
                    <li key={item.code}>
                      <AlertIcon />
                      <span>
                        <strong>{exportPreflightLabel(item.code, lang)}</strong>
                        <small>
                          {exportPreflightMessage(item, exportPreflightReport, lang)}
                        </small>
                      </span>
                      {fixLabel && (
                        <button
                          className="button-quiet export-preflight-fix"
                          disabled={operation !== null}
                          onClick={() => fixExportBlocker(item.code)}
                        >
                          {fixLabel}
                        </button>
                      )}
                    </li>
                  );
                })}
              </ul>
            )}
            {exportPreflightReport && (
              <>
                <div className="export-preflight-summary">
                  <span>
                    <strong>{Math.round(exportPreflightReport.summary.durationSeconds)}s</strong>
                    {lang === "zh" ? "成片" : "edited"}
                  </span>
                  <span>
                    <strong>{exportPreflightReport.summary.visibleCaptions}</strong>
                    {lang === "zh" ? "行字幕" : "captions"}
                  </span>
                  <span>
                    <strong>{exportPreflightReport.summary.brollItems}</strong>
                    B-roll
                  </span>
                  <span>
                    <strong>{exportPreflightReport.summary.titleItems}</strong>
                    {lang === "zh" ? "个标题" : "titles"}
                  </span>
                  <span>
                    <strong>
                      {exportPreflightReport.summary.estimatedMinMb}–{exportPreflightReport.summary.estimatedMaxMb} MB
                    </strong>
                    {lang === "zh" ? "预计体积" : "estimated"}
                  </span>
                </div>
                {exportPreflightReport.items.some((item) => item.level === "warning") && (
                  <div className="export-preflight-warnings">
                    {exportPreflightReport.items
                      .filter((item) => item.level === "warning")
                      .map((item) => (
                        <small key={item.code}>
                          <strong>{exportPreflightLabel(item.code, lang)}</strong>
                          {exportPreflightMessage(item, exportPreflightReport, lang)}
                        </small>
                      ))}
                  </div>
                )}
              </>
            )}
            {!workflowReady && finishItems !== null && exportPreflightReport?.ready && (
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
            <fieldset className="video-export-settings" disabled={isVideoExporting || operation !== null}>
              <legend>{lang === "zh" ? "视频交付规格" : "Video delivery settings"}</legend>
              <div className="video-export-settings-grid">
                <label>
                  <span>{lang === "zh" ? "容器" : "Container"}</span>
                  <select
                    value={videoExportSettings.container}
                    onChange={(event) => {
                      const container = event.target.value as VideoExportSettings["container"];
                      setVideoExportSettings((current) => ({
                        ...current,
                        container,
                        videoCodec:
                          container === "mp4" && current.videoCodec === "prores"
                            ? "h264"
                            : current.videoCodec,
                        audioCodec:
                          container === "mp4" && current.audioCodec === "pcm"
                            ? "aac"
                            : current.audioCodec,
                      }));
                    }}
                  >
                    <option value="mp4">MP4 · {lang === "zh" ? "通用交付" : "universal delivery"}</option>
                    <option value="mov">MOV · {lang === "zh" ? "后期制作" : "post-production"}</option>
                  </select>
                </label>
                <label>
                  <span>{lang === "zh" ? "视频编码" : "Video codec"}</span>
                  <select
                    value={videoExportSettings.videoCodec}
                    onChange={(event) => {
                      const videoCodec = event.target.value as VideoExportSettings["videoCodec"];
                      setVideoExportSettings((current) => ({
                        ...current,
                        videoCodec,
                        container: videoCodec === "prores" ? "mov" : current.container,
                      }));
                    }}
                  >
                    <option value="h264">H.264 · {lang === "zh" ? "兼容性最好" : "best compatibility"}</option>
                    <option value="hevc">HEVC · {lang === "zh" ? "更小文件" : "smaller file"}</option>
                    <option value="prores">ProRes 422 HQ · {lang === "zh" ? "剪辑母版" : "editing master"}</option>
                  </select>
                </label>
                <label>
                  <span>{lang === "zh" ? "画布比例" : "Canvas ratio"}</span>
                  <select
                    value={videoExportSettings.aspectRatio}
                    onChange={(event) =>
                      setVideoExportSettings((current) => ({
                        ...current,
                        aspectRatio: event.target.value as VideoExportSettings["aspectRatio"],
                      }))
                    }
                  >
                    <option value="source">{lang === "zh" ? "跟随源视频" : "Match source"}</option>
                    <option value="16:9">16:9 · {lang === "zh" ? "横屏" : "landscape"}</option>
                    <option value="9:16">9:16 · {lang === "zh" ? "竖屏短视频" : "vertical short video"}</option>
                    <option value="1:1">1:1 · {lang === "zh" ? "方形" : "square"}</option>
                    <option value="4:5">4:5 · {lang === "zh" ? "竖屏信息流" : "portrait feed"}</option>
                  </select>
                </label>
                <label>
                  <span>{lang === "zh" ? "分辨率" : "Resolution"}</span>
                  <select
                    value={videoExportSettings.resolution}
                    onChange={(event) =>
                      setVideoExportSettings((current) => ({
                        ...current,
                        resolution: event.target.value as VideoExportSettings["resolution"],
                      }))
                    }
                  >
                    <option value="source">{lang === "zh" ? "跟随源视频" : "Match source"}</option>
                    <option value="720p">720p HD</option>
                    <option value="1080p">1080p Full HD</option>
                    <option value="4k">4K UHD</option>
                  </select>
                </label>
                <label>
                  <span>{lang === "zh" ? "源画面适配" : "Source framing"}</span>
                  <select
                    disabled={videoExportSettings.aspectRatio === "source"}
                    value={videoExportSettings.canvasFit}
                    onChange={(event) =>
                      setVideoExportSettings((current) => ({
                        ...current,
                        canvasFit: event.target.value as VideoExportSettings["canvasFit"],
                      }))
                    }
                  >
                    <option value="contain">{lang === "zh" ? "完整显示 · 必要时留黑" : "Fit · letterbox if needed"}</option>
                    <option value="cover">{lang === "zh" ? "填满画布 · 居中裁切" : "Fill · center crop"}</option>
                  </select>
                </label>
                <label>
                  <span>{lang === "zh" ? "字幕" : "Subtitles"}</span>
                  <select
                    value={videoExportSettings.subtitleMode}
                    onChange={(event) =>
                      setVideoExportSettings((current) => ({
                        ...current,
                        subtitleMode: event.target.value as VideoExportSettings["subtitleMode"],
                      }))
                    }
                  >
                    <option value="burn">{lang === "zh" ? "烧录到画面" : "Burn into picture"}</option>
                    <option value="soft">{lang === "zh" ? "可开关软字幕" : "Switchable soft track"}</option>
                    <option value="none">{lang === "zh" ? "不包含字幕" : "No subtitles"}</option>
                  </select>
                </label>
                <label>
                  <span>{lang === "zh" ? "字幕内容" : "Caption content"}</span>
                  <select
                    disabled={videoExportSettings.subtitleMode === "none"}
                    value={exportCaptionTrack}
                    onChange={(event) => {
                      const [kind, ...languageParts] = event.target.value.split(":");
                      const subtitleLanguage = languageParts.join(":") || null;
                      setVideoExportSettings((current) => ({
                        ...current,
                        subtitleLanguage,
                        bilingualSubtitles: kind === "bilingual",
                      }));
                    }}
                  >
                    <option value="source">
                      {lang === "zh" ? `原文 · ${doc.meta.language || "自动"}` : `Original · ${doc.meta.language || "auto"}`}
                    </option>
                    {videoExportSettings.subtitleLanguage
                      && !translationLanguages.includes(videoExportSettings.subtitleLanguage) && (
                      <option value={exportCaptionTrack}>
                        {lang === "zh"
                          ? `译文不可用 · ${videoExportSettings.subtitleLanguage}`
                          : `Translation unavailable · ${videoExportSettings.subtitleLanguage}`}
                      </option>
                    )}
                    {translationLanguages.map((language) => (
                      <option key={`translation-${language}`} value={`translation:${language}`}>
                        {lang === "zh" ? `仅译文 · ${language}` : `Translation only · ${language}`}
                      </option>
                    ))}
                    {translationLanguages.map((language) => (
                      <option key={`bilingual-${language}`} value={`bilingual:${language}`}>
                        {lang === "zh" ? `双语 · 原文 + ${language}` : `Bilingual · original + ${language}`}
                      </option>
                    ))}
                  </select>
                </label>
                <label>
                  <span>{lang === "zh" ? "音频" : "Audio"}</span>
                  <select
                    value={videoExportSettings.audioCodec}
                    onChange={(event) => {
                      const audioCodec = event.target.value as VideoExportSettings["audioCodec"];
                      setVideoExportSettings((current) => ({
                        ...current,
                        audioCodec,
                        container: audioCodec === "pcm" ? "mov" : current.container,
                      }));
                    }}
                  >
                    <option value="aac">AAC · {lang === "zh" ? "通用压缩" : "universal compressed"}</option>
                    <option value="pcm">PCM · {lang === "zh" ? "无损后期" : "lossless post-production"}</option>
                  </select>
                </label>
                <label>
                  <span>{lang === "zh" ? "编码策略" : "Encoding strategy"}</span>
                  <select
                    disabled={videoExportSettings.videoCodec === "prores"}
                    value={videoExportSettings.encodingSpeed}
                    onChange={(event) =>
                      setVideoExportSettings((current) => ({
                        ...current,
                        encodingSpeed: event.target.value as VideoExportSettings["encodingSpeed"],
                      }))
                    }
                  >
                    <option value="fast">{lang === "zh" ? "快速 · 优先硬件编码" : "Fast · prefer hardware"}</option>
                    <option value="quality">{lang === "zh" ? "高质量 · CPU 精细压缩" : "Quality · CPU compression"}</option>
                  </select>
                </label>
              </div>
              <p className="video-export-settings-note">
                {lang === "zh"
                  ? `成片画布：${videoCanvasSummary(videoExportSettings, lang)}。`
                  : `Delivery canvas: ${videoCanvasSummary(videoExportSettings, lang)}. `}
                {videoExportSettings.aspectRatio !== "source"
                  ? videoExportSettings.canvasFit === "cover"
                    ? lang === "zh"
                      ? " 源画面会等比放大并居中裁切，节目监看与最终导出使用同一构图。"
                      : " The source is enlarged and center-cropped; the program monitor matches final framing."
                    : lang === "zh"
                      ? " 源画面会完整保留，比例不同时以黑边补齐。"
                      : " The full source remains visible, with black padding when ratios differ."
                  : ""}
                {videoExportSettings.videoCodec === "prores"
                  ? lang === "zh"
                    ? "ProRes 生成体积较大的 MOV 剪辑母版，适合继续调色和后期。"
                    : "ProRes creates a large MOV editing master for grading and post-production."
                  : videoExportSettings.videoCodec === "hevc"
                    ? lang === "zh"
                      ? "HEVC 文件更小，但旧设备和部分网页工具兼容性不如 H.264。"
                      : "HEVC is smaller, but less compatible with older devices and some web tools."
                    : lang === "zh"
                      ? "H.264 适合发布、审阅和跨设备播放。"
                      : "H.264 is the safest choice for publishing, review, and cross-device playback."}
                {videoExportSettings.subtitleMode === "soft"
                  ? lang === "zh"
                    ? " 字幕可在播放器中开关；标题与图形仍会渲染进画面。"
                    : " Captions remain switchable; titles and graphics are still rendered into the picture."
                  : ""}
                {videoExportSettings.subtitleLanguage
                  ? videoExportSettings.bilingualSubtitles
                    ? lang === "zh"
                      ? ` 字幕将按“原文 + ${videoExportSettings.subtitleLanguage}”双行输出。`
                      : ` Captions will contain original + ${videoExportSettings.subtitleLanguage} on two lines.`
                    : lang === "zh"
                      ? ` 字幕将使用 ${videoExportSettings.subtitleLanguage} 译文。`
                      : ` Captions will use the ${videoExportSettings.subtitleLanguage} translation.`
                  : ""}
              </p>
            </fieldset>
            <button
              className="export-action"
              disabled={operation !== null || !subtitleExportAllowed}
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
              disabled={operation !== null || !videoExportAllowed || isVideoExporting}
              onClick={runVideoExport}
            >
              {isVideoExporting ? <span className="spinner" /> : <PlayIcon />}
              <span>
                <strong>{c.exportVideo}</strong>
                <small>
                  {videoExportSettings.container.toUpperCase()} ·{" "}
                  {videoExportSettings.videoCodec === "prores"
                    ? "ProRes 422 HQ"
                    : videoExportSettings.videoCodec.toUpperCase()} ·{" "}
                  {videoExportSettings.resolution === "source"
                    ? lang === "zh" ? "源分辨率" : "source resolution"
                    : videoExportSettings.resolution} ·{" "}
                  {videoExportSettings.aspectRatio === "source"
                    ? lang === "zh" ? "源比例" : "source ratio"
                    : videoExportSettings.aspectRatio}
                </small>
              </span>
            </button>
            <button
              className="export-action"
              disabled={operation !== null || !finalCutExportAllowed}
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
                      : lang === "zh" ? "正在编码视频" : "Encoding video"}
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
                  ? "H.264 VideoToolbox · Apple Media Engine"
                  : videoExportJob.encoder === "hevc_videotoolbox"
                    ? "HEVC VideoToolbox · Apple Media Engine"
                  : videoExportJob.encoder === "libx264"
                    ? "H.264 libx264 · CPU"
                    : videoExportJob.encoder === "libx265"
                      ? "HEVC libx265 · CPU"
                      : videoExportJob.encoder === "prores_ks"
                        ? "ProRes 422 HQ · CPU"
                    : lang === "zh" ? "正在选择编码后端…" : "Selecting encoder…"}
                {videoExportJob.currentSeconds !== null && videoExportJob.totalSeconds !== null
                  ? ` · ${Math.round(videoExportJob.currentSeconds)}s / ${Math.round(videoExportJob.totalSeconds)}s`
                  : ""}
              </small>
              <PipelineFreshness
                state={videoExportJob.state}
                phase={videoExportJob.phase}
                updatedAt={videoExportJob.updatedAt}
                lang={lang}
              />
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
        audioMix={audioMix}
        broll={brollOverview}
        chapters={chapters}
        busy={operation !== null}
        currentTime={workbenchTime}
        cuts={cuts}
        doc={doc}
        history={editHistory}
        isPlaying={workbenchPlaying}
        lang={lang}
        pid={pid}
        rows={subtitleRows}
        titles={titles}
        onAddTitle={addTitle}
        onUpdateAudioMix={updateAudioMix}
        onOpenBroll={() => setActiveTab("broll")}
        onRedo={redoEditorEdit}
        onRemoveCues={removeTimelineCues}
        onSeek={seekWorkbench}
        onSplit={splitSubtitleLine}
        onUpdateCueTiming={updateSubtitleTiming}
        onTogglePlayback={toggleWorkbenchPlayback}
        previewCuts={previewCuts}
        onTogglePreviewCuts={() => setPreviewCuts((value) => !value)}
        collapsed={timelineCollapsed}
        autoCollapsed={timelineAutoCollapsed}
        onToggleCollapsed={() => setTimelinePreference(
          timelineCollapsed ? "expanded" : "collapsed",
        )}
        onUndo={undoEditorEdit}
        onUpdateBroll={updateBroll}
        onUpdateTitle={updateTitle}
        onRemoveTitle={removeTitle}
      />
    </section>
  );
}
