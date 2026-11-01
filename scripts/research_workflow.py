#!/usr/bin/env python3
# Copyright developing.today LLC & contributors to the project.
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Scaffold and inspect evidence-preserving research packages (Python stdlib only)."""

import argparse
import hashlib
import importlib.util
import json
import re
import shutil
import sys
import uuid
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import unquote, urlsplit


PROFILE = Path(".agents/research-workflow.json")
KINDS = {
    "requires",
    "recommended-after",
    "conflicts-with",
    "shares-files-with",
    "enables",
    "optional-if",
}
COMPLETE = {
    "complete",
    "completed",
    "done",
    "validated",
    "researched",
    "designed",
    "planned",
    "merged",
}
ACTIVE = {"running", "active", "in-progress", "in progress"}
STATE = re.compile(r"<!-- workflow:state -->\s*(.*?)\s*<!-- /workflow:state -->", re.S)
RESOLUTION = re.compile(
    r"<!-- workflow:resolution -->\s*(.*?)\s*<!-- /workflow:resolution -->", re.S
)
NOTE = re.compile(r"<!-- workflow:note -->\s*(.*?)\s*<!-- /workflow:note -->", re.S)


EXTENSIONS = ("research_workflow_items", "research_workflow_live")
# Extensions register readers (package, root) -> {note path: entry} for note
# dispositions kept outside concept files; each reader validates what it returns.
DISPOSITION_READERS = {}


def extensions():
    """Load sibling command modules by path; `python3 -I` keeps scripts/ off sys.path."""
    sys.modules.setdefault("research_workflow", sys.modules[__name__])
    for name in EXTENSIONS:
        spec = importlib.util.spec_from_file_location(
            name, Path(__file__).with_name(name + ".py")
        )
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        yield module


def fail(message):
    raise ValueError(message)


def contained(root, relative):
    path = Path(relative)
    if path.is_absolute() or ".." in path.parts:
        fail(f"Expected contained relative path: {relative}")
    result = (root / path).resolve()
    if not result.is_relative_to(root.resolve()):
        fail(f"Path escapes repository/package: {relative}")
    return result


def profile(root):
    data = json.loads((root / PROFILE).read_text())
    if data.get("version") != 1:
        fail("Unsupported research-workflow profile version")
    for key in (
        "workflow_docs",
        "requests_root",
        "plans_root",
        "request_catalog",
        "plan_catalog",
    ):
        if not isinstance(data.get(key), str):
            fail(f"Profile requires string {key}")
        contained(root, data[key])
    for key in ("reference_roots", "validation_commands"):
        if not isinstance(data.get(key), list) or not all(
            isinstance(x, str) for x in data[key]
        ):
            fail(f"Profile requires string list {key}")
    for path in data["reference_roots"]:
        contained(root, path)
    for key in ("issue_repository", "trunk"):
        if not isinstance(data.get(key), str) or not data[key].strip():
            fail(f"Profile requires nonempty {key}")
    return data


def timestamp(value=None):
    date = (
        datetime.fromisoformat(value.replace("Z", "+00:00"))
        if value
        else datetime.now(timezone.utc)
    )
    if date.tzinfo is None:
        fail("Timestamp must include timezone")
    return (
        date.astimezone(timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z")
    )


def slug(value):
    if not re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*", value):
        fail(f"Invalid lowercase kebab slug: {value}")
    return value


def marker(kind, data):
    payload = json.dumps(data, ensure_ascii=False, indent=2).replace("<", "\\u003c")
    return f"<!-- workflow:{kind} -->\n{payload}\n<!-- /workflow:{kind} -->\n"


def excerpt(identifier, pair, lines, transcript):
    start, end = pair

    def clean_line(line):
        if line.endswith("\r\n"):
            return line[:-2].rstrip(" \t") + "\r\n"
        elif line.endswith("\n"):
            return line[:-1].rstrip(" \t") + "\n"
        return line.rstrip(" \t")

    raw = "".join(clean_line(line) for line in lines[start - 1 : end])
    fence = "`" * max(3, max((len(x) + 1 for x in re.findall(r"`+", raw)), default=0))
    return (
        f"<!-- workflow:excerpt {identifier} {start} {end} -->\n\n"
        f"[Transcript lines {start}-{end}]({transcript}#L{start}-L{end})\n\n{fence}text\n"
        f"{raw}" + ("" if raw.endswith("\n") else "\n") + f"{fence}\n"
    )


def coverage(concepts, lines):
    covered = set()
    for concept in concepts:
        ranges = concept.get("source_lines")
        if not isinstance(ranges, list) or not ranges:
            fail(f"{concept.get('id', 'concept')}: source_lines required")
        own = set()
        for pair in ranges:
            if (
                not isinstance(pair, list)
                or len(pair) != 2
                or any(type(x) is not int for x in pair)
                or not 1 <= pair[0] <= pair[1] <= len(lines)
            ):
                fail(f"Invalid source line range: {pair}")
            span = set(range(pair[0], pair[1] + 1))
            if own & span:
                fail(
                    f"Conflicting overlapping source ranges in {concept.get('id', 'concept')}"
                )
            own |= span
        covered |= own
    missing = [
        n for n, line in enumerate(lines, 1) if line.strip() and n not in covered
    ]
    if missing:
        fail(
            f"Uncovered nonblank transcript lines: {missing}; assign an unresolved concept"
        )


def dependencies(state):
    ids = {x["id"] for x in state["concepts"]}
    graph = {x: [] for x in ids}
    seen = set()
    for edge in state["dependencies"]:
        key = (edge.get("from"), edge.get("to"), edge.get("kind"))
        if (
            key[0] not in ids
            or key[1] not in ids
            or key[0] == key[1]
            or key[2] not in KINDS
        ):
            fail(f"Invalid dependency: {edge}")
        if (
            key in seen
            or not isinstance(edge.get("reason"), str)
            or not edge["reason"].strip()
        ):
            fail(f"Duplicate dependency or missing reason: {edge}")
        seen.add(key)
        if edge["kind"] == "requires":
            graph[edge["from"]].append(edge["to"])

    def visit(node, stack, finished):
        if node in stack:
            fail("Hard dependency cycle: " + " -> ".join(stack + [node]))
        if node not in finished:
            for child in graph[node]:
                visit(child, stack + [node], finished)
            finished.add(node)

    done = set()
    for node in sorted(graph):
        visit(node, [], done)
    return graph


def scaffold(args, root, config):
    raw = Path(args.input).read_bytes()
    text = raw.decode("utf-8")
    lines = text.splitlines(keepends=True)
    if not text.strip():
        fail("Input must contain nonblank UTF-8 text")
    transcript_sha = hashlib.sha256(raw).hexdigest()
    if getattr(args, "expect_sha256", None) and args.expect_sha256 != transcript_sha:
        fail(
            f"Transcript sha256 mismatch: expected {args.expect_sha256}, got {transcript_sha}"
        )
    if not getattr(args, "allow_duplicate", False):
        requests_dir = contained(root, config["requests_root"])
        if requests_dir.is_dir():
            for existing in requests_dir.glob("*_research_*/main.md"):
                if f'"sha256": "{transcript_sha}"' in existing.read_text():
                    fail(
                        f"Same transcript already captured in {existing.parent.name}; pass --allow-duplicate to recapture"
                    )
    created = timestamp(args.timestamp)
    name = created.replace(":", "-") + "_research_" + slug(args.slug)
    package = contained(root, config["requests_root"]) / name
    if package.exists() or package.is_symlink():
        fail(f"Package already exists: {package}")
    outline = (
        json.loads(Path(args.outline).read_text())
        if args.outline
        else {
            "title": args.concept or args.slug.replace("-", " ").title(),
            "introduction": "Prepared from the exact request; detailed research has not run.",
            "concepts": [
                {
                    "title": args.concept or args.slug.replace("-", " ").title(),
                    "slug": args.slug,
                    "summary": "Organize and investigate the request without losing its original meaning.",
                    "source_lines": [[1, len(lines)]],
                    "thoughts": [],
                    "suggestions": [],
                }
            ],
            "dependencies": [],
        }
    )
    if re.search(r"[\r\n\u2028\u2029]", outline.get("title", "")):
        fail("Outline title must be a single line")
    concepts = outline.get("concepts")
    if not isinstance(concepts, list) or not concepts:
        fail("Outline requires at least one concept")
    multi = len(concepts) > 1
    items, slugs = [], set()
    for index, concept in enumerate(concepts, 1):
        if (
            not isinstance(concept.get("title"), str)
            or not concept["title"].strip()
            or re.search(r"[\r\n\u2028\u2029]", concept["title"])
        ):
            fail("Each concept requires a single-line title")
        suffix = slug(concept.get("slug", "concept-" + str(index)))
        if suffix in slugs:
            fail(f"Duplicate concept slug: {suffix}")
        slugs.add(suffix)
        items.append(
            {
                "id": f"C{index:03}",
                "title": concept["title"],
                "path": f"concepts/C{index:03}-{suffix}.md" if multi else "main.md",
                "status": "created",
                "source_lines": concept.get("source_lines"),
                **{
                    k: concept[k]
                    for k in ("priority", "criticality", "horizon")
                    if k in concept
                },
            }
        )
        for key in ("thoughts", "suggestions"):
            if not isinstance(concept.get(key, []), list) or not all(
                isinstance(x, str) for x in concept.get(key, [])
            ):
                fail(f"{key} must be a string list")
    coverage(items, lines)
    state = {
        "version": 1,
        "id": name,
        "created_at": created,
        "stage": "intake",
        "status": "prepared",
        "transcript": {"path": "transcript.txt", "sha256": transcript_sha},
        "concepts": items,
        "dependencies": outline.get("dependencies", []),
        "issues": [],
        "prs": [],
        "tasks": [],
        "events": [
            {
                "at": created,
                "stage": "intake",
                "status": "prepared",
                "reason": "Exact transcript captured and initial concept files prepared.",
            }
        ],
    }
    dependencies(state)
    title = outline.get("title", args.slug.replace("-", " ").title())
    intro = outline.get(
        "introduction", "Prepared from the exact request; research has not run."
    )
    main = f"# {title}\n\n{intro}\n\n" + marker("state", state)
    inventory = ["main.md", "transcript.txt"] + (
        ["design.md", "changes.md"] + [i["path"] for i in items] if multi else []
    )
    main += "\n## Files\n\n" + "".join(f"- [{p}]({p})\n" for p in inventory)
    main += f"\nTranscript SHA-256: `{state['transcript']['sha256']}`. Source bytes are immutable.\n"
    files = {"transcript.txt": raw}
    for item, concept in zip(items, concepts, strict=True):
        identifier = item["id"]
        main += (
            f'\n<a id="{identifier.lower()}"></a>\n\n## {identifier}: {item["title"]}\n\n'
            f"Document: [{item['path']}]({item['path']}). Current status: created.\n\n"
            f"{concept.get('summary', 'Initial concept extracted from the request.')}\n\n"
            "Related issue/PR results: pending; none created.\n"
        )
        body = f"# {identifier}: {item['title']}\n\n" if multi else ""
        body += (
            ("[Main reference](../main.md#" + identifier.lower() + ")\n\n")
            if multi
            else ""
        )
        body += "## Relevant transcript excerpts\n\n"
        transcript = "../transcript.txt" if multi else "transcript.txt"
        body += "\n".join(
            excerpt(identifier, pair, lines, transcript)
            for pair in item["source_lines"]
        )
        body += (
            "\n## Normalized purpose\n\n"
            + concept.get("summary", "Initial concept extracted from the request.")
            + "\n"
        )
        body += (
            "\n## Initial thoughts\n\n"
            + (
                "\n".join("- " + x for x in concept.get("thoughts", []))
                or "Not yet explored."
            )
            + "\n"
        )
        body += (
            "\n## Inferred suggestions\n\n"
            + (
                "\n".join("- Inference: " + x for x in concept.get("suggestions", []))
                or "No additional suggestions recorded yet."
            )
            + "\n"
        )
        body += "\n## Stage results\n\nPending: research, synthesis, design, planning and implementation have not run.\n"
        if multi:
            files[item["path"]] = body.encode()
        else:
            main += "\n" + body
    main += "\n## Issues and PRs\n\nNone created. Each later entry must identify its concept, stage, status and disposition.\n"
    if not multi:
        main += f"\n## Change log\n\n- {created} | intake | prepared | Initial capture; research not run.\n"
    files["main.md"] = main.encode()
    if multi:
        ready = frontier(state)
        design = "# Initial review and research order\n\n[Main reference](main.md) | [Transcript](transcript.txt)\n\n"
        design += "Hard prerequisites use `requires`; other edges express sequencing cost, conflicts, shared files or optional enabling work.\n\n"
        design += (
            "Ready frontier: "
            + ", ".join(ready["ready"])
            + ". Run one agent per ready concept with exclusive owned files.\n\n"
        )
        design += (
            "## Dependencies\n\n"
            + (
                "\n".join(
                    f"- {e['from']} {e['kind']} {e['to']}: {e['reason']}"
                    for e in state["dependencies"]
                )
                or "None identified; all concepts may be prepared in parallel."
            )
            + "\n"
        )
        design += "\n## Review status\n\nNot run. Research, design and feature planning remain separate later stages.\n"
        files["design.md"] = design.encode()
        files["changes.md"] = (
            f"# Changes\n\n[Main reference](main.md)\n\n- {created} | intake | prepared | Initial transcript and concept scaffold created.\n".encode()
        )
    # No filesystem mutations until the entire source/outline/dependency preflight succeeds.
    package.parent.mkdir(parents=True, exist_ok=True)
    package.mkdir()
    try:
        for relative, content in files.items():
            path = contained(package, relative)
            path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("xb") as stream:
                stream.write(content)
    except Exception:
        shutil.rmtree(package)
        raise
    return {"package": str(package), "id": name}


def anchor(text):
    return re.sub(r"[^\w\- ]", "", text.lower()).replace(" ", "-")


def fence_events(text):
    """Classify each line as text, open, content or close of a fence.

    Fences nest: a fence line carrying an info string (e.g. ```python)
    can never close a block — a CommonMark closing fence has no info
    string — so inside a block it opens a nested fence instead, and
    only a bare fence at least as long as the innermost one closes a
    level. A naive open/close toggle splits a block at the first
    inner fence (#393): a trigger-issue draft embedding a code example
    fell into two fragments, neither carrying both its links and its
    trailing workflow:issue marker, so neither was checked; the code
    example's own contents also leaked out and were read as prose.
    """
    stack = []
    for line in text.splitlines():
        found = re.match(r"^ {0,3}(`{3,}|~{3,})\s*(.*?)\s*$", line)
        if not found:
            yield ("content" if stack else "text"), line
            continue
        token, info = found[1], found[2]
        if not stack:
            stack = [token]
            yield "open", line
        elif token[0] != stack[-1][0]:
            # A fence of the other character is literal block content.
            yield "content", line
        elif not info and len(token) >= len(stack[-1]):
            stack.pop()
            # An inner fence's closing line stays in the outer block
            # so the nested block can be stripped whole downstream.
            yield ("content" if stack else "close"), line
        else:
            # An info-string fence opens a nested block; a bare fence
            # shorter than the innermost one is its literal content.
            if info:
                stack.append(token)
            yield "content", line


