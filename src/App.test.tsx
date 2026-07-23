import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import App from "./App";
import serializedProject from "./test/fixtures/project.json";
import type { AudioMix, ExportPreflightReport } from "./types";

const { invoke, nativeDrag } = vi.hoisted(() => ({
  invoke: vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>(),
  nativeDrag: {
    handler: null as null | ((event: {
      payload:
        | { type: "enter" | "drop"; paths: string[] }
        | { type: "over" | "leave" };
    }) => void),
  },
}));
let projectDoc: Record<string, unknown>;
let projectShowError: Error | null;
let asrReady: boolean;
let versionCommitError: Error | null;
let transcriptionStatusState: {
  pid: string;
  state: string;
  phase: string;
  progress: number;
  updatedAt?: number | null;
  error?: string | null;
  cpuPercent?: number | null;
  peakMemoryMb?: number | null;
  memoryLimitMb?: number | null;
};
let transcriptionStatusError: Error | null;
let performanceState: {
  activePipeline: string | null;
  waitingPipelines: number;
};
let finishCheckItems: Array<{
  code: string;
  ordinal: number;
  pass: boolean;
  blockers: string[];
}>;
let exportPreflightState: ExportPreflightReport;
let cutListState: Array<{
  id: string;
  kind: string;
  a_word: string;
  b_word: string;
  duration: number;
  note: string | null;
}>;
let brollOverview: {
  suggestions: Array<Record<string, unknown>>;
  accepted: Array<Record<string, unknown>>;
  errors: string[];
};
let brollListError: Error | null;
let speakerEvidenceState: {
  speakers: Array<{ id: string; paragraph_count: number }>;
  turns: Array<{
    paragraphId: number;
    speaker: string | null;
    start: number;
    end: number;
    text: string;
    cueIds: string[];
  }>;
  identified: boolean;
  unlabelled: number;
};
let speakerAnalysisStatusState: {
  pid: string;
  state: string;
  phase: string;
  progress: number;
  current: number | null;
  total: number | null;
  error: string | null;
  preview: Record<string, unknown> | null;
};
let speakerAnalysisStartState: typeof speakerAnalysisStatusState;
let speakerAnalysisHasExistingJob: boolean;
let videoExportHasExistingJob: boolean;
let videoExportStatusState: {
  pid: string;
  mode: string;
  settings: {
    container: "mp4" | "mov";
    videoCodec: "h264" | "hevc" | "prores";
    resolution: "source" | "720p" | "1080p" | "4k";
    aspectRatio: "source" | "16:9" | "9:16" | "1:1" | "4:5";
    canvasFit: "contain" | "cover";
    subtitleMode: "burn" | "soft" | "none";
    subtitleLanguage: string | null;
    bilingualSubtitles: boolean;
    audioCodec: "aac" | "pcm";
    encodingSpeed: "fast" | "quality";
  };
  state: string;
  phase: string;
  progress: number;
  currentSeconds: number | null;
  totalSeconds: number | null;
  encoder: string | null;
  error: string | null;
  path: string | null;
};
let setupJobHasExistingJob: boolean;
let setupJobStatusState: {
  kind: string;
  state: string;
  phase: string;
  error: string | null;
};
let brollPreviewHasExistingJob: boolean;
let brollPreviewStatusState: {
  pid: string;
  state: string;
  phase: string;
  progress: number;
  current: number | null;
  total: number | null;
  encoder: string | null;
  error: string | null;
  paths: string[];
};
let taskStatusState: {
  pending: number;
  done: number;
  kinds: Array<Record<string, unknown>>;
  polishQuality: null;
};
let mediaAllowError: Error | null;
let llmConfigured: boolean;
let subtitleSetError: Error | null;
let chapterRowsState: Array<{
  title: string;
  startSeg: string;
  start: number;
  end: number;
  preview: string;
}>;
let subtitleRowsState: Array<{
  id: string;
  text: string;
  speaker: string | null;
  hidden: boolean;
  start: number;
  end: number;
}>;

async function openEditorTool(
  label: "转写设置" | "说话人" | "章节" | "时间线" | "补充画面",
) {
  const workflow = await screen.findByRole("navigation", { name: "编辑主流程" });
  fireEvent.click(within(workflow).getByRole("button", { name: /打开更多编辑工具/ }));
  fireEvent.click(screen.getByRole("menuitem", { name: new RegExp(`^${label}`) }));
}
let projectListState: Array<Record<string, unknown>>;
let projectThumbnailState: {
  path: string | null;
  mediaAvailable: boolean;
  deferred: boolean;
};
let titleListState: Array<{
  id: string;
  text: string;
  start: number;
  end: number;
  x: number;
  y: number;
  fontSize: number;
  color: string;
  background: string;
  fadeIn: number;
  fadeOut: number;
}>;
let audioMixState: AudioMix;

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (path: string) => `asset://${path}`,
  invoke,
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    onDragDropEvent: async (handler: typeof nativeDrag.handler) => {
      nativeDrag.handler = handler;
      return () => {
        nativeDrag.handler = null;
      };
    },
  }),
}));

