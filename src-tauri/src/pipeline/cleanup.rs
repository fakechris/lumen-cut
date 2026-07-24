//! `runCleanup` closure chain — retake / falseStart / filler.
//!
//! Three detectors applied in order to a `Doc`.  Each detector returns
//! a `Vec<Cut>` proposal.  The proposal is appended to `ClipCuts` but
//! *only* if `apply()` is called; the API is non-mutating by default
//! so callers can preview before persisting.
//!
//! | Detector       | What it flags
//! |----------------|------------------------------------------------
//! | retake         | consecutive similar sentences (`jaccard >= 0.85`);
//! |                | cuts only the earlier, abandoned take
//! | falseStart     | short trailing fragment (`<= 3` words) after a
//! |                | `> 0.8s` gap; cuts only the fragment
//! | filler         | any word normalising onto the hard-filler list;
//! |                | cuts only that word
//!
//! The retake detector uses a **bag-of-trigrams Jaccard** similarity so
//! it works across CJK and Latin scripts without tokenizer coupling.
//! Only the hard list is cut deterministically. Context-sensitive fillers
//! such as 嗯 / 啊 / 那个 / 就是 / 其实 must not be hard-cut here.

use std::collections::{BTreeMap, HashSet};

use crate::data::doc::{Doc, Sentence};
use crate::data::soft_cut::{ClipCuts, Cut, CutKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupKind {
    Retake,
    FalseStart,
    Filler,
    /// A long pause between two adjacent words (`--min-pause` defaults to
    /// 0.8s). The pause is excised at export; the
    /// flanking words are kept.
    Silence,
}

#[derive(Debug, Clone)]
pub struct CleanupHit {
    pub kind: CleanupKind,
    pub a_sentence: String,
    pub b_sentence: String,
    /// Exact word a `Filler` hit targets; `None` for the sentence-span
    /// detectors (retake / falseStart). For `Silence`, `word_id` is the
    /// word before the pause and `word_id2` the word after it.
    pub word_id: Option<String>,
    pub word_id2: Option<String>,
    pub note: String,
}

/// Bag-of-trigrams Jaccard similarity (`|A ∩ B| / |A ∪ B|`). O(n) per pair.
fn trigram_jaccard(a: &str, b: &str) -> f64 {
    let tri = |s: &str| -> Vec<(char, char, char)> {
        let chars: Vec<char> = s.chars().collect();
        chars.windows(3).map(|w| (w[0], w[1], w[2])).collect()
    };
    let ta: HashSet<(char, char, char)> = tri(a).into_iter().collect();
    let tb: HashSet<(char, char, char)> = tri(b).into_iter().collect();
    if ta.is_empty() || tb.is_empty() {
        return 0.0;
    }
    let inter = ta.intersection(&tb).count() as f64;
    let uni = (ta.len() + tb.len()) as f64 - inter;
    if uni == 0.0 {
        0.0
    } else {
        inter / uni
    }
}

/// Minimum inter-word gap treated as a compressible silence. This pass uses
/// word timestamps rather than raw audio frames.
pub const MIN_PAUSE: f64 = 0.8;

/// Default surviving pause for an intra-sentence silence (`--compress-to
/// 300ms`).
pub const SILENCE_COMPRESS_TO: f64 = 0.3;

/// Cross-sentence pauses preserve a little more cadence at the sentence end.
pub const SENTENCE_END_RETAIN: f64 = 0.4;

/// Longer gaps are treated as protected chapter boundaries by the default
/// deterministic pass (`--max-gap 3.0`).
pub const MAX_SILENCE_GAP: f64 = 3.0;

/// Seconds to remove from a silence gap under the default cleanup policy.
pub fn compressed_silence_duration(gap: f64, sentence_end: bool) -> f64 {
    compressed_silence_duration_with(gap, sentence_end, SILENCE_COMPRESS_TO, SENTENCE_END_RETAIN)
}

/// Seconds removed when the surviving pause length is configured.
pub fn compressed_silence_duration_with(
    gap: f64,
    sentence_end: bool,
    compress_to: f64,
    sentence_end_retain: f64,
) -> f64 {
    let retain = if sentence_end {
        sentence_end_retain
    } else {
        compress_to
    };
    (gap - retain.max(0.0)).max(0.0)
}

/// Tunables for the deterministic detect pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectOptions {
    /// Minimum inter-word gap treated as compressible silence (seconds).
    pub min_pause: f64,
    /// Surviving pause for an intra-sentence silence (seconds).
    pub compress_to: f64,
    /// Surviving pause at a sentence boundary (seconds). Defaults to
    /// `max(compress_to, 0.4)` when left as the module default.
    pub sentence_end_retain: f64,
    /// Gaps longer than this are protected as deliberate beats (seconds).
    pub max_gap: f64,
    /// When false, skip Category-1 filler hard cuts.
    pub fillers: bool,
    /// When false, skip silence compression proposals.
    pub pauses: bool,
}

