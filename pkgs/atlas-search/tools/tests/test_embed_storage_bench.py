#!/usr/bin/env python3
"""Oracle: reference hydration returns matching hashes or fails closed without data leaks."""

import hashlib
import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "embed_storage_bench.py"
SPEC = importlib.util.spec_from_file_location("embed_storage_bench", SCRIPT)
BENCH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BENCH)


class ReferenceHydrationTest(unittest.TestCase):
    def test_matching_reference_returns_only_the_verified_hash(self):
        body = b"SYNTHETIC_PRIVATE_BODY_MUST_NOT_ESCAPE"
        digest = hashlib.sha256(body).hexdigest()
        self.assertEqual(BENCH.verify_reference(digest, lambda _: body), digest)

    def test_missing_reference_fails_without_including_content(self):
        body = b"SYNTHETIC_PRIVATE_BODY_MUST_NOT_ESCAPE"
        digest = hashlib.sha256(body).hexdigest()
        with self.assertRaises(BENCH.HydrationError) as raised:
            BENCH.verify_reference(digest, lambda _: None)
        self.assertEqual(str(raised.exception), "reference object unavailable")
        self.assertNotIn(body.decode(), str(raised.exception))

    def test_hot_variant_reuses_repeated_query_objects(self):
        result = BENCH.measure(BENCH.synthetic_sample(16), runs=5, cache_mode="both")
        for operation in result["hydration"].values():
            self.assertLess(operation["hot"]["r2_class_b_gets"], operation["cold"]["r2_class_b_gets"])

    def test_corrupt_reference_fails_without_including_content(self):
        body = b"SYNTHETIC_PRIVATE_BODY_MUST_NOT_ESCAPE"
        digest = hashlib.sha256(body).hexdigest()
        corrupt = b"CORRUPT_PRIVATE_BODY_MUST_NOT_ESCAPE"
        with self.assertRaises(BENCH.HydrationError) as raised:
            BENCH.verify_reference(digest, lambda _: corrupt)
        self.assertEqual(str(raised.exception), "reference object hash mismatch")
        self.assertNotIn(corrupt.decode(), str(raised.exception))


if __name__ == "__main__":
    unittest.main()
