//! `CLIFlows.runTranslateStale` — fit-budget algorithm + two-phase.
//!
//! ## Why this matters
//!
//! Translation calls are the most expensive kind of agent call. Each
//! answer is one sentence's translation in a target language, and a
//! single 5-minute video can have ~80 sentences.  We don't want to send
//! all 80 to one round-trip — the model is most useful when given the
//! surrounding glossary, brief, and recent translations, and least
//! useful when given too many sentences at once.
//!
//! The **fit-budget** algorithm packs sentences into a single call so
//! the prompt stays under the token limit, while preserving sentence
//! ordering and never splitting a sentence across calls.
//!
//! ## Two-phase
//!
//! * **Phase 1 (analysis/brief)**: each page answer carries the merged
//!   `summary` / `terms` / `namedEntities` superset. The orchestrator merges
//!   those fields into `ai/analysis.json`; user-locked terms are then
//!   materialized as `rt[]` on every relevant later page.
//! * **Phase 2 (sentences)**: source sentences are translated whole, then
//!   over-fit target groups are split/re-aligned by the align task. The
//!   packer below is order-preserving greedy first-fit.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Token budget per call. Default is 8192 (compatible with both
/// `gpt-4o-mini` and `claude-3-5-haiku`).
pub const DEFAULT_BUDGET: u32 = 8192;
/// Keep enough neighbouring cues in one request for discourse consistency
/// without making a single provider call too large or slow to recover.
pub const MAX_LINES_PER_REQUEST: usize = 32;

/// Sentence packet that the packer consumes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentencePacket {
    pub sentence_id: String,
    pub text: String,
    pub word_count: usize,
}

/// Packed batch ready to be sent to the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslateBatch {
    pub sentences: Vec<SentencePacket>,
    pub estimated_tokens: u32,
}

/// Greedy first-fit packer over sentences **in their original order**
/// For each sentence, find the first batch with enough remaining budget; if
/// none, allocate a new batch.
/// Sentence order inside every batch is therefore the document order.
///
/// The token-cost heuristic is language-aware via
/// [`tokens_per_word_for_lang`]; this two-argument form keeps the
/// historical default language (`"en"`, 1.5 tokens/word).
pub fn pack(sentences: Vec<SentencePacket>, budget: u32) -> Vec<TranslateBatch> {
    pack_with_lang(sentences, budget, "en")
}

/// [`pack`] with an explicit language for the token-cost heuristic.
pub fn pack_with_lang(
    sentences: Vec<SentencePacket>,
    budget: u32,
    lang: &str,
) -> Vec<TranslateBatch> {
    pack_for_requests(sentences, budget, usize::MAX, lang)
}

/// Pack translation calls by token budget and a hard line-count ceiling.
/// Batches are contiguous so nearby cues provide useful context and the
/// flattened result always preserves document order.
pub fn pack_for_requests(
    sentences: Vec<SentencePacket>,
    budget: u32,
    max_lines: usize,
    lang: &str,
) -> Vec<TranslateBatch> {
    let tokens_per_word = tokens_per_word_for_lang(lang);
    let mut batches: Vec<TranslateBatch> = Vec::new();
    let mut current = TranslateBatch {
        sentences: Vec::new(),
        estimated_tokens: 0,
    };
    let max_lines = max_lines.max(1);

    for s in sentences {
        let tokens = ((s.word_count as f64) * tokens_per_word).ceil() as u32 + 8;
        if !current.sentences.is_empty()
            && (current.sentences.len() >= max_lines
                || current.estimated_tokens.saturating_add(tokens) > budget)
        {
            batches.push(current);
            current = TranslateBatch {
                sentences: Vec::new(),
                estimated_tokens: 0,
            };
        }
        current.sentences.push(s);
        current.estimated_tokens = current.estimated_tokens.saturating_add(tokens);
    }
    if !current.sentences.is_empty() {
        batches.push(current);
    }
    batches
}

/// Two-phase driver.  The `brief_call` is invoked once and produces a
/// summary string that we attach to every sentence call's `system`
/// block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefResult {
    pub summary: String,
    pub glossary: HashMap<String, String>,
}

