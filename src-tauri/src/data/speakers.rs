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
}
