//! Transcript editing — find/replace and sentence split/merge operations.

use regex::Regex;
use std::collections::BTreeSet;

use crate::data::doc::{Doc, Sentence, Word};
use crate::error::{AppError, AppResult};

fn words_text(words: &[Word]) -> String {
    words.iter().fold(String::new(), |mut text, word| {
        if !text.is_empty() && surface_needs_space(&text, &word.text) {
            text.push(' ');
        }
        text.push_str(&word.text);
        text
    })
}

fn surface_needs_space(left: &str, right: &str) -> bool {
    left.chars()
        .next_back()
        .is_some_and(|c| c.is_ascii_alphanumeric())
        && right
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
}

fn join_surfaces(left: &str, right: &str) -> String {
    if surface_needs_space(left, right) {
        format!("{left} {right}")
    } else {
        format!("{left}{right}")
    }
}

/// Replace `query` with `replacement` across every sentence's text.
/// Returns the number of sentences changed.
pub fn find_replace(
    doc: &mut Doc,
    query: &str,
    replacement: &str,
    regex: bool,
) -> AppResult<usize> {
    let re = if regex {
        Some(Regex::new(query).map_err(|e| AppError::Schema(format!("regex: {e}")))?)
    } else {
        None
    };
    let mut n = 0;
    for p in &mut doc.paragraphs {
        for s in &mut p.sentences {
            let new = match &re {
                Some(r) => r.replace_all(&s.text, replacement).into_owned(),
                None => s.text.replace(query, replacement),
            };
            if new != s.text {
                s.words = crate::data::rebind::rebind_corrected(&s.words, &new);
                s.text = new;
                n += 1;
            }
        }
    }
    Ok(n)
}

/// Split a sentence into two at word index `at` (1..len-1). The first
/// half keeps the id; the second gets `<id>-b`. Returns `true` if split.
pub fn split_sentence(doc: &mut Doc, id: &str, at: usize) -> bool {
    let existing_ids: BTreeSet<String> = doc
        .paragraphs
        .iter()
        .flat_map(|paragraph| {
            paragraph
                .sentences
                .iter()
                .map(|sentence| sentence.id.clone())
        })
        .collect();
    let mut new_id = format!("{id}-b");
    let mut suffix = 2;
    while existing_ids.contains(&new_id) {
        new_id = format!("{id}-b{suffix}");
        suffix += 1;
    }
    let mut did_split = false;
    for p in &mut doc.paragraphs {
        if let Some(i) = p.sentences.iter().position(|s| s.id == id) {
            let len = p.sentences[i].words.len();
            if at == 0 || at >= len {
                return false;
            }
            let words = std::mem::take(&mut p.sentences[i].words);
            let (wa, wb): (Vec<Word>, Vec<Word>) = (words[..at].to_vec(), words[at..].to_vec());
            let a = Sentence {
                id: id.into(),
                text: words_text(&wa),
                words: wa,
            };
            let b = Sentence {
                id: new_id,
                text: words_text(&wb),
                words: wb,
            };
            p.sentences.splice(i..=i, [a, b]);
            did_split = true;
            break;
        }
    }
    if did_split {
        // A translation of the original full sentence is no longer valid for
        // either half. Removing it makes translation coverage honestly stale.
        for groups in doc.translations.values_mut() {
            groups.remove(id);
        }
    }
    did_split
}

