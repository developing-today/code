#!/usr/bin/env python3
# Copyright developing.today LLC & contributors to the project.
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Hierarchical items, status trails and generated views for research packages.

Loaded by research_workflow.py, which calls register() and check(). Commands:
`item add`, `item set-status`, `ids` and `render`. Grammar of an item line in
a concept's section of main.md (planning/workflow/records.md):

    - **C001.2** `clarified` · S2 · 2026-10-21T10:00:00Z — Note. ← `created` · S0 · 2026-10-19T06:09:02Z — Text.
      - **C001.2.1** `created` · S2 · 2026-10-21T10:00:00Z — A child, directly below its parent.

The newest trail entry comes first; older entries follow after ` ← ` and are
never removed. Generated views sit between `<!-- workflow:view NAME -->`
markers and are rewritten whole by `render`; `check` fails when one is stale.
"""

import importlib.util
import json
import os
import re
import sys
import tempfile
from datetime import datetime
from itertools import pairwise
from pathlib import Path


STATUSES = ("created", "clarified", "confirmed", "superseded", "retracted")
STAGES = tuple(f"S{n}" for n in range(9))
STAGE_NAMES = (
    "intake",
    "prepare",
    "research",
    "reconcile",
    "handoff",
    "fanout",
    "design",
    "plan",
    "implement",
)
RUN_STATUSES = (
    "not-started",
    "claimed",
    "running",
    "pushed",
    "pr-open",
    "merged",
    "validated",
    "blocked",
    "abandoned",
)
# BUILD-PLAN 3.6: these come from git and GitHub (`status --live`), never from state.
LIVE_ONLY = {"pushed", "pr-open", "merged"}
STAMP = r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ"
ID = r"C\d{3,}(?:\.\d+)*"
ITEM = re.compile(rf"^( *)- \*\*({ID})\*\* (.*)$")
LOOKS_LIKE_ITEM = re.compile(rf"^ *[-*+] +\*\*{ID}\.\d+\*\*")
ENTRY = re.compile(rf"^`([^`]+)` · (S\d+) · ({STAMP})(?: — (\S.*))?$")
SEPARATOR = re.compile(rf" ← (?=`[^`]+` · S\d+ · {STAMP}(?: — | ← |$))")
SEE = re.compile(rf"\bsee ({ID}(?:, {ID})*)")
VIEW_START = re.compile(r"^<!-- workflow:view ([A-Za-z0-9._-]+) -->$")
VIEW_END = re.compile(r"^<!-- /workflow:view ([A-Za-z0-9._-]+) -->$")
HEADING = re.compile(r"^ {0,3}(#{1,6})\s+(.+?)\s*#*$")
CONCEPT_HEADING = re.compile(rf"^## ({ID}): ")
ANCHOR_LINE = re.compile(r'^<a id="[^"]+"></a>$')
# Lines that `init` writes as placeholders and that a view supersedes when render
# first inserts it (the view states the same facts and stays current).
PLACEHOLDERS = {
    "concept": "Related issue/PR results: pending; none created.",
    "issues-and-prs": "None created. Each later entry must identify its concept, stage, status and disposition.",
    "run-status": "Not run. Research, design and feature planning remain separate later stages.",
}
CURRENT_STATUS = re.compile(
    r"^(Document: \[[^\]]+\]\([^)]+\)\.) Current status: [^.]+\.$"
)


def core():
    """Return research_workflow, loading it by path when imported on its own."""
    module = sys.modules.get("research_workflow")
    if module is None:
        spec = importlib.util.spec_from_file_location(
            "research_workflow", Path(__file__).with_name("research_workflow.py")
        )
        module = importlib.util.module_from_spec(spec)
        sys.modules["research_workflow"] = module
        spec.loader.exec_module(module)
    return module


def fail(message):
    raise ValueError(message)


# ---------------------------------------------------------------- documents


class Document:
    """A Markdown file as lines split only at "\\n", so edits keep every other byte.

    str.splitlines would also split at U+2028 and friends inside raw excerpts and
    shift line numbers; lines here keep any "\\r" so CRLF excerpts survive.
    """

    def __init__(self, path):
        self.path = path
        self.lines = path.read_bytes().decode("utf-8").split("\n")
        self.original = list(self.lines)

    def text(self):
        return "\n".join(self.lines)

    def changed(self):
        return self.lines != self.original

    def kinds(self):
        """Classify each line with research_workflow.fence_events (text/open/content/close)."""
        clean = [
            re.sub(r"[\r\x0b\x0c\x1c-\x1e\x85\u2028\u2029]", " ", x) for x in self.lines
        ]
        kinds = [kind for kind, _ in core().fence_events("\n".join(clean))]
        return kinds + ["text"] * (len(self.lines) - len(kinds))

    def prose(self):
        """Yield (index, line) for lines outside fenced blocks, without "\\r"."""
        for index, kind in enumerate(self.kinds()):
            if kind == "text":
                yield index, self.lines[index].rstrip("\r")

    def save(self):
        if not self.changed():
            return False
        handle, temporary = tempfile.mkstemp(dir=self.path.parent, prefix=".workflow-")
        try:
            with os.fdopen(handle, "wb") as stream:
                stream.write(self.text().encode("utf-8"))
            os.chmod(temporary, self.path.stat().st_mode & 0o777)
            os.replace(temporary, self.path)
        except BaseException:
            Path(temporary).unlink(missing_ok=True)
            raise
        self.original = list(self.lines)
        return True


def blank(line):
    return not line.strip()


def insert_item(lines, position, line):
    """Insert an item line inside a tight list: blank lines only where the list starts or ends."""
    listed = lambda x: x.startswith(" ") or ITEM.match(x.rstrip("\r"))  # noqa: E731
    before = (
        [""]
        if position > 0
        and not blank(lines[position - 1])
        and not listed(lines[position - 1])
        else []
    )
    after = (
        [""]
        if position < len(lines)
        and not blank(lines[position])
        and not listed(lines[position])
        else []
    )
    lines[position:position] = before + [line] + after


def splice(lines, start, end, block):
    """Replace lines[start:end] with block, keeping one blank line on each side."""
    before = [""] if start > 0 and not blank(lines[start - 1]) else []
    after = [""] if end < len(lines) and not blank(lines[end]) else []
    lines[start:end] = before + block + after
    return start + len(before)


def views(document):
    """Return {name: (start, end)} marker line indexes of generated views."""
    found, opened = {}, None
    for index, line in document.prose():
        start, end = VIEW_START.match(line), VIEW_END.match(line)
        if start:
            if opened:
                fail(f"{document.path}: view {start[1]} opens inside view {opened[0]}")
            if start[1] in found:
                fail(f"{document.path}: duplicate generated view {start[1]}")
            opened = (start[1], index)
        elif end:
            if not opened or opened[0] != end[1]:
                fail(f"{document.path}: unmatched end of generated view {end[1]}")
            found[end[1]] = (opened[1], index)
            opened = None
    if opened:
        fail(f"{document.path}: generated view {opened[0]} is never closed")
    return found


def view_block(name, content):
    return [
        f"<!-- workflow:view {name} -->",
        "",
        *content,
        "",
        f"<!-- /workflow:view {name} -->",
    ]


def view_content(document, span):
    start, end = span
    inner = [x.rstrip("\r") for x in document.lines[start + 1 : end]]
    if len(inner) < 3 or inner[0] or inner[-1]:
        fail(f"{document.path}: generated view needs a blank line inside each marker")
    return inner[1:-1]


def sections(document):
    """Return {concept_id: (heading, end)}; end excludes the next section's anchor."""
    headings = [
        (index, line)
        for index, line in document.prose()
        if (found := HEADING.match(line)) and len(found[1]) <= 2
    ]
    result = {}
    for position, (index, line) in enumerate(headings):
        found = CONCEPT_HEADING.match(line)
        if not found:
            continue
        end = (
            headings[position + 1][0]
            if position + 1 < len(headings)
            else len(document.lines)
        )
        while end > index + 1 and (
            blank(document.lines[end - 1])
            or ANCHOR_LINE.match(document.lines[end - 1].rstrip("\r"))
        ):
            end -= 1
        if found[1] in result:
            fail(f"{document.path}: duplicate section for {found[1]}")
        result[found[1]] = (index, end)
    return result


