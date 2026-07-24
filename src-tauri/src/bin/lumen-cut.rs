//! `lumen-cut` CLI entry point.
//!
//! Stage 4 surface — `auto` + `task`/`align`/`diarize`/`finish-check`/`cut`/`version`/`audit`.
//!
//! Examples:
//!   lumen-cut auto samples/demo.mp4 --source-lang en
//!   lumen-cut auto samples/demo.mp4 --source-lang en --lang zh --no-polish
//!   lumen-cut auto samples/demo.mp4 --lang zh --rough-cut
//!   lumen-cut project create demo --from samples/demo.mp4
//!   lumen-cut task start translate demo --lang en
//!   lumen-cut task start align demo --lang zh --groups g1,g2 --align-fit 16
//!   lumen-cut align list demo --lang zh --fit 16
//!   lumen-cut diarize demo
//!   lumen-cut finish-check demo --strict
//!   lumen-cut cut demo --auto
//!   lumen-cut cut demo --list --kind filler
//!   lumen-cut cut demo --add --start 1.0 --end 2.5 --note "manual"
//!   lumen-cut export demo --srt --bilingual --lang zh -o out.srt
//!   lumen-cut version demo list
//!   lumen-cut audit demo

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use clap::{Parser, Subcommand};
use serde::Serialize;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use lumen_cut::asr::{transcribe_file_with_aligner_progress, AsrProgress, AsrProgressCallback};
use lumen_cut::audit::{audit_project, finish_check_emit_for_project, Finding, Report};
use lumen_cut::data::version::{
    commit_snapshot, create_branch, restore_snapshot, switch_branch, three_way_merge,
    working_head_is_committed, Lineage, VersionKind,
};
use lumen_cut::data::ClipCuts;
use lumen_cut::data::{Doc, MediaRef, Meta};
use lumen_cut::diarize::{
    assign_speakers, diarize_file_with_model_progress, proposals_from_segments, DiarizeProgress,
    DiarizeProgressCallback,
};
use lumen_cut::error::{AppError, AppResult};
use lumen_cut::export::{
    write_ass, write_ass_with_style, write_md, write_md_with_chapters, write_srt_with,
    write_vtt_with,
};
use lumen_cut::media::{extract_audio_wav, probe};
use lumen_cut::media_url::download;
use lumen_cut::pipeline::align_list;

macro_rules! emit {
    ($json:expr, $value:expr, $($human:tt)+) => {
        if $json {
            println!("{}", serde_json::to_string(&$value)?);
        } else {
            println!($($human)+);
        }
    };
}

const CLI_PROGRESS_STEP: u8 = 5;

#[derive(Default)]
struct CliProgressState {
    phase: String,
    progress: u8,
    emitted: bool,
}

fn should_emit_cli_progress(state: &mut CliProgressState, phase: &str, progress: u8) -> bool {
    let progress = progress.min(100);
    let phase_changed = state.phase != phase;
    let advanced = progress >= state.progress.saturating_add(CLI_PROGRESS_STEP);
    let emit = !state.emitted || phase_changed || advanced || progress == 100;
    if emit {
        state.phase = phase.to_string();
        state.progress = progress;
        state.emitted = true;
    }
    emit
}

fn format_cli_progress(
    phase: &str,
    progress: u8,
    current: Option<u32>,
    total: Option<u32>,
    device: Option<&str>,
    cpu_percent: Option<u32>,
    peak_memory_mb: Option<u64>,
) -> String {
    let mut details = Vec::new();
    if let (Some(current), Some(total)) = (current, total) {
        details.push(format!("{current}/{total}"));
    }
    if let Some(device) = device.filter(|value| !value.trim().is_empty()) {
        details.push(device.to_string());
    }
    if let Some(cpu) = cpu_percent {
        details.push(format!("CPU {cpu}%"));
    }
    if let Some(memory) = peak_memory_mb {
        details.push(format!("memory {memory} MB"));
    }
    let suffix = if details.is_empty() {
        String::new()
    } else {
        format!(" · {}", details.join(" · "))
    };
    format!(
        "progress: {} {}%{}",
        phase.replace(['_', '-'], " "),
        progress.min(100),
        suffix
    )
}

fn report_cli_phase(phase: &str, progress: u8) {
    eprintln!(
        "{}",
        format_cli_progress(phase, progress, None, None, None, None, None)
    );
}

fn cli_asr_progress() -> AsrProgressCallback {
    let state = Arc::new(Mutex::new(CliProgressState::default()));
    Arc::new(move |progress: AsrProgress| {
        let Ok(mut state) = state.lock() else {
            return;
        };
        if should_emit_cli_progress(&mut state, &progress.phase, progress.progress) {
            eprintln!(
                "{}",
                format_cli_progress(
                    &progress.phase,
                    progress.progress,
                    progress.current,
                    progress.total,
                    progress.device.as_deref(),
                    progress.cpu_percent,
                    progress.peak_memory_mb,
                )
            );
        }
    })
}

fn cli_diarize_progress() -> DiarizeProgressCallback {
    let state = Arc::new(Mutex::new(CliProgressState::default()));
    Arc::new(move |progress: DiarizeProgress| {
        let Ok(mut state) = state.lock() else {
            return;
        };
        if should_emit_cli_progress(&mut state, &progress.phase, progress.progress) {
            eprintln!(
                "{}",
                format_cli_progress(
                    &progress.phase,
                    progress.progress,
                    progress.current,
                    progress.total,
                    progress.device.as_deref(),
                    progress.cpu_percent,
                    progress.peak_memory_mb,
                )
            );
        }
    })
}

