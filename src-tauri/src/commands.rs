//! Tauri IPC commands.
//!
//! Stage 5 wires every Stage-3 + Stage-4 entry point into a `#[tauri::command]`
//! so the React frontend can drive the editor in-process.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;
use tokio::io::AsyncWriteExt;

use crate::VERSION;

use crate::agent::Allocator;
use crate::audit::engine::Section;
use crate::audit::{audit_project, finish_check_emit_for_project, Code, Finding, Report};
use crate::data::version::{three_way_merge, working_head_is_committed};
use crate::data::{ClipCuts, Doc, MediaRef, Meta};
use crate::error::{AppError, AppResult};
use crate::export::{write_ass, write_md, write_srt, write_vtt};
use crate::media::{extract_audio_wav, probe};

async fn run_blocking<T, F>(label: &'static str, work: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    let started = std::time::Instant::now();
    let result = tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| AppError::Schema(format!("{label} task failed: {error}")))?;
    let elapsed_ms = started.elapsed().as_millis();
    if elapsed_ms >= 500 {
        tracing::warn!(
            operation = label,
            elapsed_ms,
            "blocking worker operation was slow"
        );
    } else {
        tracing::debug!(
            operation = label,
            elapsed_ms,
            "blocking worker operation completed"
        );
    }
    result
}

async fn persist_background_status<T, F>(
    label: &'static str,
    path: PathBuf,
    status: T,
    save: F,
) -> AppResult<()>
where
    T: Send + 'static,
    F: FnOnce(&std::path::Path, &T) -> AppResult<()> + Send + 'static,
{
    run_blocking(label, move || save(&path, &status)).await
}

fn trace_pipeline_started(pipeline: &str, pid: &str) {
    tracing::info!(pipeline, pid, "pipeline job started");
}

fn trace_pipeline_finished(pipeline: &str, pid: &str, state: &str, error: Option<&str>) {
    if let Some(error) = error {
        tracing::error!(pipeline, pid, state, error, "pipeline job finished");
    } else {
        tracing::info!(pipeline, pid, state, "pipeline job finished");
    }
}

fn unix_timestamp_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn advance_progress(current: u8, reported: u8) -> u8 {
    current.max(reported.min(100))
}

