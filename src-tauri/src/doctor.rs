//! Shared environment probes for the CLI and GUI.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde::Serialize;

use crate::paths;

/// Directories prepended to `PATH` so a GUI launch finds the same tools an
/// interactive shell does. Pure so the platform choice stays testable.
fn tool_search_paths() -> Vec<PathBuf> {
    let home = paths::home_dir();
    let runtime = paths::managed_runtime_dir();
    if cfg!(windows) {
        // Explorer-launched apps inherit the machine PATH, which usually
        // predates a per-user winget/scoop install of ffmpeg or Python.
        let mut candidates = vec![runtime.join("Scripts")];
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let local = PathBuf::from(local);
            candidates.push(local.join("Microsoft").join("WindowsApps"));
            candidates.push(local.join("Programs").join("Python").join("Launcher"));
        }
        candidates.push(home.join("scoop").join("shims"));
        candidates
    } else {
        // Finder-launched macOS apps receive `/usr/bin:/bin:…`, not the
        // interactive shell PATH.
        vec![
            runtime.join("bin"),
            home.join(".local/bin"),
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ]
    }
}

/// Normalize `PATH` once at startup, before any ffmpeg/Python health check.
pub fn configure_process_path() {
    let mut paths = tool_search_paths();
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    paths.dedup();
    if let Ok(joined) = std::env::join_paths(paths) {
        std::env::set_var("PATH", joined);
    }
}

/// Windows `CREATE_NO_WINDOW`: probes must not flash a console window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A `Command` that never blocks on stdin and, on Windows, never flashes a
/// console window. Every synchronous probe in the app builds on this.
pub fn quiet_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    command.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// [`quiet_command`] with output discarded — for "does this exist" probes.
fn silent_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = quiet_command(program);
    command.stdout(Stdio::null()).stderr(Stdio::null());
    command
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

pub fn probe_args(command: &str) -> &'static [&'static str] {
    match command {
        "ffmpeg" | "ffprobe" => &["-version"],
        "hf" | "huggingface-cli" => &["--help"],
        _ => &["--version"],
    }
}

pub fn command_available(command: &str) -> bool {
    silent_command(command)
        .args(probe_args(command))
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Interpreter names to probe for a usable CPython, most specific first.
/// Windows installers register `python` and the `py` launcher; `python3` is
/// only a Microsoft Store alias that opens the Store when unresolved.
pub fn python_commands() -> &'static [&'static str] {
    if cfg!(windows) {
        &["python", "py"]
    } else {
        &["python3"]
    }
}

/// The first interpreter from [`python_commands`] that answers its probe.
pub fn python_command() -> Option<&'static str> {
    python_commands()
        .iter()
        .copied()
        .find(|command| command_available(command))
}

/// Prefer the current Hugging Face Hub CLI (`hf`); accept the legacy
/// `huggingface-cli` executable for older installations.
pub fn huggingface_cli() -> Option<&'static str> {
    ["hf", "huggingface-cli"]
        .into_iter()
        .find(|command| command_available(command))
}