#[derive(Parser, Debug)]
#[command(version, about = "Open-source talking-head video editor")]
struct Cli {
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    Project {
        #[command(subcommand)]
        action: ProjectCmd,
    },
    /// Pipeline: media → audio → ASR → doc → optional polish/translate/align/cleanup.
    ///
    /// ASR-only (default): `auto media [--source-lang|--lang L]`.
    /// With translation: set `--lang` as the target together with either
    /// `--source-lang`, `--no-polish`, or `--rough-cut` so the one-shot
    /// agent stages run after transcription.
    Auto {
        media: String,
        /// Translate target language when multi-stage pipeline is enabled.
        /// For ASR-only runs (no translate/rough-cut intent), also used as
        /// the transcription language for backward compatibility.
        #[arg(long)]
        lang: Option<String>,
        /// Transcription / source language (preferred over `--lang` for ASR).
        #[arg(long)]
        source_lang: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        model: Option<String>,
        /// Skip polish before translate/cleanup stages (polish runs by default
        /// whenever those stages are selected).
        #[arg(long, default_value_t = false)]
        no_polish: bool,
        /// After optional polish/translate, run soft-cut detection and apply.
        #[arg(long, default_value_t = false)]
        rough_cut: bool,
        /// Override one-line fit capacity for the align stage (8..32).
        #[arg(long)]
        align_fit: Option<usize>,
        /// Translate only groups whose source text changed.
        #[arg(long, default_value_t = false)]
        stale_only: bool,
    },
    /// Drive one of the eight agent task contracts.
    Task {
        #[command(subcommand)]
        action: TaskCmd,
    },
    /// Inspect over-FIT translation groups before a targeted align task.
    Align {
        #[command(subcommand)]
        action: AlignCmd,
    },
    /// Aggregate audit blockers/warnings into a readiness verdict.
    FinishCheck {
        pid: String,
        #[arg(long)]
        strict: bool,
    },
    /// Soft-cut detect / list / add / restore.
    ///
    /// Requires exactly one action: `--auto`/`--detect`, `--list`, `--add`,
    /// or `--restore` / `--restore-all`.
    Cut {
        pid: String,
        /// Apply deterministic detect (alias of `--detect`).
        #[arg(long, alias = "detect")]
        auto: bool,
        /// List cuts (optionally filter with `--kind`).
        #[arg(long, default_value_t = false)]
        list: bool,
        /// Filter `--list` by kind: silence|filler|retake|falsestart|badtake|manual.
        #[arg(long)]
        kind: Option<String>,
        /// Add a manual cut from `--start/--end` seconds or `--words a..b`.
        #[arg(long, default_value_t = false)]
        add: bool,
        #[arg(long)]
        start: Option<f64>,
        #[arg(long)]
        end: Option<f64>,
        /// Inclusive word-id span, e.g. `w1..w4` or `w1,w4`.
        #[arg(long)]
        words: Option<String>,
        #[arg(long)]
        note: Option<String>,
        /// Restore (remove) one cut by id.
        #[arg(long)]
        restore: Option<String>,
        /// Remove every cut.
        #[arg(long, default_value_t = false)]
        restore_all: bool,
        /// With `--auto`/`--detect`, only print proposals without writing.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Minimum pause (seconds) treated as compressible silence.
        #[arg(long, default_value_t = 0.8)]
        min_pause: f64,
        /// Surviving pause length for an intra-sentence silence (seconds).
        #[arg(long, default_value_t = 0.3)]
        compress_to: f64,
        /// Gaps longer than this (seconds) are protected as deliberate beats.
        #[arg(long, default_value_t = 3.0)]
        max_gap: f64,
        /// Skip Category-1 filler hard cuts.
        #[arg(long, default_value_t = false)]
        no_fillers: bool,
        /// Skip silence-compression proposals.
        #[arg(long, default_value_t = false)]
        no_pauses: bool,
    },
    /// Version control: list / 3-way merge / dump.
    Version {
        #[command(subcommand)]
        action: VersionCmd,
    },
    /// Run the project delivery audit.
    Audit { pid: String },
    /// Export subtitles with soft-cut retime applied, and optional video/FCP.
    ///
    /// Format flags (`--srt/--vtt/--ass/--markdown/--video/--fcp`) select
    /// outputs. With none set, all timed-text formats are written under the
    /// project directory (legacy default).
    Export {
        pid: String,
        #[arg(long)]
        video: bool,
        #[arg(long)]
        fcp: bool,
        #[arg(long)]
        srt: bool,
        #[arg(long)]
        vtt: bool,
        #[arg(long)]
        ass: bool,
        #[arg(long)]
        markdown: bool,
        /// Translation-only captions (requires `--lang`).
        #[arg(long, default_value_t = false)]
        translated: bool,
        /// Source + translation captions (requires `--lang`).
        #[arg(long, default_value_t = false)]
        bilingual: bool,
        /// Caption language for translated/bilingual modes.
        #[arg(long)]
        lang: Option<String>,
        /// Include speaker labels in markdown when available.
        #[arg(long, default_value_t = false)]
        speakers: bool,
        /// Output path for a single selected format, or directory for multi.
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,
        /// Clip export to this source-timeline start (seconds).
        #[arg(long)]
        start: Option<f64>,
        /// Clip export to this source-timeline end (seconds).
        #[arg(long)]
        end: Option<f64>,
    },
    /// Speaker diarization: pyannote sidecar → doc.json `speaker` fields.
    Diarize { pid: String },
    /// Branch history: list/create/switch/delete.
    Branch {
        #[command(subcommand)]
        action: BranchCmd,
    },
    /// Subtitle operations: list/set/find/hide/restore.
    Subtitle {
        pid: String,
        #[command(subcommand)]
        action: SubtitleCmd,
    },
    /// Speaker operations: show/view/rename/merge/reidentify.
    Speakers {
        pid: String,
        #[command(subcommand)]
        action: SpeakersCmd,
    },
    /// B-roll operations: list/add/preview/remove/update.
    Broll {
        pid: String,
        #[command(subcommand)]
        action: BrollCmd,
    },
    /// Repair invalid or overlapping word timings.
    Timing {
        pid: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// List or download local models.
    Model {
        #[command(subcommand)]
        action: ModelCmd,
    },
    /// Run environment health checks.
    Doctor,
    /// Inspect project task logs.
    Logs {
        pid: String,
        #[arg(long)]
        kind: Option<String>,
    },
    /// MCP server (Claude-native tool surface over stdio JSON-RPC).
    Mcp {
        #[command(subcommand)]
        action: McpCmd,
    },
    /// Record audio (macOS avfoundation via ffmpeg) → <pid>/audio.wav.
    Record {
        pid: String,
        #[arg(long, default_value = "30")]
        seconds: u32,
    },
    /// Install / inspect the lumen-cut agent skill (npx skills).
    Skill {
        #[arg(long)]
        install: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum ProjectCmd {
    Create {
        pid: String,
        #[arg(long)]
        from: PathBuf,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    Show {
        pid: String,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Resolve a project path (and deep-link URL) for agents or the desktop app.
    Open {
        pid: String,
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Reveal the project folder in the system file manager (macOS Finder).
        #[arg(long, default_value_t = false)]
        reveal: bool,
        /// Queue the project for the desktop app to open on next launch.
        #[arg(long, default_value_t = false)]
        desktop: bool,
    },
    /// List projects under `root`.
    List {
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Delete a project directory.
    Delete {
        pid: String,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
enum TaskCmd {
    /// Materialize and enqueue an agent task under ai/<kind>/.
    Start {
        kind: String, // "translate" | "polish" | ...
        pid: String,
        #[arg(long)]
        lang: Option<String>,
        /// Translate only groups whose source text changed.
        #[arg(long)]
        stale_only: bool,
        /// Restrict align review to these translation group ids.
        #[arg(long, value_delimiter = ',')]
        groups: Vec<String>,
        /// Override compact-v4 one-line fit capacity (8..32).
        #[arg(long)]
        align_fit: Option<usize>,
        /// Prefer local wrapping for ordinary over-fit align lines.
        #[arg(long, default_value_t = false)]
        align_local: bool,
        /// Second-look policy after align rewrites: semantic|targeted|off.
        #[arg(long, default_value = "semantic")]
        second_look: String,
        /// Skip automatic phase-2 align after translate.
        #[arg(long, default_value_t = false)]
        no_phase2: bool,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Show pending / done counts for `pid`.
    Status {
        pid: String,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Enqueue a task and keep the local claim/submit HTTP server up until
    /// the run completes (or forever with `--hold`). External workers can
    /// claim via `GET /agent/next` and submit via `POST /agent/submit`.
    Serve {
        kind: String,
        pid: String,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long)]
        stale_only: bool,
        #[arg(long, value_delimiter = ',')]
        groups: Vec<String>,
        #[arg(long)]
        align_fit: Option<usize>,
        #[arg(long, default_value_t = false)]
        align_local: bool,
        #[arg(long, default_value = "semantic")]
        second_look: String,
        #[arg(long, default_value_t = false)]
        no_phase2: bool,
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// TCP port (0 = ephemeral).
        #[arg(long, default_value_t = 0)]
        port: u16,
        /// Keep listening after the task reaches a terminal state.
        #[arg(long, default_value_t = false)]
        hold: bool,
    },
}

#[derive(Subcommand, Debug)]
enum AlignCmd {
    /// List only target groups that exceed the one-line fit capacity.
    List {
        pid: String,
        #[arg(long)]
        lang: String,
        #[arg(long)]
        fit: Option<usize>,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
enum VersionCmd {
    List {
        pid: String,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Three-way merge base/ours/theirs. Inputs are JSON files mapping
    /// cue_id → text.
    Merge {
        base: PathBuf,
        ours: PathBuf,
        theirs: PathBuf,
    },
    /// Commit a `doc.json` snapshot under `versions/<id>/` + a lineage node.
    Commit {
        pid: String,
        name: String,
        #[arg(long, default_value = "")]
        note: String,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Restore a version's snapshot back to the working `doc.json`.
    Restore {
        pid: String,
        id: String,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Print lumen-cut version.
    Dump,
}

#[derive(Subcommand, Debug)]
enum BranchCmd {
    List {
        pid: String,
    },
    Create {
        pid: String,
        #[arg(long)]
        name: String,
    },
    Switch {
        pid: String,
        branch_id: String,
    },
    Delete {
        pid: String,
        branch_id: String,
    },
}

#[derive(Subcommand, Debug)]
enum SubtitleCmd {
    List {
        #[arg(long)]
        lang: Option<String>,
    },
    Set {
        id: String,
        #[arg(long)]
        text: String,
    },
    Find {
        query: String,
        #[arg(long)]
        regex: bool,
    },
    Replace {
        query: String,
        replacement: String,
        #[arg(long)]
        regex: bool,
    },
    Hide {
        id: String,
    },
    Restore {
        id: String,
    },
}

#[derive(Subcommand, Debug)]
enum SpeakersCmd {
    Show {
        #[arg(long)]
        turns: Option<u32>,
        #[arg(long)]
        cues: bool,
    },
    View {
        #[arg(long)]
        rerun: bool,
    },
    /// Assign or clear a speaker on one paragraph, cue, or time range.
    Assign {
        /// Label to write. Omit with `--clear` to remove the label.
        #[arg(long)]
        speaker: Option<String>,
        /// Clear the matched speaker label instead of writing a name.
        #[arg(long, default_value_t = false)]
        clear: bool,
        /// Target a single paragraph id.
        #[arg(long)]
        paragraph: Option<u32>,
        /// Target the paragraph that owns this cue/sentence id.
        #[arg(long)]
        cue: Option<String>,
        /// Inclusive start of a media time range (seconds).
        #[arg(long)]
        start: Option<f64>,
        /// Exclusive end of a media time range (seconds).
        #[arg(long)]
        end: Option<f64>,
    },
    Rename {
        sid: String,
        name: String,
    },
    Merge {
        from: String,
        into: String,
    },
    /// Re-run diarization. Default applies immediately; `--review` stores a proposal.
    Reidentify {
        /// Store a non-destructive proposal instead of writing labels.
        #[arg(long, default_value_t = false)]
        review: bool,
    },
    /// Show the stored re-identification proposal, if any.
    Proposals,
    /// Apply the stored re-identification proposal.
    Apply {
        /// Apply only rows where the proposed label differs (default).
        #[arg(long, default_value_t = true)]
        changed_only: bool,
        /// Apply every proposal row, including unchanged labels.
        #[arg(long, default_value_t = false)]
        all: bool,
    },
}

#[derive(Subcommand, Debug)]
enum BrollCmd {
    List,
    Add {
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        at: Option<f64>,
        #[arg(long)]
        start: Option<f64>,
        #[arg(long)]
        end: Option<f64>,
        #[arg(long)]
        dur: Option<f64>,
        #[arg(long)]
        mode: Option<String>,
        #[arg(long)]
        rect: Option<String>,
        #[arg(long)]
        fit: Option<String>,
        #[arg(long)]
        bg: Option<String>,
        #[arg(long)]
        src_start: Option<f64>,
        #[arg(long)]
        radius: Option<u32>,
        #[arg(long)]
        name: Option<String>,
    },
    Preview {
        #[arg(long, value_delimiter = ',')]
        at: Vec<f64>,
    },
    Remove {
        id: String,
    },
    Update {
        id: String,
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long)]
        start: Option<f64>,
        #[arg(long)]
        end: Option<f64>,
        #[arg(long)]
        mode: Option<String>,
        #[arg(long)]
        rect: Option<String>,
        #[arg(long)]
        fit: Option<String>,
        #[arg(long)]
        bg: Option<String>,
        #[arg(long)]
        src_start: Option<f64>,
        #[arg(long)]
        radius: Option<u32>,
        #[arg(long)]
        name: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum ModelCmd {
    List,
    Download { id: String },
}

#[derive(Subcommand, Debug)]
enum McpCmd {
    /// Serve the MCP server over stdio (for Claude / MCP clients).
    Serve,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        // stdout is a public protocol surface (`--json` and MCP stdio).
        // Keep diagnostics on stderr so one log event cannot corrupt it.
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "lumen_cut=info,warn".into()),
        )
        .init();

    if let Err(e) = run_cli().await {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

async fn run_cli() -> AppResult<()> {
    let cli = Cli::parse();
    let json = cli.json;
    match cli.cmd {
        Cmd::Version { action } => match action {
            VersionCmd::List { pid, root } => {
                let dir = root.join(&pid);
                let lineage_path = dir.join("lineage.json");
                if !lineage_path.exists() {
                    emit!(json, serde_json::Value::Null, "(no lineage yet for {pid})");
                } else {
                    let raw = std::fs::read_to_string(&lineage_path)?;
                    println!("{raw}");
                }
            }
            VersionCmd::Merge { base, ours, theirs } => {
                let base_m = read_cue_map(&base)?;
                let ours_m = read_cue_map(&ours)?;
                let theirs_m = read_cue_map(&theirs)?;
                let out = three_way_merge(&base_m, &ours_m, &theirs_m);
                if json {
                    println!("{}", serde_json::to_string(&out)?);
                } else {
                    println!(
                        "merged={} conflicts={}",
                        out.merged.len(),
                        out.conflicts.len()
                    );
                    for c in &out.conflicts {
                        println!(
                            "conflict {}: base={:?} ours={:?} theirs={:?}",
                            c.cue_id, c.base, c.ours, c.theirs
                        );
                    }
                }
            }
            VersionCmd::Commit {
                pid,
                name,
                note,
                root,
            } => {
                let dir = root.join(&pid);
                let doc = Doc::load(&dir)?;
                let mut lineage = Lineage::load(&dir)?;
                let branch = lineage
                    .active_branch
                    .clone()
                    .unwrap_or_else(|| "main".into());
                let id = commit_snapshot(
                    &dir,
                    &doc,
                    &mut lineage,
                    &branch,
                    &name,
                    &note,
                    VersionKind::Manual,
                )?;
                emit!(
                    json,
                    serde_json::json!({"pid": pid, "id": id, "name": name}),
                    "✓ version commit {pid}: {id} ({name})"
                );
            }
            VersionCmd::Restore { pid, id, root } => {
                let dir = root.join(&pid);
                let mut lineage = Lineage::load(&dir)?;
                restore_snapshot(&dir, &mut lineage, &id)?;
                emit!(
                    json,
                    serde_json::json!({"pid": pid, "restored": id}),
                    "✓ version restore {pid}: {id}"
                );
            }
            VersionCmd::Dump => {
                emit!(
                    json,
                    serde_json::json!({"version": lumen_cut::VERSION}),
                    "lumen-cut {}",
                    lumen_cut::VERSION
                );
            }
        },
        Cmd::Project { action } => match action {
            ProjectCmd::Create {
                pid,
                from,
                lang,
                title,
                root,
            } => {
                project_create(&pid, &from, lang.as_deref(), title.as_deref(), &root).await?;
                let dir = root.join(&pid);
                emit!(
                    json,
                    serde_json::json!({"pid": pid, "dir": dir}),
                    "✓ created {pid} → {}",
                    dir.display()
                );
            }
            ProjectCmd::Show { pid, root } => {
                project_show(&pid, &root)?;
            }
            ProjectCmd::Open {
                pid,
                root,
                reveal,
                desktop,
            } => {
                let summary = project_open(&pid, &root, reveal, desktop)?;
                emit!(
                    json,
                    &summary,
                    "✓ project open {pid}: {} ({})",
                    summary.path,
                    summary.url
                );
            }
            ProjectCmd::List { root } => {
                let mut pids: Vec<String> = std::fs::read_dir(&root)
                    .map(|rd| {
                        rd.filter_map(|e| e.ok())
                            .map(|e| e.file_name().to_string_lossy().into_owned())
                            .filter(|n| root.join(n).join("doc.json").exists())
                            .collect()
                    })
                    .unwrap_or_default();
                pids.sort();
                if json {
                    println!("{}", serde_json::to_string(&pids)?);
                } else {
                    for p in &pids {
                        println!("{p}");
                    }
                    println!("({} project(s))", pids.len());
                }
            }
            ProjectCmd::Delete { pid, root } => {
                std::fs::remove_dir_all(root.join(&pid))?;
                emit!(json, serde_json::json!({"deleted": pid}), "✓ deleted {pid}");
            }
        },
        Cmd::Auto {
            media,
            lang,
            source_lang,
            title,
            out,
            model,
            no_polish,
            rough_cut,
            align_fit,
            stale_only,
        } => {
            let result = run_auto(AutoOptions {
                media: &media,
                lang: lang.as_deref(),
                source_lang: source_lang.as_deref(),
                title: title.as_deref(),
                out_dir: out.as_deref(),
                model: model.as_deref(),
                no_polish,
                rough_cut,
                align_fit,
                stale_only,
            })
            .await?;
            emit!(
                json,
                &result,
                "✓ {}: words={} paragraphs={} polish={} translate={} cuts={} → srt + vtt + ass + md",
                result.pid_dir.display(),
                result.words,
                result.paragraphs,
                result.polished,
                result.translated.as_deref().unwrap_or("-"),
                result.cuts_added
            );
        }
        Cmd::Task { action } => match action {
            TaskCmd::Start {
                kind,
                pid,
                lang,
                stale_only,
                groups,
                align_fit,
                align_local,
                second_look,
                no_phase2,
                root,
            } => {
                let n = task_start(
                    &kind,
                    &pid,
                    lang.as_deref(),
                    stale_only,
                    groups,
                    align_fit,
                    align_local,
                    &second_look,
                    !no_phase2,
                    &root,
                )
                .await?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string(&serde_json::json!({"pending": n}))?
                    );
                } else {
                    println!("✓ task start {kind} {pid}: {n} pending");
                }
            }
            TaskCmd::Status { pid, root } => {
                let st = task_status(&pid, &root)?;
                if json {
                    println!("{}", serde_json::to_string(&st)?);
                } else {
                    println!(
                        "pending={} done={} failed={}",
                        st.pending, st.done, st.failed
                    );
                    for kind in &st.kinds {
                        println!(
                            "{} state={} pending={} done={} failed={}{}",
                            kind.kind,
                            kind.state,
                            kind.pending,
                            kind.done,
                            kind.failed,
                            kind.last_error
                                .as_deref()
                                .map(|error| format!(" error={error}"))
                                .unwrap_or_default()
                        );
                    }
                    if let Some(quality) = &st.polish_quality {
                        println!(
                            "polishQuality={:?} measured={}/{} residualTerms={}",
                            quality.status,
                            quality.measured_page_count,
                            quality.page_count,
                            quality.residual_term_variant_count
                        );
                    }
                }
            }
            TaskCmd::Serve {
                kind,
                pid,
                lang,
                stale_only,
                groups,
                align_fit,
                align_local,
                second_look,
                no_phase2,
                root,
                port,
                hold,
            } => {
                let result = task_serve(
                    &kind,
                    &pid,
                    lang.as_deref(),
                    stale_only,
                    groups,
                    align_fit,
                    align_local,
                    &second_look,
                    !no_phase2,
                    &root,
                    port,
                    hold,
                    json,
                )
                .await?;
                if json {
                    println!("{}", serde_json::to_string(&result)?);
                } else {
                    println!(
                        "✓ task serve {kind} {pid}: pending={} applied={} url={}",
                        result.pending, result.applied, result.url
                    );
                }
            }
        },
        Cmd::Align { action } => match action {
            AlignCmd::List {
                pid,
                lang,
                fit,
                root,
            } => {
                let doc = Doc::load(&root.join(&pid))?;
                let list = align_list(
                    &doc,
                    &lang,
                    fit.unwrap_or_else(|| lumen_cut::pipeline::aim_chars_for_lang(&lang)),
                    &pid,
                )?;
                if json {
                    println!("{}", serde_json::to_string(&list)?);
                } else {
                    for group in &list.groups {
                        println!(
                            "{} cells={:.1} fit={} overHard={} :: {}",
                            group.key,
                            group.projected_cells,
                            group.fit_chars,
                            group.over_hard,
                            group.target
                        );
                    }
                    if let Some(next) = &list.next {
                        println!("next: {next}");
                    }
                    println!("({} over-FIT group(s))", list.groups.len());
                }
            }
        },
        Cmd::FinishCheck { pid, strict } => {
            let dir = PathBuf::from(&pid);
            let doc = Doc::load(&dir)?;
            let cuts_path = dir.join("cuts.json");
            let cuts: ClipCuts = if cuts_path.exists() {
                serde_json::from_str(&std::fs::read_to_string(&cuts_path)?)?
            } else {
                ClipCuts::new()
            };
            let broll = lumen_cut::data::broll::load(&dir)?;
            let items = finish_check_emit_for_project(
                &doc,
                &cuts,
                &broll,
                &dir,
                working_head_is_committed(&dir, &doc)?,
            );
            let mut all_pass = true;
            if json {
                println!("{}", serde_json::to_string(&items)?);
                all_pass = items.iter().all(|item| item.pass);
            } else {
                for it in &items {
                    let flag = if it.pass { "PASS" } else { "FAIL" };
                    if !it.pass {
                        all_pass = false;
                    }
                    println!(
                        "{} {} ({}) blockers={}",
                        flag,
                        it.code.label(),
                        it.code as u32,
                        it.blockers.len()
                    );
                    for b in &it.blockers {
                        println!("    - {} @ {}: {}", b.code.label(), b.where_, b.message);
                    }
                }
            }
            if strict && !all_pass {
                // Shell-gate contract: strict failures return Err here and
                // main() maps any Err to exit 1 (non-zero), so
                // `finish-check --strict && export` works as a gate.
                return Err(AppError::Schema("finish-check strict mode failed".into()));
            }
        }
        Cmd::Cut {
            pid,
            auto,
            list,
            kind,
            add,
            start,
            end,
            words,
            note,
            restore,
            restore_all,
            dry_run,
            min_pause,
            compress_to,
            max_gap,
            no_fillers,
            no_pauses,
        } => {
            run_cut_command(CutCommand {
                pid: &pid,
                auto,
                list,
                kind: kind.as_deref(),
                add,
                start,
                end,
                words: words.as_deref(),
                note: note.as_deref(),
                restore: restore.as_deref(),
                restore_all,
                dry_run,
                min_pause,
                compress_to,
                max_gap,
                no_fillers,
                no_pauses,
                json,
            })?;
        }
        Cmd::Audit { pid } => {
            let dir = PathBuf::from(&pid);
            let doc = Doc::load(&dir)?;
            let cuts: ClipCuts = std::fs::read_to_string(dir.join("cuts.json"))
                .ok()
                .and_then(|raw| serde_json::from_str(&raw).ok())
                .unwrap_or_default();
            let broll = lumen_cut::data::broll::load(&dir)?;
            let r = audit_project(&doc, &cuts, &broll, &dir);
            if json {
                println!("{}", serde_json::to_string(&r)?);
            } else {
                print_report(&r);
            }
            // FAIL-severity findings exit 2; warnings still exit 0. The report
            // itself is the diagnostic output, so no extra eprintln.
            let code = audit_exit_code(&r);
            if code != 0 {
                std::process::exit(code);
            }
        }
        Cmd::Export {
            pid,
            video,
            fcp,
            srt,
            vtt,
            ass,
            markdown,
            translated,
            bilingual,
            lang,
            speakers,
            output,
            start,
            end,
        } => {
            run_export_command(ExportCommand {
                pid: &pid,
                video,
                fcp,
                srt,
                vtt,
                ass,
                markdown,
                translated,
                bilingual,
                lang: lang.as_deref(),
                speakers,
                output: output.as_deref(),
                start,
                end,
                json,
            })
            .await?;
        }
        Cmd::Diarize { pid } => {
            let dir = PathBuf::from(&pid);
            let (segments, assigned) = diarize_project(&dir).await?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string(
                        &serde_json::json!({"segments": segments, "assigned": assigned})
                    )?
                );
            } else {
                println!("✓ diarize {pid}: segments={segments} speakers={assigned}");
            }
        }
        Cmd::Branch { action } => {
            use lumen_cut::data::version::Lineage;
            match action {
                BranchCmd::List { pid } => {
                    let dir = PathBuf::from(&pid);
                    let lineage = Lineage::load(&dir)?;
                    if json {
                        println!("{}", serde_json::to_string(&lineage)?);
                    } else {
                        for b in &lineage.branches {
                            let active = if lineage.active_branch.as_deref() == Some(b.id.as_str())
                            {
                                "*"
                            } else {
                                " "
                            };
                            println!("{active} {}: {} (tip={})", b.id, b.name, b.tip);
                        }
                        println!("({} branch(es))", lineage.branches.len());
                    }
                }
                BranchCmd::Create { pid, name } => {
                    let dir = PathBuf::from(&pid);
                    let mut lineage = Lineage::load(&dir)?;
                    let id = create_branch(&dir, &mut lineage, &name, "")?;
                    emit!(
                        json,
                        serde_json::json!({"id": id, "name": name}),
                        "✓ branch create {id}"
                    );
                }
                BranchCmd::Switch { pid, branch_id } => {
                    let dir = PathBuf::from(&pid);
                    let mut lineage = Lineage::load(&dir)?;
                    switch_branch(&dir, &mut lineage, &branch_id)?;
                    emit!(
                        json,
                        serde_json::json!({"activeBranch": branch_id}),
                        "✓ branch switch {branch_id}"
                    );
                }
                BranchCmd::Delete { pid, branch_id } => {
                    let dir = PathBuf::from(&pid);
                    let mut lineage = Lineage::load(&dir)?;
                    if lineage.active_branch.as_deref() == Some(branch_id.as_str()) {
                        return Err(AppError::Schema(
                            "cannot delete the active branch; switch first".into(),
                        ));
                    }
                    let before = lineage.branches.len();
                    lineage.branches.retain(|b| b.id != branch_id);
                    lineage.save(&dir)?;
                    emit!(
                        json,
                        serde_json::json!({"deleted": branch_id, "before": before, "after": lineage.branches.len()}),
                        "✓ branch delete {branch_id} ({before} → {})",
                        lineage.branches.len()
                    );
                }
            }
        }
        Cmd::Subtitle { pid, action } => {
            use lumen_cut::data::subtitle;
            let dir = PathBuf::from(&pid);
            match action {
                SubtitleCmd::List { lang } => {
                    let doc = Doc::load(&dir)?;
                    let hidden = subtitle::load_hidden_checked(&dir)?;
                    let rows = subtitle::list(&doc, &hidden, lang.as_deref());
                    if json {
                        println!("{}", serde_json::to_string(&rows)?);
                    } else {
                        for r in &rows {
                            let h = if r.hidden { " [hidden]" } else { "" };
                            println!("{} {}: {}{}", fmt_ts(r.start), r.id, r.text, h);
                        }
                        println!("({} subtitle(s))", rows.len());
                    }
                }
                SubtitleCmd::Set { id, text } => {
                    let mut doc = Doc::load(&dir)?;
                    if subtitle::set(&mut doc, &id, &text) {
                        doc.save(&dir)?;
                        emit!(
                            json,
                            serde_json::json!({"id": id, "text": text, "changed": true}),
                            "✓ subtitle set {id}"
                        );
                    } else {
                        return Err(AppError::Schema(format!("subtitle id {id} not found")));
                    }
                }
                SubtitleCmd::Find { query, regex } => {
                    let doc = Doc::load(&dir)?;
                    let rows = subtitle::find(&doc, &query, regex)?;
                    if json {
                        println!("{}", serde_json::to_string(&rows)?);
                    } else {
                        for r in rows {
                            println!("{} {}: {}", fmt_ts(r.start), r.id, r.text);
                        }
                    }
                }
                SubtitleCmd::Hide { id } => {
                    subtitle::hide(&dir, &id)?;
                    emit!(
                        json,
                        serde_json::json!({"id": id, "hidden": true}),
                        "✓ subtitle hide {id}"
                    );
                }
                SubtitleCmd::Replace {
                    query,
                    replacement,
                    regex,
                } => {
                    let mut doc = Doc::load(&dir)?;
                    let n =
                        lumen_cut::data::edit::find_replace(&mut doc, &query, &replacement, regex)?;
                    doc.save(&dir)?;
                    emit!(
                        json,
                        serde_json::json!({"changed": n}),
                        "✓ subtitle replace: {n} sentence(s) changed"
                    );
                }
                SubtitleCmd::Restore { id } => {
                    subtitle::restore(&dir, &id)?;
                    emit!(
                        json,
                        serde_json::json!({"id": id, "hidden": false}),
                        "✓ subtitle restore {id}"
                    );
                }
            }
        }
        Cmd::Speakers { pid, action } => {
            use lumen_cut::data::speakers;
            let dir = PathBuf::from(&pid);
            match action {
                SpeakersCmd::Show { turns, cues } => {
                    let doc = Doc::load(&dir)?;
                    let info = speakers::list(&doc);
                    let turns = speaker_turns(&doc, turns.map(|value| value as usize), cues);
                    let identified = doc
                        .paragraphs
                        .iter()
                        .filter(|paragraph| !paragraph.sentences.is_empty())
                        .all(|paragraph| paragraph.speaker.is_some());
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string(&serde_json::json!({
                                "identified": identified,
                                "speakers": info,
                                "turns": turns,
                            }))?
                        );
                    } else {
                        for s in &info {
                            println!("{}: {} paragraph(s)", s.id, s.paragraph_count);
                        }
                        for turn in &turns {
                            println!(
                                "{} {:.2}..{:.2} {}..{}: {}",
                                turn.speaker,
                                turn.start,
                                turn.end,
                                turn.first_cue,
                                turn.last_cue,
                                turn.text
                            );
                        }
                        println!(
                            "identified={} ({} speaker(s), {} turn(s))",
                            identified,
                            info.len(),
                            turns.len()
                        );
                    }
                }
                SpeakersCmd::View { rerun } => {
                    let view = speaker_view_project(&dir, rerun).await?;
                    emit!(
                        json,
                        &view,
                        "✓ speakers view {pid}: {}",
                        view.path.display()
                    );
                }
                SpeakersCmd::Assign {
                    speaker,
                    clear,
                    paragraph,
                    cue,
                    start,
                    end,
                } => {
                    if clear && speaker.is_some() {
                        return Err(AppError::Schema(
                            "pass either --speaker or --clear, not both".into(),
                        ));
                    }
                    if !clear && speaker.as_ref().is_none_or(|value| value.trim().is_empty()) {
                        return Err(AppError::Schema(
                            "speakers assign requires --speaker NAME or --clear".into(),
                        ));
                    }
                    let label = if clear { None } else { speaker.as_deref() };
                    let selectors = usize::from(paragraph.is_some())
                        + usize::from(cue.is_some())
                        + usize::from(start.is_some() || end.is_some());
                    if selectors != 1 {
                        return Err(AppError::Schema(
                            "speakers assign requires exactly one of --paragraph, --cue, or --start/--end"
                                .into(),
                        ));
                    }
                    let mut doc = Doc::load(&dir)?;
                    let changed = if let Some(paragraph_id) = paragraph {
                        if speakers::assign(&mut doc, paragraph_id, label) {
                            1
                        } else {
                            return Err(AppError::Schema(format!(
                                "paragraph {paragraph_id} was not found"
                            )));
                        }
                    } else if let Some(cue_id) = cue {
                        if speakers::assign_by_cue(&mut doc, &cue_id, label) {
                            1
                        } else {
                            return Err(AppError::Schema(format!(
                                "cue `{cue_id}` was not found"
                            )));
                        }
                    } else {
                        let (Some(range_start), Some(range_end)) = (start, end) else {
                            return Err(AppError::Schema(
                                "speakers assign --start requires --end".into(),
                            ));
                        };
                        speakers::assign_by_range(&mut doc, range_start, range_end, label)
                    };
                    doc.save(&dir)?;
                    let _ = speakers::clear_proposal(&dir)?;
                    emit!(
                        json,
                        serde_json::json!({
                            "pid": pid,
                            "speaker": label,
                            "changed": changed,
                        }),
                        "✓ speakers assign {pid}: {changed} paragraph(s)"
                    );
                }
                SpeakersCmd::Reidentify { review } => {
                    if review {
                        let preview = diarize_project_review(&dir).await?;
                        emit!(
                            json,
                            &preview,
                            "✓ speakers reidentify --review {pid}: proposal={} changed={} unassigned={}",
                            preview.id,
                            preview.changed,
                            preview.unassigned
                        );
                    } else {
                        let (segments, assigned) = diarize_project(&dir).await?;
                        let _ = speakers::clear_proposal(&dir)?;
                        emit!(
                            json,
                            serde_json::json!({"pid": pid, "segments": segments, "assigned": assigned, "applied": true}),
                            "✓ speakers re-run {pid}: segments={segments} speakers={assigned}"
                        );
                    }
                }
                SpeakersCmd::Proposals => {
                    match speakers::load_proposal(&dir)? {
                        Some(set) => {
                            emit!(
                                json,
                                &set,
                                "✓ speakers proposals {pid}: {} (changed={} unassigned={})",
                                set.id,
                                set.changed,
                                set.unassigned
                            );
                        }
                        None => {
                            emit!(
                                json,
                                serde_json::json!({"pid": pid, "proposals": null}),
                                "✓ speakers proposals {pid}: (none)"
                            );
                        }
                    }
                }
                SpeakersCmd::Apply { changed_only, all } => {
                    let set = speakers::load_proposal(&dir)?.ok_or_else(|| {
                        AppError::Schema(
                            "no stored speaker proposal; run `speakers reidentify --review` first"
                                .into(),
                        )
                    })?;
                    let use_changed_only = changed_only && !all;
                    let mut doc = Doc::load(&dir)?;
                    let applied =
                        speakers::apply_proposals(&mut doc, &set.proposals, use_changed_only)?;
                    doc.save(&dir)?;
                    let _ = speakers::clear_proposal(&dir)?;
                    emit!(
                        json,
                        serde_json::json!({
                            "pid": pid,
                            "proposalId": set.id,
                            "applied": applied,
                            "changedOnly": use_changed_only,
                        }),
                        "✓ speakers apply {pid}: {applied} paragraph(s) from {}",
                        set.id
                    );
                }
                SpeakersCmd::Rename { sid, name } => {
                    let mut doc = Doc::load(&dir)?;
                    let n = speakers::rename(&mut doc, &sid, &name);
                    doc.save(&dir)?;
                    let _ = speakers::clear_proposal(&dir)?;
                    emit!(
                        json,
                        serde_json::json!({"from": sid, "to": name, "changed": n}),
                        "✓ renamed {sid} → {name} ({n} paragraph(s))"
                    );
                }
                SpeakersCmd::Merge { from, into } => {
                    let mut doc = Doc::load(&dir)?;
                    let n = speakers::merge(&mut doc, &from, &into);
                    doc.save(&dir)?;
                    let _ = speakers::clear_proposal(&dir)?;
                    emit!(
                        json,
                        serde_json::json!({"from": from, "into": into, "changed": n}),
                        "✓ merged {from} → {into} ({n} paragraph(s))"
                    );
                }
            }
        }
        Cmd::Broll { pid, action } => {
            use lumen_cut::data::broll::{
                self as placement, BackgroundMode, BrollPlacement, FitMode, PlacementMode,
                PlacementPatch,
            };
            let dir = PathBuf::from(&pid);
            let mut items = placement::load(&dir)?;
            match action {
                BrollCmd::List => {
                    let suggestions = lumen_cut::pipeline::broll::load_artifact(&dir)?;
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string(
                                &serde_json::json!({"suggestions": suggestions, "accepted": items})
                            )?
                        );
                    } else {
                        for suggestion in &suggestions {
                            println!(
                                "suggest {}..{} {:?}: {}",
                                suggestion.start, suggestion.end, suggestion.mode, suggestion.query
                            );
                        }
                        for it in &items {
                            println!("accepted {}", serde_json::to_string(it)?);
                        }
                        println!("({} suggest / {} accepted)", suggestions.len(), items.len());
                    }
                }
                BrollCmd::Add {
                    file,
                    at,
                    start,
                    end,
                    dur,
                    mode,
                    rect,
                    fit,
                    bg,
                    src_start,
                    radius,
                    name,
                } => {
                    if at.is_some() && start.is_some() {
                        return Err(AppError::Schema(
                            "use either --at or --start, not both".into(),
                        ));
                    }
                    if !file.exists() {
                        return Err(AppError::ProjectNotFound(file));
                    }
                    let file = std::fs::canonicalize(file)?;
                    let image = file
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .is_some_and(|extension| {
                            matches!(
                                extension.to_ascii_lowercase().as_str(),
                                "png" | "jpg" | "jpeg" | "webp" | "gif"
                            )
                        });
                    let start = start.or(at).unwrap_or(3.0);
                    let end = end.unwrap_or(start + dur.unwrap_or(if image { 4.0 } else { 8.0 }));
                    let id = format!("br-{}", uuid::Uuid::new_v4().simple());
                    let item =
                        BrollPlacement {
                            id: id.clone(),
                            file: file.clone(),
                            start,
                            end,
                            mode: mode.as_deref().map(str::parse).transpose()?.unwrap_or(
                                if image {
                                    PlacementMode::Pip
                                } else {
                                    PlacementMode::Fullscreen
                                },
                            ),
                            rect: rect.as_deref().map(placement::parse_rect).transpose()?,
                            fit: fit
                                .as_deref()
                                .map(str::parse)
                                .transpose()?
                                .unwrap_or(FitMode::Cover),
                            background: bg
                                .as_deref()
                                .map(str::parse)
                                .transpose()?
                                .unwrap_or(BackgroundMode::Black),
                            source_start: src_start.unwrap_or_default(),
                            radius: radius.unwrap_or_default(),
                            name,
                        };
                    item.validate()?;
                    items.push(item.clone());
                    placement::save(&dir, &items)?;
                    emit!(json, &item, "✓ broll add {id} ← {}", file.display());
                }
                BrollCmd::Remove { id } => {
                    let before = items.len();
                    items.retain(|item| item.id != id);
                    placement::save(&dir, &items)?;
                    emit!(
                        json,
                        serde_json::json!({"id": id, "removed": before - items.len(), "before": before, "after": items.len()}),
                        "✓ broll remove {id} ({before} → {})",
                        items.len()
                    );
                }
                BrollCmd::Update {
                    id,
                    file,
                    start,
                    end,
                    mode,
                    rect,
                    fit,
                    bg,
                    src_start,
                    radius,
                    name,
                } => {
                    let patch = PlacementPatch {
                        file: file.map(std::fs::canonicalize).transpose()?,
                        start,
                        end,
                        mode: mode.as_deref().map(str::parse).transpose()?,
                        rect: rect
                            .as_deref()
                            .map(placement::parse_rect)
                            .transpose()?
                            .map(Some),
                        fit: fit.as_deref().map(str::parse).transpose()?,
                        background: bg.as_deref().map(str::parse).transpose()?,
                        source_start: src_start,
                        radius,
                        name: name.map(Some),
                    };
                    if !placement::update(&mut items, &id, patch)? {
                        return Err(AppError::Schema(format!("broll id {id} not found")));
                    }
                    placement::save(&dir, &items)?;
                    let updated = items
                        .iter()
                        .find(|item| item.id == id)
                        .ok_or_else(|| AppError::Schema(format!("broll id {id} disappeared")))?;
                    emit!(json, updated, "✓ broll update {id}");
                }
                BrollCmd::Preview { at } => {
                    if items.is_empty() {
                        emit!(
                            json,
                            Vec::<String>::new(),
                            "(no accepted broll items — `broll add --file` first)"
                        );
                        return Ok(());
                    }
                    let doc = Doc::load(&dir)?;
                    let cuts_path = dir.join("cuts.json");
                    let cuts: ClipCuts = if cuts_path.exists() {
                        serde_json::from_str(&std::fs::read_to_string(cuts_path)?)?
                    } else {
                        ClipCuts::new()
                    };
                    let ass = dir.join("broll-preview.ass");
                    let style = lumen_cut::data::substyle::SubStyle::load(&dir)?;
                    let settings = lumen_cut::data::export_settings::load(&dir)?;
                    let hidden = lumen_cut::data::subtitle::load_hidden_checked(&dir)?;
                    let caption_doc =
                        lumen_cut::data::export_settings::project_caption_doc_with_hidden(
                            &doc,
                            settings.subtitle_language.as_deref(),
                            settings.bilingual_subtitles,
                            &hidden,
                        )?;
                    write_ass_with_style(&caption_doc, &cuts.cuts, &style, &ass, 1920, 1080)?;
                    let preview_video = dir.join("broll-preview.mp4");
                    lumen_cut::export::render_video_with_broll(
                        &doc,
                        &cuts.cuts,
                        &ass,
                        &preview_video,
                        &items,
                    )
                    .await?;
                    let timestamps = if at.is_empty() {
                        items
                            .iter()
                            .map(|item| (item.start + item.end) / 2.0)
                            .collect()
                    } else {
                        at
                    };
                    let mut outputs = Vec::new();
                    for timestamp in timestamps {
                        let out = dir.join(format!("broll-preview-{timestamp:.1}.png"));
                        lumen_cut::media::extract_frame(&preview_video, timestamp, &out).await?;
                        if !json {
                            println!("✓ preview {timestamp:.1}s → {}", out.display());
                        }
                        outputs.push(out);
                    }
                    if json {
                        println!("{}", serde_json::to_string(&outputs)?);
                    }
                }
            }
        }
        Cmd::Timing { pid, dry_run } => {
            let dir = PathBuf::from(&pid);
            let mut doc = Doc::load(&dir)?;
            let rep = lumen_cut::pipeline::timing::repair(&mut doc);
            if json {
                println!("{}", serde_json::to_string(&rep)?);
            }
            if dry_run {
                if !json {
                    println!(
                        "(dry-run) would fix: negative={} inverted={} zero={} overlap={}",
                        rep.clamped_negative, rep.fixed_inverted, rep.fixed_zero, rep.fixed_overlap
                    );
                }
            } else {
                doc.save(&dir)?;
                if !json {
                    println!("✓ timing repair {pid}: {} fix(es)", rep.total());
                }
            }
        }
        Cmd::Model { action } => {
            match action {
                ModelCmd::List => {
                    let home = std::env::var_os("HOME")
                        .map(PathBuf::from)
                        .unwrap_or_default();
                    let hub = home.join(".cache/huggingface/hub");
                    let mut models: Vec<String> = std::fs::read_dir(&hub)
                        .map(|rd| {
                            rd.filter_map(|e| e.ok())
                                .map(|e| e.file_name().to_string_lossy().into_owned())
                                .filter(|n| n.starts_with("models--"))
                                .collect()
                        })
                        .unwrap_or_default();
                    models.sort();
                    if json {
                        println!("{}", serde_json::to_string(&models)?);
                    } else {
                        for m in &models {
                            println!("{}", m.trim_start_matches("models--").replace("--", "/"));
                        }
                        println!("({} model(s) in {})", models.len(), hub.display());
                    }
                }
                ModelCmd::Download { id } => {
                    // Hugging Face Hub renamed its CLI to `hf`; retain the
                    // legacy executable as a compatibility fallback.
                    let executable = lumen_cut::doctor::huggingface_cli().ok_or_else(|| {
                        AppError::Io(std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "Hugging Face CLI not found — install `huggingface_hub`",
                        ))
                    })?;
                    let mut command = std::process::Command::new(executable);
                    command.args(["download", &id]);
                    if json {
                        command
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null());
                    }
                    let st = command.status();
                    match st {
                        Ok(s) if s.success() => emit!(
                            json,
                            serde_json::json!({"downloaded": id}),
                            "✓ downloaded {id}"
                        ),
                        Ok(s) => {
                            return Err(AppError::Schema(format!(
                                "{executable} download failed with status {s}"
                            )))
                        }
                        Err(error) => return Err(AppError::Io(error)),
                    }
                }
            }
        }
        Cmd::Doctor => {
            let checks = lumen_cut::doctor::checks();
            let passed = checks.iter().filter(|check| check.ok).count();
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "checks": &checks,
                        "passed": passed,
                        "total": checks.len(),
                    }))?
                );
            } else {
                for check in &checks {
                    println!(
                        "{} {:<16} {}",
                        if check.ok { "✓" } else { "✗" },
                        check.name,
                        check.detail
                    );
                }
                println!("\ndoctor: {passed}/{} checks passed", checks.len());
            }
        }
        Cmd::Logs { pid, kind } => {
            let dir = PathBuf::from(&pid).join("ai");
            if !dir.exists() {
                emit!(
                    json,
                    serde_json::json!({"pid": pid, "logs": {}}),
                    "(no ai/ logs for {pid})"
                );
            } else if let Some(k) = &kind {
                let p = dir.join(k);
                let mut files: Vec<String> = std::fs::read_dir(&p)
                    .map(|rd| {
                        rd.filter_map(|e| e.ok())
                            .map(|e| e.file_name().to_string_lossy().into_owned())
                            .collect()
                    })
                    .unwrap_or_default();
                files.sort();
                if json {
                    println!(
                        "{}",
                        serde_json::to_string(&serde_json::json!({"kind": k, "files": files}))?
                    );
                } else {
                    for f in &files {
                        println!("{k}/{f}");
                    }
                    println!("({} file(s))", files.len());
                }
            } else {
                let mut logs = std::collections::BTreeMap::new();
                for entry in std::fs::read_dir(&dir)
                    .map(|rd| rd.filter_map(|e| e.ok()).collect::<Vec<_>>())
                    .unwrap_or_default()
                {
                    if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        let k = entry.file_name().to_string_lossy().into_owned();
                        let n = std::fs::read_dir(entry.path())
                            .map(|rd| rd.count())
                            .unwrap_or(0);
                        logs.insert(k, n);
                    }
                }
                if json {
                    println!("{}", serde_json::to_string(&logs)?);
                } else {
                    for (kind, count) in &logs {
                        println!("{kind}: {count} file(s)");
                    }
                    println!("({} log file(s) total)", logs.values().sum::<usize>());
                }
            }
        }
        Cmd::Mcp { action } => match action {
            McpCmd::Serve => {
                lumen_cut::agent::mcp::run_stdio()?;
            }
        },
        Cmd::Record { pid, seconds } => {
            let dir = PathBuf::from(&pid);
            std::fs::create_dir_all(&dir)?;
            let wav = dir.join("audio.wav");
            let st = std::process::Command::new("ffmpeg")
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
                .map_err(|e| {
                    AppError::Io(std::io::Error::other(format!("ffmpeg avfoundation: {e}")))
                })?;
            if st.success() {
                emit!(
                    json,
                    serde_json::json!({"pid": pid, "seconds": seconds, "path": wav}),
                    "✓ record {pid}: {seconds}s → {}",
                    wav.display()
                );
            } else {
                return Err(AppError::Schema(
                    "ffmpeg avfoundation recording failed".into(),
                ));
            }
        }
        Cmd::Skill { install } => {
            if let Some(pkg) = install {
                let mut command = std::process::Command::new("npx");
                command
                    .args([
                        "skills",
                        "add",
                        &pkg,
                        "-g",
                        "-a",
                        "claude-code",
                        "-a",
                        "codex",
                        "-y",
                    ])
                    .env("DISABLE_TELEMETRY", "1");
                if json {
                    command
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null());
                }
                let st = command.status().map_err(|e| {
                    AppError::Io(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("npx: {e}"),
                    ))
                })?;
                if st.success() {
                    emit!(
                        json,
                        serde_json::json!({"installed": pkg}),
                        "✓ skill install {pkg}"
                    );
                } else {
                    return Err(AppError::Schema(format!(
                        "skill install failed with status {st}"
                    )));
                }
            } else {
                emit!(
                    json,
                    serde_json::json!({
                        "path": "~/.claude/skills/lumen-cut/",
                        "installUsage": "--install <pkg>"
                    }),
                    "lumen-cut skill bundle: ~/.claude/skills/lumen-cut/ — use --install <pkg> to add one"
                );
            }
        }
    }
    Ok(())
}

