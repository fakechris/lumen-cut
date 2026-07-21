import { useEffect, useState } from "react";
import {
  asrModelsDownload,
  asrRuntimeInstall,
  asrStatus,
  configShow,
  loadSettings,
  saveSettings,
  settingsExport,
} from "../api";
import type { Lang } from "../i18n";
import type { AsrStatus, Settings } from "../types";
import { PipelineView } from "./PipelineView";

interface Props {
  lang: Lang;
  pid: string | null;
}

const COPY = {
  zh: {
    eyebrow: "偏好设置",
    title: "设置",
    intro: "首次转写请按下方状态引导准备本地引擎和模型；完成后，转写与其他后台任务会按需自动运行。",
    localTitle: "本地转写",
    localDescription: "语音识别在 Mac 本机运行。这里显示真实的运行环境与模型状态。",
    asrModel: "转写模型",
    aligner: "字词对齐模型",
    runtime: "转写引擎",
    installed: "已就绪",
    missing: "未就绪",
    modelCached: "模型已下载",
    modelMissing: "模型尚未完整下载",
    installRuntime: "安装转写引擎",
    installingRuntime: "正在安装转写引擎…",
    downloadModels: "下载所需模型",
    downloadingModels: "正在下载模型（约 3 GB）…",
    refreshStatus: "重新检查",
    localHint: "首次准备需要联网。安装和下载都在后台执行，界面不会被阻塞。",
    agentTitle: "AI 功能",
    agentDescription: "用于翻译、润色、章节和 B-roll 建议。基础转写与字幕导出不需要配置。",
    automatic: "无需手动启动 Pipeline 或服务器。保存服务地址和模型后，使用相关功能时会自动启动后台任务。",
    endpoint: "服务地址",
    endpointPlaceholder: "例如 https://api.openai.com/v1/chat/completions",
    apiKey: "API Key（本地服务可留空）",
    model: "模型",
    modelPlaceholder: "例如 gpt-4.1-mini",
    workers: "并行任务数",
    save: "保存设置",
    saving: "正在保存…",
    saved: "已保存",
    advanced: "高级诊断",
    advancedHint: "面向开发和故障排查，普通剪辑流程无需进入这里。",
    error: "设置没有保存",
    incomplete: "若要启用 AI 功能，请同时填写服务地址和模型；也可以全部留空。",
    invalidEndpoint: "服务地址需要是完整的 http:// 或 https:// URL。",
  },
  en: {
    eyebrow: "Preferences",
    title: "Settings",
    intro: "Before the first transcript, use the status below to prepare the local runtime and models. Later jobs start automatically.",
    localTitle: "Local transcription",
    localDescription: "Speech recognition runs on this Mac. This reports the real runtime and model state.",
    asrModel: "Transcription model",
    aligner: "Word alignment model",
    runtime: "Transcription runtime",
    installed: "Ready",
    missing: "Not ready",
    modelCached: "Model downloaded",
    modelMissing: "Model download is missing or incomplete",
    installRuntime: "Install transcription runtime",
    installingRuntime: "Installing transcription runtime…",
    downloadModels: "Download required models",
    downloadingModels: "Downloading models (about 3 GB)…",
    refreshStatus: "Check again",
    localHint: "First-time setup needs a network connection. Installation and downloads run in the background.",
    agentTitle: "AI features",
    agentDescription: "Used for translation, polish, chapters, and B-roll suggestions. Basic transcription and subtitle export need no configuration.",
    automatic: "You never need to start a Pipeline or server manually. Save an endpoint and model; the background worker starts when a feature needs it.",
    endpoint: "Endpoint",
    endpointPlaceholder: "e.g. https://api.openai.com/v1/chat/completions",
    apiKey: "API key (optional for local services)",
    model: "Model",
    modelPlaceholder: "e.g. gpt-4.1-mini",
    workers: "Concurrent tasks",
    save: "Save settings",
    saving: "Saving…",
    saved: "Saved",
    advanced: "Advanced diagnostics",
    advancedHint: "For development and troubleshooting. Normal editing does not require this area.",
    error: "Settings were not saved",
    incomplete: "To enable AI features, provide both an endpoint and model, or leave both empty.",
    invalidEndpoint: "The endpoint must be a complete http:// or https:// URL.",
  },
} as const;

