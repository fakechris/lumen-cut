//! Caption visual preset table — ported from pireel/pireel (AGPL-3.0),
//! packages/studio-engine/src/caption-presets.ts (plus the latinJoin/joinWords
//! helpers from caption-fx.ts).
//!
//! Only static styles + current-word karaoke are ported; pireel's kinetic word
//! animations (slam/pop etc.) are intentionally out of scope. A preset governs
//! LOOK only — font size, bold and position stay on the project's `SubStyle`.

use super::doc::Word;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptionMode {
    /// Whole line shown; the spoken word is highlighted (karaoke).
    Emphasis,
    /// Clean full-line caption, no per-word behaviour.
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deco {
    Underline,
    Highlight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetFont {
    Serif,
    Mono,
}

#[derive(Debug, Clone, Copy)]
pub struct CaptionPreset {
    pub id: &'static str,
    pub mode: CaptionMode,
    /// Body text color (CSS `#rrggbb`).
    pub text: &'static str,
    /// Emphasized-word color (emphasis mode).
    pub emphasis: Option<&'static str>,
    /// Full-line backing color (CSS color, may carry alpha).
    pub bg: Option<&'static str>,
    pub deco: Option<Deco>,
    #[allow(dead_code)] // preview-only today; the ASS mapping approximates deco via style flags
    pub deco_color: Option<&'static str>,
    pub font: Option<PresetFont>,
    pub italic: bool,
}

pub const CAPTION_PRESETS: &[CaptionPreset] = &[
    // —— Word emphasis ——
    CaptionPreset {
        id: "em-yellow",
        mode: CaptionMode::Emphasis,
        text: "#ffffff",
        emphasis: Some("#ffe34f"),
        bg: None,
        deco: None,
        deco_color: None,
        font: None,
        italic: false,
    },
    CaptionPreset {
        id: "em-green",
        mode: CaptionMode::Emphasis,
        text: "#ffffff",
        emphasis: Some("#5affb6"),
        bg: None,
        deco: None,
        deco_color: None,
        font: None,
        italic: false,
    },
    CaptionPreset {
        id: "em-purple-black",
        mode: CaptionMode::Emphasis,
        text: "#ffffff",
        emphasis: Some("#cf96ff"),
        bg: Some("rgba(0,0,0,0.72)"),
        deco: None,
        deco_color: None,
        font: None,
        italic: false,
    },
    CaptionPreset {
        id: "em-serif-black",
        mode: CaptionMode::Emphasis,
        text: "#ffffff",
        emphasis: Some("#63ffc7"),
        bg: Some("rgba(0,0,0,0.72)"),
        deco: None,
        deco_color: None,
        font: Some(PresetFont::Serif),
        italic: false,
    },
    CaptionPreset {
        id: "em-underline",
        mode: CaptionMode::Emphasis,
        text: "#ffffff",
        emphasis: None,
        bg: Some("rgba(0,0,0,0.8)"),
        deco: Some(Deco::Underline),
        deco_color: Some("#ffffff"),
        font: None,
        italic: false,
    },
    CaptionPreset {
        id: "em-blue-line",
        mode: CaptionMode::Emphasis,
        text: "#111111",
        emphasis: Some("#0059ff"),
        bg: Some("rgba(255,255,255,0.78)"),
        deco: Some(Deco::Underline),
        deco_color: Some("#0059ff"),
        font: None,
        italic: false,
    },
    CaptionPreset {
        id: "em-box-purple",
        mode: CaptionMode::Emphasis,
        text: "#ffffff",
        emphasis: None,
        bg: Some("rgba(118,40,187,0.85)"),
        deco: Some(Deco::Highlight),
        deco_color: Some("rgba(0,0,0,0.4)"),
        font: None,
        italic: false,
    },
    CaptionPreset {
        id: "em-box-blue",
        mode: CaptionMode::Emphasis,
        text: "#ffffff",
        emphasis: None,
        bg: Some("rgba(0,89,255,0.85)"),
        deco: Some(Deco::Highlight),
        deco_color: Some("#000000"),
        font: None,
        italic: false,
    },
    CaptionPreset {
        id: "em-pink",
        mode: CaptionMode::Emphasis,
        text: "#fccfcf",
        emphasis: Some("#ffffff"),
        bg: Some("rgba(236,137,134,0.85)"),
        deco: None,
        deco_color: None,
        font: None,
        italic: false,
    },
    CaptionPreset {
        id: "em-gold-serif",
        mode: CaptionMode::Emphasis,
        text: "#b89d4c",
        emphasis: Some("#7f6000"),
        bg: Some("rgba(248,233,192,0.85)"),
        deco: None,
        deco_color: None,
        font: Some(PresetFont::Serif),
        italic: false,
    },
    // —— Line by line ——
    CaptionPreset {
        id: "ln-clean",
        mode: CaptionMode::Line,
        text: "#ffffff",
        emphasis: None,
        bg: None,
        deco: None,
        deco_color: None,
        font: None,
        italic: false,
    },
    CaptionPreset {
        id: "ln-black",
        mode: CaptionMode::Line,
        text: "#ffffff",
        emphasis: None,
        bg: Some("rgba(0,0,0,0.85)"),
        deco: None,
        deco_color: None,
        font: None,
        italic: false,
    },
    CaptionPreset {
        id: "ln-navy",
        mode: CaptionMode::Line,
        text: "#ffffff",
        emphasis: None,
        bg: Some("rgba(70,80,109,0.85)"),
        deco: None,
        deco_color: None,
        font: Some(PresetFont::Serif),
        italic: false,
    },
    CaptionPreset {
        id: "ln-white",
        mode: CaptionMode::Line,
        text: "#3901ee",
        emphasis: None,
        bg: Some("rgba(255,255,255,0.85)"),
        deco: None,
        deco_color: None,
        font: None,
        italic: true,
    },
    CaptionPreset {
        id: "ln-orange",
        mode: CaptionMode::Line,
        text: "#ffffff",
        emphasis: None,
        bg: Some("rgba(255,140,90,0.85)"),
        deco: None,
        deco_color: None,
        font: None,
        italic: false,
    },
    CaptionPreset {
        id: "ln-yellow",
        mode: CaptionMode::Line,
        text: "#000000",
        emphasis: None,
        bg: Some("rgba(255,227,79,0.85)"),
        deco: None,
        deco_color: None,
        font: None,
        italic: false,
    },
    CaptionPreset {
        id: "ln-red",
        mode: CaptionMode::Line,
        text: "#ffffff",
        emphasis: None,
        bg: Some("rgba(255,0,0,0.85)"),
        deco: None,
        deco_color: None,
        font: Some(PresetFont::Mono),
        italic: false,
    },
    CaptionPreset {
        id: "ln-mint",
        mode: CaptionMode::Line,
        text: "#63ffc7",
        emphasis: None,
        bg: None,
        deco: None,
        deco_color: None,
        font: None,
        italic: false,
    },
];

/// Look up a preset by id; unknown ids are `None` (callers keep the plain style).
pub fn caption_preset(id: &str) -> Option<&'static CaptionPreset> {
    CAPTION_PRESETS.iter().find(|p| p.id == id)
}

/// ASS fontname for a preset font (libass resolves against system fonts).
pub fn preset_fontname(preset: &CaptionPreset) -> Option<&'static str> {
    match preset.font {
        Some(PresetFont::Serif) => Some("Noto Serif SC"),
        Some(PresetFont::Mono) => Some("IBM Plex Mono"),
        None => None,
    }
}