beforeEach(() => {
  localStorage.clear();
  nativeDrag.handler = null;
  delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  asrReady = true;
  versionCommitError = null;
  transcriptionStatusState = {
    pid: "project-1",
    state: "completed",
    phase: "completed",
    progress: 100,
  };
  transcriptionStatusError = null;
  performanceState = {
    activePipeline: null,
    waitingPipelines: 0,
  };
  finishCheckItems = [{ code: "delivery-ready", ordinal: 1, pass: true, blockers: [] }];
  exportPreflightState = {
    ready: true,
    items: [
      { code: "settings", level: "pass", message: "settings are compatible" },
      { code: "media", level: "pass", message: "source media is readable" },
      { code: "encoder", level: "pass", message: "encoder is available" },
      {
        code: "size-estimate",
        level: "warning",
        message: "estimated output size is 12–28 MB",
      },
    ],
    summary: {
      durationSeconds: 30,
      visibleCaptions: 12,
      hiddenCaptions: 0,
      brollItems: 0,
      titleItems: 0,
      encoder: "h264_videotoolbox",
      estimatedMinMb: 12,
      estimatedMaxMb: 28,
    },
  };
  cutListState = [];
  brollOverview = { suggestions: [], accepted: [], errors: [] };
  brollListError = null;
  speakerEvidenceState = { speakers: [], turns: [], identified: false, unlabelled: 0 };
  speakerAnalysisStatusState = {
    pid: "project-1",
    state: "completed",
    phase: "completed",
    progress: 100,
    current: null,
    total: null,
    error: null,
    preview: {
      segments: 3,
      changed: 1,
      unassigned: 0,
      proposals: [{
        paragraphId: 1,
        current: "Alice",
        cluster: "SPEAKER_00",
        proposed: "SPEAKER_00",
        start: 0,
        end: 1,
        text: "Hello world",
        coverage: 0.94,
        margin: 0.88,
      }],
    },
  };
  speakerAnalysisStartState = {
    pid: "project-1",
    state: "running",
    phase: "preparing",
    progress: 0,
    current: null,
    total: null,
    error: null,
    preview: null,
  };
  speakerAnalysisHasExistingJob = false;
  videoExportHasExistingJob = false;
  videoExportStatusState = {
    pid: "project-1",
    mode: "fast",
    settings: {
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
    },
    state: "running",
    phase: "encoding",
    progress: 47,
    currentSeconds: 14.2,
    totalSeconds: 30,
    encoder: "h264_videotoolbox",
    error: null,
    path: null,
  };
  setupJobHasExistingJob = false;
  setupJobStatusState = {
    kind: "asr-runtime",
    state: "running",
    phase: "installing",
    error: null,
  };
  brollPreviewHasExistingJob = false;
  brollPreviewStatusState = {
    pid: "project-1",
    state: "running",
    phase: "encoding",
    progress: 67,
    current: 12,
    total: 30,
    encoder: "h264_videotoolbox",
    error: null,
    paths: [],
  };
  taskStatusState = { pending: 0, done: 0, kinds: [], polishQuality: null };
  mediaAllowError = null;
  llmConfigured = false;
  subtitleSetError = null;
  chapterRowsState = [];
  subtitleRowsState = [];
  projectListState = [{
    pid: "project-1",
    title: "Interview",
    description: "",
    path: "/projects/project-1",
    duration_seconds: 2212.792018,
    word_count: 0,
    paragraph_count: 0,
    updated_at: "2026-07-21T00:00:00Z",
    starred: false,
    media_available: true,
    last_opened_at: null,
  }];
  projectThumbnailState = {
    path: "/projects/project-1/.lumen-cut/project-thumbnail/thumbnail.jpg",
    mediaAvailable: true,
    deferred: false,
  };
  titleListState = [];
  audioMixState = {
    volume: 1,
    muted: false,
    fadeIn: 0,
    fadeOut: 0,
    voiceEnhance: false,
    normalizeLoudness: false,
    loudnessTarget: -16,
    music: [],
  };
  invoke.mockReset();
  projectDoc = structuredClone(serializedProject);
  projectShowError = null;
  invoke.mockImplementation(async (command, args) => {
    switch (command) {
      case "greet":
        return { msg: "ready", version: "0.2.0" };
      case "project_list":
        return projectListState;
      case "project_thumbnail":
        return projectThumbnailState;
      case "project_mark_opened":
        return {
          ...projectListState.find((project) => project.pid === "project-1"),
          last_opened_at: "2026-07-23T00:00:00Z",
        };
      case "project_search":
        return [{
          pid: "project-2",
          title: "Search match",
          description: "Customer notes",
          path: "/projects/project-2",
          duration_seconds: 32,
          word_count: 8,
          paragraph_count: 2,
          updated_at: "2026-07-21T01:00:00Z",
          starred: false,
          media_available: true,
          last_opened_at: null,
        }];
      case "project_set_star":
        return {
          pid: "project-2",
          title: "Search match",
          description: "Customer notes",
          path: "/projects/project-2",
          duration_seconds: 32,
          word_count: 8,
          paragraph_count: 2,
          updated_at: "2026-07-21T01:00:00Z",
          starred: true,
          media_available: true,
          last_opened_at: null,
        };
      case "timing_repair":
        return "3 fix(es)";
      case "project_show":
        if (projectShowError) throw projectShowError;
        return projectDoc;
      case "project_create":
        return {
          pid: "drop-project",
          title: "drop",
          description: "",
          path: "/projects/drop-project",
          duration_seconds: 12,
          word_count: 0,
          paragraph_count: 0,
          updated_at: "2026-07-21T00:00:00Z",
          starred: false,
          media_available: true,
          last_opened_at: null,
        };
      case "subtitle_list":
        return subtitleRowsState;
      case "chapter_list":
        return chapterRowsState;
      case "chapter_set_many": {
        const chapters = Array.isArray(args?.chapters)
          ? args.chapters as Array<{ title: string; startSeg: string }>
          : [];
        chapterRowsState = chapters.map((chapter, index) => {
          const row = subtitleRowsState.find((candidate) => candidate.id === chapter.startSeg);
          const next = chapters[index + 1];
          const nextRow = next
            ? subtitleRowsState.find((candidate) => candidate.id === next.startSeg)
            : null;
          return {
            ...chapter,
            start: row?.start ?? 0,
            end: nextRow?.start ?? (projectDoc.media as { durationSeconds: number }).durationSeconds,
            preview: row?.text ?? "",
          };
        });
        return true;
      }
      case "speakers_list":
        return [];
      case "subtitle_set":
        if (subtitleSetError) throw subtitleSetError;
        return true;
      case "subtitle_update_many": {
        if (subtitleSetError) throw subtitleSetError;
        const updates = Array.isArray(args?.updates)
          ? args.updates as Array<{ id: string; text: string }>
          : [];
        const updateMap = new Map(updates.map((update) => [update.id, update.text]));
        const sentences: Array<Record<string, unknown>> = [];
        let changed = 0;
        const paragraphs = (projectDoc.paragraphs || []) as Array<{
          id: number;
          speaker?: string | null;
          sentences: Array<{
            id: string;
            text: string;
            words: Array<{ id: string; text: string; start: number; end: number }>;
          }>;
        }>;
        const nextParagraphs = paragraphs.map((paragraph) => ({
          ...paragraph,
          sentences: paragraph.sentences.map((sentence) => {
            const text = updateMap.get(sentence.id);
            if (text === undefined) return sentence;
            if (text !== sentence.text) changed += 1;
            const start = sentence.words[0]?.start ?? 0;
            const end = sentence.words[sentence.words.length - 1]?.end ?? start;
            const tokens = text.split(/\s+/).filter(Boolean);
            const span = Math.max(0, end - start);
            const next = {
              ...sentence,
              text,
              words: tokens.map((token, index) => ({
                id: `${sentence.id}-edited-${index}`,
                text: token,
                start: start + (span * index) / Math.max(1, tokens.length),
                end: start + (span * (index + 1)) / Math.max(1, tokens.length),
              })),
            };
            sentences.push(next);
            return next;
          }),
        }));
        projectDoc = { ...projectDoc, paragraphs: nextParagraphs };
        subtitleRowsState = subtitleRowsState.map((row) => {
          const text = updateMap.get(row.id);
          return text === undefined ? row : { ...row, text };
        });
        return { changed, sentences };
      }
      case "subtitle_timing_set": {
        const id = String(args?.id);
        const start = Number(args?.start);
        const end = Number(args?.end);
        const paragraphs = (projectDoc.paragraphs || []) as Array<{
          id: number;
          speaker?: string | null;
          sentences: Array<{
            id: string;
            text: string;
            words: Array<{ id: string; text: string; start: number; end: number }>;
          }>;
        }>;
        projectDoc = {
          ...projectDoc,
          paragraphs: paragraphs.map((paragraph) => ({
            ...paragraph,
            sentences: paragraph.sentences.map((sentence) => {
              if (sentence.id !== id || sentence.words.length === 0) return sentence;
              const oldStart = sentence.words[0].start;
              const oldEnd = sentence.words[sentence.words.length - 1].end;
              const scale = (end - start) / (oldEnd - oldStart);
              return {
                ...sentence,
                words: sentence.words.map((word) => ({
                  ...word,
                  start: start + (word.start - oldStart) * scale,
                  end: start + (word.end - oldStart) * scale,
                })),
              };
            }),
          })),
        };
        subtitleRowsState = subtitleRowsState.map((row) =>
          row.id === id ? { ...row, start, end } : row
        );
        return true;
      }
      case "translation_set":
        return true;
      case "subtitle_set_many":
        if (subtitleSetError) throw subtitleSetError;
        return Array.isArray(args?.updates) ? args.updates.length : 0;
      case "translation_set_many":
        return Array.isArray(args?.updates) ? args.updates.length : 0;
      case "cut_list":
        return cutListState;
      case "cut_manual_many":
        return 2;
      case "edit_history_status":
        return {
          canUndo: true,
          canRedo: false,
          undoLabel: "Remove timeline regions",
          redoLabel: null,
        };
      case "title_list":
        return titleListState;
      case "title_add": {
        const title = {
          id: "title-1",
          ...(args?.input as Omit<(typeof titleListState)[number], "id">),
        };
        titleListState = [...titleListState, title];
        return title;
      }
      case "title_update": {
        const replacement = {
          id: String(args?.id),
          ...(args?.input as Omit<(typeof titleListState)[number], "id">),
        };
        titleListState = titleListState.map((title) =>
          title.id === replacement.id ? replacement : title
        );
        return replacement;
      }
      case "title_remove": {
        const before = titleListState.length;
        titleListState = titleListState.filter((title) => title.id !== args?.id);
        return titleListState.length !== before;
      }
      case "audio_mix_get":
        return audioMixState;
      case "pick_audio_file":
        return "/tmp/background.mp3";
      case "audio_asset_allow":
        return audioMixState.music.find((track) => track.id === args?.musicId)?.path ?? null;
      case "audio_mix_set":
        audioMixState = args?.mix as AudioMix;
        return audioMixState;
      case "export_settings_get":
        return {
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
      case "export_settings_set":
        return args?.settings;
      case "speaker_evidence":
        return speakerEvidenceState;
      case "speaker_reidentify_preview":
        return {
          segments: 3,
          changed: 1,
          unassigned: 0,
          proposals: [{
            paragraphId: 1,
            current: "Alice",
            cluster: "SPEAKER_00",
            proposed: "SPEAKER_00",
            start: 0,
            end: 1,
            text: "Hello world",
            coverage: 0.94,
            margin: 0.88,
          }],
        };
      case "speaker_reidentify_start":
        speakerAnalysisHasExistingJob = true;
        return speakerAnalysisStartState;
      case "speaker_reidentify_status":
        if (!speakerAnalysisHasExistingJob) throw new Error("no speaker analysis job");
        return speakerAnalysisStatusState;
      case "speaker_reidentify_cancel":
        return { ...speakerAnalysisStatusState, state: "cancelling", phase: "cancelling" };
      case "speaker_reidentify_apply":
        return 1;
      case "speaker_assign":
        return undefined;
      case "finish_check_pid":
        return finishCheckItems;
      case "export_preflight":
        return exportPreflightState;
      case "export_subtitles":
        return ["/projects/project-1/export.srt"];
      case "export_video":
        return "/projects/project-1/export.mp4";
      case "video_export_start":
        videoExportHasExistingJob = true;
        return videoExportStatusState;
      case "video_export_status":
        if (!videoExportHasExistingJob) throw new Error("no video export job");
        return videoExportStatusState;
      case "video_export_cancel":
        return { ...videoExportStatusState, state: "cancelling", phase: "cancelling" };
      case "setup_job_start":
        setupJobHasExistingJob = true;
        return setupJobStatusState;
      case "setup_job_status":
        if (!setupJobHasExistingJob) throw new Error("no setup job");
        return setupJobStatusState;
      case "setup_job_cancel":
        return { ...setupJobStatusState, state: "cancelling", phase: "cancelling" };
      case "export_fcp":
        return "/projects/project-1/export.fcpxml";
      case "project_reveal":
        return "/projects/project-1";
      case "style_get":
        return {
          name: "Default",
          fontname: "Arial",
          fontsize: 52,
          primaryColour: "&H00FFFFFF",
          outlineColour: "&H00000000",
          bold: false,
          italic: false,
          underline: false,
          strikeOut: false,
          alignment: 2,
          outline: 2,
          shadow: 2,
          marginL: 40,
          marginR: 40,
          marginV: 80,
        };
      case "task_status":
        return taskStatusState;
      case "performance_status":
        return performanceState;
      case "task_resume":
        return { resumed: 1, recoveredSubmissions: 1, agentPort: 3417 };
      case "task_pause":
        return { paused: 1, queuedCalls: 3, inFlightCalls: 1 };
      case "task_start":
        return { pending: 1, ai_dir: "/projects/project-1/ai/translate", agent_port: 3417 };
      case "version_list":
        return { v: 1, head: null, activeBranch: null, branches: [], versions: [] };
      case "broll_list":
        if (brollListError) throw brollListError;
        return brollOverview;
      case "pick_broll_file":
        return "/Users/example/product.png";
      case "broll_asset_allow":
        return "/Users/example/product.png";
      case "pick_media_file":
        return "/Users/example/Interview-restored.mp4";
      case "broll_accept_suggestion": {
        const placement = {
          id: "br-1",
          file: "/Users/example/product.png",
          start: 4,
          end: 7,
          mode: "pip",
          rect: null,
          fit: "cover",
          background: "black",
          sourceStart: 0,
          radius: 0,
          name: "product close-up",
        };
        brollOverview = { ...brollOverview, accepted: [placement] };
        return placement;
      }
      case "broll_preview_start":
        brollPreviewHasExistingJob = true;
        return brollPreviewStatusState;
      case "broll_preview_status":
        if (!brollPreviewHasExistingJob) throw new Error("no B-roll preview job");
        return brollPreviewStatusState;
      case "broll_preview_cancel":
        return { ...brollPreviewStatusState, state: "cancelling", phase: "cancelling" };
      case "version_commit":
        if (versionCommitError) throw versionCommitError;
        return "v0";
      case "config_show":
        return {
          asrEngine: "local",
          asrModel: "Qwen/Qwen3-ASR-0.6B",
          asrAligner: "Qwen/Qwen3-ForcedAligner-0.6B",
          asrCloudEndpoint: "https://api.openai.com/v1/audio/transcriptions",
          asrCloudApiKey: "",
          asrCloudModel: "whisper-1",
          diarizeModel: "pyannote/speaker-diarization-3.1",
          hfToken: "",
          llmEndpoint: llmConfigured ? "https://api.example.com/v1" : "",
          llmApiKey: "",
          llmModel: "gpt-4o-mini",
          workerCount: 3,
          hfTokenSet: true,
          llmApiKeySet: llmConfigured,
          asrCloudApiKeySet: false,
        };
      case "settings_export":
        return "/Users/example/.lumen-cut/settings.json";
      case "llm_models_list":
        return ["MiniMax-M3", "MiniMax-M4-preview"];
      case "asr_models_list":
        return ["whisper-1"];
      case "asr_status":
        return {
          engine: "local",
          selectedReady: asrReady,
          cloudConfigured: false,
          pythonPath: "/Users/example/.lumen-cut/runtime/bin/python3",
          runtimeReady: asrReady,
          runtimeDetail: "mlx-qwen3-asr 0.3.5",
          modelId: "Qwen/Qwen3-ASR-0.6B",
          modelCached: asrReady,
          alignerId: "Qwen/Qwen3-ForcedAligner-0.6B",
          alignerCached: asrReady,
          diarizeModelId: "pyannote/speaker-diarization-3.1",
          diarizeModelCached: asrReady,
          diarizePythonPath: "/Users/example/.lumen-cut/runtime/bin/python3",
          diarizeRuntimeReady: asrReady,
          diarizeRuntimeDetail: "pyannote.audio 3.4.0",
          huggingFaceTokenSet: asrReady,
          diarizeReady: asrReady,
          ready: asrReady,
        };
      case "transcription_status":
        if (transcriptionStatusError) throw transcriptionStatusError;
        return transcriptionStatusState;
      case "transcription_start":
        return { pid: "project-1", state: "running", phase: "preparing", progress: 0 };
      case "media_asset_allow":
        if (mediaAllowError) throw mediaAllowError;
        return "/Users/example/Interview.mp4";
      case "project_media_status":
        return {
          path: "/Users/example/missing/Interview.mp4",
          available: mediaAllowError === null,
          fileSize: null,
          expectedDurationSeconds: 95,
          issue: mediaAllowError ? "the original media file is missing or was moved" : null,
          suggestedPath: mediaAllowError ? "/Users/example/recovered/Interview.mp4" : null,
        };
      case "project_media_relink":
        mediaAllowError = null;
        projectDoc = {
          ...projectDoc,
          media: {
            ...(projectDoc.media as Record<string, unknown>),
            path: "/Users/example/Interview-restored.mp4",
          },
        };
        return projectDoc;
      default:
        throw new Error(`unexpected command: ${command}`);
    }
  });
});

test("opening a project renders the serialized media details", async () => {
  render(<App />);

  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  expect(await screen.findByRole("heading", { name: "Interview" })).toBeVisible();
  expect(screen.getAllByText("2212.8s")[0]).toBeVisible();
  expect(invoke).toHaveBeenCalledWith("project_mark_opened", {
    pid: "project-1",
    root: null,
  });
});

test("project library loads a generated cover without blocking project actions", async () => {
  render(<App />);

  await waitFor(() => expect(
    document.querySelector(
      'img[src="asset:///projects/project-1/.lumen-cut/project-thumbnail/thumbnail.jpg"]',
    ),
  ).not.toBeNull());
  expect(await screen.findByRole("button", { name: /Interview.*打开项目/ })).toBeEnabled();
});

test("project library explains missing media before the editor is opened", async () => {
  projectListState = [{
    ...projectListState[0],
    media_available: false,
  }];
  projectThumbnailState = {
    path: null,
    mediaAvailable: false,
    deferred: false,
  };

  render(<App />);

  expect(await screen.findByText("媒体已移动")).toBeVisible();
  expect(document.querySelector(".project-row .project-cover.offline")).not.toBeNull();
  expect(screen.getByRole("button", { name: /Interview.*打开项目/ })).toBeEnabled();
});

test("an empty project library shows the explicit three-step onboarding path", async () => {
  projectListState = [];

  render(<App />);

  expect(await screen.findByRole("heading", { name: "接下来会发生什么" })).toBeVisible();
  expect(screen.getByText("导入媒体")).toBeVisible();
  expect(screen.getByText("确认并转写")).toBeVisible();
  expect(screen.getByText("编辑并导出")).toBeVisible();
});

test("background tasks open a user-facing processing center without server setup", async () => {
  render(<App />);

  fireEvent.click(screen.getByRole("button", { name: "后台任务" }));

  expect(await screen.findByRole("heading", { name: "处理中心" })).toBeVisible();
  expect(screen.getByText(/不需要手动启动服务器/)).toBeVisible();
  expect(screen.queryByRole("button", { name: /start server/i })).not.toBeInTheDocument();
});

test("processing center explains compute queues and flags a job with no fresh progress", async () => {
  transcriptionStatusState = {
    pid: "project-1",
    state: "running",
    phase: "transcribing",
    progress: 42,
    updatedAt: 1,
    cpuPercent: 178,
    peakMemoryMb: 5700,
    memoryLimitMb: 6144,
  };
  performanceState = {
    activePipeline: "transcription",
    waitingPipelines: 2,
  };

  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: "后台任务" }));

  expect(await screen.findByText("当前重计算：转写与时码")).toBeVisible();
  expect(screen.getByText(/2 个任务正在排队.*不需要启动服务器/)).toBeVisible();
  expect(screen.getByText(/转写与时码 · 识别语音/)).toBeVisible();
  expect(screen.getByText(/内存峰值 5700 \/ 6144 MB · 93%/)).toBeVisible();
  expect(screen.getByText(/接近内存护栏/)).toBeVisible();
  expect(screen.getByText(/没有新进度.*安全停止后重试/)).toBeVisible();
});

test("processing center surfaces a corrupt persisted job status instead of hiding it", async () => {
  transcriptionStatusError = new Error("invalid job status JSON");

  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: "后台任务" }));

  expect(await screen.findByText(/Transcription: invalid job status JSON/)).toBeVisible();
  expect(screen.getByRole("button", { name: "打开日志" })).toBeEnabled();
});

test("a project rendering error shows recovery UI instead of a white window", async () => {
  vi.spyOn(console, "error").mockImplementation(() => undefined);
  projectDoc = { ...projectDoc, media: { path: "/Users/example/broken.mp4" } };
  render(<App />);

  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  expect(await screen.findByRole("alert")).toHaveTextContent("界面出现问题");
  expect(screen.getByRole("button", { name: "重新载入" })).toBeVisible();
});