def unfenced(text):
    """Exclude raw evidence/code from authored workflow syntax and link checks."""
    return "\n".join(line for kind, line in fence_events(text) if kind == "text")


def prose(text):
    clean = re.sub(r"<!--.*?-->", "", unfenced(text), flags=re.S)
    return re.sub(r"(`+)(?:(?!\1).)+?\1", "", clean)


def _fenced_blocks(text):
    """Yield (content, closed) for each fenced block, in order.

    A trailing block whose fence is never closed is yielded as well
    (closed=False), so a trigger-issue draft in an unclosed fence is
    checked rather than silently dropped (#393).
    """
    current = None
    for kind, line in fence_events(text):
        if kind == "open":
            current = []
        elif kind == "content" and current is not None:
            current.append(line)
        elif kind == "close":
            yield "\n".join(current), True
            current = None
    if current is not None:
        yield "\n".join(current), False


def fenced_blocks(text):
    """Return the contents of each fenced code block, in order.

    A trailing block whose fence is never closed is returned as well,
    so a trigger-issue draft in an unclosed fence is checked rather
    than silently dropped (#393).
    """
    return [block for block, _closed in _fenced_blocks(text)]


def draft_prose(block):
    """Return a draft's prose with embedded code examples stripped.

    Examples are buffered while their fence is open and discarded
    when it closes, so a properly closed example's contents stay
    exempt from link checks (#393). Two authoring slips are recovered
    instead of silently dropping the rest of the draft (#401):

    - a bare fence closes the innermost example whatever its length:
      an example opened with four backticks and closed with three
      was left open by fence_events()' length rule, and every line
      after the attempted close went unchecked;
    - an example still open at the end of the draft has no known
      end, so its buffered lines are kept as prose and checked
      rather than dropped: a draft whose example is never closed
      cannot exempt everything after the example's opening fence.
      Authors who want example contents exempt must close the
      example.
    """
    out, buffer, stack = [], [], []
    for line in block.splitlines():
        found = re.match(r"^ {0,3}(`{3,}|~{3,})\s*(.*?)\s*$", line)
        if not found:
            (buffer if stack else out).append(line)
            continue
        token, info = found[1], found[2]
        if stack and token[0] == stack[-1][0] and not info:
            stack.pop()
            if stack:
                buffer.append(line)
            else:
                buffer = []
        elif stack and token[0] != stack[-1][0]:
            # A fence of the other character is literal example content.
            buffer.append(line)
        else:
            if stack:
                # An info-string fence opens a nested example.
                stack.append(token)
                buffer.append(line)
            else:
                stack = [token]
                buffer = [line]
    if stack:
        out.extend(buffer)
    return "\n".join(out)


