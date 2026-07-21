//! Polish pipeline — apply + quality gate.
//!
//! The submit validator runs an **atom-LCS map gate** at 70% coverage: the
//! polished text must cover the source word set closely enough to retain its
//! word-level provenance. `PolishEstimator` also reports residual filler,
//! repeated words, and formatting regressions.
//!
//! `apply_polish` mirrors the contract's answer shape (`paragraphs →
//! sentences`) and routes every accepted correction through the same
//! word-preserving rebind used by deterministic subtitle corrections.
//! Unchanged words therefore keep their ids/timings while changed words
//! are interpolated only inside their neighbouring anchor gap.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::data::doc::Doc;
use crate::data::version::CueDiff;
use crate::error::AppResult;

/// LCS length of two atom slices, standard O(n·m) DP. Atoms are typically
/// non-whitespace characters so the gate works for both Latin words and
/// CJK characters without a tokenizer.
fn lcs_len(a: &[char], b: &[char]) -> usize {
    let (n, m) = (a.len(), b.len());
    if n == 0 || m == 0 {
        return 0;
    }
    let mut prev = vec![0usize; m + 1];
    let mut curr = vec![0usize; m + 1];
    for i in 1..=n {
        for j in 1..=m {
            curr[j] = if a[i - 1] == b[j - 1] {
                prev[j - 1] + 1
            } else {
                prev[j].max(curr[j - 1])
            };
        }
        std::mem::swap(&mut prev, &mut curr);
        curr.iter_mut().for_each(|x| *x = 0);
    }
    prev[m]
}

/// Atom-LCS coverage: fraction of `source` atoms that appear in longest
/// common subsequence with `target`. The contract gate is `>= 0.70`.
pub fn atom_lcs_coverage(source: &str, target: &str) -> f64 {
    let atoms = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<Vec<_>>();
    let (s, t) = (atoms(source), atoms(target));
    if s.is_empty() {
        return 1.0;
    }
    lcs_len(&s, &t) as f64 / s.len() as f64
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum PolishQuality {
    #[default]
    Pass,
    Warn,
    Fail,
}

/// Persisted status is intentionally two-valued: a failed estimate is rejected
/// before an artifact exists, while accepted results are `PASS` or `WARN`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum PolishQualityStatus {
    #[default]
    #[serde(rename = "PASS")]
    Pass,
    #[serde(rename = "WARN")]
    Warn,
}

/// A polish quality estimate — coverage plus the residual issues the
/// estimator flagged on the polished text.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PolishEstimate {
    pub quality: PolishQuality,
    pub coverage: f64,
    pub issues: Vec<String>,
}

/// Persisted polish-quality metrics and residual term variants.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PolishQualityArtifact {
    pub fingerprint: String,
    pub created_at: DateTime<Utc>,
    pub status: PolishQualityStatus,
    pub page_count: usize,
    pub measured_page_count: usize,
    pub retry_count: usize,
    pub recovered_page_count: usize,
    pub fallback_page_count: usize,
    pub fallback_sentence_count: usize,
    pub residual_term_variant_count: usize,
    pub residual_term_variants: Vec<ResidualVariant>,
    pub zero_duration_word_count_before: usize,
    pub zero_duration_word_count_after: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResidualVariant {
    pub canonical: String,
    pub variant: String,
    pub occurrences: usize,
}

impl PolishQualityArtifact {
    pub fn save(&self, path: &std::path::Path) -> AppResult<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(temp, path)?;
        Ok(())
    }

    pub fn load(path: &std::path::Path) -> AppResult<Self> {
        Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
    }
}

/// Hard-delete filler list shared with `pipeline::cleanup`. A residual filler in polished
/// text is a WARN (polish must preserve source fillers per contract
/// rule 3, but a *new* one it introduced is a regression).
const HARD_FILLERS: &[&str] = &[
    "um", "umm", "uh", "uhh", "er", "erm", "ah", "hmm", "mhm", "呃", "额",
];

/// Estimate polish quality for one sentence: the atom-LCS coverage gate
/// (contract §"Hard rules") plus residual detectors.
pub fn estimate_polish(source: &str, polished: &str) -> PolishEstimate {
    let coverage = atom_lcs_coverage(source, polished);
    let mut issues = Vec::new();

    if coverage < 0.70 {
        issues.push(format!(
            "atom-LCS coverage {:.0}% < 70% (text drift)",
            coverage * 100.0
        ));
    } else if coverage < 0.90 {
        issues.push(format!("atom-LCS coverage {:.0}% < 90%", coverage * 100.0));
    }
    if polished.contains("  ") {
        issues.push("double space introduced".into());
    }
    // repeated word ("the the")
    let words: Vec<&str> = polished.split_whitespace().collect();
    for w in words.windows(2) {
        if !w[0].is_empty() && w[0].eq_ignore_ascii_case(w[1]) {
            issues.push(format!("repeated word {:?}", w[0]));
            break;
        }
    }
    // residual hard filler that is NOT in the source (polish introduced it)
    let src_fillers: std::collections::HashSet<&str> = HARD_FILLERS.iter().copied().collect();
    for tok in polished.split_whitespace() {
        let norm = tok
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        if src_fillers.contains(norm.as_str()) {
            let also_in_source = source.split_whitespace().any(|s| {
                s.trim_matches(|c: char| !c.is_alphanumeric())
                    .eq_ignore_ascii_case(&norm)
            });
            if !also_in_source {
                issues.push(format!("polish introduced filler {:?}", tok));
                break;
            }
        }
    }

    let quality = if coverage < 0.70 {
        PolishQuality::Fail
    } else if !issues.is_empty() {
        PolishQuality::Warn
    } else {
        PolishQuality::Pass
    };
    PolishEstimate {
        quality,
        coverage,
        issues,
    }
}

