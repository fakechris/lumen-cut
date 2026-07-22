//! Cut-aware video export. The picture and audio are trimmed/concatenated
//! before the already-retimed ASS captions are burned in.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::data::broll::{BackgroundMode, BrollPlacement, FitMode, PlacementMode};
use crate::data::{Cut, Doc};
use crate::error::{AppError, AppResult};
use crate::proc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderPurpose {
    Preview,
    Final,
}

#[derive(Debug, Clone)]
pub struct VideoRenderProgress {
    pub progress: u8,
    pub current_seconds: f64,
    pub total_seconds: f64,
    pub encoder: String,
}

pub type VideoRenderProgressCallback = Arc<dyn Fn(VideoRenderProgress) + Send + Sync>;

pub struct VideoRenderOptions {
    pub purpose: RenderPurpose,
    pub mode: Option<String>,
    pub on_progress: Option<VideoRenderProgressCallback>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoFilter {
    pub filter_complex: String,
    pub audio_map: Option<String>,
    pub broll_inputs: Vec<PathBuf>,
}

pub fn build_video_filter(doc: &Doc, cuts: &[Cut], ass: &Path) -> AppResult<VideoFilter> {
    build_video_filter_with_broll(doc, cuts, ass, &[])
}

pub fn build_video_filter_with_broll(
    doc: &Doc,
    cuts: &[Cut],
    ass: &Path,
    placements: &[BrollPlacement],
) -> AppResult<VideoFilter> {
    let ass = escape_filter_path(ass);
    let mut graph = String::new();
    let audio_map;
    if cuts.is_empty() {
        graph.push_str("[0:v]setpts=PTS-STARTPTS[vbase];");
        audio_map = Some("0:a:0?".into());
    } else {
        let kept = super::project::kept_intervals(doc, cuts);
        if kept.is_empty() {
            return Err(AppError::Schema(
                "video export removed the entire media timeline".into(),
            ));
        }
        let has_audio = doc.media.channels.is_some_and(|channels| channels > 0);
        for (index, (start, end)) in kept.iter().enumerate() {
            graph.push_str(&format!(
                "[0:v]trim=start={start:.6}:end={end:.6},setpts=PTS-STARTPTS[v{index}];"
            ));
            if has_audio {
                graph.push_str(&format!(
                    "[0:a]atrim=start={start:.6}:end={end:.6},asetpts=PTS-STARTPTS[a{index}];"
                ));
            }
        }
        for index in 0..kept.len() {
            graph.push_str(&format!("[v{index}]"));
            if has_audio {
                graph.push_str(&format!("[a{index}]"));
            }
        }
        if has_audio {
            graph.push_str(&format!("concat=n={}:v=1:a=1[vbase][acat];", kept.len()));
            audio_map = Some("[acat]".into());
        } else {
            graph.push_str(&format!("concat=n={}:v=1:a=0[vbase];", kept.len()));
            audio_map = None;
        }
    }

    let cut_intervals = super::project::cut_intervals(doc, cuts);
    let mut current = "vbase".to_string();
    let mut broll_inputs = Vec::new();
    for placement in placements {
        placement.validate()?;
        if cut_intervals
            .iter()
            .any(|(start, end)| *start <= placement.start && placement.end <= *end)
        {
            continue;
        }
        let display_start = super::project::retime(placement.start, &cut_intervals);
        let display_end = super::project::retime(placement.end, &cut_intervals);
        if display_end <= display_start {
            continue;
        }
        broll_inputs.push(placement.file.clone());
        let input = broll_inputs.len();
        let index = input - 1;
        let duration = display_end - display_start;
        let source_end = placement.source_start + duration;
        let raw = format!("brraw{index}");
        graph.push_str(&format!(
            "[{input}:v]trim=start={:.6}:end={source_end:.6},setpts=PTS-STARTPTS+{display_start:.6}/TB[{raw}];",
            placement.source_start
        ));

        let mut overlay_source;
        let overlay_base;
        if let Some(rect) = placement.rect {
            let scaled = format!("br{index}");
            match (placement.fit, placement.background) {
                (FitMode::Cover, _) => graph.push_str(&format!(
                    "[{raw}]scale={}:{}:force_original_aspect_ratio=increase,crop={}:{}[{scaled}];",
                    rect.width, rect.height, rect.width, rect.height
                )),
                (FitMode::Contain, BackgroundMode::Black) => graph.push_str(&format!(
                    "[{raw}]scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2:color=black[{scaled}];",
                    rect.width, rect.height, rect.width, rect.height
                )),
                (FitMode::Contain, BackgroundMode::Blur) => {
                    let background = format!("brbg{index}");
                    let foreground = format!("brfg{index}");
                    let backdrop = format!("brback{index}");
                    graph.push_str(&format!(
                        "[{raw}]split=2[{background}][{foreground}];\
                         [{background}]scale={}:{}:force_original_aspect_ratio=increase,crop={}:{},boxblur=20:2[{backdrop}];\
                         [{foreground}]scale={}:{}:force_original_aspect_ratio=decrease[brfront{index}];\
                         [{backdrop}][brfront{index}]overlay=(W-w)/2:(H-h)/2[{scaled}];",
                        rect.width,
                        rect.height,
                        rect.width,
                        rect.height,
                        rect.width,
                        rect.height
                    ));
                }
            }
            overlay_source = scaled;
            overlay_base = current.clone();
        } else {
            let scaled = format!("br{index}");
            let referenced = format!("vref{index}");
            let scale = match placement.mode {
                PlacementMode::Fullscreen => {
                    "w=main_w:h=main_h:force_original_aspect_ratio=increase"
                }
                PlacementMode::Pip => {
                    "w=main_w*0.32:h=ow/mdar:force_original_aspect_ratio=decrease"
                }
            };
            graph.push_str(&format!(
                "[{raw}][{current}]scale2ref={scale}[{scaled}][{referenced}];"
            ));
            overlay_source = scaled;
            overlay_base = referenced;
        }

        if placement.radius > 0 {
            let rounded = format!("brround{index}");
            let radius = format!("min({},min(W,H)/2)", placement.radius);
            graph.push_str(&format!(
                "[{overlay_source}]format=rgba,\
                 geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':\
                 a='if(gt(abs(W/2-X),W/2-{radius})*gt(abs(H/2-Y),H/2-{radius}),\
                 if(lte(hypot({radius}-(W/2-abs(W/2-X)),{radius}-(H/2-abs(H/2-Y))),{radius}),255,0),255)'\
                 [{rounded}];"
            ));
            overlay_source = rounded;
        }

        let next = format!("vbr{index}");
        let (x, y) = match (placement.mode, placement.rect) {
            (_, Some(rect)) => (rect.x.to_string(), rect.y.to_string()),
            (PlacementMode::Fullscreen, None) => {
                ("(main_w-overlay_w)/2".into(), "(main_h-overlay_h)/2".into())
            }
            (PlacementMode::Pip, None) => {
                ("main_w-overlay_w-main_w*0.04".into(), "main_h*0.06".into())
            }
        };
        graph.push_str(&format!(
            "[{overlay_base}][{overlay_source}]overlay=x={x}:y={y}:eof_action=pass:enable='between(t,{display_start:.6},{display_end:.6})'[{next}];"
        ));
        current = next;
    }
    graph.push_str(&format!("[{current}]ass=filename='{ass}'[vout]"));

    Ok(VideoFilter {
        filter_complex: graph,
        audio_map,
        broll_inputs,
    })
}

pub async fn render_video(doc: &Doc, cuts: &[Cut], ass: &Path, output: &Path) -> AppResult<()> {
    render_video_with_broll_progress(doc, cuts, ass, output, &[], RenderPurpose::Final, None).await
}

pub async fn render_video_with_broll(
    doc: &Doc,
    cuts: &[Cut],
    ass: &Path,
    output: &Path,
    placements: &[BrollPlacement],
) -> AppResult<()> {
    render_video_with_broll_progress(
        doc,
        cuts,
        ass,
        output,
        placements,
        RenderPurpose::Final,
        None,
    )
    .await
}

pub async fn render_video_with_broll_progress(
    doc: &Doc,
    cuts: &[Cut],
    ass: &Path,
    output: &Path,
    placements: &[BrollPlacement],
    purpose: RenderPurpose,
    on_progress: Option<VideoRenderProgressCallback>,
) -> AppResult<()> {
    render_video_with_broll_options(
        doc,
        cuts,
        ass,
        output,
        placements,
        VideoRenderOptions {
            purpose,
            mode: None,
            on_progress,
        },
    )
    .await
}

pub async fn render_video_with_broll_options(
    doc: &Doc,
    cuts: &[Cut],
    ass: &Path,
    output: &Path,
    placements: &[BrollPlacement],
    options: VideoRenderOptions,
) -> AppResult<()> {
    let VideoRenderOptions {
        purpose,
        mode,
        on_progress,
    } = options;
    for placement in placements {
        if !placement.file.exists() {
            return Err(AppError::ProjectNotFound(placement.file.clone()));
        }
    }
    let filter = build_video_filter_with_broll(doc, cuts, ass, placements)?;
    let mut args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostdin".into(),
        "-y".into(),
        "-progress".into(),
        "pipe:2".into(),
        "-nostats".into(),
        "-i".into(),
        doc.media.path.display().to_string(),
    ];
    for input in &filter.broll_inputs {
        if is_still_image(input) {
            args.extend(["-loop".into(), "1".into()]);
        } else {
            args.extend(["-stream_loop".into(), "-1".into()]);
        }
        args.extend(["-i".into(), input.display().to_string()]);
    }
    args.extend([
        "-filter_complex".into(),
        filter.filter_complex,
        "-map".into(),
        "[vout]".into(),
    ]);
    if let Some(audio_map) = &filter.audio_map {
        args.extend([
            "-map".into(),
            audio_map.clone(),
            "-c:a".into(),
            "aac".into(),
        ]);
    }
    let output_duration: f64 = super::project::kept_intervals(doc, cuts)
        .iter()
        .map(|(start, end)| end - start)
        .sum();
    let encoder = encoder_for_mode(mode.as_deref())?;
    args.extend(encoder_args(&encoder, purpose));
    args.extend([
        "-movflags".into(),
        "+faststart".into(),
        "-t".into(),
        format!("{output_duration:.6}"),
        output.display().to_string(),
    ]);
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    if let Some(callback) = &on_progress {
        callback(VideoRenderProgress {
            progress: 0,
            current_seconds: 0.0,
            total_seconds: output_duration,
            encoder: encoder.clone(),
        });
    }
    let progress_callback = on_progress.clone();
    let callback_encoder = encoder.clone();
    let _ = proc::run_with_progress(
        "ffmpeg",
        &arg_refs,
        Arc::new(move |line| {
            let Some(current_seconds) = ffmpeg_out_time_seconds(&line) else {
                return;
            };
            let progress = if output_duration > 0.0 {
                ((current_seconds / output_duration) * 100.0)
                    .floor()
                    .clamp(0.0, 99.0) as u8
            } else {
                0
            };
            if let Some(callback) = &progress_callback {
                callback(VideoRenderProgress {
                    progress,
                    current_seconds: current_seconds.min(output_duration),
                    total_seconds: output_duration,
                    encoder: callback_encoder.clone(),
                });
            }
        }),
    )
    .await?;
    if let Some(callback) = on_progress {
        callback(VideoRenderProgress {
            progress: 100,
            current_seconds: output_duration,
            total_seconds: output_duration,
            encoder,
        });
    }
    Ok(())
}

