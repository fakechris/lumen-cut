use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExportContainer {
    Mp4,
    Mov,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExportVideoCodec {
    H264,
    Hevc,
    Prores,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExportResolution {
    #[serde(rename = "source")]
    Source,
    #[serde(rename = "720p")]
    Hd720,
    #[serde(rename = "1080p")]
    FullHd1080,
    #[serde(rename = "4k")]
    Uhd4k,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExportAspectRatio {
    #[default]
    #[serde(rename = "source")]
    Source,
    #[serde(rename = "16:9")]
    Landscape16x9,
    #[serde(rename = "9:16")]
    Portrait9x16,
    #[serde(rename = "1:1")]
    Square1x1,
    #[serde(rename = "4:5")]
    Portrait4x5,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExportCanvasFit {
    #[default]
    Contain,
    Cover,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExportSubtitleMode {
    Burn,
    Soft,
    None,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExportAudioCodec {
    Aac,
    Pcm,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExportEncodingSpeed {
    Fast,
    Quality,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct VideoExportSettings {
    pub container: ExportContainer,
    pub video_codec: ExportVideoCodec,
    pub resolution: ExportResolution,
    pub aspect_ratio: ExportAspectRatio,
    pub canvas_fit: ExportCanvasFit,
    pub subtitle_mode: ExportSubtitleMode,
    pub subtitle_language: Option<String>,
    pub bilingual_subtitles: bool,
    pub audio_codec: ExportAudioCodec,
    pub encoding_speed: ExportEncodingSpeed,
}

impl Default for VideoExportSettings {
    fn default() -> Self {
        Self {
            container: ExportContainer::Mp4,
            video_codec: ExportVideoCodec::H264,
            resolution: ExportResolution::Source,
            aspect_ratio: ExportAspectRatio::Source,
            canvas_fit: ExportCanvasFit::Contain,
            subtitle_mode: ExportSubtitleMode::Burn,
            subtitle_language: None,
            bilingual_subtitles: false,
            audio_codec: ExportAudioCodec::Aac,
            encoding_speed: ExportEncodingSpeed::Fast,
        }
    }
}

impl VideoExportSettings {
    pub fn validate(&self) -> AppResult<()> {
        if self.video_codec == ExportVideoCodec::Prores && self.container != ExportContainer::Mov {
            return Err(AppError::Schema(
                "ProRes export requires a MOV container".into(),
            ));
        }
        if self.audio_codec == ExportAudioCodec::Pcm && self.container != ExportContainer::Mov {
            return Err(AppError::Schema(
                "PCM audio export requires a MOV container".into(),
            ));
        }
        if let Some(language) = self.subtitle_language.as_deref() {
            if language.trim().is_empty()
                || language.len() > 64
                || !language.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
                })
            {
                return Err(AppError::Schema(
                    "subtitle translation language must be a short language tag".into(),
                ));
            }
        }
        if self.bilingual_subtitles && self.subtitle_language.is_none() {
            return Err(AppError::Schema(
                "bilingual subtitles require a translation language".into(),
            ));
        }
        Ok(())
    }

    pub const fn extension(&self) -> &'static str {
        match self.container {
            ExportContainer::Mp4 => "mp4",
            ExportContainer::Mov => "mov",
        }
    }

    /// Resolve the final picture size from the selected quality tier and
    /// canvas ratio. A quality tier represents the shorter canvas edge, so
    /// 1080p is 1920×1080 for landscape and 1080×1920 for portrait.
    ///
    /// `None` means the source stream can pass through without a canvas
    /// transform. This is deliberately limited to the fully source-matched
    /// case; selecting a fixed ratio always produces an explicit canvas.
    pub fn target_dimensions(&self, source_dimensions: Option<(u32, u32)>) -> Option<(u32, u32)> {
        if self.resolution == ExportResolution::Source
            && self.aspect_ratio == ExportAspectRatio::Source
        {
            return None;
        }
        let short_edge = match self.resolution {
            ExportResolution::Source => source_dimensions
                .map(|(width, height)| width.min(height))
                .unwrap_or(1080),
            ExportResolution::Hd720 => 720,
            ExportResolution::FullHd1080 => 1080,
            ExportResolution::Uhd4k => 2160,
        };
        let (ratio_width, ratio_height) = match self.aspect_ratio {
            ExportAspectRatio::Source => source_dimensions
                .filter(|(width, height)| *width > 0 && *height > 0)
                .unwrap_or((16, 9)),
            ExportAspectRatio::Landscape16x9 => (16, 9),
            ExportAspectRatio::Portrait9x16 => (9, 16),
            ExportAspectRatio::Square1x1 => (1, 1),
            ExportAspectRatio::Portrait4x5 => (4, 5),
        };
        Some(dimensions_for_ratio(ratio_width, ratio_height, short_edge))
    }

    /// Compatibility helper for callers that do not have media metadata.
    pub fn dimensions(&self) -> Option<(u32, u32)> {
        self.target_dimensions(None)
    }

    /// ASS uses the same coordinate system as the final picture. If source
    /// metadata is unavailable, use a 1080-quality canonical canvas rather
    /// than silently falling back to landscape for a portrait project.
    pub fn subtitle_canvas_dimensions(&self, source_dimensions: Option<(u32, u32)>) -> (u32, u32) {
        self.target_dimensions(source_dimensions)
            .or(source_dimensions)
            .unwrap_or_else(|| {
                let (width, height) = match self.aspect_ratio {
                    ExportAspectRatio::Portrait9x16 => (9, 16),
                    ExportAspectRatio::Square1x1 => (1, 1),
                    ExportAspectRatio::Portrait4x5 => (4, 5),
                    ExportAspectRatio::Source | ExportAspectRatio::Landscape16x9 => (16, 9),
                };
                dimensions_for_ratio(width, height, 1080)
            })
    }

    pub const fn legacy_mode(&self) -> &'static str {
        match self.encoding_speed {
            ExportEncodingSpeed::Fast => "fast",
            ExportEncodingSpeed::Quality => "quality",
        }
    }
}

fn dimensions_for_ratio(ratio_width: u32, ratio_height: u32, short_edge: u32) -> (u32, u32) {
    let short_edge = round_even(short_edge.max(2));
    if ratio_width >= ratio_height {
        (
            round_even(
                (f64::from(short_edge) * f64::from(ratio_width) / f64::from(ratio_height)).round()
                    as u32,
            ),
            short_edge,
        )
    } else {
        (
            short_edge,
            round_even(
                (f64::from(short_edge) * f64::from(ratio_height) / f64::from(ratio_width)).round()
                    as u32,
            ),
        )
    }
}

const fn round_even(value: u32) -> u32 {
    if value.is_multiple_of(2) {
        value
    } else {
        value + 1
    }
}

pub fn load(project_dir: &Path) -> AppResult<VideoExportSettings> {
    let path = project_dir.join("export-settings.json");
    if !path.exists() {
        return Ok(VideoExportSettings::default());
    }
    let settings: VideoExportSettings = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    settings.validate()?;
    Ok(settings)
}

pub fn save(project_dir: &Path, settings: &VideoExportSettings) -> AppResult<()> {
    settings.validate()?;
    crate::data::storage::write_json(&project_dir.join("export-settings.json"), settings)
}

pub fn project_caption_doc(
    doc: &crate::data::Doc,
    language: Option<&str>,
    bilingual: bool,
) -> AppResult<crate::data::Doc> {
    project_caption_doc_with_hidden(doc, language, bilingual, &std::collections::BTreeSet::new())
}

pub fn project_caption_doc_with_hidden(
    doc: &crate::data::Doc,
    language: Option<&str>,
    bilingual: bool,
    hidden: &std::collections::BTreeSet<String>,
) -> AppResult<crate::data::Doc> {
    let Some(language) = language else {
        if bilingual {
            return Err(AppError::Schema(
                "bilingual subtitles require a translation language".into(),
            ));
        }
        let mut projected = doc.clone();
        for sentence in projected
            .paragraphs
            .iter_mut()
            .flat_map(|paragraph| paragraph.sentences.iter_mut())
        {
            if hidden.contains(&sentence.id) {
                sentence.text.clear();
            }
        }
        return Ok(projected);
    };
    let mut projected = doc.clone();
    let has_visible_captions = projected
        .paragraphs
        .iter()
        .flat_map(|paragraph| paragraph.sentences.iter())
        .any(|sentence| !sentence.words.is_empty() && !hidden.contains(&sentence.id));
    let track = match doc.translations.get(language) {
        Some(track) => Some(track),
        None if has_visible_captions => {
            return Err(AppError::Schema(format!(
                "translation track `{language}` does not exist; translate the project before exporting"
            )));
        }
        None => None,
    };
    let mut missing = Vec::new();
    let mut stale = Vec::new();
    for paragraph in &mut projected.paragraphs {
        for sentence in &mut paragraph.sentences {
            if sentence.words.is_empty() {
                continue;
            }
            if hidden.contains(&sentence.id) {
                sentence.text.clear();
                continue;
            }
            let Some(translation) = track.and_then(|track| track.get(&sentence.id)) else {
                missing.push(sentence.id.clone());
                continue;
            };
            if translation.text.trim().is_empty() {
                missing.push(sentence.id.clone());
                continue;
            }
            if translation
                .source_text
                .as_deref()
                .is_some_and(|source| source.trim() != sentence.text.trim())
            {
                stale.push(sentence.id.clone());
                continue;
            }
            sentence.text = if bilingual {
                format!("{}\n{}", sentence.text.trim(), translation.text.trim())
            } else {
                translation.text.trim().to_string()
            };
        }
    }
    if !missing.is_empty() {
        return Err(AppError::Schema(format!(
            "translation track `{language}` is missing {} subtitle line(s), including {}",
            missing.len(),
            missing
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    if !stale.is_empty() {
        return Err(AppError::Schema(format!(
            "translation track `{language}` has {} outdated subtitle line(s), including {}; retranslate them before exporting",
            stale.len(),
            stale.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
        )));
    }
    projected.meta.language = Some(language.to_string());
    Ok(projected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Doc, MediaRef, Meta, Paragraph, Sentence, TranslationGroup, Word};

    fn translated_doc() -> Doc {
        let mut translations = std::collections::BTreeMap::new();
        translations.insert(
            "zh-Hans".into(),
            std::collections::BTreeMap::from([(
                "s1".into(),
                TranslationGroup {
                    id: "s1".into(),
                    text: "你好，世界。".into(),
                    source_words: vec!["w1".into()],
                    source_text: Some("Hello world.".into()),
                },
            )]),
        );
        Doc {
            id: "demo".into(),
            schema: 1,
            media: MediaRef {
                path: "/tmp/source.mp4".into(),
                duration_seconds: 1.0,
                sample_rate: Some(48_000),
                channels: Some(2),
            },
            meta: Meta {
                title: "Demo".into(),
                description: String::new(),
                language: Some("en".into()),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            paragraphs: vec![Paragraph {
                id: 1,
                speaker: None,
                sentences: vec![Sentence {
                    id: "s1".into(),
                    text: "Hello world.".into(),
                    words: vec![Word {
                        id: "w1".into(),
                        text: "Hello".into(),
                        start: 0.0,
                        end: 0.8,
                    }],
                }],
            }],
            translations,
        }
    }

    #[test]
    fn defaults_are_backward_compatible_with_the_existing_mp4_export() {
        let settings = VideoExportSettings::default();
        assert_eq!(settings.extension(), "mp4");
        assert_eq!(settings.legacy_mode(), "fast");
        assert_eq!(settings.dimensions(), None);
        let json = serde_json::to_value(settings).unwrap();
        assert_eq!(json["videoCodec"], "h264");
        assert_eq!(json["aspectRatio"], "source");
        assert_eq!(json["canvasFit"], "contain");
        assert_eq!(json["subtitleMode"], "burn");
        assert_eq!(json["encodingSpeed"], "fast");
    }

    #[test]
    fn settings_created_before_canvas_controls_keep_source_framing() {
        let legacy = serde_json::json!({
            "container": "mp4",
            "videoCodec": "h264",
            "resolution": "source",
            "subtitleMode": "burn",
            "subtitleLanguage": null,
            "bilingualSubtitles": false,
            "audioCodec": "aac",
            "encodingSpeed": "fast"
        });
        let settings: VideoExportSettings = serde_json::from_value(legacy).unwrap();
        assert_eq!(settings.aspect_ratio, ExportAspectRatio::Source);
        assert_eq!(settings.canvas_fit, ExportCanvasFit::Contain);
        assert_eq!(settings.target_dimensions(Some((1080, 1920))), None);
    }

    #[test]
    fn canvas_dimensions_follow_orientation_and_quality_tier() {
        let portrait = VideoExportSettings {
            aspect_ratio: ExportAspectRatio::Portrait9x16,
            resolution: ExportResolution::FullHd1080,
            ..Default::default()
        };
        assert_eq!(
            portrait.target_dimensions(Some((1920, 1080))),
            Some((1080, 1920))
        );

        let square_at_source_quality = VideoExportSettings {
            aspect_ratio: ExportAspectRatio::Square1x1,
            ..Default::default()
        };
        assert_eq!(
            square_at_source_quality.target_dimensions(Some((3840, 2160))),
            Some((2160, 2160))
        );

        let source_portrait = VideoExportSettings {
            resolution: ExportResolution::Hd720,
            ..Default::default()
        };
        assert_eq!(
            source_portrait.target_dimensions(Some((1080, 1920))),
            Some((720, 1280))
        );
        assert_eq!(
            VideoExportSettings::default().target_dimensions(Some((1080, 1920))),
            None
        );
    }

    #[test]
    fn incompatible_container_combinations_are_rejected() {
        assert!(VideoExportSettings {
            video_codec: ExportVideoCodec::Prores,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(VideoExportSettings {
            audio_codec: ExportAudioCodec::Pcm,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(VideoExportSettings {
            container: ExportContainer::Mov,
            video_codec: ExportVideoCodec::Prores,
            audio_codec: ExportAudioCodec::Pcm,
            ..Default::default()
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn settings_round_trip_in_the_project_without_entering_edit_history() {
        let dir = tempfile::tempdir().unwrap();
        let settings = VideoExportSettings {
            container: ExportContainer::Mov,
            video_codec: ExportVideoCodec::Hevc,
            resolution: ExportResolution::Uhd4k,
            aspect_ratio: ExportAspectRatio::Portrait4x5,
            canvas_fit: ExportCanvasFit::Cover,
            subtitle_mode: ExportSubtitleMode::Soft,
            subtitle_language: Some("zh-Hans".into()),
            bilingual_subtitles: true,
            audio_codec: ExportAudioCodec::Pcm,
            encoding_speed: ExportEncodingSpeed::Quality,
        };
        save(dir.path(), &settings).unwrap();
        assert_eq!(load(dir.path()).unwrap(), settings);
    }

    #[test]
    fn caption_projection_preserves_timing_for_translation_and_bilingual_delivery() {
        let doc = translated_doc();
        let translated = project_caption_doc(&doc, Some("zh-Hans"), false).unwrap();
        assert_eq!(translated.paragraphs[0].sentences[0].text, "你好，世界。");
        assert_eq!(
            translated.paragraphs[0].sentences[0].words,
            doc.paragraphs[0].sentences[0].words
        );

        let bilingual = project_caption_doc(&doc, Some("zh-Hans"), true).unwrap();
        assert_eq!(
            bilingual.paragraphs[0].sentences[0].text,
            "Hello world.\n你好，世界。"
        );
    }

    #[test]
    fn caption_projection_rejects_missing_and_stale_translation_lines() {
        let mut doc = translated_doc();
        doc.translations
            .get_mut("zh-Hans")
            .unwrap()
            .get_mut("s1")
            .unwrap()
            .source_text = Some("Old source".into());
        assert!(project_caption_doc(&doc, Some("zh-Hans"), false)
            .unwrap_err()
            .to_string()
            .contains("outdated"));
        doc.translations.get_mut("zh-Hans").unwrap().clear();
        assert!(project_caption_doc(&doc, Some("zh-Hans"), false)
            .unwrap_err()
            .to_string()
            .contains("missing 1"));
    }

    #[test]
    fn hidden_captions_do_not_require_translation_and_keep_timing_words() {
        let mut doc = translated_doc();
        doc.translations.clear();
        let hidden = std::collections::BTreeSet::from(["s1".to_string()]);
        let projected =
            project_caption_doc_with_hidden(&doc, Some("zh-Hans"), false, &hidden).unwrap();
        assert!(projected.paragraphs[0].sentences[0].text.is_empty());
        assert_eq!(
            projected.paragraphs[0].sentences[0].words,
            doc.paragraphs[0].sentences[0].words
        );
        assert!(crate::export::to_srt(&projected).is_empty());
        assert!(!crate::export::to_ass(&projected, 1920, 1080).contains("Dialogue: 0,"));
    }
}