/// Apply a polish answer: replace `sentence.text` with `after`. The
/// quality gate (`estimate_polish`) is the caller's to run on the same
/// `(before, after)` pair — kept separate so a dry-run can estimate
/// without mutating.
pub fn apply_polish(doc: &mut Doc, sentence_id: &str, after: &str) -> AppResult<Vec<CueDiff>> {
    let mut diffs = Vec::new();
    for para in &mut doc.paragraphs {
        for sent in &mut para.sentences {
            if sent.id == sentence_id {
                let before = sent.text.clone();
                sent.text = after.to_string();
                sent.words = crate::data::rebind::rebind_corrected(&sent.words, after);
                diffs.push(CueDiff::ReplaceSentence {
                    sentence_id: sent.id.clone(),
                    before,
                    after: after.to_string(),
                });
                return Ok(diffs);
            }
        }
    }
    Ok(diffs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::doc::*;
    use chrono::Utc;
    use std::path::PathBuf;

    fn doc() -> Doc {
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
            paragraphs: vec![Paragraph {
                id: 1,
                speaker: None,
                sentences: vec![Sentence {
                    id: "s1".into(),
                    text: "Hello".into(),
                    words: vec![Word {
                        id: "w0".into(),
                        text: "Hello".into(),
                        start: 0.0,
                        end: 0.5,
                    }],
                }],
            }],
            translations: Default::default(),
        }
    }

    #[test]
    fn apply_polish_replaces_text() {
        let mut d = doc();
        let diffs = apply_polish(&mut d, "s1", "Hi").unwrap();
        assert_eq!(d.paragraphs[0].sentences[0].text, "Hi");
        assert_eq!(d.paragraphs[0].sentences[0].words[0].text, "Hi");
        assert_eq!(d.paragraphs[0].sentences[0].words[0].start, 0.0);
        assert_eq!(d.paragraphs[0].sentences[0].words[0].end, 0.5);
        assert_eq!(diffs.len(), 1);
    }

    #[test]
    fn quality_artifact_uses_public_camel_case_keys() {
        let artifact = PolishQualityArtifact {
            fingerprint: "1:w0:w0:abc".into(),
            created_at: Utc::now(),
            status: PolishQualityStatus::Warn,
            page_count: 1,
            measured_page_count: 1,
            retry_count: 0,
            recovered_page_count: 0,
            fallback_page_count: 0,
            fallback_sentence_count: 0,
            residual_term_variant_count: 1,
            residual_term_variants: vec![ResidualVariant {
                canonical: "Claude Code".into(),
                variant: "Cloud Code".into(),
                occurrences: 1,
            }],
            zero_duration_word_count_before: 0,
            zero_duration_word_count_after: 0,
        };
        let value = serde_json::to_value(&artifact).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 13);
        assert_eq!(value["status"], "WARN");
        assert_eq!(value["residualTermVariants"][0]["canonical"], "Claude Code");
        assert!(value.get("zeroDurationWordCountAfter").is_some());
    }

    #[test]
    fn coverage_high_for_close_rewrite() {
        let c = atom_lcs_coverage("the quick brown fox", "the quick brown fox jumps");
        assert!(c > 0.99);
    }

    #[test]
    fn coverage_fails_on_text_drift() {
        // almost no shared atoms → coverage near 0
        let c = atom_lcs_coverage("abc", "xyz");
        assert!(c < 0.05);
    }

    #[test]
    fn estimate_passes_on_faithful_polish() {
        let e = estimate_polish("welcome back to the show", "welcome back to the show.");
        assert_eq!(e.quality, PolishQuality::Pass);
    }

    #[test]
    fn estimate_fails_below_seventy_percent_coverage() {
        // contract: < 70% atom-LCS → FAIL (rejected locally)
        let e = estimate_polish("the quick brown fox jumps over", "zzzzzzzzz qqqqqqq");
        assert_eq!(e.quality, PolishQuality::Fail);
        assert!(e.issues.iter().any(|i| i.contains("coverage")));
    }

    #[test]
    fn estimate_warns_on_introduced_filler() {
        // source has no filler; polished introduces "um" → WARN (rule 3
        // violation: polish must not add fillers).
        let e = estimate_polish("hello world", "um hello world");
        assert_eq!(e.quality, PolishQuality::Warn);
        assert!(e.issues.iter().any(|i| i.contains("filler")));
    }

    #[test]
    fn estimate_accepts_preserved_source_filler() {
        // source already has the filler → polish preserving it is PASS-side.
        let e = estimate_polish("um hello", "um hello.");
        assert_ne!(e.quality, PolishQuality::Fail);
        assert!(e.issues.iter().all(|i| !i.contains("introduced")));
    }
}