fn selected_encoder() -> String {
    if let Ok(configured) = std::env::var("LUMEN_CUT_VIDEO_ENCODER") {
        if matches!(configured.as_str(), "libx264" | "h264_videotoolbox") {
            return configured;
        }
    }
    if cfg!(target_os = "macos") {
        "h264_videotoolbox".into()
    } else {
        "libx264".into()
    }
}

pub fn encoder_for_mode(mode: Option<&str>) -> AppResult<String> {
    match mode.unwrap_or("auto") {
        "auto" => Ok(selected_encoder()),
        "quality" => Ok("libx264".into()),
        "fast" if cfg!(target_os = "macos") => Ok("h264_videotoolbox".into()),
        "fast" => Ok("libx264".into()),
        other => Err(AppError::Schema(format!(
            "unknown video export mode: {other}"
        ))),
    }
}

fn encoder_args(encoder: &str, purpose: RenderPurpose) -> Vec<String> {
    if encoder == "h264_videotoolbox" {
        let quality = if purpose == RenderPurpose::Preview {
            "55"
        } else {
            "65"
        };
        let mut args = vec![
            "-c:v".into(),
            encoder.into(),
            "-pix_fmt".into(),
            "yuv420p".into(),
            "-profile:v".into(),
            "high".into(),
            "-q:v".into(),
            quality.into(),
        ];
        if purpose == RenderPurpose::Preview {
            args.extend([
                "-realtime".into(),
                "1".into(),
                "-prio_speed".into(),
                "1".into(),
            ]);
        }
        args
    } else {
        let (preset, crf) = if purpose == RenderPurpose::Preview {
            ("veryfast", "23")
        } else {
            ("medium", "18")
        };
        vec![
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            preset.into(),
            "-crf".into(),
            crf.into(),
        ]
    }
}