fn read_cue_map(p: &Path) -> AppResult<std::collections::BTreeMap<String, String>> {
    let raw = std::fs::read_to_string(p)?;
    let m: std::collections::BTreeMap<String, String> = serde_json::from_str(&raw)?;
    Ok(m)
}

fn fmt_ts(seconds: f64) -> String {
    let t = seconds.max(0.0) as u64;
    format!("{:02}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskStatus {
    pending: usize,
    done: usize,
    failed: usize,
    kinds: Vec<lumen_cut::agent::task::TaskKindStatus>,
    polish_quality: Option<lumen_cut::pipeline::polish::PolishQualityArtifact>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AutoSummary {
    pid_dir: PathBuf,
    words: usize,
    paragraphs: usize,
    srt: PathBuf,
    vtt: PathBuf,
    ass: PathBuf,
    markdown: PathBuf,
    polished: bool,
    translated: Option<String>,
    cuts_added: usize,
}

struct AutoOptions<'a> {
    media: &'a str,
    lang: Option<&'a str>,
    source_lang: Option<&'a str>,
    title: Option<&'a str>,
    out_dir: Option<&'a Path>,
    model: Option<&'a str>,
    no_polish: bool,
    rough_cut: bool,
    align_fit: Option<usize>,
    stale_only: bool,
}

struct CutCommand<'a> {
    pid: &'a str,
    auto: bool,
    list: bool,
    kind: Option<&'a str>,
    add: bool,
    start: Option<f64>,
    end: Option<f64>,
    words: Option<&'a str>,
    note: Option<&'a str>,
    restore: Option<&'a str>,
    restore_all: bool,
    dry_run: bool,
    min_pause: f64,
    compress_to: f64,
    max_gap: f64,
    no_fillers: bool,
    no_pauses: bool,
    json: bool,
}

