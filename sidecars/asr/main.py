#!/usr/bin/env python3
"""lumen_cut_asr — Qwen3-ASR sidecar for local Apple-silicon transcription.

The sidecar runs `mlx_qwen3_asr.transcribe`
on a 16 kHz mono WAV and writes `asr_out.v1` JSON (paragraphs → sentences
→ words) that lumen-cut's `asr::AsrOutV1` parses.

Word-level timing comes from `Qwen3-ForcedAligner` when available
(`--align`). Without it the sidecar
emits one pseudo-word per segment (segment-level timing only).
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from typing import Any

# Defaults supported by the pinned mlx-qwen3-asr runtime.
DEFAULT_MODEL = "mlx-community/Qwen3-ASR-0.6B-8bit"
DEFAULT_ALIGNER = "mlx-community/Qwen3-ForcedAligner-0.6B-4bit"
MAX_CUE_CHARS_LATIN = 42
MAX_CUE_CHARS_CJK = 22
PROGRESS_PREFIX = "LUMEN_CUT_PROGRESS "


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description="transcribe audio with Qwen3-ASR (MLX)")
    p.add_argument("--audio", required=True, help="path to 16 kHz mono wav")
    p.add_argument("--model", default=DEFAULT_MODEL, help="Qwen3-ASR model id")
    p.add_argument("--language", default=None, help="language hint (e.g. Chinese, English)")
    p.add_argument("--align", default=None, nargs="?", const=DEFAULT_ALIGNER,
                   help="force-align words (Qwen3-ForcedAligner id); "
                        "pass no value for the default aligner")
    p.add_argument("--out", default="-", help="output path or '-' for stdout")
    p.add_argument(
        "--worker",
        choices=("recognize", "align"),
        default=None,
        help=argparse.SUPPRESS,
    )
    return p.parse_args()


def load_audio_duration(path: str) -> float:
    import wave
    with wave.open(path, "rb") as w:
        return w.getnframes() / float(w.getframerate() or 1)


def words_for_segment(text: str, start: float, end: float) -> list[dict[str, Any]]:
    """Pseudo word-level timing: evenly space non-whitespace atoms across
    the segment window. Adequate when no forced aligner is configured."""
    atoms = re.findall(r"\S+", text)
    if len(atoms) == 1 and re.search(r"[\u3400-\u9fff\u3040-\u30ff\uac00-\ud7af]", text):
        atoms = [c for c in text if not c.isspace()]
    atoms = atoms or [text]
    n = len(atoms)
    span = max(end - start, 0.0) / max(n, 1)
    return [
        {"text": a, "start": start + span * i, "end": start + span * (i + 1)}
        for i, a in enumerate(atoms)
    ]


def join_tokens(tokens: list[str]) -> str:
    """Join English words with spaces while keeping CJK and punctuation tight."""
    out = ""
    cjk = re.compile(r"[\u3400-\u9fff\u3040-\u30ff\uac00-\ud7af]")
    tight_left = set(",.!?;:%)]}，。！？；：、）】》")
    tight_right = set("([{（【《")
    for token in tokens:
        token = token.strip()
        if not token:
            continue
        needs_space = bool(out)
        if out and (cjk.search(out[-1]) or cjk.search(token[0])):
            needs_space = False
        if token[0] in tight_left or (out and out[-1] in tight_right):
            needs_space = False
        out += (" " if needs_space else "") + token
    return out


def max_cue_chars(language: str | None) -> int:
    value = (language or "").lower()
    return MAX_CUE_CHARS_CJK if value in {"chinese", "japanese", "korean", "zh", "ja", "ko"} else MAX_CUE_CHARS_LATIN


def build_paragraphs(
    segments: list[dict[str, Any]], language: str | None = None
) -> list[dict[str, Any]]:
    """Group word/alignment segments into subtitle-friendly sentences."""
    sentences: list[dict[str, Any]] = []
    cue_tokens: list[str] = []
    cue_words: list[dict[str, Any]] = []
    cue_start: float | None = None
    cue_end = 0.0

    def flush() -> None:
        nonlocal cue_tokens, cue_words, cue_start, cue_end
        text = join_tokens(cue_tokens)
        if text:
            sentences.append({"text": text, "words": cue_words})
        cue_tokens = []
        cue_words = []
        cue_start = None
        cue_end = 0.0

    for seg in segments:
        text = (seg.get("text") or "").strip()
        if not text:
            continue
        s = float(seg.get("start", 0.0))
        e = float(seg.get("end", s))
        words = seg.get("words") or words_for_segment(text, s, e)
        proposed = join_tokens([*cue_tokens, text])
        visible_chars = sum(not character.isspace() for character in proposed)
        if cue_tokens and (
            s - cue_end > 0.8
            or (cue_start is not None and e - cue_start > 6.0)
            or visible_chars > max_cue_chars(language)
        ):
            flush()
        if cue_start is None:
            cue_start = s
        cue_tokens.append(text)
        cue_words.extend(words)
        cue_end = e
        if text[-1:] in ".!?。！？":
            flush()
    flush()
    if not sentences:
        return []
    return [{"speaker": None, "sentences": sentences}]


def emit_progress(phase: str, progress: int, **details: Any) -> None:
    payload = {"phase": phase, "progress": max(0, min(100, int(progress))), **details}
    sys.stderr.write(PROGRESS_PREFIX + json.dumps(payload, ensure_ascii=False) + "\n")
    sys.stderr.flush()


def recognition_worker(args: argparse.Namespace) -> dict[str, Any]:
    from mlx_qwen3_asr import transcribe  # type: ignore

    def on_progress(event: dict[str, Any]) -> None:
        ratio = float(event.get("progress", 0.0) or 0.0)
        emit_progress(
            "transcribing",
            45 + round(ratio * 27),
            current=event.get("chunk_index"),
            total=event.get("total_chunks"),
        )

    result = transcribe(
        args.audio,
        model=args.model,
        language=args.language,
        return_timestamps=False,
        return_chunks=True,
        on_progress=on_progress,
    )
    return {
        "language": result.language or args.language,
        "text": result.text,
        "chunks": list(result.chunks or []),
    }


def alignment_worker(args: argparse.Namespace, recognized: dict[str, Any]) -> dict[str, Any]:
    import mlx.core as mx
    import numpy as np
    from mlx_qwen3_asr import ForcedAligner  # type: ignore
    from mlx_qwen3_asr.audio import SAMPLE_RATE, load_audio_np  # type: ignore

    # Keep reusable buffers small. Model weights remain resident, but this worker
    # never loads the ASR model, so both 0.6B models cannot occupy unified memory
    # at the same time.
    mx.set_cache_limit(256 * 1024 * 1024)
    audio = np.asarray(load_audio_np(args.audio, sr=SAMPLE_RATE), dtype=np.float32)
    chunks = list(recognized.get("chunks") or [])
    aligner = ForcedAligner(args.align)
    segments: list[dict[str, Any]] = []
    total = len(chunks)
    emit_progress("aligning", 72, current=0, total=total)

    for index, chunk in enumerate(chunks, start=1):
        text = str(chunk.get("text") or "").strip()
        start = max(0.0, float(chunk.get("start", 0.0) or 0.0))
        end = max(start, float(chunk.get("end", start) or start))
        language = str(
            chunk.get("language") or recognized.get("language") or args.language or ""
        ).strip()
        if text:
            start_sample = min(len(audio), max(0, round(start * SAMPLE_RATE)))
            end_sample = min(len(audio), max(start_sample, round(end * SAMPLE_RATE)))
            chunk_audio = audio[start_sample:end_sample]
            if language and language.lower() != "unknown" and len(chunk_audio):
                aligned = aligner.align(chunk_audio, text, language)
                segments.extend(
                    {
                        "text": item.text,
                        "start": item.start_time + start,
                        "end": item.end_time + start,
                    }
                    for item in aligned
                )
            else:
                segments.append({"text": text, "start": start, "end": end})
        mx.clear_cache()
        emit_progress(
            "aligning",
            72 + round(index / max(total, 1) * 15),
            current=index,
            total=total,
        )
    return {"segments": segments}


def _worker_command(
    python: str, stage: str, args: argparse.Namespace
) -> list[str]:
    command = [
        python,
        __file__,
        "--worker",
        stage,
        "--audio",
        args.audio,
        "--model",
        args.model,
    ]
    if args.language:
        command.extend(("--language", args.language))
    if stage == "align":
        command.extend(("--align", args.align))
    return command


def run_isolated_pipeline(
    args: argparse.Namespace,
    *,
    runner: Any = subprocess.run,
    python: str = sys.executable,
) -> dict[str, Any]:
    """Run recognition and word alignment in non-overlapping processes."""
    recognize = runner(
        _worker_command(python, "recognize", args),
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    recognized = json.loads(recognize.stdout)
    if not args.align:
        segments = [
            {
                "text": chunk.get("text", ""),
                "start": chunk.get("start", 0.0),
                "end": chunk.get("end", chunk.get("start", 0.0)),
            }
            for chunk in recognized.get("chunks", [])
            if str(chunk.get("text") or "").strip()
        ]
        return {**recognized, "segments": segments}

    align = runner(
        _worker_command(python, "align", args),
        input=json.dumps(recognized, ensure_ascii=False),
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    aligned = json.loads(align.stdout)
    return {**recognized, "segments": list(aligned.get("segments") or [])}


def main() -> int:
    args = parse_args()

    try:
        if args.worker == "recognize":
            payload = recognition_worker(args)
            sys.stdout.write(json.dumps(payload, ensure_ascii=False) + "\n")
            return 0
        if args.worker == "align":
            payload = alignment_worker(args, json.load(sys.stdin))
            sys.stdout.write(json.dumps(payload, ensure_ascii=False) + "\n")
            return 0
    except ImportError:
        sys.stderr.write(
            "lumen_cut_asr: mlx_qwen3_asr is not installed.\n"
            "  install with:  uv pip install mlx-qwen3-asr\n"
            "  or:            pip install mlx-qwen3-asr\n"
        )
        return 2

    try:
        result = run_isolated_pipeline(args)
    except Exception as e:  # noqa: BLE001
        sys.stderr.write(f"lumen_cut_asr: transcription failed: {e}\n")
        return 3

    segments = list(result.get("segments") or [])
    language = result.get("language") or args.language
    paragraphs = build_paragraphs(segments, language)
    duration = load_audio_duration(args.audio)
    out = {
        "schema_version": 1,
        "language": language,
        "duration_seconds": duration,
        "paragraphs": paragraphs,
    }

    payload = json.dumps(out, ensure_ascii=False, indent=2)
    if args.out == "-":
        sys.stdout.write(payload + "\n")
    else:
        with open(args.out, "w", encoding="utf-8") as f:
            f.write(payload)
    return 0


if __name__ == "__main__":
    sys.exit(main())
