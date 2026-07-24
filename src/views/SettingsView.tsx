import { useEffect, useRef, useState } from "react";
import {
  asrModelsList,
  asrStatus,
  configShow,
  llmModelsList,
  loadSettings,
  saveSettings,
  setupJobCancel,
  setupJobStart,
  setupJobStatus,
  settingsExport,
} from "../api";
import { PipelineFreshness } from "../components/PipelineFreshness";
import type { Lang } from "../i18n";
import {
  getLlmProvider,
  inferLlmProvider,
  LLM_PROVIDER_PRESETS,
} from "../llmProviders";
import type { AsrStatus, Settings, SetupJobStatus } from "../types";
import { PipelineView } from "./PipelineView";

interface Props {
  lang: Lang;
}

const COPY = {
  zh: {
    eyebrow: "偏好设置",
    title: "设置",
    intro: "首次转写先选择本机或云端引擎，并按状态引导完成准备；之后所有后台任务都会按需自动运行。",
    localTitle: "转写引擎与说话人",
    localDescription: "可选择本机 MLX 或兼容 OpenAI 音频接口的云端转写。说话人识别始终在本机独立运行。",
    asrEngine: "转写引擎",
    localEngine: "本机 MLX · 隐私优先",
    cloudEngine: "OpenAI Compatible · 云端",
    cloudEndpoint: "转写服务地址",
    cloudModel: "转写模型",
    cloudKey: "转写 API Key",
    cloudReady: "云端转写已配置",
    cloudIncomplete: "请补全转写服务地址、模型和 API Key",
    cloudPrivacy: "音频会从这台 Mac 直接上传到所选服务。长音频会按 10 分钟分片，任务可取消；服务必须返回真实词级时码。",
    storedSecret: "已保存；留空会继续使用原 Key",
    asrModel: "转写模型",
    aligner: "字词对齐模型",
    runtime: "转写引擎",
    diarizeModel: "说话人模型",
    diarizeRuntime: "说话人识别引擎",
    hfToken: "Hugging Face 授权",
    tokenReady: "已检测到访问令牌",
    tokenMissing: "未检测到 HF_TOKEN",
    installed: "已就绪",
    missing: "未就绪",
    modelCached: "模型已下载",
    modelMissing: "模型尚未完整下载",
    installRuntime: "安装或修复转写引擎",
    installingRuntime: "正在安装转写引擎…",
    downloadModels: "下载转写模型",
    downloadingModels: "正在下载转写模型（可能需要数 GB）…",
    installSpeakerRuntime: "启用说话人识别",
    installingSpeakerRuntime: "正在安装说话人识别引擎…",
    downloadSpeakerModel: "下载说话人模型",
    downloadingSpeakerModel: "正在下载说话人模型…",
    refreshStatus: "重新检查",
    localHint: "本机与云端转写二选一；无需启动服务器。说话人识别是可选能力，需要另行接受模型条款并设置 HF_TOKEN。",
    agentTitle: "AI 功能",
    agentDescription: "用于翻译、润色、章节和 B-roll 建议。基础转写与字幕导出不需要配置。",
    automatic: "无需手动启动 Pipeline 或服务器。保存服务地址和模型后，使用相关功能时会自动启动后台任务。",
    provider: "模型服务商",
    providerNone: "暂不启用 AI 功能",
    providerCustom: "OpenAI Compatible · 自定义服务",
    providerRemote: "云端服务",
    providerLocal: "本机服务",
    providerPrompt: "选择服务商会自动填写地址和常用模型；其他兼容服务请选择 OpenAI Compatible。",
    providerReady: "必填项已完整；保存后将在首次 AI 任务时连接",
    providerDisabled: "AI 功能未启用；转写、说话人和导出仍可正常使用",
    providerIncomplete: "还需要补全下方必填项",
    directConnection: "请求会从这台 Mac 直接发送到所选服务，lumen-cut 不会中转。Key 只写入本机配置文件。",
    endpoint: "服务地址",
    endpointPlaceholder: "例如 https://api.openai.com/v1/chat/completions",
    endpointAdvanced: "高级：查看或覆盖服务地址",
    apiKey: "API Key",
    apiKeyOptional: "API Key（可选）",
    model: "模型",
    modelPlaceholder: "例如 gpt-4.1-mini",
    modelHint: "可以从常用模型中选择，也可以直接输入服务商支持的其他模型 ID。",
    refreshModels: "获取最新模型",
    refreshingModels: "正在获取…",
    modelsNeedKey: "先填写 API Key，再获取这个账号可用的模型。",
    modelsLoaded: (count: number) => `已从服务商获取 ${count} 个模型`,
    workers: "并行任务数",
    save: "保存设置",
    saving: "正在保存…",
    saved: "已保存",
    advanced: "高级诊断",
    advancedHint: "面向开发和故障排查，普通剪辑流程无需进入这里。",
    error: "设置没有保存",
    incomplete: "若要启用 AI 功能，请同时填写服务地址和模型；也可以全部留空。",
    missingApiKey: "这个服务商需要 API Key。请填写后再保存。",
    invalidEndpoint: "服务地址需要是完整的 http:// 或 https:// URL。",
  },
  en: {
    eyebrow: "Preferences",
    title: "Settings",
    intro: "Before the first transcript, choose a local or cloud engine and follow its readiness status. Later jobs start automatically.",
    localTitle: "Transcription engine and speakers",
    localDescription: "Choose local MLX or an OpenAI-compatible audio endpoint. Speaker identification remains a separate local capability.",
    asrEngine: "Transcription engine",
    localEngine: "Local MLX · privacy first",
    cloudEngine: "OpenAI Compatible · cloud",
    cloudEndpoint: "Transcription endpoint",
    cloudModel: "Transcription model",
    cloudKey: "Transcription API key",
    cloudReady: "Cloud transcription is configured",
    cloudIncomplete: "Complete the transcription endpoint, model, and API key",
    cloudPrivacy: "Audio is uploaded directly from this Mac to the selected service. Long audio is split into 10-minute chunks and remains cancellable; the service must return real word timestamps.",
    storedSecret: "Already saved; leave blank to keep the existing key",
    asrModel: "Transcription model",
    aligner: "Word alignment model",
    runtime: "Transcription runtime",
    diarizeModel: "Speaker model",
    diarizeRuntime: "Speaker runtime",
    hfToken: "Hugging Face access",
    tokenReady: "Access token detected",
    tokenMissing: "HF_TOKEN is not set",
    installed: "Ready",
    missing: "Not ready",
    modelCached: "Model downloaded",
    modelMissing: "Model download is missing or incomplete",
    installRuntime: "Install or repair transcription runtime",
    installingRuntime: "Installing transcription runtime…",
    downloadModels: "Download transcription models",
    downloadingModels: "Downloading transcription models (several GB may be needed)…",
    installSpeakerRuntime: "Enable speaker identification",
    installingSpeakerRuntime: "Installing speaker identification runtime…",
    downloadSpeakerModel: "Download speaker model",
    downloadingSpeakerModel: "Downloading speaker model…",
    refreshStatus: "Check again",
    localHint: "Choose either local or cloud transcription; no server startup is needed. Speaker identification is optional and separately requires accepting model terms and setting HF_TOKEN.",
    agentTitle: "AI features",
    agentDescription: "Used for translation, polish, chapters, and B-roll suggestions. Basic transcription and subtitle export need no configuration.",
    automatic: "You never need to start a Pipeline or server manually. Save an endpoint and model; the background worker starts when a feature needs it.",
    provider: "Model provider",
    providerNone: "Do not enable AI features yet",
    providerCustom: "OpenAI Compatible · custom service",
    providerRemote: "Cloud providers",
    providerLocal: "Local providers",
    providerPrompt: "Provider presets fill in the endpoint and a common model. Choose OpenAI Compatible for any other service.",
    providerReady: "Required fields complete; the first AI task will connect after saving",
    providerDisabled: "AI features are off; transcription, speakers, and export still work",
    providerIncomplete: "Complete the required fields below",
    directConnection: "Requests go directly from this Mac to the selected provider; lumen-cut does not proxy them. The key is stored only in the local config file.",
    endpoint: "Endpoint",
    endpointPlaceholder: "e.g. https://api.openai.com/v1/chat/completions",
    endpointAdvanced: "Advanced: view or override endpoint",
    apiKey: "API key",
    apiKeyOptional: "API key (optional)",
    model: "Model",
    modelPlaceholder: "e.g. gpt-4.1-mini",
    modelHint: "Choose a common model or type any other model ID supported by the provider.",
    refreshModels: "Fetch latest models",
    refreshingModels: "Fetching…",
    modelsNeedKey: "Add the API key to fetch models available to this account.",
    modelsLoaded: (count: number) => `Fetched ${count} models from the provider`,
    workers: "Concurrent tasks",
    save: "Save settings",
    saving: "Saving…",
    saved: "Saved",
    advanced: "Advanced diagnostics",
    advancedHint: "For development and troubleshooting. Normal editing does not require this area.",
    error: "Settings were not saved",
    incomplete: "To enable AI features, provide both an endpoint and model, or leave both empty.",
    missingApiKey: "This provider requires an API key. Add it before saving.",
    invalidEndpoint: "The endpoint must be a complete http:// or https:// URL.",
  },
} as const;

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${value >= 100 ? value.toFixed(0) : value.toFixed(1)} ${units[index]}`;
}

export function setupTransferLabel(job: SetupJobStatus, lang: Lang) {
  const parts: string[] = [];
  if (job.current !== null && job.current !== undefined) {
    if (job.unit === "bytes") {
      parts.push(
        job.total
          ? `${formatBytes(job.current)} / ${formatBytes(job.total)}`
          : formatBytes(job.current),
      );
    } else if (job.unit === "files") {
      parts.push(
        job.total
          ? lang === "zh"
            ? `${job.current} / ${job.total} 个文件`
            : `${job.current} / ${job.total} files`
          : lang === "zh"
            ? `${job.current} 个文件`
            : `${job.current} files`,
      );
    }
  }
  if (job.bytesPerSecond) {
    parts.push(`${formatBytes(job.bytesPerSecond)}/s`);
  }
  if (job.startedAt) {
    const seconds = Math.max(0, Math.round(Date.now() / 1000 - job.startedAt));
    parts.push(
      seconds < 60
        ? lang === "zh" ? `${seconds} 秒` : `${seconds} sec`
        : lang === "zh"
          ? `${Math.floor(seconds / 60)} 分 ${seconds % 60} 秒`
          : `${Math.floor(seconds / 60)}m ${seconds % 60}s`,
    );
  }
  return parts.join(" · ");
}

export function SettingsView({ lang }: Props) {
  const c = COPY[lang];
  const [settings, setSettings] = useState<Settings>(() => loadSettings());
  const [providerId, setProviderId] = useState(() => inferLlmProvider(settings.llmEndpoint));
  const [state, setState] = useState<"idle" | "saving" | "saved" | "error">("idle");
  const [message, setMessage] = useState<string | null>(null);
  const [asr, setAsr] = useState<AsrStatus | null>(null);
  const [asrAction, setAsrAction] = useState<"install" | "download" | "install-speakers" | "download-speakers" | "check" | null>("check");
  const [setupJob, setSetupJob] = useState<SetupJobStatus | null>(null);
  const [remoteModels, setRemoteModels] = useState<string[]>([]);
  const [modelCatalogState, setModelCatalogState] = useState<"idle" | "loading" | "loaded" | "error">("idle");
  const [modelCatalogMessage, setModelCatalogMessage] = useState<string | null>(null);
  const modelCatalogRequest = useRef(0);
  const [llmApiKeyStored, setLlmApiKeyStored] = useState(false);
  const [asrCloudApiKeyStored, setAsrCloudApiKeyStored] = useState(false);
  const [cloudAsrModels, setCloudAsrModels] = useState<string[]>([]);
  const [cloudAsrCatalogState, setCloudAsrCatalogState] = useState<"idle" | "loading" | "loaded" | "error">("idle");
  const [cloudAsrCatalogMessage, setCloudAsrCatalogMessage] = useState<string | null>(null);

  const refreshModels = async (source: Settings = settings) => {
    if (!source.llmEndpoint.trim()) return;
    const requestId = ++modelCatalogRequest.current;
    setModelCatalogState("loading");
    setModelCatalogMessage(null);
    try {
      const models = await llmModelsList(source.llmEndpoint.trim(), source.llmApiKey.trim());
      if (modelCatalogRequest.current !== requestId) return;
      setRemoteModels(models);
      setModelCatalogState("loaded");
      setModelCatalogMessage(c.modelsLoaded(models.length));
    } catch (error) {
      if (modelCatalogRequest.current !== requestId) return;
      setRemoteModels([]);
      setModelCatalogState("error");
      setModelCatalogMessage(error instanceof Error ? error.message : String(error));
    }
  };

  const refreshCloudAsrModels = async (source: Settings = settings) => {
    if (!source.asrCloudEndpoint.trim()) return;
    setCloudAsrCatalogState("loading");
    setCloudAsrCatalogMessage(null);
    try {
      const models = await asrModelsList(
        source.asrCloudEndpoint.trim(),
        source.asrCloudApiKey.trim(),
      );
      setCloudAsrModels(models);
      setCloudAsrCatalogState("loaded");
      setCloudAsrCatalogMessage(c.modelsLoaded(models.length));
    } catch (error) {
      setCloudAsrModels([]);
      setCloudAsrCatalogState("error");
      setCloudAsrCatalogMessage(error instanceof Error ? error.message : String(error));
    }
  };

  useEffect(() => {
    let disposed = false;
    void Promise.all([
      configShow(),
      asrStatus(),
      setupJobStatus().catch(() => null),
    ])
      .then(([config, status, setup]) => {
        if (disposed) return;
        const asrEngine = config.asrEngine ?? "local";
        const asrCloudEndpoint = config.asrCloudEndpoint
          ?? "https://api.openai.com/v1/audio/transcriptions";
        const asrCloudModel = config.asrCloudModel ?? "whisper-1";
        setSettings({
          asrEngine,
          asrModel: config.asrModel,
          asrAligner: config.asrAligner,
          asrCloudEndpoint,
          asrCloudApiKey: "",
          asrCloudModel,
          diarizeModel: config.diarizeModel,
          hfToken: "",
          llmEndpoint: config.llmEndpoint,
          llmApiKey: "",
          llmModel: config.llmModel,
          workerCount: config.workerCount,
        });
        setLlmApiKeyStored(config.llmApiKeySet ?? Boolean(config.llmApiKey));
        setAsrCloudApiKeyStored(config.asrCloudApiKeySet ?? Boolean(config.asrCloudApiKey));
        setProviderId(inferLlmProvider(config.llmEndpoint));
        if (config.llmEndpoint.trim()) {
          void refreshModels({
            asrEngine,
            asrModel: config.asrModel,
            asrAligner: config.asrAligner,
            asrCloudEndpoint,
            asrCloudApiKey: "",
            asrCloudModel,
            diarizeModel: config.diarizeModel,
            hfToken: "",
            llmEndpoint: config.llmEndpoint,
            llmApiKey: "",
            llmModel: config.llmModel,
            workerCount: config.workerCount,
          });
        }
        setAsr(status);
        if (setup && (setup.state === "running" || setup.state === "cancelling")) {
          setSetupJob(setup);
          setAsrAction(({
            "asr-runtime": "install",
            "asr-models": "download",
            "speaker-runtime": "install-speakers",
            "speaker-model": "download-speakers",
          } as const)[setup.kind]);
        } else {
          setAsrAction(null);
        }
      })
      .catch((error) => {
        if (!disposed) setMessage(String(error));
      })
    return () => {
      disposed = true;
      modelCatalogRequest.current += 1;
    };
  }, []);

  useEffect(() => {
    if (!setupJob || !["running", "cancelling"].includes(setupJob.state)) return;
    let disposed = false;
    let timer: number | undefined;
    const poll = async () => {
      try {
        const next = await setupJobStatus();
        if (disposed) return;
        if (next.state === "completed") {
          setAsr(await asrStatus());
          if (disposed) return;
          setSetupJob(next);
          setAsrAction(null);
          return;
        }
        if (next.state === "cancelled") {
          setSetupJob(next);
          setAsrAction(null);
          return;
        }
        if (next.state === "failed") {
          setSetupJob(next);
          setMessage(next.error || "Setup failed");
          setAsrAction(null);
          return;
        }
        setSetupJob(next);
        timer = window.setTimeout(poll, 500);
      } catch (error) {
        if (!disposed) {
          setMessage(error instanceof Error ? error.message : String(error));
          setAsrAction(null);
        }
      }
    };
    timer = window.setTimeout(poll, 350);
    return () => {
      disposed = true;
      if (timer) window.clearTimeout(timer);
    };
  }, [setupJob?.state]);

  const update = <K extends keyof Settings>(key: K, value: Settings[K]) => {
    if (key === "llmEndpoint" || key === "llmApiKey") {
      modelCatalogRequest.current += 1;
      setRemoteModels([]);
      setModelCatalogState("idle");
      setModelCatalogMessage(null);
      if (key === "llmEndpoint" || value !== "") setLlmApiKeyStored(false);
    }
    if (key === "asrCloudEndpoint" || key === "asrCloudApiKey") {
      setCloudAsrModels([]);
      setCloudAsrCatalogState("idle");
      setCloudAsrCatalogMessage(null);
      if (key === "asrCloudEndpoint" || value !== "") setAsrCloudApiKeyStored(false);
    }
    setSettings((previous) => ({ ...previous, [key]: value }));
  };

  const selectedProvider = getLlmProvider(providerId);
  const modelOptions = Array.from(new Set([
    ...(selectedProvider?.models ?? []),
    ...remoteModels,
  ]));
  const providerConfigured = Boolean(
    settings.llmEndpoint.trim()
      && settings.llmModel.trim()
      && (!selectedProvider?.needsKey || settings.llmApiKey.trim() || llmApiKeyStored),
  );
  const cloudAsrConfigured = Boolean(
    settings.asrCloudEndpoint?.trim()
      && settings.asrCloudModel?.trim()
      && (settings.asrCloudApiKey?.trim() || asrCloudApiKeyStored),
  );

  const selectProvider = (nextId: string) => {
    const changedProvider = nextId !== providerId;
    setProviderId(nextId);
    setState("idle");
    setMessage(null);
    modelCatalogRequest.current += 1;
    setRemoteModels([]);
    setModelCatalogState("idle");
    setModelCatalogMessage(null);
    if (nextId === "none") {
      setLlmApiKeyStored(false);
      setSettings((previous) => ({
        ...previous,
        llmApiKey: "",
        llmEndpoint: "",
        llmModel: "",
      }));
      return;
    }
    const provider = getLlmProvider(nextId);
    if (!provider) {
      if (changedProvider) {
        setLlmApiKeyStored(false);
        setSettings((previous) => ({ ...previous, llmApiKey: "" }));
      }
      return;
    }
    if (changedProvider) setLlmApiKeyStored(false);
    setSettings((previous) => ({
      ...previous,
      llmApiKey: changedProvider ? "" : previous.llmApiKey,
      llmEndpoint: provider.endpoint,
      llmModel: provider.model,
    }));
  };

  const save = async () => {
    setState("saving");
    setMessage(null);
    try {
      const normalized = {
        ...settings,
        hfToken: settings.hfToken.trim(),
        llmEndpoint: settings.llmEndpoint.trim(),
        llmApiKey: settings.llmApiKey.trim(),
        llmModel: settings.llmModel.trim(),
        workerCount: Math.max(1, Math.min(8, Math.round(settings.workerCount || 1))),
      };
      if (normalized.asrEngine === "openai-compatible" && !cloudAsrConfigured) {
        throw new Error(c.cloudIncomplete);
      }
      if (normalized.llmEndpoint && !normalized.llmModel) {
        throw new Error(c.incomplete);
      }
      if (selectedProvider?.needsKey && !normalized.llmApiKey && !llmApiKeyStored) {
        throw new Error(c.missingApiKey);
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
      if (normalized.asrEngine === "openai-compatible") {
        let protocol = "";
        try {
          protocol = new URL(normalized.asrCloudEndpoint).protocol;
        } catch {
          throw new Error(c.invalidEndpoint);
        }
        if (!["http:", "https:"].includes(protocol)) throw new Error(c.invalidEndpoint);
      }
      setSettings(normalized);
      saveSettings(normalized);
      await settingsExport(normalized);
      setAsr(await asrStatus());
      if (normalized.llmApiKey) setLlmApiKeyStored(true);
      if (normalized.asrCloudApiKey) setAsrCloudApiKeyStored(true);
      setSettings((previous) => ({
        ...previous,
        hfToken: "",
        llmApiKey: "",
        asrCloudApiKey: "",
      }));
      setState("saved");
      window.setTimeout(() => setState("idle"), 1800);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
      setState("error");
    }
  };

  const beginSetup = async (
    kind: SetupJobStatus["kind"],
    action: Exclude<typeof asrAction, "check" | null>,
    persistSettings: boolean,
  ) => {
    setAsrAction(action);
    setMessage(null);
    try {
      if (persistSettings) {
        saveSettings(settings);
        await settingsExport(settings);
      }
      setSetupJob(await setupJobStart(kind));
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
      setAsrAction(null);
    }
  };

  const installAsr = () => beginSetup("asr-runtime", "install", false);

  const downloadAsrModels = () => beginSetup("asr-models", "download", true);

  const installSpeakers = () => beginSetup("speaker-runtime", "install-speakers", false);

  const downloadSpeakerModel = () => beginSetup("speaker-model", "download-speakers", true);

  const cancelSetup = async () => {
    try {
      setSetupJob(await setupJobCancel());
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
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

  const selectDiarizeModel = async (diarizeModel: string) => {
    const next = { ...settings, diarizeModel };
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
            <span>{c.asrEngine}</span>
            <select
              value={settings.asrEngine}
              onChange={(event) => update(
                "asrEngine",
                event.target.value as Settings["asrEngine"],
              )}
            >
              <option value="local">{c.localEngine}</option>
              <option value="openai-compatible">{c.cloudEngine}</option>
            </select>
          </label>
          {settings.asrEngine === "openai-compatible" && (
            <>
              <div className={`provider-configuration-state${cloudAsrConfigured ? " ready" : ""}`} role="status">
                <span className={cloudAsrConfigured ? "status-dot ready" : "status-dot"} />
                <div>
                  <strong>{cloudAsrConfigured ? c.cloudReady : c.cloudIncomplete}</strong>
                  <small>{c.cloudPrivacy}</small>
                </div>
              </div>
              <label>
                <span>{c.cloudKey}</span>
                <input
                  aria-label={c.cloudKey}
                  autoComplete="off"
                  placeholder={asrCloudApiKeyStored ? c.storedSecret : "sk-…"}
                  type="password"
                  value={settings.asrCloudApiKey}
                  onChange={(event) => update("asrCloudApiKey", event.target.value)}
                />
              </label>
              <label className="provider-model-field">
                <span>{c.cloudModel}</span>
                <div className="provider-model-input">
                  <input
                    aria-label={c.cloudModel}
                    list={cloudAsrModels.length ? "asr-cloud-model-options" : undefined}
                    value={settings.asrCloudModel}
                    onChange={(event) => update("asrCloudModel", event.target.value)}
                  />
                  <button
                    className="button-quiet"
                    disabled={cloudAsrCatalogState === "loading"
                      || !settings.asrCloudEndpoint.trim()
                      || (!settings.asrCloudApiKey.trim() && !asrCloudApiKeyStored)}
                    type="button"
                    onClick={() => void refreshCloudAsrModels()}
                  >
                    {cloudAsrCatalogState === "loading" ? c.refreshingModels : c.refreshModels}
                  </button>
                </div>
                {cloudAsrModels.length ? (
                  <datalist id="asr-cloud-model-options">
                    {cloudAsrModels.map((model) => <option key={model} value={model} />)}
                  </datalist>
                ) : null}
                <small className="field-hint">
                  {lang === "zh"
                    ? "默认 whisper-1；当前必须支持 verbose_json 和 word timestamps，不能用推测时码代替。"
                    : "Defaults to whisper-1. The model must support verbose_json word timestamps; inferred timing is never substituted."}
                </small>
                {cloudAsrCatalogMessage ? (
                  <small className={`field-hint model-catalog-message ${cloudAsrCatalogState}`} role={cloudAsrCatalogState === "error" ? "alert" : "status"}>
                    {cloudAsrCatalogMessage}
                  </small>
                ) : null}
              </label>
              <label>
                <span>{c.cloudEndpoint}</span>
                <input
                  aria-label={c.cloudEndpoint}
                  value={settings.asrCloudEndpoint}
                  onChange={(event) => update("asrCloudEndpoint", event.target.value)}
                />
              </label>
            </>
          )}
          {settings.asrEngine === "local" && (
            <>
          <label>
            <span>{c.asrModel}</span>
            <select
              disabled={asrAction !== null}
              value={settings.asrModel}
              onChange={(event) => void selectAsrModel(event.target.value)}
            >
              <option value="mlx-community/Qwen3-ASR-0.6B-8bit">Qwen3-ASR 0.6B · {lang === "zh" ? "低内存推荐" : "memory-efficient"}</option>
              <option value="mlx-community/Qwen3-ASR-1.7B-8bit">Qwen3-ASR 1.7B · {lang === "zh" ? "更高精度" : "higher accuracy"}</option>
              <option value="Qwen/Qwen3-ASR-0.6B">Qwen3-ASR 0.6B FP16 · {lang === "zh" ? "高内存兼容" : "high-memory compatibility"}</option>
              <option value="Qwen/Qwen3-ASR-1.7B">Qwen3-ASR 1.7B FP16 · {lang === "zh" ? "高内存" : "high memory"}</option>
            </select>
          </label>
          <label>
            <span>{c.aligner}</span>
            <input value={settings.asrAligner} readOnly />
          </label>
            </>
          )}
          <label>
            <span>{c.diarizeModel}</span>
            <select
              disabled={asrAction !== null}
              value={settings.diarizeModel}
              onChange={(event) => void selectDiarizeModel(event.target.value)}
            >
              <option value="pyannote/speaker-diarization-3.1">Speaker Diarization 3.1 · {lang === "zh" ? "已验证" : "verified"}</option>
            </select>
          </label>
          <label>
            <span>{c.hfToken}</span>
            <input
              autoComplete="off"
              placeholder={asr?.huggingFaceTokenSet ? c.storedSecret : "hf_…"}
              type="password"
              value={settings.hfToken}
              onChange={(event) => update("hfToken", event.target.value)}
            />
          </label>

          <div className="asr-health" aria-live="polite">
            {settings.asrEngine === "local" && (
              <>
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
              </>
            )}
            <div>
              <span className={asr?.diarizeRuntimeReady ? "status-dot ready" : "status-dot"} />
              <strong>{c.diarizeRuntime}</strong>
              <small>{asr?.diarizeRuntimeReady ? `${c.installed} · ${asr.diarizeRuntimeDetail}` : asr?.diarizeRuntimeDetail || c.missing}</small>
            </div>
            <div>
              <span className={asr?.diarizeModelCached ? "status-dot ready" : "status-dot"} />
              <strong>{settings.diarizeModel}</strong>
              <small>{asr?.diarizeModelCached ? c.modelCached : c.modelMissing}</small>
            </div>
            <div>
              <span className={asr?.huggingFaceTokenSet ? "status-dot ready" : "status-dot"} />
              <strong>{c.hfToken}</strong>
              <small>{asr?.huggingFaceTokenSet ? c.tokenReady : c.tokenMissing}</small>
            </div>
          </div>

          <div className="settings-save asr-actions">
            {settings.asrEngine === "local" && !asr?.runtimeReady && (
              <button className="button-primary" disabled={asrAction !== null} onClick={installAsr}>
                {asrAction === "install" ? c.installingRuntime : c.installRuntime}
              </button>
            )}
            {settings.asrEngine === "local" && asr?.runtimeReady && (!asr.modelCached || !asr.alignerCached) && (
              <button
                className="button-primary"
                disabled={asrAction !== null}
                onClick={downloadAsrModels}
              >
                {asrAction === "download" ? c.downloadingModels : c.downloadModels}
              </button>
            )}
            {!asr?.diarizeRuntimeReady && (
              <button className="button-quiet" disabled={asrAction !== null} onClick={installSpeakers}>
                {asrAction === "install-speakers" ? c.installingSpeakerRuntime : c.installSpeakerRuntime}
              </button>
            )}
            {asr?.diarizeRuntimeReady && !asr.diarizeModelCached && (
              <button
                className="button-quiet"
                disabled={asrAction !== null || (!settings.hfToken.trim() && !asr.huggingFaceTokenSet)}
                onClick={downloadSpeakerModel}
              >
                {asrAction === "download-speakers" ? c.downloadingSpeakerModel : c.downloadSpeakerModel}
              </button>
            )}
            <button className="button-quiet" disabled={asrAction !== null} onClick={refreshAsr}>
              {c.refreshStatus}
            </button>
          </div>
          {setupJob && ["running", "cancelling"].includes(setupJob.state) && (
            <div className="setup-job-progress" role="status" aria-live="polite">
              <div>
                <strong>
                  {setupJob.phase === "waiting"
                    ? lang === "zh" ? "正在等待计算资源" : "Waiting for compute capacity"
                    : setupJob.phase === "preparing"
                      ? lang === "zh" ? "正在准备本地环境" : "Preparing the local environment"
                    : setupJob.phase === "downloading"
                      ? lang === "zh" ? "正在下载模型文件" : "Downloading model files"
                      : setupJob.phase === "installing"
                        ? lang === "zh" ? "正在安装本地运行环境" : "Installing local runtime"
                        : setupJob.phase === "verifying"
                          ? lang === "zh" ? "正在验证安装完整性" : "Verifying setup integrity"
                        : lang === "zh" ? "正在安全停止" : "Stopping safely"}
                </strong>
                <span>
                  {setupJob.kind}
                  {setupJob.progress !== null && setupJob.progress !== undefined
                    ? ` · ${setupJob.progress}%`
                    : ""}
                </span>
              </div>
              {setupJob.progress !== null && setupJob.progress !== undefined ? (
                <progress
                  aria-label={lang === "zh" ? "环境准备进度" : "Setup progress"}
                  max={100}
                  value={setupJob.progress}
                />
              ) : (
                <progress aria-label={lang === "zh" ? "环境准备进度" : "Setup progress"} />
              )}
              {setupJob.detail && <small className="setup-job-detail">{setupJob.detail}</small>}
              {setupTransferLabel(setupJob, lang) && (
                <small className="setup-job-transfer">{setupTransferLabel(setupJob, lang)}</small>
              )}
              <small>
                {lang === "zh"
                  ? "百分比表示可验证的安装阶段和模型文件进度；有可靠字节总量时会同时显示大小与实时速度。"
                  : "Percent reflects verified setup stages and model-file progress. Size and live throughput appear when trustworthy byte totals are available."}
              </small>
              <PipelineFreshness
                state={setupJob.state}
                phase={setupJob.phase}
                updatedAt={setupJob.updatedAt}
                lang={lang}
              />
              <button
                className="button-quiet"
                disabled={setupJob.state === "cancelling"}
                onClick={() => void cancelSetup()}
              >
                {setupJob.state === "cancelling"
                  ? lang === "zh" ? "正在停止…" : "Stopping…"
                  : lang === "zh" ? "取消任务" : "Cancel setup"}
              </button>
            </div>
          )}
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
        <div className="settings-form ai-provider-settings">
          <label>
            <span>{c.provider}</span>
            <select
              aria-label={c.provider}
              value={providerId}
              onChange={(event) => selectProvider(event.target.value)}
            >
              <option value="none">{c.providerNone}</option>
              <option value="custom">{c.providerCustom}</option>
              <optgroup label={c.providerRemote}>
                {LLM_PROVIDER_PRESETS.filter((provider) => !provider.local && !provider.custom).map((provider) => (
                  <option key={provider.id} value={provider.id}>{provider.label}</option>
                ))}
              </optgroup>
              <optgroup label={c.providerLocal}>
                {LLM_PROVIDER_PRESETS.filter((provider) => provider.local).map((provider) => (
                  <option key={provider.id} value={provider.id}>{provider.label}</option>
                ))}
              </optgroup>
            </select>
            <small className="field-hint">{c.providerPrompt}</small>
          </label>

          <div className={`provider-configuration-state${providerConfigured ? " ready" : ""}`} role="status">
            <span className={providerConfigured ? "status-dot ready" : providerId === "none" ? "status-dot muted" : "status-dot"} />
            <div>
              <strong>
                {providerId === "none"
                  ? c.providerDisabled
                  : providerConfigured
                    ? c.providerReady
                    : c.providerIncomplete}
              </strong>
              {providerId !== "none" && <small>{c.directConnection}</small>}
            </div>
          </div>

          {providerId !== "none" && (
            <>
              {!selectedProvider?.local && (
                <label>
                  <span>{selectedProvider?.needsKey ? c.apiKey : c.apiKeyOptional}</span>
                  <input
                    aria-label={selectedProvider?.needsKey ? c.apiKey : c.apiKeyOptional}
                    aria-required={selectedProvider?.needsKey || undefined}
                    autoComplete="off"
                    placeholder={llmApiKeyStored ? c.storedSecret : selectedProvider?.needsKey ? "sk-…" : undefined}
                    type="password"
                    value={settings.llmApiKey}
                    onChange={(event) => update("llmApiKey", event.target.value)}
                  />
                </label>
              )}

              <label className="provider-model-field">
                <span>{c.model}</span>
                <div className="provider-model-input">
                  <input
                    aria-label={c.model}
                    list={modelOptions.length ? "llm-model-options" : undefined}
                    placeholder={c.modelPlaceholder}
                    value={settings.llmModel}
                    onChange={(event) => update("llmModel", event.target.value)}
                  />
                  <button
                    className="button-quiet"
                    disabled={modelCatalogState === "loading"
                      || !settings.llmEndpoint.trim()
                      || Boolean(selectedProvider?.needsKey && !settings.llmApiKey.trim() && !llmApiKeyStored)}
                    type="button"
                    onClick={() => void refreshModels()}
                  >
                    {modelCatalogState === "loading" ? c.refreshingModels : c.refreshModels}
                  </button>
                </div>
                {modelOptions.length ? (
                  <datalist id="llm-model-options">
                    {modelOptions.map((model) => <option key={model} value={model} />)}
                  </datalist>
                ) : null}
                <small className="field-hint">{c.modelHint}</small>
                {selectedProvider?.needsKey && !settings.llmApiKey.trim() && !llmApiKeyStored ? (
                  <small className="field-hint">{c.modelsNeedKey}</small>
                ) : null}
                {modelCatalogMessage ? (
                  <small className={`field-hint model-catalog-message ${modelCatalogState}`} role={modelCatalogState === "error" ? "alert" : "status"}>
                    {modelCatalogMessage}
                  </small>
                ) : null}
              </label>

              {selectedProvider?.custom ? (
                <label>
                  <span>{c.endpoint}</span>
                  <input
                    aria-label={c.endpoint}
                    placeholder={c.endpointPlaceholder}
                    value={settings.llmEndpoint}
                    onChange={(event) => update("llmEndpoint", event.target.value)}
                  />
                </label>
              ) : (
                <details className="provider-endpoint-details">
                  <summary>{c.endpointAdvanced}</summary>
                  <label>
                    <span>{c.endpoint}</span>
                    <input
                      aria-label={c.endpoint}
                      placeholder={c.endpointPlaceholder}
                      value={settings.llmEndpoint}
                      onChange={(event) => update("llmEndpoint", event.target.value)}
                    />
                  </label>
                </details>
              )}

              <label>
                <span>{c.workers}</span>
                <input
                  aria-label={c.workers}
                  max={8}
                  min={1}
                  type="number"
                  value={settings.workerCount}
                  onChange={(event) => update("workerCount", Number(event.target.value))}
                />
              </label>
            </>
          )}
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
        <PipelineView lang={lang} />
      </details>
    </section>
  );
}
