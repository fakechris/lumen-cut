//! Task orchestration: materialise calls, validate submissions, and apply
//! completed answers back to a project.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::agent::{Allocator, PendingCall};
use crate::data::{ClipCuts, Cut, CutKind, Doc, TranslationGroup};
use crate::error::{AppError, AppResult};
use crate::pipeline::{
    pack_for_requests, SentencePacket, DEFAULT_BUDGET, MAX_LINES_PER_REQUEST,
    REQUEST_OVERHEAD_BUDGET,
};

#[derive(Debug, Clone)]
pub struct PreparedCall {
    pub call: PendingCall,
    pub pending_path: PathBuf,
    pub submitted_path: PathBuf,
    pub done_path: PathBuf,
    pub failed_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct PreparedTask {
    pub project_dir: PathBuf,
    pub kind: String,
    pub lang: Option<String>,
    pub ai_dir: PathBuf,
    pub calls: Vec<PreparedCall>,
}

pub fn set_task_state(task: &PreparedTask, state: &str, error: Option<&str>) -> AppResult<()> {
    let path = task.ai_dir.join("task.json");
    let mut value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| AppError::Schema("task metadata is not an object".into()))?;
    object.insert("state".into(), serde_json::Value::String(state.into()));
    object.insert("error".into(), serde_json::json!(error));
    crate::data::storage::write_json(&path, &value)
}

#[derive(Debug, Clone, Default)]
pub struct TaskOptions {
    pub stale_only: bool,
    pub groups: Vec<String>,
    pub align_fit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredTask {
    kind: String,
    lang: Option<String>,
    #[serde(default = "stored_task_running")]
    state: String,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    stale_only: bool,
    #[serde(default)]
    groups: Vec<String>,
    #[serde(default)]
    align_fit: Option<usize>,
}

fn stored_task_running() -> String {
    "running".into()
}

#[derive(Debug, Serialize)]
struct TranslateLine {
    id: String,
    source: String,
    #[serde(rename = "maxChars")]
    max_chars: usize,
    rt: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct TranslateContextLine {
    id: String,
    source: String,
}

#[derive(Debug, Serialize)]
struct TranslatePayload {
    lang: String,
    lines: Vec<TranslateLine>,
    #[serde(rename = "contextBefore")]
    context_before: Vec<TranslateContextLine>,
    #[serde(rename = "contextAfter")]
    context_after: Vec<TranslateContextLine>,
}

#[derive(Debug, Clone)]
struct LockedTerm {
    canonical: String,
    variants: Vec<String>,
}

#[derive(Debug, Serialize)]
struct PolishSentence {
    id: String,
    text: String,
}

#[derive(Debug, Serialize)]
struct PolishParagraph {
    id: u32,
    sentences: Vec<PolishSentence>,
}

#[derive(Debug, Serialize)]
struct PolishPayload {
    paragraphs: Vec<PolishParagraph>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimedWord {
    id: String,
    text: String,
    start: f64,
    end: f64,
    paragraph_id: u32,
    speaker: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimelinePayload {
    media_duration: f64,
    words: Vec<TimedWord>,
}

pub fn prepare_task(project_dir: &Path, kind: &str, lang: Option<&str>) -> AppResult<PreparedTask> {
    prepare_task_with_options(project_dir, kind, lang, false)
}

pub fn prepare_task_with_options(
    project_dir: &Path,
    kind: &str,
    lang: Option<&str>,
    stale_only: bool,
) -> AppResult<PreparedTask> {
    prepare_task_with_task_options(
        project_dir,
        kind,
        lang,
        TaskOptions {
            stale_only,
            ..Default::default()
        },
    )
}

pub fn prepare_task_with_task_options(
    project_dir: &Path,
    kind: &str,
    lang: Option<&str>,
    options: TaskOptions,
) -> AppResult<PreparedTask> {
    if !matches!(
        kind,
        "translate" | "align" | "polish" | "segment" | "repunct" | "chapters" | "cleanup" | "broll"
    ) {
        return Err(AppError::Schema(format!(
            "task kind `{kind}` is not implemented; supported kinds: translate, align, polish, segment, repunct, chapters, cleanup, broll"
        )));
    }
    let doc = Doc::load(project_dir)?;
    let locked_terms = load_locked_terms(project_dir);
    if kind == "cleanup" {
        let cuts_path = project_dir.join("cuts.json");
        let mut cuts: ClipCuts = std::fs::read_to_string(&cuts_path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        crate::pipeline::cleanup::apply(&doc, &mut cuts);
        crate::data::storage::write_json(&cuts_path, &cuts)?;
    }
    let ai_dir = project_dir.join("ai").join(kind);
    let pending_dir = ai_dir.join("pending");
    let done_dir = ai_dir.join("done");
    let failed_dir = ai_dir.join("failed");
    let submitted_dir = ai_dir.join("submitted");
    std::fs::create_dir_all(&pending_dir)?;
    std::fs::create_dir_all(&done_dir)?;
    std::fs::create_dir_all(&failed_dir)?;
    std::fs::create_dir_all(&submitted_dir)?;

    let contract = crate::agent::contract::contract_for_kind(kind).map(str::to_string);
    if let Some(body) = &contract {
        crate::data::storage::write(&ai_dir.join("contract.md"), body.as_bytes())?;
    }

    let payloads = match kind {
        "translate" => translate_payloads(&doc, lang, options.stale_only, &locked_terms)?,
        "align" => align_payloads(&doc, lang, &options.groups, options.align_fit)?,
        "polish" => polish_payloads(&doc)?,
        "segment" => vec![segment_payload(&doc)],
        "repunct" => vec![repunct_payload(&doc)],
        "chapters" => vec![chapters_payload(&doc)],
        "cleanup" | "broll" => vec![timeline_payload(&doc)?],
        _ => unreachable!(),
    };
    let run_id = uuid::Uuid::new_v4().simple().to_string();
    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    crate::data::storage::write_json(
        &ai_dir.join("task.json"),
        &serde_json::json!({
            "kind": kind,
            "lang": lang,
            "state": "preparing",
            "runId": &run_id,
            "staleOnly": options.stale_only,
            "groups": &options.groups,
            "alignFit": options.align_fit,
            "calls": payloads.len(),
            "startedAt": started_at,
        }),
    )?;
    let mut calls = Vec::with_capacity(payloads.len());
    for (index, payload) in payloads.into_iter().enumerate() {
        let call_id = format!("{kind}-{run_id}-{index:04}");
        let pending_path = pending_dir.join(format!("{call_id}.json"));
        let submitted_path = submitted_dir.join(format!("{call_id}.json"));
        let done_path = done_dir.join(format!("{call_id}.json"));
        let failed_path = failed_dir.join(format!("{call_id}.json"));
        let body = serde_json::to_string_pretty(&payload)?;
        crate::data::storage::write(&pending_path, body.as_bytes())?;
        calls.push(PreparedCall {
            call: PendingCall {
                id: call_id,
                kind: kind.to_string(),
                word_count: payload_word_count(&payload),
                char_count: body.chars().count(),
                payload_ref: pending_path.to_string_lossy().into_owned(),
                submission_ref: Some(submitted_path.to_string_lossy().into_owned()),
                problems: vec![],
                contract: contract.clone(),
            },
            pending_path,
            submitted_path,
            done_path,
            failed_path,
        });
    }
    crate::data::storage::write_json(
        &ai_dir.join("task.json"),
        &serde_json::json!({
            "kind": kind,
            "lang": lang,
            "state": "running",
            "runId": &run_id,
            "staleOnly": options.stale_only,
            "groups": &options.groups,
            "alignFit": options.align_fit,
            "calls": calls.len(),
            "startedAt": started_at,
        }),
    )?;
    Ok(PreparedTask {
        project_dir: project_dir.to_path_buf(),
        kind: kind.to_string(),
        lang: lang.map(str::to_string),
        ai_dir,
        calls,
    })
}

/// Rebuild an unfinished task from its durable pending requests and accepted
/// submissions. Completed/failed call files fence stale pending files left by
/// a crash between the final rename and cleanup.
pub fn load_recoverable_task(project_dir: &Path, kind: &str) -> AppResult<Option<PreparedTask>> {
    let ai_dir = project_dir.join("ai").join(kind);
    let task_path = ai_dir.join("task.json");
    if !task_path.exists() {
        return Ok(None);
    }
    let stored: StoredTask = serde_json::from_str(&std::fs::read_to_string(task_path)?)?;
    if stored.kind != kind {
        return Err(AppError::Schema(format!(
            "stored task kind `{}` does not match directory `{kind}`",
            stored.kind
        )));
    }
    if stored.state == "preparing" {
        if let Some(run_id) = stored.run_id.as_deref() {
            remove_task_run_files(&ai_dir, kind, run_id)?;
        }
        return prepare_task_with_task_options(
            project_dir,
            kind,
            stored.lang.as_deref(),
            TaskOptions {
                stale_only: stored.stale_only,
                groups: stored.groups,
                align_fit: stored.align_fit,
            },
        )
        .map(Some);
    }
    let pending_dir = ai_dir.join("pending");
    let submitted_dir = ai_dir.join("submitted");
    let done_dir = ai_dir.join("done");
    let failed_dir = ai_dir.join("failed");
    std::fs::create_dir_all(&submitted_dir)?;
    let contract = std::fs::read_to_string(ai_dir.join("contract.md")).ok();
    let mut entries = match std::fs::read_dir(&pending_dir) {
        Ok(entries) => entries.filter_map(Result::ok).collect::<Vec<_>>(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    entries.sort_by_key(|entry| entry.file_name());
    let mut calls = Vec::new();
    for entry in entries {
        let pending_path = entry.path();
        if pending_path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Some(call_id) = pending_path
            .file_stem()
            .and_then(|value| value.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        let done_path = done_dir.join(format!("{call_id}.json"));
        let failed_path = failed_dir.join(format!("{call_id}.json"));
        let submitted_path = submitted_dir.join(format!("{call_id}.json"));
        if done_path.exists() || failed_path.exists() {
            let _ = std::fs::remove_file(&pending_path);
            let _ = std::fs::remove_file(&submitted_path);
            continue;
        }
        let body = std::fs::read_to_string(&pending_path)?;
        let payload: serde_json::Value = serde_json::from_str(&body)?;
        calls.push(PreparedCall {
            call: PendingCall {
                id: call_id,
                kind: kind.to_string(),
                word_count: payload_word_count(&payload),
                char_count: body.chars().count(),
                payload_ref: pending_path.to_string_lossy().into_owned(),
                submission_ref: Some(submitted_path.to_string_lossy().into_owned()),
                problems: vec![],
                contract: contract.clone(),
            },
            pending_path,
            submitted_path,
            done_path,
            failed_path,
        });
    }
    if calls.is_empty() {
        return Ok(None);
    }
    Ok(Some(PreparedTask {
        project_dir: project_dir.to_path_buf(),
        kind: kind.to_string(),
        lang: stored.lang,
        ai_dir,
        calls,
    }))
}

pub fn load_matching_recoverable_task(
    project_dir: &Path,
    kind: &str,
    requested_lang: Option<&str>,
) -> AppResult<Option<PreparedTask>> {
    let Some(task) = load_recoverable_task(project_dir, kind)? else {
        return Ok(None);
    };
    if task.lang.as_deref() != requested_lang {
        return Err(AppError::Schema(format!(
            "unfinished {kind} task uses language `{}`; resume or finish it before starting language `{}`",
            task.lang.as_deref().unwrap_or("none"),
            requested_lang.unwrap_or("none"),
        )));
    }
    Ok(Some(task))
}

fn remove_task_run_files(ai_dir: &Path, kind: &str, run_id: &str) -> AppResult<()> {
    let prefix = format!("{kind}-{run_id}-");
    for phase in ["pending", "submitted"] {
        let dir = ai_dir.join(phase);
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for entry in entries.filter_map(Result::ok) {
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                std::fs::remove_file(entry.path())?;
            }
        }
    }
    Ok(())
}

fn load_tasks(project_dir: &Path, include_paused_and_failed: bool) -> AppResult<Vec<PreparedTask>> {
    let ai_dir = project_dir.join("ai");
    let entries = match std::fs::read_dir(&ai_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(error.into()),
    };
    let mut kinds = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    kinds.sort();
    let mut tasks = Vec::new();
    for kind in kinds {
        let task_path = ai_dir.join(&kind).join("task.json");
        if let Ok(raw) = std::fs::read_to_string(&task_path) {
            let stored: StoredTask = serde_json::from_str(&raw)?;
            if stored.state == "completed"
                || (!include_paused_and_failed
                    && matches!(stored.state.as_str(), "paused" | "failed"))
            {
                continue;
            }
        }
        if let Some(task) = load_recoverable_task(project_dir, &kind)? {
            tasks.push(task);
        }
    }
    Ok(tasks)
}

pub fn load_recoverable_tasks(project_dir: &Path) -> AppResult<Vec<PreparedTask>> {
    load_tasks(project_dir, false)
}

/// Explicit user resume includes paused and failed apply loops. Automatic
/// startup recovery intentionally uses [`load_recoverable_tasks`] instead.
pub fn load_resumable_tasks(project_dir: &Path) -> AppResult<Vec<PreparedTask>> {
    load_tasks(project_dir, true)
}

pub fn prepare_retry_task(project_dir: &Path, kind: &str) -> AppResult<PreparedTask> {
    let path = project_dir.join("ai").join(kind).join("task.json");
    let stored: StoredTask = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
    if stored.kind != kind {
        return Err(AppError::Schema(format!(
            "stored task kind `{}` does not match retry kind `{kind}`",
            stored.kind
        )));
    }
    prepare_task_with_task_options(
        project_dir,
        kind,
        stored.lang.as_deref(),
        TaskOptions {
            stale_only: stored.stale_only || kind == "translate",
            groups: stored.groups,
            align_fit: stored.align_fit,
        },
    )
}

/// Restore already-ACKed worker results and enqueue only calls that still need
/// model work. Returns the number of submissions recovered from disk.
pub fn restore_or_enqueue(allocator: &Allocator, task: &PreparedTask) -> AppResult<usize> {
    let mut restored = 0;
    for prepared in &task.calls {
        if prepared.submitted_path.exists() {
            let submission: crate::agent::CompletedSubmission =
                serde_json::from_str(&std::fs::read_to_string(&prepared.submitted_path)?)?;
            if submission.call_id != prepared.call.id {
                return Err(AppError::Schema(format!(
                    "submission {} belongs to another call",
                    prepared.submitted_path.display()
                )));
            }
            allocator.restore_completed(submission);
            restored += 1;
        } else if !allocator.contains_call(&prepared.call.id) {
            allocator.enqueue(prepared.call.clone());
        }
    }
    Ok(restored)
}

/// Count every pending/completed call across task kinds. Status is a project
/// view, not a translate-only view.
pub fn task_counts(project_dir: &Path) -> (usize, usize) {
    let ai = project_dir.join("ai");
    let mut pending = 0;
    let mut done = 0;
    let Ok(kinds) = std::fs::read_dir(ai) else {
        return (0, 0);
    };
    for kind in kinds
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
    {
        let count = |name: &str| {
            std::fs::read_dir(kind.path().join(name))
                .map(|entries| entries.filter_map(Result::ok).count())
                .unwrap_or_default()
        };
        let failed_names: std::collections::BTreeSet<std::ffi::OsString> =
            std::fs::read_dir(kind.path().join("failed"))
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|entry| entry.file_name())
                .collect();
        let done_names: std::collections::BTreeSet<std::ffi::OsString> =
            std::fs::read_dir(kind.path().join("done"))
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|entry| entry.file_name())
                .collect();
        pending += std::fs::read_dir(kind.path().join("pending"))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|entry| {
                        !failed_names.contains(&entry.file_name())
                            && !done_names.contains(&entry.file_name())
                    })
                    .count()
            })
            .unwrap_or_default();
        done += count("done");
    }
    (pending, done)
}

fn timeline_payload(doc: &Doc) -> AppResult<serde_json::Value> {
    let words = doc
        .paragraphs
        .iter()
        .flat_map(|paragraph| {
            paragraph.sentences.iter().flat_map(move |sentence| {
                sentence.words.iter().map(move |word| TimedWord {
                    id: word.id.clone(),
                    text: word.text.clone(),
                    start: word.start,
                    end: word.end,
                    paragraph_id: paragraph.id,
                    speaker: paragraph.speaker.clone(),
                })
            })
        })
        .collect();
    Ok(serde_json::to_value(TimelinePayload {
        media_duration: doc.media.duration_seconds,
        words,
    })?)
}

fn translate_payloads(
    doc: &Doc,
    lang: Option<&str>,
    stale_only: bool,
    locked_terms: &[LockedTerm],
) -> AppResult<Vec<serde_json::Value>> {
    let lang = lang
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AppError::Schema("translate requires a target language".into()))?;
    let all_packets: Vec<SentencePacket> = doc
        .paragraphs
        .iter()
        .flat_map(|paragraph| paragraph.sentences.iter())
        .map(|sentence| SentencePacket {
            sentence_id: sentence.id.clone(),
            text: sentence.text.clone(),
            word_count: sentence.words.len(),
        })
        .collect();
    let packets: Vec<SentencePacket> = all_packets
        .iter()
        .filter(|sentence| {
            !stale_only
                || doc
                    .translations
                    .get(lang)
                    .and_then(|groups| groups.get(&sentence.sentence_id))
                    .is_none_or(|group| {
                        group.source_text.as_deref() != Some(sentence.text.as_str())
                    })
        })
        .cloned()
        .collect();
    let max_chars = crate::pipeline::translate::hard_chars_for_lang(lang);
    let positions: HashMap<&str, usize> = all_packets
        .iter()
        .enumerate()
        .map(|(index, sentence)| (sentence.sentence_id.as_str(), index))
        .collect();
    let request_budget = DEFAULT_BUDGET.saturating_sub(REQUEST_OVERHEAD_BUDGET);
    pack_for_requests(packets, request_budget, MAX_LINES_PER_REQUEST, lang)
        .into_iter()
        .map(|batch| {
            if batch.estimated_tokens > request_budget {
                return Err(AppError::Schema(
                    "a source subtitle is too large for one translation request".into(),
                ));
            }
            let first = batch
                .sentences
                .first()
                .and_then(|sentence| positions.get(sentence.sentence_id.as_str()))
                .copied()
                .unwrap_or_default();
            let last = batch
                .sentences
                .last()
                .and_then(|sentence| positions.get(sentence.sentence_id.as_str()))
                .copied()
                .unwrap_or(first);
            let context_line = |sentence: &SentencePacket| TranslateContextLine {
                id: sentence.sentence_id.clone(),
                source: sentence.text.chars().take(100).collect(),
            };
            let after_start = last.saturating_add(1);
            let after_end = after_start.saturating_add(3).min(all_packets.len());
            serde_json::to_value(TranslatePayload {
                lang: lang.to_string(),
                context_before: all_packets[first.saturating_sub(3)..first]
                    .iter()
                    .map(context_line)
                    .collect(),
                context_after: all_packets[after_start..after_end]
                    .iter()
                    .map(context_line)
                    .collect(),
                lines: batch
                    .sentences
                    .into_iter()
                    .map(|sentence| TranslateLine {
                        id: sentence.sentence_id,
                        rt: locked_terms
                            .iter()
                            .filter(|term| {
                                std::iter::once(term.canonical.as_str())
                                    .chain(term.variants.iter().map(String::as_str))
                                    .any(|candidate| {
                                        sentence
                                            .text
                                            .to_lowercase()
                                            .contains(&candidate.to_lowercase())
                                    })
                            })
                            .map(|term| term.canonical.clone())
                            .take(2)
                            .map(|term| term.chars().take(32).collect())
                            .collect(),
                        source: sentence.text,
                        max_chars,
                    })
                    .collect(),
            })
            .map_err(AppError::from)
        })
        .collect()
}

fn load_locked_terms(project_dir: &Path) -> Vec<LockedTerm> {
    let value: serde_json::Value =
        match std::fs::read_to_string(project_dir.join("ai/analysis.json"))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
        {
            Some(value) => value,
            None => return Vec::new(),
        };
    value["terms"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|term| term["locked"].as_bool() == Some(true))
        .filter_map(|term| {
            Some(LockedTerm {
                canonical: term["term"].as_str()?.to_string(),
                variants: term["observedVariants"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|variant| variant.as_str().map(str::to_string))
                    .collect(),
            })
        })
        .collect()
}

fn projected_cells(text: &str) -> f64 {
    text.chars()
        .map(|character| {
            if character.is_whitespace() || character.is_ascii_punctuation() {
                0.0
            } else if character.is_ascii() {
                0.5
            } else {
                1.0
            }
        })
        .sum()
}

fn source_with_seams(doc: &Doc, source_words: &[String]) -> AppResult<String> {
    let words: std::collections::BTreeMap<&str, &crate::data::Word> = doc
        .all_words()
        .into_iter()
        .map(|word| (word.id.as_str(), word))
        .collect();
    let mut output = String::new();
    for (index, id) in source_words.iter().enumerate() {
        let word = words.get(id.as_str()).ok_or_else(|| {
            AppError::Schema(format!("align group references unknown word `{id}`"))
        })?;
        if index > 0 {
            output.push(' ');
            output.push_str(&format!("<@{id}>"));
        }
        output.push_str(&word.text);
    }
    Ok(output)
}

fn target_with_seams(text: &str) -> String {
    let mut output = String::from("<#0>");
    for (index, character) in text.chars().enumerate() {
        if index > 0 {
            output.push_str(&format!("<@t{index}>"));
        }
        output.push(character);
    }
    output.push_str("<#1>");
    output
}

fn align_payloads(
    doc: &Doc,
    lang: Option<&str>,
    groups: &[String],
    align_fit: Option<usize>,
) -> AppResult<Vec<serde_json::Value>> {
    let lang = lang
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AppError::Schema("align requires a target language".into()))?;
    let translations = doc
        .translations
        .get(lang)
        .ok_or_else(|| AppError::Schema(format!("no `{lang}` translations to align")))?;
    let fit = align_fit
        .unwrap_or_else(|| crate::pipeline::translate::aim_chars_for_lang(lang))
        .clamp(8, 32);
    let requested: BTreeSet<&str> = groups.iter().map(String::as_str).collect();
    for id in &requested {
        if !translations.contains_key(*id) {
            return Err(AppError::Schema(format!(
                "align group `{id}` does not exist in `{lang}`"
            )));
        }
    }

