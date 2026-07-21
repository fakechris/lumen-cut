#!/usr/bin/env python3
"""lumen_cut_diarize — pyannote.audio speaker-diarization sidecar.

Reads a 16 kHz mono WAV (matching `media.rs`'s contract), runs pyannote, and
emits `diarize_out.v1` JSON with raw speaker segments. Stage 3 leaves the
speaker-alignment work to Stage 4 (the `align-speakers` audit).
"""
from __future__ import annotations

import argparse
import json
import sys
from typing import Any

DEFAULT_MODEL = "pyannote/speaker-diarization-3.1"


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description="diarize audio with pyannote")
    p.add_argument("--audio", required=True, help="path to 16 kHz mono wav")
    p.add_argument("--model", default=DEFAULT_MODEL, help="pyannote pipeline model id")
    p.add_argument("--out", default="-", help="output path or '-' for stdout")
    return p.parse_args()


def main() -> int:
    args = parse_args()

    try:
        from pyannote.audio import Pipeline  # type: ignore
    except ImportError:
        sys.stderr.write(
            "lumen_cut_diarize: pyannote.audio is not installed.\n"
            "  install with:  uv pip install pyannote.audio\n"
            "  requires:      HuggingFace token with pyannote/segmentation-3.0 access\n"
        )
        return 2

    try:
        pipe = Pipeline.from_pretrained(args.model)
    except Exception as e:  # noqa: BLE001
        sys.stderr.write(
            f"lumen_cut_diarize: failed to load {args.model}: {e}\n"
        )
        return 3

    diarization = pipe(args.audio)
    segments: list[dict[str, Any]] = []
    for turn, _, speaker in diarization.itertracks(yield_label=True):
        segments.append({
            "speaker": str(speaker),
            "start": float(turn.start),
            "end": float(turn.end),
        })

    payload = json.dumps(
        {"schema_version": 1, "segments": segments},
        ensure_ascii=False,
        indent=2,
    )
    if args.out == "-":
        sys.stdout.write(payload)
        sys.stdout.write("\n")
    else:
        with open(args.out, "w", encoding="utf-8") as f:
            f.write(payload)
    return 0


if __name__ == "__main__":
    sys.exit(main())
