// Thin wrappers around Tauri's invoke() so the views don't sprinkle
// string-command names everywhere.

import { invoke } from "@tauri-apps/api/core";
import type {
  AutoResult,
  Doc,
  FinishCheckItem,
  MergeSummary,
  ModelConfig,
  ProjectSummary,
  RecordingStarted,
  RecordingStopped,
  ReportSummary,
  Settings,
  SpeakerInfo,
  SubtitleRow,
  SubtitleStyle,
  TaskStatus,
  TranscriptionJobStatus,
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

export async function projectList(): Promise<ProjectSummary[]> {
  return invoke("project_list", { root: null });
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

export async function speakersList(pid: string): Promise<SpeakerInfo[]> {
  return invoke("speakers_list", { pid, root: null });
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

export async function exportSubtitles(
  pid: string,
): Promise<string[]> {
  return invoke("export_subtitles", { pid, root: null });
}

export async function exportVideo(pid: string): Promise<string> {
  return invoke("export_video", { pid, root: null });
}

export async function versionMerge(
  base: Record<string, string>,
  ours: Record<string, string>,
  theirs: Record<string, string>,
): Promise<MergeSummary> {
  return invoke("version_merge", { base, ours, theirs });
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
  localStorage.setItem(SETTINGS_KEY, JSON.stringify(s));
}

function defaultSettings(): Settings {
  return {
    llmEndpoint: "",
    llmApiKey: "",
    llmModel: "",
    workerCount: 3,
  };
}
