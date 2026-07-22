export interface LlmProviderPreset {
  id: string;
  label: string;
  endpoint: string;
  model: string;
  models: string[];
  needsKey: boolean;
  local?: boolean;
  custom?: boolean;
}

export const LLM_PROVIDER_PRESETS: LlmProviderPreset[] = [
  {
    id: "openai",
    label: "OpenAI",
    endpoint: "https://api.openai.com/v1/chat/completions",
    model: "gpt-4.1-mini",
    models: ["gpt-4.1-mini", "gpt-4.1", "gpt-4o-mini"],
    needsKey: true,
  },
  {
    id: "anthropic",
    label: "Anthropic Claude",
    endpoint: "https://api.anthropic.com/v1/messages",
    model: "claude-sonnet-4-5",
    models: ["claude-sonnet-4-5", "claude-haiku-4-5"],
    needsKey: true,
  },
  {
    id: "deepseek",
    label: "DeepSeek 深度求索",
    endpoint: "https://api.deepseek.com/v1/chat/completions",
    model: "deepseek-chat",
    models: ["deepseek-chat", "deepseek-reasoner"],
    needsKey: true,
  },
  {
    id: "minimax-cn",
    label: "MiniMax · 中国大陆",
    endpoint: "https://api.minimaxi.com/v1/chat/completions",
    model: "MiniMax-M2.7",
    models: [
      "MiniMax-M2.7",
      "MiniMax-M2.7-highspeed",
      "MiniMax-M2.5",
      "MiniMax-M2.5-highspeed",
    ],
    needsKey: true,
  },
  {
    id: "minimax-global",
    label: "MiniMax · 海外",
    endpoint: "https://api.minimax.io/v1/chat/completions",
    model: "MiniMax-M2.7",
    models: [
      "MiniMax-M2.7",
      "MiniMax-M2.7-highspeed",
      "MiniMax-M2.5",
      "MiniMax-M2.5-highspeed",
    ],
    needsKey: true,
  },
  {
    id: "glm-cn",
    label: "GLM 智谱 · 中国大陆",
    endpoint: "https://open.bigmodel.cn/api/paas/v4/chat/completions",
    model: "glm-5.2",
    models: ["glm-5.2", "glm-5.1", "glm-5-turbo", "glm-4.7", "glm-4.7-flash"],
    needsKey: true,
  },
  {
    id: "glm-global",
    label: "GLM Z.AI · 海外",
    endpoint: "https://api.z.ai/api/paas/v4/chat/completions",
    model: "glm-5.1",
    models: ["glm-5.1", "glm-5-turbo", "glm-5", "glm-4.7", "glm-4.7-flash"],
    needsKey: true,
  },
  {
    id: "qwen",
    label: "通义千问 · 阿里云百炼",
    endpoint: "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions",
    model: "qwen-plus",
    models: ["qwen-plus", "qwen-max", "qwen-turbo", "qwen-long"],
    needsKey: true,
  },
  {
    id: "kimi",
    label: "Kimi · 月之暗面",
    endpoint: "https://api.moonshot.cn/v1/chat/completions",
    model: "kimi-latest",
    models: ["kimi-latest", "moonshot-v1-8k", "moonshot-v1-32k", "moonshot-v1-128k"],
    needsKey: true,
  },
  {
    id: "siliconflow",
    label: "硅基流动 SiliconFlow",
    endpoint: "https://api.siliconflow.cn/v1/chat/completions",
    model: "deepseek-ai/DeepSeek-V3",
    models: [
      "deepseek-ai/DeepSeek-V3",
      "deepseek-ai/DeepSeek-R1",
      "Qwen/Qwen2.5-72B-Instruct",
      "Qwen/Qwen2.5-Coder-32B-Instruct",
    ],
    needsKey: true,
  },
  {
    id: "openrouter",
    label: "OpenRouter · 海外聚合",
    endpoint: "https://openrouter.ai/api/v1/chat/completions",
    model: "openai/gpt-5.2",
    models: [
      "openai/gpt-5.2",
      "anthropic/claude-sonnet-4.6",
      "anthropic/claude-haiku-4.5",
      "deepseek/deepseek-chat",
    ],
    needsKey: true,
  },
  {
    id: "ollama",
    label: "Ollama · 本机模型",
    endpoint: "http://localhost:11434/v1/chat/completions",
    model: "qwen3:8b",
    models: ["qwen3:8b", "qwen3:14b", "llama3.1:8b"],
    needsKey: false,
    local: true,
  },
  {
    id: "custom",
    label: "OpenAI Compatible · 自定义",
    endpoint: "",
    model: "",
    models: [],
    needsKey: false,
    custom: true,
  },
];

export function inferLlmProvider(endpoint: string): string {
  const normalized = endpoint.trim().replace(/\/$/, "");
  if (!normalized) return "none";
  return LLM_PROVIDER_PRESETS.find(
    (provider) => provider.endpoint.replace(/\/$/, "") === normalized,
  )?.id ?? "custom";
}

export function getLlmProvider(id: string): LlmProviderPreset | undefined {
  return LLM_PROVIDER_PRESETS.find((provider) => provider.id === id);
}
