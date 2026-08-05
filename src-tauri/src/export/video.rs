//! Cut-aware video export. The picture and audio are trimmed/concatenated
//! before the already-retimed ASS captions are burned in.
//!
//! When only soft (or no) captions change and the timeline is otherwise intact,
//! export uses stream-copy remux instead of a full re-encode.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use tracing::info;

use crate::data::audio_mix::AudioMix;
use crate::data::broll::{BackgroundMode, BrollPlacement, FitMode, PlacementMode, Rect};
use crate::data::export_settings::{
    ExportAudioCodec, ExportCanvasFit, ExportEncodingSpeed, ExportVideoCodec, VideoExportSettings,
};
use crate::data::framing::{
    any_non_full, framing_for_interval, treat_scale, ShotFraming, ShotTreatment,
};
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
    pub audio_mix: AudioMix,
    pub settings: Option<VideoExportSettings>,
    pub soft_subtitle: Option<PathBuf>,
    pub include_ass: bool,
    /// Per-shot framing entries (empty = every shot rendered `full`).
    pub framings: Vec<ShotFraming>,
    /// WYSIWYG caption overlay: ffconcat timeline of frontend-rendered PNG
    /// caption states (app exports). Burns the exact monitor picture instead
    /// of ASS captions; titles still burn via ASS on top.
    pub caption_overlay: Option<PathBuf>,
}

/// How the final video will be produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportRenderPath {
    /// `-c:v copy -c:a copy` remux, optionally mux soft captions.
    StreamCopy,
    /// Full filter graph + re-encode.
    Reencode,
}

/// Whether this export can stream-copy the source A/V (only remux + soft captions).
pub fn can_stream_copy_export(
    settings: &VideoExportSettings,
    cuts: &[Cut],
    placements: &[BrollPlacement],
    audio_mix: &AudioMix,
    include_ass: bool,
    framings: &[ShotFraming],
) -> bool {
    settings.allows_stream_copy()
        && cuts.is_empty()
        && placements.is_empty()
        && audio_mix.is_passthrough()
        && !include_ass
        && !any_non_full(framings)
}

pub fn export_render_path(
    settings: &VideoExportSettings,
    cuts: &[Cut],
    placements: &[BrollPlacement],
    audio_mix: &AudioMix,
    include_ass: bool,
    framings: &[ShotFraming],
) -> ExportRenderPath {
    if can_stream_copy_export(settings, cuts, placements, audio_mix, include_ass, framings) {
        ExportRenderPath::StreamCopy
    } else {
        ExportRenderPath::Reencode
    }
}

/// Human-readable reason when re-encode is required (for preflight/UI).
pub fn reencode_reason(
    settings: &VideoExportSettings,
    cuts: &[Cut],
    placements: &[BrollPlacement],
    audio_mix: &AudioMix,
    include_ass: bool,
    framings: &[ShotFraming],
) -> String {
    if can_stream_copy_export(settings, cuts, placements, audio_mix, include_ass, framings) {
        return "stream-copy remux".into();
    }
    let mut reasons = Vec::new();
    if matches!(
        settings.subtitle_mode,
        crate::data::export_settings::ExportSubtitleMode::Burn
    ) {
        reasons.push("burned-in captions");
    }
    if include_ass {
        reasons.push("titles or graphics burned into picture");
    }
    if !cuts.is_empty() {
        reasons.push("soft cuts on the timeline");
    }
    if any_non_full(framings) {
        reasons.push("per-shot framing");
    }
    if !placements.is_empty() {
        reasons.push("B-roll overlays");
    }
    if !audio_mix.is_passthrough() {
        reasons.push("audio mix / music / enhance");
    }
    if settings.resolution != crate::data::export_settings::ExportResolution::Source
        || settings.aspect_ratio != crate::data::export_settings::ExportAspectRatio::Source
    {
        reasons.push("canvas resize or aspect change");
    }
    if settings.video_codec == ExportVideoCodec::Prores {
        reasons.push("ProRes master encode");
    }
    if settings.video_codec == ExportVideoCodec::Hevc {
        reasons.push("HEVC re-encode");
    }
    if settings.audio_codec == ExportAudioCodec::Pcm {
        reasons.push("PCM audio re-encode");
    }
    if reasons.is_empty() {
        reasons.push("delivery settings require re-encode");
    }
    reasons.join(", ")
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoFilter {
    pub filter_complex: String,
    pub audio_map: Option<String>,
    pub broll_inputs: Vec<PathBuf>,
    pub music_inputs: Vec<PathBuf>,
    /// ffconcat caption-state timeline to add as an ffmpeg input (overlay).
    pub caption_concat: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Default)]
struct VideoCanvas {
    frame_size: Option<(u32, u32)>,
    output_dimensions: Option<(u32, u32)>,
    fit: ExportCanvasFit,
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
    build_video_filter_with_broll_audio(doc, cuts, ass, placements, &AudioMix::default())
}

pub fn build_video_filter_with_broll_audio(
    doc: &Doc,
    cuts: &[Cut],
    ass: &Path,
    placements: &[BrollPlacement],
    audio_mix: &AudioMix,
) -> AppResult<VideoFilter> {
    build_video_filter_inner(
        doc,
        cuts,
        Some(ass),
        placements,
        audio_mix,
        &[],
        VideoCanvas::default(),
        None,
    )
}

