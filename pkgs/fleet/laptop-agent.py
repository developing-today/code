#!/usr/bin/env python3
"""Strict Python-client compatibility selector (code#16088).

CLI: health | jobs | kill JOB | exec-batch FILE.json |
     exec|run [--cwd DIR] [--timeout S] [--shell] -- COMMAND... |
     poll JOB [--wait S] | fetch [--chunk BYTES] PATH OUT
All arguments are validated before any subprocess or side effect (#3352).
Transport implementation remains in laptop-agent-legacy.py until the selected
fleet binary explicitly advertises the versioned Python CLI contract.
"""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import selectors
import shutil
import signal
import subprocess
import sys
import time

CONTRACT = "laptop-agent-python-cli/v1"
PROBE_TIMEOUT_S = 2.0
PROBE_MAX_BYTES = 65536
_legacy = None


def _load_legacy():
    global _legacy
    if _legacy is None:
        spec = importlib.util.spec_from_file_location(
            "laptop_agent_legacy", Path(__file__).with_name("laptop-agent-legacy.py")
        )
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        _legacy = module
    return _legacy


def _parser(name):
    return argparse.ArgumentParser(prog=f"laptop-agent.py {name}".rstrip(), allow_abbrev=False)


def validate(argv):
    """Strict full CLI declaration; return legacy-compatible spelling."""
    root = _parser("")
    verbs = ("health", "jobs", "kill", "exec-batch", "exec", "run", "poll", "fetch")
    if not argv or argv[0] in ("-h", "--help"):
        root.add_argument("command", choices=verbs)
        root.parse_args(argv)
    sub, rest = argv[0], argv[1:]
    if sub not in verbs:
        root.error(f"unknown command {sub!r}")
    ap = _parser(sub)
    if sub in ("exec", "run"):
        ap.add_argument("--cwd")
        ap.add_argument("--timeout", type=int, default=120)
        ap.add_argument("--shell", action="store_true")
        i = rest.index("--") if "--" in rest else len(rest)
        opts = ap.parse_args(rest[:i])
        if i == len(rest) or not rest[i + 1:]:
            ap.error("no command given (use -- cmd...)")
        if opts.timeout <= 0:
            ap.error("--timeout must be positive")
    elif sub == "fetch":
        ap.add_argument("--chunk", type=int, default=1 << 20)
        ap.add_argument("path")
        ap.add_argument("out")
        opts = ap.parse_args(rest)
        if not 1 <= opts.chunk <= 1 << 20:
            ap.error("--chunk must be 1..1048576")
        # Legacy fetch/poll parse manually: normalize --option=value too.
        return [sub, "--chunk", str(opts.chunk), opts.path, opts.out]
    elif sub == "poll":
        ap.add_argument("job")
        ap.add_argument("--wait", type=int, default=0)
        opts = ap.parse_args(rest)
        if opts.wait < 0:
            ap.error("--wait must be nonnegative")
        return [sub, opts.job, "--wait", str(opts.wait)]
    else:
        if sub == "kill":
            ap.add_argument("job")
        elif sub == "exec-batch":
            ap.add_argument("file")
        ap.parse_args(rest)
    return list(argv)


def compatible(document):
    """Availability alone, version strings, and unknown contracts prove nothing."""
    return (
        isinstance(document, dict)
        and isinstance(document.get("compatibility_contracts"), list)
        and CONTRACT in document["compatibility_contracts"]
        and isinstance(document.get("domains"), list)
        and any(
            isinstance(d, dict)
            and d.get("name") == "laptop-agent"
            and d.get("available") is True
            and d.get("state") == "available"
            for d in document["domains"]
        )
    )


def probe(binary):
    """Fresh, local-only capability probe; bounded time AND output, never cached."""
    proc = None
    try:
        proc = subprocess.Popen(
            [binary, "capabilities"], stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, start_new_session=True,
        )
        deadline = time.monotonic() + PROBE_TIMEOUT_S
        output = bytearray()
        with selectors.DefaultSelector() as ready:
            ready.register(proc.stdout, selectors.EVENT_READ)
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not ready.select(remaining):
                    return False
                chunk = os.read(proc.stdout.fileno(), min(4096, PROBE_MAX_BYTES + 1 - len(output)))
                if not chunk:
                    break
                output.extend(chunk)
                if len(output) > PROBE_MAX_BYTES:
                    return False
        remaining = deadline - time.monotonic()
        if remaining <= 0 or proc.wait(timeout=remaining) != 0:
            return False
        return compatible(json.loads(output))
    except (OSError, ValueError, subprocess.TimeoutExpired):
        return False
    finally:
        if proc is not None:
            # Also reap descendants that inherited stdout on timeout/oversize.
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            proc.wait()
            proc.stdout.close()


def select_main():
    argv = sys.argv[1:]
    legacy_argv = validate(argv)
    binary = shutil.which("fleet")
    if binary:
        binary = os.path.abspath(binary)
    if binary and probe(binary):
        try:
            # Process replacement preserves stdin/stdout/stderr, env, signals,
            # argv boundaries and exit status. NEVER replay after this point.
            os.execv(binary, [binary, "laptop-agent", *argv])
        except OSError as exc:
            print(f"laptop-agent: cannot exec selected fleet: {exc}", file=sys.stderr)
            raise SystemExit(126)
    sys.argv[1:] = legacy_argv
    _load_legacy().main()


if __name__ == "__main__":
    select_main()
else:
    # Historical import API, including assignments to module globals used by
    # transport functions. A __getattr__ proxy would split those globals.
    # Execute only the frozen implementation, never the selector/probe, for
    # broker imports (including importlib callers not entered in sys.modules).
    _legacy_path = Path(__file__).with_name("laptop-agent-legacy.py")
    exec(compile(_legacy_path.read_bytes(), str(_legacy_path), "exec"), globals())
