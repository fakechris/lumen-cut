#!/usr/bin/env python3
"""lumen_cut_diarize — pyannote.audio speaker-diarization sidecar.

Reads a 16 kHz mono WAV (matching `media.rs`'s contract), runs pyannote, and
emits `diarize_out.v1` JSON with raw speaker segments. Stage 3 leaves the
speaker-alignment work to Stage 4 (the `align-speakers` audit).
"""
from __future__ import annotations

import argparse
import json
import os
import resource
import sys
import time
from typing import Any, TextIO

DEFAULT_MODEL = "pyannote/speaker-diarization-3.1"
PROGRESS_PREFIX = "LUMEN_CUT_PROGRESS "
CPU_THREAD_LIMIT = 4
DEFAULT_MEMORY_LIMIT_MB = 6144


def emit_progress(
    phase: str,
    progress: int,
    *,
    current: int | None = None,
    total: int | None = None,
    stream: TextIO | None = None,
    **details: Any,
) -> None:
    stream = stream or sys.stderr
    payload: dict[str, Any] = {"phase": phase, "progress": progress}
    if current is not None:
        payload["current"] = int(current)
    if total is not None:
        payload["total"] = int(total)
    payload.update(details)
    stream.write(PROGRESS_PREFIX + json.dumps(payload, separators=(",", ":")) + "\n")
    stream.flush()


class ResourceMonitor:
    """Report process cost and stop at a safe boundary before memory runs away."""

    def __init__(self, device: str) -> None:
        self.device = device
        self.started_at = time.monotonic()
        self.started_cpu = time.process_time()
        configured_limit = os.environ.get("LUMEN_CUT_MAX_SIDECAR_MEMORY_MB")
        self.memory_limit_mb = (
            int(configured_limit) if configured_limit else DEFAULT_MEMORY_LIMIT_MB
        )

    @staticmethod
    def peak_memory_mb() -> float:
        peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
        # macOS reports bytes while Linux reports KiB.
        divisor = 1024 * 1024 if sys.platform == "darwin" else 1024
        return peak / divisor

    def snapshot(self) -> dict[str, Any]:
        elapsed = max(0.001, time.monotonic() - self.started_at)
        cpu_seconds = max(0.0, time.process_time() - self.started_cpu)
        peak_memory_mb = self.peak_memory_mb()
        if self.memory_limit_mb > 0 and peak_memory_mb > self.memory_limit_mb:
            raise MemoryError(
                "speaker analysis exceeded its memory guardrail "
                f"({peak_memory_mb:.0f} MB > {self.memory_limit_mb} MB)"
            )
        return {
            "device": self.device,
            "elapsed_seconds": round(elapsed, 1),
            "cpu_percent": round(cpu_seconds / elapsed * 100),
            "peak_memory_mb": round(peak_memory_mb),
            "memory_limit_mb": self.memory_limit_mb,
        }


class StructuredProgressHook:
    """Convert pyannote's per-step hook into stable whole-job progress."""

    _STEPS = {
        "segmentation": ("segmenting", 8, 60),
        "speaker_counting": ("counting", 60, 64),
        "embeddings": ("embedding", 64, 92),
        "discrete_diarization": ("finalizing", 92, 98),
    }

    def __init__(
        self,
        stream: TextIO | None = None,
        monitor: ResourceMonitor | None = None,
    ) -> None:
        self.stream = stream or sys.stderr
        self.monitor = monitor
        self.progress = 5

    def __call__(
        self,
        step_name: str,
        step_artifact: Any,
        file: Any = None,
        total: int | None = None,
        completed: int | None = None,
    ) -> None:
        del step_artifact, file
        step = self._STEPS.get(step_name)
        if step is None:
            return
        phase, start, end = step
        completed_value = int(completed) if completed is not None else None
        total_value = int(total) if total is not None else None
        if completed_value is not None and total_value is not None:
            completed_value = min(completed_value, total_value)
        if completed_value is not None and total_value is not None and total_value > 0:
            ratio = min(1.0, max(0.0, completed_value / total_value))
            progress = round(start + (end - start) * ratio)
        else:
            progress = end
        progress = max(self.progress, progress)
        self.progress = progress
        emit_progress(
            phase,
            progress,
            current=completed_value,
            total=total_value,
            stream=self.stream,
            **(self.monitor.snapshot() if self.monitor else {}),
        )


def configure_compute_backend(pipe: Any) -> str:
    """Prefer Apple Metal while keeping a bounded, explicit CPU fallback."""
    import torch  # type: ignore

    requested = os.environ.get("LUMEN_CUT_DIARIZE_DEVICE", "auto").strip().lower()
    mps = getattr(getattr(torch, "backends", None), "mps", None)
    if requested != "cpu" and mps is not None and mps.is_available():
        try:
            pipe.to(torch.device("mps"))
            return "mps"
        except Exception as exc:  # noqa: BLE001
            sys.stderr.write(
                "lumen_cut_diarize: Metal initialization failed; "
                f"falling back to CPU: {exc}\n"
            )

    thread_limit = min(CPU_THREAD_LIMIT, max(1, os.cpu_count() or 1))
    torch.set_num_threads(thread_limit)
    return "cpu"


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description="diarize audio with pyannote")
    p.add_argument("--audio", required=True, help="path to 16 kHz mono wav")
    p.add_argument("--model", default=DEFAULT_MODEL, help="pyannote pipeline model id")
    p.add_argument("--out", default="-", help="output path or '-' for stdout")
    return p.parse_args()


def main() -> int:
    args = parse_args()
    emit_progress("loading", 1)

    # Let PyTorch run unsupported MPS operators on CPU instead of failing the
    # whole job. This must be set before importing torch via pyannote.audio.
    os.environ.setdefault("PYTORCH_ENABLE_MPS_FALLBACK", "1")

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

    device = configure_compute_backend(pipe)
    monitor = ResourceMonitor(device)
    emit_progress("loading", 5, **monitor.snapshot())
    diarization = pipe(args.audio, hook=StructuredProgressHook(monitor=monitor))
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
    emit_progress("completed", 100, **monitor.snapshot())
    return 0


if __name__ == "__main__":
    sys.exit(main())
