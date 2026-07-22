//! Crash-safe file persistence for authoritative project and job state.
//!
//! A temporary file is fully written and synced before an atomic rename. On
//! Unix the containing directory is synced as well, so a normal quit, crash,
//! or sudden restart cannot expose a half-written JSON document.

use std::io::Write;
use std::path::Path;

use serde::Serialize;

use crate::error::{AppError, AppResult};

pub fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> AppResult<()> {
    write(path, serde_json::to_vec_pretty(value)?.as_slice())
}

pub fn write(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::Schema("persistent file has no parent directory".into()))?;
    std::fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .ok_or_else(|| AppError::Schema("persistent file has no name".into()))?
        .to_string_lossy();
    let temporary = parent.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));

    let result = (|| -> AppResult<()> {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        #[cfg(unix)]
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_json_replaces_the_complete_value_without_leaving_temp_files() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.json");
        write_json(&path, &serde_json::json!({"state": "running"})).unwrap();
        write_json(
            &path,
            &serde_json::json!({"state": "completed", "result": [1, 2, 3]}),
        )
        .unwrap();

        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(value["state"], "completed");
        assert_eq!(value["result"], serde_json::json!([1, 2, 3]));
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_atomic_replace_cleans_up_its_temporary_file() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("state.json");
        std::fs::create_dir(&target).unwrap();

        assert!(write_json(&target, &serde_json::json!({"state": "completed"})).is_err());
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
        assert!(target.is_dir());
    }
}