/// Translation sub-line size relative to the main line (applied as an ASS
/// `\fs` override on the sub-line). pireel uses 0.7, but CJK glyphs read far
/// smaller than Latin at the same em size, so lumen-cut uses 0.85. Kept in
/// sync with CAPTION_SUB_LINE_SCALE in src/views/editor/captionPresets.ts so
/// the preview matches the export.
pub const CAPTION_SUB_LINE_SCALE: f64 = 0.85;

/// CSS color (`#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb()`/`rgba()`) → ASS `&HAABBGGRR`.
/// ASS alpha is inverted opacity: 0 = opaque, 255 = transparent.
pub fn css_color_to_ass(value: &str) -> Option<String> {
    let v = value.trim();
    let (r, g, b, a) = if let Some(hex) = v.strip_prefix('#') {
        let expand = |h: &str| u8::from_str_radix(&h.repeat(2), 16).ok();
        let byte = |h: &str| u8::from_str_radix(h, 16).ok();
        match hex.len() {
            3 => (
                expand(&hex[0..1])?,
                expand(&hex[1..2])?,
                expand(&hex[2..3])?,
                255,
            ),
            6 => (byte(&hex[0..2])?, byte(&hex[2..4])?, byte(&hex[4..6])?, 255),
            8 => (
                byte(&hex[0..2])?,
                byte(&hex[2..4])?,
                byte(&hex[4..6])?,
                byte(&hex[6..8])?,
            ),
            _ => return None,
        }
    } else if v.starts_with("rgb") {
        let inner = v[v.find('(')? + 1..v.rfind(')')?].to_owned();
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        if parts.len() != 3 && parts.len() != 4 {
            return None;
        }
        let channel = |s: &str| -> Option<u8> { s.parse::<f64>().ok().map(|n| n.round() as u8) };
        let alpha = if parts.len() == 4 {
            // rgba() alpha is 0–1 (CSS) — accept 0–255 too for safety.
            let raw: f64 = parts[3].parse().ok()?;
            let unit = if raw > 1.0 { raw / 255.0 } else { raw };
            (unit.clamp(0.0, 1.0) * 255.0).round() as u8
        } else {
            255
        };
        (
            channel(parts[0])?,
            channel(parts[1])?,
            channel(parts[2])?,
            alpha,
        )
    } else {
        return None;
    };
    let ass_alpha = 255u8.saturating_sub(a);
    Some(format!("&H{ass_alpha:02X}{b:02X}{g:02X}{r:02X}"))
}