fn explicit_sidecar_override_ready(script_variable: &str) -> bool {
    std::env::var_os(script_variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .is_some_and(|path| path.is_file())
        && std::env::var_os("LUMEN_CUT_PYTHON").is_some_and(|value| !value.is_empty())
}

fn validate_transcription_preflight(
    config: &crate::data::modelconfig::ModelConfig,
    model_override: Option<&str>,
) -> AppResult<()> {
    match config.asr_engine {
        crate::data::modelconfig::AsrEngine::OpenaiCompatible => {
            if !crate::data::modelconfig::cloud_asr_configured(config) {
                return Err(AppError::Schema(
                    "cloud transcription is incomplete; open Settings → Speech & models and add an endpoint, API key, and model"
                        .into(),
                ));
            }
        }
        crate::data::modelconfig::AsrEngine::Local => {
            if explicit_sidecar_override_ready("LUMEN_CUT_ASR_SCRIPT") {
                return Ok(());
            }
            let status = crate::asr::runtime_status();
            if !status.runtime_ready {
                return Err(AppError::Schema(
                    "local transcription runtime is not installed; open Settings → Speech & models and install the transcription runtime"
                        .into(),
                ));
            }
            let model = model_override
                .filter(|model| !model.trim().is_empty())
                .unwrap_or(&config.asr_model);
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            if !crate::data::modelconfig::model_cached(&home, model) {
                return Err(AppError::Schema(format!(
                    "transcription model {model} is not downloaded; open Settings → Speech & models and download it"
                )));
            }
            if !status.aligner_cached {
                return Err(AppError::Schema(format!(
                    "word-timing model {} is not downloaded; open Settings → Speech & models and download it",
                    config.asr_aligner
                )));
            }
        }
    }
    Ok(())
}

fn validate_speaker_preflight(model: &str) -> AppResult<()> {
    if explicit_sidecar_override_ready("LUMEN_CUT_DIARIZE_SCRIPT") {
        return Ok(());
    }
    let status = crate::asr::runtime_status();
    if !status.diarize_runtime_ready {
        return Err(AppError::Schema(
            "speaker analysis runtime is not installed; open Settings → Speech & models and install the speaker runtime"
                .into(),
        ));
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    if !crate::data::modelconfig::diarize_model_cached(&home, model) {
        return Err(AppError::Schema(format!(
            "speaker model {model} is not downloaded; open Settings → Speech & models and download it"
        )));
    }
    Ok(())
}

fn validate_ai_provider_preflight(config: &crate::data::modelconfig::ModelConfig) -> AppResult<()> {
    if !crate::data::modelconfig::llm_configured(config) {
        return Err(AppError::Schema(
            "AI provider is not configured; open Settings → AI features and choose a provider and model"
                .into(),
        ));
    }
    let endpoint = reqwest::Url::parse(config.llm_endpoint.trim())
        .map_err(|_| AppError::Schema("AI provider URL is invalid; check it in Settings".into()))?;
    if !matches!(endpoint.scheme(), "http" | "https") {
        return Err(AppError::Schema(
            "AI provider URL must use http or https; check it in Settings".into(),
        ));
    }
    Ok(())
}

/// Tauri apps do not have a reliable working directory. Keep GUI projects in
/// a user-owned, stable location unless the caller explicitly supplies one.
fn resolve_project_root(root: Option<PathBuf>) -> PathBuf {
    if let Some(root) = root {
        return root;
    }
    if let Some(root) = std::env::var_os("LUMEN_CUT_PROJECTS_ROOT").filter(|v| !v.is_empty()) {
        return PathBuf::from(root);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    #[cfg(target_os = "macos")]
    {
        home.join("Library/Application Support/lumen-cut/Projects")
    }
    #[cfg(not(target_os = "macos"))]
    {
        home.join(".lumen-cut/projects")
    }
}

fn resolve_project_dir(pid: &str, root: Option<PathBuf>) -> AppResult<PathBuf> {
    let trimmed = pid.trim();
    let is_single_component = std::path::Path::new(trimmed).components().count() == 1;
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || !is_single_component
        || trimmed.contains(['/', '\\'])
    {
        return Err(AppError::Schema("invalid project id".into()));
    }
    Ok(resolve_project_root(root).join(trimmed))
}

// ============================================================================
// Stage-1 command
// ============================================================================

#[derive(Debug, Serialize)]
pub struct Greet {
    pub msg: String,
    pub version: &'static str,
}

#[tauri::command]
pub async fn greet() -> Greet {
    Greet {
        msg: "lumen-cut ready".to_string(),
        version: VERSION,
    }
}

// ============================================================================
// Project commands
// ============================================================================

/// Open the native macOS file chooser. Returning `None` is a normal user
/// cancellation, not an error.
#[tauri::command]
pub async fn pick_media_file(app: tauri::AppHandle) -> AppResult<Option<String>> {
    let (send, receive) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter(
            "Audio and video",
            &[
                "mp4", "mov", "m4v", "mkv", "webm", "mp3", "m4a", "wav", "aac", "flac", "aiff",
            ],
        )
        .pick_file(move |selected| {
            let _ = send.send(selected);
        });
    let selected = receive
        .await
        .map_err(|_| AppError::Schema("native file dialog closed unexpectedly".into()))?;
    Ok(selected.and_then(|file| {
        file.as_path()
            .map(|path| path.to_string_lossy().into_owned())
    }))
}

/// Pick a still image or video to place on the B-roll track. The callback API
/// keeps the native dialog off AppKit's synchronous command path.
#[tauri::command]
pub async fn pick_broll_file(app: tauri::AppHandle) -> AppResult<Option<String>> {
    let (send, receive) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter(
            "Images and video",
            &[
                "png", "jpg", "jpeg", "webp", "gif", "mp4", "mov", "m4v", "mkv", "webm",
            ],
        )
        .pick_file(move |selected| {
            let _ = send.send(selected);
        });
    let selected = receive
        .await
        .map_err(|_| AppError::Schema("native file dialog closed unexpectedly".into()))?;
    Ok(selected.and_then(|file| {
        file.as_path()
            .map(|path| path.to_string_lossy().into_owned())
    }))
}

/// Pick a background-music file without blocking AppKit's event loop.
#[tauri::command]
pub async fn pick_audio_file(app: tauri::AppHandle) -> AppResult<Option<String>> {
    let (send, receive) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter(
            "Audio",
            &[
                "aac", "aif", "aiff", "flac", "m4a", "mp3", "ogg", "opus", "wav",
            ],
        )
        .pick_file(move |selected| {
            let _ = send.send(selected);
        });
    let selected = receive
        .await
        .map_err(|_| AppError::Schema("native file dialog closed unexpectedly".into()))?;
    Ok(selected.and_then(|file| {
        file.as_path()
            .map(|path| path.to_string_lossy().into_owned())
    }))
}

#[derive(Debug, Deserialize)]
pub struct CreateProjectArgs {
    pub pid: String,
    pub from: PathBuf,
    pub lang: Option<String>,
    pub title: Option<String>,
    pub root: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
pub struct ProjectSummary {
    pub pid: String,
    pub title: String,
    pub description: String,
    pub path: PathBuf,
    pub duration_seconds: f64,
    pub word_count: usize,
    pub paragraph_count: usize,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub starred: bool,
    pub media_available: bool,
    pub last_opened_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ProjectLocalState {
    starred: bool,
    last_opened_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn load_project_local_state(dir: &std::path::Path) -> ProjectLocalState {
    std::fs::read_to_string(dir.join("project-state.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_project_local_state(dir: &std::path::Path, state: &ProjectLocalState) -> AppResult<()> {
    crate::data::storage::write_json(&dir.join("project-state.json"), state)
}

fn project_summary(dir: PathBuf, doc: &Doc) -> ProjectSummary {
    let pid = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| doc.id.clone());
    let local = load_project_local_state(&dir);
    let updated_at = crate::data::activity::load(&dir)
        .map(|activity| activity.max(doc.meta.updated_at))
        .unwrap_or(doc.meta.updated_at);
    ProjectSummary {
        pid,
        title: doc.meta.title.clone(),
        description: doc.meta.description.clone(),
        path: dir.clone(),
        duration_seconds: doc.media.duration_seconds,
        word_count: doc.all_words().len(),
        paragraph_count: doc.paragraphs.len(),
        updated_at,
        starred: local.starred,
        media_available: doc.media.path.is_file(),
        last_opened_at: local.last_opened_at,
    }
}

fn project_matches(doc: &Doc, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    let contains = |value: &str| value.to_lowercase().contains(&query);
    contains(&doc.id)
        || contains(&doc.meta.title)
        || contains(&doc.meta.description)
        || doc.paragraphs.iter().any(|paragraph| {
            paragraph.speaker.as_deref().is_some_and(contains)
                || paragraph
                    .sentences
                    .iter()
                    .any(|sentence| contains(&sentence.text))
        })
        || doc
            .translations
            .values()
            .flat_map(|groups| groups.values())
            .any(|translation| contains(&translation.text))
}

fn project_index(root: &std::path::Path, query: &str) -> AppResult<Vec<ProjectSummary>> {
    if !root.exists() {
        return Ok(vec![]);
    }
    let mut projects = std::fs::read_dir(root)?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let dir = entry.path();
            if let Err(error) = crate::data::version::recover_interrupted_restore(&dir) {
                tracing::error!(
                    project = %dir.display(),
                    %error,
                    "could not recover an interrupted project restore"
                );
                return None;
            }
            let doc = Doc::load(&dir).ok()?;
            project_matches(&doc, query).then(|| project_summary(dir, &doc))
        })
        .collect::<Vec<_>>();
    projects.sort_by(|left, right| {
        right
            .starred
            .cmp(&left.starred)
            .then_with(|| {
                right
                    .last_opened_at
                    .unwrap_or(right.updated_at)
                    .cmp(&left.last_opened_at.unwrap_or(left.updated_at))
            })
            .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
    });
    Ok(projects)
}

#[tauri::command]
pub async fn project_create(args: CreateProjectArgs) -> AppResult<ProjectSummary> {
    use chrono::Utc;
    let media_path = tokio::fs::canonicalize(&args.from).await?;
    let info = probe(&media_path).await?;
    let root = resolve_project_root(args.root.clone());
    tokio::fs::create_dir_all(&root).await?;
    let dir = resolve_project_dir(&args.pid, args.root)?;
    let doc = Doc {
        id: args.pid.clone(),
        schema: 1,
        media: MediaRef {
            path: media_path,
            duration_seconds: info.duration_seconds,
            sample_rate: info.sample_rate,
            channels: info.channels,
        },
        meta: Meta {
            title: args.title.clone().unwrap_or_else(|| args.pid.clone()),
            description: String::new(),
            language: args.lang.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
        paragraphs: vec![],
        translations: Default::default(),
    };
    let save_doc = doc.clone();
    let save_dir = dir.clone();
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("project save", move || save_doc.save(&save_dir)).await?;
    Ok(ProjectSummary {
        pid: args.pid,
        title: doc.meta.title,
        description: doc.meta.description,
        path: dir,
        duration_seconds: info.duration_seconds,
        word_count: 0,
        paragraph_count: 0,
        updated_at: doc.meta.updated_at,
        starred: false,
        media_available: true,
        last_opened_at: None,
    })
}

#[tauri::command]
pub async fn project_show(pid: String, root: Option<PathBuf>) -> AppResult<Doc> {
    let dir = resolve_project_dir(&pid, root)?;
    run_blocking("project load", move || {
        crate::data::version::recover_interrupted_restore(&dir)?;
        Doc::load(&dir)
    })
    .await
}

#[tauri::command]
pub async fn project_list(root: Option<PathBuf>) -> AppResult<Vec<ProjectSummary>> {
    let root = resolve_project_root(root);
    run_blocking("project index", move || project_index(&root, "")).await
}

#[tauri::command]
pub async fn project_search(
    query: String,
    root: Option<PathBuf>,
) -> AppResult<Vec<ProjectSummary>> {
    let root = resolve_project_root(root);
    run_blocking("project search", move || project_index(&root, &query)).await
}

#[tauri::command]
pub async fn project_set_star(
    pid: String,
    starred: bool,
    root: Option<PathBuf>,
) -> AppResult<ProjectSummary> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("project star update", move || {
        let doc = Doc::load(&dir)?;
        let mut local = load_project_local_state(&dir);
        local.starred = starred;
        save_project_local_state(&dir, &local)?;
        Ok(project_summary(dir, &doc))
    })
    .await
}

#[tauri::command]
pub async fn project_mark_opened(pid: String, root: Option<PathBuf>) -> AppResult<ProjectSummary> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("project recent update", move || {
        let doc = Doc::load(&dir)?;
        let mut local = load_project_local_state(&dir);
        local.last_opened_at = Some(chrono::Utc::now());
        save_project_local_state(&dir, &local)?;
        Ok(project_summary(dir, &doc))
    })
    .await
}

#[tauri::command]
pub async fn project_update_meta(
    pid: String,
    title: String,
    description: String,
    language: Option<String>,
    root: Option<PathBuf>,
) -> AppResult<Doc> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("project metadata update", move || {
        crate::data::edit_history::record(
            &dir,
            "Edit project details",
            || {
                let mut doc = Doc::load(&dir)?;
                let title = title.trim();
                if title.is_empty() {
                    return Err(AppError::Schema("project title cannot be empty".into()));
                }
                let next_description = description.trim().to_string();
                let next_language = language
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty());
                let changed = doc.meta.title != title
                    || doc.meta.description != next_description
                    || doc.meta.language != next_language;
                if changed {
                    doc.meta.title = title.to_string();
                    doc.meta.description = next_description;
                    doc.meta.language = next_language;
                    doc.meta.updated_at = chrono::Utc::now();
                    doc.save(&dir)?;
                }
                Ok((doc, changed))
            },
            |(_, changed)| *changed,
        )
        .map(|(doc, _)| doc)
    })
    .await
}

#[tauri::command]
pub async fn project_reveal(pid: String, root: Option<PathBuf>) -> AppResult<String> {
    let dir = resolve_project_dir(&pid, root)?;
    match tokio::fs::metadata(&dir).await {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(AppError::ProjectNotFound(dir)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::ProjectNotFound(dir));
        }
        Err(error) => return Err(AppError::Io(error)),
    }
    #[cfg(target_os = "macos")]
    tokio::process::Command::new("open")
        .args(["-R"])
        .arg(dir.join("doc.json"))
        .spawn()?;
    #[cfg(not(target_os = "macos"))]
    tokio::process::Command::new("open").arg(&dir).spawn()?;
    Ok(dir.to_string_lossy().into_owned())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri injects each independent pipeline state.
pub async fn project_delete(
    pid: String,
    root: Option<PathBuf>,
    transcription: tauri::State<'_, TranscriptionState>,
    recording: tauri::State<'_, RecordingState>,
    speakers: tauri::State<'_, SpeakerAnalysisState>,
    broll: tauri::State<'_, BrollPreviewState>,
    video: tauri::State<'_, VideoExportState>,
    agents: tauri::State<'_, AgentServerState>,
) -> AppResult<bool> {
    let dir = resolve_project_dir(&pid, root)?;
    let job_running = |state: &str| matches!(state, "running" | "cancelling");
    if transcription
        .jobs
        .lock()
        .expect("transcription state poisoned")
        .get(&pid)
        .is_some_and(|job| job_running(&job.status.state))
    {
        return Err(AppError::Schema(
            "cannot delete a project while transcription is running".into(),
        ));
    }
    if recording
        .session
        .lock()
        .expect("recording state poisoned")
        .as_ref()
        .is_some_and(|session| session.pid == pid)
    {
        return Err(AppError::Schema(
            "cannot delete a project while recording is running".into(),
        ));
    }
    if speakers
        .jobs
        .lock()
        .expect("speaker analysis state poisoned")
        .get(&pid)
        .is_some_and(|job| job_running(&job.status.state))
        || broll
            .jobs
            .lock()
            .expect("B-roll preview state poisoned")
            .get(&pid)
            .is_some_and(|job| job_running(&job.status.state))
        || video
            .jobs
            .lock()
            .expect("video export state poisoned")
            .get(&pid)
            .is_some_and(|job| job_running(&job.status.state))
        || agents
            .active_tasks
            .lock()
            .expect("state poisoned")
            .keys()
            .any(|key| key.starts_with(&format!("{}::", dir.display())))
    {
        return Err(AppError::Schema(
            "cannot delete a project while a background pipeline is running".into(),
        ));
    }
    let _mutation = lock_project_mutation(&dir).await;
    if !tokio::fs::try_exists(&dir).await? {
        return Ok(false);
    }
    tokio::fs::remove_dir_all(dir).await?;
    Ok(true)
}

/// Keep the asset protocol narrowed to the media belonging to the project
/// currently open in the editor. The frontend never supplies an arbitrary
/// filesystem path.
pub struct MediaAssetState {
    current: Mutex<Option<PathBuf>>,
    broll: Mutex<Option<PathBuf>>,
    music: Mutex<HashSet<PathBuf>>,
    timeline_cache: Mutex<Option<PathBuf>>,
    project_thumbnails: Mutex<HashSet<PathBuf>>,
    failed_project_thumbnails: Mutex<HashSet<PathBuf>>,
}

impl Default for MediaAssetState {
    fn default() -> Self {
        Self {
            current: Mutex::new(None),
            broll: Mutex::new(None),
            music: Mutex::new(HashSet::new()),
            timeline_cache: Mutex::new(None),
            project_thumbnails: Mutex::new(HashSet::new()),
            failed_project_thumbnails: Mutex::new(HashSet::new()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectThumbnail {
    pub path: Option<String>,
    pub media_available: bool,
    pub deferred: bool,
}

fn allow_project_thumbnail_cache(
    app: &tauri::AppHandle,
    state: &MediaAssetState,
    cache_dir: &std::path::Path,
) -> AppResult<()> {
    let mut allowed = state
        .project_thumbnails
        .lock()
        .expect("project thumbnail state poisoned");
    if allowed.insert(cache_dir.to_path_buf()) {
        app.asset_protocol_scope()
            .allow_directory(cache_dir, true)
            .map_err(|error| AppError::Schema(format!("project thumbnail scope: {error}")))?;
    }
    Ok(())
}

fn project_thumbnail_args(
    media: &std::path::Path,
    duration: f64,
    output: &std::path::Path,
    audio_only: bool,
) -> Vec<String> {
    let mut args = vec!["-nostdin".into(), "-y".into(), "-v".into(), "error".into()];
    if !audio_only {
        args.extend([
            "-ss".into(),
            format!("{:.3}", (duration.max(0.0) * 0.1).min(30.0)),
        ]);
    }
    args.extend(["-i".into(), media.to_string_lossy().into_owned()]);
    if audio_only {
        args.extend([
            "-filter_complex".into(),
            "aformat=channel_layouts=mono,showwavespic=s=640x360:colors=#9f4f24:scale=sqrt,format=yuvj420p".into(),
            "-frames:v".into(),
            "1".into(),
        ]);
    } else {
        args.extend([
            "-vf".into(),
            "scale=640:360:force_original_aspect_ratio=increase,crop=640:360,format=yuvj420p"
                .into(),
            "-frames:v".into(),
            "1".into(),
            "-q:v".into(),
            "3".into(),
        ]);
    }
    args.push(output.to_string_lossy().into_owned());
    args
}

#[tauri::command]
pub async fn project_thumbnail(
    pid: String,
    root: Option<PathBuf>,
    app: tauri::AppHandle,
    state: tauri::State<'_, MediaAssetState>,
) -> AppResult<ProjectThumbnail> {
    let dir = resolve_project_dir(&pid, root)?;
    let prepared = run_blocking("project thumbnail cache validation", move || {
        let doc = Doc::load(&dir)?;
        if !doc.media.path.is_file() {
            return Ok(None);
        }
        let media = std::fs::canonicalize(&doc.media.path)?;
        let signature = timeline_visual_signature(&media)?;
        let cache_dir = dir.join(".lumen-cut").join("project-thumbnail");
        let thumbnail = cache_dir.join("thumbnail.jpg");
        let cached = std::fs::read_to_string(cache_dir.join("manifest.json"))
            .ok()
            .and_then(|raw| serde_json::from_str::<TimelineVisualCache>(&raw).ok())
            .is_some_and(|value| value == signature)
            && thumbnail.is_file();
        let audio_only = media
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "aac" | "aif" | "aiff" | "flac" | "m4a" | "mp3" | "ogg" | "opus" | "wav"
                )
            });
        Ok(Some((
            media,
            doc.media.duration_seconds.max(0.001),
            signature,
            cache_dir,
            thumbnail,
            cached,
            audio_only,
        )))
    })
    .await?;

    let Some((media, duration, signature, cache_dir, thumbnail, cached, audio_only)) = prepared
    else {
        return Ok(ProjectThumbnail {
            path: None,
            media_available: false,
            deferred: false,
        });
    };
    if cached {
        allow_project_thumbnail_cache(&app, &state, &cache_dir)?;
        return Ok(ProjectThumbnail {
            path: Some(thumbnail.to_string_lossy().into_owned()),
            media_available: true,
            deferred: false,
        });
    }
    if state
        .failed_project_thumbnails
        .lock()
        .expect("project thumbnail failure state poisoned")
        .contains(&cache_dir)
    {
        return Ok(ProjectThumbnail {
            path: None,
            media_available: true,
            deferred: false,
        });
    }
    if crate::performance::active_heavy_label().is_some() {
        return Ok(ProjectThumbnail {
            path: None,
            media_available: true,
            deferred: true,
        });
    }

    let _heavy_work = crate::performance::acquire_heavy("project-thumbnail").await?;
    tokio::fs::create_dir_all(&cache_dir).await?;
    let temporary = cache_dir.join("thumbnail.tmp.jpg");
    let _ = tokio::fs::remove_file(&temporary).await;
    let args = project_thumbnail_args(&media, duration, &temporary, audio_only);
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(12),
        crate::proc::run("ffmpeg", &arg_refs),
    )
    .await;
    match result {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(error);
        }
        Err(_) => {
            let _ = tokio::fs::remove_file(&temporary).await;
            state
                .failed_project_thumbnails
                .lock()
                .expect("project thumbnail failure state poisoned")
                .insert(cache_dir);
            tracing::warn!(
                project = %pid,
                media = %media.display(),
                "project thumbnail timed out; continuing without a thumbnail"
            );
            return Ok(ProjectThumbnail {
                path: None,
                media_available: true,
                deferred: true,
            });
        }
    }
    let bytes = tokio::fs::read(&temporary).await?;
    crate::data::storage::write(&thumbnail, &bytes)?;
    let _ = tokio::fs::remove_file(&temporary).await;
    crate::data::storage::write_json(&cache_dir.join("manifest.json"), &signature)?;
    allow_project_thumbnail_cache(&app, &state, &cache_dir)?;
    Ok(ProjectThumbnail {
        path: Some(thumbnail.to_string_lossy().into_owned()),
        media_available: true,
        deferred: false,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMediaStatus {
    pub path: PathBuf,
    pub available: bool,
    pub file_size: Option<u64>,
    pub expected_duration_seconds: f64,
    pub issue: Option<String>,
    pub suggested_path: Option<PathBuf>,
}

fn find_nearby_media(project_dir: &std::path::Path, missing: &std::path::Path) -> Option<PathBuf> {
    let target = missing.file_name()?;
    let mut roots = Vec::new();
    if let Some(parent) = missing.parent().filter(|parent| parent.is_dir()) {
        roots.push(parent.to_path_buf());
    }
    roots.push(project_dir.to_path_buf());
    if let Some(root) = project_dir.parent() {
        roots.push(root.to_path_buf());
    }

    let mut candidates = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut budget = 2_000usize;
    let mut pending = roots
        .into_iter()
        .map(|path| (path, 0usize))
        .collect::<std::collections::VecDeque<_>>();
    while let Some((directory, depth)) = pending.pop_front() {
        if budget == 0 {
            break;
        }
        let canonical = match std::fs::canonicalize(&directory) {
            Ok(path) => path,
            Err(_) => continue,
        };
        if !visited.insert(canonical.clone()) {
            continue;
        }
        let entries = match std::fs::read_dir(&canonical) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            if budget == 0 {
                break;
            }
            budget -= 1;
            let path = entry.path();
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(_) => continue,
            };
            if kind.is_file()
                && entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&target.to_string_lossy())
                && path != missing
            {
                candidates.push(path);
                if candidates.len() > 1 {
                    return None;
                }
            } else if kind.is_dir()
                && depth < 2
                && !entry.file_name().to_string_lossy().starts_with('.')
            {
                pending.push_back((path, depth + 1));
            }
        }
    }
    candidates.pop()
}

#[tauri::command]
pub async fn project_media_status(
    pid: String,
    root: Option<PathBuf>,
) -> AppResult<ProjectMediaStatus> {
    let dir = resolve_project_dir(&pid, root)?;
    run_blocking("project media status", move || {
        let doc = Doc::load(&dir)?;
        match std::fs::metadata(&doc.media.path) {
            Ok(metadata) if metadata.is_file() => Ok(ProjectMediaStatus {
                path: doc.media.path,
                available: true,
                file_size: Some(metadata.len()),
                expected_duration_seconds: doc.media.duration_seconds,
                issue: None,
                suggested_path: None,
            }),
            Ok(_) => Ok(ProjectMediaStatus {
                suggested_path: find_nearby_media(&dir, &doc.media.path),
                path: doc.media.path,
                available: false,
                file_size: None,
                expected_duration_seconds: doc.media.duration_seconds,
                issue: Some("the saved media path is not a file".into()),
            }),
            Err(error) => {
                let suggested_path = find_nearby_media(&dir, &doc.media.path);
                Ok(ProjectMediaStatus {
                    path: doc.media.path,
                    available: false,
                    file_size: None,
                    expected_duration_seconds: doc.media.duration_seconds,
                    issue: Some(if error.kind() == std::io::ErrorKind::NotFound {
                        "the original media file is missing or was moved".into()
                    } else {
                        format!("the original media file cannot be read: {error}")
                    }),
                    suggested_path,
                })
            }
        }
    })
    .await
}

#[tauri::command]
pub async fn project_media_relink(
    pid: String,
    path: PathBuf,
    root: Option<PathBuf>,
    app: tauri::AppHandle,
    state: tauri::State<'_, MediaAssetState>,
) -> AppResult<Doc> {
    let media_path = tokio::fs::canonicalize(path).await?;
    let metadata = tokio::fs::metadata(&media_path).await?;
    if !metadata.is_file() {
        return Err(AppError::Schema(
            "replacement media path is not a file".into(),
        ));
    }
    let info = probe(&media_path).await?;
    if !info.duration_seconds.is_finite() || info.duration_seconds <= 0.0 {
        return Err(AppError::Schema(
            "replacement file has no readable audio or video duration".into(),
        ));
    }

    let dir = resolve_project_dir(&pid, root)?;
    let cache_dir = dir.join(".lumen-cut").join("timeline-visuals");
    let _mutation = lock_project_mutation(&dir).await;
    let saved_media_path = media_path.clone();
    let (doc, changed) = run_blocking("project media relink", move || {
        let mut doc = Doc::load(&dir)?;
        let expected = doc.media.duration_seconds;
        let difference = (info.duration_seconds - expected).abs();
        let tolerance = (expected * 0.02).max(2.0);
        if expected > 0.0 && difference > tolerance {
            return Err(AppError::Schema(format!(
                "replacement duration differs from this project by {:.1}s (expected {:.1}s, found {:.1}s); choose the original media or an equivalent copy",
                difference, expected, info.duration_seconds
            )));
        }
        let changed = doc.media.path != saved_media_path
            || (doc.media.duration_seconds - info.duration_seconds).abs() > f64::EPSILON
            || doc.media.sample_rate != info.sample_rate
            || doc.media.channels != info.channels;
        let result = crate::data::edit_history::record(
            &dir,
            "Relink project media",
            || {
                doc.media.path = saved_media_path;
                doc.media.duration_seconds = info.duration_seconds;
                doc.media.sample_rate = info.sample_rate;
                doc.media.channels = info.channels;
                doc.meta.updated_at = chrono::Utc::now();
                doc.save(&dir)?;
                Ok((doc, changed))
            },
            |(_, changed)| *changed,
        )?;
        Ok(result)
    })
    .await?;

    if changed {
        let _ = tokio::fs::remove_dir_all(&cache_dir).await;
    }
    let scope = app.asset_protocol_scope();
    {
        let mut current = state.current.lock().expect("media asset state poisoned");
        if let Some(previous) = current.as_ref().filter(|path| *path != &media_path) {
            scope
                .forbid_file(previous)
                .map_err(|error| AppError::Schema(format!("media scope: {error}")))?;
        }
        scope
            .allow_file(&media_path)
            .map_err(|error| AppError::Schema(format!("media scope: {error}")))?;
        *current = Some(media_path);
    }
    {
        let mut timeline_cache = state
            .timeline_cache
            .lock()
            .expect("timeline asset state poisoned");
        if let Some(previous) = timeline_cache.take() {
            let _ = scope.forbid_directory(previous, true);
        }
    }
    Ok(doc)
}

#[tauri::command]
pub async fn broll_asset_allow(
    pid: String,
    id: String,
    root: Option<PathBuf>,
    app: tauri::AppHandle,
    state: tauri::State<'_, MediaAssetState>,
) -> AppResult<String> {
    let dir = resolve_project_dir(&pid, root)?;
    let asset = run_blocking("B-roll asset validation", move || {
        let placement = crate::data::broll::load(&dir)?
            .into_iter()
            .find(|placement| placement.id == id)
            .ok_or_else(|| AppError::Schema(format!("B-roll id {id} not found")))?;
        let asset = std::fs::canonicalize(placement.file)?;
        if !asset.is_file() {
            return Err(AppError::ProjectNotFound(asset));
        }
        Ok(asset)
    })
    .await?;

    let scope = app.asset_protocol_scope();
    let mut current = state.broll.lock().expect("B-roll asset state poisoned");
    if let Some(previous) = current.as_ref().filter(|path| *path != &asset) {
        scope
            .forbid_file(previous)
            .map_err(|error| AppError::Schema(format!("B-roll media scope: {error}")))?;
    }
    scope
        .allow_file(&asset)
        .map_err(|error| AppError::Schema(format!("B-roll media scope: {error}")))?;
    *current = Some(asset.clone());
    Ok(asset.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn audio_asset_allow(
    pid: String,
    music_id: String,
    root: Option<PathBuf>,
    app: tauri::AppHandle,
    state: tauri::State<'_, MediaAssetState>,
) -> AppResult<String> {
    let dir = resolve_project_dir(&pid, root)?;
    let (asset, current_assets) = run_blocking("music asset validation", move || {
        let mix = crate::data::audio_mix::load(&dir)?;
        let track = mix
            .music
            .iter()
            .find(|track| track.id == music_id)
            .ok_or_else(|| AppError::Schema(format!("music track {music_id} not found")))?;
        let asset = std::fs::canonicalize(&track.path)?;
        if !asset.is_file() {
            return Err(AppError::ProjectNotFound(asset));
        }
        let current_assets = mix
            .music
            .iter()
            .filter_map(|track| std::fs::canonicalize(&track.path).ok())
            .collect::<HashSet<_>>();
        Ok((asset, current_assets))
    })
    .await?;

    let scope = app.asset_protocol_scope();
    let mut allowed = state.music.lock().expect("music asset state poisoned");
    let stale = allowed
        .difference(&current_assets)
        .cloned()
        .collect::<Vec<_>>();
    for previous in stale {
        scope
            .forbid_file(&previous)
            .map_err(|error| AppError::Schema(format!("music media scope: {error}")))?;
        allowed.remove(&previous);
    }
    if allowed.insert(asset.clone()) {
        scope
            .allow_file(&asset)
            .map_err(|error| AppError::Schema(format!("music media scope: {error}")))?;
    }
    Ok(asset.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn media_asset_allow(
    pid: String,
    root: Option<PathBuf>,
    app: tauri::AppHandle,
    state: tauri::State<'_, MediaAssetState>,
) -> AppResult<String> {
    let dir = resolve_project_dir(&pid, root)?;
    let media_path = run_blocking("media asset validation", move || {
        let doc = Doc::load(&dir)?;
        let media_path = std::fs::canonicalize(&doc.media.path)?;
        if !media_path.is_file() {
            return Err(AppError::ProjectNotFound(media_path));
        }
        Ok(media_path)
    })
    .await?;

    let scope = app.asset_protocol_scope();
    let mut current = state.current.lock().expect("media asset state poisoned");
    if let Some(previous) = current.as_ref().filter(|path| *path != &media_path) {
        scope
            .forbid_file(previous)
            .map_err(|error| AppError::Schema(format!("media scope: {error}")))?;
    }
    scope
        .allow_file(&media_path)
        .map_err(|error| AppError::Schema(format!("media scope: {error}")))?;
    *current = Some(media_path.clone());
    Ok(media_path.to_string_lossy().into_owned())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineVisuals {
    pub contact_sheet: Option<String>,
    pub waveform: Option<String>,
    pub deferred: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct TimelineVisualCache {
    version: u32,
    media_size: u64,
    media_modified_ms: u128,
}

fn timeline_visual_signature(path: &std::path::Path) -> AppResult<TimelineVisualCache> {
    let metadata = std::fs::metadata(path)?;
    let modified = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| AppError::Schema(format!("media modification time: {error}")))?;
    Ok(TimelineVisualCache {
        version: 1,
        media_size: metadata.len(),
        media_modified_ms: modified.as_millis(),
    })
}

fn contact_sheet_args(media: &str, duration: f64, output: &str) -> Vec<String> {
    const FRAMES: usize = 12;
    let mut args = vec!["-nostdin".into(), "-y".into(), "-v".into(), "error".into()];
    for index in 0..FRAMES {
        let at = duration * (index as f64 + 0.5) / FRAMES as f64;
        args.extend(["-ss".into(), format!("{at:.3}"), "-i".into(), media.into()]);
    }
    let mut filter = (0..FRAMES)
        .map(|index| {
            format!(
                "[{index}:v]scale=120:68:force_original_aspect_ratio=increase,\
                 crop=120:68,setsar=1,setpts=PTS-STARTPTS[v{index}]"
            )
        })
        .collect::<Vec<_>>()
        .join(";");
    filter.push(';');
    for index in 0..FRAMES {
        filter.push_str(&format!("[v{index}]"));
    }
    filter.push_str(&format!("hstack=inputs={FRAMES}[out]"));
    args.extend([
        "-filter_complex".into(),
        filter,
        "-map".into(),
        "[out]".into(),
        "-frames:v".into(),
        "1".into(),
        output.into(),
    ]);
    args
}

fn allow_timeline_cache(
    app: &tauri::AppHandle,
    state: &MediaAssetState,
    cache_dir: &std::path::Path,
) -> AppResult<()> {
    let scope = app.asset_protocol_scope();
    let mut current = state
        .timeline_cache
        .lock()
        .expect("timeline asset state poisoned");
    if let Some(previous) = current.as_ref().filter(|path| *path != cache_dir) {
        scope
            .forbid_directory(previous, true)
            .map_err(|error| AppError::Schema(format!("timeline visual scope: {error}")))?;
    }
    scope
        .allow_directory(cache_dir, true)
        .map_err(|error| AppError::Schema(format!("timeline visual scope: {error}")))?;
    *current = Some(cache_dir.to_path_buf());
    Ok(())
}

/// Prepare a compact contact sheet and waveform for the timeline. Generation
/// is cached by media identity and runs through async child processes, never
/// on AppKit's event thread.
#[tauri::command]
pub async fn timeline_visuals(
    pid: String,
    root: Option<PathBuf>,
    app: tauri::AppHandle,
    state: tauri::State<'_, MediaAssetState>,
) -> AppResult<TimelineVisuals> {
    let dir = resolve_project_dir(&pid, root)?;
    let (media, duration, signature, cache_dir, cached) =
        run_blocking("timeline visual cache validation", move || {
            let doc = Doc::load(&dir)?;
            let media = std::fs::canonicalize(&doc.media.path)?;
            if !media.is_file() {
                return Err(AppError::ProjectNotFound(media));
            }
            let signature = timeline_visual_signature(&media)?;
            let cache_dir = dir.join(".lumen-cut").join("timeline-visuals");
            let manifest = cache_dir.join("manifest.json");
            let cached = std::fs::read_to_string(&manifest)
                .ok()
                .and_then(|raw| serde_json::from_str::<TimelineVisualCache>(&raw).ok())
                .is_some_and(|value| value == signature);
            Ok((
                media,
                doc.media.duration_seconds.max(0.001),
                signature,
                cache_dir,
                cached,
            ))
        })
        .await?;

    let contact_sheet = cache_dir.join("contact-sheet.jpg");
    let waveform = cache_dir.join("waveform.png");
    let existing = |path: &std::path::Path| path.is_file().then(|| path.to_path_buf());
    if cached {
        allow_timeline_cache(&app, &state, &cache_dir)?;
        return Ok(TimelineVisuals {
            contact_sheet: existing(&contact_sheet).map(|path| path.to_string_lossy().into_owned()),
            waveform: existing(&waveform).map(|path| path.to_string_lossy().into_owned()),
            deferred: false,
        });
    }

    if crate::performance::active_heavy_label().is_some() {
        return Ok(TimelineVisuals {
            contact_sheet: None,
            waveform: None,
            deferred: true,
        });
    }
    let _heavy_work = crate::performance::acquire_heavy("timeline-visuals").await?;
    tokio::fs::create_dir_all(&cache_dir).await?;
    let _ = tokio::fs::remove_file(&contact_sheet).await;
    let _ = tokio::fs::remove_file(&waveform).await;
    let media_arg = media.to_string_lossy().into_owned();

    let audio_only = media
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "aac" | "aif" | "aiff" | "flac" | "m4a" | "mp3" | "ogg" | "opus" | "wav"
            )
        });
    if !audio_only {
        let output = contact_sheet.to_string_lossy().into_owned();
        let args = contact_sheet_args(&media_arg, duration, &output);
        let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let _ = crate::proc::run("ffmpeg", &refs).await;
    }

    let waveform_output = waveform.to_string_lossy().into_owned();
    let _ = crate::proc::run(
        "ffmpeg",
        &[
            "-nostdin",
            "-y",
            "-v",
            "error",
            "-i",
            &media_arg,
            "-filter_complex",
            "aformat=channel_layouts=mono,showwavespic=s=1440x64:colors=#9f4f24:scale=sqrt",
            "-frames:v",
            "1",
            &waveform_output,
        ],
    )
    .await;

    if !contact_sheet.is_file() && !waveform.is_file() {
        return Err(AppError::Schema(
            "ffmpeg could not create timeline thumbnails or a waveform".into(),
        ));
    }
    crate::data::storage::write_json(&cache_dir.join("manifest.json"), &signature)?;
    allow_timeline_cache(&app, &state, &cache_dir)?;
    Ok(TimelineVisuals {
        contact_sheet: existing(&contact_sheet).map(|path| path.to_string_lossy().into_owned()),
        waveform: existing(&waveform).map(|path| path.to_string_lossy().into_owned()),
        deferred: false,
    })
}

// ============================================================================
// Auto pipeline
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct AutoArgs {
    pub media: String,
    pub pid: Option<String>,
    pub lang: Option<String>,
    pub title: Option<String>,
    pub out: Option<PathBuf>,
    pub model: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AutoResult {
    pub pid_dir: PathBuf,
    pub srt: PathBuf,
    pub vtt: PathBuf,
    pub ass: PathBuf,
    pub md: PathBuf,
    pub word_count: usize,
    pub paragraph_count: usize,
}

fn normalize_transcription_doc(doc: &mut Doc, language_hint: Option<String>) {
    // Keep ASR language detection unless the user supplied an explicit hint.
    // Assigning `None` here used to erase a valid detected language.
    if let Some(language) = language_hint {
        doc.meta.language = Some(language);
    }
    // Forced alignment can contain occasional zero-length or overlapping word
    // boundaries. Normalize them before the first save so every downstream
    // editor and export sees a valid timeline.
    crate::pipeline::timing::repair(doc);
}

fn ensure_not_cancelled() -> AppResult<()> {
    if crate::proc::cancellation_requested() {
        Err(AppError::Cancelled)
    } else {
        Ok(())
    }
}

fn remove_if_present(path: &std::path::Path) -> AppResult<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn invalidate_transcript_derived_state(dir: &std::path::Path) -> AppResult<()> {
    for name in ["hidden.json", "cuts.json", "chapters.json"] {
        remove_if_present(&dir.join(name))?;
    }
    for path in [dir.join("ai"), dir.join(".lumen-cut").join("edit-history")] {
        match std::fs::remove_dir_all(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn persist_transcription_result(doc: Doc, pid_dir: PathBuf) -> AppResult<AutoResult> {
    let previous = Doc::load(&pid_dir)
        .ok()
        .filter(|doc| !doc.paragraphs.is_empty());
    let mut recovery = None;
    if let Some(previous) = previous {
        let mut lineage = crate::data::version::Lineage::load(&pid_dir)?;
        let branch = lineage
            .active_branch
            .clone()
            .unwrap_or_else(|| "main".into());
        let id = crate::data::version::commit_snapshot(
            &pid_dir,
            &previous,
            &mut lineage,
            &branch,
            "Before retranscription",
            "Automatic recovery point before replacing the transcript",
            crate::data::version::VersionKind::Auto,
        )?;
        recovery = Some((lineage, id));
    }

    let apply = || {
        if recovery.is_some() {
            // A retranscription must write a genuinely fresh document.
            // Doc::save normally preserves forward-compatible fields, but
            // cue-derived fields such as chapters belong to the old word IDs.
            remove_if_present(&pid_dir.join("doc.json"))?;
            invalidate_transcript_derived_state(&pid_dir)?;
        }
        doc.save(&pid_dir)?;
        let srt = pid_dir.join("out.srt");
        let vtt = pid_dir.join("out.vtt");
        let ass = pid_dir.join("out.ass");
        let md = pid_dir.join("out.md");
        write_srt(&doc, &srt)?;
        write_vtt(&doc, &vtt)?;
        write_ass(&doc, &ass, 1920, 1080)?;
        write_md(&doc, &md)?;
        Ok(AutoResult {
            pid_dir: pid_dir.clone(),
            srt,
            vtt,
            ass,
            md,
            word_count: doc.all_words().len(),
            paragraph_count: doc.paragraphs.len(),
        })
    };

    match apply() {
        Ok(result) => Ok(result),
        Err(error) => {
            if let Some((mut lineage, id)) = recovery {
                if let Err(restore_error) =
                    crate::data::version::restore_snapshot(&pid_dir, &mut lineage, &id)
                {
                    return Err(AppError::Schema(format!(
                        "retranscription failed ({error}) and recovery failed ({restore_error})"
                    )));
                }
            }
            Err(error)
        }
    }
}

async fn run_auto_impl<F>(args: AutoArgs, report: F) -> AppResult<AutoResult>
where
    F: Fn(&str, u8, Option<crate::asr::AsrProgress>) + Send + Sync + 'static,
{
    let report = Arc::new(report);
    report("waiting", 1, None);
    let _heavy_work = crate::performance::acquire_heavy("transcription").await?;
    report("preparing", 5, None);
    ensure_not_cancelled()?;
    let model_override = args.model.clone();
    let model_config = run_blocking("transcription preflight", move || {
        let config = crate::data::modelconfig::load();
        validate_transcription_preflight(&config, model_override.as_deref())?;
        Ok(config)
    })
    .await?;
    let out_dir = resolve_project_root(args.out);
    tokio::fs::create_dir_all(&out_dir).await?;

    let requested_pid = args.pid.filter(|pid| !pid.trim().is_empty());
    let download_dir = requested_pid
        .as_ref()
        .map(|pid| out_dir.join(pid))
        .unwrap_or_else(|| out_dir.clone());
    tokio::fs::create_dir_all(&download_dir).await?;
    let media_path = if args.media.starts_with("http://") || args.media.starts_with("https://") {
        report("downloading", 12, None);
        let download_report = report.clone();
        crate::media_url::download_with_progress(
            &args.media,
            &download_dir.join("source.%(ext)s"),
            Some(Arc::new(move |progress| {
                let whole_job_progress = 12 + ((u16::from(progress.percent) * 11) / 100) as u8;
                download_report("downloading", whole_job_progress.min(23), None);
            })),
        )
        .await?
    } else {
        PathBuf::from(&args.media)
    };
    ensure_not_cancelled()?;
    if !tokio::fs::try_exists(&media_path).await? {
        return Err(AppError::ProjectNotFound(media_path));
    }
    let media_path = tokio::fs::canonicalize(media_path).await?;

    let pid_stem = requested_pid.unwrap_or_else(|| {
        media_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".to_string())
    });
    let pid_dir = out_dir.join(&pid_stem);
    tokio::fs::create_dir_all(&pid_dir).await?;
    let wav = pid_dir.join("audio.wav");
    // A recording project already uses `<pid>/audio.wav` as its media source.
    // Avoid asking ffmpeg to overwrite its own input.
    if media_path != wav {
        report("extracting", 25, None);
        extract_audio_wav(&media_path, &wav).await?;
    }
    ensure_not_cancelled()?;
    report("analyzing", 35, None);
    let info = probe(&media_path).await?;
    ensure_not_cancelled()?;

    report("transcribing", 45, None);
    let progress_report = report.clone();
    let progress_callback = Some(Arc::new(move |progress: crate::asr::AsrProgress| {
        let phase = progress.phase.clone();
        progress_report(&phase, progress.progress, Some(progress));
    }) as crate::asr::AsrProgressCallback);
    let asr_out = match model_config.asr_engine {
        crate::data::modelconfig::AsrEngine::Local => {
            let model = args.model.as_deref().unwrap_or(&model_config.asr_model);
            crate::asr::transcribe_file_with_aligner_progress(
                &wav,
                model,
                args.lang.as_deref(),
                Some(&model_config.asr_aligner),
                progress_callback,
            )
            .await?
        }
        crate::data::modelconfig::AsrEngine::OpenaiCompatible => {
            let model = args
                .model
                .as_deref()
                .unwrap_or(&model_config.asr_cloud_model);
            crate::asr::cloud::transcribe_file(
                &wav,
                info.duration_seconds,
                &model_config.asr_cloud_endpoint,
                &model_config.asr_cloud_api_key,
                model,
                args.lang.as_deref(),
                progress_callback,
            )
            .await?
        }
    };
    ensure_not_cancelled()?;

    report("saving", 88, None);
    let mut doc: Doc = asr_out.into();
    doc.id = pid_stem.clone();
    doc.media = MediaRef {
        path: media_path.clone(),
        duration_seconds: info.duration_seconds,
        sample_rate: info.sample_rate,
        channels: info.channels,
    };
    doc.meta.title = args.title.clone().unwrap_or_else(|| pid_stem.clone());
    normalize_transcription_doc(&mut doc, args.lang.clone());
    doc.meta.updated_at = chrono::Utc::now();
    report("exporting", 94, None);
    // Transcription can take minutes. Keep editing responsive while compute or
    // network work runs, and serialize only the final authoritative swap.
    let _mutation = lock_project_mutation(&pid_dir).await;
    let result = run_blocking("project save and subtitle export", move || {
        persist_transcription_result(doc, pid_dir)
    })
    .await?;
    report("completed", 100, None);
    Ok(result)
}

#[tauri::command]
pub async fn run_auto(args: AutoArgs) -> AppResult<AutoResult> {
    run_auto_impl(args, |_, _, _| {}).await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionJobStatus {
    pub pid: String,
    pub state: String,
    pub phase: String,
    pub progress: u8,
    #[serde(default)]
    pub current: Option<u32>,
    #[serde(default)]
    pub total: Option<u32>,
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub elapsed_seconds: Option<f64>,
    #[serde(default)]
    pub cpu_percent: Option<u32>,
    #[serde(default)]
    pub peak_memory_mb: Option<u64>,
    #[serde(default)]
    pub memory_limit_mb: Option<u64>,
    #[serde(default)]
    pub mlx_active_memory_mb: Option<u64>,
    #[serde(default)]
    pub mlx_cache_memory_mb: Option<u64>,
    #[serde(default)]
    pub started_at: Option<u64>,
    #[serde(default)]
    pub updated_at: Option<u64>,
    pub error: Option<String>,
}

struct TranscriptionJob {
    status: TranscriptionJobStatus,
    cancel: Arc<AtomicBool>,
    status_path: PathBuf,
}

#[derive(Clone, Default)]
pub struct TranscriptionState {
    jobs: Arc<Mutex<HashMap<String, TranscriptionJob>>>,
}

fn transcription_status_path(pid: &str, root: Option<PathBuf>) -> AppResult<PathBuf> {
    // Reuse the project-id boundary check before using the id as a filename.
    let _ = resolve_project_dir(pid, root.clone())?;
    Ok(resolve_project_root(root)
        .join(".jobs")
        .join(format!("{pid}.json")))
}

fn save_transcription_status(
    path: &std::path::Path,
    status: &TranscriptionJobStatus,
) -> AppResult<()> {
    crate::data::storage::write_json(path, status)
}

fn load_transcription_status(path: &std::path::Path) -> AppResult<TranscriptionJobStatus> {
    Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
}

fn load_recovered_transcription_status(
    path: &std::path::Path,
) -> AppResult<TranscriptionJobStatus> {
    let mut status = load_transcription_status(path)?;
    if matches!(status.state.as_str(), "running" | "cancelling") {
        status.state = "failed".into();
        status.phase = "failed".into();
        status.error = Some(
            "the previous transcription was interrupted when lumen-cut closed; retry to start it again"
                .into(),
        );
        status.updated_at = Some(unix_timestamp_seconds());
        save_transcription_status(path, &status)?;
    }
    Ok(status)
}

fn update_transcription_job(
    jobs: &Mutex<HashMap<String, TranscriptionJob>>,
    pid: &str,
    phase: &str,
    progress: u8,
    details: Option<crate::asr::AsrProgress>,
) {
    if let Some(job) = jobs
        .lock()
        .expect("transcription state poisoned")
        .get_mut(pid)
    {
        if job.status.phase != phase {
            tracing::info!(
                pipeline = "transcription",
                pid,
                phase,
                "pipeline phase changed"
            );
        }
        job.status.phase = phase.to_string();
        job.status.progress = advance_progress(job.status.progress, progress);
        job.status.updated_at = Some(unix_timestamp_seconds());
        if let Some(details) = details {
            job.status.current = details.current;
            job.status.total = details.total;
            job.status.device = details.device;
            job.status.elapsed_seconds = details.elapsed_seconds;
            job.status.cpu_percent = details.cpu_percent;
            job.status.peak_memory_mb = details.peak_memory_mb;
            job.status.memory_limit_mb = details.memory_limit_mb;
            job.status.mlx_active_memory_mb = details.mlx_active_memory_mb;
            job.status.mlx_cache_memory_mb = details.mlx_cache_memory_mb;
        }
    }
}

#[tauri::command]
pub async fn transcription_start(
    args: AutoArgs,
    state: tauri::State<'_, TranscriptionState>,
) -> AppResult<TranscriptionJobStatus> {
    let pid = args
        .pid
        .as_deref()
        .map(str::trim)
        .filter(|pid| !pid.is_empty())
        .ok_or_else(|| AppError::Schema("transcription requires a project id".into()))?
        .to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    let job_dir = resolve_project_root(args.out.clone()).join(&pid);
    let status_path = transcription_status_path(&pid, args.out.clone())?;
    let remove_incomplete_url_project = (args.media.starts_with("http://")
        || args.media.starts_with("https://"))
        && !job_dir.join("doc.json").exists();
    let now = unix_timestamp_seconds();
    let status = TranscriptionJobStatus {
        pid: pid.clone(),
        state: "running".into(),
        phase: "preparing".into(),
        progress: 0,
        current: None,
        total: None,
        device: None,
        elapsed_seconds: None,
        cpu_percent: None,
        peak_memory_mb: None,
        memory_limit_mb: None,
        mlx_active_memory_mb: None,
        mlx_cache_memory_mb: None,
        started_at: Some(now),
        updated_at: Some(now),
        error: None,
    };
    {
        let mut jobs = state.jobs.lock().expect("transcription state poisoned");
        if jobs
            .get(&pid)
            .is_some_and(|job| matches!(job.status.state.as_str(), "running" | "cancelling"))
        {
            return Err(AppError::Schema(
                "this project already has a transcription in progress".into(),
            ));
        }
        jobs.insert(
            pid.clone(),
            TranscriptionJob {
                status: status.clone(),
                cancel: cancel.clone(),
                status_path: status_path.clone(),
            },
        );
    }
    let initial_status = status.clone();
    let initial_status_path = status_path.clone();
    if let Err(error) = run_blocking("save transcription status", move || {
        save_transcription_status(&initial_status_path, &initial_status)
    })
    .await
    {
        state
            .jobs
            .lock()
            .expect("transcription state poisoned")
            .remove(&pid);
        return Err(error);
    }
    trace_pipeline_started("transcription", &pid);

    let jobs = state.jobs.clone();
    let task_pid = pid.clone();
    tauri::async_runtime::spawn(async move {
        let report_jobs = jobs.clone();
        let report_pid = task_pid.clone();
        let work = run_auto_impl(args, move |phase, progress, details| {
            update_transcription_job(&report_jobs, &report_pid, phase, progress, details);
        });
        let result = crate::proc::with_cancellation(cancel.clone(), work).await;
        if result.is_err() && remove_incomplete_url_project {
            let _ = std::fs::remove_dir_all(&job_dir);
        }
        let mut final_status = {
            let guard = jobs.lock().expect("transcription state poisoned");
            let Some(job) = guard.get(&task_pid) else {
                return;
            };
            let mut status = job.status.clone();
            match result {
                Ok(_) => {
                    status.state = "completed".into();
                    status.phase = "completed".into();
                    status.progress = 100;
                    status.error = None;
                }
                Err(AppError::Cancelled) => {
                    status.state = "cancelled".into();
                    status.phase = "cancelled".into();
                    status.error = None;
                }
                Err(error) => {
                    status.state = "failed".into();
                    status.phase = "failed".into();
                    status.error = Some(error.to_string());
                }
            }
            status.updated_at = Some(unix_timestamp_seconds());
            status
        };
        if let Some(job) = jobs
            .lock()
            .expect("transcription state poisoned")
            .get_mut(&task_pid)
        {
            job.status = final_status.clone();
        }
        let persisted = final_status.clone();
        if let Err(error) = persist_background_status(
            "save final transcription status",
            status_path,
            persisted,
            save_transcription_status,
        )
        .await
        {
            final_status.state = "failed".into();
            final_status.phase = "failed".into();
            final_status.error = Some(format!(
                "transcription finished but its recovery status could not be saved: {error}"
            ));
        }
        if let Some(job) = jobs
            .lock()
            .expect("transcription state poisoned")
            .get_mut(&task_pid)
        {
            job.status = final_status.clone();
        }
        trace_pipeline_finished(
            "transcription",
            &task_pid,
            &final_status.state,
            final_status.error.as_deref(),
        );
    });
    Ok(status)
}

#[tauri::command]
pub async fn transcription_status(
    pid: String,
    state: tauri::State<'_, TranscriptionState>,
) -> AppResult<TranscriptionJobStatus> {
    let active = state
        .jobs
        .lock()
        .expect("transcription state poisoned")
        .get(&pid)
        .map(|job| (job.status.clone(), job.status_path.clone()));
    if let Some((status, path)) = active {
        persist_background_status(
            "checkpoint transcription status",
            path.clone(),
            status.clone(),
            save_transcription_status,
        )
        .await?;
        let latest = state
            .jobs
            .lock()
            .expect("transcription state poisoned")
            .get(&pid)
            .map(|job| job.status.clone())
            .unwrap_or_else(|| status.clone());
        if latest.state != status.state
            || latest.phase != status.phase
            || latest.progress != status.progress
            || latest.updated_at != status.updated_at
        {
            persist_background_status(
                "checkpoint latest transcription status",
                path,
                latest.clone(),
                save_transcription_status,
            )
            .await?;
        }
        return Ok(latest);
    }
    let status_path = transcription_status_path(&pid, None)?;
    run_blocking("load transcription status", move || {
        load_recovered_transcription_status(&status_path).map_err(|error| match error {
            AppError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
                AppError::Schema("no transcription job for this project".into())
            }
            other => other,
        })
    })
    .await
}

#[tauri::command]
pub async fn transcription_cancel(
    pid: String,
    state: tauri::State<'_, TranscriptionState>,
) -> AppResult<TranscriptionJobStatus> {
    let (status, path) = {
        let mut jobs = state.jobs.lock().expect("transcription state poisoned");
        let job = jobs
            .get_mut(&pid)
            .ok_or_else(|| AppError::Schema("no transcription job for this project".into()))?;
        if job.status.state == "running" {
            job.cancel.store(true, Ordering::Relaxed);
            job.status.state = "cancelling".into();
            job.status.phase = "cancelling".into();
            job.status.updated_at = Some(unix_timestamp_seconds());
        }
        (job.status.clone(), job.status_path.clone())
    };
    persist_background_status(
        "checkpoint transcription cancellation",
        path,
        status.clone(),
        save_transcription_status,
    )
    .await?;
    Ok(status)
}

#[tauri::command]
pub async fn transcription_retry(
    pid: String,
    state: tauri::State<'_, TranscriptionState>,
) -> AppResult<TranscriptionJobStatus> {
    let dir = resolve_project_dir(&pid, None)?;
    let (media, lang, title) = run_blocking("transcription retry preparation", move || {
        let doc = Doc::load(&dir)?;
        Ok((
            doc.media.path.to_string_lossy().into_owned(),
            doc.meta.language,
            Some(doc.meta.title),
        ))
    })
    .await?;
    transcription_start(
        AutoArgs {
            media,
            pid: Some(pid),
            lang,
            title,
            out: None,
            model: None,
        },
        state,
    )
    .await
}

// ============================================================================
// Task / agent commands
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct TaskStartArgs {
    pub kind: String,
    pub pid: String,
    pub lang: Option<String>,
    pub root: Option<PathBuf>,
    #[serde(default)]
    pub stale_only: bool,
    #[serde(default)]
    pub groups: Vec<String>,
    pub align_fit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct TaskStartResult {
    pub pending: usize,
    pub ai_dir: PathBuf,
    pub agent_port: u16,
}

async fn launch_prepared_task(
    state: &AgentServerState,
    pid: String,
    task: crate::agent::task::PreparedTask,
) -> AppResult<(u16, bool, usize)> {
    run_blocking("AI provider preflight", || {
        validate_ai_provider_preflight(&crate::data::modelconfig::load())
    })
    .await?;
    let key = format!("{}::{}", task.project_dir.display(), task.kind);
    let (agent_port, allocator) = ensure_agent_server(state, None).await?;
    let pause = Arc::new(AtomicBool::new(false));
    {
        let mut active = state.active_tasks.lock().expect("state poisoned");
        if active.contains_key(&key) {
            return Ok((agent_port, false, 0));
        }
        active.insert(
            key.clone(),
            ActiveAgentTask {
                pause: pause.clone(),
                task: task.clone(),
            },
        );
    }
    let restore_allocator = allocator.clone();
    let restore_task = task.clone();
    let restored = match run_blocking("task recovery", move || {
        let restored = crate::agent::task::restore_or_enqueue(&restore_allocator, &restore_task)?;
        crate::agent::task::set_task_state(&restore_task, "running", None)?;
        Ok(restored)
    })
    .await
    {
        Ok(restored) => restored,
        Err(error) => {
            state
                .active_tasks
                .lock()
                .expect("state poisoned")
                .remove(&key);
            return Err(error);
        }
    };
    let project_mutation = project_mutation_mutex(&task.project_dir).await;
    let task_kind = task.kind.clone();
    let status_task = task.clone();
    let pending = task.calls.len();
    let active_tasks = state.active_tasks.clone();
    tracing::info!(
        pipeline = "ai-task",
        pid,
        kind = task_kind,
        pending,
        restored,
        "pipeline job started"
    );
    tokio::spawn(async move {
        let result = crate::agent::task::wait_and_apply_with_lock_and_pause(
            allocator,
            task,
            std::time::Duration::from_secs(30 * 60),
            project_mutation,
            pause,
        )
        .await;
        let terminal_state = match &result {
            Ok(_) => "completed",
            Err(error) if error.to_string().contains("paused after waiting") => "paused",
            Err(_) => "failed",
        };
        let terminal_error = result.as_ref().err().map(ToString::to_string);
        if let Err(error) = crate::agent::task::set_task_state(
            &status_task,
            terminal_state,
            terminal_error.as_deref(),
        ) {
            tracing::error!(
                pipeline = "ai-task",
                kind = task_kind,
                %error,
                "failed to persist task state"
            );
        }
        active_tasks.lock().expect("state poisoned").remove(&key);
        match result {
            Ok(applied) => tracing::info!(
                pipeline = "ai-task",
                pid,
                kind = task_kind,
                applied,
                "pipeline job finished"
            ),
            Err(error) => tracing::error!(
                pipeline = "ai-task",
                pid,
                kind = task_kind,
                %error,
                "pipeline job failed"
            ),
        }
    });
    Ok((agent_port, true, restored))
}

#[tauri::command]
pub async fn task_start(
    state: tauri::State<'_, AgentServerState>,
    args: TaskStartArgs,
) -> AppResult<TaskStartResult> {
    let task_pid = args.pid.clone();
    let dir = resolve_project_dir(&args.pid, args.root.clone())?;
    let kind = args.kind;
    let lang = args.lang;
    let task = run_blocking("task preparation", move || {
        if let Some(task) =
            crate::agent::task::load_matching_recoverable_task(&dir, &kind, lang.as_deref())?
        {
            Ok(task)
        } else {
            crate::agent::task::prepare_task_with_task_options(
                &dir,
                &kind,
                lang.as_deref(),
                crate::agent::task::TaskOptions {
                    stale_only: args.stale_only,
                    groups: args.groups,
                    align_fit: args.align_fit,
                },
            )
        }
    })
    .await?;
    let pending = task.calls.len();
    let ai_dir = task.ai_dir.clone();
    let (agent_port, _, _) = launch_prepared_task(&state, task_pid, task).await?;
    Ok(TaskStartResult {
        pending,
        ai_dir,
        agent_port,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskResumeResult {
    pub resumed: usize,
    pub recovered_submissions: usize,
    pub agent_port: Option<u16>,
}

#[tauri::command]
pub async fn task_resume(
    state: tauri::State<'_, AgentServerState>,
    pid: String,
    root: Option<PathBuf>,
) -> AppResult<TaskResumeResult> {
    let dir = resolve_project_dir(&pid, root)?;
    let tasks = run_blocking("load recoverable tasks", move || {
        crate::agent::task::load_resumable_tasks(&dir)
    })
    .await?;
    let mut resumed = 0;
    let mut recovered_submissions = 0;
    let mut agent_port = None;
    for task in tasks {
        let (port, started, recovered) = launch_prepared_task(&state, pid.clone(), task).await?;
        agent_port = Some(port);
        resumed += usize::from(started);
        recovered_submissions += recovered;
    }
    Ok(TaskResumeResult {
        resumed,
        recovered_submissions,
        agent_port,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPauseResult {
    pub paused: usize,
    pub queued_calls: usize,
    pub in_flight_calls: usize,
}

#[tauri::command]
pub async fn task_pause(
    state: tauri::State<'_, AgentServerState>,
    pid: String,
    kind: Option<String>,
    root: Option<PathBuf>,
) -> AppResult<TaskPauseResult> {
    let dir = resolve_project_dir(&pid, root)?;
    let prefix = format!("{}::", dir.display());
    let active = state
        .active_tasks
        .lock()
        .expect("state poisoned")
        .iter()
        .filter(|(key, task)| {
            key.starts_with(&prefix) && kind.as_deref().is_none_or(|kind| task.task.kind == kind)
        })
        .map(|(_, task)| task.clone())
        .collect::<Vec<_>>();
    if active.is_empty() {
        return Ok(TaskPauseResult {
            paused: 0,
            queued_calls: 0,
            in_flight_calls: 0,
        });
    }

    let allocator = state
        .allocator
        .lock()
        .expect("state poisoned")
        .as_ref()
        .map(|handle| handle.allocator.clone());
    let mut queued_calls = 0;
    let mut in_flight_calls = 0;
    for item in &active {
        item.pause.store(true, Ordering::Relaxed);
        if let Some(allocator) = allocator.as_ref() {
            let ids = item
                .task
                .calls
                .iter()
                .map(|prepared| prepared.call.id.clone())
                .collect::<HashSet<_>>();
            let (queued, in_flight) = allocator.pause_calls(&ids);
            queued_calls += queued;
            in_flight_calls += in_flight;
        }
    }
    let tasks = active
        .iter()
        .map(|item| item.task.clone())
        .collect::<Vec<_>>();
    run_blocking("persist paused tasks", move || {
        for task in &tasks {
            crate::agent::task::set_task_state(
                task,
                "paused",
                Some("Paused by user; in-flight model requests may finish and will be preserved."),
            )?;
        }
        Ok(())
    })
    .await?;

    Ok(TaskPauseResult {
        paused: active.len(),
        queued_calls,
        in_flight_calls,
    })
}

#[tauri::command]
pub async fn task_retry(
    state: tauri::State<'_, AgentServerState>,
    pid: String,
    kind: String,
    root: Option<PathBuf>,
) -> AppResult<TaskStartResult> {
    let dir = resolve_project_dir(&pid, root)?;
    let key = format!("{}::{kind}", dir.display());
    if state
        .active_tasks
        .lock()
        .expect("state poisoned")
        .contains_key(&key)
    {
        return Err(AppError::Schema(format!(
            "task {kind} is still running; pause it before retrying"
        )));
    }
    let task = run_blocking("retry task preparation", move || {
        crate::agent::task::prepare_retry_task(&dir, &kind)
    })
    .await?;
    let pending = task.calls.len();
    let ai_dir = task.ai_dir.clone();
    let (agent_port, _, _) = launch_prepared_task(&state, pid, task).await?;
    Ok(TaskStartResult {
        pending,
        ai_dir,
        agent_port,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPrioritizeResult {
    pub moved_calls: usize,
}

#[tauri::command]
pub async fn task_prioritize(
    state: tauri::State<'_, AgentServerState>,
    pid: String,
    kind: String,
    root: Option<PathBuf>,
) -> AppResult<TaskPrioritizeResult> {
    let dir = resolve_project_dir(&pid, root)?;
    let key = format!("{}::{kind}", dir.display());
    let task = state
        .active_tasks
        .lock()
        .expect("state poisoned")
        .get(&key)
        .map(|active| active.task.clone())
        .ok_or_else(|| AppError::Schema(format!("task {kind} is not currently running")))?;
    let allocator = state
        .allocator
        .lock()
        .expect("state poisoned")
        .as_ref()
        .map(|handle| handle.allocator.clone())
        .ok_or_else(|| AppError::Schema("agent allocator is not running".into()))?;
    let ids = task
        .calls
        .iter()
        .map(|prepared| prepared.call.id.clone())
        .collect::<HashSet<_>>();
    Ok(TaskPrioritizeResult {
        moved_calls: allocator.prioritize_calls(&ids),
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatus {
    pub pending: usize,
    pub done: usize,
    pub failed: usize,
    pub kinds: Vec<crate::agent::task::TaskKindStatus>,
    pub polish_quality: Option<crate::pipeline::polish::PolishQualityArtifact>,
}

#[tauri::command]
pub async fn task_status(pid: String, root: Option<PathBuf>) -> AppResult<TaskStatus> {
    let project_dir = resolve_project_dir(&pid, root)?;
    run_blocking("task status", move || {
        let kinds = crate::agent::task::task_kind_statuses(&project_dir);
        let pending = kinds.iter().map(|status| status.pending).sum();
        let done = kinds.iter().map(|status| status.done).sum();
        let failed = kinds.iter().map(|status| status.failed).sum();
        let polish_quality = crate::pipeline::polish::PolishQualityArtifact::load(
            &project_dir.join("ai/polish-quality.json"),
        )
        .ok();
        Ok(TaskStatus {
            pending,
            done,
            failed,
            kinds,
            polish_quality,
        })
    })
    .await
}

// ============================================================================
// Pipeline commands
// ============================================================================

#[tauri::command]
pub async fn finish_check_pid(
    pid: String,
    settings: Option<crate::data::export_settings::VideoExportSettings>,
    root: Option<PathBuf>,
) -> AppResult<Vec<FinishCheckItem>> {
    let dir = resolve_project_dir(&pid, root)?;
    run_blocking("finish check", move || {
        let doc = Doc::load(&dir)?;
        let cuts_path = dir.join("cuts.json");
        let cuts: ClipCuts = if cuts_path.exists() {
            serde_json::from_str(&std::fs::read_to_string(&cuts_path)?)?
        } else {
            ClipCuts::new()
        };
        let broll = crate::data::broll::load(&dir)?;
        let mut items = finish_check_emit_for_project(
            &doc,
            &cuts,
            &broll,
            &dir,
            working_head_is_committed(&dir, &doc)?,
        );
        if settings.is_some() {
            for item in &mut items {
                item.blockers.retain(|finding| {
                    finding.code == Code::TranslationEmpty
                        || finding.code.section() != Section::Translation
                });
                item.pass = item.blockers.is_empty();
            }
        }
        Ok(items
            .into_iter()
            .map(|i| FinishCheckItem {
                code: i.code.label().to_string(),
                ordinal: i.code as u32,
                pass: i.pass,
                blockers: i.blockers.iter().map(|b| b.message.clone()).collect(),
            })
            .collect())
    })
    .await
}

#[derive(Debug, Serialize)]
pub struct FinishCheckItem {
    pub code: String,
    pub ordinal: u32,
    pub pass: bool,
    pub blockers: Vec<String>,
}

#[tauri::command]
pub async fn cut_auto(pid: String, root: Option<PathBuf>) -> AppResult<usize> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("automatic cut save", move || {
        crate::data::edit_history::record(
            &dir,
            "Apply suggested cuts",
            || {
                let doc = Doc::load(&dir)?;
                let cuts_path = dir.join("cuts.json");
                let mut cuts: ClipCuts = if cuts_path.exists() {
                    serde_json::from_str(&std::fs::read_to_string(&cuts_path)?)?
                } else {
                    ClipCuts::new()
                };
                let added = crate::pipeline::cleanup::apply(&doc, &mut cuts);
                if added > 0 {
                    crate::data::storage::write_json(&cuts_path, &cuts)?;
                }
                Ok(added)
            },
            |added| *added > 0,
        )
    })
    .await
}

#[derive(Debug)]
struct ManualCutCandidate {
    a_word: String,
    b_word: String,
    end: f64,
    note: String,
    start: f64,
}

fn add_manual_cuts(dir: &std::path::Path, cue_ids: Vec<String>) -> AppResult<usize> {
    if cue_ids.is_empty() {
        return Err(AppError::Schema("select at least one subtitle".into()));
    }
    let label = if cue_ids.len() == 1 {
        "Remove timeline region"
    } else {
        "Remove timeline regions"
    };
    crate::data::edit_history::record(
        dir,
        label,
        || {
            let doc = Doc::load(dir)?;
            let mut seen = HashSet::new();
            let mut candidates = Vec::with_capacity(cue_ids.len());
            for cue_id in cue_ids {
                if !seen.insert(cue_id.clone()) {
                    continue;
                }
                let sentence = doc
                    .paragraphs
                    .iter()
                    .flat_map(|paragraph| paragraph.sentences.iter())
                    .find(|sentence| sentence.id == cue_id)
                    .ok_or_else(|| {
                        AppError::Schema(format!("subtitle {cue_id} is no longer available"))
                    })?;
                let first = sentence
                    .words
                    .first()
                    .ok_or_else(|| AppError::Schema("subtitle has no timed words".into()))?;
                let last = sentence
                    .words
                    .last()
                    .ok_or_else(|| AppError::Schema("subtitle has no timed words".into()))?;
                candidates.push(ManualCutCandidate {
                    a_word: first.id.clone(),
                    b_word: last.id.clone(),
                    end: last.end,
                    note: sentence.text.clone(),
                    start: first.start,
                });
            }
            candidates.sort_by(|left, right| {
                left.start
                    .partial_cmp(&right.start)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            // Adjacent or overlapping subtitle selections are one contiguous
            // removal. Keeping them as separate cuts would double-count their
            // overlap in duration checks and make one user action harder to
            // reason about.
            let mut groups: Vec<ManualCutCandidate> = Vec::with_capacity(candidates.len());
            for candidate in candidates {
                if let Some(previous) = groups.last_mut() {
                    if candidate.start <= previous.end + 0.001 {
                        if candidate.end > previous.end {
                            previous.end = candidate.end;
                            previous.b_word = candidate.b_word;
                        }
                        if !candidate.note.is_empty() {
                            if !previous.note.is_empty() {
                                previous.note.push(' ');
                            }
                            previous.note.push_str(&candidate.note);
                        }
                        continue;
                    }
                }
                groups.push(candidate);
            }

            let cuts_path = dir.join("cuts.json");
            let mut cuts: ClipCuts = if cuts_path.exists() {
                serde_json::from_str(&std::fs::read_to_string(&cuts_path)?)?
            } else {
                ClipCuts::new()
            };
            let existing = cuts
                .cuts
                .iter()
                .filter_map(|cut| cut.resolved_interval(&doc))
                .collect::<Vec<_>>();
            if groups.iter().any(|candidate| {
                existing.iter().any(|(cut_start, cut_end)| {
                    candidate.start < *cut_end && *cut_start < candidate.end
                })
            }) {
                return Err(AppError::Schema(
                    "the selected subtitles overlap an existing removed region".into(),
                ));
            }

            let added = groups.len();
            for candidate in groups {
                cuts.add(crate::data::Cut {
                    id: format!("manual-{}", uuid::Uuid::new_v4().simple()),
                    note: (!candidate.note.is_empty()).then_some(candidate.note),
                    a_word: candidate.a_word,
                    b_word: candidate.b_word,
                    kind: crate::data::CutKind::Manual,
                    duration: (candidate.end - candidate.start).max(0.0),
                });
            }
            crate::data::storage::write_json(&cuts_path, &cuts)?;
            Ok(added)
        },
        |added| *added > 0,
    )
}

#[tauri::command]
pub async fn cut_manual(pid: String, cue_id: String, root: Option<PathBuf>) -> AppResult<bool> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("manual timeline cut", move || {
        Ok(add_manual_cuts(&dir, vec![cue_id])? > 0)
    })
    .await
}

#[tauri::command]
pub async fn cut_manual_many(
    pid: String,
    cue_ids: Vec<String>,
    root: Option<PathBuf>,
) -> AppResult<usize> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("manual timeline cuts", move || {
        add_manual_cuts(&dir, cue_ids)
    })
    .await
}

#[tauri::command]
pub async fn cut_restore(pid: String, cut_id: String, root: Option<PathBuf>) -> AppResult<bool> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("cut restore", move || {
        crate::data::edit_history::record(
            &dir,
            "Restore timeline region",
            || {
                let cuts_path = dir.join("cuts.json");
                let mut cuts: ClipCuts = if cuts_path.exists() {
                    serde_json::from_str(&std::fs::read_to_string(&cuts_path)?)?
                } else {
                    return Ok(false);
                };
                let removed = cuts.restore(&cut_id);
                if removed {
                    crate::data::storage::write_json(&cuts_path, &cuts)?;
                }
                Ok(removed)
            },
            |changed| *changed,
        )
    })
    .await
}

#[derive(Debug, Serialize)]
pub struct CutSummary {
    pub id: String,
    pub kind: String,
    pub a_word: String,
    pub b_word: String,
    pub duration: f64,
    pub note: Option<String>,
}

#[tauri::command]
pub async fn cut_list(pid: String, root: Option<PathBuf>) -> AppResult<Vec<CutSummary>> {
    let dir = resolve_project_dir(&pid, root)?;
    run_blocking("cut list", move || {
        let cuts_path = dir.join("cuts.json");
        let cuts: ClipCuts = if cuts_path.exists() {
            serde_json::from_str(&std::fs::read_to_string(&cuts_path)?)?
        } else {
            return Ok(vec![]);
        };
        Ok(cuts
            .cuts
            .iter()
            .map(|c| CutSummary {
                id: c.id.clone(),
                kind: format!("{:?}", c.kind).to_lowercase(),
                a_word: c.a_word.clone(),
                b_word: c.b_word.clone(),
                duration: c.duration,
                note: c.note.clone(),
            })
            .collect())
    })
    .await
}

/// Settings as sent by the frontend over IPC (snake_case) and persisted
/// to `~/.lumen-cut/settings.json` in camelCase.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    rename_all(serialize = "camelCase", deserialize = "snake_case"),
    default
)]
pub struct SettingsPayload {
    pub asr_model: String,
    pub asr_aligner: String,
    pub asr_engine: crate::data::modelconfig::AsrEngine,
    pub asr_cloud_endpoint: String,
    pub asr_cloud_api_key: String,
    pub asr_cloud_model: String,
    pub diarize_model: String,
    pub hf_token: String,
    pub llm_endpoint: String,
    pub llm_api_key: String,
    pub llm_model: String,
    pub worker_count: u32,
}

static SETTINGS_MUTATIONS: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

impl Default for SettingsPayload {
    fn default() -> Self {
        let config = crate::data::modelconfig::ModelConfig::default();
        Self {
            asr_model: config.asr_model,
            asr_aligner: config.asr_aligner,
            asr_engine: config.asr_engine,
            asr_cloud_endpoint: config.asr_cloud_endpoint,
            asr_cloud_api_key: config.asr_cloud_api_key,
            asr_cloud_model: config.asr_cloud_model,
            diarize_model: config.diarize_model,
            hf_token: config.hf_token,
            llm_endpoint: config.llm_endpoint,
            llm_api_key: config.llm_api_key,
            llm_model: config.llm_model,
            worker_count: config.worker_count,
        }
    }
}

#[tauri::command]
pub async fn settings_export(
    state: tauri::State<'_, AgentServerState>,
    mut settings: SettingsPayload,
) -> AppResult<String> {
    let _mutation = SETTINGS_MUTATIONS
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    // Secrets are write-only to the webview. An empty field preserves a key
    // only while its matching endpoint is unchanged; switching providers must
    // never silently reuse credentials issued for the previous host.
    let previous = crate::data::modelconfig::load();
    if settings.hf_token.trim().is_empty() {
        settings.hf_token = previous.hf_token;
    }
    if settings.llm_api_key.trim().is_empty()
        && settings.llm_endpoint.trim() == previous.llm_endpoint.trim()
    {
        settings.llm_api_key = previous.llm_api_key;
    }
    if settings.asr_cloud_api_key.trim().is_empty()
        && settings.asr_cloud_endpoint.trim() == previous.asr_cloud_endpoint.trim()
    {
        settings.asr_cloud_api_key = previous.asr_cloud_api_key;
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let persisted = settings.clone();
    let path = run_blocking("settings save", move || {
        let path = write_settings_file(&home, &settings)?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await?;
    apply_worker_count(&state, &persisted);
    Ok(path)
}

fn write_settings_file(home: &std::path::Path, settings: &SettingsPayload) -> AppResult<PathBuf> {
    let dir = home.join(".lumen-cut");
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let path = dir.join("settings.json");
    let temporary = dir.join(format!(
        "settings.json.{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let body = serde_json::to_string_pretty(&settings)?;
    let result = (|| -> AppResult<()> {
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(body.as_bytes())?;
            file.sync_all()?;
        }
        #[cfg(not(unix))]
        std::fs::write(&temporary, body)?;
        std::fs::rename(&temporary, &path)?;
        #[cfg(unix)]
        std::fs::File::open(&dir)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result?;
    Ok(path)
}

/// Make `workerCount` effective: it is the allocator capacity for future
/// `agent_serve` calls, and a live server is resized in place.
fn apply_worker_count(state: &AgentServerState, settings: &SettingsPayload) {
    let cap = (settings.worker_count as usize).max(1);
    *state.worker_count.lock().expect("state poisoned") = cap;
    if let Some(h) = state.allocator.lock().expect("state poisoned").as_ref() {
        h.allocator.set_capacity(cap);
    }
}

#[tauri::command]
pub async fn audit_pid(pid: String, root: Option<PathBuf>) -> AppResult<ReportSummary> {
    let dir = resolve_project_dir(&pid, root)?;
    run_blocking("project audit", move || {
        let doc = Doc::load(&dir)?;
        let cuts: ClipCuts = std::fs::read_to_string(dir.join("cuts.json"))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        let broll = crate::data::broll::load(&dir)?;
        let r = audit_project(&doc, &cuts, &broll, &dir);
        Ok(ReportSummary::from(&r))
    })
    .await
}

#[derive(Debug, Serialize)]
pub struct ReportSummary {
    pub findings: Vec<FindingSummary>,
    pub has_failures: bool,
    pub has_warnings: bool,
}

#[derive(Debug, Serialize)]
pub struct FindingSummary {
    pub code: String,
    pub severity: String,
    pub location: String,
    pub message: String,
}

impl From<&Finding> for FindingSummary {
    fn from(f: &Finding) -> Self {
        Self {
            code: f.code.label().to_string(),
            severity: format!("{:?}", f.severity).to_lowercase(),
            location: f.where_.clone(),
            message: f.message.clone(),
        }
    }
}

impl From<&Report> for ReportSummary {
    fn from(r: &Report) -> Self {
        Self {
            findings: r.findings.iter().map(FindingSummary::from).collect(),
            has_failures: r.has_failures(),
            has_warnings: r.has_warnings(),
        }
    }
}

#[tauri::command]
pub async fn version_merge(
    base: BTreeMap<String, String>,
    ours: BTreeMap<String, String>,
    theirs: BTreeMap<String, String>,
) -> AppResult<MergeSummary> {
    let out = three_way_merge(&base, &ours, &theirs);
    Ok(MergeSummary {
        merged: out.merged,
        conflicts: out
            .conflicts
            .into_iter()
            .map(|c| ConflictSummary {
                cue_id: c.cue_id,
                base: c.base,
                ours: c.ours,
                theirs: c.theirs,
            })
            .collect(),
    })
}

#[derive(Debug, Serialize)]
pub struct MergeSummary {
    pub merged: BTreeMap<String, String>,
    pub conflicts: Vec<ConflictSummary>,
}

#[derive(Debug, Serialize)]
pub struct ConflictSummary {
    pub cue_id: String,
    pub base: String,
    pub ours: String,
    pub theirs: String,
}

// ============================================================================
// Agent server (for the Pipeline view to drive an LLM worker)
// ============================================================================

pub struct AgentServerState {
    pub allocator: Mutex<Option<AllocatorHandle>>,
    /// Configured worker count, used as the allocator capacity for future
    /// `agent_serve` calls.
    pub worker_count: Mutex<usize>,
    pub built_in_workers_started: Mutex<bool>,
    /// Project/kind pairs with a live apply loop. Durable task recovery may be
    /// requested repeatedly as views mount; this prevents duplicate enqueue.
    active_tasks: Arc<Mutex<HashMap<String, ActiveAgentTask>>>,
}

#[derive(Clone)]
struct ActiveAgentTask {
    pause: Arc<AtomicBool>,
    task: crate::agent::task::PreparedTask,
}

/// One app-wide microphone capture. Recording is intentionally separate from
/// project creation: the UI starts capture immediately, lets the user decide
/// when to stop, then creates the project from the finalized WAV.
pub struct RecordingState {
    session: Mutex<Option<RecordingSession>>,
    starting: AtomicBool,
}

struct RecordingSession {
    pid: String,
    wav: PathBuf,
    child: tokio::process::Child,
    _mutation: tokio::sync::OwnedMutexGuard<()>,
}

impl Default for RecordingState {
    fn default() -> Self {
        Self {
            session: Mutex::new(None),
            starting: AtomicBool::new(false),
        }
    }
}

impl Drop for RecordingState {
    fn drop(&mut self) {
        if let Ok(slot) = self.session.get_mut() {
            if let Some(session) = slot.as_mut() {
                let _ = session.child.start_kill();
                let _ = std::fs::remove_file(&session.wav);
                if let Some(dir) = session.wav.parent() {
                    let _ = std::fs::remove_dir(dir);
                }
            }
        }
    }
}

impl Default for AgentServerState {
    fn default() -> Self {
        Self {
            allocator: Mutex::new(None),
            worker_count: Mutex::new(crate::agent::DEFAULT_CAPACITY),
            built_in_workers_started: Mutex::new(false),
            active_tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

pub struct AllocatorHandle {
    pub allocator: std::sync::Arc<Allocator>,
    pub addr: std::net::SocketAddr,
    pub pool: std::sync::Arc<std::sync::Mutex<crate::agent::pool::WorkerPool>>,
}

async fn ensure_agent_server(
    state: &AgentServerState,
    port: Option<u16>,
) -> AppResult<(u16, std::sync::Arc<Allocator>)> {
    let existing = {
        let slot = state.allocator.lock().expect("state poisoned");
        slot.as_ref()
            .map(|handle| (handle.addr.port(), handle.allocator.clone()))
    };
    if let Some(existing) = existing {
        maybe_spawn_builtin_workers(state, existing.1.clone()).await;
        return Ok(existing);
    }

    use tokio::net::TcpListener;
    let port = port.unwrap_or(0);
    let capacity = *state.worker_count.lock().expect("state poisoned");
    let allocator = std::sync::Arc::new(Allocator::new(capacity));
    let pool = std::sync::Arc::new(std::sync::Mutex::new(
        crate::agent::pool::WorkerPool::new_workers(capacity),
    ));
    let (addr, router) = crate::agent::http::bind(port, allocator.clone(), pool.clone())
        .await
        .map_err(AppError::Io)?;
    let listener = TcpListener::bind(addr).await.map_err(AppError::Io)?;
    let local_addr = listener.local_addr().map_err(AppError::Io)?;
    {
        let mut slot = state.allocator.lock().expect("state poisoned");
        *slot = Some(AllocatorHandle {
            allocator: allocator.clone(),
            addr: local_addr,
            pool,
        });
    }
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            tracing::error!(%error, "agent server stopped");
        }
    });
    maybe_spawn_builtin_workers(state, allocator.clone()).await;
    Ok((local_addr.port(), allocator))
}

async fn maybe_spawn_builtin_workers(
    state: &AgentServerState,
    allocator: std::sync::Arc<Allocator>,
) {
    if crate::agent::runtime::load_bridge_config().is_none() {
        return;
    }
    let should_start = {
        let mut started = state
            .built_in_workers_started
            .lock()
            .expect("state poisoned");
        if *started {
            false
        } else {
            *started = true;
            true
        }
    };
    if should_start {
        let count = *state.worker_count.lock().expect("state poisoned");
        crate::agent::runtime::spawn_workers(allocator, count).await;
    }
}

#[tauri::command]
pub async fn agent_serve(
    state: tauri::State<'_, AgentServerState>,
    port: Option<u16>,
) -> AppResult<u16> {
    let (port, _) = ensure_agent_server(&state, port).await?;
    Ok(port)
}

#[tauri::command]
pub async fn agent_enqueue(
    state: tauri::State<'_, AgentServerState>,
    call_id: String,
    kind: String,
    word_count: usize,
    payload_ref: String,
    problems: Option<Vec<String>>,
) -> AppResult<()> {
    let contract = crate::agent::contract::contract_for_kind(&kind).map(str::to_string);
    let payload_path = payload_ref.clone();
    let char_count = run_blocking("agent payload sizing", move || {
        Ok(payload_char_count(&payload_path))
    })
    .await?;
    enqueue_call(
        &state,
        crate::agent::PendingCall {
            id: call_id,
            kind,
            word_count,
            char_count,
            payload_ref,
            submission_ref: None,
            problems: problems.unwrap_or_default(),
            contract,
        },
    )
}

fn enqueue_call(state: &AgentServerState, call: crate::agent::PendingCall) -> AppResult<()> {
    let g = state.allocator.lock().expect("state poisoned");
    let h = g
        .as_ref()
        .ok_or_else(|| AppError::Schema("agent not serving".into()))?;
    h.allocator.enqueue(call);
    Ok(())
}

/// Snapshot of the worker pool — who has heartbeated, who is stale.
/// Routes to the same `WorkerPool` the HTTP `/agent/workers` endpoint reads
/// so the GUI and external workers agree on state.
#[tauri::command]
pub async fn agent_workers(
    state: tauri::State<'_, AgentServerState>,
) -> AppResult<Vec<crate::agent::pool::WorkerStatus>> {
    let g = state.allocator.lock().expect("state poisoned");
    let h = g
        .as_ref()
        .ok_or_else(|| AppError::Schema("agent not serving".into()))?;
    let mut p = h.pool.lock().expect("pool poisoned");
    let _ = p.reap_stale();
    Ok(p.workers().to_vec())
}

/// Character count of the prompt payload backing a call — drives the
/// adaptive lease bucket.
/// Best effort: an unreadable payload falls back to 0 (small bucket).
fn payload_char_count(payload_ref: &str) -> usize {
    match std::fs::read_to_string(payload_ref) {
        Ok(s) => s.chars().count(),
        Err(e) => {
            tracing::debug!(payload_ref, "payload unreadable at enqueue: {e}");
            0
        }
    }
}

// ============================================================================
// Audit re-export for frontend (Code enum)
// ============================================================================

#[tauri::command]
pub async fn audit_codes() -> Vec<&'static str> {
    // Stable public audit-code labels in display order.
    Code::all().iter().map(|c| c.label()).collect()
}

// ============================================================================
// Project editing and export commands
// ============================================================================

#[tauri::command]
pub async fn subtitle_list(
    pid: String,
    root: Option<PathBuf>,
) -> AppResult<Vec<crate::data::subtitle::SubtitleRow>> {
    let dir = resolve_project_dir(&pid, root)?;
    run_blocking("subtitle list", move || {
        let doc = Doc::load(&dir)?;
        let hidden = crate::data::subtitle::load_hidden_checked(&dir)?;
        Ok(crate::data::subtitle::list(&doc, &hidden, None))
    })
    .await
}

#[tauri::command]
pub async fn subtitle_set(
    pid: String,
    id: String,
    text: String,
    root: Option<PathBuf>,
) -> AppResult<bool> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("subtitle update", move || {
        crate::data::edit_history::record(
            &dir,
            "Edit transcript",
            || {
                let mut doc = Doc::load(&dir)?;
                let changed = crate::data::subtitle::set(&mut doc, &id, &text);
                if changed {
                    doc.meta.updated_at = chrono::Utc::now();
                    doc.save(&dir)?;
                }
                Ok(changed)
            },
            |changed| *changed,
        )
    })
    .await
}

#[tauri::command]
pub async fn subtitle_timing_set(
    pid: String,
    id: String,
    start: f64,
    end: f64,
    root: Option<PathBuf>,
) -> AppResult<bool> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("subtitle timing update", move || {
        crate::data::edit_history::record(
            &dir,
            "Adjust subtitle timing",
            || {
                let mut doc = Doc::load(&dir)?;
                let changed = crate::data::subtitle::set_timing(&mut doc, &id, start, end)?;
                if changed {
                    doc.meta.updated_at = chrono::Utc::now();
                    doc.save(&dir)?;
                }
                Ok(changed)
            },
            |changed| *changed,
        )
    })
    .await
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleUpdate {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleUpdateResult {
    pub changed: usize,
    pub sentences: Vec<crate::data::Sentence>,
}

#[tauri::command]
pub async fn subtitle_update_many(
    pid: String,
    updates: Vec<SubtitleUpdate>,
    root: Option<PathBuf>,
) -> AppResult<SubtitleUpdateResult> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("subtitle batch update with timing", move || {
        crate::data::edit_history::record(
            &dir,
            if updates.len() == 1 {
                "Edit transcript"
            } else {
                "Edit transcript lines"
            },
            || {
                let mut doc = Doc::load(&dir)?;
                let sentence_ids = doc
                    .paragraphs
                    .iter()
                    .flat_map(|paragraph| paragraph.sentences.iter())
                    .map(|sentence| sentence.id.as_str())
                    .collect::<HashSet<_>>();
                let unknown_ids = updates
                    .iter()
                    .filter(|update| !sentence_ids.contains(update.id.as_str()))
                    .map(|update| update.id.clone())
                    .collect::<Vec<_>>();
                if !unknown_ids.is_empty() {
                    return Err(AppError::Schema(format!(
                        "transcript cues not found: {}",
                        unknown_ids.join(", ")
                    )));
                }

                let updates_by_id = updates
                    .iter()
                    .map(|update| (update.id.as_str(), update.text.as_str()))
                    .collect::<HashMap<_, _>>();
                let changed = crate::data::subtitle::set_many(&mut doc, &updates_by_id);
                if changed > 0 {
                    doc.meta.updated_at = chrono::Utc::now();
                    doc.save(&dir)?;
                }
                let by_id = doc
                    .paragraphs
                    .iter()
                    .flat_map(|paragraph| paragraph.sentences.iter())
                    .map(|sentence| (sentence.id.as_str(), sentence))
                    .collect::<HashMap<_, _>>();
                let sentences = updates
                    .iter()
                    .filter_map(|update| {
                        by_id
                            .get(update.id.as_str())
                            .map(|sentence| (*sentence).clone())
                    })
                    .collect();
                Ok(SubtitleUpdateResult { changed, sentences })
            },
            |result| result.changed > 0,
        )
    })
    .await
}

#[tauri::command]
pub async fn subtitle_set_many(
    pid: String,
    updates: Vec<SubtitleUpdate>,
    root: Option<PathBuf>,
) -> AppResult<usize> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("subtitle batch update", move || {
        crate::data::edit_history::record(
            &dir,
            "Edit transcript lines",
            || {
                let mut doc = Doc::load(&dir)?;
                let sentence_ids = doc
                    .paragraphs
                    .iter()
                    .flat_map(|paragraph| paragraph.sentences.iter())
                    .map(|sentence| sentence.id.as_str())
                    .collect::<HashSet<_>>();
                let unknown_ids = updates
                    .iter()
                    .filter(|update| !sentence_ids.contains(update.id.as_str()))
                    .map(|update| update.id.clone())
                    .collect::<Vec<_>>();
                if !unknown_ids.is_empty() {
                    return Err(AppError::Schema(format!(
                        "transcript cues not found: {}",
                        unknown_ids.join(", ")
                    )));
                }

                let updates_by_id = updates
                    .iter()
                    .map(|update| (update.id.as_str(), update.text.as_str()))
                    .collect::<HashMap<_, _>>();
                let changed = crate::data::subtitle::set_many(&mut doc, &updates_by_id);
                if changed > 0 {
                    doc.meta.updated_at = chrono::Utc::now();
                    doc.save(&dir)?;
                }
                Ok(changed)
            },
            |changed| *changed > 0,
        )
    })
    .await
}

#[tauri::command]
pub async fn chapter_list(
    pid: String,
    root: Option<PathBuf>,
) -> AppResult<Vec<crate::data::chapter::ChapterRow>> {
    let dir = resolve_project_dir(&pid, root)?;
    run_blocking("chapter list", move || {
        let doc = Doc::load(&dir)?;
        let chapters = crate::data::chapter::load(&dir)?;
        crate::data::chapter::rows(&doc, &chapters)
    })
    .await
}

#[tauri::command]
pub async fn chapter_set_many(
    pid: String,
    chapters: Vec<crate::data::chapter::Chapter>,
    root: Option<PathBuf>,
) -> AppResult<bool> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("chapter update", move || {
        crate::data::edit_history::record(
            &dir,
            "Edit chapters",
            || {
                let doc = Doc::load(&dir)?;
                crate::data::chapter::replace(&dir, &doc, chapters)
            },
            |changed| *changed,
        )
    })
    .await
}

#[tauri::command]
pub async fn translation_set(
    pid: String,
    lang: String,
    id: String,
    text: String,
    root: Option<PathBuf>,
) -> AppResult<bool> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("translation update", move || {
        crate::data::edit_history::record(
            &dir,
            "Edit translation",
            || {
                let mut doc = Doc::load(&dir)?;
                let changed = crate::data::subtitle::set_translation(&mut doc, &lang, &id, &text);
                if changed {
                    doc.meta.updated_at = chrono::Utc::now();
                    doc.save(&dir)?;
                }
                Ok(changed)
            },
            |changed| *changed,
        )
    })
    .await
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationUpdate {
    pub id: String,
    pub text: String,
}

#[tauri::command]
pub async fn translation_set_many(
    pid: String,
    lang: String,
    updates: Vec<TranslationUpdate>,
    root: Option<PathBuf>,
) -> AppResult<usize> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("translation batch update", move || {
        crate::data::edit_history::record(
            &dir,
            "Edit translations",
            || {
                let mut doc = Doc::load(&dir)?;
                let sentence_ids = doc
                    .paragraphs
                    .iter()
                    .flat_map(|paragraph| paragraph.sentences.iter())
                    .map(|sentence| sentence.id.as_str())
                    .collect::<HashSet<_>>();
                let unknown_ids = updates
                    .iter()
                    .filter(|update| !sentence_ids.contains(update.id.as_str()))
                    .map(|update| update.id.clone())
                    .collect::<Vec<_>>();
                if !unknown_ids.is_empty() {
                    return Err(AppError::Schema(format!(
                        "translation cues not found: {}",
                        unknown_ids.join(", ")
                    )));
                }

                let mut changed = 0;
                for update in &updates {
                    let previous = doc
                        .translations
                        .get(&lang)
                        .and_then(|track| track.get(&update.id))
                        .map(|translation| translation.text.as_str());
                    if previous == Some(update.text.as_str()) {
                        continue;
                    }
                    if crate::data::subtitle::set_translation(
                        &mut doc,
                        &lang,
                        &update.id,
                        &update.text,
                    ) {
                        changed += 1;
                    }
                }
                if changed > 0 {
                    doc.meta.updated_at = chrono::Utc::now();
                    doc.save(&dir)?;
                }
                Ok(changed)
            },
            |changed| *changed > 0,
        )
    })
    .await
}

#[tauri::command]
pub async fn subtitle_visibility(
    pid: String,
    id: String,
    hidden: bool,
    root: Option<PathBuf>,
) -> AppResult<bool> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("subtitle visibility", move || {
        crate::data::edit_history::record(
            &dir,
            if hidden {
                "Hide subtitle"
            } else {
                "Restore subtitle"
            },
            || {
                if hidden {
                    crate::data::subtitle::hide(&dir, &id)
                } else {
                    crate::data::subtitle::restore(&dir, &id)
                }
            },
            |changed| *changed,
        )
    })
    .await
}

#[tauri::command]
pub async fn subtitle_replace(
    pid: String,
    query: String,
    replacement: String,
    regex: bool,
    root: Option<PathBuf>,
) -> AppResult<usize> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("subtitle replacement", move || {
        crate::data::edit_history::record(
            &dir,
            "Replace transcript text",
            || {
                let mut doc = Doc::load(&dir)?;
                let changed =
                    crate::data::edit::find_replace(&mut doc, &query, &replacement, regex)?;
                if changed > 0 {
                    doc.meta.updated_at = chrono::Utc::now();
                    doc.save(&dir)?;
                }
                Ok(changed)
            },
            |changed| *changed > 0,
        )
    })
    .await
}

#[tauri::command]
pub async fn edit_history_status(
    pid: String,
    root: Option<PathBuf>,
) -> AppResult<crate::data::edit_history::EditHistoryStatus> {
    let dir = resolve_project_dir(&pid, root)?;
    run_blocking("edit history status", move || {
        crate::data::edit_history::status(&dir)
    })
    .await
}

#[tauri::command]
pub async fn edit_undo(
    pid: String,
    root: Option<PathBuf>,
) -> AppResult<crate::data::edit_history::EditHistoryAction> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("editor undo", move || crate::data::edit_history::undo(&dir)).await
}

#[tauri::command]
pub async fn edit_redo(
    pid: String,
    root: Option<PathBuf>,
) -> AppResult<crate::data::edit_history::EditHistoryAction> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("editor redo", move || crate::data::edit_history::redo(&dir)).await
}

#[tauri::command]
pub async fn speakers_list(
    pid: String,
    root: Option<PathBuf>,
) -> AppResult<Vec<crate::data::speakers::SpeakerInfo>> {
    let dir = resolve_project_dir(&pid, root)?;
    run_blocking("speaker list", move || {
        let doc = Doc::load(&dir)?;
        Ok(crate::data::speakers::list(&doc))
    })
    .await
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerEvidence {
    pub speakers: Vec<crate::data::speakers::SpeakerInfo>,
    pub turns: Vec<crate::data::speakers::SpeakerTurn>,
    pub identified: bool,
    pub unlabelled: usize,
}

#[tauri::command]
pub async fn speaker_evidence(pid: String, root: Option<PathBuf>) -> AppResult<SpeakerEvidence> {
    let dir = resolve_project_dir(&pid, root)?;
    run_blocking("speaker evidence", move || {
        let doc = Doc::load(&dir)?;
        let turns = crate::data::speakers::turns(&doc);
        let unlabelled = turns.iter().filter(|turn| turn.speaker.is_none()).count();
        Ok(SpeakerEvidence {
            speakers: crate::data::speakers::list(&doc),
            identified: unlabelled == 0 && !turns.is_empty(),
            turns,
            unlabelled,
        })
    })
    .await
}

#[tauri::command]
pub async fn speaker_rename(
    pid: String,
    from: String,
    to: String,
    root: Option<PathBuf>,
) -> AppResult<usize> {
    let from = from.trim().to_owned();
    let to = to.trim().to_owned();
    if from.is_empty() || to.is_empty() {
        return Err(AppError::Schema("speaker names cannot be empty".into()));
    }
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("speaker rename", move || {
        crate::data::edit_history::record(
            &dir,
            "Rename speaker",
            || {
                let mut doc = Doc::load(&dir)?;
                let changed = crate::data::speakers::rename(&mut doc, &from, &to);
                if changed > 0 {
                    doc.meta.updated_at = chrono::Utc::now();
                    doc.save(&dir)?;
                }
                Ok(changed)
            },
            |changed| *changed > 0,
        )
    })
    .await
}

#[tauri::command]
pub async fn speaker_merge(
    pid: String,
    from: String,
    into: String,
    root: Option<PathBuf>,
) -> AppResult<usize> {
    let from = from.trim().to_owned();
    let into = into.trim().to_owned();
    if from.is_empty() || into.is_empty() || from == into {
        return Err(AppError::Schema(
            "speaker merge requires two different non-empty names".into(),
        ));
    }
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("speaker merge", move || {
        crate::data::edit_history::record(
            &dir,
            "Merge speakers",
            || {
                let original = Doc::load(&dir)?;
                let mut doc = original.clone();
                let changed = crate::data::speakers::merge(&mut doc, &from, &into);
                if changed > 0 {
                    if !working_head_is_committed(&dir, &original)? {
                        let mut lineage = crate::data::version::Lineage::load(&dir)?;
                        let branch = lineage
                            .active_branch
                            .clone()
                            .unwrap_or_else(|| "main".into());
                        crate::data::version::commit_snapshot(
                            &dir,
                            &original,
                            &mut lineage,
                            &branch,
                            "Before speaker merge",
                            "automatic recovery snapshot",
                            crate::data::version::VersionKind::Auto,
                        )?;
                    }
                    doc.meta.updated_at = chrono::Utc::now();
                    doc.save(&dir)?;
                }
                Ok(changed)
            },
            |changed| *changed > 0,
        )
    })
    .await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerAssignmentInput {
    pub paragraph_id: u32,
    pub speaker: Option<String>,
}

#[tauri::command]
pub async fn speaker_assign(
    pid: String,
    input: SpeakerAssignmentInput,
    root: Option<PathBuf>,
) -> AppResult<()> {
    let speaker = input
        .speaker
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("speaker assignment", move || {
        crate::data::edit_history::record(
            &dir,
            "Assign speaker",
            || {
                let mut doc = Doc::load(&dir)?;
                let paragraph = doc
                    .paragraphs
                    .iter_mut()
                    .find(|paragraph| paragraph.id == input.paragraph_id)
                    .ok_or_else(|| {
                        AppError::Schema(format!("paragraph {} was not found", input.paragraph_id))
                    })?;
                let changed = paragraph.speaker != speaker;
                if changed {
                    paragraph.speaker = speaker;
                    doc.meta.updated_at = chrono::Utc::now();
                    doc.save(&dir)?;
                }
                Ok(changed)
            },
            |changed| *changed,
        )
        .map(|_| ())
    })
    .await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerReidentifyProposal {
    pub paragraph_id: u32,
    pub current: Option<String>,
    pub cluster: String,
    pub proposed: String,
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub coverage: f64,
    pub margin: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerReidentifyPreview {
    pub segments: usize,
    pub changed: usize,
    pub unassigned: usize,
    pub proposals: Vec<SpeakerReidentifyProposal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerAnalysisJobStatus {
    pub pid: String,
    pub state: String,
    pub phase: String,
    pub progress: u8,
    pub current: Option<u32>,
    pub total: Option<u32>,
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub elapsed_seconds: Option<f64>,
    #[serde(default)]
    pub cpu_percent: Option<u32>,
    #[serde(default)]
    pub peak_memory_mb: Option<u64>,
    #[serde(default)]
    pub memory_limit_mb: Option<u64>,
    #[serde(default)]
    pub started_at: Option<u64>,
    #[serde(default)]
    pub updated_at: Option<u64>,
    pub error: Option<String>,
    pub preview: Option<SpeakerReidentifyPreview>,
}

struct SpeakerAnalysisJob {
    status: SpeakerAnalysisJobStatus,
    cancel: Arc<AtomicBool>,
    status_path: PathBuf,
}

#[derive(Clone, Default)]
pub struct SpeakerAnalysisState {
    jobs: Arc<Mutex<HashMap<String, SpeakerAnalysisJob>>>,
}

fn speaker_analysis_status_path(pid: &str, root: Option<PathBuf>) -> AppResult<PathBuf> {
    let _ = resolve_project_dir(pid, root.clone())?;
    Ok(resolve_project_root(root)
        .join(".jobs")
        .join(format!("{pid}.speakers.json")))
}

fn save_speaker_analysis_status(
    path: &std::path::Path,
    status: &SpeakerAnalysisJobStatus,
) -> AppResult<()> {
    crate::data::storage::write_json(path, status)
}

fn load_speaker_analysis_status(path: &std::path::Path) -> AppResult<SpeakerAnalysisJobStatus> {
    Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
}

fn load_recovered_speaker_analysis_status(
    path: &std::path::Path,
) -> AppResult<SpeakerAnalysisJobStatus> {
    let mut status = load_speaker_analysis_status(path)?;
    if matches!(status.state.as_str(), "running" | "cancelling") {
        status.state = "failed".into();
        status.phase = "failed".into();
        status.error = Some(
            "the previous speaker analysis was interrupted when lumen-cut closed; start it again"
                .into(),
        );
        status.updated_at = Some(unix_timestamp_seconds());
        save_speaker_analysis_status(path, &status)?;
    }
    Ok(status)
}

fn update_speaker_analysis_job(
    jobs: &Mutex<HashMap<String, SpeakerAnalysisJob>>,
    pid: &str,
    progress: crate::diarize::DiarizeProgress,
) {
    if let Some(job) = jobs
        .lock()
        .expect("speaker analysis state poisoned")
        .get_mut(pid)
    {
        if job.status.phase != progress.phase {
            tracing::info!(
                pipeline = "speaker-analysis",
                pid,
                phase = progress.phase,
                "pipeline phase changed"
            );
        }
        job.status.phase = progress.phase;
        job.status.progress = advance_progress(job.status.progress, progress.progress);
        job.status.current = progress.current;
        job.status.total = progress.total;
        job.status.device = progress.device;
        job.status.elapsed_seconds = progress.elapsed_seconds;
        job.status.cpu_percent = progress.cpu_percent;
        job.status.peak_memory_mb = progress.peak_memory_mb;
        job.status.memory_limit_mb = progress.memory_limit_mb;
        job.status.updated_at = Some(unix_timestamp_seconds());
    }
}

fn paragraph_bounds(paragraph: &crate::data::Paragraph) -> Option<(f64, f64)> {
    let mut words = paragraph
        .sentences
        .iter()
        .flat_map(|sentence| sentence.words.iter());
    let first = words.next()?;
    let end = words.last().unwrap_or(first).end;
    Some((first.start, end))
}

fn speaker_preview_matches_doc(doc: &Doc, preview: &SpeakerReidentifyPreview) -> bool {
    let mut doc = doc.clone();
    crate::diarize::normalize_speaker_paragraphs(&mut doc);
    preview.proposals.iter().all(|proposal| {
        doc.paragraphs
            .iter()
            .find(|paragraph| paragraph.id == proposal.paragraph_id)
            .and_then(|paragraph| {
                let bounds = paragraph_bounds(paragraph)?;
                let text = paragraph
                    .sentences
                    .iter()
                    .map(|sentence| sentence.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                Some(
                    paragraph.speaker == proposal.current
                        && (bounds.0 - proposal.start).abs() <= 0.001
                        && (bounds.1 - proposal.end).abs() <= 0.001
                        && text == proposal.text,
                )
            })
            .unwrap_or(false)
    })
}

#[tauri::command]
pub async fn speaker_reidentify_preview(
    pid: String,
    root: Option<PathBuf>,
) -> AppResult<SpeakerReidentifyPreview> {
    speaker_reidentify_preview_impl(pid, root, None).await
}

async fn speaker_reidentify_preview_impl(
    pid: String,
    root: Option<PathBuf>,
    on_progress: Option<crate::diarize::DiarizeProgressCallback>,
) -> AppResult<SpeakerReidentifyPreview> {
    if let Some(callback) = on_progress.as_ref() {
        callback(crate::diarize::DiarizeProgress {
            phase: "waiting".into(),
            progress: 0,
            current: None,
            total: None,
            device: None,
            elapsed_seconds: None,
            cpu_percent: None,
            peak_memory_mb: None,
            memory_limit_mb: None,
        });
    }
    let _heavy_work = crate::performance::acquire_heavy("speaker-analysis").await?;
    let dir = resolve_project_dir(&pid, root)?;
    let audio_mutation = lock_project_mutation(&dir).await;
    let load_dir = dir.clone();
    let (doc, model) = run_blocking("speaker preview preparation", move || {
        let mut doc = Doc::load(&load_dir)?;
        crate::diarize::normalize_speaker_paragraphs(&mut doc);
        let model = crate::data::modelconfig::load().diarize_model;
        validate_speaker_preflight(&model)?;
        Ok((doc, model))
    })
    .await?;
    let wav = dir.join("audio.wav");
    if !tokio::fs::try_exists(&wav).await? {
        extract_audio_wav(&doc.media.path, &wav).await?;
    }
    drop(audio_mutation);
    let output =
        crate::diarize::diarize_file_with_model_progress(&wav, &model, on_progress).await?;
    let segment_count = output.segments.len();
    run_blocking("speaker preview", move || {
        let mut unassigned = 0;
        let matches = doc
            .paragraphs
            .iter()
            .filter_map(|paragraph| {
                let Some((start, end)) = paragraph_bounds(paragraph) else {
                    unassigned += 1;
                    return None;
                };
                let Some(matched) = crate::diarize::match_paragraph(paragraph, &output.segments)
                else {
                    unassigned += 1;
                    return None;
                };
                Some((paragraph, matched, start, end))
            })
            .collect::<Vec<_>>();
        // A fresh diarizer uses anonymous cluster ids. Preserve human names by
        // greedily matching each new cluster to the current label with the
        // greatest measured overlap, while keeping the mapping one-to-one.
        let mut scores = BTreeMap::<(String, String), f64>::new();
        for (paragraph, matched, _, _) in &matches {
            if let Some(current) = paragraph.speaker.as_ref() {
                *scores
                    .entry((matched.speaker.clone(), current.clone()))
                    .or_default() += matched.covered_seconds;
            }
        }
        let mut ranked = scores.into_iter().collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.0.cmp(&right.0))
        });
        let mut cluster_names = HashMap::<String, String>::new();
        let mut used_names = HashSet::<String>::new();
        for ((cluster, current), _) in ranked {
            if !cluster_names.contains_key(&cluster) && used_names.insert(current.clone()) {
                cluster_names.insert(cluster, current);
            }
        }
        let proposals = matches
            .into_iter()
            .map(|(paragraph, matched, start, end)| {
                let cluster = matched.speaker;
                SpeakerReidentifyProposal {
                    paragraph_id: paragraph.id,
                    current: paragraph.speaker.clone(),
                    proposed: cluster_names
                        .get(&cluster)
                        .cloned()
                        .unwrap_or_else(|| cluster.clone()),
                    cluster,
                    start,
                    end,
                    text: paragraph
                        .sentences
                        .iter()
                        .map(|sentence| sentence.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                    coverage: matched.coverage,
                    margin: matched.margin,
                }
            })
            .collect::<Vec<_>>();
        let changed = proposals
            .iter()
            .filter(|proposal| proposal.current.as_deref() != Some(proposal.proposed.as_str()))
            .count();
        Ok(SpeakerReidentifyPreview {
            segments: segment_count,
            changed,
            unassigned,
            proposals,
        })
    })
    .await
}

#[tauri::command]
pub async fn speaker_reidentify_start(
    pid: String,
    root: Option<PathBuf>,
    state: tauri::State<'_, SpeakerAnalysisState>,
) -> AppResult<SpeakerAnalysisJobStatus> {
    let status_path = speaker_analysis_status_path(&pid, root.clone())?;
    let cancel = Arc::new(AtomicBool::new(false));
    let now = unix_timestamp_seconds();
    let status = SpeakerAnalysisJobStatus {
        pid: pid.clone(),
        state: "running".into(),
        phase: "preparing".into(),
        progress: 0,
        current: None,
        total: None,
        device: None,
        elapsed_seconds: None,
        cpu_percent: None,
        peak_memory_mb: None,
        memory_limit_mb: None,
        started_at: Some(now),
        updated_at: Some(now),
        error: None,
        preview: None,
    };
    {
        let mut jobs = state.jobs.lock().expect("speaker analysis state poisoned");
        if jobs
            .get(&pid)
            .is_some_and(|job| matches!(job.status.state.as_str(), "running" | "cancelling"))
        {
            return Err(AppError::Schema(
                "this project already has a speaker analysis in progress".into(),
            ));
        }
        jobs.insert(
            pid.clone(),
            SpeakerAnalysisJob {
                status: status.clone(),
                cancel: cancel.clone(),
                status_path: status_path.clone(),
            },
        );
    }
    let initial_status = status.clone();
    let initial_path = status_path.clone();
    if let Err(error) = run_blocking("save speaker analysis status", move || {
        save_speaker_analysis_status(&initial_path, &initial_status)
    })
    .await
    {
        state
            .jobs
            .lock()
            .expect("speaker analysis state poisoned")
            .remove(&pid);
        return Err(error);
    }
    trace_pipeline_started("speaker-analysis", &pid);

    let jobs = state.jobs.clone();
    let task_pid = pid.clone();
    tauri::async_runtime::spawn(async move {
        let progress_jobs = jobs.clone();
        let progress_pid = task_pid.clone();
        let work = speaker_reidentify_preview_impl(
            task_pid.clone(),
            root,
            Some(Arc::new(move |progress| {
                update_speaker_analysis_job(&progress_jobs, &progress_pid, progress);
            })),
        );
        let result = crate::proc::with_cancellation(cancel, work).await;
        let mut final_status = {
            let guard = jobs.lock().expect("speaker analysis state poisoned");
            let Some(job) = guard.get(&task_pid) else {
                return;
            };
            let mut status = job.status.clone();
            match result {
                Ok(preview) => {
                    status.state = "completed".into();
                    status.phase = "completed".into();
                    status.progress = 100;
                    status.current = None;
                    status.total = None;
                    status.error = None;
                    status.preview = Some(preview);
                }
                Err(AppError::Cancelled) => {
                    status.state = "cancelled".into();
                    status.phase = "cancelled".into();
                    status.error = None;
                }
                Err(error) => {
                    status.state = "failed".into();
                    status.phase = "failed".into();
                    status.error = Some(error.to_string());
                }
            }
            status.updated_at = Some(unix_timestamp_seconds());
            status
        };
        if let Some(job) = jobs
            .lock()
            .expect("speaker analysis state poisoned")
            .get_mut(&task_pid)
        {
            job.status = final_status.clone();
        }
        let persisted = final_status.clone();
        if let Err(error) = persist_background_status(
            "save final speaker analysis status",
            status_path,
            persisted,
            save_speaker_analysis_status,
        )
        .await
        {
            final_status.state = "failed".into();
            final_status.phase = "failed".into();
            final_status.error = Some(format!(
                "speaker analysis finished but its proposal could not be saved: {error}"
            ));
        }
        if let Some(job) = jobs
            .lock()
            .expect("speaker analysis state poisoned")
            .get_mut(&task_pid)
        {
            job.status = final_status.clone();
        }
        trace_pipeline_finished(
            "speaker-analysis",
            &task_pid,
            &final_status.state,
            final_status.error.as_deref(),
        );
    });
    Ok(status)
}

#[tauri::command]
pub async fn speaker_reidentify_status(
    pid: String,
    state: tauri::State<'_, SpeakerAnalysisState>,
) -> AppResult<SpeakerAnalysisJobStatus> {
    let active = state
        .jobs
        .lock()
        .expect("speaker analysis state poisoned")
        .get(&pid)
        .map(|job| (job.status.clone(), job.status_path.clone()));
    let status_path = active
        .as_ref()
        .map(|(_, path)| path.clone())
        .unwrap_or(speaker_analysis_status_path(&pid, None)?);
    let project_dir = if active.is_some() {
        status_path
            .parent()
            .and_then(std::path::Path::parent)
            .map(|root| root.join(&pid))
            .ok_or_else(|| AppError::Schema("invalid speaker analysis status path".into()))?
    } else {
        resolve_project_dir(&pid, None)?
    };
    if let Some((memory_status, _)) = active {
        let checked_path = status_path.clone();
        let checked = run_blocking("checkpoint speaker analysis status", move || {
            let mut status = memory_status;
            if let Some(preview) = status.preview.as_ref() {
                let doc = Doc::load(&project_dir)?;
                if !speaker_preview_matches_doc(&doc, preview) {
                    status.preview = None;
                }
            }
            save_speaker_analysis_status(&checked_path, &status)?;
            Ok(status)
        })
        .await?;
        let latest = state
            .jobs
            .lock()
            .expect("speaker analysis state poisoned")
            .get(&pid)
            .map(|job| job.status.clone())
            .unwrap_or_else(|| checked.clone());
        if latest.state != checked.state
            || latest.phase != checked.phase
            || latest.progress != checked.progress
            || latest.updated_at != checked.updated_at
        {
            persist_background_status(
                "checkpoint latest speaker analysis status",
                status_path,
                latest.clone(),
                save_speaker_analysis_status,
            )
            .await?;
            return Ok(latest);
        }
        if let Some(job) = state
            .jobs
            .lock()
            .expect("speaker analysis state poisoned")
            .get_mut(&pid)
        {
            job.status = checked.clone();
        }
        return Ok(checked);
    }
    let status = run_blocking("load speaker analysis status", move || {
        let mut status =
            load_recovered_speaker_analysis_status(&status_path).map_err(|error| match error {
                AppError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
                    AppError::Schema("no speaker analysis job for this project".into())
                }
                other => other,
            })?;
        if let Some(preview) = status.preview.as_ref() {
            let doc = Doc::load(&project_dir)?;
            if !speaker_preview_matches_doc(&doc, preview) {
                status.preview = None;
            }
        }
        save_speaker_analysis_status(&status_path, &status)?;
        Ok(status)
    })
    .await?;
    Ok(status)
}

#[tauri::command]
pub async fn speaker_reidentify_cancel(
    pid: String,
    state: tauri::State<'_, SpeakerAnalysisState>,
) -> AppResult<SpeakerAnalysisJobStatus> {
    let (status, path) = {
        let mut jobs = state.jobs.lock().expect("speaker analysis state poisoned");
        let job = jobs
            .get_mut(&pid)
            .ok_or_else(|| AppError::Schema("no speaker analysis job for this project".into()))?;
        if job.status.state == "running" {
            job.cancel.store(true, Ordering::Relaxed);
            job.status.state = "cancelling".into();
            job.status.phase = "cancelling".into();
            job.status.updated_at = Some(unix_timestamp_seconds());
        }
        (job.status.clone(), job.status_path.clone())
    };
    persist_background_status(
        "checkpoint speaker analysis cancellation",
        path,
        status.clone(),
        save_speaker_analysis_status,
    )
    .await?;
    Ok(status)
}

#[tauri::command]
pub async fn speaker_reidentify_apply(
    pid: String,
    proposals: Vec<SpeakerReidentifyProposal>,
    root: Option<PathBuf>,
) -> AppResult<usize> {
    if proposals.is_empty() {
        return Err(AppError::Schema("speaker proposal is empty".into()));
    }
    let mut paragraph_ids = HashSet::new();
    for proposal in &proposals {
        if proposal.proposed.trim().is_empty()
            || !proposal.start.is_finite()
            || !proposal.end.is_finite()
            || proposal.end <= proposal.start
            || !crate::diarize::reliable_speaker_match(proposal.coverage, proposal.margin)
            || !paragraph_ids.insert(proposal.paragraph_id)
        {
            return Err(AppError::Schema("speaker proposal is invalid".into()));
        }
    }
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("speaker proposal apply", move || {
        crate::data::edit_history::record(
            &dir,
            "Apply speaker analysis",
            || {
                let original = Doc::load(&dir)?;
                let mut doc = original.clone();
                crate::diarize::normalize_speaker_paragraphs(&mut doc);
                for proposal in &proposals {
                    let paragraph = doc
                        .paragraphs
                        .iter()
                        .find(|paragraph| paragraph.id == proposal.paragraph_id)
                        .ok_or_else(|| {
                            AppError::Schema(format!(
                                "speaker proposal is stale: paragraph {} is missing",
                                proposal.paragraph_id
                            ))
                        })?;
                    let bounds = paragraph_bounds(paragraph).ok_or_else(|| {
                        AppError::Schema(format!(
                            "speaker proposal is stale: paragraph {} has no timed words",
                            proposal.paragraph_id
                        ))
                    })?;
                    let current_text = paragraph
                        .sentences
                        .iter()
                        .map(|sentence| sentence.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    if paragraph.speaker != proposal.current
                        || (bounds.0 - proposal.start).abs() > 0.001
                        || (bounds.1 - proposal.end).abs() > 0.001
                        || current_text != proposal.text
                    {
                        return Err(AppError::Schema(
                            "speaker proposal is stale; run identification again".into(),
                        ));
                    }
                }
                if !working_head_is_committed(&dir, &original)? {
                    let mut lineage = crate::data::version::Lineage::load(&dir)?;
                    let branch = lineage
                        .active_branch
                        .clone()
                        .unwrap_or_else(|| "main".into());
                    crate::data::version::commit_snapshot(
                        &dir,
                        &original,
                        &mut lineage,
                        &branch,
                        "Before speaker re-identification",
                        "automatic recovery snapshot",
                        crate::data::version::VersionKind::Auto,
                    )?;
                }
                let mut changed = 0;
                for proposal in proposals {
                    let paragraph = doc
                        .paragraphs
                        .iter_mut()
                        .find(|paragraph| paragraph.id == proposal.paragraph_id)
                        .expect("speaker proposals were validated");
                    if paragraph.speaker.as_deref() != Some(proposal.proposed.as_str()) {
                        paragraph.speaker = Some(proposal.proposed);
                        changed += 1;
                    }
                }
                if changed > 0 {
                    doc.meta.updated_at = chrono::Utc::now();
                    doc.save(&dir)?;
                }
                Ok(changed)
            },
            |changed| *changed > 0,
        )
    })
    .await
}

#[derive(Debug, Serialize)]
pub struct BrollOverview {
    pub suggestions: Vec<crate::pipeline::broll::BrollSuggestion>,
    pub accepted: Vec<crate::data::broll::BrollPlacement>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrollPreviewJobStatus {
    pub pid: String,
    pub state: String,
    pub phase: String,
    pub progress: u8,
    pub current: Option<f64>,
    pub total: Option<f64>,
    pub encoder: Option<String>,
    #[serde(default)]
    pub started_at: Option<u64>,
    #[serde(default)]
    pub updated_at: Option<u64>,
    pub error: Option<String>,
    pub paths: Vec<String>,
}

struct BrollPreviewJob {
    status: BrollPreviewJobStatus,
    cancel: Arc<AtomicBool>,
    status_path: PathBuf,
}

#[derive(Clone, Default)]
pub struct BrollPreviewState {
    current: Arc<Mutex<Vec<PathBuf>>>,
    jobs: Arc<Mutex<HashMap<String, BrollPreviewJob>>>,
}

#[derive(Clone)]
struct BrollPreviewProgress {
    phase: String,
    progress: u8,
    current: Option<f64>,
    total: Option<f64>,
    encoder: Option<String>,
}

type BrollPreviewProgressCallback = Arc<dyn Fn(BrollPreviewProgress) + Send + Sync>;

fn broll_preview_status_path(pid: &str, root: Option<PathBuf>) -> AppResult<PathBuf> {
    let _ = resolve_project_dir(pid, root.clone())?;
    Ok(resolve_project_root(root)
        .join(".jobs")
        .join(format!("{pid}.broll-preview.json")))
}

fn save_broll_preview_status(
    path: &std::path::Path,
    status: &BrollPreviewJobStatus,
) -> AppResult<()> {
    crate::data::storage::write_json(path, status)
}

fn load_broll_preview_status(path: &std::path::Path) -> AppResult<BrollPreviewJobStatus> {
    Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
}

fn load_recovered_broll_preview_status(path: &std::path::Path) -> AppResult<BrollPreviewJobStatus> {
    let mut status = load_broll_preview_status(path)?;
    if matches!(status.state.as_str(), "running" | "cancelling") {
        status.state = "failed".into();
        status.phase = "failed".into();
        status.error = Some(
            "the previous B-roll preview was interrupted when lumen-cut closed; start it again"
                .into(),
        );
        status.updated_at = Some(unix_timestamp_seconds());
        save_broll_preview_status(path, &status)?;
    }
    Ok(status)
}

fn validated_broll_preview_paths(
    project_dir: PathBuf,
    candidates: Vec<String>,
) -> AppResult<Vec<PathBuf>> {
    let project_dir = std::fs::canonicalize(project_dir)?;
    Ok(candidates
        .into_iter()
        .filter_map(|candidate| {
            let path = std::fs::canonicalize(candidate).ok()?;
            let name = path.file_name()?.to_string_lossy();
            (path.is_file()
                && path.starts_with(&project_dir)
                && name.starts_with("broll-preview-")
                && path.extension().and_then(|value| value.to_str()) == Some("png"))
            .then_some(path)
        })
        .collect())
}

async fn restore_broll_preview_assets(
    pid: &str,
    mut status: BrollPreviewJobStatus,
    app: &tauri::AppHandle,
    state: &BrollPreviewState,
) -> AppResult<BrollPreviewJobStatus> {
    if status.state != "completed" || status.paths.is_empty() {
        return Ok(status);
    }
    let project_dir = resolve_project_dir(pid, None)?;
    let candidates = status.paths.clone();
    let valid = run_blocking("validate recovered B-roll preview assets", move || {
        validated_broll_preview_paths(project_dir, candidates)
    })
    .await?;
    let scope = app.asset_protocol_scope();
    let mut current = state.current.lock().expect("B-roll preview state poisoned");
    for previous in current.iter().filter(|path| !valid.contains(path)) {
        scope
            .forbid_file(previous)
            .map_err(|error| AppError::Schema(format!("B-roll preview scope: {error}")))?;
    }
    for path in &valid {
        scope
            .allow_file(path)
            .map_err(|error| AppError::Schema(format!("B-roll preview scope: {error}")))?;
    }
    *current = valid.clone();
    status.paths = valid
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    Ok(status)
}

fn update_broll_preview_job(
    jobs: &Mutex<HashMap<String, BrollPreviewJob>>,
    pid: &str,
    progress: BrollPreviewProgress,
) {
    if let Some(job) = jobs
        .lock()
        .expect("B-roll preview state poisoned")
        .get_mut(pid)
    {
        if job.status.phase != progress.phase {
            tracing::info!(
                pipeline = "broll-preview",
                pid,
                phase = progress.phase,
                "pipeline phase changed"
            );
        }
        job.status.phase = progress.phase;
        job.status.progress = advance_progress(job.status.progress, progress.progress);
        job.status.current = progress.current;
        job.status.total = progress.total;
        if progress.encoder.is_some() {
            job.status.encoder = progress.encoder;
        }
        job.status.updated_at = Some(unix_timestamp_seconds());
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrollPlacementInput {
    pub file: PathBuf,
    pub start: f64,
    pub end: f64,
    pub mode: Option<crate::data::broll::PlacementMode>,
    pub fit: Option<crate::data::broll::FitMode>,
    pub background: Option<crate::data::broll::BackgroundMode>,
    pub rect: Option<crate::data::broll::Rect>,
    pub source_start: Option<f64>,
    pub radius: Option<u32>,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleClipInput {
    pub text: String,
    pub start: f64,
    pub end: f64,
    pub x: f64,
    pub y: f64,
    pub font_size: u32,
    pub color: String,
    pub background: String,
    #[serde(default)]
    pub fade_in: f64,
    #[serde(default)]
    pub fade_out: f64,
}

fn title_from_input(
    input: TitleClipInput,
    id: String,
    duration: f64,
) -> AppResult<crate::data::title::TitleClip> {
    let title = crate::data::title::TitleClip {
        id,
        text: input.text.trim().to_owned(),
        start: input.start,
        end: input.end,
        x: input.x,
        y: input.y,
        font_size: input.font_size,
        color: input.color.to_ascii_uppercase(),
        background: input.background.to_ascii_uppercase(),
        fade_in: input.fade_in,
        fade_out: input.fade_out,
    };
    title.validate()?;
    if duration > 0.0 && title.end > duration {
        return Err(AppError::Schema(format!(
            "title end {:.2}s exceeds media duration {duration:.2}s",
            title.end
        )));
    }
    Ok(title)
}

#[tauri::command]
pub async fn title_list(
    pid: String,
    root: Option<PathBuf>,
) -> AppResult<Vec<crate::data::title::TitleClip>> {
    let dir = resolve_project_dir(&pid, root)?;
    run_blocking("title list", move || {
        Doc::load(&dir)?;
        crate::data::title::load(&dir)
    })
    .await
}

#[tauri::command]
pub async fn title_add(
    pid: String,
    input: TitleClipInput,
    root: Option<PathBuf>,
) -> AppResult<crate::data::title::TitleClip> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("title add", move || {
        crate::data::edit_history::record(
            &dir,
            "Add title",
            || {
                let doc = Doc::load(&dir)?;
                let mut titles = crate::data::title::load(&dir)?;
                let title = title_from_input(
                    input,
                    format!("title-{}", uuid::Uuid::new_v4().simple()),
                    doc.media.duration_seconds,
                )?;
                titles.push(title.clone());
                crate::data::title::save(&dir, &titles)?;
                Ok(title)
            },
            |_| true,
        )
    })
    .await
}

#[tauri::command]
pub async fn title_update(
    pid: String,
    id: String,
    input: TitleClipInput,
    root: Option<PathBuf>,
) -> AppResult<crate::data::title::TitleClip> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("title update", move || {
        crate::data::edit_history::record(
            &dir,
            "Adjust title",
            || {
                let doc = Doc::load(&dir)?;
                let mut titles = crate::data::title::load(&dir)?;
                let index = titles
                    .iter()
                    .position(|title| title.id == id)
                    .ok_or_else(|| AppError::Schema(format!("title id {id} not found")))?;
                let replacement = title_from_input(input, id, doc.media.duration_seconds)?;
                let changed = titles[index] != replacement;
                titles[index] = replacement.clone();
                if changed {
                    crate::data::title::save(&dir, &titles)?;
                }
                Ok((replacement, changed))
            },
            |(_, changed)| *changed,
        )
        .map(|(title, _)| title)
    })
    .await
}

#[tauri::command]
pub async fn title_remove(pid: String, id: String, root: Option<PathBuf>) -> AppResult<bool> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("title remove", move || {
        crate::data::edit_history::record(
            &dir,
            "Remove title",
            || {
                let mut titles = crate::data::title::load(&dir)?;
                let before = titles.len();
                titles.retain(|title| title.id != id);
                if titles.len() == before {
                    return Ok(false);
                }
                crate::data::title::save(&dir, &titles)?;
                Ok(true)
            },
            |changed| *changed,
        )
    })
    .await
}

fn load_project_cuts(dir: &std::path::Path) -> AppResult<ClipCuts> {
    let cuts_path = dir.join("cuts.json");
    if cuts_path.exists() {
        Ok(serde_json::from_str(&std::fs::read_to_string(cuts_path)?)?)
    } else {
        Ok(ClipCuts::new())
    }
}

fn project_edited_duration(dir: &std::path::Path, doc: &Doc) -> AppResult<f64> {
    let cuts = load_project_cuts(dir)?;
    Ok(crate::export::project::kept_intervals(doc, &cuts.cuts)
        .iter()
        .map(|(start, end)| end - start)
        .sum())
}

#[tauri::command]
pub async fn audio_mix_get(
    pid: String,
    root: Option<PathBuf>,
) -> AppResult<crate::data::audio_mix::AudioMix> {
    let dir = resolve_project_dir(&pid, root)?;
    run_blocking("audio mix load", move || {
        let doc = Doc::load(&dir)?;
        crate::data::audio_mix::load(&dir)?.fit_to_duration(project_edited_duration(&dir, &doc)?)
    })
    .await
}

#[tauri::command]
pub async fn audio_mix_set(
    pid: String,
    mix: crate::data::audio_mix::AudioMix,
    root: Option<PathBuf>,
) -> AppResult<crate::data::audio_mix::AudioMix> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("audio mix update", move || {
        crate::data::edit_history::record(
            &dir,
            "Adjust audio mix",
            || {
                let doc = Doc::load(&dir)?;
                let duration = project_edited_duration(&dir, &doc)?;
                let mut mix = mix;
                for track in &mut mix.music {
                    track.path = std::fs::canonicalize(&track.path)?;
                    if !track.path.is_file() {
                        return Err(AppError::ProjectNotFound(track.path.clone()));
                    }
                }
                mix.validate(duration)?;
                let changed = crate::data::audio_mix::load(&dir)? != mix;
                if changed {
                    crate::data::audio_mix::save(&dir, &mix, duration)?;
                }
                Ok((mix, changed))
            },
            |(_, changed)| *changed,
        )
        .map(|(mix, _)| mix)
    })
    .await
}

#[tauri::command]
pub async fn export_settings_get(
    pid: String,
    root: Option<PathBuf>,
) -> AppResult<crate::data::export_settings::VideoExportSettings> {
    let dir = resolve_project_dir(&pid, root)?;
    run_blocking("export settings load", move || {
        Doc::load(&dir)?;
        crate::data::export_settings::load(&dir)
    })
    .await
}

#[tauri::command]
pub async fn export_settings_set(
    pid: String,
    settings: crate::data::export_settings::VideoExportSettings,
    root: Option<PathBuf>,
) -> AppResult<crate::data::export_settings::VideoExportSettings> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("export settings update", move || {
        Doc::load(&dir)?;
        crate::data::export_settings::save(&dir, &settings)?;
        Ok(settings)
    })
    .await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPreflightItem {
    pub code: String,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPreflightSummary {
    pub duration_seconds: f64,
    pub visible_captions: usize,
    pub hidden_captions: usize,
    pub broll_items: usize,
    pub title_items: usize,
    pub encoder: String,
    pub estimated_min_mb: u64,
    pub estimated_max_mb: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPreflightReport {
    pub ready: bool,
    pub items: Vec<ExportPreflightItem>,
    pub summary: ExportPreflightSummary,
}

fn push_export_preflight_item(
    items: &mut Vec<ExportPreflightItem>,
    code: &str,
    level: &str,
    message: String,
) {
    items.push(ExportPreflightItem {
        code: code.into(),
        level: level.into(),
        message,
    });
}

fn export_size_estimate_mb(
    settings: &crate::data::export_settings::VideoExportSettings,
    duration_seconds: f64,
    dimensions: Option<(u32, u32)>,
    has_audio: bool,
) -> (u64, u64) {
    use crate::data::export_settings::{ExportAudioCodec, ExportVideoCodec};
    let pixel_scale = dimensions
        .map(|(width, height)| {
            (f64::from(width) * f64::from(height) / (1920.0 * 1080.0)).clamp(0.25, 4.0)
        })
        .unwrap_or(1.0);
    let video_mbps = match settings.video_codec {
        ExportVideoCodec::H264 => 8.0 * pixel_scale,
        ExportVideoCodec::Hevc => 5.0 * pixel_scale,
        ExportVideoCodec::Prores => 220.0 * pixel_scale,
    };
    let audio_mbps = if !has_audio {
        0.0
    } else {
        match settings.audio_codec {
            ExportAudioCodec::Aac => 0.192,
            ExportAudioCodec::Pcm => 1.536,
        }
    };
    let nominal_mb = duration_seconds.max(0.0) * (video_mbps + audio_mbps) / 8.0;
    (
        (nominal_mb * 0.65).ceil().max(1.0) as u64,
        (nominal_mb * 1.5).ceil().max(1.0) as u64,
    )
}

async fn export_preflight_impl(
    pid: &str,
    settings: crate::data::export_settings::VideoExportSettings,
    root: Option<PathBuf>,
) -> AppResult<ExportPreflightReport> {
    let dir = resolve_project_dir(pid, root)?;
    let snapshot_dir = dir.clone();
    let (
        doc,
        cuts_result,
        broll_result,
        hidden_result,
        titles_result,
        audio_mix_result,
        style_result,
    ) = run_blocking("export preflight snapshot", move || {
        let doc = Doc::load(&snapshot_dir)?;
        let cuts = load_project_cuts(&snapshot_dir);
        let broll = crate::data::broll::load(&snapshot_dir);
        let hidden = crate::data::subtitle::load_hidden_checked(&snapshot_dir);
        let titles = crate::data::title::load(&snapshot_dir);
        let audio_mix = crate::data::audio_mix::load(&snapshot_dir);
        let style = crate::data::substyle::SubStyle::load(&snapshot_dir);
        Ok((doc, cuts, broll, hidden, titles, audio_mix, style))
    })
    .await?;

    let mut items = Vec::new();
    let cuts = match cuts_result {
        Ok(cuts) => cuts,
        Err(error) => {
            push_export_preflight_item(
                &mut items,
                "timeline-data",
                "blocker",
                format!("timeline edits cannot be read: {error}"),
            );
            ClipCuts::new()
        }
    };
    let broll = match broll_result {
        Ok(broll) => broll,
        Err(error) => {
            push_export_preflight_item(
                &mut items,
                "broll",
                "blocker",
                format!("B-roll placements cannot be read: {error}"),
            );
            Vec::new()
        }
    };
    let titles = match titles_result {
        Ok(titles) => titles,
        Err(error) => {
            push_export_preflight_item(
                &mut items,
                "titles",
                "blocker",
                format!("titles cannot be read: {error}"),
            );
            Vec::new()
        }
    };
    let hidden = match hidden_result {
        Ok(hidden) => hidden,
        Err(error) => {
            push_export_preflight_item(
                &mut items,
                "caption-state",
                "blocker",
                format!("caption visibility cannot be read: {error}"),
            );
            Default::default()
        }
    };
    if let Err(error) = style_result {
        push_export_preflight_item(
            &mut items,
            "style",
            "blocker",
            format!("subtitle style cannot be read: {error}"),
        );
    }
    if let Err(error) = settings.validate() {
        push_export_preflight_item(&mut items, "settings", "blocker", error.to_string());
    } else {
        items.push(ExportPreflightItem {
            code: "settings".into(),
            level: "pass".into(),
            message: format!(
                "{} / {:?} settings are compatible",
                settings.extension().to_uppercase(),
                settings.video_codec
            ),
        });
    }

    let media_info = if !doc.media.path.is_file() {
        push_export_preflight_item(
            &mut items,
            "media",
            "blocker",
            format!("source media is missing: {}", doc.media.path.display()),
        );
        None
    } else {
        match crate::media::probe(&doc.media.path).await {
            Ok(info)
                if info.duration_seconds > 0.001
                    && info.width.is_some_and(|width| width > 0)
                    && info.height.is_some_and(|height| height > 0) =>
            {
                items.push(ExportPreflightItem {
                    code: "media".into(),
                    level: "pass".into(),
                    message: format!(
                        "source media is readable ({}×{}, {:.1}s)",
                        info.width.unwrap_or_default(),
                        info.height.unwrap_or_default(),
                        info.duration_seconds
                    ),
                });
                if (info.duration_seconds - doc.media.duration_seconds).abs()
                    > (info.duration_seconds * 0.01).max(0.25)
                {
                    items.push(ExportPreflightItem {
                        code: "media-duration".into(),
                        level: "warning".into(),
                        message: format!(
                            "project duration {:.1}s differs from source duration {:.1}s",
                            doc.media.duration_seconds, info.duration_seconds
                        ),
                    });
                }
                Some(info)
            }
            Ok(_) => {
                push_export_preflight_item(
                    &mut items,
                    "media",
                    "blocker",
                    "source media has no decodable video stream or duration".into(),
                );
                None
            }
            Err(error) => {
                push_export_preflight_item(
                    &mut items,
                    "media",
                    "blocker",
                    format!("source media cannot be decoded: {error}"),
                );
                None
            }
        }
    };

    let duration_seconds: f64 = crate::export::project::kept_intervals(&doc, &cuts.cuts)
        .iter()
        .map(|(start, end)| end - start)
        .sum();
    if duration_seconds <= 0.001 {
        push_export_preflight_item(
            &mut items,
            "timeline",
            "blocker",
            "the current cuts remove the entire media timeline".into(),
        );
    } else {
        items.push(ExportPreflightItem {
            code: "timeline".into(),
            level: "pass".into(),
            message: format!("edited duration is {duration_seconds:.1}s"),
        });
    }

    if !items
        .iter()
        .any(|item| item.code == "titles" && item.level == "blocker")
    {
        let invalid_titles = titles
            .iter()
            .filter(|title| title.end > doc.media.duration_seconds + 0.05)
            .map(|title| {
                format!(
                    "{}: title ends after the source timeline ({:.1}s > {:.1}s)",
                    title.id, title.end, doc.media.duration_seconds
                )
            })
            .collect::<Vec<_>>();
        if invalid_titles.is_empty() {
            items.push(ExportPreflightItem {
                code: "titles".into(),
                level: "pass".into(),
                message: format!("{} title item(s) are ready", titles.len()),
            });
        } else {
            push_export_preflight_item(&mut items, "titles", "blocker", invalid_titles.join("; "));
        }
    }

    let has_background_music = audio_mix_result
        .as_ref()
        .ok()
        .is_some_and(|mix| !mix.music.is_empty());
    match audio_mix_result.and_then(|mix| mix.fit_to_duration(duration_seconds)) {
        Ok(mix) => {
            let mut music_ready = 0usize;
            for track in &mix.music {
                if !track.path.is_file() {
                    push_export_preflight_item(
                        &mut items,
                        "audio",
                        "blocker",
                        format!(
                            "background music {} is missing: {}",
                            track.id,
                            track.path.display()
                        ),
                    );
                } else {
                    match crate::media::probe(&track.path).await {
                        Ok(info)
                            if info.duration_seconds > 0.001
                                && info.channels.is_some_and(|channels| channels > 0) =>
                        {
                            music_ready += 1;
                        }
                        Ok(_) => push_export_preflight_item(
                            &mut items,
                            "audio",
                            "blocker",
                            format!(
                                "background music {} has no decodable audio stream",
                                track.id
                            ),
                        ),
                        Err(error) => push_export_preflight_item(
                            &mut items,
                            "audio",
                            "blocker",
                            format!("background music {} cannot be decoded: {error}", track.id),
                        ),
                    }
                }
            }
            if music_ready == mix.music.len() && music_ready > 0 {
                items.push(ExportPreflightItem {
                    code: "audio".into(),
                    level: "pass".into(),
                    message: format!(
                        "audio mix and {music_ready} background music clip(s) are ready"
                    ),
                });
            } else if mix.music.is_empty() {
                items.push(ExportPreflightItem {
                    code: "audio".into(),
                    level: "pass".into(),
                    message: if mix.muted {
                        "program audio will be muted".into()
                    } else {
                        format!(
                            "audio mix is ready ({}%, {:.1}s in / {:.1}s out)",
                            (mix.volume * 100.0).round(),
                            mix.fade_in,
                            mix.fade_out
                        )
                    },
                });
            }
        }
        Err(error) => push_export_preflight_item(
            &mut items,
            "audio",
            "blocker",
            format!("audio mix cannot be used: {error}"),
        ),
    }

    let visible_captions = doc
        .paragraphs
        .iter()
        .flat_map(|paragraph| paragraph.sentences.iter())
        .filter(|sentence| !sentence.words.is_empty() && !hidden.contains(&sentence.id))
        .count();
    let hidden_captions = doc
        .paragraphs
        .iter()
        .flat_map(|paragraph| paragraph.sentences.iter())
        .filter(|sentence| !sentence.words.is_empty() && hidden.contains(&sentence.id))
        .count();
    if settings.subtitle_mode == crate::data::export_settings::ExportSubtitleMode::None {
        items.push(ExportPreflightItem {
            code: "captions".into(),
            level: "pass".into(),
            message: "video export is configured without captions".into(),
        });
    } else {
        match crate::data::export_settings::project_caption_doc_with_hidden(
            &doc,
            settings.subtitle_language.as_deref(),
            settings.bilingual_subtitles,
            &hidden,
        ) {
            Ok(_) => items.push(ExportPreflightItem {
                code: "captions".into(),
                level: "pass".into(),
                message: format!("{visible_captions} visible caption line(s) are ready"),
            }),
            Err(error) => {
                push_export_preflight_item(&mut items, "captions", "blocker", error.to_string())
            }
        }
    }
    if hidden_captions > 0 {
        items.push(ExportPreflightItem {
            code: "hidden-captions".into(),
            level: "warning".into(),
            message: format!("{hidden_captions} hidden caption line(s) will not be exported"),
        });
    }

    let mut invalid_broll = Vec::new();
    for placement in &broll {
        if let Err(error) = placement.validate() {
            invalid_broll.push(format!("{}: {error}", placement.id));
        } else if placement.end > doc.media.duration_seconds + 0.05 {
            invalid_broll.push(format!(
                "{}: placement ends after the source timeline ({:.1}s > {:.1}s)",
                placement.id, placement.end, doc.media.duration_seconds
            ));
        } else if !placement.file.is_file() {
            invalid_broll.push(format!(
                "{}: asset is missing ({})",
                placement.id,
                placement.file.display()
            ));
        } else {
            match crate::media::probe(&placement.file).await {
                Ok(info)
                    if info.width.is_some_and(|width| width > 0)
                        && info.height.is_some_and(|height| height > 0) =>
                {
                    if !is_still_image(&placement.file)
                        && info.duration_seconds + 0.05
                            < placement.source_start + (placement.end - placement.start)
                    {
                        invalid_broll.push(format!(
                            "{}: source range ends at {:.1}s but the asset is only {:.1}s",
                            placement.id,
                            placement.source_start + (placement.end - placement.start),
                            info.duration_seconds
                        ));
                    }
                }
                Ok(_) => invalid_broll.push(format!(
                    "{}: asset has no decodable visual stream",
                    placement.id
                )),
                Err(error) => invalid_broll.push(format!(
                    "{}: asset cannot be decoded ({error})",
                    placement.id
                )),
            }
        }
    }
    for (index, placement) in broll.iter().enumerate() {
        if broll[index + 1..]
            .iter()
            .any(|other| placement.start < other.end && other.start < placement.end)
        {
            invalid_broll.push(format!(
                "{}: placement overlaps another B-roll item",
                placement.id
            ));
        }
    }
    if !items
        .iter()
        .any(|item| item.code == "broll" && item.level == "blocker")
    {
        if invalid_broll.is_empty() {
            items.push(ExportPreflightItem {
                code: "broll".into(),
                level: "pass".into(),
                message: format!("{} B-roll item(s) are available", broll.len()),
            });
        } else {
            push_export_preflight_item(&mut items, "broll", "blocker", invalid_broll.join("; "));
        }
    }

    let encoder = match crate::export::video::encoder_for_settings(&settings) {
        Ok(encoder) => {
            match crate::proc::run(
                "ffmpeg",
                &["-hide_banner", "-loglevel", "error", "-encoders"],
            )
            .await
            {
                Ok(encoders) if encoders.contains(&encoder) => items.push(ExportPreflightItem {
                    code: "encoder".into(),
                    level: "pass".into(),
                    message: format!("{encoder} is available"),
                }),
                Ok(_) => push_export_preflight_item(
                    &mut items,
                    "encoder",
                    "blocker",
                    format!("FFmpeg does not provide the selected encoder `{encoder}`"),
                ),
                Err(error) => push_export_preflight_item(
                    &mut items,
                    "encoder",
                    "blocker",
                    format!("FFmpeg is unavailable: {error}"),
                ),
            }
            encoder
        }
        Err(_) => "unavailable".into(),
    };

    let source_dimensions = media_info
        .as_ref()
        .and_then(|info| info.width.zip(info.height));
    let dimensions = settings
        .target_dimensions(source_dimensions)
        .or(source_dimensions);
    let (estimated_min_mb, estimated_max_mb) = export_size_estimate_mb(
        &settings,
        duration_seconds,
        dimensions,
        media_info
            .as_ref()
            .and_then(|info| info.channels)
            .or(doc.media.channels)
            .is_some_and(|channels| channels > 0)
            || has_background_music,
    );
    items.push(ExportPreflightItem {
        code: "size-estimate".into(),
        level: "warning".into(),
        message: format!(
            "estimated output size is {estimated_min_mb}–{estimated_max_mb} MB; actual size depends on content"
        ),
    });
    let ready = !items.iter().any(|item| item.level == "blocker");
    Ok(ExportPreflightReport {
        ready,
        items,
        summary: ExportPreflightSummary {
            duration_seconds,
            visible_captions,
            hidden_captions,
            broll_items: broll.len(),
            title_items: titles.len(),
            encoder,
            estimated_min_mb,
            estimated_max_mb,
        },
    })
}

#[tauri::command]
pub async fn export_preflight(
    pid: String,
    settings: crate::data::export_settings::VideoExportSettings,
    root: Option<PathBuf>,
) -> AppResult<ExportPreflightReport> {
    export_preflight_impl(&pid, settings, root).await
}

static PROJECT_MUTATIONS: OnceLock<
    tokio::sync::Mutex<HashMap<PathBuf, Weak<tokio::sync::Mutex<()>>>>,
> = OnceLock::new();

async fn project_mutation_mutex(dir: &std::path::Path) -> Arc<tokio::sync::Mutex<()>> {
    let registry = PROJECT_MUTATIONS.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()));
    {
        let mut locks = registry.lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(dir).and_then(Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(tokio::sync::Mutex::new(()));
            locks.insert(dir.to_path_buf(), Arc::downgrade(&lock));
            lock
        }
    }
}

async fn lock_project_mutation(dir: &std::path::Path) -> tokio::sync::OwnedMutexGuard<()> {
    project_mutation_mutex(dir).await.lock_owned().await
}

const fn default_pip_rect() -> crate::data::broll::Rect {
    crate::data::broll::Rect {
        x: 1229,
        y: 65,
        width: 614,
        height: 346,
    }
}

fn is_still_image(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "gif"
            )
        })
}

fn validate_broll_span(
    doc: &Doc,
    placements: &[crate::data::broll::BrollPlacement],
    start: f64,
    end: f64,
    ignore_id: Option<&str>,
) -> AppResult<()> {
    if doc.media.duration_seconds > 0.0 && end > doc.media.duration_seconds {
        return Err(AppError::Schema(format!(
            "B-roll end {end:.2}s exceeds media duration {:.2}s",
            doc.media.duration_seconds
        )));
    }
    if placements.iter().any(|placement| {
        ignore_id != Some(placement.id.as_str()) && start < placement.end && placement.start < end
    }) {
        return Err(AppError::Schema(
            "B-roll placement overlaps an existing placement".into(),
        ));
    }
    Ok(())
}

fn broll_placement_from_input(
    input: BrollPlacementInput,
    id: String,
) -> AppResult<crate::data::broll::BrollPlacement> {
    let file = std::fs::canonicalize(input.file)?;
    if !file.is_file() {
        return Err(AppError::ProjectNotFound(file));
    }
    let image = is_still_image(&file);
    let mode = input.mode.unwrap_or(if image {
        crate::data::broll::PlacementMode::Pip
    } else {
        crate::data::broll::PlacementMode::Fullscreen
    });
    let placement = crate::data::broll::BrollPlacement {
        id,
        file,
        start: input.start,
        end: input.end,
        mode,
        rect: (mode == crate::data::broll::PlacementMode::Pip)
            .then(|| input.rect.unwrap_or_else(default_pip_rect)),
        fit: input.fit.unwrap_or_default(),
        background: input.background.unwrap_or_default(),
        source_start: input.source_start.unwrap_or_default(),
        radius: input.radius.unwrap_or_default(),
        name: input.name.and_then(|name| {
            let name = name.trim().to_string();
            (!name.is_empty()).then_some(name)
        }),
    };
    placement.validate()?;
    Ok(placement)
}

#[tauri::command]
pub async fn broll_list(pid: String, root: Option<PathBuf>) -> AppResult<BrollOverview> {
    let dir = resolve_project_dir(&pid, root)?;
    run_blocking("B-roll list", move || {
        Doc::load(&dir)?;
        let mut errors = Vec::new();
        let suggestions = crate::pipeline::broll::load_artifact(&dir).unwrap_or_else(|error| {
            errors.push(format!("suggestions: {error}"));
            Vec::new()
        });
        let accepted = crate::data::broll::load(&dir).unwrap_or_else(|error| {
            errors.push(format!("placements: {error}"));
            Vec::new()
        });
        for placement in &accepted {
            if !placement.file.is_file() {
                errors.push(format!(
                    "placement {}: media is missing or was moved ({})",
                    placement.id,
                    placement.file.display()
                ));
            }
        }
        Ok(BrollOverview {
            suggestions,
            accepted,
            errors,
        })
    })
    .await
}

#[tauri::command]
pub async fn broll_add(
    pid: String,
    input: BrollPlacementInput,
    root: Option<PathBuf>,
) -> AppResult<crate::data::broll::BrollPlacement> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("B-roll add", move || {
        crate::data::edit_history::record(
            &dir,
            "Add B-roll",
            || {
                let doc = Doc::load(&dir)?;
                let mut placements = crate::data::broll::load(&dir)?;
                let placement = broll_placement_from_input(
                    input,
                    format!("br-{}", uuid::Uuid::new_v4().simple()),
                )?;
                validate_broll_span(&doc, &placements, placement.start, placement.end, None)?;
                placements.push(placement.clone());
                crate::data::broll::save(&dir, &placements)?;
                Ok(placement)
            },
            |_| true,
        )
    })
    .await
}

#[tauri::command]
pub async fn broll_accept_suggestion(
    pid: String,
    suggestion: crate::pipeline::broll::BrollSuggestion,
    file: PathBuf,
    root: Option<PathBuf>,
) -> AppResult<crate::data::broll::BrollPlacement> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("B-roll suggestion accept", move || {
        crate::data::edit_history::record(
            &dir,
            "Add suggested B-roll",
            || {
                let doc = Doc::load(&dir)?;
                let suggestions = crate::pipeline::broll::load_artifact(&dir)?;
                let suggestion = suggestions
                    .iter()
                    .find(|candidate| **candidate == suggestion)
                    .ok_or_else(|| {
                        AppError::Schema(
                            "B-roll suggestions changed while choosing an asset; refresh and try again"
                                .into(),
                        )
                    })?;
                let placements = crate::data::broll::load(&dir)?;
                let existing = placements
                    .iter()
                    .map(|placement| (placement.start, placement.end))
                    .collect::<Vec<_>>();
                let problems =
                    crate::pipeline::broll::lint(&doc, std::slice::from_ref(suggestion), &existing);
                if !problems.is_empty() {
                    return Err(AppError::Schema(
                        problems
                            .iter()
                            .map(|problem| format!("{}: {}", problem.loc, problem.problem))
                            .collect::<Vec<_>>()
                            .join("; "),
                    ));
                }
                let words = doc
                    .all_words()
                    .into_iter()
                    .map(|word| (word.id.as_str(), (word.start, word.end)))
                    .collect::<HashMap<_, _>>();
                let start = words
                    .get(suggestion.start.as_str())
                    .map(|times| times.0)
                    .ok_or_else(|| {
                        AppError::Schema("B-roll suggestion start word is missing".into())
                    })?;
                let end = words
                    .get(suggestion.end.as_str())
                    .map(|times| times.1)
                    .ok_or_else(|| {
                        AppError::Schema("B-roll suggestion end word is missing".into())
                    })?;
                let mode = match suggestion.mode {
                    crate::pipeline::broll::BrollMode::Fullscreen => {
                        crate::data::broll::PlacementMode::Fullscreen
                    }
                    crate::pipeline::broll::BrollMode::Pip => {
                        crate::data::broll::PlacementMode::Pip
                    }
                };
                let input = BrollPlacementInput {
                    file,
                    start,
                    end,
                    mode: Some(mode),
                    fit: None,
                    background: None,
                    rect: None,
                    source_start: None,
                    radius: None,
                    name: Some(suggestion.query.clone()),
                };
                let placement = broll_placement_from_input(
                    input,
                    format!("br-{}", uuid::Uuid::new_v4().simple()),
                )?;
                let mut placements = placements;
                placements.push(placement.clone());
                crate::data::broll::save(&dir, &placements)?;
                Ok(placement)
            },
            |_| true,
        )
    })
    .await
}

#[tauri::command]
pub async fn broll_update(
    pid: String,
    id: String,
    input: BrollPlacementInput,
    root: Option<PathBuf>,
) -> AppResult<crate::data::broll::BrollPlacement> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("B-roll update", move || {
        crate::data::edit_history::record(
            &dir,
            "Adjust B-roll",
            || {
                let doc = Doc::load(&dir)?;
                let mut placements = crate::data::broll::load(&dir)?;
                if !placements.iter().any(|placement| placement.id == id) {
                    return Err(AppError::Schema(format!("B-roll id {id} not found")));
                }
                let requested_rect = input.rect;
                let mut replacement = broll_placement_from_input(input, id.clone())?;
                validate_broll_span(
                    &doc,
                    &placements,
                    replacement.start,
                    replacement.end,
                    Some(&id),
                )?;
                let index = placements
                    .iter()
                    .position(|placement| placement.id == id)
                    .ok_or_else(|| AppError::Schema(format!("B-roll id {id} disappeared")))?;
                replacement.rect = match replacement.mode {
                    crate::data::broll::PlacementMode::Fullscreen => None,
                    crate::data::broll::PlacementMode::Pip => requested_rect
                        .or(placements[index].rect)
                        .or(Some(default_pip_rect())),
                };
                let changed = placements[index] != replacement;
                placements[index] = replacement.clone();
                if changed {
                    crate::data::broll::save(&dir, &placements)?;
                }
                Ok((replacement, changed))
            },
            |(_, changed)| *changed,
        )
        .map(|(placement, _)| placement)
    })
    .await
}

#[tauri::command]
pub async fn broll_remove(pid: String, id: String, root: Option<PathBuf>) -> AppResult<bool> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("B-roll remove", move || {
        crate::data::edit_history::record(
            &dir,
            "Remove B-roll",
            || {
                let mut placements = crate::data::broll::load(&dir)?;
                let before = placements.len();
                placements.retain(|placement| placement.id != id);
                if placements.len() == before {
                    return Ok(false);
                }
                crate::data::broll::save(&dir, &placements)?;
                Ok(true)
            },
            |changed| *changed,
        )
    })
    .await
}

fn broll_preview_points(
    doc: &Doc,
    cuts: &ClipCuts,
    placements: &[crate::data::broll::BrollPlacement],
) -> Vec<(f64, f64, usize)> {
    let intervals = crate::export::cut_intervals(doc, &cuts.cuts);
    let kept = crate::export::kept_intervals(doc, &cuts.cuts);
    placements
        .iter()
        .enumerate()
        .filter_map(|(index, placement)| {
            let (source_start, source_end) = kept
                .iter()
                .filter_map(|(start, end)| {
                    let overlap_start = placement.start.max(*start);
                    let overlap_end = placement.end.min(*end);
                    (overlap_end > overlap_start).then_some((overlap_start, overlap_end))
                })
                .max_by(|left, right| (left.1 - left.0).total_cmp(&(right.1 - right.0)))?;
            let source_time = (source_start + source_end) / 2.0;
            let program_time = crate::export::retime(source_time, &intervals);
            Some((program_time, source_time, index))
        })
        .collect()
}

#[tauri::command]
pub async fn broll_preview(
    pid: String,
    at: Vec<f64>,
    root: Option<PathBuf>,
    app: tauri::AppHandle,
    state: tauri::State<'_, BrollPreviewState>,
) -> AppResult<Vec<String>> {
    broll_preview_impl(pid, at, root, app, state.inner().clone(), None).await
}

async fn broll_preview_impl(
    pid: String,
    at: Vec<f64>,
    root: Option<PathBuf>,
    app: tauri::AppHandle,
    state: BrollPreviewState,
    on_progress: Option<BrollPreviewProgressCallback>,
) -> AppResult<Vec<String>> {
    if let Some(callback) = on_progress.as_ref() {
        callback(BrollPreviewProgress {
            phase: "waiting".into(),
            progress: 0,
            current: None,
            total: None,
            encoder: None,
        });
    }
    let _heavy_work = crate::performance::acquire_heavy("broll-preview").await?;
    if let Some(callback) = on_progress.as_ref() {
        callback(BrollPreviewProgress {
            phase: "preparing".into(),
            progress: 3,
            current: None,
            total: None,
            encoder: None,
        });
    }
    let dir = resolve_project_dir(&pid, root)?;
    let prepare_dir = dir.clone();
    let has_custom_timestamps = !at.is_empty();
    let (doc, cuts, placements, ass) = {
        let _mutation = lock_project_mutation(&dir).await;
        run_blocking("B-roll preview preparation", move || {
            let doc = Doc::load(&prepare_dir)?;
            let cuts = load_project_cuts(&prepare_dir)?;
            let placements = crate::data::broll::load(&prepare_dir)?;
            let ass = prepare_dir.join("broll-preview.ass");
            if !placements.is_empty() && has_custom_timestamps {
                let style = crate::data::substyle::SubStyle::load(&prepare_dir)?;
                let settings = crate::data::export_settings::load(&prepare_dir)?;
                let hidden = crate::data::subtitle::load_hidden_checked(&prepare_dir)?;
                let caption_doc = crate::data::export_settings::project_caption_doc_with_hidden(
                    &doc,
                    settings.subtitle_language.as_deref(),
                    settings.bilingual_subtitles,
                    &hidden,
                )?;
                crate::export::write_ass_with_style(
                    &caption_doc,
                    &cuts.cuts,
                    &style,
                    &ass,
                    1920,
                    1080,
                )?;
            }
            Ok((doc, cuts, placements, ass))
        })
        .await?
    };
    if placements.is_empty() {
        let scope = app.asset_protocol_scope();
        let mut current = state.current.lock().expect("B-roll preview state poisoned");
        for previous in current.iter() {
            scope
                .forbid_file(previous)
                .map_err(|error| AppError::Schema(format!("B-roll preview scope: {error}")))?;
        }
        current.clear();
        if let Some(callback) = on_progress {
            callback(BrollPreviewProgress {
                phase: "completed".into(),
                progress: 100,
                current: None,
                total: None,
                encoder: None,
            });
        }
        return Ok(Vec::new());
    }
    let mut outputs = Vec::new();
    if at.is_empty() {
        let points = broll_preview_points(&doc, &cuts, &placements);
        let frame_total = points.len();
        for (index, (program_time, source_time, placement_index)) in points.into_iter().enumerate()
        {
            let output = dir.join(format!(
                "broll-preview-{program_time:.1}-{placement_index}.png"
            ));
            crate::export::video::render_broll_snapshot(
                &doc,
                &placements[placement_index],
                source_time,
                &output,
            )
            .await?;
            outputs.push(output.to_string_lossy().into_owned());
            if let Some(callback) = on_progress.as_ref() {
                callback(BrollPreviewProgress {
                    phase: "frames".into(),
                    progress: 5 + (((index + 1) * 94) / frame_total.max(1)) as u8,
                    current: Some((index + 1) as f64),
                    total: Some(frame_total as f64),
                    encoder: None,
                });
            }
        }
    } else {
        let video = dir.join("broll-preview.mp4");
        let render_report = on_progress.clone();
        crate::export::video::render_video_with_broll_progress(
            &doc,
            &cuts.cuts,
            &ass,
            &video,
            &placements,
            crate::export::video::RenderPurpose::Preview,
            render_report.map(|callback| {
                Arc::new(move |progress: crate::export::video::VideoRenderProgress| {
                    callback(BrollPreviewProgress {
                        phase: "encoding".into(),
                        progress: 5 + ((u16::from(progress.progress) * 85) / 100) as u8,
                        current: Some(progress.current_seconds),
                        total: Some(progress.total_seconds),
                        encoder: Some(progress.encoder),
                    });
                }) as crate::export::video::VideoRenderProgressCallback
            }),
        )
        .await?;
        let frame_total = at.len();
        for (index, timestamp) in at.into_iter().enumerate() {
            let output = dir.join(format!("broll-preview-{timestamp:.1}.png"));
            crate::media::extract_frame(&video, timestamp, &output).await?;
            outputs.push(output.to_string_lossy().into_owned());
            if let Some(callback) = on_progress.as_ref() {
                callback(BrollPreviewProgress {
                    phase: "frames".into(),
                    progress: 90 + (((index + 1) * 9) / frame_total.max(1)) as u8,
                    current: Some((index + 1) as f64),
                    total: Some(frame_total as f64),
                    encoder: None,
                });
            }
        }
    }
    let scope = app.asset_protocol_scope();
    let output_paths = outputs.iter().map(PathBuf::from).collect::<Vec<_>>();
    let mut current = state.current.lock().expect("B-roll preview state poisoned");
    for previous in current
        .iter()
        .filter(|previous| !output_paths.contains(previous))
    {
        scope
            .forbid_file(previous)
            .map_err(|error| AppError::Schema(format!("B-roll preview scope: {error}")))?;
    }
    for output in &output_paths {
        scope
            .allow_file(output)
            .map_err(|error| AppError::Schema(format!("B-roll preview scope: {error}")))?;
    }
    *current = output_paths;
    if let Some(callback) = on_progress {
        callback(BrollPreviewProgress {
            phase: "completed".into(),
            progress: 100,
            current: None,
            total: None,
            encoder: None,
        });
    }
    Ok(outputs)
}

#[tauri::command]
pub async fn broll_preview_start(
    pid: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, BrollPreviewState>,
) -> AppResult<BrollPreviewJobStatus> {
    let status_path = broll_preview_status_path(&pid, None)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let now = unix_timestamp_seconds();
    let status = BrollPreviewJobStatus {
        pid: pid.clone(),
        state: "running".into(),
        phase: "waiting".into(),
        progress: 0,
        current: None,
        total: None,
        encoder: None,
        started_at: Some(now),
        updated_at: Some(now),
        error: None,
        paths: vec![],
    };
    {
        let mut jobs = state.jobs.lock().expect("B-roll preview state poisoned");
        if jobs
            .get(&pid)
            .is_some_and(|job| matches!(job.status.state.as_str(), "running" | "cancelling"))
        {
            return Err(AppError::Schema(
                "this project already has a B-roll preview in progress".into(),
            ));
        }
        jobs.insert(
            pid.clone(),
            BrollPreviewJob {
                status: status.clone(),
                cancel: cancel.clone(),
                status_path: status_path.clone(),
            },
        );
    }
    let initial = status.clone();
    let initial_path = status_path.clone();
    if let Err(error) = run_blocking("save B-roll preview status", move || {
        save_broll_preview_status(&initial_path, &initial)
    })
    .await
    {
        state
            .jobs
            .lock()
            .expect("B-roll preview state poisoned")
            .remove(&pid);
        return Err(error);
    }
    trace_pipeline_started("broll-preview", &pid);

    let preview_state = state.inner().clone();
    let jobs = preview_state.jobs.clone();
    let task_pid = pid.clone();
    tauri::async_runtime::spawn(async move {
        let progress_jobs = jobs.clone();
        let progress_pid = task_pid.clone();
        let work = broll_preview_impl(
            task_pid.clone(),
            vec![],
            None,
            app,
            preview_state,
            Some(Arc::new(move |progress| {
                update_broll_preview_job(&progress_jobs, &progress_pid, progress);
            })),
        );
        let result = crate::proc::with_cancellation(cancel, work).await;
        let mut final_status = {
            let guard = jobs.lock().expect("B-roll preview state poisoned");
            let Some(job) = guard.get(&task_pid) else {
                return;
            };
            let mut status = job.status.clone();
            match result {
                Ok(paths) => {
                    status.state = "completed".into();
                    status.phase = "completed".into();
                    status.progress = 100;
                    status.paths = paths;
                    status.error = None;
                }
                Err(AppError::Cancelled) => {
                    status.state = "cancelled".into();
                    status.phase = "cancelled".into();
                    status.error = None;
                }
                Err(error) => {
                    status.state = "failed".into();
                    status.phase = "failed".into();
                    status.error = Some(error.to_string());
                }
            }
            status.updated_at = Some(unix_timestamp_seconds());
            status
        };
        if let Some(job) = jobs
            .lock()
            .expect("B-roll preview state poisoned")
            .get_mut(&task_pid)
        {
            job.status = final_status.clone();
        }
        let persisted = final_status.clone();
        if let Err(error) = persist_background_status(
            "save final B-roll preview status",
            status_path,
            persisted,
            save_broll_preview_status,
        )
        .await
        {
            final_status.state = "failed".into();
            final_status.phase = "failed".into();
            final_status.error = Some(format!(
                "B-roll preview finished but its recovery status could not be saved: {error}"
            ));
        }
        if let Some(job) = jobs
            .lock()
            .expect("B-roll preview state poisoned")
            .get_mut(&task_pid)
        {
            job.status = final_status.clone();
        }
        trace_pipeline_finished(
            "broll-preview",
            &task_pid,
            &final_status.state,
            final_status.error.as_deref(),
        );
    });
    Ok(status)
}

#[tauri::command]
pub async fn broll_preview_status(
    pid: String,
    restore_assets: Option<bool>,
    app: tauri::AppHandle,
    state: tauri::State<'_, BrollPreviewState>,
) -> AppResult<BrollPreviewJobStatus> {
    let active = state
        .jobs
        .lock()
        .expect("B-roll preview state poisoned")
        .get(&pid)
        .map(|job| (job.status.clone(), job.status_path.clone()));
    if let Some((status, path)) = active {
        persist_background_status(
            "checkpoint B-roll preview status",
            path.clone(),
            status.clone(),
            save_broll_preview_status,
        )
        .await?;
        let latest = state
            .jobs
            .lock()
            .expect("B-roll preview state poisoned")
            .get(&pid)
            .map(|job| job.status.clone())
            .unwrap_or_else(|| status.clone());
        if latest.state != status.state
            || latest.phase != status.phase
            || latest.progress != status.progress
            || latest.updated_at != status.updated_at
        {
            persist_background_status(
                "checkpoint latest B-roll preview status",
                path.clone(),
                latest.clone(),
                save_broll_preview_status,
            )
            .await?;
        }
        if !restore_assets.unwrap_or(false) {
            return Ok(latest);
        }
        let latest_paths = latest.paths.clone();
        let restored = restore_broll_preview_assets(&pid, latest, &app, state.inner()).await?;
        if restored.paths != latest_paths {
            persist_background_status(
                "save recovered B-roll preview status",
                path,
                restored.clone(),
                save_broll_preview_status,
            )
            .await?;
            if let Some(job) = state
                .jobs
                .lock()
                .expect("B-roll preview state poisoned")
                .get_mut(&pid)
            {
                job.status = restored.clone();
            }
        }
        return Ok(restored);
    }
    let path = broll_preview_status_path(&pid, None)?;
    let loaded = run_blocking("load B-roll preview status", move || {
        load_recovered_broll_preview_status(&path).map_err(|error| match error {
            AppError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
                AppError::Schema("no B-roll preview job for this project".into())
            }
            other => other,
        })
    })
    .await?;
    if !restore_assets.unwrap_or(false) {
        return Ok(loaded);
    }
    let original_paths = loaded.paths.clone();
    let restored = restore_broll_preview_assets(&pid, loaded, &app, state.inner()).await?;
    if restored.paths != original_paths {
        let path = broll_preview_status_path(&pid, None)?;
        persist_background_status(
            "save recovered B-roll preview status",
            path,
            restored.clone(),
            save_broll_preview_status,
        )
        .await?;
    }
    Ok(restored)
}

#[tauri::command]
pub async fn broll_preview_cancel(
    pid: String,
    state: tauri::State<'_, BrollPreviewState>,
) -> AppResult<BrollPreviewJobStatus> {
    let (status, path) = {
        let mut jobs = state.jobs.lock().expect("B-roll preview state poisoned");
        let job = jobs
            .get_mut(&pid)
            .ok_or_else(|| AppError::Schema("no B-roll preview job for this project".into()))?;
        if job.status.state == "running" {
            job.cancel.store(true, Ordering::Relaxed);
            job.status.state = "cancelling".into();
            job.status.phase = "cancelling".into();
            job.status.updated_at = Some(unix_timestamp_seconds());
        }
        (job.status.clone(), job.status_path.clone())
    };
    persist_background_status(
        "checkpoint B-roll preview cancellation",
        path,
        status.clone(),
        save_broll_preview_status,
    )
    .await?;
    Ok(status)
}

#[derive(Debug, Serialize)]
pub struct DiarizeResult {
    pub segments: usize,
    pub paragraphs_assigned: usize,
}

#[tauri::command]
pub async fn diarize_pid(pid: String, root: Option<PathBuf>) -> AppResult<DiarizeResult> {
    let _heavy_work = crate::performance::acquire_heavy("speaker-analysis-cli").await?;
    let dir = resolve_project_dir(&pid, root)?;
    let load_dir = dir.clone();
    let (media_path, model) = run_blocking("diarization preparation", move || {
        let doc = Doc::load(&load_dir)?;
        let model = crate::data::modelconfig::load().diarize_model;
        validate_speaker_preflight(&model)?;
        Ok((doc.media.path, model))
    })
    .await?;
    let wav = dir.join("audio.wav");
    {
        let _audio_mutation = lock_project_mutation(&dir).await;
        if !tokio::fs::try_exists(&wav).await? {
            extract_audio_wav(&media_path, &wav).await?;
        }
    }
    let out = crate::diarize::diarize_file_with_model(&wav, &model).await?;
    let segments = out.segments;
    let segment_count = segments.len();
    let _mutation = lock_project_mutation(&dir).await;
    let paragraphs_assigned = run_blocking("diarization save", move || {
        let mut doc = Doc::load(&dir)?;
        let original = doc.clone();
        let paragraphs_assigned = crate::diarize::assign_speakers(&mut doc, &segments);
        if paragraphs_assigned > 0 && doc != original {
            if !working_head_is_committed(&dir, &original)? {
                let mut lineage = crate::data::version::Lineage::load(&dir)?;
                let branch = lineage
                    .active_branch
                    .clone()
                    .unwrap_or_else(|| "main".into());
                crate::data::version::commit_snapshot(
                    &dir,
                    &original,
                    &mut lineage,
                    &branch,
                    "Before speaker diarization",
                    "automatic recovery snapshot",
                    crate::data::version::VersionKind::Auto,
                )?;
            }
            doc.meta.updated_at = chrono::Utc::now();
            doc.save(&dir)?;
            crate::data::activity::touch(&dir)?;
        }
        Ok(paragraphs_assigned)
    })
    .await?;
    Ok(DiarizeResult {
        segments: segment_count,
        paragraphs_assigned,
    })
}

#[tauri::command]
pub async fn timing_repair(pid: String, root: Option<PathBuf>) -> AppResult<String> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("timing repair", move || {
        let original = Doc::load(&dir)?;
        let mut doc = original.clone();
        let rep = crate::pipeline::timing::repair(&mut doc);
        if rep.total() > 0 {
            if !working_head_is_committed(&dir, &original)? {
                let mut lineage = crate::data::version::Lineage::load(&dir)?;
                let branch = lineage
                    .active_branch
                    .clone()
                    .unwrap_or_else(|| "main".into());
                crate::data::version::commit_snapshot(
                    &dir,
                    &original,
                    &mut lineage,
                    &branch,
                    "before timing repair",
                    "automatic recovery snapshot",
                    crate::data::version::VersionKind::Auto,
                )?;
            }
            doc.meta.updated_at = chrono::Utc::now();
            doc.save(&dir)?;
            crate::data::activity::touch(&dir)?;
        }
        Ok(format!("{} fix(es)", rep.total()))
    })
    .await
}

#[tauri::command]
pub async fn model_list() -> Vec<String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    run_blocking("model cache list", move || {
        Ok(
            std::fs::read_dir(crate::data::modelconfig::hugging_face_cache_root(&home))
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .filter(|n| n.starts_with("models--"))
                        .collect()
                })
                .unwrap_or_default(),
        )
    })
    .await
    .unwrap_or_default()
}

fn derive_models_endpoint(endpoint: &str) -> AppResult<String> {
    let mut url = reqwest::Url::parse(endpoint.trim())
        .map_err(|_| AppError::Schema("AI service URL is invalid".into()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::Schema(
            "AI service URL must use http or https".into(),
        ));
    }
    let path = url.path().trim_end_matches('/');
    let base = [
        "/chat/completions",
        "/text/chatcompletion_v2",
        "/messages",
        "/responses",
        "/audio/transcriptions",
    ]
    .iter()
    .find_map(|suffix| path.strip_suffix(suffix))
    .ok_or_else(|| {
        AppError::Schema("cannot infer a model catalog URL from this AI service URL".into())
    })?;
    url.set_path(&format!("{base}/models"));
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.into())
}

fn parse_llm_models(body: &str) -> AppResult<Vec<String>> {
    let value: serde_json::Value = serde_json::from_str(body)?;
    let data = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| AppError::Schema("provider returned an invalid model catalog".into()))?;
    let mut seen = HashSet::new();
    let models = data
        .iter()
        .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .filter(|id| seen.insert((*id).to_owned()))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if models.is_empty() {
        return Err(AppError::Schema(
            "provider returned an empty model catalog".into(),
        ));
    }
    Ok(models)
}

/// Fetch the provider's current OpenAI-compatible model catalog. This is a
/// short asynchronous network request and never runs on the UI thread.
#[tauri::command]
pub async fn llm_models_list(endpoint: String, api_key: String) -> AppResult<Vec<String>> {
    let api_key = if api_key.trim().is_empty() {
        let requested_endpoint = endpoint.trim().to_string();
        run_blocking("settings credential lookup", move || {
            let config = crate::data::modelconfig::load();
            Ok(if config.llm_endpoint.trim() == requested_endpoint {
                config.llm_api_key
            } else {
                String::new()
            })
        })
        .await?
    } else {
        api_key
    };
    models_list_impl(endpoint, api_key).await
}

async fn models_list_impl(endpoint: String, api_key: String) -> AppResult<Vec<String>> {
    let catalog_url = derive_models_endpoint(&endpoint)?;
    let anthropic = reqwest::Url::parse(&catalog_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .is_some_and(|host| host == "api.anthropic.com" || host.ends_with(".anthropic.com"));
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| AppError::Schema(format!("model catalog client: {error}")))?;
    let mut request = client.get(catalog_url);
    if !api_key.trim().is_empty() {
        request = if anthropic {
            request
                .header("x-api-key", api_key.trim())
                .header("anthropic-version", "2023-06-01")
        } else {
            request.bearer_auth(api_key.trim())
        };
    }
    let response = request
        .send()
        .await
        .map_err(|error| AppError::Schema(format!("model catalog request failed: {error}")))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| AppError::Schema(format!("model catalog response failed: {error}")))?;
    if !status.is_success() {
        let summary = body.chars().take(400).collect::<String>();
        return Err(AppError::Schema(format!(
            "model catalog returned {}: {}",
            status.as_u16(),
            summary
        )));
    }
    parse_llm_models(&body)
}

#[tauri::command]
pub async fn asr_models_list(endpoint: String, api_key: String) -> AppResult<Vec<String>> {
    let api_key = if api_key.trim().is_empty() {
        let requested_endpoint = endpoint.trim().to_string();
        run_blocking("ASR credential lookup", move || {
            let config = crate::data::modelconfig::load();
            Ok(if config.asr_cloud_endpoint.trim() == requested_endpoint {
                config.asr_cloud_api_key
            } else {
                String::new()
            })
        })
        .await?
    } else {
        api_key
    };
    models_list_impl(endpoint, api_key).await
}

/// Report whether local ASR can really run. This imports the Python package
/// and validates complete model snapshots instead of checking directory names.
#[tauri::command]
pub async fn asr_status() -> AppResult<crate::asr::RuntimeStatus> {
    run_blocking("ASR environment probe", || Ok(crate::asr::runtime_status())).await
}

/// Create an app-owned Python 3.12 environment and install the ASR runtime.
#[tauri::command]
pub async fn asr_runtime_install() -> AppResult<crate::asr::RuntimeStatus> {
    crate::asr::install_asr_runtime().await
}

/// Download the configured ASR and word-alignment model snapshots.
#[tauri::command]
pub async fn asr_models_download() -> AppResult<crate::asr::RuntimeStatus> {
    crate::asr::download_asr_models().await
}

/// Install the optional, separately-failing speaker identification runtime.
#[tauri::command]
pub async fn diarize_runtime_install() -> AppResult<crate::asr::RuntimeStatus> {
    crate::asr::install_diarize_runtime().await
}

/// Download the gated speaker pipeline after the user supplies authorization.
#[tauri::command]
pub async fn diarize_model_download() -> AppResult<crate::asr::RuntimeStatus> {
    crate::asr::download_diarize_model().await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupJobStatus {
    pub kind: String,
    pub state: String,
    pub phase: String,
    #[serde(default)]
    pub started_at: Option<u64>,
    #[serde(default)]
    pub updated_at: Option<u64>,
    pub error: Option<String>,
}

struct SetupJob {
    status: SetupJobStatus,
    cancel: Arc<AtomicBool>,
    status_path: PathBuf,
}

#[derive(Clone, Default)]
pub struct SetupJobState {
    job: Arc<Mutex<Option<SetupJob>>>,
}

fn setup_status_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".lumen-cut/setup-job.json")
}

fn save_setup_status(path: &std::path::Path, status: &SetupJobStatus) -> AppResult<()> {
    crate::data::storage::write_json(path, status)
}

fn load_setup_status(path: &std::path::Path) -> AppResult<SetupJobStatus> {
    Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
}

fn load_recovered_setup_status(path: &std::path::Path) -> AppResult<SetupJobStatus> {
    let mut status = load_setup_status(path)?;
    if matches!(status.state.as_str(), "running" | "cancelling") {
        status.state = "failed".into();
        status.phase = "failed".into();
        status.error = Some(
            "the previous setup task was interrupted when lumen-cut closed; start it again".into(),
        );
        status.updated_at = Some(unix_timestamp_seconds());
        save_setup_status(path, &status)?;
    }
    Ok(status)
}

fn setup_phase(kind: &str) -> Option<&'static str> {
    match kind {
        "asr-runtime" | "speaker-runtime" => Some("installing"),
        "asr-models" | "speaker-model" => Some("downloading"),
        _ => None,
    }
}

