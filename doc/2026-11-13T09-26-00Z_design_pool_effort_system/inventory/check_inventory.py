#!/usr/bin/env python3
"""Check statement-map.json: unique ids, complete fields, verbatim sources.

Stdlib only. Run as: python3 -I inventory/check_inventory.py
Exits 0 when every statement passes, 1 otherwise.

Source resolution: brief.md and brief-decisions-supplement.md are read from the
design folder (the parent of this inventory/ directory). Rules whose source is
"launch-packet" are checked against LAUNCH_PACKET below, a verbatim copy of the
launch packet's standing-rules paragraph; the packet is not a file in the repo.
"""
import json
import re
import sys
from collections import Counter
from pathlib import Path

LAUNCH_PACKET = (
    "GitHub via gh CLI only; merge commits only, never force-push main; CI runs on main only; "
    "secrets by name only; a 100%-walled pool leg is never attempted; parked/abandoned design states "
    "are Drew's alone to set; starter issue is an epic under EPIC GUARD (never closed merely because "
    "subtasks complete); per-provider question \"is a native decision model worth it vs a cheap LLM "
    "(DeepSeek Flash class)\" must be answered with measured data; _commandcode seats are suspect "
    "(Command Code lost Sol plan coverage, free Beam tier ended permanently, monthly credits exhausted; "
    "PR #9401 removed it from six pools)."
)

KINDS = {"requirement", "preference", "example", "constraint", "fact-v1", "question", "rule"}
THEMES = {
    "T0", "T1", "T2", "T3", "T4", "T5", "T6", "T7", "T8", "T9",
    "T10", "T11", "T12", "T13", "T14", "T15", "T16", "T17", "T18",
}
DISPOSITIONS = {"v1-exists", "new-design", "new-slice", "research", "ruling-needed"}
COVERAGE = {"full", "partial", "related"}

DESIGN = Path(__file__).resolve().parent.parent
MAP = Path(__file__).resolve().parent / "statement-map.json"


def normalise(text):
    return re.sub(r"\s+", " ", text.replace("**", "")).strip()


def load_source(source):
    if source.startswith("launch-packet"):
        return normalise(LAUNCH_PACKET)
    path = DESIGN / Path(source).name
    if not path.is_file():
        raise FileNotFoundError(str(path))
    return normalise(path.read_text(encoding="utf-8"))


def main():
    rows = json.loads(MAP.read_text(encoding="utf-8"))
    failures = []
    ids = [r.get("id") for r in rows]
    dupes = sorted(i for i, n in Counter(ids).items() if n > 1)
    if dupes:
        failures.append(f"duplicate ids: {dupes}")

    sources = {}
    for r in rows:
        sid = r.get("id", "<no id>")
        for key in ("text", "theme", "kind"):
            if not str(r.get(key) or "").strip():
                failures.append(f"{sid}: empty {key}")
        if r.get("kind") not in KINDS:
            failures.append(f"{sid}: unknown kind {r.get('kind')!r}")
        theme_code = str(r.get("theme") or "").split(" ", 1)[0]
        if theme_code not in THEMES:
            failures.append(f"{sid}: unknown theme {r.get('theme')!r}")
        if r.get("disposition_hint") not in DISPOSITIONS:
            failures.append(f"{sid}: unknown disposition_hint {r.get('disposition_hint')!r}")
        for ex in r.get("existing_issues", []):
            if ex.get("coverage") not in COVERAGE:
                failures.append(f"{sid}: bad coverage {ex.get('coverage')!r}")
            for key in ("repo", "number", "title", "state"):
                if ex.get(key) in (None, ""):
                    failures.append(f"{sid}: existing issue missing {key}")

        source = r.get("source", "")
        if source not in sources:
            try:
                sources[source] = load_source(source)
            except OSError as err:
                failures.append(f"{sid}: cannot read source {source!r}: {err}")
                sources[source] = None
        body = sources[source]
        if body is not None and normalise(r.get("text", "")) not in body:
            failures.append(f"{sid}: text not found verbatim in {source}: {r.get('text', '')[:80]!r}")

    by_theme = Counter(r.get("theme", "") for r in rows)
    by_kind = Counter(r.get("kind", "") for r in rows)
    print(f"statements: {len(rows)}")
    print("per theme:")
    for theme in sorted(by_theme, key=lambda t: (int(t.split(" ")[0][1:]) if t.split(" ")[0][1:].isdigit() else 99)):
        print(f"  {by_theme[theme]:4d}  {theme}")
    print("per kind:")
    for kind in sorted(by_kind):
        print(f"  {by_kind[kind]:4d}  {kind}")
    if failures:
        print(f"FAIL: {len(failures)} problem(s)")
        for line in failures:
            print(f"  - {line}")
        return 1
    print("OK: ids unique, fields complete, every text verbatim in its source")
    return 0


if __name__ == "__main__":
    sys.exit(main())
