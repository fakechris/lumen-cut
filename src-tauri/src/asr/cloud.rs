use std::future::Future;
use std::path::Path;
use std::time::{Duration, Instant};

use futures::StreamExt;
use serde::Deserialize;

use crate::asr::{AsrOutV1, AsrParagraph, AsrProgress, AsrProgressCallback, AsrSentence, AsrWord};
use crate::error::{AppError, AppResult};

const CHUNK_SECONDS: f64 = 600.0;
const MAX_UPLOAD_BYTES: usize = 24 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_ERROR_BYTES: usize = 64 * 1024;

#[derive(Debug, Deserialize)]
struct TranscriptionResponse {
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    text: String,
    #[serde(default)]
    words: Vec<TranscriptionWord>,
}

#[derive(Debug, Deserialize)]
struct TranscriptionWord {
    #[serde(alias = "text")]
    word: String,
    start: f64,
    end: f64,
}

struct CloudRequestConfig<'a> {
    endpoint: &'a str,
    api_key: &'a str,
    model: &'a str,
    language: Option<&'a str>,
}

pub async fn transcribe_file(
    wav: &Path,
    duration_seconds: f64,
    endpoint: &str,
    api_key: &str,
    model: &str,
    language: Option<&str>,
    on_progress: Option<AsrProgressCallback>,
) -> AppResult<AsrOutV1> {
    validate_configuration(endpoint, api_key, model)?;
    if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        return Err(AppError::Schema(
            "cloud transcription requires a positive media duration".into(),
        ));
    }
    let total_chunks = (duration_seconds / CHUNK_SECONDS).ceil().max(1.0) as u32;
    let work_dir = wav
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".lumen-cut")
        .join("cloud-asr")
        .join(uuid::Uuid::new_v4().simple().to_string());
    if total_chunks > 1 {
        tokio::fs::create_dir_all(&work_dir).await?;
    }
    let config = CloudRequestConfig {
        endpoint,
        api_key,
        model,
        language,
    };
    let result = transcribe_chunks(
        wav,
        duration_seconds,
        total_chunks,
        &work_dir,
        &config,
        on_progress,
    )
    .await;
    if total_chunks > 1 {
        let _ = tokio::fs::remove_dir_all(&work_dir).await;
    }
    result
}

