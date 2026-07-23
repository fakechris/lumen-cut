import { useCallback, useEffect, useRef, useState } from "react";
import {
  brollPreviewStatus,
  revealLogs,
  speakerReidentifyStatus,
  taskStatus,
  transcriptionStatus,
  videoExportStatus,
} from "../api";
import type { Lang } from "../i18n";
import type {
  BrollPreviewJobStatus,
  ProjectSummary,
  SpeakerAnalysisJobStatus,
  TaskStatus,
  TranscriptionJobStatus,
  VideoExportJobStatus,
} from "../types";

interface Props {
  lang: Lang;
  projects: ProjectSummary[];
  onOpenProject: (id: string, title: string) => void;
}

type ProjectTaskState = {
  project: ProjectSummary;
  status: TaskStatus | null;
  transcription: TranscriptionJobStatus | null;
  speakers: SpeakerAnalysisJobStatus | null;
  broll: BrollPreviewJobStatus | null;
  videoExport: VideoExportJobStatus | null;
  error: string | null;
};

const isRunning = (state: string | undefined) => state === "running" || state === "cancelling";
const isFailed = (state: string | undefined) => state === "failed";

function hasActiveTask(item: ProjectTaskState) {
  return (item.status?.pending ?? 0) > 0
    || isRunning(item.transcription?.state)
    || isRunning(item.speakers?.state)
    || isRunning(item.broll?.state)
    || isRunning(item.videoExport?.state);
}

function stateLabel(item: ProjectTaskState, lang: Lang) {
  const { status } = item;
  const failed = status?.kinds.reduce((sum, task) => sum + task.failed, 0) ?? 0;
  const paused = status?.kinds.some((task) => task.state === "paused" || task.state === "failed");
  const hasStandalone = Boolean(item.transcription || item.speakers || item.broll || item.videoExport);
  if ((!status || status.kinds.length === 0) && !hasStandalone) {
    return lang === "zh" ? "暂无任务" : "No tasks";
  }
  if ([item.transcription, item.speakers, item.broll, item.videoExport].some((job) => isFailed(job?.state))) {
    return lang === "zh" ? "需要处理" : "Needs attention";
  }
  if (paused || failed > 0) return lang === "zh" ? "需要处理" : "Needs attention";
  if (hasActiveTask(item)) return lang === "zh" ? "正在后台处理" : "Running in background";
  return lang === "zh" ? "最近任务已完成" : "Latest tasks complete";
}

async function optional<T>(request: Promise<T>): Promise<T | null> {
  try {
    return await request;
  } catch {
    return null;
  }
}

async function loadProjectState(project: ProjectSummary): Promise<ProjectTaskState> {
  const [status, transcription, speakers, broll, videoExport] = await Promise.all([
    optional(taskStatus(project.pid)),
    optional(transcriptionStatus(project.pid)),
    optional(speakerReidentifyStatus(project.pid)),
    optional(brollPreviewStatus(project.pid)),
    optional(videoExportStatus(project.pid)),
  ]);
  return {
    project,
    status,
    transcription,
    speakers,
    broll,
    videoExport,
    error: null,
  };
}