def concept_pointers(text):
    """Return the draft's Concept Document pointer lines, as a set.

    The pointer line (``- Concept Document: [..](..)``) is, with the
    workflow:issue marker, one of the two elements that define a
    trigger-issue draft: it is the link the draft check exists to
    validate (#382).
    """
    return {
        line.strip()
        for line in text.splitlines()
        if re.match(r"^ {0,3}- Concept Document:", line)
    }


def issue_drafts(text):
    """Yield fenced trigger-issue drafts: blocks carrying a workflow:issue marker.

    A draft is authored to be copied out of its document into a GitHub issue,
    so its links must resolve from the document's own directory — the same
    base check_link applies to ordinary prose links. Other fenced blocks are
    raw evidence/code and stay exempt, as unfenced() documents.
    """
    for block, closed in _fenced_blocks(text):
        if "workflow:issue" in block:
            if not closed:
                # The draft's fence was never closed at document level,
                # so a trailing bare fence line in the block may be the
                # draft's own closing fence, consumed as the embedded
                # example's close (#401): dropping it lets draft_prose()
                # see that example as the unclosed one it is and recover
                # the draft prose trapped inside it. But the same shape
                # arises when the trailing fence is a properly closed
                # example's genuine close and the example is simply the
                # draft's last element (#410): dropping it there strips
                # the closed example's exemption and link-checks its
                # contents. The shapes are indistinguishable from the
                # block alone, so drop the fence only when doing so
                # surfaces a draft-defining element the kept prose
                # lacks entirely: the workflow:issue marker itself
                # (#401, #411), or a Concept Document pointer when the
                # kept prose carries no pointer at all (#414 — a draft
                # whose marker sits before an unclosed example traps
                # its only pointer inside the example the same way).
                # The pointer signal must stay that narrow: a properly
                # closed appendix example (#410) that itself depicts a
                # draft carries its own distinct pointer line, and the
                # broader #414 set-difference signal fired on exactly
                # that, stripping the closed example's exemption
                # (#422). So a draft already carrying a pointer in its
                # kept prose never drops the fence on the pointer
                # signal, and its appendix example stays exempt.
                # Trapped prose that is neither element, and a second
                # pointer trapped in a draft already carrying one
                # before the example, remain indistinguishable from a
                # closed example's contents and stay exempt (#414,
                # #422).
                lines = block.splitlines()
                found = (
                    re.match(r"^ {0,3}(`{3,}|~{3,})\s*(.*?)\s*$", lines[-1])
                    if lines
                    else None
                )
                if found and not found[2]:
                    dropped = "\n".join(lines[:-1])
                    kept = draft_prose(block)
                    recovered = draft_prose(dropped)
                    if (
                        "workflow:issue" not in kept and "workflow:issue" in recovered
                    ) or (not concept_pointers(kept) and concept_pointers(recovered)):
                        block = dropped
            # Nested fenced blocks inside a draft are embedded code
            # examples, not draft prose: strip them whole (as
            # unfenced() does at document level) so their contents are
            # not link-checked either (#393); draft_prose() recovers
            # the parts after an unclosed example instead (#401).
            clean = re.sub(r"<!--.*?-->", "", draft_prose(block), flags=re.S)
            yield re.sub(r"(`+)(?:(?!\1).)+?\1", "", clean)


