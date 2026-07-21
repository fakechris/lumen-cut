//! Version control on top of `doc.json`.
//!
//! The on-disk lineage contains branches and version nodes. Snapshot paths are
//! conventional (`versions/<id>/doc.json`) rather than stored in each node.
//!
//! Each commit writes a full `doc.json` snapshot under `versions/<id>/`
//! (git-like, no diff encoding), and `restore` copies that snapshot back
//! to the working `doc.json`. The 3-way cue merge (kept below) is the
//! agent-driven reconciliation step.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::data::doc::Doc;
use crate::error::{AppError, AppResult};

/// Per-document version graph: the branches plus every committed node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Lineage {
    #[serde(default = "lineage_format_version")]
    pub v: u32,
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub branches: Vec<Branch>,
    #[serde(default, rename = "versions", alias = "nodes")]
    pub nodes: Vec<VersionNode>,
    /// The currently active branch id. `None` means a single-branch working
    /// copy.
    #[serde(default, skip_serializing)]
    pub active_branch: Option<String>,
}

const fn lineage_format_version() -> u32 {
    1
}

impl Default for Lineage {
    fn default() -> Self {
        Self {
            v: lineage_format_version(),
            head: None,
            branches: Vec::new(),
            nodes: Vec::new(),
            active_branch: None,
        }
    }
}

impl Lineage {
    /// Load `lineage.json` from a project dir, or start a fresh graph.
    pub fn load(dir: &Path) -> AppResult<Self> {
        let p = dir.join("lineage.json");
        if !p.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&p)?;
        let mut lineage: Self = serde_json::from_str(&raw)?;
        if let Ok(active) = std::fs::read_to_string(dir.join("active-branch")) {
            let active = active.trim();
            if !active.is_empty() {
                lineage.active_branch = Some(active.into());
            }
        }
        Ok(lineage)
    }

    /// Persist to `<dir>/lineage.json`.
    pub fn save(&self, dir: &Path) -> AppResult<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(
            dir.join("lineage.json"),
            serde_json::to_string_pretty(self)?,
        )?;
        if let Some(active) = &self.active_branch {
            std::fs::write(dir.join("active-branch"), active)?;
        }
        Ok(())
    }

    /// The most recently committed node (working head). For a single-branch
    /// working copy, the last node is the fallback head.
    pub fn head(&self) -> Option<&VersionNode> {
        self.active_branch
            .as_deref()
            .and_then(|id| self.branches.iter().find(|branch| branch.id == id))
            .and_then(|branch| self.node(&branch.tip))
            .or_else(|| self.head.as_deref().and_then(|id| self.node(id)))
            .or_else(|| self.nodes.last())
    }

    pub fn node(&self, id: &str) -> Option<&VersionNode> {
        self.nodes.iter().find(|n| n.id == id)
    }
}

/// A version snapshot. `diffs` carries an optional per-cue change log in
/// addition to the full document snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VersionNode {
    pub id: String,
    #[serde(default)]
    pub parent: Option<String>,
    pub branch: String,
    pub name: String,
    #[serde(default)]
    pub note: String,
    pub at: chrono::DateTime<chrono::Utc>,
    pub kind: VersionKind,
    /// Per-cue change log versus the parent. Empty when only the full snapshot
    /// is available.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diffs: Vec<CueDiff>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VersionKind {
    Manual,
    Agent,
    Auto,
    Restore,
}

/// A branch in the version graph.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Branch {
    pub id: String,
    pub name: String,
    /// Head VersionNode id of this branch.
    pub tip: String,
    /// First VersionNode id of this branch (the branch point's child).
    pub root: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum CueDiff {
    Replace {
        cue_id: String,
        before: String,
        after: String,
    },
    ReplaceSentence {
        sentence_id: String,
        before: String,
        after: String,
    },
    ReplaceGroup {
        group_id: String,
        lang: String,
        before: String,
        after: String,
    },
    CutAdded {
        cut_id: String,
    },
    CutRemoved {
        cut_id: String,
    },
    Reindex {
        map: Vec<(String, String)>,
    },
}

impl CueDiff {
    pub fn kind_label(&self) -> &'static str {
        match self {
            CueDiff::Replace { .. } => "replace",
            CueDiff::ReplaceSentence { .. } => "replaceSentence",
            CueDiff::ReplaceGroup { .. } => "replaceGroup",
            CueDiff::CutAdded { .. } => "cutAdded",
            CueDiff::CutRemoved { .. } => "cutRemoved",
            CueDiff::Reindex { .. } => "reindex",
        }
    }
}

/// Path of a version's full snapshot: `<project>/versions/<id>/doc.json`.
pub fn snapshot_path(dir: &Path, id: &str) -> PathBuf {
    dir.join("versions").join(id).join("doc.json")
}

