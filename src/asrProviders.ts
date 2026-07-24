export interface AsrProviderPreset {
  id: string;
  label: string;
  endpoint: string;
  model: string;
  models: string[];
  needsKey: boolean;
  custom?: boolean;
  note?: string;
}

/** OpenAI-compatible cloud ASR endpoints with word-timestamp support. */
export const ASR_PROVIDER_PRESETS: AsrProviderPreset[] = [
  {
    id: "openai",
    label: "OpenAI Whisper",
    endpoint: "https://api.openai.com/v1/audio/transcriptions",
    model: "whisper-1",
    models: ["whisper-1", "gpt-4o-transcribe", "gpt-4o-mini-transcribe"],
    needsKey: true,
    note: "Requires verbose_json word timestamps.",
  },
  {
    id: "groq",
    label: "Groq Whisper",
    endpoint: "https://api.groq.com/openai/v1/audio/transcriptions",
    model: "whisper-large-v3",
    models: ["whisper-large-v3", "whisper-large-v3-turbo", "distil-whisper-large-v3-en"],
    needsKey: true,
  },
  {
    id: "siliconflow",
    label: "硅基流动 SiliconFlow",
    endpoint: "https://api.siliconflow.cn/v1/audio/transcriptions",
    model: "FunAudioLLM/SenseVoiceSmall",
    models: ["FunAudioLLM/SenseVoiceSmall", "TeleAI/TeleSpeechASR"],
    needsKey: true,
  },
  {
    id: "deepinfra",
    label: "DeepInfra",
    endpoint: "https://api.deepinfra.com/v1/openai/audio/transcriptions",
    model: "openai/whisper-large-v3",
    models: ["openai/whisper-large-v3", "openai/whisper-large-v3-turbo"],
    needsKey: true,
  },
  {
    id: "custom",
    label: "OpenAI Compatible · 自定义",
    endpoint: "",
    model: "",
    models: [],
    needsKey: true,
    custom: true,
  },
];

export function matchAsrProviderPreset(endpoint: string): AsrProviderPreset {
  const normalized = endpoint.trim().replace(/\/+$/, "");
  const found = ASR_PROVIDER_PRESETS.find((preset) => {
    if (preset.custom) return false;
    return preset.endpoint.replace(/\/+$/, "") === normalized;
  });
  return found ?? ASR_PROVIDER_PRESETS[ASR_PROVIDER_PRESETS.length - 1];
}