async fn transcribe_chunks(
    wav: &Path,
    duration_seconds: f64,
    total_chunks: u32,
    work_dir: &Path,
    config: &CloudRequestConfig<'_>,
    on_progress: Option<AsrProgressCallback>,
) -> AppResult<AsrOutV1> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(20 * 60))
        .build()
        .map_err(cloud_error)?;
    let started = Instant::now();
    let mut language_detected = None;
    let mut all_words = Vec::new();
    let mut previous_text = String::new();

    report_progress(
        &on_progress,
        started,
        "preparing",
        0,
        total_chunks,
        5,
    );

    for index in 0..total_chunks {
        ensure_not_cancelled()?;
        let offset = f64::from(index) * CHUNK_SECONDS;
        let chunk_duration = (duration_seconds - offset).min(CHUNK_SECONDS);
        let base = 10 + ((index * 80) / total_chunks.max(1)) as u8;
        let chunk_path = if total_chunks == 1 {
            wav.to_path_buf()
        } else {
            report_progress(
                &on_progress,
                started,
                "extracting",
                index,
                total_chunks,
                base,
            );
            let path = work_dir.join(format!("chunk-{index:05}.wav"));
            extract_chunk(wav, &path, offset, chunk_duration).await?;
            path
        };
        report_progress(
            &on_progress,
            started,
            "uploading",
            index,
            total_chunks,
            base.saturating_add(4).min(90),
        );
        let response = upload_chunk(
            &client,
            config,
            &chunk_path,
            (!previous_text.is_empty()).then_some(previous_text.as_str()),
        )
        .await;
        if total_chunks > 1 {
            let _ = tokio::fs::remove_file(&chunk_path).await;
        }
        report_progress(
            &on_progress,
            started,
            "transcribing",
            index,
            total_chunks,
            base.saturating_add(10).min(92),
        );
        let response = response?;
        if language_detected.is_none() {
            language_detected = response.language.filter(|value| !value.trim().is_empty());
        }
        if response.words.is_empty() {
            return Err(AppError::Schema(format!(
                "cloud transcription model `{}` returned no word timestamps; choose a model/provider that supports verbose_json with word timestamps",
                config.model
            )));
        }
        for word in response.words {
            validate_word(&word)?;
            all_words.push(AsrWord {
                text: word.word.trim().to_string(),
                start: word.start + offset,
                end: word.end + offset,
            });
        }
        previous_text = response
            .text
            .chars()
            .rev()
            .take(240)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        report_progress(
            &on_progress,
            started,
            "transcribing",
            index + 1,
            total_chunks,
            10 + (((index + 1) * 80) / total_chunks.max(1)) as u8,
        );
    }

    report_progress(
        &on_progress,
        started,
        "assembling",
        total_chunks,
        total_chunks,
        95,
    );
    let sentences = cue_sentences(all_words);
    if sentences.is_empty() {
        return Err(AppError::Schema(
            "cloud transcription returned no usable timed words".into(),
        ));
    }
    report_progress(
        &on_progress,
        started,
        "complete",
        total_chunks,
        total_chunks,
        100,
    );
    Ok(AsrOutV1 {
        schema_version: 1,
        language: config
            .language
            .or(language_detected.as_deref())
            .map(str::to_string),
        duration_seconds,
        paragraphs: vec![AsrParagraph {
            speaker: None,
            sentences,
        }],
    })
}

fn validate_configuration(endpoint: &str, api_key: &str, model: &str) -> AppResult<()> {
    let url = reqwest::Url::parse(endpoint.trim())
        .map_err(|_| AppError::Schema("cloud transcription endpoint is invalid".into()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::Schema(
            "cloud transcription endpoint must use http or https".into(),
        ));
    }
    if api_key.trim().is_empty() {
        return Err(AppError::Schema(
            "cloud transcription API key is not configured".into(),
        ));
    }
    if model.trim().is_empty() {
        return Err(AppError::Schema(
            "cloud transcription model is not configured".into(),
        ));
    }
    Ok(())
}

async fn extract_chunk(wav: &Path, output: &Path, offset: f64, duration: f64) -> AppResult<()> {
    let input = wav.display().to_string();
    let output = output.display().to_string();
    crate::proc::run(
        "ffmpeg",
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-ss",
            &format!("{offset:.6}"),
            "-t",
            &format!("{duration:.6}"),
            "-i",
            &input,
            "-vn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_s16le",
            &output,
        ],
    )
    .await?;
    Ok(())
}

async fn upload_chunk(
    client: &reqwest::Client,
    config: &CloudRequestConfig<'_>,
    path: &Path,
    prompt: Option<&str>,
) -> AppResult<TranscriptionResponse> {
    let bytes = tokio::fs::read(path).await?;
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(AppError::Schema(format!(
            "cloud transcription chunk is {} MB, above the safe 24 MB upload limit",
            bytes.len().div_ceil(1024 * 1024)
        )));
    }
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(cloud_error)?;
    let mut form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", config.model.trim().to_string())
        .text("response_format", "verbose_json")
        .text("timestamp_granularities[]", "word")
        .text("timestamp_granularities[]", "segment");
    if let Some(language) = config.language.filter(|value| !value.trim().is_empty()) {
        form = form.text("language", language.trim().to_string());
    }
    if let Some(prompt) = prompt.filter(|value| !value.trim().is_empty()) {
        form = form.text("prompt", prompt.to_string());
    }
    let request = client
        .post(config.endpoint.trim())
        .bearer_auth(config.api_key.trim())
        .multipart(form);
    let response = cancellable(async { request.send().await.map_err(cloud_error) }).await?;
    let status = response.status();
    let limit = if status.is_success() {
        MAX_RESPONSE_BYTES
    } else {
        MAX_ERROR_BYTES
    };
    let body = read_bounded(response, limit).await?;
    if !status.is_success() {
        let detail = String::from_utf8_lossy(&body);
        return Err(AppError::Sidecar {
            sidecar: "cloud_asr",
            message: format!("provider returned HTTP {status}: {}", detail.trim()),
        });
    }
    serde_json::from_slice(&body).map_err(|error| AppError::Sidecar {
        sidecar: "cloud_asr",
        message: format!("invalid transcription response: {error}"),
    })
}

