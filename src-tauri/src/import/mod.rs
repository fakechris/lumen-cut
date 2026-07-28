//! "Import from Navi/Voice" — `lumen-transcript.v1` interchange → project.
//!
//! Consumes the suite-wide transcript exchange format (shared serde types in
//! the `lumen-transcript` crate, canonical schema in
//! `lumen-suite/contracts/lumen-transcript.v1.schema.json`) and builds a
//! `Doc` directly, skipping the ASR sidecar entirely.
//!
//! Mapping decisions (per `lumen-suite/contracts/TRANSCRIPT.md` §2.2/§4):
//! - Imported segment boundaries are authoritative: each segment becomes one
//!   `Sentence` (one subtitle cue); text is never re-segmented here.
//! - Consecutive segments with the same resolved speaker merge into one
//!   `Paragraph`. Speaker-less runs split on silences longer than
//!   [`SPEAKERLESS_PARAGRAPH_GAP`] so a plain navi export does not collapse
//!   into a single paragraph.
//! - `paragraph.speaker` stores the display string (`display_name ?? id`);
//!   cut's rename/merge tooling operates on that string directly.
//! - Segments without word timing get one synthetic word spanning
//!   `[start, end]` (word-level highlight degrades to whole-sentence).
//! - Per-segment translations land in `doc.translations[lang][sentence_id]`
//!   with `source_words` empty and `source_text` set, so staleness detection
//!   degrades to text comparison.
//! - `confidence` is dropped (the cut doc has no slot for it); `provenance`
//!   is preserved verbatim under the `importProvenance` top-level key, which
//!   cut's unknown-field retention carries across saves.

use std::path::{Path, PathBuf};

use chrono::Utc;
use lumen_transcript::TranscriptV1;

use crate::data::{Doc, MediaRef, Meta, Paragraph, Sentence, TranslationGroup, Word};
use crate::error::{AppError, AppResult};

/// Silence gap (seconds) that starts a new paragraph when neither the current
/// nor the previous segment carries a speaker label.
pub const SPEAKERLESS_PARAGRAPH_GAP: f64 = 2.0;

