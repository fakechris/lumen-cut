//! Flat `cues[]` interoperability model.
//!
//! A cue contains `{id, startMs, endMs, text, translation, speaker, hidden}`.
//! lumen-cut keeps a paragraph-grouped working model internally; this module
//! converts between that model and a portable flat cue list.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::data::doc::{Doc, MediaRef, Meta, Paragraph, Sentence, Word};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Cue {
    pub id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    #[serde(default)]
    pub translation: Option<String>,
    #[serde(default)]
    pub speaker: Option<String>,
    #[serde(default)]
    pub hidden: bool,
}

/// Flatten a doc into a flat cue list. `lang` selects the translation
/// track for each cue's `translation` when set.
pub fn to_cues(doc: &Doc, lang: Option<&str>) -> Vec<Cue> {
    let mut out = Vec::new();
    for para in &doc.paragraphs {
        for sent in &para.sentences {
            let (start, end) = sent
                .words
                .first()
                .zip(sent.words.last())
                .map(|(f, l)| (f.start, l.end))
                .unwrap_or((0.0, 0.0));
            let translation = lang
                .and_then(|l| doc.translations.get(l).and_then(|g| g.get(&sent.id)))
                .map(|g| g.text.clone());
            out.push(Cue {
                id: sent.id.clone(),
                start_ms: (start * 1000.0).round() as i64,
                end_ms: (end * 1000.0).round() as i64,
                text: sent.text.clone(),
                translation,
                speaker: para.speaker.clone(),
                hidden: false,
            });
        }
    }
    out
}

/// Build a doc from a flat cue list, grouping consecutive cues by speaker
/// into paragraphs.
pub fn from_cues(cues: &[Cue]) -> Doc {
    let mut paragraphs: Vec<Paragraph> = Vec::new();
    let mut cur: Option<Paragraph> = None;
    for c in cues {
        let start = c.start_ms as f64 / 1000.0;
        let end = c.end_ms as f64 / 1000.0;
        let words = vec![Word {
            id: format!("{}-w0", c.id),
            text: c.text.clone(),
            start,
            end,
        }];
        let sent = Sentence {
            id: c.id.clone(),
            text: c.text.clone(),
            words,
        };
        let same_speaker = cur.is_some()
            && cur.as_ref().and_then(|p| p.speaker.as_deref()) == c.speaker.as_deref();
        if same_speaker {
            if let Some(p) = cur.as_mut() {
                p.sentences.push(sent);
            }
        } else {
            if let Some(p) = cur.take() {
                paragraphs.push(p);
            }
            cur = Some(Paragraph {
                id: paragraphs.len() as u32 + 1,
                speaker: c.speaker.clone(),
                sentences: vec![sent],
            });
        }
    }
    if let Some(p) = cur {
        paragraphs.push(p);
    }
    let duration = cues.last().map(|c| c.end_ms as f64 / 1000.0).unwrap_or(0.0);
    let mut doc = Doc {
        id: "imported".into(),
        schema: 1,
        media: MediaRef {
            path: Default::default(),
            duration_seconds: duration,
            sample_rate: None,
            channels: None,
        },
        meta: Meta {
            title: "imported".into(),
            description: String::new(),
            language: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        },
        paragraphs,
        translations: Default::default(),
    };
    let translated = doc.translations.entry("translated".into()).or_default();
    for cue in cues {
        if let Some(text) = cue.translation.as_deref().filter(|text| !text.is_empty()) {
            translated.insert(
                cue.id.clone(),
                crate::data::doc::TranslationGroup {
                    id: cue.id.clone(),
                    text: text.to_string(),
                    source_words: vec![format!("{}-w0", cue.id)],
                    source_text: Some(cue.text.clone()),
                },
            );
        }
    }
    if translated.is_empty() {
        doc.translations.remove("translated");
    }
    doc
}

/// Load `<dir>/cues.json` if present.
pub fn load(dir: &Path) -> Vec<Cue> {
    std::fs::read_to_string(dir.join("cues.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Save the cue list to `<dir>/cues.json`.
pub fn save(dir: &Path, cues: &[Cue]) -> crate::error::AppResult<()> {
    crate::data::storage::write_json(&dir.join("cues.json"), cues)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Doc {
        Doc {
            id: "p".into(),
            schema: 1,
            media: MediaRef {
                path: "/tmp/x".into(),
                duration_seconds: 4.0,
                sample_rate: None,
                channels: None,
            },
            meta: Meta {
                title: "t".into(),
                description: String::new(),
                language: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            paragraphs: vec![
                Paragraph {
                    id: 1,
                    speaker: Some("A".into()),
                    sentences: vec![Sentence {
                        id: "s1".into(),
                        text: "hello".into(),
                        words: vec![Word {
                            id: "w0".into(),
                            text: "hello".into(),
                            start: 0.0,
                            end: 1.0,
                        }],
                    }],
                },
                Paragraph {
                    id: 2,
                    speaker: Some("B".into()),
                    sentences: vec![Sentence {
                        id: "s2".into(),
                        text: "world".into(),
                        words: vec![Word {
                            id: "w1".into(),
                            text: "world".into(),
                            start: 1.5,
                            end: 2.5,
                        }],
                    }],
                },
            ],
            translations: Default::default(),
        }
    }

    #[test]
    fn flatten_then_restore_round_trips_text() {
        let d = doc();
        let cues = to_cues(&d, None);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_ms, 0);
        assert_eq!(cues[1].end_ms, 2500);
        let back = from_cues(&cues);
        let texts: Vec<&str> = back
            .paragraphs
            .iter()
            .flat_map(|p| p.sentences.iter().map(|s| s.text.as_str()))
            .collect();
        assert_eq!(texts, vec!["hello", "world"]);
        // two speakers → two paragraphs
        assert_eq!(back.paragraphs.len(), 2);
    }

    #[test]
    fn cue_translation_survives_import() {
        let cues = vec![Cue {
            id: "s1".into(),
            start_ms: 0,
            end_ms: 1000,
            text: "hello".into(),
            translation: Some("你好".into()),
            speaker: None,
            hidden: false,
        }];
        let back = from_cues(&cues);
        assert_eq!(back.translations["translated"]["s1"].text, "你好");
    }

    #[test]
    fn cues_serde_camel_case() {
        let c = Cue {
            id: "s1".into(),
            start_ms: 0,
            end_ms: 1000,
            text: "hi".into(),
            translation: None,
            speaker: None,
            hidden: false,
        };
        let s = serde_json::to_string(&c).unwrap();
        assert!(s.contains("\"startMs\""));
        assert!(s.contains("\"endMs\""));
        assert!(!s.contains("start_ms"));
    }
}
