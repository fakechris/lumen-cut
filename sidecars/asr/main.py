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
import sys
from typing import Any

# Defaults supported by the pinned mlx-qwen3-asr runtime.
DEFAULT_MODEL = "Qwen/Qwen3-ASR-0.6B"
DEFAULT_ALIGNER = "Qwen/Qwen3-ForcedAligner-0.6B"
MAX_CUE_CHARS_LATIN = 42
MAX_CUE_CHARS_CJK = 22


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description="transcribe audio with Qwen3-ASR (MLX)")
    p.add_argument("--audio", required=True, help="path to 16 kHz mono wav")
    p.add_argument("--model", default=DEFAULT_MODEL, help="Qwen3-ASR model id")
    p.add_argument("--language", default=None, help="language hint (e.g. Chinese, English)")
    p.add_argument("--align", default=None, nargs="?", const=DEFAULT_ALIGNER,
                   help="force-align words (Qwen3-ForcedAligner id); "
                        "pass no value for the default aligner")
    p.add_argument("--out", default="-", help="output path or '-' for stdout")
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


def main() -> int:
    args = parse_args()

    try:
        from mlx_qwen3_asr import transcribe  # type: ignore
    except ImportError:
        sys.stderr.write(
            "lumen_cut_asr: mlx_qwen3_asr is not installed.\n"
            "  install with:  uv pip install mlx-qwen3-asr\n"
            "  or:            pip install mlx-qwen3-asr\n"
        )
        return 2

    try:
        result = transcribe(
            args.audio,
            model=args.model,
            language=args.language,
            return_timestamps=True,
            # mlx-qwen3-asr resolves a model id or local path itself. Passing
            # the id also keeps compatibility with runtime releases whose
            # ForcedAligner class has no `from_pretrained` constructor.
            forced_aligner=args.align,
        )
    except Exception as e:  # noqa: BLE001
        sys.stderr.write(f"lumen_cut_asr: transcription failed: {e}\n")
        return 3

    segments = list(result.segments or [])
    paragraphs = build_paragraphs(segments, result.language or args.language)
    duration = load_audio_duration(args.audio)
    out = {
        "schema_version": 1,
        "language": result.language or args.language,
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