test("missing project media can be relinked without losing the editor", async () => {
  mediaAllowError = new Error("original media was moved");
  render(<App />);

  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  expect(await screen.findByText("项目媒体已断开")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "重新定位媒体…" }));

  await waitFor(() => expect(invoke).toHaveBeenCalledWith(
    "project_media_relink",
    {
      pid: "project-1",
      path: "/Users/example/Interview-restored.mp4",
      root: null,
    },
  ));
  expect(await screen.findByText(/项目媒体已重新连接/)).toBeVisible();
});

test("a unique nearby media match can be validated and reconnected in one click", async () => {
  mediaAllowError = new Error("original media was moved");
  render(<App />);

  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  const suggestion = await screen.findByRole("button", {
    name: "连接找到的文件：Interview.mp4",
  });
  expect(suggestion).toHaveAttribute(
    "title",
    "/Users/example/recovered/Interview.mp4",
  );
  fireEvent.click(suggestion);

  await waitFor(() => expect(invoke).toHaveBeenCalledWith(
    "project_media_relink",
    {
      pid: "project-1",
      path: "/Users/example/recovered/Interview.mp4",
      root: null,
    },
  ));
  expect(await screen.findByText(/项目媒体已重新连接/)).toBeVisible();
});

test("corrupt optional B-roll data does not block the transcript editor", async () => {
  brollListError = new Error("invalid broll.json");
  render(<App />);

  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  expect(await screen.findByRole("heading", { name: "Interview" })).toBeVisible();
  expect(screen.getByText(/B-roll 数据无法加载，转写稿仍可继续编辑/)).toBeVisible();
  expect(screen.queryByText("界面出现问题")).not.toBeInTheDocument();
});

test("a serialized transcript project can open every editor surface", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Alice",
      sentences: [{
        id: "s1",
        text: "Hello world",
        words: [
          { id: "w1", text: "Hello", start: 0, end: 0.5 },
          { id: "w2", text: "world", start: 0.5, end: 1 },
        ],
      }],
    }],
  };
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  const tabs = await screen.findByRole("navigation", { name: "编辑主流程" });
  expect(Array.from(tabs.querySelectorAll<HTMLButtonElement>(".editor-tab-primary"))
    .map((button) => button.textContent))
    .toEqual(["转写稿", "字幕", "翻译", "样式"]);
  for (const label of ["转写稿", "字幕", "翻译", "样式"]) {
    const tab = within(tabs).getByRole("button", { name: label });
    fireEvent.click(tab);
    expect(tab).toHaveAttribute("aria-current", "page");
    expect(screen.queryByText("界面出现问题")).not.toBeInTheDocument();
  }
  fireEvent.click(within(tabs).getByRole("button", { name: "打开更多编辑工具" }));
  for (const label of ["转写设置", "说话人", "章节", "时间线", "补充画面"]) {
    const tool = screen.getByRole("menuitem", { name: new RegExp(`^${label}`) });
    fireEvent.click(tool);
    expect(screen.queryByText("界面出现问题")).not.toBeInTheDocument();
    if (label !== "补充画面") {
      fireEvent.click(within(tabs).getByRole("button", { name: /打开更多编辑工具/ }));
    }
  }
  const editorHeader = screen.getByRole("heading", { name: "Interview" }).closest("header");
  if (!editorHeader) throw new Error("editor header was not rendered");
  for (const label of ["项目", "历史", "检查", "导出作品"]) {
    fireEvent.click(within(editorHeader).getByRole("button", { name: label }));
    expect(screen.queryByText("界面出现问题")).not.toBeInTheDocument();
  }
});

test("the editor tools menu supports keyboard discovery and escape recovery", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Keyboard navigation",
        words: [{ id: "w1", text: "Keyboard", start: 0, end: 1 }],
      }],
    }],
  };

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  const workflow = await screen.findByRole("navigation", { name: "编辑主流程" });
  const menuButton = within(workflow).getByRole("button", { name: "打开更多编辑工具" });
  menuButton.focus();
  fireEvent.keyDown(menuButton, { key: "ArrowDown" });

  const transcription = await screen.findByRole("menuitem", { name: /^转写设置/ });
  await waitFor(() => expect(transcription).toHaveFocus());
  fireEvent.keyDown(transcription, { key: "ArrowDown" });
  expect(screen.getByRole("menuitem", { name: /^说话人/ })).toHaveFocus();

  fireEvent.keyDown(document, { key: "Escape" });
  expect(menuButton).toHaveFocus();
  expect(menuButton).toHaveAttribute("aria-expanded", "false");
});

test("generated chapters are visible, editable, seekable, and regenerable", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [
        {
          id: "s1",
          text: "Opening context",
          words: [{ id: "w1", text: "Opening", start: 0, end: 1 }],
        },
        {
          id: "s2",
          text: "Core proposal",
          words: [{ id: "w2", text: "Core", start: 10, end: 11 }],
        },
      ],
    }],
  };
  subtitleRowsState = [
    { id: "s1", text: "Opening context", speaker: "Host", hidden: false, start: 0, end: 1 },
    { id: "s2", text: "Core proposal", speaker: "Host", hidden: false, start: 10, end: 11 },
  ];
  chapterRowsState = [
    { title: "Introduction", startSeg: "s1", start: 0, end: 10, preview: "Opening context" },
    { title: "Proposal", startSeg: "s2", start: 10, end: 30, preview: "Core proposal" },
  ];
  llmConfigured = true;

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "打开更多编辑工具" }));
  fireEvent.click(screen.getByRole("menuitem", { name: /^章节/ }));

  expect((await screen.findAllByText("Opening context")).length).toBeGreaterThan(0);
  expect(screen.getAllByText("Core proposal").length).toBeGreaterThan(0);
  expect(screen.getByRole("button", { name: /跳到章节 2: Proposal/ })).toBeVisible();
  fireEvent.change(screen.getByRole("textbox", { name: "章节标题 2" }), {
    target: { value: "The core proposal" },
  });
  expect(screen.getByText("1 个标题未保存")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "保存章节" }));

  expect(await screen.findByText("已保存 2 个章节。")).toBeVisible();
  expect(invoke).toHaveBeenCalledWith("chapter_set_many", {
    pid: "project-1",
    chapters: [
      { title: "Introduction", startSeg: "s1" },
      { title: "The core proposal", startSeg: "s2" },
    ],
    root: null,
  });

  fireEvent.click(screen.getByRole("button", { name: "重新生成" }));
  fireEvent.click(screen.getByRole("button", { name: "确认重新生成" }));
  expect(invoke).toHaveBeenCalledWith("task_start", {
    args: {
      kind: "chapters",
      pid: "project-1",
      lang: null,
      root: null,
      stale_only: false,
    },
  });
});

test("unsaved chapter titles survive an application restart", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [
        {
          id: "s1",
          text: "Opening context",
          words: [{ id: "w1", text: "Opening", start: 0, end: 1 }],
        },
        {
          id: "s2",
          text: "Core proposal",
          words: [{ id: "w2", text: "Core", start: 10, end: 11 }],
        },
      ],
    }],
  };
  subtitleRowsState = [
    { id: "s1", text: "Opening context", speaker: "Host", hidden: false, start: 0, end: 1 },
    { id: "s2", text: "Core proposal", speaker: "Host", hidden: false, start: 10, end: 11 },
  ];
  chapterRowsState = [
    { title: "Introduction", startSeg: "s1", start: 0, end: 10, preview: "Opening context" },
    { title: "Proposal", startSeg: "s2", start: 10, end: 30, preview: "Core proposal" },
  ];

  const first = render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  await openEditorTool("章节");
  fireEvent.change(await screen.findByRole("textbox", { name: "章节标题 2" }), {
    target: { value: "尚未保存的核心方案" },
  });

  await waitFor(() => expect(
    JSON.parse(localStorage.getItem("lumen-cut.chapterDrafts") || "{}"),
  ).toMatchObject({
    "project-1": {
      s2: {
        sourceTitle: "Proposal",
        title: "尚未保存的核心方案",
      },
    },
  }));

  first.unmount();
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  await openEditorTool("章节");

  expect(await screen.findByRole("textbox", { name: "章节标题 2" }))
    .toHaveValue("尚未保存的核心方案");
  expect(screen.getByText("1 个标题未保存")).toBeVisible();
});

test("the subtitle workspace exposes delivery timing metrics instead of duplicating transcript mode", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "This caption is deliberately dense",
        words: [{ id: "w1", text: "dense", start: 0, end: 0.4 }],
      }],
    }],
  };
  subtitleRowsState = [{
    id: "s1",
    text: "This caption is deliberately dense",
    speaker: "Host",
    hidden: false,
    start: 0,
    end: 0.4,
  }];

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  expect(screen.queryByRole("combobox", { name: "字幕质量筛选" })).not.toBeInTheDocument();

  fireEvent.click(await screen.findByRole("button", { name: "字幕" }));
  expect(screen.getByRole("combobox", { name: "字幕质量筛选" })).toBeVisible();
  expect(screen.getByText("显示时间过短")).toBeVisible();
  expect(screen.getByText("1 条需检查阅读节奏")).toBeVisible();
  fireEvent.change(screen.getByRole("combobox", { name: "字幕质量筛选" }), {
    target: { value: "issues" },
  });
  expect(screen.getByRole("textbox", { name: "字幕 1" })).toHaveValue(
    "This caption is deliberately dense",
  );
});

test("subtitle timing can be fine-tuned inside the neighboring cue window", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [
        {
          id: "s1",
          text: "Hello there",
          words: [
            { id: "w1", text: "Hello", start: 1, end: 1.5 },
            { id: "w2", text: "there", start: 1.5, end: 2 },
          ],
        },
        {
          id: "s2",
          text: "Next line",
          words: [{ id: "w3", text: "Next", start: 3, end: 4 }],
        },
      ],
    }],
  };
  subtitleRowsState = [
    { id: "s1", text: "Hello there", speaker: "Host", hidden: false, start: 1, end: 2 },
    { id: "s2", text: "Next line", speaker: "Host", hidden: false, start: 3, end: 4 },
  ];

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "字幕" }));
  const editor = screen.getByRole("textbox", { name: "字幕 1" });
  fireEvent.click(within(editor.closest("article")!).getByRole("button", {
    name: "时码 / 结构",
  }));
  expect(screen.getByText(/可用窗口 00:00.0–00:03.0/)).toBeVisible();
  fireEvent.change(screen.getByRole("spinbutton", { name: "字幕开始 1" }), {
    target: { value: "0.75" },
  });
  fireEvent.change(screen.getByRole("spinbutton", { name: "字幕结束 1" }), {
    target: { value: "2.5" },
  });
  fireEvent.click(screen.getByRole("button", { name: "应用时码" }));

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("subtitle_timing_set", {
    pid: "project-1",
    id: "s1",
    start: 0.75,
    end: 2.5,
    root: null,
  }));
  expect(await screen.findByText(/字幕时码已更新为 0.75s–2.50s，可撤销/)).toBeVisible();
  expect(screen.getByText("1.8s")).toBeVisible();
});

test("saving one transcript line updates locally without reloading the whole project", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Original line",
        words: [{ id: "w1", text: "Original", start: 0, end: 1 }],
      }],
    }],
  };
  subtitleRowsState = [{
    id: "s1",
    text: "Original line",
    speaker: "Host",
    hidden: false,
    start: 0,
    end: 1,
  }];

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  const projectLoadsBeforeSave = invoke.mock.calls.filter(
    ([command]) => command === "project_show",
  ).length;

  const editor = await screen.findByRole("textbox", { name: "字幕 1" });
  fireEvent.change(editor, { target: { value: "Corrected line" } });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));

  expect(await screen.findByText("这句转写已保存。")).toBeVisible();
  expect(editor).toHaveValue("Corrected line");
  fireEvent.click(within(editor.closest("article")!).getByRole("button", { name: "拆分 / 合并" }));
  expect(editor.closest("article")?.querySelector(".split-word-stream")).toHaveTextContent(
    "Corrected",
  );
  expect(editor.closest("article")?.querySelector(".split-word-stream")).toHaveTextContent("line");
  expect(invoke).toHaveBeenCalledWith("subtitle_update_many", {
    pid: "project-1",
    updates: [{ id: "s1", text: "Corrected line" }],
    root: null,
  });
  expect(invoke.mock.calls.filter(([command]) => command === "project_show")).toHaveLength(
    projectLoadsBeforeSave,
  );
  expect(invoke.mock.calls.filter(([command]) => command === "edit_history_status").length)
    .toBeGreaterThan(1);
});

