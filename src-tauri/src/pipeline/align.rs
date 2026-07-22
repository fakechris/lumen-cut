//! `AIFlowRunner.runAlignOnly` — `--groups` targeted re-align.
//!
//! Re-alignment is invoked when:
//! 1.  The user re-edits a sentence.
//! 2.  A translation drops a word (so the timeline is shorter).
//! 3.  Polish soft-merges two sentences.
//!
//! The algorithm is **word-id rebind** + **seam provenance**:
//!
//! 1.  Walk the affected `group_id`s.
//! 2.  Preserve unchanged source-word ids and timings.
//! 3.  Emit `CueDiff::Replace` entries that the version-control module
//!     persists.
//!
//! Targeted mode (`--groups g1,g2`) only touches those groups; without
//! `--groups`, every group is realigned.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::data::doc::Doc;
use crate::data::version::CueDiff;
use crate::error::AppResult;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlignSpec {
    pub sentence_id: String,
    pub group_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AlignOutcome {
    pub diffs: Vec<CueDiff>,
    pub touched_groups: Vec<String>,
    /// Word-id rebind projection: `(word_id, projected_start, projected_end)`.
    #[serde(default)]
    pub timing: Vec<(String, f64, f64)>,
}

/// One `align list` row. Only target groups over the requested one-line fit
/// are returned; `overHard` uses the align contract's fixed 20-cell ceiling.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AlignCandidate {
    pub key: String,
    pub source_words: Vec<String>,
    pub target: String,
    pub seam_preview: String,
    pub fit_chars: usize,
    pub projected_cells: f64,
    pub over_hard: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AlignList {
    pub lang: String,
    pub fit_chars: usize,
    pub groups: Vec<AlignCandidate>,
    pub next: Option<String>,
}

/// Persisted provenance for a translation rebind operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TranslateRebindArtifact {
    pub fingerprint: String,
    pub lang: String,
    pub created_at: DateTime<Utc>,
    pub seams: Vec<RebindSeam>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reading_merges: Option<Vec<RebindReadingMerge>>,
}

/// One persisted translation seam.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RebindSeam {
    pub group_key: String,
    pub aligned_end_id: String,
    pub final_end_id: String,
    pub kept_raw: i64,
    pub displacement_words: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
}

/// Reading-speed merge metadata persisted with a rebind artifact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RebindReadingMerge {
    pub group_key: String,
    pub over_aim: bool,
    pub crosses_sentence: bool,
}

impl TranslateRebindArtifact {
    /// Construct the lossless subset available from lumen-cut's document
    /// model. With no explicit seam edit, aligned and final end ids are
    /// identical and no displacement is claimed.
    pub fn from_doc(doc: &Doc, lang: &str) -> Self {
        let seams = doc
            .translations
            .get(lang)
            .into_iter()
            .flat_map(|groups| groups.values())
            .filter_map(|group| {
                let end_id = group.source_words.last()?.clone();
                Some(RebindSeam {
                    group_key: group.id.clone(),
                    aligned_end_id: end_id.clone(),
                    final_end_id: end_id,
                    kept_raw: group.source_words.len() as i64,
                    displacement_words: false,
                    origin: None,
                    locked: None,
                })
            })
            .collect();
        Self {
            fingerprint: fingerprint_words(doc),
            lang: lang.to_string(),
            created_at: Utc::now(),
            seams,
            reading_merges: None,
        }
    }

    pub fn save(&self, path: &std::path::Path) -> AppResult<()> {
        crate::data::storage::write_json(path, self)
    }

    pub fn load(path: &std::path::Path) -> AppResult<Self> {
        Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
    }
}

