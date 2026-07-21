//! Minimal ASS ("Advanced SubStation Alpha") burn-in renderer.
//!
//! The aim is "ffmpeg accepts what we write" — a sane subset of the spec —
//! not a full implementation. Stage 5 will refine styling per project.

use std::fmt::Write;
use std::path::Path;

use crate::data::soft_cut::Cut;
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
    let _ = writeln!(
        out,
        "{}",
        crate::data::substyle::SubStyle::default().ass_style_line()
    );
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
            let _ = writeln!(
                out,
                "Dialogue: 0,{},{},Default,,0,0,0,,{}",
                fmt(ns),
                fmt(ne),
                text
            );
        }
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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, to_ass_with(doc, cuts, width, height))?;
    Ok(())
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
}