#[tauri::command]
pub async fn setup_job_start(
    kind: String,
    state: tauri::State<'_, SetupJobState>,
) -> AppResult<SetupJobStatus> {
    let phase =
        setup_phase(&kind).ok_or_else(|| AppError::Schema(format!("unknown setup job: {kind}")))?;
    let cancel = Arc::new(AtomicBool::new(false));
    let now = unix_timestamp_seconds();
    let status = SetupJobStatus {
        kind: kind.clone(),
        state: "running".into(),
        phase: "waiting".into(),
        started_at: Some(now),
        updated_at: Some(now),
        error: None,
    };
    {
        let mut active = state.job.lock().expect("setup job state poisoned");
        if active
            .as_ref()
            .is_some_and(|job| matches!(job.status.state.as_str(), "running" | "cancelling"))
        {
            return Err(AppError::Schema(
                "another runtime or model setup task is already running".into(),
            ));
        }
        *active = Some(SetupJob {
            status: status.clone(),
            cancel: cancel.clone(),
            status_path: setup_status_path(),
        });
    }
    let path = setup_status_path();
    let initial = status.clone();
    let initial_path = path.clone();
    if let Err(error) = run_blocking("save setup status", move || {
        save_setup_status(&initial_path, &initial)
    })
    .await
    {
        *state.job.lock().expect("setup job state poisoned") = None;
        return Err(error);
    }
    trace_pipeline_started("setup", &kind);

    let job = state.job.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(active) = job.lock().expect("setup job state poisoned").as_mut() {
            active.status.phase = phase.into();
            active.status.updated_at = Some(unix_timestamp_seconds());
        }
        let work = async {
            match kind.as_str() {
                "asr-runtime" => crate::asr::install_asr_runtime().await.map(|_| ()),
                "asr-models" => crate::asr::download_asr_models().await.map(|_| ()),
                "speaker-runtime" => crate::asr::install_diarize_runtime().await.map(|_| ()),
                "speaker-model" => crate::asr::download_diarize_model().await.map(|_| ()),
                _ => unreachable!("setup kind was validated"),
            }
        };
        let result = crate::proc::with_cancellation(cancel, work).await;
        let mut final_status = {
            let guard = job.lock().expect("setup job state poisoned");
            let Some(active) = guard.as_ref() else {
                return;
            };
            let mut status = active.status.clone();
            match result {
                Ok(()) => {
                    status.state = "completed".into();
                    status.phase = "completed".into();
                    status.error = None;
                }
                Err(AppError::Cancelled) => {
                    status.state = "cancelled".into();
                    status.phase = "cancelled".into();
                    status.error = None;
                }
                Err(error) => {
                    status.state = "failed".into();
                    status.phase = "failed".into();
                    status.error = Some(error.to_string());
                }
            }
            status.updated_at = Some(unix_timestamp_seconds());
            status
        };
        if let Some(active) = job.lock().expect("setup job state poisoned").as_mut() {
            active.status = final_status.clone();
        }
        let persisted = final_status.clone();
        if let Err(error) = persist_background_status(
            "save final setup status",
            path,
            persisted,
            save_setup_status,
        )
        .await
        {
            final_status.state = "failed".into();
            final_status.phase = "failed".into();
            final_status.error = Some(format!(
                "setup finished but its recovery status could not be saved: {error}"
            ));
        }
        if let Some(active) = job.lock().expect("setup job state poisoned").as_mut() {
            active.status = final_status.clone();
        }
        trace_pipeline_finished(
            "setup",
            &kind,
            &final_status.state,
            final_status.error.as_deref(),
        );
    });
    Ok(status)
}

