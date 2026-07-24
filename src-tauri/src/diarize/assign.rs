//! Speaker assignment — pure mapping from raw diarization segments onto
//! `doc.json` paragraphs.
//!
//! Align pyannote's VAD and clustering output with the transcript. Each
//! paragraph gets the speaker whose segments cover the
//! largest share of the paragraph's word-level timestamps; paragraphs with
//! no overlap keep `speaker: None`.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::data::speakers::SpeakerProposal;
use crate::data::{Doc, Paragraph};

use super::DiarSegment;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerMatch {
    pub speaker: String,
    pub covered_seconds: f64,
    pub timed_seconds: f64,
    pub coverage: f64,
    pub margin: f64,
}

pub const MIN_SPEAKER_COVERAGE: f64 = 0.5;
pub const MIN_SPEAKER_MARGIN: f64 = 0.15;

pub fn reliable_speaker_match(coverage: f64, margin: f64) -> bool {
    coverage >= MIN_SPEAKER_COVERAGE && margin >= MIN_SPEAKER_MARGIN
}

/// Build non-destructive re-identification proposals from fresh segments.
/// Preserves human names by greedily mapping new clusters onto current labels
/// with the largest measured overlap (one-to-one).
pub fn proposals_from_segments(
    doc: &Doc,
    segments: &[DiarSegment],
) -> (Vec<SpeakerProposal>, usize) {
    let mut unassigned = 0usize;
    let matches = doc
        .paragraphs
        .iter()
        .filter_map(|paragraph| {
            let Some((start, end)) = paragraph_time_bounds(paragraph) else {
                unassigned += 1;
                return None;
            };
            let Some(matched) = match_paragraph(paragraph, segments) else {
                unassigned += 1;
                return None;
            };
            Some((paragraph, matched, start, end))
        })
        .collect::<Vec<_>>();

    let mut scores = BTreeMap::<(String, String), f64>::new();
    for (paragraph, matched, _, _) in &matches {
        if let Some(current) = paragraph.speaker.as_ref() {
            *scores
                .entry((matched.speaker.clone(), current.clone()))
                .or_default() += matched.covered_seconds;
        }
    }
    let mut ranked = scores.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });
    let mut cluster_names = HashMap::<String, String>::new();
    let mut used_names = HashSet::<String>::new();
    for ((cluster, current), _) in ranked {
        if !cluster_names.contains_key(&cluster) && used_names.insert(current.clone()) {
            cluster_names.insert(cluster, current);
        }
    }

    let proposals = matches
        .into_iter()
        .map(|(paragraph, matched, start, end)| {
            let cluster = matched.speaker;
            SpeakerProposal {
                paragraph_id: paragraph.id,
                current: paragraph.speaker.clone(),
                proposed: cluster_names
                    .get(&cluster)
                    .cloned()
                    .unwrap_or_else(|| cluster.clone()),
                cluster,
                start,
                end,
                text: paragraph
                    .sentences
                    .iter()
                    .map(|sentence| sentence.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                coverage: matched.coverage,
                margin: matched.margin,
            }
        })
        .collect();
    (proposals, unassigned)
}

fn paragraph_time_bounds(paragraph: &Paragraph) -> Option<(f64, f64)> {
    let words = paragraph
        .sentences
        .iter()
        .flat_map(|sentence| sentence.words.iter())
        .collect::<Vec<_>>();
    let first = words.first()?;
    let last = words.last()?;
    Some((first.start, last.end))
}

/// Normalize transcript paragraphs to the granularity used by diarization:
/// one subtitle sentence per paragraph. Sentence and word ids stay stable,
/// while paragraph ids are rebuilt deterministically in document order.
///
/// The UI runs this on an in-memory clone for previews. It is only persisted
/// when the user applies the proposal, so opening speaker tools never mutates
/// an existing project.
pub fn normalize_speaker_paragraphs(doc: &mut Doc) -> usize {
    if doc
        .paragraphs
        .iter()
        .all(|paragraph| paragraph.sentences.len() <= 1)
    {
        return 0;
    }

    let before = doc.paragraphs.len();
    let mut normalized = Vec::new();
    for paragraph in std::mem::take(&mut doc.paragraphs) {
        if paragraph.sentences.is_empty() {
            normalized.push(Paragraph {
                id: normalized.len() as u32 + 1,
                speaker: paragraph.speaker,
                sentences: Vec::new(),
            });
            continue;
        }
        for sentence in paragraph.sentences {
            normalized.push(Paragraph {
                id: normalized.len() as u32 + 1,
                speaker: paragraph.speaker.clone(),
                sentences: vec![sentence],
            });
        }
    }
    doc.paragraphs = normalized;
    doc.paragraphs.len().saturating_sub(before)
}

