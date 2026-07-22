import { useEffect, useState } from "react";
import {
  asrStatus,
  configShow,
  loadSettings,
  saveSettings,
  setupJobCancel,
  setupJobStart,
  setupJobStatus,
  settingsExport,
} from "../api";
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
  pid: string | null;
}

const COPY = {
  zh: {
    eyebrow: "偏好设置",
    title: "设置",
    intro: "首次转写请按下方状态引导准备本地引擎和模型；完成后，转写与其他后台任务会按需自动运行。",
    localTitle: "本地模型与运行环境",
    localDescription: "转写、词级对齐和说话人识别都在 Mac 本机运行。这里显示每一项真实状态。",
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
    localHint: "先准备转写即可开始工作；说话人识别是可选能力，需要另行接受模型条款并设置 HF_TOKEN。安装和下载都在后台执行。",
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
    intro: "Before the first transcript, use the status below to prepare the local runtime and models. Later jobs start automatically.",
    localTitle: "Local models and runtimes",
    localDescription: "Transcription, word alignment, and speaker identification run on this Mac. Every real dependency is reported here.",
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
    localHint: "Prepare transcription first and start working. Speaker identification is optional and separately requires accepting model terms and setting HF_TOKEN. All setup runs in the background.",
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

export function SettingsView({ lang, pid }: Props) {
  const c = COPY[lang];
  const [settings, setSettings] = useState<Settings>(() => loadSettings());
  const [providerId, setProviderId] = useState(() => inferLlmProvider(settings.llmEndpoint));
  const [state, setState] = useState<"idle" | "saving" | "saved" | "error">("idle");
  const [message, setMessage] = useState<string | null>(null);
  const [asr, setAsr] = useState<AsrStatus | null>(null);
  const [asrAction, setAsrAction] = useState<"install" | "download" | "install-speakers" | "download-speakers" | "check" | null>("check");
  const [setupJob, setSetupJob] = useState<SetupJobStatus | null>(null);

  useEffect(() => {
    let disposed = false;
    void Promise.all([
      configShow(),
      asrStatus(),
      setupJobStatus().catch(() => null),
    ])
      .then(([config, status, setup]) => {
        if (disposed) return;
        setSettings({
          asrModel: config.asrModel,
          asrAligner: config.asrAligner,
          diarizeModel: config.diarizeModel,
          hfToken: config.hfToken,
          llmEndpoint: config.llmEndpoint,
          llmApiKey: config.llmApiKey,
          llmModel: config.llmModel,
          workerCount: config.workerCount,
        });
        setProviderId(inferLlmProvider(config.llmEndpoint));
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

  const update = <K extends keyof Settings>(key: K, value: Settings[K]) =>
    setSettings((previous) => ({ ...previous, [key]: value }));

  const selectedProvider = getLlmProvider(providerId);
  const providerConfigured = Boolean(
    settings.llmEndpoint.trim()
      && settings.llmModel.trim()
      && (!selectedProvider?.needsKey || settings.llmApiKey.trim()),
  );

  const selectProvider = (nextId: string) => {
    const changedProvider = nextId !== providerId;
    setProviderId(nextId);
    setState("idle");
    setMessage(null);
    if (nextId === "none") {
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
        setSettings((previous) => ({ ...previous, llmApiKey: "" }));
      }
      return;
    }
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
      if (normalized.llmEndpoint && !normalized.llmModel) {
        throw new Error(c.incomplete);
      }
      if (selectedProvider?.needsKey && !normalized.llmApiKey) {
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
              placeholder="hf_…"
              type="password"
              value={settings.hfToken}
              onChange={(event) => update("hfToken", event.target.value)}
            />
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
            {!asr?.runtimeReady && (
              <button className="button-primary" disabled={asrAction !== null} onClick={installAsr}>
                {asrAction === "install" ? c.installingRuntime : c.installRuntime}
              </button>
            )}
            {asr?.runtimeReady && (!asr.modelCached || !asr.alignerCached) && (
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
                    : setupJob.phase === "downloading"
                      ? lang === "zh" ? "正在下载模型文件" : "Downloading model files"
                      : setupJob.phase === "installing"
                        ? lang === "zh" ? "正在安装本地运行环境" : "Installing local runtime"
                        : lang === "zh" ? "正在安全停止" : "Stopping safely"}
                </strong>
                <span>{setupJob.kind}</span>
              </div>
              <progress aria-label={lang === "zh" ? "环境准备进度" : "Setup progress"} />
              <small>
                {lang === "zh"
                  ? "当前工具没有提供可信的总字节数，因此这里明确显示不定进度；详细失败输出会保留末尾日志。"
                  : "This tool does not expose a trustworthy total byte count, so progress is explicitly indeterminate; failure output keeps a bounded log tail."}
              </small>
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
                    placeholder={selectedProvider?.needsKey ? "sk-…" : undefined}
                    type="password"
                    value={settings.llmApiKey}
                    onChange={(event) => update("llmApiKey", event.target.value)}
                  />
                </label>
              )}

              <label>
                <span>{c.model}</span>
                <input
                  aria-label={c.model}
                  list={selectedProvider?.models.length ? "llm-model-options" : undefined}
                  placeholder={c.modelPlaceholder}
                  value={settings.llmModel}
                  onChange={(event) => update("llmModel", event.target.value)}
                />
                {selectedProvider?.models.length ? (
                  <datalist id="llm-model-options">
                    {selectedProvider.models.map((model) => <option key={model} value={model} />)}
                  </datalist>
                ) : null}
                <small className="field-hint">{c.modelHint}</small>
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
        <PipelineView embedded lang={lang} pid={pid} />
      </details>
    </section>
  );
}