#[tauri::command]
pub async fn setup_job_status(state: tauri::State<'_, SetupJobState>) -> AppResult<SetupJobStatus> {
    let active = state
        .job
        .lock()
        .expect("setup job state poisoned")
        .as_ref()
        .map(|job| (job.status.clone(), job.status_path.clone()));
    if let Some((status, path)) = active {
        persist_background_status(
            "checkpoint setup status",
            path.clone(),
            status.clone(),
            save_setup_status,
        )
        .await?;
        let latest = state
            .job
            .lock()
            .expect("setup job state poisoned")
            .as_ref()
            .map(|job| job.status.clone())
            .unwrap_or_else(|| status.clone());
        if latest.state != status.state
            || latest.phase != status.phase
            || latest.updated_at != status.updated_at
        {
            persist_background_status(
                "checkpoint latest setup status",
                path,
                latest.clone(),
                save_setup_status,
            )
            .await?;
        }
        return Ok(latest);
    }
    let path = setup_status_path();
    run_blocking("load setup status", move || {
        load_recovered_setup_status(&path).map_err(|error| match error {
            AppError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
                AppError::Schema("no setup job has been started".into())
            }
            other => other,
        })
    })
    .await
}

#[tauri::command]
pub async fn setup_job_cancel(state: tauri::State<'_, SetupJobState>) -> AppResult<SetupJobStatus> {
    let (status, path) = {
        let mut job = state.job.lock().expect("setup job state poisoned");
        let active = job
            .as_mut()
            .ok_or_else(|| AppError::Schema("no setup job is running".into()))?;
        if active.status.state == "running" {
            active.cancel.store(true, Ordering::Relaxed);
            active.status.state = "cancelling".into();
            active.status.phase = "cancelling".into();
            active.status.updated_at = Some(unix_timestamp_seconds());
        }
        (active.status.clone(), active.status_path.clone())
    };
    persist_background_status(
        "checkpoint setup cancellation",
        path,
        status.clone(),
        save_setup_status,
    )
    .await?;
    Ok(status)
}

