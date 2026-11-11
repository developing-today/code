"""Validate the committed Worker decoder corpus against Python's oracle."""

import html
import html.entities
import json
import unittest
from pathlib import Path


HERE = Path(__file__).resolve().parent
CORPUS = json.loads((HERE / "html5-entity-corpus.json").read_text(encoding="utf-8"))


class Html5EntityCorpusTest(unittest.TestCase):
    def test_corpus_covers_every_html5_entity_key(self):
        self.assertEqual(CORPUS["oracle"], "Python html.unescape")
        self.assertEqual(CORPUS["entity_key_count"], len(html.entities.html5))
        inputs = {case["input"] for case in CORPUS["cases"]}
        self.assertLessEqual({f"&{key}" for key in html.entities.html5}, inputs)
        self.assertLessEqual(
            {
                f"&{key}{suffix}"
                for key in html.entities.html5
                if not key.endswith(";")
                for suffix in ("x", "=")
            },
            inputs,
        )
        self.assertLessEqual(
            {
                "&notanentity;",
                "&ampx",
                "&copy=",
                "&#0;",
                "&#1;",
                "&#9;",
                "&#127;",
                "&#128;",
                "&#55296;",
                "&#65534;",
                "&#1114111;",
                "&#1114112;",
            },
            inputs,
        )

    def test_expected_values_match_html_unescape(self):
        for case in CORPUS["cases"]:
            with self.subTest(input=case["input"]):
                self.assertEqual(html.unescape(case["input"]), case["expected"])


if __name__ == "__main__":
    unittest.main()