def link_targets(text):
    targets = []
    for match in re.finditer(
        r'!?\[[^\]]*\]\((?:<([^>]+)>|([^\s)]+))(?:\s+["\'][^"\']*["\'])?\)', text
    ):
        targets.append(match[1] or match[2])
    targets += re.findall(r"^ {0,3}\[[^\]]+\]:\s*<?([^\s>]+)>?", text, re.M)
    return targets


def links(path, root):
    raw = path.read_text()
    targets = link_targets(prose(raw))
    for draft in issue_drafts(raw):
        targets += link_targets(draft)
    for href in targets:
        check_link(path, root, href)


def check_link(path, root, href):
    href = unquote(href)
    parsed = urlsplit(href)
    if parsed.scheme or parsed.netloc:
        return
    if "{{" in href and "}}" in href:
        return
    base = root if parsed.path.startswith("/") else path.parent
    target = (
        (base / parsed.path.lstrip("/")).resolve() if parsed.path else path.resolve()
    )
    if not target.is_relative_to(root):
        fail(f"{path}: link escapes repository: {href}")
    if not target.exists():
        fail(f"{path}: missing link target: {href}")
    if parsed.fragment and target.is_file():
        if re.fullmatch(r"L\d+(?:-L\d+)?", parsed.fragment):
            values = [int(x) for x in re.findall(r"\d+", parsed.fragment)]
            if min(values) < 1 or max(values) > len(target.read_text().splitlines()):
                fail(f"{path}: invalid line anchor: {href}")
        elif target.suffix == ".md":
            headings, counts = set(), {}
            for heading in re.findall(
                r"^ {0,3}#{1,6}\s+(.+?)\s*#*$", prose(target.read_text()), re.M
            ):
                base = anchor(heading)
                n = counts.get(base, 0)
                headings.add(base + (f"-{n}" if n else ""))
                counts[base] = n + 1
            if (
                parsed.fragment not in headings
                and f'id="{parsed.fragment}"' not in target.read_text()
            ):
                fail(f"{path}: missing heading anchor: {href}")


