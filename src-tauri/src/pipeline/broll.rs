//! B-roll suggestion artifact and structural lint.
//!
//! `BrollEngine.suggest` asks the LLM for suggestion rows, then validates
//! those rows. It does not pre-classify transcript sentences with keyword
//! heuristics. Structural problems use a stable `{loc, problem}` shape.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::data::doc::Doc;
use crate::error::{AppError, AppResult};

/// B-roll placement mode from the LLM contract.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BrollMode {
    Fullscreen,
    Pip,
}

/// One LLM suggestion. Accepted/renderable B-roll is stored separately as a
/// `BrollPlacement`; suggestions never mutate the document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrollSuggestion {
    pub start: String,
    pub end: String,
    pub mode: BrollMode,
    pub query: String,
    pub reason: String,
}

/// The artifact written to `ai/broll-suggestions.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct BrollSuggestionsArtifact {
    #[serde(default)]
    pub suggestions: Vec<BrollSuggestion>,
}

/// A location and human-readable structural problem.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuralProblem {
    pub loc: String,
    pub problem: String,
}

/// Maximum number of B-roll suggestions accepted for one project.
pub const MAX_BROLL_SUGGESTIONS: usize = 8;

/// Read the validated LLM artifact. A project that has not run the B-roll
/// stage has no suggestions rather than fabricated keyword matches.
pub fn load_artifact(project_dir: &Path) -> AppResult<Vec<BrollSuggestion>> {
    let path = project_dir.join("ai").join("broll-suggestions.json");
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let artifact: BrollSuggestionsArtifact = serde_json::from_str(&raw)?;
    if artifact.suggestions.len() > MAX_BROLL_SUGGESTIONS {
        return Err(AppError::Schema(format!(
            "{} carries {} B-roll suggestions; maximum is {}",
            path.display(),
            artifact.suggestions.len(),
            MAX_BROLL_SUGGESTIONS
        )));
    }
    Ok(artifact.suggestions)
}