test("transcript drafts survive row saves and tab switches, then save atomically", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [
        {
          id: "s1",
          text: "First original",
          words: [{ id: "w1", text: "First", start: 0, end: 1 }],
        },
        {
          id: "s2",
          text: "Second original",
          words: [{ id: "w2", text: "Second", start: 1, end: 2 }],
        },
      ],
    }],
  };
  subtitleRowsState = [
    {
      id: "s1",
      text: "First original",
      speaker: "Host",
      hidden: false,
      start: 0,
      end: 1,
    },
    {
      id: "s2",
      text: "Second original",
      speaker: "Host",
      hidden: false,
      start: 1,
      end: 2,
    },
  ];

  const { unmount } = render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  const first = await screen.findByRole("textbox", { name: "字幕 1" });
  const second = screen.getByRole("textbox", { name: "字幕 2" });
  fireEvent.change(first, { target: { value: "First corrected" } });
  fireEvent.change(second, { target: { value: "Second corrected" } });
  expect(screen.getByText("2 条修改未保存")).toBeVisible();

  fireEvent.click(within(first.closest("article")!).getByRole("button", { name: "保存" }));
  expect(await screen.findByText("这句转写已保存。")).toBeVisible();
  expect(screen.getByRole("textbox", { name: "字幕 2" })).toHaveValue("Second corrected");
  expect(screen.getByText("1 条修改未保存")).toBeVisible();

  const tabs = screen.getByRole("navigation", { name: "编辑主流程" });
  fireEvent.click(within(tabs).getByRole("button", { name: "翻译" }));
  fireEvent.click(within(tabs).getByRole("button", { name: "转写稿" }));
  expect(screen.getByRole("textbox", { name: "字幕 2" })).toHaveValue("Second corrected");
  expect(screen.getByText("1 条修改未保存")).toBeVisible();

  fireEvent.click(screen.getByRole("button", { name: "设置" }));
  fireEvent.click(screen.getByRole("button", { name: "编辑" }));
  expect(await screen.findByRole("textbox", { name: "字幕 2" })).toHaveValue("Second corrected");

  unmount();
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  expect(await screen.findByRole("textbox", { name: "字幕 2" })).toHaveValue("Second corrected");
  expect(screen.getByText("1 条修改未保存")).toBeVisible();

  subtitleSetError = new Error("disk is temporarily unavailable");
  fireEvent.click(screen.getByRole("button", { name: "全部保存" }));
  expect(await screen.findByText("disk is temporarily unavailable")).toBeVisible();
  expect(screen.getByRole("textbox", { name: "字幕 2" })).toHaveValue("Second corrected");
  expect(screen.getByText("1 条修改未保存")).toBeVisible();

  subtitleSetError = null;
  fireEvent.click(screen.getByRole("button", { name: "全部保存" }));
  expect(await screen.findByText("已原子保存 1 条转写（1 条有变化），可一次撤销。")).toBeVisible();
  expect(invoke).toHaveBeenCalledWith("subtitle_update_many", {
    pid: "project-1",
    updates: [{ id: "s2", text: "Second corrected" }],
    root: null,
  });
  expect(screen.queryByText(/修改未保存/)).not.toBeInTheDocument();
});

test("speaker re-identification is previewed before it can change labels", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Alice",
      sentences: [{
        id: "s1",
        text: "Hello world",
        words: [
          { id: "w1", text: "Hello", start: 0, end: 0.5 },
          { id: "w2", text: "world", start: 0.5, end: 1 },
        ],
      }],
    }],
  };
  speakerEvidenceState = {
    speakers: [{ id: "Alice", paragraph_count: 1 }],
    turns: [{
      paragraphId: 1,
      speaker: "Alice",
      start: 0,
      end: 1,
      text: "Hello world",
      cueIds: ["s1"],
    }],
    identified: true,
    unlabelled: 0,
  };

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  const editorTabs = await screen.findByRole("navigation", { name: "编辑主流程" });
  expect(Array.from(editorTabs.querySelectorAll<HTMLButtonElement>(".editor-tab-primary"))
    .map((button) => button.textContent))
    .toEqual(["转写稿", "字幕", "翻译", "样式"]);
  expect(screen.getByText("1 位说话人 · 结果已保存")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "管理说话人" }));
  expect(within(editorTabs).getByText("说话人")).toBeVisible();
  expect(invoke).not.toHaveBeenCalledWith("speaker_reidentify_start", expect.anything());

  expect(await screen.findByText("逐段证据")).toBeVisible();
  expect(screen.getByText("Hello world")).toBeVisible();
  expect(screen.getByText("结果已保存")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "重新识别说话人" }));
  expect(screen.getByText("确认重新识别？")).toBeVisible();
  expect(invoke).not.toHaveBeenCalledWith("speaker_reidentify_start", expect.anything());
  fireEvent.click(screen.getByRole("button", { name: "确认重新识别" }));

  expect(await screen.findByText("1 个字幕片段标签将改变")).toBeVisible();
  expect(screen.getByText("Alice → SPEAKER_00")).toBeVisible();
  expect(invoke).toHaveBeenCalledWith("speaker_reidentify_start", {
    pid: "project-1",
    root: null,
  });
  expect(invoke).toHaveBeenCalledWith("speaker_reidentify_status", {
    pid: "project-1",
  });
  expect(invoke).not.toHaveBeenCalledWith("speaker_reidentify_apply", expect.anything());
  expect(screen.getByRole("button", { name: "请先勾选" })).toBeDisabled();

  fireEvent.click(screen.getByRole("checkbox", { name: "选择字幕片段 1" }));
  fireEvent.click(screen.getByRole("button", { name: "应用 1 项" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("speaker_reidentify_apply", {
    pid: "project-1",
    proposals: [expect.objectContaining({ paragraphId: 1 })],
    root: null,
  }));
});

test("speaker analysis exposes its current phase and real progress", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: null,
      sentences: [{
        id: "s1",
        text: "Hello world",
        words: [
          { id: "w1", text: "Hello", start: 0, end: 0.5 },
          { id: "w2", text: "world", start: 0.5, end: 1 },
        ],
      }],
    }],
  };
  speakerAnalysisStatusState = {
    pid: "project-1",
    state: "running",
    phase: "embedding",
    progress: 81,
    current: 3,
    total: 5,
    error: null,
    preview: null,
  };
  speakerAnalysisStartState = speakerAnalysisStatusState;

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  await openEditorTool("说话人");
  fireEvent.click(screen.getByRole("button", { name: "分析说话人" }));

  expect(await screen.findByText("正在提取声纹特征")).toBeVisible();
  expect(screen.getByText("81%")).toBeVisible();
  expect(screen.getByRole("progressbar", { name: "说话人分析进度" })).toHaveValue(81);
  expect(screen.getByText(/处理批次 3 \/ 5/)).toBeVisible();
});

test("a completed speaker proposal is restored after reopening the app", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Alice",
      sentences: [{
        id: "s1",
        text: "Hello world",
        words: [
          { id: "w1", text: "Hello", start: 0, end: 0.5 },
          { id: "w2", text: "world", start: 0.5, end: 1 },
        ],
      }],
    }],
  };
  speakerAnalysisHasExistingJob = true;

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  await openEditorTool("说话人");

  expect(await screen.findByText("1 个字幕片段标签将改变")).toBeVisible();
  expect(screen.getByText("Alice → SPEAKER_00")).toBeVisible();
});

test("unfinished AI tasks resume automatically when a project is reopened", async () => {
  taskStatusState = {
    pending: 1,
    done: 0,
    kinds: [{ kind: "translate", pending: 1, done: 0, failed: 0 }],
    polishQuality: null,
  };

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  await waitFor(() => {
    expect(invoke).toHaveBeenCalledWith("task_resume", {
      pid: "project-1",
      root: null,
    });
  });
  expect(await screen.findByText(/已恢复 1 个未完成任务/)).toBeVisible();
});

test("translation shows completed batches instead of an indefinite running state", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Hello world",
        words: [{ id: "w1", text: "Hello", start: 0, end: 1 }],
      }],
    }],
  };
  taskStatusState = {
    pending: 8,
    done: 2,
    kinds: [{ kind: "translate", lang: "zh", calls: 10, pending: 8, done: 2, failed: 0 }],
    polishQuality: null,
  };

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  const workflow = await screen.findByRole("navigation", { name: "编辑主流程" });
  fireEvent.click(within(workflow).getByRole("button", { name: /翻译/ }));

  expect(await screen.findByText("已完成 2 / 10 批")).toBeVisible();
  expect(screen.getByRole("progressbar", { name: "翻译进度" })).toHaveAttribute("value", "2");
  expect(screen.getByRole("progressbar", { name: "翻译进度" })).toHaveAttribute("max", "10");
});

test("translation supports custom language tags and exposes another unfinished language", async () => {
  llmConfigured = true;
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Hello world",
        words: [{ id: "w1", text: "Hello", start: 0, end: 1 }],
      }],
    }],
  };
  taskStatusState = {
    pending: 8,
    done: 2,
    kinds: [{
      kind: "translate",
      lang: "zh",
      calls: 10,
      pending: 8,
      done: 2,
      failed: 0,
      state: "paused",
    }],
    polishQuality: null,
  };

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "翻译" }));
  fireEvent.change(screen.getByRole("combobox", { name: "目标语言" }), {
    target: { value: "ja" },
  });
  expect(await screen.findByText(/ZH 翻译仍有 8 个批次未完成/)).toBeVisible();
  expect(screen.getByRole("button", { name: "其他语言任务待处理" })).toBeDisabled();

  fireEvent.click(screen.getByRole("button", { name: "切回该任务" }));
  expect(screen.getByRole("combobox", { name: "目标语言" })).toHaveValue("zh");

  fireEvent.change(screen.getByRole("textbox", { name: "自定义目标语言代码" }), {
    target: { value: "de-CH" },
  });
  fireEvent.click(screen.getByRole("button", { name: "使用" }));
  expect(screen.getByRole("combobox", { name: "目标语言" })).toHaveValue("de-CH");
});

test("translation text can be edited and saved per cue", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Hello world",
        words: [{ id: "w1", text: "Hello", start: 0, end: 1 }],
      }],
    }],
    translations: {
      zh: { s1: { text: "你好世界" } },
    },
  };

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "翻译" }));

  const editor = await screen.findByRole("textbox", { name: /编辑译文 00:00/ });
  fireEvent.change(editor, { target: { value: "世界你好" } });
  fireEvent.blur(editor);

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("translation_set", {
    pid: "project-1",
    lang: "zh",
    id: "s1",
    text: "世界你好",
    root: null,
  }));
});

test("unsaved translation drafts survive an application restart", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Hello world",
        words: [{ id: "w1", text: "Hello", start: 0, end: 1 }],
      }],
    }],
    translations: {
      zh: { s1: { text: "你好世界" } },
    },
  };

  const first = render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "翻译" }));
  fireEvent.change(
    await screen.findByRole("textbox", { name: /编辑译文 00:00/ }),
    { target: { value: "尚未保存的译文" } },
  );

  await waitFor(() => expect(
    JSON.parse(localStorage.getItem("lumen-cut.translationDrafts") || "{}"),
  ).toMatchObject({
    "project-1": {
      zh: {
        s1: {
          text: "尚未保存的译文",
          savedText: "你好世界",
        },
      },
    },
  }));

  first.unmount();
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "翻译" }));

  expect(await screen.findByRole("textbox", { name: /编辑译文 00:00/ }))
    .toHaveValue("尚未保存的译文");
  expect(screen.getByText("1 条修改尚未保存")).toBeVisible();
});

test("unsaved translations are batch-saved before switching target language", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Hello world",
        words: [{ id: "w1", text: "Hello", start: 0, end: 1 }],
      }],
    }],
    translations: {
      zh: { s1: { text: "你好世界" } },
    },
  };

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "翻译" }));

  const editor = await screen.findByRole("textbox", { name: /编辑译文 00:00/ });
  fireEvent.change(editor, { target: { value: "世界你好" } });
  expect(screen.getByText("1 条修改尚未保存")).toBeVisible();

  fireEvent.change(screen.getByRole("combobox", { name: "目标语言" }), {
    target: { value: "ja" },
  });
  expect(screen.getByText(/切换到 日本語 前/)).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "保存并切换" }));

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("translation_set_many", {
    pid: "project-1",
    lang: "zh",
    updates: [{ id: "s1", text: "世界你好" }],
    root: null,
  }));
  await waitFor(() => expect(
    screen.getByRole("combobox", { name: "目标语言" }),
  ).toHaveValue("ja"));
});

test("translation updates only stale lines unless full retranslation is explicitly confirmed", async () => {
  llmConfigured = true;
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Hello changed world",
        words: [{ id: "w1", text: "Hello", start: 0, end: 1 }],
      }],
    }],
    translations: {
      zh: { s1: { text: "你好世界", sourceText: "Hello world" } },
    },
  };

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "翻译" }));
  fireEvent.click(await screen.findByRole("button", { name: "更新 1 条变化" }));

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("task_start", {
    args: {
      kind: "translate",
      pid: "project-1",
      lang: "zh",
      root: null,
      stale_only: true,
    },
  }));

  fireEvent.click(screen.getByRole("button", { name: "重新翻译全部…" }));
  fireEvent.click(screen.getByRole("button", { name: "确认全部重译" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("task_start", {
    args: {
      kind: "translate",
      pid: "project-1",
      lang: "zh",
      root: null,
      stale_only: false,
    },
  }));
});

test("an active translation can be paused from the translation workspace", async () => {
  llmConfigured = true;
  taskStatusState = {
    pending: 1,
    done: 1,
    kinds: [{
      kind: "translate",
      lang: "zh",
      pending: 1,
      done: 1,
      failed: 0,
      calls: 2,
      state: "running",
    }],
    polishQuality: null,
  };
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Hello world",
        words: [{ id: "w1", text: "Hello", start: 0, end: 1 }],
      }],
    }],
    translations: {
      zh: { s1: { text: "你好世界", sourceText: "Hello world" } },
    },
  };

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  const workflow = await screen.findByRole("navigation", { name: "编辑主流程" });
  fireEvent.click(within(workflow).getByRole("button", { name: /翻译/ }));
  fireEvent.click(await screen.findByRole("button", { name: "暂停翻译" }));

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("task_pause", {
    pid: "project-1",
    kind: "translate",
    root: null,
  }));
  expect(await screen.findByText(/3 个排队批次已停止，1 个在途请求完成后会安全保存/)).toBeVisible();
});

test("setup blocks transcription until the local runtime and models are ready", async () => {
  asrReady = false;
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  expect(await screen.findByText(/本地转写尚未准备好/)).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "准备转写环境" }));

  expect(await screen.findByRole("heading", { name: "设置" })).toBeVisible();
  expect(invoke).not.toHaveBeenCalledWith("transcription_start", expect.anything());
});

