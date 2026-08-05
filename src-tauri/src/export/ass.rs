//! Minimal ASS ("Advanced SubStation Alpha") burn-in renderer.
//!
//! The aim is "ffmpeg accepts what we write" — a sane subset of the spec —
//! not a full implementation. Stage 5 will refine styling per project.

use std::fmt::Write;
use std::path::Path;

use crate::data::soft_cut::Cut;
use crate::data::substyle::SubStyle;
use crate::data::title::TitleClip;
use crate::data::Doc;
use crate::error::AppResult;

use super::project::{cut_intervals, fully_cut, retime};

/// Render `doc.json` to a minimal but valid ASS script.
pub fn to_ass(doc: &Doc, width: u32, height: u32) -> String {
    to_ass_with(doc, &[], width, height)
}

/// Render ASS with soft-cut projection: cues inside a cut are dropped, the
/// rest are retimed onto the post-cut timeline.
pub fn to_ass_with(doc: &Doc, cuts: &[Cut], width: u32, height: u32) -> String {
    to_ass_with_style(doc, cuts, &SubStyle::default(), width, height)
}

pub fn to_ass_with_style(
    doc: &Doc,
    cuts: &[Cut],
    style: &SubStyle,
    width: u32,
    height: u32,
) -> String {
    to_ass_with_style_and_titles(doc, cuts, style, &[], width, height)
}

pub fn to_ass_with_titles(
    doc: &Doc,
    cuts: &[Cut],
    titles: &[TitleClip],
    width: u32,
    height: u32,
) -> String {
    to_ass_with_style_and_titles(doc, cuts, &SubStyle::default(), titles, width, height)
}

pub fn to_ass_with_style_and_titles(
    doc: &Doc,
    cuts: &[Cut],
    style: &SubStyle,
    titles: &[TitleClip],
    width: u32,
    height: u32,
) -> String {
    to_ass_with_titles_impl(doc, cuts, style, titles, width, height, true)
}

pub fn to_ass_titles_only(
    doc: &Doc,
    cuts: &[Cut],
    titles: &[TitleClip],
    width: u32,
    height: u32,
) -> String {
    to_ass_titles_only_with_style(doc, cuts, &SubStyle::default(), titles, width, height)
}

pub fn to_ass_titles_only_with_style(
    doc: &Doc,
    cuts: &[Cut],
    style: &SubStyle,
    titles: &[TitleClip],
    width: u32,
    height: u32,
) -> String {
    to_ass_with_titles_impl(doc, cuts, style, titles, width, height, false)
}

