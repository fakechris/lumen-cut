//! Tauri IPC commands.
//!
//! Stage 5 wires every Stage-3 + Stage-4 entry point into a `#[tauri::command]`
//! so the React frontend can drive the editor in-process.

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

use crate::VERSION;

use crate::agent::Allocator;
use crate::audit::{audit_project, finish_check_emit_for_project, Code, Finding, Report};
use crate::data::version::{three_way_merge, working_head_is_committed};
use crate::data::{ClipCuts, Doc, MediaRef, Meta};
use crate::error::{AppError, AppResult};
use crate::export::{write_ass, write_md, write_srt, write_vtt};
use crate::media::{extract_audio_wav, probe};

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
pub fn greet() -> Greet {
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
pub fn pick_media_file(app: tauri::AppHandle) -> AppResult<Option<String>> {
    let selected = app
        .dialog()
        .file()
        .add_filter(
            "Audio and video",
            &[
                "mp4", "mov", "m4v", "mkv", "webm", "mp3", "m4a", "wav", "aac", "flac", "aiff",
            ],
        )
        .blocking_pick_file();
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
    pub path: PathBuf,
    pub duration_seconds: f64,
    pub word_count: usize,
    pub paragraph_count: usize,
}

#[tauri::command]
pub async fn project_create(args: CreateProjectArgs) -> AppResult<ProjectSummary> {
    use chrono::Utc;
    let media_path = std::fs::canonicalize(&args.from)?;
    let info = probe(&media_path).await?;
    let root = resolve_project_root(args.root.clone());
    std::fs::create_dir_all(&root)?;
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
    doc.save(&dir)?;
    Ok(ProjectSummary {
        pid: args.pid,
        title: doc.meta.title,
        path: dir,
        duration_seconds: info.duration_seconds,
        word_count: 0,
        paragraph_count: 0,
    })
}

#[tauri::command]
pub fn project_show(pid: String, root: Option<PathBuf>) -> AppResult<Doc> {
    Doc::load(&resolve_project_dir(&pid, root)?)
}

#[tauri::command]
pub fn project_list(root: Option<PathBuf>) -> AppResult<Vec<ProjectSummary>> {
    let root = resolve_project_root(root);
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    let entries = std::fs::read_dir(&root)?;
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        if Doc::load(&p).is_ok() {
            if let Ok(doc) = Doc::load(&p) {
                let name = p
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                out.push(ProjectSummary {
                    pid: name,
                    title: doc.meta.title.clone(),
                    path: p,
                    duration_seconds: doc.media.duration_seconds,
                    word_count: doc.all_words().len(),
                    paragraph_count: doc.paragraphs.len(),
                });
            }
        }
    }
    Ok(out)
}

#[tauri::command]
pub fn project_update_meta(
    pid: String,
    title: String,
    description: String,
    language: Option<String>,
    root: Option<PathBuf>,
) -> AppResult<Doc> {
    let dir = resolve_project_dir(&pid, root)?;
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
}

