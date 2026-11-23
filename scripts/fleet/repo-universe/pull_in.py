#!/usr/bin/env python3
"""Repo-universe pull-in surface (#1652, piece 3 of #1511).

The write side of the repo-universe tracker. Piece 2's checker
(`check_issues.py`) is read-only: it diffs watched repos against
persisted state and emits delta reports. This module runs the same
cycle semantics (same inventory, same persisted state file, same
diff) and ACTS on the delta, per the surface Drew confirmed on
2026-11-14 (mirrored tracking issues in developing-today/code):

- Each `new` delta item gets a mirror issue in the target repo,
  titled `[<owner>/<repo>#<n>] <source title>`, whose body links the
  source issue and carries the dedupe marker
  `<!-- repo-universe:source=<owner>/<repo>#<n> -->`. The marker is
  searched before any create, so re-runs create zero duplicates.
  Mirror labels: `source/universe` plus the source repo's class
  label `universe/<class>` (class from the inventory: owned, fork,
  upstream-dependency, archive-nested).
- A `changed` delta whose title moved retitles the open mirror.
- A `closed` delta resolves the mirror: a comment linking the
  source, then close. A mirror CONVERTED to locally-owned work —
  its marker removed, the conversion noted in a comment — is never
  found by the marker search and is therefore never resolved; that
  absence IS the conversion mechanism, by design.
- A `reopened` delta reopens a resolved mirror (with a comment).
- A first run for a repo BASELINES it (the checker's rule): no
  mirrors are created for pre-existing history.
- The target repo itself is never mirrored. Its issues already live
  there, and mirroring it would mirror every mirror created in it,
  one level deeper each cycle. Its state is still recorded.
- A `new` issue that is already closed gets no mirror (an open
  mirror found for it is resolved), so a mirror never opens for
  finished work.
- A failed fetch is an explicit ERROR line and a non-zero exit; the
  repo's persisted state is left untouched and no mirror action is
  taken for it, so a failure never masquerades as an empty repo or
  as a mass closure.

Upstream-dependency mirrors carry a disposition label set at triage
— exactly one of `universe/waiting-on-upstream`,
`universe/contribute-fix`, `universe/local-workaround`. This module
coins all of the labels it depends on (see LABEL_DEFINITIONS) but
never sets a disposition itself: disposition is a triage judgment,
not a sync action. Owned-repo mirrors route into triage like any
fleet issue (the fleet monitor's normal queue/queued path).

State is SHARED with the checker: the same state.json the checker
maintains, so a delta is acted on exactly once no matter which of
the two runs first. Runs are serialised with an flock on
`<state-dir>/pull-in.lock`; the systemd units in `systemd/` also
order the pull-in service after the checker service. Fixture mode
(`--fixture`, same shape as the checker's) replaces the GitHub
fetch, and `--fixture-mirrors` replaces the mirror store with a
JSON file, so the full cycle is testable offline with no GitHub
writes. `--dry-run` plans actions against the real store without
mutating it. Stdlib only; GitHub writes go through `gh`.
"""

from __future__ import annotations

import argparse
import fcntl
import json
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_issues  # noqa: E402  (sibling module: diff/state semantics)

TARGET_REPO_DEFAULT = "developing-today/code"
SOURCE_LABEL = "source/universe"
CLASSES = ("owned", "fork", "upstream-dependency", "archive-nested")
DISPOSITION_LABELS = (
    "universe/waiting-on-upstream",
    "universe/contribute-fix",
    "universe/local-workaround",
)
MARKER_RE = re.compile(r"<!-- repo-universe:source=([\w.-]+/[\w.-]+#\d+) -->")

LABEL_DEFINITIONS = {
    SOURCE_LABEL: (
        "2da44e",
        "Mirror of an issue tracked by the repo-universe tracker (#1511)",
    ),
    "universe/owned": ("0e8a16", "Repo-universe class: owned repo"),
    "universe/fork": ("fbca04", "Repo-universe class: fork"),
    "universe/upstream-dependency": (
        "5319e7",
        "Repo-universe class: upstream dependency",
    ),
    "universe/archive-nested": (
        "6e7781",
        "Repo-universe class: archive-nested (inventory only)",
    ),
    "universe/waiting-on-upstream": (
        "cfd3d7",
        "Universe mirror disposition: waiting on upstream",
    ),
    "universe/contribute-fix": (
        "1d76db",
        "Universe mirror disposition: contribute a fix upstream",
    ),
    "universe/local-workaround": (
        "d93f0b",
        "Universe mirror disposition: local workaround",
    ),
}


