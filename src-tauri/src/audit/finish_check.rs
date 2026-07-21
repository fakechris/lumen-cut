//! Finish-check pipeline.
//!
//! Each finish-check pass is **a separate code** so that the audit banner
//! can show progress through the export gate. The codes form an ordered
//! chain: passing each unlocks the next; failing emits a *blocker* finding
//! with `Severity::Fail` that the export step rejects.

use serde::{Deserialize, Serialize};

use std::path::Path;

use crate::audit::engine::{audit_project, audit_with_project, Code, Finding, Report, Severity};
use crate::data::doc::Doc;
use crate::data::soft_cut::ClipCuts;

/// The eight finish-check codes. Run order matters: 1..3 are pre-export
/// checks; 4..6 are pre-burn checks; 7..8 are final-export checks.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum FinishCheck {
    TranscribeComplete = 1,
    TranslationsFilled = 2,
    AuditPass = 3,
    AlignedWithMedia = 4,
    SoftCutsSane = 5,
    SpeakerLabels = 6,
    ExportReady = 7,
    VersionHeadCommitted = 8,
}

impl FinishCheck {
    pub fn all() -> [Self; 8] {
        [
            Self::TranscribeComplete,
            Self::TranslationsFilled,
            Self::AuditPass,
            Self::AlignedWithMedia,
            Self::SoftCutsSane,
            Self::SpeakerLabels,
            Self::ExportReady,
            Self::VersionHeadCommitted,
        ]
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::TranscribeComplete => "transcribe-complete",
            Self::TranslationsFilled => "translations-filled",
            Self::AuditPass => "audit-pass",
            Self::AlignedWithMedia => "aligned-with-media",
            Self::SoftCutsSane => "soft-cuts-sane",
            Self::SpeakerLabels => "speaker-labels",
            Self::ExportReady => "export-ready",
            Self::VersionHeadCommitted => "version-head-committed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmitItem {
    pub code: FinishCheck,
    pub pass: bool,
    pub blockers: Vec<Finding>,
    /// WARN findings are surfaced for review but never gate `pass`.
    pub warnings: Vec<Finding>,
}

/// Run all eight and emit a report.
pub fn finish_check_emit(doc: &Doc, cuts: &ClipCuts) -> Vec<EmitItem> {
    finish_check_emit_with_head(doc, cuts, false)
}

/// Project-aware finish check. `head_committed` is computed by comparing the
/// working document with the active branch tip snapshot.
pub fn finish_check_emit_with_head(
    doc: &Doc,
    cuts: &ClipCuts,
    head_committed: bool,
) -> Vec<EmitItem> {
    finish_check_emit_with_project(doc, cuts, &[], head_committed)
}

pub fn finish_check_emit_with_project(
    doc: &Doc,
    cuts: &ClipCuts,
    broll: &[crate::data::broll::BrollPlacement],
    head_committed: bool,
) -> Vec<EmitItem> {
    let audit_report = audit_with_project(doc, cuts, broll);
    finish_check_from_audit(doc, cuts, audit_report, head_committed)
}

/// Project entry point that includes persisted align/seam evidence in the
/// export gate.
pub fn finish_check_emit_for_project(
    doc: &Doc,
    cuts: &ClipCuts,
    broll: &[crate::data::broll::BrollPlacement],
    project_dir: &Path,
    head_committed: bool,
) -> Vec<EmitItem> {
    let audit_report = audit_project(doc, cuts, broll, project_dir);
    finish_check_from_audit(doc, cuts, audit_report, head_committed)
}

fn finish_check_from_audit(
    doc: &Doc,
    cuts: &ClipCuts,
    audit_report: Report,
    head_committed: bool,
) -> Vec<EmitItem> {
    let mut out = Vec::new();

    // (1) transcribe-complete
    let transcribe_blockers: Vec<Finding> = audit_report
        .by_code(Code::TranslationEmpty)
        .into_iter()
        .cloned()
        .collect();
    out.push(EmitItem {
        code: FinishCheck::TranscribeComplete,
        pass: transcribe_blockers.is_empty(),
        blockers: transcribe_blockers,
        warnings: vec![],
    });

    // (2) translations-filled
    let tx_blockers: Vec<Finding> = audit_report
        .by_code(Code::TranslationMissing)
        .into_iter()
        .cloned()
        .collect();
    out.push(EmitItem {
        code: FinishCheck::TranslationsFilled,
        pass: tx_blockers.is_empty(),
        blockers: tx_blockers,
        warnings: vec![],
    });

    // (3) audit-pass
    let audit_blockers: Vec<Finding> = audit_report
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Fail)
        .cloned()
        .collect();
    out.push(EmitItem {
        code: FinishCheck::AuditPass,
        pass: audit_blockers.is_empty(),
        blockers: audit_blockers,
        warnings: vec![],
    });