/// Stable document fingerprint: word count, boundary ids, and a base-36
/// FNV-1a hash over UTF-8 word text.
pub fn fingerprint_words(doc: &Doc) -> String {
    let words = doc.all_words();
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for word in &words {
        for byte in word.text.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= 0x1f;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let first = words.first().map(|word| word.id.as_str()).unwrap_or("-");
    let last = words.last().map(|word| word.id.as_str()).unwrap_or("-");
    format!("{}:{first}:{last}:{}", words.len(), base36_u64(hash))
}

fn base36_u64(mut value: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 {
        return "0".into();
    }
    let mut output = Vec::with_capacity(13);
    while value != 0 {
        output.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    output.reverse();
    String::from_utf8(output).expect("base-36 digits are ASCII")
}

/// Targeted align. `groups` empty means "align everything".
pub fn align_targeted(doc: &Doc, groups: &[String]) -> AlignOutcome {
    let mut out = AlignOutcome::default();
    let in_scope = |gid: &str| groups.is_empty() || groups.iter().any(|g| g == gid);

    for para in &doc.paragraphs {
        for sent in &para.sentences {
            let gid = sent.id.clone();
            if !in_scope(&gid) {
                continue;
            }
            out.touched_groups.push(gid.clone());

            // Word-id rebind projection onto the sentence time window.
            for (wid, (s, e)) in rebind_word_timing(&sent.words) {
                out.timing.push((wid, s, e));
            }

            // Compute new text from word list. If the text and the
            // join of word.texts differ, we emit a Replace diff. This
            // is the targeted re-align "soft re-pack": we keep the
            // word timing, but the surfaced text comes from words[].
            let mut new_text = String::new();
            for (i, w) in sent.words.iter().enumerate() {
                if i > 0 {
                    new_text.push(' ');
                }
                new_text.push_str(&w.text);
            }
            if new_text != sent.text && !sent.words.is_empty() {
                out.diffs.push(CueDiff::ReplaceSentence {
                    sentence_id: sent.id.clone(),
                    before: sent.text.clone(),
                    after: new_text,
                });
            }
        }
    }
    out
}

/// Inspect an existing target language without mutating it and return only
/// groups that exceed the one-line fit.
pub fn align_list(doc: &Doc, lang: &str, fit: usize, pid: &str) -> AppResult<AlignList> {
    let fit = fit.clamp(8, 32);
    let translations = doc
        .translations
        .get(lang)
        .ok_or_else(|| crate::error::AppError::Schema(format!("no `{lang}` translations")))?;
    let mut groups = Vec::new();
    for (key, group) in translations {
        let projected_cells = target_cells(&group.text);
        if projected_cells <= fit as f64 {
            continue;
        }
        groups.push(AlignCandidate {
            key: key.clone(),
            source_words: group.source_words.clone(),
            target: group.text.clone(),
            seam_preview: target_seam_preview(&group.text),
            fit_chars: fit,
            projected_cells,
            over_hard: projected_cells > 20.0,
        });
    }
    let next = (!groups.is_empty()).then(|| {
        format!(
            "lumen-cut task start align {pid} --lang {lang} --groups {} --align-fit {fit}",
            groups
                .iter()
                .map(|group| group.key.as_str())
                .collect::<Vec<_>>()
                .join(",")
        )
    });
    Ok(AlignList {
        lang: lang.to_string(),
        fit_chars: fit,
        groups,
        next,
    })
}

fn target_cells(text: &str) -> f64 {
    text.chars()
        .map(|character| {
            if character.is_whitespace() || character.is_ascii_punctuation() {
                0.0
            } else if character.is_ascii() {
                0.5
            } else {
                1.0
            }
        })
        .sum()
}

fn target_seam_preview(text: &str) -> String {
    let mut preview = String::from("<#0>");
    for (index, character) in text.chars().enumerate() {
        if index > 0 {
            preview.push_str(&format!("<@t{index}>"));
        }
        preview.push(character);
    }
    preview.push_str("<#1>");
    preview
}

/// Helper: the **soft-cut projection** — given the kept spans from
/// `data::soft_cut::kept_spans`, return a `BTreeMap<word_id, f64>` of
/// projected end times for caption burn-in.
pub fn project_end_times(doc: &Doc, kept_spans: &[(f64, f64)]) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    // For each word, walk the kept spans: the word's effective end is the
    // original end, clamped to the kept span it falls into.
    for span in kept_spans {
        for w in doc.all_words() {
            if w.start >= span.0 && w.end <= span.1 {
                out.entry(w.id.clone()).or_insert(w.end);
            }
        }
    }
    out
}

/// Lossless word-id rebind projection.
///
/// Unchanged words keep their ids and timings. Uniformly
/// redistributing the whole sentence destroys correct ASR timings, so the
/// projection deliberately returns each current word unchanged. Local
/// interpolation belongs to the corrected-word path, which must carry the
/// explicit `refreshedWordIds` set rather than guessing from text.
pub fn rebind_word_timing(words: &[crate::data::doc::Word]) -> BTreeMap<String, (f64, f64)> {
    words
        .iter()
        .map(|word| (word.id.clone(), (word.start, word.end)))
        .collect()
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
                duration_seconds: 4.0,
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
                    id: "g1".into(),
                    text: "alpha beta".into(), // intentionally wrong vs words
                    words: vec![
                        Word {
                            id: "w0".into(),
                            text: "alpha".into(),
                            start: 0.0,
                            end: 0.4,
                        },
                        Word {
                            id: "w1".into(),
                            text: "beta".into(),
                            start: 0.5,
                            end: 0.9,
                        },
                    ],
                }],
            }],
            translations: Default::default(),
        }
    }

    #[test]
    fn targeted_align_emits_replace() {
        let mut d = doc();
        // text "alpha beta" but only 1 word in words[] → misalignment
        d.paragraphs[0].sentences[0].words = vec![Word {
            id: "w0".into(),
            text: "alpha".into(),
            start: 0.0,
            end: 0.4,
        }];
        let out = align_targeted(&d, &["g1".into()]);
        assert_eq!(out.touched_groups, vec!["g1"]);
        assert_eq!(out.diffs.len(), 1);
    }

    #[test]
    fn untargeted_align_skips_out_of_scope() {
        let out = align_targeted(&doc(), &["g2".into()]);
        assert_eq!(out.touched_groups.len(), 0);
    }

    #[test]
    fn project_end_times_clips_to_kept() {
        let d = doc();
        let kept = vec![(0.0, 1.0)];
        let proj = project_end_times(&d, &kept);
        assert_eq!(proj.get("w0"), Some(&0.4));
        // w1 spans 0.5..0.9, all inside [0.0, 1.0] → kept
        assert_eq!(proj.get("w1"), Some(&0.9));
    }

    #[test]
    fn rebind_preserves_valid_word_identity_and_timing() {
        // Unchanged words keep both their ids and original timings.
        let words = &doc().paragraphs[0].sentences[0].words;
        let proj = rebind_word_timing(words);
        let (s0, e0) = proj["w0"];
        let (s1, e1) = proj["w1"];
        assert!((s0 - 0.0).abs() < 1e-9);
        assert!((e0 - 0.4).abs() < 1e-9);
        assert!((s1 - 0.5).abs() < 1e-9);
        assert!((e1 - 0.9).abs() < 1e-9);
    }

    #[test]
    fn rebind_artifact_uses_public_camel_case_shape() {
        let mut source = doc();
        source.translations.insert(
            "zh".into(),
            BTreeMap::from([(
                "g1".into(),
                TranslationGroup {
                    id: "g1".into(),
                    text: "甲乙".into(),
                    source_words: vec!["w0".into(), "w1".into()],
                    source_text: Some("alpha beta".into()),
                },
            )]),
        );
        let artifact = TranslateRebindArtifact::from_doc(&source, "zh");
        let value = serde_json::to_value(&artifact).unwrap();
        assert_eq!(
            value
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["createdAt", "fingerprint", "lang", "seams"]
        );
        assert_eq!(artifact.fingerprint, "2:w0:w1:3l7hy1y7of7z7");
        assert_eq!(artifact.seams.len(), 1);
        assert_eq!(artifact.seams[0].group_key, "g1");
        assert_eq!(artifact.seams[0].aligned_end_id, "w1");
        assert_eq!(artifact.seams[0].final_end_id, "w1");
    }

    #[test]
    fn align_targeted_projects_timing() {
        let out = align_targeted(&doc(), &["g1".into()]);
        assert_eq!(out.timing.len(), 2); // w0, w1 projected
    }

    #[test]
    fn align_list_returns_only_over_fit_target_groups() {
        let mut source = doc();
        source.translations.insert(
            "zh".into(),
            BTreeMap::from([
                (
                    "g1".into(),
                    TranslationGroup {
                        id: "g1".into(),
                        text: "短句".into(),
                        source_words: vec!["w0".into()],
                        source_text: Some("alpha".into()),
                    },
                ),
                (
                    "g2".into(),
                    TranslationGroup {
                        id: "g2".into(),
                        text: "这是一个明显超过八个字符的一整句翻译".into(),
                        source_words: vec!["w1".into()],
                        source_text: Some("beta".into()),
                    },
                ),
            ]),
        );
        let list = align_list(&source, "zh", 8, "demo").unwrap();
        assert_eq!(list.groups.len(), 1);
        assert_eq!(list.groups[0].key, "g2");
        assert_eq!(list.groups[0].fit_chars, 8);
        assert!(list.groups[0].seam_preview.starts_with("<#0>"));
        assert!(list.next.unwrap().contains("--groups g2"));
    }
}