#[allow(clippy::too_many_arguments)] // One cohesive filter-graph snapshot; grouping would only rename the bag.
fn build_video_filter_inner(
    doc: &Doc,
    cuts: &[Cut],
    ass: Option<&Path>,
    placements: &[BrollPlacement],
    audio_mix: &AudioMix,
    framings: &[ShotFraming],
    canvas: VideoCanvas,
    caption_overlay: Option<&Path>,
) -> AppResult<VideoFilter> {
    let mut graph = String::new();
    let kept = super::project::kept_intervals(doc, cuts);
    let output_duration: f64 = kept.iter().map(|(start, end)| end - start).sum();
    audio_mix.validate(output_duration)?;
    // Per-shot framing, resolved onto the kept segments. `full` (or no entry)
    // renders the segment unchanged.
    let shot_framings: Vec<Option<&ShotFraming>> = kept
        .iter()
        .map(|(start, end)| {
            framing_for_interval(framings, *start, *end)
                .filter(|framing| framing.treatment != ShotTreatment::Full)
        })
        .collect();
    let framing_active = shot_framings.iter().any(Option::is_some);
    // Framing is computed in canvas pixels, so a framed export must know the
    // canvas size (always probed on the render path).
    let framing_dims = canvas.output_dimensions.or(canvas.frame_size);
    if framing_active && framing_dims.is_none() {
        return Err(AppError::Schema(
            "per-shot framing requires known canvas dimensions".into(),
        ));
    }
    let mut audio_map;
    let mut dialogue_source = None;
    // When any shot is framed, the canvas fit moves into each segment chain so
    // the framing can position the fitted frame; the post-concat canvas step
    // is skipped.
    let segment_canvas = |chain: &mut String| {
        if let Some((width, height)) = framing_dims {
            if !chain.is_empty() {
                chain.push(',');
            }
            chain.push_str(&canvas_fit_filter(canvas, width, height));
        }
    };
    if cuts.is_empty() {
        let mut chain = String::new();
        if framing_active {
            segment_canvas(&mut chain);
            if let Some(framing) = shot_framings.first().copied().flatten() {
                if let Some(filters) = framing_filter_chain(framing, framing_dims.unwrap()) {
                    chain.push(',');
                    chain.push_str(&filters);
                }
            }
            graph.push_str(&format!("[0:v]setpts=PTS-STARTPTS,{chain}[vbase];"));
        } else {
            graph.push_str("[0:v]setpts=PTS-STARTPTS[vbase];");
        }
        audio_map = Some("0:a:0?".into());
        if doc.media.channels.is_some_and(|channels| channels > 0) {
            dialogue_source = Some("0:a".to_string());
        }
    } else {
        if kept.is_empty() {
            return Err(AppError::Schema(
                "video export removed the entire media timeline".into(),
            ));
        }
        let has_audio = doc.media.channels.is_some_and(|channels| channels > 0);
        for (index, (start, end)) in kept.iter().enumerate() {
            let mut chain = String::new();
            if framing_active {
                segment_canvas(&mut chain);
                if let Some(framing) = shot_framings[index] {
                    if let Some(filters) = framing_filter_chain(framing, framing_dims.unwrap()) {
                        chain.push(',');
                        chain.push_str(&filters);
                    }
                }
            }
            let separator = if chain.is_empty() { "" } else { "," };
            graph.push_str(&format!(
                "[0:v]trim=start={start:.6}:end={end:.6},setpts=PTS-STARTPTS{separator}{chain}[v{index}];"
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
            dialogue_source = Some("acat".to_string());
        } else {
            graph.push_str(&format!("concat=n={}:v=1:a=0[vbase];", kept.len()));
            audio_map = None;
        }
    }
    if audio_mix != &AudioMix::default() {
        if let Some(source) = dialogue_source.as_deref() {
            let gain = if audio_mix.muted {
                0.0
            } else {
                audio_mix.volume
            };
            let mut filters = Vec::new();
            if !audio_mix.muted && audio_mix.voice_enhance {
                filters.extend([
                    "highpass=f=80".to_string(),
                    "afftdn=nr=8:nf=-45:tn=1".to_string(),
                    "acompressor=threshold=0.125:ratio=3:attack=20:release=250:makeup=1.5"
                        .to_string(),
                ]);
            }
            if !audio_mix.muted && audio_mix.normalize_loudness {
                filters.push(format!(
                    "loudnorm=I={:.1}:LRA=11:TP=-1.5",
                    audio_mix.loudness_target
                ));
                filters.push("aresample=48000".to_string());
            }
            filters.push(format!("volume={gain:.4}"));
            if audio_mix.fade_in > 0.0 {
                filters.push(format!("afade=t=in:st=0:d={:.6}", audio_mix.fade_in));
            }
            if audio_mix.fade_out > 0.0 {
                filters.push(format!(
                    "afade=t=out:st={:.6}:d={:.6}",
                    (output_duration - audio_mix.fade_out).max(0.0),
                    audio_mix.fade_out
                ));
            }
            graph.push_str(&format!("[{source}]{}[amix];", filters.join(",")));
            audio_map = Some("[amix]".into());
            dialogue_source = Some("amix".to_string());
        }
    }

    let cut_intervals = super::project::cut_intervals(doc, cuts);
    let mut current = if framing_active {
        // The canvas fit already ran inside each segment chain.
        "vbase".to_string()
    } else if let Some((width, height)) = canvas.output_dimensions {
        match canvas.fit {
            ExportCanvasFit::Contain => graph.push_str(&format!(
                "[vbase]scale=w={width}:h={height}:force_original_aspect_ratio=decrease:force_divisible_by=2:reset_sar=1,\
                 pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black[vcanvas];"
            )),
            ExportCanvasFit::Cover => graph.push_str(&format!(
                "[vbase]scale=w={width}:h={height}:force_original_aspect_ratio=increase:force_divisible_by=2:reset_sar=1,\
                 crop={width}:{height}:(iw-ow)/2:(ih-oh)/2[vcanvas];"
            )),
        }
        "vcanvas".to_string()
    } else {
        "vbase".to_string()
    };
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
            let rect = scale_design_rect(rect, canvas.frame_size);
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

        let radius = scale_design_radius(placement.radius, canvas.frame_size);
        if radius > 0 {
            let rounded = format!("brround{index}");
            let radius = format!("min({radius},min(W,H)/2)");
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
            (_, Some(rect)) => {
                let rect = scale_design_rect(rect, canvas.frame_size);
                (rect.x.to_string(), rect.y.to_string())
            }
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
    let music_inputs = audio_mix
        .music
        .iter()
        .map(|track| track.path.clone())
        .collect::<Vec<_>>();
    let mut music_labels = Vec::new();
    for (music_index, track) in audio_mix.music.iter().enumerate() {
        let input = 1 + broll_inputs.len() + music_index;
        let track_duration = track.end - track.start;
        let mut filters = vec![
            format!(
                "atrim=start={:.6}:duration={track_duration:.6}",
                track.source_start
            ),
            format!("asetpts=PTS-STARTPTS+{:.6}/TB", track.start),
            format!("volume={:.4}", track.volume),
        ];
        if track.fade_in > 0.0 {
            filters.push(format!("afade=t=in:st=0:d={:.6}", track.fade_in));
        }
        if track.fade_out > 0.0 {
            filters.push(format!(
                "afade=t=out:st={:.6}:d={:.6}",
                (track_duration - track.fade_out).max(0.0),
                track.fade_out
            ));
        }
        let label = format!("music{music_index}");
        graph.push_str(&format!("[{input}:a]{}[{label}];", filters.join(",")));
        music_labels.push(label);
    }
    if !music_labels.is_empty() {
        if let Some(source) = dialogue_source {
            let ducking = audio_mix
                .music
                .iter()
                .enumerate()
                .filter_map(|(index, track)| track.ducking.then_some(index))
                .collect::<Vec<_>>();
            let dialogue_label = if ducking.is_empty() {
                source
            } else {
                let dialogue_label = "dialogue".to_string();
                graph.push_str(&format!(
                    "[{source}]asplit={}[{dialogue_label}]{};",
                    ducking.len() + 1,
                    ducking
                        .iter()
                        .map(|index| format!("[sidechain{index}]"))
                        .collect::<String>()
                ));
                dialogue_label
            };
            for index in ducking {
                graph.push_str(&format!(
                    "[music{index}][sidechain{index}]sidechaincompress=threshold=0.025:ratio=10:attack=20:release=500[ducked{index}];"
                ));
                music_labels[index] = format!("ducked{index}");
            }
            graph.push_str(&format!(
                "[{dialogue_label}]{}amix=inputs={}:duration=first:normalize=0[aout];",
                music_labels
                    .iter()
                    .map(|label| format!("[{label}]"))
                    .collect::<String>(),
                music_labels.len() + 1
            ));
        } else if music_labels.len() == 1 {
            graph.push_str(&format!("[{}]anull[aout];", music_labels[0]));
        } else {
            graph.push_str(&format!(
                "{}amix=inputs={}:duration=longest:normalize=0[aout];",
                music_labels
                    .iter()
                    .map(|label| format!("[{label}]"))
                    .collect::<String>(),
                music_labels.len()
            ));
        }
        audio_map = Some("[aout]".into());
    }
    // WYSIWYG captions: the frontend-rendered PNG states ride a concat
    // input; resampled to 60 fps so karaoke steps land within 17 ms of their
    // word boundary at any delivery rate. Overlaid below titles/ASS.
    if let Some(_concat) = caption_overlay {
        let caption_input = 1 + broll_inputs.len() + music_inputs.len();
        graph.push_str(&format!(
            "[{caption_input}:v]format=rgba,fps=60,setsar=1[cap];             [{current}][cap]overlay=0:0:format=auto:eof_action=pass[vcap];"
        ));
        current = "vcap".to_string();
    }
    if let Some(ass) = ass {
        graph.push_str(&format!(
            "[{current}]ass=filename='{}'[vout]",
            escape_filter_path(ass)
        ));
    } else {
        graph.push_str(&format!("[{current}]null[vout]"));
    }

    Ok(VideoFilter {
        filter_complex: graph,
        audio_map,
        broll_inputs,
        music_inputs,
        caption_concat: caption_overlay.map(Path::to_path_buf),
    })
}

/// Canvas-fit filter used inside a per-segment chain when framing is active.
/// Mirrors the post-concat canvas step: `contain` scales down and pads black,
/// `cover` scales up and crops. Without explicit output dimensions the source
/// frame is the canvas; every segment is normalized to the same size so
/// framed and unframed segments concat cleanly.
fn canvas_fit_filter(canvas: VideoCanvas, width: u32, height: u32) -> String {
    if canvas.output_dimensions.is_some() {
        match canvas.fit {
            ExportCanvasFit::Contain => format!(
                "scale=w={width}:h={height}:force_original_aspect_ratio=decrease:force_divisible_by=2:reset_sar=1,\
                 pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black"
            ),
            ExportCanvasFit::Cover => format!(
                "scale=w={width}:h={height}:force_original_aspect_ratio=increase:force_divisible_by=2:reset_sar=1,\
                 crop={width}:{height}:(iw-ow)/2:(ih-oh)/2"
            ),
        }
    } else {
        format!("scale=w={width}:h={height}:reset_sar=1")
    }
}

fn even_dim(value: f64) -> u32 {
    ((value.round() as u32) & !1).max(2)
}

/// Filter chain rendering one shot's framing on a canvas-sized frame.
/// Adapted from pireel (AGPL-3.0), `composition-core.ts` `shotTransformVars`:
/// the frame scales about its center and is then positioned. `punch-in`
/// crops a centered window and scales it back to the canvas; corner/split
/// scale down and pad onto black — the same background the `contain`
/// canvas fit uses.
fn framing_filter_chain(framing: &ShotFraming, dims: (u32, u32)) -> Option<String> {
    let (width, height) = dims;
    let scale = treat_scale(framing.treatment, framing.size);
    match framing.treatment {
        ShotTreatment::Full => None,
        ShotTreatment::PunchIn => {
            let crop_w = even_dim(f64::from(width) / scale);
            let crop_h = even_dim(f64::from(height) / scale);
            Some(format!(
                "crop=w={crop_w}:h={crop_h}:x=(iw-ow)/2:y=(ih-oh)/2,scale=w={width}:h={height}"
            ))
        }
        ShotTreatment::CornerBr | ShotTreatment::CornerTl => {
            let small_w = even_dim(f64::from(width) * scale);
            let small_h = even_dim(f64::from(height) * scale);
            // pireel leaves a 2% margin from the corner.
            let margin_x = (f64::from(width) * 0.02).round() as u32;
            let margin_y = (f64::from(height) * 0.02).round() as u32;
            let (x, y) = if framing.treatment == ShotTreatment::CornerBr {
                (width - small_w - margin_x, height - small_h - margin_y)
            } else {
                (margin_x, margin_y)
            };
            Some(format!(
                "scale=w={small_w}:h={small_h},pad={width}:{height}:{x}:{y}:color=black"
            ))
        }
        ShotTreatment::SplitL | ShotTreatment::SplitR => {
            let small_w = even_dim(f64::from(width) * scale);
            let small_h = even_dim(f64::from(height) * scale);
            // Half-split hugs its edge, vertically centered.
            let x = if framing.treatment == ShotTreatment::SplitL {
                0
            } else {
                width - small_w
            };
            let y = (height - small_h) / 2;
            Some(format!(
                "scale=w={small_w}:h={small_h},pad={width}:{height}:{x}:{y}:color=black"
            ))
        }
    }
}

fn scale_design_rect(rect: Rect, frame_size: Option<(u32, u32)>) -> Rect {
    let Some((width, height)) = frame_size else {
        return rect;
    };
    let scale_x = f64::from(width) / 1920.0;
    let scale_y = f64::from(height) / 1080.0;
    Rect {
        x: (f64::from(rect.x) * scale_x).round() as u32,
        y: (f64::from(rect.y) * scale_y).round() as u32,
        width: ((f64::from(rect.width) * scale_x).round() as u32).max(1),
        height: ((f64::from(rect.height) * scale_y).round() as u32).max(1),
    }
}

fn scale_design_radius(radius: u32, frame_size: Option<(u32, u32)>) -> u32 {
    let Some((width, height)) = frame_size else {
        return radius;
    };
    let scale = (f64::from(width) / 1920.0).min(f64::from(height) / 1080.0);
    (f64::from(radius) * scale).round() as u32
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
            audio_mix: AudioMix::default(),
            settings: None,
            soft_subtitle: None,
            include_ass: true,
            framings: Vec::new(),
            caption_overlay: None,
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
    let started = Instant::now();
    let VideoRenderOptions {
        purpose,
        mode,
        on_progress,
        audio_mix,
        settings,
        soft_subtitle,
        include_ass,
        framings,
        caption_overlay,
    } = options;
    let settings = settings.unwrap_or_else(|| VideoExportSettings {
        encoding_speed: match mode.as_deref() {
            Some("quality") => ExportEncodingSpeed::Quality,
            Some("fast") => ExportEncodingSpeed::Fast,
            _ => ExportEncodingSpeed::MatchSource,
        },
        ..Default::default()
    });
    settings.validate()?;
    for placement in placements {
        if !placement.file.exists() {
            return Err(AppError::ProjectNotFound(placement.file.clone()));
        }
    }
    for track in &audio_mix.music {
        if !track.path.is_file() {
            return Err(AppError::ProjectNotFound(track.path.clone()));
        }
    }

    let output_duration: f64 = super::project::kept_intervals(doc, cuts)
        .iter()
        .map(|(start, end)| end - start)
        .sum();

    let path = export_render_path(
        &settings,
        cuts,
        placements,
        &audio_mix,
        // A caption overlay burns into the picture just like ASS does.
        include_ass || caption_overlay.is_some(),
        &framings,
    );
    if path == ExportRenderPath::StreamCopy && purpose == RenderPurpose::Final {
        info!(
            path = "stream-copy",
            soft_subtitle = soft_subtitle.is_some(),
            "video export using remux (no re-encode)"
        );
        render_stream_copy_remux(
            doc,
            output,
            soft_subtitle.as_deref(),
            &settings,
            output_duration,
            on_progress,
        )
        .await?;
        info!(
            path = "stream-copy",
            elapsed_ms = started.elapsed().as_millis() as u64,
            "video export remux finished"
        );
        return Ok(());
    }

    let source_info = crate::media::probe(&doc.media.path).await?;
    let source_dimensions = source_info.width.zip(source_info.height);
    let source_bitrate = source_info.bit_rate;
    let output_dimensions = settings.target_dimensions(source_dimensions);
    // B-roll rectangles are stored in normalized 1920×1080 design space and
    // must be projected onto the final canvas, not the uncropped source.
    let frame_size = output_dimensions.or(source_dimensions);
    let filter = build_video_filter_inner(
        doc,
        cuts,
        include_ass.then_some(ass),
        placements,
        &audio_mix,
        &framings,
        VideoCanvas {
            frame_size,
            output_dimensions,
            fit: settings.canvas_fit,
        },
        caption_overlay.as_deref(),
    )?;
    let filter_ms = started.elapsed().as_millis() as u64;
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
    for input in &filter.music_inputs {
        args.extend([
            "-stream_loop".into(),
            "-1".into(),
            "-i".into(),
            input.display().to_string(),
        ]);
    }
    // WYSIWYG caption states: one concat input right after music so the
    // filter graph's caption input index (1 + broll + music) lines up.
    if let Some(concat) = &filter.caption_concat {
        args.extend([
            "-f".into(),
            "concat".into(),
            "-safe".into(),
            "0".into(),
            "-i".into(),
            concat.display().to_string(),
        ]);
    }
    let soft_subtitle_input = soft_subtitle.as_ref().map(|path| {
        let index = 1
            + filter.broll_inputs.len()
            + filter.music_inputs.len()
            + usize::from(filter.caption_concat.is_some());
        args.extend(["-i".into(), path.display().to_string()]);
        index
    });
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
            audio_encoder(settings.audio_codec).into(),
        ]);
    }
    if let Some(input) = soft_subtitle_input {
        let subtitle_language = settings
            .subtitle_language
            .as_deref()
            .or(doc.meta.language.as_deref())
            .unwrap_or("und");
        args.extend([
            "-map".into(),
            format!("{input}:s:0"),
            "-c:s".into(),
            "mov_text".into(),
            "-metadata:s:s:0".into(),
            format!("language={subtitle_language}"),
        ]);
    }
    let encoder = encoder_for_settings(&settings)?;
    args.extend(encoder_args(
        &encoder,
        purpose,
        settings.encoding_speed,
        source_bitrate,
    ));
    args.extend([
        "-movflags".into(),
        "+faststart".into(),
        "-t".into(),
        format!("{output_duration:.6}"),
        output.display().to_string(),
    ]);
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    info!(
        path = "reencode",
        encoder = %encoder,
        speed = ?settings.encoding_speed,
        source_bitrate,
        filter_prepare_ms = filter_ms,
        reason = %reencode_reason(&settings, cuts, placements, &audio_mix, include_ass || caption_overlay.is_some(), &framings),
        "video export re-encode starting"
    );
    if let Some(callback) = &on_progress {
        callback(VideoRenderProgress {
            progress: 0,
            current_seconds: 0.0,
            total_seconds: output_duration,
            encoder: encoder.clone(),
        });
    }
    let encode_started = Instant::now();
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
    info!(
        path = "reencode",
        encoder = %encoder,
        encode_ms = encode_started.elapsed().as_millis() as u64,
        total_ms = started.elapsed().as_millis() as u64,
        "video export re-encode finished"
    );
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

async fn render_stream_copy_remux(
    doc: &Doc,
    output: &Path,
    soft_subtitle: Option<&Path>,
    settings: &VideoExportSettings,
    output_duration: f64,
    on_progress: Option<VideoRenderProgressCallback>,
) -> AppResult<()> {
    let encoder = "copy".to_string();
    if let Some(callback) = &on_progress {
        callback(VideoRenderProgress {
            progress: 0,
            current_seconds: 0.0,
            total_seconds: output_duration,
            encoder: encoder.clone(),
        });
    }
    // Explicit maps only: never pull source subtitle/data streams into the remux.
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
    if let Some(path) = soft_subtitle {
        args.extend(["-i".into(), path.display().to_string()]);
    }
    args.extend([
        "-map".into(),
        "0:v:0".into(),
        "-map".into(),
        "0:a?".into(),
        "-c:v".into(),
        "copy".into(),
        "-c:a".into(),
        "copy".into(),
    ]);
    if soft_subtitle.is_some() {
        let subtitle_language = settings
            .subtitle_language
            .as_deref()
            .or(doc.meta.language.as_deref())
            .unwrap_or("und");
        args.extend([
            "-map".into(),
            "1:0".into(),
            "-c:s".into(),
            "mov_text".into(),
            "-metadata:s:s:0".into(),
            format!("language={subtitle_language}"),
        ]);
    }
    args.extend([
        "-movflags".into(),
        "+faststart".into(),
        "-t".into(),
        format!("{output_duration:.6}"),
        output.display().to_string(),
    ]);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
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

pub async fn render_broll_snapshot(
    doc: &Doc,
    placement: &BrollPlacement,
    source_time: f64,
    output: &Path,
) -> AppResult<()> {
    if !placement.file.exists() {
        return Err(AppError::ProjectNotFound(placement.file.clone()));
    }
    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let media_info = crate::media::probe(&doc.media.path).await?;
    let frame_size = media_info.width.zip(media_info.height);
    let source_end = placement.end.min(doc.media.duration_seconds);
    if source_end <= placement.start {
        return Err(AppError::Schema(
            "B-roll placement is outside the current media duration".into(),
        ));
    }
    let source_time = source_time.clamp(placement.start, source_end);
    let asset_time = placement.source_start + (source_time - placement.start).max(0.0);

    let mut snapshot_doc = doc.clone();
    snapshot_doc.media.duration_seconds = 0.1;
    snapshot_doc.media.sample_rate = None;
    snapshot_doc.media.channels = None;
    let mut snapshot_placement = placement.clone();
    snapshot_placement.start = 0.0;
    snapshot_placement.end = 0.1;
    snapshot_placement.source_start = 0.0;
    let filter = build_video_filter_inner(
        &snapshot_doc,
        &[],
        None,
        std::slice::from_ref(&snapshot_placement),
        &AudioMix::default(),
        &[],
        VideoCanvas {
            frame_size,
            ..Default::default()
        },
        None,
    )?;

    let mut args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostdin".into(),
        "-y".into(),
        "-ss".into(),
        format!("{source_time:.6}"),
        "-i".into(),
        doc.media.path.display().to_string(),
    ];
    for input in &filter.broll_inputs {
        if is_still_image(input) {
            args.extend(["-loop".into(), "1".into()]);
        } else {
            args.extend([
                "-ss".into(),
                format!("{asset_time:.6}"),
                "-stream_loop".into(),
                "-1".into(),
            ]);
        }
        args.extend(["-i".into(), input.display().to_string()]);
    }
    args.extend([
        "-filter_complex".into(),
        filter.filter_complex,
        "-map".into(),
        "[vout]".into(),
        "-frames:v".into(),
        "1".into(),
        "-an".into(),
        output.display().to_string(),
    ]);
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    proc::run("ffmpeg", &arg_refs).await?;
    Ok(())
}

/// Every encoder the export pipeline knows how to build arguments for.
/// `LUMEN_CUT_VIDEO_ENCODER` is validated against this so a typo cannot make
/// ffmpeg fail deep inside a long render.
const KNOWN_ENCODERS: &[&str] = &[
    "libx264",
    "libx265",
    "h264_videotoolbox",
    "hevc_videotoolbox",
    "h264_nvenc",
    "hevc_nvenc",
    "h264_qsv",
    "hevc_qsv",
    "h264_amf",
    "hevc_amf",
];

/// Hardware H.264 encoders to try on Windows, best quality first: NVIDIA
/// NVENC, then Intel Quick Sync, then AMD AMF.
const WINDOWS_H264_HARDWARE: &[&str] = &["h264_nvenc", "h264_qsv", "h264_amf"];
const WINDOWS_HEVC_HARDWARE: &[&str] = &["hevc_nvenc", "hevc_qsv", "hevc_amf"];

fn is_hardware_encoder(encoder: &str) -> bool {
    encoder.ends_with("_nvenc") || encoder.ends_with("_qsv") || encoder.ends_with("_amf")
}

/// Whether `encoder` can actually open a session on this machine.
///
/// `ffmpeg -encoders` only reports what the build supports, which on Windows
/// is a poor proxy: an ffmpeg build with NVENC compiled in still fails on a
/// machine with no NVIDIA GPU. A throwaway one-frame encode is the only
/// honest test. Results are cached, so this costs at most one ffmpeg run per
/// candidate per process.
fn hardware_encoder_available(encoder: &str) -> bool {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, bool>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(known) = cache
        .lock()
        .expect("encoder probe cache poisoned")
        .get(encoder)
    {
        return *known;
    }
    let available = crate::doctor::quiet_command("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=128x128:d=0.1",
            "-c:v",
            encoder,
            "-frames:v",
            "1",
            "-f",
            "null",
            "-",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    cache
        .lock()
        .expect("encoder probe cache poisoned")
        .insert(encoder.to_string(), available);
    available
}

/// The fastest usable encoder for `codec`, or `None` to use the software one.
fn hardware_encoder(codec: ExportVideoCodec) -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        // VideoToolbox is always present on a supported macOS host and falls
        // back to software internally (`-allow_sw`), so it needs no probe.
        return match codec {
            ExportVideoCodec::H264 => Some("h264_videotoolbox"),
            ExportVideoCodec::Hevc => Some("hevc_videotoolbox"),
            ExportVideoCodec::Prores => None,
        };
    }
    if !cfg!(windows) {
        return None;
    }
    let candidates = match codec {
        ExportVideoCodec::H264 => WINDOWS_H264_HARDWARE,
        ExportVideoCodec::Hevc => WINDOWS_HEVC_HARDWARE,
        ExportVideoCodec::Prores => return None,
    };
    candidates
        .iter()
        .copied()
        .find(|encoder| hardware_encoder_available(encoder))
}

