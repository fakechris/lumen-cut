use std::process::Command;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lumen-cut-cli"))
}

#[test]
fn json_mode_keeps_tracing_off_stdout() {
    let temp = tempfile::tempdir().expect("tempdir");
    let media = temp.path().join("input.wav");
    let root = temp.path().join("projects");

    let generated = Command::new("ffmpeg")
        .args([
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=16000:cl=mono",
            "-t",
            "0.1",
        ])
        .arg(&media)
        .status()
        .expect("ffmpeg is a required runtime dependency");
    assert!(generated.success(), "failed to generate test media");

    let output = cli()
        .args(["--json", "project", "create", "demo", "--from"])
        .arg(&media)
        .arg("--root")
        .arg(&root)
        .output()
        .expect("run lumen-cut-cli");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must contain exactly one JSON value");
    assert_eq!(value["pid"], "demo");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("created project"),
        "the diagnostic should be preserved on stderr"
    );
}

#[test]
fn doctor_json_is_one_machine_readable_value() {
    let output = cli()
        .args(["--json", "doctor"])
        .output()
        .expect("run lumen-cut-cli");
    assert!(output.status.success());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor stdout must be JSON");
    assert!(value["checks"].is_array());
    assert!(value["total"].is_number());
}

#[cfg(unix)]
#[test]
fn diarize_progress_stays_on_stderr_while_json_stdout_remains_clean() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("audio.wav"), b"").expect("audio placeholder");
    std::fs::write(
        temp.path().join("doc.json"),
        r#"{
          "id":"demo","schema":1,
          "media":{"path":"input.wav","durationSeconds":1.0,"sampleRate":16000,"channels":1},
          "meta":{"title":"demo","description":"","language":"en",
            "createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z"},
          "paragraphs":[{
            "id":1,"speaker":null,"sentences":[{
              "id":"s1","text":"hello","words":[
                {"id":"w1","text":"hello","start":0.1,"end":0.9}
              ]
            }]
          }],
          "translations":{}
        }"#,
    )
    .expect("write doc");
    let stub = temp.path().join("diarize-stub.sh");
    std::fs::write(
        &stub,
        "#!/bin/sh\n\
         printf '%s\\n' 'LUMEN_CUT_PROGRESS {\"phase\":\"loading_model\",\"progress\":0,\"device\":\"mps\",\"cpu_percent\":25,\"peak_memory_mb\":250}' >&2\n\
         printf '%s\\n' 'LUMEN_CUT_PROGRESS {\"phase\":\"loading_model\",\"progress\":3,\"device\":\"mps\"}' >&2\n\
         printf '%s\\n' 'LUMEN_CUT_PROGRESS {\"phase\":\"loading_model\",\"progress\":100,\"device\":\"mps\"}' >&2\n\
         printf '%s' '{\"schema_version\":1,\"segments\":[{\"speaker\":\"SPEAKER_00\",\"start\":0.0,\"end\":1.0}]}'\n",
    )
    .expect("write sidecar stub");
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
        .expect("make sidecar executable");

    let output = cli()
        .args(["--json", "diarize"])
        .arg(temp.path())
        .env("LUMEN_CUT_PYTHON", &stub)
        .env("LUMEN_CUT_DIARIZE_SCRIPT", &stub)
        .output()
        .expect("run lumen-cut-cli diarize");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must contain one JSON value");
    assert_eq!(value["segments"], 1);
    assert_eq!(value["assigned"], 1);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("progress: loading model 0%"));
    assert!(stderr.contains("progress: loading model 100%"));
    assert!(
        !stderr.contains("loading model 3%"),
        "small progress changes should be throttled"
    );
}

#[cfg(unix)]
#[test]
fn auto_reports_post_processing_through_completed_without_polluting_json() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().expect("tempdir");
    let media = temp.path().join("input.wav");
    let generated = Command::new("ffmpeg")
        .args([
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=16000",
            "-t",
            "0.2",
        ])
        .arg(&media)
        .status()
        .expect("ffmpeg is a required runtime dependency");
    assert!(generated.success(), "failed to generate test audio");
    let stub = temp.path().join("asr-stub.sh");
    std::fs::write(
        &stub,
        "#!/bin/sh\n\
         printf '%s' '{\"schema_version\":1,\"language\":\"en\",\"duration_seconds\":0.2,\"paragraphs\":[{\"speaker\":null,\"sentences\":[{\"text\":\"hello\",\"words\":[{\"text\":\"hello\",\"start\":0.0,\"end\":0.2}]}]}]}'\n",
    )
    .expect("write ASR stub");
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
        .expect("make ASR stub executable");

    let output = cli()
        .args(["--json", "auto"])
        .arg(&media)
        .arg("--out")
        .arg(temp.path().join("out"))
        .env("LUMEN_CUT_PYTHON", &stub)
        .env("LUMEN_CUT_ASR_SCRIPT", &stub)
        .output()
        .expect("run lumen-cut-cli auto");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must contain one JSON value");
    assert_eq!(value["words"], 1);
    let stderr = String::from_utf8_lossy(&output.stderr);
    for phase in [
        "progress: extracting 15%",
        "progress: analyzing 35%",
        "progress: saving 90%",
        "progress: exporting 95%",
        "progress: completed 100%",
    ] {
        assert!(stderr.contains(phase), "missing {phase} in:\n{stderr}");
    }
}