/// Per-language token-cost heuristic used by [`pack_with_lang`]: CJK
/// text averages ~1.1 tokens per word/char in OpenAI's tiktoken, Latin
/// ~1.5 per word.
pub fn tokens_per_word_for_lang(lang: &str) -> f64 {
    match lang {
        "zh" | "ja" | "ko" => 1.1, // CJK: one token per char on average
        _ => 1.5,
    }
}

// ---- Character-capacity model, two-phase translation, and staleness ----

/// Per-line character aim for the target language. The configurable fit range
/// is 8–32 characters, with a CJK default of 16.
pub fn aim_chars_for_lang(lang: &str) -> usize {
    match lang {
        "zh" | "ja" | "ko" => 16,
        _ => 12,
    }
}

/// Hard per-line char cap = round(aim × 1.4). CJK → 22, Latin → 17.
pub fn hard_chars_for_lang(lang: &str) -> usize {
    ((aim_chars_for_lang(lang) as f64) * 1.4).round() as usize
}

/// Character-capacity packer: sentences are first-fit into batches whose running
/// non-whitespace char count stays under the hard cap; a sentence that
/// would overflow starts a new batch. Order-preserving within each batch.
pub fn pack_by_chars(sentences: Vec<SentencePacket>, lang: &str) -> Vec<TranslateBatch> {
    let hard = hard_chars_for_lang(lang).max(1);
    let cc = |s: &str| s.chars().filter(|c| !c.is_whitespace()).count().max(1);
    let mut batches: Vec<TranslateBatch> = Vec::new();
    let mut remain: Vec<usize> = Vec::new();
    for s in sentences {
        let n = cc(&s.text);
        let mut placed = false;
        for (i, r) in remain.iter_mut().enumerate() {
            if *r >= n {
                batches[i].sentences.push(s.clone());
                batches[i].estimated_tokens += n as u32;
                *r -= n;
                placed = true;
                break;
            }
        }
        if !placed {
            let cap = hard.max(n);
            batches.push(TranslateBatch {
                sentences: vec![s],
                estimated_tokens: n as u32,
            });
            remain.push(cap.saturating_sub(n));
        }
    }
    batches
}

/// The two translate phases (contract `translate.md` §"Two-phase model"):
/// Phase 1 translates whole natural sentences; Phase 2 splits &
/// re-aligns only the sentences that exceed the one-line fit capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranslatePhase {
    Translating,
    Splitting,
}

impl TranslatePhase {
    pub fn dir(self) -> &'static str {
        match self {
            TranslatePhase::Translating => "translating",
            TranslatePhase::Splitting => "splitting",
        }
    }
}

/// Tracks groups whose source changed since the last translation so a re-run
/// can process only dirty groups.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranslateStaleness {
    #[serde(default)]
    pub stale_group_keys: std::collections::BTreeSet<String>,
}

