#!/usr/bin/env bash
# check-suite-registry.sh — the coverage ratchet (#6188, Phase 4 of
# the #6185 test-suite coverage contract, rule 5).
#
# Compares the on-disk suite inventory against the registry
# (.github/test-suite-registry.json) and the venues' contents, and
# fails — naming the offender — when:
#   (a) a test suite exists on disk inside the registry's stated
#       in-scope boundary but no registry entry's paths cover it
#       (an unwired/unregistered suite);
#   (b) a mapped suite's venue no longer names the suite: neither
#       the workflow file nor, for flake-check venues, flake.nix
#       mentions any anchor of the entry (its id, an invocation
#       file, its job-step tokens, or its paths);
#   (c) a registry row dangles: a suite path glob matches nothing
#       on disk, an excluded-suite path matches nothing, or a
#       venue workflow file no longer exists.
# Waivered suites are registry entries carrying a reason (a suite
# entry with a non-empty "waiver", an excluded_suites row, or a
# top-level waivers entry naming the suite id or a path glob) —
# never exceptions coded into this check.
#
# Rule 2's enablement half (Actions API state) is deliberately NOT
# checked here: it cannot be read from inside the tree and is
# audited separately (Phase 3 of #6185).
#
# Usage: check-suite-registry.sh [REPO_ROOT]
# Exit: 0 = registry and tree agree; 1 = violations (named on stdout).
set -euo pipefail

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"

exec python3 - "$ROOT" <<'PYEOF'
import fnmatch, json, re, sys
from pathlib import Path

root = Path(sys.argv[1])
reg_path = root / ".github/test-suite-registry.json"
try:
    reg = json.loads(reg_path.read_text())
except Exception as e:
    print(f"FAIL registry: cannot read {reg_path}: {e}")
    sys.exit(1)

failures = []

def glob_match(pattern, path):
    pp, xp = pattern.split("/"), path.split("/")
    def rec(pi, xi):
        if pi == len(pp):
            return xi == len(xp)
        if pp[pi] == "**":
            return any(rec(pi + 1, k) for k in range(xi, len(xp) + 1))
        if xi >= len(xp):
            return False
        return fnmatch.fnmatchcase(xp[xi], pp[pi]) and rec(pi + 1, xi + 1)
    return rec(0, 0)

def any_match(patterns, path):
    return any(glob_match(p, path) for p in patterns)

suites = reg.get("suites", [])
waivers = reg.get("waivers", []) or []
waived_ids = {w.get("suite") for w in waivers if isinstance(w, dict) and w.get("suite")}
waived_paths = [w["path"] for w in waivers if isinstance(w, dict) and w.get("path")]

def is_waived(suite):
    return suite.get("id") in waived_ids or bool(str(suite.get("waiver", "")).strip())

# ---- (c) dangling registry rows -------------------------------------
for s in suites:
    for p in s.get("paths", []):
        if not list(root.glob(p)):
            failures.append(f"dangling: suite {s['id']} path {p} matches nothing on disk")
    v = s.get("venue") or {}
    wf = v.get("workflow")
    if wf and not (root / wf).exists():
        failures.append(f"dangling: suite {s['id']} venue workflow {wf} no longer exists")
for e in reg.get("excluded_suites", []):
    if not list(root.glob(e["path"])):
        failures.append(f"dangling: excluded suite path {e['path']} matches nothing on disk")

# ---- (b) venue naming ------------------------------------------------
def anchors(s):
    out = {s["id"]}
    inv = s.get("invocation", "")
    for tok in re.findall(r"[\w./-]+\.(?:sh|py|mjs|ts|js|go|rs|nix)\b", inv):
        out.add(Path(tok).name)
        out.add(tok)
    v = s.get("venue") or {}
    for tok in re.split(r"[^A-Za-z0-9_-]+", v.get("job_step", "")):
        if len(tok) >= 5:
            out.add(tok)
    for p in s.get("paths", []):
        static = []
        for part in p.split("/"):
            if "*" in part or "?" in part:
                break
            static.append(part)
        if len(static) >= 2:
            out.add("/".join(static))
        if "*" not in p:
            out.add(p)
    return out

flake_file = root / "flake.nix"
flake_text = flake_file.read_text(errors="replace") if flake_file.exists() else ""
for s in suites:
    if s.get("status") != "mapped" or is_waived(s):
        continue
    v = s.get("venue") or {}
    wf = v.get("workflow")
    if not wf:
        failures.append(f"venue: suite {s['id']} is mapped but names no venue workflow")
        continue
    wf_path = root / wf
    if not wf_path.exists():
        continue  # already reported as dangling
    text = wf_path.read_text(errors="replace")
    if "flake check" in s.get("invocation", ""):
        text += "\n" + flake_text
    if not any(a in text for a in anchors(s)):
        failures.append(f"venue: suite {s['id']} is no longer named by its venue {wf}")

# ---- (a) unwired on-disk suites inside the stated boundary ----------
boundaries = (reg.get("scope") or {}).get("boundaries", [])
in_roots = [b["path"].replace("/**", "") for b in boundaries
            if b.get("path") and b.get("decision") == "in_scope"]
out_pats = [b["path"] for b in boundaries
            if b.get("path") and b.get("decision") == "out_of_scope"]
suite_pats = [p for s in suites for p in s.get("paths", [])]
excl_pats = [e["path"] for e in reg.get("excluded_suites", [])]

TEST_RES = [
    re.compile(r"(^|/)tests?/.*\.rs$"),          # rust integration suites
    re.compile(r"_test\.go$"),                    # go suites
    re.compile(r"\.test\.(ts|tsx|js|mjs|cjs)$"),  # bun/node suites
    re.compile(r"\.spec\.(ts|tsx|js)$"),          # playwright/spec suites
    re.compile(r"(^|/)(test_[^/]*|[^/]*_test)\.py$"),
    re.compile(r"(^|/)(test_[^/]*|[^/]*_test)\.sh$"),
    re.compile(r"\.test\.sh$"),
]
# Fixtures, golden data, and test helpers are inputs to suites, not
# suites (the registry's own scope rule) — as are rust module files.
SKIP_DIRS = {"fixtures", "testdata", "test_data", "__fixtures__", "common", "support", "helpers"}

unwired = []
for r in in_roots:
    base = root / r
    if not base.exists():
        continue
    for f in sorted(base.rglob("*")):
        if not f.is_file():
            continue
        rel = f.relative_to(root).as_posix()
        if any(part in SKIP_DIRS for part in f.relative_to(base).parts[:-1]):
            continue
        if f.name == "mod.rs":
            continue
        if not any(rx.search(rel) for rx in TEST_RES):
            continue
        if any_match(out_pats, rel) or any_match(excl_pats, rel):
            continue
        if any_match(waived_paths, rel):
            continue
        if any_match(suite_pats, rel):
            continue
        unwired.append(rel)
for rel in unwired:
    failures.append(f"unwired: test suite {rel} exists inside the registry boundary but no registry entry covers it")

if failures:
    print(f"check-suite-registry: {len(failures)} violation(s):")
    for f in failures:
        print(f"  FAIL {f}")
    sys.exit(1)
print(f"check-suite-registry: OK — {len(suites)} registry suites; "
      f"no dangling rows, every mapped venue still names its suite, "
      f"no unwired suites inside the boundary ({', '.join(in_roots)})")
PYEOF
