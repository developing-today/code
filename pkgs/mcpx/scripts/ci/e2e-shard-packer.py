#!/usr/bin/env python3
"""Deal the mcpx internal/e2e suite into shards by measured test time (#1994).

The suite is packed by longest-processing-time (LPT) greedy: tests are
sorted by weight, then each is placed in the currently lightest shard.
Weights come from a committed timing seed (test name -> seconds).  A test
with no measurement gets the median of the measured tests; when the seed
is missing or empty every test weighs 1.0, so the result is the old
count-balanced deal.

The shard count is arithmetic, not a constant:

    N = clamp(ceil(total_weight / target_leg_seconds), 1, max_shards)

Stop-rule (see .github/workflows/mcpx-test.yml): adding a shard only
helps while the e2e leg is the longest job.  Once T/N (total test time
over shard count) falls below a leg's fixed cost (checkout + Go setup +
deno/bun/node setup, ~35 s each) plus the observed queue delay, another
shard buys nothing.  Run 41530429335 measured shards at 2 m 50 s-3 m 05 s
with a 38-45 min queue delay, so queue delay dominated and no further
splitting was justified.  This script makes the T/N term visible so that
decision is re-evaluated on data instead of guessed.

Emits, for GitHub Actions:
  matrix=<json>  include entries {shard, count, run, seconds, longest}
  total=<n>
  shards=<n>
  longest_leg=<seconds>

and writes a human-readable pack table to $GITHUB_STEP_SUMMARY when set.

The coverage assertion is part of the contract: every discovered test must
be assigned exactly once, or the job fails rather than running a shard
that silently executes nothing.
"""

from __future__ import annotations

import argparse
import glob
import json
import math
import os
import re
import statistics
import sys

TEST_RE = re.compile(r"^func (Test[A-Za-z0-9_]*)\s*\(", re.MULTILINE)

# Per-test rate when no seed exists: see load_weights.
DEFAULT_SECONDS = 3.0


# --- Build-constraint awareness (#6906) -----------------------------------
# The shard legs run `go test` with no -tags on linux/amd64 runners
# (.github/workflows/mcpx-test.yml), so discovery must return exactly the
# tests that untagged build contains. A text-only scan also returns tests
# behind unsatisfied //go:build constraints (e.g. the realsoftware-tagged
# e2e tests): those are dealt into shards, counted in matrix.count, and can
# never be reported by the leg that owns them, so the leg's count check
# fails. The rules below mirror go/build's default-context matching for
# the legs' context: GOOS/GOARCH filename suffixes plus //go:build (and
# legacy // +build) expressions evaluated with no custom tags set.

DEFAULT_GOOS = "linux"
DEFAULT_GOARCH = "amd64"

_KNOWN_GOOS = {
    "aix", "android", "darwin", "dragonfly", "freebsd", "hurd", "illumos",
    "ios", "js", "linux", "netbsd", "openbsd", "plan9", "solaris",
    "wasip1", "windows",
}
_KNOWN_GOARCH = {
    "386", "amd64", "arm", "arm64", "loong64", "mips", "mips64", "mipsle",
    "mips64le", "ppc64", "ppc64le", "riscv64", "s390x", "sparc64", "wasm",
}

# Tags the default build satisfies beyond GOOS/GOARCH: "unix" (linux is a
# unix GOOS), the "gc" compiler, and cumulative go1.x release tags (the
# legs build with the repo's current Go from go.mod, so a release tag at
# or below any plausible toolchain version is satisfied; a future go1.x
# tag beyond the toolchain would also gate new code the legs cannot run
# yet, which discovery cannot resolve without invoking go list).
_SATISFIED_TAGS = {DEFAULT_GOOS, DEFAULT_GOARCH, "unix", "gc"}


def _tag_satisfied(tag: str) -> bool:
    if tag in _SATISFIED_TAGS:
        return True
    if re.fullmatch(r"go1\.\d+", tag):
        return True
    return False


_TOKEN_RE = re.compile(r"\s*(?:(&&)|(\|\|)|(!)|(\()|(\))|([A-Za-z0-9_.]+))")


