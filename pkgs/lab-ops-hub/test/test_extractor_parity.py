# Paired-fixture parity tests for the container Extractor — #569.
# The SAME fixture file drives the Worker fallback's test
# (test/extractor.test.ts), so both doors are pinned to identical
# {title, links, text} results. Stdlib only, like the other tests here.
import importlib.util
import json
import pathlib
import re
import unittest

_HERE = pathlib.Path(__file__).resolve().parent
_APP_PATH = _HERE.parent / "container" / "app.py"
_spec = importlib.util.spec_from_file_location("labops_container_app", _APP_PATH)
app = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(app)

_FIXTURES = json.loads((_HERE / "extractor-fixtures.json").read_text(encoding="utf-8"))


def extract(html):
    ex = app.Extractor()
    ex.feed(html)
    text = re.sub(r"\s+", " ", " ".join(ex.text)).strip()
    return {"title": ex.title.strip(), "links": ex.links, "text": text}


class TestExtractorParity(unittest.TestCase):
    def test_fixtures(self):
        for f in _FIXTURES:
            with self.subTest(fixture=f["name"]):
                got = extract(f["html"])
                self.assertEqual(got["title"], f["title"], "title")
                self.assertEqual(got["links"], f["links"], "links")
                self.assertEqual(got["text"], f["text"], "text")


if __name__ == "__main__":
    unittest.main()
