use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TitleClip {
    pub id: String,
    pub text: String,
    pub start: f64,
    pub end: f64,
    #[serde(default = "default_x")]
    pub x: f64,
    #[serde(default = "default_y")]
    pub y: f64,
    #[serde(default = "default_font_size")]
    pub font_size: u32,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default = "default_background")]
    pub background: String,
    #[serde(default)]
    pub fade_in: f64,
    #[serde(default)]
    pub fade_out: f64,
}

const fn default_x() -> f64 {
    0.5
}

const fn default_y() -> f64 {
    0.18
}

const fn default_font_size() -> u32 {
    64
}

fn default_color() -> String {
    "#FFFFFF".into()
}

fn default_background() -> String {
    "#00000099".into()
}

fn valid_hex_color(value: &str) -> bool {
    matches!(value.len(), 7 | 9)
        && value.starts_with('#')
        && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

impl TitleClip {
    pub fn validate(&self) -> AppResult<()> {
        if self.id.trim().is_empty() {
            return Err(AppError::Schema("title id cannot be empty".into()));
        }
        if self.text.trim().is_empty() {
            return Err(AppError::Schema("title text cannot be empty".into()));
        }
        if !self.start.is_finite()
            || !self.end.is_finite()
            || self.start < 0.0
            || self.end <= self.start
        {
            return Err(AppError::Schema(format!(
                "title {} has invalid timeline [{},{}]",
                self.id, self.start, self.end
            )));
        }
        if !self.x.is_finite()
            || !self.y.is_finite()
            || !(0.0..=1.0).contains(&self.x)
            || !(0.0..=1.0).contains(&self.y)
        {
            return Err(AppError::Schema(format!(
                "title {} has a position outside the stage",
                self.id
            )));
        }
        if !(12..=240).contains(&self.font_size) {
            return Err(AppError::Schema(format!(
                "title {} font size must be between 12 and 240",
                self.id
            )));
        }
        if !valid_hex_color(&self.color) || !valid_hex_color(&self.background) {
            return Err(AppError::Schema(format!(
                "title {} color must use #RRGGBB or #RRGGBBAA",
                self.id
            )));
        }
        if !self.fade_in.is_finite()
            || !self.fade_out.is_finite()
            || self.fade_in < 0.0
            || self.fade_out < 0.0
            || self.fade_in + self.fade_out > self.end - self.start + 0.001
        {
            return Err(AppError::Schema(format!(
                "title {} fades must fit inside the title duration",
                self.id
            )));
        }
        Ok(())
    }
}

pub fn load(project_dir: &Path) -> AppResult<Vec<TitleClip>> {
    let path = project_dir.join("titles.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let titles: Vec<TitleClip> = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    for title in &titles {
        title.validate()?;
    }
    Ok(titles)
}

pub fn save(project_dir: &Path, titles: &[TitleClip]) -> AppResult<()> {
    for title in titles {
        title.validate()?;
    }
    crate::data::storage::write_json(&project_dir.join("titles.json"), titles)
}

pub fn ass_color(value: &str) -> String {
    let red = &value[1..3];
    let green = &value[3..5];
    let blue = &value[5..7];
    let alpha = value.get(7..9).unwrap_or("FF");
    let opacity = 255_u8.saturating_sub(u8::from_str_radix(alpha, 16).unwrap_or(255));
    format!("&H{opacity:02X}{blue}{green}{red}&")
}

pub fn ass_text(value: &str) -> String {
    value
        .trim()
        .replace('\\', r"\\")
        .replace('{', r"\{")
        .replace('}', r"\}")
        .replace('\n', r"\N")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Doc, MediaRef, Meta};
    use std::path::PathBuf;

    fn title() -> TitleClip {
        TitleClip {
            id: "title-1".into(),
            text: "Opening title".into(),
            start: 1.0,
            end: 3.0,
            x: 0.5,
            y: 0.2,
            font_size: 72,
            color: "#12ABEF".into(),
            background: "#00000099".into(),
            fade_in: 0.4,
            fade_out: 0.6,
        }
    }

    #[test]
    fn title_round_trips_and_rejects_invalid_stage_values() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &[title()]).unwrap();
        assert_eq!(load(dir.path()).unwrap(), vec![title()]);

        let mut invalid = title();
        invalid.x = 1.1;
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn ass_helpers_escape_text_and_convert_rgba() {
        assert_eq!(ass_text("A {title}\nB"), r"A \{title\}\NB");
        assert_eq!(ass_color("#12ABEF99"), "&H66EFAB12&");
    }

    #[test]
    fn title_is_present_in_burn_in_and_editable_timeline_exports() {
        let doc = Doc {
            id: "p".into(),
            schema: 1,
            media: MediaRef {
                path: PathBuf::from("/tmp/source.mp4"),
                duration_seconds: 10.0,
                sample_rate: None,
                channels: None,
            },
            meta: Meta {
                title: "Demo".into(),
                description: String::new(),
                language: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            paragraphs: Vec::new(),
            translations: Default::default(),
        };
        let title = title();

        let ass =
            crate::export::to_ass_with_titles(&doc, &[], std::slice::from_ref(&title), 1920, 1080);
        assert!(ass.contains(r"\pos(960,216)\fs72"));
        assert!(ass.contains(r"\fad(400,600)"));
        assert!(ass.contains("Opening title"));

        let fcp = crate::export::to_fcpxml_with_broll_titles(&doc, &[], &[], &[title], 1920, 1080);
        assert!(fcp.contains("role=\"titles.text\""));
        assert!(fcp.contains("lumen-cut.fontSize=72"));
        assert!(fcp.contains("fadeIn=0.400; fadeOut=0.600"));
    }
}