struct ExportCommand<'a> {
    pid: &'a str,
    video: bool,
    fcp: bool,
    srt: bool,
    vtt: bool,
    ass: bool,
    markdown: bool,
    translated: bool,
    bilingual: bool,
    lang: Option<&'a str>,
    speakers: bool,
    output: Option<&'a Path>,
    start: Option<f64>,
    end: Option<f64>,
    json: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskServeResult {
    pending: usize,
    applied: usize,
    url: String,
    held: bool,
}

fn parse_second_look(raw: &str) -> AppResult<lumen_cut::agent::task::SecondLookMode> {
    lumen_cut::agent::task::SecondLookMode::parse(raw).ok_or_else(|| {
        AppError::Schema(format!(
            "invalid --second-look `{raw}` (expected semantic|targeted|off)"
        ))
    })
}

async fn task_serve(
    kind: &str,
    pid: &str,
    lang: Option<&str>,
    stale_only: bool,
    groups: Vec<String>,
    align_fit: Option<usize>,
    align_local: bool,
    second_look: &str,
    auto_phase2: bool,
    root: &Path,
    port: u16,
    hold: bool,
    json: bool,
) -> AppResult<TaskServeResult> {
    let dir = root.join(pid);
    let second_look = parse_second_look(second_look)?;
    let task = if let Some(task) = lumen_cut::agent::task::load_recoverable_task(&dir, kind)? {
        task
    } else {
        lumen_cut::agent::task::prepare_task_with_task_options(
            &dir,
            kind,
            lang,
            lumen_cut::agent::task::TaskOptions {
                stale_only,
                groups,
                align_fit,
                align_local,
                second_look,
                auto_phase2,
            },
        )?
    };
    let pending = task.calls.len();
    let capacity = lumen_cut::data::modelconfig::load().worker_count.max(1) as usize;
    let allocator = std::sync::Arc::new(lumen_cut::agent::Allocator::new(capacity));
    let pool = std::sync::Arc::new(std::sync::Mutex::new(
        lumen_cut::agent::pool::WorkerPool::new_workers(capacity),
    ));
    let (addr, router) = lumen_cut::agent::http::bind(port, allocator.clone(), pool).await?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?;
    let url = format!("http://127.0.0.1:{}", local_addr.port());
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            tracing::error!(%error, "agent server stopped");
        }
    });
    if lumen_cut::agent::runtime::load_bridge_config().is_some() {
        lumen_cut::agent::runtime::spawn_workers(allocator.clone(), capacity).await;
    } else if !json {
        eprintln!(
            "agent claim/submit listening on {url} (GET /agent/next, POST /agent/submit)"
        );
        eprintln!("no built-in LLM workers — external workers must claim work");
    }
    let recovered = lumen_cut::agent::task::restore_or_enqueue(&allocator, &task)?;
    info!(kind, pid, recovered, "restored durable task submissions");
    let applied = match lumen_cut::agent::task::wait_and_apply(
        allocator.clone(),
        task.clone(),
        std::time::Duration::from_secs(30 * 60),
    )
    .await
    {
        Ok(applied) => {
            lumen_cut::agent::task::set_task_state(&task, "completed", None)?;
            applied
        }
        Err(error) => {
            let message = error.to_string();
            lumen_cut::agent::task::set_task_state(&task, "failed", Some(&message))?;
            if !hold {
                return Err(error);
            }
            warn!(%error, "task serve wait failed; holding server because --hold");
            0
        }
    };
    if hold {
        if !json {
            eprintln!("holding agent server on {url} (Ctrl-C to exit)");
        }
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    }
    Ok(TaskServeResult {
        pending,
        applied,
        url,
        held: hold,
    })
}

