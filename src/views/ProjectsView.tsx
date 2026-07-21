import { useEffect, useRef, useState } from "react";
import {
  pickMediaFile,
  projectCreate,
  projectDelete,
  projectList,
  projectReveal,
  recordingCancel,
  recordingStart,
  recordingStop,
  transcriptionCancel,
  transcriptionStart,
  transcriptionStatus,
} from "../api";
import {
  ChevronRightIcon,
  FolderIcon,
  LinkIcon,
  MicrophoneIcon,
  UploadIcon,
} from "../components/Icons";
import type { Lang } from "../i18n";
import type { ProjectSummary, TranscriptionJobStatus } from "../types";

interface Props {
  currentPid: string | null;
  lang: Lang;
  onDeleteProject: (pid: string) => void;
  onOpenProject: (pid: string, title?: string) => void;
}

type Composer = "url" | "record" | null;
type BusyKind = "file" | "url" | "record" | null;
interface ActiveRecording {
  pid: string;
  title: string;
  startedAt: number;
}
interface UrlIngestJob {
  pid: string;
  title: string;
  status: TranscriptionJobStatus;
}

const URL_JOB_KEY = "lumen-cut.activeUrlIngest";

const COPY = {
  zh: {
    eyebrow: "开始创作",
    title: "把声音变成可编辑的文字",
    description: "选择视频或音频，lumen-cut 会建立项目。下一步会清楚告诉你何时开始转写。",
    chooseFile: "选择文件",
    chooseHint: "视频或音频",
    pasteUrl: "粘贴链接",
    urlHint: "YouTube 或媒体 URL",
    record: "录制音频",
    recordHint: "使用 Mac 麦克风",
    urlLabel: "媒体链接",
    urlPlaceholder: "https://…",
    importUrl: "下载并转写",
    cancelImport: "取消下载与转写",
    cancellingImport: "正在停止…",
    cancelledImport: "下载与转写已取消。",
    beginRecord: "开始录音",
    stopRecord: "停止并使用录音",
    discardRecord: "取消录音",
    recording: "正在录音",
    recordingHint: "现在可以开始说话，完成后点击“停止并使用录音”。",
    finishingRecord: "正在保存录音并创建项目…",
    importing: "正在读取媒体并创建项目…",
    downloading: "正在下载并转写，较长的视频需要一些时间…",
    recent: "项目",
    empty: "还没有项目。选择一个文件就能开始。",
    open: "打开项目",
    words: "字词",
    paragraphs: "段落",
    ready: "等待转写",
    moreActions: "更多项目操作",
    reveal: "在 Finder 中显示",
    delete: "删除项目",
    deleteConfirm: "删除项目文件？原始媒体不会被删除。",
    cancel: "取消",
    deleted: "项目已删除，原始媒体仍保留在原位置。",
    errorTitle: "这一步没有完成",
  },
  en: {
    eyebrow: "Get started",
    title: "Turn speech into editable text",
    description: "Choose a video or audio file and lumen-cut will create a project. The next step makes transcription explicit.",
    chooseFile: "Choose file",
    chooseHint: "Video or audio",
    pasteUrl: "Paste URL",
    urlHint: "YouTube or media URL",
    record: "Record audio",
    recordHint: "Use your Mac microphone",
    urlLabel: "Media URL",
    urlPlaceholder: "https://…",
    importUrl: "Download & transcribe",
    cancelImport: "Cancel download & transcription",
    cancellingImport: "Stopping…",
    cancelledImport: "Download and transcription were cancelled.",
    beginRecord: "Start recording",
    stopRecord: "Stop & use recording",
    discardRecord: "Cancel recording",
    recording: "Recording",
    recordingHint: "Start speaking now. When you are done, stop to create the project.",
    finishingRecord: "Saving your recording and creating the project…",
    importing: "Reading media and creating your project…",
    downloading: "Downloading and transcribing. Longer media can take a while…",
    recent: "Projects",
    empty: "No projects yet. Choose a file to get started.",
    open: "Open project",
    words: "words",
    paragraphs: "paragraphs",
    ready: "Ready to transcribe",
    moreActions: "More project actions",
    reveal: "Show in Finder",
    delete: "Delete project",
    deleteConfirm: "Delete project files? The original media will not be removed.",
    cancel: "Cancel",
    deleted: "Project deleted. The original media remains in its original location.",
    errorTitle: "That step did not finish",
  },
} as const;

