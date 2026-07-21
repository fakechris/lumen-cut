//! Word/cue reindexing (Stage 3 slice).  When a source edit (`subtitle set`,
//! `find/replace`) changes the word set, all downstream token ids move.
//! Reindex rewrites word ids to a contiguous `wN` sequence and emits the
//! diff consumed by callers (pipeline `apply`).
//!
//! The mapping is intentionally simple — stable across callers — so a later
//! Stage 4 module can plug it into the audit/finish-check pipeline.

use serde::{Deserialize, Serialize};

use crate::data::doc::Doc;

/// Diff emitted by `reindex` callers: original → remapped ids, removed ids.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ReindexMap {
    pub mapping: Vec<(String, String)>,
    pub removed: Vec<String>,
}

/// Re-write word ids so they form `w0, w1, … wN` in source order. Returns the
/// diff.  The doc is **not** mutated; callers `apply` via a separate path
/// that owns the persistence decision.
pub fn reindex_words(doc: &Doc) -> ReindexMap {
    let mut map = ReindexMap::default();
    for (new_idx, w) in doc.all_words().into_iter().enumerate() {
        let new_id = format!("w{}", new_idx);
        if w.id != new_id {
            map.mapping.push((w.id.clone(), new_id));
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::doc::*;
    use std::path::PathBuf;

    fn sample_with_word_ids(ids: &[&str]) -> Doc {
        let d = Doc {
            id: "p".into(),
            schema: 1,
            media: MediaRef {
                path: PathBuf::from("/tmp/x.mp4"),
                duration_seconds: 0.0,
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
            paragraphs: vec![Paragraph {
                id: 1,
                speaker: None,
                sentences: vec![Sentence {
                    id: "s1".into(),
                    text: String::new(),
                    words: ids
                        .iter()
                        .enumerate()
                        .map(|(i, id)| Word {
                            id: (*id).to_string(),
                            text: format!("w{i}"),
                            start: i as f64,
                            end: i as f64 + 0.1,
                        })
                        .collect(),
                }],
            }],
            translations: Default::default(),
        };
        d
    }

    #[test]
    fn stable_when_already_w_n() {
        let d = sample_with_word_ids(&["w0", "w1", "w2"]);
        let m = reindex_words(&d);
        assert!(m.mapping.is_empty());
    }

    #[test]
    fn renumbers_when_ids_drift() {
        let d = sample_with_word_ids(&["x9", "x20", "x21"]);
        let m = reindex_words(&d);
        // x9 -> w0, x20 -> w1, x21 -> w2 (order-independent for old ids).
        let mapped: std::collections::BTreeSet<_> =
            m.mapping.iter().map(|(_, b)| b.clone()).collect();
        assert!(mapped.contains("w0"));
        assert!(mapped.contains("w1"));
        assert!(mapped.contains("w2"));
    }
}
