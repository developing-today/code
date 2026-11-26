#!/usr/bin/env python3
"""Oracle: gh-post's runtime dependencies resolve beside its repository file.

Import only: never invoke a posting command or touch production state. An
isolated interpreter prevents a developer's ~/tools/PYTHONPATH from masking
missing packaged dependencies.
"""

import json
from pathlib import Path
import subprocess
import sys
import unittest


ROOT = Path(__file__).resolve().parent


class RuntimeDependencyTests(unittest.TestCase):
    def test_repository_relative_runtime_imports(self):
        probe = r'''
import importlib.machinery
import importlib.util
import json
import sys
loader = importlib.machinery.SourceFileLoader("gh_post_runtime", sys.argv[1])
spec = importlib.util.spec_from_loader(loader.name, loader)
module = importlib.util.module_from_spec(spec)
loader.exec_module(module)
print(json.dumps({name: {
    "error": repr(getattr(module, error)),
    "path": getattr(getattr(module, name), "__file__", None),
} for name, error in (
    ("content_ledger", "_LEDGER_IMPORT_ERROR"),
    ("content_throttle", "_THROTTLE_IMPORT_ERROR"),
)}))
'''
        result = subprocess.run(
            [sys.executable, "-I", "-c", probe, str(ROOT / "gh-post")],
            capture_output=True, text=True, check=True,
        )
        modules = json.loads(result.stdout)
        for name, directory in (("content_ledger", "content-ledger"),
                                ("content_throttle", "content-throttle")):
            with self.subTest(module=name):
                self.assertEqual(modules[name]["error"], "None")
                self.assertIsNotNone(modules[name]["path"])
                self.assertEqual(Path(modules[name]["path"]).resolve(),
                                 ROOT / directory / f"{name}.py")


if __name__ == "__main__":
    unittest.main()