async fn task_start(
    kind: &str,
    pid: &str,
    lang: Option<&str>,
    stale_only: bool,
    groups: Vec<String>,
    align_fit: Option<usize>,
    align_local: bool,
    second_look: &str,
    auto_phase2: bool,
    root: &Path,
) -> AppResult<usize> {
    let dir = root.join(pid);
    let second_look = parse_second_look(second_look)?;
    let task = if let Some(task) = lumen_cut::agent::task::load_recoverable_task(&dir, kind)? {
        task
    } else {
        lumen_cut::agent::task::prepare_task_with_task_options(
            &dir,
            kind,
            lang,
            lumen_cut::agent::task::TaskOptions {
                stale_only,
                groups,
                align_fit,
                align_local,
                second_look,
                auto_phase2,
            },
        )?
    };
    let pending = task.calls.len();
    let capacity = lumen_cut::data::modelconfig::load().worker_count.max(1) as usize;
    let allocator = std::sync::Arc::new(lumen_cut::agent::Allocator::new(capacity));
    let pool = std::sync::Arc::new(std::sync::Mutex::new(
        lumen_cut::agent::pool::WorkerPool::new_workers(capacity),
    ));
    let (addr, router) = lumen_cut::agent::http::bind(0, allocator.clone(), pool).await?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?;
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            tracing::error!(%error, "agent server stopped");
        }
    });
    if lumen_cut::agent::runtime::load_bridge_config().is_some() {
        lumen_cut::agent::runtime::spawn_workers(allocator.clone(), capacity).await;
    } else {
        eprintln!(
            "agent waiting on http://127.0.0.1:{} (configure llmEndpoint for built-in workers)",
            local_addr.port()
        );
    }
    let recovered = lumen_cut::agent::task::restore_or_enqueue(&allocator, &task)?;
    info!(kind, pid, recovered, "restored durable task submissions");
    let result = lumen_cut::agent::task::wait_and_apply(
        allocator,
        task.clone(),
        std::time::Duration::from_secs(30 * 60),
    )
    .await;
    match result {
        Ok(applied) => {
            lumen_cut::agent::task::set_task_state(&task, "completed", None)?;
            info!(kind, pid, applied, "task completed and applied");
            Ok(pending)
        }
        Err(error) => {
            let message = error.to_string();
            lumen_cut::agent::task::set_task_state(&task, "failed", Some(&message))?;
            Err(error)
        }
    }
}

