//! Timing repair — fix the word-time issues `audit` flags
//! (`invalid-word-time`, `word-time-boundary`, `zero-duration-words`).
//!
//! Walks words in document order and: clamps negative starts, repairs
//! inverted intervals (`end < start`), floors zero-duration words, and
//! pushes an overlapping start past the previous word's end.

use serde::{Deserialize, Serialize};

use crate::data::doc::Doc;

/// Minimum word duration (seconds). Keep a 1 ms margin above the audit's
/// strict 0.05 s threshold so floating-point serialization cannot round a
/// repaired interval back below the gate.
pub const MIN_DUR: f64 = 0.051;
/// Overlap jitter tolerance — matches `audit::engine`'s 0.05 s window.
pub const JITTER: f64 = 0.05;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepairReport {
    pub clamped_negative: usize,
    pub fixed_inverted: usize,
    pub fixed_zero: usize,
    pub fixed_overlap: usize,
    pub clamped_media: usize,
}

impl RepairReport {
    pub fn total(&self) -> usize {
        self.clamped_negative
            + self.fixed_inverted
            + self.fixed_zero
            + self.fixed_overlap
            + self.clamped_media
    }
}

/// Repair word timing in place.
pub fn repair(doc: &mut Doc) -> RepairReport {
    let mut rep = RepairReport::default();
    let mut prev_end = -1.0_f64;
    for w in doc
        .paragraphs
        .iter_mut()
        .flat_map(|p| p.sentences.iter_mut())
        .flat_map(|s| s.words.iter_mut())
    {
        if w.start < 0.0 {
            w.start = 0.0;
            rep.clamped_negative += 1;
        }
        if w.end < w.start {
            w.end = w.start + MIN_DUR;
            rep.fixed_inverted += 1;
        }
        if w.end - w.start < MIN_DUR {
            w.end = w.start + MIN_DUR;
            rep.fixed_zero += 1;
        }
        // Repair every actual overlap, including overlaps inside the audit's
        // jitter allowance. Otherwise extending a zero-duration word can
        // create a new sentence-boundary failure in the following cue.
        if w.start < prev_end {
            w.start = prev_end;
            if w.end - w.start < MIN_DUR {
                w.end = w.start + MIN_DUR;
            }
            rep.fixed_overlap += 1;
        }
        prev_end = prev_end.max(w.end);
    }

    // Forced alignment can extend the tail a few frames past the media. Walk
    // backwards so capping the last word never creates a new overlap with the
    // word before it. This only compresses the affected tail.
    let media_end = doc.media.duration_seconds;
    if media_end > 0.0 {
        let mut next_start = media_end;
        for word in doc
            .paragraphs
            .iter_mut()
            .rev()
            .flat_map(|paragraph| paragraph.sentences.iter_mut().rev())
            .flat_map(|sentence| sentence.words.iter_mut().rev())
        {
            if word.end > next_start {
                word.end = next_start;
                word.start = word.start.min((word.end - MIN_DUR).max(0.0));
                rep.clamped_media += 1;
            }
            next_start = word.start;
        }
    }
    rep
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::doc::*;

    fn doc(words: Vec<(f64, f64)>) -> Doc {
        Doc {
            id: "p".into(),
            schema: 1,
            media: MediaRef {
                path: "/tmp/x".into(),
                duration_seconds: 10.0,
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
                    text: "x".into(),
                    words: words
                        .into_iter()
                        .enumerate()
                        .map(|(i, (s, e))| Word {
                            id: format!("w{i}"),
                            text: format!("w{i}"),
                            start: s,
                            end: e,
                        })
                        .collect(),
                }],
            }],
            translations: Default::default(),
        }
    }

    #[test]
    fn clamps_negative_and_inverted() {
        let mut d = doc(vec![(-1.0, 0.5), (1.0, 0.5)]); // negative, inverted
        let rep = repair(&mut d);
        assert_eq!(rep.clamped_negative, 1);
        assert_eq!(rep.fixed_inverted, 1);
        let ws = &d.paragraphs[0].sentences[0].words;
        assert_eq!(ws[0].start, 0.0);
        assert!(ws[1].end >= ws[1].start);
    }

    #[test]
    fn floors_zero_duration() {
        let mut d = doc(vec![(0.0, 0.001)]);
        let rep = repair(&mut d);
        assert_eq!(rep.fixed_zero, 1);
        let w = &d.paragraphs[0].sentences[0].words[0];
        assert!((w.end - w.start) >= MIN_DUR);
    }

    #[test]
    fn pushes_overlap_past_previous() {
        // w0 [0,2], w1 [1,3] overlaps → w1.start pushed to 2
        let mut d = doc(vec![(0.0, 2.0), (1.0, 3.0)]);
        let rep = repair(&mut d);
        assert_eq!(rep.fixed_overlap, 1);
        assert!(d.paragraphs[0].sentences[0].words[1].start >= 2.0 - JITTER);
        assert!(
            d.paragraphs[0].sentences[0].words[1].end - d.paragraphs[0].sentences[0].words[1].start
                >= MIN_DUR
        );
    }

    #[test]
    fn clamps_the_aligned_tail_inside_media_without_creating_overlap() {
        let mut d = doc(vec![(8.0, 9.98), (9.98, 10.2)]);
        let rep = repair(&mut d);
        let words = &d.paragraphs[0].sentences[0].words;
        assert_eq!(rep.clamped_media, 2);
        assert!(words[1].end <= d.media.duration_seconds);
        assert!(words[1].end - words[1].start >= MIN_DUR);
        assert!(words[0].end <= words[1].start);
    }
}