/// Commit a full `doc.json` snapshot under `versions/<id>/` and append a
/// `VersionNode` to the lineage. Returns the new version id.
pub fn commit_snapshot(
    dir: &Path,
    doc: &Doc,
    lineage: &mut Lineage,
    branch: &str,
    name: &str,
    note: &str,
    kind: VersionKind,
) -> AppResult<String> {
    let mut seq = lineage.nodes.len();
    let id = loop {
        let candidate = format!("v{seq}");
        if lineage.node(&candidate).is_none() {
            break candidate;
        }
        seq += 1;
    };
    let parent = lineage
        .branches
        .iter()
        .find(|item| item.id == branch)
        .map(|item| item.tip.clone())
        .or_else(|| lineage.head().map(|head| head.id.clone()));
    let snap_dir = dir.join("versions").join(&id);
    // Seed a snapshot with the native flat document when present. `Doc::save`
    // then updates understood cue fields while preserving unknown keys.
    let working_doc = dir.join("doc.json");
    if let Ok(raw) = std::fs::read_to_string(&working_doc) {
        let is_flat = serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .is_some_and(|value| value.get("cues").is_some() && value.get("paragraphs").is_none());
        if is_flat {
            std::fs::create_dir_all(&snap_dir)?;
            std::fs::write(snap_dir.join("doc.json"), raw)?;
        }
    }
    doc.save(&snap_dir)?;
    lineage.nodes.push(VersionNode {
        id: id.clone(),
        parent,
        branch: branch.into(),
        name: name.into(),
        note: note.into(),
        at: chrono::Utc::now(),
        kind,
        diffs: Vec::new(),
    });
    lineage.head = Some(id.clone());
    if let Some(item) = lineage.branches.iter_mut().find(|item| item.id == branch) {
        item.tip = id.clone();
    } else {
        lineage.branches.push(Branch {
            id: branch.into(),
            name: if branch == "main" {
                "Main".into()
            } else {
                branch.into()
            },
            tip: id.clone(),
            root: id.clone(),
            created_at: chrono::Utc::now(),
            note: String::new(),
        });
    }
    if lineage.active_branch.is_none() {
        lineage.active_branch = Some(branch.into());
    }
    lineage.save(dir)?;
    Ok(id)
}

/// Restore a version: copy its `versions/<id>/doc.json` snapshot back to
/// the working `doc.json`. The restore itself is recorded as a new node of
/// kind `Restore`.
pub fn restore_snapshot(dir: &Path, lineage: &mut Lineage, id: &str) -> AppResult<()> {
    let snap = snapshot_path(dir, id);
    if !snap.exists() {
        return Err(AppError::ProjectNotFound(snap));
    }
    let doc = Doc::load(snap.parent().unwrap())?;
    doc.save(dir)?;
    let branch = lineage
        .active_branch
        .clone()
        .unwrap_or_else(|| "main".into());
    commit_snapshot(
        dir,
        &doc,
        lineage,
        &branch,
        &format!("restore {id}"),
        &format!("restored from {id}"),
        VersionKind::Restore,
    )?;
    Ok(())
}

/// Fork a branch from the active tip. The new branch is not activated until
/// [`switch_branch`] is called, so creating a draft never rewrites the
/// current working document.
pub fn create_branch(
    dir: &Path,
    lineage: &mut Lineage,
    name: &str,
    note: &str,
) -> AppResult<String> {
    if lineage.nodes.is_empty() {
        let doc = Doc::load(dir)?;
        commit_snapshot(
            dir,
            &doc,
            lineage,
            "main",
            "initial",
            "created before first branch",
            VersionKind::Auto,
        )?;
    }
    let tip = lineage
        .head()
        .map(|head| head.id.clone())
        .ok_or_else(|| AppError::Schema("cannot branch without a version snapshot".into()))?;
    let mut seq = lineage.branches.len();
    let id = loop {
        let candidate = format!("b{seq}");
        if !lineage.branches.iter().any(|branch| branch.id == candidate) {
            break candidate;
        }
        seq += 1;
    };
    lineage.branches.push(Branch {
        id: id.clone(),
        name: name.into(),
        tip: tip.clone(),
        root: tip,
        created_at: chrono::Utc::now(),
        note: note.into(),
    });
    lineage.save(dir)?;
    Ok(id)
}

/// Activate a branch and restore its tip snapshot into the working document.
pub fn switch_branch(dir: &Path, lineage: &mut Lineage, id: &str) -> AppResult<()> {
    let tip = lineage
        .branches
        .iter()
        .find(|branch| branch.id == id)
        .map(|branch| branch.tip.clone())
        .ok_or_else(|| AppError::Schema(format!("branch {id} not found")))?;
    let snap = snapshot_path(dir, &tip);
    if !snap.exists() {
        return Err(AppError::ProjectNotFound(snap));
    }
    let doc = Doc::load(
        snap.parent()
            .ok_or_else(|| AppError::Schema(format!("invalid snapshot path for {tip}")))?,
    )?;
    doc.save(dir)?;
    lineage.active_branch = Some(id.into());
    lineage.head = Some(tip);
    lineage.save(dir)
}

