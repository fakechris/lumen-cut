import { useCallback, useEffect, useRef, useState } from "react";
import {
  brollPreviewCancel,
  brollPreviewStart,
  brollPreviewStatus,
  performanceStatus,
  revealLogs,
  speakerReidentifyCancel,
  speakerReidentifyStart,
  speakerReidentifyStatus,
  taskPause,
  taskPrioritize,
  taskResume,
  taskRetry,
  taskStatus,
  transcriptionCancel,
  transcriptionRetry,
  transcriptionStatus,
  videoExportCancel,
  videoExportStart,
  videoExportStatus,
} from "../api";
import type { Lang } from "../i18n";
import type {
  BrollPreviewJobStatus,
  PerformanceStatus,
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

function etaLabel(progress: number, elapsedSeconds: number | null | undefined, lang: Lang) {
  if (!elapsedSeconds || progress <= 0 || progress >= 100) return null;
  const remaining = Math.max(0, elapsedSeconds * (100 - progress) / progress);
  if (!Number.isFinite(remaining)) return null;
  if (remaining < 90) {
    return lang === "zh" ? `约 ${Math.max(1, Math.round(remaining))} 秒` : `about ${Math.max(1, Math.round(remaining))} sec`;
  }
  const minutes = Math.max(1, Math.round(remaining / 60));
  return lang === "zh" ? `约 ${minutes} 分钟` : `about ${minutes} min`;
}

function resourceSummary(
  peakMemoryMb: number | null | undefined,
  memoryLimitMb: number | null | undefined,
  lang: Lang,
) {
  if (peakMemoryMb === null || peakMemoryMb === undefined) return null;
  const peak = Math.round(peakMemoryMb);
  if (!memoryLimitMb || memoryLimitMb <= 0) {
    return {
      label: lang === "zh" ? `内存峰值 ${peak} MB` : `Peak memory ${peak} MB`,
      warning: null,
    };
  }
  const limit = Math.round(memoryLimitMb);
  const ratio = peakMemoryMb / memoryLimitMb;
  return {
    label: lang === "zh"
      ? `内存峰值 ${peak} / ${limit} MB · ${Math.round(ratio * 100)}%`
      : `Peak memory ${peak} / ${limit} MB · ${Math.round(ratio * 100)}%`,
    warning: ratio >= 0.85
      ? lang === "zh"
        ? "接近内存护栏；继续增长时任务会安全失败，避免拖垮桌面。"
        : "Near the memory guardrail. The task will fail safely if usage keeps growing."
      : null,
  };
}

function phaseLabel(phase: string, lang: Lang) {
  const labels: Record<string, [string, string]> = {
    waiting: ["等待计算资源", "Waiting for compute"],
    preparing: ["准备数据", "Preparing"],
    downloading: ["下载媒体", "Downloading media"],
    extracting: ["提取音频", "Extracting audio"],
    analyzing: ["分析媒体", "Analyzing media"],
    transcribing: ["识别语音", "Recognizing speech"],
    aligning: ["生成词级时码", "Aligning word timing"],
    saving: ["保存结果", "Saving results"],
    exporting: ["生成字幕", "Creating captions"],
    loading: ["加载模型", "Loading model"],
    segmenting: ["检测说话人片段", "Detecting speaker segments"],
    counting: ["估算说话人数", "Estimating speaker count"],
    embedding: ["提取声纹特征", "Computing voice embeddings"],
    finalizing: ["整理分析结果", "Finalizing"],
    encoding: ["视频编码", "Encoding video"],
    frames: ["生成预览帧", "Rendering preview frames"],
    completed: ["已完成", "Completed"],
    cancelling: ["正在安全停止", "Stopping safely"],
    cancelled: ["已取消", "Cancelled"],
    failed: ["失败", "Failed"],
  };
  return labels[phase]?.[lang === "zh" ? 0 : 1] ?? phase;
}

export function agentActivityLabel(
  task: TaskStatus["kinds"][number],
  lang: Lang,
) {
  if (task.queued === undefined && task.inFlight === undefined) return null;
  const queued = task.queued ?? 0;
  const inFlight = task.inFlight ?? 0;
  const retrying = task.retrying ?? 0;
  const parts: string[] = [];
  if (inFlight > 0) {
    parts.push(lang === "zh" ? `${inFlight} 个请求在途` : `${inFlight} in flight`);
  }
  if (queued > 0) {
    parts.push(lang === "zh" ? `${queued} 个等待发送` : `${queued} queued`);
  }
  if (retrying > 0) {
    parts.push(lang === "zh" ? `${retrying} 个正在重试` : `${retrying} retrying`);
  }
  if (task.attempt && task.maxAttempts && task.attempt > 1) {
    parts.push(
      lang === "zh"
        ? `当前第 ${task.attempt}/${task.maxAttempts} 次尝试`
        : `attempt ${task.attempt}/${task.maxAttempts}`,
    );
  }
  if (parts.length === 0 && task.pending > 0) {
    return lang === "zh" ? "模型已返回，正在校验并保存结果" : "Validating and saving returned results";
  }
  return parts.join(" · ");
}

function freshnessLabel(
  state: string,
  phase: string,
  updatedAt: number | null | undefined,
  lang: Lang,
) {
  if (!isRunning(state) || phase === "waiting" || !updatedAt) return null;
  const age = Math.max(0, Date.now() / 1000 - updatedAt);
  if (age < 45) return null;
  const duration = age < 120
    ? (lang === "zh" ? `${Math.round(age)} 秒` : `${Math.round(age)} sec`)
    : (lang === "zh"
      ? `${Math.max(2, Math.round(age / 60))} 分钟`
      : `${Math.max(2, Math.round(age / 60))} min`);
  return {
    stale: age >= 120,
    text: lang === "zh"
      ? `${duration}没有新进度；任务仍在监测，可查看资源占用或安全停止后重试。`
      : `No new progress for ${duration}. The task is still monitored; check resources or stop safely and retry.`,
  };
}

function heavyPipelineLabel(pipeline: string, lang: Lang) {
  const labels: Record<string, [string, string]> = {
    transcription: ["转写与时码", "Transcription"],
    "speaker-analysis": ["说话人分析", "Speaker analysis"],
    "speaker-analysis-cli": ["说话人分析", "Speaker analysis"],
    "broll-preview": ["B-roll 预览", "B-roll preview"],
    "video-export": ["视频导出", "Video export"],
    "project-thumbnail": ["项目缩略图", "Project thumbnail"],
    "timeline-visuals": ["时间线画面", "Timeline visuals"],
  };
  return labels[pipeline]?.[lang === "zh" ? 0 : 1] ?? pipeline;
}

async function optional<T>(request: Promise<T>): Promise<T | null> {
  try {
    return await request;
  } catch {
    return null;
  }
}

type OptionalJob<T> = {
  value: T | null;
  error: string | null;
};

async function optionalJob<T>(label: string, request: Promise<T>): Promise<OptionalJob<T>> {
  try {
    return { value: await request, error: null };
  } catch (error) {
    const message = String(error).replace(/^Error:\s*/i, "");
    if (/^no .+ (job|analysis)( for this project)?$/i.test(message)) {
      return { value: null, error: null };
    }
    return {
      value: null,
      error: `${label}: ${message}`,
    };
  }
}

async function loadProjectState(project: ProjectSummary): Promise<ProjectTaskState> {
  const [status, transcription, speakers, broll, videoExport] = await Promise.all([
    optionalJob("AI tasks", taskStatus(project.pid)),
    optionalJob("Transcription", transcriptionStatus(project.pid)),
    optionalJob("Speaker analysis", speakerReidentifyStatus(project.pid)),
    optionalJob("B-roll preview", brollPreviewStatus(project.pid)),
    optionalJob("Video export", videoExportStatus(project.pid)),
  ]);
  return {
    project,
    status: status.value,
    transcription: transcription.value,
    speakers: speakers.value,
    broll: broll.value,
    videoExport: videoExport.value,
    error: [
      status.error,
      transcription.error,
      speakers.error,
      broll.error,
      videoExport.error,
    ].filter(Boolean).join(" · ") || null,
  };
}

export function TaskCenterView({ lang, projects, onOpenProject }: Props) {
  const [items, setItems] = useState<ProjectTaskState[]>([]);
  const [loading, setLoading] = useState(true);
  const [logMessage, setLogMessage] = useState<string | null>(null);
  const [actionKey, setActionKey] = useState<string | null>(null);
  const [performance, setPerformance] = useState<PerformanceStatus | null>(null);
  const refreshGeneration = useRef(0);
  const itemsRef = useRef<ProjectTaskState[]>([]);
  const previouslyActive = useRef(new Set<string>());
  const zh = lang === "zh";

  const refresh = useCallback(async () => {
    const generation = ++refreshGeneration.current;
    setLoading(true);
    const performanceRequest = optional(performanceStatus());
    const next: ProjectTaskState[] = [];
    for (let index = 0; index < projects.length; index += 4) {
      next.push(...await Promise.all(projects.slice(index, index + 4).map(loadProjectState)));
    }
    if (generation !== refreshGeneration.current) return;
    setPerformance(await performanceRequest);
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
      void optional(performanceStatus()).then(setPerformance);
      void Promise.all(activeProjects.map(loadProjectState)).then((updates) => {
        const byPid = new Map(updates.map((item) => [item.project.pid, item]));
        setItems((current) => {
          const next = current.map((item) => byPid.get(item.project.pid) ?? item);
          for (const item of next) {
            const wasActive = previouslyActive.current.has(item.project.pid);
            const active = hasActiveTask(item);
            if (wasActive
              && !active
              && typeof Notification !== "undefined"
              && Notification.permission === "granted") {
              new Notification(
                lang === "zh" ? "Lumen Cut 处理完成" : "Lumen Cut processing complete",
                { body: item.project.title },
              );
            }
          }
          previouslyActive.current = new Set(
            next.filter(hasActiveTask).map((item) => item.project.pid),
          );
          return next;
        });
      });
    }, 2500);
    return () => window.clearInterval(timer);
  }, [items, lang]);

  useEffect(() => {
    previouslyActive.current = new Set(
      items.filter(hasActiveTask).map((item) => item.project.pid),
    );
  }, [items]);

  const runAction = async (
    key: string,
    action: () => Promise<unknown>,
    success: string,
  ) => {
    setActionKey(key);
    setLogMessage(null);
    try {
      await action();
      setLogMessage(success);
      await refresh();
    } catch (error) {
      setLogMessage(String(error).replace(/^Error:\s*/i, ""));
    } finally {
      setActionKey(null);
    }
  };

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

      <div className="task-compute-status" role="status">
        <strong>
          {performance?.activePipeline
            ? `${zh ? "当前重计算" : "Active compute"}：${heavyPipelineLabel(performance.activePipeline, lang)}`
            : (zh ? "当前没有重计算任务占用设备" : "No compute-heavy task is using the device")}
        </strong>
        <small>
          {(performance?.waitingPipelines ?? 0) > 0
            ? (zh
              ? `${performance!.waitingPipelines} 个任务正在排队；系统会自动串行运行，用户不需要启动服务器。`
              : `${performance!.waitingPipelines} task(s) queued. They run automatically in sequence; no server setup is required.`)
            : (zh ? "重任务会自动排队，避免 CPU、GPU 与统一内存互相争抢。" : "Heavy jobs queue automatically to avoid CPU, GPU, and unified-memory contention.")}
        </small>
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
              id: "transcription",
              name: zh ? "转写与时码" : "Transcription",
              state: item.transcription.state,
              phase: item.transcription.phase,
              progress: item.transcription.progress,
              detail: [
                item.transcription.device,
                item.transcription.cpuPercent != null ? `CPU ${Math.round(item.transcription.cpuPercent)}%` : null,
              ].filter(Boolean).join(" · "),
              resource: resourceSummary(
                item.transcription.peakMemoryMb,
                item.transcription.memoryLimitMb,
                lang,
              ),
              error: item.transcription.error || null,
              updatedAt: item.transcription.updatedAt,
              eta: etaLabel(
                item.transcription.progress,
                item.transcription.elapsedSeconds,
                lang,
              ),
              cancel: isRunning(item.transcription.state)
                ? () => transcriptionCancel(project.pid)
                : null,
              retry: isFailed(item.transcription.state)
                ? () => transcriptionRetry(project.pid)
                : null,
            },
            item.speakers && {
              id: "speakers",
              name: zh ? "说话人" : "Speakers",
              state: item.speakers.state,
              phase: item.speakers.phase,
              progress: item.speakers.progress,
              detail: [
                item.speakers.device?.toUpperCase(),
                item.speakers.cpuPercent != null ? `CPU ${Math.round(item.speakers.cpuPercent)}%` : null,
              ].filter(Boolean).join(" · "),
              resource: resourceSummary(
                item.speakers.peakMemoryMb,
                item.speakers.memoryLimitMb,
                lang,
              ),
              error: item.speakers.error,
              updatedAt: item.speakers.updatedAt,
              eta: etaLabel(item.speakers.progress, item.speakers.elapsedSeconds, lang),
              cancel: isRunning(item.speakers.state)
                ? () => speakerReidentifyCancel(project.pid)
                : null,
              retry: isFailed(item.speakers.state)
                ? () => speakerReidentifyStart(project.pid)
                : null,
            },
            item.broll && {
              id: "broll",
              name: "B-roll",
              state: item.broll.state,
              phase: item.broll.phase,
              progress: item.broll.progress,
              detail: item.broll.encoder || "",
              resource: null,
              error: item.broll.error,
              updatedAt: item.broll.updatedAt,
              eta: null,
              cancel: isRunning(item.broll.state)
                ? () => brollPreviewCancel(project.pid)
                : null,
              retry: isFailed(item.broll.state)
                ? () => brollPreviewStart(project.pid)
                : null,
            },
            item.videoExport && {
              id: "video-export",
              name: zh ? "视频导出" : "Video export",
              state: item.videoExport.state,
              phase: item.videoExport.phase,
              progress: item.videoExport.progress,
              detail: item.videoExport.encoder || "",
              resource: null,
              error: item.videoExport.error,
              updatedAt: item.videoExport.updatedAt,
              eta: null,
              cancel: isRunning(item.videoExport.state)
                ? () => videoExportCancel(project.pid)
                : null,
              retry: isFailed(item.videoExport.state)
                ? () => videoExportStart(project.pid, item.videoExport!.settings)
                : null,
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
                {error && <p className="task-status-read-error">{error}</p>}
                {(standalone.length > 0 || (status && status.kinds.length > 0)) ? (
                  <>
                    {standalone.map((job) => (
                      <div className="task-standalone-job" key={job.id}>
                        <span className="task-job-heading">
                          <span>
                            {job.name} · {phaseLabel(job.phase, lang)}
                            {job.eta && <small> · {job.eta}</small>}
                          </span>
                          <b>{Math.round(job.progress)}%</b>
                        </span>
                        <progress max={100} value={job.progress} />
                        {freshnessLabel(job.state, job.phase, job.updatedAt, lang) && (
                          <small className={freshnessLabel(job.state, job.phase, job.updatedAt, lang)?.stale ? "task-stale-warning" : "task-update-age"}>
                            {freshnessLabel(job.state, job.phase, job.updatedAt, lang)?.text}
                          </small>
                        )}
                        {job.resource && (
                          <small className={job.resource.warning ? "task-resource-warning" : "task-resource-status"}>
                            {job.resource.label}
                            {job.resource.warning ? ` · ${job.resource.warning}` : ""}
                          </small>
                        )}
                        <div className="task-job-detail">
                          <span>
                            {job.detail && <small>{job.detail}</small>}
                            {job.error && <small className="task-inline-error">{job.error}</small>}
                          </span>
                          {job.cancel && (
                            <button
                              className="button-quiet"
                              disabled={actionKey === `${project.pid}:${job.id}`}
                              onClick={() => void runAction(
                                `${project.pid}:${job.id}`,
                                job.cancel!,
                                zh ? "已发送安全停止请求。" : "Requested a safe stop.",
                              )}
                            >
                              {actionKey === `${project.pid}:${job.id}`
                                ? zh ? "正在停止…" : "Stopping…"
                                : zh ? "停止" : "Stop"}
                            </button>
                          )}
                          {job.retry && (
                            <button
                              className="button-quiet"
                              disabled={actionKey === `${project.pid}:${job.id}`}
                              onClick={() => void runAction(
                                `${project.pid}:${job.id}`,
                                job.retry!,
                                zh ? "任务已重新启动。" : "Restarted the task.",
                              )}
                            >
                              {actionKey === `${project.pid}:${job.id}`
                                ? zh ? "正在启动…" : "Starting…"
                                : zh ? "重试" : "Retry"}
                            </button>
                          )}
                        </div>
                      </div>
                    ))}
                    <div className="task-agent-jobs">
                      {status?.kinds.map((task) => {
                        const taskKey = `${project.pid}:agent:${task.kind}`;
                        const calls = task.calls ?? task.pending + task.done + task.failed;
                        const taskProgress = calls > 0 ? (task.done / calls) * 100 : 100;
                        const elapsed = task.startedAt
                          ? Math.max(0, Date.now() / 1000 - task.startedAt)
                          : null;
                        const eta = etaLabel(taskProgress, elapsed, lang);
                        const freshness = freshnessLabel(
                          task.state ?? "completed",
                          task.state ?? "completed",
                          task.updatedAt,
                          lang,
                        );
                        const activity = agentActivityLabel(task, lang);
                        return (
                          <div className="task-agent-job" key={`${task.kind}-${task.lang || ""}`}>
                            <span>
                              <strong>{task.kind}{task.lang ? ` · ${task.lang.toUpperCase()}` : ""}</strong>
                              <small>
                                {task.pending > 0
                                  ? `${task.done}/${calls}`
                                  : task.failed > 0
                                    ? `${task.failed} ${zh ? "失败" : "failed"}`
                                    : zh ? "完成" : "done"}
                                {eta ? ` · ${eta}` : ""}
                              </small>
                              {activity && <small className="task-live-activity">{activity}</small>}
                              {freshness && (
                                <small className={freshness.stale ? "task-stale-warning" : "task-update-age"}>
                                  {freshness.text}
                                </small>
                              )}
                            </span>
                            <div>
                              {task.state === "running" && task.pending > 0 && (
                                <>
                                  <button
                                    className="button-quiet"
                                    disabled={actionKey === taskKey}
                                    onClick={() => void runAction(
                                      taskKey,
                                      () => taskPrioritize(project.pid, task.kind),
                                      zh ? "已将剩余批次移到队列前面。" : "Moved remaining batches to the front.",
                                    )}
                                  >
                                    {zh ? "优先处理" : "Prioritize"}
                                  </button>
                                  <button
                                    className="button-quiet"
                                    disabled={actionKey === taskKey}
                                    onClick={() => void runAction(
                                      taskKey,
                                      () => taskPause(project.pid, task.kind),
                                      zh
                                        ? "任务已暂停；正在执行的请求完成后会安全保存。"
                                        : "Paused; in-flight requests will be saved when they finish.",
                                    )}
                                  >
                                    {zh ? "暂停" : "Pause"}
                                  </button>
                                </>
                              )}
                              {task.state === "paused" && (
                                <button
                                  className="button-quiet"
                                  disabled={actionKey === taskKey}
                                  onClick={() => void runAction(
                                    taskKey,
                                    () => taskResume(project.pid),
                                    zh ? "任务已恢复。" : "Resumed the task.",
                                  )}
                                >
                                  {zh ? "继续" : "Resume"}
                                </button>
                              )}
                              {(task.state === "failed" || task.failed > 0) && (
                                <button
                                  className="button-quiet"
                                  disabled={actionKey === taskKey}
                                  onClick={() => void runAction(
                                    taskKey,
                                    () => taskRetry(project.pid, task.kind),
                                    zh ? "已重新生成失败任务。" : "Rebuilt the failed task.",
                                  )}
                                >
                                  {zh ? "重试" : "Retry"}
                                </button>
                              )}
                            </div>
                            {task.pending > 0 && <progress max={100} value={taskProgress} />}
                            {task.lastError && (
                              <details>
                                <summary>{zh ? "查看错误" : "View error"}</summary>
                                <pre>{task.lastError}</pre>
                              </details>
                            )}
                          </div>
                        );
                      })}
                    </div>
                    {status && status.kinds.length > 0 && <progress max={100} value={progress} />}
                  </>
                ) : !error ? (
                  <small>{zh ? "启动一项处理后，阶段与进度会显示在这里。" : "Stages and progress appear here when processing starts."}</small>
                ) : null}
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
