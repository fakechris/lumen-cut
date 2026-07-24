//! `lumen-cut` — Type B talking-head video editor.
//!
//! Public re-exports for both the Tauri runtime (`run`) and the CLI
//! (`lib_run_cli`). Stage 4 adds agent / pipeline / audit modules on
//! top of the Stage-3 `asr` + `diarize` + `export` types.

pub mod agent;
pub mod asr;
pub mod audit;
pub mod commands;
pub mod data;
pub mod diarize;
pub mod doctor;
pub mod error;
pub mod export;
pub mod media;
pub mod media_url;
pub mod performance;
pub mod pipeline;
pub mod proc;

pub use commands::greet;
pub use error::{AppError, AppResult};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Persistent diagnostics for GUI launches, which otherwise have no terminal.
/// Keep this outside project folders so exports and repositories never pick it up.
pub fn log_directory() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(".lumen-cut/logs")
}

/// Tauri runtime entry point.
pub fn run() {
    // Apps launched from Finder inherit a minimal PATH that omits Homebrew and
    // user tools. Normalize it before any ffmpeg/Python health check or job.
    doctor::configure_process_path();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::AgentServerState::default())
        .manage(commands::RecordingState::default())
        .manage(commands::MediaAssetState::default())
        .manage(commands::BrollPreviewState::default())
        .manage(commands::TranscriptionState::default())
        .manage(commands::SpeakerAnalysisState::default())
        .manage(commands::VideoExportState::default())
        .manage(commands::SetupJobState::default())
        .invoke_handler(tauri::generate_handler![
            commands::greet,
            commands::pick_media_file,
            commands::pick_broll_file,
            commands::pick_audio_file,
            commands::project_create,
            commands::project_show,
            commands::project_pending_open_take,
            commands::project_list,
            commands::project_search,
            commands::project_set_star,
            commands::project_mark_opened,
            commands::project_update_meta,
            commands::project_reveal,
            commands::project_delete,
            commands::project_media_status,
            commands::project_media_relink,
            commands::project_thumbnail,
            commands::media_asset_allow,
            commands::broll_asset_allow,
            commands::audio_asset_allow,
            commands::timeline_visuals,
            commands::title_list,
            commands::title_add,
            commands::title_update,
            commands::title_remove,
            commands::audio_mix_get,
            commands::audio_mix_set,
            commands::export_settings_get,
            commands::export_settings_set,
            commands::export_preflight,
            commands::run_auto,
            commands::transcription_start,
            commands::transcription_status,
            commands::transcription_cancel,
            commands::transcription_retry,
            commands::task_start,
            commands::task_resume,
            commands::task_pause,
            commands::task_retry,
            commands::task_prioritize,
            commands::task_status,
            commands::finish_check_pid,
            commands::cut_auto,
            commands::cut_manual,
            commands::cut_manual_many,
            commands::cut_restore,
            commands::cut_list,
            commands::audit_pid,
            commands::version_merge,
            commands::agent_serve,
            commands::agent_enqueue,
            commands::agent_workers,
            commands::audit_codes,
            commands::subtitle_list,
            commands::subtitle_set,
            commands::subtitle_set_many,
            commands::subtitle_update_many,
            commands::subtitle_timing_set,
            commands::chapter_list,
            commands::chapter_set_many,
            commands::translation_set,
            commands::translation_set_many,
            commands::subtitle_visibility,
            commands::subtitle_replace,
            commands::edit_history_status,
            commands::edit_undo,
            commands::edit_redo,
            commands::speakers_list,
            commands::speaker_evidence,
            commands::speaker_rename,
            commands::speaker_merge,
            commands::speaker_assign,
            commands::speaker_reidentify_preview,
            commands::speaker_reidentify_start,
            commands::speaker_reidentify_status,
            commands::speaker_reidentify_cancel,
            commands::speaker_reidentify_apply,
            commands::broll_list,
            commands::broll_add,
            commands::broll_accept_suggestion,
            commands::broll_update,
            commands::broll_remove,
            commands::broll_preview,
            commands::broll_preview_start,
            commands::broll_preview_status,
            commands::broll_preview_cancel,
            commands::diarize_pid,
            commands::timing_repair,
            commands::model_list,
            commands::asr_status,
            commands::asr_runtime_install,
            commands::asr_models_download,
            commands::diarize_runtime_install,
            commands::diarize_model_download,
            commands::setup_job_start,
            commands::setup_job_status,
            commands::setup_job_cancel,
            commands::logs_list,
            commands::logs_reveal,
            commands::recording_start,
            commands::recording_stop,
            commands::recording_cancel,
            commands::run_doctor,
            commands::performance_status,
            commands::export_video,
            commands::video_export_start,
            commands::video_export_status,
            commands::video_export_cancel,
            commands::export_fcp,
            commands::export_subtitles,
            commands::version_list,
            commands::version_commit,
            commands::version_restore,
            commands::branch_create,
            commands::branch_switch,
            commands::split_line,
            commands::merge_lines,
            commands::style_get,
            commands::style_set,
            commands::config_show,
            commands::llm_models_list,
            commands::asr_models_list,
            commands::settings_export,
        ])
        .build(tauri::generate_context!())
        .expect("error while building lumen-cut");
    app.run(|_app, event| {
        if matches!(
            event,
            tauri::RunEvent::Exit | tauri::RunEvent::ExitRequested { .. }
        ) {
            proc::terminate_all_processes();
        }
    });
}

/// Library entry point used by the CLI bin. The bin owns its own clap
/// parser and dispatches from there; this re-export keeps the public
/// API symmetric between Tauri and CLI. Returns `Ok(())` because the
/// bin is a separate compilation unit; calling here from the lib is
/// only useful for embedded usages.
pub fn lib_run_cli() -> AppResult<()> {
    Ok(())
}
