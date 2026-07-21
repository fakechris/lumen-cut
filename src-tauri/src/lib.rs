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
pub mod pipeline;
pub mod proc;

pub use commands::greet;
pub use error::{AppError, AppResult};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Tauri runtime entry point.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::AgentServerState::default())
        .manage(commands::RecordingState::default())
        .manage(commands::MediaAssetState::default())
        .manage(commands::TranscriptionState::default())
        .invoke_handler(tauri::generate_handler![
            commands::greet,
            commands::pick_media_file,
            commands::project_create,
            commands::project_show,
            commands::project_list,
            commands::project_update_meta,
            commands::project_reveal,
            commands::project_delete,
            commands::media_asset_allow,
            commands::run_auto,
            commands::transcription_start,
            commands::transcription_status,
            commands::transcription_cancel,
            commands::task_start,
            commands::task_status,
            commands::finish_check_pid,
            commands::cut_auto,
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
            commands::subtitle_visibility,
            commands::subtitle_replace,
            commands::speakers_list,
            commands::speaker_rename,
            commands::speaker_merge,
            commands::broll_list,
            commands::broll_preview,
            commands::diarize_pid,
            commands::timing_repair,
            commands::model_list,
            commands::logs_list,
            commands::record_audio,
            commands::recording_start,
            commands::recording_stop,
            commands::recording_cancel,
            commands::run_doctor,
            commands::export_video,
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
            commands::settings_export,
        ])
        .run(tauri::generate_context!())
        .expect("error while running lumen-cut");
}

/// Library entry point used by the CLI bin. The bin owns its own clap
/// parser and dispatches from there; this re-export keeps the public
/// API symmetric between Tauri and CLI. Returns `Ok(())` because the
/// bin is a separate compilation unit; calling here from the lib is
/// only useful for embedded usages.
pub fn lib_run_cli() -> AppResult<()> {
    Ok(())
}