    let word_times: std::collections::BTreeMap<&str, (f64, f64)> = doc
        .all_words()
        .into_iter()
        .map(|word| (word.id.as_str(), (word.start, word.end)))
        .collect();
    let mut pairs = Vec::new();
    for (id, group) in translations {
        let cells = projected_cells(&group.text);
        if (!requested.is_empty() && !requested.contains(id.as_str()))
            || (requested.is_empty() && cells <= fit as f64)
        {
            continue;
        }
        if group.source_words.is_empty() {
            return Err(AppError::Schema(format!(
                "align group `{id}` has no source-word provenance"
            )));
        }
        let mut problems = Vec::new();
        if cells > fit as f64 {
            problems.push("overFit");
        }
        if cells > 20.0 {
            problems.push("overHard");
        }
        let mut advisory = Vec::new();
        let duration = group
            .source_words
            .first()
            .and_then(|first| word_times.get(first.as_str()))
            .zip(
                group
                    .source_words
                    .last()
                    .and_then(|last| word_times.get(last.as_str())),
            )
            .map(|(first, last)| (last.1 - first.0).max(0.001))
            .unwrap_or(0.001);
        if cells / duration > 9.0 {
            advisory.push("advisory-cps");
        }
        pairs.push(serde_json::json!({
            "id": id,
            "sm": source_with_seams(doc, &group.source_words)?,
            "tm": target_with_seams(&group.text),
            "problems": problems,
            "advisory": advisory,
            "pt": [],
            "rt": [],
        }));
    }
    if pairs.is_empty() {
        return Ok(Vec::new());
    }
    Ok(vec![serde_json::json!({
        "lang": lang,
        "budgets": {"s": 60, "t": 14, "f": fit},
        "pairs": pairs,
    })])
}

fn polish_payloads(doc: &Doc) -> AppResult<Vec<serde_json::Value>> {
    doc.paragraphs
        .iter()
        .map(|paragraph| {
            serde_json::to_value(PolishPayload {
                paragraphs: vec![PolishParagraph {
                    id: paragraph.id,
                    sentences: paragraph
                        .sentences
                        .iter()
                        .map(|sentence| PolishSentence {
                            id: sentence.id.clone(),
                            text: sentence.text.clone(),
                        })
                        .collect(),
                }],
            })
            .map_err(AppError::from)
        })
        .collect()
}

fn repunct_payload(doc: &Doc) -> serde_json::Value {
    let segs: Vec<_> = doc
        .paragraphs
        .iter()
        .map(|paragraph| {
            let words: Vec<_> = paragraph
                .sentences
                .iter()
                .flat_map(|sentence| sentence.words.iter())
                .collect();
            let cm: Vec<_> = words
                .iter()
                .enumerate()
                .map(|(index, word)| {
                    serde_json::json!({
                        "id": format!("c-{}", word.id),
                        "wordId": word.id,
                        "left": word.text,
                        "right": words.get(index + 1).map(|next| next.text.as_str()),
                    })
                })
                .collect();
            serde_json::json!({
                "id": paragraph.id,
                "text": paragraph
                    .sentences
                    .iter()
                    .map(|sentence| sentence.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                "cm": cm,
            })
        })
        .collect();
    serde_json::json!({"segs": segs})
}

fn segment_payload(doc: &Doc) -> serde_json::Value {
    let sentences: Vec<_> = doc
        .paragraphs
        .iter()
        .flat_map(|paragraph| paragraph.sentences.iter())
        .map(|sentence| serde_json::json!({"id": sentence.id, "text": sentence.text}))
        .collect();
    let text = sentences
        .iter()
        .filter_map(|sentence| sentence["text"].as_str())
        .collect::<Vec<_>>()
        .join(" ");
    serde_json::json!({"text": text, "sentences": sentences})
}

fn chapters_payload(doc: &Doc) -> serde_json::Value {
    let segments: Vec<_> = doc
        .paragraphs
        .iter()
        .flat_map(|paragraph| paragraph.sentences.iter())
        .map(|sentence| {
            let start = sentence
                .words
                .first()
                .map(|word| word.start)
                .unwrap_or_default();
            serde_json::json!({
                "id": sentence.id,
                "start": start,
                "line": format!("[{} · {:02}:{:02}] {}", sentence.id, (start / 60.0) as u64, start as u64 % 60, sentence.text),
            })
        })
        .collect();
    serde_json::json!({"segments": segments})
}

fn payload_word_count(payload: &serde_json::Value) -> usize {
    payload.to_string().split_whitespace().count().max(1)
}

pub fn validate_call_answer(call: &PendingCall, answer: &str) -> Result<(), Vec<String>> {
    if !matches!(
        call.kind.as_str(),
        "translate" | "align" | "polish" | "segment" | "repunct" | "chapters" | "cleanup" | "broll"
    ) {
        return (!answer.trim().is_empty())
            .then_some(())
            .ok_or_else(|| vec!["empty answer".into()]);
    }
    let answer_value = parse_answer_value(&call.kind, answer)?;
    let payload_raw =
        std::fs::read_to_string(&call.payload_ref).map_err(|e| vec![format!("payload: {e}")])?;
    let payload: serde_json::Value =
        serde_json::from_str(&payload_raw).map_err(|e| vec![format!("payload JSON: {e}")])?;
    match call.kind.as_str() {
        "translate" => validate_translate(&payload, &answer_value),
        "align" => validate_align(&payload, &answer_value),
        "polish" => validate_polish(&payload, &answer_value),
        "segment" => validate_segment(&payload, &answer_value),
        "repunct" => validate_repunct(&payload, &answer_value),
        "chapters" => validate_chapters(&payload, &answer_value),
        "cleanup" => validate_cleanup(&payload, &answer_value),
        "broll" => validate_broll(&payload, &answer_value),
        _ => unreachable!(),
    }
}

fn parse_answer_value(kind: &str, answer: &str) -> Result<serde_json::Value, Vec<String>> {
    if kind != "chapters" {
        return serde_json::from_str(answer.trim()).map_err(|e| vec![format!("not JSON: {e}")]);
    }
    let mut chapters = Vec::new();
    for (index, line) in answer
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
    {
        let value: serde_json::Value = serde_json::from_str(line.trim())
            .map_err(|error| vec![format!("NDJSON line {}: {error}", index + 1)])?;
        if !value.is_object() {
            return Err(vec![format!("NDJSON line {} is not an object", index + 1)]);
        }
        chapters.push(value);
    }
    if chapters.is_empty() {
        return Err(vec!["zero lines".into()]);
    }
    Ok(serde_json::json!({"chapters": chapters}))
}

fn marker_ids(text: &str) -> Vec<&str> {
    let mut ids = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<@") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find('>') else {
            break;
        };
        ids.push(&rest[..end]);
        rest = &rest[end + 1..];
    }
    ids
}