fn to_ass_with_titles_impl(
    doc: &Doc,
    cuts: &[Cut],
    style: &SubStyle,
    titles: &[TitleClip],
    width: u32,
    height: u32,
    include_transcript: bool,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "[Script Info]");
    let _ = writeln!(out, "Title: lumen-cut export");
    let _ = writeln!(out, "ScriptType: v4.00+");
    let _ = writeln!(out, "WrapStyle: 0");
    let _ = writeln!(out, "ScaledBorderAndShadow: yes");
    let _ = writeln!(out, "PlayResX: {width}");
    let _ = writeln!(out, "PlayResY: {height}");
    let _ = writeln!(out);
    let _ = writeln!(out, "[V4+ Styles]");
    let _ = writeln!(
        out,
        "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding"
    );
    // Project style names are UI labels. Keep the ASS event style identifier
    // stable so commas or renamed presets cannot disconnect cues from their style.
    let mut render_style = style.clone();
    render_style.name = "Default".into();
    // Caption preset (pireel port): overrides the look — colors / typeface /
    // italic / backing box. Size, bold, alignment and margins stay user-owned.
    let preset = style
        .caption_preset
        .as_deref()
        .and_then(crate::data::caption_presets::caption_preset);
    let mut border_style = 1;
    if let Some(p) = preset {
        if let Some(primary) = crate::data::caption_presets::css_color_to_ass(p.text) {
            render_style.primary_colour = primary;
        }
        if let Some(fontname) = crate::data::caption_presets::preset_fontname(p) {
            render_style.fontname = fontname.into();
        }
        render_style.italic |= p.italic;
        // Per-word underline/highlight decorations have no per-word ASS
        // equivalent — underline approximates to the whole-line flag, and the
        // highlight box relies on the preset's full-line backing instead.
        if p.deco == Some(crate::data::caption_presets::Deco::Underline) {
            render_style.underline = true;
        }
        if let Some(box_color) =
            p.bg.and_then(crate::data::caption_presets::css_color_to_ass)
        {
            // BorderStyle 3 = opaque box: OutlineColour becomes the backing and
            // Outline its padding (~0.25em, mirroring the preset pill padding).
            // Backed text gets no drop shadow (the preset rule for bare vs backed).
            render_style.outline_colour = box_color;
            render_style.outline = (render_style.fontsize / 4).max(4);
            render_style.shadow = 0;
            border_style = 3;
        }
    }
    let _ = writeln!(out, "{}", render_style.ass_style_line_with(border_style));
    let _ = writeln!(out);
    let _ = writeln!(out, "[Events]");
    let _ = writeln!(
        out,
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text"
    );

    let fmt = |t: f64| {
        let h = (t / 3600.0) as u32;
        let m = ((t / 60.0) % 60.0) as u32;
        let s = (t % 60.0) as u32;
        let cs = ((t * 100.0) % 100.0) as u32;
        format!("{h:01}:{m:02}:{s:02}.{cs:02}")
    };

    let iv = cut_intervals(doc, cuts);
    if include_transcript {
        for para in &doc.paragraphs {
            for sent in &para.sentences {
                if sent.words.is_empty() {
                    continue;
                }
                let start = sent.words.first().map(|w| w.start).unwrap_or(0.0);
                let end = sent.words.last().map(|w| w.end).unwrap_or(start + 1.0);
                if fully_cut(start, end, &iv) {
                    continue;
                }
                let (ns, ne) = (retime(start, &iv), retime(end, &iv));
                if ne <= ns {
                    continue;
                }
                let trimmed = sent.text.trim();
                if trimmed.is_empty() {
                    continue;
                }
                // Preset captions become one Dialogue per line (main + optional
                // translation sub-line): \k durations accumulate from the event
                // start, so a shared event would karaoke the sub-line only after
                // the main line finished, and a BorderStyle-3 backing boxes one
                // event instead of hugging each line like the preview's pills.
                let caption = preset.and_then(|p| {
                    crate::data::caption_presets::preset_caption_lines(
                        trimmed,
                        &sent.words,
                        p,
                        render_style.fontsize,
                        if border_style == 3 {
                            render_style.outline
                        } else {
                            0
                        },
                        &|t| retime(t, &iv),
                    )
                });
                match caption {
                    Some(caption) => {
                        // 0 in the event margin fields = take the style value;
                        // the main line lifts above the sub-line block instead.
                        let main_margin_v = if caption.main_margin_lift > 0 {
                            render_style.margin_v + caption.main_margin_lift
                        } else {
                            0
                        };
                        let _ = writeln!(
                            out,
                            "Dialogue: 0,{},{},Default,,0,0,{},,{}",
                            fmt(ns),
                            fmt(ne),
                            main_margin_v,
                            caption.main
                        );
                        if let Some(sub) = caption.sub {
                            let _ = writeln!(
                                out,
                                "Dialogue: 0,{},{},Default,,0,0,0,,{}",
                                fmt(ns),
                                fmt(ne),
                                sub
                            );
                        }
                    }
                    None => {
                        let _ = writeln!(
                            out,
                            "Dialogue: 0,{},{},Default,,0,0,0,,{}",
                            fmt(ns),
                            fmt(ne),
                            trimmed.replace('\n', "\\N")
                        );
                    }
                }
            }
        }
    }
    for title in titles {
        if fully_cut(title.start, title.end, &iv) {
            continue;
        }
        let start = retime(title.start, &iv);
        let end = retime(title.end, &iv);
        if end <= start {
            continue;
        }
        let x = (title.x * width as f64).round() as u32;
        let y = (title.y * height as f64).round() as u32;
        let color = crate::data::title::ass_color(&title.color);
        let background = crate::data::title::ass_color(&title.background);
        let text = crate::data::title::ass_text(&title.text);
        let duration = end - start;
        let fade_in = title.fade_in.min(duration);
        let fade_out = title.fade_out.min((duration - fade_in).max(0.0));
        let fade_in_ms = (fade_in * 1000.0).round() as u64;
        let fade_out_ms = (fade_out * 1000.0).round() as u64;
        let _ = writeln!(
            out,
            "Dialogue: 3,{},{},Default,,0,0,0,,{{\\an5\\pos({x},{y})\\fs{}\\1c{color}\\3c{background}\\bord12\\shad0\\fad({fade_in_ms},{fade_out_ms})}}{text}",
            fmt(start),
            fmt(end),
            title.font_size,
        );
    }
    out
}