def heading_index(document, title):
    for index, line in document.prose():
        found = HEADING.match(line)
        if found and len(found[1]) == 2 and found[2] == title:
            return index
    return None


def section_end(document, start):
    for index, line in document.prose():
        found = HEADING.match(line)
        if index > start and found and len(found[1]) <= 2:
            return index
    return len(document.lines)


# -------------------------------------------------------------------- items


def parse_stamp(value):
    if not re.fullmatch(STAMP, value):
        fail(f"Timestamp must look like 2026-10-21T10:00:00Z: {value}")
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def parse_trail(rest, where):
    entries = []
    for part in SEPARATOR.split(rest):
        found = ENTRY.match(part)
        if not found:
            fail(
                f"{where}: trail entry does not match `status` · S<n> · <UTC> — text: {part}"
            )
        status, stage, at, text = found.groups()
        if status not in STATUSES:
            fail(
                f"{where}: unknown item status {status}; use one of {', '.join(STATUSES)}"
            )
        if stage not in STAGES:
            fail(f"{where}: unknown stage {stage}; use S0..S8")
        parse_stamp(at)
        entries.append({"status": status, "stage": stage, "at": at, "text": text})
    if not entries[0]["text"]:
        fail(f"{where}: the newest trail entry needs its text after ' — '")
    for newer, older in pairwise(entries):
        if parse_stamp(newer["at"]) < parse_stamp(older["at"]):
            fail(
                f"{where}: trail must run newest first ({newer['at']} before {older['at']})"
            )
    return entries