#[tauri::command]
pub async fn logs_list(pid: String, root: Option<PathBuf>) -> AppResult<Vec<(String, usize)>> {
    let dir = resolve_project_dir(&pid, root)?.join("ai");
    run_blocking("task log list", move || {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.filter_map(|e| e.ok()) {
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let k = e.file_name().to_string_lossy().into_owned();
                    let n = std::fs::read_dir(e.path())
                        .map(|rd| rd.count())
                        .unwrap_or(0);
                    out.push((k, n));
                }
            }
        }
        Ok(out)
    })
    .await
}

/// Reveal the persistent application log written by the non-blocking tracing
/// worker. This is deliberately user-triggered; diagnostics never open windows
/// on their own.
#[tauri::command]
pub async fn logs_reveal() -> AppResult<String> {
    let dir = crate::log_directory();
    tokio::fs::create_dir_all(&dir).await?;
    let status = tokio::process::Command::new("open")
        .arg(&dir)
        .status()
        .await?;
    if !status.success() {
        return Err(AppError::Schema(format!(
            "could not reveal diagnostics folder ({status})"
        )));
    }
    Ok(dir.to_string_lossy().into_owned())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStarted {
    pub pid: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStopped {
    pub pid: String,
    pub path: String,
    pub duration_seconds: f64,
}

fn recording_output(pid: &str, root: Option<PathBuf>) -> AppResult<PathBuf> {
    let trimmed = pid.trim();
    let is_single_component = std::path::Path::new(trimmed).components().count() == 1;
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || !is_single_component
        || trimmed.contains(['/', '\\'])
    {
        return Err(AppError::Schema("invalid recording project id".into()));
    }
    Ok(resolve_project_root(root).join(trimmed).join("audio.wav"))
}

#[tauri::command]
pub async fn recording_start(
    pid: String,
    root: Option<PathBuf>,
    state: tauri::State<'_, RecordingState>,
) -> AppResult<RecordingStarted> {
    let wav = recording_output(&pid, root)?;
    if state
        .starting
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(AppError::Schema(
            "another microphone recording is already in progress".into(),
        ));
    }
    if state
        .session
        .lock()
        .expect("recording state poisoned")
        .is_some()
    {
        state.starting.store(false, Ordering::Release);
        return Err(AppError::Schema(
            "another microphone recording is already in progress".into(),
        ));
    }

    let started = async {
        let project_dir = wav
            .parent()
            .ok_or_else(|| AppError::Schema("recording path has no project directory".into()))?;
        let mutation = lock_project_mutation(project_dir).await;
        if let Some(dir) = wav.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        if tokio::fs::try_exists(&wav).await? {
            tokio::fs::remove_file(&wav).await?;
        }

        let mut command = tokio::process::Command::new("ffmpeg");
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "avfoundation",
                "-i",
                ":0",
                "-ac",
                "1",
                "-ar",
                "16000",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(&wav)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|error| {
            AppError::Io(std::io::Error::other(format!(
                "ffmpeg microphone recording: {error}"
            )))
        })?;

        // Missing devices and denied microphone access normally make ffmpeg
        // exit immediately. Yield to the runtime while checking startup so
        // neither AppKit nor a Tokio worker thread is put to sleep.
        tokio::time::sleep(Duration::from_millis(140)).await;
        if let Some(status) = child.try_wait()? {
            let _ = tokio::fs::remove_file(&wav).await;
            return Err(AppError::Schema(format!(
                "ffmpeg microphone recording stopped before it started ({status})"
            )));
        }
        Ok(RecordingSession {
            pid: pid.clone(),
            wav: wav.clone(),
            child,
            _mutation: mutation,
        })
    }
    .await;
    state.starting.store(false, Ordering::Release);
    let session = started?;
    let mut slot = state.session.lock().expect("recording state poisoned");
    if slot.is_some() {
        return Err(AppError::Schema(
            "another microphone recording is already in progress".into(),
        ));
    }
    *slot = Some(session);
    trace_pipeline_started("recording", &pid);
    Ok(RecordingStarted {
        pid,
        path: wav.to_string_lossy().into_owned(),
    })
}

