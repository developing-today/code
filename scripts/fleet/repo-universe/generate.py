#!/usr/bin/env python3
"""Repo-universe inventory generator (#1650, piece 1 of #1511).

Builds the universe list from two sources:

1. GitHub enumeration of the developing-today and
   developing-today-forks orgs and the dezren39 account
   (`gh api --paginate orgs/developing-today/repos`,
   `gh api --paginate orgs/developing-today-forks/repos` and
   `gh api --paginate 'user/repos?affiliation=owner'`, authenticated
   as dezren39), captured as JSON files. The two org captures are
   merged before classification: developing-today/opencode and
   developing-today/openchamber moved to the developing-today-forks
   org on 2026-11-04 (#5615), so enumerating only developing-today
   would lose them on a full regeneration. (NOT
   `users/dezren39/repos`: that endpoint returns public repos only
   and silently misses private owned repos.)
2. The 2026-10-27 all-repos census (goal-workspace
   `hidden_files/git-cleanup-2026-10-27/all-repos-census.md`): every
   clone remote on laptop + sandbox, including the nested clones
   inside `/home/user/repo-archive`.

and writes the maintained artifact `doc/fleet/repo-universe.yaml`.

Classification (defaults; overridable per repo, see below):

- `owned`               owner is developing-today, developing-today-forks or dezren39 and the
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
    generate.py --org-json org.json --forks-org-json forks-org.json \
        --user-json user.json \
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

OWNED_OWNERS = {"developing-today", "developing-today-forks", "dezren39"}
# The two orgs enumerated for the universe (#5615): opencode and
# openchamber moved from developing-today to developing-today-forks
# on 2026-11-04, so both orgs are captured and merged.
ORGS = ("developing-today", "developing-today-forks")
CLASSES = ("owned", "fork", "upstream-dependency", "archive-nested")
WATCHES = ("issues", "inventory-only")

URL_RE = re.compile(
    r"github\.com[:/]([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+?)(?:\.git)?(?:[\s|)'\"]|$)"
)

# The GitHub shape of a full_name: an owner (alphanumerics and single
# hyphens, never leading/trailing) and a name (alphanumerics, '.', '_',
# '-') joined by exactly one slash (#6814).
FULL_NAME_RE = re.compile(r"[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?/[A-Za-z0-9._-]+")


def validate_entry(entry: object, source: str, index: int) -> dict:
    """One enumeration entry must be a repo object with a string
    full_name. Valid JSON with the wrong shape — a string/null entry
    in a page array, an API error object saved into a capture, a repo
    object missing full_name — must surface as a named error
    identifying the capture source and the offending entry, never as
    a bare TypeError/KeyError that aborts the run with no indication
    of which capture or entry is at fault (#6716; the contract #6652
    established on the checker side). The fields inside an object
    entry are validated too (#6814): full_name must be a well-formed
    owner/name, html_url (when present) a string without whitespace
    or control characters, and fork (when present) a JSON boolean.
    """
    if not isinstance(entry, dict):
        raise ValueError(
            f"{source}: enumeration entry {index} is not a JSON object: {entry!r:.80}"
        )
    if not isinstance(entry.get("full_name"), str):
        raise ValueError(
            f"{source}: enumeration entry {index} has no string "
            f"full_name: {entry!r:.80}"
        )
    # Field-level shape (#6814, the residual of #6716 one level down):
    # an object entry's fields are validated too, because build()
    # emits full_name and html_url verbatim into the maintained
    # artifact (a newline in either forges a phantom repo entry) and
    # classify() reads fork (a string "false" is truthy and silently
    # un-watches a real owned repo). A wrong-shaped field fails here,
    # at the one gate every consumer passes through, with the same
    # named error.
    full_name = entry["full_name"]
    if not FULL_NAME_RE.fullmatch(full_name) or full_name.split("/")[1] in (
        ".",
        "..",
    ):
        raise ValueError(
            f"{source}: enumeration entry {index} has a malformed "
            f"full_name (not owner/name): {full_name!r:.80}"
        )
    html_url = entry.get("html_url")
    if html_url is not None:
        if not isinstance(html_url, str):
            raise ValueError(
                f"{source}: enumeration entry {index} has a non-string "
                f"html_url: {html_url!r:.80}"
            )
        if any(ch.isspace() or ord(ch) < 0x20 or ord(ch) == 0x7F for ch in html_url):
            raise ValueError(
                f"{source}: enumeration entry {index} has an html_url "
                f"with whitespace/control characters: {html_url!r:.80}"
            )
    fork = entry.get("fork")
    if fork is not None and not isinstance(fork, bool):
        raise ValueError(
            f"{source}: enumeration entry {index} has a non-boolean fork: {fork!r:.80}"
        )
    return entry


def decode_pages(text: str, source: str = "enumeration capture") -> list[dict]:
    """Decode one or more concatenated JSON documents.

    `gh api --paginate` emits one JSON array (or object) per page, so a
    captured enumeration file — like live --paginate output — is a
    stream of documents, not a single JSON value. Decode them all and
    flatten the entries. Every entry is shape-validated as it lands
    (validate_entry), so a wrong-shape capture fails at decode time
    naming the source and entry instead of crashing a consumer.
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
            for entry in doc:
                entries.append(validate_entry(entry, source, len(entries)))
        else:
            entries.append(validate_entry(doc, source, len(entries)))
    return entries


def load_enumeration(path: str | None) -> list[dict]:
    if path is None:
        return []
    return decode_pages(Path(path).read_text(), source=f"enumeration capture {path}")


def merge_repos(*groups: list[dict]) -> list[dict]:
    """Merge enumeration captures into one list, deduped by
    full_name (first capture wins). A repo lives in exactly one org,
    but the org and user captures can overlap, and the two org
    captures are merged before classification (#5615)."""
    merged: dict[str, dict] = {}
    for group_no, group in enumerate(groups):
        for i, repo in enumerate(group):
            repo = validate_entry(repo, f"merge_repos group {group_no}", i)
            merged.setdefault(repo["full_name"], repo)
    return list(merged.values())


EXPECTED_USER_LOGIN = "dezren39"


def authenticated_login() -> str:
    out = subprocess.run(
        ["gh", "api", "user"],
        capture_output=True,
        text=True,
        check=True,
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
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    # --paginate concatenates one JSON array per page.
    return decode_pages(out, source=f"live enumeration {endpoint}")


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
        nested = (
            "/repo-archive/" in line.split("|")[1]
            if "|" in line
            else "/repo-archive/" in line
        )
        entry = found.setdefault(
            full, {"url": f"https://github.com/{full}", "nested": False}
        )
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


def build(
    org_repos: list[dict],
    user_repos: list[dict],
    census_text: str,
    census_label: str,
    overrides: dict[str, dict],
) -> str:
    census = parse_census(census_text)
    gh: dict[str, dict] = {}
    # build re-indexes by full_name, so it validates entries itself:
    # a caller that bypassed decode_pages/merge_repos gets the same
    # named error, not a bare KeyError (#6716).
    for i, repo in enumerate(org_repos):
        repo = validate_entry(repo, "build org_repos", i)
        gh[repo["full_name"]] = repo
    for i, repo in enumerate(user_repos):
        repo = validate_entry(repo, "build user_repos", i)
        gh[repo["full_name"]] = repo

    names = sorted(set(gh) | set(census), key=str.lower)
    lines: list[str] = []
    lines.append(
        "# Repo universe inventory — maintained artifact (#1650, piece 1 of #1511)."
    )
    lines.append("#")
    lines.append("# Generated by scripts/fleet/repo-universe/generate.py from:")
    lines.append(
        "#   - GitHub enumeration: orgs/developing-today/repos + orgs/developing-today-forks/repos + user/repos?affiliation=owner (as dezren39)"
    )
    lines.append(f"#   - Census: {census_label}")
    lines.append(
        f"#   - Census sha256: {hashlib.sha256(census_text.encode()).hexdigest()}"
    )
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
        url = (
            (repo or {}).get("html_url")
            or (centry or {}).get("url")
            or f"https://github.com/{full}"
        )
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
    ap.add_argument(
        "--org-json",
        help="captured `gh api --paginate orgs/developing-today/repos` JSON (the developing-today half of the two-org enumeration; merged with --forks-org-json before classification)",
    )
    ap.add_argument(
        "--forks-org-json",
        help="captured `gh api --paginate orgs/developing-today-forks/repos` JSON (the developing-today-forks half of the two-org enumeration; merged with --org-json before classification)",
    )
    ap.add_argument(
        "--user-json",
        help="captured `gh api --paginate 'user/repos?affiliation=owner'` JSON (authenticated as dezren39; users/dezren39/repos is public-only)",
    )
    ap.add_argument(
        "--live",
        action="store_true",
        help="fetch the enumeration via gh instead of files",
    )
    ap.add_argument("--census", required=True, help="path to all-repos-census.md")
    ap.add_argument(
        "--census-label",
        default="all-repos-census.md (2026-10-27)",
        help="provenance string recorded in the artifact header",
    )
    ap.add_argument("--output", required=True, help="artifact path to write")
    args = ap.parse_args(argv)

    if args.live:
        require_expected_account()
        # Both orgs (#5615): developing-today-forks holds opencode and
        # openchamber since 2026-11-04; enumerating developing-today
        # alone would drop them from a full regeneration.
        org_repos = merge_repos(
            *(fetch_live(f"orgs/{org}/repos?per_page=100&type=all") for org in ORGS)
        )
        # user/repos?affiliation=owner, NOT users/dezren39/repos: the
        # latter returns public repos only and silently misses private
        # owned repos (dezren39/hatch-personal among them, found at
        # build time — the census row was the only sign).
        user_repos = fetch_live("user/repos?per_page=100&affiliation=owner")
    else:
        # All three captures are required offline: a missing one
        # would silently regenerate with that source emptied,
        # deleting its GitHub-only repos from the artifact.
        missing = [
            flag
            for flag, value in (
                ("--org-json", args.org_json),
                ("--forks-org-json", args.forks_org_json),
                ("--user-json", args.user_json),
            )
            if not value
        ]
        if missing:
            ap.error(
                "offline mode requires " + " and ".join(missing) + " (or use --live)"
            )
        org_repos = merge_repos(
            load_enumeration(args.org_json),
            load_enumeration(args.forks_org_json),
        )
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
    try:
        sys.exit(main())
    except ValueError as exc:
        # A wrong-shape enumeration capture (#6716) is a named,
        # actionable failure, not a traceback.
        raise SystemExit(f"error: {exc}")
