"""Mechanical checks for a feature plan written in the ordering.md step format.

Usage: python3 -I plan_check.py <plan.md> [<package dir>]

Prints one line per problem, or "ok: <n> steps in <w> waves". With a package
directory it also lists `stated` and `human-confirmed` decisions no step cites.
Exits 1 when it finds a problem.
"""

import re
import sys
from collections import defaultdict
from pathlib import Path

FIELDS = (
    "Tier:",
    "Goal:",
    "Enables:",
    "Requires:",
    "Files:",
    "Cites:",
    "Test first:",
    "Done when:",
    "Size:",
)


def parse(text):
    wave, step, steps = None, None, []
    for line in text.splitlines():
        if found := re.match(r"### Wave (\d+)", line):
            wave, step = int(found[1]), None
        elif line.startswith("## "):
            wave, step = None, None
        elif wave and (found := re.match(r"(\d+)\. \*\*(\[P\] )?", line)):
            step = {
                "number": int(found[1]),
                "wave": wave,
                "parallel": bool(found[2]),
                "text": "",
            }
            steps.append(step)
        elif step is not None and line.startswith("   - "):
            step["text"] += line + "\n"
    for fields in steps:
        text = fields["text"]
        files = re.search(r"Files: (.*?)(?: Cites:|$)", text, re.MULTILINE)
        fields["files"] = re.findall(r"`([^`]+)`", files[1]) if files else []
        bare = re.sub(r"\([^)]*\)", "", text)
        requires = re.search(r"Requires: ([^.\n]*)", bare)
        fields["requires"] = (
            [int(n) for n in re.findall(r"\d+", requires[1])] if requires else []
        )
    return steps


def problems(steps, package):
    found = []
    by_number = {}
    for step in steps:
        number = step["number"]
        if number in by_number:
            first = by_number[number]
            found.append(
                f"duplicate step number {number}: first in wave {first['wave']}, "
                f"repeated in wave {step['wave']}"
            )
        else:
            by_number[number] = step
    for step in steps:
        number = step["number"]
        found += [
            f"step {number}: missing {f}" for f in FIELDS if f not in step["text"]
        ]
        done = re.search(r"Done when: (.*)", step["text"])
        if done and "`" not in done[1]:
            found.append(f"step {number}: done check is not a command")
        for target in step["requires"]:
            if target >= number or target not in by_number:
                found.append(f"step {number}: requires {target}, not a lower step")
            elif by_number[target]["wave"] >= step["wave"]:
                found.append(
                    f"step {number}: requires {target} in its own or a later wave"
                )
    waves = defaultdict(list)
    for step in steps:
        waves[step["wave"]].append(step)
    for wave, wave_steps in sorted(waves.items()):
        writers = defaultdict(list)
        for step in wave_steps:
            number = step["number"]
            for path in step["files"]:
                writers[path].append(number)
            if len(wave_steps) > 1 and not step["parallel"]:
                found.append(
                    f"wave {wave}: step {number} has no [P] but shares the wave"
                )
        found += [
            f"wave {wave}: {path} written by steps {', '.join(map(str, ns))}"
            for path, ns in writers.items()
            if len(ns) > 1
        ]
    if package:
        cited = "".join(step["text"] for step in steps)
        for record in sorted(Path(package, "decisions").glob("D*.md")):
            source = re.search(r"^- Source: (\S+)", record.read_text(), re.MULTILINE)
            decision = record.name.split("-")[0]
            confirmed = source and source[1] in {"stated", "human-confirmed"}
            if confirmed and not re.search(rf"\b{decision}\b", cited):
                found.append(f"{decision} ({source[1]}) is cited by no step")
    return found


def main():
    if len(sys.argv) not in (2, 3):
        sys.exit(__doc__)
    steps = parse(Path(sys.argv[1]).read_text(encoding="utf-8"))
    found = problems(steps, sys.argv[2] if len(sys.argv) == 3 else None)
    if not steps:
        found.append("no steps found under '### Wave <n>' headings")
    waves = len({step["wave"] for step in steps})
    print("\n".join(found) or f"ok: {len(steps)} steps in {waves} waves")
    sys.exit(1 if found else 0)


if __name__ == "__main__":
    main()
