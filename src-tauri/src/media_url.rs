//! yt-dlp wrapper for URL ingestion (YouTube, Vimeo, Loom, …).
//!
//! Prefer mp4/avc1 video with m4a audio and let ffmpeg mux them. The final
//! artefact path is taken from
//! `--print after_move:filepath` so merged outputs resolve correctly;
//! the sidecar's exit code surfaces cleanly through `crate::proc::run`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::proc;

/// Format selector: mp4/avc1 video first, m4a audio,
/// progressive mp4 next, then any mergeable pair.
const FORMAT: &str =
    "bv*[ext=mp4][vcodec^=avc1]+ba[ext=m4a]/bv*[ext=mp4]+ba[ext=m4a]/b[ext=mp4]/bv*+ba/b";
const DOWNLOAD_PROGRESS_PREFIX: &str = "LUMEN_CUT_DOWNLOAD ";
const DOWNLOAD_PROGRESS_TEMPLATE: &str = r#"download:LUMEN_CUT_DOWNLOAD {"percent":"%(progress._percent_str)s","downloaded":"%(progress.downloaded_bytes)s","total":"%(progress.total_bytes_estimate)s","eta":"%(progress.eta)s"}"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadProgress {
    pub percent: u8,
    pub downloaded_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub eta_seconds: Option<u64>,
}

pub type DownloadProgressCallback = Arc<dyn Fn(DownloadProgress) + Send + Sync>;

/// Download a URL to the given output template (`%(ext)s` recommended).
pub async fn download(url: &str, output_template: &Path) -> AppResult<PathBuf> {
    download_with_progress(url, output_template, None).await
}

pub async fn download_with_progress(
    url: &str,
    output_template: &Path,
    on_progress: Option<DownloadProgressCallback>,
) -> AppResult<PathBuf> {
    if let Some(parent) = output_template.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tpl = output_template.display().to_string();
    let args = [
        "--no-playlist",
        "--newline",
        "--no-colors",
        "--progress-template",
        DOWNLOAD_PROGRESS_TEMPLATE,
        "--no-mtime",
        "--no-part",
        "--print",
        "after_move:filepath",
        "-f",
        FORMAT,
        "-o",
        &tpl,
        url,
    ];
    let out = if let Some(callback) = on_progress {
        proc::run_with_progress(
            "yt-dlp",
            &args,
            Arc::new(move |line| {
                if let Some(progress) = parse_progress_line(&line) {
                    callback(progress);
                }
            }),
        )
        .await?
    } else {
        proc::run("yt-dlp", &args).await?
    };

    parse_output_path(&out).ok_or_else(|| AppError::YtDlp("no output line".to_string()))
}

fn parse_progress_line(line: &str) -> Option<DownloadProgress> {
    #[derive(Deserialize)]
    struct RawProgress {
        percent: String,
        downloaded: String,
        total: String,
        eta: String,
    }

    let raw: RawProgress =
        serde_json::from_str(line.strip_prefix(DOWNLOAD_PROGRESS_PREFIX)?).ok()?;
    let percent = raw
        .percent
        .trim()
        .trim_end_matches('%')
        .parse::<f64>()
        .ok()?
        .round()
        .clamp(0.0, 100.0) as u8;
    let parse_optional = |value: &str| value.trim().parse::<u64>().ok();
    Some(DownloadProgress {
        percent,
        downloaded_bytes: parse_optional(&raw.downloaded),
        total_bytes: parse_optional(&raw.total),
        eta_seconds: parse_optional(&raw.eta),
    })
}

/// Resolve the downloaded file path from yt-dlp stdout.
///
/// Priority: the bare path printed by `--print after_move:filepath`
/// (emitted after any merge/move, so it names the final artefact); then a
/// `[Merger] Merging formats into "X"` line; then a `[download]
/// Destination: X` line. Post-merge chatter such as `[download] Deleting
/// original file …` is ignored.
fn parse_output_path(stdout: &str) -> Option<PathBuf> {
    let mut printed = None;
    let mut merged = None;
    let mut dest = None;
    for line in stdout.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(p) = line
            .strip_prefix("[Merger] Merging formats into \"")
            .and_then(|s| s.strip_suffix('"'))
        {
            merged = Some(PathBuf::from(p));
        } else if let Some(p) = line.strip_prefix("[download] Destination: ") {
            dest = Some(PathBuf::from(p));
        } else if !line.starts_with('[') {
            // Bare path line — output of `--print after_move:filepath`.
            printed = Some(PathBuf::from(line));
        }
    }
    printed.or(merged).or(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_print_after_move_over_merger_chatter() {
        // Real yt-dlp output shape for a merged (bestvideo+bestaudio) run.
        let out = "[download] Destination: /dl/source.f137.mp4\n\
                   [download] 100% of   12.34MiB in 00:00:02\n\
                   [download] Destination: /dl/source.f140.m4a\n\
                   [download] 100% of    1.23MiB in 00:00:01\n\
                   [Merger] Merging formats into \"/dl/source.mp4\"\n\
                   [download] Deleting original file /dl/source.f137.mp4 (pass -k to keep)\n\
                   [download] Deleting original file /dl/source.f140.m4a (pass -k to keep)\n\
                   /dl/source.mp4\n";
        assert_eq!(
            parse_output_path(out),
            Some(PathBuf::from("/dl/source.mp4"))
        );
    }

    #[test]
    fn falls_back_to_merger_line_without_print() {
        let out = "[download] Destination: /dl/source.f137.mp4\n\
                   [download] Destination: /dl/source.f140.m4a\n\
                   [Merger] Merging formats into \"/dl/source.mp4\"\n\
                   [download] Deleting original file /dl/source.f137.mp4 (pass -k to keep)\n\
                   [download] Deleting original file /dl/source.f140.m4a (pass -k to keep)\n";
        assert_eq!(
            parse_output_path(out),
            Some(PathBuf::from("/dl/source.mp4"))
        );
    }

    #[test]
    fn falls_back_to_destination_for_single_file() {
        let out = "[download] Destination: /dl/source.mp4\n\
                   [download] 100% of   13.57MiB in 00:00:03\n";
        assert_eq!(
            parse_output_path(out),
            Some(PathBuf::from("/dl/source.mp4"))
        );
    }

    #[test]
    fn empty_output_yields_none() {
        assert_eq!(parse_output_path("\n  \n"), None);
    }

    #[test]
    fn parses_machine_readable_download_progress() {
        let progress = parse_progress_line(
            r#"LUMEN_CUT_DOWNLOAD {"percent":" 42.7%","downloaded":"1048576","total":"2457600","eta":"8"}"#,
        )
        .unwrap();
        assert_eq!(progress.percent, 43);
        assert_eq!(progress.downloaded_bytes, Some(1_048_576));
        assert_eq!(progress.total_bytes, Some(2_457_600));
        assert_eq!(progress.eta_seconds, Some(8));
        assert!(parse_progress_line("[download] Destination: source.mp4").is_none());
    }
}
