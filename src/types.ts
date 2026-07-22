// Shared TypeScript types — mirror the serde structs in `src-tauri/src`.

export interface GreetResponse {
  msg: string;
  version: string;
}

export interface ProjectSummary {
  pid: string;
  title: string;
  description: string;
  path: string;
  duration_seconds: number;
  word_count: number;
  paragraph_count: number;
  updated_at: string;
  starred: boolean;
}

export interface SpeakerInfo {
  id: string;
  paragraph_count: number;
}

export interface SpeakerTurn {
  paragraphId: number;
  speaker: string | null;
  start: number;
  end: number;
  text: string;
  cueIds: string[];
}

export interface SpeakerEvidence {
  speakers: SpeakerInfo[];
  turns: SpeakerTurn[];
  identified: boolean;
  unlabelled: number;
}

export interface SpeakerReidentifyProposal {
  paragraphId: number;
  current: string | null;
  cluster: string;
  proposed: string;
  start: number;
  end: number;
  text: string;
  coverage: number;
  margin: number;
}

export interface SpeakerReidentifyPreview {
  segments: number;
  changed: number;
  unassigned: number;
  proposals: SpeakerReidentifyProposal[];
}

export interface RecordingStarted {
  pid: string;
  path: string;
}

export interface RecordingStopped extends RecordingStarted {
  durationSeconds: number;
}

export interface DocMedia {
  path: string;
  durationSeconds: number;
  sampleRate?: number;
  channels?: number;
}

export interface DocWord {
  id: string;
  text: string;
  start: number;
  end: number;
}

export interface DocSentence {
  id: string;
  text: string;
  words: DocWord[];
}

export interface DocParagraph {
  id: number;
  speaker?: string | null;
  sentences: DocSentence[];
}

