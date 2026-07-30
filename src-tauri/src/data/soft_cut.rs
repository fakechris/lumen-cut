//! Soft-cut regions on `Clip.cut`. Reversible, reviewable, applied at export
//! time. Word/translation timings are untouched by cuts; captions re-time at
//! export through the same kept-span map as the picture, so they can never
//! desynchronise.
//!
//! This is the Stage-4 slice of the soft-cut data model. The deterministic
//! pass (`cut detect`) is implemented downstream in `crate::pipeline::cut`.

use serde::{Deserialize, Serialize};

use crate::data::doc::Doc;

/// A single soft cut on the timeline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Cut {
    /// Stable cut id (string form like `c-s4-e2.10-s4-e8.40`). Used by
    /// `cut restore <id>` and by the audit engine to reference cuts by
    /// provenance.
    pub id: String,
    /// Optional user-supplied note ("retake", "long pause", …).
    #[serde(default)]
    pub note: Option<String>,
    /// Inclusive start word id (e.g. `wN`).
    pub a_word: String,
    /// Inclusive end word id.
    pub b_word: String,
    /// Source-coded kind. ``silence`` is the deterministic pass's default;
    /// `filler` cuts hesitation words; `retake` cuts a failed attempt at the
    /// same idea (the kept take is untouched); `falseStart` cuts a short
    /// abandoned fragment after a pause; `badTake` is an umbrella for
    /// everything else.
    pub kind: CutKind,
    /// Seconds actually removed from the source timeline. For word cuts this
    /// normally equals `b_word.end - a_word.start`. For silence compression
    /// it is smaller than the full inter-word gap because the surviving
    /// pause is encoded as `gap - duration`.
    pub duration: f64,
}

