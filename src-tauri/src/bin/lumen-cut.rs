//! `lumen-cut` CLI entry point.
//!
//! Stage 4 surface — `auto` + `task`/`align`/`diarize`/`finish-check`/`cut`/`version`/`audit`.
//!
//! Examples:
//!   lumen-cut auto samples/demo.mp4 --lang zh
//!   lumen-cut project create demo --from samples/demo.mp4
//!   lumen-cut task start translate demo --lang en
//!   lumen-cut align list demo --lang zh
//!   lumen-cut diarize demo
//!   lumen-cut finish-check demo --strict
//!   lumen-cut cut demo --auto
//!   lumen-cut version demo list
//!   lumen-cut audit demo

use std::path::{Path, PathBuf};

use chrono::Utc;
use clap::{Parser, Subcommand};
use serde::Serialize;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use lumen_cut::asr::transcribe_file_with_aligner;
use lumen_cut::audit::{audit_project, finish_check_emit_for_project, Finding, Report};
use lumen_cut::data::version::{
    commit_snapshot, create_branch, restore_snapshot, switch_branch, three_way_merge,
    working_head_is_committed, Lineage, VersionKind,
};
use lumen_cut::data::ClipCuts;
use lumen_cut::data::{Doc, MediaRef, Meta};
use lumen_cut::diarize::{assign_speakers, diarize_file};
use lumen_cut::error::{AppError, AppResult};
use lumen_cut::export::{
    write_ass, write_ass_with, write_md, write_md_with_chapters, write_srt, write_srt_with,
    write_vtt, write_vtt_with,
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
    /// Pipeline: media → audio → ASR → doc.json → srt/vtt.
    Auto {
        media: String,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        model: Option<String>,
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
    /// Soft-cut apply / restore.
    Cut {
        pid: String,
        #[arg(long)]
        auto: bool,
        #[arg(long)]
        restore: Option<String>,
    },
    /// Version control: list / 3-way merge / dump.
    Version {
        #[command(subcommand)]
        action: VersionCmd,
    },
    /// Run the project delivery audit.
    Audit { pid: String },
    /// Export subtitles with soft-cut retime applied (srt/vtt/ass/md),
    /// or burn-in video with `--video`.
    Export {
        pid: String,
        #[arg(long)]
        video: bool,
        #[arg(long)]
        fcp: bool,
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
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Show pending / done counts for `pid`.
    Status {
        pid: String,
        #[arg(long, default_value = ".")]
        root: PathBuf,
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
    Rename {
        sid: String,
        name: String,
    },
    Merge {
        from: String,
        into: String,
    },
    Reidentify,
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
            title,
            out,
            model,
        } => {
            let result = run_auto(
                &media,
                lang.as_deref(),
                title.as_deref(),
                out.as_deref(),
                model.as_deref(),
            )
            .await?;
            emit!(
                json,
                &result,
                "✓ {}: words={} paragraphs={} → srt + vtt + ass + md",
                result.pid_dir.display(),
                result.words,
                result.paragraphs
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
                root,
            } => {
                let n = task_start(
                    &kind,
                    &pid,
                    lang.as_deref(),
                    stale_only,
                    groups,
                    align_fit,
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
                    println!("pending={} done={}", st.pending, st.done);
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
        Cmd::Cut { pid, auto, restore } => {
            let dir = PathBuf::from(&pid);
            let doc = Doc::load(&dir)?;
            let cuts_path = dir.join("cuts.json");
            let mut cuts: ClipCuts = if cuts_path.exists() {
                serde_json::from_str(&std::fs::read_to_string(&cuts_path)?)?
            } else {
                ClipCuts::new()
            };
            if let Some(id) = restore {
                if cuts.restore(&id) {
                    std::fs::write(&cuts_path, serde_json::to_string_pretty(&cuts)?)?;
                    emit!(
                        json,
                        serde_json::json!({"id": id, "restored": true, "total": cuts.cuts.len()}),
                        "✓ cut restore {id}"
                    );
                } else {
                    emit!(
                        json,
                        serde_json::json!({"id": id, "restored": false, "total": cuts.cuts.len()}),
                        "(no-op) cut {id} not found"
                    );
                }
                return Ok(());
            }
            if auto {
                // Auto-detect filler/silence by adding a single filler
                // cut per detected filler word. Stage 4 ships the
                // minimal `pipeline::cleanup::detect` + apply path.
                let added = lumen_cut::pipeline::cleanup::apply(&doc, &mut cuts);
                std::fs::write(&cuts_path, serde_json::to_string_pretty(&cuts)?)?;
                emit!(
                    json,
                    serde_json::json!({"pid": pid, "added": added, "total": cuts.cuts.len()}),
                    "✓ cut auto {pid}: added={added} total={}",
                    cuts.cuts.len()
                );
                return Ok(());
            }
            return Err(AppError::Schema(
                "cut requires either --auto or --restore <cut-id>".into(),
            ));
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
        Cmd::Export { pid, video, fcp } => {
            let dir = PathBuf::from(&pid);
            let doc = Doc::load(&dir)?;
            let cuts_path = dir.join("cuts.json");
            let cuts: ClipCuts = if cuts_path.exists() {
                serde_json::from_str(&std::fs::read_to_string(&cuts_path)?)?
            } else {
                ClipCuts::new()
            };
            write_srt_with(&doc, &cuts.cuts, &dir.join("export.srt"))?;
            write_vtt_with(&doc, &cuts.cuts, &dir.join("export.vtt"))?;
            write_ass_with(&doc, &cuts.cuts, &dir.join("export.ass"), 1920, 1080)?;
            write_md_with_chapters(&doc, &cuts.cuts, &dir, &dir.join("export.md"))?;
            // Also write the portable flat `cues[]` view.
            lumen_cut::data::cues::save(&dir, &lumen_cut::data::cues::to_cues(&doc, None))?;
            let broll = lumen_cut::data::broll::load(&dir)?;
            if fcp {
                let fcp_path = dir.join("export.fcpxml");
                lumen_cut::export::write_fcp_with_broll(
                    &doc, &cuts.cuts, &broll, &fcp_path, 1920, 1080,
                )?;
                if !json {
                    println!("✓ export fcp → {}", fcp_path.display());
                }
            }
            if video {
                let ass_path = dir.join("export.ass");
                let mp4 = dir.join("export.mp4");
                lumen_cut::export::render_video_with_broll(
                    &doc, &cuts.cuts, &ass_path, &mp4, &broll,
                )
                .await?;
            }
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "pid": pid,
                        "cuts": cuts.cuts.len(),
                        "srt": dir.join("export.srt"),
                        "vtt": dir.join("export.vtt"),
                        "ass": dir.join("export.ass"),
                        "markdown": dir.join("export.md"),
                        "cues": dir.join("cues.json"),
                        "video": video.then(|| dir.join("export.mp4")),
                        "fcp": fcp.then(|| dir.join("export.fcpxml")),
                    }))?
                );
            } else if video {
                println!(
                    "✓ export {pid}: {} cut(s) → export.srt/vtt/ass/md + cut-aware {}",
                    cuts.cuts.len(),
                    dir.join("export.mp4").display()
                );
            } else {
                println!(
                    "✓ export {}: applied {} cut(s) → export.srt/vtt/ass/md",
                    pid,
                    cuts.cuts.len()
                );
            }
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
                    let hidden = subtitle::load_hidden(&dir);
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
                SpeakersCmd::Reidentify => {
                    let (segments, assigned) = diarize_project(&dir).await?;
                    emit!(
                        json,
                        serde_json::json!({"pid": pid, "segments": segments, "assigned": assigned}),
                        "✓ speakers re-run {pid}: segments={segments} speakers={assigned}"
                    );
                }
                SpeakersCmd::Rename { sid, name } => {
                    let mut doc = Doc::load(&dir)?;
                    let n = speakers::rename(&mut doc, &sid, &name);
                    doc.save(&dir)?;
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
                    let cuts: ClipCuts = std::fs::read_to_string(dir.join("cuts.json"))
                        .ok()
                        .and_then(|raw| serde_json::from_str(&raw).ok())
                        .unwrap_or_default();
                    let ass = dir.join("broll-preview.ass");
                    write_ass_with(&doc, &cuts.cuts, &ass, 1920, 1080)?;
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
    polish_quality: Option<lumen_cut::pipeline::polish::PolishQualityArtifact>,
}

#[derive(Serialize)]
struct AutoSummary {
    pid_dir: PathBuf,
    words: usize,
    paragraphs: usize,
    srt: PathBuf,
    vtt: PathBuf,
    ass: PathBuf,
    markdown: PathBuf,
}

async fn task_start(
    kind: &str,
    pid: &str,
    lang: Option<&str>,
    stale_only: bool,
    groups: Vec<String>,
    align_fit: Option<usize>,
    root: &Path,
) -> AppResult<usize> {
    let dir = root.join(pid);
    let task = lumen_cut::agent::task::prepare_task_with_task_options(
        &dir,
        kind,
        lang,
        lumen_cut::agent::task::TaskOptions {
            stale_only,
            groups,
            align_fit,
        },
    )?;
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
    if let Some(config) = lumen_cut::agent::runtime::load_bridge_config() {
        lumen_cut::agent::runtime::spawn_workers(allocator.clone(), config, capacity).await;
    } else {
        eprintln!(
            "agent waiting on http://127.0.0.1:{} (configure llmEndpoint for built-in workers)",
            local_addr.port()
        );
    }
    for prepared in &task.calls {
        allocator.enqueue(prepared.call.clone());
    }
    let applied = lumen_cut::agent::task::wait_and_apply(
        allocator,
        task,
        std::time::Duration::from_secs(30 * 60),
    )
    .await?;
    info!(kind, pid, applied, "task completed and applied");
    Ok(pending)
}

fn task_status(pid: &str, root: &Path) -> AppResult<TaskStatus> {
    let project_dir = root.join(pid);
    let (pending, done) = lumen_cut::agent::task::task_counts(&project_dir);
    let polish_quality = lumen_cut::pipeline::polish::PolishQualityArtifact::load(
        &project_dir.join("ai/polish-quality.json"),
    )
    .ok();
    Ok(TaskStatus {
        pending,
        done,
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
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cues: Option<Vec<String>>,
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
        turns.push(SpeakerTurn {
            speaker: paragraph
                .speaker
                .clone()
                .unwrap_or_else(|| "unidentified".into()),
            start: first.start,
            end: last.end,
            first_cue: first_cue.clone(),
            last_cue: last_cue.clone(),
            text: paragraph
                .sentences
                .iter()
                .map(|sentence| sentence.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            cues: include_cues.then_some(cue_ids),
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
        Some(
            diarize_file(&wav)
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
    let out = diarize_file(&wav).await?;
    let assigned = assign_speakers(&mut doc, &out.segments);
    doc.save(dir)?;
    Ok((out.segments.len(), assigned))
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

async fn run_auto(
    media: &str,
    lang: Option<&str>,
    title: Option<&str>,
    out_dir: Option<&Path>,
    model: Option<&str>,
) -> AppResult<AutoSummary> {
    let out_dir = out_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&out_dir)?;

    let media_path = if media.starts_with("http://") || media.starts_with("https://") {
        let tmpl = out_dir.join("source.%(ext)s");
        download(media, &tmpl).await?
    } else {
        PathBuf::from(media)
    };
    if !media_path.exists() {
        return Err(AppError::ProjectNotFound(media_path));
    }

    let wav = out_dir.join("audio.wav");
    extract_audio_wav(&media_path, &wav).await?;

    let info = probe(&media_path).await?;
    let pid_stem = media_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".to_string());
    let pid_dir = out_dir.join(&pid_stem);
    std::fs::create_dir_all(&pid_dir)?;

    let model_config = lumen_cut::data::modelconfig::load();
    let model = model.unwrap_or(&model_config.asr_model);
    let asr =
        transcribe_file_with_aligner(&wav, model, lang, Some(&model_config.asr_aligner)).await?;

    let mut doc: Doc = asr.into();
    doc.id = pid_stem.clone();
    doc.media = MediaRef {
        path: media_path.clone(),
        duration_seconds: info.duration_seconds,
        sample_rate: info.sample_rate,
        channels: info.channels,
    };
    doc.meta.title = title.unwrap_or(&pid_stem).to_string();
    doc.meta.updated_at = Utc::now();
    doc.save(&pid_dir)?;

    let srt_path = pid_dir.join("out.srt");
    let vtt_path = pid_dir.join("out.vtt");
    let ass_path = pid_dir.join("out.ass");
    let md_path = pid_dir.join("out.md");
    write_srt(&doc, &srt_path)?;
    write_vtt(&doc, &vtt_path)?;
    write_ass(&doc, &ass_path, 1920, 1080)?;
    write_md(&doc, &md_path)?;

    Ok(AutoSummary {
        pid_dir,
        words: doc.all_words().len(),
        paragraphs: doc.paragraphs.len(),
        srt: srt_path,
        vtt: vtt_path,
        ass: ass_path,
        markdown: md_path,
    })
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
        assert_eq!(turns[0].cues.as_deref(), Some(&["p1s1".to_string()][..]));
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
