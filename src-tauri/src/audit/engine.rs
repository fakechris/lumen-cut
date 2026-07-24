//! Project delivery audit engine.
//!
//! The stable code table contains 56 kebab-case findings grouped into eight
//! sections: `broll`, `cleanup`, `cut`, `polish`, `source`,
//! `structural`, `target`, `translation`. Pure document/timeline detectors
//! run in [`audit`] and project-artifact detectors run in [`audit_project`].
//! Every code has a concrete feeding path: align/polish/analysis artifacts
//! cover provenance and terminology, while preserved forward-compatible
//! `breaks`/`paraBreaks`/`clips` cover the remaining structural codes.
//!
//! Severity is two-valued (`Warn`/`Fail`); passing means no finding rather
//! than a third severity tier.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::data::doc::Doc;
use crate::data::soft_cut::{ClipCuts, CutKind};

/// Two-valued audit severity. Passing is the absence of a finding.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Warn,
    Fail,
}

/// The eight audit sections displayed by the review workflow.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Section {
    Broll,
    Cleanup,
    Cut,
    Polish,
    Source,
    Structural,
    Target,
    Translation,
}

/// Stable audit codes. Variant order groups by section, and `label()` is an
/// explicit match so rendering never depends on rename heuristics.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Code {
    // broll — B-roll geometry / asset integrity
    BrollEdgeIntrusion,
    BrollInsideCut,
    BrollMissingAsset,
    BrollRectOutOfBounds,
    BrollFlash,
    BrollOverlap,
    // cleanup — deterministic cleanup residue
    CleanupFillerZeroDuration,
    CleanupFixedFillerResidual,
    // cut — soft-cut partition integrity
    CutHeavyRemoval,
    CutMidWordBoundary,
    CutPartitionBroken,
    CutProvenanceUnavailable,
    // polish — polish residue
    PolishIntroducedZeroDuration,
    PolishResidualTerm,
    // source — source fidelity / drift / width
    SourceContentDrift,
    SourceContentDuplication,
    SourceContentLoss,
    SourceFidelityIndeterminate,
    SourceFidelityUnavailable,
    SourceFlash,
    SourceWidth,
    // structural — word time / paragraph pin / provenance
    InvalidWordTime,
    OrphanParagraphPin,
    OrphanTranslationPins,
    OrphanBreak,
    ParagraphCrossesTranslation,
    ParagraphPinNotSplit,
    PipelineFallback,
    WordTimeBoundary,
    ZeroDurationWords,
    // target — target subtitle fit / term budget
    TargetAtomicTermOverBudget,
    TargetFlashCompleteSentence,
    TargetFlash,
    TargetMergeableFragments,
    TargetSplittableOverAim,
    TargetTermAuditUnavailable,
    TargetTermSplit,
    TargetWidthAim,
    TargetWidth,
    // translation — translation integrity / seam
    TranslationAdjacentDuplicate,
    TranslationBoundaryAuditUnavailable,
    TranslationBoundaryDrift,
    TranslationEmpty,
    TranslationExtra,
    TranslationFlashMergeCrossSentence,
    TranslationLockedSeamMoved,
    TranslationMissing,
    TranslationPinNotCueEnd,
    TranslationSeamContentWord,
    TranslationSeamLargeRebind,
    TranslationSeamProvenanceUnavailable,
    TranslationSentenceEndMismatch,
    TranslationStale,
    TranslationStampExtra,
    TranslationStampMissing,
    TranslationTermSourceCoverage,
}

