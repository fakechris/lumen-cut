#![cfg(unix)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lumen-cut-cli"))
}

fn assert_success(output: &Output, label: &str) {
    assert!(
        output.status.success(),
        "{label} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn json_output(output: Output, label: &str) -> serde_json::Value {
    assert_success(&output, label);
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{label} did not return JSON: {error}"))
}

fn write_executable(path: &Path, body: &str) {
    std::fs::write(path, body).expect("write executable fixture");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("mark fixture executable");
}

fn start_openai_fixture() -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind provider fixture");
    let endpoint = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().expect("provider address")
    );
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept provider request");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("provider read timeout");
        let mut request = vec![0_u8; 64 * 1024];
        let _ = stream.read(&mut request).expect("read provider request");
        let answer = serde_json::json!({
            "summary": "Two speakers exchange a short greeting.",
            "terms": [],
            "namedEntities": [],
            "translations": {
                "p1s1": "嗯，你好，世界",
                "p1s2": "第二位说话人",
            },
        })
        .to_string();
        let event = serde_json::json!({
            "choices": [{"delta": {"content": answer}}],
        });
        let body = format!("data: {event}\n\ndata: [DONE]\n\n");
        write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("write provider response");
        stream.flush().expect("flush provider response");
    });
    (endpoint, handle)
}