def class_label(repo_class: str) -> str:
    return f"universe/{repo_class}"


def source_key(repo: str, number: int) -> str:
    return f"{repo}#{number}"


def marker_for(repo: str, number: int) -> str:
    return f"<!-- repo-universe:source={source_key(repo, number)} -->"


def mirror_title(repo: str, issue: dict) -> str:
    return f"[{repo}#{issue['number']}] {issue['title']}"


def mirror_body(repo: str, repo_class: str, issue: dict) -> str:
    lines = [
        f"Mirror of {issue['html_url']} — tracked by the repo-universe tracker (#1511; pull-in piece 3, #1652).",
        "",
        f"Source: {source_key(repo, issue['number'])} — {issue['html_url']}",
        f"Source repo class: {repo_class}",
        "",
        "This is a mirrored tracking issue: it exists so the fleet workflow "
        "(labels, triage, the repo-health queue) can work the source issue "
        "from developing-today/code. When the source issue closes, the next "
        "pull-in cycle comments here with the source link and closes this "
        "mirror — unless this mirror has been converted into a locally-owned "
        "work issue (remove the marker below and note the conversion in a "
        "comment; converted mirrors are left alone).",
        "",
        "Upstream-dependency mirrors: at triage, apply exactly one disposition "
        "label — `universe/waiting-on-upstream`, `universe/contribute-fix`, "
        "or `universe/local-workaround`.",
        "",
        marker_for(repo, issue["number"]),
        "",
    ]
    return "\n".join(lines)


def parse_inventory_classes(text: str) -> list[dict]:
    """Watched repos with their effective class.

    Same fixed-shape inventory and override rule as the checker's
    parse_inventory (overrides win), extended to carry each repo's
    effective `class` alongside its name: the class decides the
    mirror's class label. Returns [{"name": ..., "class": ...}]
    for repos whose effective watch is `issues`, in file order.
    """
    overrides: dict[str, dict] = {}
    repos: list[dict] = []
    section = None
    current: dict | None = None
    override_name: str | None = None
    for raw in text.splitlines():
        if raw.startswith("overrides:"):
            section = "overrides"
            continue
        if raw.startswith("repos:"):
            section = "repos"
            continue
        if section == "overrides":
            m = re.match(r"^  ([\w.-]+/[\w.-]+):\s*$", raw)
            if m:
                override_name = m.group(1)
                overrides.setdefault(override_name, {})
                continue
            m = re.match(r"^    (class|watch):\s*(\S+)\s*$", raw)
            if m and override_name:
                overrides[override_name][m.group(1)] = m.group(2)
            continue
        if section == "repos":
            m = re.match(r"^  - name:\s*(\S+)\s*$", raw)
            if m:
                current = {"name": m.group(1)}
                repos.append(current)
                continue
            m = re.match(r"^    (class|watch):\s*(\S+)\s*$", raw)
            if m and current is not None:
                current[m.group(1)] = m.group(2)
    watched = []
    for repo in repos:
        over = overrides.get(repo["name"], {})
        if over.get("watch", repo.get("watch")) == "issues":
            watched.append(
                {
                    "name": repo["name"],
                    "class": over.get("class", repo.get("class", "owned")),
                }
            )
    return watched


class MirrorError(Exception):
    """A mirror-store operation failed; carries a human-readable reason."""