async fn read_bounded(response: reqwest::Response, limit: usize) -> AppResult<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(AppError::Sidecar {
            sidecar: "cloud_asr",
            message: format!("provider response exceeded the {limit} byte safety limit"),
        });
    }
    cancellable(async move {
        let mut stream = response.bytes_stream();
        let mut body = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(cloud_error)?;
            if body.len() + chunk.len() > limit {
                return Err(AppError::Sidecar {
                    sidecar: "cloud_asr",
                    message: format!("provider response exceeded the {limit} byte safety limit"),
                });
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    })
    .await
}

async fn cancellable<F, T>(future: F) -> AppResult<T>
where
    F: Future<Output = AppResult<T>>,
{
    tokio::pin!(future);
    loop {
        tokio::select! {
            result = &mut future => return result,
            _ = tokio::time::sleep(Duration::from_millis(100)) => ensure_not_cancelled()?,
        }
    }
}

fn ensure_not_cancelled() -> AppResult<()> {
    if crate::proc::cancellation_requested() {
        Err(AppError::Cancelled)
    } else {
        Ok(())
    }
}

fn cloud_error(error: reqwest::Error) -> AppError {
    AppError::Sidecar {
        sidecar: "cloud_asr",
        message: error.to_string(),
    }
}

fn validate_word(word: &TranscriptionWord) -> AppResult<()> {
    if word.word.trim().is_empty()
        || !word.start.is_finite()
        || !word.end.is_finite()
        || word.start < 0.0
        || word.end <= word.start
    {
        return Err(AppError::Schema(
            "cloud transcription returned an invalid timed word".into(),
        ));
    }
    Ok(())
}

fn cue_sentences(words: Vec<AsrWord>) -> Vec<AsrSentence> {
    let mut sentences = Vec::new();
    let mut cue = Vec::new();
    for word in words {
        let should_flush = cue.last().is_some_and(|previous: &AsrWord| {
            word.start - previous.end > 0.8
                || word.end
                    - cue
                        .first()
                        .map(|word: &AsrWord| word.start)
                        .unwrap_or(word.start)
                    > 6.0
                || visible_chars_with(&cue, &word.text) > 42
        });
        if should_flush {
            push_cue(&mut sentences, std::mem::take(&mut cue));
        }
        let ends_sentence = word
            .text
            .trim_end()
            .ends_with(['.', '?', '!', '。', '？', '！']);
        cue.push(word);
        if ends_sentence {
            push_cue(&mut sentences, std::mem::take(&mut cue));
        }
    }
    push_cue(&mut sentences, cue);
    sentences
}

fn visible_chars_with(words: &[AsrWord], next: &str) -> usize {
    words
        .iter()
        .map(|word| {
            word.text
                .chars()
                .filter(|character| !character.is_whitespace())
                .count()
        })
        .sum::<usize>()
        + next
            .chars()
            .filter(|character| !character.is_whitespace())
            .count()
}

fn push_cue(sentences: &mut Vec<AsrSentence>, words: Vec<AsrWord>) {
    if words.is_empty() {
        return;
    }
    let text = join_tokens(words.iter().map(|word| word.text.as_str()));
    if !text.is_empty() {
        sentences.push(AsrSentence { text, words });
    }
}

