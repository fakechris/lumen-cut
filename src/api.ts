// Thin wrappers around Tauri's invoke() so the views don't sprinkle
// string-command names everywhere.

import { invoke } from "@tauri-apps/api/core";
import type {
  AsrStatus,
  AutoResult,
  BrollOverview,
  BrollPlacement,
  BrollPlacementInput,
  BrollSuggestion,
  BrollPreviewJobStatus,
  Doc,
  DoctorCheck,
  FinishCheckItem,
  MergeSummary,
  ModelConfig,
  ProjectSummary,
  RecordingStarted,
  RecordingStopped,
  ReportSummary,
  Settings,
  SetupJobStatus,
  SpeakerEvidence,
  SpeakerAnalysisJobStatus,
  SpeakerInfo,
  SpeakerReidentifyPreview,
  SpeakerReidentifyProposal,
  SubtitleRow,
  SubtitleStyle,
  TaskStatus,
  TranscriptionJobStatus,
  VersionHistory,
  VideoExportJobStatus,
} from "./types";

export interface CutSummary {
  id: string;
  kind: string;
  a_word: string;
  b_word: string;
  duration: number;
  note: string | null;
}

export async function greet(): Promise<{ msg: string; version: string }> {
  return invoke("greet");
}

export async function pickMediaFile(): Promise<string | null> {
  return invoke("pick_media_file");
}

export async function pickBrollFile(): Promise<string | null> {
  return invoke("pick_broll_file");
}

export async function projectList(): Promise<ProjectSummary[]> {
  return invoke("project_list", { root: null });
}

export async function projectSearch(query: string): Promise<ProjectSummary[]> {
  return invoke("project_search", { query, root: null });
}

export async function projectSetStar(
  pid: string,
  starred: boolean,
): Promise<ProjectSummary> {
  return invoke("project_set_star", { pid, starred, root: null });
}

export async function projectShow(pid: string): Promise<Doc> {
  return invoke("project_show", { pid, root: null });
}

export async function projectUpdateMeta(
  pid: string,
  title: string,
  description: string,
  language: string | null,
): Promise<Doc> {
  return invoke("project_update_meta", {
    pid,
    title,
    description,
    language,
    root: null,
  });
}

export async function projectReveal(pid: string): Promise<string> {
  return invoke("project_reveal", { pid, root: null });
}

export async function projectDelete(pid: string): Promise<boolean> {
  return invoke("project_delete", { pid, root: null });
}

export async function allowProjectMedia(pid: string): Promise<string> {
  return invoke("media_asset_allow", { pid, root: null });
}

export async function projectCreate(
  pid: string,
  from: string,
  lang: string | null,
  title: string | null,
): Promise<ProjectSummary> {
  return invoke("project_create", {
    args: { pid, from, lang, title, root: null },
  });
}

export async function runAuto(
  media: string,
  lang: string | null,
  title: string | null,
  model: string | null,
  pid: string | null = null,
): Promise<AutoResult> {
  return invoke("run_auto", {
    args: { media, pid, lang, title, out: null, model },
  });
}

export async function transcriptionStart(
  media: string,
  lang: string | null,
  title: string | null,
  model: string | null,
  pid: string,
): Promise<TranscriptionJobStatus> {
  return invoke("transcription_start", {
    args: { media, pid, lang, title, out: null, model },
  });
}

export async function transcriptionStatus(
  pid: string,
): Promise<TranscriptionJobStatus> {
  return invoke("transcription_status", { pid });
}

export async function transcriptionCancel(
  pid: string,
): Promise<TranscriptionJobStatus> {
  return invoke("transcription_cancel", { pid });
}

export async function recordAudio(
  pid: string,
  seconds: number,
): Promise<string> {
  return invoke("record_audio", { pid, seconds, root: null });
}

export async function recordingStart(pid: string): Promise<RecordingStarted> {
  return invoke("recording_start", { pid, root: null });
}

export async function recordingStop(pid: string): Promise<RecordingStopped> {
  return invoke("recording_stop", { pid });
}

export async function recordingCancel(pid: string): Promise<boolean> {
  return invoke("recording_cancel", { pid });
}

export async function taskStart(
  kind: string,
  pid: string,
  lang: string | null,
): Promise<{ pending: number; ai_dir: string; agent_port: number }> {
  return invoke("task_start", {
    args: { kind, pid, lang, root: null, stale_only: false },
  });
}

export async function taskStatus(pid: string): Promise<TaskStatus> {
  return invoke("task_status", { pid, root: null });
}

export async function taskResume(pid: string): Promise<{
  resumed: number;
  recoveredSubmissions: number;
  agentPort: number | null;
}> {
  return invoke("task_resume", { pid, root: null });
}

export async function subtitleList(pid: string): Promise<SubtitleRow[]> {
  return invoke("subtitle_list", { pid, root: null });
}

export async function subtitleSet(
  pid: string,
  id: string,
  text: string,
): Promise<boolean> {
  return invoke("subtitle_set", { pid, id, text, root: null });
}

