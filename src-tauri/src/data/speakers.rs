//! Speaker listing, renaming, merging, assignment, and proposal apply.
//!
//! Speakers live on `paragraph.speaker`. `rename`/`merge` rewrite that
//! field across the doc; `reidentify` is a diarize re-run (caller side).
//! Non-destructive re-runs land in `speakers-proposal.json` until applied.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::data::doc::Doc;
use crate::data::storage;
use crate::error::{AppError, AppResult};

const PROPOSAL_FILE: &str = "speakers-proposal.json";

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

/// One non-destructive re-identification suggestion for a paragraph.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerProposal {
    pub paragraph_id: u32,
    pub current: Option<String>,
    pub cluster: String,
    pub proposed: String,
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub coverage: f64,
    pub margin: f64,
}

/// Stored proposal set for review-before-apply re-identification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerProposalSet {
    pub id: String,
    pub created_at: String,
    pub segments: usize,
    pub changed: usize,
    pub unassigned: usize,
    pub proposals: Vec<SpeakerProposal>,
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
    paragraph.speaker = normalize_label(speaker);
    true
}

/// Assign or clear the paragraph that owns `cue_id` (sentence id).
pub fn assign_by_cue(doc: &mut Doc, cue_id: &str, speaker: Option<&str>) -> bool {
    let label = normalize_label(speaker);
    for paragraph in &mut doc.paragraphs {
        if paragraph
            .sentences
            .iter()
            .any(|sentence| sentence.id == cue_id)
        {
            paragraph.speaker = label;
            return true;
        }
    }
    false
}

/// Assign every paragraph whose timed words overlap `[start, end)`.
/// Returns the number of paragraphs touched.
pub fn assign_by_range(doc: &mut Doc, start: f64, end: f64, speaker: Option<&str>) -> usize {
    if !start.is_finite() || !end.is_finite() || end <= start {
        return 0;
    }
    let label = normalize_label(speaker);
    let mut changed = 0;
    for paragraph in &mut doc.paragraphs {
        let words = paragraph
            .sentences
            .iter()
            .flat_map(|sentence| sentence.words.iter())
            .collect::<Vec<_>>();
        let Some(first) = words.first() else {
            continue;
        };
        let Some(last) = words.last() else {
            continue;
        };
        let overlap = first.start.max(start) < last.end.min(end);
        if overlap {
            paragraph.speaker = label.clone();
            changed += 1;
        }
    }
    changed
}

fn normalize_label(speaker: Option<&str>) -> Option<String> {
    speaker
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub fn proposal_path(dir: &Path) -> PathBuf {
    dir.join(PROPOSAL_FILE)
}

pub fn save_proposal(dir: &Path, set: &SpeakerProposalSet) -> AppResult<()> {
    storage::write_json(&proposal_path(dir), set)
}

pub fn load_proposal(dir: &Path) -> AppResult<Option<SpeakerProposalSet>> {
    let path = proposal_path(dir);
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(&std::fs::read_to_string(path)?)?))
}

pub fn clear_proposal(dir: &Path) -> AppResult<bool> {
    let path = proposal_path(dir);
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_file(path)?;
    Ok(true)
}