fn join_tokens<'a>(tokens: impl Iterator<Item = &'a str>) -> String {
    let mut output = String::new();
    for token in tokens {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let first = token.chars().next().unwrap_or_default();
        let last = output.chars().last();
        let tight_left = ",.!?;:%)]}，。！？；：、）】》".contains(first);
        let tight_right = last.is_some_and(|character| "([{（【《".contains(character));
        let cjk_boundary = last.is_some_and(is_cjk) || is_cjk(first);
        if !output.is_empty() && !tight_left && !tight_right && !cjk_boundary {
            output.push(' ');
        }
        output.push_str(token);
    }
    output
}

fn is_cjk(character: char) -> bool {
    matches!(
        character as u32,
        0x3400..=0x9fff | 0x3040..=0x30ff | 0xac00..=0xd7af
    )
}

fn report_progress(
    callback: &Option<AsrProgressCallback>,
    started: Instant,
    phase: &str,
    current: u32,
    total: u32,
    progress: u8,
) {
    if let Some(callback) = callback {
        callback(AsrProgress {
            phase: phase.into(),
            progress,
            current: Some(current),
            total: Some(total),
            device: Some("cloud".into()),
            elapsed_seconds: Some(started.elapsed().as_secs_f64()),
            cpu_percent: None,
            peak_memory_mb: None,
            memory_limit_mb: None,
            mlx_active_memory_mb: None,
            mlx_cache_memory_mb: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Bytes;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::post;
    use axum::Router;

    #[test]
    fn cloud_words_become_bounded_caption_cues_without_fake_timing() {
        let words = vec![
            AsrWord {
                text: "Hello".into(),
                start: 0.0,
                end: 0.4,
            },
            AsrWord {
                text: "world.".into(),
                start: 0.4,
                end: 0.9,
            },
            AsrWord {
                text: "Next".into(),
                start: 2.0,
                end: 2.4,
            },
        ];
        let cues = cue_sentences(words);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "Hello world.");
        assert_eq!(cues[1].words[0].start, 2.0);
    }

    #[test]
    fn invalid_or_untimed_words_are_rejected() {
        assert!(validate_word(&TranscriptionWord {
            word: "bad".into(),
            start: 2.0,
            end: 1.0,
        })
        .is_err());
        assert!(validate_configuration("file:///tmp/api", "key", "model").is_err());
    }

    #[tokio::test]
    async fn compatible_endpoint_receives_multipart_and_returns_real_word_timing() {
        async fn transcribe(headers: HeaderMap, body: Bytes) -> (StatusCode, String) {
            assert_eq!(
                headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok()),
                Some("Bearer secret")
            );
            let body = String::from_utf8_lossy(&body);
            assert!(body.contains("whisper-1"));
            assert!(body.contains("verbose_json"));
            assert!(body.contains("timestamp_granularities[]"));
            (
                StatusCode::OK,
                serde_json::json!({
                    "language": "en",
                    "text": "Hello world.",
                    "words": [
                        {"word": "Hello", "start": 0.0, "end": 0.4},
                        {"word": "world.", "start": 0.4, "end": 0.9}
                    ]
                })
                .to_string(),
            )
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/transcribe", post(transcribe)),
            )
            .await
            .unwrap();
        });
        let temp = tempfile::tempdir().unwrap();
        let wav = temp.path().join("audio.wav");
        tokio::fs::write(&wav, b"test audio").await.unwrap();
        let output = transcribe_file(
            &wav,
            5.0,
            &format!("http://{address}/transcribe"),
            "secret",
            "whisper-1",
            Some("en"),
            None,
        )
        .await
        .unwrap();
        server.abort();

        assert_eq!(output.language.as_deref(), Some("en"));
        assert_eq!(output.paragraphs[0].sentences[0].text, "Hello world.");
        assert_eq!(output.paragraphs[0].sentences[0].words[1].end, 0.9);
    }
}
