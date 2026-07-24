//! Crash-safe file persistence for authoritative project and job state.
//!
//! A temporary file is fully written and synced before an atomic rename. On
//! Unix the containing directory is synced as well, so a normal quit, crash,
//! or sudden restart cannot expose a half-written JSON document.

use std::io::{Read, Write};
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

/// Atomically replace `target` with a streaming copy of `source`.
///
/// Unlike `read` followed by `write`, peak memory stays constant for large
/// transcript and cue files.
pub fn copy(source: &Path, target: &Path) -> AppResult<()> {
    let parent = target
        .parent()
        .ok_or_else(|| AppError::Schema("persistent file has no parent directory".into()))?;
    std::fs::create_dir_all(parent)?;
    let file_name = target
        .file_name()
        .ok_or_else(|| AppError::Schema("persistent file has no name".into()))?
        .to_string_lossy();
    let temporary = parent.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));

    let result = (|| -> AppResult<()> {
        let mut input = std::io::BufReader::new(std::fs::File::open(source)?);
        let mut output = std::io::BufWriter::new(
            std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?,
        );
        std::io::copy(&mut input, &mut output)?;
        output.flush()?;
        output.get_ref().sync_all()?;
        drop(output);
        std::fs::rename(&temporary, target)?;
        #[cfg(unix)]
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Create an immutable snapshot efficiently. APFS clone files share unchanged
/// blocks copy-on-write; all other filesystems fall back to a streaming kernel
/// copy without loading the file into process memory.
pub fn clone_or_copy(source: &Path, target: &Path) -> AppResult<()> {
    let parent = target
        .parent()
        .ok_or_else(|| AppError::Schema("snapshot file has no parent directory".into()))?;
    std::fs::create_dir_all(parent)?;
    #[cfg(target_os = "macos")]
    if try_clonefile(source, target) {
        return Ok(());
    }
    std::fs::copy(source, target)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn try_clonefile(source: &Path, target: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    unsafe extern "C" {
        fn clonefile(
            source: *const std::os::raw::c_char,
            target: *const std::os::raw::c_char,
            flags: u32,
        ) -> std::os::raw::c_int;
    }

    let Ok(source) = CString::new(source.as_os_str().as_bytes()) else {
        return false;
    };
    let Ok(target) = CString::new(target.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: both C strings are NUL-terminated and live for the duration of
    // the call; flags=0 is the documented clonefile default.
    unsafe { clonefile(source.as_ptr(), target.as_ptr(), 0) == 0 }
}

/// Compare two files with a fixed-size buffer instead of allocating both.
pub fn files_equal(left: &Path, right: &Path) -> AppResult<bool> {
    if std::fs::metadata(left)?.len() != std::fs::metadata(right)?.len() {
        return Ok(false);
    }
    let mut left = std::io::BufReader::new(std::fs::File::open(left)?);
    let mut right = std::io::BufReader::new(std::fs::File::open(right)?);
    let mut left_buffer = [0_u8; 64 * 1024];
    let mut right_buffer = [0_u8; 64 * 1024];
    loop {
        let left_read = left.read(&mut left_buffer)?;
        let right_read = right.read(&mut right_buffer)?;
        if left_read != right_read || left_buffer[..left_read] != right_buffer[..right_read] {
            return Ok(false);
        }
        if left_read == 0 {
            return Ok(true);
        }
    }
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

    #[test]
    fn streaming_copy_clone_and_compare_handle_large_files_without_changing_content() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.bin");
        let snapshot = temp.path().join("snapshot.bin");
        let restored = temp.path().join("nested/restored.bin");
        let mut bytes = vec![0_u8; 2 * 1024 * 1024];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = (index % 251) as u8;
        }
        write(&source, &bytes).unwrap();
        clone_or_copy(&source, &snapshot).unwrap();
        write(&source, b"replacement source").unwrap();
        copy(&snapshot, &restored).unwrap();
        assert!(!files_equal(&source, &snapshot).unwrap());
        assert!(files_equal(&snapshot, &restored).unwrap());
        assert_eq!(
            std::fs::metadata(&restored).unwrap().len(),
            bytes.len() as u64
        );

        std::fs::OpenOptions::new()
            .append(true)
            .open(&restored)
            .unwrap()
            .write_all(b"different")
            .unwrap();
        assert!(!files_equal(&snapshot, &restored).unwrap());
    }
}