def read_state(package):
    text = (package / "main.md").read_text()
    matches = STATE.findall(unfenced(text))
    if len(matches) != 1:
        fail(f"{package}: expected exactly one workflow state block")
    state = json.loads(matches[0])
    if state.get("version") != 1:
        fail("Unsupported workflow state version")
    return state


def frontier(state):
    graph = dependencies(state)
    finished = {c["id"] for c in state["concepts"] if c["status"] in COMPLETE}
    busy = {
        c["id"]
        for c in state["concepts"]
        if c["status"] in ACTIVE or c["status"] in COMPLETE
    }
    blocked = {
        node: [p for p in prereqs if p not in finished]
        for node, prereqs in graph.items()
    }
    return {
        "ready": sorted(
            c["id"]
            for c in state["concepts"]
            if c["id"] not in busy and not blocked[c["id"]]
        ),
        "blocked": {
            key: value
            for key, value in blocked.items()
            if value and key not in finished
        },
        "constraints": [
            edge for edge in state["dependencies"] if edge["kind"] != "requires"
        ],
        "dimensions": {
            c["id"]: {
                k: c.get(k, "unspecified")
                for k in ("priority", "criticality", "horizon")
            }
            for c in state["concepts"]
        },
    }


def check_package(package, root, require_resolved=False):
    if not package.resolve().is_relative_to(root):
        fail("Package escapes repository")
    state = read_state(package)
    if (
        state.get("id") != package.name
        or not isinstance(state.get("events"), list)
        or not state["events"]
    ):
        fail(
            "Package identity must match its directory and include stage event history"
        )
    if not all(
        isinstance(state.get(key), str) and state[key].strip()
        for key in ("stage", "status", "created_at")
    ):
        fail("Package requires nonempty stage, status and created_at")
    transcript = contained(package, state["transcript"]["path"])
    raw = transcript.read_bytes()
    if hashlib.sha256(raw).hexdigest() != state["transcript"]["sha256"]:
        fail(f"{package}: transcript hash mismatch")
    lines = raw.decode("utf-8").splitlines(keepends=True)
    items = state["concepts"]
    ids = [c["id"] for c in items]
    paths = [c["path"] for c in items]
    if not items or len(set(ids)) != len(ids) or len(set(paths)) != len(paths):
        fail("Missing concepts or duplicate concept IDs/paths")
    if any(not re.fullmatch(r"C\d{3,}(?:\.\d+)*", x) for x in ids):
        fail("Invalid concept ID")
    coverage(items, lines)
    dependencies(state)
    for item in items:
        document = contained(package, item["path"])
        if document.suffix != ".md" or (len(items) == 1 and item["path"] != "main.md"):
            fail("Concepts must own Markdown documents; single concept owns main.md")
        if not document.is_file():
            fail(f"Missing concept document: {document}")
        text = document.read_bytes().decode("utf-8")
        relative = (
            "../transcript.txt"
            if item["path"].startswith("concepts/")
            else "transcript.txt"
        )
        for pair in item["source_lines"]:
            if excerpt(item["id"], pair, lines, relative) not in text:
                fail(f"{document}: exact source excerpt missing or changed: {pair}")
        if item["status"] in COMPLETE and re.search(
            r"Not yet explored\.|Pending: research|\bTODO\b|\bTBD\b", prose(text)
        ):
            fail(f"{document}: completed concept still contains placeholders")
    tasks = state.get("tasks", [])
    task_ids = [t["id"] for t in tasks]
    if len(task_ids) != len(set(task_ids)):
        fail("Duplicate task IDs")
    owners = []
    for task in tasks:
        if task.get("status") not in ACTIVE:
            continue
        if not task.get("owner") or not task.get("paths"):
            fail("Active task requires owner and owned paths")
        for path in task["paths"]:
            owned = contained(root, path)
            for prior, owner in owners:
                if task["owner"] != owner and (
                    owned == prior
                    or owned.is_relative_to(prior)
                    or prior.is_relative_to(owned)
                ):
                    fail(
                        f"Active task ownership overlap: {path}; serialize these tasks"
                    )
            owners.append((owned, task["owner"]))
    resolutions = {}
    for item in items:
        for block in RESOLUTION.findall(
            unfenced(contained(package, item["path"]).read_text())
        ):
            entry = json.loads(block)
            if entry.get("note") in resolutions:
                fail("Duplicate note disposition")
            resolutions[entry.get("note")] = (item["id"], entry)
    recorded = set()
    for read in DISPOSITION_READERS.values():
        for key in read(package, root):
            if key in resolutions or key in recorded:
                fail(f"Duplicate note disposition: {key}")
            recorded.add(key)
    unresolved = []
    note_paths = sorted((package / "notes").glob("*.md"))
    if (set(resolutions) | set(recorded)) - {
        p.relative_to(package).as_posix() for p in note_paths
    }:
        fail("Note disposition refers to a missing note")
    for path in note_paths:
        content = path.read_bytes().decode("utf-8")
        found = NOTE.search(content)
        if not found:
            fail(f"{path}: expected one immutable note marker")
        note = json.loads(found[1])
        body = content[found.end() :]
        if not body.startswith("\n\n") or hashlib.sha256(
            body[2:].encode()
        ).hexdigest() != note.get("body_sha256"):
            fail(f"{path}: immutable note body hash mismatch")
        if (
            note.get("from") not in ids
            or note.get("to") not in ids
            or note["from"] == note["to"]
        ):
            fail(f"{path}: invalid note participants")
        key = path.relative_to(package).as_posix()
        if key in recorded:
            continue
        if key not in resolutions:
            unresolved.append(key)
        else:
            receiver, entry = resolutions[key]
            if (
                receiver != note["to"]
                or entry.get("disposition")
                not in {
                    "incorporated",
                    "acknowledged",
                    "declined",
                    "deferred",
                    "superseded",
                }
                or not entry.get("target")
                or not entry.get("reason")
            ):
                fail(f"{path}: invalid receiver disposition/target/reason")
            if entry.get("disposition") == "deferred" and "Revisit when:" not in (
                entry.get("reason") or ""
            ):
                fail(
                    f"{path}: deferred disposition requires a "
                    "'Revisit when: <event>' trigger in its reason"
                )
            target = contained(package, urlsplit(entry["target"]).path)
            if not target.exists():
                fail(f"{path}: resolution target missing")
            check_link(package / "main.md", root, entry["target"])
    if require_resolved and unresolved:
        fail("Unresolved notes: " + ", ".join(unresolved))
    for path in package.rglob("*.md"):
        links(path, root)
    if len(items) > 1:
        for required in ("design.md", "changes.md"):
            if not (package / required).is_file():
                fail(f"Missing package file: {required}")
    if state.get("status") in COMPLETE and any(
        c["status"] not in COMPLETE for c in items
    ):
        fail("Completed package has incomplete concepts")
    return {
        "package": str(package),
        "id": state["id"],
        "stage": state["stage"],
        "status": state["status"],
        "unresolved_notes": unresolved,
        **frontier(state),
    }


