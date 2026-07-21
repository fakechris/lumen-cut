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


if __name__ == "__main__":
    unittest.main()
