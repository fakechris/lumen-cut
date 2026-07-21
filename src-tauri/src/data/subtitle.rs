//! Subtitle cue listing, editing, search, and visibility state.
//!
//! Operates on `doc.json` sentences (= cues). Hide/restore state lives in
//! a sibling `hidden.json` so visibility changes do not alter the transcript
//! model.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::data::doc::Doc;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleRow {
    pub id: String,
    pub text: String,
    pub speaker: Option<String>,
    pub hidden: bool,
    pub start: f64,
    pub end: f64,
}

/// One row per sentence. `lang` selects the translation track when set.
pub fn list(doc: &Doc, hidden: &BTreeSet<String>, lang: Option<&str>) -> Vec<SubtitleRow> {
    let mut out = Vec::new();
    for para in &doc.paragraphs {
        for sent in &para.sentences {
            let text = match lang {
                Some(l) => doc
                    .translations
                    .get(l)
                    .and_then(|g| g.get(&sent.id))
                    .map(|g| g.text.clone())
                    .unwrap_or_else(|| sent.text.clone()),
                None => sent.text.clone(),
            };
            let (start, end) = sent
                .words
                .first()
                .zip(sent.words.last())
                .map(|(f, l)| (f.start, l.end))
                .unwrap_or((0.0, 0.0));
            out.push(SubtitleRow {
                id: sent.id.clone(),
                text,
                speaker: para.speaker.clone(),
                hidden: hidden.contains(&sent.id),
                start,
                end,
            });
        }
    }
    out
}

/// Set a sentence's text. Returns `true` if the id was found.
pub fn set(doc: &mut Doc, id: &str, text: &str) -> bool {
    for para in &mut doc.paragraphs {
        for sent in &mut para.sentences {
            if sent.id == id {
                sent.text = text.into();
                sent.words = crate::data::rebind::rebind_corrected(&sent.words, text);
                return true;
            }
        }
    }
    false
}

/// Find subtitles whose text matches `query` (substring, case-insensitive;
/// or a regex when `regex` is set).
pub fn find(doc: &Doc, query: &str, regex: bool) -> AppResult<Vec<SubtitleRow>> {
    let rows = list(doc, &BTreeSet::new(), None);
    if regex {
        let re = regex::Regex::new(query).map_err(|e| AppError::Schema(format!("regex: {e}")))?;
        Ok(rows.into_iter().filter(|r| re.is_match(&r.text)).collect())
    } else {
        let q = query.to_lowercase();
        Ok(rows
            .into_iter()
            .filter(|r| r.text.to_lowercase().contains(&q))
            .collect())
    }
}

#[derive(Serialize, Deserialize, Default)]
pub struct HiddenFile {
    #[serde(default)]
    pub hidden: BTreeSet<String>,
}

pub fn load_hidden(dir: &Path) -> BTreeSet<String> {
    std::fs::read_to_string(dir.join("hidden.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<HiddenFile>(&s).ok())
        .map(|h| h.hidden)
        .unwrap_or_default()
}

pub fn save_hidden(dir: &Path, set: &BTreeSet<String>) -> AppResult<()> {
    std::fs::write(
        dir.join("hidden.json"),
        serde_json::to_string_pretty(&HiddenFile {
            hidden: set.clone(),
        })?,
    )?;
    Ok(())
}

/// Hide a subtitle id. Returns `true` if it was newly hidden.
pub fn hide(dir: &Path, id: &str) -> AppResult<bool> {
    let mut s = load_hidden(dir);
    let new = s.insert(id.to_string());
    save_hidden(dir, &s)?;
    Ok(new)
}

/// Restore a hidden subtitle id. Returns `true` if it was hidden.
pub fn restore(dir: &Path, id: &str) -> AppResult<bool> {
    let mut s = load_hidden(dir);
    let removed = s.remove(id);
    save_hidden(dir, &s)?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::doc::*;

    fn doc_with(sentences: Vec<(&str, &str)>) -> Doc {
        Doc {
            id: "p".into(),
            schema: 1,
            media: MediaRef {
                path: "/tmp/x.mp4".into(),
                duration_seconds: 5.0,
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
                speaker: Some("S1".into()),
                sentences: sentences
                    .into_iter()
                    .map(|(id, text)| Sentence {
                        id: id.into(),
                        text: text.into(),
                        words: vec![Word {
                            id: format!("{id}-w0"),
                            text: text.into(),
                            start: 0.0,
                            end: 1.0,
                        }],
                    })
                    .collect(),
            }],
            translations: Default::default(),
        }
    }

    #[test]
    fn list_one_row_per_sentence() {
        let d = doc_with(vec![("s1", "hello"), ("s2", "world")]);
        let rows = list(&d, &BTreeSet::new(), None);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].speaker.as_deref(), Some("S1"));
    }

    #[test]
    fn set_updates_text() {
        let mut d = doc_with(vec![("s1", "hello")]);
        assert!(set(&mut d, "s1", "hi"));
        assert_eq!(d.paragraphs[0].sentences[0].text, "hi");
        assert_eq!(d.paragraphs[0].sentences[0].words[0].text, "hi");
        assert_eq!(
            (
                d.paragraphs[0].sentences[0].words[0].start,
                d.paragraphs[0].sentences[0].words[0].end
            ),
            (0.0, 1.0)
        );
        assert!(!set(&mut d, "ghost", "x"));
    }

    #[test]
    fn find_substring_and_regex() {
        let d = doc_with(vec![("s1", "hello world"), ("s2", "goodbye world")]);
        assert_eq!(find(&d, "world", false).unwrap().len(), 2);
        assert_eq!(find(&d, "hello", false).unwrap().len(), 1);
        assert_eq!(find(&d, "^hello", true).unwrap().len(), 1);
    }

    #[test]
    fn hide_and_restore_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(hide(tmp.path(), "s1").unwrap());
        assert!(!hide(tmp.path(), "s1").unwrap()); // already hidden
        let h = load_hidden(tmp.path());
        assert!(h.contains("s1"));
        assert!(restore(tmp.path(), "s1").unwrap());
        assert!(!restore(tmp.path(), "s1").unwrap());
    }
}
