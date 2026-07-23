import { useEffect, useState } from "react";
import type { Lang } from "../../i18n";
import type { SubtitleStyle } from "../../types";

const SAVED_STYLE_LIBRARY_KEY = "lumen-cut.savedSubtitleStyles.v1";

type SavedStyle = {
  name: string;
  style: SubtitleStyle;
};

interface Props {
  busy: boolean;
  lang: Lang;
  savedStyle: SubtitleStyle;
  style: SubtitleStyle;
  onPreview: (style: SubtitleStyle) => void;
  onReset: () => void;
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

function isSubtitleStyle(value: unknown): value is SubtitleStyle {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const style = value as Record<string, unknown>;
  return ["name", "fontname", "primaryColour", "outlineColour"]
    .every((key) => typeof style[key] === "string")
    && ["fontsize", "alignment", "outline", "shadow", "marginL", "marginR", "marginV"]
      .every((key) => Number.isFinite(style[key]))
    && ["bold", "italic", "underline", "strikeOut"]
      .every((key) => typeof style[key] === "boolean");
}

function initialSavedStyles(): SavedStyle[] {
  try {
    const parsed = JSON.parse(localStorage.getItem(SAVED_STYLE_LIBRARY_KEY) || "[]");
    if (!Array.isArray(parsed)) return [];
    return parsed.flatMap((entry): SavedStyle[] => {
      if (!entry || typeof entry !== "object" || Array.isArray(entry)) return [];
      const candidate = entry as Record<string, unknown>;
      const name = typeof candidate.name === "string" ? candidate.name.trim() : "";
      return name && isSubtitleStyle(candidate.style)
        ? [{ name, style: { ...candidate.style, name } }]
        : [];
    }).slice(0, 24);
  } catch {
    return [];
  }
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

export function StyleWorkspace({
  busy,
  lang,
  savedStyle,
  style,
  onPreview,
  onReset,
  onSave,
}: Props) {
  const [draft, setDraft] = useState(style);
  const [saved, setSaved] = useState(false);
  const [savedStyles, setSavedStyles] = useState<SavedStyle[]>(initialSavedStyles);
  const [libraryName, setLibraryName] = useState("");
  const [libraryFeedback, setLibraryFeedback] = useState<string | null>(null);
  const dirty = JSON.stringify(draft) !== JSON.stringify(savedStyle);

  useEffect(() => {
    setDraft(style);
  }, [style]);

  useEffect(() => {
    if (!dirty) return;
    const warn = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [dirty]);

  useEffect(() => {
    try {
      if (savedStyles.length === 0) {
        localStorage.removeItem(SAVED_STYLE_LIBRARY_KEY);
      } else {
        localStorage.setItem(SAVED_STYLE_LIBRARY_KEY, JSON.stringify(savedStyles));
      }
    } catch {
      setLibraryFeedback(lang === "zh"
        ? "样式库无法写入本机存储；当前项目预览不受影响。"
        : "The style library could not be written to local storage. Project preview is unaffected.");
    }
  }, [lang, savedStyles]);

  const update = <K extends keyof SubtitleStyle>(key: K, value: SubtitleStyle[K]) => {
    setDraft((previous) => {
      const next = { ...previous, [key]: value };
      onPreview(next);
      return next;
    });
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
    setDraft((previous) => {
      const next = { ...previous, ...values };
      onPreview(next);
      return next;
    });
    setSaved(false);
  };

  const applySavedStyle = (savedStyle: SavedStyle) => {
    const next = { ...savedStyle.style, name: savedStyle.name };
    setDraft(next);
    onPreview(next);
    setSaved(false);
    setLibraryFeedback(lang === "zh"
      ? `已预览“${savedStyle.name}”；保存后才会应用到当前项目。`
      : `Previewing “${savedStyle.name}”. Save to apply it to this project.`);
  };

  const saveToLibrary = () => {
    const name = libraryName.trim().replace(/[\u0000-\u001f\u007f]/g, "").slice(0, 48);
    if (!name) return;
    const entry = { name, style: { ...draft, name } };
    setSavedStyles((current) => [
      ...current.filter((item) => item.name.toLocaleLowerCase() !== name.toLocaleLowerCase()),
      entry,
    ].slice(-24));
    setLibraryName("");
    setLibraryFeedback(lang === "zh"
      ? `“${name}”已保存到这台 Mac，可在其他项目复用。`
      : `“${name}” is saved on this Mac for reuse in other projects.`);
  };

  const removeFromLibrary = (name: string) => {
    setSavedStyles((current) => current.filter((item) => item.name !== name));
    setLibraryFeedback(lang === "zh"
      ? `已从样式库删除“${name}”；已使用它的项目不会改变。`
      : `Removed “${name}” from the library. Existing projects are unchanged.`);
  };

  const reset = () => {
    setDraft(savedStyle);
    onReset();
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
        <p>
          {lang === "zh"
            ? "实时预览已同步到左侧节目监看"
            : "Live preview is also shown in the program monitor"}
        </p>
      </section>

      <section className="style-controls">
        <div className="saved-style-library">
          <header>
            <span>{lang === "zh" ? "我的样式" : "My styles"}</span>
            <small>
              {lang === "zh"
                ? "保存在这台 Mac，跨项目复用"
                : "Saved on this Mac for every project"}
            </small>
          </header>
          {savedStyles.length > 0 ? (
            <div className="saved-style-list">
              {savedStyles.map((item) => (
                <div key={item.name}>
                  <button
                    aria-label={`${lang === "zh" ? "预览样式" : "Preview style"} ${item.name}`}
                    onClick={() => applySavedStyle(item)}
                    type="button"
                  >
                    <strong>{item.name}</strong>
                    <small>{item.style.fontname} · {item.style.fontsize}px</small>
                  </button>
                  <button
                    aria-label={`${lang === "zh" ? "删除样式" : "Delete style"} ${item.name}`}
                    className="saved-style-delete"
                    onClick={() => removeFromLibrary(item.name)}
                    title={lang === "zh" ? "只从样式库删除" : "Remove from library only"}
                    type="button"
                  >
                    ×
                  </button>
                </div>
              ))}
            </div>
          ) : (
            <p>{lang === "zh" ? "还没有可复用样式。" : "No reusable styles yet."}</p>
          )}
          <div className="saved-style-create">
            <input
              aria-label={lang === "zh" ? "新样式名称" : "New style name"}
              maxLength={48}
              placeholder={lang === "zh" ? "例如：访谈白字" : "e.g. Interview white"}
              value={libraryName}
              onChange={(event) => setLibraryName(event.target.value)}
              onKeyDown={(event) => {
                if (event.key !== "Enter" || !libraryName.trim()) return;
                event.preventDefault();
                saveToLibrary();
              }}
            />
            <button
              className="button-quiet"
              disabled={!libraryName.trim()}
              onClick={saveToLibrary}
              type="button"
            >
              {lang === "zh" ? "保存到样式库" : "Save to library"}
            </button>
          </div>
          {libraryFeedback && <small className="saved-style-feedback" role="status">
            {libraryFeedback}
          </small>}
        </div>

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
          <button
            aria-label={lang === "zh" ? "删除线" : "Strikethrough"}
            aria-pressed={draft.strikeOut}
            className={draft.strikeOut ? "active strike" : "strike"}
            onClick={() => update("strikeOut", !draft.strikeOut)}
          >S</button>
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

        <div className="control-row two-column">
          <label>
            <span>{lang === "zh" ? "左侧安全边距" : "Left safe margin"}</span>
            <input
              max={800}
              min={0}
              type="number"
              value={draft.marginL}
              onChange={(event) => update("marginL", Number(event.target.value))}
            />
          </label>
          <label>
            <span>{lang === "zh" ? "右侧安全边距" : "Right safe margin"}</span>
            <input
              max={800}
              min={0}
              type="number"
              value={draft.marginR}
              onChange={(event) => update("marginR", Number(event.target.value))}
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
          <span className={dirty ? "style-unsaved" : ""}>
            {dirty
              ? (lang === "zh" ? "有未保存修改；切换页面后草稿仍会保留" : "Unsaved changes; the draft remains when switching pages")
              : saved
                ? (lang === "zh" ? "已保存到项目" : "Saved to project")
                : (lang === "zh" ? "当前样式已保存" : "Current style is saved")}
          </span>
          {dirty && (
            <button className="button-quiet" disabled={busy} onClick={reset}>
              {lang === "zh" ? "重置" : "Reset"}
            </button>
          )}
          <button className="button-primary" disabled={busy || !dirty} onClick={save}>
            {busy
              ? (lang === "zh" ? "保存中…" : "Saving…")
              : (lang === "zh" ? "保存样式" : "Save style")}
          </button>
        </div>
      </section>
    </div>
  );
}
