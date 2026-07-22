//! The on-disk project file `doc.json`.
//!
//! Public fields serialize as **camelCase**. Renamed fields also accept their
//! legacy snake_case spelling through `#[serde(alias = …)]`, so existing
//! lumen-cut projects continue to load.
//!
//! lumen-cut keeps a paragraph-grouped working model and exposes a flat
//! `cues.json` interoperability projection. When a flat cues envelope is
//! loaded, unknown top-level fields are retained byte-semantically on save so
//! forward-compatible data is not discarded.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// Top-level container. The project lives on disk as `<root>/doc.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Doc {
    pub id: String,
    pub schema: u32,
    pub media: MediaRef,
    pub meta: Meta,
    pub paragraphs: Vec<Paragraph>,
    /// `(lang, group)` keyed translation map. Optional because not every
    /// project is translated.
    #[serde(default)]
    pub translations:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, TranslationGroup>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaRef {
    pub path: PathBuf,
    #[serde(alias = "duration_seconds")]
    pub duration_seconds: f64,
    #[serde(alias = "sample_rate")]
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub language: Option<String>,
    #[serde(alias = "created_at")]
    pub created_at: DateTime<Utc>,
    #[serde(alias = "updated_at")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Paragraph {
    pub id: u32,
    pub speaker: Option<String>,
    pub sentences: Vec<Sentence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Sentence {
    pub id: String,
    pub text: String,
    pub words: Vec<Word>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Word {
    pub id: String,
    pub text: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TranslationGroup {
    pub id: String,
    pub text: String,
    /// Stamps of the source tokens in the group; used for re-translate tracking.
    #[serde(default, alias = "source_words")]
    pub source_words: Vec<String>,
    /// Source sentence text at translation time. A later polish/manual edit
    /// makes this group stale without relying on unstable word indices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_text: Option<String>,
}

impl Doc {
    /// Load a project from `<root>/doc.json`.
    pub fn load(root: &Path) -> AppResult<Self> {
        let path = root.join("doc.json");
        let raw = std::fs::read_to_string(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => AppError::ProjectNotFound(path.clone()),
            _ => AppError::from(e),
        })?;
        if let Ok(doc) = serde_json::from_str::<Doc>(&raw) {
            return Ok(doc);
        }
        load_flat_cues_compat(&raw).map_err(|error| {
            AppError::Schema(format!(
                "{} is neither a lumen-cut paragraph document nor a compatible flat cues document: {error}",
                path.display()
            ))
        })
    }

    /// Persist to `<root>/doc.json` (write-through-temp for crash safety).
    pub fn save(&self, root: &Path) -> AppResult<()> {
        std::fs::create_dir_all(root)?;
        let target = root.join("doc.json");
        let raw = serialize_preserving_flat_document(self, &target)?;
        crate::data::storage::write(&target, raw.as_bytes())?;
        let lang = self.translations.keys().next().map(String::as_str);
        crate::data::cues::save(root, &crate::data::cues::to_cues(self, lang))?;
        Ok(())
    }

    /// Flat all words (in order) for quick stats / single-pass review.
    pub fn all_words(&self) -> Vec<&Word> {
        self.paragraphs
            .iter()
            .flat_map(|p| p.sentences.iter())
            .flat_map(|s| s.words.iter())
            .collect()
    }
}

fn serialize_preserving_flat_document(doc: &Doc, target: &Path) -> AppResult<String> {
    let existing = std::fs::read_to_string(target)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
    let Some(mut value) = existing else {
        return Ok(serde_json::to_string_pretty(doc)?);
    };
    if value.get("cues").is_none() || value.get("paragraphs").is_some() {
        let generated = serde_json::to_value(doc)?;
        if let (Some(target), Some(source)) = (value.as_object_mut(), generated.as_object()) {
            // Keep native/forward-compatible top-level keys such as
            // `chapters`, while replacing every field owned by the working
            // paragraph model.
            for (key, field) in source {
                target.insert(key.clone(), field.clone());
            }
            return Ok(serde_json::to_string_pretty(&value)?);
        }
        return Ok(serde_json::to_string_pretty(doc)?);
    }
    let Some(object) = value.as_object_mut() else {
        return Ok(serde_json::to_string_pretty(doc)?);
    };

    let old_cues: std::collections::BTreeMap<String, serde_json::Value> = object
        .get("cues")
        .and_then(|cues| cues.as_array())
        .into_iter()
        .flatten()
        .filter_map(|cue| Some((cue.get("id")?.as_str()?.to_string(), cue.clone())))
        .collect();
    let lang = doc.translations.keys().next().map(String::as_str);
    let cues = crate::data::cues::to_cues(doc, lang)
        .into_iter()
        .map(|cue| {
            let id = cue.id.clone();
            let generated = serde_json::to_value(cue)?;
            let mut merged = old_cues
                .get(&id)
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            if let (Some(target), Some(source)) = (merged.as_object_mut(), generated.as_object()) {
                for (key, value) in source {
                    // Hidden state lives outside the paragraph working model,
                    // so retain the native cue's value when it already exists.
                    if key != "hidden" || !target.contains_key(key) {
                        target.insert(key.clone(), value.clone());
                    }
                }
            } else {
                merged = generated;
            }
            Ok::<_, serde_json::Error>(merged)
        })
        .collect::<Result<Vec<_>, _>>()?;
    object.insert("cues".into(), serde_json::Value::Array(cues));

    set_existing_string(object, &["id", "projectId"], &doc.id);
    set_existing_string(object, &["title"], &doc.meta.title);
    if let Some(language) = &doc.meta.language {
        set_existing_string(object, &["language"], language);
    }
    set_existing_string(object, &["mediaPath"], &doc.media.path.to_string_lossy());
    if object.contains_key("durationSeconds") {
        object.insert(
            "durationSeconds".into(),
            serde_json::json!(doc.media.duration_seconds),
        );
    }
    Ok(serde_json::to_string_pretty(&value)?)
}

fn set_existing_string(
    object: &mut serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
    value: &str,
) {
    for key in keys {
        if object.contains_key(*key) {
            object.insert((*key).into(), serde_json::Value::String(value.into()));
        }
    }
}

fn load_flat_cues_compat(raw: &str) -> Result<Doc, String> {
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|error| error.to_string())?;
    let cues_value = value
        .get("cues")
        .ok_or_else(|| "missing top-level `cues`".to_string())?;
    let cues: Vec<crate::data::cues::Cue> =
        serde_json::from_value(cues_value.clone()).map_err(|error| error.to_string())?;
    let mut doc = crate::data::cues::from_cues(&cues);
    if let Some(id) = value
        .get("id")
        .or_else(|| value.get("projectId"))
        .and_then(|field| field.as_str())
    {
        doc.id = id.to_string();
    }
    if let Some(title) = value
        .pointer("/meta/title")
        .or_else(|| value.get("title"))
        .and_then(|field| field.as_str())
    {
        doc.meta.title = title.to_string();
    }
    if let Some(language) = value
        .pointer("/meta/language")
        .or_else(|| value.get("language"))
        .and_then(|field| field.as_str())
    {
        doc.meta.language = Some(language.to_string());
    }
    if let Some(path) = value
        .pointer("/media/path")
        .or_else(|| value.get("mediaPath"))
        .and_then(|field| field.as_str())
    {
        doc.media.path = path.into();
    }
    if let Some(duration) = value
        .pointer("/media/durationSeconds")
        .or_else(|| value.get("durationSeconds"))
        .and_then(|field| field.as_f64())
    {
        doc.media.duration_seconds = duration;
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Doc {
        Doc {
            id: "p1".into(),
            schema: 1,
            media: MediaRef {
                path: PathBuf::from("/tmp/x.mp4"),
                duration_seconds: 1.0,
                sample_rate: Some(16_000),
                channels: Some(1),
            },
            meta: Meta {
                title: "t".into(),
                description: String::new(),
                language: Some("zh".into()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            paragraphs: vec![Paragraph {
                id: 1,
                speaker: None,
                sentences: vec![Sentence {
                    id: "s1".into(),
                    text: "hi".into(),
                    words: vec![Word {
                        id: "w0".into(),
                        text: "hi".into(),
                        start: 0.0,
                        end: 0.5,
                    }],
                }],
            }],
            translations: Default::default(),
        }
    }

    #[test]
    fn roundtrip_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let doc = sample();
        doc.save(dir.path()).unwrap();
        let loaded = Doc::load(dir.path()).unwrap();
        assert_eq!(doc, loaded);
    }

    #[test]
    fn serializes_camel_case() {
        let raw = serde_json::to_string(&sample()).unwrap();
        // Public files use camelCase keys, not snake_case.
        assert!(raw.contains("\"durationSeconds\""));
        assert!(raw.contains("\"sampleRate\""));
        assert!(raw.contains("\"createdAt\""));
        assert!(raw.contains("\"updatedAt\""));
        assert!(!raw.contains("duration_seconds"));
        assert!(!raw.contains("created_at"));
    }

    #[test]
    fn loads_legacy_snake_case() {
        // A doc.json written before the camelCase change must still load.
        let legacy = r#"{
            "id":"p1","schema":1,
            "media":{"path":"/tmp/x.mp4","duration_seconds":1.0,"sample_rate":16000,"channels":1},
            "meta":{"title":"t","description":"","language":"zh",
                    "created_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"},
            "paragraphs":[],
            "translations":{}
        }"#;
        let doc: Doc = serde_json::from_str(legacy).unwrap();
        assert_eq!(doc.media.duration_seconds, 1.0);
        assert_eq!(doc.media.sample_rate, Some(16_000));
    }

    #[test]
    fn loads_camel_case() {
        let modern = r#"{
            "id":"p1","schema":1,
            "media":{"path":"/tmp/x.mp4","durationSeconds":2.5,"sampleRate":16000,"channels":1},
            "meta":{"title":"t","description":"","language":"en",
                    "createdAt":"2024-01-01T00:00:00Z","updatedAt":"2024-01-01T00:00:00Z"},
            "paragraphs":[],
            "translations":{}
        }"#;
        let doc: Doc = serde_json::from_str(modern).unwrap();
        assert_eq!(doc.media.duration_seconds, 2.5);
    }

    #[test]
    fn loads_flat_cues_document_as_compatibility_input() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("doc.json"),
            r#"{
                "id":"flat-1",
                "title":"Flat",
                "language":"en",
                "mediaPath":"/tmp/flat.mp4",
                "durationSeconds":3.0,
                "cues":[{
                    "id":"s1","startMs":0,"endMs":1000,"text":"hello",
                    "translation":"你好","speaker":"A","hidden":false
                }]
            }"#,
        )
        .unwrap();
        let doc = Doc::load(dir.path()).unwrap();
        assert_eq!(doc.id, "flat-1");
        assert_eq!(doc.meta.title, "Flat");
        assert_eq!(doc.media.path, PathBuf::from("/tmp/flat.mp4"));
        assert_eq!(doc.paragraphs[0].sentences[0].text, "hello");
        assert_eq!(doc.translations["translated"]["s1"].text, "你好");
    }

    #[test]
    fn save_always_emits_flat_cues_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        sample().save(dir.path()).unwrap();
        let cues = crate::data::cues::load(dir.path());
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].id, "s1");
    }

    #[test]
    fn saving_flat_document_preserves_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("doc.json"),
            r#"{
                "id":"flat-1",
                "title":"Flat",
                "language":"en",
                "mediaPath":"/tmp/flat.mp4",
                "durationSeconds":3.0,
                "stageStamps":{"translate":"opaque"},
                "privateFutureField":{"must":"survive"},
                "cues":[{
                    "id":"s1","startMs":0,"endMs":1000,"text":"hello",
                    "speaker":"A","hidden":true,"futureCueField":42
                }]
            }"#,
        )
        .unwrap();
        let mut doc = Doc::load(dir.path()).unwrap();
        doc.paragraphs[0].sentences[0].text = "edited".into();
        doc.save(dir.path()).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("doc.json")).unwrap())
                .unwrap();
        assert!(saved.get("paragraphs").is_none());
        assert_eq!(saved["privateFutureField"]["must"], "survive");
        assert_eq!(saved["stageStamps"]["translate"], "opaque");
        assert_eq!(saved["cues"][0]["futureCueField"], 42);
        assert_eq!(saved["cues"][0]["text"], "edited");
        assert_eq!(saved["cues"][0]["hidden"], true);
    }

    #[test]
    fn frontend_project_fixture_matches_the_rust_ipc_contract() {
        let fixture = include_str!("../../../src/test/fixtures/project.json");
        let doc: Doc = serde_json::from_str(fixture).unwrap();
        let serialized = serde_json::to_value(doc).unwrap();
        assert_eq!(serialized["media"]["durationSeconds"], 2212.792018);
        assert_eq!(serialized["media"]["sampleRate"], 44100);
        assert_eq!(serialized["meta"]["createdAt"], "2026-07-21T10:08:00Z");
        assert!(serialized["media"].get("duration_seconds").is_none());
    }
}