fn take_recording(pid: &str, state: &RecordingState) -> AppResult<RecordingSession> {
    let mut slot = state.session.lock().expect("recording state poisoned");
    let Some(active) = slot.as_ref() else {
        return Err(AppError::Schema("there is no active recording".into()));
    };
    if active.pid != pid {
        return Err(AppError::Schema(format!(
            "recording belongs to a different project ({})",
            active.pid
        )));
    }
    Ok(slot.take().expect("recording session disappeared"))
}

async fn stop_recording_session(
    mut session: RecordingSession,
) -> AppResult<(PathBuf, std::process::ExitStatus)> {
    if let Some(mut stdin) = session.child.stdin.take() {
        let _ = stdin.write_all(b"q\n").await;
        let _ = stdin.flush().await;
    }

    let status = match tokio::time::timeout(Duration::from_secs(5), session.child.wait()).await {
        Ok(status) => status?,
        Err(_) => {
            session.child.kill().await?;
            session.child.wait().await?
        }
    };
    Ok((session.wav, status))
}

async fn finalize_recording(session: RecordingSession) -> AppResult<PathBuf> {
    let (wav, status) = stop_recording_session(session).await?;
    let usable = status.success()
        && tokio::fs::metadata(&wav)
            .await
            .map(|metadata| metadata.len() > 44)
            .unwrap_or(false);
    if !usable {
        let _ = tokio::fs::remove_file(&wav).await;
        return Err(AppError::Schema(
            "ffmpeg did not produce a usable microphone recording".into(),
        ));
    }
    Ok(wav)
}

#[tauri::command]
pub async fn recording_stop(
    pid: String,
    state: tauri::State<'_, RecordingState>,
) -> AppResult<RecordingStopped> {
    let session = take_recording(&pid, &state)?;
    let wav = finalize_recording(session).await?;
    let info = probe(&wav).await?;
    trace_pipeline_finished("recording", &pid, "completed", None);
    Ok(RecordingStopped {
        pid,
        path: wav.to_string_lossy().into_owned(),
        duration_seconds: info.duration_seconds,
    })
}

#[tauri::command]
pub async fn recording_cancel(
    pid: String,
    state: tauri::State<'_, RecordingState>,
) -> AppResult<bool> {
    let session = take_recording(&pid, &state)?;
    let dir = session.wav.parent().map(PathBuf::from);
    let wav = session.wav.clone();
    stop_recording_session(session).await?;
    let _ = tokio::fs::remove_file(&wav).await;
    if let Some(dir) = dir {
        let _ = tokio::fs::remove_dir(dir).await;
    }
    trace_pipeline_finished("recording", &pid, "cancelled", None);
    Ok(true)
}

/// Run the environment health checks used by the CLI and diagnostics UI.
#[tauri::command]
pub async fn run_doctor() -> AppResult<Vec<crate::doctor::Check>> {
    run_blocking("environment health checks", || Ok(crate::doctor::checks())).await
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceStatus {
    pub active_pipeline: Option<String>,
    pub waiting_pipelines: usize,
}

#[tauri::command]
pub async fn performance_status() -> PerformanceStatus {
    PerformanceStatus {
        active_pipeline: crate::performance::active_heavy_label(),
        waiting_pipelines: crate::performance::waiting_heavy_count(),
    }
}

/// Burn-in export: write export.ass then ffmpeg → export.mp4.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoExportJobStatus {
    pub pid: String,
    #[serde(default = "default_video_export_mode")]
    pub mode: String,
    #[serde(default)]
    pub settings: crate::data::export_settings::VideoExportSettings,
    pub state: String,
    pub phase: String,
    pub progress: u8,
    pub current_seconds: Option<f64>,
    pub total_seconds: Option<f64>,
    pub encoder: Option<String>,
    #[serde(default)]
    pub started_at: Option<u64>,
    #[serde(default)]
    pub updated_at: Option<u64>,
    pub error: Option<String>,
    pub path: Option<String>,
}

fn default_video_export_mode() -> String {
    "fast".into()
}

struct VideoExportJob {
    status: VideoExportJobStatus,
    cancel: Arc<AtomicBool>,
    status_path: PathBuf,
}

#[derive(Clone, Default)]
pub struct VideoExportState {
    jobs: Arc<Mutex<HashMap<String, VideoExportJob>>>,
}

fn video_export_status_path(pid: &str, root: Option<PathBuf>) -> AppResult<PathBuf> {
    let _ = resolve_project_dir(pid, root.clone())?;
    Ok(resolve_project_root(root)
        .join(".jobs")
        .join(format!("{pid}.video-export.json")))
}

fn save_video_export_status(
    path: &std::path::Path,
    status: &VideoExportJobStatus,
) -> AppResult<()> {
    crate::data::storage::write_json(path, status)
}

fn load_video_export_status(path: &std::path::Path) -> AppResult<VideoExportJobStatus> {
    let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let has_settings = value.get("settings").is_some();
    let mut status: VideoExportJobStatus = serde_json::from_value(value)?;
    if !has_settings && status.mode == "quality" {
        status.settings.encoding_speed = crate::data::export_settings::ExportEncodingSpeed::Quality;
    }
    Ok(status)
}

fn load_recovered_video_export_status(path: &std::path::Path) -> AppResult<VideoExportJobStatus> {
    let mut status = load_video_export_status(path)?;
    if matches!(status.state.as_str(), "running" | "cancelling") {
        status.state = "failed".into();
        status.phase = "failed".into();
        status.error = Some(
            "the previous video export was interrupted when lumen-cut closed; start it again"
                .into(),
        );
        status.updated_at = Some(unix_timestamp_seconds());
        save_video_export_status(path, &status)?;
    }
    Ok(status)
}

fn update_video_export_job(
    jobs: &Mutex<HashMap<String, VideoExportJob>>,
    pid: &str,
    progress: crate::export::video::VideoRenderProgress,
) {
    if let Some(job) = jobs
        .lock()
        .expect("video export state poisoned")
        .get_mut(pid)
    {
        if job.status.phase != "encoding" {
            tracing::info!(
                pipeline = "video-export",
                pid,
                phase = "encoding",
                "pipeline phase changed"
            );
        }
        job.status.phase = "encoding".into();
        job.status.progress = advance_progress(job.status.progress, progress.progress);
        job.status.current_seconds = Some(progress.current_seconds);
        job.status.total_seconds = Some(progress.total_seconds);
        job.status.encoder = Some(progress.encoder);
        job.status.updated_at = Some(unix_timestamp_seconds());
    }
}

async fn export_video_impl(
    pid: String,
    root: Option<PathBuf>,
    requested_settings: Option<crate::data::export_settings::VideoExportSettings>,
    on_progress: Option<crate::export::video::VideoRenderProgressCallback>,
) -> AppResult<String> {
    let _heavy_work = crate::performance::acquire_heavy("video-export").await?;
    let dir = resolve_project_dir(&pid, root)?;
    let probe_dir = dir.clone();
    let media_path = run_blocking("load video export media", move || {
        Ok(Doc::load(&probe_dir)?.media.path)
    })
    .await?;
    let media_info = crate::media::probe(&media_path).await?;
    let source_dimensions = media_info.width.zip(media_info.height);
    let prepare_dir = dir.clone();
    let (doc, cuts, ass, soft_subtitle, include_ass, broll, audio_mix, settings) = {
        // Hold the project mutation lock only while taking an export snapshot.
        // Encoding may take minutes and must not stall transcript editing.
        let _mutation = lock_project_mutation(&dir).await;
        run_blocking("video export preparation", move || {
            let doc = Doc::load(&prepare_dir)?;
            let settings = match requested_settings {
                Some(settings) => settings,
                None => crate::data::export_settings::load(&prepare_dir)?,
            };
            settings.validate()?;
            let cuts = load_project_cuts(&prepare_dir)?;
            let ass = prepare_dir.join("export.ass");
            let titles = crate::data::title::load(&prepare_dir)?;
            let style = crate::data::substyle::SubStyle::load(&prepare_dir)?;
            let hidden = crate::data::subtitle::load_hidden_checked(&prepare_dir)?;
            let caption_doc = if settings.subtitle_mode
                == crate::data::export_settings::ExportSubtitleMode::None
            {
                doc.clone()
            } else {
                crate::data::export_settings::project_caption_doc_with_hidden(
                    &doc,
                    settings.subtitle_language.as_deref(),
                    settings.bilingual_subtitles,
                    &hidden,
                )?
            };
            let (canvas_width, canvas_height) =
                settings.subtitle_canvas_dimensions(source_dimensions);
            let include_ass = match settings.subtitle_mode {
                crate::data::export_settings::ExportSubtitleMode::Burn => {
                    crate::export::write_ass_with_style_and_titles(
                        &caption_doc,
                        &cuts.cuts,
                        &style,
                        &titles,
                        &ass,
                        canvas_width,
                        canvas_height,
                    )?;
                    true
                }
                crate::data::export_settings::ExportSubtitleMode::Soft
                | crate::data::export_settings::ExportSubtitleMode::None
                    if !titles.is_empty() =>
                {
                    crate::export::write_ass_titles_only_with_style(
                        &doc,
                        &cuts.cuts,
                        &style,
                        &titles,
                        &ass,
                        canvas_width,
                        canvas_height,
                    )?;
                    true
                }
                _ => false,
            };
            let soft_subtitle = (settings.subtitle_mode
                == crate::data::export_settings::ExportSubtitleMode::Soft)
                .then(|| prepare_dir.join("export-soft.srt"));
            if let Some(path) = &soft_subtitle {
                crate::export::write_srt_with(&caption_doc, &cuts.cuts, path)?;
            }
            let broll = crate::data::broll::load(&prepare_dir)?;
            let audio_mix = crate::data::audio_mix::load(&prepare_dir)?.fit_to_duration(
                crate::export::project::kept_intervals(&doc, &cuts.cuts)
                    .iter()
                    .map(|(start, end)| end - start)
                    .sum(),
            )?;
            Ok((
                doc,
                cuts,
                ass,
                soft_subtitle,
                include_ass,
                broll,
                audio_mix,
                settings,
            ))
        })
        .await?
    };
    let output = dir.join(format!("export.{}", settings.extension()));
    let in_progress = dir.join(format!("export.in-progress.{}", settings.extension()));
    let _ = tokio::fs::remove_file(&in_progress).await;
    let render = crate::export::video::render_video_with_broll_options(
        &doc,
        &cuts.cuts,
        &ass,
        &in_progress,
        &broll,
        crate::export::video::VideoRenderOptions {
            purpose: crate::export::video::RenderPurpose::Final,
            mode: None,
            on_progress,
            audio_mix,
            settings: Some(settings),
            soft_subtitle,
            include_ass,
        },
    )
    .await;
    if let Err(error) = render {
        let _ = tokio::fs::remove_file(&in_progress).await;
        return Err(error);
    }
    let final_path = output.clone();
    run_blocking("finalize video export", move || {
        std::fs::File::open(&in_progress)?.sync_all()?;
        std::fs::rename(&in_progress, &final_path)?;
        #[cfg(unix)]
        std::fs::File::open(&dir)?.sync_all()?;
        Ok(())
    })
    .await?;
    Ok(output.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn export_video(pid: String, root: Option<PathBuf>) -> AppResult<String> {
    export_video_impl(pid, root, None, None).await
}

#[tauri::command]
pub async fn video_export_start(
    pid: String,
    settings: Option<crate::data::export_settings::VideoExportSettings>,
    mode: Option<String>,
    state: tauri::State<'_, VideoExportState>,
) -> AppResult<VideoExportJobStatus> {
    let settings = settings.unwrap_or_else(|| crate::data::export_settings::VideoExportSettings {
        encoding_speed: if mode.as_deref() == Some("quality") {
            crate::data::export_settings::ExportEncodingSpeed::Quality
        } else {
            crate::data::export_settings::ExportEncodingSpeed::Fast
        },
        ..Default::default()
    });
    settings.validate()?;
    crate::export::video::encoder_for_settings(&settings)?;
    let mode = settings.legacy_mode().to_string();
    let project_dir = resolve_project_dir(&pid, None)?;
    let settings_to_save = settings.clone();
    {
        let _mutation = lock_project_mutation(&project_dir).await;
        run_blocking("save video export settings", move || {
            crate::data::export_settings::save(&project_dir, &settings_to_save)
        })
        .await?;
    }
    let preflight = export_preflight_impl(&pid, settings.clone(), None).await?;
    if !preflight.ready {
        let blockers = preflight
            .items
            .iter()
            .filter(|item| item.level == "blocker")
            .map(|item| item.message.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(AppError::Schema(format!(
            "video export preflight failed: {blockers}"
        )));
    }
    let status_path = video_export_status_path(&pid, None)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let now = unix_timestamp_seconds();
    let status = VideoExportJobStatus {
        pid: pid.clone(),
        mode: mode.clone(),
        settings: settings.clone(),
        state: "running".into(),
        phase: "preparing".into(),
        progress: 0,
        current_seconds: None,
        total_seconds: None,
        encoder: None,
        started_at: Some(now),
        updated_at: Some(now),
        error: None,
        path: None,
    };
    {
        let mut jobs = state.jobs.lock().expect("video export state poisoned");
        if jobs
            .get(&pid)
            .is_some_and(|job| matches!(job.status.state.as_str(), "running" | "cancelling"))
        {
            return Err(AppError::Schema(
                "this project already has a video export in progress".into(),
            ));
        }
        jobs.insert(
            pid.clone(),
            VideoExportJob {
                status: status.clone(),
                cancel: cancel.clone(),
                status_path: status_path.clone(),
            },
        );
    }
    let initial_status = status.clone();
    let initial_path = status_path.clone();
    if let Err(error) = run_blocking("save video export status", move || {
        save_video_export_status(&initial_path, &initial_status)
    })
    .await
    {
        state
            .jobs
            .lock()
            .expect("video export state poisoned")
            .remove(&pid);
        return Err(error);
    }
    trace_pipeline_started("video-export", &pid);

    let jobs = state.jobs.clone();
    let task_pid = pid.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(job) = jobs
            .lock()
            .expect("video export state poisoned")
            .get_mut(&task_pid)
        {
            job.status.phase = "waiting".into();
            job.status.updated_at = Some(unix_timestamp_seconds());
        }
        let progress_jobs = jobs.clone();
        let progress_pid = task_pid.clone();
        let work = export_video_impl(
            task_pid.clone(),
            None,
            Some(settings),
            Some(Arc::new(move |progress| {
                update_video_export_job(&progress_jobs, &progress_pid, progress);
            })),
        );
        let result = crate::proc::with_cancellation(cancel, work).await;
        let mut final_status = {
            let guard = jobs.lock().expect("video export state poisoned");
            let Some(job) = guard.get(&task_pid) else {
                return;
            };
            let mut status = job.status.clone();
            match result {
                Ok(path) => {
                    status.state = "completed".into();
                    status.phase = "completed".into();
                    status.progress = 100;
                    status.error = None;
                    status.path = Some(path);
                }
                Err(AppError::Cancelled) => {
                    status.state = "cancelled".into();
                    status.phase = "cancelled".into();
                    status.error = None;
                }
                Err(error) => {
                    status.state = "failed".into();
                    status.phase = "failed".into();
                    status.error = Some(error.to_string());
                }
            }
            status.updated_at = Some(unix_timestamp_seconds());
            status
        };
        if let Some(job) = jobs
            .lock()
            .expect("video export state poisoned")
            .get_mut(&task_pid)
        {
            job.status = final_status.clone();
        }
        let persisted = final_status.clone();
        if let Err(error) = persist_background_status(
            "save final video export status",
            status_path,
            persisted,
            save_video_export_status,
        )
        .await
        {
            final_status.state = "failed".into();
            final_status.phase = "failed".into();
            final_status.error = Some(format!(
                "video export finished but its recovery status could not be saved: {error}"
            ));
        }
        if let Some(job) = jobs
            .lock()
            .expect("video export state poisoned")
            .get_mut(&task_pid)
        {
            job.status = final_status.clone();
        }
        trace_pipeline_finished(
            "video-export",
            &task_pid,
            &final_status.state,
            final_status.error.as_deref(),
        );
    });
    Ok(status)
}

#[tauri::command]
pub async fn video_export_status(
    pid: String,
    state: tauri::State<'_, VideoExportState>,
) -> AppResult<VideoExportJobStatus> {
    let active = state
        .jobs
        .lock()
        .expect("video export state poisoned")
        .get(&pid)
        .map(|job| (job.status.clone(), job.status_path.clone()));
    if let Some((status, path)) = active {
        persist_background_status(
            "checkpoint video export status",
            path.clone(),
            status.clone(),
            save_video_export_status,
        )
        .await?;
        let latest = state
            .jobs
            .lock()
            .expect("video export state poisoned")
            .get(&pid)
            .map(|job| job.status.clone())
            .unwrap_or_else(|| status.clone());
        if latest.state != status.state
            || latest.phase != status.phase
            || latest.progress != status.progress
            || latest.updated_at != status.updated_at
        {
            persist_background_status(
                "checkpoint latest video export status",
                path,
                latest.clone(),
                save_video_export_status,
            )
            .await?;
        }
        return Ok(latest);
    }
    let status_path = video_export_status_path(&pid, None)?;
    run_blocking("load video export status", move || {
        load_recovered_video_export_status(&status_path).map_err(|error| match error {
            AppError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
                AppError::Schema("no video export job for this project".into())
            }
            other => other,
        })
    })
    .await
}

#[tauri::command]
pub async fn video_export_cancel(
    pid: String,
    state: tauri::State<'_, VideoExportState>,
) -> AppResult<VideoExportJobStatus> {
    let (status, path) = {
        let mut jobs = state.jobs.lock().expect("video export state poisoned");
        let job = jobs
            .get_mut(&pid)
            .ok_or_else(|| AppError::Schema("no video export job for this project".into()))?;
        if job.status.state == "running" {
            job.cancel.store(true, Ordering::Relaxed);
            job.status.state = "cancelling".into();
            job.status.phase = "cancelling".into();
            job.status.updated_at = Some(unix_timestamp_seconds());
        }
        (job.status.clone(), job.status_path.clone())
    };
    persist_background_status(
        "checkpoint video export cancellation",
        path,
        status.clone(),
        save_video_export_status,
    )
    .await?;
    Ok(status)
}

#[tauri::command]
pub async fn export_fcp(pid: String, root: Option<PathBuf>) -> AppResult<String> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("Final Cut export", move || {
        let doc = Doc::load(&dir)?;
        let cuts = load_project_cuts(&dir)?;
        let path = dir.join("export.fcpxml");
        let broll = crate::data::broll::load(&dir)?;
        let titles = crate::data::title::load(&dir)?;
        crate::export::write_fcp_with_broll_titles(
            &doc, &cuts.cuts, &broll, &titles, &path, 1920, 1080,
        )?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
}

#[tauri::command]
pub async fn export_subtitles(pid: String, root: Option<PathBuf>) -> AppResult<Vec<String>> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("subtitle export", move || {
        let doc = Doc::load(&dir)?;
        let cuts = load_project_cuts(&dir)?;
        let paths = [
            dir.join("export.srt"),
            dir.join("export.vtt"),
            dir.join("export.ass"),
            dir.join("export.md"),
        ];
        let style = crate::data::substyle::SubStyle::load(&dir)?;
        let settings = crate::data::export_settings::load(&dir)?;
        let hidden = crate::data::subtitle::load_hidden_checked(&dir)?;
        let caption_doc = crate::data::export_settings::project_caption_doc_with_hidden(
            &doc,
            settings.subtitle_language.as_deref(),
            settings.bilingual_subtitles,
            &hidden,
        )?;
        crate::export::write_srt_with(&caption_doc, &cuts.cuts, &paths[0])?;
        crate::export::write_vtt_with(&caption_doc, &cuts.cuts, &paths[1])?;
        crate::export::write_ass_with_style(
            &caption_doc,
            &cuts.cuts,
            &style,
            &paths[2],
            1920,
            1080,
        )?;
        crate::export::write_md_with_chapters(&doc, &cuts.cuts, &dir, &paths[3])?;
        Ok(paths
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn version_list(pid: String, root: Option<PathBuf>) -> AppResult<VersionHistory> {
    let dir = resolve_project_dir(&pid, root)?;
    run_blocking("version history load", move || {
        let lineage = crate::data::version::Lineage::load(&dir)?;
        Ok(VersionHistory::from(lineage))
    })
    .await
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionHistory {
    pub v: u32,
    pub head: Option<String>,
    pub active_branch: Option<String>,
    pub branches: Vec<crate::data::version::Branch>,
    pub versions: Vec<crate::data::version::VersionNode>,
}

impl From<crate::data::version::Lineage> for VersionHistory {
    fn from(lineage: crate::data::version::Lineage) -> Self {
        Self {
            v: lineage.v,
            head: lineage.head().map(|node| node.id.clone()),
            active_branch: lineage.active_branch,
            branches: lineage.branches,
            versions: lineage.nodes,
        }
    }
}

#[tauri::command]
pub async fn version_commit(
    pid: String,
    name: String,
    note: String,
    root: Option<PathBuf>,
) -> AppResult<String> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("version snapshot", move || {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::Schema("version name cannot be empty".into()));
        }
        let doc = Doc::load(&dir)?;
        let mut lineage = crate::data::version::Lineage::load(&dir)?;
        let branch = lineage
            .active_branch
            .clone()
            .unwrap_or_else(|| "main".into());
        let id = crate::data::version::commit_snapshot(
            &dir,
            &doc,
            &mut lineage,
            &branch,
            name,
            note.trim(),
            crate::data::version::VersionKind::Manual,
        )?;
        crate::data::activity::touch(&dir)?;
        Ok(id)
    })
    .await
}

#[tauri::command]
pub async fn version_restore(pid: String, id: String, root: Option<PathBuf>) -> AppResult<()> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("version restore", move || {
        let doc = Doc::load(&dir)?;
        let mut lineage = crate::data::version::Lineage::load(&dir)?;
        if !working_head_is_committed(&dir, &doc)? {
            let branch = lineage
                .active_branch
                .clone()
                .unwrap_or_else(|| "main".into());
            crate::data::version::commit_snapshot(
                &dir,
                &doc,
                &mut lineage,
                &branch,
                &format!("before restore {id}"),
                "automatic recovery snapshot",
                crate::data::version::VersionKind::Auto,
            )?;
        }
        crate::data::version::restore_snapshot(&dir, &mut lineage, &id)?;
        crate::data::activity::touch(&dir)?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn branch_create(pid: String, name: String, root: Option<PathBuf>) -> AppResult<String> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("branch create", move || {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::Schema("branch name cannot be empty".into()));
        }
        let doc = Doc::load(&dir)?;
        let mut lineage = crate::data::version::Lineage::load(&dir)?;
        if !working_head_is_committed(&dir, &doc)? {
            let current_branch = lineage
                .active_branch
                .clone()
                .unwrap_or_else(|| "main".into());
            crate::data::version::commit_snapshot(
                &dir,
                &doc,
                &mut lineage,
                &current_branch,
                "before branch",
                "automatic recovery snapshot",
                crate::data::version::VersionKind::Auto,
            )?;
        }
        let id = crate::data::version::create_branch(&dir, &mut lineage, name, "")?;
        crate::data::version::switch_branch(&dir, &mut lineage, &id)?;
        crate::data::activity::touch(&dir)?;
        Ok(id)
    })
    .await
}

#[tauri::command]
pub async fn branch_switch(pid: String, id: String, root: Option<PathBuf>) -> AppResult<()> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("branch switch", move || {
        let doc = Doc::load(&dir)?;
        if !working_head_is_committed(&dir, &doc)? {
            return Err(AppError::Schema(
                "save the current project as a version before switching branches".into(),
            ));
        }
        let mut lineage = crate::data::version::Lineage::load(&dir)?;
        crate::data::version::switch_branch(&dir, &mut lineage, &id)?;
        crate::data::activity::touch(&dir)?;
        Ok(())
    })
    .await
}

// ---- line editing, style, and cloud configuration ----

#[tauri::command]
pub async fn split_line(
    pid: String,
    id: String,
    at: usize,
    root: Option<PathBuf>,
) -> AppResult<bool> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("subtitle split", move || {
        crate::data::edit_history::record(
            &dir,
            "Split subtitle",
            || {
                let mut doc = Doc::load(&dir)?;
                let ok = crate::data::edit::split_sentence(&mut doc, &id, at);
                if ok {
                    doc.meta.updated_at = chrono::Utc::now();
                    doc.save(&dir)?;
                }
                Ok(ok)
            },
            |changed| *changed,
        )
    })
    .await
}

#[tauri::command]
pub async fn merge_lines(
    pid: String,
    id1: String,
    id2: String,
    root: Option<PathBuf>,
) -> AppResult<bool> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("subtitle merge", move || {
        crate::data::edit_history::record(
            &dir,
            "Merge subtitles",
            || {
                let mut doc = Doc::load(&dir)?;
                let ok = crate::data::edit::merge_sentences(&mut doc, &id1, &id2);
                if ok {
                    doc.meta.updated_at = chrono::Utc::now();
                    doc.save(&dir)?;
                }
                Ok(ok)
            },
            |changed| *changed,
        )
    })
    .await
}

#[cfg(test)]
mod recording_tests {
    use super::recording_output;

    #[test]
    fn recording_output_rejects_path_traversal_and_blank_ids() {
        let root = std::path::PathBuf::from("/tmp/lumen-cut-recording-test");
        for invalid in ["", " ", ".", "..", "../escape", "nested/id", r"nested\id"] {
            assert!(recording_output(invalid, Some(root.clone())).is_err());
        }
        assert_eq!(
            recording_output("recording-20260720", Some(root.clone())).unwrap(),
            root.join("recording-20260720/audio.wav")
        );
    }
}

#[cfg(test)]
mod transcription_status_tests {
    use super::{
        load_recovered_transcription_status, save_transcription_status, TranscriptionJobStatus,
    };

    #[test]
    fn interrupted_job_becomes_a_retryable_failure() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("job.json");
        save_transcription_status(
            &path,
            &TranscriptionJobStatus {
                pid: "project-1".into(),
                state: "running".into(),
                phase: "transcribing".into(),
                progress: 52,
                current: Some(3),
                total: Some(8),
                device: Some("mlx-metal".into()),
                elapsed_seconds: Some(20.0),
                cpu_percent: Some(78),
                peak_memory_mb: Some(2800),
                memory_limit_mb: Some(6144),
                mlx_active_memory_mb: Some(1900),
                mlx_cache_memory_mb: Some(240),
                started_at: Some(10),
                updated_at: Some(20),
                error: None,
            },
        )
        .unwrap();

        let recovered = load_recovered_transcription_status(&path).unwrap();
        assert_eq!(recovered.state, "failed");
        assert_eq!(recovered.phase, "failed");
        assert_eq!(recovered.progress, 52);
        assert!(recovered.updated_at.unwrap() > 20);
        assert!(recovered.error.unwrap().contains("retry"));

        let persisted = load_recovered_transcription_status(&path).unwrap();
        assert_eq!(persisted.state, "failed");
    }
}

#[cfg(test)]
mod speaker_analysis_status_tests {
    use super::{
        load_recovered_speaker_analysis_status, save_speaker_analysis_status,
        SpeakerAnalysisJobStatus,
    };

    #[test]
    fn interrupted_speaker_analysis_becomes_a_retryable_failure() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("speaker.json");
        save_speaker_analysis_status(
            &path,
            &SpeakerAnalysisJobStatus {
                pid: "project-1".into(),
                state: "running".into(),
                phase: "embedding".into(),
                progress: 81,
                current: Some(3),
                total: Some(5),
                device: Some("mps".into()),
                elapsed_seconds: Some(12.4),
                cpu_percent: Some(87),
                peak_memory_mb: Some(2431),
                memory_limit_mb: Some(6144),
                started_at: Some(10),
                updated_at: Some(20),
                error: None,
                preview: None,
            },
        )
        .unwrap();

        let recovered = load_recovered_speaker_analysis_status(&path).unwrap();
        assert_eq!(recovered.state, "failed");
        assert_eq!(recovered.phase, "failed");
        assert_eq!(recovered.progress, 81);
        assert!(recovered.updated_at.unwrap() > 20);
        assert!(recovered.error.unwrap().contains("start it again"));

        let persisted = load_recovered_speaker_analysis_status(&path).unwrap();
        assert_eq!(persisted.state, "failed");
    }
}

#[cfg(test)]
mod background_status_persistence_tests {
    use super::{advance_progress, persist_background_status};
    use crate::error::AppError;

    #[test]
    fn pipeline_progress_is_bounded_and_never_moves_backwards() {
        assert_eq!(advance_progress(45, 20), 45);
        assert_eq!(advance_progress(45, 72), 72);
        assert_eq!(advance_progress(99, 255), 100);
    }

    #[tokio::test]
    async fn a_final_status_write_failure_is_never_reported_as_success() {
        let temp = tempfile::tempdir().unwrap();
        let error = persist_background_status(
            "persist test status",
            temp.path().join("status.json"),
            "completed".to_string(),
            |_path, _status| Err(AppError::Schema("disk unavailable".into())),
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("disk unavailable"));
    }
}

#[cfg(test)]
mod video_export_status_tests {
    use super::{
        load_recovered_video_export_status, save_video_export_status, VideoExportJobStatus,
    };

    #[test]
    fn interrupted_video_export_becomes_a_retryable_failure() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("export.json");
        save_video_export_status(
            &path,
            &VideoExportJobStatus {
                pid: "project-1".into(),
                mode: "fast".into(),
                settings: Default::default(),
                state: "running".into(),
                phase: "encoding".into(),
                progress: 47,
                current_seconds: Some(14.2),
                total_seconds: Some(30.0),
                encoder: Some("h264_videotoolbox".into()),
                started_at: Some(10),
                updated_at: Some(20),
                error: None,
                path: None,
            },
        )
        .unwrap();

        let recovered = load_recovered_video_export_status(&path).unwrap();
        assert_eq!(recovered.state, "failed");
        assert_eq!(recovered.phase, "failed");
        assert_eq!(recovered.progress, 47);
        assert!(recovered.updated_at.unwrap() > 20);
        assert!(recovered.error.unwrap().contains("start it again"));
    }

    #[test]
    fn legacy_video_export_status_recovers_with_backward_compatible_settings() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("legacy-export.json");
        std::fs::write(
            &path,
            r#"{
                "pid":"project-1",
                "mode":"quality",
                "state":"completed",
                "phase":"completed",
                "progress":100,
                "currentSeconds":30.0,
                "totalSeconds":30.0,
                "encoder":"libx264",
                "error":null,
                "path":"/tmp/export.mp4"
            }"#,
        )
        .unwrap();

        let recovered = load_recovered_video_export_status(&path).unwrap();
        assert_eq!(
            recovered.settings.encoding_speed,
            crate::data::export_settings::ExportEncodingSpeed::Quality
        );
        assert_eq!(recovered.mode, "quality");
    }
}