function identityFrom(input: string, prefix = "project") {
  const leaf = input.split(/[\\/]/).pop()?.split("?")[0] || prefix;
  const title = leaf.replace(/\.[^.]+$/, "") || prefix;
  const slug =
    title
      .normalize("NFKC")
      .replace(/[^\p{L}\p{N}._-]+/gu, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 48) || prefix;
  const now = new Date();
  const stamp = [
    now.getFullYear(),
    String(now.getMonth() + 1).padStart(2, "0"),
    String(now.getDate()).padStart(2, "0"),
    "-",
    String(now.getHours()).padStart(2, "0"),
    String(now.getMinutes()).padStart(2, "0"),
    String(now.getSeconds()).padStart(2, "0"),
    "-",
    String(now.getMilliseconds()).padStart(3, "0"),
  ].join("");
  return { pid: `${slug}-${stamp}`, title };
}

function humanError(error: unknown, lang: Lang) {
  const raw = String(error).replace(/^Error:\s*/i, "");
  if (/ffmpeg/i.test(raw)) {
    return lang === "zh"
      ? "无法读取或录制媒体。请确认 ffmpeg 已安装，并允许 lumen-cut 使用麦克风。"
      : "The media could not be read or recorded. Check that ffmpeg is installed and microphone access is allowed.";
  }
  if (/yt-dlp/i.test(raw)) {
    return lang === "zh"
      ? "链接下载失败。请检查链接是否公开可访问，以及 yt-dlp 是否已安装。"
      : "The URL could not be downloaded. Check that it is public and yt-dlp is installed.";
  }
  if (/while transcription|while recording/i.test(raw)) {
    return lang === "zh"
      ? "项目仍在处理媒体，请先等待任务结束或取消任务。"
      : "This project is still processing media. Wait for it to finish or cancel the task first.";
  }
  return raw;
}

function recordingClock(elapsedSeconds: number) {
  const minutes = Math.floor(elapsedSeconds / 60);
  const seconds = elapsedSeconds % 60;
  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
}

function ingestPhaseLabel(phase: TranscriptionJobStatus["phase"], lang: Lang) {
  const labels: Record<TranscriptionJobStatus["phase"], [string, string]> = {
    preparing: ["正在准备下载", "Preparing download"],
    downloading: ["正在下载媒体", "Downloading media"],
    extracting: ["正在提取音频", "Extracting audio"],
    analyzing: ["正在分析媒体", "Analyzing media"],
    transcribing: ["正在识别语音与时码", "Recognizing speech and timing"],
    saving: ["正在创建项目", "Creating the project"],
    exporting: ["正在生成字幕文件", "Creating subtitle files"],
    completed: ["项目已就绪", "Project ready"],
    cancelling: ["正在安全停止", "Stopping safely"],
    cancelled: ["已取消", "Cancelled"],
    failed: ["处理失败", "Processing failed"],
  };
  return labels[phase][lang === "zh" ? 0 : 1];
}

