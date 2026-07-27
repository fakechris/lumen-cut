//! Export artifact naming: `{project}-{YYYYMMDD-HHmmss}.{ext}` so a new
//! export never silently overwrites a previous one.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};

/// Fallback stem when the project name sanitizes to nothing.
const FALLBACK_STEM: &str = "export";

/// Make a project name safe for use as a file-name stem: path separators and
/// filesystem-illegal characters become `-`, whitespace runs compress to a
/// single space, and an empty result falls back to [`FALLBACK_STEM`].
pub fn sanitize_stem(name: &str) -> String {
    let mut out = String::new();
    let mut last_space = true; // suppress leading whitespace
    for ch in name.chars() {
        let ch = if ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            ch
        } else if ch.is_whitespace() {
            ' '
        } else {
            '-'
        };
        if ch == ' ' {
            if last_space || out.is_empty() {
                continue;
            }
            last_space = true;
        } else {
            last_space = false;
        }
        out.push(ch);
    }
    let out = out
        .trim_end_matches([' ', '-', '.'])
        .trim_start_matches(['-', '.'])
        .to_string();
    if out.is_empty() {
        FALLBACK_STEM.to_string()
    } else {
        out
    }
}

/// `{safe-name}-{YYYYMMDD-HHmmss}` stamped in local time.
pub fn export_file_stem(project_name: &str, now: DateTime<Local>) -> String {
    format!(
        "{}-{}",
        sanitize_stem(project_name),
        now.format("%Y%m%d-%H%M%S")
    )
}

/// Stem whose `{stem}.{extension}` file does not exist in `dir`; appends
/// `-2`, `-3`, … when a same-second export already produced that file.
pub fn unique_export_stem(dir: &Path, stem: &str, extension: &str) -> String {
    if !dir.join(format!("{stem}.{extension}")).exists() {
        return stem.to_string();
    }
    for suffix in 2.. {
        let candidate = format!("{stem}-{suffix}");
        if !dir.join(format!("{candidate}.{extension}")).exists() {
            return candidate;
        }
    }
    unreachable!("unbounded suffix search cannot end")
}

/// Full path of [`unique_export_stem`] joined with its extension.
pub fn unique_export_path(dir: &Path, stem: &str, extension: &str) -> PathBuf {
    dir.join(format!(
        "{}.{}",
        unique_export_stem(dir, stem, extension),
        extension
    ))
}

/// Temporary render target derived from the final path:
/// `name.ext` becomes `name.in-progress.ext` next to it.
pub fn in_progress_path(final_path: &Path) -> PathBuf {
    let stem = final_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| FALLBACK_STEM.to_string());
    let extension = final_path
        .extension()
        .map(|extension| extension.to_string_lossy().into_owned())
        .unwrap_or_default();
    final_path.with_file_name(format!("{stem}.in-progress.{extension}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone};

    fn fixed_now() -> DateTime<Local> {
        let naive = NaiveDate::from_ymd_opt(2026, 7, 27)
            .unwrap()
            .and_hms_opt(9, 45, 33)
            .unwrap();
        Local.from_local_datetime(&naive).earliest().unwrap()
    }

    #[test]
    fn sanitize_keeps_plain_names_and_unicode() {
        assert_eq!(sanitize_stem("Interview"), "Interview");
        assert_eq!(sanitize_stem("采访 项目-02_v1.5"), "采访 项目-02_v1.5");
    }

    #[test]
    fn sanitize_replaces_illegal_characters_with_dashes() {
        assert_eq!(
            sanitize_stem("a/b\\c:d*e?f\"g<h>i|j"),
            "a-b-c-d-e-f-g-h-i-j"
        );
    }

    #[test]
    fn sanitize_compresses_whitespace_and_trims_edges() {
        assert_eq!(sanitize_stem("  my \t project  "), "my project");
        assert_eq!(sanitize_stem("--draft--"), "draft");
        assert_eq!(sanitize_stem("name."), "name");
    }

    #[test]
    fn sanitize_falls_back_when_nothing_usable_remains() {
        assert_eq!(sanitize_stem(""), "export");
        assert_eq!(sanitize_stem("  ///  "), "export");
        assert_eq!(sanitize_stem("..."), "export");
    }

    #[test]
    fn export_stem_combines_name_and_local_timestamp() {
        assert_eq!(
            export_file_stem("Interview", fixed_now()),
            "Interview-20260727-094533"
        );
        assert_eq!(export_file_stem("a/b", fixed_now()), "a-b-20260727-094533");
        assert_eq!(export_file_stem(" ", fixed_now()), "export-20260727-094533");
    }

    #[test]
    fn unique_stem_appends_suffixes_on_collision() {
        let dir = tempfile::tempdir().unwrap();
        let stem = "Interview-20260727-094533";
        assert_eq!(unique_export_stem(dir.path(), stem, "mp4"), stem);
        std::fs::write(dir.path().join(format!("{stem}.mp4")), b"x").unwrap();
        assert_eq!(
            unique_export_stem(dir.path(), stem, "mp4"),
            format!("{stem}-2")
        );
        std::fs::write(dir.path().join(format!("{stem}-2.mp4")), b"x").unwrap();
        assert_eq!(
            unique_export_stem(dir.path(), stem, "mp4"),
            format!("{stem}-3")
        );
        // A different extension does not count as a collision.
        assert_eq!(unique_export_stem(dir.path(), stem, "srt"), stem);
    }

    #[test]
    fn unique_path_joins_dir_and_extension() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            unique_export_path(dir.path(), "a-20260727-094533", "fcpxml"),
            dir.path().join("a-20260727-094533.fcpxml")
        );
    }

    #[test]
    fn in_progress_path_derives_from_the_final_name() {
        let dir = Path::new("/tmp/project");
        assert_eq!(
            in_progress_path(&dir.join("Interview-20260727-094533.mp4")),
            dir.join("Interview-20260727-094533.in-progress.mp4")
        );
        assert_eq!(
            in_progress_path(&dir.join("Interview-20260727-094533-2.mov")),
            dir.join("Interview-20260727-094533-2.in-progress.mov")
        );
    }
}
