"""Unit tests for the pure merge logic in update-choice-options.py.

Run: python3 -m unittest test_update_choice_options -v
(from this directory; stdlib only, no Grist needed.)
"""

import importlib.util
import pathlib
import unittest

_spec = importlib.util.spec_from_file_location(
    "update_choice_options", pathlib.Path(__file__).with_name("update-choice-options.py"))
_mod = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_mod)
merge_choices = _mod.merge_choices


class MergeChoices(unittest.TestCase):
    def test_appends_new_preserving_order_and_other_keys(self):
        wo = {"choices": ["code", "infra"],
              "choiceOptions": {"code": {"fillColor": "#1d76db"}}}
        merged, added = merge_choices(wo, ["id"])
        self.assertEqual(merged["choices"], ["code", "infra", "id"])
        self.assertEqual(added, ["id"])
        self.assertEqual(merged["choiceOptions"], {"code": {"fillColor": "#1d76db"}})

    def test_idempotent_no_duplicates(self):
        wo = {"choices": ["code", "infra", "id"]}
        merged, added = merge_choices(wo, ["id", "code"])
        self.assertEqual(merged["choices"], ["code", "infra", "id"])
        self.assertEqual(added, [])

    def test_empty_widget_options(self):
        merged, added = merge_choices({}, ["a", "b"])
        self.assertEqual(merged["choices"], ["a", "b"])
        self.assertEqual(added, ["a", "b"])

    def test_duplicate_additions_in_one_call(self):
        merged, added = merge_choices({"choices": []}, ["x", "x"])
        self.assertEqual(merged["choices"], ["x"])
        self.assertEqual(added, ["x"])

    def test_input_not_mutated(self):
        wo = {"choices": ["a"]}
        merge_choices(wo, ["b"])
        self.assertEqual(wo["choices"], ["a"])


if __name__ == "__main__":
    unittest.main()
