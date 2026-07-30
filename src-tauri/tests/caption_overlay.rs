//! End-to-end test for the WYSIWYG caption overlay: a sealed caption-frame
//! manifest + PNG states are burned into a real export with the production
//! ffmpeg path (render_video_with_broll_options), and the output frames are
//! probed to confirm each state lands in its time window with alpha composited.
//!
//! Run with: cargo test --test caption_overlay

use std::path::Path;
use std::process::Command;

use lumen_cut::export::caption_frames::{write_concat, CaptionManifest, CaptionManifestSegment};
use lumen_cut::export::video::{
    render_video_with_broll_options, RenderPurpose, VideoRenderOptions,
};

fn ffmpeg(args: &[&str]) {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .expect("ffmpeg runs");
    assert!(
        output.status.success(),
        "ffmpeg {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// One full-canvas 1920×1080 PNG in the given lavfi color (the caption
/// states for this test are full-frame tints so luma assertions are
/// unambiguous; transparent = black@0.0).
fn make_caption_png(path: &Path, color: &str) {
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("color=c={color}:size=1920x1080,format=rgba"),
        "-frames:v",
        "1",
        path.to_str().unwrap(),
    ]);
}

/// Average luma of the whole frame in the output at `seconds`.
fn caption_region_luma(video: &Path, seconds: f64) -> f64 {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "info"])
        .args([
            "-ss",
            &seconds.to_string(),
            "-i",
            video.to_str().unwrap(),
            "-vf",
            "signalstats,metadata=print:key=lavfi.signalstats.YAVG:file=-",
            "-f",
            "null",
            "-",
        ])
        .output()
        .expect("ffmpeg probe runs");
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text
        .lines()
        .find(|line| line.contains("YAVG="))
        .unwrap_or_else(|| panic!("no YAVG in {text}"));
    line.split("YAVG=").nth(1).unwrap().trim().parse().unwrap()
}

fn doc() -> lumen_cut::data::Doc {
    lumen_cut::data::Doc {
        id: "overlay-e2e".into(),
        schema: 1,
        media: lumen_cut::data::MediaRef {
            path: Path::new("/tmp/nonexistent.mp4").into(),
            duration_seconds: 6.0,
            sample_rate: Some(48_000),
            channels: Some(1),
        },
        meta: lumen_cut::data::Meta {
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

#[tokio::test]
async fn caption_overlay_burns_each_state_in_its_window() {
    if Command::new("ffmpeg").arg("-version").output().is_err() {
        eprintln!("ffmpeg unavailable; skipping caption overlay e2e");
        return;
    }
    let dir = std::env::temp_dir().join(format!("caption-overlay-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let frames = dir.join("frames");
    std::fs::create_dir_all(&frames).unwrap();

    // Source video: mid-gray 1920×1080, 6 s, with a tone.
    let source = dir.join("source.mp4");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=1920x1080:rate=30",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=48000",
        "-t",
        "6",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        source.to_str().unwrap(),
    ]);

    // Three caption states: red [0,2), lime [2,4), transparent [4,6).
    make_caption_png(&frames.join("red.png"), "red");
    make_caption_png(&frames.join("green.png"), "lime");
    make_caption_png(&frames.join("empty.png"), "black@0.0");
    let manifest = CaptionManifest {
        version: 1,
        hash: "e2e".into(),
        width: 1920,
        height: 1080,
        duration: 6.0,
        segments: vec![
            CaptionManifestSegment {
                file: "red.png".into(),
                start: 0.0,
                end: 2.0,
            },
            CaptionManifestSegment {
                file: "green.png".into(),
                start: 2.0,
                end: 4.0,
            },
            CaptionManifestSegment {
                file: "empty.png".into(),
                start: 4.0,
                end: 6.0,
            },
        ],
    };
    let concat = write_concat(&manifest, &frames).unwrap();

    let output = dir.join("out.mp4");
    let mut render_doc = doc();
    render_doc.media.path = source.clone();
    render_video_with_broll_options(
        &render_doc,
        &[],
        &dir.join("unused.ass"),
        &output,
        &[],
        VideoRenderOptions {
            purpose: RenderPurpose::Final,
            mode: None,
            on_progress: None,
            audio_mix: Default::default(),
            settings: None,
            soft_subtitle: None,
            include_ass: false,
            framings: vec![],
            caption_overlay: Some(concat),
        },
    )
    .await
    .expect("overlay export renders");

    // Red block at t=1 (Y of red ≈ 76), green at t=3 (Y of 0,200,0 ≈ 118,
    // well above gray testsrc midtones), source only at t=5 (no block).
    let red_y = caption_region_luma(&output, 1.0);
    let green_y = caption_region_luma(&output, 3.0);
    let tail_y = caption_region_luma(&output, 5.0);
    assert!(
        red_y < 110.0,
        "red state should dominate at t=1 (YAVG {red_y})"
    );
    assert!(
        green_y > red_y + 20.0,
        "green state should replace red at t=3 (red {red_y}, green {green_y})"
    );
    assert!(
        (tail_y - green_y).abs() > 10.0,
        "transparent state should restore the source at t=5 (tail {tail_y}, green {green_y})"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
