//! Speaker listing, renaming, and merging.
//!
//! Speakers live on `paragraph.speaker`. `rename`/`merge` rewrite that
//! field across the doc; `reidentify` is a diarize re-run (caller side).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::data::doc::Doc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerInfo {
    pub id: String,
    pub paragraph_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerTurn {
    pub paragraph_id: u32,
    pub speaker: Option<String>,
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub cue_ids: Vec<String>,
}

/// Distinct speakers with the number of paragraphs each owns.
pub fn list(doc: &Doc) -> Vec<SpeakerInfo> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for p in &doc.paragraphs {
        if let Some(s) = &p.speaker {
            *counts.entry(s.clone()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .map(|(id, paragraph_count)| SpeakerInfo {
            id,
            paragraph_count,
        })
        .collect()
}

/// One inspectable turn per paragraph, including the exact transcript and
/// media range used when a human reviews a speaker label.
pub fn turns(doc: &Doc) -> Vec<SpeakerTurn> {
    doc.paragraphs
        .iter()
        .filter_map(|paragraph| {
            let words = paragraph
                .sentences
                .iter()
                .flat_map(|sentence| sentence.words.iter())
                .collect::<Vec<_>>();
            let (Some(first), Some(last)) = (words.first(), words.last()) else {
                return None;
            };
            Some(SpeakerTurn {
                paragraph_id: paragraph.id,
                speaker: paragraph.speaker.clone(),
                start: first.start,
                end: last.end,
                text: paragraph
                    .sentences
                    .iter()
                    .map(|sentence| sentence.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                cue_ids: paragraph
                    .sentences
                    .iter()
                    .map(|sentence| sentence.id.clone())
                    .collect(),
            })
        })
        .collect()
}

/// Assign or clear one paragraph label. Returns false for an unknown id.
pub fn assign(doc: &mut Doc, paragraph_id: u32, speaker: Option<&str>) -> bool {
    let Some(paragraph) = doc
        .paragraphs
        .iter_mut()
        .find(|paragraph| paragraph.id == paragraph_id)
    else {
        return false;
    };
    paragraph.speaker = speaker
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    true
}

/// Rename every paragraph whose speaker is `from` to `to`. Returns the
/// count touched.
pub fn rename(doc: &mut Doc, from: &str, to: &str) -> usize {
    let mut n = 0;
    for p in &mut doc.paragraphs {
        if p.speaker.as_deref() == Some(from) {
            p.speaker = Some(to.into());
            n += 1;
        }
    }
    n
}

/// Merge `from` into `into` (alias of `rename`).
pub fn merge(doc: &mut Doc, from: &str, into: &str) -> usize {
    rename(doc, from, into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::doc::*;

    fn doc(speakers: Vec<Option<&str>>) -> Doc {
        Doc {
            id: "p".into(),
            schema: 1,
            media: MediaRef {
                path: "/tmp/x".into(),
                duration_seconds: 1.0,
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
            paragraphs: speakers
                .into_iter()
                .enumerate()
                .map(|(i, sp)| Paragraph {
                    id: i as u32,
                    speaker: sp.map(String::from),
                    sentences: vec![],
                })
                .collect(),
            translations: Default::default(),
        }
    }

    #[test]
    fn list_counts_speakers() {
        let d = doc(vec![Some("S1"), Some("S1"), Some("S2"), None]);
        let info = list(&d);
        assert_eq!(
            info.iter().find(|s| s.id == "S1").unwrap().paragraph_count,
            2
        );
        assert_eq!(
            info.iter().find(|s| s.id == "S2").unwrap().paragraph_count,
            1
        );
    }

    #[test]
    fn rename_rewrites_speaker() {
        let mut d = doc(vec![Some("SPEAKER_00"), Some("SPEAKER_01")]);
        let n = rename(&mut d, "SPEAKER_00", "Alice");
        assert_eq!(n, 1);
        assert_eq!(d.paragraphs[0].speaker.as_deref(), Some("Alice"));
    }

    #[test]
    fn merge_combines_speakers() {
        let mut d = doc(vec![Some("A"), Some("B"), Some("A")]);
        assert_eq!(merge(&mut d, "B", "A"), 1);
        assert!(d
            .paragraphs
            .iter()
            .all(|p| p.speaker.as_deref() == Some("A")));
    }

    #[test]
    fn turns_include_reviewable_boundaries_and_text() {
        let mut d = doc(vec![Some("A")]);
        d.paragraphs[0].sentences.push(Sentence {
            id: "cue-1".into(),
            text: "Hello there".into(),
            words: vec![
                Word {
                    id: "w0".into(),
                    text: "Hello".into(),
                    start: 1.0,
                    end: 1.5,
                },
                Word {
                    id: "w1".into(),
                    text: "there".into(),
                    start: 1.6,
                    end: 2.0,
                },
            ],
        });
        assert_eq!(
            turns(&d),
            vec![SpeakerTurn {
                paragraph_id: 0,
                speaker: Some("A".into()),
                start: 1.0,
                end: 2.0,
                text: "Hello there".into(),
                cue_ids: vec!["cue-1".into()],
            }]
        );
    }

    #[test]
    fn assign_sets_and_clears_one_paragraph() {
        let mut d = doc(vec![Some("A"), Some("B")]);
        assert!(assign(&mut d, 1, Some("  Host  ")));
        assert_eq!(d.paragraphs[1].speaker.as_deref(), Some("Host"));
        assert!(assign(&mut d, 1, None));
        assert_eq!(d.paragraphs[1].speaker, None);
        assert!(!assign(&mut d, 99, Some("Missing")));
    }
}
