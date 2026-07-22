use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PlacementMode {
    Fullscreen,
    #[default]
    Pip,
}

impl std::str::FromStr for PlacementMode {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "fullscreen" => Ok(Self::Fullscreen),
            "pip" => Ok(Self::Pip),
            _ => Err(AppError::Schema(format!(
                "invalid B-roll mode `{value}`; expected fullscreen|pip"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FitMode {
    #[default]
    Cover,
    Contain,
}

impl std::str::FromStr for FitMode {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "cover" => Ok(Self::Cover),
            "contain" => Ok(Self::Contain),
            _ => Err(AppError::Schema(format!(
                "invalid B-roll fit `{value}`; expected cover|contain"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundMode {
    Blur,
    #[default]
    Black,
}

impl std::str::FromStr for BackgroundMode {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "blur" => Ok(Self::Blur),
            "black" => Ok(Self::Black),
            _ => Err(AppError::Schema(format!(
                "invalid B-roll background `{value}`; expected blur|black"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrollPlacement {
    pub id: String,
    pub file: PathBuf,
    #[serde(default)]
    pub start: f64,
    #[serde(default = "default_end")]
    pub end: f64,
    #[serde(default)]
    pub mode: PlacementMode,
    #[serde(default)]
    pub rect: Option<Rect>,
    #[serde(default)]
    pub fit: FitMode,
    #[serde(default)]
    pub background: BackgroundMode,
    #[serde(default)]
    pub source_start: f64,
    #[serde(default)]
    pub radius: u32,
    #[serde(default)]
    pub name: Option<String>,
}

const fn default_end() -> f64 {
    4.0
}

impl BrollPlacement {
    pub fn validate(&self) -> AppResult<()> {
        if self.id.trim().is_empty() {
            return Err(AppError::Schema("B-roll id cannot be empty".into()));
        }
        if self.file.as_os_str().is_empty() {
            return Err(AppError::Schema("B-roll asset path cannot be empty".into()));
        }
        if !self.start.is_finite()
            || !self.end.is_finite()
            || self.start < 0.0
            || self.end <= self.start
        {
            return Err(AppError::Schema(format!(
                "B-roll {} has invalid timeline [{},{}]",
                self.id, self.start, self.end
            )));
        }
        if !self.source_start.is_finite() || self.source_start < 0.0 {
            return Err(AppError::Schema(format!(
                "B-roll {} has invalid source start",
                self.id
            )));
        }
        if self
            .rect
            .is_some_and(|rect| rect.width == 0 || rect.height == 0)
        {
            return Err(AppError::Schema(format!(
                "B-roll {} has an empty rectangle",
                self.id
            )));
        }
        Ok(())
    }
}

pub fn load(project_dir: &Path) -> AppResult<Vec<BrollPlacement>> {
    let path = project_dir.join("broll.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let placements: Vec<BrollPlacement> = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    for placement in &placements {
        placement.validate()?;
    }
    Ok(placements)
}

pub fn save(project_dir: &Path, placements: &[BrollPlacement]) -> AppResult<()> {
    for placement in placements {
        placement.validate()?;
    }
    std::fs::create_dir_all(project_dir)?;
    crate::data::storage::write_json(&project_dir.join("broll.json"), placements)
}

#[derive(Debug, Clone, Default)]
pub struct PlacementPatch {
    pub file: Option<PathBuf>,
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub mode: Option<PlacementMode>,
    pub rect: Option<Option<Rect>>,
    pub fit: Option<FitMode>,
    pub background: Option<BackgroundMode>,
    pub source_start: Option<f64>,
    pub radius: Option<u32>,
    pub name: Option<Option<String>>,
}

pub fn update(
    placements: &mut [BrollPlacement],
    id: &str,
    patch: PlacementPatch,
) -> AppResult<bool> {
    let Some(index) = placements.iter().position(|placement| placement.id == id) else {
        return Ok(false);
    };
    let mut candidate = placements[index].clone();
    if let Some(file) = patch.file {
        candidate.file = file;
    }
    if let Some(start) = patch.start {
        candidate.start = start;
    }
    if let Some(end) = patch.end {
        candidate.end = end;
    }
    if let Some(mode) = patch.mode {
        candidate.mode = mode;
    }
    if let Some(rect) = patch.rect {
        candidate.rect = rect;
    }
    if let Some(fit) = patch.fit {
        candidate.fit = fit;
    }
    if let Some(background) = patch.background {
        candidate.background = background;
    }
    if let Some(source_start) = patch.source_start {
        candidate.source_start = source_start;
    }
    if let Some(radius) = patch.radius {
        candidate.radius = radius;
    }
    if let Some(name) = patch.name {
        candidate.name = name;
    }
    candidate.validate()?;
    placements[index] = candidate;
    Ok(true)
}

pub fn parse_rect(raw: &str) -> AppResult<Rect> {
    let values: Vec<u32> = raw
        .split(',')
        .map(str::trim)
        .map(|value| {
            value.parse::<u32>().map_err(|_| {
                AppError::Schema(
                    "B-roll rect must be x,y,width,height with nonnegative integers".into(),
                )
            })
        })
        .collect::<AppResult<_>>()?;
    if values.len() != 4 || values[2] == 0 || values[3] == 0 {
        return Err(AppError::Schema(
            "B-roll rect must be x,y,width,height with positive width/height".into(),
        ));
    }
    Ok(Rect {
        x: values[0],
        y: values[1],
        width: values[2],
        height: values[3],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_placement_round_trips_all_render_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let placement = BrollPlacement {
            id: "br-1".into(),
            file: "/tmp/shot.png".into(),
            start: 3.0,
            end: 7.0,
            mode: PlacementMode::Pip,
            rect: Some(Rect {
                x: 100,
                y: 80,
                width: 640,
                height: 360,
            }),
            fit: FitMode::Contain,
            background: BackgroundMode::Blur,
            source_start: 1.25,
            radius: 16,
            name: Some("Keyboard".into()),
        };
        save(tmp.path(), std::slice::from_ref(&placement)).unwrap();
        assert_eq!(load(tmp.path()).unwrap(), vec![placement]);
    }

    #[test]
    fn invalid_timeline_is_rejected_before_persisting() {
        let tmp = tempfile::tempdir().unwrap();
        let placement = BrollPlacement {
            id: "br-1".into(),
            file: "/tmp/shot.png".into(),
            start: 8.0,
            end: 7.0,
            mode: PlacementMode::Fullscreen,
            rect: None,
            fit: FitMode::Cover,
            background: BackgroundMode::Black,
            source_start: 0.0,
            radius: 0,
            name: None,
        };
        assert!(save(tmp.path(), &[placement]).is_err());
        assert!(!tmp.path().join("broll.json").exists());
    }

    #[test]
    fn update_changes_requested_fields_and_preserves_the_rest() {
        let mut placements = vec![BrollPlacement {
            id: "br-1".into(),
            file: "/tmp/old.png".into(),
            start: 3.0,
            end: 7.0,
            mode: PlacementMode::Pip,
            rect: None,
            fit: FitMode::Cover,
            background: BackgroundMode::Black,
            source_start: 0.0,
            radius: 0,
            name: Some("Old".into()),
        }];
        let changed = update(
            &mut placements,
            "br-1",
            PlacementPatch {
                file: Some("/tmp/new.mov".into()),
                end: Some(9.0),
                mode: Some(PlacementMode::Fullscreen),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(changed);
        assert_eq!(placements[0].file, PathBuf::from("/tmp/new.mov"));
        assert_eq!(placements[0].start, 3.0);
        assert_eq!(placements[0].end, 9.0);
        assert_eq!(placements[0].name.as_deref(), Some("Old"));
    }

    #[test]
    fn rect_parser_requires_four_nonnegative_integers() {
        assert_eq!(
            parse_rect("10,20,640,360").unwrap(),
            Rect {
                x: 10,
                y: 20,
                width: 640,
                height: 360
            }
        );
        assert!(parse_rect("10,20,640").is_err());
        assert!(parse_rect("-1,20,640,360").is_err());
    }
}