def check_profile(root, config):
    docs = contained(root, config["workflow_docs"])
    if not docs.exists():
        fail(f"Missing workflow documentation: {docs}")
    documents = list(docs.rglob("*.md")) if docs.is_dir() else [docs]
    documents += [
        contained(root, path)
        for path in (config["request_catalog"], config["plan_catalog"])
    ]
    for document in documents:
        if not document.is_file():
            fail(f"Missing configured document: {document}")
        if document.suffix == ".md":
            links(document, root)
    for path, budget in config.get("rulebook_budget", {}).items():
        document = contained(root, path)
        if len(document.read_bytes()) > budget:
            fail(f"{path}: exceeds rulebook byte budget {budget}")
    for name in config.get("lifecycle_skills", []):
        document = contained(root, f".agents/skills/{slug(name)}/SKILL.md")
        text = document.read_text()
        found = re.match(r"^---\n(.*?)\n---\n", text, re.S)
        if not found:
            fail(f"{document}: missing YAML frontmatter")
        fields = {}
        for line in found[1].splitlines():
            match = re.match(r"([a-z][a-z_-]*): (.+)$", line)
            if not match:
                fail(f"{document}: frontmatter must use simple key: value scalars")
            key, value = match.groups()
            quoted = value[0] in "\"'" and value[-1] == value[0]
            if not quoted and (
                ": " in value or " #" in value or value[0] in ">|[{&*!%@`"
            ):
                fail(f"{document}: quote ambiguous YAML scalar {key}")
            fields[key] = value.strip("\"'")
        if (
            fields.get("name") != name
            or not 1 <= len(fields.get("description", "")) <= 1024
        ):
            fail(f"{document}: invalid skill name or description")
        links(document, root)
    trigger_file = config.get(
        "triggers", (Path(config["workflow_docs"]) / "triggers.json").as_posix()
    )
    if contained(root, trigger_file).exists():
        triggers = json.loads(contained(root, trigger_file).read_text())
        if not isinstance(triggers, list):
            fail("Trigger fixtures must be an array")
        for entry in triggers:
            if not isinstance(entry.get("skill"), str):
                fail("Trigger fixture requires skill")
            for key in ("should_load", "should_not_load"):
                if (
                    not isinstance(entry.get(key), list)
                    or len(entry[key]) < 5
                    or not all(isinstance(x, str) and x.strip() for x in entry[key])
                ):
                    fail(
                        f"Trigger fixture {entry['skill']}: {key} needs at least five text cases"
                    )


