#!/usr/bin/env python3
"""Plan and record batches for the pkgs/id pre-merge gate (#12229).

Design record (issue #12229, "What to build" step 2):

- **Batch size.** Up to ``DEFAULT_BATCH_SIZE`` (5) ready pkgs/id PRs
  per batch. Small enough that a red batch bisects in a few runs,
  large enough that ten batches cover a week of id PRs.
- **What runs.** One command per batch: ``cargo check --workspace``
  for pkgs/id, then ``cargo test`` for the union of the crates the
  batch's PRs affect (per ``affected-crates.py``). Commands run
  inside pkgs/id's own nix dev shell (``nix develop`` of the
  candidate's pkgs/id flake, which wires the wayland/X11 system
  libraries); the fleet build shell lacks those libraries and made
  batch 1 infrastructure-red (#12229).
- **Red batch.** Bisected by halving (``find_culprits``): a subset
  that stays red is split until each red PR is isolated. Green
  sub-batches merge; a red PR leaves the batch and goes back to its
  author with the failing command from the log. Green PRs use GitHub's
  ``--merge`` strategy to preserve PR ancestry.
- **Switching it off.** ``ID_GATE_OFF=1`` in the environment, or
  ``enabled=False`` / ``--off`` here, plans no batches. Nightly CI
  is unchanged either way (issue decision D2's undo path).
- **The log.** One JSON object per evaluated batch, appended to a
  JSONL file: ``ts``, ``batch``, ``prs``, ``command``, ``result``.
  Acceptance criterion 2 of #12229 ("its log shows each batch's
  command and result") reads this file. Each record also names its
  verdict's place in the run toward ten consecutive green batches:
  ``probe`` marks bisect sub-evaluations (a probe is never a landed
  batch), and ``consecutive_green`` counts consecutive green landed
  batches including this record — ``0`` on a red landed batch,
  ``null`` on a probe. ``gate.py status`` reads the count back
  against the target of 10.

Usage:
  python3 scripts/id/gate.py plan --prs 101,102,103 [--batch-size 5] [--off]
  python3 scripts/id/gate.py bisect --batch 101,102,103 --red 102
  python3 scripts/id/gate.py run --batch 1 --prs 12470
  python3 scripts/id/gate.py status [--file scripts/id/gate-log.jsonl]
  python3 scripts/id/gate.py log --file gate-log.jsonl --batch 1 \
      --prs 101,102 --command "cargo check --workspace" --result green
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable, Iterable, Mapping, Sequence


DEFAULT_BATCH_SIZE = 5
GREEN_TARGET = 10
OFF_ENV = "ID_GATE_OFF"
MANIFEST = "pkgs/id/Cargo.toml"
REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_LOG_PATH = REPO_ROOT / "scripts/id/gate-log.jsonl"


def gate_enabled() -> bool:
    """The kill switch: ID_GATE_OFF set to anything but "" or "0"."""
    return os.environ.get(OFF_ENV, "") not in ("", "0")


def plan_batches(
    prs: Sequence[int],
    batch_size: int = DEFAULT_BATCH_SIZE,
    enabled: bool = True,
) -> list[list[int]]:
    """Split ready PRs into batches of at most ``batch_size``, in order."""
    if not enabled or gate_enabled():
        return []
    if batch_size < 1:
        raise ValueError("batch_size must be at least 1")
    return [list(prs[i : i + batch_size]) for i in range(0, len(prs), batch_size)]


def batch_command(prs: Sequence[int], affected: Mapping[int, Iterable[str]]) -> str:
    """The one command the gate runs for a batch (see design record)."""
    crates = sorted({crate for pr in prs for crate in affected.get(pr, ())})
    command = f"cargo check --workspace --manifest-path {MANIFEST}"
    if crates:
        tested = " ".join(f"-p {crate}" for crate in crates)
        command += f" && cargo test --manifest-path {MANIFEST} {tested}"
    return command


def find_culprits(
    batch: Sequence[int], is_red: Callable[[list[int]], bool]
) -> list[int]:
    """Every PR in ``batch`` whose presence makes a run red.

    ``is_red(subset)`` evaluates a (sub)batch. The walk halves the
    batch, so a single red PR in a batch of n costs at most
    2*ceil(log2(n))+1 evaluations; a green batch costs one.
    """

    def walk(subset: list[int]) -> list[int]:
        if not is_red(subset):
            return []
        if len(subset) == 1:
            return subset
        mid = len(subset) // 2
        return walk(subset[:mid]) + walk(subset[mid:])

    return walk(list(batch))


def log_record(
    batch: int,
    prs: Sequence[int],
    command: str,
    result: str,
    *,
    probe: bool = False,
    consecutive_green: int | None = None,
    ts: str | None = None,
) -> dict[str, object]:
    """One batch-log entry: the batch, its command and its result.

    ``probe`` marks a bisect sub-evaluation: it is not a landed batch,
    so it never advances or resets the consecutive-green count and its
    ``consecutive_green`` stays ``None``. A landed (non-probe) green
    record carries the count of consecutive green landed batches
    including itself; a landed red record carries ``0``.
    """
    if result not in ("green", "red"):
        raise ValueError(f"result must be 'green' or 'red', got {result!r}")
    if probe and consecutive_green is not None:
        raise ValueError("a probe record carries no consecutive_green count")
    if not probe and consecutive_green is None:
        raise ValueError("a landed record must carry its consecutive_green count")
    return {
        "ts": ts or datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "batch": batch,
        "prs": list(prs),
        "command": command,
        "result": result,
        "probe": probe,
        "consecutive_green": consecutive_green,
    }


def consecutive_green(records: Sequence[Mapping[str, object]]) -> int:
    """Consecutive green landed batches at the end of ``records``.

    Bisect probes are skipped: only a whole batch landing green
    advances the count, and only a whole batch landing red resets it.
    Records written before this field existed carry no
    ``consecutive_green`` key and are read by ``result`` alone.
    """
    count = 0
    for record in records:
        if record.get("probe", False):
            continue
        count = count + 1 if record.get("result") == "green" else 0
    return count


def gate_status(records: Sequence[Mapping[str, object]]) -> dict[str, object]:
    """The criterion-2 reader: green count against the target of ten."""
    landed = [r for r in records if not r.get("probe", False)]
    count = consecutive_green(records)
    return {
        "consecutive_green": count,
        "target": GREEN_TARGET,
        "batches": len(landed),
        "complete": count >= GREEN_TARGET,
    }


def append_batch_record(
    path: Path,
    batch: int,
    prs: Sequence[int],
    command: str,
    result: str,
    *,
    probe: bool = False,
) -> dict[str, object]:
    """Build one log record against the log so far and append it.

    The consecutive-green count is derived from the records already in
    the file, so the log itself always shows the count advancing.
    """
    prior = consecutive_green(read_log(path))
    count = None if probe else (prior + 1 if result == "green" else 0)
    record = log_record(
        batch, prs, command, result, probe=probe, consecutive_green=count
    )
    append_log(path, record)
    return record


def append_log(path: Path, record: Mapping[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(record, sort_keys=True) + "\n")


def read_log(path: Path) -> list[dict[str, object]]:
    if not path.exists():
        return []
    return [
        json.loads(line)
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]


def _run_git(args: list[str], *, cwd: Path | None = None, **_: object) -> str:
    result = subprocess.run(
        ["git", *args],
        cwd=cwd,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return result.stdout.strip()


def _run_gh(args: list[str], *, cwd: Path | None = None, **_: object) -> str:
    result = subprocess.run(
        ["gh", *args],
        cwd=cwd,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return result.stdout.strip()


def _run_devshell(command: str, *, cwd: Path | None = None, **_: object) -> int:
    """Run one batch command inside pkgs/id's own nix dev shell.

    Batch 1 (#12229) ran under ``fleet shell --build`` and died in the
    wayland-sys build script: that shell lacks the wayland client
    libraries. pkgs/id's flake wires them (``pkgs/id/nix-common.nix``),
    so the batch command — recorded verbatim in the gate log — runs
    unchanged, from the candidate root, inside ``nix develop`` of the
    candidate's pkgs/id. The manual re-evaluation of batch 1's candidate
    in exactly this shell is the green reference run.
    """
    root = cwd if cwd is not None else REPO_ROOT
    argv = [
        "nix",
        "--extra-experimental-features",
        "nix-command flakes",
        "develop",
        str(root / "pkgs" / "id"),
        "--command",
        "bash",
        "-c",
        command,
    ]
    result = subprocess.run(argv, cwd=root)
    return result.returncode


def run_batch(
    prs: Sequence[int],
    batch: int,
    *,
    affected: Mapping[int, Iterable[str]] | None = None,
    log_path: Path = DEFAULT_LOG_PATH,
    repo: Path = REPO_ROOT,
    git: Callable[..., str] = _run_git,
    shell: Callable[..., int] = _run_devshell,
    gh: Callable[..., str] = _run_gh,
) -> str:
    """Build and gate one ordered batch. Command seams make the driver unit-testable."""
    if gate_enabled():
        return "off"
    if not prs:
        raise ValueError("a batch must contain at least one PR")
    git(["fetch", "origin", "main"], cwd=repo)
    affected_by_pr: dict[int, Iterable[str]] = dict(affected or {})

    def evaluate(subset: list[int], number: int) -> bool:
        # Each verdict is assembled from current main, never from a prior probe.
        with tempfile.TemporaryDirectory(prefix="id-gate-") as temp:
            candidate = Path(temp) / "candidate"
            git(
                ["worktree", "add", "--detach", str(candidate), "origin/main"], cwd=repo
            )
            try:
                for pr in subset:
                    data = json.loads(
                        gh(["pr", "view", str(pr), "--json", "headRefOid"], cwd=repo)
                    )
                    oid = data["headRefOid"]
                    git(["fetch", "origin", f"refs/pull/{pr}/head"], cwd=candidate)
                    git(["merge", "--no-ff", "--no-edit", "FETCH_HEAD"], cwd=candidate)
                    if pr not in affected_by_pr:
                        paths = git(
                            ["diff", "--name-only", f"origin/main...{oid}"], cwd=repo
                        )
                        selector = candidate / "scripts/id/affected-crates.py"
                        selected = subprocess.run(
                            [sys.executable, str(selector), "-"],
                            cwd=candidate,
                            input=paths,
                            text=True,
                            check=True,
                            stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE,
                        ).stdout.splitlines()
                        affected_by_pr[pr] = selected
                command = batch_command(subset, affected_by_pr)
                result = shell(command, cwd=candidate, prs=subset)
                verdict = result == 0
                append_batch_record(
                    log_path,
                    number,
                    subset,
                    command,
                    "green" if verdict else "red",
                    probe=subset != list(prs),
                )
                return verdict
            finally:
                git(["worktree", "remove", "--force", str(candidate)], cwd=repo)

    root_red = False

    def is_red(subset: list[int]) -> bool:
        nonlocal root_red
        verdict = not evaluate(subset, batch)
        if subset == list(prs):
            root_red = verdict
        return verdict

    red = find_culprits(prs, is_red)
    # Preserve safety if a failure only appears from an interaction between
    # PRs and every proper subset happens to pass: leave the whole set open.
    if root_red and not red:
        red = list(prs)
    green = [pr for pr in prs if pr not in set(red)]

    # `gh pr merge --merge` creates the durable merge commit and marks each
    # source PR merged; each accepted PR is applied in the same order tested.
    for pr in green:
        gh(["pr", "merge", str(pr), "--merge"], cwd=repo)
    return "red" if red else "green"


def _prs(text: str) -> list[int]:
    return [int(part) for part in text.split(",") if part.strip()]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    # The subcommand dest is "subcommand", not "command": the log
    # subcommand takes a --command option (the batch command to record),
    # and a dest of "command" would let that option overwrite the
    # subcommand name itself, so `gate.py log ...` would silently do
    # nothing. (That was the pre-existing behaviour of the log CLI.)
    sub = parser.add_subparsers(dest="subcommand", required=True)

    plan = sub.add_parser("plan", help="plan batches for ready PRs")
    plan.add_argument("--prs", required=True, help="comma-separated PR numbers")
    plan.add_argument("--batch-size", type=int, default=DEFAULT_BATCH_SIZE)
    plan.add_argument("--off", action="store_true", help="kill switch: plan nothing")

    bisect = sub.add_parser("bisect", help="isolate the red PRs in a red batch")
    bisect.add_argument("--batch", required=True, help="comma-separated PR numbers")
    bisect.add_argument("--red", required=True, help="comma-separated red PR numbers")

    log = sub.add_parser("log", help="append one batch result to the gate log")
    log.add_argument("--file", required=True)
    log.add_argument("--batch", type=int, required=True)
    log.add_argument("--prs", required=True, help="comma-separated PR numbers")
    log.add_argument("--command", required=True)
    log.add_argument("--result", required=True, choices=("green", "red"))
    log.add_argument(
        "--probe",
        action="store_true",
        help="bisect sub-evaluation: never a landed batch, no count",
    )

    status = sub.add_parser(
        "status", help="report the consecutive-green count toward ten"
    )
    status.add_argument(
        "--file", default=str(DEFAULT_LOG_PATH), help="JSONL batch log path"
    )

    run = sub.add_parser("run", help="assemble, test, and merge one planned batch")
    run.add_argument("--batch", type=int, required=True)
    run.add_argument(
        "--prs", required=True, help="comma-separated PR numbers, in merge order"
    )
    run.add_argument(
        "--log", default=str(DEFAULT_LOG_PATH), help="JSONL batch log path"
    )

    args = parser.parse_args(argv)

    if args.subcommand == "run":
        result = run_batch(_prs(args.prs), args.batch, log_path=Path(args.log))
        print(
            json.dumps({"batch": args.batch, "prs": _prs(args.prs), "result": result})
        )
    elif args.subcommand == "plan":
        batches = plan_batches(
            _prs(args.prs), batch_size=args.batch_size, enabled=not args.off
        )
        print(json.dumps({"batches": batches}))
    elif args.subcommand == "bisect":
        red = set(_prs(args.red))
        culprits = find_culprits(
            _prs(args.batch), lambda subset: any(pr in red for pr in subset)
        )
        print(json.dumps({"culprits": culprits}))
    elif args.subcommand == "log":
        append_batch_record(
            Path(args.file),
            args.batch,
            _prs(args.prs),
            args.command,
            args.result,
            probe=args.probe,
        )
    elif args.subcommand == "status":
        print(json.dumps(gate_status(read_log(Path(args.file)))))
    return 0


if __name__ == "__main__":
    sys.exit(main())