    // (4) aligned-with-media — the latest word end must fit the media.
    // Summing word durations is incorrect for pauses and overlaps.
    let latest_word_end: f64 = doc
        .all_words()
        .into_iter()
        .map(|word| word.end)
        .fold(0.0_f64, f64::max);
    let aligned_pass = latest_word_end <= doc.media.duration_seconds + 0.05;
    let mut blockers = Vec::new();
    if !aligned_pass {
        blockers.push(Finding {
            code: Code::WordTimeBoundary,
            severity: Severity::Fail,
            where_: "<doc>".into(),
            message: format!(
                "latest word exceeds media duration: {latest_word_end:.2}s > {:.2}s",
                doc.media.duration_seconds
            ),
        });
    }
    out.push(EmitItem {
        code: FinishCheck::AlignedWithMedia,
        pass: aligned_pass,
        blockers,
        warnings: vec![],
    });

    // (5) soft-cuts-sane: not more than 40% of media duration cut
    let cut_dur = crate::export::removed_duration(doc, &cuts.cuts);
    let ratio = cut_dur / doc.media.duration_seconds.max(0.001);
    let sane = ratio <= 0.40;
    let mut blockers = Vec::new();
    if !sane {
        blockers.push(Finding {
            code: Code::CutHeavyRemoval,
            severity: Severity::Fail,
            where_: "<cuts>".into(),
            message: format!("{ratio:.0}% of media is cut (>40%)"),
        });
    }
    out.push(EmitItem {
        code: FinishCheck::SoftCutsSane,
        pass: sane,
        blockers,
        warnings: vec![],
    });

    // (6) speaker-labels: missing labels are WARN-only advisories — they
    // land in `warnings` and never gate readiness.
    let unlabeled: Vec<Finding> = doc
        .paragraphs
        .iter()
        .filter(|p| p.speaker.is_none())
        .map(|p| Finding {
            code: Code::OrphanParagraphPin,
            severity: Severity::Warn,
            where_: format!("paragraph#{}", p.id),
            message: "missing speaker label".into(),
        })
        .collect();
    out.push(EmitItem {
        code: FinishCheck::SpeakerLabels,
        pass: true,
        blockers: vec![],
        warnings: unlabeled,
    });

    // (7) export-ready — composite of (3) + (4) + (5)
    let composite = out[2].pass && out[3].pass && out[4].pass;
    out.push(EmitItem {
        code: FinishCheck::ExportReady,
        pass: composite,
        blockers: if composite {
            vec![]
        } else {
            [&out[2].blockers, &out[3].blockers, &out[4].blockers]
                .into_iter()
                .flat_map(|v| v.iter().cloned())
                .collect()
        },
        warnings: vec![],
    });

    // (8) version-head-committed — compare the working document against the
    // active branch tip. A project with no lineage is intentionally not
    // export-ready until it has an explicit snapshot.
    let version_blockers = if head_committed {
        vec![]
    } else {
        vec![Finding {
            code: Code::PipelineFallback,
            severity: Severity::Fail,
            where_: "<version>".into(),
            message: "working document is not committed at the active branch tip".into(),
        }]
    };
    out.push(EmitItem {
        code: FinishCheck::VersionHeadCommitted,
        pass: head_committed,
        blockers: version_blockers.clone(),
        warnings: vec![],
    });
    if !head_committed {
        out[6].pass = false;
        out[6].blockers.extend(version_blockers);
    }

    out
}

/// Advisory fix plan. The fix pass never mutates `cuts.json`; it returns CLI
/// commands the user (or an agent) may run to clear each blocker.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FixAdvice {
    /// Number of failing items that received at least one suggestion.
    pub fixed: usize,
    /// Suggested commands, e.g. `lumen-cut cut <pid> --restore <cut-id>`.
    pub suggestions: Vec<String>,
}

