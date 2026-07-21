//! Tauri IPC commands.
//!
//! Stage 5 wires every Stage-3 + Stage-4 entry point into a `#[tauri::command]`
//! so the React frontend can drive the editor in-process.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;
use tokio::io::AsyncWriteExt;

use crate::VERSION;

use crate::agent::Allocator;
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
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| AppError::Schema(format!("{label} task failed: {error}")))?
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
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ProjectLocalState {
    starred: bool,
}

fn load_project_local_state(dir: &std::path::Path) -> ProjectLocalState {
    std::fs::read_to_string(dir.join("project-state.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_project_local_state(dir: &std::path::Path, state: &ProjectLocalState) -> AppResult<()> {
    let target = dir.join("project-state.json");
    let temporary = dir.join("project-state.json.tmp");
    std::fs::write(&temporary, serde_json::to_string_pretty(state)?)?;
    std::fs::rename(temporary, target)?;
    Ok(())
}

fn project_summary(dir: PathBuf, doc: &Doc) -> ProjectSummary {
    let pid = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| doc.id.clone());
    ProjectSummary {
        pid,
        title: doc.meta.title.clone(),
        description: doc.meta.description.clone(),
        path: dir.clone(),
        duration_seconds: doc.media.duration_seconds,
        word_count: doc.all_words().len(),
        paragraph_count: doc.paragraphs.len(),
        updated_at: doc.meta.updated_at,
        starred: load_project_local_state(&dir).starred,
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
            let doc = Doc::load(&dir).ok()?;
            project_matches(&doc, query).then(|| project_summary(dir, &doc))
        })
        .collect::<Vec<_>>();
    projects.sort_by(|left, right| {
        right
            .starred
            .cmp(&left.starred)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
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
    })
}

