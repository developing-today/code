#!/usr/bin/env python3
"""Unit tests for scripts/id/affected-crates.py using a small Cargo graph."""
from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("affected-crates.py")
SPEC = importlib.util.spec_from_file_location("affected_crates", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
affected_crates = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(affected_crates)


def package(name: str, relative_manifest: str, dependencies: list[str]) -> dict[str, object]:
    return {
        "id": f"path+file:///fixture#{name}@0.1.0",
        "name": name,
        "manifest_path": str(affected_crates.REPO_ROOT / relative_manifest),
        "dependencies": [
            {"name": dependency, "source": None} for dependency in dependencies
        ],
    }


class AffectedCratesTests(unittest.TestCase):
    def setUp(self) -> None:
        self.packages = [
            package("id", "pkgs/id/Cargo.toml", []),
            package("id-core", "pkgs/id/crates/id-core/Cargo.toml", []),
            package("id-store", "pkgs/id/crates/id-store/Cargo.toml", ["id-core"]),
            package("id-sync", "pkgs/id/crates/id-sync/Cargo.toml", ["id-core", "id-store"]),
            package("id-render", "pkgs/id/crates/id-render/Cargo.toml", ["id-core"]),
            package("id-cli", "pkgs/id/crates/id-cli/Cargo.toml", ["id-sync", "id-render"]),
            package("id-web", "pkgs/id/crates/id-web/Cargo.toml", ["id-cli"]),
        ]
        self.metadata = {
            "workspace_members": [p["id"] for p in self.packages],
            "packages": self.packages,
        }

    def test_id_core_diff_selects_all_transitive_dependents(self) -> None:
        patch = """diff --git a/pkgs/id/crates/id-core/src/lib.rs b/pkgs/id/crates/id-core/src/lib.rs
index 1234567..abcdef0 100644
--- a/pkgs/id/crates/id-core/src/lib.rs
+++ b/pkgs/id/crates/id-core/src/lib.rs
@@ -1 +1 @@
-old
+new
"""
        paths = affected_crates.diff_paths(patch)
        self.assertEqual(
            affected_crates.select_affected(paths, self.metadata),
            ["id-cli", "id-core", "id-render", "id-store", "id-sync", "id-web"],
        )

    def test_id_render_diff_selects_only_it_and_its_dependents(self) -> None:
        paths = affected_crates.diff_paths(
            "pkgs/id/crates/id-render/src/lib.rs\n"
        )
        self.assertEqual(
            affected_crates.select_affected(paths, self.metadata),
            ["id-cli", "id-render", "id-web"],
        )

    def test_workspace_manifest_change_selects_every_member(self) -> None:
        self.assertEqual(
            affected_crates.select_affected(
                ["pkgs/id/Cargo.toml"], self.metadata
            ),
            sorted(p["name"] for p in self.packages),
        )

    def test_deleted_file_uses_old_path(self) -> None:
        patch = """diff --git a/pkgs/id/crates/id-render/src/old.rs b/pkgs/id/crates/id-render/src/old.rs
deleted file mode 100644
--- a/pkgs/id/crates/id-render/src/old.rs
+++ /dev/null
"""
        self.assertEqual(
            affected_crates.select_affected(
                affected_crates.diff_paths(patch), self.metadata
            ),
            ["id-cli", "id-render", "id-web"],
        )


if __name__ == "__main__":
    unittest.main()
