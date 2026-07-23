//! Subtitle cue listing, editing, search, and visibility state.
//!
//! Operates on `doc.json` sentences (= cues). Hide/restore state lives in
//! a sibling `hidden.json` so visibility changes do not alter the transcript
//! model.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::data::doc::{Doc, TranslationGroup};
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

/// Apply a batch of text edits in one document traversal.
///
/// The command layer validates ids before calling this function. A map makes
/// large "save all" operations O(cues + updates) instead of scanning every cue
/// once for every update.
pub fn set_many(doc: &mut Doc, updates: &HashMap<&str, &str>) -> usize {
    let mut changed = 0;
    for paragraph in &mut doc.paragraphs {
        for sentence in &mut paragraph.sentences {
            let Some(text) = updates.get(sentence.id.as_str()) else {
                continue;
            };
            if sentence.text == *text {
                continue;
            }
            sentence.text = (*text).into();
            sentence.words = crate::data::rebind::rebind_corrected(&sentence.words, text);
            changed += 1;
        }
    }
    changed
}

/// Retimes one cue while preserving every real word boundary proportionally.
/// The neighboring cue window is authoritative: timing edits may use silence
/// around a cue but can never overlap another cue or leave the media range.
pub fn set_timing(doc: &mut Doc, id: &str, start: f64, end: f64) -> AppResult<bool> {
    if !start.is_finite() || !end.is_finite() || start < 0.0 || end - start < 0.1 {
        return Err(AppError::Schema(
            "subtitle timing must be finite, nonnegative, and at least 0.1s long".into(),
        ));
    }
    if end > doc.media.duration_seconds + 0.001 {
        return Err(AppError::Schema(format!(
            "subtitle ends after the media ({end:.3}s > {:.3}s)",
            doc.media.duration_seconds
        )));
    }

    let cues = doc
        .paragraphs
        .iter()
        .flat_map(|paragraph| paragraph.sentences.iter())
        .filter_map(|sentence| {
            sentence
                .words
                .first()
                .zip(sentence.words.last())
                .map(|(first, last)| (sentence.id.as_str(), first.start, last.end))
        })
        .collect::<Vec<_>>();
    let index = cues
        .iter()
        .position(|(cue_id, _, _)| *cue_id == id)
        .ok_or_else(|| AppError::Schema(format!("subtitle cue `{id}` was not found")))?;
    let earliest = index
        .checked_sub(1)
        .and_then(|previous| cues.get(previous))
        .map(|(_, _, previous_end)| *previous_end)
        .unwrap_or(0.0);
    let latest = cues
        .get(index + 1)
        .map(|(_, next_start, _)| *next_start)
        .unwrap_or(doc.media.duration_seconds);
    if start + 0.001 < earliest || end > latest + 0.001 {
        return Err(AppError::Schema(format!(
            "subtitle timing must stay inside the available {:.3}s–{:.3}s window",
            earliest, latest
        )));
    }

    let sentence = doc
        .paragraphs
        .iter_mut()
        .flat_map(|paragraph| paragraph.sentences.iter_mut())
        .find(|sentence| sentence.id == id)
        .ok_or_else(|| AppError::Schema(format!("subtitle cue `{id}` was not found")))?;
    let (old_start, old_end) = sentence
        .words
        .first()
        .zip(sentence.words.last())
        .map(|(first, last)| (first.start, last.end))
        .ok_or_else(|| AppError::Schema(format!("subtitle cue `{id}` has no word timing")))?;
    if old_end - old_start <= f64::EPSILON {
        return Err(AppError::Schema(format!(
            "subtitle cue `{id}` has invalid source timing"
        )));
    }
    if (old_start - start).abs() <= 0.000_001 && (old_end - end).abs() <= 0.000_001 {
        return Ok(false);
    }

    let scale = (end - start) / (old_end - old_start);
    for word in &mut sentence.words {
        word.start = start + (word.start - old_start) * scale;
        word.end = start + (word.end - old_start) * scale;
    }
    Ok(true)
}

