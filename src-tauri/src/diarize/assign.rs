//! Speaker assignment — pure mapping from raw diarization segments onto
//! `doc.json` paragraphs.
//!
//! Align pyannote's VAD and clustering output with the transcript. Each
//! paragraph gets the speaker whose segments cover the
//! largest share of the paragraph's word-level timestamps; paragraphs with
//! no overlap keep `speaker: None`.

use std::collections::BTreeMap;

use crate::data::{Doc, Paragraph};

use super::DiarSegment;

/// Assign each paragraph its dominant speaker. Returns the number of
/// paragraphs that received a speaker. Paragraphs whose words overlap no
/// diarization segment are left untouched, so pre-existing (e.g. manual)
/// labels survive a re-run.
pub fn assign_speakers(doc: &mut Doc, segments: &[DiarSegment]) -> usize {
    let mut assigned = 0;
    for para in &mut doc.paragraphs {
        if let Some(speaker) = paragraph_speaker(para, segments) {
            para.speaker = Some(speaker);
            assigned += 1;
        }
    }
    assigned
}

/// Accumulate per-speaker coverage seconds over the paragraph's word
/// timestamps and return the speaker with the largest total. Deterministic:
/// on exact ties the lexicographically largest label wins (`BTreeMap`
/// iteration order + `Iterator::max_by` returning the last maximum).
fn paragraph_speaker(para: &Paragraph, segments: &[DiarSegment]) -> Option<String> {
    let mut coverage: BTreeMap<&str, f64> = BTreeMap::new();
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
    coverage
        .into_iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(speaker, _)| speaker.to_string())
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
    fn ties_resolve_deterministically() {
        // Identical coverage for A and B → deterministic pick (largest label).
        let mut d = doc(vec![para(1, vec![word("w0", 0.0, 2.0)])]);
        let segs = vec![seg("A", 0.0, 2.0), seg("B", 0.0, 2.0)];
        assign_speakers(&mut d, &segs);
        assert_eq!(d.paragraphs[0].speaker.as_deref(), Some("B"));
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
}