export async function translationSet(
  pid: string,
  lang: string,
  id: string,
  text: string,
): Promise<boolean> {
  return invoke("translation_set", { pid, lang, id, text, root: null });
}

export async function subtitleReplace(
  pid: string,
  query: string,
  replacement: string,
): Promise<number> {
  return invoke("subtitle_replace", {
    pid,
    query,
    replacement,
    regex: false,
    root: null,
  });
}

export async function subtitleVisibility(
  pid: string,
  id: string,
  hidden: boolean,
): Promise<boolean> {
  return invoke("subtitle_visibility", { pid, id, hidden, root: null });
}

export async function splitSubtitle(
  pid: string,
  id: string,
  at: number,
): Promise<boolean> {
  return invoke("split_line", { pid, id, at, root: null });
}

export async function mergeSubtitles(
  pid: string,
  id1: string,
  id2: string,
): Promise<boolean> {
  return invoke("merge_lines", { pid, id1, id2, root: null });
}

export async function styleGet(pid: string): Promise<SubtitleStyle> {
  return invoke("style_get", { pid, root: null });
}

export async function styleSet(
  pid: string,
  style: SubtitleStyle,
): Promise<void> {
  return invoke("style_set", { pid, style, root: null });
}

export async function configShow(): Promise<ModelConfig> {
  return invoke("config_show");
}

export async function llmModelsList(endpoint: string, apiKey: string): Promise<string[]> {
  return invoke("llm_models_list", { endpoint, apiKey });
}

export async function asrStatus(): Promise<AsrStatus> {
  return invoke("asr_status");
}

export async function asrRuntimeInstall(): Promise<AsrStatus> {
  return invoke("asr_runtime_install");
}

export async function asrModelsDownload(): Promise<AsrStatus> {
  return invoke("asr_models_download");
}

export async function diarizeRuntimeInstall(): Promise<AsrStatus> {
  return invoke("diarize_runtime_install");
}

export async function diarizeModelDownload(): Promise<AsrStatus> {
  return invoke("diarize_model_download");
}

export async function setupJobStart(kind: SetupJobStatus["kind"]): Promise<SetupJobStatus> {
  return invoke("setup_job_start", { kind });
}

export async function setupJobStatus(): Promise<SetupJobStatus> {
  return invoke("setup_job_status");
}

export async function setupJobCancel(): Promise<SetupJobStatus> {
  return invoke("setup_job_cancel");
}

export async function runDoctor(): Promise<DoctorCheck[]> {
  return invoke("run_doctor");
}

export async function revealLogs(): Promise<string> {
  return invoke("logs_reveal");
}

export async function speakersList(pid: string): Promise<SpeakerInfo[]> {
  return invoke("speakers_list", { pid, root: null });
}

export async function speakerEvidence(pid: string): Promise<SpeakerEvidence> {
  return invoke("speaker_evidence", { pid, root: null });
}

export async function speakerRename(
  pid: string,
  from: string,
  to: string,
): Promise<number> {
  return invoke("speaker_rename", { pid, from, to, root: null });
}

export async function speakerMerge(
  pid: string,
  from: string,
  into: string,
): Promise<number> {
  return invoke("speaker_merge", { pid, from, into, root: null });
}

export async function speakerAssign(
  pid: string,
  paragraphId: number,
  speaker: string | null,
): Promise<void> {
  return invoke("speaker_assign", {
    pid,
    input: { paragraphId, speaker },
    root: null,
  });
}

export async function speakerReidentifyPreview(
  pid: string,
): Promise<SpeakerReidentifyPreview> {
  return invoke("speaker_reidentify_preview", { pid, root: null });
}

export async function speakerReidentifyStart(
  pid: string,
): Promise<SpeakerAnalysisJobStatus> {
  return invoke("speaker_reidentify_start", { pid, root: null });
}

export async function speakerReidentifyStatus(
  pid: string,
): Promise<SpeakerAnalysisJobStatus> {
  return invoke("speaker_reidentify_status", { pid });
}

export async function speakerReidentifyCancel(
  pid: string,
): Promise<SpeakerAnalysisJobStatus> {
  return invoke("speaker_reidentify_cancel", { pid });
}

export async function speakerReidentifyApply(
  pid: string,
  proposals: SpeakerReidentifyProposal[],
): Promise<number> {
  return invoke("speaker_reidentify_apply", { pid, proposals, root: null });
}

export async function brollList(pid: string): Promise<BrollOverview> {
  return invoke("broll_list", { pid, root: null });
}

export async function brollAdd(
  pid: string,
  input: BrollPlacementInput,
): Promise<BrollPlacement> {
  return invoke("broll_add", { pid, input, root: null });
}

export async function brollAcceptSuggestion(
  pid: string,
  suggestion: BrollSuggestion,
  file: string,
): Promise<BrollPlacement> {
  return invoke("broll_accept_suggestion", {
    pid,
    suggestion,
    file,
    root: null,
  });
}

export async function brollUpdate(
  pid: string,
  id: string,
  input: BrollPlacementInput,
): Promise<BrollPlacement> {
  return invoke("broll_update", { pid, id, input, root: null });
}

