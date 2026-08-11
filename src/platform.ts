/** Host-platform detection for the webview.
 *
 *  The Rust side answers this with `cfg!`, but the UI needs it too: shortcut
 *  labels and the WYSIWYG caption canvas both have to name platform-specific
 *  things (modifier glyphs, installed typefaces). Keep the font choices in
 *  sync with `preset_fontname` in `src-tauri/src/data/caption_presets.rs`. */

function userAgent(): string {
  if (typeof navigator === "undefined") return "";
  return navigator.userAgent ?? "";
}

/** WebView2 reports `Windows NT`; WKWebView reports `Mac OS X`. */
export function isWindows(): boolean {
  return /Windows/i.test(userAgent());
}

export function isMac(): boolean {
  return /Mac OS X|Macintosh/i.test(userAgent());
}

/** Glyph for the "primary" accelerator: ⌘ on macOS, `Ctrl` elsewhere.
 *  The handlers themselves already accept `metaKey || ctrlKey`. */
export function modifierLabel(): string {
  return isMac() ? "⌘" : "Ctrl";
}

/** Glyph for shift, spelled out where the symbol is not idiomatic. */
export function shiftLabel(): string {
  return isMac() ? "⇧" : "Shift";
}

/** `Ctrl Z` / `⌘Z`. Windows shortcut text conventionally uses `+`. */
export function shortcutLabel(key: string, options?: { shift?: boolean }): string {
  if (isMac()) {
    return `${options?.shift ? shiftLabel() : ""}${modifierLabel()}${key}`;
  }
  return [options?.shift ? shiftLabel() : null, modifierLabel(), key]
    .filter(Boolean)
    .join("+");
}