class GhMirrorStore:
    """Live mirror store: mirror issues in the target repo, via `gh`.

    Mirrors are found by searching issue bodies for the dedupe
    marker token — the search runs over open AND closed issues, so
    a resolved mirror is never recreated.
    """

    def __init__(self, target_repo: str):
        self.target_repo = target_repo

    def _gh(self, args: list[str], ok_codes=(0,)) -> subprocess.CompletedProcess:
        try:
            out = subprocess.run(
                ["gh", *args], capture_output=True, text=True, timeout=300
            )
        except (OSError, subprocess.SubprocessError) as exc:
            raise MirrorError(f"gh invocation failed: {exc}") from exc
        if out.returncode not in ok_codes:
            detail = (out.stderr or out.stdout).strip().splitlines()
            raise MirrorError(
                f"gh {' '.join(args[:3])} failed: {detail[0] if detail else 'unknown error'}"
            )
        return out

    def find_mirror(self, key: str) -> dict | None:
        out = self._gh(
            [
                "issue",
                "list",
                "--repo",
                self.target_repo,
                "--state",
                "all",
                "--search",
                f'"repo-universe:source={key}" in:body',
                "--json",
                "number,title,body,state,labels",
                "--limit",
                "50",
            ]
        )
        try:
            candidates = json.loads(out.stdout)
        except ValueError as exc:
            raise MirrorError("unparseable gh issue list output") from exc
        for cand in candidates:
            m = MARKER_RE.search(cand.get("body") or "")
            if m and m.group(1) == key:
                return {
                    "number": cand["number"],
                    "title": cand["title"],
                    "body": cand.get("body") or "",
                    "state": (cand.get("state") or "").lower(),
                    "labels": [label["name"] for label in cand.get("labels", [])],
                }
        return None

    def create_mirror(self, title: str, body: str, labels: list[str]) -> dict:
        args = [
            "issue",
            "create",
            "--repo",
            self.target_repo,
            "--title",
            title,
            "--body",
            body,
        ]
        for label in labels:
            args += ["--label", label]
        out = self._gh(args)
        url = out.stdout.strip().splitlines()[-1] if out.stdout.strip() else ""
        m = re.search(r"/issues/(\d+)$", url)
        if not m:
            raise MirrorError(f"gh issue create returned no issue URL: {url!r}")
        return {
            "number": int(m.group(1)),
            "title": title,
            "body": body,
            "state": "open",
            "labels": list(labels),
        }

    def update_title(self, number: int, title: str) -> None:
        self._gh(
            ["issue", "edit", str(number), "--repo", self.target_repo, "--title", title]
        )

    def comment(self, number: int, body: str) -> None:
        self._gh(
            [
                "issue",
                "comment",
                str(number),
                "--repo",
                self.target_repo,
                "--body",
                body,
            ]
        )

    def close(self, number: int) -> None:
        self._gh(["issue", "close", str(number), "--repo", self.target_repo])

    def reopen(self, number: int) -> None:
        self._gh(["issue", "reopen", str(number), "--repo", self.target_repo])

    def ensure_label(self, name: str, color: str, description: str) -> None:
        self._gh(
            [
                "label",
                "create",
                name,
                "--repo",
                self.target_repo,
                "--color",
                color,
                "--description",
                description,
                "--force",
            ]
        )


class FixtureMirrorStore:
    """In-memory / JSON-file mirror store for tests and dry cycles.

    Mirrors are dicts {number, title, body, state, labels, comments};
    find parses the dedupe marker out of bodies exactly like the
    live store's search, so a mirror whose marker was removed (the
    conversion mechanism) is invisible here too.
    """

    def __init__(self, mirrors: list[dict] | None = None):
        self.mirrors = [dict(m) for m in (mirrors or [])]
        self._next = max((m["number"] for m in self.mirrors), default=1000) + 1

    def find_mirror(self, key: str) -> dict | None:
        for mirror in self.mirrors:
            m = MARKER_RE.search(mirror.get("body") or "")
            if m and m.group(1) == key:
                return mirror
        return None

    def _get(self, number: int) -> dict:
        for mirror in self.mirrors:
            if mirror["number"] == number:
                return mirror
        raise MirrorError(f"fixture mirror {number} not found")

    def create_mirror(self, title: str, body: str, labels: list[str]) -> dict:
        mirror = {
            "number": self._next,
            "title": title,
            "body": body,
            "state": "open",
            "labels": list(labels),
            "comments": [],
        }
        self._next += 1
        self.mirrors.append(mirror)
        return mirror

    def update_title(self, number: int, title: str) -> None:
        self._get(number)["title"] = title

    def comment(self, number: int, body: str) -> None:
        self._get(number).setdefault("comments", []).append(body)

    def close(self, number: int) -> None:
        self._get(number)["state"] = "closed"

    def reopen(self, number: int) -> None:
        self._get(number)["state"] = "open"

    def ensure_label(self, name: str, color: str, description: str) -> None:
        pass  # labels are plain strings in fixture mode

    @classmethod
    def load(cls, path: Path) -> "FixtureMirrorStore":
        return cls(json.loads(path.read_text()))

    def save(self, path: Path) -> None:
        path.write_text(json.dumps(self.mirrors, indent=1) + "\n")


