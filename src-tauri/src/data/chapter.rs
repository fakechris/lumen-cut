//! Editable chapter markers derived from transcript cue boundaries.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::data::Doc;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chapter {
    pub title: String,
    pub start_seg: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterRow {
    pub title: String,
    pub start_seg: String,
    pub start: f64,
    pub end: f64,
    pub preview: String,
}

pub fn load(project_dir: &Path) -> AppResult<Vec<Chapter>> {
    let path = project_dir.join("chapters.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&std::fs::read_to_string(path)?)
        .map_err(|error| AppError::Schema(format!("invalid chapters.json: {error}")))
}

fn normalize(doc: &Doc, chapters: Vec<Chapter>) -> AppResult<Vec<Chapter>> {
    let sentences = doc
        .paragraphs
        .iter()
        .flat_map(|paragraph| paragraph.sentences.iter())
        .collect::<Vec<_>>();
    let order = sentences
        .iter()
        .enumerate()
        .map(|(index, sentence)| (sentence.id.as_str(), index))
        .collect::<std::collections::HashMap<_, _>>();

    let mut normalized = Vec::with_capacity(chapters.len());
    let mut previous = None;
    for (index, chapter) in chapters.into_iter().enumerate() {
        let title = chapter.title.trim();
        if title.is_empty() {
            return Err(AppError::Schema(format!(
                "chapter {} has an empty title",
                index + 1
            )));
        }
        if title.chars().count() > 200 {
            return Err(AppError::Schema(format!(
                "chapter {} title exceeds 200 characters",
                index + 1
            )));
        }
        let rank = *order.get(chapter.start_seg.as_str()).ok_or_else(|| {
            AppError::Schema(format!(
                "chapter {} references unknown cue `{}`",
                index + 1,
                chapter.start_seg
            ))
        })?;
        if index == 0 && rank != 0 {
            return Err(AppError::Schema(
                "the first chapter must start at the first cue".into(),
            ));
        }
        if previous.is_some_and(|last| rank <= last) {
            return Err(AppError::Schema(
                "chapter starts must be unique and follow transcript order".into(),
            ));
        }
        previous = Some(rank);
        normalized.push(Chapter {
            title: title.to_owned(),
            start_seg: chapter.start_seg,
        });
    }
    Ok(normalized)
}

pub fn rows(doc: &Doc, chapters: &[Chapter]) -> AppResult<Vec<ChapterRow>> {
    let normalized = normalize(doc, chapters.to_vec())?;
    let sentences = doc
        .paragraphs
        .iter()
        .flat_map(|paragraph| paragraph.sentences.iter())
        .collect::<Vec<_>>();
    let by_id = sentences
        .iter()
        .map(|sentence| (sentence.id.as_str(), *sentence))
        .collect::<std::collections::HashMap<_, _>>();

    Ok(normalized
        .iter()
        .enumerate()
        .filter_map(|(index, chapter)| {
            let sentence = by_id.get(chapter.start_seg.as_str())?;
            let start = sentence
                .words
                .first()
                .map(|word| word.start)
                .unwrap_or_default();
            let end = normalized
                .get(index + 1)
                .and_then(|next| by_id.get(next.start_seg.as_str()))
                .and_then(|next| next.words.first())
                .map(|word| word.start)
                .unwrap_or(doc.media.duration_seconds);
            Some(ChapterRow {
                title: chapter.title.clone(),
                start_seg: chapter.start_seg.clone(),
                start,
                end: end.max(start),
                preview: sentence.text.clone(),
            })
        })
        .collect())
}

fn restore(path: &Path, previous: Option<&[u8]>) -> AppResult<()> {
    if let Some(bytes) = previous {
        crate::data::storage::write(path, bytes)
    } else if path.exists() {
        std::fs::remove_file(path)?;
        Ok(())
    } else {
        Ok(())
    }
}

pub fn replace(project_dir: &Path, doc: &Doc, chapters: Vec<Chapter>) -> AppResult<bool> {
    let chapters = normalize(doc, chapters)?;
    let current = load(project_dir)?;
    if current == chapters {
        return Ok(false);
    }

    let chapters_path = project_dir.join("chapters.json");
    let doc_path = project_dir.join("doc.json");
    let previous_chapters = std::fs::read(&chapters_path).ok();
    let previous_doc = std::fs::read(&doc_path)?;
    let mut native: serde_json::Value = serde_json::from_slice(&previous_doc)?;
    native
        .as_object_mut()
        .ok_or_else(|| AppError::Schema("doc.json must be an object".into()))?
        .insert("chapters".into(), serde_json::to_value(&chapters)?);

    crate::data::storage::write_json(&chapters_path, &chapters)?;
    if let Err(error) = crate::data::storage::write_json(&doc_path, &native) {
        if let Err(restore_error) = restore(&chapters_path, previous_chapters.as_deref()) {
            return Err(AppError::Schema(format!(
                "could not save chapters: {error}; rollback also failed: {restore_error}"
            )));
        }
        return Err(error);
    }
    Ok(true)
}
