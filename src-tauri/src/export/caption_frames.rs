//! WYSIWYG caption burn-in, Rust half.
//!
//! The in-app export burns captions by overlaying PNG frames rendered by the
//! frontend with the SAME Canvas2D renderer as the program monitor
//! (src/captions/captionRender.ts), instead of generating ASS and letting
//! libass rasterize it. Flow:
//!
//!   1. `caption_export_prepare` builds the render spec from the same project
//!      snapshot the export uses (projected caption doc, cuts, style, canvas)
//!      — retimed cues with real word timing — and opens a frame-upload
//!      session under `<project>/.lumen-cut/caption-frames/`.
//!   2. The frontend renders one PNG per caption state (deduped by content
//!      hash) and streams them via `caption_frames_push` (raw IPC body, no
//!      JSON round-trip), then `caption_frames_seal` writes the timeline
//!      manifest LAST, so a manifest on disk always means a complete upload.
//!   3. `export_video_impl` (commands.rs) picks up the sealed manifest when
//!      its content hash matches the export's own snapshot and hands the
//!      ffconcat timeline to `render_video_with_broll_options` (video.rs),
//!      which overlays it with ffmpeg. When no sealed manifest exists (CLI /
//!      MCP exports have no webview) the export keeps burning ASS captions —
//!      that path is untouched and remains the only option there.
//!
//! Titles are unaffected: they still burn via ASS on top of the overlay.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::data::soft_cut::Cut;
use crate::data::substyle::SubStyle;
use crate::data::Doc;
use crate::error::{AppError, AppResult};

use super::project::{cut_intervals, fully_cut, retime};

/// Bump when the frontend renderer changes in a way that invalidates sealed
/// frame sets from older builds (part of the content hash).
pub const CAPTION_RENDERER_VERSION: u32 = 1;

const FRAMES_DIR_NAME: &str = "caption-frames";

#[derive(Clone, Default)]
pub struct CaptionFramesState {
    sessions: Arc<Mutex<HashMap<String, CaptionFramesSession>>>,
}

struct CaptionFramesSession {
    dir: PathBuf,
    hash: String,
}