test("returning from setup refreshes model readiness without reopening the project", async () => {
  asrReady = false;
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "准备转写环境" }));

  asrReady = true;
  fireEvent.click(screen.getByRole("button", { name: "编辑" }));

  expect(await screen.findByRole("button", { name: "开始转写" })).toBeVisible();
  await waitFor(() =>
    expect(screen.queryByText(/本地转写尚未准备好/)).not.toBeInTheDocument());
  fireEvent.click(screen.getByRole("button", { name: "开始转写" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("transcription_start", {
    args: {
      pid: "project-1",
      media: "/Users/example/Interview.mp4",
      lang: null,
      title: "Interview",
      out: null,
      model: null,
    },
  }));
});

test("a project load failure offers retry and recovers without a permanent spinner", async () => {
  projectShowError = new Error("doc.json is temporarily unavailable");
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  expect(await screen.findByRole("heading", { name: "项目数据暂时无法读取" })).toBeVisible();
  expect(screen.getByText("doc.json is temporarily unavailable")).toBeVisible();
  expect(screen.queryByText("正在打开项目…")).not.toBeInTheDocument();

  projectShowError = null;
  fireEvent.click(screen.getByRole("button", { name: "重试打开" }));

  expect(await screen.findByRole("button", { name: "开始转写" })).toBeVisible();
  expect(screen.queryByRole("heading", { name: "项目数据暂时无法读取" })).not.toBeInTheDocument();
});

test("runtime setup runs as a cancellable background job with explicit indeterminate progress", async () => {
  asrReady = false;
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: "设置" }));
  fireEvent.click(await screen.findByRole("button", { name: "安装或修复转写引擎" }));

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("setup_job_start", {
    kind: "asr-runtime",
  }));
  expect(screen.getByRole("progressbar", { name: "环境准备进度" })).not.toHaveAttribute("value");
  expect(screen.getByText(/没有提供可信的总字节数/)).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "取消任务" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("setup_job_cancel"));
});

test("advanced diagnostics cannot imply that users must start a server", async () => {
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: "设置" }));
  fireEvent.click(screen.getByText("高级诊断"));

  expect(screen.getByText(/不需要手动启动 Pipeline 或服务器/)).toBeVisible();
  expect(screen.queryByRole("button", { name: /start server/i })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "运行环境检查" })).toBeVisible();
});

test("timeline keyboard shortcuts are discoverable, replayable, and restore focus", async () => {
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  await screen.findByRole("button", { name: "开始转写" });

  fireEvent.keyDown(window, { key: "?", code: "Slash", shiftKey: true });
  expect(screen.getByText("编辑快捷键")).toBeVisible();
  expect(screen.getByText("播放 / 暂停")).toBeVisible();
  expect(screen.getByText("保存当前转写或翻译")).toBeVisible();

  fireEvent.keyDown(window, { key: "Escape" });
  expect(screen.queryByText("编辑快捷键")).not.toBeInTheDocument();
  const shortcutButton = screen.getByRole("button", { name: "快捷键" });
  expect(shortcutButton).toHaveFocus();
  expect(fireEvent.keyDown(shortcutButton, { key: " ", code: "Space" })).toBe(true);
});

test("a long transcription reports the word-alignment phase instead of looking stuck", async () => {
  transcriptionStatusState = {
    pid: "project-1",
    state: "running",
    phase: "aligning",
    progress: 81,
    error: null,
  };
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  expect(await screen.findAllByText("正在生成词级时码")).not.toHaveLength(0);
  expect(screen.getByText("81%")).toBeVisible();
  expect(screen.getByRole("progressbar")).toHaveValue(81);
});

test("an interrupted transcription is explained and can be retried", async () => {
  transcriptionStatusState = {
    pid: "project-1",
    state: "failed",
    phase: "failed",
    progress: 52,
    error: "the previous transcription was interrupted when lumen-cut closed; retry to start it again",
  };
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  expect(await screen.findByRole("alert")).toHaveTextContent("上次转写因 lumen-cut 关闭而中断");
  fireEvent.click(screen.getByRole("button", { name: "重试" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("transcription_start", {
    args: expect.objectContaining({ pid: "project-1" }),
  }));
});

test("retranscription requires confirmation and explains its automatic recovery point", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Existing transcript",
        words: [{ id: "w1", text: "Existing", start: 0, end: 1 }],
      }],
    }],
    translations: {
      zh: { s1: { text: "现有译文", sourceText: "Existing transcript" } },
    },
  };

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  await openEditorTool("转写设置");
  fireEvent.click(screen.getByRole("button", { name: "重新转写" }));

  expect(screen.getByRole("alert")).toHaveTextContent("重新转写会替换当前转写稿");
  expect(screen.getByRole("alert")).toHaveTextContent("自动保存完整恢复版本");
  expect(invoke).not.toHaveBeenCalledWith("transcription_start", expect.anything());

  fireEvent.click(screen.getByRole("button", { name: "保存恢复点并重新转写" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("transcription_start", {
    args: expect.objectContaining({ pid: "project-1" }),
  }));
});

test("settings exposes the real local transcription status", async () => {
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: "设置" }));

  expect(await screen.findByRole("heading", { name: "转写引擎与说话人" })).toBeVisible();
  expect(screen.getByText(/mlx-qwen3-asr 0.3.5/)).toBeVisible();
  expect(screen.getAllByText("模型已下载")).toHaveLength(3);
  expect(screen.getByText(/pyannote\.audio 3\.4\.0/)).toBeVisible();
  expect(screen.queryByRole("button", { name: /start server/i })).not.toBeInTheDocument();
});

test("AI settings configure a provider without requiring a pipeline server", async () => {
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: "设置" }));

  const provider = await screen.findByRole("combobox", { name: "模型服务商" });
  expect(provider).toHaveValue("none");
  expect(screen.getByText(/AI 功能未启用/)).toBeVisible();

  fireEvent.change(provider, { target: { value: "deepseek" } });
  expect(screen.getByLabelText("模型")).toHaveValue("deepseek-chat");
  expect(screen.getByText("还需要补全下方必填项")).toBeVisible();
  fireEvent.change(screen.getByLabelText("API Key"), { target: { value: "sk-test" } });
  expect(screen.getByText("必填项已完整；保存后将在首次 AI 任务时连接")).toBeVisible();

  fireEvent.click(screen.getByText("高级：查看或覆盖服务地址"));
  expect(screen.getByLabelText("服务地址")).toHaveValue(
    "https://api.deepseek.com/v1/chat/completions",
  );
  fireEvent.click(screen.getByRole("button", { name: "保存设置" }));

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("settings_export", {
    settings: expect.objectContaining({
      llm_endpoint: "https://api.deepseek.com/v1/chat/completions",
      llm_api_key: "sk-test",
      llm_model: "deepseek-chat",
      worker_count: 3,
    }),
  }));
  expect(screen.queryByRole("button", { name: /start server/i })).not.toBeInTheDocument();
});

test("AI settings expose OpenAI-compatible, MiniMax regions, and GLM presets", async () => {
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: "设置" }));

  const provider = await screen.findByRole("combobox", { name: "模型服务商" });
  expect(within(provider).getByRole("option", { name: "OpenAI Compatible · 自定义服务" })).toBeVisible();
  expect(within(provider).getByRole("option", { name: "MiniMax · 中国大陆" })).toBeVisible();
  expect(within(provider).getByRole("option", { name: "MiniMax · 海外" })).toBeVisible();
  expect(within(provider).getByRole("option", { name: "GLM 智谱 · 中国大陆" })).toBeVisible();
  expect(within(provider).getByRole("option", { name: "GLM Z.AI · 海外" })).toBeVisible();

  fireEvent.change(provider, { target: { value: "minimax-cn" } });
  expect(screen.getByLabelText("模型")).toHaveValue("MiniMax-M3");
  fireEvent.click(screen.getByText("高级：查看或覆盖服务地址"));
  expect(screen.getByLabelText("服务地址")).toHaveValue(
    "https://api.minimaxi.com/v1/chat/completions",
  );

  fireEvent.change(provider, { target: { value: "glm-global" } });
  expect(screen.getByLabelText("模型")).toHaveValue("glm-5.1");
  expect(screen.getByLabelText("服务地址")).toHaveValue(
    "https://api.z.ai/api/paas/v4/chat/completions",
  );

  fireEvent.change(provider, { target: { value: "custom" } });
  expect(screen.getByLabelText("服务地址")).toBeVisible();
  expect(screen.getByLabelText("API Key（可选）")).toBeVisible();
  fireEvent.change(screen.getByLabelText("服务地址"), {
    target: { value: "https://llm.example.com/v1/chat/completions" },
  });
  fireEvent.change(screen.getByLabelText("模型"), { target: { value: "my-model" } });
  expect(screen.getByText("必填项已完整；保存后将在首次 AI 任务时连接")).toBeVisible();
});

test("AI settings refresh the selected provider model catalog", async () => {
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: "设置" }));

  const provider = await screen.findByRole("combobox", { name: "模型服务商" });
  fireEvent.change(provider, { target: { value: "minimax-global" } });
  fireEvent.change(screen.getByLabelText("API Key"), { target: { value: "sk-minimax" } });
  fireEvent.click(screen.getByRole("button", { name: "获取最新模型" }));

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("llm_models_list", {
    endpoint: "https://api.minimax.io/v1/chat/completions",
    apiKey: "sk-minimax",
  }));
  expect(await screen.findByText("已从服务商获取 2 个模型")).toBeVisible();
  fireEvent.change(screen.getByLabelText("模型"), { target: { value: "MiniMax-M4-preview" } });
  expect(screen.getByLabelText("模型")).toHaveValue("MiniMax-M4-preview");
});

test("cloud transcription is configured without exposing its key to web storage", async () => {
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: "设置" }));

  fireEvent.change(
    await screen.findByRole("combobox", { name: "转写引擎" }),
    { target: { value: "openai-compatible" } },
  );
  expect(screen.getByText(/音频会从这台 Mac 直接上传/)).toBeVisible();
  fireEvent.change(screen.getByLabelText("转写 API Key"), {
    target: { value: "asr-secret" },
  });
  fireEvent.click(screen.getByRole("button", { name: "保存设置" }));

  await waitFor(() => expect(invoke).toHaveBeenCalledWith(
    "settings_export",
    expect.objectContaining({
      settings: expect.objectContaining({
        asr_engine: "openai-compatible",
        asr_cloud_endpoint: "https://api.openai.com/v1/audio/transcriptions",
        asr_cloud_api_key: "asr-secret",
        asr_cloud_model: "whisper-1",
      }),
    }),
  ));
  expect(localStorage.getItem("lumen-cut.settings.v1")).not.toContain("asr-secret");
  expect(screen.getByLabelText("转写 API Key")).toHaveValue("");
});

test("export requires a delivery check and exposes Final Cut output", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Ready to export",
        words: [{ id: "w1", text: "Ready", start: 0, end: 1 }],
      }],
    }],
  };
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "导出作品" }));

  expect(screen.getByRole("button", { name: /导出 Final Cut 工程/ })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "开始检查" }));
  await waitFor(() => expect(screen.getByText("当前版本可以交付")).toBeVisible());

  fireEvent.click(screen.getByRole("button", { name: /导出 Final Cut 工程/ }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("export_fcp", {
    pid: "project-1",
    root: null,
  }));
});

test("delivery preflight localizes blockers and links directly to the repair workspace", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Ready to export",
        words: [{ id: "w1", text: "Ready", start: 0, end: 1 }],
      }],
    }],
  };
  exportPreflightState = {
    ready: false,
    items: [
      { code: "settings", level: "pass", message: "settings are compatible" },
      { code: "media", level: "pass", message: "source media is readable" },
      { code: "timeline", level: "pass", message: "edited duration is 30s" },
      {
        code: "captions",
        level: "blocker",
        message: "translation track `zh-Hans` is missing 1 subtitle line",
      },
      { code: "broll", level: "pass", message: "assets are available" },
      { code: "encoder", level: "pass", message: "encoder is available" },
      { code: "size-estimate", level: "warning", message: "estimated size" },
    ],
    summary: {
      durationSeconds: 30,
      visibleCaptions: 1,
      hiddenCaptions: 0,
      brollItems: 0,
      titleItems: 0,
      encoder: "h264_videotoolbox",
      estimatedMinMb: 12,
      estimatedMaxMb: 28,
    },
  };

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "导出作品" }));
  fireEvent.click(screen.getByRole("button", { name: "开始检查" }));

  expect(await screen.findByText("字幕轨道")).toBeVisible();
  expect(screen.getByText(/所选字幕内容尚未准备好/)).toBeVisible();
  expect(screen.getByRole("button", { name: /导出字幕/ })).toBeDisabled();
  expect(screen.getByRole("button", { name: /导出带字幕视频/ })).toBeDisabled();
  expect(screen.getByRole("button", { name: /导出 Final Cut 工程/ })).toBeEnabled();

  fireEvent.click(screen.getByRole("button", { name: "前往翻译" }));
  expect(screen.getByRole("button", { name: "翻译" })).toHaveAttribute(
    "aria-current",
    "page",
  );
});

test("corrupt timeline data blocks every dependent delivery and opens the timeline", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Ready to export",
        words: [{ id: "w1", text: "Ready", start: 0, end: 1 }],
      }],
    }],
  };
  exportPreflightState = {
    ...exportPreflightState,
    ready: false,
    items: [
      {
        code: "timeline-data",
        level: "blocker",
        message: "timeline edits cannot be read",
      },
    ],
  };

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "导出作品" }));
  fireEvent.click(screen.getByRole("button", { name: "开始检查" }));

  expect(await screen.findByText("时间线数据")).toBeVisible();
  expect(screen.getByRole("button", { name: /导出字幕/ })).toBeDisabled();
  expect(screen.getByRole("button", { name: /导出带字幕视频/ })).toBeDisabled();
  expect(screen.getByRole("button", { name: /导出 Final Cut 工程/ })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "查看时间线" }));
  expect(screen.getByRole("button", { name: /当前工具：时间线/ })).toHaveClass("active");
});

