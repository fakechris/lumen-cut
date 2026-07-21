import { useEffect, useState } from "react";
import type { Lang } from "../../i18n";
import type { SubtitleStyle } from "../../types";

interface Props {
  busy: boolean;
  lang: Lang;
  style: SubtitleStyle;
  onSave: (style: SubtitleStyle) => Promise<void>;
}

function assToHex(value: string) {
  const match = value.match(/&H[0-9A-Fa-f]{2}([0-9A-Fa-f]{2})([0-9A-Fa-f]{2})([0-9A-Fa-f]{2})/);
  return match ? `#${match[3]}${match[2]}${match[1]}` : "#ffffff";
}

function hexToAss(value: string) {
  const clean = value.replace("#", "");
  if (clean.length !== 6) return "&H00FFFFFF";
  return `&H00${clean.slice(4, 6)}${clean.slice(2, 4)}${clean.slice(0, 2)}`.toUpperCase();
}

const STYLE_PRESETS: Array<{
  id: string;
  zh: string;
  en: string;
  descriptionZh: string;
  descriptionEn: string;
  values: Partial<SubtitleStyle>;
}> = [
  {
    id: "clean",
    zh: "清晰白字",
    en: "Clean white",
    descriptionZh: "通用访谈与课程",
    descriptionEn: "Interviews and courses",
    values: {
      name: "Clean white",
      fontname: "PingFang SC",
      fontsize: 52,
      primaryColour: "&H00FFFFFF",
      outlineColour: "&H00000000",
      bold: false,
      italic: false,
      underline: false,
      strikeOut: false,
      alignment: 2,
      outline: 2,
      shadow: 1,
      marginV: 80,
    },
  },
  {
    id: "creator",
    zh: "创作者黄字",
    en: "Creator yellow",
    descriptionZh: "短视频与重点表达",
    descriptionEn: "Short-form emphasis",
    values: {
      name: "Creator yellow",
      fontname: "PingFang SC",
      fontsize: 58,
      primaryColour: "&H0000E8FF",
      outlineColour: "&H00141414",
      bold: true,
      italic: false,
      underline: false,
      strikeOut: false,
      alignment: 2,
      outline: 3,
      shadow: 1,
      marginV: 92,
    },
  },
  {
    id: "minimal",
    zh: "极简小字",
    en: "Minimal",
    descriptionZh: "纪录片与安静画面",
    descriptionEn: "Documentary and quiet scenes",
    values: {
      name: "Minimal",
      fontname: "Helvetica Neue",
      fontsize: 42,
      primaryColour: "&H00FFFFFF",
      outlineColour: "&H00101010",
      bold: false,
      italic: false,
      underline: false,
      strikeOut: false,
      alignment: 2,
      outline: 1,
      shadow: 0,
      marginV: 68,
    },
  },
  {
    id: "top",
    zh: "顶部标题",
    en: "Top title",
    descriptionZh: "避开画面底部信息",
    descriptionEn: "Keeps the lower frame clear",
    values: {
      name: "Top title",
      fontname: "PingFang SC",
      fontsize: 50,
      primaryColour: "&H00FFFFFF",
      outlineColour: "&H00000000",
      bold: true,
      italic: false,
      underline: false,
      strikeOut: false,
      alignment: 8,
      outline: 2,
      shadow: 1,
      marginV: 72,
    },
  },
];

