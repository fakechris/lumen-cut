//! Per-shot framing — how each kept media segment is framed on the canvas.
//!
//! A "shot" is one kept interval of the source timeline (the complement of
//! the soft cuts). Framing entries are anchored to source-timeline seconds
//! and matched to a kept interval at export/preview time, so they survive
//! later cut edits as long as the segment's midpoint still lands inside the
//! entry's range. Absent entry = `full` (the source frame, unchanged).
//!
//! The treatment model (six presets + a unitless 0–100 size) is adapted from
//! pireel (AGPL-3.0), `packages/studio-engine/src/composition-core.ts`
//! (`ShotTreatment`, `treatScale`, `shotTransformVars`, `TREAT_SIZE_DEFAULT`).
//! https://github.com/fakechris/pireel

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// How a shot is framed within the export canvas.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ShotTreatment {
    /// Source frame as-is (default; entries with this treatment are not stored).
    Full,
    /// Zoom into the center of the frame.
    PunchIn,
    /// Shrink to the bottom-right corner.
    CornerBr,
    /// Shrink to the top-left corner.
    CornerTl,
    /// Shrink onto the left half.
    SplitL,
    /// Shrink onto the right half.
    SplitR,
}

/// Framing for one shot, anchored to source-timeline seconds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ShotFraming {
    pub id: String,
    pub start: f64,
    pub end: f64,
    pub treatment: ShotTreatment,
    /// Unitless 0–100 size (CapCut convention, from pireel): punch-in = zoom
    /// amount, corner = small-window size, split = occupied width. `None` =
    /// the treatment's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
}

/// Per-treatment default size (0–100), ported from pireel's
/// `TREAT_SIZE_DEFAULT`.
pub fn treat_size_default(treatment: ShotTreatment) -> f64 {
    match treatment {
        ShotTreatment::Full => 0.0,
        ShotTreatment::PunchIn => 18.0,
        ShotTreatment::CornerBr | ShotTreatment::CornerTl => 35.0,
        ShotTreatment::SplitL | ShotTreatment::SplitR => 50.0,
    }
}

/// Framing size 0–100 → frame scale, ported from pireel's `treatScale`:
/// punch-in 1.05–2.0, corner 0.2–0.6, split 0.3–0.7.
pub fn treat_scale(treatment: ShotTreatment, size: Option<f64>) -> f64 {
    let v = (size.unwrap_or_else(|| treat_size_default(treatment)) / 100.0).clamp(0.0, 1.0);
    match treatment {
        ShotTreatment::PunchIn => 1.05 + v * 0.95,
        ShotTreatment::CornerBr | ShotTreatment::CornerTl => 0.2 + v * 0.4,
        ShotTreatment::SplitL | ShotTreatment::SplitR => 0.3 + v * 0.4,
        ShotTreatment::Full => 1.0,
    }
}

impl ShotFraming {
    pub fn validate(&self) -> AppResult<()> {
        if self.id.trim().is_empty() {
            return Err(AppError::Schema("framing id cannot be empty".into()));
        }
        if !self.start.is_finite()
            || !self.end.is_finite()
            || self.start < 0.0
            || self.end <= self.start
        {
            return Err(AppError::Schema(format!(
                "framing {} has invalid timeline [{},{}]",
                self.id, self.start, self.end
            )));
        }
        if let Some(size) = self.size {
            if !size.is_finite() || !(0.0..=100.0).contains(&size) {
                return Err(AppError::Schema(format!(
                    "framing {} size must be between 0 and 100",
                    self.id
                )));
            }
        }
        Ok(())
    }
}

