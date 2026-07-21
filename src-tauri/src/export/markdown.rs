//! Markdown transcript renderer. Like the subtitle renderers, it emits one
//! cue per ASR sentence. When `doc.translations` is populated the source line is
//! followed by a `{lang}: …` line per available translation, so the same
//! `.md` round-trips a bilingual transcript without a second file.
//! The layout uses a heading per paragraph, `[start → end]` per cue, source
//! text, and then translations. It is intentionally easy to read and diff.

use std::fmt::Write;
use std::path::Path;

use crate::data::soft_cut::Cut;
use crate::data::Doc;
use crate::error::AppResult;

use super::project::{cut_intervals, fully_cut, retime};

/// `HH:MM:SS` — second resolution is enough for a reading-oriented
/// Markdown transcript; sub-second precision lives in SRT/VTT/ASS.
fn fmt_ts(seconds: f64) -> String {
    let s = seconds.max(0.0);
    let total = s.round() as u64;
    let h = total / 3_600;
    let m = (total / 60) % 60;
    let sec = total % 60;
    format!("{h:02}:{m:02}:{sec:02}")
}

/// Render `doc.json` to a Markdown transcript. Deterministic.
pub fn to_md(doc: &Doc) -> String {
    to_md_with(doc, &[])
}

/// Render Markdown with soft-cut projection.
pub fn to_md_with(doc: &Doc, cuts: &[Cut]) -> String {
    let iv = cut_intervals(doc, cuts);
    let mut out = String::new();
    let _ = writeln!(out, "# {}\n", doc.meta.title.trim());
    if !doc.meta.description.is_empty() {
        let _ = writeln!(out, "> {}\n", doc.meta.description.trim());
    }

    for para in &doc.paragraphs {
        let header = match &para.speaker {
            Some(s) if !s.trim().is_empty() => format!("段落 {} — {s}", para.id),
            _ => format!("段落 {}", para.id),
        };
        let _ = writeln!(out, "## {header}\n");

        for sent in &para.sentences {
            if sent.words.is_empty() {
                continue;
            }
            let start = sent.words.first().map(|w| w.start).unwrap_or(0.0);
            let end = sent.words.last().map(|w| w.end).unwrap_or(start);
            if fully_cut(start, end, &iv) {
                continue;
            }
            let (ns, ne) = (retime(start, &iv), retime(end, &iv));
            if ne <= ns {
                continue;
            }
            let _ = writeln!(out, "**[{} → {}]**", fmt_ts(ns), fmt_ts(ne));
            let _ = writeln!(out, "{}\n", sent.text.trim());

            // Bilingual: one line per populated translation, in lang order
            // (BTreeMap keeps the ordering stable).
            for (lang, groups) in &doc.translations {
                if let Some(g) = groups.get(&sent.id) {
                    if !g.text.trim().is_empty() {
                        let _ = writeln!(out, "{lang}: {}\n", g.text.trim());
                    }
                }
            }
        }
    }
    out
}

/// Write Markdown to disk.
pub fn write_md(doc: &Doc, path: &Path) -> AppResult<()> {
    write_md_with(doc, &[], path)
}

/// Write Markdown with soft-cut projection to disk.
pub fn write_md_with(doc: &Doc, cuts: &[Cut], path: &Path) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, to_md_with(doc, cuts))?;
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Chapter {
    title: String,
    start_seg: String,
}

/// Write a project-aware Markdown export. Chapter metadata is stored in the
/// native top-level `doc.chapters` shape and mirrored in `chapters.json`; this
/// renderer consumes the mirror so its API remains compatible with imported
/// flat-cue documents.
pub fn write_md_with_chapters(
    doc: &Doc,
    cuts: &[Cut],
    project_dir: &Path,
    path: &Path,
) -> AppResult<()> {
    let mut markdown = to_md_with(doc, cuts);
    let chapters: Vec<Chapter> = std::fs::read_to_string(project_dir.join("chapters.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    if !chapters.is_empty() {
        let starts: std::collections::BTreeMap<&str, f64> = doc
            .paragraphs
            .iter()
            .flat_map(|paragraph| paragraph.sentences.iter())
            .map(|sentence| {
                (
                    sentence.id.as_str(),
                    sentence
                        .words
                        .first()
                        .map(|word| word.start)
                        .unwrap_or_default(),
                )
            })
            .collect();
        let _ = writeln!(markdown, "## Chapters\n");
        for chapter in chapters {
            if let Some(start) = starts.get(chapter.start_seg.as_str()) {
                let _ = writeln!(
                    markdown,
                    "- [{}] {}",
                    fmt_ts(retime(*start, &cut_intervals(doc, cuts))),
                    chapter.title.trim()
                );
            }
        }
        markdown.push('\n');
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, markdown)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::doc::*;
    use chrono::Utc;
    use std::collections::BTreeMap;
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
                title: "Demo".into(),
                description: "a talk".into(),
                language: Some("en".into()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            paragraphs: vec![Paragraph {
                id: 1,
                speaker: Some("S1".into()),
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
    fn md_has_title_para_and_cue_window() {
        let s = to_md(&fixture());
        assert!(s.contains("# Demo"));
        assert!(s.contains("段落 1 — S1"));
        assert!(s.contains("**[00:00:00 → 00:00:01]**"));
        assert!(s.contains("Hello world."));
    }

    #[test]
    fn md_appends_translation_line_when_bilingual() {
        let mut d = fixture();
        d.translations.insert(
            "zh".into(),
            BTreeMap::from([(
                "s1".into(),
                TranslationGroup {
                    id: "s1".into(),
                    text: "你好，世界。".into(),
                    source_words: vec![],
                    source_text: None,
                },
            )]),
        );
        let s = to_md(&d);
        assert!(s.contains("zh: 你好，世界。"));
        // source still present above the translation
        assert!(s.find("Hello world.").unwrap() < s.find("zh: 你好").unwrap());
    }

    #[test]
    fn write_md_roundtrips_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("out.md");
        write_md(&fixture(), &p).unwrap();
        let on_disk = std::fs::read_to_string(p).unwrap();
        assert!(on_disk.contains("# Demo"));
    }

    #[test]
    fn project_markdown_includes_resolved_chapter_timestamps() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("chapters.json"),
            r#"[{"title":"Intro","startSeg":"s1"}]"#,
        )
        .unwrap();
        let path = dir.path().join("out.md");
        write_md_with_chapters(&fixture(), &[], dir.path(), &path).unwrap();
        let markdown = std::fs::read_to_string(path).unwrap();
        assert!(markdown.contains("## Chapters"));
        assert!(markdown.contains("- [00:00:00] Intro"));
    }
}