pub fn write_ass(doc: &Doc, path: &Path, width: u32, height: u32) -> AppResult<()> {
    write_ass_with(doc, &[], path, width, height)
}

/// Write ASS with soft-cut projection to disk.
pub fn write_ass_with(
    doc: &Doc,
    cuts: &[Cut],
    path: &Path,
    width: u32,
    height: u32,
) -> AppResult<()> {
    crate::data::storage::write(path, to_ass_with(doc, cuts, width, height).as_bytes())
}

pub fn write_ass_with_style(
    doc: &Doc,
    cuts: &[Cut],
    style: &SubStyle,
    path: &Path,
    width: u32,
    height: u32,
) -> AppResult<()> {
    crate::data::storage::write(
        path,
        to_ass_with_style(doc, cuts, style, width, height).as_bytes(),
    )
}

pub fn write_ass_with_titles(
    doc: &Doc,
    cuts: &[Cut],
    titles: &[TitleClip],
    path: &Path,
    width: u32,
    height: u32,
) -> AppResult<()> {
    crate::data::storage::write(
        path,
        to_ass_with_titles(doc, cuts, titles, width, height).as_bytes(),
    )
}

pub fn write_ass_with_style_and_titles(
    doc: &Doc,
    cuts: &[Cut],
    style: &SubStyle,
    titles: &[TitleClip],
    path: &Path,
    width: u32,
    height: u32,
) -> AppResult<()> {
    crate::data::storage::write(
        path,
        to_ass_with_style_and_titles(doc, cuts, style, titles, width, height).as_bytes(),
    )
}

pub fn write_ass_titles_only(
    doc: &Doc,
    cuts: &[Cut],
    titles: &[TitleClip],
    path: &Path,
    width: u32,
    height: u32,
) -> AppResult<()> {
    crate::data::storage::write(
        path,
        to_ass_titles_only(doc, cuts, titles, width, height).as_bytes(),
    )
}