fn text_without_markers(text: &str) -> String {
    let mut output = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        output.push_str(&rest[..start]);
        let marker = &rest[start..];
        if (marker.starts_with("<@") || marker.starts_with("<#")) && marker.find('>').is_some() {
            let end = marker.find('>').unwrap();
            rest = &marker[end + 1..];
        } else {
            output.push('<');
            rest = &marker[1..];
        }
    }
    output.push_str(rest);
    output
}

fn align_text_issue(text: &str) -> Option<&'static str> {
    if text.contains('⋯') || text.contains("...") {
        return Some("U+22EF/ASCII ellipsis");
    }
    if text.contains("?!") || text.contains("!?") {
        return Some("repeated question-exclamation");
    }
    if text
        .chars()
        .any(|character| ('０'..='９').contains(&character))
    {
        return Some("fullwidth digits");
    }
    None
}

fn validate_align(
    payload: &serde_json::Value,
    answer: &serde_json::Value,
) -> Result<(), Vec<String>> {
    let payload_pairs = payload["pairs"]
        .as_array()
        .ok_or_else(|| vec!["payload missing pairs".into()])?;
    let answer_pairs = answer["pairs"]
        .as_array()
        .ok_or_else(|| vec!["missing `pairs` array".into()])?;
    let expected: std::collections::BTreeMap<&str, &serde_json::Value> = payload_pairs
        .iter()
        .filter_map(|pair| Some((pair["id"].as_str()?, pair)))
        .collect();
    let hard: BTreeSet<&str> = expected
        .iter()
        .filter(|(_, pair)| {
            pair["problems"]
                .as_array()
                .is_some_and(|problems| !problems.is_empty())
        })
        .map(|(id, _)| *id)
        .collect();
    let mut returned = BTreeSet::new();
    for (index, change) in answer_pairs.iter().enumerate() {
        let id = change["id"]
            .as_str()
            .ok_or_else(|| vec![format!("pairs[{index}] missing id")])?;
        let Some(pair) = expected.get(id) else {
            return Err(vec![format!("pairs[{index}] unknown candidate `{id}`")]);
        };
        if !returned.insert(id) {
            return Err(vec![format!("pairs[{index}] duplicate id `{id}`")]);
        }
        let source_markers = marker_ids(pair["sm"].as_str().unwrap_or_default());
        let target_markers = marker_ids(pair["tm"].as_str().unwrap_or_default());
        match change["action"].as_str() {
            Some("recut") => {
                let cuts = change["cuts"]
                    .as_array()
                    .ok_or_else(|| vec![format!("pairs[{index}] recut missing cuts")])?;
                if cuts.is_empty() {
                    return Err(vec![format!(
                        "pairs[{index}] recut must change at least one boundary"
                    )]);
                }
                let mut last_source = None;
                let mut last_target = None;
                let mut target_boundaries = Vec::new();
                for (cut_index, cut) in cuts.iter().enumerate() {
                    let source = cut["s"].as_str().ok_or_else(|| {
                        vec![format!("pairs[{index}].cuts[{cut_index}] missing s")]
                    })?;
                    let target = cut["t"].as_str().ok_or_else(|| {
                        vec![format!("pairs[{index}].cuts[{cut_index}] missing t")]
                    })?;
                    let Some(source_rank) = source_markers
                        .iter()
                        .position(|candidate| *candidate == source)
                    else {
                        return Err(vec![format!(
                            "pairs[{index}].cuts[{cut_index}] unknown candidate `{source}`"
                        )]);
                    };
                    let Some(target_rank) = target_markers
                        .iter()
                        .position(|candidate| *candidate == target)
                    else {
                        return Err(vec![format!(
                            "pairs[{index}].cuts[{cut_index}] unknown candidate `{target}`"
                        )]);
                    };
                    if last_source.is_some_and(|last| source_rank <= last)
                        || last_target.is_some_and(|last| target_rank <= last)
                    {
                        return Err(vec![format!(
                            "pairs[{index}].cuts[{cut_index}] unordered candidate"
                        )]);
                    }
                    last_source = Some(source_rank);
                    last_target = Some(target_rank);
                    target_boundaries.push(target_rank + 1);
                }
                let target_text = text_without_markers(pair["tm"].as_str().unwrap_or_default());
                let target_chars: Vec<char> = target_text.chars().collect();
                let mut boundaries = Vec::with_capacity(target_boundaries.len() + 2);
                boundaries.push(0);
                boundaries.extend(target_boundaries);
                boundaries.push(target_chars.len());
                for window in boundaries.windows(2) {
                    let unit: String = target_chars[window[0]..window[1]].iter().collect();
                    if projected_cells(&unit) > 20.0 {
                        return Err(vec![format!("pairs[{index}] unit over hard")]);
                    }
                }
            }
            Some("rewrite") => {
                if !matches!(
                    change["reasonCode"].as_str(),
                    Some(
                        "mistranslation"
                            | "omission"
                            | "terminology"
                            | "grammar"
                            | "translationese"
                            | "reorder"
                    )
                ) {
                    return Err(vec![format!(
                        "pairs[{index}] rewrite missing/invalid reasonCode"
                    )]);
                }
                let pieces = change["pieces"]
                    .as_array()
                    .ok_or_else(|| vec![format!("pairs[{index}] rewrite missing pieces")])?;
                if pieces.is_empty() {
                    return Err(vec![format!("pairs[{index}] rewrite has no pieces")]);
                }
                let mut last_rank = None;
                for (piece_index, piece) in pieces.iter().enumerate() {
                    let through = piece["through"].as_str().ok_or_else(|| {
                        vec![format!(
                            "pairs[{index}].pieces[{piece_index}] missing through"
                        )]
                    })?;
                    let rank = if through == "end" {
                        source_markers.len()
                    } else {
                        source_markers
                            .iter()
                            .position(|candidate| *candidate == through)
                            .ok_or_else(|| {
                                vec![format!(
                                    "pairs[{index}].pieces[{piece_index}] unknown candidate `{through}`"
                                )]
                            })?
                    };
                    if last_rank.is_some_and(|last| rank <= last) {
                        return Err(vec![format!(
                            "pairs[{index}].pieces[{piece_index}] unordered candidate"
                        )]);
                    }
                    last_rank = Some(rank);
                    let text = piece["t"]
                        .as_str()
                        .filter(|text| !text.trim().is_empty())
                        .ok_or_else(|| {
                            vec![format!("pairs[{index}].pieces[{piece_index}] empty text")]
                        })?;
                    if let Some(issue) = align_text_issue(text) {
                        return Err(vec![format!(
                            "pairs[{index}].pieces[{piece_index}] {issue}"
                        )]);
                    }
                    if projected_cells(text) > 20.0 {
                        return Err(vec![format!(
                            "pairs[{index}].pieces[{piece_index}] unit over hard"
                        )]);
                    }
                }
                if pieces.last().and_then(|piece| piece["through"].as_str()) != Some("end") {
                    return Err(vec![format!("pairs[{index}] incomplete coverage")]);
                }
            }
            Some(action) => return Err(vec![format!("pairs[{index}] invalid action `{action}`")]),
            None => return Err(vec![format!("pairs[{index}] missing action")]),
        }
    }
    let missing: Vec<&str> = hard.difference(&returned).copied().collect();
    if !missing.is_empty() {
        return Err(vec![format!("incomplete coverage: {missing:?}")]);
    }
    Ok(())
}

fn is_repunct_mark(mark: &str) -> bool {
    matches!(
        mark,
        "，" | "。" | "、" | "？" | "！" | "；" | "：" | "…" | "," | "." | "?" | "!" | ";" | ":"
    )
}

fn validate_repunct(
    payload: &serde_json::Value,
    answer: &serde_json::Value,
) -> Result<(), Vec<String>> {
    let payload_segs = payload["segs"]
        .as_array()
        .ok_or_else(|| vec!["payload missing segs".into()])?;
    let candidates: std::collections::BTreeMap<u64, BTreeSet<&str>> = payload_segs
        .iter()
        .filter_map(|segment| {
            Some((
                segment["id"].as_u64()?,
                segment["cm"]
                    .as_array()?
                    .iter()
                    .filter_map(|candidate| candidate["id"].as_str())
                    .collect(),
            ))
        })
        .collect();
    let segs = answer["segs"]
        .as_array()
        .ok_or_else(|| vec!["missing `segs` array".into()])?;
    let mut seen_segments = BTreeSet::new();
    let mut seen_cuts = BTreeSet::new();
    for (segment_index, segment) in segs.iter().enumerate() {
        let id = segment["id"]
            .as_u64()
            .ok_or_else(|| vec![format!("segs[{segment_index}] missing id")])?;
        let Some(allowed) = candidates.get(&id) else {
            return Err(vec![format!("segs[{segment_index}] unknown segment id")]);
        };
        if !seen_segments.insert(id) {
            return Err(vec![format!("segs[{segment_index}] duplicate segment id")]);
        }
        let cuts = segment["cuts"]
            .as_array()
            .ok_or_else(|| vec![format!("segs[{segment_index}] missing cuts")])?;
        for (cut_index, cut) in cuts.iter().enumerate() {
            let candidate = cut["id"].as_str().ok_or_else(|| {
                vec![format!(
                    "segs[{segment_index}].cuts[{cut_index}] missing id"
                )]
            })?;
            if !allowed.contains(candidate) {
                return Err(vec![format!(
                    "segs[{segment_index}].cuts[{cut_index}] unknown candidate id"
                )]);
            }
            if !seen_cuts.insert(candidate) {
                return Err(vec![format!(
                    "segs[{segment_index}].cuts[{cut_index}] duplicate candidate id"
                )]);
            }
            let mark = cut["m"].as_str().ok_or_else(|| {
                vec![format!(
                    "segs[{segment_index}].cuts[{cut_index}] missing mark"
                )]
            })?;
            if !is_repunct_mark(mark) {
                return Err(vec![format!(
                    "segs[{segment_index}].cuts[{cut_index}] mark not in palette"
                )]);
            }
        }
    }
    Ok(())
}

fn segment_ranges(
    payload: &serde_json::Value,
    answer: &serde_json::Value,
) -> Result<Vec<(usize, usize)>, Vec<String>> {
    let source: Vec<&str> = payload["sentences"]
        .as_array()
        .ok_or_else(|| vec!["payload missing sentences".into()])?
        .iter()
        .filter_map(|sentence| sentence["text"].as_str())
        .collect();
    let paragraphs = answer["paragraphs"]
        .as_array()
        .ok_or_else(|| vec!["no paragraphs".into()])?;
    if paragraphs.is_empty() {
        return Err(vec!["no paragraphs".into()]);
    }
    let mut cursor = 0;
    let mut ranges = Vec::with_capacity(paragraphs.len());
    for (index, paragraph) in paragraphs.iter().enumerate() {
        let text = paragraph
            .as_str()
            .filter(|text| !text.is_empty())
            .ok_or_else(|| vec![format!("paragraphs[{index}] must be non-empty text")])?;
        let Some(end) =
            ((cursor + 1)..=source.len()).find(|end| source[cursor..*end].join(" ") == text)
        else {
            return Err(vec![format!(
                "paragraphs[{index}] verbatim violated or mid-sentence break"
            )]);
        };
        ranges.push((cursor, end));
        cursor = end;
    }
    if cursor != source.len() {
        return Err(vec!["paragraph count mismatch".into()]);
    }
    Ok(ranges)
}

fn validate_segment(
    payload: &serde_json::Value,
    answer: &serde_json::Value,
) -> Result<(), Vec<String>> {
    segment_ranges(payload, answer).map(|_| ())
}

