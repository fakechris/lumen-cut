//! Per-platform user directories.
//!
//! Every path the app owns outside a project folder resolves here, so the
//! Unix `$HOME/.lumen-cut` convention and the Windows `%LOCALAPPDATA%` Known
//! Folder convention stay in one place instead of spreading `#[cfg]` blocks
//! through the command layer.

use std::path::{Path, PathBuf};

/// Overrides [`state_dir`] wholesale. Used by tests and by anyone relocating
/// application state off the system drive.
pub const ENV_STATE_DIR: &str = "LUMEN_CUT_STATE_DIR";

/// Read `key` as a path, treating unset, empty and whitespace-only as absent.
fn nonempty_env_path(key: &str) -> Option<PathBuf> {
    let value = std::env::var_os(key)?;
    match value.to_str() {
        Some(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
        }
        None => (!value.is_empty()).then(|| PathBuf::from(value)),
    }
}

/// User home directory: `HOME` → `USERPROFILE` → `HOMEDRIVE`+`HOMEPATH` →
/// the system temp dir as a last resort.
///
/// `HOME` is checked first on every platform so the Hugging Face cache lookup
/// agrees with `huggingface_hub`, which does the same.
pub fn home_dir() -> PathBuf {
    for key in ["HOME", "USERPROFILE"] {
        if let Some(path) = nonempty_env_path(key) {
            return path;
        }
    }
    match (std::env::var_os("HOMEDRIVE"), std::env::var_os("HOMEPATH")) {
        (Some(drive), Some(path)) if !drive.is_empty() && !path.is_empty() => {
            let mut home = PathBuf::from(drive);
            home.push(path);
            home
        }
        _ => std::env::temp_dir(),
    }
}

/// `%LOCALAPPDATA%`, falling back to the documented default location.
#[cfg(windows)]
pub fn local_app_data() -> PathBuf {
    nonempty_env_path("LOCALAPPDATA").unwrap_or_else(|| home_dir().join("AppData").join("Local"))
}

/// Application state root — settings, logs, the managed Python runtime and
/// resumable job status.
///
/// * Windows: `%LOCALAPPDATA%\lumen-cut`
/// * elsewhere: `~/.lumen-cut`
pub fn state_dir() -> PathBuf {
    if let Some(dir) = nonempty_env_path(ENV_STATE_DIR) {
        return dir;
    }
    #[cfg(windows)]
    {
        local_app_data().join("lumen-cut")
    }
    #[cfg(not(windows))]
    {
        home_dir().join(".lumen-cut")
    }
}

/// `settings.json` — the single persisted config document.
pub fn settings_file() -> PathBuf {
    state_dir().join("settings.json")
}

/// Persistent diagnostics for GUI launches, which have no terminal.
pub fn log_dir() -> PathBuf {
    state_dir().join("logs")
}

/// Managed uv virtualenv for the Python sidecars.
pub fn managed_runtime_dir() -> PathBuf {
    state_dir().join("runtime")
}

/// Interpreter inside [`managed_runtime_dir`]. uv follows the platform
/// virtualenv layout: `Scripts\python.exe` on Windows, `bin/python3` elsewhere.
pub fn managed_python() -> PathBuf {
    let runtime = managed_runtime_dir();
    if cfg!(windows) {
        runtime.join("Scripts").join("python.exe")
    } else {
        runtime.join("bin").join("python3")
    }
}

/// Default GUI project library. Tauri apps have no reliable working
/// directory, so projects live in a stable user-owned location.
pub fn projects_root() -> PathBuf {
    if let Some(root) = nonempty_env_path("LUMEN_CUT_PROJECTS_ROOT") {
        return root;
    }
    #[cfg(target_os = "macos")]
    {
        home_dir().join("Library/Application Support/lumen-cut/Projects")
    }
    #[cfg(windows)]
    {
        state_dir().join("Projects")
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        state_dir().join("projects")
    }
}

/// Executable name for `stem` on this platform.
pub fn executable_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

/// Whether the reveal command's exit status says anything about success.
///
/// `explorer.exe` returns a non-zero code even when it opens the window, so
/// on Windows the status must be ignored rather than surfaced as a failure.
pub const REVEAL_REPORTS_EXIT_STATUS: bool = !cfg!(windows);

/// Reveal `path` in the platform file manager, selecting it when it is a file.
pub fn reveal_command(path: &Path) -> (&'static str, Vec<String>) {
    let target = path.to_string_lossy().into_owned();
    if cfg!(target_os = "macos") {
        let select = path.is_file();
        let mut args = Vec::new();
        if select {
            args.push("-R".to_string());
        }
        args.push(target);
        ("open", args)
    } else if cfg!(windows) {
        // `explorer.exe /select,<file>` highlights the file in its folder;
        // given a directory it opens the directory itself.
        if path.is_file() {
            ("explorer.exe", vec![format!("/select,{target}")])
        } else {
            ("explorer.exe", vec![target])
        }
    } else {
        ("xdg-open", vec![target])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes tests that mutate process environment variables.
    pub static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn state_dir_override_wins_over_every_platform_default() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let previous = std::env::var_os(ENV_STATE_DIR);
        std::env::set_var(ENV_STATE_DIR, "/tmp/lumen-cut-state-override");
        let dir = state_dir();
        match previous {
            Some(value) => std::env::set_var(ENV_STATE_DIR, value),
            None => std::env::remove_var(ENV_STATE_DIR),
        }
        assert_eq!(dir, PathBuf::from("/tmp/lumen-cut-state-override"));
    }

    #[test]
    fn settings_logs_and_runtime_live_under_one_state_root() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let previous = std::env::var_os(ENV_STATE_DIR);
        let temp = tempfile::tempdir().unwrap();
        std::env::set_var(ENV_STATE_DIR, temp.path());
        let (settings, logs, python) = (settings_file(), log_dir(), managed_python());
        match previous {
            Some(value) => std::env::set_var(ENV_STATE_DIR, value),
            None => std::env::remove_var(ENV_STATE_DIR),
        }
        assert_eq!(settings, temp.path().join("settings.json"));
        assert_eq!(logs, temp.path().join("logs"));
        assert!(python.starts_with(temp.path().join("runtime")));
        assert_eq!(
            python.file_name().unwrap().to_string_lossy(),
            if cfg!(windows) {
                "python.exe"
            } else {
                "python3"
            }
        );
    }

    #[test]
    fn home_falls_back_across_the_platform_environment_variables() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let previous_home = std::env::var_os("HOME");
        let previous_profile = std::env::var_os("USERPROFILE");
        std::env::remove_var("HOME");
        std::env::set_var("USERPROFILE", "/tmp/lumen-cut-profile");
        let profile = home_dir();
        std::env::set_var("HOME", "/tmp/lumen-cut-home");
        let home = home_dir();
        match previous_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match previous_profile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
        assert_eq!(profile, PathBuf::from("/tmp/lumen-cut-profile"));
        assert_eq!(home, PathBuf::from("/tmp/lumen-cut-home"));
    }

    #[test]
    fn reveal_uses_the_native_file_manager_for_the_target_kind() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("doc.json");
        std::fs::write(&file, "{}").unwrap();
        let (dir_command, _) = reveal_command(temp.path());
        let (file_command, file_args) = reveal_command(&file);
        if cfg!(target_os = "macos") {
            assert_eq!(dir_command, "open");
            assert_eq!(file_args[0], "-R");
        } else if cfg!(windows) {
            assert_eq!(file_command, "explorer.exe");
            assert!(file_args[0].starts_with("/select,"));
        } else {
            assert_eq!(dir_command, "xdg-open");
        }
    }
}
