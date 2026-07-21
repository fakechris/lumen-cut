//! Subtitle style model for ASS `[V4+ Styles]` rendering.
//!
//! The desktop editor persists this structure in `style.json` and the ASS
//! exporter converts it to the corresponding style line.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubStyle {
    pub name: String,
    pub fontname: String,
    pub fontsize: u32,
    /// Primary colour in ASS `&H00BBGGRR` form.
    pub primary_colour: String,
    pub outline_colour: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike_out: bool,
    /// 1–9 numpad alignment.
    pub alignment: u32,
    pub outline: u32,
    pub shadow: u32,
    pub margin_l: u32,
    pub margin_r: u32,
    pub margin_v: u32,
}

impl Default for SubStyle {
    fn default() -> Self {
        Self {
            name: "Default".into(),
            fontname: "Arial".into(),
            fontsize: 52,
            primary_colour: "&H00FFFFFF".into(),
            outline_colour: "&H00000000".into(),
            bold: false,
            italic: false,
            underline: false,
            strike_out: false,
            alignment: 2,
            outline: 2,
            shadow: 2,
            margin_l: 40,
            margin_r: 40,
            margin_v: 80,
        }
    }
}

impl SubStyle {
    /// Render the ASS `[V4+ Styles]` `Style:` line.
    pub fn ass_style_line(&self) -> String {
        let b = if self.bold { -1 } else { 0 };
        let i = if self.italic { -1 } else { 0 };
        let u = if self.underline { -1 } else { 0 };
        let so = if self.strike_out { -1 } else { 0 };
        format!(
            "Style: {name},{font},{size},{pri},&H000000FF,{ol},&H00000000,{b},{i},{u},{so},100,100,0,0,1,{outline},{shadow},{align},{ml},{mr},{mv},1",
            name = self.name,
            font = self.fontname,
            size = self.fontsize,
            pri = self.primary_colour,
            ol = self.outline_colour,
            b = b,
            i = i,
            u = u,
            so = so,
            outline = self.outline,
            shadow = self.shadow,
            align = self.alignment,
            ml = self.margin_l,
            mr = self.margin_r,
            mv = self.margin_v,
        )
    }

    /// Load `<dir>/style.json` if present, else the default.
    pub fn load_or_default(dir: &std::path::Path) -> Self {
        std::fs::read_to_string(dir.join("style.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_renders_valid_ass_style_line() {
        let line = SubStyle::default().ass_style_line();
        assert!(line.starts_with("Style: Default,Arial,52,"));
        // 23 comma-separated fields in a V4+ Style line.
        assert_eq!(line.matches(',').count(), 22);
    }

    #[test]
    fn bold_italic_flip_signatures() {
        let s = SubStyle {
            bold: true,
            italic: true,
            ..SubStyle::default()
        };
        let line = s.ass_style_line();
        // signatures are -1 when on (positions 8,9 after "Style: name,font,...")
        assert!(line.contains(",-1,-1,"));
    }

    #[test]
    fn round_trip_serde() {
        let s = SubStyle::default();
        let json = serde_json::to_string(&s).unwrap();
        let back: SubStyle = serde_json::from_str(&json).unwrap();
        assert_eq!(back.fontsize, 52);
        assert!(json.contains("\"fontsize\""));
    }
}