fn ffmpeg_out_time_seconds(line: &str) -> Option<f64> {
    line.strip_prefix("out_time_us=")?
        .parse::<f64>()
        .ok()
        .map(|microseconds| microseconds / 1_000_000.0)
}

fn is_still_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp"
            )
        })
}

fn escape_filter_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace(':', "\\:")
        .replace('\'', "\\'")
        .replace(',', "\\,")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{CutKind, MediaRef, Meta, Paragraph, Sentence, Word};

    fn doc() -> Doc {
        Doc {
            id: "demo".into(),
            schema: 1,
            media: MediaRef {
                path: "/tmp/in.mp4".into(),
                duration_seconds: 6.0,
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
                    text: "one two three".into(),
                    words: vec![
                        Word {
                            id: "w0".into(),
                            text: "one".into(),
                            start: 0.0,
                            end: 1.0,
                        },
                        Word {
                            id: "w1".into(),
                            text: "two".into(),
                            start: 1.0,
                            end: 3.0,
                        },
                        Word {
                            id: "w2".into(),
                            text: "three".into(),
                            start: 3.0,
                            end: 5.0,
                        },
                    ],
                }],
            }],
            translations: Default::default(),
        }
    }

    #[test]
    fn cut_filter_trims_and_concatenates_audio_and_video() {
        let cut = Cut {
            id: "c1".into(),
            note: None,
            a_word: "w1".into(),
            b_word: "w1".into(),
            kind: CutKind::Manual,
            duration: 2.0,
        };
        let plan = build_video_filter(&doc(), &[cut], Path::new("/tmp/a b.ass")).unwrap();
        assert!(plan
            .filter_complex
            .contains("trim=start=0.000000:end=1.000000"));
        assert!(plan
            .filter_complex
            .contains("atrim=start=3.000000:end=6.000000"));
        assert!(plan.filter_complex.contains("concat=n=2:v=1:a=1"));
        assert_eq!(plan.audio_map.as_deref(), Some("[acat]"));
    }

    #[test]
    fn no_cut_filter_only_burns_subtitles() {
        let plan = build_video_filter(&doc(), &[], Path::new("/tmp/a.ass")).unwrap();
        assert!(!plan.filter_complex.contains("trim="));
        assert!(plan.filter_complex.contains("ass=filename="));
        assert_eq!(plan.audio_map.as_deref(), Some("0:a:0?"));
    }

    #[test]
    fn cut_filter_supports_video_without_audio_stream() {
        let mut video_only = doc();
        video_only.media.channels = None;
        video_only.media.sample_rate = None;
        let cut = Cut {
            id: "c1".into(),
            note: None,
            a_word: "w1".into(),
            b_word: "w1".into(),
            kind: CutKind::Manual,
            duration: 2.0,
        };
        let plan = build_video_filter(&video_only, &[cut], Path::new("/tmp/a.ass")).unwrap();
        assert!(!plan.filter_complex.contains("[0:a]"));
        assert!(plan.filter_complex.contains("concat=n=2:v=1:a=0"));
        assert_eq!(plan.audio_map, None);
    }

    #[test]
    fn accepted_broll_is_composited_as_an_extra_video_input() {
        let placement = crate::data::broll::BrollPlacement {
            id: "br-1".into(),
            file: "/tmp/shot.png".into(),
            start: 2.0,
            end: 4.0,
            mode: crate::data::broll::PlacementMode::Fullscreen,
            rect: None,
            fit: crate::data::broll::FitMode::Cover,
            background: crate::data::broll::BackgroundMode::Black,
            source_start: 0.0,
            radius: 0,
            name: None,
        };
        let plan =
            build_video_filter_with_broll(&doc(), &[], Path::new("/tmp/a.ass"), &[placement])
                .unwrap();
        assert_eq!(
            plan.broll_inputs,
            vec![std::path::PathBuf::from("/tmp/shot.png")]
        );
        assert!(plan.filter_complex.contains("[1:v]"));
        assert!(plan.filter_complex.contains("overlay="));
        assert!(plan.filter_complex.contains("between(t,2.000000,4.000000)"));
    }

    #[test]
    fn contained_broll_honors_blurred_background() {
        let placement = crate::data::broll::BrollPlacement {
            id: "br-1".into(),
            file: "/tmp/portrait.png".into(),
            start: 2.0,
            end: 4.0,
            mode: crate::data::broll::PlacementMode::Pip,
            rect: Some(crate::data::broll::Rect {
                x: 20,
                y: 30,
                width: 640,
                height: 360,
            }),
            fit: crate::data::broll::FitMode::Contain,
            background: crate::data::broll::BackgroundMode::Blur,
            source_start: 0.0,
            radius: 0,
            name: None,
        };
        let plan =
            build_video_filter_with_broll(&doc(), &[], Path::new("/tmp/a.ass"), &[placement])
                .unwrap();
        assert!(plan.filter_complex.contains("split=2"));
        assert!(plan.filter_complex.contains("boxblur="));
        assert!(plan.filter_complex.contains("overlay=(W-w)/2:(H-h)/2"));
    }

    #[test]
    fn rounded_broll_applies_alpha_mask_after_scaling() {
        let placement = crate::data::broll::BrollPlacement {
            id: "br-round".into(),
            file: "/tmp/portrait.png".into(),
            start: 2.0,
            end: 4.0,
            mode: crate::data::broll::PlacementMode::Pip,
            rect: Some(crate::data::broll::Rect {
                x: 20,
                y: 30,
                width: 640,
                height: 360,
            }),
            fit: crate::data::broll::FitMode::Cover,
            background: crate::data::broll::BackgroundMode::Black,
            source_start: 0.0,
            radius: 24,
            name: None,
        };
        let plan =
            build_video_filter_with_broll(&doc(), &[], Path::new("/tmp/a.ass"), &[placement])
                .unwrap();
        assert!(plan.filter_complex.contains("format=rgba,geq="));
        assert!(plan.filter_complex.contains("hypot("));
        assert!(plan.filter_complex.contains("[brround0]"));
        assert!(plan.filter_complex.contains("[vbase][brround0]overlay="));
    }

    #[test]
    fn parses_ffmpeg_machine_progress_without_fake_percentages() {
        assert_eq!(ffmpeg_out_time_seconds("out_time_us=2500000"), Some(2.5));
        assert_eq!(ffmpeg_out_time_seconds("progress=continue"), None);
        assert_eq!(ffmpeg_out_time_seconds("out_time_us=N/A"), None);
    }

    #[test]
    fn videotoolbox_profiles_separate_preview_speed_from_final_quality() {
        let preview = encoder_args("h264_videotoolbox", RenderPurpose::Preview);
        let final_render = encoder_args("h264_videotoolbox", RenderPurpose::Final);
        assert!(preview.windows(2).any(|pair| pair == ["-realtime", "1"]));
        assert!(preview.windows(2).any(|pair| pair == ["-q:v", "55"]));
        assert!(final_render.windows(2).any(|pair| pair == ["-q:v", "65"]));
        assert!(!final_render
            .windows(2)
            .any(|pair| pair == ["-realtime", "1"]));
    }

    #[test]
    fn export_mode_makes_the_speed_quality_tradeoff_explicit() {
        assert_eq!(encoder_for_mode(Some("quality")).unwrap(), "libx264");
        let fast = encoder_for_mode(Some("fast")).unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(fast, "h264_videotoolbox");
        } else {
            assert_eq!(fast, "libx264");
        }
        assert!(encoder_for_mode(Some("mystery")).is_err());
    }
}
