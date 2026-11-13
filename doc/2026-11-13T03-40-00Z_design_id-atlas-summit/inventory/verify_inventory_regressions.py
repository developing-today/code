#!/usr/bin/env python3
"""Stdlib regression suite: python3 -I verify_inventory_regressions.py --source PATH"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path


USAGE = "usage: verify_inventory_regressions.py --source PATH"


def arguments(argv: list[str]) -> Path:
    values: dict[str, str] = {}
    i = 0
    while i < len(argv):
        arg = argv[i]
        if "=" in arg:
            key, value = arg.split("=", 1)
        else:
            key = arg
            if i + 1 >= len(argv) or argv[i + 1].startswith("--"):
                raise ValueError(f"missing value for {key}")
            value = argv[i + 1]
            i += 1
        if key != "--source":
            raise ValueError(f"unrecognized argument: {key}")
        if not value or key in values:
            raise ValueError(f"invalid or duplicate option: {key}")
        values[key] = value
        i += 1
    if "--source" not in values:
        raise ValueError("missing required option: --source")
    return Path(values["--source"])


class Suite:
    def __init__(self, source: str, checker: Path, temp: Path):
        self.source = source
        self.checker = checker
        self.temp = temp
        self.passed = 0

    def run_case(self, name: str, content: str, expected_error: str | None = None) -> None:
        case = self.temp / name
        case.mkdir()
        source_path = case / "source.md"
        output_path = case / "map.json"
        source_path.write_text(content, encoding="utf-8")
        result = subprocess.run(
            [sys.executable, "-I", str(self.checker), "--source", str(source_path), "--json", str(output_path)],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if expected_error is None:
            if result.returncode != 0:
                raise AssertionError(f"{name}: expected success, got {result.returncode}: {result.stderr}")
            rows = json.loads(output_path.read_text(encoding="utf-8"))
            if len(rows) != 384 or len({row["id"] for row in rows}) != 384:
                raise AssertionError(f"{name}: expected 384 unique statement rows")
        else:
            if result.returncode == 0:
                raise AssertionError(f"{name}: expected rejection, checker succeeded")
            if expected_error not in result.stderr:
                raise AssertionError(f"{name}: expected {expected_error!r} in stderr, got {result.stderr!r}")
            if output_path.exists():
                raise AssertionError(f"{name}: rejected input wrote output")
        self.passed += 1
        print(f"PASS {name}")

    def run(self) -> int:
        self.run_case("valid-real-source", self.source)
        self.run_case("duplicate-definition", self.source.replace("S0.2 Primitives/adapters/hooks", "S0.1 Primitives/adapters/hooks", 1), "duplicate statement definitions")
        self.run_case("missing-map-target", self.source.replace("| S0.1–S0.17 |", "| S0.2–S0.17 |", 1), "unmapped statements")
        missing_plus_later_fake = self.source.replace("| S0.1–S0.17 |", "| S0.2–S0.17 |", 1).replace(
            "## 4. Version plan", "## 4. Version plan\n\n| S0.1 | **#9999** |", 1
        )
        self.run_case("later-fake-home-cannot-mask-gap", missing_plus_later_fake, "unmapped statements")
        self.run_case("unknown-extra-endpoint", self.source.replace("| S0.1–S0.17 |", "| S0.1, S0.999 |", 1), "unknown map endpoint: S0.999")
        self.run_case("malformed-map-syntax", self.source.replace("| S0.1–S0.17 |", "| S0.1; S0.2 |", 1), "malformed map range token")
        self.run_case("reversed-range", self.source.replace("| S0.1–S0.17 |", "| S0.17–S0.1 |", 1), "reversed map range")
        self.run_case("bad-home", self.source.replace("**#9110**", "**not-an-issue**", 1), "malformed issue home")
        self.run_case("count-mismatch", self.source.replace("Extensibility clarification (17)", "Extensibility clarification (18)", 1), "differ from fixed oracle")
        self.run_case("no-map-section", self.source.split("## 3. Statement → issue map", 1)[0], "section 3 statement map not found")

        # The checker must reject malformed CLI input before attempting to read
        # the deliberately nonexistent source or creating the designated output.
        output = self.temp / "invalid-cli-output.json"
        result = subprocess.run(
            [sys.executable, "-I", str(self.checker), "--source", str(self.temp / "does-not-exist"), "--json", str(output), "--bogus", "x"],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode != 2 or "unrecognized argument: --bogus" not in result.stderr or output.exists():
            raise AssertionError("invalid-cli-no-output: strict CLI rejection/side-effect oracle failed")
        self.passed += 1
        print("PASS invalid-cli-no-output")

        same = self.temp / "same-file.md"
        same.write_text("do not overwrite", encoding="utf-8")
        result = subprocess.run(
            [sys.executable, "-I", str(self.checker), "--source", str(same), "--json", str(same)],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode != 2 or "must not name the same path" not in result.stderr or same.read_text(encoding="utf-8") != "do not overwrite":
            raise AssertionError("source-output-alias: checker did not protect source")
        self.passed += 1
        print("PASS source-output-alias")

        source_file = self.temp / "hardlink-source.md"
        hardlink = self.temp / "hardlink-output.json"
        source_file.write_text(self.source, encoding="utf-8")
        os.link(source_file, hardlink)
        before = source_file.read_bytes()
        result = subprocess.run(
            [sys.executable, "-I", str(self.checker), "--source", str(source_file), "--json", str(hardlink)],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode != 2 or "same file" not in result.stderr or source_file.read_bytes() != before or hardlink.read_bytes() != before:
            raise AssertionError("hardlink-alias: checker failed to protect aliased source bytes")
        self.passed += 1
        print("PASS hardlink-alias-preserves-source")

        symlink_source = self.temp / "symlink-source.md"
        symlink_output = self.temp / "symlink-output.json"
        symlink_source.write_text(self.source, encoding="utf-8")
        symlink_output.symlink_to(symlink_source)
        before = symlink_source.read_bytes()
        result = subprocess.run(
            [sys.executable, "-I", str(self.checker), "--source", str(symlink_source), "--json", str(symlink_output)],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode != 2 or "same path" not in result.stderr or not symlink_output.is_symlink() or symlink_source.read_bytes() != before:
            raise AssertionError("symlink-alias: checker failed to preserve symlink/source")
        self.passed += 1
        print("PASS symlink-alias-preserves-source")
        print(f"{self.passed} regression cases passed")
        return self.passed


def main(argv: list[str]) -> int:
    try:
        source_path = arguments(argv)
    except ValueError as exc:
        print(f"{USAGE}\nerror: {exc}", file=sys.stderr)
        return 2
    try:
        source = source_path.read_text(encoding="utf-8")
    except OSError as exc:
        print(f"source error: {exc}", file=sys.stderr)
        return 1
    checker = Path(__file__).with_name("check_inventory.py").resolve()
    temp_root = os.environ.get("TMPDIR", "/home/user/.local/share/opencode/tmp/opencode")
    with tempfile.TemporaryDirectory(prefix="inventory-regressions-", dir=temp_root) as directory:
        Suite(source, checker, Path(directory)).run()
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
