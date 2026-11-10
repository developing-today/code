"""Renders RESULTS.md from scripts/RESULTS.template.md and the logs written by the test runs.

The example strings come from print-output.txt, not from retyping, and each one is checked against
its measured length before it is inserted. Run with: python3 -I scripts/render_results.py
"""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def read(name):
    return (ROOT / name).read_text()


def value_after(prefix, text):
    for line in text.splitlines():
        if line.startswith(prefix):
            return line[len(prefix):]
    raise SystemExit(f"missing line starting with {prefix!r}")


def main():
    printed = read("print-output.txt")

    a_example = value_after("A example: ", printed)
    d_example = value_after("D example: ", printed)
    b_match = re.search(r"B string \((\d+) chars\): (\S+)", printed)
    if not b_match:
        raise SystemExit("B string not found in print-output.txt")
    b_declared, b_example = int(b_match.group(1)), b_match.group(2)

    d_declared = int(re.search(r"D string \((\d+) chars\)", printed).group(1))
    checks = [
        ("A", len(a_example), 124),
        ("B", len(b_example), b_declared),
        ("B declared", b_declared, 361),
        ("D", len(d_example), d_declared),
        ("D declared", d_declared, 277),
    ]
    for label, actual, expected in checks:
        if actual != expected:
            raise SystemExit(f"{label}: length {actual}, expected {expected}")

    test_lines = []
    for line in read("test-output.txt").splitlines():
        if line.startswith("test "):
            test_lines.append(line)
        elif line.startswith("test result:") and not line.startswith("test result: ok. 0 passed"):
            test_lines.append(line)
    if not any(line.startswith("test result: ok. 19 passed") for line in test_lines):
        raise SystemExit("expected 19 passing tests in test-output.txt")

    template = read("scripts/RESULTS.template.md")
    rendered = (
        template.replace("@@A_EXAMPLE@@", a_example)
        .replace("@@B_EXAMPLE@@", b_example)
        .replace("@@D_EXAMPLE@@", d_example)
        .replace("@@TEST_OUTPUT@@", "\n".join(test_lines))
        .replace("@@PRINT_OUTPUT@@", printed.rstrip("\n"))
        .replace("@@MUTATION_OUTPUT@@", read("mutation-output.txt").rstrip("\n"))
    )
    if "@@" in rendered:
        raise SystemExit("unfilled placeholder left in RESULTS.md")
    (ROOT / "RESULTS.md").write_text(rendered)
    print(f"wrote RESULTS.md: {rendered.count(chr(10)) + 1} lines")
    print(f"lengths checked: A={len(a_example)} B={len(b_example)} D={len(d_example)}")


if __name__ == "__main__":
    main()
