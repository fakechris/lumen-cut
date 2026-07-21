//! FCPXML 1.10 export — cut-aware primary media, connected B-roll, and
//! subtitle title clips.

use std::fmt::Write;
use std::path::Path;

use crate::data::broll::{BrollPlacement, FitMode, PlacementMode};
use crate::data::doc::Doc;
use crate::data::soft_cut::Cut;
use crate::error::AppResult;

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn secs(s: f64) -> String {
    format!("{:.3}s", s.max(0.0))
}

/// Render `doc.json` to an FCP-XML (1.9) string. Each sentence becomes a
/// `<title>` on the main spine, offset/duration from its word timing.
pub fn to_fcpxml(doc: &Doc, width: u32, height: u32) -> String {
    to_fcpxml_with(doc, &[], width, height)
}

pub fn to_fcpxml_with(doc: &Doc, cuts: &[Cut], width: u32, height: u32) -> String {
    to_fcpxml_with_broll(doc, cuts, &[], width, height)
}

struct FcpBroll<'a> {
    placement: &'a BrollPlacement,
    resource: String,
    offset: f64,
    duration: f64,
}

pub fn to_fcpxml_with_broll(
    doc: &Doc,
    cuts: &[Cut],
    placements: &[BrollPlacement],
    width: u32,
    height: u32,
) -> String {
    let intervals = super::project::cut_intervals(doc, cuts);
    let timeline_duration: f64 = super::project::kept_intervals(doc, cuts)
        .iter()
        .map(|(start, end)| end - start)
        .sum();
    let broll: Vec<FcpBroll<'_>> = placements
        .iter()
        .enumerate()
        .filter_map(|(index, placement)| {
            let offset = super::project::retime(placement.start, &intervals);
            let end = super::project::retime(placement.end, &intervals);
            (end > offset).then(|| FcpBroll {
                placement,
                resource: format!("r{}", index + 2),
                offset,
                duration: end - offset,
            })
        })
        .collect();
    let mut out = String::new();
    let _ = writeln!(out, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    let _ = writeln!(out, "<!DOCTYPE fcpxml>");
    let _ = writeln!(out, "<fcpxml version=\"1.10\">");
    let _ = writeln!(out, "  <resources>");
    let _ = writeln!(
        out,
        "    <format id=\"r1\" frameDuration=\"1/30s\" width=\"{width}\" height=\"{height}\"/>"
    );
    let media_name = doc
        .media
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Source");
    if let Some(channels) = doc.media.channels.filter(|channels| *channels > 0) {
        let _ = writeln!(
            out,
            "    <asset id=\"rMain\" name=\"{}\" start=\"0s\" duration=\"{}\" hasVideo=\"1\" hasAudio=\"1\" format=\"r1\" audioSources=\"1\" audioChannels=\"{}\" audioRate=\"{}\">",
            xml_escape(media_name),
            secs(doc.media.duration_seconds),
            channels,
            doc.media.sample_rate.unwrap_or(48_000)
        );
    } else {
        let _ = writeln!(
            out,
            "    <asset id=\"rMain\" name=\"{}\" start=\"0s\" duration=\"{}\" hasVideo=\"1\" hasAudio=\"0\" format=\"r1\">",
            xml_escape(media_name),
            secs(doc.media.duration_seconds)
        );
    }
    let _ = writeln!(
        out,
        "      <media-rep kind=\"original-media\" src=\"{}\"/>",
        xml_escape(&file_url(&doc.media.path))
    );
    let _ = writeln!(out, "    </asset>");
    let _ = writeln!(
        out,
        "    <effect id=\"rTitle\" name=\"Basic Title\" uid=\".../Titles.localized/Bumper:Opener.localized/Basic Title.localized/Basic Title.moti\"/>"
    );
    for item in &broll {
        let asset_duration = item.placement.source_start + item.duration;
        let name = item
            .placement
            .name
            .as_deref()
            .or_else(|| {
                item.placement
                    .file
                    .file_name()
                    .and_then(|name| name.to_str())
            })
            .unwrap_or("B-roll");
        let _ = writeln!(
            out,
            "    <asset id=\"{}\" name=\"{}\" start=\"0s\" duration=\"{}\" hasVideo=\"1\" format=\"r1\">",
            item.resource,
            xml_escape(name),
            secs(asset_duration)
        );
        let _ = writeln!(
            out,
            "      <media-rep kind=\"original-media\" src=\"{}\"/>",
            xml_escape(&file_url(&item.placement.file))
        );
        let _ = writeln!(out, "    </asset>");
    }
    let _ = writeln!(out, "  </resources>");
    let _ = writeln!(out, "  <library>");
    let _ = writeln!(out, "    <event name=\"lumen-cut\">");
    let _ = writeln!(
        out,
        "      <project name=\"{}\">",
        xml_escape(&doc.meta.title)
    );
    let _ = writeln!(
        out,
        "        <sequence format=\"r1\" duration=\"{}\">",
        secs(timeline_duration)
    );
    let _ = writeln!(out, "          <spine>");
    let _ = writeln!(
        out,
        "            <gap name=\"lumen-cut timeline\" offset=\"0s\" start=\"0s\" duration=\"{}\">",
        secs(timeline_duration)
    );
    let _ = writeln!(out, "              <spine lane=\"0\" offset=\"0s\">");
    for (start, end) in super::project::kept_intervals(doc, cuts) {
        let offset = super::project::retime(start, &intervals);
        let _ = writeln!(
            out,
            "                <asset-clip ref=\"rMain\" offset=\"{}\" start=\"{}\" duration=\"{}\" name=\"{}\" srcEnable=\"all\"/>",
            secs(offset),
            secs(start),
            secs(end - start),
            xml_escape(media_name)
        );
    }
    let _ = writeln!(out, "              </spine>");
    for para in &doc.paragraphs {
        for sent in &para.sentences {
            if sent.words.is_empty() {
                continue;
            }
            let start = sent.words.first().unwrap().start;
            let end = sent.words.last().unwrap().end;
            if super::project::fully_cut(start, end, &intervals) {
                continue;
            }
            let start = super::project::retime(start, &intervals);
            let end = super::project::retime(end, &intervals);
            let dur = (end - start).max(0.0);
            let text = xml_escape(&sent.text);
            let _ = writeln!(
                out,
                "              <title ref=\"rTitle\" lane=\"2\" offset=\"{}\" duration=\"{}\" name=\"{text}\" role=\"subtitle.text\">",
                secs(start),
                secs(dur)
            );
            let _ = writeln!(out, "                <text>{text}</text>");
            let _ = writeln!(out, "              </title>");
        }
    }
    for item in &broll {
        write_broll_video(&mut out, item, width, height);
    }
    let _ = writeln!(out, "            </gap>");
    let _ = writeln!(out, "          </spine>");
    let _ = writeln!(out, "        </sequence>");
    let _ = writeln!(out, "      </project>");
    let _ = writeln!(out, "    </event>");
    let _ = writeln!(out, "  </library>");
    let _ = writeln!(out, "</fcpxml>");
    out
}

fn write_broll_video(out: &mut String, item: &FcpBroll<'_>, width: u32, height: u32) {
    let placement = item.placement;
    let name = placement
        .name
        .as_deref()
        .or_else(|| placement.file.file_name().and_then(|name| name.to_str()))
        .unwrap_or("B-roll");
    let _ = writeln!(
        out,
        "              <video ref=\"{}\" lane=\"1\" offset=\"{}\" start=\"{}\" duration=\"{}\" name=\"{}\" role=\"video.broll\">",
        item.resource,
        secs(item.offset),
        secs(placement.source_start),
        secs(item.duration),
        xml_escape(name)
    );
    let _ = writeln!(
        out,
        "                <note>lumen-cut.radius={}; background={:?}</note>",
        placement.radius, placement.background
    );
    let conform = match placement.fit {
        FitMode::Cover => "fill",
        FitMode::Contain => "fit",
    };
    let _ = writeln!(out, "                <adjust-conform type=\"{conform}\"/>");
    if let Some(rect) = placement.rect {
        let scale_x = rect.width as f64 / width.max(1) as f64 * 100.0;
        let scale_y = rect.height as f64 / height.max(1) as f64 * 100.0;
        let center_x = rect.x as f64 + rect.width as f64 / 2.0;
        let center_y = rect.y as f64 + rect.height as f64 / 2.0;
        let position_x = (center_x - width as f64 / 2.0) / height.max(1) as f64 * 100.0;
        let position_y = (height as f64 / 2.0 - center_y) / height.max(1) as f64 * 100.0;
        let _ = writeln!(
            out,
            "                <adjust-transform position=\"{position_x:.3} {position_y:.3}\" scale=\"{scale_x:.3} {scale_y:.3}\"/>"
        );
    } else if placement.mode == PlacementMode::Pip {
        let _ = writeln!(
            out,
            "                <adjust-transform position=\"53.333 35.000\" scale=\"32.000 32.000\"/>"
        );
    }
    let _ = writeln!(out, "              </video>");
}

fn file_url(path: &Path) -> String {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut output = String::from("file://");
    for byte in absolute.to_string_lossy().as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'/' | b'-' | b'_' | b'.' | b'~') {
            output.push(*byte as char);
        } else {
            let _ = write!(output, "%{byte:02X}");
        }
    }
    output
}