class DryRunStore:
    """Read-through wrapper: finds hit the real store, writes are
    only recorded in `planned` and never mutate anything."""

    def __init__(self, inner):
        self.inner = inner
        self.planned: list[str] = []

    def find_mirror(self, key: str):
        return self.inner.find_mirror(key)

    def create_mirror(self, title: str, body: str, labels: list[str]) -> dict:
        self.planned.append(f"create: {title} labels={labels}")
        m = MARKER_RE.search(body)
        return {
            "number": -1,
            "title": title,
            "body": body,
            "state": "open",
            "labels": list(labels),
            "_key": m.group(1) if m else None,
        }

    def update_title(self, number: int, title: str) -> None:
        self.planned.append(f"retitle #{number}: {title}")

    def comment(self, number: int, body: str) -> None:
        self.planned.append(f"comment #{number}")

    def close(self, number: int) -> None:
        self.planned.append(f"close #{number}")

    def reopen(self, number: int) -> None:
        self.planned.append(f"reopen #{number}")

    def ensure_label(self, name: str, color: str, description: str) -> None:
        self.planned.append(f"ensure-label {name}")


def run_pull_in(
    inventory_text: str,
    state: dict,
    fetch,
    store,
    now: str,
    target_repo: str = TARGET_REPO_DEFAULT,
):
    """Run one pull-in cycle over all watched repos.

    Returns (new_state, report_lines, had_error, actions). Pure
    apart from the injected `fetch` callable and the mirror store,
    so tests drive it with fixtures. `actions` is a list of
    human-readable action strings, in the order taken.
    """
    repos = parse_inventory_classes(inventory_text)
    if not isinstance(state, dict):
        raise check_issues.FetchError("persisted state is not a JSON object")
    prior_repos = state.get("repos", {})
    if not isinstance(prior_repos, dict):
        raise check_issues.FetchError("persisted state 'repos' is not a JSON object")
    new_repos = dict(prior_repos)
    lines = [f"# Repo-universe pull-in — {now}", ""]
    had_error = False
    actions: list[str] = []
    for repo_info in repos:
        repo, repo_class = repo_info["name"], repo_info["class"]
        prior_entry = prior_repos.get(repo)
        if prior_entry is not None and not check_issues._valid_prior_entry(prior_entry):
            had_error = True
            lines.append(f"## {repo}")
            lines.append(
                "- ERROR: persisted state entry is malformed; repo skipped, state left untouched"
            )
            lines.append("")
            continue
        try:
            issues = fetch(repo)
        except check_issues.FetchError as exc:
            had_error = True
            lines.append(f"## {repo}")
            lines.append(f"- ERROR: fetch failed: {exc}")
            lines.append("")
            continue
        result = check_issues.diff_repo(
            prior_entry["issues"] if prior_entry else None, issues
        )
        entry = {
            "watermark": max((i["updated_at"] or "" for i in issues), default=None),
            "issues": result["state"],
        }
        if prior_entry and prior_entry.get("issues") == result["state"]:
            entry["watermark"] = prior_entry.get("watermark")
        new_repos[repo] = entry
        lines.append(f"## {repo}")
        if result["baseline"]:
            c = result["counts"]
            lines.append(
                f"- baseline: {c['total']} issues recorded ({c['open']} open, "
                f"{c['closed']} closed); no mirrors created for history"
            )
            lines.append("")
            continue
        if repo == target_repo:
            lines.append("- target repo: its issues live here already; never mirrored")
            lines.append("")
            continue
        repo_actions = _act_on_delta(repo, repo_class, result, store)
        actions.extend(repo_actions)
        if repo_actions:
            lines.extend(f"- {a}" for a in repo_actions)
        else:
            lines.append("- no changes")
        lines.append("")
    lines.insert(
        2,
        f"Repos checked: {len(repos)}; mirror actions: {len(actions)}; errors: {'yes' if had_error else 'no'}",
    )
    new_state = {"version": 1, "last_run": now, "repos": new_repos}
    return new_state, lines, had_error, actions