export function TaskCenterView({ lang, projects, onOpenProject }: Props) {
  const [items, setItems] = useState<ProjectTaskState[]>([]);
  const [loading, setLoading] = useState(true);
  const [logMessage, setLogMessage] = useState<string | null>(null);
  const refreshGeneration = useRef(0);
  const itemsRef = useRef<ProjectTaskState[]>([]);
  const zh = lang === "zh";

  const refresh = useCallback(async () => {
    const generation = ++refreshGeneration.current;
    setLoading(true);
    const next: ProjectTaskState[] = [];
    for (let index = 0; index < projects.length; index += 4) {
      next.push(...await Promise.all(projects.slice(index, index + 4).map(loadProjectState)));
    }
    if (generation !== refreshGeneration.current) return;
    next.sort((left, right) => {
      const active = Number(hasActiveTask(right)) - Number(hasActiveTask(left));
      if (active !== 0) return active;
      return right.project.updated_at.localeCompare(left.project.updated_at);
    });
    itemsRef.current = next;
    setItems(next);
    setLoading(false);
  }, [projects]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    itemsRef.current = items;
    if (!items.some(hasActiveTask)) return;
    const timer = window.setInterval(() => {
      const activeProjects = itemsRef.current.filter(hasActiveTask).map((item) => item.project);
      if (activeProjects.length === 0) return;
      void Promise.all(activeProjects.map(loadProjectState)).then((updates) => {
        const byPid = new Map(updates.map((item) => [item.project.pid, item]));
        setItems((current) => current.map((item) => byPid.get(item.project.pid) ?? item));
      });
    }, 2500);
    return () => window.clearInterval(timer);
  }, [items]);

  const openLogs = async () => {
    try {
      const path = await revealLogs();
      setLogMessage(zh ? `已在 Finder 打开：${path}` : `Opened in Finder: ${path}`);
    } catch (error) {
      setLogMessage(String(error));
    }
  };

  const activeCount = items.filter(hasActiveTask).length;

  return (
    <section className="task-center-view">
      <header className="task-center-header">
        <div>
          <p className="eyebrow">{zh ? "后台任务" : "Background tasks"}</p>
          <h1>{zh ? "处理中心" : "Processing center"}</h1>
          <p>
            {zh
              ? "转写、说话人、翻译、B-roll 和导出会按需自动启动；不需要手动启动服务器。"
              : "Transcription, speakers, translation, B-roll, and export start when needed. No server setup is required."}
          </p>
        </div>
        <div className="task-center-actions">
          <button className="button-quiet" disabled={loading} onClick={() => void refresh()}>
            {loading ? (zh ? "正在刷新…" : "Refreshing…") : (zh ? "刷新" : "Refresh")}
          </button>
          <button className="button-quiet" onClick={() => void openLogs()}>
            {zh ? "打开日志" : "Open logs"}
          </button>
        </div>
      </header>

      <div className="task-center-summary">
        <strong>{activeCount}</strong>
        <span>{zh ? "个项目正在处理" : "projects processing"}</span>
        <small>{zh ? "离开此页面不会中断任务" : "Tasks continue when you leave this page"}</small>
      </div>

      {logMessage && <p className="task-center-log-message" role="status">{logMessage}</p>}

      <div className="task-center-list">
        {items.map((item) => {
          const { project, status, error } = item;
          const failedTotal = status?.kinds.reduce((sum, task) => sum + task.failed, 0) ?? 0;
          const total = status ? status.pending + status.done + failedTotal : 0;
          const progress = total > 0 ? ((status?.done ?? 0) / total) * 100 : 0;
          const standalone = [
            item.transcription && {
              name: zh ? "转写与时码" : "Transcription",
              state: item.transcription.state,
              phase: item.transcription.phase,
              progress: item.transcription.progress,
              detail: [
                item.transcription.device,
                item.transcription.cpuPercent !== null ? `CPU ${Math.round(item.transcription.cpuPercent)}%` : null,
                item.transcription.peakMemoryMb !== null ? `${Math.round(item.transcription.peakMemoryMb)} MB` : null,
              ].filter(Boolean).join(" · "),
            },
            item.speakers && {
              name: zh ? "说话人" : "Speakers",
              state: item.speakers.state,
              phase: item.speakers.phase,
              progress: item.speakers.progress,
              detail: [
                item.speakers.device?.toUpperCase(),
                item.speakers.cpuPercent !== null ? `CPU ${Math.round(item.speakers.cpuPercent)}%` : null,
                item.speakers.peakMemoryMb !== null ? `${Math.round(item.speakers.peakMemoryMb)} MB` : null,
              ].filter(Boolean).join(" · "),
            },
            item.broll && {
              name: "B-roll",
              state: item.broll.state,
              phase: item.broll.phase,
              progress: item.broll.progress,
              detail: item.broll.encoder || "",
            },
            item.videoExport && {
              name: zh ? "视频导出" : "Video export",
              state: item.videoExport.state,
              phase: item.videoExport.phase,
              progress: item.videoExport.progress,
              detail: item.videoExport.encoder || "",
            },
          ].filter((job): job is NonNullable<typeof job> => Boolean(job));
          return (
            <article key={project.pid}>
              <button
                className="task-project-button"
                onClick={() => onOpenProject(project.pid, project.title)}
              >
                <span className={hasActiveTask(item) ? "task-project-pulse" : "task-project-dot"} />
                <span>
                  <strong>{project.title}</strong>
                  <small>{stateLabel(item, lang)}</small>
                </span>
              </button>
              <div className="task-project-progress">
                {error ? (
                  <p>{error}</p>
                ) : (standalone.length > 0 || (status && status.kinds.length > 0)) ? (
                  <>
                    {standalone.map((job) => (
                      <div className="task-standalone-job" key={job.name}>
                        <span>
                          {job.name} · {job.phase}
                          <b>{Math.round(job.progress)}%</b>
                        </span>
                        <progress max={100} value={job.progress} />
                        {job.detail && <small>{job.detail}</small>}
                      </div>
                    ))}
                    <div>
                      {status?.kinds.slice(0, 5).map((task) => (
                        <span key={`${task.kind}-${task.lang || ""}`}>
                          {task.kind}{task.lang ? ` · ${task.lang.toUpperCase()}` : ""}
                          <b>{task.pending > 0 ? `${task.done}/${task.calls ?? task.pending + task.done + task.failed}` : task.failed > 0 ? `${task.failed} ${zh ? "失败" : "failed"}` : (zh ? "完成" : "done")}</b>
                        </span>
                      ))}
                    </div>
                    {status && status.kinds.length > 0 && <progress max={100} value={progress} />}
                  </>
                ) : (
                  <small>{zh ? "启动一项处理后，阶段与进度会显示在这里。" : "Stages and progress appear here when processing starts."}</small>
                )}
              </div>
            </article>
          );
        })}
        {!loading && items.length === 0 && (
          <div className="task-center-empty">
            {zh ? "还没有项目。先导入一段视频或音频。" : "No projects yet. Import a video or audio file first."}
          </div>
        )}
      </div>
    </section>
  );
}
