import contextlib
import importlib.util
import io
import json
import pathlib
import sys
import types
import unittest
from unittest import mock


MODULE_PATH = pathlib.Path(__file__).with_name("main.py")
SPEC = importlib.util.spec_from_file_location("lumen_cut_diarize", MODULE_PATH)
assert SPEC and SPEC.loader
DIARIZE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DIARIZE)


class _Turn:
    start = 0.2
    end = 1.4


class _Diarization:
    def itertracks(self, yield_label: bool = False):
        assert yield_label
        yield _Turn(), None, "SPEAKER_00"


class _Pipeline:
    moved_to = None

    @classmethod
    def from_pretrained(cls, model: str):
        assert model == "test-model"
        return cls()

    def __call__(self, audio: str, hook=None):
        assert audio == "/tmp/audio.wav"
        assert hook is not None
        hook("segmentation", None, total=10, completed=4)
        hook("segmentation", None, total=10, completed=10)
        hook("speaker_counting", None)
        hook("embeddings", None, total=5, completed=3)
        hook("embeddings", None, total=5, completed=5)
        hook("discrete_diarization", None)
        return _Diarization()

    def to(self, device):
        type(self).moved_to = device


class DiarizeProgressTests(unittest.TestCase):
    def test_progress_normalizes_numpy_style_integer_counters(self) -> None:
        class IntLike:
            def __init__(self, value: int) -> None:
                self.value = value

            def __int__(self) -> int:
                return self.value

        stream = io.StringIO()
        DIARIZE.emit_progress(
            "segmenting",
            12,
            current=IntLike(3),
            total=IntLike(9),
            stream=stream,
        )
        payload = json.loads(stream.getvalue().removeprefix("LUMEN_CUT_PROGRESS "))
        self.assertEqual(payload["current"], 3)
        self.assertEqual(payload["total"], 9)

    def test_progress_clamps_pyannote_batch_overshoot(self) -> None:
        stream = io.StringIO()
        hook = DIARIZE.StructuredProgressHook(stream=stream)
        hook("segmentation", None, completed=32, total=21)
        payload = json.loads(stream.getvalue().removeprefix("LUMEN_CUT_PROGRESS "))
        self.assertEqual(payload["current"], 21)
        self.assertEqual(payload["total"], 21)

    def test_cli_streams_structured_progress_without_polluting_result_json(self) -> None:
        fake_audio = types.ModuleType("pyannote.audio")
        fake_audio.Pipeline = _Pipeline
        fake_pyannote = types.ModuleType("pyannote")
        fake_pyannote.audio = fake_audio
        fake_torch = types.ModuleType("torch")
        fake_torch.backends = types.SimpleNamespace(
            mps=types.SimpleNamespace(is_available=lambda: True),
        )
        fake_torch.device = lambda name: name
        fake_torch.set_num_threads = lambda count: None
        _Pipeline.moved_to = None
        stdout = io.StringIO()
        stderr = io.StringIO()
        argv = [
            str(MODULE_PATH),
            "--audio",
            "/tmp/audio.wav",
            "--model",
            "test-model",
            "--out",
            "-",
        ]

        with (
            mock.patch.dict(sys.modules, {
                "pyannote": fake_pyannote,
                "pyannote.audio": fake_audio,
                "torch": fake_torch,
            }),
            mock.patch.object(sys, "argv", argv),
            contextlib.redirect_stdout(stdout),
            contextlib.redirect_stderr(stderr),
        ):
            self.assertEqual(DIARIZE.main(), 0)

        result = json.loads(stdout.getvalue())
        self.assertEqual(result["segments"][0]["speaker"], "SPEAKER_00")
        self.assertEqual(_Pipeline.moved_to, "mps")
        updates = [
            json.loads(line.removeprefix("LUMEN_CUT_PROGRESS "))
            for line in stderr.getvalue().splitlines()
            if line.startswith("LUMEN_CUT_PROGRESS ")
        ]
        self.assertEqual(updates[0]["phase"], "loading")
        backend_updates = [update for update in updates if update.get("device")]
        self.assertTrue(backend_updates)
        self.assertTrue(all(update["device"] == "mps" for update in backend_updates))
        self.assertTrue(all("elapsed_seconds" in update for update in backend_updates))
        self.assertTrue(all("cpu_percent" in update for update in backend_updates))
        self.assertTrue(all("peak_memory_mb" in update for update in backend_updates))
        self.assertIn("segmenting", [update["phase"] for update in updates])
        self.assertIn("embedding", [update["phase"] for update in updates])
        self.assertEqual(updates[-1]["phase"], "completed")
        self.assertEqual(updates[-1]["progress"], 100)
        self.assertTrue(all(
            current["progress"] <= following["progress"]
            for current, following in zip(updates, updates[1:])
        ))
        self.assertIn(
            29,
            [
                update["progress"]
                for update in updates
                if update["phase"] == "segmenting" and update.get("current") == 4
            ],
        )

    def test_compute_backend_caps_cpu_threads_when_mps_is_unavailable(self) -> None:
        fake_torch = types.ModuleType("torch")
        fake_torch.backends = types.SimpleNamespace(
            mps=types.SimpleNamespace(is_available=lambda: False),
        )
        thread_counts = []
        fake_torch.set_num_threads = thread_counts.append

        with (
            mock.patch.dict(sys.modules, {"torch": fake_torch}),
            mock.patch.object(DIARIZE.os, "cpu_count", return_value=12),
        ):
            self.assertEqual(DIARIZE.configure_compute_backend(_Pipeline()), "cpu")

        self.assertEqual(thread_counts, [4])

    def test_compute_backend_can_force_cpu_for_reproducible_fallbacks(self) -> None:
        fake_torch = types.ModuleType("torch")
        fake_torch.backends = types.SimpleNamespace(
            mps=types.SimpleNamespace(is_available=lambda: True),
        )
        fake_torch.device = lambda name: name
        thread_counts = []
        fake_torch.set_num_threads = thread_counts.append
        _Pipeline.moved_to = None

        with (
            mock.patch.dict(sys.modules, {"torch": fake_torch}),
            mock.patch.dict(DIARIZE.os.environ, {"LUMEN_CUT_DIARIZE_DEVICE": "cpu"}),
        ):
            self.assertEqual(DIARIZE.configure_compute_backend(_Pipeline()), "cpu")

        self.assertIsNone(_Pipeline.moved_to)
        self.assertEqual(thread_counts, [4])


if __name__ == "__main__":
    unittest.main()