/// Convert a parsed interchange document into a cut project document.
///
/// The returned doc has a fresh id, an empty title, and an unbound media
/// reference (the declared `media.path`, if any, is copied but not checked);
/// [`import_transcript_file`] finalizes identity and media binding.
pub fn doc_from_transcript(transcript: &TranscriptV1) -> AppResult<Doc> {
    let display_name = |speaker: Option<&str>| -> Option<String> {
        let id = speaker?;
        let named = transcript
            .speakers
            .iter()
            .flatten()
            .find(|entry| entry.id == id)
            .and_then(|entry| entry.display_name.as_deref());
        Some(named.unwrap_or(id).to_string())
    };

    let segments: Vec<_> = transcript
        .segments
        .iter()
        .filter(|segment| !segment.text.trim().is_empty())
        .collect();
    if segments.is_empty() {
        return Err(AppError::Schema(
            "transcript has no text segments to import (speaker-only documents cannot become a project)"
                .into(),
        ));
    }

    let mut paragraphs: Vec<Paragraph> = Vec::new();
    let mut translations: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, TranslationGroup>,
    > = Default::default();
    let mut word_index = 0usize;
    let mut previous_end: Option<f64> = None;

    for (index, segment) in segments.iter().enumerate() {
        ensure_span(segment.start, segment.end, || format!("segment {index}"))?;
        let speaker = display_name(segment.speaker.as_deref());
        let starts_paragraph = match paragraphs.last() {
            None => true,
            Some(previous) => {
                previous.speaker != speaker
                    || (speaker.is_none()
                        && previous_end
                            .is_some_and(|end| segment.start - end > SPEAKERLESS_PARAGRAPH_GAP))
            }
        };
        if starts_paragraph {
            paragraphs.push(Paragraph {
                id: paragraphs.len() as u32 + 1,
                speaker,
                sentences: Vec::new(),
            });
        }
        let paragraph = paragraphs.last_mut().expect("paragraph pushed above");
        let sentence_id = format!("p{}s{}", paragraph.id, paragraph.sentences.len() + 1);

        let words = match segment.words.as_deref().filter(|words| !words.is_empty()) {
            Some(words) => words
                .iter()
                .enumerate()
                .map(|(wi, word)| {
                    ensure_span(word.start, word.end, || {
                        format!("segment {index} word {wi}")
                    })?;
                    let id = format!("w{word_index}");
                    word_index += 1;
                    Ok(Word {
                        id,
                        text: word.word.clone(),
                        start: word.start,
                        end: word.end,
                    })
                })
                .collect::<AppResult<Vec<_>>>()?,
            None => {
                // No word timing: one synthetic word spanning the segment so
                // everything downstream that derives time from words keeps
                // working (highlighting degrades to whole-sentence).
                let id = format!("w{word_index}");
                word_index += 1;
                vec![Word {
                    id,
                    text: segment.text.clone(),
                    start: segment.start,
                    end: segment.end,
                }]
            }
        };

        for (lang, text) in segment.translations.iter().flatten() {
            translations.entry(lang.clone()).or_default().insert(
                sentence_id.clone(),
                TranslationGroup {
                    id: sentence_id.clone(),
                    text: text.clone(),
                    source_words: Vec::new(),
                    source_text: Some(segment.text.clone()),
                },
            );
        }

        paragraph.sentences.push(Sentence {
            id: sentence_id,
            text: segment.text.clone(),
            words,
        });
        previous_end = Some(segment.end);
    }

    let media = transcript.media.as_ref();
    let declared_duration = media
        .and_then(|media| media.duration_seconds)
        .filter(|duration| duration.is_finite() && *duration > 0.0);
    let duration_seconds = declared_duration.unwrap_or_else(|| {
        segments
            .iter()
            .map(|segment| segment.end)
            .fold(0.0, f64::max)
    });

    Ok(Doc {
        id: uuid::Uuid::new_v4().to_string(),
        schema: 1,
        media: MediaRef {
            path: media
                .and_then(|media| media.path.as_deref())
                .map(PathBuf::from)
                .unwrap_or_default(),
            duration_seconds,
            sample_rate: media.and_then(|media| media.sample_rate),
            channels: media.and_then(|media| media.channels),
        },
        meta: Meta {
            title: String::new(),
            description: String::new(),
            language: transcript
                .provenance
                .as_ref()
                .and_then(|provenance| provenance.language.clone()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
        paragraphs,
        translations,
    })
}

fn ensure_span(start: f64, end: f64, what: impl Fn() -> String) -> AppResult<()> {
    if !start.is_finite() || !end.is_finite() || end < start {
        return Err(AppError::Schema(format!(
            "{}: invalid time span [{start}, {end}] (need finite seconds with end >= start)",
            what()
        )));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// Path of the `lumen-transcript.v1` JSON file.
    pub transcript: PathBuf,
    /// Bind this media file instead of the transcript's `media.path`.
    pub media: Option<PathBuf>,
    /// Project id (also the default title).
    pub pid: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOutcome {
    pub pid: String,
    pub dir: PathBuf,
    /// Media path recorded in the project (declared path when unbound).
    pub media_path: Option<PathBuf>,
    /// Whether the media file was found and probed. When `false` the project
    /// is created in the pending-relink state the existing
    /// `project_media_status` / `project_media_relink` flow already handles.
    pub media_bound: bool,
    pub media_issue: Option<String>,
    pub paragraphs: usize,
    pub sentences: usize,
    pub words: usize,
    pub speakers: Vec<String>,
    pub translation_languages: Vec<String>,
    pub duration_seconds: f64,
    pub language: Option<String>,
    pub timing_repairs: usize,
}

/// Import an interchange file as a new project at `dir` (no ASR run).
///
/// `dir` must not already contain a project; media binding follows
/// TRANSCRIPT.md §5: existing file → bind via ffprobe, missing file → create
/// the project pending relink.
pub async fn import_transcript_file(
    dir: &Path,
    options: ImportOptions,
) -> AppResult<ImportOutcome> {
    let raw = tokio::fs::read_to_string(&options.transcript)
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => AppError::ProjectNotFound(options.transcript.clone()),
            _ => AppError::from(error),
        })?;
    let transcript = TranscriptV1::from_json_str(&raw).map_err(|error| {
        AppError::Schema(format!(
            "{} is not a valid lumen-transcript.v1 document: {error}",
            options.transcript.display()
        ))
    })?;

    if dir.join("doc.json").exists() {
        return Err(AppError::Schema(format!(
            "project already exists at {}; choose another --pid",
            dir.display()
        )));
    }

    let mut doc = doc_from_transcript(&transcript)?;
    doc.id.clone_from(&options.pid);
    doc.meta.title = options.title.clone().unwrap_or_else(|| options.pid.clone());

    let (media_bound, media_issue) = bind_media(&mut doc, options.media.as_deref()).await?;
    let repairs = crate::pipeline::timing::repair(&mut doc);
    doc.save(dir)?;
    persist_import_provenance(dir, &transcript)?;

    Ok(ImportOutcome {
        pid: options.pid,
        dir: dir.to_path_buf(),
        media_path: (!doc.media.path.as_os_str().is_empty()).then(|| doc.media.path.clone()),
        media_bound,
        media_issue,
        paragraphs: doc.paragraphs.len(),
        sentences: doc
            .paragraphs
            .iter()
            .map(|paragraph| paragraph.sentences.len())
            .sum(),
        words: doc.all_words().len(),
        speakers: {
            let mut speakers: Vec<String> = doc
                .paragraphs
                .iter()
                .filter_map(|paragraph| paragraph.speaker.clone())
                .collect();
            speakers.sort();
            speakers.dedup();
            speakers
        },
        translation_languages: doc.translations.keys().cloned().collect(),
        duration_seconds: doc.media.duration_seconds,
        language: doc.meta.language.clone(),
        timing_repairs: repairs.total(),
    })
}

/// Resolve the media binding: an explicit override must exist; a declared
/// transcript path is bound when present on disk and left for the existing
/// relink flow otherwise.
async fn bind_media(
    doc: &mut Doc,
    media_override: Option<&Path>,
) -> AppResult<(bool, Option<String>)> {
    let (candidate, explicit) = match media_override {
        Some(path) => (Some(path.to_path_buf()), true),
        None => (
            (!doc.media.path.as_os_str().is_empty()).then(|| doc.media.path.clone()),
            false,
        ),
    };
    let Some(candidate) = candidate else {
        return Ok((
            false,
            Some("transcript references no media file; relink one in the app".into()),
        ));
    };
    if !candidate.is_file() {
        if explicit {
            return Err(AppError::ProjectNotFound(candidate));
        }
        let issue = format!(
            "media file {} is missing; the project was created pending relink",
            candidate.display()
        );
        doc.media.path = candidate;
        return Ok((false, Some(issue)));
    }

    let media_path = tokio::fs::canonicalize(&candidate).await?;
    let info = crate::media::probe(&media_path).await?;
    if !info.duration_seconds.is_finite() || info.duration_seconds <= 0.0 {
        return Err(AppError::Schema(format!(
            "{} has no readable audio or video duration",
            media_path.display()
        )));
    }
    // Same tolerance as `project_media_relink`: a grossly different file is
    // almost certainly the wrong one, and every timestamp would be off.
    let expected = doc.media.duration_seconds;
    let difference = (info.duration_seconds - expected).abs();
    let tolerance = (expected * 0.02).max(2.0);
    if expected > 0.0 && difference > tolerance {
        return Err(AppError::Schema(format!(
            "media duration differs from the transcript by {difference:.1}s (expected {expected:.1}s, found {:.1}s); pass the original media or an equivalent copy",
            info.duration_seconds
        )));
    }
    doc.media = MediaRef {
        path: media_path,
        duration_seconds: info.duration_seconds,
        sample_rate: info.sample_rate,
        channels: info.channels,
    };
    Ok((true, None))
}

/// Keep the interchange provenance on the project file (`importProvenance`).
/// `Doc::save` retains unknown top-level keys, so this survives later edits.
fn persist_import_provenance(dir: &Path, transcript: &TranscriptV1) -> AppResult<()> {
    let Some(provenance) = &transcript.provenance else {
        return Ok(());
    };
    let path = dir.join("doc.json");
    let mut value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
    if let Some(object) = value.as_object_mut() {
        object.insert("importProvenance".into(), serde_json::to_value(provenance)?);
        crate::data::storage::write(&path, serde_json::to_string_pretty(&value)?.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_transcript::{Media, Provenance, Segment, Speaker, Word as TranscriptWord};

    fn meeting_transcript() -> TranscriptV1 {
        TranscriptV1::new(vec![
            Segment::new(0.32, 2.9, "大家好，我们开始今天的站会。")
                .with_id("seg-1")
                .with_speaker("SPEAKER_00")
                .with_confidence(0.94)
                .with_words(vec![
                    TranscriptWord::new("大家", 0.32, 0.78),
                    TranscriptWord::new("好", 0.78, 1.02),
                    TranscriptWord::new("我们", 1.35, 1.7),
                    TranscriptWord::new("开始", 1.7, 2.1),
                    TranscriptWord::new("今天", 2.1, 2.45),
                    TranscriptWord::new("的", 2.45, 2.55),
                    TranscriptWord::new("站会", 2.55, 2.9),
                ])
                .with_translation("en", "Hi everyone, let's start today's standup."),
            Segment::new(3.4, 7.1, "好的，我先说，昨天把导入功能收尾了。")
                .with_id("seg-2")
                .with_speaker("SPEAKER_01")
                .with_words(vec![
                    TranscriptWord::new("好的", 3.4, 3.8).with_confidence(0.97),
                    TranscriptWord::new("我先说", 4.05, 4.62),
                    TranscriptWord::new("昨天把导入功能收尾了", 5.0, 7.1),
                ]),
        ])
        .with_provenance(Provenance {
            language: Some("zh".into()),
            ..Provenance::new("lumen-cut")
        })
        .with_media(Media {
            path: Some("/nonexistent/standup-2026-07-25.mp4".into()),
            duration_seconds: Some(7.2),
            sample_rate: Some(44_100),
            channels: Some(2),
            ..Media::default()
        })
        .with_speakers(vec![
            Speaker::new("SPEAKER_00").with_display_name("Alice"),
            Speaker::new("SPEAKER_01"),
        ])
    }

    #[test]
    fn word_timing_is_mapped_verbatim_with_global_word_ids() {
        let doc = doc_from_transcript(&meeting_transcript()).unwrap();
        let words = doc.all_words();
        assert_eq!(words.len(), 10);
        assert_eq!(words[0].id, "w0");
        assert_eq!(words[9].id, "w9");
        assert_eq!((words[0].text.as_str(), words[0].start), ("大家", 0.32));
        assert_eq!((words[9].start, words[9].end), (5.0, 7.1));
        // One sentence per segment: imported boundaries are authoritative.
        let sentences: Vec<_> = doc
            .paragraphs
            .iter()
            .flat_map(|paragraph| paragraph.sentences.iter())
            .collect();
        assert_eq!(sentences.len(), 2);
        assert_eq!(sentences[0].id, "p1s1");
        assert_eq!(sentences[1].id, "p2s1");
    }

    #[test]
    fn missing_word_timing_synthesizes_a_sentence_spanning_word() {
        let transcript = TranscriptV1::from_timed_texts([
            (0.0, 30.0, "今天先对一下上周的进展。"),
            (30.0, 61.5, "存储层的迁移已经完成。"),
        ]);
        let doc = doc_from_transcript(&transcript).unwrap();
        let words = doc.all_words();
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "今天先对一下上周的进展。");
        assert_eq!((words[0].start, words[0].end), (0.0, 30.0));
        assert_eq!((words[1].start, words[1].end), (30.0, 61.5));
        // Contiguous speaker-less segments stay in one paragraph.
        assert_eq!(doc.paragraphs.len(), 1);
        // Duration falls back to max segment end when media has none.
        assert_eq!(doc.media.duration_seconds, 61.5);
    }

    #[test]
    fn speakerless_segments_split_on_long_silence() {
        let transcript =
            TranscriptV1::from_timed_texts([(0.0, 5.0, "第一段。"), (9.0, 12.0, "第二段。")]);
        let doc = doc_from_transcript(&transcript).unwrap();
        assert_eq!(doc.paragraphs.len(), 2);
    }

    #[test]
    fn speaker_display_name_falls_back_to_id_and_groups_consecutive_runs() {
        let mut transcript = meeting_transcript();
        // A third segment continuing SPEAKER_01 must join its paragraph.
        transcript
            .segments
            .push(Segment::new(7.1, 8.0, "补充一句。").with_speaker("SPEAKER_01"));
        let doc = doc_from_transcript(&transcript).unwrap();
        assert_eq!(doc.paragraphs.len(), 2);
        assert_eq!(doc.paragraphs[0].speaker.as_deref(), Some("Alice"));
        // No display_name in the speaker table → the raw id is the label.
        assert_eq!(doc.paragraphs[1].speaker.as_deref(), Some("SPEAKER_01"));
        assert_eq!(doc.paragraphs[1].sentences.len(), 2);
        assert_eq!(doc.paragraphs[1].sentences[1].id, "p2s2");
    }

    #[test]
    fn translations_fill_groups_with_source_text_and_empty_source_words() {
        let doc = doc_from_transcript(&meeting_transcript()).unwrap();
        let group = &doc.translations["en"]["p1s1"];
        assert_eq!(group.text, "Hi everyone, let's start today's standup.");
        assert!(group.source_words.is_empty());
        assert_eq!(
            group.source_text.as_deref(),
            Some("大家好，我们开始今天的站会。")
        );
        assert!(!doc.translations["en"].contains_key("p2s1"));
    }

    #[test]
    fn wrong_schema_and_invalid_spans_are_rejected() {
        let err = TranscriptV1::from_json_str(r#"{"schema":"lumen-transcript.v2","segments":[]}"#)
            .unwrap_err();
        assert!(err.to_string().contains("lumen-transcript.v1"));
        assert!(TranscriptV1::from_json_str(r#"{"segments":[]}"#).is_err());

        let inverted = TranscriptV1::new(vec![Segment::new(5.0, 1.0, "倒置")]);
        assert!(doc_from_transcript(&inverted).is_err());
        let empty = TranscriptV1::new(vec![Segment::new(0.0, 1.0, "  ")]);
        assert!(doc_from_transcript(&empty).is_err());
    }

    #[tokio::test]
    async fn import_with_missing_media_creates_a_pending_relink_project() {
        let root = tempfile::tempdir().unwrap();
        let transcript_path = root.path().join("meeting.lumen-transcript.json");
        std::fs::write(
            &transcript_path,
            meeting_transcript().to_json_string_pretty().unwrap(),
        )
        .unwrap();

        let dir = root.path().join("meeting");
        let outcome = import_transcript_file(
            &dir,
            ImportOptions {
                transcript: transcript_path,
                media: None,
                pid: "meeting".into(),
                title: None,
            },
        )
        .await
        .unwrap();

        assert!(!outcome.media_bound);
        assert!(outcome.media_issue.as_deref().unwrap().contains("missing"));
        assert_eq!(outcome.paragraphs, 2);
        assert_eq!(outcome.words, 10);
        assert_eq!(outcome.speakers, vec!["Alice", "SPEAKER_01"]);

        let doc = Doc::load(&dir).unwrap();
        assert_eq!(doc.id, "meeting");
        // The declared path is kept so `project_media_status` can suggest a
        // relink; the media is simply unavailable until then.
        assert_eq!(
            doc.media.path,
            PathBuf::from("/nonexistent/standup-2026-07-25.mp4")
        );
        assert_eq!(doc.media.duration_seconds, 7.2);
        // Provenance is preserved for auditability.
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("doc.json")).unwrap()).unwrap();
        assert_eq!(raw["importProvenance"]["app"], "lumen-cut");
        // Flat cues sidecar is generated by the normal save path.
        assert_eq!(crate::data::cues::load(&dir).len(), 2);
    }

    #[tokio::test]
    async fn import_refuses_to_overwrite_an_existing_project() {
        let root = tempfile::tempdir().unwrap();
        let transcript_path = root.path().join("t.json");
        std::fs::write(
            &transcript_path,
            meeting_transcript().to_json_string().unwrap(),
        )
        .unwrap();
        let dir = root.path().join("meeting");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("doc.json"), "{}").unwrap();

        let error = import_transcript_file(
            &dir,
            ImportOptions {
                transcript: transcript_path,
                media: None,
                pid: "meeting".into(),
                title: None,
            },
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("already exists"));
    }

    #[tokio::test]
    async fn explicit_media_override_must_exist() {
        let root = tempfile::tempdir().unwrap();
        let transcript_path = root.path().join("t.json");
        std::fs::write(
            &transcript_path,
            meeting_transcript().to_json_string().unwrap(),
        )
        .unwrap();

        let error = import_transcript_file(
            &root.path().join("meeting"),
            ImportOptions {
                transcript: transcript_path,
                media: Some(root.path().join("nope.mp4")),
                pid: "meeting".into(),
                title: None,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AppError::ProjectNotFound(_)));
    }
}
