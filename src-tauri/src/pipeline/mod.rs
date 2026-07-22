//! Pipelines — translate, align, cleanup, broll, polish, transcribe.

pub mod align;
pub mod broll;
pub mod cleanup;
pub mod polish;
pub mod timing;
pub mod transcribe;
pub mod translate;

pub use align::{
    align_list, align_targeted, fingerprint_words, project_end_times, AlignCandidate, AlignList,
    AlignOutcome, AlignSpec, RebindReadingMerge, RebindSeam, TranslateRebindArtifact,
};
pub use broll::{
    lint as lint_broll, load_artifact as load_broll_suggestions, BrollMode, BrollSuggestion,
    BrollSuggestionsArtifact, StructuralProblem,
};
pub use cleanup::{apply, cut_from_hit, detect, CleanupHit, CleanupKind};
pub use polish::apply_polish;
pub use transcribe::re_transcribe;
pub use translate::{
    aim_chars_for_lang, hard_chars_for_lang, pack, pack_by_chars, pack_for_requests,
    pack_with_lang, tokens_per_word_for_lang, BriefResult, SentencePacket, TranslateBatch,
    TranslateStaleness, DEFAULT_BUDGET, MAX_LINES_PER_REQUEST, REQUEST_OVERHEAD_BUDGET,
};
