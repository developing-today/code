#!/usr/bin/env python3
"""Affected-tests selector for pkgs/mcpx — implements the agent test tiers
decided in developing-today/code#1966 (policy text: pkgs/mcpx/TESTING.md).

Given a diff base, print the smallest set of test commands that covers the
change, by tier:

  T0  gofmt -l on edited .go files; go vet on touched packages
  T1  unit tests of each package with an edited .go file
  T1b unit tests of packages importing a package whose exported
      declarations changed
  T2  targeted e2e: test functions whose bodies mention a name the diff
      touches in a registry file (settings wire, api ops, cli flags /
      commands), plus the rule-4 harness sample when the test harness
      changed. Budget: at most 15 tests (the count is what an agent can
      see before running; the printed estimate, ~0.5 s/test + one binary
      build, is the time view of the same budget — #1966 decision).
  T3  guard tests for any registry the diff touched
  T4  nix build .#mcpx, only when package.nix / go.mod / go.sum changed

Dry-run by default: commands are printed, nothing is executed. --run
executes them in order and stops at the first failure. If no e2e test is
selected the selector says so and NEVER falls back to the full suite —
the full suite is CI's job (rule 1).

The diff is the working tree against the base (committed + uncommitted +
untracked), so the selector works before anything is committed:
    git diff --name-only <base>   +   git ls-files --others

Usage:
    affected-tests.py [--base origin/main] [--run]
    just mcpx-test-affected [base] [--run]
"""

import argparse
import os
import re
import subprocess
import sys

T2_BUDGET = 15  # tests; see TESTING.md rule/T2 note for the decision record
E2E_SECONDS_PER_TEST = 0.5

# Rule 4 (#1966): harness changes affect every e2e test; run this sample.
HARNESS_SAMPLE = [
    "TestNoOrphanProcessesAfterStop",
    "TestEnvCleanupStopsTheDaemonItStarted",
    "TestEveryDeclaredRouteAnswersOverTheSocket",
]

# T3 guard map: registry area -> [(package, [guard test names])].
# Names verified against the tree when this map was written; #1971 owns
# the guards themselves, this map only wires selection to them.
GUARDS = {
    "settings": [
        ("internal/cli", ["TestEverySettingFlagReachesTheResolvedValue"]),
        ("internal/settings", ["TestEveryPoolDefaultIsTheDefaultsJSONValue"]),
        ("internal/config", ["TestEveryDeclaredKeyIsRecognisedInAnyCase"]),
    ],
    "api": [
        ("internal/api", [
            "TestEverySchemaIsValidJSON",
            "TestEveryParameterIsDescribed",
            "TestEveryDaemonRouteIsDeclared",
            "TestEveryDeclaredOpHasARoute",
            "TestEveryNonStreamingOpIsReachableAsAnMCPTool",
        ]),
    ],
    "cli": [
        ("internal/cli", [
            "TestEveryCommandAppearsInGeneratedArtefacts",
            "TestEveryDispatchedCommandIsDeclared",
            "TestEveryOperationIsReachableFromTheCLI",
            "TestEveryCommandIsInAListedGroup",
            "TestEveryCommandReachesAnOperationOrSaysWhy",
        ]),
    ],
}

# e2e guards are selected into the T2 -run set rather than their own
# command, so a registry change still pays only one e2e binary build.
E2E_GUARDS = {
    "api": ["TestEveryDeclaredRouteAnswersOverTheSocket"],
    "settings": ["TestEveryDeclaredRouteAnswersOverTheSocket"],
}


def run(cmd, cwd, check=True):
    p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)
    if check and p.returncode != 0:
        sys.exit(f"command failed: {' '.join(cmd)}\n{p.stderr}")
    return p.stdout


def registry_area(path):
    """Map a pkgs/mcpx-relative path to a registry area, or None."""
    if path == "internal/settings/wire.go" or path.startswith("internal/defaults/"):
        return "settings"
    if re.match(r"internal/api/ops[^/]*\.go$", path) and not path.endswith("_test.go"):
        return "api"
    if path.startswith("internal/cli/") and path.endswith(".go") and not path.endswith("_test.go"):
        return "cli"
    return None


