//! `doc.json` schema + reindex helpers + soft-cut + version control.

pub mod broll;
pub mod cues;
pub mod doc;
pub mod edit;
pub mod modelconfig;
pub mod rebind;
pub mod reindex;
pub mod soft_cut;
pub mod speakers;
pub mod storage;
pub mod substyle;
pub mod subtitle;
pub mod version;

pub use doc::{Doc, MediaRef, Meta, Paragraph, Sentence, TranslationGroup, Word};
pub use rebind::rebind_corrected;
pub use reindex::{reindex_words, ReindexMap};
pub use soft_cut::{kept_spans, ClipCuts, Cut, CutKind, KeptSpan};
pub use version::{
    three_way_merge, CueDiff, Lineage, MergeConflict, MergeResult, VersionKind, VersionNode,
};