/// Two adjacent words both Latin/digit → a real space between them (adjacent CJK
/// doesn't need one). Ported from pireel caption-fx.ts `latinJoin`.
fn latin_join(a: &str, b: &str) -> bool {
    let tail = |c: char| c.is_ascii_alphanumeric() || ".,!?;:'\")]%".contains(c);
    let head = |c: char| c.is_ascii_alphanumeric() || "('\"[$".contains(c);
    a.chars().last().is_some_and(tail) && b.chars().next().is_some_and(head)
}

/// Join ASR word texts back into a sentence (spaces at Latin boundaries only).
fn join_words(words: &[Word]) -> String {
    let mut out = String::new();
    for (i, w) in words.iter().enumerate() {
        out.push_str(&w.text);
        if i + 1 < words.len() && latin_join(&w.text, &words[i + 1].text) {
            out.push(' ');
        }
    }
    out
}

fn without_whitespace(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// CJK scripts the approximation segments per character (same script ranges
/// the frontend's detectLang/segmentation recognises).
fn is_cjk(ch: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&ch)
        || ('\u{3040}'..='\u{30ff}').contains(&ch)
        || ('\u{ac00}'..='\u{d7af}').contains(&ch)
}

/// Rough word segmentation for the NO-REAL-WORD-TIMING approximation: CJK
/// splits per character, Latin splits on whitespace, punctuation glues onto
/// the preceding token. This is the Rust counterpart of the frontend's
/// segmentTokens (Intl.Segmenter) — coarser (single CJK chars where ICU
/// groups dictionary words) but with the same contract: the tokens rebuild
/// the input and punctuation never stands alone.
fn approx_tokens(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut latin = String::new();
    fn flush(latin: &mut String, tokens: &mut Vec<String>) {
        if !latin.is_empty() {
            tokens.push(std::mem::take(latin));
        }
    }
    for ch in text.trim().chars() {
        if ch.is_whitespace() {
            flush(&mut latin, &mut tokens);
        } else if is_cjk(ch) {
            flush(&mut latin, &mut tokens);
            tokens.push(ch.to_string());
        } else if ch.is_alphanumeric() || !latin.is_empty() {
            // Latin letters/digits, plus punctuation inside or right after a
            // Latin run, stay with that token ("hello,", "it's").
            latin.push(ch);
        } else if let Some(last) = tokens.last_mut() {
            // Punctuation right after a CJK token glues backwards ("。").
            last.push(ch);
        } else {
            latin.push(ch);
        }
    }
    flush(&mut latin, &mut tokens);
    tokens
}

/// Approximate karaoke tags for text without real word timing (translation
/// lines, translation-only cues): split into tokens and allocate the cue
/// window [start,end) linearly by token character count — the same algorithm
/// as the frontend's wordsFromText. Returns None for empty text or a
/// zero-length window (caller falls back to the plain line).
fn approx_karaoke(text: &str, start: f64, end: f64) -> Option<String> {
    let tokens = approx_tokens(text);
    if tokens.is_empty() || end <= start {
        return None;
    }
    let total: usize = tokens.iter().map(|t| t.chars().count()).sum();
    if total == 0 {
        return None;
    }
    let span = end - start;
    let mut out = String::new();
    for (i, tok) in tokens.iter().enumerate() {
        let dur = (tok.chars().count() as f64 / total as f64) * span;
        let cs = (dur * 100.0).round().max(1.0) as u32;
        out.push_str(&format!("{{\\k{cs}}}{tok}"));
        if i + 1 < tokens.len() && latin_join(tok, &tokens[i + 1]) {
            out.push(' ');
        }
    }
    Some(out)
}