fn task_status(pid: &str, root: &Path) -> AppResult<TaskStatus> {
    let project_dir = root.join(pid);
    let kinds = lumen_cut::agent::task::task_kind_statuses(&project_dir);
    let pending = kinds.iter().map(|status| status.pending).sum();
    let done = kinds.iter().map(|status| status.done).sum();
    let failed = kinds.iter().map(|status| status.failed).sum();
    let polish_quality = lumen_cut::pipeline::polish::PolishQualityArtifact::load(
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
}

fn print_report(r: &Report) {
    if r.findings.is_empty() {
        println!("(no findings)");
        return;
    }
    for f in &r.findings {
        println!(
            "{:?} {:?} {} :: {}",
            f.severity, f.code, f.where_, f.message
        );
    }
    println!(
        "summary: failures={} warnings={}",
        r.has_failures() as u32,
        r.has_warnings() as u32
    );
}

/// `audit` exit-code contract: 2 when the report holds FAIL-severity
/// findings, 0 otherwise — warnings alone are not failures.
fn audit_exit_code(r: &Report) -> i32 {
    if r.has_failures() {
        2
    } else {
        0
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpeakerTurn {
    speaker: String,
    start: f64,
    end: f64,
    first_cue: String,
    last_cue: String,
    cue_count: usize,
    text: String,
    text_truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    cues: Option<Vec<String>>,
    cues_truncated: bool,
}

const MAX_SPEAKER_TURN_TEXT_CHARS: usize = 500;
const MAX_SPEAKER_TURN_CUES: usize = 100;

fn truncate_chars(value: String, max_chars: usize) -> (String, bool) {
    if value.chars().count() <= max_chars {
        return (value, false);
    }
    if max_chars == 0 {
        return (String::new(), true);
    }
    let mut truncated: String = value.chars().take(max_chars.saturating_sub(1)).collect();
    truncated.push('…');
    (truncated, true)
}

fn speaker_turns(doc: &Doc, limit: Option<usize>, include_cues: bool) -> Vec<SpeakerTurn> {
    let mut turns = Vec::new();
    for paragraph in &doc.paragraphs {
        let cue_ids: Vec<String> = paragraph
            .sentences
            .iter()
            .map(|sentence| sentence.id.clone())
            .collect();
        let words: Vec<_> = paragraph
            .sentences
            .iter()
            .flat_map(|sentence| sentence.words.iter())
            .collect();
        let (Some(first), Some(last), Some(first_cue), Some(last_cue)) =
            (words.first(), words.last(), cue_ids.first(), cue_ids.last())
        else {
            continue;
        };
        let (text, text_truncated) = truncate_chars(
            paragraph
                .sentences
                .iter()
                .map(|sentence| sentence.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            MAX_SPEAKER_TURN_TEXT_CHARS,
        );
        let cue_count = cue_ids.len();
        let cues_truncated = include_cues && cue_count > MAX_SPEAKER_TURN_CUES;
        turns.push(SpeakerTurn {
            speaker: paragraph
                .speaker
                .clone()
                .unwrap_or_else(|| "unidentified".into()),
            start: first.start,
            end: last.end,
            first_cue: first_cue.clone(),
            last_cue: last_cue.clone(),
            cue_count,
            text,
            text_truncated,
            cues: include_cues.then(|| cue_ids.into_iter().take(MAX_SPEAKER_TURN_CUES).collect()),
            cues_truncated,
        });
    }
    if let Some(limit) = limit {
        turns.truncate(limit);
    }
    turns
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpeakerBand {
    speaker: String,
    start: f64,
    end: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpeakerView {
    path: PathBuf,
    current: Vec<SpeakerBand>,
    rerun: Option<Vec<SpeakerBand>>,
    move_seconds: std::collections::BTreeMap<String, f64>,
}

/// Render the current labels as band A and, with `--rerun`, a fresh
/// non-mutating diarization as band B. Only `speakers reidentify` writes
/// fresh labels back to the document.
async fn speaker_view_project(dir: &Path, rerun: bool) -> AppResult<SpeakerView> {
    let doc = Doc::load(dir)?;
    let wav = locate_project_wav(dir)?;
    let current: Vec<SpeakerBand> = speaker_turns(&doc, None, false)
        .into_iter()
        .map(|turn| SpeakerBand {
            speaker: turn.speaker,
            start: turn.start,
            end: turn.end,
        })
        .collect();
    let fresh = if rerun {
        let model = lumen_cut::data::modelconfig::load().diarize_model;
        Some(
            diarize_file_with_model_progress(&wav, &model, Some(cli_diarize_progress()))
                .await?
                .segments
                .into_iter()
                .map(|segment| SpeakerBand {
                    speaker: segment.speaker,
                    start: segment.start,
                    end: segment.end,
                })
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };
    let mut move_seconds = std::collections::BTreeMap::<String, f64>::new();
    if let Some(fresh) = &fresh {
        for source in &current {
            for target in fresh {
                let overlap = source.end.min(target.end) - source.start.max(target.start);
                if overlap > 0.0 {
                    *move_seconds
                        .entry(format!("{}→{}", source.speaker, target.speaker))
                        .or_default() += overlap;
                }
            }
        }
    }
    let path = dir.join("speaker-view.png");
    render_speaker_view(
        &wav,
        &path,
        doc.media.duration_seconds,
        &current,
        fresh.as_deref(),
    )?;
    Ok(SpeakerView {
        path,
        current,
        rerun: fresh,
        move_seconds,
    })
}

fn render_speaker_view(
    wav: &Path,
    path: &Path,
    duration: f64,
    current: &[SpeakerBand],
    rerun: Option<&[SpeakerBand]>,
) -> AppResult<()> {
    const WIDTH: f64 = 1600.0;
    const PALETTE: [&str; 8] = [
        "0x64d2ff", "0xff9f0a", "0x30d158", "0xbf5af2", "0xff375f", "0x5e5ce6", "0xffd60a",
        "0x40c8e0",
    ];
    let duration = duration.max(
        current
            .iter()
            .chain(rerun.into_iter().flatten())
            .map(|band| band.end)
            .fold(0.0, f64::max),
    );
    if duration <= 0.0 {
        return Err(AppError::Schema(
            "speakers view requires a positive media duration".into(),
        ));
    }
    let mut speakers = std::collections::BTreeMap::<String, usize>::new();
    for band in current.iter().chain(rerun.into_iter().flatten()) {
        if !speakers.contains_key(&band.speaker) {
            let index = speakers.len();
            speakers.insert(band.speaker.clone(), index);
        }
    }
    let mut filter =
        "[0:a]showwavespic=s=1600x320:colors=0x64d2ff,pad=1600:420:0:0:color=0x111827".to_string();
    let mut append_band = |band: &SpeakerBand, y: usize| {
        let x = (band.start.max(0.0) / duration * WIDTH).round();
        let width = ((band.end - band.start).max(0.01) / duration * WIDTH)
            .round()
            .max(1.0);
        let color = PALETTE[speakers[&band.speaker] % PALETTE.len()];
        filter.push_str(&format!(
            ",drawbox=x={x}:y={y}:w={width}:h=32:color={color}@0.9:t=fill"
        ));
    };
    for band in current {
        append_band(band, 338);
    }
    if let Some(rerun) = rerun {
        for band in rerun {
            append_band(band, 380);
        }
    }
    let status = std::process::Command::new("ffmpeg")
        .args(["-loglevel", "error", "-y", "-i"])
        .arg(wav)
        .args(["-filter_complex", &filter, "-frames:v", "1"])
        .arg(path)
        .status()
        .map_err(|error| {
            AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("ffmpeg: {error}"),
            ))
        })?;
    if !status.success() {
        return Err(AppError::Schema(
            "ffmpeg failed to render speaker diagnostic".into(),
        ));
    }
    Ok(())
}

/// Run the diarization sidecar over the project's audio and write the
/// per-paragraph speakers back to `doc.json`. Returns
/// `(segment_count, paragraphs_with_speaker)`.
async fn diarize_project(dir: &Path) -> AppResult<(usize, usize)> {
    let mut doc = Doc::load(dir)?;
    let wav = locate_project_wav(dir)?;
    if std::env::var_os("HF_TOKEN").is_none()
        && std::env::var_os("HUGGING_FACE_HUB_TOKEN").is_none()
    {
        warn!("HF_TOKEN not set; gated pyannote models may require `hf auth login`");
    }
    let model = lumen_cut::data::modelconfig::load().diarize_model;
    let out = diarize_file_with_model_progress(&wav, &model, Some(cli_diarize_progress())).await?;
    let assigned = assign_speakers(&mut doc, &out.segments);
    doc.save(dir)?;
    Ok((out.segments.len(), assigned))
}

/// Non-destructive re-identification: store a reviewable proposal without
/// mutating paragraph speakers.
async fn diarize_project_review(
    dir: &Path,
) -> AppResult<lumen_cut::data::speakers::SpeakerProposalSet> {
    use lumen_cut::data::speakers::{self, SpeakerProposalSet};

    let mut doc = Doc::load(dir)?;
    lumen_cut::diarize::normalize_speaker_paragraphs(&mut doc);
    let wav = locate_project_wav(dir)?;
    if std::env::var_os("HF_TOKEN").is_none()
        && std::env::var_os("HUGGING_FACE_HUB_TOKEN").is_none()
    {
        warn!("HF_TOKEN not set; gated pyannote models may require `hf auth login`");
    }
    let model = lumen_cut::data::modelconfig::load().diarize_model;
    let out = diarize_file_with_model_progress(&wav, &model, Some(cli_diarize_progress())).await?;
    let (proposals, unassigned) = proposals_from_segments(&doc, &out.segments);
    let changed = proposals
        .iter()
        .filter(|proposal| proposal.current.as_deref() != Some(proposal.proposed.as_str()))
        .count();
    let set = SpeakerProposalSet {
        id: format!("sp-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]),
        created_at: chrono::Utc::now().to_rfc3339(),
        segments: out.segments.len(),
        changed,
        unassigned,
        proposals,
    };
    speakers::save_proposal(dir, &set)?;
    Ok(set)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectOpenSummary {
    pid: String,
    path: String,
    url: String,
    queued_for_desktop: bool,
    revealed: bool,
}

fn project_open(
    pid: &str,
    root: &Path,
    reveal: bool,
    desktop: bool,
) -> AppResult<ProjectOpenSummary> {
    let dir = root.join(pid);
    if !dir.join("doc.json").exists() {
        return Err(AppError::ProjectNotFound(dir));
    }
    let path = dir
        .canonicalize()
        .unwrap_or_else(|_| dir.clone())
        .display()
        .to_string();
    let url = format!("lumencut://project/{pid}");
    let mut revealed = false;
    if reveal {
        let status = std::process::Command::new("open")
            .arg(&path)
            .status()
            .map_err(|error| AppError::Schema(format!("failed to reveal project: {error}")))?;
        if !status.success() {
            return Err(AppError::Schema(
                "failed to reveal project in the file manager".into(),
            ));
        }
        revealed = true;
    }
    let mut queued_for_desktop = false;
    if desktop {
        lumen_cut::commands::queue_desktop_project_open(pid, &path)?;
        queued_for_desktop = true;
    }
    Ok(ProjectOpenSummary {
        pid: pid.to_string(),
        path,
        url,
        queued_for_desktop,
        revealed,
    })
}

/// `audio.wav` lives either inside the project dir or next to it (`lumen-cut
/// auto` writes it into the out-dir, beside `<pid>/`). Missing audio is a
/// clear error, not a panic.
fn locate_project_wav(dir: &Path) -> AppResult<PathBuf> {
    let mut candidates = vec![dir.join("audio.wav")];
    if let Some(parent) = dir.parent() {
        candidates.push(parent.join("audio.wav"));
    }
    candidates.into_iter().find(|p| p.exists()).ok_or_else(|| {
        AppError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "audio.wav not found in {} or its parent — run `lumen-cut auto` first",
                dir.display()
            ),
        ))
    })
}

async fn project_create(
    pid: &str,
    from: &Path,
    lang: Option<&str>,
    title: Option<&str>,
    root: &Path,
) -> AppResult<()> {
    let info = probe(from).await?;
    std::fs::create_dir_all(root)?;
    let doc = Doc {
        id: pid.to_string(),
        schema: 1,
        media: MediaRef {
            path: from.to_path_buf(),
            duration_seconds: info.duration_seconds,
            sample_rate: info.sample_rate,
            channels: info.channels,
        },
        meta: Meta {
            title: title.unwrap_or(pid).to_string(),
            description: String::new(),
            language: lang.map(str::to_string),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
        paragraphs: vec![],
        translations: Default::default(),
    };
    let dir = root.join(pid);
    doc.save(&dir)?;
    info!(pid, dir = %dir.display(), "created project");
    Ok(())
}

fn project_show(pid: &str, root: &Path) -> AppResult<()> {
    let dir = root.join(pid);
    let doc = Doc::load(&dir)?;
    println!("{}", serde_json::to_string_pretty(&doc)?);
    Ok(())
}

async fn run_auto(opts: AutoOptions<'_>) -> AppResult<AutoSummary> {
    let out_dir = opts
        .out_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&out_dir)?;

    // Multi-stage intent: translate target needs source-lang, --no-polish, or
    // --rough-cut so plain `auto media --lang zh` stays ASR-only (compat).
    let wants_translate = opts.lang.is_some()
        && (opts.source_lang.is_some() || opts.no_polish || opts.rough_cut);
    let translate_lang = if wants_translate {
        opts.lang
    } else {
        None
    };
    let asr_lang = opts
        .source_lang
        .or(if wants_translate { None } else { opts.lang });
    let wants_polish = (wants_translate || opts.rough_cut) && !opts.no_polish;

    let media_path = if opts.media.starts_with("http://") || opts.media.starts_with("https://") {
        report_cli_phase("downloading", 0);
        let tmpl = out_dir.join("source.%(ext)s");
        let path = download(opts.media, &tmpl).await?;
        report_cli_phase("downloading", 15);
        path
    } else {
        PathBuf::from(opts.media)
    };
    if !media_path.exists() {
        return Err(AppError::ProjectNotFound(media_path));
    }

    let wav = out_dir.join("audio.wav");
    report_cli_phase("extracting", 15);
    extract_audio_wav(&media_path, &wav).await?;
    report_cli_phase("analyzing", 35);

    let info = probe(&media_path).await?;
    let pid_stem = media_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".to_string());
    let pid_dir = out_dir.join(&pid_stem);
    std::fs::create_dir_all(&pid_dir)?;

    let model_config = lumen_cut::data::modelconfig::load();
    let model = opts.model.unwrap_or(&model_config.asr_model);
    let asr = transcribe_file_with_aligner_progress(
        &wav,
        model,
        asr_lang,
        Some(&model_config.asr_aligner),
        Some(cli_asr_progress()),
    )
    .await?;

    report_cli_phase("saving", 90);
    let mut doc: Doc = asr.into();
    doc.id = pid_stem.clone();
    doc.media = MediaRef {
        path: media_path.clone(),
        duration_seconds: info.duration_seconds,
        sample_rate: info.sample_rate,
        channels: info.channels,
    };
    doc.meta.title = opts.title.unwrap_or(&pid_stem).to_string();
    if let Some(source) = asr_lang {
        doc.meta.language = Some(source.to_string());
    }
    doc.meta.updated_at = Utc::now();
    lumen_cut::pipeline::timing::repair(&mut doc);
    doc.save(&pid_dir)?;

    let mut polished = false;
    let mut translated: Option<String> = None;
    let mut cuts_added = 0usize;

    if wants_polish || wants_translate || opts.rough_cut {
        report_cli_phase("enhancing", 92);
        if wants_polish {
            let _ = task_start(
                "polish",
                &pid_stem,
                None,
                false,
                Vec::new(),
                None,
                false,
                "semantic",
                true,
                &out_dir,
            )
            .await?;
            polished = true;
        }
        if let Some(target) = translate_lang {
            let _ = task_start(
                "translate",
                &pid_stem,
                Some(target),
                opts.stale_only,
                Vec::new(),
                None,
                false,
                "semantic",
                true,
                &out_dir,
            )
            .await?;
            // Phase-2 align is auto-chained inside translate when auto_phase2.
            translated = Some(target.to_string());
        }
        if opts.rough_cut {
            let doc = Doc::load(&pid_dir)?;
            let cuts_path = pid_dir.join("cuts.json");
            let mut cuts: ClipCuts = if cuts_path.exists() {
                serde_json::from_str(&std::fs::read_to_string(&cuts_path)?)?
            } else {
                ClipCuts::new()
            };
            cuts_added = lumen_cut::pipeline::cleanup::apply(&doc, &mut cuts);
            lumen_cut::data::storage::write_json(&cuts_path, &cuts)?;
        }
        // Reload for export so polish/translate mutations are reflected.
        doc = Doc::load(&pid_dir)?;
    }

    report_cli_phase("exporting", 95);
    let srt_path = pid_dir.join("out.srt");
    let vtt_path = pid_dir.join("out.vtt");
    let ass_path = pid_dir.join("out.ass");
    let md_path = pid_dir.join("out.md");
    let cuts: ClipCuts = std::fs::read_to_string(pid_dir.join("cuts.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    write_srt_with(&doc, &cuts.cuts, &srt_path)?;
    write_vtt_with(&doc, &cuts.cuts, &vtt_path)?;
    write_ass(&doc, &ass_path, 1920, 1080)?;
    write_md(&doc, &md_path)?;
    report_cli_phase("completed", 100);

    Ok(AutoSummary {
        pid_dir,
        words: doc.all_words().len(),
        paragraphs: doc.paragraphs.len(),
        srt: srt_path,
        vtt: vtt_path,
        ass: ass_path,
        markdown: md_path,
        polished,
        translated,
        cuts_added,
    })
}

fn run_cut_command(cmd: CutCommand<'_>) -> AppResult<()> {
    let actions = usize::from(cmd.auto)
        + usize::from(cmd.list)
        + usize::from(cmd.add)
        + usize::from(cmd.restore.is_some())
        + usize::from(cmd.restore_all);
    if actions != 1 {
        return Err(AppError::Schema(
            "cut requires exactly one of --auto/--detect, --list, --add, --restore <id>, or --restore-all"
                .into(),
        ));
    }

    let dir = PathBuf::from(cmd.pid);
    let doc = Doc::load(&dir)?;
    let cuts_path = dir.join("cuts.json");
    let mut cuts: ClipCuts = if cuts_path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&cuts_path)?)?
    } else {
        ClipCuts::new()
    };

    if let Some(id) = cmd.restore {
        let restored = cuts.restore(id);
        if restored {
            lumen_cut::data::storage::write_json(&cuts_path, &cuts)?;
            emit!(
                cmd.json,
                serde_json::json!({"id": id, "restored": true, "total": cuts.cuts.len()}),
                "✓ cut restore {id}"
            );
        } else {
            emit!(
                cmd.json,
                serde_json::json!({"id": id, "restored": false, "total": cuts.cuts.len()}),
                "(no-op) cut {id} not found"
            );
        }
        return Ok(());
    }

    if cmd.restore_all {
        let removed = cuts.cuts.len();
        cuts.cuts.clear();
        lumen_cut::data::storage::write_json(&cuts_path, &cuts)?;
        emit!(
            cmd.json,
            serde_json::json!({"restoredAll": true, "removed": removed, "total": 0}),
            "✓ cut restore-all: removed={removed}"
        );
        return Ok(());
    }

    if cmd.list {
        let filter = cmd.kind.map(normalize_cut_kind_filter).transpose()?;
        let rows: Vec<_> = cuts
            .cuts
            .iter()
            .filter(|cut| filter.map(|kind| cut.kind == kind).unwrap_or(true))
            .map(|cut| {
                serde_json::json!({
                    "id": cut.id,
                    "kind": format!("{:?}", cut.kind).to_lowercase(),
                    "aWord": cut.a_word,
                    "bWord": cut.b_word,
                    "duration": cut.duration,
                    "note": cut.note,
                })
            })
            .collect();
        if cmd.json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "pid": cmd.pid,
                    "cuts": rows,
                    "total": rows.len(),
                }))?
            );
        } else {
            for cut in &cuts.cuts {
                if let Some(kind) = filter {
                    if cut.kind != kind {
                        continue;
                    }
                }
                println!(
                    "{} {:?} {}..{} ({:.2}s) {}",
                    cut.id,
                    cut.kind,
                    cut.a_word,
                    cut.b_word,
                    cut.duration,
                    cut.note.as_deref().unwrap_or("")
                );
            }
            println!("({} cut(s))", rows.len());
        }
        return Ok(());
    }

    if cmd.add {
        let cut = manual_cut_from_args(&doc, cmd.start, cmd.end, cmd.words, cmd.note)?;
        let id = cut.id.clone();
        cuts.add(cut);
        lumen_cut::data::storage::write_json(&cuts_path, &cuts)?;
        emit!(
            cmd.json,
            serde_json::json!({"id": id, "added": true, "total": cuts.cuts.len()}),
            "✓ cut add {id}"
        );
        return Ok(());
    }

    // --auto / --detect
    let detect_options = lumen_cut::pipeline::DetectOptions {
        min_pause: cmd.min_pause,
        compress_to: cmd.compress_to,
        sentence_end_retain: cmd.compress_to.max(0.4),
        max_gap: cmd.max_gap,
        fillers: !cmd.no_fillers,
        pauses: !cmd.no_pauses,
    };
    if cmd.dry_run {
        let hits = lumen_cut::pipeline::detect_with(&doc, detect_options);
        let proposals: Vec<_> = hits
            .iter()
            .filter_map(|hit| {
                lumen_cut::pipeline::cut_from_hit_with(&doc, hit, detect_options)
            })
            .map(|cut| {
                serde_json::json!({
                    "id": cut.id,
                    "kind": format!("{:?}", cut.kind).to_lowercase(),
                    "aWord": cut.a_word,
                    "bWord": cut.b_word,
                    "duration": cut.duration,
                    "note": cut.note,
                })
            })
            .collect();
        emit!(
            cmd.json,
            serde_json::json!({
                "pid": cmd.pid,
                "dryRun": true,
                "proposed": proposals.len(),
                "cuts": proposals,
                "options": {
                    "minPause": detect_options.min_pause,
                    "compressTo": detect_options.compress_to,
                    "maxGap": detect_options.max_gap,
                    "fillers": detect_options.fillers,
                    "pauses": detect_options.pauses,
                },
            }),
            "✓ cut detect dry-run {}: proposed={}",
            cmd.pid,
            proposals.len()
        );
        return Ok(());
    }

    let added = lumen_cut::pipeline::apply_with(&doc, &mut cuts, detect_options);
    lumen_cut::data::storage::write_json(&cuts_path, &cuts)?;
    emit!(
        cmd.json,
        serde_json::json!({"pid": cmd.pid, "added": added, "total": cuts.cuts.len()}),
        "✓ cut auto {}: added={} total={}",
        cmd.pid,
        added,
        cuts.cuts.len()
    );
    Ok(())
}

