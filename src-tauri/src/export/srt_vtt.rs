//! SRT and VTT renderers.  We follow the BCP-47 / `WebVTT` spec just closely
//! enough to round-trip through ffmpeg's subtitle muxer (`-i in.srt -c:s copy`).
//!
//! The two renderers are deliberately tiny (≤120 LOC) — the spec is shallow
//! and a "feature-complete" dependency would weigh more than the spec.

use std::fmt::Write;
use std::path::Path;

use crate::data::doc::Sentence;
use crate::data::soft_cut::Cut;
use crate::data::Doc;
use crate::error::AppResult;

use super::project::{cut_intervals, fully_cut, retime};

/// Format a timestamp as `HH:MM:SS,mmm` (SRT) or `HH:MM:SS.mmm` (VTT).
fn fmt_ts(seconds: f64, sep_comma: bool) -> String {
    let s = seconds.max(0.0);
    let total_ms = (s * 1000.0).round() as u64;
    let h = total_ms / 3_600_000;
    let m = (total_ms / 60_000) % 60;
    let sec = (total_ms / 1000) % 60;
    let ms = total_ms % 1000;
    let frac_sep = if sep_comma { ',' } else { '.' };
    format!("{h:02}:{m:02}:{sec:02}{frac_sep}{ms:03}")
}