def _eval_build_expr(expr: str) -> bool:
    """Evaluate a //go:build expression under the default build context."""
    tokens: list[str] = []
    pos = 0
    while pos < len(expr):
        m = _TOKEN_RE.match(expr, pos)
        if not m or m.end() == pos:
            raise ValueError(f"unparseable build expression: {expr!r}")
        pos = m.end()
        tokens.append(next(g for g in m.groups() if g is not None))
    state = {"i": 0}

    def peek() -> str | None:
        return tokens[state["i"]] if state["i"] < len(tokens) else None

    def parse_or() -> bool:
        value = parse_and()
        while peek() == "||":
            state["i"] += 1
            rhs = parse_and()
            value = value or rhs
        return value

    def parse_and() -> bool:
        value = parse_unary()
        while peek() == "&&":
            state["i"] += 1
            rhs = parse_unary()
            value = value and rhs
        return value

    def parse_unary() -> bool:
        tok = peek()
        if tok == "!":
            state["i"] += 1
            return not parse_unary()
        if tok == "(":
            state["i"] += 1
            value = parse_or()
            if peek() != ")":
                raise ValueError(f"unbalanced build expression: {expr!r}")
            state["i"] += 1
            return value
        if tok is None or tok in ("&&", "||", ")"):
            raise ValueError(f"unexpected token in build expression: {expr!r}")
        state["i"] += 1
        return _tag_satisfied(tok)

    result = parse_or()
    if state["i"] != len(tokens):
        raise ValueError(f"trailing tokens in build expression: {expr!r}")
    return result


def _eval_plus_build(lines: list[str]) -> bool:
    """Evaluate legacy // +build lines: lines AND together, space-separated
    options OR together, comma-separated terms AND together (! negates)."""
    for line in lines:
        line_ok = False
        for option in line.split():
            term_ok = True
            for term in option.split(","):
                if term.startswith("!"):
                    term_ok = term_ok and not _tag_satisfied(term[1:])
                else:
                    term_ok = term_ok and _tag_satisfied(term)
            line_ok = line_ok or term_ok
        if not line_ok:
            return False
    return True


def _filename_included(path: str) -> bool:
    """Mirror go/build's GOOS/GOARCH filename-suffix rule."""
    base = os.path.basename(path)
    if base.endswith("_test.go"):
        base = base[: -len("_test.go")]
    parts = base.split("_")
    if len(parts) >= 2 and parts[-1] in _KNOWN_GOARCH:
        if parts[-1] != DEFAULT_GOARCH:
            return False
        parts = parts[:-1]
    if len(parts) >= 2 and parts[-1] in _KNOWN_GOOS:
        if parts[-1] != DEFAULT_GOOS:
            return False
    return True


def _constraints_included(text: str) -> bool:
    """Evaluate the file's build constraints (header block only)."""
    go_build: str | None = None
    plus_build: list[str] = []
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("package "):
            break
        if stripped.startswith("//go:build"):
            go_build = stripped[len("//go:build"):].strip()
        elif stripped.startswith("// +build"):
            plus_build.append(stripped[len("// +build"):].strip())
        elif stripped and not stripped.startswith("//"):
            break
    if go_build is not None:
        return _eval_build_expr(go_build)
    if plus_build:
        return _eval_plus_build(plus_build)
    return True


def _file_included(path: str, text: str) -> bool:
    return _filename_included(path) and _constraints_included(text)


def discover(tests_dir: str) -> list[str]:
    names: set[str] = set()
    for path in sorted(glob.glob(os.path.join(tests_dir, "*_test.go"))):
        with open(path, errors="replace") as fh:
            text = fh.read()
        if not _file_included(path, text):
            continue
        # TestMain is the package test entry point, not a test: go test
        # -list never reports it and a -run leg can never report it either,
        # so counting it breaks the same count contract (#6906).
        names.update(n for n in TEST_RE.findall(text) if n != "TestMain")
    return sorted(names)


def load_weights(path: str | None, names: list[str]) -> tuple[dict[str, float], str]:
    measured: dict[str, float] = {}
    if path and os.path.exists(path):
        with open(path) as fh:
            doc = json.load(fh)
        raw = doc.get("tests", doc) if isinstance(doc, dict) else {}
        for name, seconds in raw.items():
            if name in names and isinstance(seconds, (int, float)) and seconds > 0:
                measured[name] = float(seconds)
    if not measured:
        # Equal weights, scaled by the only per-test rate this repo has:
        # run 41530429335 spent ~174 s over ~58 tests in each of six shards,
        # i.e. ~3 s per test. Unit weights would under-count the suite and
        # silently drop the shard count, so the fallback keeps that scale.
        return {name: DEFAULT_SECONDS for name in names}, (
            f"equal weights at the run-41530429335 rate ({DEFAULT_SECONDS:.0f}s/test; no timing seed)")
    median = statistics.median(measured.values())
    weights = {name: measured.get(name, median) for name in names}
    return weights, f"seed: {len(measured)} measured, median {median:.1f}s for the rest"