def owner(identifier, concepts):
    """Return the concept that owns an item id (longest matching concept prefix)."""
    matches = [c for c in concepts if identifier.startswith(c + ".")]
    return max(matches, key=len) if matches else None


def parse_items(document, concepts, created_at=None):
    """Parse and validate every item line in main.md; return {id: item}."""
    items, stack = {}, []
    spans = sections(document)
    for index, line in document.prose():
        found = ITEM.match(line)
        if not found:
            if LOOKS_LIKE_ITEM.match(line):
                fail(f"{document.path}:{index + 1}: malformed item line: {line}")
            if blank(line) or not line.startswith(" "):
                stack = []
            continue
        indent, identifier, rest = found.groups()
        where = f"{document.path}:{index + 1} {identifier}"
        concept = owner(identifier, concepts)
        if concept is None:
            fail(f"{where}: item does not belong to a known concept")
        if identifier in items:
            fail(f"{where}: duplicate item id")
        depth = identifier[len(concept) :].count(".")
        if len(indent) != 2 * (depth - 1):
            fail(f"{where}: indent must be {2 * (depth - 1)} spaces for depth {depth}")
        heading, end = spans.get(concept, (None, None))
        if spans and not (heading is not None and heading < index < end):
            fail(f"{where}: item must sit in the section of {concept}")
        stack = [x for x in stack if len(x[0]) < len(indent)]
        expected = identifier.rsplit(".", 1)[0]
        actual = stack[-1][1] if stack else concept
        if actual != expected:
            fail(f"{where}: item must sit directly under {expected}, not {actual}")
        entries = parse_trail(rest, where)
        if created_at and parse_stamp(entries[-1]["at"]) < parse_stamp(created_at):
            fail(f"{where}: trail starts before the package was created ({created_at})")
        parent = items.get(expected)
        if parent and parse_stamp(entries[-1]["at"]) < parse_stamp(
            parent["trail"][-1]["at"]
        ):
            fail(f"{where}: item is older than its parent {expected}")
        items[identifier] = {
            "id": identifier,
            "concept": concept,
            "parent": expected,
            "line": index,
            "indent": indent,
            "trail": entries,
        }
        stack.append((indent, identifier))
    for identifier, item in items.items():
        if item["parent"] != item["concept"] and item["parent"] not in items:
            fail(f"{document.path}: parent {item['parent']} of {identifier} is missing")
    children = {}
    for identifier, item in items.items():
        children.setdefault(item["parent"], []).append(
            int(identifier.rsplit(".", 1)[1])
        )
    for parent, numbers in children.items():
        if numbers != list(range(1, len(numbers) + 1)):
            fail(
                f"{document.path}: children of {parent} must be numbered 1..n in order (items are never deleted): {numbers}"
            )
    known = set(concepts) | set(items)
    for identifier, item in items.items():
        for entry in item["trail"]:
            for group in SEE.findall(entry["text"] or ""):
                for reference in group.split(", "):
                    if reference not in known:
                        fail(
                            f"{document.path}: {identifier} refers to unknown {reference}"
                        )
    return items


def block_end(document, index):
    """Return the index after an item's block: its line plus deeper-indented lines."""
    indent = len(document.lines[index]) - len(document.lines[index].lstrip(" "))
    end = index + 1
    while end < len(document.lines):
        line = document.lines[end]
        if blank(line) or len(line) - len(line.lstrip(" ")) <= indent:
            break
        end += 1
    return end