/// Escape cue payload per the WebVTT spec (`&`, `<`, `>`). SRT gets the
/// same treatment so players never mis-read cue text as tags/entities.
fn escape_cue(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `[start, end]` of a sentence on the original timeline.
fn cue_window(sent: &Sentence) -> (f64, f64) {
    let s = sent.words.first().map(|w| w.start).unwrap_or(0.0);
    let e = sent.words.last().map(|w| w.end).unwrap_or(s + 1.0);
    (s, e)
}

/// Render `doc.json` to SRT (one cue per ASR sentence).
pub fn to_srt(doc: &Doc) -> String {
    to_srt_with(doc, &[])
}

/// Render with soft-cut projection: cues inside a cut region are dropped,
/// the rest are retimed onto the post-cut timeline.
pub fn to_srt_with(doc: &Doc, cuts: &[Cut]) -> String {
    let iv = cut_intervals(doc, cuts);
    let mut out = String::new();
    let mut n: u32 = 1;
    for para in &doc.paragraphs {
        for sent in &para.sentences {
            if sent.words.is_empty() || sent.text.trim().is_empty() {
                continue;
            }
            let (s, e) = cue_window(sent);
            if fully_cut(s, e, &iv) {
                continue;
            }
            let (ns, ne) = (retime(s, &iv), retime(e, &iv));
            if ne <= ns {
                continue;
            }
            let _ = writeln!(out, "{n}");
            let _ = writeln!(out, "{} --> {}", fmt_ts(ns, true), fmt_ts(ne, true));
            let _ = writeln!(out, "{}", escape_cue(sent.text.trim()));
            let _ = writeln!(out);
            n += 1;
        }
    }
    out
}

/// Render `doc.json` to WebVTT (one cue per ASR sentence).
pub fn to_vtt(doc: &Doc) -> String {
    to_vtt_with(doc, &[])
}

/// Render WebVTT with soft-cut projection.
pub fn to_vtt_with(doc: &Doc, cuts: &[Cut]) -> String {
    let iv = cut_intervals(doc, cuts);
    let mut out = String::from("WEBVTT\n\n");
    for para in &doc.paragraphs {
        for sent in &para.sentences {
            if sent.words.is_empty() || sent.text.trim().is_empty() {
                continue;
            }
            let (s, e) = cue_window(sent);
            if fully_cut(s, e, &iv) {
                continue;
            }
            let (ns, ne) = (retime(s, &iv), retime(e, &iv));
            if ne <= ns {
                continue;
            }
            let _ = writeln!(out, "{} --> {}", fmt_ts(ns, false), fmt_ts(ne, false));
            let _ = writeln!(out, "{}", escape_cue(sent.text.trim()));
            let _ = writeln!(out);
        }
    }
    out
}

/// Write SRT to disk.
pub fn write_srt(doc: &Doc, path: &Path) -> AppResult<()> {
    write_srt_with(doc, &[], path)
}

/// Write SRT with soft-cut projection to disk.
pub fn write_srt_with(doc: &Doc, cuts: &[Cut], path: &Path) -> AppResult<()> {
    crate::data::storage::write(path, to_srt_with(doc, cuts).as_bytes())
}

/// Write VTT to disk.
pub fn write_vtt(doc: &Doc, path: &Path) -> AppResult<()> {
    write_vtt_with(doc, &[], path)
}

/// Write VTT with soft-cut projection to disk.
pub fn write_vtt_with(doc: &Doc, cuts: &[Cut], path: &Path) -> AppResult<()> {
    crate::data::storage::write(path, to_vtt_with(doc, cuts).as_bytes())
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
                duration_seconds: 2.0,
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
                    text: "Hello world.".into(),
                    words: vec![
                        Word {
                            id: "w0".into(),
                            text: "Hello".into(),
                            start: 0.0,
                            end: 0.4,
                        },
                        Word {
                            id: "w1".into(),
                            text: "world.".into(),
                            start: 0.4,
                            end: 0.8,
                        },
                    ],
                }],
            }],
            translations: Default::default(),
        }
    }

    #[test]
    fn srt_format_is_valid() {
        let s = to_srt(&fixture());
        // round-trip through ffplay by checking structural fields.
        assert!(s.contains("1\n00:00:00,000 --> 00:00:00,800\nHello world.\n"));
    }

    #[test]
    fn vtt_format_is_valid() {
        let s = to_vtt(&fixture());
        assert!(s.starts_with("WEBVTT\n"));
        assert!(s.contains("00:00:00.000 --> 00:00:00.800"));
    }

    fn fixture_with_text(text: &str) -> Doc {
        let mut d = fixture();
        d.paragraphs[0].sentences[0].text = text.into();
        d
    }

    #[test]
    fn srt_escapes_cue_text() {
        let s = to_srt(&fixture_with_text("a <b> & c"));
        assert!(s.contains("a &lt;b&gt; &amp; c\n"));
        assert!(!s.contains("<b>"));
    }

    #[test]
    fn vtt_escapes_cue_text() {
        let s = to_vtt(&fixture_with_text("x < y & y > z"));
        assert!(s.contains("x &lt; y &amp; y &gt; z\n"));
        assert!(!s.contains("x < y"));
    }

    #[test]
    fn srt_with_empty_cuts_matches_plain() {
        let d = fixture();
        assert_eq!(to_srt(&d), to_srt_with(&d, &[]));
    }

    #[test]
    fn srt_with_cut_drops_consumed_cue_and_retimes_rest() {
        use crate::data::soft_cut::{Cut, CutKind};
        let mk = |id: &str, s: f64, e: f64| crate::data::Word {
            id: id.into(),
            text: id.into(),
            start: s,
            end: e,
        };
        let d = crate::data::Doc {
            id: "p".into(),
            schema: 1,
            media: crate::data::MediaRef {
                path: PathBuf::from("/tmp/x.mp4"),
                duration_seconds: 5.0,
                sample_rate: Some(16_000),
                channels: Some(1),
            },
            meta: crate::data::Meta {
                title: "t".into(),
                description: String::new(),
                language: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            paragraphs: vec![crate::data::Paragraph {
                id: 1,
                speaker: None,
                sentences: vec![
                    crate::data::Sentence {
                        id: "s1".into(),
                        text: "one".into(),
                        words: vec![mk("w0", 0.0, 1.0)],
                    },
                    crate::data::Sentence {
                        id: "s2".into(),
                        text: "two".into(),
                        words: vec![mk("w1", 1.0, 3.0)],
                    },
                    crate::data::Sentence {
                        id: "s3".into(),
                        text: "three".into(),
                        words: vec![mk("w2", 3.0, 5.0)],
                    },
                ],
            }],
            translations: Default::default(),
        };
        // cut consumes s2 (w1: 1.0..3.0). s1 unchanged; s3 (3..5) → (1..3).
        let cuts = vec![Cut {
            id: "c".into(),
            note: None,
            a_word: "w1".into(),
            b_word: "w1".into(),
            kind: CutKind::Manual,
            duration: 2.0,
        }];
        let s = to_srt_with(&d, &cuts);
        assert!(s.contains("one"));
        assert!(!s.contains("two"));
        assert!(s.contains("three"));
        assert!(s.contains("00:00:01,000 --> 00:00:03,000"));
    }
}