#[test]
fn real_media_workflow_persists_ai_edits_and_exports_playable_video() {
    let temp = tempfile::tempdir().expect("tempdir");
    let media = temp.path().join("interview.mp4");
    let broll = temp.path().join("broll.png");
    let projects = temp.path().join("projects");
    let project = projects.join("interview");

    let generated = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=24:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=16000:duration=2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
        ])
        .arg(&media)
        .status()
        .expect("ffmpeg is a required runtime dependency");
    assert!(generated.success(), "failed to generate workflow media");
    let generated = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:size=160x90:duration=1",
            "-frames:v",
            "1",
        ])
        .arg(&broll)
        .status()
        .expect("ffmpeg is a required runtime dependency");
    assert!(generated.success(), "failed to generate B-roll fixture");

    let asr = temp.path().join("asr-stub.sh");
    write_executable(
        &asr,
        "#!/bin/sh\n\
         printf '%s\\n' 'LUMEN_CUT_PROGRESS {\"phase\":\"loading_model\",\"progress\":100,\"device\":\"mps\",\"cpu_percent\":18,\"peak_memory_mb\":256}' >&2\n\
         printf '%s\\n' 'LUMEN_CUT_PROGRESS {\"phase\":\"transcribing\",\"progress\":50,\"device\":\"mps\",\"cpu_percent\":42,\"peak_memory_mb\":384}' >&2\n\
         printf '%s' '{\"schema_version\":1,\"language\":\"en\",\"duration_seconds\":2.0,\"paragraphs\":[{\"speaker\":null,\"sentences\":[{\"text\":\"um hello world\",\"words\":[{\"text\":\"um\",\"start\":0.1,\"end\":0.3},{\"text\":\"hello\",\"start\":0.35,\"end\":0.8},{\"text\":\"world\",\"start\":0.85,\"end\":1.0}]},{\"text\":\"second speaker\",\"words\":[{\"text\":\"second\",\"start\":1.1,\"end\":1.45},{\"text\":\"speaker\",\"start\":1.5,\"end\":1.9}]}]}]}'\n",
    );
    let diarize = temp.path().join("diarize-stub.sh");
    write_executable(
        &diarize,
        "#!/bin/sh\n\
         printf '%s\\n' 'LUMEN_CUT_PROGRESS {\"phase\":\"analyzing_speakers\",\"progress\":50,\"device\":\"mps\",\"cpu_percent\":50,\"peak_memory_mb\":420}' >&2\n\
         printf '%s' '{\"schema_version\":1,\"segments\":[{\"speaker\":\"SPEAKER_00\",\"start\":0.0,\"end\":1.05},{\"speaker\":\"SPEAKER_01\",\"start\":1.05,\"end\":2.0}]}'\n",
    );

    let auto = cli()
        .args(["--json", "auto"])
        .arg(&media)
        .arg("--out")
        .arg(&projects)
        .arg("--title")
        .arg("E2E Interview")
        .env("LUMEN_CUT_PYTHON", &asr)
        .env("LUMEN_CUT_ASR_SCRIPT", &asr)
        .output()
        .expect("run auto pipeline");
    let auto = json_output(auto, "auto pipeline");
    assert_eq!(auto["words"], 5);
    assert_eq!(auto["paragraphs"], 1);
    assert!(project.join("out.srt").is_file());

    let diarized = cli()
        .args(["--json", "diarize"])
        .arg(&project)
        .env("LUMEN_CUT_PYTHON", &diarize)
        .env("LUMEN_CUT_DIARIZE_SCRIPT", &diarize)
        .output()
        .expect("run diarization");
    let diarized = json_output(diarized, "diarization");
    assert_eq!(diarized["segments"], 2);
    assert_eq!(diarized["assigned"], 2);

    let (endpoint, provider) = start_openai_fixture();
    let home = temp.path().join("home");
    std::fs::create_dir_all(home.join(".lumen-cut")).expect("settings directory");
    std::fs::write(
        home.join(".lumen-cut/settings.json"),
        serde_json::json!({
            "llmEndpoint": endpoint,
            "llmApiKey": "",
            "llmModel": "e2e-fixture",
            "workerCount": 1,
        })
        .to_string(),
    )
    .expect("provider settings");
    let translated = cli()
        .args([
            "--json",
            "task",
            "start",
            "translate",
            "interview",
            "--lang",
            "zh-Hans",
            "--root",
        ])
        .arg(&projects)
        .env("HOME", &home)
        .output()
        .expect("run translation task");
    let translated = json_output(translated, "translation task");
    assert_eq!(translated["pending"], 1);
    provider.join().expect("provider fixture");

    let status = json_output(
        cli()
            .args(["--json", "task", "status", "interview", "--root"])
            .arg(&projects)
            .output()
            .expect("read task status"),
        "task status",
    );
    assert_eq!(status["pending"], 0);
    assert_eq!(status["done"], 1);
    assert_eq!(status["failed"], 0);
    assert_eq!(status["kinds"][0]["state"], "completed");

    let translated_cues = json_output(
        cli()
            .args(["--json", "subtitle"])
            .arg(&project)
            .args(["list", "--lang", "zh-Hans"])
            .output()
            .expect("list translated subtitles"),
        "translated subtitles",
    );
    assert_eq!(translated_cues[0]["text"], "嗯，你好，世界");
    assert_eq!(translated_cues[1]["speaker"], "SPEAKER_01");

    for (label, output) in [
        (
            "speaker rename",
            cli()
                .args(["--json", "speakers"])
                .arg(&project)
                .args(["rename", "SPEAKER_00", "Host"])
                .output()
                .expect("rename speaker"),
        ),
        (
            "subtitle edit",
            cli()
                .args(["--json", "subtitle"])
                .arg(&project)
                .args(["replace", "world", "universe"])
                .output()
                .expect("replace subtitle text"),
        ),
        (
            "automatic cut",
            cli()
                .args(["--json", "cut"])
                .arg(&project)
                .arg("--auto")
                .output()
                .expect("apply automatic cut"),
        ),
        (
            "version commit",
            cli()
                .args([
                    "--json",
                    "version",
                    "commit",
                    "interview",
                    "E2E edit",
                    "--root",
                ])
                .arg(&projects)
                .output()
                .expect("commit project version"),
        ),
    ] {
        assert_success(&output, label);
    }

    let broll_added = cli()
        .args(["--json", "broll"])
        .arg(&project)
        .args(["add", "--file"])
        .arg(&broll)
        .args([
            "--start",
            "0.5",
            "--end",
            "1.2",
            "--mode",
            "pip",
            "--rect",
            "218,14,90,50",
            "--radius",
            "8",
            "--name",
            "E2E B-roll",
        ])
        .output()
        .expect("add B-roll");
    let broll_added = json_output(broll_added, "B-roll add");
    assert_eq!(broll_added["mode"], "pip");

    let exported = json_output(
        cli()
            .args(["--json", "export"])
            .arg(&project)
            .arg("--video")
            .env("LUMEN_CUT_VIDEO_ENCODER", "libx264")
            .output()
            .expect("export project"),
        "video export",
    );
    assert_eq!(exported["cuts"], 1);
    let output = project.join("export.mp4");
    assert!(output.is_file());
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,codec_name",
            "-of",
            "json",
        ])
        .arg(&output)
        .output()
        .expect("probe exported video");
    let probe = json_output(probe, "ffprobe");
    let streams = probe["streams"].as_array().expect("export streams");
    assert!(streams
        .iter()
        .any(|stream| stream["codec_type"] == "video" && stream["codec_name"] == "h264"));
    assert!(streams
        .iter()
        .any(|stream| stream["codec_type"] == "audio" && stream["codec_name"] == "aac"));

    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(project.join("doc.json")).expect("doc.json"))
            .expect("parse doc.json");
    assert_eq!(doc["paragraphs"][0]["speaker"], "Host");
    assert_eq!(
        doc["paragraphs"][0]["sentences"][0]["text"],
        "um hello universe"
    );
    assert_eq!(
        doc["translations"]["zh-Hans"]["p1s2"]["text"],
        "第二位说话人"
    );
    let task: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(project.join("ai/translate/task.json"))
            .expect("translation task manifest"),
    )
    .expect("parse task manifest");
    assert_eq!(task["state"], "completed");
    assert!(project.join("versions/v0/doc.json").is_file());
}