fn frames_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(".lumen-cut").join(FRAMES_DIR_NAME)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionSpecWord {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionSpecCue {
    pub id: String,
    pub start: f64,
    pub end: f64,
    pub source_text: String,
    pub translation_text: Option<String>,
    /// Real ASR word timing for `source_text`; empty when the cue's main text
    /// is NOT the source (translation-only export) — the frontend then
    /// approximates timing, exactly like the preview does.
    pub words: Vec<CaptionSpecWord>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionRenderSpec {
    pub version: u32,
    pub hash: String,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub style: SubStyle,
    pub cues: Vec<CaptionSpecCue>,
}

/// Letters/digits only, lowercased — the alignment currency between the
/// sentence text and its ASR words (punctuation/spacing differ across
/// tokenizers; letter counts do not).
fn alnum_fold(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Content hash binding a sealed frame set to one exact export snapshot:
/// any caption/style/cut/canvas change produces a different hash and the
/// export falls back to ASS (or the frontend re-renders).
pub fn caption_content_hash(
    caption_doc: &Doc,
    cuts: &[Cut],
    style: &SubStyle,
    width: u32,
    height: u32,
) -> AppResult<String> {
    #[derive(Serialize)]
    struct HashInput<'a> {
        version: u32,
        caption_doc: &'a Doc,
        cuts: &'a [Cut],
        style: &'a SubStyle,
        width: u32,
        height: u32,
    }
    let json = serde_json::to_vec(&HashInput {
        version: CAPTION_RENDERER_VERSION,
        caption_doc,
        cuts,
        style,
        width,
        height,
    })?;
    let mut hasher = DefaultHasher::new();
    json.hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

/// Build the caption render spec from a project snapshot. Returns None when
/// there is nothing to burn (no visible cues).
pub fn build_render_spec(
    caption_doc: &Doc,
    cuts: &[Cut],
    style: &SubStyle,
    width: u32,
    height: u32,
) -> AppResult<Option<CaptionRenderSpec>> {
    let hash = caption_content_hash(caption_doc, cuts, style, width, height)?;
    let intervals = cut_intervals(caption_doc, cuts);
    let duration: f64 = super::project::kept_intervals(caption_doc, cuts)
        .iter()
        .map(|(start, end)| end - start)
        .sum();
    let mut cues = Vec::new();
    for paragraph in &caption_doc.paragraphs {
        for sentence in &paragraph.sentences {
            if sentence.words.is_empty() {
                continue;
            }
            let start = sentence.words.first().map(|w| w.start).unwrap_or(0.0);
            let end = sentence.words.last().map(|w| w.end).unwrap_or(start + 1.0);
            if fully_cut(start, end, &intervals) {
                continue;
            }
            let (ns, ne) = (retime(start, &intervals), retime(end, &intervals));
            if ne <= ns {
                continue;
            }
            let trimmed = sentence.text.trim();
            if trimmed.is_empty() {
                continue;
            }
            // Bilingual cues are "source\ntranslation" (export_settings
            // projection); split at the first newline.
            let (source_text, translation_text) = match trimmed.split_once('\n') {
                Some((source, translation)) => (source.to_string(), Some(translation.to_string())),
                None => (trimmed.to_string(), None),
            };
            // Keep real word timing only when the words actually describe the
            // main line — translation-only exports keep source-language words
            // against translated text, where timing would be wrong.
            let words_match = !sentence.words.is_empty()
                && alnum_fold(
                    &sentence
                        .words
                        .iter()
                        .map(|w| w.text.as_str())
                        .collect::<String>(),
                ) == alnum_fold(&source_text);
            let words = if words_match {
                sentence
                    .words
                    .iter()
                    .map(|word| CaptionSpecWord {
                        text: word.text.clone(),
                        start: retime(word.start, &intervals),
                        end: retime(word.end, &intervals),
                    })
                    .filter(|word| word.end > word.start)
                    .collect()
            } else {
                Vec::new()
            };
            cues.push(CaptionSpecCue {
                id: sentence.id.clone(),
                start: ns,
                end: ne,
                source_text,
                translation_text,
                words,
            });
        }
    }
    if cues.is_empty() {
        return Ok(None);
    }
    Ok(Some(CaptionRenderSpec {
        version: CAPTION_RENDERER_VERSION,
        hash,
        width,
        height,
        duration,
        style: style.clone(),
        cues,
    }))
}

// ---------------------------------------------------------------------------
// Upload session commands
// ---------------------------------------------------------------------------

/// Open an upload session for a freshly-built spec: wipe any previous frame
/// set and remember the hash `caption_frames_seal` must match.
pub async fn caption_frames_begin_impl(
    project_dir: &Path,
    hash: &str,
    state: &CaptionFramesState,
) -> AppResult<()> {
    let dir = frames_dir(project_dir);
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await?;
    state
        .sessions
        .lock()
        .expect("caption frames state poisoned")
        .insert(
            project_dir.to_string_lossy().to_string(),
            CaptionFramesSession {
                dir,
                hash: hash.to_string(),
            },
        );
    Ok(())
}

/// No captions to burn for this snapshot: drop any stale frame set so the
/// export cannot pick it up.
pub async fn caption_frames_clear_impl(project_dir: &Path, state: &CaptionFramesState) {
    state
        .sessions
        .lock()
        .expect("caption frames state poisoned")
        .remove(&project_dir.to_string_lossy().to_string());
    let _ = tokio::fs::remove_dir_all(frames_dir(project_dir)).await;
}

fn session_dir(state: &CaptionFramesState, project_dir: &Path) -> AppResult<(PathBuf, String)> {
    let sessions = state
        .sessions
        .lock()
        .expect("caption frames state poisoned");
    sessions
        .get(&project_dir.to_string_lossy().to_string())
        .map(|session| (session.dir.clone(), session.hash.clone()))
        .ok_or_else(|| {
            AppError::Schema("no caption frame session; call caption_export_prepare first".into())
        })
}

/// Raw IPC body: `[u16 LE name-len][name utf-8][png bytes]`.
pub fn caption_frames_push_impl(
    project_dir: &Path,
    state: &CaptionFramesState,
    body: &[u8],
) -> AppResult<usize> {
    if body.len() < 2 {
        return Err(AppError::Schema("caption frame payload too short".into()));
    }
    let name_len = u16::from_le_bytes([body[0], body[1]]) as usize;
    if body.len() < 2 + name_len + 8 {
        return Err(AppError::Schema("caption frame payload truncated".into()));
    }
    let name = std::str::from_utf8(&body[2..2 + name_len])
        .map_err(|error| AppError::Schema(format!("bad caption frame name: {error}")))?;
    let png = &body[2 + name_len..];
    let safe: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if safe.is_empty() || safe.len() > 200 {
        return Err(AppError::Schema(format!(
            "bad caption frame name: {name:?}"
        )));
    }
    if png.len() < 8 || &png[1..4] != b"PNG" {
        return Err(AppError::Schema(
            "caption frame payload is not a PNG".into(),
        ));
    }
    let (dir, _hash) = session_dir(state, project_dir)?;
    std::fs::write(dir.join(safe), png)?;
    Ok(png.len())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CaptionManifestSegment {
    pub file: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionManifest {
    pub version: u32,
    pub hash: String,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub segments: Vec<CaptionManifestSegment>,
}

/// Seal the upload: verify hash + files, write the manifest LAST (its
/// presence is what makes the frame set visible to the export).
pub fn caption_frames_seal_impl(
    project_dir: &Path,
    state: &CaptionFramesState,
    manifest_json: &str,
) -> AppResult<()> {
    let manifest: CaptionManifest = serde_json::from_str(manifest_json)?;
    let (dir, hash) = session_dir(state, project_dir)?;
    if manifest.hash != hash {
        return Err(AppError::Schema(
            "caption manifest hash does not match the prepared spec; re-run caption_export_prepare"
                .into(),
        ));
    }
    if manifest.segments.is_empty() {
        return Err(AppError::Schema("caption manifest has no segments".into()));
    }
    for segment in &manifest.segments {
        if segment.end <= segment.start {
            return Err(AppError::Schema(format!(
                "caption manifest segment has a non-positive window: {segment:?}"
            )));
        }
        if !dir.join(&segment.file).is_file() {
            return Err(AppError::Schema(format!(
                "caption frame {} referenced by the manifest was not uploaded",
                segment.file
            )));
        }
    }
    std::fs::write(dir.join("manifest.json"), manifest_json)?;
    tracing::info!(
        pipeline = "caption-frames",
        segments = manifest.segments.len(),
        duration = manifest.duration,
        "caption frames sealed"
    );
    state
        .sessions
        .lock()
        .expect("caption frames state poisoned")
        .remove(&project_dir.to_string_lossy().to_string());
    Ok(())
}

pub async fn caption_frames_abort_impl(
    project_dir: &Path,
    state: &CaptionFramesState,
) -> AppResult<()> {
    state
        .sessions
        .lock()
        .expect("caption frames state poisoned")
        .remove(&project_dir.to_string_lossy().to_string());
    let _ = tokio::fs::remove_dir_all(frames_dir(project_dir)).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Export integration: sealed manifest → ffconcat timeline
// ---------------------------------------------------------------------------

/// If a sealed manifest exists and matches this export snapshot, return the
/// ffconcat timeline path for the ffmpeg overlay (written next to the frames).
pub fn sealed_overlay_plan(project_dir: &Path, expected_hash: &str) -> AppResult<Option<PathBuf>> {
    let dir = frames_dir(project_dir);
    let manifest_path = dir.join("manifest.json");
    let raw = match std::fs::read_to_string(&manifest_path) {
        Ok(raw) => raw,
        Err(_) => return Ok(None),
    };
    let manifest: CaptionManifest = serde_json::from_str(&raw)?;
    if manifest.hash != expected_hash {
        return Ok(None);
    }
    write_concat(&manifest, &dir).map(Some)
}

/// Build the ffconcat timeline: each segment's PNG holds for its window.
/// Gaps between segments are the shared empty frame (the frontend includes
/// them in the segment list), so the overlay covers the whole export.
pub fn write_concat(manifest: &CaptionManifest, dir: &Path) -> AppResult<PathBuf> {
    let mut concat = String::from("ffconcat version 1.0\n");
    let mut last_file: Option<&str> = None;
    for segment in &manifest.segments {
        let path = dir.join(&segment.file);
        if !path.is_file() {
            return Err(AppError::ProjectNotFound(path));
        }
        concat.push_str(&format!(
            "file '{}'\nduration {:.6}\n",
            path.to_string_lossy(),
            segment.end - segment.start
        ));
        last_file = Some(&segment.file);
    }
    // ffconcat applies `duration` to the preceding entry; repeat the last
    // file so its final window is honored.
    if let Some(last) = last_file {
        concat.push_str(&format!("file '{}'\n", dir.join(last).to_string_lossy()));
    }
    let path = dir.join("captions.ffconcat");
    std::fs::write(&path, concat)?;
    Ok(path)
}

/// Remove the frame set after an export (success, failure or cancel).
pub async fn cleanup_frames(project_dir: &Path) {
    let _ = tokio::fs::remove_dir_all(frames_dir(project_dir)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Doc, MediaRef, Meta, Paragraph, Sentence, Word};
    use chrono::Utc;

    fn fixture_doc() -> Doc {
        Doc {
            id: "p".into(),
            schema: 1,
            media: MediaRef {
                path: PathBuf::from("/tmp/x.mp4"),
                duration_seconds: 10.0,
                sample_rate: Some(16_000),
                channels: Some(1),
            },
            meta: Meta {
                title: "t".into(),
                description: String::new(),
                language: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            paragraphs: vec![Paragraph {
                id: 1,
                speaker: None,
                sentences: vec![Sentence {
                    id: "s1".into(),
                    text: "你好世界\nHello world".into(),
                    words: vec![
                        Word {
                            id: "w0".into(),
                            text: "你好".into(),
                            start: 1.0,
                            end: 1.5,
                        },
                        Word {
                            id: "w1".into(),
                            text: "世界".into(),
                            start: 1.5,
                            end: 2.0,
                        },
                    ],
                }],
            }],
            translations: Default::default(),
        }
    }

    #[test]
    fn spec_splits_bilingual_cue_and_keeps_source_word_timing() {
        let doc = fixture_doc();
        let spec = build_render_spec(&doc, &[], &SubStyle::default(), 1920, 1080)
            .unwrap()
            .unwrap();
        assert_eq!(spec.cues.len(), 1);
        let cue = &spec.cues[0];
        assert_eq!(cue.source_text, "你好世界");
        assert_eq!(cue.translation_text.as_deref(), Some("Hello world"));
        assert_eq!(cue.words.len(), 2);
        assert_eq!(cue.words[0].start, 1.0);
    }

    #[test]
    fn spec_drops_word_timing_when_text_is_not_the_source() {
        // Translation-only export: sentence text is the translation but the
        // words still carry source-language timing → must be dropped.
        let mut doc = fixture_doc();
        doc.paragraphs[0].sentences[0].text = "Hello world".into();
        let spec = build_render_spec(&doc, &[], &SubStyle::default(), 1920, 1080)
            .unwrap()
            .unwrap();
        assert!(spec.cues[0].words.is_empty());
    }

    #[test]
    fn hash_changes_with_style_and_canvas() {
        let doc = fixture_doc();
        let base = caption_content_hash(&doc, &[], &SubStyle::default(), 1920, 1080).unwrap();
        let bigger = caption_content_hash(&doc, &[], &SubStyle::default(), 1080, 1920).unwrap();
        let styled = caption_content_hash(
            &doc,
            &[],
            &SubStyle {
                caption_preset: Some("em-yellow".into()),
                ..Default::default()
            },
            1920,
            1080,
        )
        .unwrap();
        assert_ne!(base, bigger);
        assert_ne!(base, styled);
        assert_eq!(
            base,
            caption_content_hash(&doc, &[], &SubStyle::default(), 1920, 1080).unwrap()
        );
    }

    #[test]
    fn concat_lists_each_segment_with_its_duration() {
        let dir = std::env::temp_dir().join(format!("caption-frames-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.png"), b"\x89PNG").unwrap();
        std::fs::write(dir.join("b.png"), b"\x89PNG").unwrap();
        let manifest = CaptionManifest {
            version: 1,
            hash: "h".into(),
            width: 1920,
            height: 1080,
            duration: 3.0,
            segments: vec![
                CaptionManifestSegment {
                    file: "a.png".into(),
                    start: 0.0,
                    end: 1.25,
                },
                CaptionManifestSegment {
                    file: "b.png".into(),
                    start: 1.25,
                    end: 3.0,
                },
            ],
        };
        let path = write_concat(&manifest, &dir).unwrap();
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("duration 1.250000"));
        assert!(content.contains("duration 1.750000"));
        // Last file repeated so ffconcat honors its window.
        assert_eq!(content.matches("b.png").count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