fn validate_chapters(
    payload: &serde_json::Value,
    answer: &serde_json::Value,
) -> Result<(), Vec<String>> {
    let segments = payload["segments"]
        .as_array()
        .ok_or_else(|| vec!["payload missing segments".into()])?;
    if segments.is_empty() {
        return Err(vec!["payload has zero segments".into()]);
    }
    let order: std::collections::BTreeMap<&str, usize> = segments
        .iter()
        .enumerate()
        .filter_map(|(index, segment)| Some((segment["id"].as_str()?, index)))
        .collect();
    let chapters = answer["chapters"]
        .as_array()
        .ok_or_else(|| vec!["zero lines".into()])?;
    if chapters.is_empty() {
        return Err(vec!["zero lines".into()]);
    }
    let mut previous = None;
    for (index, chapter) in chapters.iter().enumerate() {
        let title = chapter["title"]
            .as_str()
            .filter(|title| !title.trim().is_empty())
            .ok_or_else(|| vec![format!("chapter line {} empty title", index + 1)])?;
        let _ = title;
        let id = chapter["startSeg"]
            .as_str()
            .ok_or_else(|| vec![format!("chapter line {} missing startSeg", index + 1)])?;
        let Some(&rank) = order.get(id) else {
            return Err(vec![format!(
                "chapter line {} unknown id `{id}`",
                index + 1
            )]);
        };
        if index == 0 && rank != 0 {
            return Err(vec!["first not first segment".into()]);
        }
        if previous.is_some_and(|last| rank <= last) {
            return Err(vec!["unordered startSeg".into()]);
        }
        previous = Some(rank);
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct PayloadWord<'a> {
    index: usize,
    start: f64,
    end: f64,
    paragraph_id: u64,
    speaker: Option<&'a str>,
}

fn payload_word_map(
    payload: &serde_json::Value,
) -> Result<std::collections::BTreeMap<&str, PayloadWord<'_>>, Vec<String>> {
    let words = payload["words"]
        .as_array()
        .ok_or_else(|| vec!["payload missing words".into()])?;
    Ok(words
        .iter()
        .enumerate()
        .filter_map(|(index, word)| {
            Some((
                word["id"].as_str()?,
                PayloadWord {
                    index,
                    start: word["start"].as_f64()?,
                    end: word["end"].as_f64()?,
                    paragraph_id: word["paragraphId"].as_u64()?,
                    speaker: word["speaker"].as_str(),
                },
            ))
        })
        .collect())
}

fn validate_cleanup(
    payload: &serde_json::Value,
    answer: &serde_json::Value,
) -> Result<(), Vec<String>> {
    let words = payload_word_map(payload)?;
    let cuts = answer["cuts"]
        .as_array()
        .ok_or_else(|| vec!["missing `cuts` array".into()])?;
    let mut occupied = Vec::new();
    for (index, cut) in cuts.iter().enumerate() {
        let a = cut["a"]
            .as_str()
            .ok_or_else(|| vec![format!("cuts[{index}] missing `a`")])?;
        let b = cut["b"]
            .as_str()
            .ok_or_else(|| vec![format!("cuts[{index}] missing `b`")])?;
        let Some(&a_word) = words.get(a) else {
            return Err(vec![format!("cuts[{index}] unknown start word `{a}`")]);
        };
        let Some(&b_word) = words.get(b) else {
            return Err(vec![format!("cuts[{index}] unknown end word `{b}`")]);
        };
        if a_word.index > b_word.index
            || a_word.paragraph_id != b_word.paragraph_id
            || a_word.speaker != b_word.speaker
        {
            return Err(vec![format!("cuts[{index}] partition break")]);
        }
        let cat = cut["cat"]
            .as_str()
            .ok_or_else(|| vec![format!("cuts[{index}] missing `cat`")])?;
        if !matches!(cat, "retake" | "filler" | "falseStart" | "silence") {
            return Err(vec![format!("cuts[{index}] invalid cat `{cat}`")]);
        }
        if cat == "retake"
            && cut["alt"]
                .as_array()
                .is_none_or(|alt| alt.len() != 2 || alt.iter().any(|id| id.as_str().is_none()))
        {
            return Err(vec![format!("cuts[{index}] retake missing alt")]);
        }
        if cut["reason"]
            .as_str()
            .is_none_or(|reason| reason.trim().is_empty())
        {
            return Err(vec![format!("cuts[{index}] no provenance")]);
        }
        if occupied
            .iter()
            .any(|(start, end)| a_word.index <= *end && *start <= b_word.index)
        {
            return Err(vec![format!("cuts[{index}] overlaps another cut")]);
        }
        occupied.push((a_word.index, b_word.index));
    }
    Ok(())
}

fn validate_broll(
    payload: &serde_json::Value,
    answer: &serde_json::Value,
) -> Result<(), Vec<String>> {
    let words = payload_word_map(payload)?;
    let suggestions = answer["suggestions"]
        .as_array()
        .ok_or_else(|| vec!["missing `suggestions` array".into()])?;
    if suggestions.len() > 8 {
        return Err(vec!["more than 8 suggestions".into()]);
    }
    let media_end = payload["mediaDuration"].as_f64().unwrap_or_default();
    let mut occupied = Vec::new();
    for (index, suggestion) in suggestions.iter().enumerate() {
        let start_id = suggestion["start"]
            .as_str()
            .ok_or_else(|| vec![format!("suggestions[{index}] missing start")])?;
        let end_id = suggestion["end"]
            .as_str()
            .ok_or_else(|| vec![format!("suggestions[{index}] missing end")])?;
        let Some(&start_word) = words.get(start_id) else {
            return Err(vec![format!("suggestions[{index}] unknown start")]);
        };
        let Some(&end_word) = words.get(end_id) else {
            return Err(vec![format!("suggestions[{index}] unknown end")]);
        };
        if start_word.index > end_word.index {
            return Err(vec![format!("suggestions[{index}] reversed span")]);
        }
        if !matches!(suggestion["mode"].as_str(), Some("fullscreen" | "pip")) {
            return Err(vec![format!("suggestions[{index}] invalid mode")]);
        }
        for field in ["query", "reason"] {
            if suggestion[field]
                .as_str()
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err(vec![format!("suggestions[{index}] missing {field}")]);
            }
        }
        if start_word.start < 3.0 {
            return Err(vec![format!("suggestions[{index}] start inside first 3s")]);
        }
        if media_end > 0.0 && end_word.end > media_end - 3.0 {
            return Err(vec![format!("suggestions[{index}] end inside last 3s")]);
        }
        if !(1.5..=20.0).contains(&(end_word.end - start_word.start)) {
            return Err(vec![format!("suggestions[{index}] span outside 1.5-20s")]);
        }
        if occupied.iter().any(|(other_start, other_end)| {
            start_word.start < *other_end && *other_start < end_word.end
        }) {
            return Err(vec![format!("suggestions[{index}] overlap")]);
        }
        occupied.push((start_word.start, end_word.end));
    }
    Ok(())
}

fn validate_translate(
    payload: &serde_json::Value,
    answer: &serde_json::Value,
) -> Result<(), Vec<String>> {
    let expected: BTreeSet<&str> = payload["lines"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|line| line["id"].as_str())
        .collect();
    let translations = answer["translations"]
        .as_object()
        .ok_or_else(|| vec!["missing `translations` object".into()])?;
    let actual: BTreeSet<&str> = translations.keys().map(String::as_str).collect();
    if expected != actual {
        return Err(vec![format!(
            "line coverage mismatch: expected {expected:?}, got {actual:?}"
        )]);
    }
    for field in ["summary", "terms", "namedEntities"] {
        if answer.get(field).is_none() {
            return Err(vec![format!("missing brief section `{field}`")]);
        }
    }
    if translations
        .values()
        .any(|value| value.as_str().is_none_or(|text| text.trim().is_empty()))
    {
        return Err(vec!["translation values must be non-empty strings".into()]);
    }
    for line in payload["lines"].as_array().into_iter().flatten() {
        let Some(id) = line["id"].as_str() else {
            continue;
        };
        let target = translations
            .get(id)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        for required in line["rt"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
        {
            if !target.contains(required) {
                return Err(vec![format!(
                    "rt verbatim: translation `{id}` is missing locked term `{required}`"
                )]);
            }
        }
    }
    Ok(())
}

fn validate_polish(
    payload: &serde_json::Value,
    answer: &serde_json::Value,
) -> Result<(), Vec<String>> {
    for field in ["summary", "terms", "namedEntities"] {
        if answer.get(field).is_none() {
            return Err(vec![format!("missing analysis section `{field}`")]);
        }
    }
    let source_paragraphs = payload["paragraphs"]
        .as_array()
        .ok_or_else(|| vec!["payload missing paragraphs".into()])?;
    let answer_paragraphs = answer["paragraphs"]
        .as_array()
        .ok_or_else(|| vec!["missing `paragraphs` array".into()])?;
    if source_paragraphs.len() != answer_paragraphs.len() {
        return Err(vec!["paragraph count mismatch".into()]);
    }
    for (source_paragraph, answer_paragraph) in source_paragraphs.iter().zip(answer_paragraphs) {
        let sources = source_paragraph["sentences"]
            .as_array()
            .ok_or_else(|| vec!["payload sentence shape".into()])?;
        let answers = answer_paragraph["sentences"]
            .as_array()
            .ok_or_else(|| vec!["answer sentence shape".into()])?;
        if sources.len() != answers.len() {
            return Err(vec!["sentence count mismatch".into()]);
        }
        for (source, polished) in sources.iter().zip(answers) {
            let before = source["text"].as_str().unwrap_or_default();
            let after = polished
                .as_str()
                .ok_or_else(|| vec!["polished sentence must be a string".into()])?;
            let estimate = crate::pipeline::polish::estimate_polish(before, after);
            if estimate.quality == crate::pipeline::polish::PolishQuality::Fail {
                return Err(estimate.issues);
            }
        }
    }
    Ok(())
}

fn replace_trailing_punctuation(text: &str, mark: &str) -> String {
    let stripped = text.trim_end_matches(|character| {
        matches!(
            character,
            '，' | '。'
                | '、'
                | '？'
                | '！'
                | '；'
                | '：'
                | '…'
                | ','
                | '.'
                | '?'
                | '!'
                | ';'
                | ':'
        )
    });
    format!("{stripped}{mark}")
}

pub async fn wait_and_apply(
    allocator: Arc<Allocator>,
    task: PreparedTask,
    timeout: Duration,
) -> AppResult<usize> {
    wait_and_apply_inner(allocator, task, timeout, None, None).await
}

pub async fn wait_and_apply_with_lock(
    allocator: Arc<Allocator>,
    task: PreparedTask,
    timeout: Duration,
    project_mutation: Arc<tokio::sync::Mutex<()>>,
) -> AppResult<usize> {
    wait_and_apply_inner(allocator, task, timeout, Some(project_mutation), None).await
}

pub async fn wait_and_apply_with_lock_and_pause(
    allocator: Arc<Allocator>,
    task: PreparedTask,
    timeout: Duration,
    project_mutation: Arc<tokio::sync::Mutex<()>>,
    pause: Arc<AtomicBool>,
) -> AppResult<usize> {
    wait_and_apply_inner(
        allocator,
        task,
        timeout,
        Some(project_mutation),
        Some(pause),
    )
    .await
}

async fn wait_and_apply_inner(
    allocator: Arc<Allocator>,
    task: PreparedTask,
    timeout: Duration,
    project_mutation: Option<Arc<tokio::sync::Mutex<()>>>,
    pause: Option<Arc<AtomicBool>>,
) -> AppResult<usize> {
    if task.kind == "translate" {
        return wait_and_apply_translation(allocator, task, timeout, project_mutation, pause).await;
    }
    let started = Instant::now();
    loop {
        if pause
            .as_ref()
            .is_some_and(|requested| requested.load(Ordering::Relaxed))
        {
            return Err(AppError::Schema(format!(
                "task {} paused by user; durable requests and accepted results were preserved",
                task.kind
            )));
        }
        if task
            .calls
            .iter()
            .all(|prepared| allocator.completed(&prepared.call.id).is_some())
        {
            break;
        }
        if started.elapsed() >= timeout {
            return Err(AppError::Schema(format!(
                "task {} paused after waiting for incomplete calls; durable requests and accepted results were preserved",
                task.kind
            )));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let mut submissions = Vec::with_capacity(task.calls.len());
    let mut task_errors = Vec::new();
    for prepared in &task.calls {
        let submission = allocator
            .completed(&prepared.call.id)
            .ok_or_else(|| AppError::Schema("completed call disappeared".into()))?;
        if let Some(error) = &submission.error {
            crate::data::storage::write_json(
                &prepared.failed_path,
                &serde_json::json!({"error": error}),
            )?;
            if prepared.pending_path.exists() {
                std::fs::remove_file(&prepared.pending_path)?;
            }
            if prepared.submitted_path.exists() {
                std::fs::remove_file(&prepared.submitted_path)?;
            }
            task_errors.push(format!(
                "task call {} failed; see {}",
                prepared.call.id,
                prepared.failed_path.display()
            ));
            continue;
        }
        let answer = submission
            .answer
            .as_ref()
            .ok_or_else(|| AppError::Schema("submission had neither answer nor error".into()))?;
        validate_call_answer(&prepared.call, &answer.text)
            .map_err(|errors| AppError::Schema(format!("invalid completed answer: {errors:?}")))?;
        submissions.push(submission);
    }
    if !task_errors.is_empty() {
        let aborted = format!("task {} stopped because another batch failed", task.kind);
        for prepared in &task.calls {
            if prepared.pending_path.exists() {
                crate::data::storage::write_json(
                    &prepared.failed_path,
                    &serde_json::json!({"error": &aborted}),
                )?;
                std::fs::remove_file(&prepared.pending_path)?;
                if prepared.submitted_path.exists() {
                    std::fs::remove_file(&prepared.submitted_path)?;
                }
            }
        }
        return Err(AppError::Schema(task_errors.join("; ")));
    }

    // Model execution can take minutes. Lock only the final local mutation so
    // the editor stays usable while work is in flight, but background results
    // cannot write doc/cuts/artifacts concurrently with an editor command.
    let _mutation = if let Some(mutation) = project_mutation {
        Some(mutation.lock_owned().await)
    } else {
        None
    };
    let zero_duration_words_before = (task.kind == "polish")
        .then(|| {
            Doc::load(&task.project_dir).map(|doc| {
                doc.all_words()
                    .into_iter()
                    .filter(|word| word.end - word.start < crate::pipeline::timing::MIN_DUR)
                    .count()
            })
        })
        .transpose()?
        .unwrap_or_default();
    let mut applied = 0usize;
    let mut evidence = Vec::with_capacity(task.calls.len());
    for (prepared, submission) in task.calls.iter().zip(submissions) {
        let answer = submission
            .answer
            .ok_or_else(|| AppError::Schema("validated answer disappeared".into()))?;
        let request: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&prepared.pending_path)?)?;
        let parsed_answer = parse_answer_value(&task.kind, &answer.text)
            .map_err(|errors| AppError::Schema(format!("invalid answer: {errors:?}")))?;
        evidence.push((request.clone(), parsed_answer));
        applied += apply_answer(&task, &prepared.call, &answer.text)?;
        crate::data::storage::write_json(
            &prepared.done_path,
            &serde_json::json!({
                "callId": prepared.call.id,
                "request": request,
                "answer": answer,
            }),
        )?;
        if prepared.pending_path.exists() {
            std::fs::remove_file(&prepared.pending_path)?;
        }
        if prepared.submitted_path.exists() {
            std::fs::remove_file(&prepared.submitted_path)?;
        }
    }
    persist_task_artifacts(&task, &evidence, zero_duration_words_before)?;
    Ok(applied)
}

/// Translation batches are independent and safe to persist incrementally.
/// Moving each accepted batch from pending → done immediately gives the UI a
/// truthful progress signal and preserves useful work if a later batch fails
/// or the app exits.
async fn wait_and_apply_translation(
    allocator: Arc<Allocator>,
    task: PreparedTask,
    timeout: Duration,
    project_mutation: Option<Arc<tokio::sync::Mutex<()>>>,
    pause: Option<Arc<AtomicBool>>,
) -> AppResult<usize> {
    let started = Instant::now();
    let mut remaining: BTreeSet<String> = task
        .calls
        .iter()
        .map(|prepared| prepared.call.id.clone())
        .collect();
    let mut applied = 0usize;
    let mut errors = Vec::new();

    while !remaining.is_empty() {
        if pause
            .as_ref()
            .is_some_and(|requested| requested.load(Ordering::Relaxed))
        {
            return Err(AppError::Schema(format!(
                "task {} paused by user; completed batches were saved",
                task.kind
            )));
        }
        let mut progressed = false;
        for prepared in &task.calls {
            if !remaining.contains(&prepared.call.id) {
                continue;
            }
            let Some(submission) = allocator.completed(&prepared.call.id) else {
                continue;
            };
            remaining.remove(&prepared.call.id);
            progressed = true;

            if let Some(error) = submission.error {
                record_translation_failure(prepared, &error)?;
                errors.push(format!("task call {} failed: {error}", prepared.call.id));
                continue;
            }

            let Some(answer) = submission.answer else {
                let error = "submission had neither answer nor error";
                record_translation_failure(prepared, error)?;
                errors.push(format!("task call {} failed: {error}", prepared.call.id));
                continue;
            };
            if let Err(issues) = validate_call_answer(&prepared.call, &answer.text) {
                let error = format!("invalid completed answer: {issues:?}");
                record_translation_failure(prepared, &error)?;
                errors.push(format!("task call {} failed: {error}", prepared.call.id));
                continue;
            }
            let parsed_answer = match parse_answer_value(&task.kind, &answer.text) {
                Ok(answer) => answer,
                Err(issues) => {
                    let error = format!("invalid answer: {issues:?}");
                    record_translation_failure(prepared, &error)?;
                    errors.push(format!("task call {} failed: {error}", prepared.call.id));
                    continue;
                }
            };

            let mutation_guard = if let Some(mutation) = project_mutation.clone() {
                Some(mutation.lock_owned().await)
            } else {
                None
            };
            let persist_task = task.clone();
            let persist_call = prepared.clone();
            let batch_applied = tokio::task::spawn_blocking(move || {
                let _mutation_guard = mutation_guard;
                let request: serde_json::Value =
                    serde_json::from_str(&std::fs::read_to_string(&persist_call.pending_path)?)?;
                let batch_applied = apply_answer(&persist_task, &persist_call.call, &answer.text)?;
                let completed_evidence = completed_translation_evidence(
                    &persist_task,
                    &persist_call,
                    &request,
                    &parsed_answer,
                )?;
                persist_task_artifacts(&persist_task, &completed_evidence, 0)?;
                crate::data::storage::write_json(
                    &persist_call.done_path,
                    &serde_json::json!({
                        "callId": persist_call.call.id,
                        "request": request,
                        "answer": answer,
                    }),
                )?;
                remove_if_exists(&persist_call.pending_path)?;
                remove_if_exists(&persist_call.submitted_path)?;
                Ok::<usize, AppError>(batch_applied)
            })
            .await
            .map_err(|error| {
                AppError::Schema(format!("translation persistence failed: {error}"))
            })??;
            applied += batch_applied;
        }

        if remaining.is_empty() {
            break;
        }
        if started.elapsed() >= timeout {
            return Err(AppError::Schema(format!(
                "task {} paused after waiting for incomplete calls; completed batches were saved",
                task.kind
            )));
        }
        if !progressed {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    if errors.is_empty() {
        Ok(applied)
    } else {
        Err(AppError::Schema(errors.join("; ")))
    }
}

/// Rebuild the active run's analysis evidence in call order, regardless of
/// which parallel provider request completed first. The current batch is
/// included before its done marker is published so `done` always means both
/// the document and its analysis metadata are durable.
fn completed_translation_evidence(
    task: &PreparedTask,
    current: &PreparedCall,
    current_request: &serde_json::Value,
    current_answer: &serde_json::Value,
) -> AppResult<Vec<(serde_json::Value, serde_json::Value)>> {
    let task_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(task.ai_dir.join("task.json"))?)?;
    let run_id = task_json["runId"]
        .as_str()
        .ok_or_else(|| AppError::Schema("translation task is missing its run id".into()))?;
    let prefix = format!("{}-{run_id}-", task.kind);
    let mut ordered = BTreeMap::new();
    let done_dir = task.ai_dir.join("done");
    if let Ok(entries) = std::fs::read_dir(done_dir) {
        for entry in entries.filter_map(Result::ok) {
            let Some(call_id) = entry
                .path()
                .file_stem()
                .and_then(|value| value.to_str())
                .filter(|value| value.starts_with(&prefix))
                .map(str::to_string)
            else {
                continue;
            };
            let artifact: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(entry.path())?)?;
            let request = artifact["request"].clone();
            let answer_text = artifact["answer"]["text"]
                .as_str()
                .ok_or_else(|| AppError::Schema(format!("done call {call_id} has no answer")))?;
            let answer = parse_answer_value("translate", answer_text)
                .map_err(|issues| AppError::Schema(format!("invalid done answer: {issues:?}")))?;
            ordered.insert(call_id, (request, answer));
        }
    }
    ordered.insert(
        current.call.id.clone(),
        (current_request.clone(), current_answer.clone()),
    );
    Ok(ordered.into_values().collect())
}

fn record_translation_failure(prepared: &PreparedCall, error: &str) -> AppResult<()> {
    crate::data::storage::write_json(&prepared.failed_path, &serde_json::json!({"error": error}))?;
    remove_if_exists(&prepared.pending_path)?;
    remove_if_exists(&prepared.submitted_path)
}

fn remove_if_exists(path: &Path) -> AppResult<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Io(error)),
    }
}