def single_line(value, name):
    if not isinstance(value, str) or not value.strip():
        fail(f"{name} must be nonempty text")
    if re.search(r"[\x00-\x1f\x7f\u2028\u2029]", value):
        fail(f"{name} must be one line without control characters")
    if "←" in value:
        fail(f"{name} must not contain ← (it separates trail entries)")
    return value.strip()


def stamp_now(value):
    return core().timestamp(value)


def vocabulary(args):
    if args.stage not in STAGES:
        fail(f"--stage must be one of S0..S8, not {args.stage}")
    if args.status not in STATUSES:
        fail(f"--status must be one of {', '.join(STATUSES)}, not {args.status}")


def entry_text(status, stage, at, text):
    return f"`{status}` · {stage} · {at} — {text}"


# -------------------------------------------------------------------- state


def stage_code(value):
    if value in STAGES:
        return value
    if value in STAGE_NAMES:
        return STAGES[STAGE_NAMES.index(value)]
    return None


def concept_ids(state):
    return [c["id"] for c in state["concepts"]]


def references(state, concept):
    """Issues and PRs for a concept: top-level entries with `concept`, plus the concept's own."""
    found = []
    for label, key in (("Issue", "issues"), ("PR", "prs")):
        entries = [e for e in state.get(key, []) if e.get("concept") == concept["id"]]
        entries += concept.get(key, [])
        found += [(label, e) for e in entries]
    return sorted(
        found, key=lambda x: (str(x[1].get("stage", "")), x[0], x[1].get("number", 0))
    )


def run_status(state, concept_id):
    """Recorded run status per stage from `tasks` entries carrying concept and stage."""
    runs = {}
    for task in state.get("tasks", []):
        if task.get("concept") == concept_id and task.get("stage") in STAGES:
            runs[task["stage"]] = task.get("status")
    return runs


def check_state(state):
    ids = set(concept_ids(state))
    for task in state.get("tasks", []):
        if "concept" not in task and "stage" not in task:
            continue
        if task.get("concept") not in ids or task.get("stage") not in STAGES:
            fail(f"Task {task.get('id')}: concept must be known and stage S0..S8")
        if task.get("status") not in RUN_STATUSES:
            fail(
                f"Task {task.get('id')}: run status must be one of {', '.join(RUN_STATUSES)}"
            )
        if task["status"] in LIVE_ONLY:
            fail(
                f"Task {task.get('id')}: {task['status']} comes from `status --live`; record validated after checking the merge"
            )
    for concept in state["concepts"]:
        for label, entry in references(state, concept):
            if not isinstance(entry.get("number"), int) or not isinstance(
                entry.get("url"), str
            ):
                fail(f"{concept['id']}: each {label} needs an integer number and a url")
            if "stage" in entry and entry["stage"] not in STAGES:
                fail(
                    f"{concept['id']}: {label} #{entry['number']} stage must be S0..S8"
                )
    for key in ("issues", "prs"):
        for entry in state.get(key, []):
            if entry.get("concept") not in ids:
                fail(f"{key} entry #{entry.get('number')} needs a known concept")


def waves(state):
    """Group open concepts into waves from hard `requires` edges.

    Wave 1 is what may start now (everything it requires is done), the same set
    as `ready`. Later waves assume earlier ones finish; running concepts count as
    finished after wave 1. Two concepts linked by `shares-files-with` never share
    a wave (nor wave 1 with a running concept); the later id moves on.
    """
    c = core()
    graph = c.dependencies(state)
    status = {x["id"]: x["status"] for x in state["concepts"]}
    done = sorted(k for k, v in status.items() if v in c.COMPLETE)
    running = sorted(k for k, v in status.items() if v in c.ACTIVE and k not in done)
    shared = {
        frozenset((e["from"], e["to"]))
        for e in state["dependencies"]
        if e["kind"] == "shares-files-with"
    }
    placed, remaining, result = (
        set(done),
        sorted(set(graph) - set(done) - set(running)),
        [],
    )
    while remaining:
        candidates = [n for n in remaining if all(p in placed for p in graph[n])]
        chosen = []
        for node in candidates:
            busy = chosen + (running if not result else [])
            if not any(frozenset((node, other)) in shared for other in busy):
                chosen.append(node)
        if not result:
            placed |= set(running)
        elif not chosen:
            fail("Cannot order concepts into waves")
        result.append(chosen)
        placed |= set(chosen)
        remaining = [n for n in remaining if n not in chosen]
    return {"done": done, "running": running, "waves": result}


# -------------------------------------------------------------------- views