export function SettingsView({ lang, pid }: Props) {
  const c = COPY[lang];
  const [settings, setSettings] = useState<Settings>(() => loadSettings());
  const [state, setState] = useState<"idle" | "saving" | "saved" | "error">("idle");
  const [message, setMessage] = useState<string | null>(null);
  const [asr, setAsr] = useState<AsrStatus | null>(null);
  const [asrAction, setAsrAction] = useState<"install" | "download" | "check" | null>("check");

  useEffect(() => {
    let disposed = false;
    void Promise.all([configShow(), asrStatus()])
      .then(([config, status]) => {
        if (disposed) return;
        setSettings({
          asrModel: config.asrModel,
          asrAligner: config.asrAligner,
          diarizeModel: config.diarizeModel,
          llmEndpoint: config.llmEndpoint,
          llmApiKey: config.llmApiKey,
          llmModel: config.llmModel,
          workerCount: config.workerCount,
        });
        setAsr(status);
      })
      .catch((error) => {
        if (!disposed) setMessage(String(error));
      })
      .finally(() => {
        if (!disposed) setAsrAction(null);
      });
    return () => {
      disposed = true;
    };
  }, []);

  const update = <K extends keyof Settings>(key: K, value: Settings[K]) =>
    setSettings((previous) => ({ ...previous, [key]: value }));

  const save = async () => {
    setState("saving");
    setMessage(null);
    try {
      const normalized = {
        ...settings,
        llmEndpoint: settings.llmEndpoint.trim(),
        llmApiKey: settings.llmApiKey.trim(),
        llmModel: settings.llmModel.trim(),
        workerCount: Math.max(1, Math.min(8, Math.round(settings.workerCount || 1))),
      };
      if (normalized.llmEndpoint && !normalized.llmModel) {
        throw new Error(c.incomplete);
      }
      if (normalized.llmEndpoint) {
        let protocol = "";
        try {
          protocol = new URL(normalized.llmEndpoint).protocol;
        } catch {
          throw new Error(c.invalidEndpoint);
        }
        if (!["http:", "https:"].includes(protocol)) {
          throw new Error(c.invalidEndpoint);
        }
      }
      setSettings(normalized);
      saveSettings(normalized);
      await settingsExport(normalized);
      setAsr(await asrStatus());
      setState("saved");
      window.setTimeout(() => setState("idle"), 1800);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
      setState("error");
    }
  };

  const installAsr = async () => {
    setAsrAction("install");
    setMessage(null);
    try {
      setAsr(await asrRuntimeInstall());
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setAsrAction(null);
    }
  };

  const downloadAsrModels = async () => {
    setAsrAction("download");
    setMessage(null);
    try {
      saveSettings(settings);
      await settingsExport(settings);
      setAsr(await asrModelsDownload());
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setAsrAction(null);
    }
  };

  const refreshAsr = async () => {
    setAsrAction("check");
    setMessage(null);
    try {
      setAsr(await asrStatus());
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setAsrAction(null);
    }
  };

  const selectAsrModel = async (asrModel: string) => {
    const next = { ...settings, asrModel };
    setSettings(next);
    setAsrAction("check");
    setMessage(null);
    try {
      saveSettings(next);
      await settingsExport(next);
      setAsr(await asrStatus());
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setAsrAction(null);
    }
  };

  return (
    <section className="settings-view">
      <header className="page-header">
        <p className="eyebrow">{c.eyebrow}</p>
        <h1>{c.title}</h1>
        <p>{c.intro}</p>
      </header>

      <section className="settings-section" aria-labelledby="local-asr-title">
        <div className="settings-heading">
          <h2 id="local-asr-title">{c.localTitle}</h2>
          <p>{c.localDescription}</p>
          <p className="settings-automation-note">{c.localHint}</p>
        </div>
        <div className="settings-form local-asr-settings">
          <label>
            <span>{c.asrModel}</span>
            <select
              disabled={asrAction !== null}
              value={settings.asrModel}
              onChange={(event) => void selectAsrModel(event.target.value)}
            >
              <option value="Qwen/Qwen3-ASR-0.6B">Qwen3-ASR 0.6B · {lang === "zh" ? "推荐" : "recommended"}</option>
              <option value="Qwen/Qwen3-ASR-1.7B">Qwen3-ASR 1.7B · {lang === "zh" ? "更高精度" : "higher accuracy"}</option>
            </select>
          </label>
          <label>
            <span>{c.aligner}</span>
            <input value={settings.asrAligner} readOnly />
          </label>

          <div className="asr-health" aria-live="polite">
            <div>
              <span className={asr?.runtimeReady ? "status-dot ready" : "status-dot"} />
              <strong>{c.runtime}</strong>
              <small>{asr?.runtimeReady ? `${c.installed} · ${asr.runtimeDetail}` : c.missing}</small>
            </div>
            <div>
              <span className={asr?.modelCached ? "status-dot ready" : "status-dot"} />
              <strong>{settings.asrModel}</strong>
              <small>{asr?.modelCached ? c.modelCached : c.modelMissing}</small>
            </div>
            <div>
              <span className={asr?.alignerCached ? "status-dot ready" : "status-dot"} />
              <strong>{settings.asrAligner}</strong>
              <small>{asr?.alignerCached ? c.modelCached : c.modelMissing}</small>
            </div>
          </div>

          <div className="settings-save asr-actions">
            {!asr?.runtimeReady && (
              <button className="button-primary" disabled={asrAction !== null} onClick={installAsr}>
                {asrAction === "install" ? c.installingRuntime : c.installRuntime}
              </button>
            )}
            {asr?.runtimeReady && (!asr.modelCached || !asr.alignerCached) && (
              <button className="button-primary" disabled={asrAction !== null} onClick={downloadAsrModels}>
                {asrAction === "download" ? c.downloadingModels : c.downloadModels}
              </button>
            )}
            <button className="button-quiet" disabled={asrAction !== null} onClick={refreshAsr}>
              {c.refreshStatus}
            </button>
          </div>
          {message && (
            <div className="notice error-notice" role="alert">
              <span>{message}</span>
            </div>
          )}
        </div>
      </section>

      <section className="settings-section" aria-labelledby="agent-settings-title">
        <div className="settings-heading">
          <h2 id="agent-settings-title">{c.agentTitle}</h2>
          <p>{c.agentDescription}</p>
          <p className="settings-automation-note">{c.automatic}</p>
        </div>
        <div className="settings-form">
          <label>
            <span>{c.endpoint}</span>
            <input
              placeholder={c.endpointPlaceholder}
              value={settings.llmEndpoint}
              onChange={(event) => update("llmEndpoint", event.target.value)}
            />
          </label>
          <label>
            <span>{c.apiKey}</span>
            <input
              autoComplete="off"
              type="password"
              value={settings.llmApiKey}
              onChange={(event) => update("llmApiKey", event.target.value)}
            />
          </label>
          <div className="settings-split">
            <label>
              <span>{c.model}</span>
              <input
                placeholder={c.modelPlaceholder}
                value={settings.llmModel}
                onChange={(event) => update("llmModel", event.target.value)}
              />
            </label>
            <label>
              <span>{c.workers}</span>
              <input
                max={8}
                min={1}
                type="number"
                value={settings.workerCount}
                onChange={(event) => update("workerCount", Number(event.target.value))}
              />
            </label>
          </div>
          <div className="settings-save">
            <button
              className="button-primary"
              disabled={state === "saving"}
              onClick={save}
            >
              {state === "saving" ? c.saving : c.save}
            </button>
            {state === "saved" && <span className="save-confirmation">{c.saved}</span>}
          </div>
          {state === "error" && (
            <div className="notice error-notice" role="alert">
              <strong>{c.error}</strong>
              <span>{message}</span>
            </div>
          )}
        </div>
      </section>

      <details className="advanced-diagnostics">
        <summary>
          <span>
            <strong>{c.advanced}</strong>
            <small>{c.advancedHint}</small>
          </span>
        </summary>
        <PipelineView embedded lang={lang} pid={pid} />
      </details>
    </section>
  );
}
