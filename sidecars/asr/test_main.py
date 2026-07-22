import importlib.util
import json
import pathlib
import subprocess
import types
import unittest
from unittest import mock


MODULE_PATH = pathlib.Path(__file__).with_name("main.py")
SPEC = importlib.util.spec_from_file_location("lumen_cut_asr", MODULE_PATH)
assert SPEC and SPEC.loader
ASR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ASR)


class SidecarFormattingTests(unittest.TestCase):
    def test_mlx_memory_policy_caps_unified_memory_and_cache(self) -> None:
        calls: list[tuple[str, int]] = []
        fake_mx = types.SimpleNamespace(
            set_memory_limit=lambda value: calls.append(("memory", value)),
            set_cache_limit=lambda value: calls.append(("cache", value)),
            reset_peak_memory=lambda: None,
        )
        with mock.patch.dict(
            ASR.os.environ,
            {"LUMEN_CUT_MAX_SIDECAR_MEMORY_MB": "4096"},
        ):
            monitor = ASR.configure_mlx_memory(fake_mx)

        self.assertEqual(calls, [
            ("memory", 4096 * 1024 * 1024),
            ("cache", 256 * 1024 * 1024),
        ])
        self.assertEqual(monitor.memory_limit_mb, 4096)

    def test_word_segments_become_one_readable_cue(self) -> None:
        segments = [
            {"text": "Hello", "start": 0.0, "end": 0.4},
            {"text": "world", "start": 0.4, "end": 0.9},
            {"text": ".", "start": 0.9, "end": 1.0},
        ]
        paragraphs = ASR.build_paragraphs(segments)
        sentence = paragraphs[0]["sentences"][0]
        self.assertEqual(sentence["text"], "Hello world.")
        self.assertEqual(len(sentence["words"]), 3)

    def test_large_silence_starts_a_new_cue(self) -> None:
        segments = [
            {"text": "First", "start": 0.0, "end": 0.4},
            {"text": "Second", "start": 1.4, "end": 1.9},
        ]
        paragraphs = ASR.build_paragraphs(segments)
        self.assertEqual(
            [sentence["text"] for sentence in paragraphs[0]["sentences"]],
            ["First", "Second"],
        )

    def test_cues_respect_the_editor_width_gate(self) -> None:
        segments = [
            {"text": token, "start": index * 0.3, "end": (index + 1) * 0.3}
            for index, token in enumerate(
                [
                    "This", "subtitle", "must", "wrap", "before", "it", "becomes",
                    "too", "wide", "for", "everyone", "watching", "today",
                ]
            )
        ]
        sentences = ASR.build_paragraphs(segments, "English")[0]["sentences"]
        self.assertGreater(len(sentences), 1)
        for sentence in sentences:
            visible = sum(not character.isspace() for character in sentence["text"])
            self.assertLessEqual(visible, ASR.MAX_CUE_CHARS_LATIN)

    def test_cjk_uses_a_tighter_width_gate(self) -> None:
        self.assertEqual(ASR.max_cue_chars("Chinese"), 22)
        self.assertEqual(ASR.max_cue_chars("English"), 42)

    def test_timestamp_pipeline_isolates_recognition_and_alignment_workers(self) -> None:
        recognized = {
            "language": "English",
            "chunks": [
                {
                    "text": "Hello world.",
                    "start": 0.0,
                    "end": 2.0,
                    "language": "English",
                }
            ],
        }
        aligned = {
            "segments": [
                {"text": "Hello", "start": 0.0, "end": 0.8},
                {"text": "world.", "start": 0.8, "end": 2.0},
            ]
        }
        calls: list[tuple[list[str], str | None]] = []

        def runner(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
            worker_input = kwargs.get("input")
            calls.append((command, worker_input if isinstance(worker_input, str) else None))
            stage = command[command.index("--worker") + 1]
            payload = recognized if stage == "recognize" else aligned
            return subprocess.CompletedProcess(command, 0, json.dumps(payload), "")

        args = types.SimpleNamespace(
            audio="/tmp/audio.wav",
            model="asr-model",
            language="English",
            align="aligner-model",
        )
        result = ASR.run_isolated_pipeline(args, runner=runner, python="/runtime/python")

        self.assertEqual(
            [command[command.index("--worker") + 1] for command, _ in calls],
            ["recognize", "align"],
        )
        self.assertIsNone(calls[0][1])
        self.assertEqual(json.loads(calls[1][1] or "{}"), recognized)
        self.assertEqual(result["segments"], aligned["segments"])


if __name__ == "__main__":
    unittest.main()