test("video export reports hardware backend, real progress, and cancellation", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Ready to export",
        words: [{ id: "w1", text: "Ready", start: 0, end: 1 }],
      }],
    }],
  };
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "导出作品" }));
  fireEvent.click(screen.getByRole("button", { name: "开始检查" }));
  await waitFor(() => expect(screen.getByRole("button", { name: /导出带字幕视频/ })).toBeEnabled());
  fireEvent.click(screen.getByRole("button", { name: /导出带字幕视频/ }));

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("video_export_start", {
    pid: "project-1",
    mode: null,
    settings: {
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
    },
  }));
  expect(await screen.findByRole("progressbar", { name: "视频导出进度" })).toHaveValue(47);
  expect(screen.getByText(/VideoToolbox · Apple Media Engine/)).toBeVisible();
  expect(screen.getByText(/14s \/ 30s/)).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "取消导出" }));

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("video_export_cancel", {
    pid: "project-1",
  }));
});

test("professional video export keeps compatible container, codec, subtitle, and audio settings", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Ready to export",
        words: [{ id: "w1", text: "Ready", start: 0, end: 1 }],
      }],
    }],
    translations: {
      "zh-Hans": {
        s1: {
          id: "s1",
          text: "准备导出",
          sourceWords: ["w1"],
          sourceText: "Ready to export",
        },
      },
    },
  };
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "导出作品" }));
  fireEvent.click(screen.getByRole("button", { name: "开始检查" }));
  await waitFor(() => expect(screen.getByRole("button", { name: /导出带字幕视频/ })).toBeEnabled());

  fireEvent.change(screen.getByLabelText("视频编码"), { target: { value: "prores" } });
  expect(screen.getByLabelText("容器")).toHaveValue("mov");
  fireEvent.change(screen.getByLabelText("画布比例"), { target: { value: "9:16" } });
  fireEvent.change(screen.getByLabelText("分辨率"), { target: { value: "4k" } });
  fireEvent.change(screen.getByLabelText("源画面适配"), { target: { value: "cover" } });
  fireEvent.change(screen.getByLabelText("字幕"), { target: { value: "soft" } });
  fireEvent.change(screen.getByLabelText("字幕内容"), {
    target: { value: "bilingual:zh-Hans" },
  });
  fireEvent.change(screen.getByLabelText("音频"), { target: { value: "pcm" } });
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("export_preflight", {
    pid: "project-1",
    settings: {
      container: "mov",
      videoCodec: "prores",
      resolution: "4k",
      aspectRatio: "9:16",
      canvasFit: "cover",
      subtitleMode: "soft",
      subtitleLanguage: "zh-Hans",
      bilingualSubtitles: true,
      audioCodec: "pcm",
      encodingSpeed: "fast",
    },
    root: null,
  }));
  await waitFor(() =>
    expect(screen.getByRole("button", { name: /导出带字幕视频/ })).toBeEnabled()
  );
  fireEvent.click(screen.getByRole("button", { name: /导出带字幕视频/ }));

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("video_export_start", {
    pid: "project-1",
    mode: null,
    settings: {
      container: "mov",
      videoCodec: "prores",
      resolution: "4k",
      aspectRatio: "9:16",
      canvasFit: "cover",
      subtitleMode: "soft",
      subtitleLanguage: "zh-Hans",
      bilingualSubtitles: true,
      audioCodec: "pcm",
      encodingSpeed: "fast",
    },
  }));
});

test("a failed delivery check needs an explicit draft override", async () => {
  finishCheckItems = [{
    code: "timing",
    ordinal: 1,
    pass: false,
    blockers: ["1 subtitle overlaps the next subtitle"],
  }];
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Needs review",
        words: [{ id: "w1", text: "Needs", start: 0, end: 1 }],
      }],
    }],
  };
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "导出作品" }));
  fireEvent.click(screen.getByRole("button", { name: "开始检查" }));

  expect(await screen.findByText("存在阻止正式交付的问题")).toBeVisible();
  expect(screen.getByText("1 subtitle overlaps the next subtitle")).toBeVisible();
  expect(screen.getByRole("button", { name: /导出带字幕视频/ })).toBeDisabled();

  fireEvent.click(screen.getByRole("checkbox", { name: /仍要导出草稿/ }));
  expect(screen.getByRole("button", { name: /导出带字幕视频/ })).toBeEnabled();
});

test("subtitle presets are applied before saving the project style", async () => {
  subtitleRowsState = [{
    id: "s1",
    text: "Style this line",
    speaker: "Host",
    hidden: false,
    start: 0,
    end: 1,
  }];
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Style this line",
        words: [{ id: "w1", text: "Style", start: 0, end: 1 }],
      }],
    }],
  };
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "样式" }));
  fireEvent.click(screen.getByRole("button", { name: /创作者黄字/ }));
  const previewSubtitle = document.querySelector<HTMLElement>(".program-subtitle span");
  const previewPosition = document.querySelector<HTMLElement>(".program-subtitle");
  expect(previewSubtitle).toHaveStyle({
    fontWeight: "700",
  });
  expect(previewSubtitle?.style.fontSize).toContain("cqw");
  expect(previewSubtitle?.style.fontSize).not.toContain("vw");
  expect(previewPosition?.style.bottom).toContain("%");
  expect(document.querySelector(".program-stage")).toBeInTheDocument();
  expect(screen.getByText(/有未保存修改/)).toBeVisible();

  fireEvent.click(screen.getByRole("button", { name: "转写稿" }));
  fireEvent.click(screen.getByRole("button", { name: "样式" }));
  expect(screen.getByRole("button", { name: /创作者黄字/ })).toBeVisible();
  expect(screen.getByText(/有未保存修改/)).toBeVisible();
  expect(document.querySelector<HTMLElement>(".program-subtitle span")).toHaveStyle({
    fontWeight: "700",
  });

  fireEvent.click(screen.getByRole("button", { name: "设置" }));
  fireEvent.click(screen.getByRole("button", { name: "编辑" }));
  expect(screen.getByText(/有未保存修改/)).toBeVisible();
  expect(document.querySelector<HTMLElement>(".program-subtitle span")).toHaveStyle({
    fontWeight: "700",
  });

  fireEvent.change(screen.getByRole("spinbutton", { name: "左侧安全边距" }), {
    target: { value: "72" },
  });
  fireEvent.click(screen.getByRole("button", { name: "删除线" }));
  fireEvent.click(screen.getByRole("button", { name: "保存样式" }));

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("style_set", {
    pid: "project-1",
    root: null,
    style: expect.objectContaining({
      name: "Creator yellow",
      bold: true,
      fontsize: 58,
      marginL: 72,
      primaryColour: "&H0000E8FF",
      strikeOut: true,
    }),
  }));
});

test("unsaved subtitle style previews survive an application restart", async () => {
  subtitleRowsState = [{
    id: "s1",
    text: "Style this line",
    speaker: "Host",
    hidden: false,
    start: 0,
    end: 1,
  }];
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Style this line",
        words: [{ id: "w1", text: "Style", start: 0, end: 1 }],
      }],
    }],
  };

  const first = render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "样式" }));
  fireEvent.click(screen.getByRole("button", { name: /创作者黄字/ }));

  await waitFor(() => expect(
    JSON.parse(localStorage.getItem("lumen-cut.styleDrafts.project-1") || "{}"),
  ).toMatchObject({
    name: "Creator yellow",
    bold: true,
    fontsize: 58,
  }));

  first.unmount();
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "样式" }));

  expect(await screen.findByText(/有未保存修改/)).toBeVisible();
  expect(document.querySelector<HTMLElement>(".program-subtitle span")).toHaveStyle({
    fontWeight: "700",
  });
});

test("named subtitle styles can be reused across projects without changing existing projects", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Reusable style",
        words: [{ id: "w1", text: "Reusable", start: 0, end: 1 }],
      }],
    }],
  };

  const first = render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "样式" }));
  fireEvent.change(screen.getByLabelText("字号"), { target: { value: "66" } });
  fireEvent.change(screen.getByLabelText("新样式名称"), {
    target: { value: "采访强调" },
  });
  fireEvent.click(screen.getByRole("button", { name: "保存到样式库" }));

  expect(await screen.findByText(/“采访强调”已保存到这台 Mac/)).toBeVisible();
  expect(JSON.parse(localStorage.getItem("lumen-cut.savedSubtitleStyles.v1") || "[]"))
    .toEqual([expect.objectContaining({
      name: "采访强调",
      style: expect.objectContaining({ fontsize: 66, name: "采访强调" }),
    })]);
  expect(invoke).not.toHaveBeenCalledWith("style_set", expect.anything());

  first.unmount();
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "样式" }));
  fireEvent.click(screen.getByRole("button", { name: "预览样式 采访强调" }));

  expect(screen.getByLabelText("字号")).toHaveValue(66);
  expect(screen.getByText(/保存后才会应用到当前项目/)).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "删除样式 采访强调" }));
  expect(await screen.findByText(/已从样式库删除“采访强调”/)).toBeVisible();
  expect(localStorage.getItem("lumen-cut.savedSubtitleStyles.v1")).toBeNull();
});

test("timeline edit decisions can be restored in place", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Keep this pause",
        words: [
          { id: "w1", text: "Keep", start: 0, end: 0.5 },
          { id: "w2", text: "pause", start: 2, end: 2.5 },
        ],
      }],
    }],
  };
  cutListState = [{
    id: "cut-1",
    kind: "silence",
    a_word: "w1",
    b_word: "w2",
    duration: 1.5,
    note: "Long pause",
  }];
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  await openEditorTool("时间线");

  expect(await screen.findByText("1 个区间将在成片中移除")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "恢复此区间" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("cut_restore", {
    pid: "project-1",
    cutId: "cut-1",
    root: null,
  }));
});

test("timeline keeps the video visible and follows the active subtitle cue", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [
        {
          id: "s1",
          text: "Opening line",
          words: [{ id: "w1", text: "Opening", start: 0, end: 2 }],
        },
        {
          id: "s2",
          text: "Current line",
          words: [{ id: "w2", text: "Current", start: 3, end: 5 }],
        },
        {
          id: "s3",
          text: "Closing line",
          words: [{ id: "w3", text: "Closing", start: 6, end: 9 }],
        },
      ],
    }],
  };
  const scrollTo = vi.fn();
  Object.defineProperty(HTMLElement.prototype, "scrollTo", {
    configurable: true,
    value: scrollTo,
  });

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  await openEditorTool("时间线");

  await screen.findByText("节目监看");
  const video = document.querySelector<HTMLVideoElement>(".workbench-preview video");
  if (!video) throw new Error("timeline video preview was not rendered");
  expect(document.querySelectorAll("video")).toHaveLength(1);
  expect(video.closest(".workbench-preview")).not.toBeNull();
  expect(screen.getByText("字幕轨道").closest(".timeline-edit-column")).not.toBeNull();
  expect(screen.getByRole("button", { name: "跟随播放" })).toHaveAttribute("aria-pressed", "true");

  scrollTo.mockClear();
  Object.defineProperty(video, "currentTime", { configurable: true, value: 4 });
  fireEvent.timeUpdate(video as HTMLVideoElement);
  await waitFor(() => expect(scrollTo).toHaveBeenCalledWith(expect.objectContaining({
    behavior: "smooth",
  })));
  expect(screen.getByText("Current line").closest("article")).toHaveClass("active");
  expect(screen.getByText("2 / 3")).toBeVisible();

  fireEvent.click(screen.getByRole("button", { name: "跟随播放" }));
  expect(screen.getByRole("button", { name: "跟随播放" })).toHaveAttribute("aria-pressed", "false");
  scrollTo.mockClear();
  Object.defineProperty(video, "currentTime", { configurable: true, value: 7 });
  fireEvent.timeUpdate(video as HTMLVideoElement);
  await waitFor(() => expect(screen.getByText("Closing line").closest("article")).toHaveClass("active"));
  expect(scrollTo).not.toHaveBeenCalled();
});

test("timeline range selection removes several cues as one atomic edit", async () => {
  projectDoc = {
    ...projectDoc,
    media: {
      ...(projectDoc.media as Record<string, unknown>),
      durationSeconds: 10,
    },
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [
        {
          id: "s1",
          text: "First",
          words: [{ id: "w1", text: "First", start: 1, end: 2 }],
        },
        {
          id: "s2",
          text: "Second",
          words: [{ id: "w2", text: "Second", start: 3, end: 4 }],
        },
        {
          id: "s3",
          text: "Third",
          words: [{ id: "w3", text: "Third", start: 5, end: 6 }],
        },
      ],
    }],
  };
  subtitleRowsState = [
    { id: "s1", text: "First", speaker: "Host", hidden: false, start: 1, end: 2 },
    { id: "s2", text: "Second", speaker: "Host", hidden: false, start: 3, end: 4 },
    { id: "s3", text: "Third", speaker: "Host", hidden: false, start: 5, end: 6 },
  ];

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  await openEditorTool("时间线");

  const track = await screen.findByRole("listbox", { name: /字幕轨道/ });
  Object.defineProperty(track, "getBoundingClientRect", {
    configurable: true,
    value: () => ({
      bottom: 30,
      height: 30,
      left: 0,
      right: 100,
      top: 0,
      width: 100,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    }),
  });
  fireEvent.click(track, { clientX: 15 });
  fireEvent.click(track, { clientX: 35, shiftKey: true });

  const remove = screen.getByRole("button", { name: "移除 2 段" });
  expect(remove.getAttribute("title")).toContain("作为一次编辑移除");
  fireEvent.click(remove);

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("cut_manual_many", {
    pid: "project-1",
    cueIds: ["s1", "s2"],
    root: null,
  }));
  expect(await screen.findByText(/已将 2 段字幕对应的画面和声音作为一次编辑移除/)).toBeVisible();
});