impl Default for DetectOptions {
    fn default() -> Self {
        Self {
            min_pause: MIN_PAUSE,
            compress_to: SILENCE_COMPRESS_TO,
            sentence_end_retain: SENTENCE_END_RETAIN,
            max_gap: MAX_SILENCE_GAP,
            fillers: true,
            pauses: true,
        }
    }
}

/// Hard-delete filler list. Context-sensitive words such as 嗯 / 啊 / 那个 /
/// 就是 / 其实 may only be cut by the AI review, never deterministically.
const HARD_FILLERS: &[&str] = &[
    "um", "umm", "uh", "uhh", "er", "erm", "ah", "hmm", "mhm", "呃", "额",
];

/// Lowercase and strip leading/trailing non-alphanumerics, so `"Um,"`
/// normalises to `um` and `呃，` to `呃`.
fn normalize_word(text: &str) -> String {
    text.trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

/// Run all detectors with default talking-head thresholds.
pub fn detect(doc: &Doc) -> Vec<CleanupHit> {
    detect_with(doc, DetectOptions::default())
}

/// Run detectors with explicit thresholds. Pure — does not mutate `doc` or cuts.
pub fn detect_with(doc: &Doc, options: DetectOptions) -> Vec<CleanupHit> {
    let mut out = Vec::new();
    let all_sents: Vec<&Sentence> = doc
        .paragraphs
        .iter()
        .flat_map(|p| p.sentences.iter())
        .collect();

    // (1) retake — consecutive near-identical sentences; exact verbatim
    // repeats (jaccard == 1.0) are the most common case and are included.
    for w in all_sents.windows(2) {
        let sim = trigram_jaccard(&w[0].text, &w[1].text);
        if sim >= 0.85 {
            out.push(CleanupHit {
                kind: CleanupKind::Retake,
                a_sentence: w[0].id.clone(),
                b_sentence: w[1].id.clone(),
                word_id: None,
                word_id2: None,
                note: format!("jaccard={sim:.2}"),
            });
        }
    }

    // (2) falseStart — short trailing fragment after a pause.
    for w in all_sents.windows(2) {
        let prev_end = w[0].words.last().map(|x| x.end).unwrap_or(0.0);
        let next_start = w[1].words.first().map(|x| x.start).unwrap_or(0.0);
        let gap = next_start - prev_end;
        if gap > options.min_pause && w[1].words.len() <= 3 {
            out.push(CleanupHit {
                kind: CleanupKind::FalseStart,
                a_sentence: w[0].id.clone(),
                b_sentence: w[1].id.clone(),
                word_id: None,
                word_id2: None,
                note: format!("gap={gap:.2}s short"),
            });
        }
    }

    // (3) filler — word level hard list only.
    if options.fillers {
        for s in &all_sents {
            for word in &s.words {
                let norm = normalize_word(&word.text);
                if !norm.is_empty() && HARD_FILLERS.contains(&norm.as_str()) {
                    out.push(CleanupHit {
                        kind: CleanupKind::Filler,
                        a_sentence: s.id.clone(),
                        b_sentence: s.id.clone(),
                        word_id: Some(word.id.clone()),
                        word_id2: None,
                        note: format!("filler word {:?}", word.text),
                    });
                }
            }
        }
    }

    // (4) silence — adjacent words whose inter-word gap is within
    // [min_pause, max_gap]. Longer gaps are protected as deliberate beats.
    if options.pauses {
        let flat: Vec<(&str, &crate::data::Word)> = doc
            .paragraphs
            .iter()
            .flat_map(|p| {
                p.sentences
                    .iter()
                    .flat_map(|s| s.words.iter().map(|w| (s.id.as_str(), w)))
            })
            .collect();
        for pair in flat.windows(2) {
            let (sa, wa) = pair[0];
            let (sb, wb) = pair[1];
            let gap = wb.start - wa.end;
            if gap >= options.min_pause && gap <= options.max_gap {
                let retained = if sa != sb {
                    options.sentence_end_retain
                } else {
                    options.compress_to
                };
                out.push(CleanupHit {
                    kind: CleanupKind::Silence,
                    a_sentence: sa.into(),
                    b_sentence: sb.into(),
                    word_id: Some(wa.id.clone()),
                    word_id2: Some(wb.id.clone()),
                    note: format!("silence gap {gap:.2}s, retain {retained:.1}s"),
                });
            }
        }
    }

    out
}

/// Convert a `CleanupHit` into a `Cut` proposal by joining against the
/// sentence-level word ids. The pipeline never reads raw timings here —
/// the export module does that.
///
/// The span is kind-specific so a cut never takes kept material with it:
///
/// * retake cuts only the earlier, abandoned take (sentence `a`); the
///   later, complete take is preserved.
/// * falseStart cuts only the short trailing fragment (sentence `b`).
/// * filler cuts exactly the flagged word (`a_word == b_word`).
pub fn cut_from_hit(doc: &Doc, hit: &CleanupHit) -> Option<Cut> {
    cut_from_hit_with(doc, hit, DetectOptions::default())
}

/// Convert a hit using the same compress thresholds that produced it.
pub fn cut_from_hit_with(
    doc: &Doc,
    hit: &CleanupHit,
    options: DetectOptions,
) -> Option<Cut> {
    let (a_word, b_word, kind) = match hit.kind {
        CleanupKind::Retake => {
            let a = find_sentence(doc, &hit.a_sentence)?;
            (
                a.words.first()?.id.clone(),
                a.words.last()?.id.clone(),
                CutKind::Retake,
            )
        }
        CleanupKind::FalseStart => {
            let b = find_sentence(doc, &hit.b_sentence)?;
            (
                b.words.first()?.id.clone(),
                b.words.last()?.id.clone(),
                CutKind::FalseStart,
            )
        }
        CleanupKind::Filler => {
            let w = hit.word_id.clone()?;
            (w.clone(), w, CutKind::Filler)
        }
        CleanupKind::Silence => {
            let prev = hit.word_id.clone()?;
            let next = hit.word_id2.clone()?;
            (prev, next, CutKind::Silence)
        }
    };
    let word_at: BTreeMap<&str, (f64, f64)> = doc
        .all_words()
        .into_iter()
        .map(|w| (w.id.as_str(), (w.start, w.end)))
        .collect();
    let dur = match kind {
        CutKind::Silence => word_at
            .get(a_word.as_str())
            .zip(word_at.get(b_word.as_str()))
            .map(|((_, e0), (s1, _))| {
                compressed_silence_duration_with(
                    (s1 - e0).max(0.0),
                    hit.a_sentence != hit.b_sentence,
                    options.compress_to,
                    options.sentence_end_retain,
                )
            })
            .unwrap_or(0.0),
        _ => word_at
            .get(a_word.as_str())
            .zip(word_at.get(b_word.as_str()))
            .map(|((s, _), (_, e))| e - s)
            .unwrap_or(0.0),
    };
    Some(Cut {
        id: format!("c-{:?}-{}-{}", hit.kind, a_word, b_word),
        note: Some(hit.note.clone()),
        a_word,
        b_word,
        kind,
        duration: dur,
    })
}

fn find_sentence<'a>(doc: &'a Doc, id: &str) -> Option<&'a Sentence> {
    doc.paragraphs
        .iter()
        .flat_map(|p| p.sentences.iter())
        .find(|s| s.id == id)
}