#[cfg(test)]
mod setup_job_status_tests {
    use super::{load_recovered_setup_status, save_setup_status, SetupJobStatus};

    #[test]
    fn interrupted_setup_becomes_a_retryable_failure() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("setup.json");
        save_setup_status(
            &path,
            &SetupJobStatus {
                kind: "asr-models".into(),
                state: "running".into(),
                phase: "downloading".into(),
                started_at: Some(10),
                updated_at: Some(20),
                error: None,
            },
        )
        .unwrap();
        let recovered = load_recovered_setup_status(&path).unwrap();
        assert_eq!(recovered.state, "failed");
        assert!(recovered.updated_at.unwrap() > 20);
        assert!(recovered.error.unwrap().contains("start it again"));
    }
}

#[cfg(test)]
mod broll_preview_status_tests {
    use super::{
        load_recovered_broll_preview_status, save_broll_preview_status,
        validated_broll_preview_paths, BrollPreviewJobStatus,
    };

    #[test]
    fn interrupted_broll_preview_becomes_a_retryable_failure() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("preview.json");
        save_broll_preview_status(
            &path,
            &BrollPreviewJobStatus {
                pid: "project-1".into(),
                state: "running".into(),
                phase: "encoding".into(),
                progress: 63,
                current: Some(20.0),
                total: Some(30.0),
                encoder: Some("h264_videotoolbox".into()),
                started_at: Some(10),
                updated_at: Some(20),
                error: None,
                paths: vec![],
            },
        )
        .unwrap();
        let recovered = load_recovered_broll_preview_status(&path).unwrap();
        assert_eq!(recovered.state, "failed");
        assert_eq!(recovered.progress, 63);
        assert!(recovered.updated_at.unwrap() > 20);
        assert!(recovered.error.unwrap().contains("start it again"));
    }

    #[test]
    fn recovered_preview_scope_accepts_only_generated_project_images() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project-1");
        std::fs::create_dir_all(&project).unwrap();
        let valid = project.join("broll-preview-1.0-0.png");
        let unrelated = project.join("private.png");
        let outside = temp.path().join("broll-preview-outside.png");
        std::fs::write(&valid, b"preview").unwrap();
        std::fs::write(&unrelated, b"private").unwrap();
        std::fs::write(&outside, b"outside").unwrap();

        let paths = validated_broll_preview_paths(
            project,
            vec![
                valid.to_string_lossy().into_owned(),
                unrelated.to_string_lossy().into_owned(),
                outside.to_string_lossy().into_owned(),
                temp.path()
                    .join("missing.png")
                    .to_string_lossy()
                    .into_owned(),
            ],
        )
        .unwrap();

        assert_eq!(paths, vec![std::fs::canonicalize(valid).unwrap()]);
    }
}

#[cfg(test)]
mod main_thread_safety_tests {
    #[test]
    fn tauri_commands_are_async_to_keep_the_appkit_thread_responsive() {
        let source = include_str!("commands.rs");
        let mut command_attribute_seen = false;
        let mut synchronous_commands = Vec::new();

        for line in source.lines().map(str::trim) {
            if line == "#[tauri::command]" {
                command_attribute_seen = true;
                continue;
            }
            if !command_attribute_seen || line.is_empty() || line.starts_with("//") {
                continue;
            }
            if let Some(signature) = line.strip_prefix("pub fn ") {
                synchronous_commands.push(
                    signature
                        .split(['(', '<'])
                        .next()
                        .unwrap_or(signature)
                        .to_string(),
                );
            }
            if line.starts_with("pub ") {
                command_attribute_seen = false;
            }
        }

        assert!(
            synchronous_commands.is_empty(),
            "Tauri commands run on the AppKit thread unless declared async; synchronous commands: {}",
            synchronous_commands.join(", ")
        );
    }

    #[test]
    fn native_file_dialog_never_uses_a_blocking_api() {
        let source = include_str!("commands.rs");
        let forbidden_call = [".blocking_", "pick_file()"].concat();
        assert!(
            !source.contains(&forbidden_call),
            "the native file dialog must use its callback API so AppKit can keep pumping events"
        );
    }

    #[test]
    fn ipc_commands_never_sleep_a_worker_thread() {
        let source = include_str!("commands.rs");
        let forbidden_call = ["std::thread::", "sleep"].concat();
        assert!(
            !source.contains(&forbidden_call),
            "IPC work must use async timers instead of putting an executor thread to sleep"
        );
    }

    #[test]
    fn broll_preview_uses_the_validated_project_path() {
        let source = include_str!("commands.rs");
        let start = source.find("pub async fn broll_preview(").unwrap();
        let rest = &source[start..];
        let end = rest.find("pub struct DiarizeResult").unwrap();
        assert!(rest[..end].contains("resolve_project_dir(&pid, root)?"));
        assert!(!rest[..end].contains("resolve_project_root(root).join(&pid)"));
    }
}

#[tauri::command]
pub async fn style_get(
    pid: String,
    root: Option<PathBuf>,
) -> AppResult<crate::data::substyle::SubStyle> {
    let dir = resolve_project_dir(&pid, root)?;
    run_blocking("subtitle style load", move || {
        crate::data::substyle::SubStyle::load(&dir)
    })
    .await
}