pub fn write_ass_titles_only_with_style(
    doc: &Doc,
    cuts: &[Cut],
    style: &SubStyle,
    titles: &[TitleClip],
    path: &Path,
    width: u32,
    height: u32,
) -> AppResult<()> {
    crate::data::storage::write(
        path,
        to_ass_titles_only_with_style(doc, cuts, style, titles, width, height).as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::*;
    use std::path::PathBuf;

    fn fixture() -> Doc {
        Doc {
            id: "p".into(),
            schema: 1,
            media: MediaRef {
                path: PathBuf::from("/tmp/x.mp4"),
                duration_seconds: 0.8,
                sample_rate: Some(16_000),
                channels: Some(1),
            },
            meta: Meta {
                title: "t".into(),
                description: String::new(),
                language: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            paragraphs: vec![Paragraph {
                id: 1,
                speaker: None,
                sentences: vec![Sentence {
                    id: "s1".into(),
                    text: "Hi".into(),
                    words: vec![Word {
                        id: "w0".into(),
                        text: "Hi".into(),
                        start: 0.0,
                        end: 0.5,
                    }],
                }],
            }],
            translations: Default::default(),
        }
    }

    #[test]
    fn ass_header_present() {
        let s = to_ass(&fixture(), 1920, 1080);
        assert!(s.contains("[Script Info]"));
        assert!(s.contains("PlayResX: 1920"));
        assert!(s.contains("Dialogue: 0,0:00:00.00,0:00:00.50,Default,,0,0,0,,Hi"));
    }

    #[test]
    fn titles_only_keeps_cut_retiming_without_burning_transcript_cues() {
        let title = TitleClip {
            id: "title-1".into(),
            text: "After cut".into(),
            start: 0.5,
            end: 0.8,
            x: 0.5,
            y: 0.2,
            font_size: 64,
            color: "#FFFFFF".into(),
            background: "#00000099".into(),
            fade_in: 0.0,
            fade_out: 0.0,
        };
        let cut = Cut {
            id: "cut-1".into(),
            note: None,
            a_word: "w0".into(),
            b_word: "w0".into(),
            kind: CutKind::Manual,
            duration: 0.5,
        };

        let output = to_ass_titles_only(&fixture(), &[cut], &[title], 1920, 1080);
        assert!(!output.contains("Dialogue: 0,"));
        assert!(output.contains("Dialogue: 3,0:00:00.00,0:00:00.30"));
        assert!(output.contains("After cut"));
    }

    #[test]
    fn project_style_is_rendered_under_the_stable_default_ass_identifier() {
        let style = SubStyle {
            name: "Creator, yellow".into(),
            fontname: "PingFang SC".into(),
            fontsize: 64,
            primary_colour: "&H0000E8FF".into(),
            outline_colour: "&H00141414".into(),
            bold: true,
            alignment: 8,
            outline: 4,
            shadow: 1,
            margin_v: 96,
            ..Default::default()
        };

        let output = to_ass_with_style(&fixture(), &[], &style, 1920, 1080);
        assert!(output.contains("Style: Default,PingFang SC,64,&H0000E8FF,&H000000FF,&H00141414"));
        assert!(output.contains(",-1,0,0,0,100,100,0,0,1,4,1,8,40,40,96,1"));
        assert!(output.contains("Dialogue: 0,"));
        assert!(!output.contains("Style: Creator, yellow"));
    }

    #[test]
    fn emphasis_preset_karaokes_words_with_ass_k_tags() {
        let style = SubStyle {
            caption_preset: Some("em-yellow".into()),
            ..Default::default()
        };
        let output = to_ass_with_style(&fixture(), &[], &style, 1920, 1080);
        // Sung words switch to the emphasis colour, unsung stay body white.
        assert!(output.contains("{\\1c&H004FE3FF\\2c&H00FFFFFF}{\\k50}Hi"));
    }

    #[test]
    fn backing_preset_renders_as_opaque_box_without_shadow() {
        let style = SubStyle {
            caption_preset: Some("ln-black".into()),
            ..Default::default()
        };
        let output = to_ass_with_style(&fixture(), &[], &style, 1920, 1080);
        // rgba(0,0,0,0.85) → &H26000000 as box colour; BorderStyle 3, padding
        // 52/4 = 13, no shadow. Line presets never karaoke.
        assert!(output.contains("Style: Default,Arial,52,&H00FFFFFF,&H000000FF,&H26000000,&H00000000,0,0,0,0,100,100,0,0,3,13,0,2,40,40,80,1"));
        assert!(!output.contains("{\\k"));
    }

    #[test]
    fn preset_overrides_typeface_and_text_colour() {
        let style = SubStyle {
            caption_preset: Some("em-gold-serif".into()),
            ..Default::default()
        };
        let output = to_ass_with_style(&fixture(), &[], &style, 1920, 1080);
        // The serif preset must name a face the host actually ships, or
        // libass silently substitutes: Songti SC on macOS (unlike Noto Serif
        // SC, which fontconfig fell back from), SimSun on Windows.
        let serif = if cfg!(windows) { "SimSun" } else { "Songti SC" };
        assert!(output.contains(&format!("Style: Default,{serif},52,&H004C9DB8,")));
    }

    #[test]
    fn bilingual_preset_cue_emits_two_dialogues_with_independent_karaoke() {
        let mut doc = fixture();
        doc.paragraphs[0].sentences[0].text = "Hi\n你好".into();
        let style = SubStyle {
            caption_preset: Some("em-yellow".into()),
            ..Default::default()
        };
        let output = to_ass_with_style(&doc, &[], &style, 1920, 1080);
        let dialogues: Vec<&str> = output
            .lines()
            .filter(|line| line.starts_with("Dialogue: 0,"))
            .collect();
        assert_eq!(dialogues.len(), 2);
        // Main line: real timing, MarginV lifted above the sub-line block
        // (80 + 44×1.4 ≈ 62 = 142, no backing so no box padding).
        assert!(dialogues[0].contains(",0,0,142,,{\\1c&H004FE3FF\\2c&H00FFFFFF}{\\k50}Hi"));
        // Sub-line: its own event with \fs and an independent \k sweep
        // (approximated per CJK char over the [0,0.5) window → 25cs each).
        assert!(
            dialogues[1].contains(",0,0,0,,{\\1c&H004FE3FF\\2c&H00FFFFFF\\fs44}{\\k25}你{\\k25}好")
        );
    }

    #[test]
    fn unknown_preset_id_keeps_the_plain_style() {
        let style = SubStyle {
            caption_preset: Some("bogus".into()),
            ..Default::default()
        };
        let output = to_ass_with_style(&fixture(), &[], &style, 1920, 1080);
        assert!(output.contains("Style: Default,Arial,52,&H00FFFFFF"));
        assert!(!output.contains("{\\k"));
    }
}
