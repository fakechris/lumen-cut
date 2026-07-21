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
    | "preparing"
    | "downloading"
    | "extracting"
    | "analyzing"
    | "transcribing"
    | "saving"
    | "exporting"
    | "completed"
    | "cancelling"
    | "cancelled"
    | "failed";
  progress: number;
  error?: string | null;
}

export interface TaskStatus {
  pending: number;
  done: number;
  kinds: Array<{
    kind: string;
    lang?: string | null;
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

export interface Settings {
  asrModel: string;
  asrAligner: string;
  diarizeModel: string;
  llmEndpoint: string;
  llmApiKey: string;
  llmModel: string;
  workerCount: number;
}
