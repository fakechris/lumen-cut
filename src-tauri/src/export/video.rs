//! Cut-aware video export. The picture and audio are trimmed/concatenated
//! before the already-retimed ASS captions are burned in.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::data::audio_mix::AudioMix;
use crate::data::broll::{BackgroundMode, BrollPlacement, FitMode, PlacementMode, Rect};
use crate::data::export_settings::{
    ExportAudioCodec, ExportCanvasFit, ExportEncodingSpeed, ExportVideoCodec, VideoExportSettings,
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoFilter {
    pub filter_complex: String,
    pub audio_map: Option<String>,
    pub broll_inputs: Vec<PathBuf>,
    pub music_inputs: Vec<PathBuf>,
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
        VideoCanvas::default(),
    )
}

fn build_video_filter_inner(
    doc: &Doc,
    cuts: &[Cut],
    ass: Option<&Path>,
    placements: &[BrollPlacement],
    audio_mix: &AudioMix,
    canvas: VideoCanvas,
) -> AppResult<VideoFilter> {
    let mut graph = String::new();
    let output_duration: f64 = super::project::kept_intervals(doc, cuts)
        .iter()
        .map(|(start, end)| end - start)
        .sum();
    audio_mix.validate(output_duration)?;
    let mut audio_map;
    let mut dialogue_source = None;
    if cuts.is_empty() {
        graph.push_str("[0:v]setpts=PTS-STARTPTS[vbase];");
        audio_map = Some("0:a:0?".into());
        if doc.media.channels.is_some_and(|channels| channels > 0) {
            dialogue_source = Some("0:a".to_string());
        }
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
    let mut current = if let Some((width, height)) = canvas.output_dimensions {
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
    })
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
        audio_mix,
        settings,
        soft_subtitle,
        include_ass,
    } = options;
    let settings = settings.unwrap_or_else(|| VideoExportSettings {
        encoding_speed: match mode.as_deref() {
            Some("quality") => ExportEncodingSpeed::Quality,
            _ => ExportEncodingSpeed::Fast,
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
    let source_info = crate::media::probe(&doc.media.path).await?;
    let source_dimensions = source_info.width.zip(source_info.height);
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
        VideoCanvas {
            frame_size,
            output_dimensions,
            fit: settings.canvas_fit,
        },
    )?;
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
    let soft_subtitle_input = soft_subtitle.as_ref().map(|path| {
        let index = 1 + filter.broll_inputs.len() + filter.music_inputs.len();
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
    let output_duration: f64 = super::project::kept_intervals(doc, cuts)
        .iter()
        .map(|(start, end)| end - start)
        .sum();
    let encoder = encoder_for_settings(&settings)?;
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
        VideoCanvas {
            frame_size,
            ..Default::default()
        },
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

pub fn encoder_for_settings(settings: &VideoExportSettings) -> AppResult<String> {
    settings.validate()?;
    match settings.video_codec {
        ExportVideoCodec::H264 => match settings.encoding_speed {
            ExportEncodingSpeed::Fast => Ok(selected_encoder()),
            ExportEncodingSpeed::Quality => Ok("libx264".into()),
        },
        ExportVideoCodec::Hevc => match settings.encoding_speed {
            ExportEncodingSpeed::Fast if cfg!(target_os = "macos") => {
                Ok("hevc_videotoolbox".into())
            }
            ExportEncodingSpeed::Fast | ExportEncodingSpeed::Quality => Ok("libx265".into()),
        },
        ExportVideoCodec::Prores => Ok("prores_ks".into()),
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
    if matches!(encoder, "h264_videotoolbox" | "hevc_videotoolbox") {
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
            "-q:v".into(),
            quality.into(),
        ];
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
    } else if encoder == "libx264" || encoder == "libx265" {
        let (preset, crf) = if purpose == RenderPurpose::Preview {
            ("veryfast", "23")
        } else {
            ("medium", "18")
        };
        let mut args = vec![
            "-c:v".into(),
            encoder.into(),
            "-preset".into(),
            preset.into(),
            "-crf".into(),
            crf.into(),
        ];
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
            VideoCanvas {
                frame_size: Some((1280, 720)),
                ..Default::default()
            },
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
            VideoCanvas {
                frame_size: Some((1920, 1080)),
                output_dimensions: Some((1920, 1080)),
                ..Default::default()
            },
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
            VideoCanvas {
                frame_size: Some((1080, 1920)),
                output_dimensions: Some((1080, 1920)),
                fit: ExportCanvasFit::Cover,
            },
        )
        .unwrap();

        let crop = plan.filter_complex.find("crop=1080:1920").unwrap();
        let broll = plan.filter_complex.find("[1:v]trim=").unwrap();
        assert!(crop < broll);
        assert!(plan.filter_complex.contains("overlay=x=540:y=960"));
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
        assert!(encoder_args("libx265", RenderPurpose::Final)
            .windows(2)
            .any(|pair| pair == ["-tag:v", "hvc1"]));

        let prores = VideoExportSettings {
            container: crate::data::export_settings::ExportContainer::Mov,
            video_codec: ExportVideoCodec::Prores,
            audio_codec: ExportAudioCodec::Pcm,
            ..Default::default()
        };
        assert_eq!(encoder_for_settings(&prores).unwrap(), "prores_ks");
        assert!(encoder_args("prores_ks", RenderPurpose::Final)
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
            },
        )
        .await
        .unwrap();
        let master_codecs = stream_codecs(&master_output).await;
        assert!(master_codecs.contains(&("video".into(), "prores".into())));
        assert!(master_codecs.contains(&("audio".into(), "pcm_s16le".into())));
    }
}
