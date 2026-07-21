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