/// Assign each paragraph its dominant speaker. Returns the number of
/// paragraphs that received a speaker. Paragraphs whose words overlap no
/// diarization segment are left untouched, so pre-existing (e.g. manual)
/// labels survive a re-run.
pub fn assign_speakers(doc: &mut Doc, segments: &[DiarSegment]) -> usize {
    normalize_speaker_paragraphs(doc);
    let mut assigned = 0;
    for para in &mut doc.paragraphs {
        if let Some(matched) = match_paragraph(para, segments) {
            para.speaker = Some(matched.speaker);
            assigned += 1;
        }
    }
    assigned
}

/// Accumulate per-speaker coverage seconds over the paragraph's word
/// timestamps and return the speaker with the largest total. Deterministic:
/// on exact ties the lexicographically largest label wins (`BTreeMap`
/// iteration order + `Iterator::max_by` returning the last maximum).
pub fn match_paragraph(para: &Paragraph, segments: &[DiarSegment]) -> Option<SpeakerMatch> {
    let mut coverage: BTreeMap<&str, f64> = BTreeMap::new();
    let timed_seconds = para
        .sentences
        .iter()
        .flat_map(|sentence| sentence.words.iter())
        .map(|word| (word.end - word.start).max(0.0))
        .sum::<f64>();
    for seg in segments {
        for sent in &para.sentences {
            for w in &sent.words {
                let overlap = (w.end.min(seg.end) - w.start.max(seg.start)).max(0.0);
                if overlap > 0.0 {
                    *coverage.entry(seg.speaker.as_str()).or_default() += overlap;
                }
            }
        }
    }
    let mut ranked = coverage.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| right.0.cmp(left.0))
    });
    let (speaker, covered_seconds) = ranked.first().copied()?;
    let runner_up = ranked.get(1).map(|(_, seconds)| *seconds).unwrap_or(0.0);
    let coverage = if timed_seconds > 0.0 {
        (covered_seconds / timed_seconds).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let margin = if timed_seconds > 0.0 {
        ((covered_seconds - runner_up) / timed_seconds).clamp(0.0, 1.0)
    } else {
        0.0
    };
    reliable_speaker_match(coverage, margin).then(|| SpeakerMatch {
        speaker: speaker.to_string(),
        covered_seconds,
        timed_seconds,
        coverage,
        margin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{MediaRef, Meta, Sentence, Word};
    use chrono::Utc;
    use std::path::PathBuf;

    fn seg(speaker: &str, start: f64, end: f64) -> DiarSegment {
        DiarSegment {
            speaker: speaker.into(),
            start,
            end,
        }
    }

    fn word(id: &str, start: f64, end: f64) -> Word {
        Word {
            id: id.into(),
            text: id.into(),
            start,
            end,
        }
    }

    fn para(id: u32, words: Vec<Word>) -> Paragraph {
        Paragraph {
            id,
            speaker: None,
            sentences: vec![Sentence {
                id: format!("p{id}s1"),
                text: "t".into(),
                words,
            }],
        }
    }

    fn sentence(id: &str, words: Vec<Word>) -> Sentence {
        Sentence {
            id: id.into(),
            text: id.into(),
            words,
        }
    }

    fn doc(paragraphs: Vec<Paragraph>) -> Doc {
        Doc {
            id: "p".into(),
            schema: 1,
            media: MediaRef {
                path: PathBuf::from("/tmp/x.mp4"),
                duration_seconds: 10.0,
                sample_rate: Some(16_000),
                channels: Some(1),
            },
            meta: Meta {
                title: "t".into(),
                description: String::new(),
                language: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            paragraphs,
            translations: Default::default(),
        }
    }

    #[test]
    fn dominant_speaker_wins_by_coverage() {
        // w0 [0,1] → 1s of A; w1 [2,3] + w2 [3,6] → 4s of B.
        let mut d = doc(vec![para(
            1,
            vec![
                word("w0", 0.0, 1.0),
                word("w1", 2.0, 3.0),
                word("w2", 3.0, 6.0),
            ],
        )]);
        let segs = vec![seg("A", 0.0, 2.0), seg("B", 2.0, 8.0)];
        let n = assign_speakers(&mut d, &segs);
        assert_eq!(n, 1);
        assert_eq!(d.paragraphs[0].speaker.as_deref(), Some("B"));
    }

    #[test]
    fn partial_word_overlap_is_measured() {
        // Single word [0.5, 2.5]: A covers 0.5s, B covers 1.5s.
        let mut d = doc(vec![para(1, vec![word("w0", 0.5, 2.5)])]);
        let segs = vec![seg("A", 0.0, 1.0), seg("B", 1.0, 3.0)];
        assign_speakers(&mut d, &segs);
        assert_eq!(d.paragraphs[0].speaker.as_deref(), Some("B"));
        let matched = match_paragraph(&d.paragraphs[0], &segs).unwrap();
        assert_eq!(matched.speaker, "B");
        assert!((matched.covered_seconds - 1.5).abs() < 0.001);
        assert!((matched.coverage - 0.75).abs() < 0.001);
    }

    #[test]
    fn no_overlap_keeps_speaker_none() {
        let mut d = doc(vec![para(1, vec![word("w0", 0.0, 1.0)])]);
        let segs = vec![seg("A", 10.0, 20.0)];
        let n = assign_speakers(&mut d, &segs);
        assert_eq!(n, 0);
        assert_eq!(d.paragraphs[0].speaker, None);
    }

    #[test]
    fn paragraph_without_words_gets_no_speaker() {
        let mut d = doc(vec![para(1, vec![])]);
        let n = assign_speakers(&mut d, &[seg("A", 0.0, 10.0)]);
        assert_eq!(n, 0);
        assert_eq!(d.paragraphs[0].speaker, None);
    }

    #[test]
    fn empty_segments_keep_speaker_none() {
        let mut d = doc(vec![para(1, vec![word("w0", 0.0, 1.0)])]);
        let n = assign_speakers(&mut d, &[]);
        assert_eq!(n, 0);
        assert_eq!(d.paragraphs[0].speaker, None);
    }

    #[test]
    fn ties_are_left_for_manual_review() {
        let mut d = doc(vec![para(1, vec![word("w0", 0.0, 2.0)])]);
        let segs = vec![seg("A", 0.0, 2.0), seg("B", 0.0, 2.0)];
        assert_eq!(assign_speakers(&mut d, &segs), 0);
        assert_eq!(d.paragraphs[0].speaker, None);
    }

    #[test]
    fn existing_label_preserved_when_no_overlap() {
        let mut d = doc(vec![para(1, vec![word("w0", 0.0, 1.0)])]);
        d.paragraphs[0].speaker = Some("manual".into());
        let n = assign_speakers(&mut d, &[seg("A", 10.0, 20.0)]);
        assert_eq!(n, 0);
        assert_eq!(d.paragraphs[0].speaker.as_deref(), Some("manual"));
    }

    #[test]
    fn per_paragraph_assignment_is_independent() {
        let mut d = doc(vec![
            para(1, vec![word("w0", 0.0, 1.0)]),
            para(2, vec![word("w1", 5.0, 6.0)]),
        ]);
        let segs = vec![seg("A", 0.0, 2.0), seg("B", 4.0, 8.0)];
        let n = assign_speakers(&mut d, &segs);
        assert_eq!(n, 2);
        assert_eq!(d.paragraphs[0].speaker.as_deref(), Some("A"));
        assert_eq!(d.paragraphs[1].speaker.as_deref(), Some("B"));
    }

    #[test]
    fn a_legacy_multi_sentence_paragraph_is_split_before_assignment() {
        let mut paragraph = para(42, vec![]);
        paragraph.sentences = vec![
            sentence("cue-a", vec![word("w0", 0.0, 1.0)]),
            sentence("cue-b", vec![word("w1", 5.0, 6.0)]),
        ];
        let mut d = doc(vec![paragraph]);

        let assigned = assign_speakers(&mut d, &[seg("A", 0.0, 2.0), seg("B", 4.0, 8.0)]);

        assert_eq!(assigned, 2);
        assert_eq!(d.paragraphs.len(), 2);
        assert_eq!(d.paragraphs[0].id, 1);
        assert_eq!(d.paragraphs[0].sentences[0].id, "cue-a");
        assert_eq!(d.paragraphs[0].speaker.as_deref(), Some("A"));
        assert_eq!(d.paragraphs[1].id, 2);
        assert_eq!(d.paragraphs[1].sentences[0].id, "cue-b");
        assert_eq!(d.paragraphs[1].speaker.as_deref(), Some("B"));
    }

    #[test]
    fn preview_normalization_is_deterministic_and_preserves_manual_labels() {
        let mut paragraph = para(99, vec![]);
        paragraph.speaker = Some("Chris".into());
        paragraph.sentences = vec![
            sentence("cue-a", vec![word("w0", 0.0, 1.0)]),
            sentence("cue-b", vec![word("w1", 1.0, 2.0)]),
        ];
        let mut d = doc(vec![paragraph]);

        assert_eq!(normalize_speaker_paragraphs(&mut d), 1);
        assert_eq!(normalize_speaker_paragraphs(&mut d), 0);
        assert_eq!(
            d.paragraphs
                .iter()
                .map(|paragraph| (
                    paragraph.id,
                    paragraph.speaker.as_deref(),
                    paragraph.sentences[0].id.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![(1, Some("Chris"), "cue-a"), (2, Some("Chris"), "cue-b")]
        );
    }

    #[test]
    fn tiny_overlap_is_not_a_reliable_assignment() {
        let paragraph = para(1, vec![word("w0", 0.0, 10.0)]);
        assert!(match_paragraph(&paragraph, &[seg("A", 0.0, 0.2)]).is_none());
    }

    #[test]
    fn near_tied_candidates_are_not_a_reliable_assignment() {
        let paragraph = para(1, vec![word("w0", 0.0, 10.0)]);
        assert!(match_paragraph(&paragraph, &[seg("A", 0.0, 5.1), seg("B", 5.1, 10.0)]).is_none());
    }
}