#[tauri::command]
pub async fn project_show(pid: String, root: Option<PathBuf>) -> AppResult<Doc> {
    let dir = resolve_project_dir(&pid, root)?;
    run_blocking("project load", move || Doc::load(&dir)).await
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
    run_blocking("project star update", move || {
        let doc = Doc::load(&dir)?;
        save_project_local_state(&dir, &ProjectLocalState { starred })?;
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
    run_blocking("project metadata update", move || {
        let mut doc = Doc::load(&dir)?;
        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::Schema("project title cannot be empty".into()));
        }
        doc.meta.title = title.to_string();
        doc.meta.description = description.trim().to_string();
        doc.meta.language = language
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        doc.meta.updated_at = chrono::Utc::now();
        doc.save(&dir)?;
        Ok(doc)
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
pub async fn project_delete(
    pid: String,
    root: Option<PathBuf>,
    transcription: tauri::State<'_, TranscriptionState>,
    recording: tauri::State<'_, RecordingState>,
) -> AppResult<bool> {
    if transcription
        .jobs
        .lock()
        .expect("transcription state poisoned")
        .get(&pid)
        .is_some_and(|job| matches!(job.status.state.as_str(), "running" | "cancelling"))
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
    let dir = resolve_project_dir(&pid, root)?;
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
}

impl Default for MediaAssetState {
    fn default() -> Self {
        Self {
            current: Mutex::new(None),
        }
    }
}

#[tauri::command]
pub async fn media_asset_allow(
    pid: String,
    root: Option<PathBuf>,
    app: tauri::AppHandle,
    state: tauri::State<'_, MediaAssetState>,
) -> AppResult<String> {
    let doc = Doc::load(&resolve_project_root(root).join(&pid))?;
    let media_path = std::fs::canonicalize(&doc.media.path)?;
    if !media_path.is_file() {
        return Err(AppError::ProjectNotFound(media_path));
    }

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

async fn run_auto_impl<F>(args: AutoArgs, report: F) -> AppResult<AutoResult>
where
    F: Fn(&str, u8),
{
    report("preparing", 5);
    ensure_not_cancelled()?;
    let out_dir = resolve_project_root(args.out);
    tokio::fs::create_dir_all(&out_dir).await?;

    let requested_pid = args.pid.filter(|pid| !pid.trim().is_empty());
    let download_dir = requested_pid
        .as_ref()
        .map(|pid| out_dir.join(pid))
        .unwrap_or_else(|| out_dir.clone());
    tokio::fs::create_dir_all(&download_dir).await?;
    let media_path = if args.media.starts_with("http://") || args.media.starts_with("https://") {
        report("downloading", 12);
        crate::media_url::download(&args.media, &download_dir.join("source.%(ext)s")).await?
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
        report("extracting", 25);
        extract_audio_wav(&media_path, &wav).await?;
    }
    ensure_not_cancelled()?;
    report("analyzing", 35);
    let info = probe(&media_path).await?;
    ensure_not_cancelled()?;

    let model_config =
        run_blocking("model config load", || Ok(crate::data::modelconfig::load())).await?;
    let model = args.model.as_deref().unwrap_or(&model_config.asr_model);
    report("transcribing", 45);
    let asr_out = crate::asr::transcribe_file_with_aligner(
        &wav,
        model,
        args.lang.as_deref(),
        Some(&model_config.asr_aligner),
    )
    .await?;
    ensure_not_cancelled()?;

    report("saving", 88);
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
    report("exporting", 94);
    let result = run_blocking("project save and subtitle export", move || {
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
            pid_dir,
            srt,
            vtt,
            ass,
            md,
            word_count: doc.all_words().len(),
            paragraph_count: doc.paragraphs.len(),
        })
    })
    .await?;
    report("completed", 100);
    Ok(result)
}

#[tauri::command]
pub async fn run_auto(args: AutoArgs) -> AppResult<AutoResult> {
    run_auto_impl(args, |_, _| {}).await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionJobStatus {
    pub pid: String,
    pub state: String,
    pub phase: String,
    pub progress: u8,
    pub error: Option<String>,
}

struct TranscriptionJob {
    status: TranscriptionJobStatus,
    cancel: Arc<AtomicBool>,
}

#[derive(Clone, Default)]
pub struct TranscriptionState {
    jobs: Arc<Mutex<HashMap<String, TranscriptionJob>>>,
}

fn update_transcription_job(
    jobs: &Mutex<HashMap<String, TranscriptionJob>>,
    pid: &str,
    phase: &str,
    progress: u8,
) {
    if let Some(job) = jobs
        .lock()
        .expect("transcription state poisoned")
        .get_mut(pid)
    {
        job.status.phase = phase.to_string();
        job.status.progress = progress;
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
    let remove_incomplete_url_project = (args.media.starts_with("http://")
        || args.media.starts_with("https://"))
        && !job_dir.join("doc.json").exists();
    let status = TranscriptionJobStatus {
        pid: pid.clone(),
        state: "running".into(),
        phase: "preparing".into(),
        progress: 0,
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
            },
        );
    }

    let jobs = state.jobs.clone();
    let task_pid = pid.clone();
    tauri::async_runtime::spawn(async move {
        let report_jobs = jobs.clone();
        let report_pid = task_pid.clone();
        let work = run_auto_impl(args, move |phase, progress| {
            update_transcription_job(&report_jobs, &report_pid, phase, progress);
        });
        let result = crate::proc::with_cancellation(cancel.clone(), work).await;
        if result.is_err() && remove_incomplete_url_project {
            let _ = std::fs::remove_dir_all(&job_dir);
        }
        let mut guard = jobs.lock().expect("transcription state poisoned");
        let Some(job) = guard.get_mut(&task_pid) else {
            return;
        };
        match result {
            Ok(_) => {
                job.status.state = "completed".into();
                job.status.phase = "completed".into();
                job.status.progress = 100;
                job.status.error = None;
            }
            Err(AppError::Cancelled) => {
                job.status.state = "cancelled".into();
                job.status.phase = "cancelled".into();
                job.status.error = None;
            }
            Err(error) => {
                job.status.state = "failed".into();
                job.status.phase = "failed".into();
                job.status.error = Some(error.to_string());
            }
        }
    });
    Ok(status)
}

#[tauri::command]
pub async fn transcription_status(
    pid: String,
    state: tauri::State<'_, TranscriptionState>,
) -> AppResult<TranscriptionJobStatus> {
    state
        .jobs
        .lock()
        .expect("transcription state poisoned")
        .get(&pid)
        .map(|job| job.status.clone())
        .ok_or_else(|| AppError::Schema("no transcription job for this project".into()))
}

#[tauri::command]
pub async fn transcription_cancel(
    pid: String,
    state: tauri::State<'_, TranscriptionState>,
) -> AppResult<TranscriptionJobStatus> {
    let mut jobs = state.jobs.lock().expect("transcription state poisoned");
    let job = jobs
        .get_mut(&pid)
        .ok_or_else(|| AppError::Schema("no transcription job for this project".into()))?;
    if job.status.state == "running" {
        job.cancel.store(true, Ordering::Relaxed);
        job.status.state = "cancelling".into();
        job.status.phase = "cancelling".into();
    }
    Ok(job.status.clone())
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

#[tauri::command]
pub async fn task_start(
    state: tauri::State<'_, AgentServerState>,
    args: TaskStartArgs,
) -> AppResult<TaskStartResult> {
    let root = resolve_project_root(args.root);
    let dir = root.join(&args.pid);
    let kind = args.kind;
    let lang = args.lang;
    let task = run_blocking("task preparation", move || {
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
    })
    .await?;
    let pending = task.calls.len();
    let ai_dir = task.ai_dir.clone();
    let (agent_port, allocator) = ensure_agent_server(&state, None).await?;
    for prepared in &task.calls {
        allocator.enqueue(prepared.call.clone());
    }
    tokio::spawn(async move {
        if let Err(error) = crate::agent::task::wait_and_apply(
            allocator,
            task,
            std::time::Duration::from_secs(30 * 60),
        )
        .await
        {
            tracing::error!(%error, "task apply failed");
        }
    });
    Ok(TaskStartResult {
        pending,
        ai_dir,
        agent_port,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskKindStatus {
    pub kind: String,
    pub lang: Option<String>,
    pub pending: usize,
    pub done: usize,
    pub failed: usize,
    pub last_error: Option<String>,
    pub updated_at: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatus {
    pub pending: usize,
    pub done: usize,
    pub kinds: Vec<TaskKindStatus>,
    pub polish_quality: Option<crate::pipeline::polish::PolishQualityArtifact>,
}

fn task_kind_statuses(project_dir: &std::path::Path) -> Vec<TaskKindStatus> {
    let ai_dir = project_dir.join("ai");
    let Ok(entries) = std::fs::read_dir(ai_dir) else {
        return vec![];
    };
    let mut statuses: Vec<TaskKindStatus> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let dir = entry.path();
            let kind = entry.file_name().to_string_lossy().into_owned();
            let count = |name: &str| {
                std::fs::read_dir(dir.join(name))
                    .map(|entries| entries.filter_map(Result::ok).count())
                    .unwrap_or_default()
            };
            let done = count("done");
            let failed = count("failed");
            let failed_names: std::collections::BTreeSet<std::ffi::OsString> =
                std::fs::read_dir(dir.join("failed"))
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name())
                    .collect();
            let pending = std::fs::read_dir(dir.join("pending"))
                .map(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .filter(|entry| !failed_names.contains(&entry.file_name()))
                        .count()
                })
                .unwrap_or_default();
            if pending + done + failed == 0 && !dir.join("task.json").exists() {
                return None;
            }
            let task_json = std::fs::read_to_string(dir.join("task.json"))
                .ok()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
            let lang = task_json
                .as_ref()
                .and_then(|value| value.get("lang"))
                .and_then(|value| value.as_str())
                .map(str::to_string);
            let last_error = std::fs::read_dir(dir.join("failed"))
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .max_by_key(|entry| {
                    entry
                        .metadata()
                        .ok()
                        .and_then(|metadata| metadata.modified().ok())
                })
                .and_then(|entry| std::fs::read_to_string(entry.path()).ok())
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .and_then(|value| value.get("error")?.as_str().map(str::to_string));
            let updated_at = std::fs::metadata(dir.join("task.json"))
                .or_else(|_| std::fs::metadata(&dir))
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs());
            Some(TaskKindStatus {
                kind,
                lang,
                pending,
                done,
                failed,
                last_error,
                updated_at,
            })
        })
        .collect();
    statuses.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.kind.cmp(&right.kind))
    });
    statuses
}