#[tauri::command]
pub fn project_reveal(pid: String, root: Option<PathBuf>) -> AppResult<String> {
    let dir = resolve_project_dir(&pid, root)?;
    if !dir.is_dir() {
        return Err(AppError::ProjectNotFound(dir));
    }
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .args(["-R"])
        .arg(dir.join("doc.json"))
        .spawn()?;
    #[cfg(not(target_os = "macos"))]
    std::process::Command::new("open").arg(&dir).spawn()?;
    Ok(dir.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn project_delete(
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
    if !dir.exists() {
        return Ok(false);
    }
    std::fs::remove_dir_all(dir)?;
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
pub fn media_asset_allow(
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
    std::fs::create_dir_all(&out_dir)?;

    let requested_pid = args.pid.filter(|pid| !pid.trim().is_empty());
    let download_dir = requested_pid
        .as_ref()
        .map(|pid| out_dir.join(pid))
        .unwrap_or_else(|| out_dir.clone());
    std::fs::create_dir_all(&download_dir)?;
    let media_path = if args.media.starts_with("http://") || args.media.starts_with("https://") {
        report("downloading", 12);
        crate::media_url::download(&args.media, &download_dir.join("source.%(ext)s")).await?
    } else {
        PathBuf::from(&args.media)
    };
    ensure_not_cancelled()?;
    if !media_path.exists() {
        return Err(AppError::ProjectNotFound(media_path));
    }
    let media_path = std::fs::canonicalize(media_path)?;

    let pid_stem = requested_pid.unwrap_or_else(|| {
        media_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".to_string())
    });
    let pid_dir = out_dir.join(&pid_stem);
    std::fs::create_dir_all(&pid_dir)?;
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

    let model_config = crate::data::modelconfig::load();
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
    doc.meta.language = args.lang.clone();
    doc.meta.updated_at = chrono::Utc::now();
    doc.save(&pid_dir)?;

    report("exporting", 94);
    let srt = pid_dir.join("out.srt");
    let vtt = pid_dir.join("out.vtt");
    let ass = pid_dir.join("out.ass");
    let md = pid_dir.join("out.md");
    write_srt(&doc, &srt)?;
    write_vtt(&doc, &vtt)?;
    write_ass(&doc, &ass, 1920, 1080)?;
    write_md(&doc, &md)?;

    let word_count = doc.all_words().len();
    let paragraph_count = doc.paragraphs.len();
    report("completed", 100);
    Ok(AutoResult {
        pid_dir,
        srt,
        vtt,
        ass,
        md,
        word_count,
        paragraph_count,
    })
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
pub fn transcription_start(
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
pub fn transcription_status(
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
pub fn transcription_cancel(
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
    let task = crate::agent::task::prepare_task_with_task_options(
        &dir,
        &args.kind,
        args.lang.as_deref(),
        crate::agent::task::TaskOptions {
            stale_only: args.stale_only,
            groups: args.groups,
            align_fit: args.align_fit,
        },
    )?;
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
pub fn task_status(pid: String, root: Option<PathBuf>) -> AppResult<TaskStatus> {
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
pub fn finish_check_pid(pid: String, root: Option<PathBuf>) -> AppResult<Vec<FinishCheckItem>> {
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
pub fn cut_auto(pid: String, root: Option<PathBuf>) -> AppResult<usize> {
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
pub fn cut_restore(pid: String, cut_id: String, root: Option<PathBuf>) -> AppResult<bool> {
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
pub fn cut_list(pid: String, root: Option<PathBuf>) -> AppResult<Vec<CutSummary>> {
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
/// to `~/.lumen-cut/settings.json` in camelCase — the worker script matches
/// on `llmEndpoint`/`llmApiKey`/`llmModel`/`workerCount`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct SettingsPayload {
    pub llm_endpoint: String,
    pub llm_api_key: String,
    pub llm_model: String,
    pub worker_count: u32,
}

#[tauri::command]
pub fn settings_export(
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
pub fn audit_pid(pid: String, root: Option<PathBuf>) -> AppResult<ReportSummary> {
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
pub fn version_merge(
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
}

struct RecordingSession {
    pid: String,
    wav: PathBuf,
    child: Child,
}

impl Default for RecordingState {
    fn default() -> Self {
        Self {
            session: Mutex::new(None),
        }
    }
}

impl Drop for RecordingState {
    fn drop(&mut self) {
        if let Ok(slot) = self.session.get_mut() {
            if let Some(session) = slot.as_mut() {
                let _ = session.child.kill();
                let _ = session.child.wait();
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
pub fn agent_enqueue(
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
pub fn agent_workers(
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
pub fn audit_codes() -> Vec<&'static str> {
    // Stable public audit-code labels in display order.
    Code::all().iter().map(|c| c.label()).collect()
}

// ============================================================================
// Project editing and export commands
// ============================================================================

#[tauri::command]
pub fn subtitle_list(
    pid: String,
    root: Option<PathBuf>,
) -> AppResult<Vec<crate::data::subtitle::SubtitleRow>> {
    let dir = resolve_project_root(root).join(&pid);
    let doc = Doc::load(&dir)?;
    let hidden = crate::data::subtitle::load_hidden(&dir);
    Ok(crate::data::subtitle::list(&doc, &hidden, None))
}

#[tauri::command]
pub fn subtitle_set(
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
pub fn subtitle_visibility(
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
pub fn subtitle_replace(
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
pub fn speakers_list(
    pid: String,
    root: Option<PathBuf>,
) -> AppResult<Vec<crate::data::speakers::SpeakerInfo>> {
    let dir = resolve_project_root(root).join(&pid);
    let doc = Doc::load(&dir)?;
    Ok(crate::data::speakers::list(&doc))
}

#[tauri::command]
pub fn speaker_rename(
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
pub fn speaker_merge(
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
pub fn broll_list(pid: String, root: Option<PathBuf>) -> AppResult<BrollOverview> {
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
    let doc = Doc::load(&dir)?;
    let cuts: ClipCuts = std::fs::read_to_string(dir.join("cuts.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    let placements = crate::data::broll::load(&dir)?;
    if placements.is_empty() {
        return Ok(Vec::new());
    }
    let ass = dir.join("broll-preview.ass");
    crate::export::write_ass_with(&doc, &cuts.cuts, &ass, 1920, 1080)?;
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
    let mut doc = Doc::load(&dir)?;
    let wav = dir.join("audio.wav");
    if !wav.exists() {
        extract_audio_wav(&doc.media.path, &wav).await?;
    }
    let model = crate::data::modelconfig::load().diarize_model;
    let out = crate::diarize::diarize_file_with_model(&wav, &model).await?;
    let paragraphs_assigned = crate::diarize::assign_speakers(&mut doc, &out.segments);
    doc.save(&dir)?;
    Ok(DiarizeResult {
        segments: out.segments.len(),
        paragraphs_assigned,
    })
}

#[tauri::command]
pub fn timing_repair(pid: String, root: Option<PathBuf>) -> AppResult<String> {
    let dir = resolve_project_root(root).join(&pid);
    let mut doc = Doc::load(&dir)?;
    let rep = crate::pipeline::timing::repair(&mut doc);
    doc.save(&dir)?;
    Ok(format!("{} fix(es)", rep.total()))
}

#[tauri::command]
pub fn model_list() -> Vec<String> {
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

#[tauri::command]
pub fn logs_list(pid: String, root: Option<PathBuf>) -> AppResult<Vec<(String, usize)>> {
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
    std::fs::create_dir_all(&dir)?;
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
pub fn recording_start(
    pid: String,
    root: Option<PathBuf>,
    state: tauri::State<'_, RecordingState>,
) -> AppResult<RecordingStarted> {
    let wav = recording_output(&pid, root)?;
    let mut slot = state.session.lock().expect("recording state poisoned");
    if slot.is_some() {
        return Err(AppError::Schema(
            "another microphone recording is already in progress".into(),
        ));
    }

    if let Some(dir) = wav.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if wav.exists() {
        std::fs::remove_file(&wav)?;
    }

    let mut child = std::process::Command::new("ffmpeg")
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
        .spawn()
        .map_err(|error| {
            AppError::Io(std::io::Error::other(format!(
                "ffmpeg microphone recording: {error}"
            )))
        })?;

    // Missing devices and denied microphone access normally make ffmpeg exit
    // immediately. Catch that here so the UI never displays a false recording
    // state.
    std::thread::sleep(Duration::from_millis(140));
    if let Some(status) = child.try_wait()? {
        let _ = std::fs::remove_file(&wav);
        return Err(AppError::Schema(format!(
            "ffmpeg microphone recording stopped before it started ({status})"
        )));
    }

    *slot = Some(RecordingSession {
        pid: pid.clone(),
        wav: wav.clone(),
        child,
    });
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

fn stop_recording_session(
    mut session: RecordingSession,
) -> AppResult<(PathBuf, std::process::ExitStatus)> {
    if let Some(mut stdin) = session.child.stdin.take() {
        let _ = stdin.write_all(b"q\n");
        let _ = stdin.flush();
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = session.child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            session.child.kill()?;
            break session.child.wait()?;
        }
        std::thread::sleep(Duration::from_millis(40));
    };
    Ok((session.wav, status))
}

fn finalize_recording(session: RecordingSession) -> AppResult<PathBuf> {
    let (wav, status) = stop_recording_session(session)?;
    if !status.success() || !wav.exists() || std::fs::metadata(&wav)?.len() <= 44 {
        let _ = std::fs::remove_file(&wav);
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
    let wav = tokio::task::spawn_blocking(move || finalize_recording(session))
        .await
        .map_err(|error| AppError::Schema(format!("recording stop task failed: {error}")))??;
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
    tokio::task::spawn_blocking(move || {
        let stopped = stop_recording_session(session);
        let _ = std::fs::remove_file(&wav);
        if let Some(dir) = dir {
            let _ = std::fs::remove_dir(dir);
        }
        stopped.map(|_| ())
    })
    .await
    .map_err(|error| AppError::Schema(format!("recording cancel task failed: {error}")))??;
    Ok(true)
}

/// Run the environment health checks used by the CLI and diagnostics UI.
#[tauri::command]
pub fn run_doctor() -> AppResult<Vec<crate::doctor::Check>> {
    Ok(crate::doctor::checks())
}

/// Burn-in export: write export.ass then ffmpeg → export.mp4.
#[tauri::command]
pub async fn export_video(pid: String, root: Option<PathBuf>) -> AppResult<String> {
    let dir = resolve_project_root(root).join(&pid);
    let doc = Doc::load(&dir)?;
    let cuts_path = dir.join("cuts.json");
    let cuts: ClipCuts = if cuts_path.exists() {
        serde_json::from_str(&std::fs::read_to_string(cuts_path)?)?
    } else {
        ClipCuts::new()
    };
    let ass = dir.join("export.ass");
    crate::export::write_ass_with(&doc, &cuts.cuts, &ass, 1920, 1080)?;
    let mp4 = dir.join("export.mp4");
    let broll = crate::data::broll::load(&dir)?;
    crate::export::render_video_with_broll(&doc, &cuts.cuts, &ass, &mp4, &broll).await?;
    Ok(mp4.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn export_fcp(pid: String, root: Option<PathBuf>) -> AppResult<String> {
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
pub fn export_subtitles(pid: String, root: Option<PathBuf>) -> AppResult<Vec<String>> {
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
pub fn version_list(
    pid: String,
    root: Option<PathBuf>,
) -> AppResult<crate::data::version::Lineage> {
    let dir = resolve_project_root(root).join(&pid);
    crate::data::version::Lineage::load(&dir)
}

#[tauri::command]
pub fn version_commit(
    pid: String,
    name: String,
    note: String,
    root: Option<PathBuf>,
) -> AppResult<String> {
    let dir = resolve_project_root(root).join(&pid);
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
        &name,
        &note,
        crate::data::version::VersionKind::Manual,
    )
}

#[tauri::command]
pub fn version_restore(pid: String, id: String, root: Option<PathBuf>) -> AppResult<()> {
    let dir = resolve_project_root(root).join(&pid);
    let mut lineage = crate::data::version::Lineage::load(&dir)?;
    crate::data::version::restore_snapshot(&dir, &mut lineage, &id)
}

#[tauri::command]
pub fn branch_create(pid: String, name: String, root: Option<PathBuf>) -> AppResult<String> {
    let dir = resolve_project_root(root).join(&pid);
    let mut lineage = crate::data::version::Lineage::load(&dir)?;
    crate::data::version::create_branch(&dir, &mut lineage, &name, "")
}

#[tauri::command]
pub fn branch_switch(pid: String, id: String, root: Option<PathBuf>) -> AppResult<()> {
    let dir = resolve_project_root(root).join(&pid);
    let mut lineage = crate::data::version::Lineage::load(&dir)?;
    crate::data::version::switch_branch(&dir, &mut lineage, &id)
}

// ---- line editing, style, and cloud configuration ----

#[tauri::command]
pub fn split_line(pid: String, id: String, at: usize, root: Option<PathBuf>) -> AppResult<bool> {
    let dir = resolve_project_root(root).join(&pid);
    let mut doc = Doc::load(&dir)?;
    let ok = crate::data::edit::split_sentence(&mut doc, &id, at);
    if ok {
        doc.save(&dir)?;
    }
    Ok(ok)
}

#[tauri::command]
pub fn merge_lines(
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

#[tauri::command]
pub fn style_get(pid: String, root: Option<PathBuf>) -> AppResult<crate::data::substyle::SubStyle> {
    let dir = resolve_project_root(root).join(&pid);
    Ok(crate::data::substyle::SubStyle::load_or_default(&dir))
}

#[tauri::command]
pub fn style_set(
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
pub fn config_show() -> crate::data::modelconfig::ModelConfig {
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
    fn greet_returns_ready() {
        let g = greet();
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

    #[test]
    fn project_metadata_update_is_trimmed_and_persisted() {
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
        .is_err());
    }

    fn settings() -> SettingsPayload {
        SettingsPayload {
            llm_endpoint: "http://localhost:11434/v1/chat/completions".into(),
            llm_api_key: "sk-test".into(),
            llm_model: "gpt-4o-mini".into(),
            worker_count: 7,
        }
    }

    #[test]
    fn settings_serializes_camel_case_keys() {
        let v = serde_json::to_value(settings()).unwrap();
        let obj = v.as_object().unwrap();
        for k in ["llmEndpoint", "llmApiKey", "llmModel", "workerCount"] {
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

    #[test]
    fn task_status_exposes_polish_quality_artifact() {
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

        let status = task_status("p1".into(), Some(tmp.path().to_path_buf())).unwrap();
        assert_eq!(
            status.polish_quality.as_ref().unwrap().status,
            crate::pipeline::polish::PolishQualityStatus::Warn
        );
        let json = serde_json::to_value(&status).unwrap();
        assert!(json.get("polishQuality").is_some());
    }

    #[test]
    fn task_status_reports_each_kind_and_failure_count() {
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

        let status = task_status("p1".into(), Some(tmp.path().to_path_buf())).unwrap();
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