def notes(args, package, root):
    check_package(package, root)
    state = read_state(package)
    by_id = {c["id"]: c for c in state["concepts"]}
    if (
        args.sender not in by_id
        or args.receiver not in by_id
        or args.sender == args.receiver
    ):
        fail("Notes require two distinct known concept IDs")
    body = Path(args.input).read_bytes().decode("utf-8")
    now = timestamp()
    relative = f"notes/{args.sender}__to__{args.receiver}__{uuid.uuid4().hex}.md"
    path = contained(package, relative)
    content = (
        f"# {args.sender} to {args.receiver}\n\n"
        + marker(
            "note",
            {
                "id": path.stem,
                "from": args.sender,
                "to": args.receiver,
                "at": now,
                "body_sha256": hashlib.sha256(body.encode()).hexdigest(),
            },
        )
        + "\n"
        + body
    )
    path.parent.mkdir(exist_ok=True)
    with path.open("x") as stream:
        stream.write(content)
    return {"note": relative}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("init", "check", "status", "ready", "note"):
        command = sub.add_parser(name)
        command.add_argument("--repo-root")
        if name != "init":
            command.add_argument("--package", required=name != "check")
        if name == "init":
            command.add_argument("--input", required=True)
            command.add_argument("--slug", required=True)
            choice = command.add_mutually_exclusive_group()
            choice.add_argument("--outline")
            choice.add_argument("--concept")
            command.add_argument("--timestamp")
            command.add_argument("--expect-sha256")
            command.add_argument("--allow-duplicate", action="store_true")
        elif name == "check":
            command.add_argument("--require-resolved", action="store_true")
            command.add_argument("--path", action="append", default=[])
        elif name == "note":
            command.add_argument("--from", dest="sender", required=True)
            command.add_argument("--to", dest="receiver", required=True)
            command.add_argument("--input", required=True)
    plugins = list(extensions())
    for plugin in plugins:
        plugin.register(sub)
    args = parser.parse_args(argv)
    try:
        root = Path(args.repo_root or ".").resolve()
        config = profile(root)
        package = (
            Path(args.package).resolve() if getattr(args, "package", None) else None
        )
        if package and not package.is_relative_to(
            contained(root, config["requests_root"])
        ):
            fail("Package must be inside configured requests_root")
        if args.command == "init":
            result = scaffold(args, root, config)
        elif hasattr(args, "handler"):
            result = args.handler(args, root, config, package)
        elif args.command == "note":
            result = notes(args, package, root)
        elif args.command in ("status", "ready"):
            result = check_package(package, root)
        else:
            check_profile(root, config)
            request_root = contained(root, config["requests_root"])
            name_re = re.compile(
                r"^\d{4}-\d\d-\d\dT\d\d-\d\d-\d\dZ_research_[a-z0-9]+(?:-[a-z0-9]+)*$"
            )
            packages = (
                [package]
                if package
                else sorted(
                    p
                    for p in request_root.iterdir()
                    if p.is_dir()
                    and (
                        name_re.fullmatch(p.name)
                        or (p / "main.md").is_file()
                        and STATE.search(unfenced((p / "main.md").read_text()))
                    )
                )
            )
            result = {
                "packages": [
                    check_package(p, root, args.require_resolved) for p in packages
                ]
            }
            for path in args.path:
                links(contained(root, path), root)
            for plugin in plugins:
                for checked in packages:
                    getattr(plugin, "check", lambda *_: None)(checked, root)
            result["ok"] = True
        print(json.dumps(result, ensure_ascii=False, indent=2))
        return 0
    except (
        ValueError,
        OSError,
        KeyError,
        TypeError,
        AttributeError,
        IndexError,
    ) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