def plain(value):
    """Render free text so prettier leaves it byte-for-byte alone."""
    text = re.sub(r"[\x00-\x1f\x7f\u2028\u2029]+", " ", str(value)).strip()
    return re.sub(r"([\\`*_\[\]<>~])", r"\\\1", text)


def target(document, concept, titles, prefix=""):
    """Link to a concept's section: its `<a id>` anchor, or the heading slug."""
    anchor = concept.lower()
    if f'id="{anchor}"' not in document.text():
        anchor = core().anchor(f"{concept}: {titles[concept]}")
    return f"[{concept}]({prefix}#{anchor})"


def reference_line(label, entry):
    parts = [f"{label} [#{entry['number']}]({entry['url']})"]
    if entry.get("stage"):
        parts.append(entry["stage"])
    if entry.get("state"):
        parts.append(f"`{plain(entry['state'])}`")
    line = " · ".join(parts)
    if entry.get("title"):
        line += f" — {plain(entry['title'])}"
    if entry.get("item"):
        line += f" (for {plain(entry['item'])})"
    if entry.get("reason"):
        line += f"; {plain(entry['reason'])}"
    return line


def concept_view(document, state, concept, items):
    titles = {c["id"]: c["title"] for c in state["concepts"]}
    cid = concept["id"]
    link = lambda x: target(document, x, titles)
    stage = concept.get("stage") or stage_code(state.get("stage")) or state.get("stage")
    facts = [f"`{plain(concept['status'])}`", f"stage {plain(stage)}"]
    for key in ("disposition", "priority", "criticality", "horizon"):
        if concept.get(key) and concept[key] != "unspecified":
            facts.append(f"{key} `{plain(concept[key])}`")
    lines = ["- **State:** " + " · ".join(facts)]
    if concept.get("goal"):
        lines.append(f"- **Goal:** {plain(concept['goal'])}")
    own = [x for x in items.values() if x["concept"] == cid]
    if own:
        counts = [
            f"{sum(x['trail'][0]['status'] == s for x in own)} `{s}`"
            for s in STATUSES
            if any(x["trail"][0]["status"] == s for x in own)
        ]
        lines.append(f"- **Items:** {len(own)} ({', '.join(counts)})")
    else:
        lines.append("- **Items:** none yet")
    outgoing = [e for e in state["dependencies"] if e["from"] == cid]
    if outgoing:
        lines.append("- **Relations:**")
        lines += [
            f"  - `{e['kind']}` {link(e['to'])}: {plain(e['reason'])}" for e in outgoing
        ]
    else:
        lines.append("- **Relations:** none")
    incoming = [e for e in state["dependencies"] if e["to"] == cid]
    required = [link(e["from"]) for e in incoming if e["kind"] == "requires"]
    if required:
        lines.append("- **Required by:** " + ", ".join(required))
    others = [
        f"{link(e['from'])} `{e['kind']}`" for e in incoming if e["kind"] != "requires"
    ]
    if others:
        lines.append("- **Referenced by:** " + ", ".join(others))
    refs = references(state, concept)
    if refs:
        lines.append("- **Issues and PRs:**")
        lines += ["  - " + reference_line(label, entry) for label, entry in refs]
    else:
        lines.append("- **Issues and PRs:** none yet")
    return lines


def issues_view(document, state):
    titles = {c["id"]: c["title"] for c in state["concepts"]}
    lines = []
    for concept in state["concepts"]:
        refs = references(state, concept)
        if refs:
            lines.append(
                f"- {target(document, concept['id'], titles)} {plain(concept['title'])}"
            )
            lines += ["  - " + reference_line(label, entry) for label, entry in refs]
    return lines or [
        "No issues or PRs are recorded yet. Each one will be listed here under its concept."
    ]


def stage_runs(state, concept):
    """Run status per stage: recorded tasks, S0 validated, next stage blocked when waiting."""
    recorded = run_status(state, concept["id"])
    runs = {stage: recorded.get(stage, "not-started") for stage in STAGES}
    if "S0" not in recorded:
        runs["S0"] = "validated"
    blocked = core().frontier(state)["blocked"].get(concept["id"])
    if blocked:
        for stage in STAGES:
            if runs[stage] == "not-started":
                runs[stage] = "blocked"
                break
    return runs, blocked