/// Set one translated cue while recording the source snapshot used by stale
/// translation detection. Returns `false` when the source cue does not exist.
pub fn set_translation(doc: &mut Doc, lang: &str, id: &str, text: &str) -> bool {
    let source = doc
        .paragraphs
        .iter()
        .flat_map(|paragraph| paragraph.sentences.iter())
        .find(|sentence| sentence.id == id)
        .map(|sentence| {
            (
                sentence.text.clone(),
                sentence.words.iter().map(|word| word.id.clone()).collect(),
            )
        });
    let Some((source_text, source_words)) = source else {
        return false;
    };
    doc.translations
        .entry(lang.to_string())
        .or_default()
        .insert(
            id.to_string(),
            TranslationGroup {
                id: id.to_string(),
                text: text.to_string(),
                source_words,
                source_text: Some(source_text),
            },
        );
    true
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

pub fn load_hidden_checked(dir: &Path) -> AppResult<BTreeSet<String>> {
    let path = dir.join("hidden.json");
    if !path.exists() {
        return Ok(BTreeSet::new());
    }
    Ok(serde_json::from_str::<HiddenFile>(&std::fs::read_to_string(path)?)?.hidden)
}

pub fn load_hidden(dir: &Path) -> BTreeSet<String> {
    load_hidden_checked(dir).unwrap_or_default()
}

pub fn save_hidden(dir: &Path, set: &BTreeSet<String>) -> AppResult<()> {
    crate::data::storage::write_json(
        &dir.join("hidden.json"),
        &HiddenFile {
            hidden: set.clone(),
        },
    )
}

/// Hide a subtitle id. Returns `true` if it was newly hidden.
pub fn hide(dir: &Path, id: &str) -> AppResult<bool> {
    let mut s = load_hidden_checked(dir)?;
    let new = s.insert(id.to_string());
    save_hidden(dir, &s)?;
    Ok(new)
}

/// Restore a hidden subtitle id. Returns `true` if it was hidden.
pub fn restore(dir: &Path, id: &str) -> AppResult<bool> {
    let mut s = load_hidden_checked(dir)?;
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
    fn set_many_updates_each_matching_cue_once() {
        let mut d = doc_with(vec![("s1", "hello"), ("s2", "world"), ("s3", "same")]);
        let updates = HashMap::from([("s1", "hello there"), ("s2", "world again"), ("s3", "same")]);

        assert_eq!(set_many(&mut d, &updates), 2);
        assert_eq!(d.paragraphs[0].sentences[0].text, "hello there");
        assert_eq!(
            d.paragraphs[0].sentences[1]
                .words
                .iter()
                .map(|word| word.text.as_str())
                .collect::<Vec<_>>(),
            vec!["world", "again"],
        );
        assert_eq!(d.paragraphs[0].sentences[2].text, "same");
    }

    #[test]
    fn timing_edit_preserves_word_boundaries_and_rejects_neighbor_overlap() {
        let mut doc = doc_with(vec![("s1", "hello"), ("s2", "world")]);
        doc.paragraphs[0].sentences[0].words = vec![
            Word {
                id: "w1".into(),
                text: "hello".into(),
                start: 0.5,
                end: 1.0,
            },
            Word {
                id: "w2".into(),
                text: "there".into(),
                start: 1.0,
                end: 1.5,
            },
        ];
        doc.paragraphs[0].sentences[1].words[0].start = 3.0;
        doc.paragraphs[0].sentences[1].words[0].end = 4.0;

        assert!(set_timing(&mut doc, "s1", 1.0, 3.0).unwrap());
        let words = &doc.paragraphs[0].sentences[0].words;
        assert_eq!((words[0].start, words[0].end), (1.0, 2.0));
        assert_eq!((words[1].start, words[1].end), (2.0, 3.0));
        assert!(set_timing(&mut doc, "s1", 1.0, 3.1).is_err());
        assert!(set_timing(&mut doc, "s2", 2.9, 4.0).is_err());
    }

    #[test]
    fn set_translation_records_source_snapshot() {
        let mut d = doc_with(vec![("s1", "hello")]);
        assert!(set_translation(&mut d, "zh", "s1", "你好"));
        let saved = &d.translations["zh"]["s1"];
        assert_eq!(saved.text, "你好");
        assert_eq!(saved.source_text.as_deref(), Some("hello"));
        assert_eq!(saved.source_words, vec!["s1-w0"]);
        assert!(!set_translation(&mut d, "zh", "ghost", "不存在"));
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

    #[test]
    fn corrupt_visibility_state_is_never_treated_as_an_empty_selection() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hidden.json"), "{").unwrap();
        assert!(load_hidden_checked(dir.path()).is_err());
        assert!(hide(dir.path(), "s1").is_err());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("hidden.json")).unwrap(),
            "{"
        );
    }
}