fn selected_encoder() -> String {
    if let Ok(configured) = std::env::var("LUMEN_CUT_VIDEO_ENCODER") {
        if KNOWN_ENCODERS.contains(&configured.as_str()) {
            return configured;
        }
    }
    hardware_encoder(ExportVideoCodec::H264)
        .unwrap_or("libx264")
        .into()
}

pub fn encoder_for_settings(settings: &VideoExportSettings) -> AppResult<String> {
    settings.validate()?;
    match settings.video_codec {
        ExportVideoCodec::H264 => match settings.encoding_speed {
            ExportEncodingSpeed::MatchSource | ExportEncodingSpeed::Fast => Ok(selected_encoder()),
            ExportEncodingSpeed::Quality => Ok("libx264".into()),
        },
        ExportVideoCodec::Hevc => match settings.encoding_speed {
            ExportEncodingSpeed::MatchSource | ExportEncodingSpeed::Fast => {
                Ok(hardware_encoder(ExportVideoCodec::Hevc)
                    .unwrap_or("libx265")
                    .into())
            }
            ExportEncodingSpeed::Quality => Ok("libx265".into()),
        },
        ExportVideoCodec::Prores => Ok("prores_ks".into()),
    }
}

pub fn encoder_for_mode(mode: Option<&str>) -> AppResult<String> {
    match mode.unwrap_or("auto") {
        "auto" | "match-source" | "fast" => Ok(selected_encoder()),
        "quality" => Ok("libx264".into()),
        other => Err(AppError::Schema(format!(
            "unknown video export mode: {other}"
        ))),
    }
}