#[tauri::command]
pub async fn task_status(pid: String, root: Option<PathBuf>) -> AppResult<TaskStatus> {
    let root = resolve_project_root(root);
    let project_dir = root.join(&pid);
    let (pending, done) = crate::agent::task::task_counts(&project_dir);
    let kinds = task_kind_statuses(&project_dir);
    let polish_quality = crate::pipeline::polish::PolishQualityArtifact::load(
        &project_dir.join("ai/polish-quality.json"),
    )
    .ok();
    Ok(TaskStatus {
        pending,
        done,
        kinds,
        polish_quality,
    })
}

// ============================================================================
// Pipeline commands
// ============================================================================

#[tauri::command]
pub async fn finish_check_pid(
    pid: String,
    root: Option<PathBuf>,
) -> AppResult<Vec<FinishCheckItem>> {
    let root = resolve_project_root(root);
    let dir = root.join(&pid);
    let doc = Doc::load(&dir)?;
    let cuts_path = dir.join("cuts.json");
    let cuts: ClipCuts = if cuts_path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&cuts_path)?)?
    } else {
        ClipCuts::new()
    };
    let broll = crate::data::broll::load(&dir)?;
    let items = finish_check_emit_for_project(
        &doc,
        &cuts,
        &broll,
        &dir,
        working_head_is_committed(&dir, &doc)?,
    );
    Ok(items
        .into_iter()
        .map(|i| FinishCheckItem {
            code: i.code.label().to_string(),
            ordinal: i.code as u32,
            pass: i.pass,
            blockers: i.blockers.iter().map(|b| b.message.clone()).collect(),
        })
        .collect())
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
    let root = resolve_project_root(root);
    let dir = root.join(&pid);
    let doc = Doc::load(&dir)?;
    let cuts_path = dir.join("cuts.json");
    let mut cuts: ClipCuts = if cuts_path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&cuts_path)?)?
    } else {
        ClipCuts::new()
    };
    let added = crate::pipeline::cleanup::apply(&doc, &mut cuts);
    std::fs::write(&cuts_path, serde_json::to_string_pretty(&cuts)?)?;
    Ok(added)
}

