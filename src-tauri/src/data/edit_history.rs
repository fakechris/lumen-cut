//! Durable, bounded undo/redo history for interactive editor mutations.
//!
//! The public version graph is intentionally separate: versions are named
//! recovery points, while this journal stores short-lived command snapshots
//! for the editor toolbar. Snapshots cover every authoritative file that can
//! affect the visible timeline so undo never restores only half of an edit.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

const HISTORY_VERSION: u32 = 1;
const MAX_ENTRIES: usize = 100;
const TRACKED_FILES: &[&str] = &[
    "doc.json",
    "cues.json",
    "hidden.json",
    "cuts.json",
    "broll.json",
    "style.json",
    "titles.json",
    "audio-mix.json",
    "chapters.json",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditEntry {
    id: String,
    label: String,
    at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditHistory {
    #[serde(default = "history_version")]
    version: u32,
    #[serde(default)]
    cursor: usize,
    #[serde(default)]
    entries: Vec<EditEntry>,
}

impl Default for EditHistory {
    fn default() -> Self {
        Self {
            version: HISTORY_VERSION,
            cursor: 0,
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditHistoryStatus {
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditHistoryAction {
    pub changed: bool,
    pub status: EditHistoryStatus,
}

const fn history_version() -> u32 {
    HISTORY_VERSION
}

fn root(dir: &Path) -> PathBuf {
    dir.join(".lumen-cut").join("edit-history")
}

fn manifest_path(dir: &Path) -> PathBuf {
    root(dir).join("history.json")
}

fn entry_path(dir: &Path, id: &str) -> PathBuf {
    root(dir).join("entries").join(id)
}

fn load(dir: &Path) -> AppResult<EditHistory> {
    let path = manifest_path(dir);
    if !path.exists() {
        return Ok(EditHistory::default());
    }
    let mut history: EditHistory = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    if history.version != HISTORY_VERSION {
        return Err(AppError::Schema(format!(
            "unsupported edit history version {}",
            history.version
        )));
    }
    history.cursor = history.cursor.min(history.entries.len());
    Ok(history)
}

fn save(dir: &Path, history: &EditHistory) -> AppResult<()> {
    crate::data::storage::write_json(&manifest_path(dir), history)
}

fn snapshot(dir: &Path, target: &Path) -> AppResult<()> {
    std::fs::create_dir_all(target)?;
    for name in TRACKED_FILES {
        let source = dir.join(name);
        if source.exists() {
            crate::data::storage::clone_or_copy(&source, &target.join(name))?;
        }
    }
    Ok(())
}

fn restore(dir: &Path, source: &Path) -> AppResult<()> {
    for name in TRACKED_FILES {
        let snapshot = source.join(name);
        let target = dir.join(name);
        if snapshot.exists() {
            crate::data::storage::copy(&snapshot, &target)?;
        } else if target.exists() {
            std::fs::remove_file(target)?;
        }
    }
    Ok(())
}

fn matches(dir: &Path, snapshot: &Path) -> AppResult<bool> {
    for name in TRACKED_FILES {
        let current = dir.join(name);
        let expected = snapshot.join(name);
        if current.exists() != expected.exists() {
            return Ok(false);
        }
        if current.exists() && !crate::data::storage::files_equal(&current, &expected)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn remove_entry(dir: &Path, id: &str) {
    let _ = std::fs::remove_dir_all(entry_path(dir, id));
}

fn clear_entries(dir: &Path, history: &mut EditHistory) {
    for entry in history.entries.drain(..) {
        remove_entry(dir, &entry.id);
    }
    history.cursor = 0;
}

fn current_matches(dir: &Path, history: &EditHistory) -> AppResult<bool> {
    if history.entries.is_empty() {
        return Ok(true);
    }
    let snapshot = if history.cursor == 0 {
        let entry = &history.entries[0];
        entry_path(dir, &entry.id).join("before")
    } else {
        let entry = &history.entries[history.cursor - 1];
        entry_path(dir, &entry.id).join("after")
    };
    matches(dir, &snapshot)
}

fn status_for(history: &EditHistory) -> EditHistoryStatus {
    EditHistoryStatus {
        can_undo: history.cursor > 0,
        can_redo: history.cursor < history.entries.len(),
        undo_label: history
            .cursor
            .checked_sub(1)
            .and_then(|index| history.entries.get(index))
            .map(|entry| entry.label.clone()),
        redo_label: history
            .entries
            .get(history.cursor)
            .map(|entry| entry.label.clone()),
    }
}

pub fn status(dir: &Path) -> AppResult<EditHistoryStatus> {
    let history = load(dir)?;
    if !current_matches(dir, &history)? {
        return Ok(status_for(&EditHistory::default()));
    }
    Ok(status_for(&history))
}

/// Run one editor mutation and journal it only when `changed` returns true.
/// The mutation itself remains responsible for crash-safe writes.
pub fn record<T>(
    dir: &Path,
    label: &str,
    mutate: impl FnOnce() -> AppResult<T>,
    changed: impl FnOnce(&T) -> bool,
) -> AppResult<T> {
    let mut history = load(dir)?;
    if !current_matches(dir, &history)? {
        clear_entries(dir, &mut history);
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    let path = entry_path(dir, &id);
    snapshot(dir, &path.join("before"))?;

    let result = match mutate() {
        Ok(result) => result,
        Err(error) => {
            remove_entry(dir, &id);
            return Err(error);
        }
    };
    if !changed(&result) {
        remove_entry(dir, &id);
        return Ok(result);
    }

    snapshot(dir, &path.join("after"))?;

    for entry in history.entries.drain(history.cursor..) {
        remove_entry(dir, &entry.id);
    }
    history.entries.push(EditEntry {
        id,
        label: label.trim().to_owned(),
        at: chrono::Utc::now(),
    });
    history.cursor = history.entries.len();
    while history.entries.len() > MAX_ENTRIES {
        let removed = history.entries.remove(0);
        remove_entry(dir, &removed.id);
        history.cursor = history.cursor.saturating_sub(1);
    }
    save(dir, &history)?;
    crate::data::activity::touch(dir)?;
    Ok(result)
}

pub fn undo(dir: &Path) -> AppResult<EditHistoryAction> {
    let mut history = load(dir)?;
    if history.cursor == 0 {
        return Ok(EditHistoryAction {
            changed: false,
            status: status_for(&history),
        });
    }
    let entry = &history.entries[history.cursor - 1];
    let path = entry_path(dir, &entry.id);
    if !matches(dir, &path.join("after"))? {
        return Err(AppError::Schema(
            "the project changed outside the editor history; reload before undoing".into(),
        ));
    }
    restore(dir, &path.join("before"))?;
    history.cursor -= 1;
    save(dir, &history)?;
    crate::data::activity::touch(dir)?;
    Ok(EditHistoryAction {
        changed: true,
        status: status_for(&history),
    })
}

pub fn redo(dir: &Path) -> AppResult<EditHistoryAction> {
    let mut history = load(dir)?;
    if history.cursor >= history.entries.len() {
        return Ok(EditHistoryAction {
            changed: false,
            status: status_for(&history),
        });
    }
    let entry = &history.entries[history.cursor];
    let path = entry_path(dir, &entry.id);
    if !matches(dir, &path.join("before"))? {
        return Err(AppError::Schema(
            "the project changed outside the editor history; reload before redoing".into(),
        ));
    }
    restore(dir, &path.join("after"))?;
    history.cursor += 1;
    save(dir, &history)?;
    crate::data::activity::touch(dir)?;
    Ok(EditHistoryAction {
        changed: true,
        status: status_for(&history),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_survives_a_reload_and_truncates_the_redo_branch() {
        let project = tempfile::tempdir().unwrap();
        crate::data::storage::write(&project.path().join("doc.json"), b"one").unwrap();

        record(
            project.path(),
            "Edit transcript",
            || {
                crate::data::storage::write(&project.path().join("doc.json"), b"two")?;
                Ok(true)
            },
            |changed| *changed,
        )
        .unwrap();
        assert!(status(project.path()).unwrap().can_undo);

        let undone = undo(project.path()).unwrap();
        assert!(undone.changed);
        assert_eq!(
            std::fs::read(project.path().join("doc.json")).unwrap(),
            b"one"
        );
        assert!(status(project.path()).unwrap().can_redo);

        let redone = redo(project.path()).unwrap();
        assert!(redone.changed);
        assert_eq!(
            std::fs::read(project.path().join("doc.json")).unwrap(),
            b"two"
        );

        undo(project.path()).unwrap();
        record(
            project.path(),
            "Replacement edit",
            || {
                crate::data::storage::write(&project.path().join("doc.json"), b"three")?;
                Ok(true)
            },
            |changed| *changed,
        )
        .unwrap();
        assert!(!status(project.path()).unwrap().can_redo);
        assert_eq!(
            status(project.path()).unwrap().undo_label.as_deref(),
            Some("Replacement edit")
        );
    }

    #[test]
    fn undo_refuses_to_overwrite_an_external_change() {
        let project = tempfile::tempdir().unwrap();
        crate::data::storage::write(&project.path().join("doc.json"), b"one").unwrap();
        record(
            project.path(),
            "Edit",
            || {
                crate::data::storage::write(&project.path().join("doc.json"), b"two")?;
                Ok(true)
            },
            |changed| *changed,
        )
        .unwrap();
        crate::data::storage::write(&project.path().join("doc.json"), b"external").unwrap();

        assert!(!status(project.path()).unwrap().can_undo);
        assert!(undo(project.path()).is_err());
        assert_eq!(
            std::fs::read(project.path().join("doc.json")).unwrap(),
            b"external"
        );
    }
}