impl TranslateStaleness {
    pub fn is_stale(&self, key: &str) -> bool {
        self.stale_group_keys.contains(key)
    }
    pub fn mark_stale(&mut self, key: &str) {
        self.stale_group_keys.insert(key.to_string());
    }
    pub fn is_empty(&self) -> bool {
        self.stale_group_keys.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(id: &str, words: usize) -> SentencePacket {
        SentencePacket {
            sentence_id: id.into(),
            text: "x".repeat(words),
            word_count: words,
        }
    }

    #[test]
    fn packer_groups_small_sentences() {
        let sentences = (0..5).map(|i| pkt(&format!("s{i}"), 4)).collect();
        let batches = pack(sentences, DEFAULT_BUDGET);
        // Each sentence is 4 words * 1.5 = 6 tokens. They should all fit
        // in one batch.
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].sentences.len(), 5);
    }

    #[test]
    fn packer_splits_when_budget_exhausted() {
        // 3 sentences × 5000 words × 1.5 tokens ≈ 22500 tokens > 8192 → 3 batches.
        let sentences = (0..3).map(|i| pkt(&format!("s{i}"), 5000)).collect();
        let batches = pack(sentences, DEFAULT_BUDGET);
        assert!(batches.len() >= 2);
        assert!(batches.iter().all(|b| b.estimated_tokens <= DEFAULT_BUDGET));
    }

    #[test]
    fn packer_preserves_sentence_order() {
        // No size-based reordering: sentences stay in document order
        // within a batch.
        let sentences = vec![pkt("a", 10), pkt("b", 100), pkt("c", 10)];
        let batches = pack(sentences, DEFAULT_BUDGET);
        assert_eq!(batches.len(), 1);
        let ids: Vec<&str> = batches[0]
            .sentences
            .iter()
            .map(|s| s.sentence_id.as_str())
            .collect();
        assert_eq!(ids, ["a", "b", "c"]);

        // Tight budget: b leaves no room for c, so c starts a new batch —
        // the flattened order across batches is still a, b, c.
        let batches = pack(vec![pkt("a", 10), pkt("b", 100), pkt("c", 10)], 200);
        assert_eq!(batches.len(), 2);
        let ids: Vec<&str> = batches
            .iter()
            .flat_map(|b| b.sentences.iter())
            .map(|s| s.sentence_id.as_str())
            .collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }

    #[test]
    fn packer_uses_cjk_token_cost() {
        // Same packets, same budget: zh (1.1 tok/word) packs tighter
        // than en (1.5 tok/word).
        //   zh: ceil(4×1.1)=5 tokens → 5+8 = 13 per sentence → two fit (26 ≤ 27).
        //   en: 4×1.5 = 6 tokens → 6+8 = 14 per sentence → second overflows.
        let zh = pack_with_lang(vec![pkt("a", 4), pkt("b", 4)], 27, "zh");
        assert_eq!(zh.len(), 1);
        assert_eq!(zh[0].estimated_tokens, 26);
        let en = pack_with_lang(vec![pkt("a", 4), pkt("b", 4)], 27, "en");
        assert_eq!(en.len(), 2);
    }

    #[test]
    fn tokens_per_word_lang() {
        assert!(tokens_per_word_for_lang("zh") < tokens_per_word_for_lang("en"));
    }

    #[test]
    fn aim_and_hard_caps() {
        assert_eq!(aim_chars_for_lang("zh"), 16);
        assert_eq!(aim_chars_for_lang("en"), 12);
        assert_eq!(hard_chars_for_lang("zh"), 22); // round(16*1.4)
        assert_eq!(hard_chars_for_lang("en"), 17); // round(12*1.4)
    }

    #[test]
    fn pack_by_chars_fits_under_hard_cap() {
        let s = |id: &str| SentencePacket {
            sentence_id: id.into(),
            text: "abcdefghij".into(), // 10 non-ws chars
            word_count: 10,
        };
        // CJK hard=22 → two per batch (20 ≤ 22), third overflows.
        let batches = pack_by_chars(vec![s("a"), s("b"), s("c")], "zh");
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].sentences.len(), 2);
        assert_eq!(batches[1].sentences.len(), 1);
    }

    #[test]
    fn pack_by_chars_preserves_order() {
        let s = |id: &str| SentencePacket {
            sentence_id: id.into(),
            text: id.into(),
            word_count: 1,
        };
        let batches = pack_by_chars(vec![s("a"), s("b"), s("c")], "en");
        let ids: Vec<&str> = batches
            .iter()
            .flat_map(|b| b.sentences.iter().map(|s| s.sentence_id.as_str()))
            .collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }

    #[test]
    fn translate_phase_dirs() {
        assert_eq!(TranslatePhase::Translating.dir(), "translating");
        assert_eq!(TranslatePhase::Splitting.dir(), "splitting");
    }

    #[test]
    fn staleness_tracks_dirty_groups() {
        let mut st = TranslateStaleness::default();
        assert!(!st.is_stale("g1"));
        st.mark_stale("g1");
        assert!(st.is_stale("g1"));
        assert!(!st.is_empty());
    }
}