/// A project is committed when the working `doc.json` exactly matches the
/// active branch tip snapshot. Missing lineage or a missing snapshot is not
/// treated as committed.
pub fn working_head_is_committed(dir: &Path, doc: &Doc) -> AppResult<bool> {
    let lineage = Lineage::load(dir)?;
    let Some(head) = lineage.head() else {
        return Ok(false);
    };
    let snap = snapshot_path(dir, &head.id);
    if !snap.exists() {
        return Ok(false);
    }
    let mut snapshot = Doc::load(
        snap.parent()
            .ok_or_else(|| AppError::Schema("invalid version snapshot path".into()))?,
    )?;
    // Flat native documents do not necessarily persist these compatibility
    // timestamps; importing them synthesizes `now`, so they are not evidence
    // of an uncommitted content change.
    snapshot.meta.created_at = doc.meta.created_at;
    snapshot.meta.updated_at = doc.meta.updated_at;
    Ok(&snapshot == doc)
}

/// 3-way cue-level merge. Inputs are **base**, **ours**, **theirs** cue→text
/// maps. Per cue: (1) all equal → keep; (2) base==ours → take theirs;
/// (3) base==theirs → keep ours; (4) ours==theirs → keep; (5) diverge →
/// conflict.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct MergeResult {
    pub merged: BTreeMap<String, String>,
    pub conflicts: Vec<MergeConflict>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MergeConflict {
    pub cue_id: String,
    pub base: String,
    pub ours: String,
    pub theirs: String,
}

