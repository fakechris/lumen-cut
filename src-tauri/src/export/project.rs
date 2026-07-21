//! Soft-cut projection for export — retime + skip cues over cut regions.
//!
//! Cuts produce a sorted partition table. A cue instant is mapped to its
//! post-cut display time by binary-searching that table:
//!
//!   `display(t) = t − Σ cut.duration  for every cut wholly before t`
//!
//! and an instant that lands *inside* a cut is clamped to that cut's start
//! (the cut is absorbed — the gap closes). A cue whose `[start, end]` is
//! fully consumed by a cut is dropped from the export entirely.

use crate::data::doc::Doc;
use crate::data::soft_cut::Cut;

/// Sorted, merged, non-overlapping cut intervals on the original timeline.
/// Word cuts use `(a_word.start, b_word.end)`. Silence cuts use the actual
/// removed portion encoded by [`Cut::resolved_interval`], preserving their
/// compressed pause. Degenerate and unresolvable cuts are dropped.
pub fn cut_intervals(doc: &Doc, cuts: &[Cut]) -> Vec<(f64, f64)> {
    let mut iv: Vec<(f64, f64)> = cuts
        .iter()
        .filter_map(|cut| cut.resolved_interval(doc))
        .filter(|(s, e)| e > s)
        .collect();
    iv.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (s, e) in iv {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    merged
}

/// Exact seconds removed after resolving and unioning all cut intervals.
/// Cached per-cut durations are not summed because cuts may overlap.
pub fn removed_duration(doc: &Doc, cuts: &[Cut]) -> f64 {
    cut_intervals(doc, cuts)
        .into_iter()
        .map(|(start, end)| end - start)
        .sum()
}

/// Map an original-timeline instant to its post-cut display time. An
/// instant inside a cut clamps to that cut's start (the gap closes).
pub fn retime(t: f64, intervals: &[(f64, f64)]) -> f64 {
    let mut offset = 0.0;
    for &(cs, ce) in intervals {
        if t <= cs {
            break;
        }
        if t >= ce {
            offset += ce - cs;
        } else {
            // t lies inside this cut → clamp to the cut's display position.
            return (cs - offset).max(0.0);
        }
    }
    (t - offset).max(0.0)
}

/// True when `[s, e]` is fully consumed by a single cut interval — the
/// cue is dropped from the export.
pub fn fully_cut(s: f64, e: f64, intervals: &[(f64, f64)]) -> bool {
    intervals.iter().any(|(cs, ce)| *cs <= s && e <= *ce)
}

/// Complement of [`cut_intervals`] over the media duration.
pub fn kept_intervals(doc: &Doc, cuts: &[Cut]) -> Vec<(f64, f64)> {
    let mut kept = Vec::new();
    let mut cursor = 0.0;
    for (start, end) in cut_intervals(doc, cuts) {
        let start = start.clamp(0.0, doc.media.duration_seconds);
        let end = end.clamp(0.0, doc.media.duration_seconds);
        if start > cursor {
            kept.push((cursor, start));
        }
        cursor = cursor.max(end);
    }
    if cursor < doc.media.duration_seconds {
        kept.push((cursor, doc.media.duration_seconds));
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::soft_cut::{ClipCuts, CutKind};

    fn doc_with_words(words: &[(&str, f64, f64)]) -> Doc {
        use crate::data::doc::*;
        Doc {
            id: "p".into(),
            schema: 1,
            media: MediaRef {
                path: std::path::PathBuf::from("/tmp/x.mp4"),
                duration_seconds: 10.0,
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
                speaker: None,
                sentences: vec![Sentence {
                    id: "s1".into(),
                    text: "alpha beta gamma".into(),
                    words: words
                        .iter()
                        .map(|(id, s, e)| Word {
                            id: (*id).into(),
                            text: (*id).into(),
                            start: *s,
                            end: *e,
                        })
                        .collect(),
                }],
            }],
            translations: Default::default(),
        }
    }

    fn cut(a: &str, b: &str) -> crate::data::soft_cut::Cut {
        crate::data::soft_cut::Cut {
            id: format!("c-{a}-{b}"),
            note: None,
            a_word: a.into(),
            b_word: b.into(),
            kind: CutKind::Manual,
            duration: 0.0,
        }
    }

    #[test]
    fn retime_subtracts_cuts_before_instant() {
        let doc = doc_with_words(&[("w0", 0.0, 1.0), ("w1", 1.0, 3.0), ("w2", 3.0, 5.0)]);
        let iv = cut_intervals(&doc, &[cut("w1", "w1")]); // 1.0..3.0 (2s)
        assert_eq!(retime(0.5, &iv), 0.5); // before cut
        assert_eq!(retime(3.0, &iv), 1.0); // at cut end: 3 - 2
        assert_eq!(retime(4.0, &iv), 2.0); // 4 - 2
    }

    #[test]
    fn retime_inside_cut_clamps_to_cut_start() {
        let doc = doc_with_words(&[("w0", 0.0, 1.0), ("w1", 1.0, 3.0)]);
        let iv = cut_intervals(&doc, &[cut("w0", "w1")]); // 0..3
        assert_eq!(retime(1.5, &iv), 0.0); // inside → clamp to 0
    }

    #[test]
    fn fully_cut_cue_is_dropped() {
        let iv = vec![(1.0, 3.0)];
        assert!(fully_cut(1.5, 2.5, &iv));
        assert!(!fully_cut(0.5, 1.5, &iv));
    }

    #[test]
    fn empty_cuts_identity() {
        let doc = doc_with_words(&[("w0", 0.0, 1.0)]);
        let iv = cut_intervals(&doc, &[]);
        assert!(iv.is_empty());
        assert_eq!(retime(0.7, &iv), 0.7);
    }

    #[test]
    fn overlapping_cuts_merge() {
        let doc = doc_with_words(&[
            ("w0", 0.0, 1.0),
            ("w1", 1.0, 2.0),
            ("w2", 2.0, 3.0),
            ("w3", 3.0, 4.0),
        ]);
        let cuts = crate::data::soft_cut::ClipCuts {
            cuts: vec![cut("w0", "w1"), cut("w1", "w2")], // overlap at w1
        };
        let iv = cut_intervals(&doc, &cuts.cuts);
        assert_eq!(iv, vec![(0.0, 3.0)]);
        let _ = ClipCuts::new(); // touch
    }

    #[test]
    fn silence_cut_interval_preserves_encoded_pause() {
        // A silence cut between w0 (0..1) and w1 (3..4) removes 1.7s,
        // retaining 0.3s after w0. It never cuts either flanking word.
        let doc = doc_with_words(&[("w0", 0.0, 1.0), ("w1", 3.0, 4.0)]);
        let cut = crate::data::soft_cut::Cut {
            id: "c".into(),
            note: None,
            a_word: "w0".into(),
            b_word: "w1".into(),
            kind: CutKind::Silence,
            duration: 1.7,
        };
        let iv = cut_intervals(&doc, &[cut]);
        assert!((iv[0].0 - 1.3).abs() < 1e-9);
        assert_eq!(iv[0].1, 3.0);
        assert!((removed_duration(&doc, &[]) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn kept_intervals_are_the_complement_of_merged_cuts() {
        let doc = doc_with_words(&[("w0", 0.0, 1.0), ("w1", 1.0, 3.0), ("w2", 3.0, 5.0)]);
        let kept = kept_intervals(&doc, &[cut("w1", "w1")]);
        assert_eq!(kept, vec![(0.0, 1.0), (3.0, 10.0)]);
    }
}
