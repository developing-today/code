#!/usr/bin/env python3
"""Repo-universe inventory generator (#1650, piece 1 of #1511).

Builds the universe list from two sources:

1. GitHub enumeration of the developing-today org and the dezren39
   account (`gh api --paginate orgs/developing-today/repos` and
   `gh api --paginate users/dezren39/repos`), captured as JSON files.
2. The 2026-10-27 all-repos census (goal-workspace
   `hidden_files/git-cleanup-2026-10-27/all-repos-census.md`): every
   clone remote on laptop + sandbox, including the nested clones
   inside `/home/user/repo-archive`.

and writes the maintained artifact `doc/fleet/repo-universe.yaml`.

Classification (defaults; overridable per repo, see below):

- `owned`               owner is developing-today or dezren39 and the
                        repo is not a GitHub fork.
- `fork`                GitHub fork flag is set.
- `archive-nested`      census-only repo whose clone lives inside
                        `/repo-archive/` (a nested clone row).
- `upstream-dependency` any other census-only repo: not owned, named
                        by the census because we depend on or track it.

Watch (defaults): `owned` -> `issues`; everything else ->
`inventory-only`.

Overrides: the artifact's `overrides:` section is human-edited. The
generator reads it back from the existing artifact, applies each
entry's class/watch over the computed defaults, and re-emits it
verbatim in effect — hand edits there survive regeneration; the
`repos:` list itself is fully regenerated. Re-running the generator
on unchanged inputs produces a zero diff (idempotent).

Usage:
    generate.py --org-json org.json --user-json user.json \
        --census /path/to/all-repos-census.md \
        --census-label 'hidden_files/git-cleanup-2026-10-27/all-repos-census.md (repo-health goal workspace)' \
        --output doc/fleet/repo-universe.yaml

With `--live`, the enumeration JSON is fetched via `gh` instead of
read from files. Stdlib only.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

OWNED_OWNERS = {"developing-today", "dezren39"}
CLASSES = ("owned", "fork", "upstream-dependency", "archive-nested")
WATCHES = ("issues", "inventory-only")

URL_RE = re.compile(r"github\.com[:/]([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+?)(?:\.git)?(?:[\s|)'\"]|$)")


def decode_pages(text: str) -> list[dict]:
    """Decode one or more concatenated JSON documents.

    `gh api --paginate` emits one JSON array (or object) per page, so a
    captured enumeration file — like live --paginate output — is a
    stream of documents, not a single JSON value. Decode them all and
    flatten the entries.
    """
    entries: list[dict] = []
    decoder = json.JSONDecoder()
    idx = 0
    while idx < len(text):
        while idx < len(text) and text[idx].isspace():
            idx += 1
        if idx >= len(text):
            break
        doc, idx = decoder.raw_decode(text, idx)
        if isinstance(doc, list):
            entries.extend(doc)
        else:
            entries.append(doc)
    return entries


def load_enumeration(path: str | None) -> list[dict]:
    if path is None:
        return []
    return decode_pages(Path(path).read_text())


EXPECTED_USER_LOGIN = "dezren39"


def authenticated_login() -> str:
    out = subprocess.run(
        ["gh", "api", "user"],
        capture_output=True, text=True, check=True,
    ).stdout
    return json.loads(out)["login"]


def require_expected_account() -> None:
    """--live guard: user/repos?affiliation=owner enumerates the
    AUTHENTICATED account, so running as anyone but dezren39 would
    silently overwrite the artifact with the wrong universe. Fail
    before fetching or writing anything."""
    login = authenticated_login()
    if login != EXPECTED_USER_LOGIN:
        raise SystemExit(
            f"error: --live must run authenticated as {EXPECTED_USER_LOGIN} "
            f"(authenticated as {login!r}); refusing to regenerate the "
            "repo-universe artifact from the wrong account"
        )


def fetch_live(endpoint: str) -> list[dict]:
    out = subprocess.run(
        ["gh", "api", "--paginate", endpoint],
        capture_output=True, text=True, check=True,
    ).stdout
    # --paginate concatenates one JSON array per page.
    return decode_pages(out)


def parse_census(text: str) -> dict[str, dict]:
    """full_name -> {url, nested} from census table rows / URL mentions."""
    found: dict[str, dict] = {}
    for line in text.splitlines():
        m = URL_RE.search(line)
        if not m:
            continue
        owner, name = m.group(1), m.group(2)
        if owner in ("repos", "orgs", "users"):
            continue
        full = f"{owner}/{name}"
        # A table row's first cell is the clone path; a clone under
        # /repo-archive/ is a nested clone of the archive repo.
        nested = "/repo-archive/" in line.split("|")[1] if "|" in line else "/repo-archive/" in line
        entry = found.setdefault(full, {"url": f"https://github.com/{full}", "nested": False})
        entry["nested"] = entry["nested"] or nested
    return found


def classify(full_name: str, gh_repo: dict | None, census_entry: dict | None) -> str:
    if gh_repo is not None:
        if gh_repo.get("fork"):
            return "fork"
        return "owned"
    if census_entry and census_entry["nested"]:
        return "archive-nested"
    if full_name.split("/")[0] in OWNED_OWNERS:
        # Census-only but under an owned owner (e.g. a private repo an
        # enumeration endpoint cannot see): it is ours.
        return "owned"
    return "upstream-dependency"


def default_watch(cls: str) -> str:
    return "issues" if cls == "owned" else "inventory-only"


def parse_overrides(text: str) -> dict[str, dict]:
    """Read the overrides: section of an existing artifact (the exact
    shape this generator emits; hand edits keep that shape)."""
    overrides: dict[str, dict] = {}
    in_section = False
    current: str | None = None
    for line in text.splitlines():
        if line.startswith("overrides:"):
            in_section = True
            continue
        if in_section:
            if line and not line.startswith(" ") and not line.startswith("#"):
                break
            m = re.match(r"^  ([^\s#][^:]*):\s*$", line)
            if m:
                current = m.group(1).strip()
                overrides[current] = {}
                continue
            m = re.match(r"^    (class|watch):\s*(\S+)\s*$", line)
            if m and current is not None:
                overrides[current][m.group(1)] = m.group(2)
    return overrides


def build(org_repos: list[dict], user_repos: list[dict], census_text: str,
          census_label: str, overrides: dict[str, dict]) -> str:
    census = parse_census(census_text)
    gh: dict[str, dict] = {}
    for repo in org_repos + user_repos:
        gh[repo["full_name"]] = repo

    names = sorted(set(gh) | set(census), key=str.lower)
    lines: list[str] = []
    lines.append("# Repo universe inventory — maintained artifact (#1650, piece 1 of #1511).")
    lines.append("#")
    lines.append("# Generated by scripts/fleet/repo-universe/generate.py from:")
    lines.append("#   - GitHub enumeration: orgs/developing-today/repos + users/dezren39/repos")
    lines.append(f"#   - Census: {census_label}")
    lines.append(f"#   - Census sha256: {hashlib.sha256(census_text.encode()).hexdigest()}")
    lines.append("#")
    lines.append("# Hand edits: change class/watch for a repo ONLY in the overrides:")
    lines.append("# section below — the generator merges overrides over its computed")
    lines.append("# defaults and never overwrites them. The repos: list is fully")
    lines.append("# regenerated; re-running on unchanged inputs yields a zero diff.")
    lines.append("version: 1")
    lines.append("overrides:")
    if overrides:
        for name in sorted(overrides, key=str.lower):
            lines.append(f"  {name}:")
            for key in ("class", "watch"):
                if key in overrides[name]:
                    lines.append(f"    {key}: {overrides[name][key]}")
    else:
        lines.append("  {}")
    lines.append("repos:")
    for full in names:
        repo = gh.get(full)
        centry = census.get(full)
        cls = classify(full, repo, centry)
        ov = overrides.get(full, {})
        if "class" in ov and ov["class"] in CLASSES:
            cls = ov["class"]
        # The watch default follows the FINAL class (an override that
        # makes a repo owned also makes it watched), unless the
        # override pins watch itself.
        watch = default_watch(cls)
        if "watch" in ov and ov["watch"] in WATCHES:
            watch = ov["watch"]
        url = (repo or {}).get("html_url") or (centry or {}).get("url") or f"https://github.com/{full}"
        sources = []
        if repo is not None:
            sources.append("github")
        if centry is not None:
            sources.append("census")
        lines.append(f"  - name: {full}")
        lines.append(f"    url: {url}")
        lines.append(f"    class: {cls}")
        lines.append(f"    watch: {watch}")
        lines.append(f"    sources: [{', '.join(sources)}]")
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--org-json", help="captured `gh api orgs/developing-today/repos` JSON")
    ap.add_argument("--user-json", help="captured `gh api users/dezren39/repos` JSON")
    ap.add_argument("--live", action="store_true", help="fetch the enumeration via gh instead of files")
    ap.add_argument("--census", required=True, help="path to all-repos-census.md")
    ap.add_argument("--census-label", default="all-repos-census.md (2026-10-27)",
                    help="provenance string recorded in the artifact header")
    ap.add_argument("--output", required=True, help="artifact path to write")
    args = ap.parse_args(argv)

    if args.live:
        require_expected_account()
        org_repos = fetch_live("orgs/developing-today/repos?per_page=100&type=all")
        # user/repos?affiliation=owner, NOT users/dezren39/repos: the
        # latter returns public repos only and silently misses private
        # owned repos (dezren39/hatch-personal among them, found at
        # build time — the census row was the only sign).
        user_repos = fetch_live("user/repos?per_page=100&affiliation=owner")
    else:
        org_repos = load_enumeration(args.org_json)
        user_repos = load_enumeration(args.user_json)

    census_text = Path(args.census).read_text()
    output = Path(args.output)
    overrides: dict[str, dict] = {}
    if output.exists():
        overrides = parse_overrides(output.read_text())

    artifact = build(org_repos, user_repos, census_text, args.census_label, overrides)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(artifact)
    count = artifact.count("\n  - name: ")
    print(f"wrote {output}: {count} repos, {len(overrides)} overrides applied")
    return 0


if __name__ == "__main__":
    sys.exit(main())
