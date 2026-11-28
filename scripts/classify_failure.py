#!/usr/bin/env python3
"""Scout classifier for the main-broken chain (#2176, failure-triage design).

Reads the log tails of a failed Actions run's failing jobs and classifies
the breakage before any fix spend:

    quota-or-secret   credentials / secrets / rate-limit / billing signals
    infra-runner      the runner itself died, vanished, or ran out of disk
    infra-network     DNS / connect / registry-download failures
    flake-suspect     test timeouts and other intermittency markers, with
                      no deterministic failure signature
    real-regression   a deterministic code/test failure signature
    unknown           none of the above matched

Routing (consumed by .github/workflows/main-broken.yml):

    quota-or-secret   -> escalate (zero fix attempts)
    infra-*           -> rerun    (one automatic re-run of failed jobs)
    flake-suspect     -> rerun    (one re-run; strike counting lives in
                                   the workflow, which can see the issue)
    real-regression   -> fix      (existing Jules/queue routing)
    unknown           -> fix      (existing routing, diagnosis attached)

Rules are evaluated in the order above: a quota or runner problem must
never be misread as a code regression just because the job also printed
a failing-test summary on its way down.

Input: one or more log files (plain paths, or --logs-dir DIR reading
every *.log in DIR). Only the last --tail-lines lines of each file are
considered -- the classifier is a log-TAIL reader by design. An optional
--jobs-manifest TSV (job_id<TAB>job_name per line) names the jobs in
the evidence output.

Output: one JSON object on stdout:
    {"classification": ..., "route": ..., "detail": ...,
     "evidence": ["<job>: <line>", ... up to 5], "jobs": [...]}

Exit code is 0 even for `unknown` -- an unclassifiable failure is a
result, not a classifier error. Exit code 2 is reserved for usage/IO
errors (no log files, unreadable manifest).
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

MAX_EVIDENCE = 5
DEFAULT_TAIL_LINES = 400

# Each rule: (classification, route, [compiled patterns]). Patterns are
# searched case-insensitively against each tail line. The first rule
# (in RULES order) with any match wins; within a rule, the first
# matching line per job is preferred so evidence spans jobs.
RULES: list[tuple[str, str, list[re.Pattern[str]]]] = [
    (
        "quota-or-secret",
        "escalate",
        [
            re.compile(r"bad credentials", re.I),
            re.compile(r"http\.?error:? 401|status code 401|\b401 unauthorized\b", re.I),
            re.compile(r"(secret|token|api[_-]?key|password)[^\n]{0,40}?\b(not set|missing|invalid|expired|not found)\b", re.I),
            re.compile(r"context access might be invalid", re.I),
            re.compile(r"(api )?rate limit (exceeded|reached)|secondary rate limit", re.I),
            re.compile(r"(spending limit|billing (is )?(disabled|required)|payment required|quota (exceeded|reached))", re.I),
            re.compile(r"error: (the )?secret \S+ (does not exist|not found)", re.I),
        ],
    ),
    (
        "infra-runner",
        "rerun",
        [
            re.compile(r"the runner has received a shutdown signal", re.I),
            re.compile(r"(self-hosted )?runner (lost communication|went offline|not found|was lost)", re.I),
            re.compile(r"no runner found|no available runner", re.I),
            re.compile(r"job was abandoned|the job was abandoned", re.I),
            re.compile(r"no space left on device", re.I),
            re.compile(r"runner\.? (communication|connection) (lost|dropped)", re.I),
        ],
    ),
    (
        "infra-network",
        "rerun",
        [
            re.compile(r"could not resolve host|temporary failure in name resolution|name or service not known", re.I),
            re.compile(r"failed to connect to \S+ port \d+", re.I),
            re.compile(r"connection (timed out|reset by peer|refused)", re.I),
            re.compile(r"\bEAI_AGAIN\b|\bECONNRESET\b|\bETIMEDOUT\b", re.I),
            re.compile(r"error sending request for url", re.I),
            re.compile(r"(tls|ssl) handshake (failure|failed|error)", re.I),
            re.compile(r"502 bad gateway|503 service unavailable|504 gateway time-?out", re.I),
        ],
    ),
    (
        "real-regression",
        "fix",
        [
            # Deterministic compiler diagnostics beat flake signals: a log
            # tail holding both a test timeout and a compiler error is a
            # regression to fix, not a flake to re-run (CodeRabbit Major
            # finding, PR #7806).
            re.compile(r"error\[E\d+\]:|error TS\d+:|compilation (failed|error)", re.I),
        ],
    ),
    (
        "flake-suspect",
        "rerun",
        [
            re.compile(r"test timed out|panic: test timed out", re.I),
            re.compile(r"timeout of \d+ ?(ms|s|sec).*exceeded|exceeded timeout of", re.I),
            re.compile(r"\bflaky\b|\bflaked\b|intermittent(ly)? fail", re.I),
            re.compile(r"waiting for .* timed out", re.I),
            re.compile(r"deadline exceeded.*(test|spec|case)", re.I),
        ],
    ),
    (
        "real-regression",
        "fix",
        [
            re.compile(r"^--- FAIL:|^=== RUN.*\n.*--- FAIL:", re.I | re.M),
            re.compile(r"--- FAIL:", re.I),
            re.compile(r"\bFAILED\b.*(::|test)|failures?:\s*$", re.I),
            re.compile(r"assertion(error|failed)|assert(ion)? failed", re.I),
            re.compile(r"test result: FAILED", re.I),
            re.compile(r"exit code 1.*(test|spec)", re.I),
        ],
    ),
]

SECRET_NAME_RE = re.compile(
    r"(?:secret|token|api[_-]?key)\s+([A-Z][A-Z0-9_]{2,})", re.I
)
RUN_GROUP_START_RE = re.compile(r"##\[group\]Run(?:\s|$)", re.I)
GROUP_END_RE = re.compile(r"##\[endgroup\]", re.I)


def tail_lines(text: str, n: int) -> list[str]:
    if n <= 0:
        return []
    lines = text.splitlines()
    return lines[-n:] if len(lines) > n else lines


def without_echoed_commands(text: str) -> str:
    """Drop Actions' echoed ``Run`` scripts, retaining their command output.

    GitHub wraps the shell source printed before execution in a ``Run`` log
    group. A credential-looking string in an unexecuted branch of that source
    is not evidence that a credential failed, so do not classify those lines.
    """
    kept: list[str] = []
    in_run_group = False
    for line in text.splitlines():
        if in_run_group:
            if GROUP_END_RE.search(line):
                in_run_group = False
            continue
        if RUN_GROUP_START_RE.search(line):
            in_run_group = True
            continue
        kept.append(line)
    return "\n".join(kept)


def classify(logs: dict[str, str], tail: int = DEFAULT_TAIL_LINES) -> dict:
    """logs maps job display name -> full log text. Returns the result dict."""
    tailed = {
        name: tail_lines(without_echoed_commands(text), tail)
        for name, text in logs.items()
    }
    for classification, route, patterns in RULES:
        evidence: list[str] = []
        detail = ""
        for name, lines in tailed.items():
            for line in lines:
                stripped = line.strip()
                if not stripped:
                    continue
                if any(p.search(stripped) for p in patterns):
                    evidence.append(f"{name}: {stripped[:220]}")
                    if classification == "quota-or-secret" and not detail:
                        m = SECRET_NAME_RE.search(stripped)
                        if m:
                            detail = f"credential material implicated: {m.group(1)}"
                    break  # one evidence line per job per winning rule
            if len(evidence) >= MAX_EVIDENCE:
                break
        if evidence:
            if classification == "quota-or-secret" and not detail:
                detail = "credential/quota signal in job log tail (see evidence)"
            return {
                "classification": classification,
                "route": route,
                "detail": detail,
                "evidence": evidence[:MAX_EVIDENCE],
                "jobs": sorted(logs),
            }
    # No rule matched: evidence is the last non-empty line of each job,
    # so the agent-note still carries something a human can act on.
    evidence = []
    for name, lines in tailed.items():
        for line in reversed(lines):
            if line.strip():
                evidence.append(f"{name}: {line.strip()[:220]}")
                break
        if len(evidence) >= MAX_EVIDENCE:
            break
    return {
        "classification": "unknown",
        "route": "fix",
        "detail": "no classifier rule matched the failing job log tails",
        "evidence": evidence[:MAX_EVIDENCE],
        "jobs": sorted(logs),
    }


def load_logs(paths: list[Path], manifest: Path | None) -> dict[str, str]:
    names: dict[str, str] = {}
    if manifest:
        for row in manifest.read_text(encoding="utf-8", errors="replace").splitlines():
            parts = row.split("\t")
            if len(parts) >= 2:
                names[parts[0]] = parts[1]
    logs: dict[str, str] = {}
    for path in paths:
        job = names.get(path.stem, path.stem)
        logs[job] = path.read_text(encoding="utf-8", errors="replace")
    return logs


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("logfiles", nargs="*", help="log files to classify")
    ap.add_argument("--logs-dir", type=Path, help="read every *.log in DIR")
    ap.add_argument("--jobs-manifest", type=Path, help="TSV: job_id<TAB>job_name")
    ap.add_argument("--tail-lines", type=int, default=DEFAULT_TAIL_LINES)
    args = ap.parse_args(argv)

    paths: list[Path] = [Path(p) for p in args.logfiles]
    if args.logs_dir:
        paths.extend(sorted(args.logs_dir.glob("*.log")))
    if not paths:
        print("classify_failure: no log files given", file=sys.stderr)
        return 2
    try:
        logs = load_logs(paths, args.jobs_manifest)
    except OSError as exc:
        print(f"classify_failure: {exc}", file=sys.stderr)
        return 2
    result = classify(logs, tail=args.tail_lines)
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