def run_view(main, state):
    titles = {c["id"]: c["title"] for c in state["concepts"]}
    lines = [
        (
            "S0 is `validated` when `render` runs, because render only runs on a package that passes `check`. "
            "Other stages show the `tasks` entries that record `concept`, `stage` and `status`. "
            "`pushed`, `pr-open` and `merged` come from git and GitHub through `status --live`, so they never appear here."
        ),
        "",
    ]
    for concept in state["concepts"]:
        runs, blocked = stage_runs(state, concept)
        refs = references(state, concept)
        groups = []
        for stage in STAGES:
            linked = [
                f"[#{e['number']}]({e['url']})"
                for _, e in refs
                if e.get("stage") == stage
            ]
            note = f" ({', '.join(linked)})" if linked else ""
            if runs[stage] == "blocked" and blocked:
                note += " (requires " + ", ".join(blocked) + ")"
            if (
                groups
                and not note
                and not groups[-1][3]
                and groups[-1][2] == runs[stage]
            ):
                groups[-1][1] = stage
            else:
                groups.append([stage, stage, runs[stage], note])
        text = " · ".join(
            (first if first == last else f"{first}–{last}") + f" `{status}`{note}"
            for first, last, status, note in groups
        )
        link = target(main, concept["id"], titles, "main.md")
        lines.append(
            f"- **{concept['id']}** [{plain(concept['title'])}]({link.split('](', 1)[1]}: {text}"
        )
    return lines


def waves_view(main, state):
    titles = {c["id"]: c["title"] for c in state["concepts"]}
    link = lambda x: target(main, x, titles, "main.md")
    plan = waves(state)
    lines = [
        (
            "Wave 1 can start now: everything it requires is done. Later waves assume the earlier ones finish. "
            "Concepts that share files never share a wave. Leases and open branches are live data, so run `ready` before dispatching."
        ),
        "",
        "- **Done:** " + (", ".join(map(link, plan["done"])) or "none"),
        "- **Running:** " + (", ".join(map(link, plan["running"])) or "none"),
    ]
    position = {node: n for n, wave in enumerate(plan["waves"]) for node in wave}
    for number, wave in enumerate(plan["waves"], 1):
        lines.append(f"- **Wave {number}:** " + (", ".join(map(link, wave)) or "none"))
        for edge in state["dependencies"]:
            if (
                edge["kind"] == "recommended-after"
                and edge["from"] in wave
                and position.get(edge["to"], len(plan["waves"])) >= number - 1
            ):
                cost = edge.get("rework_if_early") or edge["reason"]
                lines.append(
                    f"  - {link(edge['from'])} is better after {link(edge['to'])}: {plain(cost)}"
                )
    return lines


# ------------------------------------------------------------------- render


def load(package):
    state = core().read_state(package)
    check_state(state)
    main = Document(package / "main.md")
    design = (
        Document(package / "design.md") if (package / "design.md").is_file() else None
    )
    return state, main, design


def rendered(main):
    return bool(views(main))


def expected_views(state, main, design, items):
    result = {
        ("main", f"concept-{c['id']}"): concept_view(main, state, c, items)
        for c in state["concepts"]
    }
    result[("main", "issues-and-prs")] = issues_view(main, state)
    if design is not None:
        result[("design", "waves")] = waves_view(main, state)
        result[("design", "run-status")] = run_view(main, state)
    return result


def place_concept_view(main, state, concept, block):
    spans = sections(main)
    if concept["id"] not in spans:
        before = heading_index(main, "Issues and PRs")
        at = before if before is not None else len(main.lines)
        new = [
            f'<a id="{concept["id"].lower()}"></a>',
            "",
            f"## {concept['id']}: {concept['title']}",
            "",
        ]
        splice(main.lines, at, at, new + block)
        return
    heading, end = spans[concept["id"]]
    for index in range(heading + 1, end):
        found = CURRENT_STATUS.match(main.lines[index].rstrip("\r"))
        if found:
            main.lines[index] = found[1]
    for index in range(heading + 1, end):
        if main.lines[index].rstrip("\r") == PLACEHOLDERS["concept"]:
            splice(main.lines, index, index + 1, block)
            return
    first_item = next(
        (i for i in range(heading + 1, end) if ITEM.match(main.lines[i].rstrip("\r"))),
        end,
    )
    splice(main.lines, first_item, first_item, block)


def place_view(document, name, block, heading, placeholder=None, prefix=None):
    """Insert a missing view: over its placeholder, under its heading, or at the end."""
    lines = [x.rstrip("\r") for x in document.lines]
    for index, line in document.prose():
        if line == placeholder or (prefix and line.startswith(prefix)):
            splice(document.lines, index, index + 1, block)
            return
    at = heading_index(document, heading)
    if at is not None:
        splice(document.lines, at + 1, at + 1, block)
        return
    end = len(lines) - 1 if lines and not lines[-1] else len(lines)
    splice(document.lines, end, end, [f"## {heading}", "", *block])


