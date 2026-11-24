"""Mutation tests for the source-to-book DOC-01 coverage oracles."""

import unittest

from check_book_coverage import check_defaults, check_modules


class CoverageTests(unittest.TestCase):
    rust = "pub mod search;\npub mod source_registry;\nmod private;"
    table = (
        "## The 2 public modules, grouped by job\n\n"
        "`lib.rs` declares 2 public modules.\n\n"
        "| Module | Job |\n|---|---|\n"
        "| `source_registry` | Registry |\n| `search` | Search |"
    )
    defaults = '{"k":10,"raw":false}'
    chapter = (
        "# Protocol\n## Defaults (`query-parameters-v1.json`)\n"
        "```json\n{\"raw\":false,\"k\":10}\n```\n"
    )

    def test_document_order_does_not_affect_coverage(self):
        """Oracle: equal inventories and JSON objects pass independent of row/key order."""
        self.assertEqual(check_modules(self.rust, self.table), 2)
        self.assertEqual(check_defaults(self.defaults, self.chapter), 2)

    def test_module_drift(self):
        """Oracle: source and book inventories must match exactly, including declared counts."""
        for rust, table, diagnostic in [
            (
                self.rust + "\npub mod added;",
                self.table.replace("The 2", "The 3").replace("declares 2", "declares 3"),
                "missing.*added",
            ),
            (
                self.rust.replace("pub mod source_registry;\n", ""),
                self.table.replace("The 2", "The 1").replace("declares 2", "declares 1"),
                "stale.*source_registry",
            ),
            (self.rust, self.table.replace("The 2", "The 3").replace("declares 2", "declares 3"), "stale public-module count"),
            (self.rust, self.table + "\n| `retired` | Old |", "stale.*retired"),
            (self.rust, self.table + "\n| `search` | Again |", "duplicate"),
            ("", self.table, "empty inventory"),
            (self.rust, "plain text", "expected one public-module heading"),
            (self.rust + "\npub mod inline {}", self.table, "unsupported"),
        ]:
            with self.subTest(diagnostic=diagnostic), self.assertRaisesRegex(ValueError, diagnostic):
                check_modules(rust, table)

    def test_contract_drift(self):
        """Oracle: the typed JSON values and exact key set match the contract file."""
        for old, new, diagnostic in [
            ('"raw":false', '"raw":0', "differing.*raw"),
            ('"k":10', '"k":20', "differing.*k"),
            ('"raw":false,', "", "differing.*raw"),
            ('"k":10', '"k":10,"stale":10', "differing.*stale"),
            ('"k":10', '"k":10,"k":10', "duplicate JSON key"),
            ("```json", "```text", "fenced JSON"),
        ]:
            with self.subTest(new=new), self.assertRaisesRegex(ValueError, diagnostic):
                check_defaults(self.defaults, self.chapter.replace(old, new))


if __name__ == "__main__":
    unittest.main()