def changed_exported_idents(diff_text):
    """Exported declarations added/removed in a unified diff (approx.):
    func/type/var/const lines whose name starts uppercase."""
    idents = set()
    for line in diff_text.splitlines():
        if not line or line[0] not in "+-" or line.startswith(("+++", "---")):
            continue
        body = line[1:]
        m = re.search(r"\bfunc\s+(?:\([^)]*\)\s*)?([A-Z]\w*)", body)
        m = m or re.search(r"\btype\s+([A-Z]\w*)", body)
        m = m or re.search(r"\b(?:var|const)\s+([A-Z]\w*)", body)
        if m:
            idents.add(m.group(1))
    return idents


def diff_names(diff_text):
    """Names a registry diff touches: quoted strings (setting paths, op
    names, flag/command names) from added/removed lines."""
    names = set()
    for line in diff_text.splitlines():
        if not line or line[0] not in "+-" or line.startswith(("+++", "---")):
            continue
        for s in re.findall(r'"([^"]{3,})"', line[1:]):
            names.add(s)
    return names


def e2e_tests_mentioning(mcpx, names):
    """Parse internal/e2e test functions; return those whose body text
    contains any of the given names."""
    selected = []
    e2e = os.path.join(mcpx, "internal", "e2e")
    if not os.path.isdir(e2e):
        return selected
    for fn in sorted(os.listdir(e2e)):
        if not fn.endswith("_test.go"):
            continue
        text = open(os.path.join(e2e, fn), encoding="utf-8", errors="replace").read()
        for m in re.finditer(r"^func (Test\w+)\(t \*testing\.T\) \{", text, re.M):
            depth, i = 1, m.end()
            while i < len(text) and depth:
                if text[i] == "{":
                    depth += 1
                elif text[i] == "}":
                    depth -= 1
                i += 1
            body = text[m.start():i]
            if any(n in body for n in names):
                selected.append(m.group(1))
    return selected


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--base", default="origin/main",
                    help="diff base (default: origin/main)")
    ap.add_argument("--run", action="store_true",
                    help="execute the printed commands (default: dry-run)")
    args = ap.parse_args()

    root = run(["git", "rev-parse", "--show-toplevel"], os.getcwd()).strip()
    mcpx = os.path.join(root, "pkgs", "mcpx")
    if not os.path.isdir(mcpx):
        sys.exit("pkgs/mcpx not found — run from inside the code repo")

    changed = set(run(["git", "diff", "--name-only", args.base], root).split())
    changed |= set(run(["git", "ls-files", "--others", "--exclude-standard"], root).split())
    rel = sorted(p[len("pkgs/mcpx/"):] for p in changed if p.startswith("pkgs/mcpx/"))

    commands = []  # (tier, command string runnable from repo root)
    notes = []

    go_files = [p for p in rel if p.endswith(".go")]
    pkgs = sorted({os.path.dirname(p) for p in go_files if os.path.dirname(p)})
    unit_pkgs = [p for p in pkgs if p != "internal/e2e"]

    # T0 — format + vet on touched packages.
    if go_files:
        files = " ".join(f"pkgs/mcpx/{p}" for p in go_files)
        commands.append(("T0", f"gofmt -l {files}"))
    for p in unit_pkgs:
        commands.append(("T0", f"(cd pkgs/mcpx && go vet ./{p}/)"))

    # T1 — unit tests of edited packages.
    for p in unit_pkgs:
        commands.append(("T1", f"(cd pkgs/mcpx && go test -count=1 ./{p}/)"))

    # Diff text for T1b / registry analysis (registry + non-test sources).
    src = [p for p in go_files if not p.endswith("_test.go")]
    diff_text = ""
    if src:
        diff_text = run(["git", "diff", args.base, "--"] +
                        [f"pkgs/mcpx/{p}" for p in src], root, check=False)

    # T1b — importers of packages whose exported declarations changed.
    exported = changed_exported_idents(diff_text)
    if exported:
        listing = run(["go", "list", "-f",
                       "{{.ImportPath}} {{join .Imports \" \"}}", "./..."],
                      mcpx, check=False)
        importers = set()
        changed_pkgs = {p for p in unit_pkgs}
        for line in listing.splitlines():
            parts = line.split()
            if len(parts) < 2:
                continue
            imp_path, imports = parts[0], parts[1:]
            for cp in changed_pkgs:
                full = "github.com/dezren39/mcpx/" + cp
                if full in imports and imp_path != full:
                    importers.add(imp_path)
        for imp in sorted(importers):
            short = imp.replace("github.com/dezren39/mcpx/", "")
            if short == "internal/e2e" or short in unit_pkgs:
                continue
            commands.append(("T1b", f"(cd pkgs/mcpx && go test -count=1 ./{short}/)"))
        if importers:
            notes.append(f"T1b: exported declarations changed ({', '.join(sorted(exported))}); "
                         "importer packages added")

    # Registries touched -> names for T2, guards for T3.
    areas = {registry_area(p) for p in rel}
    areas.discard(None)
    names = diff_names(diff_text) if areas else set()

    # T2 — targeted e2e.
    e2e_selected = set()
    if names:
        e2e_selected |= set(e2e_tests_mentioning(mcpx, names))
    for a in areas:
        e2e_selected |= set(E2E_GUARDS.get(a, []))
    harness = any(p.startswith("internal/testsupport/") for p in rel)
    if harness:
        e2e_selected |= set(HARNESS_SAMPLE)
        notes.append("T2: harness changed (internal/testsupport) — rule-4 sample selected; "
                     "CI runs the rest")
    edited_e2e = {p for p in go_files if p.startswith("internal/e2e/")}
    if edited_e2e:
        # Tests defined in an edited e2e file are themselves the change.
        all_names = set()
        for p in edited_e2e:
            text = open(os.path.join(mcpx, p), encoding="utf-8", errors="replace").read()
            all_names |= set(re.findall(r"^func (Test\w+)\(", text, re.M))
        e2e_selected |= all_names
    e2e_selected = sorted(e2e_selected)
    truncated = len(e2e_selected) > T2_BUDGET
    e2e_selected = e2e_selected[:T2_BUDGET]
    if e2e_selected:
        regex = "^(" + "|".join(e2e_selected) + ")$"
        est = E2E_SECONDS_PER_TEST * len(e2e_selected)
        commands.append(("T2", f"(cd pkgs/mcpx && go test -count=1 ./internal/e2e -run '{regex}')"))
        notes.append(f"T2: {len(e2e_selected)} e2e test(s) selected, estimated ~{est:g}s "
                     "+ one binary build (budget: 15 tests / ~30s)")
        if truncated:
            notes.append("T2: selection truncated at the 15-test budget; CI runs the full suite")
    else:
        notes.append("no e2e selected; CI runs the full suite")

    # T3 — guards for touched registries.
    seen_guard = set()
    for a in sorted(areas):
        for pkg, tests in GUARDS.get(a, []):
            regex = "^(" + "|".join(tests) + ")$"
            key = (pkg, regex)
            if key in seen_guard:
                continue
            seen_guard.add(key)
            commands.append(("T3", f"(cd pkgs/mcpx && go test -count=1 ./{pkg}/ -run '{regex}')"))

    # T4 — build only when packaging/module files changed.
    if any(os.path.basename(p) in ("go.mod", "go.sum", "package.nix") for p in rel):
        commands.append(("T4", "nix build .#mcpx"))

    print(f"# affected tests for pkgs/mcpx vs {args.base} "
          f"({len(rel)} changed file(s) under pkgs/mcpx)")
    if not commands:
        print("# no tier commands selected (no code or registry changes)")
    last = None
    for tier, cmd in commands:
        if tier != last:
            print(f"\n## {tier}")
            last = tier
        print(cmd)
    if notes:
        print("\n# notes")
        for n in notes:
            print(f"# {n}")
    print("# full suite is CI's job — never run it locally for a routine change (TESTING.md rule 1)")

    if args.run:
        for _, cmd in commands:
            print(f"\n$ {cmd}", flush=True)
            p = subprocess.run(cmd, cwd=root, shell=True)
            if p.returncode != 0:
                sys.exit(p.returncode)


if __name__ == "__main__":
    main()