/// Apply stored proposals onto `doc`. When `changed_only`, skip rows where
/// `current == proposed`. Validates paragraph identity against the live doc
/// so a stale proposal fails instead of silently writing wrong labels.
pub fn apply_proposals(
    doc: &mut Doc,
    proposals: &[SpeakerProposal],
    changed_only: bool,
) -> AppResult<usize> {
    if proposals.is_empty() {
        return Err(AppError::Schema("speaker proposal is empty".into()));
    }
    let mut seen = HashSet::new();
    let mut applied = 0;
    for proposal in proposals {
        if proposal.proposed.trim().is_empty()
            || !proposal.start.is_finite()
            || !proposal.end.is_finite()
            || proposal.end <= proposal.start
            || !seen.insert(proposal.paragraph_id)
        {
            return Err(AppError::Schema("speaker proposal is invalid".into()));
        }
        if changed_only && proposal.current.as_deref() == Some(proposal.proposed.as_str()) {
            continue;
        }
        let paragraph = doc
            .paragraphs
            .iter_mut()
            .find(|paragraph| paragraph.id == proposal.paragraph_id)
            .ok_or_else(|| {
                AppError::Schema(format!(
                    "speaker proposal is stale: paragraph {} is missing",
                    proposal.paragraph_id
                ))
            })?;
        let words = paragraph
            .sentences
            .iter()
            .flat_map(|sentence| sentence.words.iter())
            .collect::<Vec<_>>();
        let (Some(first), Some(last)) = (words.first(), words.last()) else {
            return Err(AppError::Schema(format!(
                "speaker proposal is stale: paragraph {} has no timed words",
                proposal.paragraph_id
            )));
        };
        let text = paragraph
            .sentences
            .iter()
            .map(|sentence| sentence.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if paragraph.speaker != proposal.current
            || (first.start - proposal.start).abs() > 0.001
            || (last.end - proposal.end).abs() > 0.001
            || text != proposal.text
        {
            return Err(AppError::Schema(
                "speaker proposal is stale; run identification again".into(),
            ));
        }
        paragraph.speaker = Some(proposal.proposed.trim().to_owned());
        applied += 1;
    }
    Ok(applied)
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

    #[test]
    fn assign_by_cue_targets_owning_paragraph() {
        let mut d = doc(vec![Some("A"), Some("B")]);
        d.paragraphs[1].sentences.push(Sentence {
            id: "cue-b".into(),
            text: "Hi".into(),
            words: vec![Word {
                id: "w0".into(),
                text: "Hi".into(),
                start: 3.0,
                end: 3.5,
            }],
        });
        assert!(assign_by_cue(&mut d, "cue-b", Some("Host")));
        assert_eq!(d.paragraphs[1].speaker.as_deref(), Some("Host"));
        assert!(!assign_by_cue(&mut d, "missing", Some("X")));
    }

    #[test]
    fn assign_by_range_labels_overlapping_paragraphs() {
        let mut d = doc(vec![None, None]);
        d.paragraphs[0].sentences.push(Sentence {
            id: "a".into(),
            text: "one".into(),
            words: vec![Word {
                id: "w0".into(),
                text: "one".into(),
                start: 0.0,
                end: 1.0,
            }],
        });
        d.paragraphs[1].sentences.push(Sentence {
            id: "b".into(),
            text: "two".into(),
            words: vec![Word {
                id: "w1".into(),
                text: "two".into(),
                start: 5.0,
                end: 6.0,
            }],
        });
        assert_eq!(assign_by_range(&mut d, 0.5, 1.5, Some("A")), 1);
        assert_eq!(d.paragraphs[0].speaker.as_deref(), Some("A"));
        assert_eq!(d.paragraphs[1].speaker, None);
    }

    #[test]
    fn apply_proposals_writes_changed_labels_and_rejects_stale() {
        let mut d = doc(vec![Some("A")]);
        d.paragraphs[0].sentences.push(Sentence {
            id: "cue-1".into(),
            text: "Hello".into(),
            words: vec![Word {
                id: "w0".into(),
                text: "Hello".into(),
                start: 1.0,
                end: 2.0,
            }],
        });
        let proposals = vec![SpeakerProposal {
            paragraph_id: 0,
            current: Some("A".into()),
            cluster: "SPEAKER_01".into(),
            proposed: "Host".into(),
            start: 1.0,
            end: 2.0,
            text: "Hello".into(),
            coverage: 0.9,
            margin: 0.4,
        }];
        assert_eq!(apply_proposals(&mut d, &proposals, true).unwrap(), 1);
        assert_eq!(d.paragraphs[0].speaker.as_deref(), Some("Host"));
        let stale = vec![SpeakerProposal {
            paragraph_id: 0,
            current: Some("A".into()),
            cluster: "SPEAKER_01".into(),
            proposed: "Guest".into(),
            start: 1.0,
            end: 2.0,
            text: "Hello".into(),
            coverage: 0.9,
            margin: 0.4,
        }];
        assert!(apply_proposals(&mut d, &stale, true).is_err());
    }
}
