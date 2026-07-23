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
    let _ = writeln!(out, "{}", render_style.ass_style_line());
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
                let text = sent.text.trim().replace('\n', "\\N");
                if text.is_empty() {
                    continue;
                }
                let _ = writeln!(
                    out,
                    "Dialogue: 0,{},{},Default,,0,0,0,,{}",
                    fmt(ns),
                    fmt(ne),
                    text
                );
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
}
