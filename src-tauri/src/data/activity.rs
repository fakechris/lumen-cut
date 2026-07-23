use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::AppResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectActivity {
    updated_at: DateTime<Utc>,
}

fn path(project_dir: &Path) -> std::path::PathBuf {
    project_dir.join(".lumen-cut").join("activity.json")
}

pub fn load(project_dir: &Path) -> Option<DateTime<Utc>> {
    std::fs::read_to_string(path(project_dir))
        .ok()
        .and_then(|raw| serde_json::from_str::<ProjectActivity>(&raw).ok())
        .map(|activity| activity.updated_at)
}

pub fn touch(project_dir: &Path) -> AppResult<DateTime<Utc>> {
    let updated_at = Utc::now();
    crate::data::storage::write_json(&path(project_dir), &ProjectActivity { updated_at })?;
    Ok(updated_at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_timestamp_is_durable_and_local_to_project_metadata() {
        let project = tempfile::tempdir().unwrap();
        assert!(load(project.path()).is_none());
        let touched = touch(project.path()).unwrap();
        assert_eq!(load(project.path()), Some(touched));
        assert!(project.path().join(".lumen-cut/activity.json").is_file());
    }
}