pub fn three_way_merge(
    base: &BTreeMap<String, String>,
    ours: &BTreeMap<String, String>,
    theirs: &BTreeMap<String, String>,
) -> MergeResult {
    let mut out = MergeResult::default();
    let keys: std::collections::BTreeSet<&String> = base
        .keys()
        .chain(ours.keys())
        .chain(theirs.keys())
        .collect();
    for k in keys {
        let b = base.get(k).cloned().unwrap_or_default();
        let o = ours.get(k).cloned().unwrap_or_default();
        let t = theirs.get(k).cloned().unwrap_or_default();
        if o == t {
            if !o.is_empty() {
                out.merged.insert(k.clone(), o);
            }
        } else if b == o {
            if !t.is_empty() {
                out.merged.insert(k.clone(), t);
            }
        } else if b == t {
            if !o.is_empty() {
                out.merged.insert(k.clone(), o);
            }
        } else {
            out.conflicts.push(MergeConflict {
                cue_id: k.clone(),
                base: b,
                ours: o,
                theirs: t,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::doc::*;

    fn m(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn merge_no_changes() {
        let base = m(&[("w0", "alpha"), ("w1", "beta")]);
        let out = three_way_merge(&base, &base, &base);
        assert!(out.conflicts.is_empty());
        assert_eq!(out.merged["w0"], "alpha");
    }

    #[test]
    fn merge_conflicting_edits() {
        let base = m(&[("w0", "alpha")]);
        let ours = m(&[("w0", "alpha-two")]);
        let theirs = m(&[("w0", "alpha-three")]);
        let out = three_way_merge(&base, &ours, &theirs);
        assert_eq!(out.conflicts.len(), 1);
    }

    fn sample_doc() -> Doc {
        Doc {
            id: "p".into(),
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
                language: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            paragraphs: vec![],
            translations: Default::default(),
        }
    }

    #[test]
    fn commit_writes_snapshot_and_node_then_restore_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut lineage = Lineage::default();
        let mut doc = sample_doc();
        doc.meta.title = "first".into();
        let id1 = commit_snapshot(
            dir,
            &doc,
            &mut lineage,
            "main",
            "initial",
            "",
            VersionKind::Manual,
        )
        .unwrap();
        doc.meta.title = "second".into();
        let _id2 = commit_snapshot(
            dir,
            &doc,
            &mut lineage,
            "main",
            "edit",
            "",
            VersionKind::Manual,
        )
        .unwrap();

        // Snapshot files exist.
        assert!(snapshot_path(dir, &id1).exists());
        // Lineage persisted with two nodes.
        let reloaded = Lineage::load(dir).unwrap();
        assert_eq!(reloaded.nodes.len(), 2);
        assert_eq!(reloaded.nodes[0].id, "v0");
        assert_eq!(reloaded.nodes[1].parent.as_deref(), Some("v0"));

        // Restore v0 → working doc.json title reverts to "first".
        let mut lineage = reloaded;
        restore_snapshot(dir, &mut lineage, "v0").unwrap();
        let restored = Doc::load(dir).unwrap();
        assert_eq!(restored.meta.title, "first");
        // restore recorded as a Restore node.
        assert_eq!(lineage.head().unwrap().kind, VersionKind::Restore);
    }

    #[test]
    fn version_node_serializes_public_keys() {
        let n = VersionNode {
            id: "v0".into(),
            parent: None,
            branch: "main".into(),
            name: "initial".into(),
            note: "".into(),
            at: chrono::Utc::now(),
            kind: VersionKind::Manual,
            diffs: vec![],
        };
        let s = serde_json::to_string(&n).unwrap();
        for k in [
            "\"id\"",
            "\"branch\"",
            "\"name\"",
            "\"note\"",
            "\"at\"",
            "\"kind\"",
        ] {
            assert!(s.contains(k), "missing {k}: {s}");
        }
        assert!(!s.contains("created_at"));
    }

    #[test]
    fn commits_follow_and_advance_the_active_branch_tip() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut lineage = Lineage::default();
        let mut doc = sample_doc();

        let v0 = commit_snapshot(
            dir,
            &doc,
            &mut lineage,
            "main",
            "initial",
            "",
            VersionKind::Manual,
        )
        .unwrap();
        let branch = create_branch(dir, &mut lineage, "Draft", "").unwrap();
        switch_branch(dir, &mut lineage, &branch).unwrap();
        doc.meta.title = "draft".into();
        doc.save(dir).unwrap();
        let v1 = commit_snapshot(
            dir,
            &doc,
            &mut lineage,
            &branch,
            "draft edit",
            "",
            VersionKind::Manual,
        )
        .unwrap();

        assert_eq!(
            lineage.node(&v1).unwrap().parent.as_deref(),
            Some(v0.as_str())
        );
        assert_eq!(
            lineage
                .branches
                .iter()
                .find(|b| b.id == branch)
                .unwrap()
                .tip,
            v1
        );
    }

    #[test]
    fn switching_branch_restores_its_tip_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let mut lineage = Lineage::default();
        let mut doc = sample_doc();
        doc.meta.title = "main".into();
        commit_snapshot(
            dir,
            &doc,
            &mut lineage,
            "main",
            "initial",
            "",
            VersionKind::Manual,
        )
        .unwrap();
        let draft = create_branch(dir, &mut lineage, "Draft", "").unwrap();
        switch_branch(dir, &mut lineage, &draft).unwrap();
        doc.meta.title = "draft".into();
        doc.save(dir).unwrap();
        commit_snapshot(
            dir,
            &doc,
            &mut lineage,
            &draft,
            "draft",
            "",
            VersionKind::Manual,
        )
        .unwrap();

        switch_branch(dir, &mut lineage, "main").unwrap();
        assert_eq!(Doc::load(dir).unwrap().meta.title, "main");
        switch_branch(dir, &mut lineage, &draft).unwrap();
        assert_eq!(Doc::load(dir).unwrap().meta.title, "draft");
    }

    #[test]
    fn snapshot_keeps_unknown_fields_from_flat_native_doc() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(
            dir.join("doc.json"),
            r#"{
                "id":"p","title":"native","durationSeconds":1.0,
                "opaque":{"keep":true},
                "cues":[{"id":"s1","startMs":0,"endMs":1000,"text":"hello","x":7}]
            }"#,
        )
        .unwrap();
        let doc = Doc::load(dir).unwrap();
        let mut lineage = Lineage::default();
        let id = commit_snapshot(
            dir,
            &doc,
            &mut lineage,
            "main",
            "native",
            "",
            VersionKind::Manual,
        )
        .unwrap();
        let snapshot: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(snapshot_path(dir, &id)).unwrap())
                .unwrap();
        assert_eq!(snapshot["opaque"]["keep"], true);
        assert_eq!(snapshot["cues"][0]["x"], 7);
        assert!(snapshot.get("paragraphs").is_none());
        assert!(working_head_is_committed(dir, &doc).unwrap());
    }

    #[test]
    fn lineage_serializes_native_top_level_keys_without_extensions() {
        let tmp = tempfile::tempdir().unwrap();
        let mut lineage = Lineage::default();
        let doc = sample_doc();
        commit_snapshot(
            tmp.path(),
            &doc,
            &mut lineage,
            "main",
            "initial",
            "",
            VersionKind::Manual,
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join("lineage.json")).unwrap(),
        )
        .unwrap();
        for key in ["v", "head", "branches", "versions"] {
            assert!(value.get(key).is_some(), "missing {key}");
        }
        assert!(value.get("nodes").is_none());
        assert!(value.get("activeBranch").is_none());
        assert!(value["versions"][0].get("diffs").is_none());
    }
}