pub fn write_fcp(doc: &Doc, path: &Path, width: u32, height: u32) -> AppResult<()> {
    write_fcp_with(doc, &[], path, width, height)
}

pub fn write_fcp_with(
    doc: &Doc,
    cuts: &[Cut],
    path: &Path,
    width: u32,
    height: u32,
) -> AppResult<()> {
    write_fcp_with_broll(doc, cuts, &[], path, width, height)
}

pub fn write_fcp_with_broll(
    doc: &Doc,
    cuts: &[Cut],
    placements: &[BrollPlacement],
    path: &Path,
    width: u32,
    height: u32,
) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        path,
        to_fcpxml_with_broll(doc, cuts, placements, width, height),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::doc::*;
    use chrono::Utc;
    use std::path::PathBuf;

    fn doc() -> Doc {
        Doc {
            id: "p".into(),
            schema: 1,
            media: MediaRef {
                path: PathBuf::from("/tmp/x.mp4"),
                duration_seconds: 2.0,
                sample_rate: None,
                channels: None,
            },
            meta: Meta {
                title: "Demo".into(),
                description: String::new(),
                language: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            paragraphs: vec![Paragraph {
                id: 1,
                speaker: None,
                sentences: vec![Sentence {
                    id: "s1".into(),
                    text: "Hello <world>".into(),
                    words: vec![
                        Word {
                            id: "w0".into(),
                            text: "Hello".into(),
                            start: 0.0,
                            end: 0.5,
                        },
                        Word {
                            id: "w1".into(),
                            text: "world".into(),
                            start: 0.5,
                            end: 1.0,
                        },
                    ],
                }],
            }],
            translations: Default::default(),
        }
    }

    #[test]
    fn fcpxml_has_root_and_title() {
        let xml = to_fcpxml(&doc(), 1920, 1080);
        assert!(xml.contains("<fcpxml version=\"1.10\">"));
        assert!(xml.contains("<title ref=\"rTitle\""));
        assert!(xml.contains("offset=\"0.000s\""));
    }

    #[test]
    fn fcpxml_escapes_special_chars() {
        let xml = to_fcpxml(&doc(), 1920, 1080);
        assert!(xml.contains("&lt;world&gt;"));
        assert!(!xml.contains("<world>"));
    }

    #[test]
    fn fcpxml_drops_titles_fully_consumed_by_a_cut() {
        let cut = crate::data::Cut {
            id: "c1".into(),
            note: None,
            a_word: "w0".into(),
            b_word: "w1".into(),
            kind: crate::data::CutKind::Manual,
            duration: 1.0,
        };
        let xml = to_fcpxml_with(&doc(), &[cut], 1920, 1080);
        assert!(!xml.contains("<title ref="));
    }

    #[test]
    fn fcpxml_embeds_broll_as_editable_connected_video() {
        let placement = crate::data::broll::BrollPlacement {
            id: "br-1".into(),
            file: PathBuf::from("/tmp/keyboard closeup & detail.png"),
            start: 0.25,
            end: 0.75,
            mode: crate::data::broll::PlacementMode::Pip,
            rect: Some(crate::data::broll::Rect {
                x: 960,
                y: 54,
                width: 768,
                height: 432,
            }),
            fit: crate::data::broll::FitMode::Contain,
            background: crate::data::broll::BackgroundMode::Black,
            source_start: 1.5,
            radius: 20,
            name: Some("Keyboard & detail".into()),
        };
        let xml = to_fcpxml_with_broll(&doc(), &[], &[placement], 1920, 1080);
        assert!(xml.contains("<asset id=\"r2\""));
        assert!(xml.contains("src=\"file:///tmp/keyboard%20closeup%20%26%20detail.png\""));
        assert!(xml.contains(
            "<video ref=\"r2\" lane=\"1\" offset=\"0.250s\" start=\"1.500s\" duration=\"0.500s\""
        ));
        assert!(xml.contains("name=\"Keyboard &amp; detail\""));
        assert!(xml.contains("<adjust-conform type=\"fit\"/>"));
        assert!(xml.contains("<adjust-transform"));
        assert!(xml.contains("lumen-cut.radius=20"));
    }

    #[test]
    fn fcpxml_carries_cut_aware_primary_media_storyline() {
        let cut = crate::data::Cut {
            id: "c1".into(),
            note: None,
            a_word: "w0".into(),
            b_word: "w0".into(),
            kind: crate::data::CutKind::Manual,
            duration: 0.5,
        };
        let xml = to_fcpxml_with_broll(&doc(), &[cut], &[], 1920, 1080);
        assert!(xml.contains("<asset id=\"rMain\""));
        assert!(xml.contains("src=\"file:///tmp/x.mp4\""));
        assert!(xml.contains("<spine lane=\"0\" offset=\"0s\">"));
        assert!(xml.contains(
            "<asset-clip ref=\"rMain\" offset=\"0.000s\" start=\"0.500s\" duration=\"1.500s\""
        ));
    }
}