fn persist_task_artifacts(
    task: &PreparedTask,
    evidence: &[(serde_json::Value, serde_json::Value)],
    zero_duration_words_before: usize,
) -> AppResult<()> {
    if !matches!(task.kind.as_str(), "polish" | "translate") {
        return Ok(());
    }

    let analysis_path = task.project_dir.join("ai").join("analysis.json");
    let previous_analysis = std::fs::read_to_string(&analysis_path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok());
    let analysis = merged_analysis(evidence, previous_analysis.as_ref());
    write_json_atomically(&analysis_path, &analysis)?;

    if task.kind != "polish" {
        return Ok(());
    }

    let doc = Doc::load(&task.project_dir)?;
    let residual_term_variants = residual_variants(&doc, &analysis);
    let zero_duration_word_count_after = doc
        .all_words()
        .into_iter()
        .filter(|word| word.end - word.start < crate::pipeline::timing::MIN_DUR)
        .count();
    let mut status = crate::pipeline::polish::PolishQuality::Pass;
    for (request, answer) in evidence {
        let sources = answer_sentences(request, true);
        let polished = answer_sentences(answer, false);
        for (source, target) in sources.into_iter().zip(polished) {
            match crate::pipeline::polish::estimate_polish(source, target).quality {
                crate::pipeline::polish::PolishQuality::Fail => {
                    status = crate::pipeline::polish::PolishQuality::Fail;
                }
                crate::pipeline::polish::PolishQuality::Warn
                    if status != crate::pipeline::polish::PolishQuality::Fail =>
                {
                    status = crate::pipeline::polish::PolishQuality::Warn;
                }
                _ => {}
            }
        }
    }
    if (!residual_term_variants.is_empty()
        || zero_duration_word_count_after > zero_duration_words_before)
        && status == crate::pipeline::polish::PolishQuality::Pass
    {
        status = crate::pipeline::polish::PolishQuality::Warn;
    }
    let artifact = crate::pipeline::polish::PolishQualityArtifact {
        fingerprint: crate::pipeline::fingerprint_words(&doc),
        created_at: chrono::Utc::now(),
        status: if status == crate::pipeline::polish::PolishQuality::Pass {
            crate::pipeline::polish::PolishQualityStatus::Pass
        } else {
            crate::pipeline::polish::PolishQualityStatus::Warn
        },
        page_count: evidence.len(),
        measured_page_count: evidence.len(),
        retry_count: 0,
        recovered_page_count: 0,
        fallback_page_count: 0,
        fallback_sentence_count: 0,
        residual_term_variant_count: residual_term_variants.len(),
        residual_term_variants,
        zero_duration_word_count_before: zero_duration_words_before,
        zero_duration_word_count_after,
    };
    artifact.save(&task.project_dir.join("ai").join("polish-quality.json"))
}

fn answer_sentences(value: &serde_json::Value, source_shape: bool) -> Vec<&str> {
    value["paragraphs"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|paragraph| paragraph["sentences"].as_array().into_iter().flatten())
        .filter_map(|sentence| {
            if source_shape {
                sentence["text"].as_str()
            } else {
                sentence.as_str()
            }
        })
        .collect()
}

fn merged_analysis(
    evidence: &[(serde_json::Value, serde_json::Value)],
    previous: Option<&serde_json::Value>,
) -> serde_json::Value {
    let mut summaries = Vec::new();
    let mut terms: std::collections::BTreeMap<String, serde_json::Value> =
        std::collections::BTreeMap::new();
    let mut named_entities = BTreeSet::new();
    if let Some(previous) = previous {
        for term in previous["terms"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|term| term["locked"].as_bool() == Some(true))
        {
            if let Some(name) = term["term"].as_str() {
                terms.insert(name.to_string(), term.clone());
            }
        }
    }
    for (_, answer) in evidence {
        if let Some(summary) = answer["summary"]
            .as_str()
            .filter(|text| !text.trim().is_empty())
        {
            if !summaries.iter().any(|current| current == summary) {
                summaries.push(summary.to_string());
            }
        }
        for term in answer["terms"].as_array().into_iter().flatten() {
            let Some(name) = term["term"].as_str().filter(|text| !text.trim().is_empty()) else {
                continue;
            };
            match terms.entry(name.to_string()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(term.clone());
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let current = entry.get_mut();
                    let mut variants: BTreeSet<String> = current["observedVariants"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|value| value.as_str().map(str::to_string))
                        .collect();
                    variants.extend(
                        term["observedVariants"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|value| value.as_str().map(str::to_string)),
                    );
                    current["observedVariants"] =
                        serde_json::json!(variants.into_iter().collect::<Vec<_>>());
                    if term["locked"].as_bool() == Some(true) {
                        current["locked"] = serde_json::Value::Bool(true);
                    }
                    if current.get("note").is_none() && term.get("note").is_some() {
                        current["note"] = term["note"].clone();
                    }
                }
            }
        }
        named_entities.extend(
            answer["namedEntities"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|value| value.as_str().map(str::to_string)),
        );
    }
    serde_json::json!({
        "summary": summaries.join("\n\n"),
        "terms": terms.into_values().collect::<Vec<_>>(),
        "namedEntities": named_entities.into_iter().collect::<Vec<_>>(),
    })
}