export async function brollRemove(pid: string, id: string): Promise<boolean> {
  return invoke("broll_remove", { pid, id, root: null });
}

export async function brollPreview(pid: string): Promise<string[]> {
  return invoke("broll_preview", { pid, at: [], root: null });
}

export async function brollPreviewStart(pid: string): Promise<BrollPreviewJobStatus> {
  return invoke("broll_preview_start", { pid });
}

export async function brollPreviewStatus(pid: string): Promise<BrollPreviewJobStatus> {
  return invoke("broll_preview_status", { pid });
}

export async function brollPreviewCancel(pid: string): Promise<BrollPreviewJobStatus> {
  return invoke("broll_preview_cancel", { pid });
}

export async function finishCheck(pid: string): Promise<FinishCheckItem[]> {
  return invoke("finish_check_pid", { pid, root: null });
}

export async function cutAuto(pid: string): Promise<number> {
  return invoke("cut_auto", { pid, root: null });
}

export async function cutRestore(pid: string, cutId: string): Promise<boolean> {
  return invoke("cut_restore", { pid, cutId, root: null });
}

export async function cutList(pid: string): Promise<CutSummary[]> {
  return invoke("cut_list", { pid, root: null });
}

export async function auditCodes(): Promise<string[]> {
  return invoke("audit_codes");
}

export async function audit(pid: string): Promise<ReportSummary> {
  return invoke("audit_pid", { pid, root: null });
}

export async function diarize(
  pid: string,
): Promise<{ segments: number; paragraphs_assigned: number }> {
  return invoke("diarize_pid", { pid, root: null });
}

export async function timingRepair(pid: string): Promise<string> {
  return invoke("timing_repair", { pid, root: null });
}

export async function exportSubtitles(
  pid: string,
): Promise<string[]> {
  return invoke("export_subtitles", { pid, root: null });
}

export async function exportVideo(pid: string): Promise<string> {
  return invoke("export_video", { pid, root: null });
}

export async function videoExportStart(
  pid: string,
  mode: VideoExportJobStatus["mode"],
): Promise<VideoExportJobStatus> {
  return invoke("video_export_start", { pid, mode });
}

export async function videoExportStatus(pid: string): Promise<VideoExportJobStatus> {
  return invoke("video_export_status", { pid });
}

export async function videoExportCancel(pid: string): Promise<VideoExportJobStatus> {
  return invoke("video_export_cancel", { pid });
}

export async function exportFinalCut(pid: string): Promise<string> {
  return invoke("export_fcp", { pid, root: null });
}

export async function versionMerge(
  base: Record<string, string>,
  ours: Record<string, string>,
  theirs: Record<string, string>,
): Promise<MergeSummary> {
  return invoke("version_merge", { base, ours, theirs });
}

export async function versionList(pid: string): Promise<VersionHistory> {
  return invoke("version_list", { pid, root: null });
}

export async function versionCommit(
  pid: string,
  name: string,
  note: string,
): Promise<string> {
  return invoke("version_commit", { pid, name, note, root: null });
}

export async function versionRestore(pid: string, id: string): Promise<void> {
  return invoke("version_restore", { pid, id, root: null });
}

export async function branchCreate(pid: string, name: string): Promise<string> {
  return invoke("branch_create", { pid, name, root: null });
}

export async function branchSwitch(pid: string, id: string): Promise<void> {
  return invoke("branch_switch", { pid, id, root: null });
}

export async function agentServe(port: number | null): Promise<number> {
  return invoke("agent_serve", { port });
}

export async function agentWorkers(): Promise<unknown[]> {
  return invoke("agent_workers");
}

export async function settingsExport(s: Settings): Promise<string> {
  return invoke("settings_export", {
    settings: {
      asr_model: s.asrModel,
      asr_aligner: s.asrAligner,
      diarize_model: s.diarizeModel,
      hf_token: s.hfToken,
      llm_endpoint: s.llmEndpoint,
      llm_api_key: s.llmApiKey,
      llm_model: s.llmModel,
      worker_count: s.workerCount,
    },
  });
}

// Settings persistence — backed by localStorage so the UI doesn't need
// a separate Tauri command for the trivial key/value pairs.
const SETTINGS_KEY = "lumen-cut.settings.v1";

export function loadSettings(): Settings {
  try {
    const raw = localStorage.getItem(SETTINGS_KEY);
    if (raw) return { ...defaultSettings(), ...JSON.parse(raw) };
  } catch {
    // ignore
  }
  return defaultSettings();
}

export function saveSettings(s: Settings) {
  const { hfToken: _hfToken, llmApiKey: _llmApiKey, ...nonSensitive } = s;
  localStorage.setItem(SETTINGS_KEY, JSON.stringify(nonSensitive));
}

function defaultSettings(): Settings {
  return {
    asrModel: "mlx-community/Qwen3-ASR-0.6B-8bit",
    asrAligner: "mlx-community/Qwen3-ForcedAligner-0.6B-4bit",
    diarizeModel: "pyannote/speaker-diarization-3.1",
    hfToken: "",
    llmEndpoint: "",
    llmApiKey: "",
    llmModel: "",
    workerCount: 3,
  };
}
