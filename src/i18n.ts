// Minimal i18n — zh/en string map + t(). Covers the app shell (nav +
// group headers); view-internal strings fall back to their source.
export type Lang = "zh" | "en";

const STR: Record<string, Record<Lang, string>> = {
  projects: { zh: "项目", en: "Projects" },
  editor: { zh: "编辑", en: "Editor" },
  transcript: { zh: "转写", en: "Transcript" },
  settings: { zh: "设置", en: "Settings" },
  tagline: { zh: "口播编辑器", en: "Speech editor" },
  navigation: { zh: "主导航", en: "Primary navigation" },
  chooseProjectFirst: {
    zh: "先选择或创建一个项目",
    en: "Choose or create a project first",
  },
  currentProject: { zh: "当前项目", en: "Current project" },
  firstStep: { zh: "第一步", en: "First step" },
  chooseMediaHint: {
    zh: "选择视频、粘贴链接或录一段声音。",
    en: "Choose media, paste a URL, or record audio.",
  },
  toggleTheme: { zh: "切换明暗主题", en: "Toggle color theme" },
  toggleLanguage: { zh: "切换语言", en: "Switch language" },
};

export function t(key: string, lang: Lang): string {
  return STR[key]?.[lang] ?? key;
}