fn residual_variants(
    doc: &Doc,
    analysis: &serde_json::Value,
) -> Vec<crate::pipeline::polish::ResidualVariant> {
    let text = doc
        .paragraphs
        .iter()
        .flat_map(|paragraph| paragraph.sentences.iter())
        .map(|sentence| sentence.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut residuals = Vec::new();
    for term in analysis["terms"].as_array().into_iter().flatten() {
        let Some(canonical) = term["term"].as_str() else {
            continue;
        };
        for variant in term["observedVariants"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
        {
            let occurrences = text.matches(variant).count();
            if occurrences > 0 {
                residuals.push(crate::pipeline::polish::ResidualVariant {
                    canonical: canonical.to_string(),
                    variant: variant.to_string(),
                    occurrences,
                });
            }
        }
    }
    residuals.sort_by(|left, right| {
        left.canonical
            .cmp(&right.canonical)
            .then_with(|| left.variant.cmp(&right.variant))
    });
    residuals
}

fn write_json_atomically(path: &Path, value: &serde_json::Value) -> AppResult<()> {
    crate::data::storage::write_json(path, value)
}

fn apply_answer(task: &PreparedTask, call: &PendingCall, answer: &str) -> AppResult<usize> {
    let payload: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&call.payload_ref)?)?;
    let answer = parse_answer_value(&task.kind, answer)
        .map_err(|errors| AppError::Schema(format!("invalid answer: {errors:?}")))?;
    let mut doc = Doc::load(&task.project_dir)?;
    let applied = match task.kind.as_str() {
        "translate" => {
            let lang = task
                .lang
                .as_deref()
                .ok_or_else(|| AppError::Schema("translate language disappeared".into()))?;
            let translations = answer["translations"]
                .as_object()
                .ok_or_else(|| AppError::Schema("missing translations".into()))?;
            let target = doc.translations.entry(lang.to_string()).or_default();
            for (id, value) in translations {
                let text = value
                    .as_str()
                    .ok_or_else(|| AppError::Schema("translation must be string".into()))?;
                let source_words = doc
                    .paragraphs
                    .iter()
                    .flat_map(|paragraph| paragraph.sentences.iter())
                    .find(|sentence| sentence.id == *id)
                    .map(|sentence| sentence.words.iter().map(|word| word.id.clone()).collect())
                    .unwrap_or_default();
                target.insert(
                    id.clone(),
                    TranslationGroup {
                        id: id.clone(),
                        text: text.to_string(),
                        source_words,
                        source_text: doc
                            .paragraphs
                            .iter()
                            .flat_map(|paragraph| paragraph.sentences.iter())
                            .find(|sentence| sentence.id == *id)
                            .map(|sentence| sentence.text.clone()),
                    },
                );
            }
            translations.len()
        }
        "align" => {
            let lang = task
                .lang
                .as_deref()
                .ok_or_else(|| AppError::Schema("align language disappeared".into()))?;
            let changes = answer["pairs"]
                .as_array()
                .ok_or_else(|| AppError::Schema("missing align pairs".into()))?;
            let target = doc
                .translations
                .get_mut(lang)
                .ok_or_else(|| AppError::Schema(format!("no `{lang}` translations to align")))?;
            for change in changes {
                if change["action"].as_str() != Some("rewrite") {
                    continue;
                }
                let id = change["id"]
                    .as_str()
                    .ok_or_else(|| AppError::Schema("align change missing id".into()))?;
                let group = target
                    .get_mut(id)
                    .ok_or_else(|| AppError::Schema(format!("unknown align group `{id}`")))?;
                group.text = change["pieces"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|piece| piece["t"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
            }
            let changed = changes.len();
            crate::pipeline::TranslateRebindArtifact::from_doc(&doc, lang)
                .save(&task.project_dir.join("ai").join("align-artifact.json"))?;
            changed
        }
        "polish" => {
            let source_sentences: Vec<&serde_json::Value> = payload["paragraphs"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|paragraph| paragraph["sentences"].as_array().into_iter().flatten())
                .collect();
            let polished_sentences: Vec<&serde_json::Value> = answer["paragraphs"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|paragraph| paragraph["sentences"].as_array().into_iter().flatten())
                .collect();
            for (source, polished) in source_sentences.iter().zip(polished_sentences) {
                let id = source["id"].as_str().unwrap_or_default();
                let after = polished.as_str().unwrap_or_default();
                let _ = crate::pipeline::polish::apply_polish(&mut doc, id, after)?;
            }
            source_sentences.len()
        }
        "segment" => {
            let ranges = segment_ranges(&payload, &answer).map_err(|errors| {
                AppError::Schema(format!("invalid segment answer: {errors:?}"))
            })?;
            let speakers: std::collections::BTreeMap<String, Option<String>> = doc
                .paragraphs
                .iter()
                .flat_map(|paragraph| {
                    paragraph
                        .sentences
                        .iter()
                        .map(move |sentence| (sentence.id.clone(), paragraph.speaker.clone()))
                })
                .collect();
            let sentences: Vec<crate::data::Sentence> = doc
                .paragraphs
                .iter()
                .flat_map(|paragraph| paragraph.sentences.iter().cloned())
                .collect();
            doc.paragraphs = ranges
                .iter()
                .enumerate()
                .map(|(index, (start, end))| {
                    let group = sentences[*start..*end].to_vec();
                    let first_speaker = group
                        .first()
                        .and_then(|sentence| speakers.get(&sentence.id))
                        .cloned()
                        .flatten();
                    let same_speaker = group.iter().all(|sentence| {
                        speakers.get(&sentence.id).cloned().flatten() == first_speaker
                    });
                    crate::data::Paragraph {
                        id: index as u32 + 1,
                        speaker: same_speaker.then_some(first_speaker).flatten(),
                        sentences: group,
                    }
                })
                .collect();
            ranges.len()
        }
        "repunct" => {
            let candidates: std::collections::BTreeMap<String, String> = payload["segs"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|segment| segment["cm"].as_array().into_iter().flatten())
                .filter_map(|candidate| {
                    Some((
                        candidate["id"].as_str()?.to_string(),
                        candidate["wordId"].as_str()?.to_string(),
                    ))
                })
                .collect();
            let cuts: Vec<&serde_json::Value> = answer["segs"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|segment| segment["cuts"].as_array().into_iter().flatten())
                .collect();
            for cut in &cuts {
                let candidate = cut["id"]
                    .as_str()
                    .ok_or_else(|| AppError::Schema("repunct cut missing id".into()))?;
                let word_id = candidates.get(candidate).ok_or_else(|| {
                    AppError::Schema(format!("unknown repunct seam `{candidate}`"))
                })?;
                let mark = cut["m"]
                    .as_str()
                    .ok_or_else(|| AppError::Schema("repunct cut missing mark".into()))?;
                for paragraph in &mut doc.paragraphs {
                    for sentence in &mut paragraph.sentences {
                        let Some(index) =
                            sentence.words.iter().position(|word| &word.id == word_id)
                        else {
                            continue;
                        };
                        let before = sentence.words[index].text.clone();
                        let after = replace_trailing_punctuation(&before, mark);
                        sentence.words[index].text = after.clone();
                        sentence.text = sentence.text.replacen(&before, &after, 1);
                    }
                }
            }
            cuts.len()
        }
        "chapters" => {
            let chapters = answer["chapters"]
                .as_array()
                .ok_or_else(|| AppError::Schema("missing chapters".into()))?;
            crate::data::storage::write_json(&task.project_dir.join("chapters.json"), chapters)?;
            let doc_path = task.project_dir.join("doc.json");
            let mut native: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&doc_path)?)?;
            native
                .as_object_mut()
                .ok_or_else(|| AppError::Schema("doc.json must be an object".into()))?
                .insert(
                    "chapters".into(),
                    serde_json::Value::Array(chapters.clone()),
                );
            crate::data::storage::write_json(&doc_path, &native)?;
            chapters.len()
        }
        "cleanup" => {
            let mut cuts: ClipCuts = std::fs::read_to_string(task.project_dir.join("cuts.json"))
                .ok()
                .and_then(|raw| serde_json::from_str(&raw).ok())
                .unwrap_or_default();
            let word_at: std::collections::BTreeMap<&str, (f64, f64)> = doc
                .all_words()
                .into_iter()
                .map(|word| (word.id.as_str(), (word.start, word.end)))
                .collect();
            let word_sentence: std::collections::BTreeMap<&str, &str> = doc
                .paragraphs
                .iter()
                .flat_map(|paragraph| paragraph.sentences.iter())
                .flat_map(|sentence| {
                    sentence
                        .words
                        .iter()
                        .map(move |word| (word.id.as_str(), sentence.id.as_str()))
                })
                .collect();
            let answer_cuts = answer["cuts"]
                .as_array()
                .ok_or_else(|| AppError::Schema("missing cuts".into()))?;
            let before = cuts.cuts.len();
            for item in answer_cuts {
                let a = item["a"].as_str().unwrap_or_default();
                let b = item["b"].as_str().unwrap_or_default();
                let cat = item["cat"].as_str().unwrap_or_default();
                if cuts
                    .cuts
                    .iter()
                    .any(|cut| cut.a_word == a && cut.b_word == b)
                {
                    continue;
                }
                let &(a_start, a_end) = word_at
                    .get(a)
                    .ok_or_else(|| AppError::Schema(format!("unknown cut word {a}")))?;
                let &(b_start, b_end) = word_at
                    .get(b)
                    .ok_or_else(|| AppError::Schema(format!("unknown cut word {b}")))?;
                let kind = match cat {
                    "retake" => CutKind::Retake,
                    "filler" => CutKind::Filler,
                    "falseStart" => CutKind::FalseStart,
                    "silence" => CutKind::Silence,
                    _ => return Err(AppError::Schema(format!("invalid cut cat {cat}"))),
                };
                cuts.add(Cut {
                    id: format!("c-agent-{}", uuid::Uuid::new_v4()),
                    note: item["reason"].as_str().map(str::to_string),
                    a_word: a.into(),
                    b_word: b.into(),
                    kind,
                    duration: match kind {
                        CutKind::Silence => crate::pipeline::cleanup::compressed_silence_duration(
                            (b_start - a_end).max(0.0),
                            word_sentence.get(a) != word_sentence.get(b),
                        ),
                        _ => (b_end - a_start).max(0.0),
                    },
                });
            }
            crate::data::storage::write_json(&task.project_dir.join("cuts.json"), &cuts)?;
            crate::data::storage::write_json(
                &task.project_dir.join("ai").join("cuts.json"),
                &answer,
            )?;
            crate::data::activity::touch(&task.project_dir)?;
            return Ok(cuts.cuts.len() - before);
        }
        "broll" => {
            crate::data::storage::write_json(
                &task.project_dir.join("ai").join("broll-suggestions.json"),
                &answer,
            )?;
            crate::data::activity::touch(&task.project_dir)?;
            return Ok(answer["suggestions"]
                .as_array()
                .map(Vec::len)
                .unwrap_or_default());
        }
        other => {
            return Err(AppError::Schema(format!(
                "apply not implemented for `{other}`"
            )))
        }
    };
    doc.meta.updated_at = chrono::Utc::now();
    doc.save(&task.project_dir)?;
    crate::data::activity::touch(&task.project_dir)?;
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::bridge::BridgeAnswer;
    use crate::data::{MediaRef, Meta, Paragraph, Sentence, Word};
    use std::collections::BTreeMap;

    fn sample_doc() -> Doc {
        Doc {
            id: "demo".into(),
            schema: 1,
            media: MediaRef {
                path: "/tmp/media.mp4".into(),
                duration_seconds: 2.0,
                sample_rate: Some(16_000),
                channels: Some(1),
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
                    text: "hello world".into(),
                    words: vec![Word {
                        id: "w0".into(),
                        text: "hello".into(),
                        start: 0.0,
                        end: 1.0,
                    }],
                }],
            }],
            translations: BTreeMap::new(),
        }
    }

    fn doc_with_sentences(count: usize) -> Doc {
        let mut doc = sample_doc();
        doc.paragraphs[0].sentences = (0..count)
            .map(|index| Sentence {
                id: format!("s{index}"),
                text: format!("Short subtitle line {index}"),
                words: vec![Word {
                    id: format!("w{index}"),
                    text: "subtitle".into(),
                    start: index as f64,
                    end: index as f64 + 0.8,
                }],
            })
            .collect();
        doc
    }

    fn answer(text: &str) -> BridgeAnswer {
        BridgeAnswer {
            text: text.into(),
            reasoning: String::new(),
            prompt_tokens: 0,
            completion_tokens: 0,
        }
    }

    fn translation_answer_for(call: &PreparedCall) -> String {
        let payload: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&call.pending_path).unwrap()).unwrap();
        let translations = payload["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| {
                let id = line["id"].as_str().unwrap();
                (
                    id.to_string(),
                    serde_json::Value::String(format!("译文 {id}")),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        serde_json::json!({
            "summary": "context",
            "terms": [],
            "namedEntities": [],
            "translations": translations,
        })
        .to_string()
    }

    #[tokio::test]
    async fn accepted_model_result_survives_allocator_reconstruction() {
        let tmp = tempfile::tempdir().unwrap();
        sample_doc().save(tmp.path()).unwrap();
        let task = prepare_task(tmp.path(), "translate", Some("zh")).unwrap();
        let first_allocator = Arc::new(Allocator::new(1));
        assert_eq!(restore_or_enqueue(&first_allocator, &task).unwrap(), 0);
        let (_, lease) = first_allocator.allocate().unwrap();
        let raw =
            r#"{"summary":"x","terms":[],"namedEntities":[],"translations":{"s1":"你好世界"}}"#;
        first_allocator
            .submit(&lease.lease_id, Some(answer(raw)), None)
            .unwrap();
        assert!(task.calls[0].submitted_path.exists());

        let recovered_task = load_recoverable_task(tmp.path(), "translate")
            .unwrap()
            .unwrap();
        let recovered_allocator = Arc::new(Allocator::new(1));
        assert_eq!(
            restore_or_enqueue(&recovered_allocator, &recovered_task).unwrap(),
            1
        );
        assert_eq!(recovered_allocator.pending_count(), 0);
        assert!(recovered_allocator
            .completed(&recovered_task.calls[0].call.id)
            .is_some());

        wait_and_apply(recovered_allocator, recovered_task, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(
            Doc::load(tmp.path()).unwrap().translations["zh"]["s1"].text,
            "你好世界"
        );
    }

    #[test]
    fn interrupted_task_preparation_is_regenerated_from_its_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        sample_doc().save(tmp.path()).unwrap();
        let ai_dir = tmp.path().join("ai/translate");
        std::fs::create_dir_all(ai_dir.join("pending")).unwrap();
        crate::data::storage::write_json(
            &ai_dir.join("task.json"),
            &serde_json::json!({
                "kind": "translate",
                "lang": "zh",
                "state": "preparing",
                "runId": "interrupted",
                "staleOnly": false,
                "groups": [],
                "alignFit": null,
                "calls": 1,
            }),
        )
        .unwrap();
        let partial = ai_dir.join("pending/translate-interrupted-0000.json");
        crate::data::storage::write(&partial, b"{").unwrap();

        let recovered = load_recoverable_task(tmp.path(), "translate")
            .unwrap()
            .unwrap();
        assert_eq!(recovered.calls.len(), 1);
        assert!(!partial.exists());
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(ai_dir.join("task.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["state"], "running");
        assert_ne!(manifest["runId"], "interrupted");
    }

    #[tokio::test]
    async fn wait_timeout_preserves_durable_work_for_resume() {
        let tmp = tempfile::tempdir().unwrap();
        sample_doc().save(tmp.path()).unwrap();
        let task = prepare_task(tmp.path(), "translate", Some("zh")).unwrap();
        let pending = task.calls[0].pending_path.clone();
        let failed = task.calls[0].failed_path.clone();

        assert!(
            wait_and_apply(Arc::new(Allocator::new(1)), task, Duration::ZERO,)
                .await
                .is_err()
        );
        assert!(pending.exists());
        assert!(!failed.exists());

        let recovered = load_recoverable_task(tmp.path(), "translate")
            .unwrap()
            .unwrap();
        let allocator = Allocator::new(1);
        restore_or_enqueue(&allocator, &recovered).unwrap();
        assert_eq!(allocator.pending_count(), 1);
    }

    #[test]
    fn automatic_recovery_skips_paused_tasks_but_explicit_resume_can_load_them() {
        let tmp = tempfile::tempdir().unwrap();
        sample_doc().save(tmp.path()).unwrap();
        let task = prepare_task(tmp.path(), "translate", Some("zh")).unwrap();
        set_task_state(&task, "paused", Some("provider timeout")).unwrap();

        assert!(load_recoverable_tasks(tmp.path()).unwrap().is_empty());
        assert!(load_recoverable_task(tmp.path(), "translate")
            .unwrap()
            .is_some());
    }

    #[test]
    fn a_new_language_never_silently_recovers_an_unfinished_translation() {
        let tmp = tempfile::tempdir().unwrap();
        sample_doc().save(tmp.path()).unwrap();
        let task = prepare_task(tmp.path(), "translate", Some("zh")).unwrap();
        set_task_state(&task, "paused", Some("provider timeout")).unwrap();

        let matching = load_matching_recoverable_task(tmp.path(), "translate", Some("zh")).unwrap();
        assert_eq!(matching.unwrap().lang.as_deref(), Some("zh"));

        let error = load_matching_recoverable_task(tmp.path(), "translate", Some("ja"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("language `zh`"));
        assert!(error.contains("language `ja`"));
    }

    #[tokio::test]
    async fn translate_task_validates_and_applies_completed_answer() {
        let tmp = tempfile::tempdir().unwrap();
        sample_doc().save(tmp.path()).unwrap();
        let task = prepare_task(tmp.path(), "translate", Some("zh")).unwrap();
        assert_eq!(task.calls.len(), 1);
        let payload: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&task.calls[0].pending_path).unwrap())
                .unwrap();
        assert_eq!(payload["lines"][0]["id"], "s1");
        let raw =
            r#"{"summary":"x","terms":[],"namedEntities":[],"translations":{"s1":"你好世界"}}"#;
        assert!(validate_call_answer(&task.calls[0].call, raw).is_ok());

        let allocator = Arc::new(Allocator::new(1));
        allocator.enqueue(task.calls[0].call.clone());
        let (_, lease) = allocator.allocate().unwrap();
        allocator
            .submit(&lease.lease_id, Some(answer(raw)), None)
            .unwrap();
        let applied = wait_and_apply(allocator, task, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(applied, 1);
        let saved = Doc::load(tmp.path()).unwrap();
        assert_eq!(saved.translations["zh"]["s1"].text, "你好世界");
        let analysis: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join("ai/analysis.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(analysis["summary"], "x");
        assert!(tmp.path().join("ai/translate/task.json").is_file());
        assert!(!tmp.path().join("ai/translate/brief.json").is_file());
    }

    #[test]
    fn translation_batches_large_projects_in_bounded_context_windows() {
        let tmp = tempfile::tempdir().unwrap();
        doc_with_sentences(800).save(tmp.path()).unwrap();

        let task = prepare_task(tmp.path(), "translate", Some("zh")).unwrap();

        assert_eq!(task.calls.len(), 25);
        let mut payloads = Vec::new();
        for call in &task.calls {
            let payload: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&call.pending_path).unwrap())
                    .unwrap();
            assert!(payload["lines"].as_array().unwrap().len() <= 32);
            payloads.push(payload);
        }
        assert_eq!(payloads[0]["contextBefore"].as_array().unwrap().len(), 0);
        assert_eq!(payloads[0]["contextAfter"].as_array().unwrap().len(), 3);
        assert_eq!(payloads[1]["contextBefore"].as_array().unwrap().len(), 3);
        assert_eq!(payloads[24]["contextAfter"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn translation_persists_each_completed_batch_before_the_whole_job_finishes() {
        let tmp = tempfile::tempdir().unwrap();
        doc_with_sentences(40).save(tmp.path()).unwrap();
        let task = prepare_task(tmp.path(), "translate", Some("zh")).unwrap();
        assert_eq!(task.calls.len(), 2);
        let first_done = task.calls[0].done_path.clone();
        let first_ids = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(&task.calls[0].pending_path).unwrap(),
        )
        .unwrap()["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line["id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        let first_answer = translation_answer_for(&task.calls[0]);
        let second_answer = translation_answer_for(&task.calls[1]);
        let allocator = Arc::new(Allocator::new(2));
        restore_or_enqueue(&allocator, &task).unwrap();
        let wait_allocator = allocator.clone();
        let wait_task = task.clone();
        let handle = tokio::spawn(async move {
            wait_and_apply(wait_allocator, wait_task, Duration::from_secs(3)).await
        });

        let (_, first_lease) = allocator.allocate().unwrap();
        allocator
            .submit(&first_lease.lease_id, Some(answer(&first_answer)), None)
            .unwrap();
        for _ in 0..100 {
            if first_done.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        assert!(
            first_done.exists(),
            "first batch should become durable immediately"
        );
        let partially_saved = Doc::load(tmp.path()).unwrap();
        assert!(first_ids
            .iter()
            .all(|id| partially_saved.translations["zh"].contains_key(id)));

        let (_, second_lease) = allocator.allocate().unwrap();
        allocator
            .submit(&second_lease.lease_id, Some(answer(&second_answer)), None)
            .unwrap();
        assert_eq!(handle.await.unwrap().unwrap(), 40);
    }

    #[tokio::test]
    async fn invalid_translation_batch_becomes_visible_failure_instead_of_staying_pending() {
        let tmp = tempfile::tempdir().unwrap();
        sample_doc().save(tmp.path()).unwrap();
        let task = prepare_task(tmp.path(), "translate", Some("zh")).unwrap();
        let pending_path = task.calls[0].pending_path.clone();
        let failed_path = task.calls[0].failed_path.clone();
        let allocator = Arc::new(Allocator::new(1));
        restore_or_enqueue(&allocator, &task).unwrap();
        let (_, lease) = allocator.allocate().unwrap();
        allocator
            .submit(
                &lease.lease_id,
                Some(answer(r#"{"translations":{}}"#)),
                None,
            )
            .unwrap();

        let error = wait_and_apply(allocator, task, Duration::from_secs(1))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("failed"));
        assert!(failed_path.exists());
        assert!(!pending_path.exists());
    }

    #[test]
    fn progressive_translation_analysis_uses_document_order_not_completion_order() {
        let tmp = tempfile::tempdir().unwrap();
        doc_with_sentences(40).save(tmp.path()).unwrap();
        let task = prepare_task(tmp.path(), "translate", Some("zh")).unwrap();
        let first_request: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&task.calls[0].pending_path).unwrap())
                .unwrap();
        let mut first_answer: serde_json::Value =
            serde_json::from_str(&translation_answer_for(&task.calls[0])).unwrap();
        first_answer["summary"] = serde_json::json!("first batch");
        let second_request: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&task.calls[1].pending_path).unwrap())
                .unwrap();
        let mut second_answer: serde_json::Value =
            serde_json::from_str(&translation_answer_for(&task.calls[1])).unwrap();
        second_answer["summary"] = serde_json::json!("second batch");
        crate::data::storage::write_json(
            &task.calls[1].done_path,
            &serde_json::json!({
                "callId": task.calls[1].call.id,
                "request": second_request,
                "answer": answer(&second_answer.to_string()),
            }),
        )
        .unwrap();

        let evidence =
            completed_translation_evidence(&task, &task.calls[0], &first_request, &first_answer)
                .unwrap();
        let merged = merged_analysis(&evidence, None);

        assert_eq!(merged["summary"], "first batch\n\nsecond batch");
    }

    #[test]
    fn translate_validation_rejects_incomplete_coverage() {
        let tmp = tempfile::tempdir().unwrap();
        sample_doc().save(tmp.path()).unwrap();
        let task = prepare_task(tmp.path(), "translate", Some("zh")).unwrap();
        let raw = r#"{"summary":"x","terms":[],"namedEntities":[],"translations":{}}"#;
        assert!(validate_call_answer(&task.calls[0].call, raw).is_err());
    }

    #[test]
    fn translate_materializes_and_enforces_locked_terms() {
        let tmp = tempfile::tempdir().unwrap();
        sample_doc().save(tmp.path()).unwrap();
        std::fs::create_dir_all(tmp.path().join("ai")).unwrap();
        std::fs::write(
            tmp.path().join("ai/analysis.json"),
            r#"{
              "summary":"x",
              "terms":[{"term":"Hello","observedVariants":["hello"],"locked":true}],
              "namedEntities":[]
            }"#,
        )
        .unwrap();
        let task = prepare_task(tmp.path(), "translate", Some("zh")).unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&task.calls[0].pending_path).unwrap())
                .unwrap();
        assert_eq!(payload["lines"][0]["rt"], serde_json::json!(["Hello"]));

        let missing =
            r#"{"summary":"x","terms":[],"namedEntities":[],"translations":{"s1":"你好"}}"#;
        assert!(validate_call_answer(&task.calls[0].call, missing)
            .unwrap_err()
            .iter()
            .any(|error| error.contains("rt verbatim")));
        let present =
            r#"{"summary":"x","terms":[],"namedEntities":[],"translations":{"s1":"你好 Hello"}}"#;
        assert!(validate_call_answer(&task.calls[0].call, present).is_ok());
    }

    #[test]
    fn polish_validation_rejects_text_drift() {
        let tmp = tempfile::tempdir().unwrap();
        sample_doc().save(tmp.path()).unwrap();
        let task = prepare_task(tmp.path(), "polish", None).unwrap();
        let raw = r#"{"summary":"x","terms":[],"namedEntities":[],"paragraphs":[{"sentences":["zzzz"]}]}"#;
        assert!(validate_call_answer(&task.calls[0].call, raw).is_err());
    }

    #[tokio::test]
    async fn polish_persists_analysis_quality_and_word_rebind() {
        let tmp = tempfile::tempdir().unwrap();
        let mut doc = sample_doc();
        doc.paragraphs[0].sentences[0].text = "Cloud Code works".into();
        doc.paragraphs[0].sentences[0].words = vec![
            Word {
                id: "w0".into(),
                text: "Cloud".into(),
                start: 0.0,
                end: 0.4,
            },
            Word {
                id: "w1".into(),
                text: "Code".into(),
                start: 0.5,
                end: 0.9,
            },
            Word {
                id: "w2".into(),
                text: "works".into(),
                start: 1.0,
                end: 1.4,
            },
        ];
        doc.save(tmp.path()).unwrap();
        let task = prepare_task(tmp.path(), "polish", None).unwrap();
        let raw = r#"{
          "summary":"A coding tool is discussed.",
          "terms":[{"term":"Claude Code","observedVariants":["Cloud Code"]}],
          "namedEntities":["Anthropic"],
          "paragraphs":[{"sentences":["Claude Code works"]}]
        }"#;
        assert!(validate_call_answer(&task.calls[0].call, raw).is_ok());

        let allocator = Arc::new(Allocator::new(1));
        allocator.enqueue(task.calls[0].call.clone());
        let (_, lease) = allocator.allocate().unwrap();
        allocator
            .submit(&lease.lease_id, Some(answer(raw)), None)
            .unwrap();
        wait_and_apply(allocator, task, Duration::from_secs(1))
            .await
            .unwrap();

        let saved = Doc::load(tmp.path()).unwrap();
        let words = &saved.paragraphs[0].sentences[0].words;
        assert_eq!(words[1].id, "w1");
        assert_eq!(words[1].start, 0.5);
        assert_eq!(words[2].id, "w2");
        assert_eq!(words[2].end, 1.4);
        assert_eq!(words[0].text, "Claude");
        assert_ne!(words[0].id, "w0");

        let analysis: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join("ai/analysis.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(analysis["terms"][0]["term"], "Claude Code");
        let quality = crate::pipeline::polish::PolishQualityArtifact::load(
            &tmp.path().join("ai/polish-quality.json"),
        )
        .unwrap();
        assert_eq!(
            quality.status,
            crate::pipeline::polish::PolishQualityStatus::Pass
        );
        assert!(quality.residual_term_variants.is_empty());
        assert_eq!(
            quality.fingerprint,
            crate::pipeline::fingerprint_words(&saved)
        );
    }

    #[tokio::test]
    async fn cleanup_task_applies_validated_soft_cut() {
        let tmp = tempfile::tempdir().unwrap();
        sample_doc().save(tmp.path()).unwrap();
        let task = prepare_task(tmp.path(), "cleanup", None).unwrap();
        let raw = r#"{"cuts":[{"a":"w0","b":"w0","cat":"filler","reason":"hesitation"}]}"#;
        assert!(validate_call_answer(&task.calls[0].call, raw).is_ok());

        let allocator = Arc::new(Allocator::new(1));
        allocator.enqueue(task.calls[0].call.clone());
        let (_, lease) = allocator.allocate().unwrap();
        allocator
            .submit(&lease.lease_id, Some(answer(raw)), None)
            .unwrap();
        assert_eq!(
            wait_and_apply(allocator, task, Duration::from_secs(1))
                .await
                .unwrap(),
            1
        );
        let cuts: crate::data::ClipCuts =
            serde_json::from_str(&std::fs::read_to_string(tmp.path().join("cuts.json")).unwrap())
                .unwrap();
        assert_eq!(cuts.cuts.len(), 1);
        assert_eq!(cuts.cuts[0].a_word, "w0");
    }

    #[tokio::test]
    async fn broll_task_writes_suggestions_without_mutating_doc() {
        let tmp = tempfile::tempdir().unwrap();
        let mut doc = sample_doc();
        doc.media.duration_seconds = 10.0;
        doc.paragraphs[0].sentences[0].words[0].start = 4.0;
        doc.paragraphs[0].sentences[0].words[0].end = 6.0;
        doc.save(tmp.path()).unwrap();
        let before = std::fs::read_to_string(tmp.path().join("doc.json")).unwrap();
        let task = prepare_task(tmp.path(), "broll", None).unwrap();
        let raw = r#"{"suggestions":[{"start":"w0","end":"w0","mode":"pip","query":"keyboard closeup","reason":"illustrates the point"}]}"#;
        assert!(validate_call_answer(&task.calls[0].call, raw).is_ok());

        let allocator = Arc::new(Allocator::new(1));
        allocator.enqueue(task.calls[0].call.clone());
        let (_, lease) = allocator.allocate().unwrap();
        allocator
            .submit(&lease.lease_id, Some(answer(raw)), None)
            .unwrap();
        wait_and_apply(allocator, task, Duration::from_secs(1))
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(tmp.path().join("doc.json")).unwrap(),
            before
        );
        let artifact =
            std::fs::read_to_string(tmp.path().join("ai").join("broll-suggestions.json")).unwrap();
        assert!(artifact.contains("keyboard closeup"));
    }

    #[test]
    fn task_counts_aggregate_all_kinds() {
        let tmp = tempfile::tempdir().unwrap();
        sample_doc().save(tmp.path()).unwrap();
        prepare_task(tmp.path(), "translate", Some("zh")).unwrap();
        prepare_task(tmp.path(), "polish", None).unwrap();
        let (pending, done) = task_counts(tmp.path());
        assert_eq!(pending, 2);
        assert_eq!(done, 0);
    }

    #[test]
    fn task_counts_do_not_leave_failed_calls_pending() {
        let tmp = tempfile::tempdir().unwrap();
        let kind = tmp.path().join("ai/translate");
        std::fs::create_dir_all(kind.join("pending")).unwrap();
        std::fs::create_dir_all(kind.join("failed")).unwrap();
        std::fs::write(kind.join("pending/call.json"), "{}").unwrap();
        std::fs::write(kind.join("failed/call.json"), r#"{"error":"unauthorized"}"#).unwrap();

        let (pending, done) = task_counts(tmp.path());
        assert_eq!(pending, 0);
        assert_eq!(done, 0);
    }

    #[test]
    fn stale_only_translate_materializes_only_changed_sources() {
        let tmp = tempfile::tempdir().unwrap();
        let mut doc = sample_doc();
        doc.translations.insert(
            "zh".into(),
            BTreeMap::from([(
                "s1".into(),
                TranslationGroup {
                    id: "s1".into(),
                    text: "你好世界".into(),
                    source_words: vec!["w0".into()],
                    source_text: Some("hello world".into()),
                },
            )]),
        );
        doc.save(tmp.path()).unwrap();
        let clean = prepare_task_with_options(tmp.path(), "translate", Some("zh"), true).unwrap();
        assert!(clean.calls.is_empty());

        doc.paragraphs[0].sentences[0].text = "hello edited world".into();
        doc.save(tmp.path()).unwrap();
        let stale = prepare_task_with_options(tmp.path(), "translate", Some("zh"), true).unwrap();
        assert_eq!(stale.calls.len(), 1);
    }

    #[test]
    fn align_task_materializes_scoped_compact_v4_pairs() {
        let tmp = tempfile::tempdir().unwrap();
        let mut doc = sample_doc();
        doc.translations.insert(
            "zh".into(),
            BTreeMap::from([(
                "s1".into(),
                TranslationGroup {
                    id: "s1".into(),
                    text: "这是一个明显超过单行容量的中文翻译文本".into(),
                    source_words: vec!["w0".into()],
                    source_text: Some("hello world".into()),
                },
            )]),
        );
        doc.save(tmp.path()).unwrap();

        let task = prepare_task_with_task_options(
            tmp.path(),
            "align",
            Some("zh"),
            TaskOptions {
                groups: vec!["s1".into()],
                align_fit: Some(8),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(task.calls.len(), 1);
        let payload: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&task.calls[0].pending_path).unwrap())
                .unwrap();
        assert_eq!(payload["lang"], "zh");
        assert_eq!(payload["budgets"]["f"], 8);
        assert_eq!(payload["pairs"][0]["id"], "s1");
        assert!(payload["pairs"][0]["sm"]
            .as_str()
            .unwrap()
            .contains("hello"));
        assert!(payload["pairs"][0]["tm"].as_str().unwrap().contains("<@t"));
        assert_eq!(payload["pairs"][0]["problems"][0], "overFit");
    }

    #[tokio::test]
    async fn align_task_rejects_incomplete_hard_coverage_and_applies_rewrite() {
        let tmp = tempfile::tempdir().unwrap();
        let mut doc = sample_doc();
        doc.translations.insert(
            "zh".into(),
            BTreeMap::from([(
                "s1".into(),
                TranslationGroup {
                    id: "s1".into(),
                    text: "这是一个明显超过单行容量的中文翻译文本".into(),
                    source_words: vec!["w0".into()],
                    source_text: Some("hello world".into()),
                },
            )]),
        );
        doc.save(tmp.path()).unwrap();
        let task = prepare_task_with_task_options(
            tmp.path(),
            "align",
            Some("zh"),
            TaskOptions {
                groups: vec!["s1".into()],
                align_fit: Some(8),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(validate_call_answer(&task.calls[0].call, r#"{"pairs":[]}"#).is_err());
        let raw = r#"{"pairs":[{"id":"s1","action":"rewrite","reasonCode":"mistranslation","reason":"shorten","pieces":[{"through":"end","t":"精简翻译"}]}]}"#;
        assert!(validate_call_answer(&task.calls[0].call, raw).is_ok());

        let allocator = Arc::new(Allocator::new(1));
        allocator.enqueue(task.calls[0].call.clone());
        let (_, lease) = allocator.allocate().unwrap();
        allocator
            .submit(&lease.lease_id, Some(answer(raw)), None)
            .unwrap();
        assert_eq!(
            wait_and_apply(allocator, task, Duration::from_secs(1))
                .await
                .unwrap(),
            1
        );
        let saved = Doc::load(tmp.path()).unwrap();
        assert_eq!(saved.translations["zh"]["s1"].text, "精简翻译");
        assert!(tmp.path().join("ai").join("align-artifact.json").exists());
    }

    #[test]
    fn align_recut_rejects_any_remaining_unit_over_hard() {
        let tmp = tempfile::tempdir().unwrap();
        let mut doc = sample_doc();
        doc.paragraphs[0].sentences[0].words.push(Word {
            id: "w1".into(),
            text: "world".into(),
            start: 1.0,
            end: 2.0,
        });
        doc.translations.insert(
            "zh".into(),
            BTreeMap::from([(
                "s1".into(),
                TranslationGroup {
                    id: "s1".into(),
                    text: "一二三四五六七八九十一二三四五六七八九十一二三四".into(),
                    source_words: vec!["w0".into(), "w1".into()],
                    source_text: Some("hello world".into()),
                },
            )]),
        );
        doc.save(tmp.path()).unwrap();
        let task = prepare_task_with_task_options(
            tmp.path(),
            "align",
            Some("zh"),
            TaskOptions {
                groups: vec!["s1".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let leaves_twenty_three =
            r#"{"pairs":[{"id":"s1","action":"recut","cuts":[{"s":"w1","t":"t1"}]}]}"#;
        assert!(validate_call_answer(&task.calls[0].call, leaves_twenty_three).is_err());
        let balanced = r#"{"pairs":[{"id":"s1","action":"recut","cuts":[{"s":"w1","t":"t12"}]}]}"#;
        assert!(validate_call_answer(&task.calls[0].call, balanced).is_ok());
    }

    #[tokio::test]
    async fn repunct_task_accepts_only_payload_seams_and_changes_no_wording() {
        let tmp = tempfile::tempdir().unwrap();
        let mut doc = sample_doc();
        doc.paragraphs[0].sentences[0].text = "hello world".into();
        doc.paragraphs[0].sentences[0].words = vec![
            Word {
                id: "w0".into(),
                text: "hello".into(),
                start: 0.0,
                end: 0.5,
            },
            Word {
                id: "w1".into(),
                text: "world".into(),
                start: 0.5,
                end: 1.0,
            },
        ];
        doc.save(tmp.path()).unwrap();
        let task = prepare_task(tmp.path(), "repunct", None).unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&task.calls[0].pending_path).unwrap())
                .unwrap();
        assert_eq!(payload["segs"][0]["id"], 1);
        assert_eq!(payload["segs"][0]["cm"][0]["id"], "c-w0");
        assert!(validate_call_answer(
            &task.calls[0].call,
            r#"{"segs":[{"id":1,"cuts":[{"id":"unknown","m":"，"}]}]}"#
        )
        .is_err());
        let raw = r#"{"segs":[{"id":1,"cuts":[{"id":"c-w0","m":"，"},{"id":"c-w1","m":"。"}]}]}"#;
        assert!(validate_call_answer(&task.calls[0].call, raw).is_ok());

        let allocator = Arc::new(Allocator::new(1));
        allocator.enqueue(task.calls[0].call.clone());
        let (_, lease) = allocator.allocate().unwrap();
        allocator
            .submit(&lease.lease_id, Some(answer(raw)), None)
            .unwrap();
        assert_eq!(
            wait_and_apply(allocator, task, Duration::from_secs(1))
                .await
                .unwrap(),
            2
        );
        let saved = Doc::load(tmp.path()).unwrap();
        assert_eq!(saved.paragraphs[0].sentences[0].text, "hello， world。");
        let lexical: String = saved.paragraphs[0].sentences[0]
            .text
            .chars()
            .filter(|character| character.is_alphanumeric())
            .collect();
        assert_eq!(lexical, "helloworld");
    }

    #[tokio::test]
    async fn chapters_task_validates_ndjson_order_and_persists_native_chapters() {
        let tmp = tempfile::tempdir().unwrap();
        let mut doc = sample_doc();
        doc.paragraphs[0].sentences.push(Sentence {
            id: "s2".into(),
            text: "second topic".into(),
            words: vec![Word {
                id: "w1".into(),
                text: "second".into(),
                start: 1.0,
                end: 2.0,
            }],
        });
        doc.save(tmp.path()).unwrap();
        let task = prepare_task(tmp.path(), "chapters", None).unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&task.calls[0].pending_path).unwrap())
                .unwrap();
        assert_eq!(payload["segments"][0]["id"], "s1");
        assert_eq!(payload["segments"][1]["id"], "s2");
        assert!(validate_call_answer(
            &task.calls[0].call,
            "{\"title\":\"Later\",\"startSeg\":\"s2\"}\n{\"title\":\"Intro\",\"startSeg\":\"s1\"}"
        )
        .is_err());
        let raw =
            "{\"title\":\"Intro\",\"startSeg\":\"s1\"}\n{\"title\":\"Body\",\"startSeg\":\"s2\"}";
        assert!(validate_call_answer(&task.calls[0].call, raw).is_ok());

        let allocator = Arc::new(Allocator::new(1));
        allocator.enqueue(task.calls[0].call.clone());
        let (_, lease) = allocator.allocate().unwrap();
        allocator
            .submit(&lease.lease_id, Some(answer(raw)), None)
            .unwrap();
        assert_eq!(
            wait_and_apply(allocator, task, Duration::from_secs(1))
                .await
                .unwrap(),
            2
        );
        let native: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(tmp.path().join("doc.json")).unwrap())
                .unwrap();
        assert_eq!(native["chapters"][0]["title"], "Intro");
        assert_eq!(native["chapters"][1]["startSeg"], "s2");
        assert!(tmp.path().join("chapters.json").exists());

        let mut reloaded = Doc::load(tmp.path()).unwrap();
        reloaded.meta.description = "saved again".into();
        reloaded.save(tmp.path()).unwrap();
        let preserved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(tmp.path().join("doc.json")).unwrap())
                .unwrap();
        assert_eq!(preserved["chapters"][0]["title"], "Intro");
    }

    #[tokio::test]
    async fn segment_task_only_repartitions_verbatim_sentence_text() {
        let tmp = tempfile::tempdir().unwrap();
        let mut doc = sample_doc();
        doc.paragraphs[0].sentences = vec![
            Sentence {
                id: "s1".into(),
                text: "First sentence.".into(),
                words: vec![Word {
                    id: "w0".into(),
                    text: "First sentence.".into(),
                    start: 0.0,
                    end: 1.0,
                }],
            },
            Sentence {
                id: "s2".into(),
                text: "Second sentence.".into(),
                words: vec![Word {
                    id: "w1".into(),
                    text: "Second sentence.".into(),
                    start: 1.0,
                    end: 2.0,
                }],
            },
            Sentence {
                id: "s3".into(),
                text: "Third sentence.".into(),
                words: vec![Word {
                    id: "w2".into(),
                    text: "Third sentence.".into(),
                    start: 2.0,
                    end: 3.0,
                }],
            },
        ];
        doc.save(tmp.path()).unwrap();
        let task = prepare_task(tmp.path(), "segment", None).unwrap();
        let invalid = r#"{"paragraphs":["First","sentence. Second sentence. Third sentence."]}"#;
        assert!(validate_call_answer(&task.calls[0].call, invalid).is_err());
        let raw = r#"{"paragraphs":["First sentence. Second sentence.","Third sentence."]}"#;
        assert!(validate_call_answer(&task.calls[0].call, raw).is_ok());

        let allocator = Arc::new(Allocator::new(1));
        allocator.enqueue(task.calls[0].call.clone());
        let (_, lease) = allocator.allocate().unwrap();
        allocator
            .submit(&lease.lease_id, Some(answer(raw)), None)
            .unwrap();
        assert_eq!(
            wait_and_apply(allocator, task, Duration::from_secs(1))
                .await
                .unwrap(),
            2
        );
        let saved = Doc::load(tmp.path()).unwrap();
        assert_eq!(saved.paragraphs.len(), 2);
        assert_eq!(saved.paragraphs[0].sentences.len(), 2);
        assert_eq!(saved.paragraphs[1].sentences[0].id, "s3");
        let wording: Vec<_> = saved
            .paragraphs
            .iter()
            .flat_map(|paragraph| paragraph.sentences.iter())
            .map(|sentence| sentence.text.as_str())
            .collect();
        assert_eq!(
            wording,
            vec!["First sentence.", "Second sentence.", "Third sentence."]
        );
    }
}