def apply_views(state, main, design, items, only=None):
    """Write expected views into the documents; `only` limits which are touched."""
    wanted = expected_views(state, main, design, items)
    for (where, name), content in wanted.items():
        if only is not None and name not in only:
            continue
        document = main if where == "main" else design
        block = view_block(name, content)
        found = views(document)
        if name in found:
            start, end = found[name]
            document.lines[start : end + 1] = block
        elif name.startswith("concept-"):
            concept = next(c for c in state["concepts"] if f"concept-{c['id']}" == name)
            place_concept_view(document, state, concept, block)
        elif name == "issues-and-prs":
            place_view(document, name, block, "Issues and PRs", PLACEHOLDERS[name])
        elif name == "waves":
            place_view(document, name, block, "Waves", prefix="Ready frontier: ")
        else:
            place_view(document, name, block, "Review status", PLACEHOLDERS[name])


def render(package, root):
    core().check_package(package, root)
    state, main, design = load(package)
    items = parse_items(main, concept_ids(state), state.get("created_at"))
    apply_views(state, main, design, items)
    # Placing a view can add an anchor that other views link to: settle in a second pass.
    apply_views(state, main, design, parse_items(main, concept_ids(state)))
    written = [d.path.name for d in (main, design) if d is not None and d.save()]
    return {"package": str(package), "written": written}


def check(package, root):
    """Validate items everywhere; once a package is rendered, every view must be current."""
    state, main, design = load(package)
    items = parse_items(main, concept_ids(state), state.get("created_at"))
    if not rendered(main):
        return
    wanted = expected_views(state, main, design, items)
    present = {("main", n): s for n, s in views(main).items()}
    if design is not None:
        present |= {("design", n): s for n, s in views(design).items()}
    for key in present.keys() - wanted.keys():
        fail(f"{package / (key[0] + '.md')}: unknown generated view {key[1]}")
    for key, content in wanted.items():
        path = package / f"{key[0]}.md"
        if key not in present:
            fail(f"{path}: generated view {key[1]} is missing; run render")
        document = main if key[0] == "main" else design
        if view_content(document, present[key]) != content:
            fail(f"{path}: generated view {key[1]} is stale; run render")


# -------------------------------------------------------------------- edits


def append_change(package, line):
    path = package / "changes.md"
    if not path.exists():
        with path.open("xb") as stream:
            stream.write(b"# Changes\n\n[Main reference](main.md)\n\n")
    with path.open("rb+") as stream:
        stream.seek(0, os.SEEK_END)
        if stream.tell():
            stream.seek(-1, os.SEEK_END)
            if stream.read(1) != b"\n":
                stream.write(b"\n")
        stream.write((line + "\n").encode("utf-8"))


def edit(package, apply):
    """Run one item edit: validate before and after, refresh the concept view, write."""
    state, main, design = load(package)
    ids = concept_ids(state)
    items = parse_items(main, ids, state.get("created_at"))
    if not rendered(main):
        fail(f"{package / 'main.md'} has no generated views yet; run render first")
    result, concept = apply(state, main, items)
    items = parse_items(main, ids, state.get("created_at"))
    apply_views(state, main, design, items, only={f"concept-{concept}"})
    main.save()
    append_change(package, result.pop("change"))
    return result


def item_add(args, root, config, package):
    vocabulary(args)
    text = single_line(args.text, "--text")
    at = stamp_now(args.at)

    def apply(state, main, items):
        ids = concept_ids(state)
        parent = args.parent
        if parent in ids:
            concept = parent
            spans = sections(main)
            if concept not in spans:
                fail(f"main.md has no section for {concept}; run render first")
            heading, end = spans[concept]
            tops = [x for x in items.values() if x["parent"] == concept]
            if tops:
                position = block_end(main, tops[-1]["line"])
            else:
                found = views(main).get(f"concept-{concept}")
                if not found or not heading < found[0] < end:
                    fail(
                        f"main.md section {concept} has no generated view; run render first"
                    )
                position = found[1] + 1
        elif parent in items:
            concept = items[parent]["concept"]
            position = block_end(main, items[parent]["line"])
        else:
            fail(f"Unknown parent {parent}; use a concept id or an existing item id")
        siblings = [
            int(x.rsplit(".", 1)[1]) for x, i in items.items() if i["parent"] == parent
        ]
        identifier = f"{parent}.{max(siblings, default=0) + 1}"
        depth = identifier[len(concept) :].count(".")
        line = f"{'  ' * (depth - 1)}- **{identifier}** {entry_text(args.status, args.stage, at, text)}"
        insert_item(main.lines, position, line)
        return {
            "id": identifier,
            "parent": parent,
            "line": line,
            "change": f"- {at} | {args.stage} | {args.status} | {identifier} added under {parent}: {text}",
        }, concept

    return edit(package, apply)