fn normalize_cut_kind_filter(raw: &str) -> AppResult<lumen_cut::data::CutKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "silence" => Ok(lumen_cut::data::CutKind::Silence),
        "filler" => Ok(lumen_cut::data::CutKind::Filler),
        "retake" => Ok(lumen_cut::data::CutKind::Retake),
        "falsestart" | "false_start" | "false-start" => Ok(lumen_cut::data::CutKind::FalseStart),
        "badtake" | "bad_take" | "bad-take" => Ok(lumen_cut::data::CutKind::BadTake),
        "manual" => Ok(lumen_cut::data::CutKind::Manual),
        other => Err(AppError::Schema(format!(
            "unknown cut kind `{other}` (expected silence|filler|retake|falsestart|badtake|manual)"
        ))),
    }
}

fn manual_cut_from_args(
    doc: &Doc,
    start: Option<f64>,
    end: Option<f64>,
    words: Option<&str>,
    note: Option<&str>,
) -> AppResult<lumen_cut::data::Cut> {
    let all = doc.all_words();
    let (a_word, b_word) = if let Some(spec) = words {
        parse_word_span(spec, &all)?
    } else {
        let start = start.ok_or_else(|| {
            AppError::Schema("cut --add requires --words a..b or --start/--end".into())
        })?;
        let end = end.ok_or_else(|| {
            AppError::Schema("cut --add requires --words a..b or --start/--end".into())
        })?;
        if end <= start {
            return Err(AppError::Schema(
                "cut --add requires --end greater than --start".into(),
            ));
        }
        let a = all
            .iter()
            .find(|w| w.end > start)
            .or_else(|| all.last())
            .ok_or_else(|| AppError::Schema("document has no words to cut".into()))?;
        let b = all
            .iter()
            .rev()
            .find(|w| w.start < end)
            .or_else(|| all.first())
            .ok_or_else(|| AppError::Schema("document has no words to cut".into()))?;
        (a.id.clone(), b.id.clone())
    };
    let word_at: std::collections::BTreeMap<&str, (f64, f64)> = all
        .iter()
        .map(|w| (w.id.as_str(), (w.start, w.end)))
        .collect();
    let dur = word_at
        .get(a_word.as_str())
        .zip(word_at.get(b_word.as_str()))
        .map(|((s, _), (_, e))| (e - s).max(0.0))
        .unwrap_or(0.0);
    Ok(lumen_cut::data::Cut {
        id: format!("c-manual-{a_word}-{b_word}"),
        note: note.map(str::to_string),
        a_word,
        b_word,
        kind: lumen_cut::data::CutKind::Manual,
        duration: dur,
    })
}

fn parse_word_span(
    spec: &str,
    words: &[&lumen_cut::data::Word],
) -> AppResult<(String, String)> {
    let parts: Vec<&str> = if spec.contains("..") {
        spec.split("..").map(str::trim).collect()
    } else {
        spec.split(',').map(str::trim).collect()
    };
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(AppError::Schema(
            "cut --words expects `wA..wB` or `wA,wB`".into(),
        ));
    }
    let ids: std::collections::BTreeSet<&str> = words.iter().map(|w| w.id.as_str()).collect();
    if !ids.contains(parts[0]) || !ids.contains(parts[1]) {
        return Err(AppError::Schema(format!(
            "cut --words references unknown id(s): {} / {}",
            parts[0], parts[1]
        )));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

async fn run_export_command(cmd: ExportCommand<'_>) -> AppResult<()> {
    if cmd.translated && cmd.bilingual {
        return Err(AppError::Schema(
            "export accepts only one of --translated or --bilingual".into(),
        ));
    }
    if (cmd.translated || cmd.bilingual) && cmd.lang.is_none() {
        return Err(AppError::Schema(
            "export --translated/--bilingual requires --lang".into(),
        ));
    }

    let dir = PathBuf::from(cmd.pid);
    let full_doc = Doc::load(&dir)?;
    let cuts_path = dir.join("cuts.json");
    let full_cuts: ClipCuts = if cuts_path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&cuts_path)?)?
    } else {
        ClipCuts::new()
    };
    let (doc, cuts) = match (cmd.start, cmd.end) {
        (Some(start), Some(end)) if end > start => {
            let clipped_doc = lumen_cut::export::clip_doc_window(&full_doc, start, end);
            let clipped_cuts = lumen_cut::export::clip_cuts_window(
                &full_doc,
                &full_cuts.cuts,
                start,
                end,
            );
            (clipped_doc, ClipCuts { cuts: clipped_cuts })
        }
        (None, None) => (full_doc, full_cuts),
        _ => {
            return Err(AppError::Schema(
                "export --start and --end must both be set, with end > start".into(),
            ));
        }
    };
    let export_settings = lumen_cut::data::export_settings::load(&dir)?;
    let hidden = lumen_cut::data::subtitle::load_hidden_checked(&dir)?;
    let caption_lang = cmd
        .lang
        .or(export_settings.subtitle_language.as_deref());
    let bilingual = if cmd.translated {
        false
    } else if cmd.bilingual {
        true
    } else {
        export_settings.bilingual_subtitles && cmd.lang.is_none()
    };
    let use_translation = cmd.translated || cmd.bilingual || caption_lang.is_some();
    let caption_doc = if use_translation {
        lumen_cut::data::export_settings::project_caption_doc_with_hidden(
            &doc,
            caption_lang,
            bilingual && !cmd.translated,
            &hidden,
        )?
    } else {
        lumen_cut::data::export_settings::project_caption_doc_with_hidden(
            &doc,
            None,
            false,
            &hidden,
        )?
    };

    let any_format = cmd.srt || cmd.vtt || cmd.ass || cmd.markdown || cmd.video || cmd.fcp;
    let write_all_text = !any_format;
    let want_srt = write_all_text || cmd.srt;
    let want_vtt = write_all_text || cmd.vtt;
    let want_ass = write_all_text || cmd.ass || cmd.video; // video burn-in needs ASS
    let want_md = write_all_text || cmd.markdown;

    let single_text = [cmd.srt, cmd.vtt, cmd.ass, cmd.markdown, cmd.video, cmd.fcp]
        .into_iter()
        .filter(|f| *f)
        .count()
        == 1;
    let output = cmd.output.map(PathBuf::from);
    if output.is_some() && !single_text && any_format {
        // Allow -o as a directory for multi-format.
        if let Some(path) = &output {
            if path.extension().is_some() {
                return Err(AppError::Schema(
                    "export -o with multiple formats must be a directory path".into(),
                ));
            }
        }
    }

    let resolve = |default_name: &str, force_ext: &str| -> PathBuf {
        match &output {
            Some(path) if single_text => {
                if path.is_dir() || path.extension().is_none() {
                    path.join(default_name)
                } else {
                    path.clone()
                }
            }
            Some(path) => {
                if path.extension().is_none() {
                    path.join(default_name)
                } else {
                    // Multi-format with file -o already rejected; fallback.
                    path.with_extension(force_ext)
                }
            }
            None => dir.join(default_name),
        }
    };

    let mut artifacts = serde_json::Map::new();
    if want_srt {
        let path = resolve("export.srt", "srt");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_srt_with(&caption_doc, &cuts.cuts, &path)?;
        artifacts.insert("srt".into(), serde_json::json!(path));
    }
    if want_vtt {
        let path = resolve("export.vtt", "vtt");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_vtt_with(&caption_doc, &cuts.cuts, &path)?;
        artifacts.insert("vtt".into(), serde_json::json!(path));
    }
    let style = lumen_cut::data::substyle::SubStyle::load(&dir)?;
    let mut ass_path_for_video: Option<PathBuf> = None;
    if want_ass {
        let path = resolve("export.ass", "ass");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_ass_with_style(&caption_doc, &cuts.cuts, &style, &path, 1920, 1080)?;
        ass_path_for_video = Some(path.clone());
        artifacts.insert("ass".into(), serde_json::json!(path));
    }
    if want_md {
        let path = resolve("export.md", "md");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Speakers flag currently uses the standard chapter markdown writer;
        // speaker names are already embedded when present on paragraphs.
        let _ = cmd.speakers;
        write_md_with_chapters(&doc, &cuts.cuts, &dir, &path)?;
        artifacts.insert("markdown".into(), serde_json::json!(path));
    }

    lumen_cut::data::cues::save(&dir, &lumen_cut::data::cues::to_cues(&doc, None))?;
    artifacts.insert("cues".into(), serde_json::json!(dir.join("cues.json")));

    let broll = lumen_cut::data::broll::load(&dir)?;
    if cmd.fcp {
        let path = resolve("export.fcpxml", "fcpxml");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        lumen_cut::export::write_fcp_with_broll(&doc, &cuts.cuts, &broll, &path, 1920, 1080)?;
        artifacts.insert("fcp".into(), serde_json::json!(path));
    }
    if cmd.video {
        let ass_path = ass_path_for_video.unwrap_or_else(|| dir.join("export.ass"));
        if !ass_path.exists() {
            write_ass_with_style(&caption_doc, &cuts.cuts, &style, &ass_path, 1920, 1080)?;
        }
        let mp4 = resolve("export.mp4", "mp4");
        if let Some(parent) = mp4.parent() {
            std::fs::create_dir_all(parent)?;
        }
        lumen_cut::export::render_video_with_broll(&doc, &cuts.cuts, &ass_path, &mp4, &broll)
            .await?;
        artifacts.insert("video".into(), serde_json::json!(mp4));
    }

    if cmd.json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "pid": cmd.pid,
                "cuts": cuts.cuts.len(),
                "lang": caption_lang,
                "translated": cmd.translated,
                "bilingual": bilingual && !cmd.translated,
                "window": match (cmd.start, cmd.end) {
                    (Some(start), Some(end)) => serde_json::json!({"start": start, "end": end}),
                    _ => serde_json::Value::Null,
                },
                "artifacts": artifacts,
            }))?
        );
    } else {
        let names: Vec<String> = artifacts
            .values()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        println!(
            "✓ export {}: applied {} cut(s) → {}",
            cmd.pid,
            cuts.cuts.len(),
            names.join(", ")
        );
    }
    Ok(())
}

