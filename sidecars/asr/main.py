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
import sys
from typing import Any

# Default model id — resolves from the local HF cache
# (mlx-community/Qwen3-ASR-0.6B-8bit or Qwen/Qwen3-ASR-0.6B).
DEFAULT_MODEL = "mlx-community/Qwen3-ASR-0.6B-8bit"
DEFAULT_ALIGNER = "mlx-community/Qwen3-ForcedAligner-0.6B-8bit"


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
    atoms = [c for c in text if not c.isspace()] or [text]
    n = len(atoms)
    span = max(end - start, 0.0) / max(n, 1)
    return [
        {"text": a, "start": start + span * i, "end": start + span * (i + 1)}
        for i, a in enumerate(atoms)
    ]


def build_paragraphs(segments: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Group segments into one paragraph (lumen-cut's downstream pipeline
    re-segments via the `segment` contract). Each sentence keeps its
    words for word-level transcript rendering + cleanup/audit."""
    sentences: list[dict[str, Any]] = []
    for seg in segments:
        text = (seg.get("text") or "").strip()
        if not text:
            continue
        s = float(seg.get("start", 0.0))
        e = float(seg.get("end", s))
        words = seg.get("words") or words_for_segment(text, s, e)
        sentences.append({"text": text, "words": words})
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

    fa = None
    if args.align:
        try:
            from mlx_qwen3_asr import ForcedAligner  # type: ignore
            fa = ForcedAligner.from_pretrained(args.align)
        except Exception as e:  # noqa: BLE001
            sys.stderr.write(f"lumen_cut_asr: forced aligner unavailable ({e}); "
                             "falling back to segment-level timing\n")
            fa = None

    try:
        result = transcribe(
            args.audio,
            model=args.model,
            language=args.language,
            return_timestamps=True,
            forced_aligner=fa,
        )
    except Exception as e:  # noqa: BLE001
        sys.stderr.write(f"lumen_cut_asr: transcription failed: {e}\n")
        return 3

    segments = list(result.segments or [])
    paragraphs = build_paragraphs(segments)
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