def item_set_status(args, root, config, package):
    vocabulary(args)
    if args.status == "created":
        fail(
            "An existing item cannot become created; use clarified, confirmed, superseded or retracted"
        )
    note = single_line(args.note, "--note")
    at = stamp_now(args.at)
    if args.status == "superseded" and not args.see:
        fail("superseded needs --see with the item that replaces it")

    def apply(state, main, items):
        item = items.get(args.id)
        if item is None:
            fail(f"Unknown item {args.id}")
        known = set(concept_ids(state)) | set(items)
        for reference in args.see:
            if reference not in known or reference == args.id:
                fail(f"--see {reference} must name another existing item or concept")
        head = item["trail"][0]
        if parse_stamp(at) < parse_stamp(head["at"]):
            fail(f"{args.id}: {at} is earlier than its newest entry {head['at']}")
        text = (
            note.rstrip(".")
            + ("; see " + ", ".join(args.see) if args.see else "")
            + "."
        )
        index = item["line"]
        raw = main.lines[index]
        ending = "\r" if raw.endswith("\r") else ""
        prefix, rest = ITEM.match(raw.rstrip("\r")).group(1, 3)
        line = f"{prefix}- **{args.id}** {entry_text(args.status, args.stage, at, text)} ← {rest}"
        main.lines[index] = line + ending
        return {
            "id": args.id,
            "status": args.status,
            "line": line,
            "change": f"- {at} | {args.stage} | {args.status} | {args.id} {head['status']} → {args.status}: {text}",
        }, item["concept"]

    return edit(package, apply)


def list_ids(args, root, config, package):
    state, main, _ = load(package)
    items = parse_items(main, concept_ids(state), state.get("created_at"))
    manifest = package / "package.json"
    segments = (
        json.loads(manifest.read_text()).get("segments", [])
        if manifest.is_file()
        else []
    )
    return {
        "package": str(package),
        "concepts": [
            {"id": c["id"], "title": c["title"], "status": c["status"]}
            for c in state["concepts"]
        ],
        "items": [
            {
                "id": i["id"],
                "status": i["trail"][0]["status"],
                "stage": i["trail"][0]["stage"],
                "at": i["trail"][0]["at"],
            }
            for i in items.values()
        ],
        "segments": [s["id"] for s in segments],
        "notes": sorted(p.stem for p in (package / "notes").glob("*.md")),
        "decisions": sorted(
            p.name.split("-", 1)[0]
            for p in (package / "decisions").glob("D[0-9][0-9][0-9]*.md")
        ),
        "reserved": ["G"],
    }


def render_command(args, root, config, package):
    return render(package, root)


def register(sub):
    def command(parser, name, handler, help_text):
        result = parser.add_parser(name, help=help_text)
        result.add_argument("--repo-root")
        result.add_argument("--package", required=True)
        result.set_defaults(handler=handler)
        return result

    item = sub.add_parser("item", help="add an item or change its status")
    actions = item.add_subparsers(dest="action", required=True)
    add = command(actions, "add", item_add, "add a child item below its parent")
    add.add_argument(
        "--parent", required=True, help="concept id (C001) or item id (C001.2)"
    )
    add.add_argument("--stage", required=True, help="S0..S8")
    add.add_argument("--status", default="created", help=", ".join(STATUSES))
    add.add_argument("--text", required=True)
    add.add_argument("--at", help="UTC timestamp; defaults to now")
    status = command(
        actions, "set-status", item_set_status, "prepend a status to an item's trail"
    )
    status.add_argument("--id", required=True)
    status.add_argument("--status", required=True, help=", ".join(STATUSES[1:]))
    status.add_argument("--stage", required=True, help="S0..S8")
    status.add_argument("--note", required=True)
    status.add_argument(
        "--see", action="append", default=[], help="item or concept the note points to"
    )
    status.add_argument("--at", help="UTC timestamp; defaults to now")
    command(sub, "ids", list_ids, "list concept, item, segment, note and decision ids")
    command(
        sub, "render", render_command, "regenerate the views in main.md and design.md"
    )