/// Convenience: run `detect` and append non-conflicting cuts to `cuts`.
/// Returns the number of cuts added.
pub fn apply(doc: &Doc, cuts: &mut ClipCuts) -> usize {
    apply_with(doc, cuts, DetectOptions::default())
}

/// Like [`apply`] with explicit detect thresholds.
pub fn apply_with(doc: &Doc, cuts: &mut ClipCuts, options: DetectOptions) -> usize {
    let mut added = 0;
    for hit in detect_with(doc, options) {
        if let Some(cut) = cut_from_hit_with(doc, &hit, options) {
            let id = cut.id.clone();
            if !cuts.cuts.iter().any(|c| c.id == id) {
                cuts.add(cut);
                added += 1;
            }
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::doc::*;
    use chrono::Utc;
    use std::path::PathBuf;

    fn doc_with(sents: Vec<Sentence>) -> Doc {
        Doc {
            id: "p".into(),
            schema: 1,
            media: MediaRef {
                path: PathBuf::from("/tmp/x.mp4"),
                duration_seconds: 30.0,
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
                sentences: sents,
            }],
            translations: Default::default(),
        }
    }

    /// Build a sentence from `(word_id, word_text, start, end)` tuples.
    fn sent(id: &str, text: &str, words: Vec<(&str, &str, f64, f64)>) -> Sentence {
        Sentence {
            id: id.into(),
            text: text.into(),
            words: words
                .into_iter()
                .map(|(wid, wtext, s, e)| Word {
                    id: wid.into(),
                    text: wtext.into(),
                    start: s,
                    end: e,
                })
                .collect(),
        }
    }

    #[test]
    fn filler_detected() {
        let d = doc_with(vec![Sentence {
            id: "s1".into(),
            text: "um".into(),
            words: vec![Word {
                id: "w0".into(),
                text: "um".into(),
                start: 0.0,
                end: 0.2,
            }],
        }]);
        let hits = detect(&d);
        assert!(hits.iter().any(|h| h.kind == CleanupKind::Filler));
    }

    #[test]
    fn retake_detected_with_high_similarity() {
        let d = doc_with(vec![
            Sentence {
                id: "s1".into(),
                text: "today we are going to talk about the framework".into(),
                words: vec![Word {
                    id: "w0".into(),
                    text: "today".into(),
                    start: 0.0,
                    end: 0.3,
                }],
            },
            Sentence {
                id: "s2".into(),
                text: "today we are going to talk about the framework design".into(),
                words: vec![Word {
                    id: "w1".into(),
                    text: "today".into(),
                    start: 0.5,
                    end: 0.8,
                }],
            },
        ]);
        let hits = detect(&d);
        assert!(hits.iter().any(|h| h.kind == CleanupKind::Retake));
    }

    #[test]
    fn retake_detects_exact_duplicate() {
        // Verbatim re-recording (jaccard == 1.0) is the most typical
        // retake and must not be excluded.
        let d = doc_with(vec![
            sent(
                "s1",
                "the quick brown fox jumps",
                vec![("w0", "the", 0.0, 0.5), ("w1", "quick", 0.5, 1.0)],
            ),
            sent(
                "s2",
                "the quick brown fox jumps",
                vec![("w2", "the", 1.1, 1.6), ("w3", "quick", 1.6, 2.1)],
            ),
        ]);
        let hits = detect(&d);
        assert!(hits.iter().any(|h| h.kind == CleanupKind::Retake));
    }

    #[test]
    fn retake_cut_covers_only_abandoned_take() {
        let d = doc_with(vec![
            sent(
                "s1",
                "the quick brown fox jumps",
                vec![("w0", "the", 0.0, 0.5), ("w1", "quick", 0.5, 1.0)],
            ),
            sent(
                "s2",
                "the quick brown fox jumps",
                vec![("w2", "the", 1.1, 1.6), ("w3", "quick", 1.6, 2.1)],
            ),
        ]);
        let hit = detect(&d)
            .into_iter()
            .find(|h| h.kind == CleanupKind::Retake)
            .unwrap();
        let cut = cut_from_hit(&d, &hit).unwrap();
        // Only the abandoned first take is cut; the kept take's words
        // (w2..w3) must stay outside the span.
        assert_eq!(cut.kind, CutKind::Retake);
        assert_eq!(cut.a_word, "w0");
        assert_eq!(cut.b_word, "w1");
        assert!((cut.duration - 1.0).abs() < 1e-9);
    }

    #[test]
    fn false_start_cut_covers_only_fragment_and_is_typed() {
        let d = doc_with(vec![
            sent(
                "s1",
                "let me start",
                vec![
                    ("w0", "let", 0.0, 0.4),
                    ("w1", "me", 0.4, 0.8),
                    ("w2", "start", 0.8, 1.2),
                ],
            ),
            sent(
                "s2",
                "I think",
                vec![("w3", "I", 2.5, 2.8), ("w4", "think", 2.8, 3.1)],
            ),
        ]);
        let hit = detect(&d)
            .into_iter()
            .find(|h| h.kind == CleanupKind::FalseStart)
            .unwrap();
        let cut = cut_from_hit(&d, &hit).unwrap();
        // Only the short fragment (s2) is cut, s1 is untouched, and the
        // kind is no longer mislabelled as `Filler`.
        assert_eq!(cut.kind, CutKind::FalseStart);
        assert_eq!(cut.a_word, "w3");
        assert_eq!(cut.b_word, "w4");
        assert!((cut.duration - 0.6).abs() < 1e-9);
    }

    #[test]
    fn filler_cut_targets_single_word_in_sentence() {
        // 呃 mid-sentence: only that word is cut, not the whole sentence.
        let d = doc_with(vec![sent(
            "s1",
            "呃我们开始吧",
            vec![
                ("w0", "呃", 0.0, 0.2),
                ("w1", "我们", 0.2, 0.5),
                ("w2", "开始吧", 0.5, 1.0),
            ],
        )]);
        let hits: Vec<_> = detect(&d)
            .into_iter()
            .filter(|h| h.kind == CleanupKind::Filler)
            .collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].word_id.as_deref(), Some("w0"));
        let cut = cut_from_hit(&d, &hits[0]).unwrap();
        assert_eq!(cut.kind, CutKind::Filler);
        assert_eq!(cut.a_word, "w0");
        assert_eq!(cut.b_word, "w0");
        assert!((cut.duration - 0.2).abs() < 1e-9);
    }

    #[test]
    fn filler_category2_words_are_not_hard_cut() {
        // Context-sensitive fillers are AI-review-only; the deterministic
        // pass must not flag them.
        let d = doc_with(vec![
            sent(
                "s1",
                "嗯我觉得",
                vec![("w0", "嗯", 0.0, 0.3), ("w1", "我觉得", 0.3, 0.8)],
            ),
            sent(
                "s2",
                "那个方案",
                vec![("w2", "那个", 0.9, 1.2), ("w3", "方案", 1.2, 1.6)],
            ),
            sent(
                "s3",
                "啊好吧",
                vec![("w4", "啊", 1.7, 1.9), ("w5", "好吧", 1.9, 2.3)],
            ),
        ]);
        let hits = detect(&d);
        assert!(!hits.iter().any(|h| h.kind == CleanupKind::Filler));
    }

    #[test]
    fn filler_normalizes_case_and_punctuation() {
        // `Um,` normalises (lowercase + strip non-alnum) onto `um`.
        let d = doc_with(vec![sent(
            "s1",
            "Um, I think",
            vec![
                ("w0", "Um,", 0.0, 0.3),
                ("w1", "I", 0.3, 0.45),
                ("w2", "think", 0.45, 0.9),
            ],
        )]);
        let hits: Vec<_> = detect(&d)
            .into_iter()
            .filter(|h| h.kind == CleanupKind::Filler)
            .collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].word_id.as_deref(), Some("w0"));
        let cut = cut_from_hit(&d, &hits[0]).unwrap();
        assert_eq!(cut.a_word, "w0");
        assert_eq!(cut.b_word, "w0");
    }

    #[test]
    fn apply_adds_unique_cuts() {
        let d = doc_with(vec![Sentence {
            id: "s1".into(),
            text: "uh".into(),
            words: vec![Word {
                id: "w0".into(),
                text: "uh".into(),
                start: 0.0,
                end: 0.2,
            }],
        }]);
        let mut cuts = ClipCuts::new();
        let added = apply(&d, &mut cuts);
        assert_eq!(added, 1);
        assert_eq!(apply(&d, &mut cuts), 0); // dedup
    }

    #[test]
    fn silence_gap_detected_and_cut_keeps_flanks() {
        let d = doc_with(vec![
            sent("s1", "hi", vec![("w0", "hi", 0.0, 0.5)]),
            sent("s2", "there", vec![("w1", "there", 2.0, 2.4)]), // gap 1.5s
        ]);
        let sil = detect(&d)
            .into_iter()
            .find(|h| h.kind == CleanupKind::Silence)
            .unwrap();
        assert_eq!(sil.word_id.as_deref(), Some("w0"));
        assert_eq!(sil.word_id2.as_deref(), Some("w1"));
        let cut = cut_from_hit(&d, &sil).unwrap();
        assert_eq!(cut.kind, CutKind::Silence);
        assert_eq!(cut.a_word, "w0");
        assert_eq!(cut.b_word, "w1");
        // Cross-sentence cadence keeps 0.4s: 1.5s gap - 0.4s retained.
        assert!((cut.duration - 1.1).abs() < 1e-9);
        assert!(cut
            .note
            .as_deref()
            .is_some_and(|note| note.contains("retain 0.4s")));
    }

    #[test]
    fn short_gap_is_not_silence() {
        let d = doc_with(vec![
            sent("s1", "hi", vec![("w0", "hi", 0.0, 0.5)]),
            sent("s2", "there", vec![("w1", "there", 0.7, 1.1)]), // gap 0.2s
        ]);
        assert!(!detect(&d).iter().any(|h| h.kind == CleanupKind::Silence));
    }

    #[test]
    fn detect_with_higher_min_pause_skips_short_gaps() {
        let doc = fixture_two_words_with_gap(0.9);
        let default_hits = detect(&doc);
        assert!(
            default_hits
                .iter()
                .any(|hit| matches!(hit.kind, CleanupKind::Silence)),
            "default 0.8s min should catch 0.9s gap"
        );
        let strict = detect_with(
            &doc,
            DetectOptions {
                min_pause: 1.0,
                ..DetectOptions::default()
            },
        );
        assert!(
            !strict
                .iter()
                .any(|hit| matches!(hit.kind, CleanupKind::Silence)),
            "min_pause 1.0 should skip 0.9s gap"
        );
    }

    fn fixture_two_words_with_gap(gap: f64) -> Doc {
        use crate::data::doc::*;
        Doc {
            id: "p".into(),
            schema: 1,
            media: MediaRef {
                path: std::path::PathBuf::from("m.mp4"),
                duration_seconds: 5.0,
                sample_rate: None,
                channels: None,
            },
            meta: Meta {
                title: "t".into(),
                description: String::new(),
                language: Some("en".into()),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            paragraphs: vec![Paragraph {
                id: 1,
                speaker: None,
                sentences: vec![Sentence {
                    id: "s1".into(),
                    text: "hello world".into(),
                    words: vec![
                        Word {
                            id: "w1".into(),
                            text: "hello".into(),
                            start: 0.0,
                            end: 0.5,
                        },
                        Word {
                            id: "w2".into(),
                            text: "world".into(),
                            start: 0.5 + gap,
                            end: 0.5 + gap + 0.4,
                        },
                    ],
                }],
            }],
            translations: Default::default(),
        }
    }

    #[test]
    fn intra_sentence_silence_compresses_to_three_hundred_ms() {
        let d = doc_with(vec![sent(
            "s1",
            "hi there",
            vec![
                ("w0", "hi", 0.0, 0.5),
                ("w1", "there", 2.0, 2.4), // 1.5s gap
            ],
        )]);
        let hit = detect(&d)
            .into_iter()
            .find(|hit| hit.kind == CleanupKind::Silence)
            .unwrap();
        let cut = cut_from_hit(&d, &hit).unwrap();
        assert!((cut.duration - 1.2).abs() < 1e-9);
        assert!(cut
            .note
            .as_deref()
            .is_some_and(|note| note.contains("retain 0.3s")));
    }

    #[test]
    fn chapter_sized_gap_is_protected() {
        let d = doc_with(vec![
            sent("s1", "chapter one", vec![("w0", "one", 0.0, 0.5)]),
            sent("s2", "chapter two", vec![("w1", "two", 3.6, 4.0)]),
        ]);
        assert!(!detect(&d)
            .iter()
            .any(|hit| hit.kind == CleanupKind::Silence));
    }
}