/// The entry governing the kept interval `[start, end)`: the entry containing
/// the interval's midpoint wins; otherwise the entry with the largest overlap
/// (entries can outlive the exact segment they were created on after further
/// cut edits).
pub fn framing_for_interval(
    framings: &[ShotFraming],
    start: f64,
    end: f64,
) -> Option<&ShotFraming> {
    let midpoint = (start + end) / 2.0;
    if let Some(entry) = framings
        .iter()
        .find(|entry| entry.start <= midpoint && midpoint < entry.end)
    {
        return Some(entry);
    }
    framings
        .iter()
        .filter_map(|entry| {
            let overlap = entry.end.min(end) - entry.start.max(start);
            (overlap > 0.0).then_some((entry, overlap))
        })
        .max_by(|left, right| {
            left.1
                .partial_cmp(&right.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(entry, _)| entry)
}

/// True when any entry implies real framing work (everything but `full`).
pub fn any_non_full(framings: &[ShotFraming]) -> bool {
    framings
        .iter()
        .any(|entry| entry.treatment != ShotTreatment::Full)
}

pub fn load(project_dir: &Path) -> AppResult<Vec<ShotFraming>> {
    let path = project_dir.join("framing.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let framings: Vec<ShotFraming> = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    for framing in &framings {
        framing.validate()?;
    }
    Ok(framings)
}

pub fn save(project_dir: &Path, framings: &[ShotFraming]) -> AppResult<()> {
    for framing in framings {
        framing.validate()?;
    }
    crate::data::storage::write_json(&project_dir.join("framing.json"), framings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framing(treatment: ShotTreatment, size: Option<f64>) -> ShotFraming {
        ShotFraming {
            id: "fr-1".into(),
            start: 2.0,
            end: 4.0,
            treatment,
            size,
        }
    }

    #[test]
    fn treat_scale_maps_each_treatment_range() {
        // Ported expectations from pireel's treatScale.
        assert!((treat_scale(ShotTreatment::PunchIn, None) - (1.05 + 0.18 * 0.95)).abs() < 1e-9);
        assert!((treat_scale(ShotTreatment::PunchIn, Some(0.0)) - 1.05).abs() < 1e-9);
        assert!((treat_scale(ShotTreatment::PunchIn, Some(100.0)) - 2.0).abs() < 1e-9);
        assert!((treat_scale(ShotTreatment::CornerBr, Some(0.0)) - 0.2).abs() < 1e-9);
        assert!((treat_scale(ShotTreatment::CornerTl, Some(100.0)) - 0.6).abs() < 1e-9);
        assert!((treat_scale(ShotTreatment::CornerBr, None) - 0.34).abs() < 1e-9);
        assert!((treat_scale(ShotTreatment::SplitL, Some(0.0)) - 0.3).abs() < 1e-9);
        assert!((treat_scale(ShotTreatment::SplitR, Some(100.0)) - 0.7).abs() < 1e-9);
        assert!((treat_scale(ShotTreatment::SplitL, None) - 0.5).abs() < 1e-9);
        assert!((treat_scale(ShotTreatment::Full, Some(80.0)) - 1.0).abs() < 1e-9);
        // Out-of-range sizes clamp instead of blowing up the scale.
        assert!((treat_scale(ShotTreatment::PunchIn, Some(250.0)) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn framing_round_trips_and_rejects_invalid_values() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &[framing(ShotTreatment::CornerBr, Some(35.0))]).unwrap();
        assert_eq!(
            load(dir.path()).unwrap(),
            vec![framing(ShotTreatment::CornerBr, Some(35.0))]
        );

        let mut invalid = framing(ShotTreatment::Full, None);
        invalid.end = invalid.start;
        assert!(invalid.validate().is_err());
        let mut invalid = framing(ShotTreatment::Full, Some(120.0));
        assert!(invalid.validate().is_err());
        invalid.size = Some(-1.0);
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn treatment_serializes_kebab_case() {
        let raw = serde_json::to_string(&framing(ShotTreatment::PunchIn, None)).unwrap();
        assert!(raw.contains("\"treatment\":\"punch-in\""));
        // `size` is omitted when unset so older readers stay compatible.
        assert!(!raw.contains("size"));
        let back: ShotFraming = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.treatment, ShotTreatment::PunchIn);
        assert_eq!(back.size, None);
    }

    #[test]
    fn framing_for_interval_prefers_midpoint_then_max_overlap() {
        let entries = vec![
            ShotFraming {
                id: "a".into(),
                start: 0.0,
                end: 5.0,
                treatment: ShotTreatment::PunchIn,
                size: None,
            },
            ShotFraming {
                id: "b".into(),
                start: 10.0,
                end: 20.0,
                treatment: ShotTreatment::SplitL,
                size: None,
            },
        ];
        assert_eq!(
            framing_for_interval(&entries, 1.0, 3.0).map(|e| e.id.as_str()),
            Some("a")
        );
        assert_eq!(
            framing_for_interval(&entries, 11.0, 19.0).map(|e| e.id.as_str()),
            Some("b")
        );
        // Midpoint outside every entry → largest overlap wins.
        assert_eq!(
            framing_for_interval(&entries, 8.0, 12.0).map(|e| e.id.as_str()),
            Some("b")
        );
        assert_eq!(framing_for_interval(&entries, 21.0, 25.0), None);
        assert!(any_non_full(&entries));
        assert!(!any_non_full(&[framing(ShotTreatment::Full, None)]));
    }
}