/// Validate typed suggestion rows against timeline boundaries. `existing`
/// contains already accepted B-roll spans in source seconds.
pub fn lint(
    doc: &Doc,
    suggestions: &[BrollSuggestion],
    existing: &[(f64, f64)],
) -> Vec<StructuralProblem> {
    let word_at: BTreeMap<&str, (usize, f64, f64)> = doc
        .all_words()
        .into_iter()
        .enumerate()
        .map(|(index, word)| (word.id.as_str(), (index, word.start, word.end)))
        .collect();
    let media_end = doc.media.duration_seconds;
    let mut occupied = existing.to_vec();
    let mut problems = Vec::new();

    if suggestions.len() > MAX_BROLL_SUGGESTIONS {
        problems.push(StructuralProblem {
            loc: "answer".into(),
            problem: format!(
                "answer carries {} suggestions — the cap is {}",
                suggestions.len(),
                MAX_BROLL_SUGGESTIONS
            ),
        });
    }

    for (index, suggestion) in suggestions.iter().enumerate() {
        let loc = format!("suggestions[{}]", index + 1);
        if suggestion.query.trim().is_empty() {
            problems.push(StructuralProblem {
                loc: loc.clone(),
                problem: "empty query".into(),
            });
        }
        if suggestion.reason.trim().is_empty() {
            problems.push(StructuralProblem {
                loc: loc.clone(),
                problem: "empty reason".into(),
            });
        }

        let Some(&(start_index, start, _)) = word_at.get(suggestion.start.as_str()) else {
            problems.push(StructuralProblem {
                loc: loc.clone(),
                problem: format!(
                    "unknown word id '{}' — copy ids verbatim from the payload",
                    suggestion.start
                ),
            });
            continue;
        };
        let Some(&(end_index, _, end)) = word_at.get(suggestion.end.as_str()) else {
            problems.push(StructuralProblem {
                loc: loc.clone(),
                problem: format!(
                    "unknown word id '{}' — copy ids verbatim from the payload",
                    suggestion.end
                ),
            });
            continue;
        };
        if end_index < start_index || end <= start {
            problems.push(StructuralProblem {
                loc,
                problem: "end word precedes start word".into(),
            });
            continue;
        }

        let span = end - start;
        if !(1.5..=20.0).contains(&span) {
            problems.push(StructuralProblem {
                loc: loc.clone(),
                problem: format!("span {span:.1}s is outside 1.5–20s"),
            });
        }
        if start < 3.0 || (media_end > 0.0 && end > media_end - 3.0) {
            problems.push(StructuralProblem {
                loc: loc.clone(),
                problem: "span intrudes into the first/last 3s of the video".into(),
            });
        }
        if occupied
            .iter()
            .any(|(other_start, other_end)| start < *other_end && *other_start < end)
        {
            problems.push(StructuralProblem {
                loc,
                problem: "span overlaps another suggestion or existing B-roll".into(),
            });
        }
        occupied.push((start, end));
    }

    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::doc::{MediaRef, Meta, Paragraph, Sentence, Word};
    use chrono::Utc;

    fn doc() -> Doc {
        Doc {
            id: "p".into(),
            schema: 1,
            media: MediaRef {
                path: "/tmp/x.mp4".into(),
                duration_seconds: 60.0,
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
                    text: "one two three four".into(),
                    words: vec![
                        ("w0", 1.0, 2.0),
                        ("w1", 5.0, 8.0),
                        ("w2", 7.0, 12.0),
                        ("w3", 58.0, 59.0),
                    ]
                    .into_iter()
                    .map(|(id, start, end)| Word {
                        id: id.into(),
                        text: id.into(),
                        start,
                        end,
                    })
                    .collect(),
                }],
            }],
            translations: Default::default(),
        }
    }

    fn suggestion(start: &str, end: &str) -> BrollSuggestion {
        BrollSuggestion {
            start: start.into(),
            end: end.into(),
            mode: BrollMode::Fullscreen,
            query: "keyboard close-up".into(),
            reason: "illustrates the claim".into(),
        }
    }

    #[test]
    fn structural_problem_is_a_two_string_shape() {
        let json = serde_json::to_string(&StructuralProblem {
            loc: "suggestions[1]".into(),
            problem: "empty query".into(),
        })
        .unwrap();
        assert_eq!(json, r#"{"loc":"suggestions[1]","problem":"empty query"}"#);
    }

    #[test]
    fn lint_reports_content_word_span_edge_and_overlap_problems() {
        let mut empty = suggestion("w1", "w1");
        empty.query.clear();
        empty.reason.clear();
        let rows = vec![
            empty,
            suggestion("missing", "w1"),
            suggestion("w2", "w1"),
            suggestion("w0", "w0"),
            suggestion("w1", "w2"),
        ];
        let problems = lint(&doc(), &rows, &[(6.0, 9.0)]);
        let messages: Vec<&str> = problems
            .iter()
            .map(|problem| problem.problem.as_str())
            .collect();
        assert!(messages.contains(&"empty query"));
        assert!(messages.contains(&"empty reason"));
        assert!(messages
            .iter()
            .any(|message| message.starts_with("unknown word id")));
        assert!(messages.contains(&"end word precedes start word"));
        assert!(messages
            .iter()
            .any(|message| message.contains("outside 1.5–20s")));
        assert!(messages.contains(&"span intrudes into the first/last 3s of the video"));
        assert!(messages.contains(&"span overlaps another suggestion or existing B-roll"));
    }

    #[test]
    fn artifact_loader_returns_llm_rows_and_missing_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(load_artifact(tmp.path()).unwrap().is_empty());
        std::fs::create_dir_all(tmp.path().join("ai")).unwrap();
        std::fs::write(
            tmp.path().join("ai/broll-suggestions.json"),
            r#"{"suggestions":[{"start":"w1","end":"w2","mode":"pip","query":"q","reason":"r"}]}"#,
        )
        .unwrap();
        let rows = load_artifact(tmp.path()).unwrap();
        assert_eq!(
            rows,
            vec![BrollSuggestion {
                start: "w1".into(),
                end: "w2".into(),
                mode: BrollMode::Pip,
                query: "q".into(),
                reason: "r".into(),
            }]
        );
    }
}
