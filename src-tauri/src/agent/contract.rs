//! Runtime task-spec materialisation.
//!
//! Each supported background task embeds a concise response specification.
//! Workers receive that specification alongside the project payload, while
//! Rust validators remain the authoritative acceptance boundary.

/// The response specification for a task kind, or `None` for unsupported
/// kinds.
pub fn contract_for_kind(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "translate" => include_str!("../../../task-specs/translate.md"),
        "align" => include_str!("../../../task-specs/align.md"),
        "polish" => include_str!("../../../task-specs/polish.md"),
        "cleanup" => include_str!("../../../task-specs/cleanup.md"),
        "broll" => include_str!("../../../task-specs/broll.md"),
        "segment" => include_str!("../../../task-specs/segment.md"),
        "repunct" => include_str!("../../../task-specs/repunct.md"),
        "chapters" => include_str!("../../../task-specs/chapters.md"),
        _ => return None,
    })
}

/// The task-spec filename materialised in a project's AI work directory.
pub fn contract_filename(kind: &str) -> String {
    format!("{kind}.md")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_kinds_have_contracts() {
        for kind in [
            "translate",
            "align",
            "polish",
            "cleanup",
            "broll",
            "segment",
            "repunct",
            "chapters",
        ] {
            let body = contract_for_kind(kind).unwrap_or_else(|| panic!("missing {kind}"));
            assert!(!body.is_empty(), "{kind} contract empty");
        }
    }

    #[test]
    fn unknown_kind_has_no_contract() {
        assert!(contract_for_kind("nope").is_none());
    }

    #[test]
    fn translate_contract_mentions_answer_shape() {
        let body = contract_for_kind("translate").unwrap();
        assert!(body.to_lowercase().contains("translation") || body.contains("translations"));
    }

    #[test]
    fn contract_filename_is_kind_md() {
        assert_eq!(contract_filename("translate"), "translate.md");
    }
}