fn encoder_args(
    encoder: &str,
    purpose: RenderPurpose,
    speed: ExportEncodingSpeed,
    source_bitrate: Option<u64>,
) -> Vec<String> {
    if matches!(encoder, "h264_videotoolbox" | "hevc_videotoolbox") {
        let mut args = vec![
            "-c:v".into(),
            encoder.into(),
            "-pix_fmt".into(),
            "yuv420p".into(),
            // Allow VideoToolbox's software fallback: without it ffmpeg fails
            // outright (-12903) when the hardware session cannot be created —
            // virtualized machines (CI) or a busy hardware encoder. Hardware
            // is still preferred whenever it is available.
            "-allow_sw".into(),
            "1".into(),
        ];
        // Match-source + known bitrate: constrain rate instead of high fixed q.
        // Re-encoding with a (usually less efficient) encoder at the source
        // bitrate visibly degrades quality, so target 1.5x the source rate.
        let use_bitrate = purpose == RenderPurpose::Final
            && speed == ExportEncodingSpeed::MatchSource
            && source_bitrate.is_some_and(|br| br >= 100_000);
        if use_bitrate {
            let br = source_bitrate.unwrap().saturating_mul(3) / 2;
            let maxrate = ((br as f64) * 1.25).round() as u64;
            let bufsize = br.saturating_mul(2);
            args.extend([
                "-b:v".into(),
                br.to_string(),
                "-maxrate".into(),
                maxrate.to_string(),
                "-bufsize".into(),
                bufsize.to_string(),
            ]);
        } else {
            let quality = match (purpose, speed) {
                (RenderPurpose::Preview, _) => "55",
                // Was 65 (near-master); 58 is still clean but much smaller.
                (RenderPurpose::Final, ExportEncodingSpeed::Fast) => "58",
                // Was 60; 70 preserves more source detail when bitrate is unknown.
                (RenderPurpose::Final, ExportEncodingSpeed::MatchSource) => "70",
                (RenderPurpose::Final, ExportEncodingSpeed::Quality) => "55",
            };
            args.extend(["-q:v".into(), quality.into()]);
        }
        if encoder == "h264_videotoolbox" {
            args.extend(["-profile:v".into(), "high".into()]);
        } else {
            args.extend(["-tag:v".into(), "hvc1".into()]);
        }
        if purpose == RenderPurpose::Preview {
            args.extend([
                "-realtime".into(),
                "1".into(),
                "-prio_speed".into(),
                "1".into(),
            ]);
        }
        args
    } else if is_hardware_encoder(encoder) {
        // NVENC / Quick Sync / AMF. Rate control differs per vendor, but all
        // three accept the same explicit bitrate triple, so match-source
        // exports share one path with the software and VideoToolbox encoders.
        let mut args = vec![
            "-c:v".into(),
            encoder.into(),
            "-pix_fmt".into(),
            "yuv420p".into(),
        ];
        let use_bitrate = purpose == RenderPurpose::Final
            && speed == ExportEncodingSpeed::MatchSource
            && source_bitrate.is_some_and(|br| br >= 100_000);
        // Quantizer on the H.264 0–51 scale, chosen to line up with the
        // libx264 CRF values used for the same purpose/speed pair.
        let quality = match (purpose, speed) {
            (RenderPurpose::Preview, _) => 26,
            (RenderPurpose::Final, ExportEncodingSpeed::Fast) => 23,
            (RenderPurpose::Final, ExportEncodingSpeed::MatchSource) => 23,
            (RenderPurpose::Final, ExportEncodingSpeed::Quality) => 21,
        };
        let fast = purpose == RenderPurpose::Preview || speed == ExportEncodingSpeed::Fast;
        if encoder.ends_with("_nvenc") {
            args.extend([
                "-preset".into(),
                if fast { "p1".into() } else { "p4".into() },
            ]);
        } else if encoder.ends_with("_qsv") {
            args.extend([
                "-preset".into(),
                if fast {
                    "veryfast".into()
                } else {
                    "medium".into()
                },
            ]);
        } else {
            args.extend([
                "-quality".into(),
                if fast {
                    "speed".into()
                } else {
                    "balanced".into()
                },
            ]);
        }
        if use_bitrate {
            // Same 1.5x headroom as every other encoder here.
            let br = source_bitrate.unwrap().saturating_mul(3) / 2;
            let maxrate = ((br as f64) * 1.25).round() as u64;
            let bufsize = br.saturating_mul(2);
            args.extend([
                "-b:v".into(),
                br.to_string(),
                "-maxrate".into(),
                maxrate.to_string(),
                "-bufsize".into(),
                bufsize.to_string(),
            ]);
        } else if encoder.ends_with("_nvenc") {
            args.extend([
                "-rc".into(),
                "vbr".into(),
                "-cq".into(),
                quality.to_string(),
                // NVENC only honours -cq when the target bitrate is unset.
                "-b:v".into(),
                "0".into(),
            ]);
        } else if encoder.ends_with("_qsv") {
            args.extend(["-global_quality".into(), quality.to_string()]);
        } else {
            args.extend([
                "-rc".into(),
                "cqp".into(),
                "-qp_i".into(),
                quality.to_string(),
                "-qp_p".into(),
                quality.to_string(),
            ]);
        }
        if encoder.starts_with("hevc_") {
            args.extend(["-tag:v".into(), "hvc1".into()]);
        }
        args
    } else if encoder == "libx264" || encoder == "libx265" {
        let use_bitrate = purpose == RenderPurpose::Final
            && speed == ExportEncodingSpeed::MatchSource
            && source_bitrate.is_some_and(|br| br >= 100_000);
        let mut args = vec!["-c:v".into(), encoder.into()];
        if use_bitrate {
            // Same 1.5x headroom as the VideoToolbox path above: re-encoding
            // at the source bitrate loses visible quality.
            let br = source_bitrate.unwrap().saturating_mul(3) / 2;
            let maxrate = ((br as f64) * 1.25).round() as u64;
            let bufsize = br.saturating_mul(2);
            let preset = "veryfast";
            args.extend([
                "-preset".into(),
                preset.into(),
                "-b:v".into(),
                br.to_string(),
                "-maxrate".into(),
                maxrate.to_string(),
                "-bufsize".into(),
                bufsize.to_string(),
            ]);
        } else {
            // Delivery CRF 23 (was archival 18). Preview stays veryfast.
            let (preset, crf) = match (purpose, speed) {
                (RenderPurpose::Preview, _) => ("veryfast", "23"),
                (RenderPurpose::Final, ExportEncodingSpeed::Fast) => ("veryfast", "23"),
                (RenderPurpose::Final, ExportEncodingSpeed::MatchSource) => ("medium", "23"),
                (RenderPurpose::Final, ExportEncodingSpeed::Quality) => ("medium", "22"),
            };
            args.extend(["-preset".into(), preset.into(), "-crf".into(), crf.into()]);
        }
        if encoder == "libx265" {
            args.extend(["-tag:v".into(), "hvc1".into()]);
        }
        args
    } else {
        vec![
            "-c:v".into(),
            "prores_ks".into(),
            "-profile:v".into(),
            "3".into(),
            "-pix_fmt".into(),
            "yuv422p10le".into(),
        ]
    }
}

