import { useState } from "react";
import { loadSettings, saveSettings, settingsExport } from "../api";
import type { Lang } from "../i18n";
import type { Settings } from "../types";
import { PipelineView } from "./PipelineView";

interface Props {
  lang: Lang;
  pid: string | null;
}

const COPY = {
  zh: {
    eyebrow: "偏好设置",
    title: "设置",
    intro: "基础转写开箱即用；翻译、润色等 AI 功能会按需在后台自动运行。",
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
    intro: "Core transcription works out of the box. Translation and polish start automatically in the background when requested.",
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
      if (Boolean(normalized.llmEndpoint) !== Boolean(normalized.llmModel)) {
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
      setState("saved");
      window.setTimeout(() => setState("idle"), 1800);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
      setState("error");
    }
  };

  return (
    <section className="settings-view">
      <header className="page-header">
        <p className="eyebrow">{c.eyebrow}</p>
        <h1>{c.title}</h1>
        <p>{c.intro}</p>
      </header>

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
        <PipelineView embedded pid={pid} />
      </details>
    </section>
  );
}