export function ProjectsView({
  currentPid,
  lang,
  onDeleteProject,
  onOpenProject,
}: Props) {
  const c = COPY[lang];
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [composer, setComposer] = useState<Composer>(null);
  const [busy, setBusy] = useState<BusyKind>(null);
  const [error, setError] = useState<string | null>(null);
  const [url, setUrl] = useState("");
  const [recording, setRecording] = useState<ActiveRecording | null>(null);
  const [elapsedSeconds, setElapsedSeconds] = useState(0);
  const [urlJob, setUrlJob] = useState<UrlIngestJob | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [menuPid, setMenuPid] = useState<string | null>(null);
  const [confirmDeletePid, setConfirmDeletePid] = useState<string | null>(null);
  const [projectAction, setProjectAction] = useState<string | null>(null);
  const recordingRef = useRef<ActiveRecording | null>(null);

  const refresh = async () => {
    try {
      setProjects(await projectList());
    } catch (nextError) {
      setError(humanError(nextError, lang));
    }
  };

  useEffect(() => {
    void refresh();
    const saved = window.localStorage.getItem(URL_JOB_KEY);
    if (saved) {
      try {
        const identity = JSON.parse(saved) as { pid: string; title: string };
        void transcriptionStatus(identity.pid)
          .then((status) => {
            if (status.state === "running" || status.state === "cancelling") {
              setUrlJob({ ...identity, status });
              setBusy("url");
            } else {
              window.localStorage.removeItem(URL_JOB_KEY);
            }
          })
          .catch(() => window.localStorage.removeItem(URL_JOB_KEY));
      } catch {
        window.localStorage.removeItem(URL_JOB_KEY);
      }
    }
  }, []);

  useEffect(() => {
    if (!urlJob || !["running", "cancelling"].includes(urlJob.status.state)) return;
    let disposed = false;
    const timer = window.setInterval(() => {
      void transcriptionStatus(urlJob.pid)
        .then(async (status) => {
          if (disposed) return;
          if (status.state === "completed") {
            window.clearInterval(timer);
            window.localStorage.removeItem(URL_JOB_KEY);
            await refresh();
            if (disposed) return;
            setUrl("");
            setUrlJob(null);
            setBusy(null);
            onOpenProject(urlJob.pid, urlJob.title);
            return;
          }
          if (status.state === "cancelled") {
            window.clearInterval(timer);
            window.localStorage.removeItem(URL_JOB_KEY);
            setUrlJob(null);
            setBusy(null);
            setMessage(c.cancelledImport);
            return;
          }
          if (status.state === "failed") {
            window.clearInterval(timer);
            window.localStorage.removeItem(URL_JOB_KEY);
            setUrlJob(null);
            setBusy(null);
            setError(humanError(status.error || "Media processing failed", lang));
            return;
          }
          setUrlJob((current) => current && { ...current, status });
        })
        .catch((nextError) => {
          if (disposed) return;
          window.clearInterval(timer);
          setBusy(null);
          setUrlJob(null);
          setError(humanError(nextError, lang));
        });
    }, 500);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [urlJob?.pid, urlJob?.status.state, lang]);

  useEffect(() => {
    if (!recording) {
      setElapsedSeconds(0);
      return;
    }
    const update = () =>
      setElapsedSeconds(Math.max(0, Math.floor((Date.now() - recording.startedAt) / 1000)));
    update();
    const timer = window.setInterval(update, 250);
    return () => window.clearInterval(timer);
  }, [recording]);

  useEffect(
    () => () => {
      const active = recordingRef.current;
      if (active) void recordingCancel(active.pid).catch(() => undefined);
    },
    [],
  );

  const handleChooseFile = async () => {
    setError(null);
    setMessage(null);
    let path: string | null;
    try {
      path = await pickMediaFile();
    } catch (nextError) {
      setError(humanError(nextError, lang));
      return;
    }
    if (!path) return;

    setBusy("file");
    try {
      const { pid, title } = identityFrom(path);
      await projectCreate(pid, path, null, title);
      await refresh();
      onOpenProject(pid, title);
    } catch (nextError) {
      setError(humanError(nextError, lang));
    } finally {
      setBusy(null);
    }
  };

  const handleUrl = async () => {
    const mediaUrl = url.trim();
    if (!/^https?:\/\//i.test(mediaUrl)) {
      setError(
        lang === "zh"
          ? "请输入以 http:// 或 https:// 开头的完整链接。"
          : "Enter a complete URL beginning with http:// or https://.",
      );
      return;
    }
    setError(null);
    setMessage(null);
    setBusy("url");
    try {
      const { pid, title } = identityFrom(mediaUrl, "download");
      const status = await transcriptionStart(mediaUrl, null, title, null, pid);
      window.localStorage.setItem(URL_JOB_KEY, JSON.stringify({ pid, title }));
      setUrlJob({ pid, title, status });
    } catch (nextError) {
      setError(humanError(nextError, lang));
      setBusy(null);
    }
  };

  const handleCancelUrl = async () => {
    if (!urlJob) return;
    setError(null);
    setMessage(null);
    try {
      const status = await transcriptionCancel(urlJob.pid);
      setUrlJob({ ...urlJob, status });
    } catch (nextError) {
      setError(humanError(nextError, lang));
    }
  };

  const handleStartRecord = async () => {
    setError(null);
    setMessage(null);
    setBusy("record");
    try {
      const identity = identityFrom(
        lang === "zh" ? "录音" : "Recording",
        "recording",
      );
      await recordingStart(identity.pid);
      const active = { ...identity, startedAt: Date.now() };
      recordingRef.current = active;
      setRecording(active);
    } catch (nextError) {
      setError(humanError(nextError, lang));
    } finally {
      setBusy(null);
    }
  };

  const handleStopRecord = async () => {
    if (!recording) return;
    setError(null);
    setBusy("record");
    try {
      const result = await recordingStop(recording.pid);
      recordingRef.current = null;
      setRecording(null);
      await projectCreate(recording.pid, result.path, null, recording.title);
      await refresh();
      onOpenProject(recording.pid, recording.title);
    } catch (nextError) {
      recordingRef.current = null;
      setRecording(null);
      setError(humanError(nextError, lang));
    } finally {
      setBusy(null);
    }
  };

  const handleDiscardRecord = async () => {
    if (!recording) return;
    setError(null);
    setMessage(null);
    setBusy("record");
    try {
      await recordingCancel(recording.pid);
      recordingRef.current = null;
      setRecording(null);
    } catch (nextError) {
      setError(humanError(nextError, lang));
    } finally {
      setBusy(null);
    }
  };

  const handleRevealProject = async (pid: string) => {
    setError(null);
    setMessage(null);
    setProjectAction(`reveal-${pid}`);
    try {
      await projectReveal(pid);
      setMenuPid(null);
    } catch (nextError) {
      setError(humanError(nextError, lang));
    } finally {
      setProjectAction(null);
    }
  };

  const handleDeleteProject = async (pid: string) => {
    setError(null);
    setMessage(null);
    setProjectAction(`delete-${pid}`);
    try {
      const deleted = await projectDelete(pid);
      if (!deleted) {
        throw new Error(
          lang === "zh" ? "项目已经不存在。" : "The project no longer exists.",
        );
      }
      onDeleteProject(pid);
      setConfirmDeletePid(null);
      setMenuPid(null);
      setMessage(c.deleted);
      await refresh();
    } catch (nextError) {
      setError(humanError(nextError, lang));
    } finally {
      setProjectAction(null);
    }
  };

  const status =
    busy === "file"
      ? c.importing
      : busy === "url"
        ? urlJob
          ? null
          : c.downloading
        : busy === "record"
          ? recording
            ? c.finishingRecord
            : null
          : null;
  const creationLocked = busy !== null || recording !== null;

  return (
    <section className="projects-view">
      <div className="welcome">
        <p className="eyebrow">{c.eyebrow}</p>
        <h1>{c.title}</h1>
        <p className="welcome-copy">{c.description}</p>

        <div className="start-actions" aria-label={c.eyebrow}>
          <button
            className="start-action primary"
            disabled={creationLocked}
            onClick={handleChooseFile}
          >
            <span className="action-icon"><UploadIcon /></span>
            <span>
              <strong>{c.chooseFile}</strong>
              <small>{c.chooseHint}</small>
            </span>
          </button>
          <button
            aria-expanded={composer === "url"}
            className="start-action"
            disabled={creationLocked}
            onClick={() => setComposer((value) => (value === "url" ? null : "url"))}
          >
            <span className="action-icon"><LinkIcon /></span>
            <span>
              <strong>{c.pasteUrl}</strong>
              <small>{c.urlHint}</small>
            </span>
          </button>
          <button
            aria-expanded={composer === "record"}
            className="start-action"
            disabled={creationLocked}
            onClick={() => setComposer((value) => (value === "record" ? null : "record"))}
          >
            <span className="action-icon"><MicrophoneIcon /></span>
            <span>
              <strong>{c.record}</strong>
              <small>{c.recordHint}</small>
            </span>
          </button>
        </div>

        {composer === "url" && (
          <div className="composer" role="group" aria-label={c.urlLabel}>
            <label htmlFor="media-url">{c.urlLabel}</label>
            <div className="composer-row">
              <input
                autoFocus
                id="media-url"
                placeholder={c.urlPlaceholder}
                type="url"
                value={url}
                onChange={(event) => setUrl(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") void handleUrl();
                }}
              />
              <button className="button-primary" disabled={busy !== null} onClick={handleUrl}>
                {c.importUrl}
              </button>
            </div>
          </div>
        )}

        {composer === "record" && (
          <div
            className={`composer recording-composer${recording ? " is-recording" : ""}`}
            role="group"
            aria-label={c.record}
          >
            {recording ? (
              <>
                <div className="recording-live" role="status" aria-live="polite">
                  <span className="recording-dot" aria-hidden="true" />
                  <div>
                    <strong>{c.recording}</strong>
                    <small>{c.recordingHint}</small>
                  </div>
                  <time>{recordingClock(elapsedSeconds)}</time>
                </div>
                <div className="recording-actions">
                  <button
                    className="button-primary recording-stop"
                    disabled={busy !== null}
                    onClick={handleStopRecord}
                  >
                    {c.stopRecord}
                  </button>
                  <button
                    className="button-quiet"
                    disabled={busy !== null}
                    onClick={handleDiscardRecord}
                  >
                    {c.discardRecord}
                  </button>
                </div>
              </>
            ) : (
              <button
                className="button-primary"
                disabled={busy !== null}
                onClick={handleStartRecord}
              >
                <MicrophoneIcon />
                {c.beginRecord}
              </button>
            )}
          </div>
        )}

        {status && (
          <div className="status-banner" role="status">
            <span className="spinner" aria-hidden="true" />
            {status}
          </div>
        )}

        {urlJob && (
          <div className="ingest-progress" aria-live="polite">
            <div>
              <strong>{ingestPhaseLabel(urlJob.status.phase, lang)}</strong>
              <span>{urlJob.status.progress}%</span>
            </div>
            <progress max={100} value={urlJob.status.progress} />
            <button
              className="button-quiet"
              disabled={urlJob.status.state === "cancelling"}
              onClick={handleCancelUrl}
            >
              {urlJob.status.state === "cancelling" ? c.cancellingImport : c.cancelImport}
            </button>
          </div>
        )}

        {message && (
          <div className="notice info-notice" role="status">
            <span>{message}</span>
          </div>
        )}

        {error && (
          <div className="notice error-notice" role="alert">
            <strong>{c.errorTitle}</strong>
            <span>{error}</span>
          </div>
        )}
      </div>

      <div className="project-library">
        <div className="section-heading">
          <h2>{c.recent}</h2>
          <span>{projects.length}</span>
        </div>
        {projects.length === 0 ? (
          <div className="empty-library">
            <FolderIcon />
            <p>{c.empty}</p>
          </div>
        ) : (
          <div className="project-rows">
            {projects.map((project) => {
              const isCurrent = project.pid === currentPid;
              const hasTranscript = project.word_count > 0;
              return (
                <article
                  className={`project-row${isCurrent ? " current" : ""}`}
                  key={project.pid}
                >
                  <button
                    className="project-open"
                    disabled={creationLocked}
                    onClick={() => onOpenProject(project.pid, project.title)}
                  >
                    <span className="project-file-icon"><FolderIcon /></span>
                    <span className="project-main">
                      <strong>{project.title}</strong>
                      <small>
                        {hasTranscript
                          ? `${project.word_count} ${c.words} · ${project.paragraph_count} ${c.paragraphs}`
                          : c.ready}
                      </small>
                    </span>
                    <span className={`project-state ${hasTranscript ? "done" : "waiting"}`}>
                      {hasTranscript ? (lang === "zh" ? "已转写" : "Transcribed") : c.ready}
                    </span>
                    <span className="sr-only">{c.open}</span>
                    <ChevronRightIcon />
                  </button>
                  <div className="project-more">
                    <button
                      aria-expanded={menuPid === project.pid}
                      aria-label={`${c.moreActions}: ${project.title}`}
                      className="project-more-button"
                      disabled={creationLocked}
                      onClick={() => {
                        setConfirmDeletePid(null);
                        setMenuPid((current) =>
                          current === project.pid ? null : project.pid,
                        );
                      }}
                    >
                      <span aria-hidden="true">•••</span>
                    </button>
                    {menuPid === project.pid && (
                      <div className="project-menu">
                        {confirmDeletePid === project.pid ? (
                          <div className="project-delete-confirm">
                            <p>{c.deleteConfirm}</p>
                            <div>
                              <button
                                className="button-quiet"
                                disabled={projectAction !== null}
                                onClick={() => setConfirmDeletePid(null)}
                              >
                                {c.cancel}
                              </button>
                              <button
                                className="button-danger"
                                disabled={projectAction !== null}
                                onClick={() => handleDeleteProject(project.pid)}
                              >
                                {projectAction === `delete-${project.pid}`
                                  ? lang === "zh" ? "删除中…" : "Deleting…"
                                  : c.delete}
                              </button>
                            </div>
                          </div>
                        ) : (
                          <>
                            <button
                              disabled={projectAction !== null}
                              onClick={() => handleRevealProject(project.pid)}
                            >
                              {projectAction === `reveal-${project.pid}`
                                ? lang === "zh" ? "正在打开…" : "Opening…"
                                : c.reveal}
                            </button>
                            <button
                              className="danger-action"
                              disabled={projectAction !== null}
                              onClick={() => setConfirmDeletePid(project.pid)}
                            >
                              {c.delete}
                            </button>
                          </>
                        )}
                      </div>
                    )}
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </div>
    </section>
  );
}