#[tauri::command]
pub async fn style_set(
    pid: String,
    style: crate::data::substyle::SubStyle,
    root: Option<PathBuf>,
) -> AppResult<()> {
    let dir = resolve_project_dir(&pid, root)?;
    let _mutation = lock_project_mutation(&dir).await;
    run_blocking("subtitle style save", move || {
        let nonempty = |value: &str| !value.trim().is_empty();
        let ass_color = |value: &str| {
            value
                .strip_prefix("&H")
                .is_some_and(|hex| hex.len() == 8 && hex.chars().all(|ch| ch.is_ascii_hexdigit()))
        };
        if !nonempty(&style.name)
            || !nonempty(&style.fontname)
            || !(8..=200).contains(&style.fontsize)
            || !(1..=9).contains(&style.alignment)
            || style.outline > 20
            || style.shadow > 20
            || style.margin_l > 2_000
            || style.margin_r > 2_000
            || style.margin_v > 2_000
            || !ass_color(&style.primary_colour)
            || !ass_color(&style.outline_colour)
        {
            return Err(AppError::Schema(
                "subtitle style contains an invalid font, colour, alignment, effect, or margin"
                    .into(),
            ));
        }
        let previous = crate::data::substyle::SubStyle::load(&dir)?;
        crate::data::edit_history::record(
            &dir,
            "Change subtitle style",
            || crate::data::storage::write_json(&dir.join("style.json"), &style),
            |_| previous != style,
        )
    })
    .await
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicModelConfig {
    #[serde(flatten)]
    config: crate::data::modelconfig::ModelConfig,
    hf_token_set: bool,
    llm_api_key_set: bool,
    asr_cloud_api_key_set: bool,
}

fn redact_model_config(mut config: crate::data::modelconfig::ModelConfig) -> PublicModelConfig {
    let hf_token_set = !config.hf_token.trim().is_empty();
    let llm_api_key_set = !config.llm_api_key.trim().is_empty();
    let asr_cloud_api_key_set = !config.asr_cloud_api_key.trim().is_empty();
    config.hf_token.clear();
    config.llm_api_key.clear();
    config.asr_cloud_api_key.clear();
    PublicModelConfig {
        config,
        hf_token_set,
        llm_api_key_set,
        asr_cloud_api_key_set,
    }
}

#[tauri::command]
pub async fn config_show() -> AppResult<PublicModelConfig> {
    run_blocking("settings load", || {
        Ok(redact_model_config(crate::data::modelconfig::load()))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_constant_is_nonempty() {
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn ai_provider_preflight_rejects_missing_and_invalid_endpoints() {
        let mut config = crate::data::modelconfig::ModelConfig {
            llm_model: "test-model".into(),
            ..Default::default()
        };
        let missing = validate_ai_provider_preflight(&config).unwrap_err();
        assert!(missing.to_string().contains("not configured"));

        config.llm_endpoint = "file:///tmp/provider".into();
        let invalid = validate_ai_provider_preflight(&config).unwrap_err();
        assert!(invalid.to_string().contains("http or https"));

        config.llm_endpoint = "http://127.0.0.1:11434/v1/chat/completions".into();
        validate_ai_provider_preflight(&config).unwrap();
    }

    #[test]
    fn cloud_transcription_preflight_requires_complete_credentials() {
        let mut config = crate::data::modelconfig::ModelConfig {
            asr_engine: crate::data::modelconfig::AsrEngine::OpenaiCompatible,
            ..Default::default()
        };
        let error = validate_transcription_preflight(&config, None).unwrap_err();
        assert!(error.to_string().contains("endpoint, API key, and model"));

        config.asr_cloud_api_key = "secret".into();
        validate_transcription_preflight(&config, None).unwrap();
    }

    #[test]
    fn timeline_contact_sheet_uses_fast_seeks_instead_of_decoding_the_full_video() {
        let args = contact_sheet_args("/tmp/input.mp4", 120.0, "/tmp/sheet.jpg");
        assert_eq!(args.iter().filter(|arg| arg.as_str() == "-ss").count(), 12);
        assert_eq!(args.iter().filter(|arg| arg.as_str() == "-i").count(), 12);
        assert!(args.iter().any(|arg| arg.contains("hstack=inputs=12")));
        assert!(!args.iter().any(|arg| arg.starts_with("fps=")));
    }

    #[test]
    fn broll_quick_preview_chooses_one_kept_midpoint_per_placement() {
        let now = chrono::Utc::now();
        let doc = Doc {
            id: "p1".into(),
            schema: 1,
            media: MediaRef {
                path: "/tmp/source.mp4".into(),
                duration_seconds: 10.0,
                sample_rate: None,
                channels: None,
            },
            meta: Meta {
                title: "Preview".into(),
                description: String::new(),
                language: None,
                created_at: now,
                updated_at: now,
            },
            paragraphs: vec![],
            translations: Default::default(),
        };
        let placements = vec![crate::data::broll::BrollPlacement {
            id: "br-1".into(),
            file: "/tmp/asset.png".into(),
            start: 2.0,
            end: 6.0,
            mode: crate::data::broll::PlacementMode::Pip,
            rect: None,
            fit: crate::data::broll::FitMode::Cover,
            background: crate::data::broll::BackgroundMode::Black,
            source_start: 0.0,
            radius: 0,
            name: None,
        }];

        assert_eq!(
            broll_preview_points(&doc, &ClipCuts::default(), &placements),
            vec![(4.0, 4.0, 0)]
        );
    }

    #[test]
    fn transcription_normalization_preserves_detected_language_and_repairs_timing() {
        let mut doc: Doc = crate::asr::AsrOutV1 {
            schema_version: 1,
            language: Some("English".into()),
            duration_seconds: 1.0,
            paragraphs: vec![crate::asr::AsrParagraph {
                speaker: None,
                sentences: vec![crate::asr::AsrSentence {
                    text: "hello world".into(),
                    words: vec![
                        crate::asr::AsrWord {
                            text: "hello".into(),
                            start: 0.0,
                            end: 0.0,
                        },
                        crate::asr::AsrWord {
                            text: "world".into(),
                            start: 0.0,
                            end: 0.2,
                        },
                    ],
                }],
            }],
        }
        .into();
        normalize_transcription_doc(&mut doc, None);
        assert_eq!(doc.meta.language.as_deref(), Some("English"));
        let words = &doc.paragraphs[0].sentences[0].words;
        assert!(words[0].end > words[0].start);
        assert!(words[1].start >= words[0].end - crate::pipeline::timing::JITTER);
        assert!(words[1].end - words[1].start >= crate::pipeline::timing::MIN_DUR);

        normalize_transcription_doc(&mut doc, Some("Chinese".into()));
        assert_eq!(doc.meta.language.as_deref(), Some("Chinese"));
    }

    #[test]
    fn retranscription_creates_recovery_and_invalidates_only_cue_derived_state() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("p1");
        let now = chrono::Utc::now();
        let sentence = |id: &str, word_id: &str, text: &str| crate::data::doc::Sentence {
            id: id.into(),
            text: text.into(),
            words: vec![crate::data::doc::Word {
                id: word_id.into(),
                text: text.into(),
                start: 0.0,
                end: 1.0,
            }],
        };
        let mut old = Doc {
            id: "p1".into(),
            schema: 1,
            media: MediaRef {
                path: tmp.path().join("source.mp4"),
                duration_seconds: 10.0,
                sample_rate: Some(48_000),
                channels: Some(2),
            },
            meta: Meta {
                title: "Interview".into(),
                description: String::new(),
                language: Some("en".into()),
                created_at: now,
                updated_at: now,
            },
            paragraphs: vec![crate::data::doc::Paragraph {
                id: 1,
                speaker: Some("Alice".into()),
                sentences: vec![sentence("old-s1", "old-w1", "Old")],
            }],
            translations: Default::default(),
        };
        crate::data::subtitle::set_translation(&mut old, "zh", "old-s1", "旧");
        old.save(&project).unwrap();
        std::fs::write(project.join("hidden.json"), r#"{"hidden":["old-s1"]}"#).unwrap();
        std::fs::write(project.join("cuts.json"), r#"{"cuts":[]}"#).unwrap();
        std::fs::write(project.join("chapters.json"), "[]").unwrap();
        std::fs::create_dir_all(project.join("ai/translate/pending")).unwrap();
        std::fs::write(project.join("ai/translate/task.json"), "{}").unwrap();
        std::fs::create_dir_all(project.join(".lumen-cut/edit-history")).unwrap();
        std::fs::write(project.join(".lumen-cut/edit-history/history.json"), "{}").unwrap();
        for (name, contents) in [
            ("style.json", r#"{"keep":"style"}"#),
            ("titles.json", r#"{"keep":"titles"}"#),
            ("audio-mix.json", r#"{"keep":"audio"}"#),
            ("broll.json", r#"{"keep":"broll"}"#),
        ] {
            std::fs::write(project.join(name), contents).unwrap();
        }

        let fresh = Doc {
            paragraphs: vec![crate::data::doc::Paragraph {
                id: 1,
                speaker: None,
                sentences: vec![sentence("new-s1", "new-w1", "Fresh")],
            }],
            translations: Default::default(),
            ..old.clone()
        };
        let result = persist_transcription_result(fresh, project.clone()).unwrap();
        assert_eq!(result.word_count, 1);

        let saved = Doc::load(&project).unwrap();
        assert_eq!(saved.paragraphs[0].sentences[0].id, "new-s1");
        assert!(saved.paragraphs[0].speaker.is_none());
        assert!(saved.translations.is_empty());
        for name in ["hidden.json", "cuts.json", "chapters.json"] {
            assert!(!project.join(name).exists(), "{name} should be invalidated");
        }
        assert!(!project.join("ai").exists());
        assert!(!project.join(".lumen-cut/edit-history").exists());
        for (name, expected) in [
            ("style.json", r#"{"keep":"style"}"#),
            ("titles.json", r#"{"keep":"titles"}"#),
            ("audio-mix.json", r#"{"keep":"audio"}"#),
            ("broll.json", r#"{"keep":"broll"}"#),
        ] {
            assert_eq!(
                std::fs::read_to_string(project.join(name)).unwrap(),
                expected
            );
        }

        let lineage = crate::data::version::Lineage::load(&project).unwrap();
        let recovery = lineage.head().unwrap();
        assert_eq!(recovery.name, "Before retranscription");
        assert_eq!(recovery.kind, crate::data::version::VersionKind::Auto);
        let restored = Doc::load(
            crate::data::version::snapshot_path(&project, &recovery.id)
                .parent()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(restored.paragraphs[0].speaker.as_deref(), Some("Alice"));
        assert_eq!(restored.translations["zh"]["old-s1"].text, "旧");
        assert!(crate::data::version::snapshot_path(&project, &recovery.id)
            .parent()
            .unwrap()
            .join("cuts.json")
            .exists());
        assert!(crate::data::version::snapshot_path(&project, &recovery.id)
            .parent()
            .unwrap()
            .join("chapters.json")
            .exists());
    }

    #[tokio::test]
    async fn greet_returns_ready() {
        let g = greet().await;
        assert_eq!(g.msg, "lumen-cut ready");
        assert_eq!(g.version, VERSION);
    }

    #[test]
    fn gui_project_root_honors_explicit_override() {
        let tmp = tempfile::tempdir().unwrap();
        let root = resolve_project_root(Some(tmp.path().to_path_buf()));
        assert_eq!(root, tmp.path());
    }

    #[test]
    fn project_dir_accepts_only_one_safe_path_component() {
        let tmp = tempfile::tempdir().unwrap();
        let root = Some(tmp.path().to_path_buf());
        assert_eq!(
            resolve_project_dir("project-01", root.clone()).unwrap(),
            tmp.path().join("project-01")
        );
        for invalid in [
            "",
            ".",
            "..",
            "../escape",
            "nested/project",
            r"nested\project",
        ] {
            assert!(
                resolve_project_dir(invalid, root.clone()).is_err(),
                "{invalid:?} should be rejected"
            );
        }
    }

    #[tokio::test]
    async fn project_reveal_rejects_a_regular_file_as_project_directory() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("p1"), "not a project directory").unwrap();
        assert!(project_reveal("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn project_metadata_update_is_trimmed_and_persisted() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("p1");
        let created_at = chrono::Utc::now();
        Doc {
            id: "p1".into(),
            schema: 1,
            media: MediaRef {
                path: tmp.path().join("source.mp4"),
                duration_seconds: 12.5,
                sample_rate: Some(48_000),
                channels: Some(2),
            },
            meta: Meta {
                title: "Draft".into(),
                description: String::new(),
                language: None,
                created_at,
                updated_at: created_at,
            },
            paragraphs: vec![],
            translations: Default::default(),
        }
        .save(&project)
        .unwrap();

        let updated = project_update_meta(
            "p1".into(),
            "  Interview final  ".into(),
            "  Delivery notes  ".into(),
            Some(" en ".into()),
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap();
        assert_eq!(updated.meta.title, "Interview final");
        assert_eq!(updated.meta.description, "Delivery notes");
        assert_eq!(updated.meta.language.as_deref(), Some("en"));

        let reloaded = Doc::load(&project).unwrap();
        assert_eq!(reloaded.meta, updated.meta);
        assert!(project_update_meta(
            "p1".into(),
            "   ".into(),
            String::new(),
            None,
            Some(tmp.path().to_path_buf()),
        )
        .await
        .is_err());
        assert_eq!(
            edit_history_status("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .undo_label
                .as_deref(),
            Some("Edit project details")
        );
        assert!(
            edit_undo("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .changed
        );
        assert_eq!(Doc::load(&project).unwrap().meta.title, "Draft");
    }

    #[tokio::test]
    async fn project_media_status_reports_a_moved_file_without_losing_the_project() {
        let tmp = tempfile::tempdir().unwrap();
        let media = tmp.path().join("source.mp4");
        let project = tmp.path().join("p1");
        let now = chrono::Utc::now();
        let doc = Doc {
            id: "p1".into(),
            schema: 1,
            media: MediaRef {
                path: media.clone(),
                duration_seconds: 12.5,
                sample_rate: Some(48_000),
                channels: Some(2),
            },
            meta: Meta {
                title: "Interview".into(),
                description: String::new(),
                language: None,
                created_at: now,
                updated_at: now,
            },
            paragraphs: vec![],
            translations: Default::default(),
        };
        doc.save(&project).unwrap();

        let missing = project_media_status("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert!(!missing.available);
        assert!(missing.issue.unwrap().contains("missing"));
        assert_eq!(Doc::load(&project).unwrap().id, "p1");

        std::fs::write(&media, b"media").unwrap();
        let restored = project_media_status("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert!(restored.available);
        assert_eq!(restored.file_size, Some(5));
    }

    #[tokio::test]
    async fn project_media_status_suggests_only_one_nearby_same_named_file() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("p1");
        let missing = tmp.path().join("detached").join("source.mp4");
        let now = chrono::Utc::now();
        Doc {
            id: "p1".into(),
            schema: 1,
            media: MediaRef {
                path: missing,
                duration_seconds: 12.5,
                sample_rate: Some(48_000),
                channels: Some(2),
            },
            meta: Meta {
                title: "Interview".into(),
                description: String::new(),
                language: None,
                created_at: now,
                updated_at: now,
            },
            paragraphs: vec![],
            translations: Default::default(),
        }
        .save(&project)
        .unwrap();
        let nearby = tmp.path().join("recovered").join("source.mp4");
        std::fs::create_dir_all(nearby.parent().unwrap()).unwrap();
        std::fs::write(&nearby, b"media").unwrap();

        let status = project_media_status("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert_eq!(
            status.suggested_path,
            Some(std::fs::canonicalize(&nearby).unwrap())
        );

        let duplicate = tmp.path().join("another").join("source.mp4");
        std::fs::create_dir_all(duplicate.parent().unwrap()).unwrap();
        std::fs::write(duplicate, b"media").unwrap();
        let ambiguous = project_media_status("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert_eq!(ambiguous.suggested_path, None);
    }

    #[tokio::test]
    async fn manual_translation_and_style_changes_are_durable_and_undoable() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("p1");
        let now = chrono::Utc::now();
        Doc {
            id: "p1".into(),
            schema: 1,
            media: MediaRef {
                path: tmp.path().join("source.mp4"),
                duration_seconds: 2.0,
                sample_rate: Some(48_000),
                channels: Some(2),
            },
            meta: Meta {
                title: "Interview".into(),
                description: String::new(),
                language: Some("en".into()),
                created_at: now,
                updated_at: now,
            },
            paragraphs: vec![crate::data::doc::Paragraph {
                id: 1,
                speaker: None,
                sentences: vec![crate::data::doc::Sentence {
                    id: "s1".into(),
                    text: "Hello".into(),
                    words: vec![crate::data::doc::Word {
                        id: "w1".into(),
                        text: "Hello".into(),
                        start: 0.0,
                        end: 1.0,
                    }],
                }],
            }],
            translations: Default::default(),
        }
        .save(&project)
        .unwrap();

        assert!(translation_set(
            "p1".into(),
            "zh".into(),
            "s1".into(),
            "你好".into(),
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap());
        assert_eq!(
            Doc::load(&project).unwrap().translations["zh"]["s1"].text,
            "你好"
        );
        assert!(
            edit_undo("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .changed
        );
        assert!(Doc::load(&project).unwrap().translations.is_empty());
        assert!(translation_set(
            "p1".into(),
            "zh-Hans".into(),
            "s1".into(),
            "你好".into(),
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap());

        let style = crate::data::substyle::SubStyle {
            name: "Creator".into(),
            fontname: "Arial".into(),
            fontsize: 58,
            bold: true,
            ..Default::default()
        };
        style_set("p1".into(), style, Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        export_settings_set(
            "p1".into(),
            crate::data::export_settings::VideoExportSettings {
                subtitle_language: Some("zh-Hans".into()),
                bilingual_subtitles: true,
                ..Default::default()
            },
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap();
        assert!(project.join("style.json").exists());
        export_subtitles("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        let exported_ass = std::fs::read_to_string(project.join("export.ass")).unwrap();
        assert!(exported_ass.contains("Style: Default,Arial,58,"));
        assert!(exported_ass.contains(",-1,0,0,0,100,100"));
        assert!(exported_ass.contains("Hello\\N你好"));
        assert!(
            edit_undo("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .changed
        );
        assert!(!project.join("style.json").exists());
    }

    #[tokio::test]
    async fn transcript_and_translation_batch_updates_are_atomic_and_undoable() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("p1");
        let now = chrono::Utc::now();
        Doc {
            id: "p1".into(),
            schema: 1,
            media: MediaRef {
                path: tmp.path().join("source.mp4"),
                duration_seconds: 2.0,
                sample_rate: Some(48_000),
                channels: Some(2),
            },
            meta: Meta {
                title: "Interview".into(),
                description: String::new(),
                language: Some("en".into()),
                created_at: now,
                updated_at: now,
            },
            paragraphs: vec![crate::data::doc::Paragraph {
                id: 1,
                speaker: None,
                sentences: vec![
                    crate::data::doc::Sentence {
                        id: "s1".into(),
                        text: "Hello".into(),
                        words: vec![crate::data::doc::Word {
                            id: "w1".into(),
                            text: "Hello".into(),
                            start: 0.0,
                            end: 1.0,
                        }],
                    },
                    crate::data::doc::Sentence {
                        id: "s2".into(),
                        text: "World".into(),
                        words: vec![crate::data::doc::Word {
                            id: "w2".into(),
                            text: "World".into(),
                            start: 1.0,
                            end: 2.0,
                        }],
                    },
                ],
            }],
            translations: Default::default(),
        }
        .save(&project)
        .unwrap();

        let result = subtitle_update_many(
            "p1".into(),
            vec![
                SubtitleUpdate {
                    id: "s1".into(),
                    text: "Hello there".into(),
                },
                SubtitleUpdate {
                    id: "s2".into(),
                    text: "World again".into(),
                },
            ],
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap();
        assert_eq!(result.changed, 2);
        assert_eq!(result.sentences.len(), 2);
        assert_eq!(result.sentences[0].text, "Hello there");
        let saved = Doc::load(&project).unwrap();
        assert_eq!(saved.paragraphs[0].sentences[0].text, "Hello there");
        assert_eq!(saved.paragraphs[0].sentences[1].text, "World again");
        assert_eq!(
            result.sentences[0].words,
            saved.paragraphs[0].sentences[0].words
        );
        assert!(
            edit_undo("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .changed
        );
        assert_eq!(
            Doc::load(&project).unwrap().paragraphs[0].sentences[0].text,
            "Hello"
        );

        assert!(subtitle_timing_set(
            "p1".into(),
            "s1".into(),
            0.2,
            0.8,
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap());
        let retimed = Doc::load(&project).unwrap();
        assert_eq!(
            (
                retimed.paragraphs[0].sentences[0].words[0].start,
                retimed.paragraphs[0].sentences[0].words[0].end,
            ),
            (0.2, 0.8)
        );
        assert!(crate::export::to_srt(&retimed).contains("00:00:00,200 --> 00:00:00,800"));
        assert!(
            edit_undo("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .changed
        );
        let restored = Doc::load(&project).unwrap();
        assert_eq!(
            (
                restored.paragraphs[0].sentences[0].words[0].start,
                restored.paragraphs[0].sentences[0].words[0].end,
            ),
            (0.0, 1.0)
        );

        let error = subtitle_update_many(
            "p1".into(),
            vec![
                SubtitleUpdate {
                    id: "s1".into(),
                    text: "Must not persist".into(),
                },
                SubtitleUpdate {
                    id: "missing".into(),
                    text: "Missing".into(),
                },
            ],
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("missing"));
        assert_eq!(
            Doc::load(&project).unwrap().paragraphs[0].sentences[0].text,
            "Hello"
        );

        assert!(chapter_set_many(
            "p1".into(),
            vec![
                crate::data::chapter::Chapter {
                    title: "Introduction".into(),
                    start_seg: "s1".into(),
                },
                crate::data::chapter::Chapter {
                    title: "Main topic".into(),
                    start_seg: "s2".into(),
                },
            ],
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap());
        let rows = chapter_list("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].start, 0.0);
        assert_eq!(rows[0].end, 1.0);
        assert_eq!(rows[1].preview, "World");
        assert!(
            edit_undo("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .changed
        );
        assert!(!project.join("chapters.json").exists());
        let native: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(project.join("doc.json")).unwrap())
                .unwrap();
        assert!(native.get("chapters").is_none());
        let error = chapter_set_many(
            "p1".into(),
            vec![
                crate::data::chapter::Chapter {
                    title: "Introduction".into(),
                    start_seg: "s1".into(),
                },
                crate::data::chapter::Chapter {
                    title: "Missing".into(),
                    start_seg: "missing".into(),
                },
            ],
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("missing"));
        assert!(!project.join("chapters.json").exists());

        let changed = translation_set_many(
            "p1".into(),
            "zh".into(),
            vec![
                TranslationUpdate {
                    id: "s1".into(),
                    text: "你好".into(),
                },
                TranslationUpdate {
                    id: "s2".into(),
                    text: "世界".into(),
                },
            ],
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap();
        assert_eq!(changed, 2);
        let saved = Doc::load(&project).unwrap();
        assert_eq!(saved.translations["zh"]["s1"].text, "你好");
        assert_eq!(saved.translations["zh"]["s2"].text, "世界");

        assert!(
            edit_undo("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .changed
        );
        assert!(Doc::load(&project).unwrap().translations.is_empty());

        let error = translation_set_many(
            "p1".into(),
            "zh".into(),
            vec![
                TranslationUpdate {
                    id: "s1".into(),
                    text: "不应保存".into(),
                },
                TranslationUpdate {
                    id: "missing".into(),
                    text: "缺失".into(),
                },
            ],
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("missing"));
        assert!(Doc::load(&project).unwrap().translations.is_empty());
    }

    #[tokio::test]
    async fn export_preflight_uses_the_same_caption_asset_and_encoder_rules_as_export() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("p1");
        let media = tmp.path().join("source.mp4");
        crate::proc::run(
            "ffmpeg",
            &[
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=320x180:d=0.4",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                &media.display().to_string(),
            ],
        )
        .await
        .unwrap();
        let now = chrono::Utc::now();
        Doc {
            id: "p1".into(),
            schema: 1,
            media: MediaRef {
                path: media,
                duration_seconds: 0.4,
                sample_rate: None,
                channels: None,
            },
            meta: Meta {
                title: "Preflight".into(),
                description: String::new(),
                language: Some("en".into()),
                created_at: now,
                updated_at: now,
            },
            paragraphs: vec![crate::data::doc::Paragraph {
                id: 1,
                speaker: None,
                sentences: vec![crate::data::doc::Sentence {
                    id: "s1".into(),
                    text: "Hello".into(),
                    words: vec![crate::data::doc::Word {
                        id: "w1".into(),
                        text: "Hello".into(),
                        start: 0.0,
                        end: 0.3,
                    }],
                }],
            }],
            translations: Default::default(),
        }
        .save(&project)
        .unwrap();
        let settings = crate::data::export_settings::VideoExportSettings {
            subtitle_language: Some("zh".into()),
            encoding_speed: crate::data::export_settings::ExportEncodingSpeed::Quality,
            ..Default::default()
        };
        let blocked = export_preflight_impl("p1", settings.clone(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert!(!blocked.ready);
        assert!(blocked
            .items
            .iter()
            .any(|item| item.code == "captions" && item.level == "blocker"));

        crate::data::subtitle::hide(&project, "s1").unwrap();
        let ready = export_preflight_impl("p1", settings, Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert!(ready.ready, "{:?}", ready.items);
        assert_eq!(ready.summary.visible_captions, 0);
        assert!(ready.summary.estimated_max_mb >= ready.summary.estimated_min_mb);
        assert!(ready
            .items
            .iter()
            .any(|item| item.code == "hidden-captions" && item.level == "warning"));

        crate::data::audio_mix::save(
            &project,
            &crate::data::audio_mix::AudioMix {
                music: vec![crate::data::audio_mix::MusicTrack {
                    id: "music-missing".into(),
                    path: tmp.path().join("missing-music.wav"),
                    start: 0.0,
                    end: 0.3,
                    source_start: 0.0,
                    volume: 0.25,
                    fade_in: 0.0,
                    fade_out: 0.0,
                    ducking: true,
                }],
                ..Default::default()
            },
            0.4,
        )
        .unwrap();
        let blocked_music = export_preflight_impl(
            "p1",
            crate::data::export_settings::VideoExportSettings {
                encoding_speed: crate::data::export_settings::ExportEncodingSpeed::Quality,
                subtitle_mode: crate::data::export_settings::ExportSubtitleMode::None,
                ..Default::default()
            },
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap();
        assert!(!blocked_music.ready);
        assert!(blocked_music.items.iter().any(|item| {
            item.code == "audio"
                && item.level == "blocker"
                && item
                    .message
                    .contains("background music music-missing is missing")
        }));

        crate::data::broll::save(
            &project,
            &[crate::data::broll::BrollPlacement {
                id: "missing-asset".into(),
                file: tmp.path().join("missing-broll.mp4"),
                start: 0.0,
                end: 0.2,
                mode: crate::data::broll::PlacementMode::Fullscreen,
                rect: None,
                fit: crate::data::broll::FitMode::Cover,
                background: crate::data::broll::BackgroundMode::Black,
                source_start: 0.0,
                radius: 0,
                name: None,
            }],
        )
        .unwrap();
        let blocked_broll = export_preflight_impl(
            "p1",
            crate::data::export_settings::VideoExportSettings {
                encoding_speed: crate::data::export_settings::ExportEncodingSpeed::Quality,
                subtitle_mode: crate::data::export_settings::ExportSubtitleMode::None,
                ..Default::default()
            },
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap();
        assert!(!blocked_broll.ready);
        assert!(blocked_broll
            .items
            .iter()
            .any(|item| item.code == "broll" && item.level == "blocker"));

        std::fs::write(project.join("cuts.json"), "{").unwrap();
        std::fs::write(project.join("broll.json"), "{").unwrap();
        std::fs::write(project.join("titles.json"), "{").unwrap();
        std::fs::write(project.join("hidden.json"), "{").unwrap();
        std::fs::write(project.join("style.json"), "{").unwrap();
        std::fs::write(
            project.join("audio-mix.json"),
            r#"{"volume":9,"muted":false,"fadeIn":0,"fadeOut":0}"#,
        )
        .unwrap();
        let corrupt_sidecars = export_preflight_impl(
            "p1",
            crate::data::export_settings::VideoExportSettings {
                encoding_speed: crate::data::export_settings::ExportEncodingSpeed::Quality,
                subtitle_mode: crate::data::export_settings::ExportSubtitleMode::None,
                ..Default::default()
            },
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap();
        assert!(!corrupt_sidecars.ready);
        for code in [
            "timeline-data",
            "broll",
            "titles",
            "audio",
            "caption-state",
            "style",
        ] {
            assert!(
                corrupt_sidecars
                    .items
                    .iter()
                    .any(|item| item.code == code && item.level == "blocker"),
                "missing blocker {code}: {:?}",
                corrupt_sidecars.items
            );
        }
    }

    #[tokio::test]
    async fn export_preflight_rejects_audio_only_source_media() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("p1");
        let media = tmp.path().join("audio-only.m4a");
        crate::proc::run(
            "ffmpeg",
            &[
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=0.4",
                "-c:a",
                "aac",
                &media.display().to_string(),
            ],
        )
        .await
        .unwrap();
        let now = chrono::Utc::now();
        Doc {
            id: "p1".into(),
            schema: 1,
            media: MediaRef {
                path: media,
                duration_seconds: 0.4,
                sample_rate: Some(44_100),
                channels: Some(1),
            },
            meta: Meta {
                title: "Audio only".into(),
                description: String::new(),
                language: Some("en".into()),
                created_at: now,
                updated_at: now,
            },
            paragraphs: vec![crate::data::doc::Paragraph {
                id: 1,
                speaker: None,
                sentences: vec![crate::data::doc::Sentence {
                    id: "s1".into(),
                    text: "Hello".into(),
                    words: vec![crate::data::doc::Word {
                        id: "w1".into(),
                        text: "Hello".into(),
                        start: 0.0,
                        end: 0.3,
                    }],
                }],
            }],
            translations: Default::default(),
        }
        .save(&project)
        .unwrap();

        let report = export_preflight_impl(
            "p1",
            crate::data::export_settings::VideoExportSettings {
                encoding_speed: crate::data::export_settings::ExportEncodingSpeed::Quality,
                ..Default::default()
            },
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap();
        assert!(!report.ready);
        assert!(report
            .items
            .iter()
            .any(|item| item.code == "media" && item.level == "blocker"));
    }

    #[tokio::test]
    async fn export_aware_finish_check_defers_translation_coverage_to_selected_caption_track() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("p1");
        let now = chrono::Utc::now();
        Doc {
            id: "p1".into(),
            schema: 1,
            media: MediaRef {
                path: tmp.path().join("source.mp4"),
                duration_seconds: 2.0,
                sample_rate: None,
                channels: None,
            },
            meta: Meta {
                title: "Selective captions".into(),
                description: String::new(),
                language: Some("en".into()),
                created_at: now,
                updated_at: now,
            },
            paragraphs: vec![crate::data::doc::Paragraph {
                id: 1,
                speaker: Some("Host".into()),
                sentences: vec![crate::data::doc::Sentence {
                    id: "s1".into(),
                    text: "A complete source caption".into(),
                    words: vec![crate::data::doc::Word {
                        id: "w1".into(),
                        text: "caption".into(),
                        start: 0.0,
                        end: 1.0,
                    }],
                }],
            }],
            translations: std::collections::BTreeMap::from([(
                "fr".into(),
                std::collections::BTreeMap::new(),
            )]),
        }
        .save(&project)
        .unwrap();

        let general = finish_check_pid("p1".into(), None, Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert!(general
            .iter()
            .any(|item| item.code == "translations-filled" && !item.pass));

        let selected = finish_check_pid(
            "p1".into(),
            Some(crate::data::export_settings::VideoExportSettings::default()),
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap();
        assert!(selected
            .iter()
            .any(|item| item.code == "translations-filled" && item.pass));
    }

    fn save_index_project(
        root: &std::path::Path,
        pid: &str,
        title: &str,
        description: &str,
        transcript: &str,
        updated_at: chrono::DateTime<chrono::Utc>,
    ) {
        Doc {
            id: pid.into(),
            schema: 1,
            media: MediaRef {
                path: root.join(format!("{pid}.mp4")),
                duration_seconds: 10.0,
                sample_rate: None,
                channels: None,
            },
            meta: Meta {
                title: title.into(),
                description: description.into(),
                language: Some("en".into()),
                created_at: updated_at,
                updated_at,
            },
            paragraphs: vec![crate::data::Paragraph {
                id: 1,
                speaker: Some("Guest".into()),
                sentences: vec![crate::data::Sentence {
                    id: "s1".into(),
                    text: transcript.into(),
                    words: vec![],
                }],
            }],
            translations: Default::default(),
        }
        .save(&root.join(pid))
        .unwrap();
    }

    #[test]
    fn project_thumbnail_video_uses_fast_bounded_seek_and_one_frame() {
        let args = project_thumbnail_args(
            std::path::Path::new("/tmp/source.mp4"),
            800.0,
            std::path::Path::new("/tmp/thumbnail.jpg"),
            false,
        );
        let seek = args.iter().position(|value| value == "-ss").unwrap();
        let input = args.iter().position(|value| value == "-i").unwrap();

        assert!(seek < input, "input seeking should happen before decoding");
        assert_eq!(args[seek + 1], "30.000");
        assert!(args.iter().any(|value| value.contains("crop=640:360")));
        assert!(args.windows(2).any(|pair| pair == ["-frames:v", "1"]));
        assert_eq!(args.last().unwrap(), "/tmp/thumbnail.jpg");
    }

    #[test]
    fn project_thumbnail_audio_renders_a_single_waveform_without_video_seek() {
        let args = project_thumbnail_args(
            std::path::Path::new("/tmp/source.wav"),
            800.0,
            std::path::Path::new("/tmp/thumbnail.jpg"),
            true,
        );

        assert!(!args.iter().any(|value| value == "-ss"));
        assert!(args.iter().any(|value| value.contains("showwavespic")));
        assert!(args.windows(2).any(|pair| pair == ["-frames:v", "1"]));
    }

    #[tokio::test]
    async fn manual_timeline_cut_is_durable_and_undoable() {
        let tmp = tempfile::tempdir().unwrap();
        save_index_project(
            tmp.path(),
            "p1",
            "Interview",
            "",
            "remove this",
            chrono::Utc::now(),
        );
        let project = tmp.path().join("p1");
        let mut doc = Doc::load(&project).unwrap();
        doc.paragraphs[0].sentences[0].words = vec![
            crate::data::Word {
                id: "w1".into(),
                text: "remove".into(),
                start: 1.0,
                end: 1.5,
            },
            crate::data::Word {
                id: "w2".into(),
                text: "this".into(),
                start: 1.5,
                end: 2.0,
            },
        ];
        doc.save(&project).unwrap();

        assert!(
            cut_manual("p1".into(), "s1".into(), Some(tmp.path().to_path_buf()),)
                .await
                .unwrap()
        );
        assert_eq!(
            cut_list("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .len(),
            1
        );
        let history = edit_history_status("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert!(history.can_undo);

        assert!(
            edit_undo("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .changed
        );
        assert!(cut_list("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap()
            .is_empty());

        assert!(
            edit_redo("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .changed
        );
        assert_eq!(
            cut_list("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn failed_retranscription_save_restores_the_previous_authoritative_state() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("p1");
        let now = chrono::Utc::now();
        let make_doc = |sentence_id: &str, text: &str| Doc {
            id: "p1".into(),
            schema: 1,
            media: MediaRef {
                path: tmp.path().join("source.mp4"),
                duration_seconds: 2.0,
                sample_rate: Some(48_000),
                channels: Some(2),
            },
            meta: Meta {
                title: "Interview".into(),
                description: String::new(),
                language: Some("en".into()),
                created_at: now,
                updated_at: now,
            },
            paragraphs: vec![crate::data::doc::Paragraph {
                id: 1,
                speaker: Some("Host".into()),
                sentences: vec![crate::data::doc::Sentence {
                    id: sentence_id.into(),
                    text: text.into(),
                    words: vec![crate::data::doc::Word {
                        id: format!("{sentence_id}-word"),
                        text: text.into(),
                        start: 0.0,
                        end: 1.0,
                    }],
                }],
            }],
            translations: Default::default(),
        };
        make_doc("old", "Old").save(&project).unwrap();
        std::fs::write(project.join("hidden.json"), r#"{"hidden":["old"]}"#).unwrap();
        std::fs::write(project.join("cuts.json"), r#"{"cuts":[]}"#).unwrap();
        std::fs::create_dir(project.join("out.srt")).unwrap();

        assert!(persist_transcription_result(make_doc("new", "New"), project.clone()).is_err());
        let restored = Doc::load(&project).unwrap();
        assert_eq!(restored.paragraphs[0].sentences[0].id, "old");
        assert!(project.join("hidden.json").is_file());
        assert!(project.join("cuts.json").is_file());
        assert!(crate::data::version::Lineage::load(&project)
            .unwrap()
            .nodes
            .iter()
            .any(|node| node.name == "Before retranscription"));
    }

    #[tokio::test]
    async fn multiple_timeline_cuts_are_atomic_and_need_only_one_undo() {
        let tmp = tempfile::tempdir().unwrap();
        save_index_project(
            tmp.path(),
            "p1",
            "Interview",
            "",
            "remove these",
            chrono::Utc::now(),
        );
        let project = tmp.path().join("p1");
        let mut doc = Doc::load(&project).unwrap();
        doc.paragraphs[0].sentences = vec![
            crate::data::Sentence {
                id: "s1".into(),
                text: "remove one".into(),
                words: vec![
                    crate::data::Word {
                        id: "w1".into(),
                        text: "remove".into(),
                        start: 1.0,
                        end: 1.4,
                    },
                    crate::data::Word {
                        id: "w2".into(),
                        text: "one".into(),
                        start: 1.5,
                        end: 2.0,
                    },
                ],
            },
            crate::data::Sentence {
                id: "s2".into(),
                text: "remove two".into(),
                words: vec![
                    crate::data::Word {
                        id: "w3".into(),
                        text: "remove".into(),
                        start: 3.0,
                        end: 3.4,
                    },
                    crate::data::Word {
                        id: "w4".into(),
                        text: "two".into(),
                        start: 3.5,
                        end: 4.0,
                    },
                ],
            },
        ];
        doc.save(&project).unwrap();

        assert_eq!(
            cut_manual_many(
                "p1".into(),
                vec!["s1".into(), "s2".into()],
                Some(tmp.path().to_path_buf()),
            )
            .await
            .unwrap(),
            2
        );
        assert_eq!(
            cut_list("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .len(),
            2
        );
        let history = edit_history_status("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert_eq!(
            history.undo_label.as_deref(),
            Some("Remove timeline regions")
        );

        assert!(
            edit_undo("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .changed
        );
        assert!(cut_list("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap()
            .is_empty());

        assert!(cut_manual_many(
            "p1".into(),
            vec!["s1".into(), "missing".into()],
            Some(tmp.path().to_path_buf()),
        )
        .await
        .is_err());
        assert!(cut_list("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn audio_mix_is_durable_and_undoable_with_the_timeline() {
        let tmp = tempfile::tempdir().unwrap();
        save_index_project(
            tmp.path(),
            "p1",
            "Interview",
            "",
            "audio",
            chrono::Utc::now(),
        );
        let music_path = tmp.path().join("music.wav");
        std::fs::write(&music_path, b"test music").unwrap();
        let music_path = std::fs::canonicalize(music_path).unwrap();
        let mix = crate::data::audio_mix::AudioMix {
            volume: 1.2,
            muted: false,
            fade_in: 0.5,
            fade_out: 1.0,
            voice_enhance: true,
            normalize_loudness: true,
            loudness_target: -16.0,
            music: vec![crate::data::audio_mix::MusicTrack {
                id: "music-main".into(),
                path: music_path,
                start: 0.5,
                end: 4.5,
                source_start: 1.0,
                volume: 0.25,
                fade_in: 0.5,
                fade_out: 0.5,
                ducking: true,
            }],
        };

        assert_eq!(
            audio_mix_set("p1".into(), mix.clone(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap(),
            mix
        );
        assert_eq!(
            audio_mix_get("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap(),
            mix
        );
        assert!(
            edit_undo("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .changed
        );
        assert_eq!(
            audio_mix_get("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap(),
            crate::data::audio_mix::AudioMix::default()
        );
        assert!(
            edit_redo("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .changed
        );
        assert_eq!(
            audio_mix_get("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap(),
            mix
        );
    }

    #[tokio::test]
    async fn title_edits_are_durable_and_round_trip_through_undo_redo() {
        let tmp = tempfile::tempdir().unwrap();
        save_index_project(
            tmp.path(),
            "p1",
            "Interview",
            "",
            "title",
            chrono::Utc::now(),
        );
        let input = TitleClipInput {
            text: "Opening title".into(),
            start: 1.0,
            end: 3.0,
            x: 0.5,
            y: 0.18,
            font_size: 72,
            color: "#FFFFFF".into(),
            background: "#00000099".into(),
            fade_in: 0.25,
            fade_out: 0.5,
        };

        let added = title_add("p1".into(), input, Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert_eq!(
            title_list("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap(),
            vec![added.clone()]
        );
        assert!(
            edit_undo("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .changed
        );
        assert!(title_list("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap()
            .is_empty());
        assert!(
            edit_redo("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .changed
        );
        assert_eq!(
            title_list("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap(),
            vec![added]
        );
    }

    #[tokio::test]
    async fn project_index_searches_content_and_persists_starred_order() {
        let tmp = tempfile::tempdir().unwrap();
        let now = chrono::Utc::now();
        save_index_project(
            tmp.path(),
            "older",
            "Design interview",
            "Customer notes",
            "A phrase only in the transcript",
            now - chrono::Duration::hours(1),
        );
        save_index_project(
            tmp.path(),
            "newer",
            "Weekly update",
            "Shipping notes",
            "Nothing special",
            now,
        );

        let transcript_matches = project_search(
            "ONLY IN THE TRANSCRIPT".into(),
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap();
        assert_eq!(transcript_matches.len(), 1);
        assert_eq!(transcript_matches[0].pid, "older");
        assert_eq!(transcript_matches[0].description, "Customer notes");

        let activity = crate::data::activity::touch(&tmp.path().join("older")).unwrap();
        let starred = project_set_star("older".into(), true, Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert!(starred.starred);
        assert_eq!(starred.updated_at, activity);
        let projects = project_list(Some(tmp.path().to_path_buf())).await.unwrap();
        assert_eq!(projects[0].pid, "older");
        assert!(projects[0].starred);

        let persisted: ProjectLocalState = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join("older/project-state.json")).unwrap(),
        )
        .unwrap();
        assert!(persisted.starred);
    }

    #[tokio::test]
    async fn project_open_time_is_durable_and_survives_star_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let now = chrono::Utc::now();
        save_index_project(
            tmp.path(),
            "older",
            "Older project",
            "",
            "",
            now - chrono::Duration::days(2),
        );
        save_index_project(tmp.path(), "newer", "Newer project", "", "", now);

        let initial = project_list(Some(tmp.path().to_path_buf())).await.unwrap();
        assert_eq!(initial[0].pid, "newer");

        let opened = project_mark_opened("older".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        let opened_at = opened.last_opened_at.expect("last open time");
        let reordered = project_list(Some(tmp.path().to_path_buf())).await.unwrap();
        assert_eq!(reordered[0].pid, "older");
        assert_eq!(reordered[0].last_opened_at, Some(opened_at));

        let starred = project_set_star("older".into(), true, Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert!(starred.starred);
        assert_eq!(starred.last_opened_at, Some(opened_at));

        let persisted: ProjectLocalState = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join("older/project-state.json")).unwrap(),
        )
        .unwrap();
        assert!(persisted.starred);
        assert_eq!(persisted.last_opened_at, Some(opened_at));
    }

    #[tokio::test]
    async fn speaker_evidence_assignment_and_preview_apply_are_recoverable() {
        let tmp = tempfile::tempdir().unwrap();
        save_index_project(
            tmp.path(),
            "p1",
            "Interview",
            "",
            "Hello there",
            chrono::Utc::now(),
        );
        let project = tmp.path().join("p1");
        let mut doc = Doc::load(&project).unwrap();
        doc.paragraphs[0].speaker = Some("Alice".into());
        doc.paragraphs[0].sentences[0].words = vec![
            crate::data::Word {
                id: "w0".into(),
                text: "Hello".into(),
                start: 1.0,
                end: 1.4,
            },
            crate::data::Word {
                id: "w1".into(),
                text: "there".into(),
                start: 1.5,
                end: 2.0,
            },
        ];
        doc.save(&project).unwrap();

        let evidence = speaker_evidence("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert!(evidence.identified);
        assert_eq!(evidence.turns[0].text, "Hello there");
        assert_eq!((evidence.turns[0].start, evidence.turns[0].end), (1.0, 2.0));

        speaker_assign(
            "p1".into(),
            SpeakerAssignmentInput {
                paragraph_id: 1,
                speaker: Some("Host".into()),
            },
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap();
        assert_eq!(
            Doc::load(&project).unwrap().paragraphs[0]
                .speaker
                .as_deref(),
            Some("Host")
        );

        let proposals = vec![SpeakerReidentifyProposal {
            paragraph_id: 1,
            current: Some("Host".into()),
            cluster: "SPEAKER_00".into(),
            proposed: "SPEAKER_00".into(),
            start: 1.0,
            end: 2.0,
            text: "Hello there".into(),
            coverage: 0.95,
            margin: 0.9,
        }];
        let preview = SpeakerReidentifyPreview {
            segments: 1,
            changed: 1,
            unassigned: 0,
            proposals: proposals.clone(),
        };
        assert!(speaker_preview_matches_doc(
            &Doc::load(&project).unwrap(),
            &preview
        ));
        assert_eq!(
            speaker_reidentify_apply(
                "p1".into(),
                proposals.clone(),
                Some(tmp.path().to_path_buf()),
            )
            .await
            .unwrap(),
            1
        );
        assert_eq!(
            Doc::load(&project).unwrap().paragraphs[0]
                .speaker
                .as_deref(),
            Some("SPEAKER_00")
        );
        assert!(!speaker_preview_matches_doc(
            &Doc::load(&project).unwrap(),
            &preview
        ));
        assert_eq!(
            version_list("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .versions
                .len(),
            1
        );
        assert!(
            speaker_reidentify_apply("p1".into(), proposals, Some(tmp.path().to_path_buf()),)
                .await
                .is_err()
        );
        let history = edit_history_status("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert_eq!(
            history.undo_label.as_deref(),
            Some("Apply speaker analysis")
        );
        assert!(
            edit_undo("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .changed
        );
        assert_eq!(
            Doc::load(&project).unwrap().paragraphs[0]
                .speaker
                .as_deref(),
            Some("Host")
        );
        assert!(
            edit_undo("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .changed
        );
        assert_eq!(
            Doc::load(&project).unwrap().paragraphs[0]
                .speaker
                .as_deref(),
            Some("Alice")
        );
        assert!(crate::data::activity::load(&project).is_some());
    }

    #[tokio::test]
    async fn version_commands_expose_head_and_active_branch() {
        let tmp = tempfile::tempdir().unwrap();
        save_index_project(
            tmp.path(),
            "p1",
            "Interview",
            "",
            "A recoverable transcript",
            chrono::Utc::now(),
        );

        let version = version_commit(
            "p1".into(),
            "Baseline".into(),
            "Before editing".into(),
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap();
        let initial = version_list("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert_eq!(initial.head.as_deref(), Some(version.as_str()));
        assert_eq!(initial.versions.len(), 1);
        let wire = serde_json::to_value(&initial).unwrap();
        assert_eq!(wire.get("v").and_then(serde_json::Value::as_u64), Some(1));
        assert!(wire.get("versions").is_some());
        assert!(wire.get("activeBranch").is_some());

        let project = tmp.path().join("p1");
        let mut edited = Doc::load(&project).unwrap();
        edited.meta.title = "Unsaved branch work".into();
        edited.save(&project).unwrap();

        let branch = branch_create(
            "p1".into(),
            "Alternative".into(),
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap();
        let branched = version_list("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert_eq!(branched.active_branch.as_deref(), Some(branch.as_str()));
        assert!(branched.branches.iter().any(|item| item.id == branch));
        assert_eq!(
            Doc::load(&project).unwrap().meta.title,
            "Unsaved branch work"
        );
        assert_eq!(branched.versions.len(), 2);

        let mut dirty = Doc::load(&project).unwrap();
        dirty.meta.title = "Still editing".into();
        dirty.save(&project).unwrap();
        assert!(
            branch_switch("p1".into(), "main".into(), Some(tmp.path().to_path_buf()),)
                .await
                .is_err()
        );
        assert_eq!(Doc::load(&project).unwrap().meta.title, "Still editing");
    }

    #[tokio::test]
    async fn timing_repair_caps_media_tail_and_saves_a_recovery_version() {
        let tmp = tempfile::tempdir().unwrap();
        save_index_project(
            tmp.path(),
            "p1",
            "Interview",
            "",
            "tail",
            chrono::Utc::now(),
        );
        let project = tmp.path().join("p1");
        let mut doc = Doc::load(&project).unwrap();
        doc.media.duration_seconds = 10.0;
        doc.paragraphs[0].sentences[0].words = vec![crate::data::Word {
            id: "w0".into(),
            text: "tail".into(),
            start: 9.98,
            end: 10.2,
        }];
        doc.save(&project).unwrap();

        let result = timing_repair("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert_eq!(result, "1 fix(es)");
        let repaired = Doc::load(&project).unwrap();
        assert!(repaired.all_words()[0].end <= repaired.media.duration_seconds);
        let history = version_list("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert_eq!(history.versions.len(), 1);
        assert_eq!(history.versions[0].name, "before timing repair");
    }

    fn broll_input(file: &std::path::Path, start: f64, end: f64) -> BrollPlacementInput {
        BrollPlacementInput {
            file: file.to_path_buf(),
            start,
            end,
            mode: Some(crate::data::broll::PlacementMode::Pip),
            fit: None,
            background: None,
            rect: None,
            source_start: None,
            radius: Some(12),
            name: Some("Product close-up".into()),
        }
    }

    #[tokio::test]
    async fn broll_commands_cover_suggestion_accept_update_and_remove() {
        let tmp = tempfile::tempdir().unwrap();
        save_index_project(
            tmp.path(),
            "p1",
            "Interview",
            "",
            "show the product",
            chrono::Utc::now(),
        );
        let project = tmp.path().join("p1");
        let mut doc = Doc::load(&project).unwrap();
        doc.media.duration_seconds = 30.0;
        doc.paragraphs[0].sentences[0].words = vec![
            crate::data::Word {
                id: "wc".into(),
                text: "cut".into(),
                start: 1.0,
                end: 3.0,
            },
            crate::data::Word {
                id: "w0".into(),
                text: "show".into(),
                start: 5.0,
                end: 5.8,
            },
            crate::data::Word {
                id: "w1".into(),
                text: "product".into(),
                start: 6.0,
                end: 7.0,
            },
        ];
        doc.save(&project).unwrap();
        std::fs::create_dir_all(project.join("ai")).unwrap();
        std::fs::write(
            project.join("ai/broll-suggestions.json"),
            r#"{"suggestions":[{"start":"w0","end":"w1","mode":"pip","query":"product detail","reason":"show the object"}]}"#,
        )
        .unwrap();
        let asset = tmp.path().join("asset.png");
        std::fs::write(&asset, "fixture").unwrap();
        let suggestion = crate::pipeline::broll::BrollSuggestion {
            start: "w0".into(),
            end: "w1".into(),
            mode: crate::pipeline::broll::BrollMode::Pip,
            query: "product detail".into(),
            reason: "show the object".into(),
        };

        let accepted = broll_accept_suggestion(
            "p1".into(),
            suggestion.clone(),
            asset.clone(),
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap();
        assert_eq!((accepted.start, accepted.end), (5.0, 7.0));
        assert_eq!(accepted.name.as_deref(), Some("product detail"));
        assert_eq!(accepted.rect, Some(default_pip_rect()));

        let mut stale = suggestion;
        stale.query = "changed while picker was open".into();
        assert!(broll_accept_suggestion(
            "p1".into(),
            stale,
            asset.clone(),
            Some(tmp.path().to_path_buf()),
        )
        .await
        .is_err());

        let overlap = broll_add(
            "p1".into(),
            broll_input(&asset, 6.5, 8.0),
            Some(tmp.path().to_path_buf()),
        )
        .await;
        assert!(overlap.is_err());

        let updated = broll_update(
            "p1".into(),
            accepted.id.clone(),
            broll_input(&asset, 8.0, 12.0),
            Some(tmp.path().to_path_buf()),
        )
        .await
        .unwrap();
        assert_eq!((updated.start, updated.end), (8.0, 12.0));
        let cuts = ClipCuts {
            cuts: vec![crate::data::Cut {
                id: "cut-before-broll".into(),
                note: None,
                a_word: "wc".into(),
                b_word: "wc".into(),
                kind: crate::data::CutKind::Manual,
                duration: 2.0,
            }],
        };
        assert_eq!(
            broll_preview_points(&doc, &cuts, std::slice::from_ref(&updated)),
            vec![(8.0, 10.0, 0)]
        );
        std::fs::write(project.join("ai/broll-suggestions.json"), "not json").unwrap();
        let partial = broll_list("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert_eq!(partial.accepted.len(), 1);
        assert!(partial.suggestions.is_empty());
        assert_eq!(partial.errors.len(), 1);
        assert!(
            broll_remove("p1".into(), accepted.id, Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
        );
        assert!(crate::data::broll::load(&project).unwrap().is_empty());
    }

    #[tokio::test]
    async fn concurrent_broll_adds_do_not_lose_updates() {
        let tmp = tempfile::tempdir().unwrap();
        save_index_project(
            tmp.path(),
            "p1",
            "Interview",
            "",
            "two placements",
            chrono::Utc::now(),
        );
        let project = tmp.path().join("p1");
        let mut doc = Doc::load(&project).unwrap();
        doc.media.duration_seconds = 30.0;
        doc.save(&project).unwrap();
        let first_asset = tmp.path().join("first.png");
        let second_asset = tmp.path().join("second.png");
        std::fs::write(&first_asset, "first").unwrap();
        std::fs::write(&second_asset, "second").unwrap();
        let root = tmp.path().to_path_buf();

        let (first, second) = tokio::join!(
            broll_add(
                "p1".into(),
                broll_input(&first_asset, 4.0, 7.0),
                Some(root.clone()),
            ),
            broll_add(
                "p1".into(),
                broll_input(&second_asset, 9.0, 12.0),
                Some(root),
            )
        );
        first.unwrap();
        second.unwrap();
        assert_eq!(crate::data::broll::load(&project).unwrap().len(), 2);
    }

    #[tokio::test]
    async fn concurrent_doc_edits_share_one_project_transaction() {
        let tmp = tempfile::tempdir().unwrap();
        save_index_project(
            tmp.path(),
            "p1",
            "Interview",
            "",
            "original",
            chrono::Utc::now(),
        );
        let project = tmp.path().join("p1");
        let mut doc = Doc::load(&project).unwrap();
        doc.paragraphs[0].sentences = (0..24)
            .map(|index| crate::data::Sentence {
                id: format!("s{index}"),
                text: format!("original {index}"),
                words: vec![],
            })
            .collect();
        doc.save(&project).unwrap();

        let mut edits = tokio::task::JoinSet::new();
        for index in 0..24 {
            let root = tmp.path().to_path_buf();
            edits.spawn(async move {
                subtitle_set(
                    "p1".into(),
                    format!("s{index}"),
                    format!("edited {index}"),
                    Some(root),
                )
                .await
            });
        }
        while let Some(result) = edits.join_next().await {
            assert!(result.unwrap().unwrap());
        }

        let saved = Doc::load(&project).unwrap();
        for index in 0..24 {
            assert_eq!(
                saved.paragraphs[0].sentences[index].text,
                format!("edited {index}")
            );
        }
    }

    #[tokio::test]
    async fn broll_list_rejects_project_path_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            broll_list("../outside".into(), Some(tmp.path().to_path_buf()))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn speaker_preview_safely_splits_legacy_multi_sentence_projects_on_apply() {
        let tmp = tempfile::tempdir().unwrap();
        save_index_project(
            tmp.path(),
            "p1",
            "Legacy interview",
            "",
            "First",
            chrono::Utc::now(),
        );
        let project = tmp.path().join("p1");
        let mut doc = Doc::load(&project).unwrap();
        doc.paragraphs[0].id = 42;
        doc.paragraphs[0].speaker = None;
        doc.paragraphs[0].sentences[0].id = "cue-a".into();
        doc.paragraphs[0].sentences[0].text = "First speaker".into();
        doc.paragraphs[0].sentences[0].words = vec![crate::data::Word {
            id: "w0".into(),
            text: "First speaker".into(),
            start: 0.0,
            end: 1.0,
        }];
        let mut second = doc.paragraphs[0].sentences[0].clone();
        second.id = "cue-b".into();
        second.text = "Second speaker".into();
        second.words[0].id = "w1".into();
        second.words[0].text = "Second speaker".into();
        second.words[0].start = 2.0;
        second.words[0].end = 3.0;
        doc.paragraphs[0].sentences.push(second);
        doc.save(&project).unwrap();

        let proposals = vec![
            SpeakerReidentifyProposal {
                paragraph_id: 1,
                current: None,
                cluster: "SPEAKER_00".into(),
                proposed: "Alice".into(),
                start: 0.0,
                end: 1.0,
                text: "First speaker".into(),
                coverage: 1.0,
                margin: 1.0,
            },
            SpeakerReidentifyProposal {
                paragraph_id: 2,
                current: None,
                cluster: "SPEAKER_01".into(),
                proposed: "Bob".into(),
                start: 2.0,
                end: 3.0,
                text: "Second speaker".into(),
                coverage: 1.0,
                margin: 1.0,
            },
        ];
        let preview = SpeakerReidentifyPreview {
            segments: 2,
            changed: 2,
            unassigned: 0,
            proposals: proposals.clone(),
        };
        assert!(speaker_preview_matches_doc(
            &Doc::load(&project).unwrap(),
            &preview
        ));

        assert_eq!(
            speaker_reidentify_apply("p1".into(), proposals, Some(tmp.path().to_path_buf()),)
                .await
                .unwrap(),
            2
        );
        let applied = Doc::load(&project).unwrap();
        assert_eq!(applied.paragraphs.len(), 2);
        assert_eq!(applied.paragraphs[0].sentences[0].id, "cue-a");
        assert_eq!(applied.paragraphs[0].speaker.as_deref(), Some("Alice"));
        assert_eq!(applied.paragraphs[1].sentences[0].id, "cue-b");
        assert_eq!(applied.paragraphs[1].speaker.as_deref(), Some("Bob"));

        assert!(
            edit_undo("p1".into(), Some(tmp.path().to_path_buf()))
                .await
                .unwrap()
                .changed
        );
        let restored = Doc::load(&project).unwrap();
        assert_eq!(restored.paragraphs.len(), 1);
        assert_eq!(restored.paragraphs[0].id, 42);
        assert_eq!(restored.paragraphs[0].sentences.len(), 2);
    }

    fn settings() -> SettingsPayload {
        SettingsPayload {
            llm_endpoint: "http://localhost:11434/v1/chat/completions".into(),
            llm_api_key: "sk-test".into(),
            llm_model: "gpt-4o-mini".into(),
            worker_count: 7,
            ..SettingsPayload::default()
        }
    }

    #[test]
    fn settings_serializes_camel_case_keys() {
        let v = serde_json::to_value(settings()).unwrap();
        let obj = v.as_object().unwrap();
        for k in [
            "asrEngine",
            "asrModel",
            "asrAligner",
            "asrCloudEndpoint",
            "asrCloudApiKey",
            "asrCloudModel",
            "diarizeModel",
            "hfToken",
            "llmEndpoint",
            "llmApiKey",
            "llmModel",
            "workerCount",
        ] {
            assert!(obj.contains_key(k), "missing camelCase key {k}");
        }
        assert!(!obj.contains_key("llm_endpoint"));
        assert!(!obj.contains_key("worker_count"));
    }

    #[test]
    fn settings_still_deserializes_frontend_snake_case() {
        // The IPC payload the frontend sends today keeps working.
        let back: SettingsPayload = serde_json::from_value(serde_json::json!({
            "llm_endpoint": "e",
            "hf_token": "hf_test",
            "llm_api_key": "k",
            "llm_model": "m",
            "worker_count": 2
        }))
        .unwrap();
        assert_eq!(back.worker_count, 2);
        assert_eq!(back.llm_model, "m");
        assert_eq!(back.hf_token, "hf_test");
    }

    #[test]
    fn webview_config_redacts_every_secret_but_reports_presence() {
        let config = crate::data::modelconfig::ModelConfig {
            hf_token: "hf-secret".into(),
            llm_api_key: "llm-secret".into(),
            asr_cloud_api_key: "asr-secret".into(),
            ..crate::data::modelconfig::ModelConfig::default()
        };
        let public = redact_model_config(config);
        assert!(public.config.hf_token.is_empty());
        assert!(public.config.llm_api_key.is_empty());
        assert!(public.config.asr_cloud_api_key.is_empty());
        assert!(public.hf_token_set);
        assert!(public.llm_api_key_set);
        assert!(public.asr_cloud_api_key_set);
        let serialized = serde_json::to_string(&public).unwrap();
        assert!(!serialized.contains("secret"));
    }

    #[test]
    fn write_settings_file_emits_camel_case_json() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_settings_file(tmp.path(), &settings()).unwrap();
        let raw = std::fs::read_to_string(path).unwrap();
        assert!(raw.contains("\"llmEndpoint\""), "got: {raw}");
        assert!(raw.contains("\"llmApiKey\""), "got: {raw}");
        assert!(raw.contains("\"llmModel\""), "got: {raw}");
        assert!(raw.contains("\"hfToken\""), "got: {raw}");
        assert!(raw.contains("\"workerCount\""), "got: {raw}");
        assert!(!raw.contains("llm_endpoint"), "got: {raw}");
    }

    #[test]
    fn apply_worker_count_resizes_live_allocator_and_state() {
        let state = AgentServerState::default();
        let alloc = std::sync::Arc::new(Allocator::new(1));
        let pool = std::sync::Arc::new(std::sync::Mutex::new(
            crate::agent::pool::WorkerPool::new_workers(1),
        ));
        *state.allocator.lock().unwrap() = Some(AllocatorHandle {
            allocator: alloc.clone(),
            addr: "127.0.0.1:9".parse().unwrap(),
            pool,
        });
        apply_worker_count(&state, &settings());
        assert_eq!(alloc.capacity(), 7);
        assert_eq!(*state.worker_count.lock().unwrap(), 7);
    }

    /// Regression for the double-allocator split: a call enqueued through
    /// the IPC-side state must be claimable from the HTTP router's state
    /// (both share one allocator since `agent_serve`/`bind` were fixed).
    #[test]
    fn enqueue_is_claimable_through_shared_allocator() {
        use axum::response::IntoResponse;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let state = AgentServerState::default();
            let capacity = *state.worker_count.lock().unwrap();
            let allocator = std::sync::Arc::new(Allocator::new(capacity));
            let pool = std::sync::Arc::new(std::sync::Mutex::new(
                crate::agent::pool::WorkerPool::new_workers(1),
            ));
            let (addr, _router) = crate::agent::http::bind(0, allocator.clone(), pool.clone())
                .await
                .unwrap();
            *state.allocator.lock().unwrap() = Some(AllocatorHandle {
                allocator: allocator.clone(),
                addr,
                pool: pool.clone(),
            });
            enqueue_call(
                &state,
                crate::agent::PendingCall {
                    id: "c1".into(),
                    kind: "polish".into(),
                    word_count: 5,
                    char_count: 5,
                    payload_ref: "/tmp/x".into(),
                    submission_ref: None,
                    problems: vec![],
                    contract: None,
                },
            )
            .unwrap();
            // The HTTP layer claims from the *same* allocator instance.
            let resp = crate::agent::http::agent_next(axum::extract::State(
                crate::agent::http::ServerState::new(allocator, pool.clone()),
            ))
            .await
            .into_response();
            assert_eq!(resp.status(), axum::http::StatusCode::OK);
        });
    }

    #[test]
    fn payload_char_count_counts_chars_and_tolerates_missing_files() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("prompt.json");
        std::fs::write(&p, "héllo wörld").unwrap(); // 11 chars, 12 bytes
        assert_eq!(payload_char_count(p.to_str().unwrap()), 11);
        assert_eq!(payload_char_count("/nonexistent/prompt.json"), 0);
    }

    #[tokio::test]
    async fn task_status_exposes_polish_quality_artifact() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("p1");
        let artifact = crate::pipeline::polish::PolishQualityArtifact {
            fingerprint: "0:::0".into(),
            created_at: chrono::Utc::now(),
            status: crate::pipeline::polish::PolishQualityStatus::Warn,
            page_count: 2,
            measured_page_count: 2,
            retry_count: 0,
            recovered_page_count: 0,
            fallback_page_count: 0,
            fallback_sentence_count: 0,
            residual_term_variant_count: 0,
            residual_term_variants: vec![],
            zero_duration_word_count_before: 0,
            zero_duration_word_count_after: 0,
        };
        artifact
            .save(&project.join("ai/polish-quality.json"))
            .unwrap();

        let status = task_status("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert_eq!(
            status.polish_quality.as_ref().unwrap().status,
            crate::pipeline::polish::PolishQualityStatus::Warn
        );
        let json = serde_json::to_value(&status).unwrap();
        assert!(json.get("polishQuality").is_some());
    }

    #[tokio::test]
    async fn task_status_reports_each_kind_and_failure_count() {
        let tmp = tempfile::tempdir().unwrap();
        let translate = tmp.path().join("p1/ai/translate");
        std::fs::create_dir_all(translate.join("pending")).unwrap();
        std::fs::create_dir_all(translate.join("done")).unwrap();
        std::fs::create_dir_all(translate.join("failed")).unwrap();
        std::fs::write(
            translate.join("task.json"),
            r#"{"kind":"translate","lang":"ja","calls":3}"#,
        )
        .unwrap();
        std::fs::write(translate.join("pending/a.json"), "{}").unwrap();
        std::fs::write(translate.join("done/b.json"), "{}").unwrap();
        std::fs::write(
            translate.join("failed/a.json"),
            r#"{"error":"provider rejected the request"}"#,
        )
        .unwrap();

        let status = task_status("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert_eq!(status.kinds.len(), 1);
        assert_eq!(status.kinds[0].kind, "translate");
        assert_eq!(status.kinds[0].lang.as_deref(), Some("ja"));
        assert_eq!(status.kinds[0].calls, 3);
        assert_eq!(status.kinds[0].done, 1);
        assert_eq!(status.kinds[0].failed, 1);
        assert_eq!(status.kinds[0].pending, 0);
        assert_eq!(status.pending, 0);
        assert_eq!(status.failed, 1);
        assert_eq!(
            status.kinds[0].last_error.as_deref(),
            Some("provider rejected the request")
        );
    }

    #[tokio::test]
    async fn task_status_only_counts_the_active_run() {
        let tmp = tempfile::tempdir().unwrap();
        let translate = tmp.path().join("p1/ai/translate");
        std::fs::create_dir_all(translate.join("pending")).unwrap();
        std::fs::create_dir_all(translate.join("done")).unwrap();
        std::fs::create_dir_all(translate.join("failed")).unwrap();
        std::fs::write(
            translate.join("task.json"),
            r#"{"kind":"translate","lang":"zh","runId":"current","calls":2,"state":"failed","error":"provider timeout"}"#,
        )
        .unwrap();
        std::fs::write(translate.join("pending/translate-current-0001.json"), "{}").unwrap();
        std::fs::write(translate.join("done/translate-current-0000.json"), "{}").unwrap();
        std::fs::write(translate.join("done/translate-old-0000.json"), "{}").unwrap();
        std::fs::write(
            translate.join("failed/translate-old-0001.json"),
            r#"{"error":"stale failure"}"#,
        )
        .unwrap();

        let status = task_status("p1".into(), Some(tmp.path().to_path_buf()))
            .await
            .unwrap();

        assert_eq!(status.kinds[0].calls, 2);
        assert_eq!(status.kinds[0].done, 1);
        assert_eq!(status.kinds[0].pending, 1);
        assert_eq!(status.kinds[0].failed, 0);
        assert_eq!(status.kinds[0].state, "failed");
        assert_eq!(
            status.kinds[0].last_error.as_deref(),
            Some("provider timeout")
        );
        assert_eq!(status.done, 1);
        assert_eq!(status.pending, 1);
    }

    #[test]
    fn llm_model_catalog_uses_the_provider_api_root() {
        assert_eq!(
            derive_models_endpoint("https://api.minimax.io/v1/chat/completions").unwrap(),
            "https://api.minimax.io/v1/models"
        );
        assert_eq!(
            derive_models_endpoint("https://open.bigmodel.cn/api/paas/v4/chat/completions")
                .unwrap(),
            "https://open.bigmodel.cn/api/paas/v4/models"
        );
        assert_eq!(
            derive_models_endpoint("http://localhost:11434/v1/chat/completions").unwrap(),
            "http://localhost:11434/v1/models"
        );
    }

    #[test]
    fn llm_model_catalog_parses_unique_openai_compatible_ids() {
        let models = parse_llm_models(
            r#"{"object":"list","data":[{"id":"MiniMax-M3"},{"id":"MiniMax-M3"},{"id":"MiniMax-M2.7"}]}"#,
        )
        .unwrap();
        assert_eq!(models, vec!["MiniMax-M3", "MiniMax-M2.7"]);
    }
}
