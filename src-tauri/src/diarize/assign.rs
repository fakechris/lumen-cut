//! Speaker assignment — pure mapping from raw diarization segments onto
//! `doc.json` paragraphs.
//!
//! Align pyannote's VAD and clustering output with the transcript. Each
//! paragraph gets the speaker whose segments cover the
//! largest share of the paragraph's word-level timestamps; paragraphs with
//! no overlap keep `speaker: None`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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

/// Assign each paragraph its dominant speaker. Returns the number of
/// paragraphs that received a speaker. Paragraphs whose words overlap no
/// diarization segment are left untouched, so pre-existing (e.g. manual)
/// labels survive a re-run.
pub fn assign_speakers(doc: &mut Doc, segments: &[DiarSegment]) -> usize {
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
