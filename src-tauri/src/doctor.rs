//! Shared environment probes for the CLI and GUI.

use std::process::{Command, Stdio};

use serde::Serialize;

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
    Command::new(command)
        .args(probe_args(command))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
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
        ("python3", "python3"),
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
    let hub_cli = huggingface_cli();
    output.push(Check {
        name: "hf".into(),
        ok: hub_cli.is_some(),
        detail: hub_cli
            .map(|command| format!("available via `{command}`"))
            .unwrap_or_else(|| "unavailable or failed its probe".into()),
    });
    let token = std::env::var_os("HF_TOKEN").or_else(|| std::env::var_os("HUGGING_FACE_HUB_TOKEN"));
    output.push(Check {
        name: "HF_TOKEN".into(),
        ok: token.is_some(),
        detail: if token.is_some() {
            "set".into()
        } else {
            "unset (gated models need it)".into()
        },
    });
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let config = crate::data::modelconfig::load();
    for (name, model) in [
        ("Qwen3-ASR", config.asr_model.as_str()),
        ("ForcedAligner", config.asr_aligner.as_str()),
        ("pyannote", config.diarize_model.as_str()),
        ("sortformer", "nvidia/diar_streaming_sortformer_4spk-v2.1"),
        ("wespeaker", "pyannote/wespeaker-voxceleb-resnet34-LM"),
    ] {
        let ok = crate::data::modelconfig::model_cached(&home, model);
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
            "Qwen3-ASR",
            "ForcedAligner",
            "pyannote",
            "sortformer",
            "wespeaker",
        ] {
            assert!(names.iter().any(|name| name == expected));
        }
    }
}