impl Code {
    pub fn label(self) -> &'static str {
        match self {
            Code::BrollEdgeIntrusion => "broll-edge-intrusion",
            Code::BrollInsideCut => "broll-inside-cut",
            Code::BrollMissingAsset => "broll-missing-asset",
            Code::BrollRectOutOfBounds => "broll-rect-out-of-bounds",
            Code::BrollFlash => "broll-flash",
            Code::BrollOverlap => "broll-overlap",
            Code::CleanupFillerZeroDuration => "cleanup-filler-zero-duration",
            Code::CleanupFixedFillerResidual => "cleanup-fixed-filler-residual",
            Code::CutHeavyRemoval => "cut-heavy-removal",
            Code::CutMidWordBoundary => "cut-mid-word-boundary",
            Code::CutPartitionBroken => "cut-partition-broken",
            Code::CutProvenanceUnavailable => "cut-provenance-unavailable",
            Code::PolishIntroducedZeroDuration => "polish-introduced-zero-duration",
            Code::PolishResidualTerm => "polish-residual-term",
            Code::SourceContentDrift => "source-content-drift",
            Code::SourceContentDuplication => "source-content-duplication",
            Code::SourceContentLoss => "source-content-loss",
            Code::SourceFidelityIndeterminate => "source-fidelity-indeterminate",
            Code::SourceFidelityUnavailable => "source-fidelity-unavailable",
            Code::SourceFlash => "source-flash",
            Code::SourceWidth => "source-width",
            Code::InvalidWordTime => "invalid-word-time",
            Code::OrphanParagraphPin => "orphan-paragraph-pin",
            Code::OrphanTranslationPins => "orphan-translation-pins",
            Code::OrphanBreak => "orphan-break",
            Code::ParagraphCrossesTranslation => "paragraph-crosses-translation",
            Code::ParagraphPinNotSplit => "paragraph-pin-not-split",
            Code::PipelineFallback => "pipeline-fallback",
            Code::WordTimeBoundary => "word-time-boundary",
            Code::ZeroDurationWords => "zero-duration-words",
            Code::TargetAtomicTermOverBudget => "target-atomic-term-over-budget",
            Code::TargetFlashCompleteSentence => "target-flash-complete-sentence",
            Code::TargetFlash => "target-flash",
            Code::TargetMergeableFragments => "target-mergeable-fragments",
            Code::TargetSplittableOverAim => "target-splittable-over-aim",
            Code::TargetTermAuditUnavailable => "target-term-audit-unavailable",
            Code::TargetTermSplit => "target-term-split",
            Code::TargetWidthAim => "target-width-aim",
            Code::TargetWidth => "target-width",
            Code::TranslationAdjacentDuplicate => "translation-adjacent-duplicate",
            Code::TranslationBoundaryAuditUnavailable => "translation-boundary-audit-unavailable",
            Code::TranslationBoundaryDrift => "translation-boundary-drift",
            Code::TranslationEmpty => "translation-empty",
            Code::TranslationExtra => "translation-extra",
            Code::TranslationFlashMergeCrossSentence => "translation-flash-merge-cross-sentence",
            Code::TranslationLockedSeamMoved => "translation-locked-seam-moved",
            Code::TranslationMissing => "translation-missing",
            Code::TranslationPinNotCueEnd => "translation-pin-not-cue-end",
            Code::TranslationSeamContentWord => "translation-seam-content-word",
            Code::TranslationSeamLargeRebind => "translation-seam-large-rebind",
            Code::TranslationSeamProvenanceUnavailable => "translation-seam-provenance-unavailable",
            Code::TranslationSentenceEndMismatch => "translation-sentence-end-mismatch",
            Code::TranslationStale => "translation-stale",
            Code::TranslationStampExtra => "translation-stamp-extra",
            Code::TranslationStampMissing => "translation-stamp-missing",
            Code::TranslationTermSourceCoverage => "translation-term-source-coverage",
        }
    }

    /// Severity is based on delivery impact: corruption, missing data, and
    /// invalid boundaries fail; recoverable quality concerns warn.
    ///
    /// Caption **aim** / flash density are presentation quality and never
    /// hard-block export. Only **hard capacity** (`TargetWidth` /
    /// `SourceWidth`) remains a Fail for width.
    pub fn severity(self) -> Severity {
        match self {
            // fail — data corruption / missing / out-of-bounds (export gates)
            Code::TranslationEmpty
            | Code::TranslationMissing
            | Code::InvalidWordTime
            | Code::WordTimeBoundary
            | Code::CutHeavyRemoval
            | Code::CutMidWordBoundary
            | Code::CutPartitionBroken
            | Code::CutProvenanceUnavailable
            | Code::SourceContentLoss
            | Code::SourceFidelityUnavailable
            | Code::SourceFidelityIndeterminate
            | Code::BrollRectOutOfBounds
            | Code::BrollMissingAsset
            | Code::BrollInsideCut
            | Code::BrollEdgeIntrusion
            | Code::BrollOverlap
            | Code::SourceWidth
            | Code::TargetWidth
            | Code::TranslationBoundaryAuditUnavailable
            | Code::TranslationSeamProvenanceUnavailable
            | Code::ParagraphCrossesTranslation
            | Code::ParagraphPinNotSplit
            | Code::TranslationPinNotCueEnd
            | Code::PipelineFallback => Severity::Fail,

            // warn — quality regressions, repairable (including soft fit / flash)
            Code::TranslationStampMissing
            | Code::TranslationStampExtra
            | Code::TranslationExtra
            | Code::TranslationStale
            | Code::SourceFlash
            | Code::TranslationAdjacentDuplicate
            | Code::TranslationBoundaryDrift
            | Code::TranslationSentenceEndMismatch
            | Code::TranslationFlashMergeCrossSentence
            | Code::TranslationLockedSeamMoved
            | Code::TranslationSeamContentWord
            | Code::TranslationSeamLargeRebind
            | Code::TranslationTermSourceCoverage
            | Code::TargetTermAuditUnavailable
            | Code::SourceContentDrift
            | Code::SourceContentDuplication
            | Code::TargetAtomicTermOverBudget
            | Code::TargetMergeableFragments
            | Code::TargetSplittableOverAim
            | Code::TargetWidthAim
            | Code::TargetFlash
            | Code::TargetFlashCompleteSentence
            | Code::TargetTermSplit
            | Code::CleanupFillerZeroDuration
            | Code::CleanupFixedFillerResidual
            | Code::PolishIntroducedZeroDuration
            | Code::PolishResidualTerm
            | Code::ZeroDurationWords
            | Code::OrphanBreak
            | Code::OrphanParagraphPin
            | Code::OrphanTranslationPins
            | Code::BrollFlash => Severity::Warn,
        }
    }

    pub fn section(self) -> Section {
        match self {
            Code::BrollEdgeIntrusion
            | Code::BrollInsideCut
            | Code::BrollMissingAsset
            | Code::BrollRectOutOfBounds
            | Code::BrollFlash
            | Code::BrollOverlap => Section::Broll,
            Code::CleanupFillerZeroDuration | Code::CleanupFixedFillerResidual => Section::Cleanup,
            Code::CutHeavyRemoval
            | Code::CutMidWordBoundary
            | Code::CutPartitionBroken
            | Code::CutProvenanceUnavailable => Section::Cut,
            Code::PolishIntroducedZeroDuration | Code::PolishResidualTerm => Section::Polish,
            Code::SourceContentDrift
            | Code::SourceContentDuplication
            | Code::SourceContentLoss
            | Code::SourceFidelityIndeterminate
            | Code::SourceFidelityUnavailable
            | Code::SourceFlash
            | Code::SourceWidth => Section::Source,
            Code::InvalidWordTime
            | Code::OrphanParagraphPin
            | Code::OrphanTranslationPins
            | Code::OrphanBreak
            | Code::ParagraphCrossesTranslation
            | Code::ParagraphPinNotSplit
            | Code::PipelineFallback
            | Code::WordTimeBoundary
            | Code::ZeroDurationWords => Section::Structural,
            Code::TargetAtomicTermOverBudget
            | Code::TargetFlashCompleteSentence
            | Code::TargetFlash
            | Code::TargetMergeableFragments
            | Code::TargetSplittableOverAim
            | Code::TargetTermAuditUnavailable
            | Code::TargetTermSplit
            | Code::TargetWidthAim
            | Code::TargetWidth => Section::Target,
            Code::TranslationAdjacentDuplicate
            | Code::TranslationBoundaryAuditUnavailable
            | Code::TranslationBoundaryDrift
            | Code::TranslationEmpty
            | Code::TranslationExtra
            | Code::TranslationFlashMergeCrossSentence
            | Code::TranslationLockedSeamMoved
            | Code::TranslationMissing
            | Code::TranslationPinNotCueEnd
            | Code::TranslationSeamContentWord
            | Code::TranslationSeamLargeRebind
            | Code::TranslationSeamProvenanceUnavailable
            | Code::TranslationSentenceEndMismatch
            | Code::TranslationStale
            | Code::TranslationStampExtra
            | Code::TranslationStampMissing
            | Code::TranslationTermSourceCoverage => Section::Translation,
        }
    }

    /// Every stable code in section order. Used by
    /// the `audit_codes` surface so the frontend can render the full table.
    pub fn all() -> &'static [Code] {
        &[
            // broll
            Code::BrollEdgeIntrusion,
            Code::BrollInsideCut,
            Code::BrollMissingAsset,
            Code::BrollRectOutOfBounds,
            Code::BrollFlash,
            Code::BrollOverlap,
            // cleanup
            Code::CleanupFillerZeroDuration,
            Code::CleanupFixedFillerResidual,
            // cut
            Code::CutHeavyRemoval,
            Code::CutMidWordBoundary,
            Code::CutPartitionBroken,
            Code::CutProvenanceUnavailable,
            // polish
            Code::PolishIntroducedZeroDuration,
            Code::PolishResidualTerm,
            // source
            Code::SourceContentDrift,
            Code::SourceContentDuplication,
            Code::SourceContentLoss,
            Code::SourceFidelityIndeterminate,
            Code::SourceFidelityUnavailable,
            Code::SourceFlash,
            Code::SourceWidth,
            // structural
            Code::InvalidWordTime,
            Code::OrphanParagraphPin,
            Code::OrphanTranslationPins,
            Code::OrphanBreak,
            Code::ParagraphCrossesTranslation,
            Code::ParagraphPinNotSplit,
            Code::PipelineFallback,
            Code::WordTimeBoundary,
            Code::ZeroDurationWords,
            // target
            Code::TargetAtomicTermOverBudget,
            Code::TargetFlashCompleteSentence,
            Code::TargetFlash,
            Code::TargetMergeableFragments,
            Code::TargetSplittableOverAim,
            Code::TargetTermAuditUnavailable,
            Code::TargetTermSplit,
            Code::TargetWidthAim,
            Code::TargetWidth,
            // translation
            Code::TranslationAdjacentDuplicate,
            Code::TranslationBoundaryAuditUnavailable,
            Code::TranslationBoundaryDrift,
            Code::TranslationEmpty,
            Code::TranslationExtra,
            Code::TranslationFlashMergeCrossSentence,
            Code::TranslationLockedSeamMoved,
            Code::TranslationMissing,
            Code::TranslationPinNotCueEnd,
            Code::TranslationSeamContentWord,
            Code::TranslationSeamLargeRebind,
            Code::TranslationSeamProvenanceUnavailable,
            Code::TranslationSentenceEndMismatch,
            Code::TranslationStale,
            Code::TranslationStampExtra,
            Code::TranslationStampMissing,
            Code::TranslationTermSourceCoverage,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Finding {
    pub code: Code,
    pub severity: Severity,
    pub where_: String, // paragraph/sentence/word id
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn has_failures(&self) -> bool {
        self.findings.iter().any(|f| f.severity == Severity::Fail)
    }

    pub fn has_warnings(&self) -> bool {
        self.findings.iter().any(|f| f.severity == Severity::Warn)
    }

    pub fn by_code(&self, code: Code) -> Vec<&Finding> {
        self.findings.iter().filter(|f| f.code == code).collect()
    }
}

/// Hard-delete filler list shared with `pipeline::cleanup`. A zero-duration
/// hit on one of these is
/// `cleanup-filler-zero-duration`; a zero-duration non-filler is
/// `zero-duration-words`.
const HARD_FILLERS: &[&str] = &[
    "um", "umm", "uh", "uhh", "er", "erm", "ah", "hmm", "mhm", "呃", "额",
];

fn normalize_word(text: &str) -> String {
    text.trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

fn normalize_content(text: &str) -> String {
    text.chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut expected = needle.chars();
    let mut next = expected.next();
    for character in haystack.chars() {
        if next == Some(character) {
            next = expected.next();
        }
    }
    next.is_none()
}

/// Bag-of-trigrams Jaccard similarity — same heuristic as the cleanup
/// retake detector, duplicated here so the audit module stays standalone.
fn trigram_jaccard(a: &str, b: &str) -> f64 {
    let tri = |s: &str| -> HashSet<(char, char, char)> {
        let chars: Vec<char> = s.chars().collect();
        chars.windows(3).map(|w| (w[0], w[1], w[2])).collect()
    };
    let (ta, tb) = (tri(a), tri(b));
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

/// Run the detectors over `doc`. Deterministic — same input → same output.
/// Only the codes whose detectors are implementable on the current `Doc`
/// shape are emitted; the rest of the namespace is defined but silent until
/// the feeding pipeline (two-phase translate, align projection, B-roll
/// geometry) lands.
pub fn audit(doc: &Doc) -> Report {
    let mut r = Report::default();

    let push = |r: &mut Report, code: Code, where_: String, message: String| {
        r.findings.push(Finding {
            code,
            severity: code.severity(),
            where_,
            message,
        });
    };

    // (translation-empty) — no recognisable speech at all.
    if doc.paragraphs.is_empty()
        || doc
            .paragraphs
            .iter()
            .all(|p| p.sentences.is_empty() || p.sentences.iter().all(|s| s.words.is_empty()))
    {
        push(
            &mut r,
            Code::TranslationEmpty,
            "<doc>".into(),
            "doc.json contains no recognisable speech".into(),
        );
    }

    // (translation-missing / translation-extra) — per translation language,
    // one finding per group that is missing, empty, or identical to the
    // source; plus one finding per translation id with no source sentence.
    let source_ids: HashSet<&str> = doc
        .paragraphs
        .iter()
        .flat_map(|p| p.sentences.iter())
        .map(|s| s.id.as_str())
        .collect();

    // Source subtitle presentation gates. Source speech can be read faster
    // than translated CJK, but still becomes an unusable flash above 17
    // visible characters per second.
    let source_lang = doc.meta.language.as_deref().unwrap_or("en");
    // Source subtitles are rendered as up to two lines and use a wider Latin
    // measure than translation prompt packets. Reusing the 17-character
    // translation packing cap here rejected nearly every normal English cue.
    let source_hard = match source_lang.to_ascii_lowercase().as_str() {
        "zh" | "ja" | "ko" | "chinese" | "japanese" | "korean" => 22,
        _ => 42,
    };
    for sentence in doc
        .paragraphs
        .iter()
        .flat_map(|paragraph| paragraph.sentences.iter())
    {
        let surface = normalize_content(&sentence.text);
        let timed = sentence
            .words
            .iter()
            .map(|word| normalize_content(&word.text))
            .collect::<String>();
        match (surface.is_empty(), timed.is_empty()) {
            (false, true) => push(
                &mut r,
                Code::SourceFidelityUnavailable,
                sentence.id.clone(),
                "surface text has no timed-word fidelity source".into(),
            ),
            (true, false) => push(
                &mut r,
                Code::SourceContentLoss,
                sentence.id.clone(),
                "surface text lost all timed-word content".into(),
            ),
            (true, true) if !sentence.text.trim().is_empty() => push(
                &mut r,
                Code::SourceFidelityIndeterminate,
                sentence.id.clone(),
                "source contains no comparable alphanumeric content".into(),
            ),
            (false, false) if surface != timed => {
                if is_subsequence(&surface, &timed) {
                    push(
                        &mut r,
                        Code::SourceContentLoss,
                        sentence.id.clone(),
                        "surface text omits timed-word content".into(),
                    );
                } else if is_subsequence(&timed, &surface) {
                    push(
                        &mut r,
                        Code::SourceContentDrift,
                        sentence.id.clone(),
                        "surface text adds content without word timing".into(),
                    );
                    push(
                        &mut r,
                        Code::PolishIntroducedZeroDuration,
                        sentence.id.clone(),
                        "polish introduced content with no timed-word interval".into(),
                    );
                } else {
                    push(
                        &mut r,
                        Code::SourceContentDrift,
                        sentence.id.clone(),
                        "surface text diverges from timed words".into(),
                    );
                    push(
                        &mut r,
                        Code::SourceContentLoss,
                        sentence.id.clone(),
                        "surface text no longer covers all timed words".into(),
                    );
                }
            }
            _ => {}
        }
        let chars = sentence
            .text
            .chars()
            .filter(|character| !character.is_whitespace())
            .count();
        if chars > source_hard {
            push(
                &mut r,
                Code::SourceWidth,
                sentence.id.clone(),
                format!("source width {chars} exceeds hard capacity {source_hard}"),
            );
        }
        let duration = sentence
            .words
            .first()
            .zip(sentence.words.last())
            .map(|(first, last)| (last.end - first.start).max(0.001))
            .unwrap_or(0.001);
        let cps = chars as f64 / duration;
        if cps > 17.0 {
            push(
                &mut r,
                Code::SourceFlash,
                sentence.id.clone(),
                format!("source reads at {cps:.1} chars/s (>17)"),
            );
        }
    }

    for (lang, groups) in &doc.translations {
        for para in &doc.paragraphs {
            for sent in &para.sentences {
                let reason = match groups.get(&sent.id) {
                    None => Some("missing"),
                    Some(g) if g.text.trim().is_empty() => Some("empty"),
                    Some(g) if g.text == sent.text => Some("equals source"),
                    Some(_) => None,
                };
                if let Some(reason) = reason {
                    push(
                        &mut r,
                        Code::TranslationMissing,
                        format!("{lang}/{}", sent.id),
                        format!("translation[{lang}] for {} {reason}", sent.id),
                    );
                }
            }
        }
        for id in groups.keys() {
            if !source_ids.contains(id.as_str()) {
                push(
                    &mut r,
                    Code::TranslationExtra,
                    format!("{lang}/{id}"),
                    format!("translation[{lang}] has id {id} with no source sentence"),
                );
            }
        }

        let word_owner: std::collections::BTreeMap<&str, (u32, &str, bool, f64, f64)> = doc
            .paragraphs
            .iter()
            .flat_map(|paragraph| {
                paragraph.sentences.iter().flat_map(move |sentence| {
                    let last = sentence.words.last().map(|word| word.id.as_str());
                    sentence.words.iter().map(move |word| {
                        (
                            word.id.as_str(),
                            (
                                paragraph.id,
                                sentence.id.as_str(),
                                last == Some(word.id.as_str()),
                                word.start,
                                word.end,
                            ),
                        )
                    })
                })
            })
            .collect();
        let aim = crate::pipeline::translate::aim_chars_for_lang(lang);
        let hard = crate::pipeline::translate::hard_chars_for_lang(lang);
        for (id, group) in groups {
            if let Some(source) = doc
                .paragraphs
                .iter()
                .flat_map(|paragraph| paragraph.sentences.iter())
                .find(|sentence| sentence.id == *id)
            {
                let expected: Vec<&str> =
                    source.words.iter().map(|word| word.id.as_str()).collect();
                let stamped: Vec<&str> = group.source_words.iter().map(String::as_str).collect();
                let missing: Vec<&str> = expected
                    .iter()
                    .copied()
                    .filter(|word| !stamped.contains(word))
                    .collect();
                let extra: Vec<&str> = stamped
                    .iter()
                    .copied()
                    .filter(|word| !expected.contains(word))
                    .collect();
                if !missing.is_empty() {
                    push(
                        &mut r,
                        Code::TranslationStampMissing,
                        format!("{lang}/{id}"),
                        format!("translation stamp omits source words {missing:?}"),
                    );
                }
                if !extra.is_empty() {
                    push(
                        &mut r,
                        Code::TranslationStampExtra,
                        format!("{lang}/{id}"),
                        format!("translation stamp includes extra words {extra:?}"),
                    );
                }
                if stamped != expected {
                    push(
                        &mut r,
                        Code::TranslationBoundaryDrift,
                        format!("{lang}/{id}"),
                        "translation source-word boundary differs from its source cue".into(),
                    );
                }
            }
            if group.source_words.is_empty() {
                push(
                    &mut r,
                    Code::TranslationBoundaryAuditUnavailable,
                    format!("{lang}/{id}"),
                    "translation has no source-word boundary provenance".into(),
                );
                continue;
            }
            let owners: Vec<_> = group
                .source_words
                .iter()
                .filter_map(|word_id| word_owner.get(word_id.as_str()))
                .collect();
            if owners.len() != group.source_words.len() {
                push(
                    &mut r,
                    Code::OrphanTranslationPins,
                    format!("{lang}/{id}"),
                    "translation references source words that do not exist".into(),
                );
                continue;
            }
            let paragraphs: HashSet<u32> = owners.iter().map(|owner| owner.0).collect();
            if paragraphs.len() > 1 {
                push(
                    &mut r,
                    Code::ParagraphCrossesTranslation,
                    format!("{lang}/{id}"),
                    "translation source range crosses a paragraph boundary".into(),
                );
            }
            let sentences: HashSet<&str> = owners.iter().map(|owner| owner.1).collect();
            if sentences.len() > 1 {
                push(
                    &mut r,
                    Code::ParagraphPinNotSplit,
                    format!("{lang}/{id}"),
                    "translation source range spans multiple source sentences".into(),
                );
            }
            if owners.last().is_some_and(|owner| !owner.2) {
                push(
                    &mut r,
                    Code::TranslationPinNotCueEnd,
                    format!("{lang}/{id}"),
                    "translation source range does not end at a cue boundary".into(),
                );
            }
            let chars = group
                .text
                .chars()
                .filter(|character| !character.is_whitespace())
                .count();
            if chars > hard {
                push(
                    &mut r,
                    Code::TargetWidth,
                    format!("{lang}/{id}"),
                    format!("target width {chars} exceeds hard capacity {hard}"),
                );
            } else if chars > aim {
                push(
                    &mut r,
                    Code::TargetWidthAim,
                    format!("{lang}/{id}"),
                    format!("target width {chars} exceeds aim {aim}"),
                );
                if group
                    .text
                    .trim_matches(|character: char| {
                        matches!(
                            character,
                            '，' | '。' | '、' | '；' | '：' | ',' | '.' | ';' | ':'
                        )
                    })
                    .chars()
                    .any(|character| {
                        matches!(
                            character,
                            '，' | '。' | '、' | '；' | '：' | ',' | '.' | ';' | ':'
                        )
                    })
                {
                    push(
                        &mut r,
                        Code::TargetSplittableOverAim,
                        format!("{lang}/{id}"),
                        "target exceeds aim and has a safe punctuation seam".into(),
                    );
                }
            }
            let start = owners.first().map(|owner| owner.3).unwrap_or_default();
            let end = owners.last().map(|owner| owner.4).unwrap_or(start);
            let cps = chars as f64 / (end - start).max(0.001);
            if cps > 9.0 {
                push(
                    &mut r,
                    Code::TargetFlash,
                    format!("{lang}/{id}"),
                    format!("target reads at {cps:.1} chars/s (>9)"),
                );
                if ends_sentence(&group.text) {
                    push(
                        &mut r,
                        Code::TargetFlashCompleteSentence,
                        format!("{lang}/{id}"),
                        "a complete target sentence flashes too quickly".into(),
                    );
                }
            }
            if let Some(source) = doc
                .paragraphs
                .iter()
                .flat_map(|paragraph| paragraph.sentences.iter())
                .find(|sentence| sentence.id == *id)
            {
                if group
                    .source_text
                    .as_deref()
                    .is_some_and(|translated_from| translated_from != source.text)
                {
                    push(
                        &mut r,
                        Code::TranslationStale,
                        format!("{lang}/{id}"),
                        "source text changed after this translation was created".into(),
                    );
                }
                if ends_sentence(&source.text) != ends_sentence(&group.text) {
                    push(
                        &mut r,
                        Code::TranslationSentenceEndMismatch,
                        format!("{lang}/{id}"),
                        "source and target sentence-ending punctuation disagree".into(),
                    );
                }
            }
        }
    }

    // (invalid-word-time / zero-duration-words / cleanup-filler-zero-duration)
    for w in doc.all_words() {
        let dur = w.end - w.start;
        if w.start < 0.0 || w.end < w.start {
            push(
                &mut r,
                Code::InvalidWordTime,
                w.id.clone(),
                format!(
                    "word {:?} has invalid interval [{:.3},{:.3}]",
                    w.text, w.start, w.end
                ),
            );
            continue;
        }
        if dur < 0.05 {
            let norm = normalize_word(&w.text);
            let code = if !norm.is_empty() && HARD_FILLERS.contains(&norm.as_str()) {
                Code::CleanupFillerZeroDuration
            } else {
                Code::ZeroDurationWords
            };
            push(
                &mut r,
                code,
                w.id.clone(),
                format!("word {:?} has ~zero duration ({:.3}s)", w.text, dur),
            );
        }
    }

    // (word-time-boundary) — two consecutive words/sentences whose
    // intervals overlap by more than the 0.05 s jitter tolerance.
    let mut last_end = -1.0_f64;
    let mut first = true;
    for para in &doc.paragraphs {
        for sent in &para.sentences {
            if let Some(w0) = sent.words.first() {
                if !first && w0.start < last_end - 0.05 {
                    push(
                        &mut r,
                        Code::WordTimeBoundary,
                        sent.id.clone(),
                        format!(
                            "start {:.3} overlaps previous end {:.3}",
                            w0.start, last_end
                        ),
                    );
                }
                first = false;
            }
            if let Some(w_last) = sent.words.last() {
                last_end = last_end.max(w_last.end);
            }
        }
    }

    // (source-content-duplication) — adjacent near-identical source
    // sentences (a retake left in the transcript).
    let sents: Vec<_> = doc
        .paragraphs
        .iter()
        .flat_map(|p| p.sentences.iter())
        .collect();
    for pair in sents.windows(2) {
        if trigram_jaccard(&pair[0].text, &pair[1].text) >= 0.85 {
            push(
                &mut r,
                Code::SourceContentDuplication,
                pair[1].id.clone(),
                format!("sentence {} duplicates {}", pair[1].id, pair[0].id),
            );
        }
    }

    // (translation-adjacent-duplicate) — adjacent identical translations.
    for (lang, groups) in &doc.translations {
        let ordered: Vec<(&String, &str)> = sents
            .iter()
            .filter_map(|s| groups.get(&s.id).map(|g| (&s.id, g.text.as_str())))
            .collect();
        for pair in ordered.windows(2) {
            if !pair[0].1.is_empty() && pair[0].1 == pair[1].1 {
                push(
                    &mut r,
                    Code::TranslationAdjacentDuplicate,
                    format!("{lang}/{}", pair[1].0),
                    format!("translation[{lang}] {} == {}", pair[1].0, pair[0].0),
                );
            }
        }
        let aim = crate::pipeline::translate::aim_chars_for_lang(lang);
        for pair in sents.windows(2) {
            let (Some(left), Some(right)) = (groups.get(&pair[0].id), groups.get(&pair[1].id))
            else {
                continue;
            };
            let left_chars = left
                .text
                .chars()
                .filter(|character| !character.is_whitespace())
                .count();
            let right_chars = right
                .text
                .chars()
                .filter(|character| !character.is_whitespace())
                .count();
            if !ends_sentence(&left.text)
                && !ends_sentence(&right.text)
                && left_chars + right_chars <= aim
            {
                push(
                    &mut r,
                    Code::TargetMergeableFragments,
                    format!("{lang}/{}", pair[1].id),
                    format!(
                        "adjacent target fragments {} and {} fit in one unit",
                        pair[0].id, pair[1].id
                    ),
                );
                let left_window = pair[0]
                    .words
                    .first()
                    .zip(pair[0].words.last())
                    .map(|(first, last)| (first.start, last.end));
                let right_window = pair[1]
                    .words
                    .first()
                    .zip(pair[1].words.last())
                    .map(|(first, last)| (first.start, last.end));
                if let (Some((left_start, left_end)), Some((_, right_end))) =
                    (left_window, right_window)
                {
                    let left_cps = left_chars as f64 / (left_end - left_start).max(0.001);
                    let merged_cps =
                        (left_chars + right_chars) as f64 / (right_end - left_start).max(0.001);
                    if left_cps > 9.0 && merged_cps <= 9.0 {
                        push(
                            &mut r,
                            Code::TranslationFlashMergeCrossSentence,
                            format!("{lang}/{}", pair[0].id),
                            "flashing target can be merged across the next sentence".into(),
                        );
                    }
                }
            }
        }
    }

    r
}

fn ends_sentence(text: &str) -> bool {
    text.trim_end()
        .chars()
        .last()
        .is_some_and(|character| matches!(character, '.' | '!' | '?' | '。' | '！' | '？'))
}

/// Run the document detectors plus the project-timeline detectors that need
/// `cuts.json`. Keeping this separate from [`audit`] preserves the pure
/// document API while ensuring CLI/GUI/MCP project audits do not silently
/// discard cut provenance.
pub fn audit_with_cuts(doc: &Doc, cuts: &ClipCuts) -> Report {
    let mut report = audit(doc);
    let words: std::collections::BTreeMap<&str, (f64, f64)> = doc
        .all_words()
        .into_iter()
        .map(|word| (word.id.as_str(), (word.start, word.end)))
        .collect();
    let mut raw_intervals: Vec<(&str, f64, f64)> = Vec::new();

    let ordered_words = doc.all_words();
    let word_index: std::collections::BTreeMap<&str, usize> = ordered_words
        .iter()
        .enumerate()
        .map(|(index, word)| (word.id.as_str(), index))
        .collect();
    let mut filler_covered = BTreeSet::new();
    let cleanup_started = cuts.cuts.iter().any(|cut| cut.kind == CutKind::Filler);
    if cleanup_started {
        for cut in cuts.cuts.iter().filter(|cut| cut.kind == CutKind::Filler) {
            if let (Some(&start), Some(&end)) = (
                word_index.get(cut.a_word.as_str()),
                word_index.get(cut.b_word.as_str()),
            ) {
                for index in start.min(end)..=start.max(end) {
                    filler_covered.insert(index);
                }
            }
        }
        for (index, word) in ordered_words.iter().enumerate() {
            let normalized = normalize_word(&word.text);
            if HARD_FILLERS.contains(&normalized.as_str()) && !filler_covered.contains(&index) {
                report.findings.push(Finding {
                    code: Code::CleanupFixedFillerResidual,
                    severity: Code::CleanupFixedFillerResidual.severity(),
                    where_: word.id.clone(),
                    message: format!("hard filler {:?} remains after cleanup", word.text),
                });
            }
        }
    }

    for cut in &cuts.cuts {
        if !words.contains_key(cut.a_word.as_str()) {
            report.findings.push(Finding {
                code: Code::CutProvenanceUnavailable,
                severity: Code::CutProvenanceUnavailable.severity(),
                where_: cut.id.clone(),
                message: format!("cut start word {} does not exist", cut.a_word),
            });
            continue;
        }
        if !words.contains_key(cut.b_word.as_str()) {
            report.findings.push(Finding {
                code: Code::CutProvenanceUnavailable,
                severity: Code::CutProvenanceUnavailable.severity(),
                where_: cut.id.clone(),
                message: format!("cut end word {} does not exist", cut.b_word),
            });
            continue;
        }
        let Some((start, end)) = cut.resolved_interval(doc) else {
            continue;
        };
        if end <= start {
            report.findings.push(Finding {
                code: Code::CutPartitionBroken,
                severity: Code::CutPartitionBroken.severity(),
                where_: cut.id.clone(),
                message: format!("cut has invalid timeline interval [{start:.3},{end:.3}]"),
            });
            continue;
        }
        raw_intervals.push((&cut.id, start, end));
    }

    raw_intervals.sort_by(|a, b| {
        a.1.partial_cmp(&b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
    });
    for pair in raw_intervals.windows(2) {
        if pair[1].1 < pair[0].2 {
            report.findings.push(Finding {
                code: Code::CutPartitionBroken,
                severity: Code::CutPartitionBroken.severity(),
                where_: pair[1].0.to_string(),
                message: format!("cut overlaps {}", pair[0].0),
            });
        }
    }

    // Heavy-removal uses the union of resolvable timeline intervals. This
    // avoids double-counting overlapping cuts and does not trust the cached
    // `Cut.duration` field.
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (_, start, end) in raw_intervals {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    let removed: f64 = merged.iter().map(|(start, end)| end - start).sum();
    let ratio = removed / doc.media.duration_seconds.max(0.001);
    if ratio > 0.40 {
        report.findings.push(Finding {
            code: Code::CutHeavyRemoval,
            severity: Code::CutHeavyRemoval.severity(),
            where_: "<cuts>".into(),
            message: format!("{ratio:.0}% of media is cut (>40%)"),
        });
    }

    report
}

/// Full project audit, including accepted B-roll placements and their assets.
pub fn audit_with_project(
    doc: &Doc,
    cuts: &ClipCuts,
    placements: &[crate::data::broll::BrollPlacement],
) -> Report {
    let mut report = audit_with_cuts(doc, cuts);
    let mut ordered: Vec<_> = placements.iter().collect();
    ordered.sort_by(|left, right| {
        left.start
            .partial_cmp(&right.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let cut_intervals = crate::export::cut_intervals(doc, &cuts.cuts);

    for placement in &ordered {
        let push = |report: &mut Report, code: Code, message: String| {
            report.findings.push(Finding {
                code,
                severity: code.severity(),
                where_: placement.id.clone(),
                message,
            });
        };
        if !placement.file.is_file() {
            push(
                &mut report,
                Code::BrollMissingAsset,
                format!("B-roll asset does not exist: {}", placement.file.display()),
            );
        }
        if let Some(rect) = placement.rect {
            if rect.x.saturating_add(rect.width) > 1920 || rect.y.saturating_add(rect.height) > 1080
            {
                push(
                    &mut report,
                    Code::BrollRectOutOfBounds,
                    format!(
                        "rect {},{},{},{} exceeds 1920x1080",
                        rect.x, rect.y, rect.width, rect.height
                    ),
                );
            }
        }
        if placement.start < 3.0
            || (doc.media.duration_seconds > 0.0
                && placement.end > doc.media.duration_seconds - 3.0)
        {
            push(
                &mut report,
                Code::BrollEdgeIntrusion,
                "B-roll placement enters the first or last 3 seconds".into(),
            );
        }
        if placement.end - placement.start < 1.5 {
            push(
                &mut report,
                Code::BrollFlash,
                "B-roll placement is shorter than 1.5 seconds".into(),
            );
        }
        if cut_intervals
            .iter()
            .any(|(start, end)| placement.start < *end && *start < placement.end)
        {
            push(
                &mut report,
                Code::BrollInsideCut,
                "B-roll placement intersects a removed timeline span".into(),
            );
        }
    }

    for pair in ordered.windows(2) {
        if pair[1].start < pair[0].end {
            report.findings.push(Finding {
                code: Code::BrollOverlap,
                severity: Code::BrollOverlap.severity(),
                where_: pair[1].id.clone(),
                message: format!("B-roll placement overlaps {}", pair[0].id),
            });
        }
    }
    report
}

/// Run the full audit with project-local pipeline evidence.
///
/// `audit_with_project` stays pure for callers that only have in-memory
/// state. Project entry points should use this function so a completed align
/// run cannot silently lose its word/seam provenance.
pub fn audit_project(
    doc: &Doc,
    cuts: &ClipCuts,
    placements: &[crate::data::broll::BrollPlacement],
    project_dir: &Path,
) -> Report {
    let mut report = audit_with_project(doc, cuts, placements);
    audit_align_artifact(doc, project_dir, &mut report);
    audit_polish_artifact(doc, project_dir, &mut report);
    audit_term_artifacts(doc, project_dir, &mut report);
    audit_native_structure(doc, project_dir, &mut report);
    report
}

fn project_finding(report: &mut Report, code: Code, where_: String, message: String) {
    if report
        .findings
        .iter()
        .any(|finding| finding.code == code && finding.where_ == where_)
    {
        return;
    }
    report.findings.push(Finding {
        code,
        severity: code.severity(),
        where_,
        message,
    });
}

fn align_done_exists(project_dir: &Path) -> bool {
    std::fs::read_dir(project_dir.join("ai/align/done"))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .any(|entry| entry.path().is_file())
        })
        .unwrap_or(false)
}

fn stage_done_exists(project_dir: &Path, stage: &str) -> bool {
    std::fs::read_dir(project_dir.join("ai").join(stage).join("done"))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .any(|entry| entry.path().is_file())
        })
        .unwrap_or(false)
}

fn audit_polish_artifact(doc: &Doc, project_dir: &Path, report: &mut Report) {
    let path = project_dir.join("ai/polish-quality.json");
    if !path.is_file() {
        if stage_done_exists(project_dir, "polish") {
            project_finding(
                report,
                Code::PipelineFallback,
                "<polish>".into(),
                "completed polish output has no polish-quality.json".into(),
            );
        }
        return;
    }
    let artifact = match crate::pipeline::polish::PolishQualityArtifact::load(&path) {
        Ok(artifact) => artifact,
        Err(_) => {
            project_finding(
                report,
                Code::PipelineFallback,
                "<polish>".into(),
                "polish-quality.json does not match PolishQualityArtifact".into(),
            );
            return;
        }
    };
    if artifact.fingerprint != crate::pipeline::fingerprint_words(doc) {
        project_finding(
            report,
            Code::PipelineFallback,
            "<polish>".into(),
            "polish-quality.json fingerprint is stale".into(),
        );
        return;
    }
    if artifact.recovered_page_count > 0
        || artifact.fallback_page_count > 0
        || artifact.fallback_sentence_count > 0
    {
        project_finding(
            report,
            Code::PipelineFallback,
            "<polish>".into(),
            format!(
                "polish used recovery/fallback (pages {}, fallback pages {}, sentences {})",
                artifact.recovered_page_count,
                artifact.fallback_page_count,
                artifact.fallback_sentence_count
            ),
        );
    }
    if artifact.zero_duration_word_count_after > artifact.zero_duration_word_count_before {
        project_finding(
            report,
            Code::PolishIntroducedZeroDuration,
            "<polish>".into(),
            format!(
                "polish increased zero-duration words from {} to {}",
                artifact.zero_duration_word_count_before, artifact.zero_duration_word_count_after
            ),
        );
    }
    for residual in artifact.residual_term_variants {
        project_finding(
            report,
            Code::PolishResidualTerm,
            format!("<term:{}>", residual.canonical),
            format!(
                "analysis-confirmed variant {:?} remains {} time(s)",
                residual.variant, residual.occurrences
            ),
        );
    }
}

#[derive(Debug)]
struct AuditTerm {
    canonical: String,
    variants: Vec<String>,
    locked: bool,
}

fn audit_term_artifacts(doc: &Doc, project_dir: &Path, report: &mut Report) {
    if doc.translations.is_empty() {
        return;
    }
    let path = project_dir.join("ai/analysis.json");
    let analysis: serde_json::Value = match std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
    {
        Some(value) => value,
        None => {
            for lang in doc.translations.keys() {
                project_finding(
                    report,
                    Code::TargetTermAuditUnavailable,
                    format!("{lang}/<terms>"),
                    "translation has no decodable ai/analysis.json term evidence".into(),
                );
            }
            return;
        }
    };
    let Some(raw_terms) = analysis.get("terms").and_then(serde_json::Value::as_array) else {
        for lang in doc.translations.keys() {
            project_finding(
                report,
                Code::TargetTermAuditUnavailable,
                format!("{lang}/<terms>"),
                "analysis artifact has no terms array".into(),
            );
        }
        return;
    };
    let terms: Vec<AuditTerm> = raw_terms
        .iter()
        .filter_map(|term| {
            Some(AuditTerm {
                canonical: term.get("term")?.as_str()?.to_string(),
                variants: term
                    .get("observedVariants")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|variant| variant.as_str().map(str::to_string))
                    .collect(),
                locked: term.get("locked").and_then(serde_json::Value::as_bool) == Some(true),
            })
        })
        .collect();

    let sentences: Vec<_> = doc
        .paragraphs
        .iter()
        .flat_map(|paragraph| paragraph.sentences.iter())
        .collect();
    for (lang, groups) in &doc.translations {
        let hard = crate::pipeline::translate::hard_chars_for_lang(lang);
        for term in terms.iter().filter(|term| term.locked) {
            let cells = term
                .canonical
                .chars()
                .filter(|character| !character.is_whitespace())
                .count();
            if cells > hard {
                project_finding(
                    report,
                    Code::TargetAtomicTermOverBudget,
                    format!("{lang}/<term:{}>", term.canonical),
                    format!("locked atomic term width {cells} exceeds target hard capacity {hard}"),
                );
            }

            for pair in sentences.windows(2) {
                let (Some(left), Some(right)) = (groups.get(&pair[0].id), groups.get(&pair[1].id))
                else {
                    continue;
                };
                let joined = format!("{}{}", left.text, right.text);
                if contains_folded(&joined, &term.canonical)
                    && !contains_folded(&left.text, &term.canonical)
                    && !contains_folded(&right.text, &term.canonical)
                {
                    project_finding(
                        report,
                        Code::TargetTermSplit,
                        format!("{lang}/{}/{}", pair[0].id, pair[1].id),
                        format!(
                            "locked target term {:?} is split across adjacent groups",
                            term.canonical
                        ),
                    );
                }
            }

            for sentence in &sentences {
                let source_mentions_term = std::iter::once(term.canonical.as_str())
                    .chain(term.variants.iter().map(String::as_str))
                    .any(|candidate| contains_folded(&sentence.text, candidate));
                if !source_mentions_term {
                    continue;
                }
                let own_has_term = groups
                    .get(&sentence.id)
                    .is_some_and(|group| contains_folded(&group.text, &term.canonical));
                if own_has_term {
                    continue;
                }
                if let Some((other_id, _)) = groups.iter().find(|(id, group)| {
                    *id != &sentence.id && contains_folded(&group.text, &term.canonical)
                }) {
                    project_finding(
                        report,
                        Code::TranslationTermSourceCoverage,
                        format!("{lang}/{}", sentence.id),
                        format!(
                            "locked term {:?} belongs to this source cue but appears in target group {}",
                            term.canonical, other_id
                        ),
                    );
                }
            }
        }
    }
}

fn contains_folded(text: &str, needle: &str) -> bool {
    text.to_lowercase().contains(&needle.to_lowercase())
}

fn audit_native_structure(doc: &Doc, project_dir: &Path, report: &mut Report) {
    let value: serde_json::Value = match std::fs::read_to_string(project_dir.join("doc.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
    {
        Some(value) => value,
        None => return,
    };
    let known_words: HashSet<&str> = doc
        .all_words()
        .into_iter()
        .map(|word| word.id.as_str())
        .collect();
    for (field, code, label) in [
        ("breaks", Code::OrphanBreak, "break"),
        (
            "paraBreaks",
            Code::OrphanParagraphPin,
            "paragraph break pin",
        ),
    ] {
        for id in value[field]
            .as_object()
            .into_iter()
            .flat_map(serde_json::Map::keys)
        {
            if !known_words.contains(id.as_str()) {
                project_finding(
                    report,
                    code,
                    id.clone(),
                    format!("native {label} references an unknown word id"),
                );
            }
        }
    }

    // Compatible clip rows use `{id,start,end,src,cut?}`. A cut boundary
    // strictly inside a timed word is a mid-word cut.
    for clip in value["clips"].as_array().into_iter().flatten() {
        if clip["cut"].as_bool() != Some(true) {
            continue;
        }
        let id = clip["id"].as_str().unwrap_or("<clip>");
        for (edge, time) in [
            ("start", clip["start"].as_f64()),
            ("end", clip["end"].as_f64()),
        ] {
            let Some(time) = time else {
                continue;
            };
            if let Some(word) = doc
                .all_words()
                .into_iter()
                .find(|word| time > word.start + 0.001 && time < word.end - 0.001)
            {
                project_finding(
                    report,
                    Code::CutMidWordBoundary,
                    id.to_string(),
                    format!(
                        "native cut {edge} {time:.3}s falls inside word {} [{:.3},{:.3}]",
                        word.id, word.start, word.end
                    ),
                );
            }
        }
    }
}

fn audit_align_artifact(doc: &Doc, project_dir: &Path, report: &mut Report) {
    let path = project_dir.join("ai/align-artifact.json");
    if !path.is_file() {
        if align_done_exists(project_dir) {
            project_finding(
                report,
                Code::TranslationSeamProvenanceUnavailable,
                "<align>".into(),
                "completed align output has no align-artifact.json".into(),
            );
        }
        return;
    }

    let value: serde_json::Value = match std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
    {
        Some(value) => value,
        None => {
            project_finding(
                report,
                Code::TranslationSeamProvenanceUnavailable,
                "<align>".into(),
                "align-artifact.json is not decodable JSON".into(),
            );
            return;
        }
    };

    // These keys belonged to an obsolete wrapper. Current artifacts serialize
    // the `TranslateRebindArtifact` fields directly.
    if value.get("projection").is_some() || value.get("schemaVersion").is_some() {
        project_finding(
            report,
            Code::PipelineFallback,
            "<align>".into(),
            "align artifact uses the legacy synthetic projection wrapper".into(),
        );
    }

    let artifact: crate::pipeline::TranslateRebindArtifact = match serde_json::from_value(value) {
        Ok(artifact) => artifact,
        Err(_) => {
            project_finding(
                report,
                Code::TranslationSeamProvenanceUnavailable,
                "<align>".into(),
                "align artifact does not match TranslateRebindArtifact".into(),
            );
            return;
        }
    };

    let lang = artifact.lang.as_str();
    let Some(groups) = doc.translations.get(lang) else {
        project_finding(
            report,
            Code::TranslationSeamProvenanceUnavailable,
            format!("{lang}/<align>"),
            "align artifact language is not present in the document".into(),
        );
        return;
    };
    if artifact.fingerprint != crate::pipeline::fingerprint_words(doc) {
        project_finding(
            report,
            Code::TranslationSeamProvenanceUnavailable,
            format!("{lang}/<align>"),
            "align artifact word fingerprint is stale".into(),
        );
    }

    let mut seen = BTreeSet::new();
    for seam in &artifact.seams {
        let location = format!("{lang}/{}", seam.group_key);
        if !seen.insert(seam.group_key.as_str()) {
            project_finding(
                report,
                Code::TranslationSeamProvenanceUnavailable,
                location,
                "align artifact contains duplicate group seam".into(),
            );
            continue;
        }
        let Some(group) = groups.get(&seam.group_key) else {
            project_finding(
                report,
                Code::TranslationSeamProvenanceUnavailable,
                location,
                "align artifact seam group no longer exists".into(),
            );
            continue;
        };
        for (label, id) in [
            ("alignedEndId", &seam.aligned_end_id),
            ("finalEndId", &seam.final_end_id),
        ] {
            if !group.source_words.contains(id) {
                project_finding(
                    report,
                    Code::TranslationSeamProvenanceUnavailable,
                    location.clone(),
                    format!("align seam {label} `{id}` is not in the source group"),
                );
            }
        }
        if seam.locked == Some(true) && seam.aligned_end_id != seam.final_end_id {
            project_finding(
                report,
                Code::TranslationLockedSeamMoved,
                location.clone(),
                "locked translation seam moved during rebind".into(),
            );
        }
        if seam.displacement_words {
            project_finding(
                report,
                Code::TranslationSeamLargeRebind,
                location.clone(),
                "translation seam reports word displacement".into(),
            );
        }
        if group.source_words.last() != Some(&seam.final_end_id) {
            project_finding(
                report,
                Code::TranslationSeamContentWord,
                location,
                "translation seam ends on a non-final content word".into(),
            );
        }
    }
    for group_key in groups.keys() {
        if !seen.contains(group_key.as_str()) {
            project_finding(
                report,
                Code::TranslationSeamProvenanceUnavailable,
                format!("{lang}/{group_key}"),
                "translation group has no persisted rebind seam".into(),
            );
        }
    }

    for merge in artifact.reading_merges.into_iter().flatten() {
        let location = format!("{lang}/{}", merge.group_key);
        if !groups.contains_key(&merge.group_key) {
            project_finding(
                report,
                Code::TranslationSeamProvenanceUnavailable,
                location,
                "reading merge references an unknown translation group".into(),
            );
        } else if merge.crosses_sentence {
            project_finding(
                report,
                Code::TranslationFlashMergeCrossSentence,
                location,
                "reading-speed merge crosses a sentence boundary".into(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::doc::*;
    use chrono::Utc;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn doc(words: Vec<Word>, language: Option<&str>, speaker: Option<&str>) -> Doc {
        Doc {
            id: "p".into(),
            schema: 1,
            media: MediaRef {
                path: PathBuf::from("/tmp/x.mp4"),
                duration_seconds: 3.0,
                sample_rate: Some(16_000),
                channels: Some(1),
            },
            meta: Meta {
                title: "t".into(),
                description: String::new(),
                language: language.map(str::to_string),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            paragraphs: vec![Paragraph {
                id: 1,
                speaker: speaker.map(str::to_string),
                sentences: vec![Sentence {
                    id: "s1".into(),
                    text: words
                        .iter()
                        .map(|w| w.text.clone())
                        .collect::<Vec<_>>()
                        .join(" "),
                    words,
                }],
            }],
            translations: Default::default(),
        }
    }

    #[test]
    fn code_namespace_is_complete_and_unique() {
        // 56 codes, every label is non-empty kebab, severity/section total
        // exhaustively match (compiler-enforced), and no duplicate labels.
        assert_eq!(Code::all().len(), 56);
        let mut labels: Vec<&str> = Code::all().iter().map(|c| c.label()).collect();
        labels.sort();
        let dedup = labels.clone();
        labels.dedup();
        assert_eq!(labels.len(), dedup.len(), "duplicate code labels");
        assert!(labels.iter().all(|l| l.contains('-') && !l.contains('_')));
    }

    #[test]
    fn empty_transcript_is_translation_empty_fail() {
        let d = doc(vec![], None, None);
        let r = audit(&d);
        let f = r.by_code(Code::TranslationEmpty);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Fail);
    }

    #[test]
    fn translation_missing_one_per_group() {
        let mut d = doc(
            vec![
                Word {
                    id: "w0".into(),
                    text: "hello".into(),
                    start: 0.0,
                    end: 0.4,
                },
                Word {
                    id: "w1".into(),
                    text: "world".into(),
                    start: 0.4,
                    end: 0.8,
                },
            ],
            None,
            None,
        );
        d.translations.insert(
            "en".into(),
            BTreeMap::from([(
                "s1".into(),
                TranslationGroup {
                    id: "s1".into(),
                    text: "hello world".into(),
                    source_words: vec![],
                    source_text: None,
                },
            )]),
        );
        let r = audit(&d);
        let f = r.by_code(Code::TranslationMissing);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].where_, "en/s1");
    }

    #[test]
    fn translation_extra_for_orphan_id() {
        let mut d = doc(
            vec![Word {
                id: "w0".into(),
                text: "hi".into(),
                start: 0.0,
                end: 0.5,
            }],
            None,
            None,
        );
        d.translations.insert(
            "en".into(),
            BTreeMap::from([(
                "ghost".into(),
                TranslationGroup {
                    id: "ghost".into(),
                    text: "boo".into(),
                    source_words: vec![],
                    source_text: None,
                },
            )]),
        );
        assert_eq!(audit(&d).by_code(Code::TranslationExtra).len(), 1);
    }

    #[test]
    fn zero_duration_filler_is_cleanup_code() {
        let d = doc(
            vec![
                Word {
                    id: "w0".into(),
                    text: "well".into(),
                    start: 0.0,
                    end: 0.3,
                },
                Word {
                    id: "w1".into(),
                    text: "Um,".into(),
                    start: 0.3,
                    end: 0.31,
                },
            ],
            None,
            None,
        );
        assert_eq!(audit(&d).by_code(Code::CleanupFillerZeroDuration).len(), 1);
        assert!(audit(&d).by_code(Code::ZeroDurationWords).is_empty());
    }

    #[test]
    fn invalid_word_time_flagged_fail() {
        let d = doc(
            vec![Word {
                id: "w0".into(),
                text: "x".into(),
                start: 1.0,
                end: 0.5,
            }],
            None,
            None,
        );
        assert_eq!(audit(&d).by_code(Code::InvalidWordTime).len(), 1);
    }

    #[test]
    fn overlapping_cues_are_word_time_boundary() {
        let mut d = doc(
            vec![Word {
                id: "w0".into(),
                text: "a".into(),
                start: 0.0,
                end: 1.0,
            }],
            None,
            None,
        );
        d.paragraphs[0].sentences.push(Sentence {
            id: "s2".into(),
            text: "b".into(),
            words: vec![Word {
                id: "w1".into(),
                text: "b".into(),
                start: 0.8,
                end: 1.2,
            }],
        });
        assert_eq!(audit(&d).by_code(Code::WordTimeBoundary).len(), 1);
    }

    #[test]
    fn adjacent_duplicate_source_flagged() {
        let mut d = doc(
            vec![Word {
                id: "w0".into(),
                text: "the quick brown fox".into(),
                start: 0.0,
                end: 1.0,
            }],
            None,
            None,
        );
        d.paragraphs[0].sentences.push(Sentence {
            id: "s2".into(),
            text: "the quick brown fox".into(),
            words: vec![Word {
                id: "w1".into(),
                text: "the".into(),
                start: 1.1,
                end: 1.4,
            }],
        });
        assert_eq!(audit(&d).by_code(Code::SourceContentDuplication).len(), 1);
    }

    #[test]
    fn fabricated_codes_are_gone() {
        // The pre-refactor detectors (chinese-punct-zh, pinyin-mismatch,
        // rate-spike, misaligned-cue) are not public audit codes. The engine
        // must never emit them for inputs that used to trip them.
        let d = doc(
            vec![Word {
                id: "w0".into(),
                text: "你好, 世界.".into(),
                start: 0.0,
                end: 0.5,
            }],
            Some("zh"),
            None,
        );
        let r = audit(&d);
        // only the legitimate detectors fire; none of the removed ones exist
        assert!(r
            .findings
            .iter()
            .all(|f| f.code != Code::ZeroDurationWords || f.severity == Severity::Warn));
    }

    #[test]
    fn cut_with_unknown_word_reports_unavailable_provenance() {
        let d = doc(
            vec![Word {
                id: "w0".into(),
                text: "hello".into(),
                start: 0.0,
                end: 0.5,
            }],
            None,
            None,
        );
        let cuts = crate::data::soft_cut::ClipCuts {
            cuts: vec![crate::data::soft_cut::Cut {
                id: "c1".into(),
                note: None,
                a_word: "missing".into(),
                b_word: "w0".into(),
                kind: crate::data::soft_cut::CutKind::Manual,
                duration: 0.5,
            }],
        };
        assert_eq!(
            audit_with_cuts(&d, &cuts)
                .by_code(Code::CutProvenanceUnavailable)
                .len(),
            1
        );
    }

    #[test]
    fn overlapping_raw_cuts_report_broken_partition() {
        let d = doc(
            vec![
                Word {
                    id: "w0".into(),
                    text: "a".into(),
                    start: 0.0,
                    end: 1.0,
                },
                Word {
                    id: "w1".into(),
                    text: "b".into(),
                    start: 1.0,
                    end: 2.0,
                },
                Word {
                    id: "w2".into(),
                    text: "c".into(),
                    start: 2.0,
                    end: 3.0,
                },
            ],
            None,
            None,
        );
        let cuts = crate::data::soft_cut::ClipCuts {
            cuts: vec![
                crate::data::soft_cut::Cut {
                    id: "c1".into(),
                    note: None,
                    a_word: "w0".into(),
                    b_word: "w1".into(),
                    kind: crate::data::soft_cut::CutKind::Manual,
                    duration: 2.0,
                },
                crate::data::soft_cut::Cut {
                    id: "c2".into(),
                    note: None,
                    a_word: "w1".into(),
                    b_word: "w2".into(),
                    kind: crate::data::soft_cut::CutKind::Manual,
                    duration: 2.0,
                },
            ],
        };
        assert_eq!(
            audit_with_cuts(&d, &cuts)
                .by_code(Code::CutPartitionBroken)
                .len(),
            1
        );
    }

    #[test]
    fn merged_cut_duration_drives_heavy_removal() {
        let mut d = doc(
            vec![
                Word {
                    id: "w0".into(),
                    text: "a".into(),
                    start: 0.0,
                    end: 2.0,
                },
                Word {
                    id: "w1".into(),
                    text: "b".into(),
                    start: 2.0,
                    end: 3.0,
                },
            ],
            None,
            None,
        );
        d.media.duration_seconds = 4.0;
        let cuts = crate::data::soft_cut::ClipCuts {
            cuts: vec![crate::data::soft_cut::Cut {
                id: "c1".into(),
                note: None,
                a_word: "w0".into(),
                b_word: "w0".into(),
                kind: crate::data::soft_cut::CutKind::Manual,
                duration: 0.0,
            }],
        };
        assert_eq!(
            audit_with_cuts(&d, &cuts)
                .by_code(Code::CutHeavyRemoval)
                .len(),
            1
        );
    }

    #[test]
    fn target_width_and_flash_use_translation_word_provenance() {
        let mut d = doc(
            vec![Word {
                id: "w0".into(),
                text: "hello".into(),
                start: 0.0,
                end: 0.5,
            }],
            Some("en"),
            None,
        );
        d.translations.insert(
            "zh".into(),
            BTreeMap::from([(
                "s1".into(),
                TranslationGroup {
                    id: "s1".into(),
                    text: "这是一个明显超过单行硬限制并且闪得太快的完整句子。".into(),
                    source_words: vec!["w0".into()],
                    source_text: Some("hello".into()),
                },
            )]),
        );
        let report = audit(&d);
        assert_eq!(report.by_code(Code::TargetWidth).len(), 1);
        assert_eq!(report.by_code(Code::TargetFlash).len(), 1);
        assert_eq!(report.by_code(Code::TargetFlashCompleteSentence).len(), 1);
        assert_eq!(
            report.by_code(Code::TargetWidth)[0].severity,
            Severity::Fail
        );
        assert_eq!(
            report.by_code(Code::TargetFlash)[0].severity,
            Severity::Warn
        );
        assert_eq!(
            report.by_code(Code::TargetFlashCompleteSentence)[0].severity,
            Severity::Warn
        );
        assert_eq!(Code::TargetWidthAim.severity(), Severity::Warn);
    }

    #[test]
    fn source_edit_marks_translation_stale() {
        let mut d = doc(
            vec![Word {
                id: "w0".into(),
                text: "edited".into(),
                start: 0.0,
                end: 1.0,
            }],
            Some("en"),
            None,
        );
        d.paragraphs[0].sentences[0].text = "edited".into();
        d.translations.insert(
            "zh".into(),
            BTreeMap::from([(
                "s1".into(),
                TranslationGroup {
                    id: "s1".into(),
                    text: "已编辑。".into(),
                    source_words: vec!["w0".into()],
                    source_text: Some("original".into()),
                },
            )]),
        );
        assert_eq!(audit(&d).by_code(Code::TranslationStale).len(), 1);
    }

    #[test]
    fn source_layout_and_translation_stamp_detectors_are_fed() {
        let mut d = doc(
            vec![
                Word {
                    id: "w0".into(),
                    text: "an extremely long source subtitle that cannot fit on two lines".into(),
                    start: 0.0,
                    end: 0.2,
                },
                Word {
                    id: "w1".into(),
                    text: "tail".into(),
                    start: 0.2,
                    end: 0.4,
                },
            ],
            Some("en"),
            None,
        );
        d.paragraphs[0].sentences.push(Sentence {
            id: "s2".into(),
            text: "other".into(),
            words: vec![Word {
                id: "w2".into(),
                text: "other".into(),
                start: 0.4,
                end: 1.0,
            }],
        });
        d.translations.insert(
            "zh".into(),
            BTreeMap::from([(
                "s1".into(),
                TranslationGroup {
                    id: "s1".into(),
                    text: "这是偏长，仍然可以安全拆开的字幕内容".into(),
                    source_words: vec!["w1".into(), "w2".into()],
                    source_text: Some(d.paragraphs[0].sentences[0].text.clone()),
                },
            )]),
        );
        let report = audit(&d);
        assert_eq!(report.by_code(Code::SourceWidth).len(), 1);
        assert_eq!(report.by_code(Code::SourceFlash).len(), 1);
        assert_eq!(
            report.by_code(Code::SourceFlash)[0].severity,
            Severity::Warn
        );
        assert_eq!(report.by_code(Code::TranslationStampMissing).len(), 1);
        assert_eq!(report.by_code(Code::TranslationStampExtra).len(), 1);
        assert_eq!(report.by_code(Code::TranslationBoundaryDrift).len(), 1);
        assert_eq!(report.by_code(Code::TargetSplittableOverAim).len(), 1);
    }

    #[test]
    fn adjacent_target_fragments_report_merge_opportunity() {
        let mut d = doc(
            vec![Word {
                id: "w0".into(),
                text: "one".into(),
                start: 0.0,
                end: 0.2,
            }],
            Some("en"),
            None,
        );
        d.paragraphs[0].sentences.push(Sentence {
            id: "s2".into(),
            text: "two".into(),
            words: vec![Word {
                id: "w1".into(),
                text: "two".into(),
                start: 0.2,
                end: 2.0,
            }],
        });
        d.translations.insert(
            "zh".into(),
            BTreeMap::from([
                (
                    "s1".into(),
                    TranslationGroup {
                        id: "s1".into(),
                        text: "这是第一段".into(),
                        source_words: vec!["w0".into()],
                        source_text: Some("one".into()),
                    },
                ),
                (
                    "s2".into(),
                    TranslationGroup {
                        id: "s2".into(),
                        text: "接着第二段".into(),
                        source_words: vec!["w1".into()],
                        source_text: Some("two".into()),
                    },
                ),
            ]),
        );
        let report = audit(&d);
        assert_eq!(report.by_code(Code::TargetMergeableFragments).len(), 1);
        assert_eq!(
            report
                .by_code(Code::TranslationFlashMergeCrossSentence)
                .len(),
            1
        );
    }

    #[test]
    fn cleanup_audit_reports_uncut_hard_filler_after_cleanup_started() {
        let d = doc(
            vec![
                Word {
                    id: "w0".into(),
                    text: "um".into(),
                    start: 0.0,
                    end: 0.2,
                },
                Word {
                    id: "w1".into(),
                    text: "uh".into(),
                    start: 0.2,
                    end: 0.4,
                },
            ],
            Some("en"),
            None,
        );
        let cuts = ClipCuts {
            cuts: vec![crate::data::Cut {
                id: "c1".into(),
                note: Some("filler".into()),
                a_word: "w0".into(),
                b_word: "w0".into(),
                kind: CutKind::Filler,
                duration: 0.2,
            }],
        };
        assert_eq!(
            audit_with_cuts(&d, &cuts)
                .by_code(Code::CleanupFixedFillerResidual)
                .len(),
            1
        );
    }

    #[test]
    fn source_fidelity_compares_surface_text_to_timed_words() {
        let mut lost = doc(
            vec![
                Word {
                    id: "w0".into(),
                    text: "hello".into(),
                    start: 0.0,
                    end: 0.5,
                },
                Word {
                    id: "w1".into(),
                    text: "world".into(),
                    start: 0.5,
                    end: 1.0,
                },
            ],
            Some("en"),
            None,
        );
        lost.paragraphs[0].sentences[0].text = "hello".into();
        assert_eq!(audit(&lost).by_code(Code::SourceContentLoss).len(), 1);

        let mut introduced = lost.clone();
        introduced.paragraphs[0].sentences[0].text = "hello brave world".into();
        let report = audit(&introduced);
        assert_eq!(report.by_code(Code::SourceContentDrift).len(), 1);
        assert_eq!(report.by_code(Code::PolishIntroducedZeroDuration).len(), 1);

        let mut unavailable = lost;
        unavailable.paragraphs[0].sentences[0].words.clear();
        unavailable.paragraphs[0].sentences[0].text = "surface only".into();
        assert_eq!(
            audit(&unavailable)
                .by_code(Code::SourceFidelityUnavailable)
                .len(),
            1
        );
    }

    #[test]
    fn translation_pin_spanning_source_sentences_requires_paragraph_split() {
        let mut d = doc(
            vec![Word {
                id: "w0".into(),
                text: "one".into(),
                start: 0.0,
                end: 1.0,
            }],
            Some("en"),
            None,
        );
        d.paragraphs[0].sentences.push(Sentence {
            id: "s2".into(),
            text: "two".into(),
            words: vec![Word {
                id: "w1".into(),
                text: "two".into(),
                start: 1.0,
                end: 2.0,
            }],
        });
        d.translations.insert(
            "zh".into(),
            BTreeMap::from([(
                "s1".into(),
                TranslationGroup {
                    id: "s1".into(),
                    text: "一二".into(),
                    source_words: vec!["w0".into(), "w1".into()],
                    source_text: Some("one".into()),
                },
            )]),
        );
        assert_eq!(audit(&d).by_code(Code::ParagraphPinNotSplit).len(), 1);
    }

    #[test]
    fn project_audit_checks_broll_assets_geometry_and_overlap() {
        let mut d = doc(
            vec![Word {
                id: "w0".into(),
                text: "hello".into(),
                start: 0.0,
                end: 10.0,
            }],
            Some("en"),
            None,
        );
        d.media.duration_seconds = 12.0;
        let placements = vec![
            crate::data::broll::BrollPlacement {
                id: "br-1".into(),
                file: "/definitely/missing.png".into(),
                start: 4.0,
                end: 7.0,
                mode: crate::data::broll::PlacementMode::Pip,
                rect: Some(crate::data::broll::Rect {
                    x: 1800,
                    y: 900,
                    width: 400,
                    height: 300,
                }),
                fit: crate::data::broll::FitMode::Cover,
                background: crate::data::broll::BackgroundMode::Black,
                source_start: 0.0,
                radius: 0,
                name: None,
            },
            crate::data::broll::BrollPlacement {
                id: "br-2".into(),
                file: "/definitely/missing-2.png".into(),
                start: 6.0,
                end: 8.0,
                mode: crate::data::broll::PlacementMode::Fullscreen,
                rect: None,
                fit: crate::data::broll::FitMode::Cover,
                background: crate::data::broll::BackgroundMode::Black,
                source_start: 0.0,
                radius: 0,
                name: None,
            },
        ];
        let report = audit_with_project(&d, &crate::data::ClipCuts::new(), &placements);
        assert_eq!(report.by_code(Code::BrollMissingAsset).len(), 2);
        assert_eq!(report.by_code(Code::BrollRectOutOfBounds).len(), 1);
        assert_eq!(report.by_code(Code::BrollOverlap).len(), 1);
    }

    fn align_doc() -> Doc {
        let mut value = doc(
            vec![
                Word {
                    id: "w0".into(),
                    text: "hello".into(),
                    start: 0.0,
                    end: 0.5,
                },
                Word {
                    id: "w1".into(),
                    text: "world".into(),
                    start: 0.5,
                    end: 1.0,
                },
            ],
            Some("en"),
            None,
        );
        value.translations.insert(
            "zh".into(),
            BTreeMap::from([(
                "s1".into(),
                TranslationGroup {
                    id: "s1".into(),
                    text: "你好世界".into(),
                    source_words: vec!["w0".into(), "w1".into()],
                    source_text: Some("hello world".into()),
                },
            )]),
        );
        value
    }

    #[test]
    fn completed_align_without_artifact_reports_unavailable_provenance() {
        let temp = tempfile::tempdir().unwrap();
        let done = temp.path().join("ai/align/done");
        std::fs::create_dir_all(&done).unwrap();
        std::fs::write(done.join("align-call.json"), "{}").unwrap();

        let report = audit_project(
            &align_doc(),
            &crate::data::ClipCuts::new(),
            &[],
            temp.path(),
        );
        assert_eq!(
            report
                .by_code(Code::TranslationSeamProvenanceUnavailable)
                .len(),
            1
        );
    }

    #[test]
    fn valid_align_artifact_is_consumed_without_provenance_finding() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("ai")).unwrap();
        let document = align_doc();
        let artifact = crate::pipeline::TranslateRebindArtifact::from_doc(&document, "zh");
        std::fs::write(
            temp.path().join("ai/align-artifact.json"),
            serde_json::to_vec(&artifact).unwrap(),
        )
        .unwrap();

        let report = audit_project(&document, &crate::data::ClipCuts::new(), &[], temp.path());
        assert!(report
            .by_code(Code::TranslationSeamProvenanceUnavailable)
            .is_empty());
        assert!(report.by_code(Code::PipelineFallback).is_empty());
    }

    #[test]
    fn align_artifact_flags_fallback_and_locked_marker_mutation() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("ai")).unwrap();
        let document = align_doc();
        let mut artifact = crate::pipeline::TranslateRebindArtifact::from_doc(&document, "zh");
        artifact.seams[0].aligned_end_id = "w0".into();
        artifact.seams[0].locked = Some(true);
        let mut value = serde_json::to_value(artifact).unwrap();
        value["projection"] = serde_json::json!("uniformFallback");
        std::fs::write(
            temp.path().join("ai/align-artifact.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();

        let report = audit_project(&document, &crate::data::ClipCuts::new(), &[], temp.path());
        assert_eq!(report.by_code(Code::PipelineFallback).len(), 1);
        assert_eq!(report.by_code(Code::TranslationLockedSeamMoved).len(), 1);
    }

    #[test]
    fn polish_quality_artifact_surfaces_residual_terms() {
        let temp = tempfile::tempdir().unwrap();
        let document = doc(
            vec![
                Word {
                    id: "w0".into(),
                    text: "Cloud".into(),
                    start: 0.0,
                    end: 0.5,
                },
                Word {
                    id: "w1".into(),
                    text: "Code".into(),
                    start: 0.5,
                    end: 1.0,
                },
            ],
            Some("en"),
            None,
        );
        let artifact = crate::pipeline::polish::PolishQualityArtifact {
            fingerprint: crate::pipeline::fingerprint_words(&document),
            created_at: Utc::now(),
            status: crate::pipeline::polish::PolishQualityStatus::Warn,
            page_count: 1,
            measured_page_count: 1,
            retry_count: 0,
            recovered_page_count: 0,
            fallback_page_count: 0,
            fallback_sentence_count: 0,
            residual_term_variant_count: 1,
            residual_term_variants: vec![crate::pipeline::polish::ResidualVariant {
                canonical: "Claude Code".into(),
                variant: "Cloud Code".into(),
                occurrences: 1,
            }],
            zero_duration_word_count_before: 0,
            zero_duration_word_count_after: 0,
        };
        artifact
            .save(&temp.path().join("ai/polish-quality.json"))
            .unwrap();

        let report = audit_project(&document, &crate::data::ClipCuts::new(), &[], temp.path());
        assert_eq!(report.by_code(Code::PolishResidualTerm).len(), 1);
    }

    #[test]
    fn translated_project_without_analysis_reports_term_audit_unavailable() {
        let temp = tempfile::tempdir().unwrap();
        let report = audit_project(
            &align_doc(),
            &crate::data::ClipCuts::new(),
            &[],
            temp.path(),
        );
        let finding = report.by_code(Code::TargetTermAuditUnavailable);
        assert_eq!(finding.len(), 1);
        assert_eq!(finding[0].severity, Severity::Warn);
    }

    #[test]
    fn locked_terms_detect_split_budget_and_source_coverage() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("ai")).unwrap();
        let mut document = doc(
            vec![Word {
                id: "w0".into(),
                text: "Claude Code".into(),
                start: 0.0,
                end: 2.0,
            }],
            Some("en"),
            None,
        );
        document.paragraphs[0].sentences.push(Sentence {
            id: "s2".into(),
            text: "continues".into(),
            words: vec![Word {
                id: "w1".into(),
                text: "continues".into(),
                start: 2.0,
                end: 4.0,
            }],
        });
        document.paragraphs[0].sentences.push(Sentence {
            id: "s3".into(),
            text: "elsewhere".into(),
            words: vec![Word {
                id: "w2".into(),
                text: "elsewhere".into(),
                start: 4.0,
                end: 6.0,
            }],
        });
        let long_term = "ABCDEFGHIJKLMNOPQRSTUVW";
        document.translations.insert(
            "zh".into(),
            BTreeMap::from([
                (
                    "s1".into(),
                    TranslationGroup {
                        id: "s1".into(),
                        text: "Claude".into(),
                        source_words: vec!["w0".into()],
                        source_text: Some("Claude Code".into()),
                    },
                ),
                (
                    "s2".into(),
                    TranslationGroup {
                        id: "s2".into(),
                        text: format!(" Code {long_term}"),
                        source_words: vec!["w1".into()],
                        source_text: Some("continues".into()),
                    },
                ),
                (
                    "s3".into(),
                    TranslationGroup {
                        id: "s3".into(),
                        text: "Claude Code".into(),
                        source_words: vec!["w2".into()],
                        source_text: Some("elsewhere".into()),
                    },
                ),
            ]),
        );
        std::fs::write(
            temp.path().join("ai/analysis.json"),
            serde_json::to_vec(&serde_json::json!({
                "summary": "x",
                "terms": [
                    {"term": "Claude Code", "observedVariants": [], "locked": true},
                    {"term": long_term, "observedVariants": [], "locked": true}
                ],
                "namedEntities": []
            }))
            .unwrap(),
        )
        .unwrap();

        let report = audit_project(&document, &crate::data::ClipCuts::new(), &[], temp.path());
        assert_eq!(report.by_code(Code::TargetTermSplit).len(), 1);
        assert_eq!(report.by_code(Code::TargetAtomicTermOverBudget).len(), 1);
        assert_eq!(report.by_code(Code::TranslationTermSourceCoverage).len(), 1);
        assert!(report.by_code(Code::TargetTermAuditUnavailable).is_empty());
    }

    #[test]
    fn native_break_pins_and_clip_boundaries_are_audited() {
        let temp = tempfile::tempdir().unwrap();
        let document = doc(
            vec![Word {
                id: "w0".into(),
                text: "hello".into(),
                start: 0.0,
                end: 1.0,
            }],
            Some("en"),
            None,
        );
        document.save(temp.path()).unwrap();
        let path = temp.path().join("doc.json");
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        value["breaks"] = serde_json::json!({"ghost-break": "sentence"});
        value["paraBreaks"] = serde_json::json!({"ghost-pin": true});
        value["clips"] = serde_json::json!([
            {"id":"c1","start":0.5,"end":1.0,"src":0.0,"cut":true}
        ]);
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();

        let report = audit_project(&document, &crate::data::ClipCuts::new(), &[], temp.path());
        assert_eq!(report.by_code(Code::OrphanBreak).len(), 1);
        assert_eq!(report.by_code(Code::OrphanParagraphPin).len(), 1);
        assert_eq!(report.by_code(Code::CutMidWordBoundary).len(), 1);
    }
}