/// Merge two sentences (same paragraph) into one, keeping the first id.
/// Returns `true` if merged.
pub fn merge_sentences(doc: &mut Doc, id1: &str, id2: &str) -> bool {
    let mut merged_ids: Option<(String, String, String)> = None;
    for p in &mut doc.paragraphs {
        let pos1 = p.sentences.iter().position(|s| s.id == id1);
        let pos2 = p.sentences.iter().position(|s| s.id == id2);
        if let (Some(i), Some(j)) = (pos1, pos2) {
            let (a, b) = if i < j { (i, j) } else { (j, i) };
            if b != a + 1 {
                return false;
            }
            let mut words = p.sentences[a].words.clone();
            words.extend(p.sentences[b].words.clone());
            let text = join_surfaces(&p.sentences[a].text, &p.sentences[b].text);
            let kept_id = p.sentences[a].id.clone();
            let removed_id = p.sentences[b].id.clone();
            let merged = Sentence {
                id: kept_id.clone(),
                text: text.clone(),
                words,
            };
            p.sentences.splice(a..=b, std::iter::once(merged));
            merged_ids = Some((kept_id, removed_id, text));
            break;
        }
    }
    let Some((kept_id, removed_id, source_text)) = merged_ids else {
        return false;
    };
    for groups in doc.translations.values_mut() {
        let first = groups.remove(&kept_id);
        let second = groups.remove(&removed_id);
        if let (Some(mut first), Some(second)) = (first, second) {
            first.id = kept_id.clone();
            first.text = join_surfaces(&first.text, &second.text);
            first.source_words.extend(second.source_words);
            first.source_text = Some(source_text.clone());
            groups.insert(kept_id.clone(), first);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::doc::*;

    fn doc_2s() -> Doc {
        let mk = |sid: &str, t: &str, words: Vec<&str>| Sentence {
            id: sid.into(),
            text: t.into(),
            words: words
                .into_iter()
                .enumerate()
                .map(|(i, w)| Word {
                    id: format!("{sid}-w{i}"),
                    text: w.into(),
                    start: i as f64,
                    end: i as f64 + 0.5,
                })
                .collect(),
        };
        Doc {
            id: "p".into(),
            schema: 1,
            media: MediaRef {
                path: "/tmp/x".into(),
                duration_seconds: 5.0,
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
                sentences: vec![
                    mk("s1", "alpha beta gamma", vec!["alpha", "beta", "gamma"]),
                    mk("s2", "delta epsilon", vec!["delta", "epsilon"]),
                ],
            }],
            translations: Default::default(),
        }
    }

    #[test]
    fn find_replace_substring_and_regex() {
        let mut d = doc_2s();
        assert_eq!(find_replace(&mut d, "a", "X", false).unwrap(), 2); // both sentences have 'a'
        assert!(d.paragraphs[0].sentences[0].text.contains('X'));

        let mut d = doc_2s();
        assert_eq!(find_replace(&mut d, r"^alpha", "ALPHA", true).unwrap(), 1);
        assert_eq!(d.paragraphs[0].sentences[0].text, "ALPHA beta gamma");
    }

    #[test]
    fn split_sentence_in_two() {
        let mut d = doc_2s();
        assert!(split_sentence(&mut d, "s1", 1)); // split after "alpha"
        let p = &d.paragraphs[0];
        assert_eq!(p.sentences.len(), 3);
        assert_eq!(p.sentences[0].id, "s1");
        assert_eq!(p.sentences[1].id, "s1-b");
        assert_eq!(p.sentences[0].words.len(), 1);
        assert_eq!(p.sentences[1].words.len(), 2);
    }

    #[test]
    fn merge_two_sentences() {
        let mut d = doc_2s();
        assert!(merge_sentences(&mut d, "s1", "s2"));
        assert_eq!(d.paragraphs[0].sentences.len(), 1);
        assert_eq!(d.paragraphs[0].sentences[0].words.len(), 5);
        assert_eq!(d.paragraphs[0].sentences[0].id, "s1");
    }

    #[test]
    fn split_at_boundary_is_noop() {
        let mut d = doc_2s();
        assert!(!split_sentence(&mut d, "s1", 0));
        assert!(!split_sentence(&mut d, "s1", 3)); // len
    }

    #[test]
    fn cjk_split_and_merge_do_not_invent_spaces() {
        let mut d = doc_2s();
        d.paragraphs[0].sentences[0].text = "你好世界".into();
        d.paragraphs[0].sentences[0].words = ["你", "好", "世", "界"]
            .into_iter()
            .enumerate()
            .map(|(index, text)| Word {
                id: format!("c{index}"),
                text: text.into(),
                start: index as f64,
                end: index as f64 + 0.5,
            })
            .collect();
        assert!(split_sentence(&mut d, "s1", 2));
        assert_eq!(d.paragraphs[0].sentences[0].text, "你好");
        assert_eq!(d.paragraphs[0].sentences[1].text, "世界");
        assert!(merge_sentences(&mut d, "s1", "s1-b"));
        assert_eq!(d.paragraphs[0].sentences[0].text, "你好世界");
    }

    #[test]
    fn repeated_split_uses_a_unique_sentence_id() {
        let mut d = doc_2s();
        assert!(split_sentence(&mut d, "s1", 2));
        assert!(split_sentence(&mut d, "s1", 1));
        let ids: Vec<_> = d.paragraphs[0]
            .sentences
            .iter()
            .map(|sentence| sentence.id.as_str())
            .collect();
        assert_eq!(ids, ["s1", "s1-b2", "s1-b", "s2"]);
    }

    #[test]
    fn merge_rejects_non_adjacent_sentences() {
        let mut d = doc_2s();
        assert!(split_sentence(&mut d, "s1", 1));
        assert!(!merge_sentences(&mut d, "s1", "s2"));
    }
}