pub fn checks() -> Vec<Check> {
    let mut output = Vec::new();
    for (name, command) in [
        ("ffmpeg", "ffmpeg"),
        ("ffprobe", "ffprobe"),
        ("yt-dlp", "yt-dlp"),
    ] {
        let ok = command_available(command);
        output.push(Check {
            name: name.into(),
            ok,
            detail: if ok {
                "available".into()
            } else {
                "unavailable or failed its probe".into()
            },
        });
    }
    // The check keeps the `python3` name across platforms so the diagnostics
    // UI and its tests stay stable; the detail names the executable found.
    let python = python_command();
    output.push(Check {
        name: "python3".into(),
        ok: python.is_some(),
        detail: python
            .map(|command| format!("available via `{command}`"))
            .unwrap_or_else(|| "unavailable or failed its probe".into()),
    });
    let hub_cli = huggingface_cli();
    output.push(Check {
        name: "hf".into(),
        ok: hub_cli.is_some(),
        detail: hub_cli
            .map(|command| format!("available via `{command}`"))
            .unwrap_or_else(|| "unavailable or failed its probe".into()),
    });
    let asr = crate::asr::runtime_status();
    output.push(Check {
        name: "ASR runtime".into(),
        ok: asr.runtime_ready,
        detail: match asr.python_path {
            Some(path) => format!("{} via {path}", asr.runtime_detail),
            None => asr.runtime_detail,
        },
    });
    output.push(Check {
        name: "speaker runtime".into(),
        ok: asr.diarize_runtime_ready,
        detail: asr.diarize_runtime_detail,
    });
    let config = crate::data::modelconfig::load();
    let token = (!config.hf_token.trim().is_empty())
        .then_some(config.hf_token.as_str())
        .or_else(|| {
            std::env::var_os("HF_TOKEN")
                .or_else(|| std::env::var_os("HUGGING_FACE_HUB_TOKEN"))
                .as_ref()
                .map(|_| "environment")
        });
    output.push(Check {
        name: "HF_TOKEN".into(),
        ok: token.is_some(),
        detail: if token.is_some() {
            "set".into()
        } else {
            "unset (gated models need it)".into()
        },
    });
    let home = paths::home_dir();
    for (name, model) in [
        ("Qwen3-ASR", config.asr_model.as_str()),
        ("ForcedAligner", config.asr_aligner.as_str()),
        ("pyannote", config.diarize_model.as_str()),
    ] {
        // Transcription passes a shared local snapshot (discovered via
        // lumen-models, e.g. a lumen-asr install) to the sidecar when one
        // exists, so report that directory as the effective source.
        if name == "Qwen3-ASR" {
            if let Some(dir) = crate::asr::local_qwen_model_dir(model) {
                output.push(Check {
                    name: name.into(),
                    ok: true,
                    detail: format!("shared: {}", dir.display()),
                });
                continue;
            }
        }
        let ok = if name == "pyannote" {
            crate::data::modelconfig::diarize_model_cached(&home, model)
        } else {
            crate::data::modelconfig::model_cached(&home, model)
        };
        output.push(Check {
            name: name.into(),
            ok,
            detail: if ok {
                format!("cached: {model}")
            } else {
                format!("not cached: {model}")
            },
        });
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_each_tools_supported_probe_flag() {
        assert_eq!(probe_args("ffmpeg"), ["-version"]);
        assert_eq!(probe_args("ffprobe"), ["-version"]);
        assert_eq!(probe_args("hf"), ["--help"]);
        assert_eq!(probe_args("huggingface-cli"), ["--help"]);
        assert_eq!(probe_args("python3"), ["--version"]);
        assert_eq!(probe_args("python"), ["--version"]);
        assert_eq!(probe_args("py"), ["--version"]);
    }

    #[test]
    fn python_probe_skips_the_windows_store_alias() {
        // `python3` on Windows is a Store stub that opens the Store rather
        // than running an interpreter, so it must never be probed there.
        if cfg!(windows) {
            assert_eq!(python_commands(), ["python", "py"]);
        } else {
            assert_eq!(python_commands(), ["python3"]);
        }
    }

    #[test]
    fn path_seeds_target_this_platforms_user_tool_locations() {
        let seeds = tool_search_paths();
        assert!(!seeds.is_empty());
        let joined = seeds
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("|");
        if cfg!(windows) {
            assert!(joined.contains("Scripts"));
            assert!(!joined.contains("homebrew"));
        } else {
            assert!(joined.contains("/opt/homebrew/bin"));
        }
        // The managed virtualenv must be searched before anything the system
        // installed, so an app-managed runtime wins over a stale global one.
        assert!(seeds[0].starts_with(paths::managed_runtime_dir()));
    }

    #[test]
    fn check_set_covers_tools_token_and_all_model_families() {
        let names: Vec<String> = checks().into_iter().map(|check| check.name).collect();
        assert_eq!(names.len(), 11);
        for expected in [
            "ffmpeg",
            "ffprobe",
            "yt-dlp",
            "python3",
            "hf",
            "HF_TOKEN",
            "ASR runtime",
            "speaker runtime",
            "Qwen3-ASR",
            "ForcedAligner",
            "pyannote",
        ] {
            assert!(names.iter().any(|name| name == expected));
        }
    }
}