test("program monitor skips removed regions and reports edited duration", async () => {
  projectDoc = {
    ...projectDoc,
    media: {
      ...(projectDoc.media as Record<string, unknown>),
      durationSeconds: 10,
    },
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Remove me",
        words: [{ id: "w1", text: "Remove", start: 1, end: 2 }],
      }],
    }],
  };
  subtitleRowsState = [
    { id: "s1", text: "Remove me", speaker: "Host", hidden: false, start: 1, end: 2 },
  ];
  cutListState = [{
    id: "cut-1",
    kind: "manual",
    a_word: "w1",
    b_word: "w1",
    duration: 1,
    note: "Remove me",
  }];

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  await openEditorTool("时间线");

  const previewCuts = await screen.findByRole("button", { name: "跳过切口" });
  expect(previewCuts).toHaveAttribute("aria-pressed", "true");
  expect(screen.getByText("成片 00:09")).toBeVisible();

  const video = document.querySelector<HTMLVideoElement>(".workbench-preview video");
  if (!video) throw new Error("program monitor video was not rendered");
  Object.defineProperty(video, "currentTime", {
    configurable: true,
    value: 1.5,
    writable: true,
  });
  fireEvent.play(video);
  await screen.findByRole("button", { name: "暂停" });
  fireEvent.timeUpdate(video);
  expect(video.currentTime).toBeCloseTo(2.001);

  fireEvent.click(previewCuts);
  expect(previewCuts).toHaveAttribute("aria-pressed", "false");
  video.currentTime = 1.5;
  fireEvent.timeUpdate(video);
  expect(video.currentTime).toBe(1.5);
});

test("title editor persists fade keyframes for preview and export", async () => {
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "+ 标题" }));

  const titleText = screen.getByPlaceholderText("输入标题");
  fireEvent.change(titleText, {
    target: { value: "Opening title" },
  });
  fireEvent.change(screen.getByLabelText("淡入（秒）"), {
    target: { value: "0.5" },
  });
  fireEvent.change(screen.getByLabelText("淡出（秒）"), {
    target: { value: "0.75" },
  });
  const form = titleText.closest("form");
  if (!form) throw new Error("title editor form was not rendered");
  expect(form.checkValidity()).toBe(true);
  fireEvent.submit(form);

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("title_add", {
    pid: "project-1",
    input: expect.objectContaining({
      text: "Opening title",
      fadeIn: 0.5,
      fadeOut: 0.75,
    }),
    root: null,
  }));
});

test("timeline audio mix is persisted and applied to the program monitor", async () => {
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "音频 100%" }));

  fireEvent.click(screen.getByRole("checkbox", { name: "静音原始音频" }));
  fireEvent.change(screen.getByRole("slider", { name: /音量/ }), {
    target: { value: "75" },
  });
  fireEvent.change(screen.getByLabelText("淡出（秒）"), {
    target: { value: "1" },
  });
  fireEvent.click(screen.getByRole("checkbox", { name: /增强对白/ }));
  fireEvent.click(screen.getByRole("checkbox", { name: /标准化响度/ }));
  fireEvent.change(screen.getByLabelText("目标响度"), {
    target: { value: "-14" },
  });
  expect(screen.getByText(/节目监看仅预览音量、静音和淡入淡出/)).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "保存音频设置" }));

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("audio_mix_set", {
    pid: "project-1",
    mix: {
      volume: 0.75,
      muted: true,
      fadeIn: 0,
      fadeOut: 1,
      voiceEnhance: true,
      normalizeLoudness: true,
      loudnessTarget: -14,
      music: [],
    },
    root: null,
  }));
  expect(await screen.findByText(/音频设置已保存/)).toBeVisible();
  const video = document.querySelector<HTMLVideoElement>(".workbench-preview video");
  if (!video) throw new Error("program monitor video was not rendered");
  await waitFor(() => expect(video.muted).toBe(true));
});

test("unsaved timeline title drafts survive an application restart", async () => {
  const first = render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "+ 标题" }));
  fireEvent.change(screen.getByPlaceholderText("输入标题"), {
    target: { value: "尚未保存的片头" },
  });

  await waitFor(() => expect(
    JSON.parse(localStorage.getItem("lumen-cut.timelineDrafts.project-1") || "{}"),
  ).toMatchObject({
    selectedTitleId: null,
    titleDraft: { text: "尚未保存的片头" },
    titlePanelOpen: true,
  }));

  first.unmount();
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  expect(await screen.findByPlaceholderText("输入标题")).toHaveValue("尚未保存的片头");
  expect(screen.getByText("标题修改尚未保存，重启后仍会恢复。")).toBeVisible();
});

test("unsaved timeline audio changes survive an application restart", async () => {
  const first = render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "音频 100%" }));
  fireEvent.change(screen.getByRole("slider", { name: /音量/ }), {
    target: { value: "75" },
  });

  await waitFor(() => expect(
    JSON.parse(localStorage.getItem("lumen-cut.timelineDrafts.project-1") || "{}"),
  ).toMatchObject({
    audioDraft: { volume: 0.75 },
    audioPanelOpen: true,
  }));

  first.unmount();
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  expect(await screen.findByRole("slider", { name: /音量/ })).toHaveValue("75");
  expect(screen.getByText("音频修改尚未保存，重启后仍会恢复。")).toBeVisible();
});

test("background music can be added, timed, ducked, and saved with the project mix", async () => {
  const pause = vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => undefined);
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "音频 100%" }));
  fireEvent.click(screen.getByRole("button", { name: /添加音乐/ }));

  expect((await screen.findAllByText("background.mp3"))[0]).toBeVisible();
  expect(screen.getByRole("checkbox", { name: /对白时自动压低音乐/ })).toBeChecked();
  fireEvent.change(screen.getByRole("slider", { name: /音乐音量/ }), {
    target: { value: "32" },
  });
  fireEvent.change(screen.getByRole("spinbutton", { name: "素材起点" }), {
    target: { value: "3.5" },
  });
  const save = screen.getByRole("button", { name: "保存音频设置" });
  await waitFor(() => expect(save).toBeEnabled());
  fireEvent.click(save);

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("audio_mix_set", {
    pid: "project-1",
    mix: expect.objectContaining({
      music: expect.arrayContaining([expect.objectContaining({
        path: "/tmp/background.mp3",
        sourceStart: 3.5,
        volume: 0.32,
        ducking: true,
      })]),
    }),
    root: null,
  }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("audio_asset_allow", {
    pid: "project-1",
    musicId: expect.stringMatching(/^music-/),
    root: null,
  }));
  pause.mockRestore();
});

test("multiple background music clips can be selected and edited independently", async () => {
  const pause = vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => undefined);
  const first = {
    id: "music-first",
    path: "/tmp/first.mp3",
    start: 0,
    end: 8,
    sourceStart: 0,
    volume: 0.2,
    fadeIn: 0.5,
    fadeOut: 0.5,
    ducking: true,
  };
  const second = {
    id: "music-second",
    path: "/tmp/second.mp3",
    start: 8,
    end: 18,
    sourceStart: 2,
    volume: 0.35,
    fadeIn: 1,
    fadeOut: 1,
    ducking: false,
  };
  audioMixState = { ...audioMixState, music: [first, second] };

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "音频 100%" }));

  const list = await screen.findByRole("listbox", { name: "音乐片段列表" });
  expect(within(list).getAllByRole("option")).toHaveLength(2);
  fireEvent.click(within(list).getByRole("option", { name: /second\.mp3/ }));
  expect(within(list).getByRole("option", { name: /second\.mp3/ }))
    .toHaveAttribute("aria-selected", "true");
  fireEvent.change(screen.getByRole("slider", { name: /音乐音量/ }), {
    target: { value: "61" },
  });
  fireEvent.change(screen.getByRole("spinbutton", { name: "成片开始" }), {
    target: { value: "9.5" },
  });
  fireEvent.click(screen.getByRole("button", { name: "保存音频设置" }));

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("audio_mix_set", {
    pid: "project-1",
    mix: expect.objectContaining({
      music: [
        first,
        expect.objectContaining({
          id: "music-second",
          start: 9.5,
          volume: 0.61,
        }),
      ],
    }),
    root: null,
  }));
  await waitFor(() => {
    expect(invoke).toHaveBeenCalledWith("audio_asset_allow", {
      pid: "project-1",
      musicId: "music-first",
      root: null,
    });
    expect(invoke).toHaveBeenCalledWith("audio_asset_allow", {
      pid: "project-1",
      musicId: "music-second",
      root: null,
    });
  });
  pause.mockRestore();
});

test("legacy single-object music drafts migrate to the multi-clip project format", async () => {
  const pause = vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => undefined);
  const legacyTrack = {
    path: "/tmp/legacy.mp3",
    start: 1,
    end: 5,
    sourceStart: 0.5,
    volume: 0.3,
    fadeIn: 0.25,
    fadeOut: 0.5,
    ducking: true,
  };
  const savedMix = {
    volume: 1,
    muted: false,
    fadeIn: 0,
    fadeOut: 0,
    voiceEnhance: false,
    normalizeLoudness: false,
    loudnessTarget: -16,
    music: [{ id: "music-1", ...legacyTrack }],
  };
  audioMixState = savedMix;
  localStorage.setItem("lumen-cut.timelineDrafts.project-1", JSON.stringify({
    audioDraft: { ...savedMix, volume: 0.75, music: legacyTrack },
    audioSource: { ...savedMix, music: legacyTrack },
    audioPanelOpen: true,
    selectedTitleId: null,
    titleDraft: null,
    titlePanelOpen: false,
    titleSource: null,
  }));

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  expect(await screen.findByText(/背景音乐 · 1 段/)).toBeVisible();
  expect(screen.getByText("音频修改尚未保存，重启后仍会恢复。")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "保存音频设置" }));

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("audio_mix_set", {
    pid: "project-1",
    mix: expect.objectContaining({
      volume: 0.75,
      music: [{ id: "music-1", ...legacyTrack }],
    }),
    root: null,
  }));
  pause.mockRestore();
});

test("background music moves on the edited program timeline and persists as one edit", async () => {
  const pause = vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => undefined);
  projectDoc = {
    ...projectDoc,
    media: {
      ...(projectDoc.media as Record<string, unknown>),
      durationSeconds: 10,
    },
    paragraphs: [{
      id: 1,
      speaker: "Host",
      sentences: [{
        id: "s1",
        text: "Remove this",
        words: [{ id: "w1", text: "Remove", start: 1, end: 2 }],
      }],
    }],
  };
  subtitleRowsState = [
    { id: "s1", text: "Remove this", speaker: "Host", hidden: false, start: 1, end: 2 },
  ];
  cutListState = [{
    id: "cut-1",
    kind: "manual",
    a_word: "w1",
    b_word: "w1",
    duration: 1,
    note: "Remove this",
  }];
  audioMixState = {
    ...audioMixState,
    music: [{
      id: "music-main",
      path: "/tmp/background.mp3",
      start: 2,
      end: 6,
      sourceStart: 0,
      volume: 0.25,
      fadeIn: 0.5,
      fadeOut: 0.5,
      ducking: true,
    }],
  };

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  const clip = await screen.findByRole("button", { name: /背景音乐.*background\.mp3/ });
  const canvas = clip.closest(".timeline-canvas");
  if (!canvas) throw new Error("timeline canvas was not rendered");
  Object.defineProperty(canvas, "getBoundingClientRect", {
    configurable: true,
    value: () => ({
      bottom: 160,
      height: 160,
      left: 0,
      right: 1000,
      top: 0,
      width: 1000,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    }),
  });
  let captured = false;
  Object.defineProperties(clip, {
    setPointerCapture: {
      configurable: true,
      value: () => {
        captured = true;
      },
    },
    hasPointerCapture: {
      configurable: true,
      value: () => captured,
    },
    releasePointerCapture: {
      configurable: true,
      value: () => {
        captured = false;
      },
    },
  });

  expect(clip).toHaveStyle({ left: "30%", width: "40%" });
  fireEvent.pointerDown(clip, { clientX: 300, pointerId: 1 });
  expect(await screen.findByText(/背景音乐 · 1 段/)).toBeVisible();
  fireEvent.pointerMove(clip, { clientX: 500, pointerId: 1 });
  fireEvent.pointerUp(clip, { clientX: 500, pointerId: 1 });

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("audio_mix_set", {
    pid: "project-1",
    mix: expect.objectContaining({
      music: expect.arrayContaining([expect.objectContaining({
        start: 4,
        end: 8,
        sourceStart: 0,
      })]),
    }),
    root: null,
  }));
  expect(screen.getByRole("button", { name: /背景音乐.*background\.mp3/ }))
    .toHaveStyle({ left: "50%", width: "40%" });
  pause.mockRestore();
});

