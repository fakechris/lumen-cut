//! Microphone capture input selection.
//!
//! ffmpeg names capture devices per platform: macOS uses AVFoundation index
//! syntax, Windows uses DirectShow device names, Linux uses PulseAudio. This
//! module resolves the `-f <backend> -i <device>` pair once so the GUI command
//! and the CLI record subcommand stay in agreement.

use crate::error::AppResult;

/// Overrides the resolved device, e.g. `LUMEN_CUT_AUDIO_INPUT="audio=Mic"`.
/// The value is passed to ffmpeg's `-i` verbatim.
pub const ENV_AUDIO_INPUT: &str = "LUMEN_CUT_AUDIO_INPUT";

/// Overrides the capture backend, e.g. `LUMEN_CUT_AUDIO_BACKEND=alsa`.
pub const ENV_AUDIO_BACKEND: &str = "LUMEN_CUT_AUDIO_BACKEND";

/// ffmpeg input device demuxer for this platform.
pub fn default_backend() -> &'static str {
    if cfg!(target_os = "macos") {
        "avfoundation"
    } else if cfg!(windows) {
        "dshow"
    } else {
        "pulse"
    }
}

/// A DirectShow capture device as reported by `ffmpeg -list_devices`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DshowDevice {
    /// Human-readable name, e.g. `Microphone (Realtek Audio)`.
    pub name: String,
    /// Stable `@device_cm_{…}` moniker. Preferred for `-i` because it is
    /// pure ASCII and survives console code pages that mangle the friendly
    /// name of a non-English device.
    pub alternative_name: Option<String>,
}

impl DshowDevice {
    /// The `audio=…` argument ffmpeg expects.
    pub fn input_argument(&self) -> String {
        match &self.alternative_name {
            Some(alternative) => format!("audio={alternative}"),
            None => format!("audio={}", self.name),
        }
    }
}

/// Parse the audio devices out of `ffmpeg -list_devices true -f dshow -i dummy`
/// diagnostics. ffmpeg writes this to stderr and exits non-zero by design, so
/// only the text is meaningful.
///
/// Lines look like:
/// ```text
/// [dshow @ 0000…] "Microphone (Realtek Audio)" (audio)
/// [dshow @ 0000…]   Alternative name "@device_cm_{33D9…}\wave_{9B36…}"
/// ```
pub fn parse_dshow_audio_devices(diagnostics: &str) -> Vec<DshowDevice> {
    let mut devices: Vec<DshowDevice> = Vec::new();
    // Index of the audio device the next `Alternative name` line describes.
    // A video device clears it, since ffmpeg prints monikers for those too.
    let mut pending: Option<usize> = None;
    for line in diagnostics.lines() {
        let Some(quoted) = quoted_value(line) else {
            continue;
        };
        if line.contains("Alternative name") {
            if let Some(index) = pending.take() {
                devices[index].alternative_name = Some(quoted);
            }
        } else if line.contains("(audio)") {
            devices.push(DshowDevice {
                name: quoted,
                alternative_name: None,
            });
            pending = Some(devices.len() - 1);
        } else if line.contains("(video)") {
            pending = None;
        }
    }
    devices
}

/// Extract the first double-quoted span of `line`.
fn quoted_value(line: &str) -> Option<String> {
    let start = line.find('"')? + 1;
    let end = line[start..].find('"')? + start;
    Some(line[start..end].to_string())
}

/// Ask ffmpeg to enumerate DirectShow devices.
#[cfg(windows)]
async fn dshow_devices() -> AppResult<Vec<DshowDevice>> {
    use crate::error::AppError;
    use std::process::Stdio;

    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .args([
            "-hide_banner",
            "-list_devices",
            "true",
            "-f",
            "dshow",
            "-i",
            "dummy",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let output = command.output().await.map_err(|error| AppError::Sidecar {
        sidecar: "ffmpeg",
        message: format!("could not list DirectShow devices: {error}"),
    })?;
    Ok(parse_dshow_audio_devices(&String::from_utf8_lossy(
        &output.stderr,
    )))
}

/// The `-f <backend> -i <device>` pair for the default microphone.
///
/// On Windows there is no positional "first device" syntax, so the device is
/// enumerated. macOS and Linux name their default input directly.
pub async fn microphone_input() -> AppResult<Vec<String>> {
    let backend = std::env::var(ENV_AUDIO_BACKEND)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default_backend().to_string());

    if let Some(input) = std::env::var(ENV_AUDIO_INPUT)
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(vec!["-f".into(), backend, "-i".into(), input]);
    }

    #[cfg(windows)]
    {
        let devices = dshow_devices().await?;
        let device = devices.first().ok_or_else(|| {
            crate::error::AppError::Schema(
                "no DirectShow audio input device was found; check Settings → Privacy \
                 → Microphone and that a microphone is connected"
                    .into(),
            )
        })?;
        return Ok(vec![
            "-f".into(),
            backend,
            "-i".into(),
            device.input_argument(),
        ]);
    }

    #[cfg(not(windows))]
    {
        let device = if cfg!(target_os = "macos") {
            ":0" // AVFoundation "no video, first audio device".
        } else {
            "default"
        };
        Ok(vec!["-f".into(), backend, "-i".into(), device.into()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_matches_the_platform_ffmpeg_ships_with() {
        if cfg!(target_os = "macos") {
            assert_eq!(default_backend(), "avfoundation");
        } else if cfg!(windows) {
            assert_eq!(default_backend(), "dshow");
        } else {
            assert_eq!(default_backend(), "pulse");
        }
    }

    #[test]
    fn parses_audio_devices_and_prefers_the_ascii_moniker() {
        let diagnostics = concat!(
            "[dshow @ 000001] \"HD Webcam\" (video)\n",
            "[dshow @ 000001]   Alternative name \"@device_pnp_\\\\?\\usb#vid_0000\"\n",
            "[dshow @ 000001] \"麦克风 (Realtek Audio)\" (audio)\n",
            "[dshow @ 000001]   Alternative name \"@device_cm_{33D9A762}\\wave_{9B365890}\"\n",
            "[dshow @ 000001] \"Line In\" (audio)\n",
        );
        let devices = parse_dshow_audio_devices(diagnostics);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].name, "麦克风 (Realtek Audio)");
        assert_eq!(
            devices[0].input_argument(),
            "audio=@device_cm_{33D9A762}\\wave_{9B365890}"
        );
        // Without a moniker the friendly name is the only option.
        assert_eq!(devices[1].input_argument(), "audio=Line In");
    }

    #[test]
    fn ignores_diagnostics_without_any_capture_device() {
        assert!(parse_dshow_audio_devices("[dshow @ 1] dummy: Immediate exit").is_empty());
    }

    #[tokio::test]
    async fn explicit_override_bypasses_device_discovery() {
        let previous = std::env::var_os(ENV_AUDIO_INPUT);
        std::env::set_var(ENV_AUDIO_INPUT, "audio=Chosen Device");
        let input = microphone_input().await;
        match previous {
            Some(value) => std::env::set_var(ENV_AUDIO_INPUT, value),
            None => std::env::remove_var(ENV_AUDIO_INPUT),
        }
        let input = input.unwrap();
        assert_eq!(input[0], "-f");
        assert_eq!(input[1], default_backend());
        assert_eq!(input[2], "-i");
        assert_eq!(input[3], "audio=Chosen Device");
    }
}