/// Build the Dialogue text for a caption-preset sentence.
///
/// Emphasis presets with an emphasis colour get `\k` karaoke tags: the inline
/// `\1c` (sung = emphasis colour) / `\2c` (unsung = body colour) overrides make
/// each word switch colour as it is spoken. Timing sources, in priority order:
///   - a source line the word stream rebuilds verbatim → REAL ASR word timing,
///     retimed onto the post-cut timeline;
///   - everything else (translation lines, translation-only cues, edited text)
///     → APPROXIMATION: the cue window is allocated linearly across the text's
///     tokens (approx_tokens/approx_karaoke) — translations have no real word
///     timing of their own. The preview applies the same approximation.
/// Bilingual cues ("source\ntranslation") karaoke both lines, and the
/// translation gets a `\fs` override at CAPTION_SUB_LINE_SCALE so the export
/// matches the preview's sub-line ratio.
///
/// Returns `None` when nothing can be timed (empty text, no words, zero-length
/// window): the caller falls back to the plain whole-line style.
pub fn preset_dialogue_text(
    text: &str,
    words: &[Word],
    preset: &CaptionPreset,
    font_size: u32,
    retime: &dyn Fn(f64) -> f64,
) -> Option<String> {
    let emphasis = preset
        .emphasis
        .filter(|_| preset.mode == CaptionMode::Emphasis)?;
    let primary = css_color_to_ass(emphasis)?;
    let secondary = css_color_to_ass(preset.text)?;
    // Bilingual cues are "source\ntranslation"; words only cover the source line.
    let (source, translation) = match text.split_once('\n') {
        Some((source, rest)) => (source, Some(rest)),
        None => (text, None),
    };
    if source.is_empty() || words.is_empty() {
        return None;
    }
    // Karaoke window: the cue's retimed span (the same bounds ass.rs puts on
    // the Dialogue event).
    let start = retime(words.first()?.start);
    let end = retime(words.last()?.end);
    let main = if without_whitespace(&join_words(words)) == without_whitespace(source) {
        // Real ASR word timing; cut-away words collapse to a 1cs blip.
        let mut out = String::new();
        for (i, w) in words.iter().enumerate() {
            let cs = ((retime(w.end) - retime(w.start)) * 100.0).round().max(1.0) as u32;
            out.push_str(&format!("{{\\k{cs}}}{}", w.text));
            if i + 1 < words.len() && latin_join(&w.text, &words[i + 1].text) {
                out.push(' ');
            }
        }
        out
    } else {
        // Approximation: translation-only cue, or text the word stream cannot
        // reproduce — linear token timing over the cue window.
        approx_karaoke(source, start, end)?
    };
    let mut out = format!("{{\\1c{primary}\\2c{secondary}}}{main}");
    if let Some(rest) = translation {
        let sub_size = (font_size as f64 * CAPTION_SUB_LINE_SCALE).round().max(1.0) as u32;
        let sub = approx_karaoke(rest, start, end)?;
        out.push_str(&format!("\\N{{\\fs{sub_size}}}{sub}"));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, start: f64, end: f64) -> Word {
        Word {
            id: text.into(),
            text: text.into(),
            start,
            end,
        }
    }

    #[test]
    fn preset_table_has_eighteen_entries_in_two_modes() {
        assert_eq!(CAPTION_PRESETS.len(), 18);
        assert_eq!(
            CAPTION_PRESETS
                .iter()
                .filter(|p| p.mode == CaptionMode::Emphasis)
                .count(),
            10
        );
        assert_eq!(
            CAPTION_PRESETS
                .iter()
                .filter(|p| p.mode == CaptionMode::Line)
                .count(),
            8
        );
        assert!(caption_preset("em-yellow").is_some());
        assert!(caption_preset("no-such-preset").is_none());
    }

    #[test]
    fn css_colors_convert_to_ass_bgr() {
        assert_eq!(css_color_to_ass("#ffffff").as_deref(), Some("&H00FFFFFF"));
        assert_eq!(css_color_to_ass("#ffe34f").as_deref(), Some("&H004FE3FF"));
        // Alpha is inverted opacity: rgba(0,0,0,0.72) ≈ 28% transparent black.
        assert_eq!(
            css_color_to_ass("rgba(0,0,0,0.72)").as_deref(),
            Some("&H47000000")
        );
        assert_eq!(css_color_to_ass("#00000080").as_deref(), Some("&H7F000000"));
        assert_eq!(
            css_color_to_ass("rgb(255, 227, 79)").as_deref(),
            Some("&H004FE3FF")
        );
        assert!(css_color_to_ass("not-a-color").is_none());
    }

    #[test]
    fn karaoke_tags_follow_word_timing() {
        let preset = caption_preset("em-yellow").unwrap();
        let words = [word("Hello", 1.0, 1.4), word("world", 1.4, 2.0)];
        let text = preset_dialogue_text("Hello world", &words, preset, 52, &|t| t).unwrap();
        assert_eq!(
            text,
            "{\\1c&H004FE3FF\\2c&H00FFFFFF}{\\k40}Hello {\\k60}world"
        );
    }

    #[test]
    fn bilingual_cue_karaokes_both_lines_and_scales_the_sub_line() {
        let preset = caption_preset("em-yellow").unwrap();
        let words = [word("你好", 0.0, 0.5), word("世界", 0.5, 1.0)];
        let text = preset_dialogue_text("你好世界\nhello", &words, preset, 52, &|t| t).unwrap();
        // Main line: real ASR timing. Sub-line: approximated (single token over
        // the whole [0,1) window) and shrunk to 52 × 0.85 ≈ 44 via \fs.
        assert_eq!(
            text,
            "{\\1c&H004FE3FF\\2c&H00FFFFFF}{\\k50}你好{\\k50}世界\\N{\\fs44}{\\k100}hello"
        );
    }

    #[test]
    fn approx_tokens_split_cjk_per_char_and_latin_on_whitespace() {
        assert_eq!(approx_tokens("你好世界"), vec!["你", "好", "世", "界"]);
        assert_eq!(approx_tokens("hello world"), vec!["hello", "world"]);
        assert_eq!(approx_tokens("Hello, world"), vec!["Hello,", "world"]);
        // Punctuation glues onto the preceding CJK token.
        assert_eq!(approx_tokens("你好。世界"), vec!["你", "好。", "世", "界"]);
        assert!(approx_tokens("  ").is_empty());
    }

    #[test]
    fn sub_line_scale_is_the_cjk_adjusted_ratio() {
        assert_eq!(CAPTION_SUB_LINE_SCALE, 0.85);
    }

    #[test]
    fn translation_only_cue_gets_approximated_karaoke() {
        let preset = caption_preset("em-yellow").unwrap();
        // The words describe the source language, not this translation text:
        // timing falls back to linear allocation over the cue window [0,1).
        let words = [word("你好", 0.0, 1.0)];
        let text = preset_dialogue_text("hello there", &words, preset, 52, &|t| t).unwrap();
        assert_eq!(
            text,
            "{\\1c&H004FE3FF\\2c&H00FFFFFF}{\\k50}hello {\\k50}there"
        );
        // Line presets never karaoke.
        let line = caption_preset("ln-clean").unwrap();
        assert!(preset_dialogue_text("你好", &words, line, 52, &|t| t).is_none());
    }

    #[test]
    fn untimeable_text_falls_back_to_plain_line() {
        let preset = caption_preset("em-yellow").unwrap();
        let words = [word("你好", 0.0, 1.0)];
        // Empty translation line: nothing to allocate → no dialogue text.
        assert!(preset_dialogue_text("你好世界\n  ", &words, preset, 52, &|t| t).is_none());
        // No words at all: no window to allocate over.
        assert!(preset_dialogue_text("hello", &[], preset, 52, &|t| t).is_none());
        // Zero-length window.
        let frozen = [word("你好", 1.0, 1.0)];
        assert!(preset_dialogue_text("hello", &frozen, preset, 52, &|t| t).is_none());
    }

    #[test]
    fn retiming_collapses_cut_words_to_minimum_duration() {
        let preset = caption_preset("em-yellow").unwrap();
        let words = [word("ab", 0.0, 1.0), word("cd", 1.0, 2.0)];
        // Simulate a cut that removes the second word's span entirely.
        let text = preset_dialogue_text("abcd", &words, preset, 52, &|t| {
            if t >= 1.0 {
                1.0
            } else {
                t
            }
        })
        .unwrap();
        assert!(text.contains("{\\k100}ab {\\k1}cd"));
    }
}