impl Cut {
    /// Resolve this cut to the exact source-timeline interval removed at
    /// export. Silence anchors are the words flanking a pause; `duration`
    /// records only the removed portion, so the surviving pause stays after
    /// the left word. Legacy silence cuts whose duration equals the entire
    /// gap continue to close the gap completely.
    pub fn resolved_interval(&self, doc: &Doc) -> Option<(f64, f64)> {
        let a = doc
            .all_words()
            .into_iter()
            .find(|word| word.id == self.a_word)?;
        let b = doc
            .all_words()
            .into_iter()
            .find(|word| word.id == self.b_word)?;
        match self.kind {
            CutKind::Silence => {
                let gap = (b.start - a.end).max(0.0);
                let removed = self.duration.clamp(0.0, gap);
                Some((b.start - removed, b.start))
            }
            _ => Some((a.start, b.end)),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CutKind {
    Silence,
    Filler,
    Retake,
    FalseStart,
    BadTake,
    Manual,
}

/// The full cut list stored on the project document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ClipCuts {
    pub cuts: Vec<Cut>,
}

impl ClipCuts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a cut. Stable id is generated from kind + range to allow audit
    /// to reason about cuts across reapplies.
    pub fn add(&mut self, cut: Cut) {
        self.cuts.push(cut);
    }

    /// Restore (delete) a cut by id. Returns true if a cut was removed.
    pub fn restore(&mut self, id: &str) -> bool {
        let before = self.cuts.len();
        self.cuts.retain(|c| c.id != id);
        before != self.cuts.len()
    }

    /// Restore the source-timeline range `[start, end)`: every cut covering
    /// part of the range is removed or trimmed so the range becomes kept
    /// again. Returns the seconds given back to the timeline (0 = no-op).
    ///
    /// Word-anchored cuts split at word boundaries: covered words that fall
    /// fully inside the range are un-cut, and the survivors keep their cut
    /// as up to two trimmed clones of the original. Silence cuts encode only
    /// a removed *duration* anchored at the right flanking word, so a
    /// partial restore shortens that duration instead.
    pub fn restore_range(&mut self, doc: &Doc, start: f64, end: f64) -> f64 {
        const EPS: f64 = 1e-3;
        if end <= start + EPS {
            return 0.0;
        }
        let words = doc.all_words();
        let mut pieces: Vec<(f64, f64)> = Vec::new();
        let mut next: Vec<Cut> = Vec::with_capacity(self.cuts.len());
        for cut in self.cuts.drain(..) {
            let Some((cut_start, cut_end)) = cut.resolved_interval(doc) else {
                next.push(cut);
                continue;
            };
            let overlap = (cut_end.min(end) - cut_start.max(start)).max(0.0);
            if overlap <= EPS {
                next.push(cut);
                continue;
            }
            if cut.kind == CutKind::Silence {
                pieces.push((cut_start.max(start), cut_end.min(end)));
                let remaining = cut.duration - overlap;
                if remaining > EPS {
                    let mut trimmed = cut;
                    trimmed.duration = remaining;
                    next.push(trimmed);
                }
                continue;
            }
            // Word-anchored cut: un-cut the covered words inside the range,
            // keep the survivors as contiguous trimmed runs.
            let lo = words.iter().position(|word| word.id == cut.a_word);
            let hi = words.iter().position(|word| word.id == cut.b_word);
            let (Some(lo), Some(hi)) = (lo, hi) else {
                next.push(cut);
                continue;
            };
            let (lo, hi) = (lo.min(hi), lo.max(hi));
            let survivors: Vec<usize> = (lo..=hi)
                .filter(|&index| {
                    let word = words[index];
                    word.start < start - EPS || word.end > end + EPS
                })
                .collect();
            if survivors.len() == hi - lo + 1 {
                // The range clips word interiors but no whole word — keep the
                // cut untouched rather than guessing at sub-word timing.
                next.push(cut);
                continue;
            }
            pieces.push((cut_start.max(start), cut_end.min(end)));
            let mut run_start: Option<usize> = None;
            let mut previous: Option<usize> = None;
            let mut part = 0usize;
            for index in survivors.into_iter().chain(std::iter::once(usize::MAX)) {
                let contiguous = previous.is_some_and(|prev| index == prev + 1);
                if !contiguous {
                    if let (Some(first), Some(last)) = (run_start, previous) {
                        part += 1;
                        next.push(Cut {
                            id: format!("{}~{part}", cut.id),
                            note: cut.note.clone(),
                            a_word: words[first].id.clone(),
                            b_word: words[last].id.clone(),
                            kind: cut.kind,
                            duration: (words[last].end - words[first].start).max(0.0),
                        });
                    }
                    run_start = (index != usize::MAX).then_some(index);
                }
                previous = (index != usize::MAX).then_some(index);
            }
        }
        self.cuts = next;
        // Seconds restored, unioned so overlapping cuts do not double-count.
        pieces.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut restored = 0.0;
        let mut cursor: Option<(f64, f64)> = None;
        for (piece_start, piece_end) in pieces {
            match cursor.as_mut() {
                Some((_, last_end)) if piece_start <= *last_end => {
                    restored += (piece_end - *last_end).max(0.0);
                    *last_end = last_end.max(piece_end);
                }
                _ => {
                    restored += piece_end - piece_start;
                    cursor = Some((piece_start, piece_end));
                }
            }
        }
        restored
    }

    /// Total seconds removed, used as a `>40%` WARN gate.
    pub fn total_duration(&self) -> f64 {
        self.cuts.iter().map(|c| c.duration).sum()
    }
}

/// Cut region relative to a word-aligned projection. Used by export to skip
/// spans without mutating the source `Doc`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct KeptSpan {
    pub start: f64,
    pub end: f64,
}