test("timeline music drag preserves an existing unsaved audio draft until explicit save", async () => {
  const pause = vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => undefined);
  projectDoc = {
    ...projectDoc,
    media: {
      ...(projectDoc.media as Record<string, unknown>),
      durationSeconds: 10,
    },
  };
  const savedMusic = {
    id: "music-main",
    path: "/tmp/background.mp3",
    start: 2,
    end: 6,
    sourceStart: 0,
    volume: 0.25,
    fadeIn: 0.5,
    fadeOut: 0.5,
    ducking: true,
  };
  audioMixState = { ...audioMixState, music: [savedMusic] };
  localStorage.setItem("lumen-cut.timelineDrafts.project-1", JSON.stringify({
    audioDraft: { ...audioMixState, volume: 0.75 },
    audioSource: audioMixState,
    audioPanelOpen: false,
    selectedTitleId: null,
    titleDraft: null,
    titlePanelOpen: false,
    titleSource: null,
  }));

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  const clip = await screen.findByRole("button", { name: /背景音乐.*background\.mp3/ });
  const canvas = clip.closest(".timeline-canvas");
  if (!canvas) throw new Error("timeline canvas was not rendered");
  Object.defineProperty(canvas, "getBoundingClientRect", {
    configurable: true,
    value: () => ({
      bottom: 160,
      height: 160,
      left: 0,
      right: 1000,
      top: 0,
      width: 1000,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    }),
  });
  let captured = false;
  Object.defineProperties(clip, {
    setPointerCapture: {
      configurable: true,
      value: () => {
        captured = true;
      },
    },
    hasPointerCapture: {
      configurable: true,
      value: () => captured,
    },
    releasePointerCapture: {
      configurable: true,
      value: () => {
        captured = false;
      },
    },
  });

  fireEvent.pointerDown(clip, { clientX: 200, pointerId: 1 });
  fireEvent.pointerMove(clip, { clientX: 400, pointerId: 1 });
  fireEvent.pointerUp(clip, { clientX: 400, pointerId: 1 });

  expect(invoke).not.toHaveBeenCalledWith("audio_mix_set", expect.anything());
  expect(screen.getByText("音频修改尚未保存，重启后仍会恢复。")).toBeVisible();
  expect(screen.getByRole("slider", { name: /^音量/ })).toHaveValue("75");
  fireEvent.click(screen.getByRole("button", { name: "保存音频设置" }));

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("audio_mix_set", {
    pid: "project-1",
    mix: expect.objectContaining({
      volume: 0.75,
      music: expect.arrayContaining([expect.objectContaining({
        start: 4,
        end: 8,
      })]),
    }),
    root: null,
  }));
  pause.mockRestore();
});

test("timeline audio drafts do not silently overwrite an externally changed saved mix", async () => {
  audioMixState = {
    volume: 0.5,
    muted: false,
    fadeIn: 0,
    fadeOut: 0,
    voiceEnhance: false,
    normalizeLoudness: false,
    loudnessTarget: -16,
    music: [],
  };
  localStorage.setItem("lumen-cut.timelineDrafts.project-1", JSON.stringify({
    audioDraft: {
      volume: 0.75,
      muted: false,
      fadeIn: 0,
      fadeOut: 0,
      voiceEnhance: false,
      normalizeLoudness: false,
      loudnessTarget: -16,
      music: [],
    },
    audioSource: {
      volume: 1,
      muted: false,
      fadeIn: 0,
      fadeOut: 0,
      voiceEnhance: false,
      normalizeLoudness: false,
      loudnessTarget: -16,
      music: [],
    },
    audioPanelOpen: true,
    selectedTitleId: null,
    titleDraft: null,
    titlePanelOpen: false,
    titleSource: null,
  }));

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  expect(await screen.findByText(/保存的音频设置已在其他操作中改变/)).toBeVisible();
  expect(screen.getByRole("slider", { name: /音量/ })).toHaveValue("75");
  fireEvent.click(screen.getByRole("button", { name: "保留我的草稿" }));
  expect(screen.queryByText(/保存的音频设置已在其他操作中改变/)).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "保存音频设置" }));

  await waitFor(() => expect(invoke).toHaveBeenCalledWith("audio_mix_set", {
    pid: "project-1",
    mix: {
      volume: 0.75,
      muted: false,
      fadeIn: 0,
      fadeOut: 0,
      voiceEnhance: false,
      normalizeLoudness: false,
      loudnessTarget: -16,
      music: [],
    },
    root: null,
  }));
});

test("timeline title drafts surface conflicts with an externally changed saved title", async () => {
  const original = {
    text: "Original title",
    start: 0,
    end: 3,
    x: 0.5,
    y: 0.18,
    fontSize: 64,
    color: "#FFFFFF",
    background: "#00000099",
    fadeIn: 0,
    fadeOut: 0,
  };
  titleListState = [{
    id: "title-1",
    ...original,
    text: "Remote title",
  }];
  localStorage.setItem("lumen-cut.timelineDrafts.project-1", JSON.stringify({
    audioDraft: { volume: 1, muted: false, fadeIn: 0, fadeOut: 0 },
    audioSource: { volume: 1, muted: false, fadeIn: 0, fadeOut: 0 },
    audioPanelOpen: false,
    selectedTitleId: "title-1",
    titleDraft: { ...original, text: "My local title" },
    titlePanelOpen: true,
    titleSource: original,
  }));

  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));

  expect(await screen.findByText(/这个标题的已保存版本发生了变化/)).toBeVisible();
  expect(screen.getByPlaceholderText("输入标题")).toHaveValue("My local title");
  fireEvent.click(screen.getByRole("button", { name: "使用已保存版本" }));

  expect(screen.getByPlaceholderText("输入标题")).toHaveValue("Remote title");
  expect(screen.queryByText(/这个标题的已保存版本发生了变化/)).not.toBeInTheDocument();
  await waitFor(() => expect(
    localStorage.getItem("lumen-cut.timelineDrafts.project-1"),
  ).toBeNull());
});

test("project library can search transcript content, star, and repair a project", async () => {
  render(<App />);

  const search = await screen.findByPlaceholderText("搜索项目、备注或转写内容");
  fireEvent.change(search, { target: { value: "customer phrase" } });

  expect(await screen.findByText("Search match")).toBeVisible();
  fireEvent.change(screen.getByRole("combobox", { name: "项目排序" }), {
    target: { value: "name" },
  });
  expect(localStorage.getItem("lumen-cut.projectSort")).toBe("name");
  fireEvent.click(screen.getByRole("button", { name: "收藏项目: Search match" }));
  expect(await screen.findByRole("button", { name: "取消收藏: Search match" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );

  fireEvent.click(screen.getByRole("button", { name: "更多项目操作: Search match" }));
  fireEvent.click(screen.getByRole("button", { name: "修复转写时间轴" }));
  expect(screen.getByText(/修改前自动保存恢复版本/)).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "确认修复" }));
  expect(await screen.findByText(/项目检查完成 · 3 fix\(es\)/)).toBeVisible();

  expect(invoke).toHaveBeenCalledWith("project_search", {
    query: "customer phrase",
    root: null,
  });
  expect(invoke).toHaveBeenCalledWith("project_set_star", {
    pid: "project-2",
    starred: true,
    root: null,
  });
  expect(invoke).toHaveBeenCalledWith("timing_repair", {
    pid: "project-2",
    root: null,
  });
});

test("dropping desktop media uses the same non-blocking import path", async () => {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: { metadata: { currentWindow: { label: "main" } } },
  });
  render(<App />);
  await waitFor(() => expect(nativeDrag.handler).not.toBeNull());

  act(() => nativeDrag.handler?.({
    payload: { type: "enter", paths: ["/Users/example/drop.mov"] },
  }));
  expect(screen.getByText("松开即可导入媒体")).toBeVisible();

  act(() => nativeDrag.handler?.({
    payload: { type: "drop", paths: ["/Users/example/drop.mov"] },
  }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("project_create", {
    args: expect.objectContaining({
      from: "/Users/example/drop.mov",
      title: "drop",
    }),
  }));
});

test("project editor exposes recoverable version snapshots", async () => {
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "历史" }));

  fireEvent.change(screen.getByPlaceholderText("例如：校对完成"), {
    target: { value: "Before batch polish" },
  });
  fireEvent.change(screen.getByPlaceholderText("记录这次修改的目的"), {
    target: { value: "Safe restore point" },
  });
  fireEvent.click(screen.getByRole("button", { name: "保存当前版本" }));

  expect(await screen.findByText("当前项目已保存为可恢复版本。")).toBeVisible();
  expect(invoke).toHaveBeenCalledWith("version_commit", {
    pid: "project-1",
    name: "Before batch polish",
    note: "Safe restore point",
    root: null,
  });
});

test("a failed version save preserves the draft for retry", async () => {
  versionCommitError = new Error("disk is temporarily unavailable");
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  fireEvent.click(await screen.findByRole("button", { name: "历史" }));

  const name = screen.getByPlaceholderText("例如：校对完成");
  const note = screen.getByPlaceholderText("记录这次修改的目的");
  fireEvent.change(name, { target: { value: "Retry me" } });
  fireEvent.change(note, { target: { value: "Keep this note" } });
  fireEvent.click(screen.getByRole("button", { name: "保存当前版本" }));

  expect(await screen.findByText("disk is temporarily unavailable")).toBeVisible();
  expect(name).toHaveValue("Retry me");
  expect(note).toHaveValue("Keep this note");
});

test("B-roll suggestions have a discoverable asset acceptance flow", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Alice",
      sentences: [{
        id: "s1",
        text: "Show the product",
        words: [
          { id: "w1", text: "Show", start: 4, end: 5 },
          { id: "w2", text: "product", start: 6, end: 7 },
        ],
      }],
    }],
  };
  brollOverview = {
    suggestions: [{
      start: "w1",
      end: "w2",
      mode: "pip",
      query: "product close-up",
      reason: "Show the object being discussed",
    }],
    accepted: [],
    errors: [],
  };
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  await openEditorTool("补充画面");

  expect(await screen.findByText("product close-up")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "选择素材并添加" }));

  await waitFor(() => {
    expect(invoke).toHaveBeenCalledWith("broll_accept_suggestion", {
      pid: "project-1",
      suggestion: {
        start: "w1",
        end: "w2",
        mode: "pip",
        query: "product close-up",
        reason: "Show the object being discussed",
      },
      file: "/Users/example/product.png",
      root: null,
    });
  });
  expect(await screen.findByText("素材已按建议时段加入 B-roll 轨道。")).toBeVisible();
  expect(screen.getByRole("heading", { name: "已加入成片" }).closest("header")).toHaveTextContent("1");
});

test("B-roll quick preview reports frame progress and cancellation", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Alice",
      sentences: [{
        id: "s1",
        text: "Show the product",
        words: [
          { id: "w1", text: "Show", start: 4, end: 5 },
          { id: "w2", text: "product", start: 6, end: 7 },
        ],
      }],
    }],
  };
  brollOverview = {
    suggestions: [],
    accepted: [{
      id: "br-1",
      file: "/Users/example/product.png",
      start: 4,
      end: 7,
      mode: "pip",
      rect: null,
      fit: "cover",
      background: "black",
      sourceStart: 0,
      radius: 0,
      name: "product",
    }],
    errors: [],
  };
  brollPreviewStatusState = {
    ...brollPreviewStatusState,
    phase: "frames",
    progress: 67,
    current: 2,
    total: 3,
    encoder: null,
  };
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  await openEditorTool("补充画面");
  fireEvent.click(await screen.findByRole("button", { name: "生成快速预览" }));

  expect(await screen.findByRole("progressbar", { name: "B-roll 预览进度" })).toHaveValue(67);
  expect(screen.getByText("正在直接合成代表帧")).toBeVisible();
  expect(screen.getByText(/2 \/ 3/)).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "取消预览" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("broll_preview_cancel", {
    pid: "project-1",
  }));
});

test("B-roll adjustment drafts survive tab switches and preview before saving", async () => {
  projectDoc = {
    ...projectDoc,
    paragraphs: [{
      id: 1,
      speaker: "Alice",
      sentences: [{
        id: "s1",
        text: "Show the product",
        words: [{ id: "w1", text: "Show", start: 0, end: 1 }],
      }],
    }],
  };
  brollOverview = {
    suggestions: [],
    accepted: [{
      id: "br-1",
      file: "/Users/example/product.png",
      start: 4,
      end: 7,
      mode: "pip",
      rect: null,
      fit: "cover",
      background: "black",
      sourceStart: 0,
      radius: 0,
      name: "product",
    }],
    errors: [],
  };

  const first = render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  await openEditorTool("补充画面");

  const start = screen.getAllByRole("spinbutton", { name: "开始（秒）" })[0];
  fireEvent.change(start, { target: { value: "0" } });
  expect(screen.getByText(/修改未保存/)).toBeVisible();
  expect(screen.getByRole("button", { name: "保存调整" })).toBeEnabled();
  await waitFor(() => expect(document.querySelector(".program-broll")).toBeInTheDocument());

  fireEvent.click(screen.getByRole("button", { name: "转写稿" }));
  await openEditorTool("补充画面");
  expect(screen.getAllByRole("spinbutton", { name: "开始（秒）" })[0]).toHaveValue(0);
  expect(screen.getByText(/修改未保存/)).toBeVisible();

  await waitFor(() => expect(
    JSON.parse(localStorage.getItem("lumen-cut.brollDrafts.project-1") || "{}"),
  ).toMatchObject({
    placements: {
      "br-1": { start: 0, end: 7, file: "/Users/example/product.png" },
    },
  }));

  first.unmount();
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: /Interview.*打开项目/ }));
  await openEditorTool("补充画面");
  expect(screen.getAllByRole("spinbutton", { name: "开始（秒）" })[0]).toHaveValue(0);
  expect(screen.getByText(/修改未保存/)).toBeVisible();
});