export interface DocMeta {
  title: string;
  description: string;
  language?: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface Doc {
  id: string;
  schema: number;
  media: DocMedia;
  meta: DocMeta;
  paragraphs: DocParagraph[];
  translations: Record<string, Record<string, DocTranslation>>;
}

export interface DocTranslation {
  text: string;
}

export interface AutoResult {
  pid_dir: string;
  srt: string;
  vtt: string;
  ass: string;
  md: string;
  word_count: number;
  paragraph_count: number;
}

export interface TranscriptionJobStatus {
  pid: string;
  state: "running" | "cancelling" | "completed" | "cancelled" | "failed";
  phase:
    | "waiting"
    | "preparing"
    | "downloading"
    | "extracting"
    | "analyzing"
    | "transcribing"
    | "aligning"
    | "saving"
    | "exporting"
    | "completed"
    | "cancelling"
    | "cancelled"
    | "failed";
  progress: number;
  current: number | null;
  total: number | null;
  device: "mlx-metal" | null;
  elapsedSeconds: number | null;
  cpuPercent: number | null;
  peakMemoryMb: number | null;
  memoryLimitMb: number | null;
  mlxActiveMemoryMb: number | null;
  mlxCacheMemoryMb: number | null;
  error?: string | null;
}

export interface SpeakerAnalysisJobStatus {
  pid: string;
  state: "running" | "cancelling" | "completed" | "cancelled" | "failed";
  phase:
    | "waiting"
    | "preparing"
    | "loading"
    | "segmenting"
    | "counting"
    | "embedding"
    | "finalizing"
    | "completed"
    | "cancelling"
    | "cancelled"
    | "failed";
  progress: number;
  current: number | null;
  total: number | null;
  device: "mps" | "cpu" | null;
  elapsedSeconds: number | null;
  cpuPercent: number | null;
  peakMemoryMb: number | null;
  memoryLimitMb: number | null;
  error: string | null;
  preview: SpeakerReidentifyPreview | null;
}

export interface VideoExportJobStatus {
  pid: string;
  mode: "fast" | "quality";
  state: "running" | "cancelling" | "completed" | "cancelled" | "failed";
  phase: "waiting" | "preparing" | "encoding" | "completed" | "cancelling" | "cancelled" | "failed";
  progress: number;
  currentSeconds: number | null;
  totalSeconds: number | null;
  encoder: "h264_videotoolbox" | "libx264" | null;
  error: string | null;
  path: string | null;
}

export interface SetupJobStatus {
  kind: "asr-runtime" | "asr-models" | "speaker-runtime" | "speaker-model";
  state: "running" | "cancelling" | "completed" | "cancelled" | "failed";
  phase: "waiting" | "installing" | "downloading" | "completed" | "cancelling" | "cancelled" | "failed";
  error: string | null;
}

export interface BrollPreviewJobStatus {
  pid: string;
  state: "running" | "cancelling" | "completed" | "cancelled" | "failed";
  phase: "waiting" | "preparing" | "encoding" | "frames" | "completed" | "cancelling" | "cancelled" | "failed";
  progress: number;
  current: number | null;
  total: number | null;
  encoder: "h264_videotoolbox" | "libx264" | null;
  error: string | null;
  paths: string[];
}

export interface TaskStatus {
  pending: number;
  done: number;
  kinds: Array<{
    kind: string;
    lang?: string | null;
    state?: "running" | "completed" | "paused" | "failed";
    calls?: number;
    pending: number;
    done: number;
    failed: number;
    lastError?: string | null;
    updatedAt?: number | null;
  }>;
  polishQuality?: {
    fingerprint: string;
    createdAt: string;
    status: "PASS" | "WARN";
    pageCount: number;
    measuredPageCount: number;
    retryCount: number;
    recoveredPageCount: number;
    fallbackPageCount: number;
    fallbackSentenceCount: number;
    residualTermVariantCount: number;
    residualTermVariants: Array<{
      canonical: string;
      variant: string;
      occurrences: number;
    }>;
    zeroDurationWordCountBefore: number;
    zeroDurationWordCountAfter: number;
  } | null;
}

export interface SubtitleRow {
  id: string;
  text: string;
  speaker?: string | null;
  hidden: boolean;
  start: number;
  end: number;
}

export interface SubtitleStyle {
  name: string;
  fontname: string;
  fontsize: number;
  primaryColour: string;
  outlineColour: string;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  strikeOut: boolean;
  alignment: number;
  outline: number;
  shadow: number;
  marginL: number;
  marginR: number;
  marginV: number;
}

export interface ModelConfig {
  asrModel: string;
  asrAligner: string;
  diarizeModel: string;
  hfToken: string;
  llmEndpoint: string;
  llmApiKey: string;
  llmModel: string;
  workerCount: number;
}

export interface AsrStatus {
  pythonPath: string | null;
  runtimeReady: boolean;
  runtimeDetail: string;
  modelId: string;
  modelCached: boolean;
  alignerId: string;
  alignerCached: boolean;
  diarizeModelId: string;
  diarizeModelCached: boolean;
  diarizePythonPath: string | null;
  diarizeRuntimeReady: boolean;
  diarizeRuntimeDetail: string;
  huggingFaceTokenSet: boolean;
  diarizeReady: boolean;
  ready: boolean;
}

export interface DoctorCheck {
  name: string;
  ok: boolean;
  detail: string;
}

export interface FinishCheckItem {
  code: string;
  ordinal: number;
  pass: boolean;
  blockers: string[];
}

export interface FindingSummary {
  code: string;
  severity: string;
  location: string;
  message: string;
}

export interface ReportSummary {
  findings: FindingSummary[];
  has_failures: boolean;
  has_warnings: boolean;
}

export interface ConflictSummary {
  cue_id: string;
  base: string;
  ours: string;
  theirs: string;
}

export interface MergeSummary {
  merged: Record<string, string>;
  conflicts: ConflictSummary[];
}

export interface VersionNode {
  id: string;
  parent?: string | null;
  branch: string;
  name: string;
  note: string;
  at: string;
  kind: "manual" | "agent" | "auto" | "restore";
  diffs?: unknown[];
}

export interface ProjectBranch {
  id: string;
  name: string;
  tip: string;
  root: string;
  createdAt: string;
  note: string;
}

export interface VersionHistory {
  v: number;
  head?: string | null;
  activeBranch?: string | null;
  branches: ProjectBranch[];
  versions: VersionNode[];
}

export type BrollMode = "fullscreen" | "pip";
export type BrollFit = "cover" | "contain";
export type BrollBackground = "black" | "blur";

export interface BrollSuggestion {
  start: string;
  end: string;
  mode: BrollMode;
  query: string;
  reason: string;
}

export interface BrollPlacement {
  id: string;
  file: string;
  start: number;
  end: number;
  mode: BrollMode;
  rect?: { x: number; y: number; width: number; height: number } | null;
  fit: BrollFit;
  background: BrollBackground;
  sourceStart: number;
  radius: number;
  name?: string | null;
}

export interface BrollPlacementInput {
  file: string;
  start: number;
  end: number;
  mode: BrollMode;
  fit: BrollFit;
  background: BrollBackground;
  sourceStart: number;
  radius: number;
  name: string | null;
}

export interface BrollOverview {
  suggestions: BrollSuggestion[];
  accepted: BrollPlacement[];
  errors: string[];
}

export interface Settings {
  asrModel: string;
  asrAligner: string;
  diarizeModel: string;
  hfToken: string;
  llmEndpoint: string;
  llmApiKey: string;
  llmModel: string;
  workerCount: number;
}