def lpt_pack(weights: dict[str, float], shards: int) -> list[list[str]]:
    buckets: list[list[str]] = [[] for _ in range(shards)]
    totals = [0.0] * shards
    for name in sorted(weights, key=lambda n: (-weights[n], n)):
        target = min(range(shards), key=lambda i: (totals[i], i))
        buckets[target].append(name)
        totals[target] += weights[name]
    return [sorted(b) for b in buckets]


def pack(
    names: list[str],
    weights: dict[str, float],
    target_seconds: float,
    max_shards: int,
) -> tuple[int, list[list[str]]]:
    total = sum(weights.values())
    wanted = max(1, math.ceil(total / target_seconds)) if target_seconds > 0 else 1
    shards = max(1, min(max_shards, wanted))
    buckets = lpt_pack(weights, shards)
    # Two shards that got equal totals is impossible in practice, but never
    # emit an empty shard: drop them and remember how many buckets were dealt.
    buckets = [b for b in buckets if b]
    return len(buckets), buckets


def assert_covered(names: list[str], buckets: list[list[str]]) -> None:
    assigned = [name for bucket in buckets for name in bucket]
    if len(assigned) != len(set(assigned)):
        dupes = sorted({n for n in assigned if assigned.count(n) > 1})
        raise SystemExit(f"coverage defect: tests assigned more than once: {dupes}")
    missing = sorted(set(names) - set(assigned))
    if missing:
        raise SystemExit(f"coverage defect: tests not assigned: {missing}")
    extra = sorted(set(assigned) - set(names))
    if extra:
        raise SystemExit(f"coverage defect: handlers invented tests: {extra}")
    if len(buckets) == 0 and names:
        raise SystemExit("coverage defect: no shards emitted for a non-empty suite")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--tests-dir", default="pkgs/mcpx/internal/e2e")
    ap.add_argument("--timings", default="pkgs/mcpx/scripts/ci/e2e-timings.json")
    ap.add_argument("--target-seconds", type=float, default=170.0,
                    help="desired wall time of one e2e leg's test work")
    ap.add_argument("--max-shards", type=int, default=6)
    ap.add_argument("--github-output", default=os.environ.get("GITHUB_OUTPUT"))
    ap.add_argument("--github-summary", default=os.environ.get("GITHUB_STEP_SUMMARY"))
    args = ap.parse_args()

    names = discover(args.tests_dir)
    if not names:
        raise SystemExit(f"coverage defect: no Test functions found under {args.tests_dir}")
    weights, weight_note = load_weights(args.timings, names)
    shards, buckets = pack(names, weights, args.target_seconds, args.max_shards)
    assert_covered(names, buckets)

    include = []
    for i, bucket in enumerate(buckets):
        seconds = sum(weights[n] for n in bucket)
        include.append({
            "shard": i,
            "count": len(bucket),
            "run": "|".join(bucket),
            "seconds": round(seconds, 1),
            "longest": max((weights[n] for n in bucket), default=0.0),
        })
    longest_total = max(item["seconds"] for item in include) if include else 0.0

    matrix = json.dumps({"include": include})
    print(f"e2e tests: {len(names)} in {shards} shards ({weight_note}); "
          f"expected longest leg {longest_total:.0f}s test time")
    for item in include:
        print(f"  shard {item['shard']}: {item['count']:3d} tests, "
              f"{item['seconds']:7.1f}s test time, longest {item['longest']:.1f}s")

    if args.github_output:
        with open(args.github_output, "a") as fh:
            fh.write("matrix=" + matrix + "\n")
            fh.write(f"total={len(names)}\n")
            fh.write(f"shards={shards}\n")
            fh.write(f"longest_leg={longest_total:.1f}\n")
    if args.github_summary:
        lines = [
            "## mcpx-test init: measured-time e2e pack (#1994)",
            "",
            f"- tests: **{len(names)}** in **{shards}** shards",
            f"- weights: {weight_note}",
            f"- target leg: {args.target_seconds:.0f}s test time; "
            f"expected longest leg: **{longest_total:.0f}s**",
            "",
            "| shard | tests | test seconds | longest test |",
            "| ---: | ---: | ---: | ---: |",
        ]
        lines += [
            f"| {item['shard']} | {item['count']} | {item['seconds']:.1f} | {item['longest']:.1f} |"
            for item in include
        ]
        with open(args.github_summary, "a") as fh:
            fh.write("\n".join(lines) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