/// Build the advisory fix plan for the failing items. Pure: `cuts` and
/// `doc` are only read, never modified.
pub fn finish_check_fix(pid: &str, items: &[EmitItem], cuts: &ClipCuts, doc: &Doc) -> FixAdvice {
    let mut advice = FixAdvice::default();
    for it in items {
        if it.pass {
            continue;
        }
        let before = advice.suggestions.len();
        match it.code {
            FinishCheck::TranscribeComplete => {
                advice
                    .suggestions
                    .push(format!("lumen-cut task start transcribe {pid}"));
            }
            FinishCheck::TranslationsFilled => {
                advice
                    .suggestions
                    .push(format!("lumen-cut task start translate {pid}"));
            }
            FinishCheck::AuditPass | FinishCheck::ExportReady => {
                advice.suggestions.push(format!("lumen-cut audit {pid}"));
            }
            FinishCheck::AlignedWithMedia => {
                advice
                    .suggestions
                    .push(format!("lumen-cut task start cleanup {pid}"));
            }
            FinishCheck::SoftCutsSane => {
                // Suggest restoring cuts (latest first) until the cut total
                // drops to ≤40% of the media duration. When the media
                // duration is missing, fall back to the last word's end.
                let media_secs = if doc.media.duration_seconds > 0.0 {
                    doc.media.duration_seconds
                } else {
                    doc.all_words()
                        .into_iter()
                        .map(|w| w.end)
                        .fold(0.0_f64, f64::max)
                };
                let denom = media_secs.max(0.001);
                let mut remaining = cuts.cuts.clone();
                remaining.sort_by(|left, right| {
                    let left_start = left
                        .resolved_interval(doc)
                        .map(|(start, _)| start)
                        .unwrap_or(f64::INFINITY);
                    let right_start = right
                        .resolved_interval(doc)
                        .map(|(start, _)| start)
                        .unwrap_or(f64::INFINITY);
                    left_start
                        .partial_cmp(&right_start)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| left.id.cmp(&right.id))
                });
                let ordered = remaining.clone();
                let mut cut_secs = crate::export::removed_duration(doc, &remaining);
                for c in ordered.iter().rev() {
                    if cut_secs / denom <= 0.40 {
                        break;
                    }
                    advice
                        .suggestions
                        .push(format!("lumen-cut cut {pid} --restore {}", c.id));
                    remaining.retain(|candidate| candidate.id != c.id);
                    cut_secs = crate::export::removed_duration(doc, &remaining);
                }
            }
            FinishCheck::VersionHeadCommitted => {
                advice.suggestions.push(format!(
                    "lumen-cut version commit {pid} \"ready for export\""
                ));
            }
            FinishCheck::SpeakerLabels => {}
        }
        if advice.suggestions.len() > before {
            advice.fixed += 1;
        }
    }
    advice
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::doc::*;
    use chrono::Utc;
    use std::path::PathBuf;

    fn empty_doc() -> Doc {
        Doc {
            id: "p".into(),
            schema: 1,
            media: MediaRef {
                path: PathBuf::from("/tmp/x.mp4"),
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
            paragraphs: vec![],
            translations: Default::default(),
        }
    }

    #[test]
    fn empty_doc_fails_transcribe_complete() {
        let r = finish_check_emit(&empty_doc(), &ClipCuts::new());
        assert!(!r[0].pass);
        assert_eq!(r[0].code, FinishCheck::TranscribeComplete);
    }

    #[test]
    fn huge_cut_ratio_fails_soft_cuts_sane() {
        let mut doc = empty_doc();
        doc.paragraphs.push(para_without_speaker(3.0));
        let mut cuts = ClipCuts::new();
        cuts.add(crate::data::soft_cut::Cut {
            id: "c1".into(),
            note: None,
            a_word: "w0".into(),
            b_word: "w0".into(),
            kind: crate::data::soft_cut::CutKind::Manual,
            duration: 3.0, // 60% of 5s
        });
        let r = finish_check_emit(&doc, &cuts);
        assert!(!r[4].pass);
    }

    fn cut(id: &str, duration: f64) -> crate::data::soft_cut::Cut {
        crate::data::soft_cut::Cut {
            id: id.into(),
            note: None,
            a_word: "w0".into(),
            b_word: "w0".into(),
            kind: crate::data::soft_cut::CutKind::Manual,
            duration,
        }
    }

    fn para_without_speaker(end: f64) -> Paragraph {
        Paragraph {
            id: 1,
            speaker: None,
            sentences: vec![Sentence {
                id: "s1".into(),
                text: "hi".into(),
                words: vec![Word {
                    id: "w0".into(),
                    text: "hi".into(),
                    start: 0.0,
                    end,
                }],
            }],
        }
    }

    #[test]
    fn fix_is_advisory_and_never_mutates_cuts() {
        let mut doc = empty_doc(); // 5s media
        doc.paragraphs.push(para_without_speaker(3.0));
        let mut cuts = ClipCuts::new();
        cuts.add(cut("c1", 3.0)); // 60% of 5s
        let items = finish_check_emit(&doc, &cuts);
        let advice = finish_check_fix("p", &items, &cuts, &doc);
        assert!(!advice.suggestions.is_empty());
        assert!(advice.fixed >= 1);
        // advisory only: cuts.json data is untouched
        assert_eq!(cuts.total_duration(), 3.0);
        assert_eq!(cuts.cuts.len(), 1);
    }

    #[test]
    fn soft_cuts_fix_suggests_restores_until_forty_percent() {
        let mut doc = empty_doc(); // 5s media
        doc.paragraphs.push(Paragraph {
            id: 1,
            speaker: None,
            sentences: vec![Sentence {
                id: "s1".into(),
                text: "one two".into(),
                words: vec![
                    Word {
                        id: "w0".into(),
                        text: "one".into(),
                        start: 0.0,
                        end: 1.5,
                    },
                    Word {
                        id: "w1".into(),
                        text: "two".into(),
                        start: 1.5,
                        end: 3.0,
                    },
                ],
            }],
        });
        let mut cuts = ClipCuts::new();
        cuts.add(cut("c1", 1.5));
        cuts.add(crate::data::soft_cut::Cut {
            id: "c2".into(),
            note: None,
            a_word: "w1".into(),
            b_word: "w1".into(),
            kind: crate::data::soft_cut::CutKind::Manual,
            duration: 1.5,
        }); // 3.0s total = 60% of 5s
        let items = finish_check_emit(&doc, &cuts);
        let advice = finish_check_fix("p", &items, &cuts, &doc);
        // Latest first: restoring c2 alone drops the total to 1.5s = 30%.
        assert!(advice
            .suggestions
            .contains(&"lumen-cut cut p --restore c2".to_string()));
        assert!(!advice.suggestions.iter().any(|s| s.contains("c1")));
    }

    #[test]
    fn soft_cuts_fix_falls_back_to_word_ends_when_duration_missing() {
        let mut doc = empty_doc();
        doc.media.duration_seconds = 0.0; // probing failed
        doc.paragraphs.push(Paragraph {
            id: 1,
            speaker: None,
            sentences: vec![Sentence {
                id: "s1".into(),
                text: "one two".into(),
                words: vec![
                    Word {
                        id: "w0".into(),
                        text: "one".into(),
                        start: 0.0,
                        end: 1.0,
                    },
                    Word {
                        id: "w1".into(),
                        text: "two".into(),
                        start: 4.5,
                        end: 5.0,
                    },
                ],
            }],
        }); // last word ends at 5s
        let mut cuts = ClipCuts::new();
        cuts.add(cut("c1", 1.0)); // 20% of the effective 5s
        let items = finish_check_emit(&doc, &cuts);
        let advice = finish_check_fix("p", &items, &cuts, &doc);
        // 1.0/5.0 = 20% ≤ 40% → no restore advice for c1.
        assert!(!advice.suggestions.iter().any(|s| s.contains("--restore")));
    }

    #[test]
    fn warn_findings_do_not_block_speaker_labels() {
        let mut doc = empty_doc();
        doc.paragraphs.push(para_without_speaker(0.5));
        let r = finish_check_emit(&doc, &ClipCuts::new());
        assert_eq!(r[5].code, FinishCheck::SpeakerLabels);
        assert!(r[5].pass, "WARN findings must not gate readiness");
        assert!(r[5].blockers.is_empty());
        assert_eq!(r[5].warnings.len(), 1);
        assert_eq!(r[5].warnings[0].severity, Severity::Warn);
    }

    #[test]
    fn uncommitted_head_blocks_final_readiness() {
        let mut doc = empty_doc();
        doc.paragraphs.push(para_without_speaker(0.5));
        let report = finish_check_emit_with_head(&doc, &ClipCuts::new(), false);
        assert!(!report[6].pass);
        assert!(!report[7].pass);
        assert_eq!(report[7].code, FinishCheck::VersionHeadCommitted);
    }
}
