import importlib.util
import pathlib
import unittest


MODULE_PATH = pathlib.Path(__file__).with_name("main.py")
SPEC = importlib.util.spec_from_file_location("lumen_cut_asr", MODULE_PATH)
assert SPEC and SPEC.loader
ASR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ASR)


class SidecarFormattingTests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