#[tauri::command]
pub async fn cut_restore(pid: String, cut_id: String, root: Option<PathBuf>) -> AppResult<bool> {
    let root = resolve_project_root(root);
    let dir = root.join(&pid);
    let cuts_path = dir.join("cuts.json");
    let mut cuts: ClipCuts = if cuts_path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&cuts_path)?)?
    } else {
        return Ok(false);
    };
    let removed = cuts.restore(&cut_id);
    if removed {
        std::fs::write(&cuts_path, serde_json::to_string_pretty(&cuts)?)?;
    }
    Ok(removed)
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
    let root = resolve_project_root(root);
    let dir = root.join(&pid);
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
}

/// Settings as sent by the frontend over IPC (snake_case) and persisted
/// to `~/.lumen-cut/settings.json` in camelCase.
#[derive(Debug, Serialize, Deserialize)]
#[serde(
    rename_all(serialize = "camelCase", deserialize = "snake_case"),
    default
)]
pub struct SettingsPayload {
    pub asr_model: String,
    pub asr_aligner: String,
    pub diarize_model: String,
    pub llm_endpoint: String,
    pub llm_api_key: String,
    pub llm_model: String,
    pub worker_count: u32,
}

impl Default for SettingsPayload {
    fn default() -> Self {
        let config = crate::data::modelconfig::ModelConfig::default();
        Self {
            asr_model: config.asr_model,
            asr_aligner: config.asr_aligner,
            diarize_model: config.diarize_model,
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
    settings: SettingsPayload,
) -> AppResult<String> {
    apply_worker_count(&state, &settings);
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let path = write_settings_file(&home, &settings)?;
    Ok(path.to_string_lossy().into_owned())
}

fn write_settings_file(home: &std::path::Path, settings: &SettingsPayload) -> AppResult<PathBuf> {
    let dir = home.join(".lumen-cut");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("settings.json");
    let body = serde_json::to_string_pretty(&settings)?;
    std::fs::write(&path, body)?;
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
    let root = resolve_project_root(root);
    let dir = root.join(&pid);
    let doc = Doc::load(&dir)?;
    let cuts: ClipCuts = std::fs::read_to_string(dir.join("cuts.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    let broll = crate::data::broll::load(&dir)?;
    let r = audit_project(&doc, &cuts, &broll, &dir);
    Ok(ReportSummary::from(&r))
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
    let Some(config) = crate::agent::runtime::load_bridge_config() else {
        return;
    };
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
        crate::agent::runtime::spawn_workers(allocator, config, count).await;
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
    enqueue_call(
        &state,
        crate::agent::PendingCall {
            id: call_id,
            kind,
            word_count,
            char_count: payload_char_count(&payload_ref),
            payload_ref,
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
    let dir = resolve_project_root(root).join(&pid);
    let doc = Doc::load(&dir)?;
    let hidden = crate::data::subtitle::load_hidden(&dir);
    Ok(crate::data::subtitle::list(&doc, &hidden, None))
}

#[tauri::command]
pub async fn subtitle_set(
    pid: String,
    id: String,
    text: String,
    root: Option<PathBuf>,
) -> AppResult<bool> {
    let dir = resolve_project_root(root).join(&pid);
    let mut doc = Doc::load(&dir)?;
    let changed = crate::data::subtitle::set(&mut doc, &id, &text);
    if changed {
        doc.save(&dir)?;
    }
    Ok(changed)
}

#[tauri::command]
pub async fn subtitle_visibility(
    pid: String,
    id: String,
    hidden: bool,
    root: Option<PathBuf>,
) -> AppResult<bool> {
    let dir = resolve_project_root(root).join(&pid);
    if hidden {
        crate::data::subtitle::hide(&dir, &id)
    } else {
        crate::data::subtitle::restore(&dir, &id)
    }
}

#[tauri::command]
pub async fn subtitle_replace(
    pid: String,
    query: String,
    replacement: String,
    regex: bool,
    root: Option<PathBuf>,
) -> AppResult<usize> {
    let dir = resolve_project_root(root).join(&pid);
    let mut doc = Doc::load(&dir)?;
    let changed = crate::data::edit::find_replace(&mut doc, &query, &replacement, regex)?;
    if changed > 0 {
        doc.save(&dir)?;
    }
    Ok(changed)
}

#[tauri::command]
pub async fn speakers_list(
    pid: String,
    root: Option<PathBuf>,
) -> AppResult<Vec<crate::data::speakers::SpeakerInfo>> {
    let dir = resolve_project_root(root).join(&pid);
    let doc = Doc::load(&dir)?;
    Ok(crate::data::speakers::list(&doc))
}

#[tauri::command]
pub async fn speaker_rename(
    pid: String,
    from: String,
    to: String,
    root: Option<PathBuf>,
) -> AppResult<usize> {
    let from = from.trim();
    let to = to.trim();
    if from.is_empty() || to.is_empty() {
        return Err(AppError::Schema("speaker names cannot be empty".into()));
    }
    let dir = resolve_project_root(root).join(&pid);
    let mut doc = Doc::load(&dir)?;
    let changed = crate::data::speakers::rename(&mut doc, from, to);
    if changed > 0 {
        doc.save(&dir)?;
    }
    Ok(changed)
}

#[tauri::command]
pub async fn speaker_merge(
    pid: String,
    from: String,
    into: String,
    root: Option<PathBuf>,
) -> AppResult<usize> {
    let from = from.trim();
    let into = into.trim();
    if from.is_empty() || into.is_empty() || from == into {
        return Err(AppError::Schema(
            "speaker merge requires two different non-empty names".into(),
        ));
    }
    let dir = resolve_project_root(root).join(&pid);
    let mut doc = Doc::load(&dir)?;
    let changed = crate::data::speakers::merge(&mut doc, from, into);
    if changed > 0 {
        doc.save(&dir)?;
    }
    Ok(changed)
}

#[derive(Debug, Serialize)]
pub struct BrollOverview {
    pub suggestions: Vec<crate::pipeline::broll::BrollSuggestion>,
    pub accepted: Vec<crate::data::broll::BrollPlacement>,
}

#[tauri::command]
pub async fn broll_list(pid: String, root: Option<PathBuf>) -> AppResult<BrollOverview> {
    let dir = resolve_project_root(root).join(&pid);
    Ok(BrollOverview {
        suggestions: crate::pipeline::broll::load_artifact(&dir)?,
        accepted: crate::data::broll::load(&dir)?,
    })
}

#[tauri::command]
pub async fn broll_preview(
    pid: String,
    at: Vec<f64>,
    root: Option<PathBuf>,
) -> AppResult<Vec<String>> {
    let dir = resolve_project_root(root).join(&pid);
    let prepare_dir = dir.clone();
    let (doc, cuts, placements, ass) = run_blocking("B-roll preview preparation", move || {
        let doc = Doc::load(&prepare_dir)?;
        let cuts: ClipCuts = std::fs::read_to_string(prepare_dir.join("cuts.json"))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        let placements = crate::data::broll::load(&prepare_dir)?;
        let ass = prepare_dir.join("broll-preview.ass");
        if !placements.is_empty() {
            crate::export::write_ass_with(&doc, &cuts.cuts, &ass, 1920, 1080)?;
        }
        Ok((doc, cuts, placements, ass))
    })
    .await?;
    if placements.is_empty() {
        return Ok(Vec::new());
    }
    let video = dir.join("broll-preview.mp4");
    crate::export::render_video_with_broll(&doc, &cuts.cuts, &ass, &video, &placements).await?;
    let timestamps = if at.is_empty() {
        placements
            .iter()
            .map(|placement| (placement.start + placement.end) / 2.0)
            .collect()
    } else {
        at
    };
    let mut outputs = Vec::new();
    for timestamp in timestamps {
        let output = dir.join(format!("broll-preview-{timestamp:.1}.png"));
        crate::media::extract_frame(&video, timestamp, &output).await?;
        outputs.push(output.to_string_lossy().into_owned());
    }
    Ok(outputs)
}

#[derive(Debug, Serialize)]
pub struct DiarizeResult {
    pub segments: usize,
    pub paragraphs_assigned: usize,
}

#[tauri::command]
pub async fn diarize_pid(pid: String, root: Option<PathBuf>) -> AppResult<DiarizeResult> {
    let dir = resolve_project_root(root).join(&pid);
    let load_dir = dir.clone();
    let (mut doc, model) = run_blocking("diarization preparation", move || {
        Ok((
            Doc::load(&load_dir)?,
            crate::data::modelconfig::load().diarize_model,
        ))
    })
    .await?;
    let wav = dir.join("audio.wav");
    if !tokio::fs::try_exists(&wav).await? {
        extract_audio_wav(&doc.media.path, &wav).await?;
    }
    let out = crate::diarize::diarize_file_with_model(&wav, &model).await?;
    let segments = out.segments;
    let segment_count = segments.len();
    let paragraphs_assigned = run_blocking("diarization save", move || {
        let paragraphs_assigned = crate::diarize::assign_speakers(&mut doc, &segments);
        doc.save(&dir)?;
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
    let dir = resolve_project_root(root).join(&pid);
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
    std::fs::read_dir(home.join(".cache/huggingface/hub"))
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with("models--"))
                .collect()
        })
        .unwrap_or_default()
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
    crate::asr::install_runtime().await
}

/// Download the configured ASR and word-alignment model snapshots.
#[tauri::command]
pub async fn asr_models_download() -> AppResult<crate::asr::RuntimeStatus> {
    crate::asr::download_models().await
}

#[tauri::command]
pub async fn logs_list(pid: String, root: Option<PathBuf>) -> AppResult<Vec<(String, usize)>> {
    let dir = resolve_project_root(root).join(&pid).join("ai");
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
}

#[tauri::command]
pub async fn record_audio(pid: String, seconds: u32, root: Option<PathBuf>) -> AppResult<String> {
    let dir = resolve_project_root(root).join(&pid);
    tokio::fs::create_dir_all(&dir).await?;
    let wav = dir.join("audio.wav");
    let st = tokio::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "avfoundation",
            "-i",
            ":0",
            "-t",
            &seconds.to_string(),
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(&wav)
        .status()
        .await
        .map_err(|e| AppError::Io(std::io::Error::other(format!("ffmpeg: {e}"))))?;
    if st.success() {
        Ok(wav.to_string_lossy().into_owned())
    } else {
        Err(AppError::Schema("ffmpeg recording failed".into()))
    }
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
    Ok(true)
}

/// Run the environment health checks used by the CLI and diagnostics UI.
#[tauri::command]
pub async fn run_doctor() -> AppResult<Vec<crate::doctor::Check>> {
    run_blocking("environment health checks", || Ok(crate::doctor::checks())).await
}

/// Burn-in export: write export.ass then ffmpeg → export.mp4.
#[tauri::command]
pub async fn export_video(pid: String, root: Option<PathBuf>) -> AppResult<String> {
    let dir = resolve_project_root(root).join(&pid);
    let prepare_dir = dir.clone();
    let (doc, cuts, ass, broll) = run_blocking("video export preparation", move || {
        let doc = Doc::load(&prepare_dir)?;
        let cuts_path = prepare_dir.join("cuts.json");
        let cuts: ClipCuts = if cuts_path.exists() {
            serde_json::from_str(&std::fs::read_to_string(cuts_path)?)?
        } else {
            ClipCuts::new()
        };
        let ass = prepare_dir.join("export.ass");
        crate::export::write_ass_with(&doc, &cuts.cuts, &ass, 1920, 1080)?;
        let broll = crate::data::broll::load(&prepare_dir)?;
        Ok((doc, cuts, ass, broll))
    })
    .await?;
    let mp4 = dir.join("export.mp4");
    crate::export::render_video_with_broll(&doc, &cuts.cuts, &ass, &mp4, &broll).await?;
    Ok(mp4.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn export_fcp(pid: String, root: Option<PathBuf>) -> AppResult<String> {
    let dir = resolve_project_root(root).join(&pid);
    let doc = Doc::load(&dir)?;
    let cuts_path = dir.join("cuts.json");
    let cuts: ClipCuts = if cuts_path.exists() {
        serde_json::from_str(&std::fs::read_to_string(cuts_path)?)?
    } else {
        ClipCuts::new()
    };
    let path = dir.join("export.fcpxml");
    let broll = crate::data::broll::load(&dir)?;
    crate::export::write_fcp_with_broll(&doc, &cuts.cuts, &broll, &path, 1920, 1080)?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn export_subtitles(pid: String, root: Option<PathBuf>) -> AppResult<Vec<String>> {
    let dir = resolve_project_root(root).join(&pid);
    let doc = Doc::load(&dir)?;
    let cuts: ClipCuts = std::fs::read_to_string(dir.join("cuts.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    let paths = [
        dir.join("export.srt"),
        dir.join("export.vtt"),
        dir.join("export.ass"),
        dir.join("export.md"),
    ];
    crate::export::write_srt_with(&doc, &cuts.cuts, &paths[0])?;
    crate::export::write_vtt_with(&doc, &cuts.cuts, &paths[1])?;
    crate::export::write_ass_with(&doc, &cuts.cuts, &paths[2], 1920, 1080)?;
    crate::export::write_md_with_chapters(&doc, &cuts.cuts, &dir, &paths[3])?;
    Ok(paths
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect())
}

#[tauri::command]
pub async fn version_list(pid: String, root: Option<PathBuf>) -> AppResult<VersionHistory> {
    let dir = resolve_project_root(root).join(&pid);
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
    let dir = resolve_project_root(root).join(&pid);
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
        crate::data::version::commit_snapshot(
            &dir,
            &doc,
            &mut lineage,
            &branch,
            name,
            note.trim(),
            crate::data::version::VersionKind::Manual,
        )
    })
    .await
}

#[tauri::command]
pub async fn version_restore(pid: String, id: String, root: Option<PathBuf>) -> AppResult<()> {
    let dir = resolve_project_root(root).join(&pid);
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
        crate::data::version::restore_snapshot(&dir, &mut lineage, &id)
    })
    .await
}

#[tauri::command]
pub async fn branch_create(pid: String, name: String, root: Option<PathBuf>) -> AppResult<String> {
    let dir = resolve_project_root(root).join(&pid);
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
        Ok(id)
    })
    .await
}

#[tauri::command]
pub async fn branch_switch(pid: String, id: String, root: Option<PathBuf>) -> AppResult<()> {
    let dir = resolve_project_root(root).join(&pid);
    run_blocking("branch switch", move || {
        let doc = Doc::load(&dir)?;
        if !working_head_is_committed(&dir, &doc)? {
            return Err(AppError::Schema(
                "save the current project as a version before switching branches".into(),
            ));
        }
        let mut lineage = crate::data::version::Lineage::load(&dir)?;
        crate::data::version::switch_branch(&dir, &mut lineage, &id)
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
    let dir = resolve_project_root(root).join(&pid);
    let mut doc = Doc::load(&dir)?;
    let ok = crate::data::edit::split_sentence(&mut doc, &id, at);
    if ok {
        doc.save(&dir)?;
    }
    Ok(ok)
}

#[tauri::command]
pub async fn merge_lines(
    pid: String,
    id1: String,
    id2: String,
    root: Option<PathBuf>,
) -> AppResult<bool> {
    let dir = resolve_project_root(root).join(&pid);
    let mut doc = Doc::load(&dir)?;
    let ok = crate::data::edit::merge_sentences(&mut doc, &id1, &id2);
    if ok {
        doc.save(&dir)?;
    }
    Ok(ok)
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
}

#[tauri::command]
pub async fn style_get(
    pid: String,
    root: Option<PathBuf>,
) -> AppResult<crate::data::substyle::SubStyle> {
    let dir = resolve_project_root(root).join(&pid);
    Ok(crate::data::substyle::SubStyle::load_or_default(&dir))
}

#[tauri::command]
pub async fn style_set(
    pid: String,
    style: crate::data::substyle::SubStyle,
    root: Option<PathBuf>,
) -> AppResult<()> {
    let dir = resolve_project_root(root).join(&pid);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join("style.json"),
        serde_json::to_string_pretty(&style)?,
    )?;
    Ok(())
}

#[tauri::command]
pub async fn config_show() -> crate::data::modelconfig::ModelConfig {
    crate::data::modelconfig::load()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_constant_is_nonempty() {
        assert!(!VERSION.is_empty());
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

        let starred = project_set_star("older".into(), true, Some(tmp.path().to_path_buf()))
            .await
            .unwrap();
        assert!(starred.starred);
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
            "asrModel",
            "asrAligner",
            "diarizeModel",
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
            "llm_api_key": "k",
            "llm_model": "m",
            "worker_count": 2
        }))
        .unwrap();
        assert_eq!(back.worker_count, 2);
        assert_eq!(back.llm_model, "m");
    }

    #[test]
    fn write_settings_file_emits_camel_case_json() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_settings_file(tmp.path(), &settings()).unwrap();
        let raw = std::fs::read_to_string(path).unwrap();
        assert!(raw.contains("\"llmEndpoint\""), "got: {raw}");
        assert!(raw.contains("\"llmApiKey\""), "got: {raw}");
        assert!(raw.contains("\"llmModel\""), "got: {raw}");
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
        assert_eq!(status.kinds[0].done, 1);
        assert_eq!(status.kinds[0].failed, 1);
        assert_eq!(status.kinds[0].pending, 0);
        assert_eq!(status.pending, 0);
        assert_eq!(
            status.kinds[0].last_error.as_deref(),
            Some("provider rejected the request")
        );
    }
}