export function StyleWorkspace({ busy, lang, style, onSave }: Props) {
  const [draft, setDraft] = useState(style);
  const [saved, setSaved] = useState(false);

  useEffect(() => setDraft(style), [style]);

  const update = <K extends keyof SubtitleStyle>(key: K, value: SubtitleStyle[K]) => {
    setDraft((previous) => ({ ...previous, [key]: value }));
    setSaved(false);
  };

  const save = async () => {
    try {
      await onSave(draft);
      setSaved(true);
    } catch {
      setSaved(false);
    }
  };

  const applyPreset = (values: Partial<SubtitleStyle>) => {
    setDraft((previous) => ({ ...previous, ...values }));
    setSaved(false);
  };

  return (
    <div className="style-workspace">
      <section className="style-preview">
        <div className="preview-frame">
          <div
            className="subtitle-preview-text"
            style={{
              color: assToHex(draft.primaryColour),
              fontFamily: draft.fontname,
              fontSize: `${Math.max(18, draft.fontsize * 0.55)}px`,
              fontStyle: draft.italic ? "italic" : "normal",
              fontWeight: draft.bold ? 700 : 400,
              textDecoration: `${draft.underline ? "underline " : ""}${draft.strikeOut ? "line-through" : ""}`.trim() || "none",
              WebkitTextStroke: `${Math.max(0, draft.outline * 0.55)}px ${assToHex(draft.outlineColour)}`,
              textShadow: draft.shadow > 0
                ? `${draft.shadow}px ${draft.shadow}px ${assToHex(draft.outlineColour)}`
                : "none",
            }}
          >
            {lang === "zh" ? "让每一句话都清楚、好看。" : "Make every line clear and considered."}
          </div>
        </div>
        <p>{lang === "zh" ? "导出预览" : "Export preview"}</p>
      </section>

      <section className="style-controls">
        <div className="style-presets">
          <span>{lang === "zh" ? "快速样式" : "Quick styles"}</span>
          <div>
            {STYLE_PRESETS.map((preset) => (
              <button
                key={preset.id}
                type="button"
                onClick={() => applyPreset(preset.values)}
              >
                <strong>{lang === "zh" ? preset.zh : preset.en}</strong>
                <small>{lang === "zh" ? preset.descriptionZh : preset.descriptionEn}</small>
              </button>
            ))}
          </div>
        </div>

        <div className="control-row two-column">
          <label>
            <span>{lang === "zh" ? "字体" : "Typeface"}</span>
            <input value={draft.fontname} onChange={(event) => update("fontname", event.target.value)} />
          </label>
          <label>
            <span>{lang === "zh" ? "字号" : "Size"}</span>
            <input
              max={120}
              min={16}
              type="number"
              value={draft.fontsize}
              onChange={(event) => update("fontsize", Number(event.target.value))}
            />
          </label>
        </div>

        <div className="format-buttons" aria-label={lang === "zh" ? "文字格式" : "Text formatting"}>
          <button
            aria-pressed={draft.bold}
            className={draft.bold ? "active" : ""}
            onClick={() => update("bold", !draft.bold)}
          >B</button>
          <button
            aria-pressed={draft.italic}
            className={draft.italic ? "active italic" : "italic"}
            onClick={() => update("italic", !draft.italic)}
          >I</button>
          <button
            aria-pressed={draft.underline}
            className={draft.underline ? "active underline" : "underline"}
            onClick={() => update("underline", !draft.underline)}
          >U</button>
        </div>

        <div className="control-row two-column">
          <label>
            <span>{lang === "zh" ? "文字颜色" : "Text color"}</span>
            <input
              type="color"
              value={assToHex(draft.primaryColour)}
              onChange={(event) => update("primaryColour", hexToAss(event.target.value))}
            />
          </label>
          <label>
            <span>{lang === "zh" ? "描边颜色" : "Outline color"}</span>
            <input
              type="color"
              value={assToHex(draft.outlineColour)}
              onChange={(event) => update("outlineColour", hexToAss(event.target.value))}
            />
          </label>
        </div>

        <div className="control-row three-column">
          <label>
            <span>{lang === "zh" ? "描边" : "Outline"}</span>
            <input
              max={10}
              min={0}
              type="number"
              value={draft.outline}
              onChange={(event) => update("outline", Number(event.target.value))}
            />
          </label>
          <label>
            <span>{lang === "zh" ? "阴影" : "Shadow"}</span>
            <input
              max={10}
              min={0}
              type="number"
              value={draft.shadow}
              onChange={(event) => update("shadow", Number(event.target.value))}
            />
          </label>
          <label>
            <span>{lang === "zh" ? "底部边距" : "Bottom margin"}</span>
            <input
              max={400}
              min={0}
              type="number"
              value={draft.marginV}
              onChange={(event) => update("marginV", Number(event.target.value))}
            />
          </label>
        </div>

        <div className="alignment-control">
          <span>{lang === "zh" ? "字幕位置" : "Position"}</span>
          <div className="alignment-grid">
            {[7, 8, 9, 4, 5, 6, 1, 2, 3].map((value) => (
              <button
                aria-label={`${lang === "zh" ? "位置" : "Position"} ${value}`}
                aria-pressed={draft.alignment === value}
                className={draft.alignment === value ? "active" : ""}
                key={value}
                onClick={() => update("alignment", value)}
              />
            ))}
          </div>
        </div>

        <div className="style-save-row">
          <button className="button-primary" disabled={busy} onClick={save}>
            {busy
              ? (lang === "zh" ? "保存中…" : "Saving…")
              : (lang === "zh" ? "保存样式" : "Save style")}
          </button>
          {saved && <span>{lang === "zh" ? "已保存到项目" : "Saved to project"}</span>}
        </div>
      </section>
    </div>
  );
}
