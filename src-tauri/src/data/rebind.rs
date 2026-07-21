//! Word-preserving transcript correction.
//!
//! Unchanged words retain their ids and timing, while corrected words are
//! interpolated only inside the surrounding anchor gap.

use crate::data::doc::Word;

#[derive(Debug)]
struct Token {
    text: String,
    anchor: Option<usize>,
}

/// Rebind a corrected sentence onto its existing timed words.
pub fn rebind_corrected(words: &[Word], corrected: &str) -> Vec<Word> {
    let corrected = corrected.trim();
    if corrected.is_empty() || words.is_empty() {
        return Vec::new();
    }
    let cjk = corrected.chars().any(is_cjk);
    let old_surface = if cjk {
        words.iter().map(|word| word.text.as_str()).collect()
    } else {
        words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };
    if old_surface == corrected {
        return words.to_vec();
    }

    let tokens = if cjk {
        cjk_tokens(words, corrected)
    } else {
        latin_tokens(words, corrected)
    };
    materialize(words, tokens)
}

fn latin_tokens(words: &[Word], corrected: &str) -> Vec<Token> {
    let target: Vec<&str> = corrected.split_whitespace().collect();
    let source: Vec<&str> = words.iter().map(|word| word.text.as_str()).collect();
    let anchors = lcs_anchors(&source, &target);
    target
        .into_iter()
        .enumerate()
        .map(|(index, text)| Token {
            text: text.to_string(),
            anchor: anchors
                .iter()
                .find_map(|(source, target)| (*target == index).then_some(*source)),
        })
        .collect()
}

fn cjk_tokens(words: &[Word], corrected: &str) -> Vec<Token> {
    let chars: Vec<char> = corrected.chars().filter(|ch| !ch.is_whitespace()).collect();
    let mut tokens = Vec::new();
    let mut cursor = 0;
    let mut source_cursor = 0;

    while cursor < chars.len() {
        let mut anchor = None;
        for (index, word) in words.iter().enumerate().skip(source_cursor) {
            let needle: Vec<char> = word.text.chars().filter(|ch| !ch.is_whitespace()).collect();
            if !needle.is_empty() && chars[cursor..].starts_with(&needle) {
                anchor = Some((index, needle.len(), word.text.clone()));
                break;
            }
        }
        if let Some((index, width, text)) = anchor {
            tokens.push(Token {
                text,
                anchor: Some(index),
            });
            source_cursor = index + 1;
            cursor += width;
        } else {
            tokens.push(Token {
                text: chars[cursor].to_string(),
                anchor: None,
            });
            cursor += 1;
        }
    }
    tokens
}

fn lcs_anchors(source: &[&str], target: &[&str]) -> Vec<(usize, usize)> {
    let mut dp = vec![vec![0usize; target.len() + 1]; source.len() + 1];
    for i in (0..source.len()).rev() {
        for j in (0..target.len()).rev() {
            dp[i][j] = if source[i] == target[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut anchors = Vec::new();
    while i < source.len() && j < target.len() {
        if source[i] == target[j] {
            anchors.push((i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    anchors
}

fn materialize(words: &[Word], tokens: Vec<Token>) -> Vec<Word> {
    let mut output: Vec<Option<Word>> = tokens
        .iter()
        .map(|token| {
            token.anchor.map(|index| {
                let mut word = words[index].clone();
                word.text.clone_from(&token.text);
                word
            })
        })
        .collect();
    let mut cursor = 0;
    let mut serial = 0;
    while cursor < output.len() {
        if output[cursor].is_some() {
            cursor += 1;
            continue;
        }
        let start_index = cursor;
        while cursor < output.len() && output[cursor].is_none() {
            cursor += 1;
        }
        let end_index = cursor;
        let left = output[..start_index]
            .iter()
            .rev()
            .flatten()
            .next()
            .map(|word| word.end)
            .unwrap_or(words[0].start);
        let right = output[end_index..]
            .iter()
            .flatten()
            .next()
            .map(|word| word.start)
            .unwrap_or_else(|| words.last().expect("non-empty").end)
            .max(left);
        let step = (right - left) / (end_index - start_index) as f64;
        for index in start_index..end_index {
            let start = left + step * (index - start_index) as f64;
            let end = if index + 1 == end_index {
                right
            } else {
                start + step
            };
            output[index] = Some(Word {
                id: format!("{}-r{serial}", words[0].id),
                text: tokens[index].text.clone(),
                start,
                end,
            });
            serial += 1;
        }
    }
    output.into_iter().flatten().collect()
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4dbf
            | 0x4e00..=0x9fff
            | 0x3040..=0x30ff
            | 0xac00..=0xd7af
            | 0xf900..=0xfaff
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(values: &[(&str, &str, f64, f64)]) -> Vec<Word> {
        values
            .iter()
            .map(|(id, text, start, end)| Word {
                id: (*id).into(),
                text: (*text).into(),
                start: *start,
                end: *end,
            })
            .collect()
    }

    #[test]
    fn latin_passthrough_keeps_every_word() {
        let source = words(&[("w0", "hello", 0.0, 0.4), ("w1", "world", 0.6, 1.0)]);
        assert_eq!(rebind_corrected(&source, "hello world"), source);
    }

    #[test]
    fn homophone_fix_only_rebinds_changed_word() {
        let source = words(&[
            ("w0", "there", 0.0, 0.4),
            ("w1", "site", 0.6, 1.0),
            ("w2", "works", 1.2, 1.6),
        ]);
        let rebound = rebind_corrected(&source, "there sight works");
        assert_eq!(rebound[0], source[0]);
        assert_eq!(rebound[2], source[2]);
        assert_ne!(rebound[1].id, source[1].id);
        assert_eq!((rebound[1].start, rebound[1].end), (0.4, 1.2));
    }

    #[test]
    fn corrected_cjk_tail_interpolates_after_unchanged_anchor() {
        let source = words(&[("w0", "你", 0.0, 0.4), ("w1", "好", 0.5, 0.9)]);
        let rebound = rebind_corrected(&source, "你号");
        assert_eq!(rebound[0], source[0]);
        assert_ne!(rebound[1].id, source[1].id);
        assert_eq!((rebound[1].start, rebound[1].end), (0.4, 0.9));
    }
}