// keep `Finding` reachable for documentation links
#[allow(dead_code)]
fn _finding_alias(_: Finding) {}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_cut::audit::{Code, Severity};
    use lumen_cut::data::{Paragraph, Sentence, Word};

    // Tests below mutate process env; serialise them so they cannot race.
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn lock_env() -> tokio::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().await
    }

    fn clear_sidecar_env() {
        std::env::remove_var("LUMEN_CUT_PYTHON");
        std::env::remove_var("LUMEN_CUT_DIARIZE_SCRIPT");
    }

    fn finding(severity: Severity, code: Code) -> Finding {
        Finding {
            code,
            severity,
            where_: "s1".into(),
            message: "m".into(),
        }
    }

    #[test]
    fn audit_exit_code_is_2_on_fail_only() {
        let mut r = Report::default();
        assert_eq!(audit_exit_code(&r), 0);
        r.findings
            .push(finding(Severity::Warn, Code::TranslationStale));
        r.findings
            .push(finding(Severity::Warn, Code::ZeroDurationWords));
        assert_eq!(audit_exit_code(&r), 0);
        r.findings
            .push(finding(Severity::Fail, Code::WordTimeBoundary));
        assert_eq!(audit_exit_code(&r), 2);
    }

    #[test]
    fn project_create_accepts_the_same_root_override_as_other_project_commands() {
        let cli = Cli::try_parse_from([
            "lumen-cut-cli",
            "project",
            "create",
            "demo",
            "--from",
            "/tmp/in.mp4",
            "--root",
            "/tmp/projects",
        ])
        .unwrap();
        match cli.cmd {
            Cmd::Project {
                action: ProjectCmd::Create { root, .. },
            } => assert_eq!(root, PathBuf::from("/tmp/projects")),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn auto_parses_wave1_pipeline_flags() {
        let cli = Cli::try_parse_from([
            "lumen-cut-cli",
            "auto",
            "talk.mp4",
            "--source-lang",
            "en",
            "--lang",
            "zh",
            "--no-polish",
            "--rough-cut",
            "--align-fit",
            "16",
            "--stale-only",
            "--title",
            "Demo",
        ])
        .unwrap();
        match cli.cmd {
            Cmd::Auto {
                source_lang,
                lang,
                no_polish,
                rough_cut,
                align_fit,
                stale_only,
                title,
                ..
            } => {
                assert_eq!(source_lang.as_deref(), Some("en"));
                assert_eq!(lang.as_deref(), Some("zh"));
                assert!(no_polish);
                assert!(rough_cut);
                assert_eq!(align_fit, Some(16));
                assert!(stale_only);
                assert_eq!(title.as_deref(), Some("Demo"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn export_parses_format_and_caption_mode_flags() {
        let cli = Cli::try_parse_from([
            "lumen-cut-cli",
            "export",
            "demo",
            "--srt",
            "--bilingual",
            "--lang",
            "zh",
            "-o",
            "/tmp/out.srt",
        ])
        .unwrap();
        match cli.cmd {
            Cmd::Export {
                srt,
                bilingual,
                lang,
                output,
                translated,
                ..
            } => {
                assert!(srt);
                assert!(bilingual);
                assert!(!translated);
                assert_eq!(lang.as_deref(), Some("zh"));
                assert_eq!(output, Some(PathBuf::from("/tmp/out.srt")));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn cut_parses_list_add_and_detect_flags() {
        let list = Cli::try_parse_from(["lumen-cut-cli", "cut", "demo", "--list", "--kind", "filler"])
            .unwrap();
        match list.cmd {
            Cmd::Cut { list, kind, .. } => {
                assert!(list);
                assert_eq!(kind.as_deref(), Some("filler"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
        let add = Cli::try_parse_from([
            "lumen-cut-cli",
            "cut",
            "demo",
            "--add",
            "--words",
            "w1..w3",
            "--note",
            "manual",
        ])
        .unwrap();
        match add.cmd {
            Cmd::Cut {
                add,
                words,
                note,
                ..
            } => {
                assert!(add);
                assert_eq!(words.as_deref(), Some("w1..w3"));
                assert_eq!(note.as_deref(), Some("manual"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn speakers_parses_assign_review_and_apply() {
        let assign = Cli::try_parse_from([
            "lumen-cut-cli",
            "speakers",
            "demo",
            "assign",
            "--speaker",
            "Host",
            "--paragraph",
            "2",
        ])
        .unwrap();
        match assign.cmd {
            Cmd::Speakers {
                action:
                    SpeakersCmd::Assign {
                        speaker,
                        clear,
                        paragraph,
                        cue,
                        start,
                        end,
                    },
                ..
            } => {
                assert_eq!(speaker.as_deref(), Some("Host"));
                assert!(!clear);
                assert_eq!(paragraph, Some(2));
                assert!(cue.is_none());
                assert!(start.is_none());
                assert!(end.is_none());
            }
            other => panic!("unexpected command: {other:?}"),
        }
        let review = Cli::try_parse_from([
            "lumen-cut-cli",
            "speakers",
            "demo",
            "reidentify",
            "--review",
        ])
        .unwrap();
        match review.cmd {
            Cmd::Speakers {
                action: SpeakersCmd::Reidentify { review },
                ..
            } => assert!(review),
            other => panic!("unexpected command: {other:?}"),
        }
        let apply = Cli::try_parse_from(["lumen-cut-cli", "speakers", "demo", "apply", "--all"])
            .unwrap();
        match apply.cmd {
            Cmd::Speakers {
                action: SpeakersCmd::Apply { changed_only, all },
                ..
            } => {
                assert!(changed_only);
                assert!(all);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn project_open_parses_reveal_and_desktop_flags() {
        let cli = Cli::try_parse_from([
            "lumen-cut-cli",
            "project",
            "open",
            "demo",
            "--root",
            "/tmp/projects",
            "--reveal",
            "--desktop",
        ])
        .unwrap();
        match cli.cmd {
            Cmd::Project {
                action:
                    ProjectCmd::Open {
                        pid,
                        root,
                        reveal,
                        desktop,
                    },
            } => {
                assert_eq!(pid, "demo");
                assert_eq!(root, PathBuf::from("/tmp/projects"));
                assert!(reveal);
                assert!(desktop);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn task_start_still_parses_align_groups_and_fit() {
        let cli = Cli::try_parse_from([
            "lumen-cut-cli",
            "task",
            "start",
            "align",
            "demo",
            "--lang",
            "zh",
            "--groups",
            "g1,g2",
            "--align-fit",
            "14",
        ])
        .unwrap();
        match cli.cmd {
            Cmd::Task {
                action:
                    TaskCmd::Start {
                        kind,
                        lang,
                        groups,
                        align_fit,
                        ..
                    },
            } => {
                assert_eq!(kind, "align");
                assert_eq!(lang.as_deref(), Some("zh"));
                assert_eq!(groups, vec!["g1".to_string(), "g2".to_string()]);
                assert_eq!(align_fit, Some(14));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn finish_check_fix_is_advisory_only() {
        use lumen_cut::audit::{finish_check_emit, finish_check_fix};
        use lumen_cut::data::{Paragraph, Sentence, Word};

        let doc = Doc {
            id: "p".into(),
            schema: 1,
            media: MediaRef {
                path: PathBuf::from("m.mp4"),
                duration_seconds: 10.0,
                sample_rate: None,
                channels: None,
            },
            meta: Meta {
                title: "t".into(),
                description: String::new(),
                language: Some("en".into()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            paragraphs: vec![Paragraph {
                id: 1,
                speaker: None,
                sentences: vec![Sentence {
                    id: "s1".into(),
                    text: "hello".into(),
                    words: vec![Word {
                        id: "w1".into(),
                        text: "hello".into(),
                        start: 0.0,
                        end: 1.0,
                    }],
                }],
            }],
            translations: Default::default(),
        };
        let mut cuts = ClipCuts::new();
        cuts.add(lumen_cut::data::Cut {
            id: "c1".into(),
            note: None,
            a_word: "w1".into(),
            b_word: "w1".into(),
            kind: lumen_cut::data::CutKind::Manual,
            duration: 9.0,
        });
        let before = cuts.clone();
        let items = finish_check_emit(&doc, &cuts);
        let advice = finish_check_fix("p", &items, &cuts, &doc);
        assert_eq!(cuts, before, "finish_check_fix must not mutate cuts");
        assert!(
            advice
                .suggestions
                .iter()
                .any(|s| s.contains("cut") || s.contains("audit") || s.contains("translate") || s.contains("version")),
            "expected advisory suggestions, got {advice:?}"
        );
    }

    #[test]
    fn wav_lookup_prefers_project_dir_then_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("demo");
        std::fs::create_dir_all(&dir).unwrap();
        // `auto` layout: audio.wav sits next to the project dir.
        std::fs::write(tmp.path().join("audio.wav"), b"").unwrap();
        assert_eq!(
            locate_project_wav(&dir).unwrap(),
            tmp.path().join("audio.wav")
        );
        // An in-dir audio.wav wins when both exist.
        std::fs::write(dir.join("audio.wav"), b"").unwrap();
        assert_eq!(locate_project_wav(&dir).unwrap(), dir.join("audio.wav"));
    }

    #[test]
    fn wav_lookup_missing_is_a_clear_error() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("demo");
        std::fs::create_dir_all(&dir).unwrap();
        let err = locate_project_wav(&dir).unwrap_err();
        assert!(err.to_string().contains("audio.wav"), "unexpected: {err}");
    }

    fn sample_doc() -> Doc {
        let para = |id: u32, sid: &str, wid: &str, start: f64, end: f64| Paragraph {
            id,
            speaker: None,
            sentences: vec![Sentence {
                id: sid.into(),
                text: "t".into(),
                words: vec![Word {
                    id: wid.into(),
                    text: "t".into(),
                    start,
                    end,
                }],
            }],
        };
        Doc {
            id: "demo".into(),
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
            paragraphs: vec![
                para(1, "p1s1", "w0", 0.0, 1.0),
                para(2, "p2s1", "w1", 5.0, 6.0),
            ],
            translations: Default::default(),
        }
    }

    #[test]
    fn speaker_show_turns_include_exact_cue_boundaries_and_respect_limit() {
        let mut doc = sample_doc();
        doc.paragraphs[0].speaker = Some("Ada".into());
        doc.paragraphs[1].speaker = Some("Lin".into());
        let turns = speaker_turns(&doc, Some(1), true);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].speaker, "Ada");
        assert_eq!(turns[0].first_cue, "p1s1");
        assert_eq!(turns[0].last_cue, "p1s1");
        assert_eq!(turns[0].cue_count, 1);
        assert!(!turns[0].text_truncated);
        assert!(!turns[0].cues_truncated);
        assert_eq!(turns[0].cues.as_deref(), Some(&["p1s1".to_string()][..]));
    }

    #[test]
    fn speaker_show_turns_bound_long_text_and_cue_lists_without_losing_span() {
        let mut doc = sample_doc();
        let template = doc.paragraphs[0].sentences[0].clone();
        doc.paragraphs[0].sentences = (0..(MAX_SPEAKER_TURN_CUES + 5))
            .map(|index| {
                let mut sentence = template.clone();
                sentence.id = format!("cue-{index}");
                sentence.text = "long speaker text ".repeat(8);
                sentence.words[0].id = format!("word-{index}");
                sentence.words[0].start = index as f64;
                sentence.words[0].end = index as f64 + 0.5;
                sentence
            })
            .collect();

        let turns = speaker_turns(&doc, Some(1), true);
        let turn = &turns[0];
        assert_eq!(turn.first_cue, "cue-0");
        assert_eq!(turn.last_cue, format!("cue-{}", MAX_SPEAKER_TURN_CUES + 4));
        assert_eq!(turn.cue_count, MAX_SPEAKER_TURN_CUES + 5);
        assert_eq!(turn.cues.as_ref().unwrap().len(), MAX_SPEAKER_TURN_CUES);
        assert!(turn.cues_truncated);
        assert!(turn.text_truncated);
        assert!(turn.text.chars().count() <= MAX_SPEAKER_TURN_TEXT_CHARS);
    }

    #[test]
    fn cli_progress_is_throttled_by_phase_and_five_percent_steps() {
        let mut state = CliProgressState::default();
        assert!(should_emit_cli_progress(&mut state, "recognize", 0));
        assert!(!should_emit_cli_progress(&mut state, "recognize", 4));
        assert!(should_emit_cli_progress(&mut state, "recognize", 5));
        assert!(!should_emit_cli_progress(&mut state, "recognize", 9));
        assert!(should_emit_cli_progress(&mut state, "align", 0));
        assert!(should_emit_cli_progress(&mut state, "align", 100));

        let line = format_cli_progress(
            "forced_align",
            42,
            Some(21),
            Some(50),
            Some("MLX"),
            Some(123),
            Some(456),
        );
        assert_eq!(
            line,
            "progress: forced align 42% · 21/50 · MLX · CPU 123% · memory 456 MB"
        );
        assert_eq!(
            format_cli_progress("completed", 100, None, None, None, None, None),
            "progress: completed 100%"
        );
    }

    #[tokio::test]
    async fn diarize_project_writes_speakers_back_to_doc() {
        let _env = lock_env().await;
        clear_sidecar_env();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("demo");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("audio.wav"), b"").unwrap();
        sample_doc().save(&dir).unwrap();

        // Stub "python": prints a diarize_out.v1 payload, ignores args.
        let stub = tmp.path().join("stub_python.sh");
        std::fs::write(
            &stub,
            "#!/bin/sh\nprintf '%s' '{\"schema_version\":1,\"segments\":[{\"speaker\":\"SPEAKER_A\",\"start\":0.0,\"end\":2.0},{\"speaker\":\"SPEAKER_B\",\"start\":4.0,\"end\":8.0}]}'\n",
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::env::set_var("LUMEN_CUT_PYTHON", &stub);
        std::env::set_var("LUMEN_CUT_DIARIZE_SCRIPT", &stub);

        let (segments, assigned) = diarize_project(&dir).await.unwrap();
        assert_eq!(segments, 2);
        assert_eq!(assigned, 2);

        let saved = Doc::load(&dir).unwrap();
        assert_eq!(saved.paragraphs[0].speaker.as_deref(), Some("SPEAKER_A"));
        assert_eq!(saved.paragraphs[1].speaker.as_deref(), Some("SPEAKER_B"));
        clear_sidecar_env();
    }

    #[tokio::test]
    async fn diarize_project_missing_audio_is_a_clear_error() {
        let _env = lock_env().await;
        clear_sidecar_env();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("demo");
        std::fs::create_dir_all(&dir).unwrap();
        sample_doc().save(&dir).unwrap();
        let err = diarize_project(&dir).await.unwrap_err();
        assert!(err.to_string().contains("audio.wav"), "unexpected: {err}");
    }
}
