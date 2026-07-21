//! Re-transcribe pipeline. Wraps `crate::asr` for
//! the `--re-transcribe` flow (used after a soft-cut restore + renumber).

use crate::asr::{transcribe_file, AsrOutV1};
use crate::error::AppResult;
use std::path::Path;

pub async fn re_transcribe(wav: &Path, model: &str, lang: Option<&str>) -> AppResult<AsrOutV1> {
    transcribe_file(wav, model, lang).await
}