def _act_on_delta(repo: str, repo_class: str, result: dict, store) -> list[str]:
    actions: list[str] = []
    labels = [SOURCE_LABEL, class_label(repo_class)]
    for issue in sorted(result["new"], key=lambda i: i["number"]):
        key = source_key(repo, issue["number"])
        existing = store.find_mirror(key)
        if issue["state"] == "closed":
            if existing is not None and existing["state"] == "open":
                actions.append(_resolve(store, existing, key, issue))
            continue
        if existing is not None:
            if existing["state"] == "closed":
                store.comment(
                    existing["number"],
                    f"Source {key} is open again ({issue['html_url']}); reopening this mirror.",
                )
                store.reopen(existing["number"])
                actions.append(
                    f"reopened mirror #{existing['number']} for {key} (was resolved, source open)"
                )
            elif existing["title"] != mirror_title(repo, issue):
                store.update_title(existing["number"], mirror_title(repo, issue))
                actions.append(f"retitled mirror #{existing['number']} for {key}")
            else:
                actions.append(
                    f"mirror exists for {key}: #{existing['number']} (no duplicate created)"
                )
            continue
        created = store.create_mirror(
            mirror_title(repo, issue), mirror_body(repo, repo_class, issue), labels
        )
        actions.append(f"created mirror #{created['number']} for {key}")
    for issue in sorted(result["changed"], key=lambda i: i["number"]):
        key = source_key(repo, issue["number"])
        existing = store.find_mirror(key)
        if (
            existing is not None
            and existing["state"] == "open"
            and existing["title"] != mirror_title(repo, issue)
        ):
            store.update_title(existing["number"], mirror_title(repo, issue))
            actions.append(f"retitled mirror #{existing['number']} for {key}")
    for issue in sorted(result["reopened"], key=lambda i: i["number"]):
        key = source_key(repo, issue["number"])
        existing = store.find_mirror(key)
        if existing is not None and existing["state"] == "closed":
            store.comment(
                existing["number"],
                f"Source {key} reopened ({issue['html_url']}); reopening this mirror.",
            )
            store.reopen(existing["number"])
            actions.append(f"reopened mirror #{existing['number']} for {key}")
    for issue in sorted(result["closed"], key=lambda i: i["number"]):
        key = source_key(repo, issue["number"])
        existing = store.find_mirror(key)
        if existing is None:
            # No mirror, or the mirror was converted to locally-owned
            # work (marker removed): either way, nothing to resolve.
            continue
        if existing["state"] == "closed":
            continue
        actions.append(_resolve(store, existing, key, issue))
    return actions


def _resolve(store, existing: dict, key: str, issue: dict) -> str:
    store.comment(
        existing["number"],
        f"Source {key} closed ({issue['html_url']}); resolving this mirror.",
    )
    store.close(existing["number"])
    return f"resolved mirror #{existing['number']} for {key} (source closed)"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--inventory", default="doc/fleet/repo-universe.yaml")
    parser.add_argument(
        "--state-dir", default="~/.local/state/repo-universe-issue-checker"
    )
    parser.add_argument("--target-repo", default=TARGET_REPO_DEFAULT)
    parser.add_argument(
        "--fixture", help="JSON fixture replacing GitHub fetches (checker shape)"
    )
    parser.add_argument(
        "--fixture-mirrors",
        help="JSON file used as the mirror store instead of live GitHub",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="plan actions against the store without mutating it",
    )
    parser.add_argument("--now", help="ISO-8601 run timestamp (default: now, UTC)")
    args = parser.parse_args(argv)

    now = args.now or datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    state_dir = Path(args.state_dir).expanduser()
    state_dir.mkdir(parents=True, exist_ok=True)
    state_path = state_dir / "state.json"
    state = (
        json.loads(state_path.read_text())
        if state_path.exists()
        else {"version": 1, "repos": {}}
    )
    inventory_text = Path(args.inventory).read_text()

    if args.fixture:
        fixture = check_issues.load_fixture(Path(args.fixture))
        fetch = lambda repo: check_issues.fetch_issues_fixture(repo, fixture)  # noqa: E731
    else:
        fetch = check_issues.fetch_issues_gh

    if args.fixture_mirrors:
        store = FixtureMirrorStore.load(Path(args.fixture_mirrors))
    else:
        store = GhMirrorStore(args.target_repo)
    if args.dry_run:
        store = DryRunStore(store)

    lock_path = state_dir / "pull-in.lock"
    with open(lock_path, "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        for name, (color, description) in LABEL_DEFINITIONS.items():
            store.ensure_label(name, color, description)
        try:
            new_state, lines, had_error, actions = run_pull_in(
                inventory_text, state, fetch, store, now, args.target_repo
            )
        except check_issues.FetchError as exc:
            print(f"error: {exc}; no report or state written", file=sys.stderr)
            return 2
        date = now[:10]
        report_path = state_dir / f"pull-in-{date}.md"
        suffix = 1
        while report_path.exists():
            suffix += 1
            report_path = state_dir / f"pull-in-{date}-{suffix}.md"
        report_path.write_text("\n".join(lines) + "\n")
        state_path.write_text(json.dumps(new_state, indent=1, sort_keys=True) + "\n")
        if args.fixture_mirrors and not args.dry_run:
            store.save(Path(args.fixture_mirrors))
    for action in actions:
        print(f"action: {action}")
    if isinstance(store, DryRunStore):
        for planned in store.planned:
            print(f"planned: {planned}")
    print(f"report: {report_path}")
    print(f"state: {state_path}")
    return 1 if had_error else 0


if __name__ == "__main__":
    sys.exit(main())