/// Given cuts that each express a `a..b` word-id span with start/end timings,
/// return the **kept** spans (the timeline minus the cut durations).
///
/// The output timeline is monotonically increasing; cuts are absorbed onto
/// the right edge as a fixed offset. The total preserved duration equals
/// `media.duration - sum(cuts.duration)` plus any silence within `compress-to`
/// retention that the export renderer fills.
// pub fn kept_spans: careful — Stage 4 calls into this from `pipeline::cleanup::render`.
pub fn kept_spans(doc: &Doc, cuts: &[Cut]) -> Vec<KeptSpan> {
    let mut intervals: Vec<(f64, f64)> = cuts
        .iter()
        .filter_map(|cut| cut.resolved_interval(doc))
        .collect();
    intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    // Drop degenerate zero-length.
    intervals.retain(|(a, b)| b > a);

    let mut kept: Vec<KeptSpan> = Vec::new();
    let mut cursor = 0.0;
    for (cs, ce) in intervals {
        if cs > cursor {
            kept.push(KeptSpan {
                start: cursor,
                end: cs,
            });
        }
        cursor = cursor.max(ce);
    }
    if cursor < doc.media.duration_seconds {
        kept.push(KeptSpan {
            start: cursor,
            end: doc.media.duration_seconds,
        });
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::doc::{MediaRef, Meta, Paragraph, Sentence, Word};
    use chrono::Utc;

    fn fixture() -> Doc {
        Doc {
            id: "p".into(),
            schema: 1,
            media: MediaRef {
                path: Default::default(),
                duration_seconds: 5.0,
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
            paragraphs: vec![Paragraph {
                id: 1,
                speaker: None,
                sentences: vec![Sentence {
                    id: "s1".into(),
                    text: "alpha beta gamma delta epsilon".into(),
                    words: vec![
                        ("w0", 0.0, 1.0),
                        ("w1", 1.0, 2.0),
                        ("w2", 3.0, 4.0),
                        ("w3", 4.0, 4.5),
                        ("w4", 4.5, 5.0),
                    ]
                    .into_iter()
                    .map(|(id, s, e)| Word {
                        id: id.into(),
                        text: id.into(),
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
    fn kept_spans_preserve_compressed_silence() {
        let doc = fixture();
        let cuts = vec![Cut {
            id: "c1".into(),
            note: None,
            a_word: "w1".into(),
            b_word: "w2".into(),
            kind: CutKind::Silence,
            duration: 0.7,
        }];
        let kept = kept_spans(&doc, &cuts);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].start, 0.0);
        assert!((kept[0].end - 2.3).abs() < 1e-9);
        assert_eq!(kept[1].start, 3.0);
        assert_eq!(kept[1].end, 5.0);
    }

    #[test]
    fn legacy_full_gap_silence_still_closes_gap() {
        let doc = fixture();
        let cut = Cut {
            id: "c1".into(),
            note: None,
            a_word: "w1".into(),
            b_word: "w2".into(),
            kind: CutKind::Silence,
            duration: 1.0,
        };
        assert_eq!(cut.resolved_interval(&doc), Some((2.0, 3.0)));
    }

    #[test]
    fn restore_removes_by_id() {
        let mut c = ClipCuts::new();
        c.add(Cut {
            id: "c1".into(),
            note: None,
            a_word: "w0".into(),
            b_word: "w1".into(),
            kind: CutKind::Manual,
            duration: 1.0,
        });
        assert!(c.restore("c1"));
        assert!(!c.restore("c1"));
    }

    fn word_cut(id: &str, a: &str, b: &str, a_start: f64, b_end: f64) -> Cut {
        Cut {
            id: id.into(),
            note: Some("removed".into()),
            a_word: a.into(),
            b_word: b.into(),
            kind: CutKind::Manual,
            duration: b_end - a_start,
        }
    }

    #[test]
    fn restore_range_removes_fully_covered_cut() {
        let doc = fixture();
        let mut cuts = ClipCuts {
            cuts: vec![word_cut("c1", "w1", "w2", 1.0, 4.0)],
        };
        let restored = cuts.restore_range(&doc, 0.5, 4.5);
        assert!((restored - 3.0).abs() < 1e-9);
        assert!(cuts.cuts.is_empty());
        assert_eq!(kept_spans(&doc, &cuts.cuts).len(), 1);
    }

    #[test]
    fn restore_range_splits_cut_around_restored_word() {
        let doc = fixture(); // w1: 1..2, w2: 3..4, w3: 4..4.5
        let mut cuts = ClipCuts {
            cuts: vec![word_cut("c1", "w1", "w3", 1.0, 4.5)],
        };
        let restored = cuts.restore_range(&doc, 3.0, 4.0); // exactly w2
        assert!((restored - 1.0).abs() < 1e-9);
        assert_eq!(cuts.cuts.len(), 2);
        assert_eq!(cuts.cuts[0].a_word, "w1");
        assert_eq!(cuts.cuts[0].b_word, "w1");
        assert!((cuts.cuts[0].duration - 1.0).abs() < 1e-9);
        assert_eq!(cuts.cuts[1].a_word, "w3");
        assert_eq!(cuts.cuts[1].b_word, "w3");
        assert_eq!(cuts.cuts[1].kind, CutKind::Manual);
        assert_eq!(cuts.cuts[1].note.as_deref(), Some("removed"));
        // The restored word is kept again; its neighbours are still cut.
        let kept = kept_spans(&doc, &cuts.cuts);
        assert!(kept.iter().any(|span| span.start <= 3.0 && span.end >= 4.0));
    }

    #[test]
    fn restore_range_trims_cut_edges() {
        let doc = fixture();
        let mut cuts = ClipCuts {
            cuts: vec![word_cut("c1", "w1", "w3", 1.0, 4.5)],
        };
        let restored = cuts.restore_range(&doc, 0.0, 2.5); // restores w1 only
                                                           // The cut interval is continuous, so the pause up to 2.5 returns too.
        assert!((restored - 1.5).abs() < 1e-9);
        assert_eq!(cuts.cuts.len(), 1);
        assert_eq!(cuts.cuts[0].a_word, "w2");
        assert_eq!(cuts.cuts[0].b_word, "w3");
        assert!((cuts.cuts[0].duration - 1.5).abs() < 1e-9);
    }

    #[test]
    fn restore_range_spans_multiple_cuts() {
        let doc = fixture();
        let mut cuts = ClipCuts {
            cuts: vec![
                word_cut("c1", "w0", "w1", 0.0, 2.0),
                word_cut("c2", "w3", "w4", 4.0, 5.0),
            ],
        };
        let restored = cuts.restore_range(&doc, 1.0, 4.5);
        // Restores w1 (1s) from c1 and w3 (0.5s) from c2; w2 was never cut.
        assert!((restored - 1.5).abs() < 1e-9);
        assert_eq!(cuts.cuts.len(), 2);
        assert_eq!(cuts.cuts[0].a_word, "w0");
        assert_eq!(cuts.cuts[0].b_word, "w0");
        assert_eq!(cuts.cuts[1].a_word, "w4");
        assert_eq!(cuts.cuts[1].b_word, "w4");
    }

    #[test]
    fn restore_range_without_overlap_is_noop() {
        let doc = fixture();
        let mut cuts = ClipCuts {
            cuts: vec![word_cut("c1", "w0", "w1", 0.0, 2.0)],
        };
        let before = cuts.cuts.clone();
        assert_eq!(cuts.restore_range(&doc, 2.0, 3.0), 0.0); // the pause
        assert_eq!(cuts.cuts, before);
    }

    #[test]
    fn restore_range_inside_a_word_keeps_cut_untouched() {
        let doc = fixture();
        let mut cuts = ClipCuts {
            cuts: vec![word_cut("c1", "w0", "w0", 0.0, 1.0)],
        };
        let before = cuts.cuts.clone();
        assert_eq!(cuts.restore_range(&doc, 0.25, 0.75), 0.0);
        assert_eq!(cuts.cuts, before);
    }

    #[test]
    fn restore_range_shortens_silence_cut_duration() {
        let doc = fixture(); // gap between w1 (ends 2.0) and w2 (starts 3.0)
        let mut cuts = ClipCuts {
            cuts: vec![Cut {
                id: "c1".into(),
                note: None,
                a_word: "w1".into(),
                b_word: "w2".into(),
                kind: CutKind::Silence,
                duration: 0.7,
            }],
        };
        // Resolved window is (2.3, 3.0); restoring 2.5..4.0 eats 0.5s of it.
        let restored = cuts.restore_range(&doc, 2.5, 4.0);
        assert!((restored - 0.5).abs() < 1e-9);
        assert_eq!(cuts.cuts.len(), 1);
        assert!((cuts.cuts[0].duration - 0.2).abs() < 1e-9);
        // Restoring the rest removes the cut entirely.
        let restored = cuts.restore_range(&doc, 0.0, 3.0);
        assert!((restored - 0.2).abs() < 1e-9);
        assert!(cuts.cuts.is_empty());
    }

    #[test]
    fn cut_kind_serde_backward_compatible() {
        // Old cuts.json files (written before `FalseStart` existed) must
        // still deserialize: every legacy kind string is unaffected.
        let old = r#"{"cuts":[
            {"id":"c1","a_word":"w0","b_word":"w1","kind":"silence","duration":1.0},
            {"id":"c2","a_word":"w1","b_word":"w2","kind":"filler","duration":0.5},
            {"id":"c3","a_word":"w2","b_word":"w3","kind":"retake","duration":2.0},
            {"id":"c4","a_word":"w3","b_word":"w4","kind":"badtake","duration":3.0},
            {"id":"c5","a_word":"w0","b_word":"w4","kind":"manual","duration":4.0}
        ]}"#;
        let cuts: ClipCuts = serde_json::from_str(old).unwrap();
        assert_eq!(cuts.cuts.len(), 5);
        assert_eq!(cuts.cuts[2].kind, CutKind::Retake);
        assert_eq!(cuts.cuts[3].kind, CutKind::BadTake);

        // `falsestart` round-trips with the enum's lowercase convention.
        let json = serde_json::to_string(&CutKind::FalseStart).unwrap();
        assert_eq!(json, "\"falsestart\"");
        let back: CutKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, CutKind::FalseStart);
    }
}