#[test]
fn align_list_json_is_read_only_and_returns_only_over_fit_groups() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("projects");
    let project = root.join("demo");
    std::fs::create_dir_all(&project).expect("project directory");
    let doc_path = project.join("doc.json");
    std::fs::write(
        &doc_path,
        r#"{
          "id":"demo","schema":1,
          "media":{"path":"input.mp4","durationSeconds":2.0,"sampleRate":null,"channels":null},
          "meta":{"title":"demo","description":"","language":"en",
            "createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z"},
          "paragraphs":[],
          "translations":{"zh":{
            "short":{"id":"short","text":"短句","sourceWords":["w1"]},
            "long":{"id":"long","text":"这是一个明显超过八格的一行翻译","sourceWords":["w2"]}
          }}
        }"#,
    )
    .expect("write doc");
    let before = std::fs::read(&doc_path).expect("read doc before");

    let output = cli()
        .args([
            "--json", "align", "list", "demo", "--lang", "zh", "--fit", "8", "--root",
        ])
        .arg(&root)
        .output()
        .expect("run lumen-cut-cli align list");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("align list stdout must be JSON");
    assert_eq!(value["lang"], "zh");
    assert_eq!(value["fitChars"], 8);
    assert_eq!(value["groups"].as_array().map(Vec::len), Some(1));
    assert_eq!(value["groups"][0]["key"], "long");
    assert_eq!(
        std::fs::read(&doc_path).expect("read doc after"),
        before,
        "align list must not mutate the project"
    );
}

#[test]
fn cut_without_an_action_never_synthesizes_a_test_cut() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        temp.path().join("doc.json"),
        r#"{
          "id":"demo","schema":1,
          "media":{"path":"input.mp4","durationSeconds":1.0,"sampleRate":null,"channels":null},
          "meta":{"title":"demo","description":"","language":"en",
            "createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z"},
          "paragraphs":[],"translations":{}
        }"#,
    )
    .expect("write doc");

    let output = cli()
        .arg("cut")
        .arg(temp.path())
        .output()
        .expect("run lumen-cut-cli cut");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cut requires either --auto or --restore")
    );
    assert!(!temp.path().join("cuts.json").exists());
}

#[test]
fn speakers_view_renders_png_without_mutating_labels() {
    let temp = tempfile::tempdir().expect("tempdir");
    let wav = temp.path().join("audio.wav");
    let generated = Command::new("ffmpeg")
        .args([
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=16000",
            "-t",
            "1",
        ])
        .arg(&wav)
        .status()
        .expect("ffmpeg is a required runtime dependency");
    assert!(generated.success(), "failed to generate test audio");
    let doc_path = temp.path().join("doc.json");
    std::fs::write(
        &doc_path,
        r#"{
          "id":"demo","schema":1,
          "media":{"path":"audio.wav","durationSeconds":1.0,"sampleRate":16000,"channels":1},
          "meta":{"title":"demo","description":"","language":"en",
            "createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z"},
          "paragraphs":[{
            "id":1,"speaker":"Ada","sentences":[{
              "id":"s1","text":"hello","words":[
                {"id":"w1","text":"hello","start":0.1,"end":0.9}
              ]
            }]
          }],
          "translations":{}
        }"#,
    )
    .expect("write doc");
    let before = std::fs::read(&doc_path).expect("read doc before");

    let output = cli()
        .arg("--json")
        .arg("speakers")
        .arg(temp.path())
        .arg("view")
        .output()
        .expect("run speakers view");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("speakers view stdout must be JSON");
    assert_eq!(value["current"][0]["speaker"], "Ada");
    let png = std::fs::read(temp.path().join("speaker-view.png")).expect("diagnostic PNG");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(std::fs::read(&doc_path).expect("read doc after"), before);
}