fn audio_encoder(codec: ExportAudioCodec) -> &'static str {
    match codec {
        ExportAudioCodec::Aac => "aac",
        ExportAudioCodec::Pcm => "pcm_s16le",
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
    fn audio_mix_is_applied_after_cut_concatenation() {
        let cut = Cut {
            id: "c1".into(),
            note: None,
            a_word: "w1".into(),
            b_word: "w1".into(),
            kind: CutKind::Manual,
            duration: 2.0,
        };
        let mix = AudioMix {
            volume: 1.25,
            muted: false,
            fade_in: 0.5,
            fade_out: 1.0,
            voice_enhance: false,
            normalize_loudness: false,
            loudness_target: -16.0,
            music: vec![],
        };
        let plan =
            build_video_filter_with_broll_audio(&doc(), &[cut], Path::new("/tmp/a.ass"), &[], &mix)
                .unwrap();

        assert!(plan.filter_complex.contains(
            "[acat]volume=1.2500,afade=t=in:st=0:d=0.500000,afade=t=out:st=3.000000:d=1.000000[amix]"
        ));
        assert_eq!(plan.audio_map.as_deref(), Some("[amix]"));
    }

    #[test]
    fn dialogue_enhancement_and_loudness_are_applied_before_manual_gain() {
        let mix = AudioMix {
            volume: 0.8,
            voice_enhance: true,
            normalize_loudness: true,
            loudness_target: -14.0,
            ..Default::default()
        };
        let plan =
            build_video_filter_with_broll_audio(&doc(), &[], Path::new("/tmp/a.ass"), &[], &mix)
                .unwrap();

        assert!(plan.filter_complex.contains(
            "[0:a]highpass=f=80,afftdn=nr=8:nf=-45:tn=1,\
acompressor=threshold=0.125:ratio=3:attack=20:release=250:makeup=1.5,\
loudnorm=I=-14.0:LRA=11:TP=-1.5,aresample=48000,volume=0.8000[amix]"
        ));
    }

    #[test]
    fn muted_audio_mix_exports_silence_without_dropping_the_track() {
        let plan = build_video_filter_with_broll_audio(
            &doc(),
            &[],
            Path::new("/tmp/a.ass"),
            &[],
            &AudioMix {
                muted: true,
                ..Default::default()
            },
        )
        .unwrap();

        assert!(plan.filter_complex.contains("[0:a]volume=0.0000[amix]"));
        assert_eq!(plan.audio_map.as_deref(), Some("[amix]"));
    }

    #[test]
    fn background_music_is_looped_trimmed_ducked_and_mixed_after_dialogue_processing() {
        let plan = build_video_filter_with_broll_audio(
            &doc(),
            &[],
            Path::new("/tmp/a.ass"),
            &[],
            &AudioMix {
                voice_enhance: true,
                music: vec![crate::data::audio_mix::MusicTrack {
                    id: "music-a".into(),
                    path: "/tmp/music.wav".into(),
                    start: 1.0,
                    end: 5.0,
                    source_start: 2.0,
                    volume: 0.25,
                    fade_in: 0.5,
                    fade_out: 1.0,
                    ducking: true,
                }],
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(plan.music_inputs, vec![PathBuf::from("/tmp/music.wav")]);
        assert!(plan.filter_complex.contains(
            "[1:a]atrim=start=2.000000:duration=4.000000,\
asetpts=PTS-STARTPTS+1.000000/TB,volume=0.2500,\
afade=t=in:st=0:d=0.500000,afade=t=out:st=3.000000:d=1.000000[music0]"
        ));
        assert!(plan.filter_complex.contains(
            "[amix]asplit=2[dialogue][sidechain0];\
[music0][sidechain0]sidechaincompress=threshold=0.025:ratio=10:attack=20:release=500[ducked0];\
[dialogue][ducked0]amix=inputs=2:duration=first:normalize=0[aout]"
        ));
        assert_eq!(plan.audio_map.as_deref(), Some("[aout]"));
    }

    #[test]
    fn multiple_music_clips_use_distinct_inputs_and_one_final_program_mix() {
        let plan = build_video_filter_with_broll_audio(
            &doc(),
            &[],
            Path::new("/tmp/a.ass"),
            &[],
            &AudioMix {
                music: vec![
                    crate::data::audio_mix::MusicTrack {
                        id: "music-a".into(),
                        path: "/tmp/a.wav".into(),
                        start: 0.0,
                        end: 2.0,
                        source_start: 0.0,
                        volume: 0.2,
                        fade_in: 0.0,
                        fade_out: 0.0,
                        ducking: true,
                    },
                    crate::data::audio_mix::MusicTrack {
                        id: "music-b".into(),
                        path: "/tmp/b.wav".into(),
                        start: 3.0,
                        end: 5.0,
                        source_start: 1.0,
                        volume: 0.3,
                        fade_in: 0.0,
                        fade_out: 0.0,
                        ducking: false,
                    },
                ],
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(
            plan.music_inputs,
            vec![PathBuf::from("/tmp/a.wav"), PathBuf::from("/tmp/b.wav")]
        );
        assert!(plan
            .filter_complex
            .contains("[1:a]atrim=start=0.000000:duration=2.000000"));
        assert!(plan
            .filter_complex
            .contains("[2:a]atrim=start=1.000000:duration=2.000000"));
        assert!(plan
            .filter_complex
            .contains("[amix]asplit=2[dialogue][sidechain0];"));
        assert!(plan.filter_complex.contains(
            "[music0][sidechain0]sidechaincompress=threshold=0.025:ratio=10:attack=20:release=500[ducked0];"
        ));
        assert!(plan
            .filter_complex
            .contains("[dialogue][ducked0][music1]amix=inputs=3:duration=first:normalize=0[aout]"));
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
    fn design_canvas_rect_scales_to_the_export_frame() {
        let placement = crate::data::broll::BrollPlacement {
            id: "br-scaled".into(),
            file: "/tmp/portrait.png".into(),
            start: 2.0,
            end: 4.0,
            mode: crate::data::broll::PlacementMode::Pip,
            rect: Some(crate::data::broll::Rect {
                x: 192,
                y: 108,
                width: 960,
                height: 540,
            }),
            fit: crate::data::broll::FitMode::Cover,
            background: crate::data::broll::BackgroundMode::Black,
            source_start: 0.0,
            radius: 20,
            name: None,
        };
        let plan = build_video_filter_inner(
            &doc(),
            &[],
            Some(Path::new("/tmp/a.ass")),
            &[placement],
            &AudioMix::default(),
            &[],
            VideoCanvas {
                frame_size: Some((1280, 720)),
                ..Default::default()
            },
            None,
        )
        .unwrap();

        assert!(plan.filter_complex.contains("scale=640:360"));
        assert!(plan.filter_complex.contains("overlay=x=128:y=72"));
        assert!(plan.filter_complex.contains("min(13,min(W,H)/2)"));
    }

    #[test]
    fn fixed_resolution_scales_to_fit_and_pads_without_stretching() {
        let plan = build_video_filter_inner(
            &doc(),
            &[],
            None,
            &[],
            &AudioMix::default(),
            &[],
            VideoCanvas {
                frame_size: Some((1920, 1080)),
                output_dimensions: Some((1920, 1080)),
                ..Default::default()
            },
            None,
        )
        .unwrap();

        assert!(plan.filter_complex.contains(
            "scale=w=1920:h=1080:force_original_aspect_ratio=decrease:force_divisible_by=2:reset_sar=1"
        ));
        assert!(plan
            .filter_complex
            .contains("pad=1920:1080:(ow-iw)/2:(oh-ih)/2:color=black[vcanvas]"));
        assert!(plan.filter_complex.ends_with("[vcanvas]null[vout]"));
    }

    #[test]
    fn cover_canvas_is_created_before_broll_and_crops_without_stretching() {
        let placement = crate::data::broll::BrollPlacement {
            id: "br-portrait".into(),
            file: "/tmp/portrait.png".into(),
            start: 2.0,
            end: 4.0,
            mode: crate::data::broll::PlacementMode::Pip,
            rect: Some(crate::data::broll::Rect {
                x: 960,
                y: 540,
                width: 480,
                height: 270,
            }),
            fit: crate::data::broll::FitMode::Cover,
            background: crate::data::broll::BackgroundMode::Black,
            source_start: 0.0,
            radius: 0,
            name: None,
        };
        let plan = build_video_filter_inner(
            &doc(),
            &[],
            None,
            &[placement],
            &AudioMix::default(),
            &[],
            VideoCanvas {
                frame_size: Some((1080, 1920)),
                output_dimensions: Some((1080, 1920)),
                fit: ExportCanvasFit::Cover,
            },
            None,
        )
        .unwrap();

        let crop = plan.filter_complex.find("crop=1080:1920").unwrap();
        let broll = plan.filter_complex.find("[1:v]trim=").unwrap();
        assert!(crop < broll);
        assert!(plan.filter_complex.contains("overlay=x=540:y=960"));
    }

    fn framing(treatment: ShotTreatment, size: Option<f64>) -> ShotFraming {
        ShotFraming {
            id: "fr-1".into(),
            start: 0.0,
            end: 6.0,
            treatment,
            size,
        }
    }

    #[test]
    fn framing_chain_computes_each_treatment_geometry() {
        let dims = (1920, 1080);
        // punch-in: centered crop window scaled back to the canvas.
        assert_eq!(
            framing_filter_chain(&framing(ShotTreatment::PunchIn, None), dims).as_deref(),
            Some("crop=w=1572:h=884:x=(iw-ow)/2:y=(ih-oh)/2,scale=w=1920:h=1080")
        );
        assert_eq!(
            framing_filter_chain(&framing(ShotTreatment::PunchIn, Some(100.0)), dims).as_deref(),
            Some("crop=w=960:h=540:x=(iw-ow)/2:y=(ih-oh)/2,scale=w=1920:h=1080")
        );
        // corner: shrink to 0.34× and hug the corner with a 2% margin.
        assert_eq!(
            framing_filter_chain(&framing(ShotTreatment::CornerBr, None), dims).as_deref(),
            Some("scale=w=652:h=366,pad=1920:1080:1230:692:color=black")
        );
        assert_eq!(
            framing_filter_chain(&framing(ShotTreatment::CornerTl, None), dims).as_deref(),
            Some("scale=w=652:h=366,pad=1920:1080:38:22:color=black")
        );
        // split: shrink to 0.5× and hug the left/right edge, vertically centered.
        assert_eq!(
            framing_filter_chain(&framing(ShotTreatment::SplitL, None), dims).as_deref(),
            Some("scale=w=960:h=540,pad=1920:1080:0:270:color=black")
        );
        assert_eq!(
            framing_filter_chain(&framing(ShotTreatment::SplitR, None), dims).as_deref(),
            Some("scale=w=960:h=540,pad=1920:1080:960:270:color=black")
        );
        assert_eq!(
            framing_filter_chain(&framing(ShotTreatment::Full, None), dims),
            None
        );
    }

    #[test]
    fn framing_moves_the_canvas_fit_into_each_segment_chain() {
        let cut = Cut {
            id: "c1".into(),
            note: None,
            a_word: "w1".into(),
            b_word: "w1".into(),
            kind: CutKind::Manual,
            duration: 2.0,
        };
        // Frame only the second kept segment (3..6).
        let entries = vec![ShotFraming {
            id: "fr-1".into(),
            start: 3.0,
            end: 6.0,
            treatment: ShotTreatment::PunchIn,
            size: None,
        }];
        let plan = build_video_filter_inner(
            &doc(),
            &[cut],
            None,
            &[],
            &AudioMix::default(),
            &entries,
            VideoCanvas {
                frame_size: Some((1920, 1080)),
                output_dimensions: Some((1920, 1080)),
                fit: ExportCanvasFit::Contain,
            },
            None,
        )
        .unwrap();

        // The first segment is canvas-fitted but not framed.
        assert!(plan.filter_complex.contains(
            "[0:v]trim=start=0.000000:end=1.000000,setpts=PTS-STARTPTS,\
scale=w=1920:h=1080:force_original_aspect_ratio=decrease:force_divisible_by=2:reset_sar=1,\
pad=1920:1080:(ow-iw)/2:(oh-ih)/2:color=black[v0];"
        ));
        // The second segment is canvas-fitted and then punch-in framed.
        assert!(plan.filter_complex.contains(
            "pad=1920:1080:(ow-iw)/2:(oh-ih)/2:color=black,\
crop=w=1572:h=884:x=(iw-ow)/2:y=(ih-oh)/2,scale=w=1920:h=1080[v1];"
        ));
        // Concat still hard-cuts the segments; the global canvas step is gone.
        assert!(plan
            .filter_complex
            .contains("concat=n=2:v=1:a=1[vbase][acat];"));
        assert!(!plan.filter_complex.contains("[vcanvas]"));
        assert!(plan.filter_complex.ends_with("[vbase]null[vout]"));
    }

    #[test]
    fn framing_applies_after_a_cover_canvas_fit() {
        let entries = vec![framing(ShotTreatment::CornerBr, None)];
        let plan = build_video_filter_inner(
            &doc(),
            &[],
            None,
            &[],
            &AudioMix::default(),
            &entries,
            VideoCanvas {
                frame_size: Some((1080, 1920)),
                output_dimensions: Some((1080, 1920)),
                fit: ExportCanvasFit::Cover,
            },
            None,
        )
        .unwrap();

        assert!(plan.filter_complex.contains(
            "[0:v]setpts=PTS-STARTPTS,\
scale=w=1080:h=1920:force_original_aspect_ratio=increase:force_divisible_by=2:reset_sar=1,\
crop=1080:1920:(iw-ow)/2:(ih-oh)/2,\
scale=w=366:h=652,pad=1080:1920:692:1230:color=black[vbase];"
        ));
    }

    #[test]
    fn framing_without_output_dimensions_normalizes_to_the_source_frame() {
        let entries = vec![framing(ShotTreatment::SplitR, Some(50.0))];
        let plan = build_video_filter_inner(
            &doc(),
            &[],
            None,
            &[],
            &AudioMix::default(),
            &entries,
            VideoCanvas {
                frame_size: Some((1920, 1080)),
                ..Default::default()
            },
            None,
        )
        .unwrap();

        assert!(plan.filter_complex.contains(
            "[0:v]setpts=PTS-STARTPTS,scale=w=1920:h=1080:reset_sar=1,\
scale=w=960:h=540,pad=1920:1080:960:270:color=black[vbase];"
        ));
        // Unknown canvas dimensions must not silently drop the framing.
        assert!(build_video_filter_inner(
            &doc(),
            &[],
            None,
            &[],
            &AudioMix::default(),
            &entries,
            VideoCanvas::default(),
            None,
        )
        .is_err());
    }

    /// Whether a VideoToolbox encoder actually works in this environment by
    /// running a one-frame encode with the exact arg shape the render would
    /// use. Virtualized CI runners accept `-allow_sw 1` at session creation
    /// but then fail on encoder properties (-12900), so session creation
    /// alone is not a reliable probe.
    async fn videotoolbox_usable(encoder_args: &[String]) -> bool {
        let mut args = vec![
            "-hide_banner".to_string(),
            "-loglevel".to_string(),
            "error".to_string(),
            "-nostdin".to_string(),
            "-y".to_string(),
            "-f".to_string(),
            "lavfi".to_string(),
            "-i".to_string(),
            "color=c=blue:s=64x64:d=0.1".to_string(),
        ];
        args.extend(encoder_args.iter().cloned());
        args.extend([
            "-frames:v".to_string(),
            "1".to_string(),
            "-f".to_string(),
            "null".to_string(),
            "-".to_string(),
        ]);
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        crate::proc::run("ffmpeg", &arg_refs).await.is_ok()
    }

    #[tokio::test]
    async fn real_export_with_shot_framing_renders_the_framed_canvas() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.mp4");
        make_test_media(&source).await;
        let mut media_doc = doc();
        media_doc.media.path = source;
        media_doc.media.duration_seconds = 0.6;
        let output = temp.path().join("framed.mp4");
        let mut settings = VideoExportSettings {
            encoding_speed: ExportEncodingSpeed::Fast,
            resolution: crate::data::export_settings::ExportResolution::Hd720,
            ..Default::default()
        };
        // The framing assertions are encoder-agnostic. Where VideoToolbox
        // does not actually work (virtualized CI), fall back to libx264 —
        // Quality selects it on every platform.
        if let Ok(encoder) = encoder_for_settings(&settings) {
            if encoder.contains("videotoolbox") {
                let probe_args = encoder_args(
                    &encoder,
                    RenderPurpose::Final,
                    settings.encoding_speed,
                    None,
                );
                if !videotoolbox_usable(&probe_args).await {
                    settings.encoding_speed = ExportEncodingSpeed::Quality;
                }
            }
        }
        render_video_with_broll_options(
            &media_doc,
            &[],
            &temp.path().join("unused.ass"),
            &output,
            &[],
            VideoRenderOptions {
                caption_overlay: None,
                purpose: RenderPurpose::Final,
                mode: None,
                on_progress: None,
                audio_mix: AudioMix::default(),
                settings: Some(settings),
                soft_subtitle: None,
                include_ass: false,
                framings: vec![ShotFraming {
                    id: "fr-1".into(),
                    start: 0.0,
                    end: 0.6,
                    treatment: ShotTreatment::CornerBr,
                    size: None,
                }],
            },
        )
        .await
        .unwrap();
        // The generated filter graph must run in ffmpeg and keep the canvas.
        let rendered = crate::media::probe(&output).await.unwrap();
        assert_eq!(rendered.width.zip(rendered.height), Some((1280, 720)));
    }

    #[test]
    fn all_full_framing_keeps_the_legacy_graph() {
        let entries = vec![framing(ShotTreatment::Full, None)];
        let plan = build_video_filter_inner(
            &doc(),
            &[],
            None,
            &[],
            &AudioMix::default(),
            &entries,
            VideoCanvas::default(),
            None,
        )
        .unwrap();
        assert!(plan
            .filter_complex
            .contains("[0:v]setpts=PTS-STARTPTS[vbase];"));
    }

    #[tokio::test]
    async fn quick_broll_snapshot_renders_without_encoding_the_full_timeline() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.mp4");
        let asset = temp.path().join("asset.mp4");
        let output = temp.path().join("preview.png");
        for (path, color, size) in [(&source, "blue", "640x360"), (&asset, "red", "320x180")] {
            crate::proc::run(
                "ffmpeg",
                &[
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-nostdin",
                    "-y",
                    "-f",
                    "lavfi",
                    "-i",
                    &format!("color=c={color}:s={size}:d=1"),
                    "-c:v",
                    "libx264",
                    "-pix_fmt",
                    "yuv420p",
                    &path.display().to_string(),
                ],
            )
            .await
            .unwrap();
        }

        let mut snapshot_doc = doc();
        snapshot_doc.media.path = source;
        snapshot_doc.media.duration_seconds = 1.0;
        let placement = crate::data::broll::BrollPlacement {
            id: "br-snapshot".into(),
            file: asset,
            start: 0.0,
            end: 1.0,
            mode: crate::data::broll::PlacementMode::Pip,
            rect: Some(crate::data::broll::Rect {
                x: 960,
                y: 0,
                width: 960,
                height: 540,
            }),
            fit: crate::data::broll::FitMode::Cover,
            background: crate::data::broll::BackgroundMode::Black,
            source_start: 0.0,
            radius: 12,
            name: None,
        };

        render_broll_snapshot(&snapshot_doc, &placement, 0.5, &output)
            .await
            .unwrap();
        let rendered = crate::media::probe(&output).await.unwrap();
        assert_eq!(rendered.width.zip(rendered.height), Some((640, 360)));
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
        let preview = encoder_args(
            "h264_videotoolbox",
            RenderPurpose::Preview,
            ExportEncodingSpeed::Fast,
            None,
        );
        let final_render = encoder_args(
            "h264_videotoolbox",
            RenderPurpose::Final,
            ExportEncodingSpeed::Fast,
            None,
        );
        assert!(preview.windows(2).any(|pair| pair == ["-realtime", "1"]));
        assert!(preview.windows(2).any(|pair| pair == ["-q:v", "55"]));
        assert!(final_render.windows(2).any(|pair| pair == ["-q:v", "58"]));
        assert!(!final_render
            .windows(2)
            .any(|pair| pair == ["-realtime", "1"]));
    }

    #[test]
    fn videotoolbox_allows_software_fallback_but_software_encoders_do_not() {
        for encoder in ["h264_videotoolbox", "hevc_videotoolbox"] {
            for speed in [
                ExportEncodingSpeed::MatchSource,
                ExportEncodingSpeed::Fast,
                ExportEncodingSpeed::Quality,
            ] {
                let args = encoder_args(encoder, RenderPurpose::Final, speed, None);
                assert!(
                    args.windows(2).any(|pair| pair == ["-allow_sw", "1"]),
                    "{encoder} {speed:?} must allow the software fallback"
                );
            }
        }
        let software = encoder_args(
            "libx264",
            RenderPurpose::Final,
            ExportEncodingSpeed::Fast,
            None,
        );
        assert!(!software.windows(2).any(|pair| pair == ["-allow_sw", "1"]));
    }

    #[test]
    fn match_source_uses_source_bitrate_when_available() {
        let args = encoder_args(
            "libx264",
            RenderPurpose::Final,
            ExportEncodingSpeed::MatchSource,
            Some(2_000_000),
        );
        assert!(args.windows(2).any(|pair| pair == ["-b:v", "3000000"]));
        assert!(!args.windows(2).any(|pair| pair[0] == "-crf"));
    }

    #[test]
    fn quality_delivery_uses_moderate_crf_not_archival() {
        let args = encoder_args(
            "libx264",
            RenderPurpose::Final,
            ExportEncodingSpeed::Quality,
            None,
        );
        assert!(args.windows(2).any(|pair| pair == ["-crf", "22"]));
    }

    #[test]
    fn stream_copy_requires_soft_or_none_and_clean_timeline() {
        let soft = VideoExportSettings {
            subtitle_mode: crate::data::export_settings::ExportSubtitleMode::Soft,
            ..Default::default()
        };
        assert!(can_stream_copy_export(
            &soft,
            &[],
            &[],
            &AudioMix::default(),
            false,
            &[]
        ));
        assert!(!can_stream_copy_export(
            &VideoExportSettings::default(), // burn
            &[],
            &[],
            &AudioMix::default(),
            false,
            &[]
        ));
        assert!(!can_stream_copy_export(
            &soft,
            &[],
            &[],
            &AudioMix::default(),
            true, // titles burn
            &[]
        ));
        // Any real framing work also forces a re-encode.
        assert!(!can_stream_copy_export(
            &soft,
            &[],
            &[],
            &AudioMix::default(),
            false,
            &[ShotFraming {
                id: "fr-1".into(),
                start: 0.0,
                end: 2.0,
                treatment: ShotTreatment::PunchIn,
                size: None,
            }]
        ));
        assert!(reencode_reason(
            &soft,
            &[],
            &[],
            &AudioMix::default(),
            false,
            &[ShotFraming {
                id: "fr-1".into(),
                start: 0.0,
                end: 2.0,
                treatment: ShotTreatment::PunchIn,
                size: None,
            }]
        )
        .contains("per-shot framing"));
    }

    #[test]
    fn export_mode_makes_the_speed_quality_tradeoff_explicit() {
        assert_eq!(encoder_for_mode(Some("quality")).unwrap(), "libx264");
        let fast = encoder_for_mode(Some("fast")).unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(fast, "h264_videotoolbox");
        } else if cfg!(windows) {
            // Which hardware encoder wins depends on the GPU present, and a
            // machine without one legitimately falls back to software.
            assert!(
                WINDOWS_H264_HARDWARE.contains(&fast.as_str()) || fast == "libx264",
                "unexpected Windows encoder: {fast}"
            );
        } else {
            assert_eq!(fast, "libx264");
        }
        assert!(encoder_for_mode(Some("mystery")).is_err());
    }

    #[test]
    fn hardware_encoders_use_vendor_rate_control_not_crf() {
        for (encoder, expected) in [
            ("h264_nvenc", "-cq"),
            ("h264_qsv", "-global_quality"),
            ("h264_amf", "-qp_i"),
        ] {
            let args = encoder_args(
                encoder,
                RenderPurpose::Final,
                ExportEncodingSpeed::Fast,
                None,
            );
            assert!(
                args.iter().any(|arg| arg == expected),
                "{encoder} is missing {expected}: {args:?}"
            );
            // `-crf` is libx26x-only; passing it to a hardware encoder errors.
            assert!(!args.iter().any(|arg| arg == "-crf"), "{encoder}: {args:?}");
        }
    }

    #[test]
    fn hardware_encoders_honour_a_known_source_bitrate() {
        let args = encoder_args(
            "hevc_nvenc",
            RenderPurpose::Final,
            ExportEncodingSpeed::MatchSource,
            Some(2_000_000),
        );
        assert!(args.windows(2).any(|pair| pair == ["-b:v", "3000000"]));
        // HEVC in MP4 needs the hvc1 tag to play in QuickTime and Windows.
        assert!(args.windows(2).any(|pair| pair == ["-tag:v", "hvc1"]));
    }

    #[test]
    fn encoder_override_rejects_names_the_arg_builder_cannot_handle() {
        let previous = std::env::var_os("LUMEN_CUT_VIDEO_ENCODER");
        std::env::set_var("LUMEN_CUT_VIDEO_ENCODER", "h264_totally_made_up");
        let bogus = selected_encoder();
        std::env::set_var("LUMEN_CUT_VIDEO_ENCODER", "libx264");
        let valid = selected_encoder();
        match previous {
            Some(value) => std::env::set_var("LUMEN_CUT_VIDEO_ENCODER", value),
            None => std::env::remove_var("LUMEN_CUT_VIDEO_ENCODER"),
        }
        assert_ne!(bogus, "h264_totally_made_up");
        assert_eq!(valid, "libx264");
    }

    #[test]
    fn professional_presets_select_the_expected_encoders_and_audio_codecs() {
        let h264 = VideoExportSettings {
            encoding_speed: ExportEncodingSpeed::Quality,
            ..Default::default()
        };
        assert_eq!(encoder_for_settings(&h264).unwrap(), "libx264");

        let hevc = VideoExportSettings {
            video_codec: ExportVideoCodec::Hevc,
            encoding_speed: ExportEncodingSpeed::Quality,
            ..Default::default()
        };
        assert_eq!(encoder_for_settings(&hevc).unwrap(), "libx265");
        assert!(encoder_args(
            "libx265",
            RenderPurpose::Final,
            ExportEncodingSpeed::Quality,
            None
        )
        .windows(2)
        .any(|pair| pair == ["-tag:v", "hvc1"]));

        let prores = VideoExportSettings {
            container: crate::data::export_settings::ExportContainer::Mov,
            video_codec: ExportVideoCodec::Prores,
            audio_codec: ExportAudioCodec::Pcm,
            ..Default::default()
        };
        assert_eq!(encoder_for_settings(&prores).unwrap(), "prores_ks");
        assert!(encoder_args(
            "prores_ks",
            RenderPurpose::Final,
            ExportEncodingSpeed::Quality,
            None
        )
        .windows(2)
        .any(|pair| pair == ["-profile:v", "3"]));
        assert_eq!(audio_encoder(ExportAudioCodec::Aac), "aac");
        assert_eq!(audio_encoder(ExportAudioCodec::Pcm), "pcm_s16le");
    }

    async fn make_test_media(path: &Path) {
        crate::proc::run(
            "ffmpeg",
            &[
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=320x180:d=0.6",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=0.6",
                "-shortest",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                &path.display().to_string(),
            ],
        )
        .await
        .unwrap();
    }

    async fn stream_codecs(path: &Path) -> Vec<(String, String)> {
        let output = crate::proc::run(
            "ffprobe",
            &[
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type,codec_name",
                "-of",
                "json",
                &path.display().to_string(),
            ],
        )
        .await
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        value["streams"]
            .as_array()
            .unwrap()
            .iter()
            .map(|stream| {
                (
                    stream["codec_type"].as_str().unwrap().to_string(),
                    stream["codec_name"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn real_exports_cover_soft_caption_delivery_and_prores_pcm_mastering() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.mp4");
        make_test_media(&source).await;
        let mut media_doc = doc();
        media_doc.media.path = source;
        media_doc.media.duration_seconds = 0.6;
        media_doc.translations.insert(
            "zh-Hans".into(),
            std::collections::BTreeMap::from([(
                "s1".into(),
                crate::data::TranslationGroup {
                    id: "s1".into(),
                    text: "一二三".into(),
                    source_words: vec!["w0".into(), "w1".into(), "w2".into()],
                    source_text: Some("one two three".into()),
                },
            )]),
        );

        let srt = temp.path().join("captions.srt");
        let caption_doc =
            crate::data::export_settings::project_caption_doc(&media_doc, Some("zh-Hans"), true)
                .unwrap();
        crate::export::write_srt_with(&caption_doc, &[], &srt).unwrap();
        let soft_output = temp.path().join("soft.mp4");
        render_video_with_broll_options(
            &media_doc,
            &[],
            &temp.path().join("unused.ass"),
            &soft_output,
            &[],
            VideoRenderOptions {
                caption_overlay: None,
                purpose: RenderPurpose::Final,
                mode: None,
                on_progress: None,
                audio_mix: AudioMix::default(),
                settings: Some(VideoExportSettings {
                    encoding_speed: ExportEncodingSpeed::Quality,
                    resolution: crate::data::export_settings::ExportResolution::Hd720,
                    subtitle_mode: crate::data::export_settings::ExportSubtitleMode::Soft,
                    subtitle_language: Some("zh-Hans".into()),
                    bilingual_subtitles: true,
                    ..Default::default()
                }),
                soft_subtitle: Some(srt),
                include_ass: false,
                framings: Vec::new(),
            },
        )
        .await
        .unwrap();
        let soft_codecs = stream_codecs(&soft_output).await;
        assert!(soft_codecs.contains(&("video".into(), "h264".into())));
        assert!(soft_codecs.contains(&("audio".into(), "aac".into())));
        assert!(soft_codecs.contains(&("subtitle".into(), "mov_text".into())));
        let extracted_captions = crate::proc::run(
            "ffmpeg",
            &[
                "-hide_banner",
                "-loglevel",
                "error",
                "-i",
                &soft_output.display().to_string(),
                "-map",
                "0:s:0",
                "-f",
                "srt",
                "-",
            ],
        )
        .await
        .unwrap();
        assert!(extracted_captions.contains("one two three"));
        assert!(extracted_captions.contains("一二三"));
        let soft_media = crate::media::probe(&soft_output).await.unwrap();
        assert_eq!(soft_media.width.zip(soft_media.height), Some((1280, 720)));

        // Soft + source canvas + no edits → stream-copy remux (size ≈ source).
        let remux_srt = temp.path().join("remux.srt");
        crate::export::write_srt_with(&caption_doc, &[], &remux_srt).unwrap();
        let remux_output = temp.path().join("remux.mp4");
        let source_bytes = std::fs::metadata(&media_doc.media.path).unwrap().len();
        render_video_with_broll_options(
            &media_doc,
            &[],
            &temp.path().join("unused-remux.ass"),
            &remux_output,
            &[],
            VideoRenderOptions {
                caption_overlay: None,
                purpose: RenderPurpose::Final,
                mode: None,
                on_progress: None,
                audio_mix: AudioMix::default(),
                settings: Some(VideoExportSettings {
                    subtitle_mode: crate::data::export_settings::ExportSubtitleMode::Soft,
                    subtitle_language: Some("zh-Hans".into()),
                    bilingual_subtitles: true,
                    encoding_speed: ExportEncodingSpeed::MatchSource,
                    ..Default::default()
                }),
                soft_subtitle: Some(remux_srt),
                include_ass: false,
                framings: Vec::new(),
            },
        )
        .await
        .unwrap();
        let remux_bytes = std::fs::metadata(&remux_output).unwrap().len();
        // Remux must stay near source size (not 3–5× bloat from high-q re-encode).
        assert!(
            remux_bytes < source_bytes.saturating_mul(2).max(source_bytes + 200_000),
            "remux {remux_bytes} vs source {source_bytes}"
        );
        let remux_codecs = stream_codecs(&remux_output).await;
        assert!(remux_codecs.contains(&("subtitle".into(), "mov_text".into())));

        let music = temp.path().join("music.wav");
        let second_music = temp.path().join("second-music.wav");
        crate::proc::run(
            "ffmpeg",
            &[
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=880:duration=0.2",
                &music.display().to_string(),
            ],
        )
        .await
        .unwrap();
        crate::proc::run(
            "ffmpeg",
            &[
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=0.2",
                &second_music.display().to_string(),
            ],
        )
        .await
        .unwrap();
        let music_output = temp.path().join("music-mix.mp4");
        render_video_with_broll_options(
            &media_doc,
            &[],
            &temp.path().join("unused.ass"),
            &music_output,
            &[],
            VideoRenderOptions {
                caption_overlay: None,
                purpose: RenderPurpose::Final,
                mode: None,
                on_progress: None,
                audio_mix: AudioMix {
                    music: vec![
                        crate::data::audio_mix::MusicTrack {
                            id: "music-render-a".into(),
                            path: music,
                            start: 0.05,
                            end: 0.5,
                            source_start: 0.0,
                            volume: 0.25,
                            fade_in: 0.05,
                            fade_out: 0.05,
                            ducking: true,
                        },
                        crate::data::audio_mix::MusicTrack {
                            id: "music-render-b".into(),
                            path: second_music,
                            start: 0.1,
                            end: 0.55,
                            source_start: 0.0,
                            volume: 0.2,
                            fade_in: 0.05,
                            fade_out: 0.05,
                            ducking: false,
                        },
                    ],
                    ..Default::default()
                },
                settings: Some(VideoExportSettings {
                    encoding_speed: ExportEncodingSpeed::Quality,
                    subtitle_mode: crate::data::export_settings::ExportSubtitleMode::None,
                    ..Default::default()
                }),
                soft_subtitle: None,
                include_ass: false,
                framings: Vec::new(),
            },
        )
        .await
        .unwrap();
        assert!(stream_codecs(&music_output)
            .await
            .contains(&("audio".into(), "aac".into())));

        let portrait_output = temp.path().join("portrait.mp4");
        render_video_with_broll_options(
            &media_doc,
            &[],
            &temp.path().join("unused.ass"),
            &portrait_output,
            &[],
            VideoRenderOptions {
                caption_overlay: None,
                purpose: RenderPurpose::Final,
                mode: None,
                on_progress: None,
                audio_mix: AudioMix::default(),
                settings: Some(VideoExportSettings {
                    encoding_speed: ExportEncodingSpeed::Quality,
                    resolution: crate::data::export_settings::ExportResolution::Hd720,
                    aspect_ratio: crate::data::export_settings::ExportAspectRatio::Portrait9x16,
                    canvas_fit: ExportCanvasFit::Cover,
                    subtitle_mode: crate::data::export_settings::ExportSubtitleMode::None,
                    ..Default::default()
                }),
                soft_subtitle: None,
                include_ass: false,
                framings: Vec::new(),
            },
        )
        .await
        .unwrap();
        let portrait_media = crate::media::probe(&portrait_output).await.unwrap();
        assert_eq!(
            portrait_media.width.zip(portrait_media.height),
            Some((720, 1280))
        );

        let encoders = crate::proc::run(
            "ffmpeg",
            &["-hide_banner", "-loglevel", "error", "-encoders"],
        )
        .await
        .unwrap();
        if encoders.contains("libx265") {
            let hevc_output = temp.path().join("hevc.mp4");
            render_video_with_broll_options(
                &media_doc,
                &[],
                &temp.path().join("unused.ass"),
                &hevc_output,
                &[],
                VideoRenderOptions {
                    caption_overlay: None,
                    purpose: RenderPurpose::Final,
                    mode: None,
                    on_progress: None,
                    audio_mix: AudioMix::default(),
                    settings: Some(VideoExportSettings {
                        video_codec: ExportVideoCodec::Hevc,
                        encoding_speed: ExportEncodingSpeed::Quality,
                        subtitle_mode: crate::data::export_settings::ExportSubtitleMode::None,
                        ..Default::default()
                    }),
                    soft_subtitle: None,
                    include_ass: false,
                    framings: Vec::new(),
                },
            )
            .await
            .unwrap();
            assert!(stream_codecs(&hevc_output)
                .await
                .contains(&("video".into(), "hevc".into())));
        }

        let styled_ass = temp.path().join("styled.ass");
        let export_style = crate::data::substyle::SubStyle {
            fontname: "Arial".into(),
            fontsize: 72,
            primary_colour: "&H000000FF".into(),
            outline_colour: "&H00000000".into(),
            bold: true,
            alignment: 8,
            outline: 3,
            margin_v: 90,
            ..Default::default()
        };
        crate::export::write_ass_with_style(
            &media_doc,
            &[],
            &export_style,
            &styled_ass,
            1920,
            1080,
        )
        .unwrap();
        assert!(std::fs::read_to_string(&styled_ass)
            .unwrap()
            .contains("Style: Default,Arial,72,&H000000FF"));

        let master_output = temp.path().join("master.mov");
        render_video_with_broll_options(
            &media_doc,
            &[],
            &styled_ass,
            &master_output,
            &[],
            VideoRenderOptions {
                caption_overlay: None,
                purpose: RenderPurpose::Final,
                mode: None,
                on_progress: None,
                audio_mix: AudioMix::default(),
                settings: Some(VideoExportSettings {
                    container: crate::data::export_settings::ExportContainer::Mov,
                    video_codec: ExportVideoCodec::Prores,
                    audio_codec: ExportAudioCodec::Pcm,
                    ..Default::default()
                }),
                soft_subtitle: None,
                include_ass: true,
                framings: Vec::new(),
            },
        )
        .await
        .unwrap();
        let master_codecs = stream_codecs(&master_output).await;
        assert!(master_codecs.contains(&("video".into(), "prores".into())));
        assert!(master_codecs.contains(&("audio".into(), "pcm_s16le".into())));
    }
}
