#!/usr/bin/env python3
"""Regression tests for #2720 — cf-container-hello /name/ decode guard.

The Worker's /name/ branch passed its path segment through an unguarded
decodeURIComponent, so a malformed percent-encoding (%E0%A4%A, %zz, a
trailing %) threw a raw URIError out of the Worker — a non-JSON 500
instead of a client error (design #2747 Phase 2: malformed spelling is
a 400, never a 500).

The fix guards just the decode: the URIError becomes the file's
standard json(...) 400 error response. These tests pin that:

- a malformed name gets the guarded 400 (and no raw URIError text),
- a valid (even percent-encoded) name still decodes exactly as before,
- the 401/503/worker-marker paths are untouched by the guard.

Run: npm test  (python3 -m unittest discover -s test -p "test_*.py")
"""

import re
import unittest
from pathlib import Path

PKG = Path(__file__).resolve().parent.parent
SRC = (PKG / "src" / "index.ts").read_text()


class NameDecodeIsGuarded(unittest.TestCase):
    def test_decode_sits_inside_try_catch(self):
        # The /name/ decode must be guarded — find the decode of the
        # /name/ slice inside a try block with a catch.
        m = re.search(
            r"try\s*\{(?P<body>[^}]*decodeURIComponent\(pathname\.slice\(\"/name/\"\.length\)\)[^}]*)\}\s*catch",
            SRC,
            re.DOTALL,
        )
        self.assertIsNotNone(m, "the /name/ decodeURIComponent is not inside try/catch")

    def test_malformed_name_returns_400_json(self):
        # The catch branch answers in the file's standard json(...) error
        # shape with a 400 — and carries no raw URIError text.
        m = re.search(
            r"\}\s*catch\s*\{\s*return json\(\{ error: \"[^\"]+\" \}, 400\);",
            SRC,
        )
        self.assertIsNotNone(m, "no catch branch returning json 400 found")
        self.assertNotIn("URIError", m.group(0))

    def test_valid_encoded_names_still_decode(self):
        # The success path still resolves through decodeURIComponent on
        # the /name/ slice — valid encodings (e.g. "a%20b", "100%25"
        # -> literal "100%") behave exactly as before.
        self.assertIn('decodeURIComponent(pathname.slice("/name/".length))', SRC)

    def test_guard_wraps_only_the_decode(self):
        # The 401/503/worker-marker paths must stay byte-identical: the
        # auth gate and marker branch come before the guarded decode and
        # contain no try/catch of their own.
        gate = SRC.index("isAuthorized(request, env)")
        marker = SRC.index('"/worker-marker"')
        guarded = SRC.index("malformed percent-encoding in name")
        self.assertLess(marker, gate)
        self.assertLess(gate, guarded)
        pre_guard = SRC[: SRC.index("try {")]
        self.assertNotIn("catch", pre_guard.split("isAuthorized(request, env)")[-1])


if __name__ == "__main__":
    unittest.main()
