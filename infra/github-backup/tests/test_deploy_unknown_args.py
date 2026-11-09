#!/usr/bin/env python3
"""test_deploy_unknown_args.py — regression tests for #3287.

deploy.mjs only membership-checked --dry-run / --deploy-version and
never rejected an argument it did not recognize, so `deploy.mjs
--dry-rnu` (or any unknown flag) silently ran the real deploy sequence
(versions upload, deployments list, secret put x2) with exit 0. The fix
accounts for every argument before any bws/wrangler/git subprocess or
deploy side effect: the only accepted invocations are bare,
--dry-run, and --deploy-version <id>; anything else aborts with a
usage error naming the offending argument (exit 2).

Drives the real deploy.mjs (node) with stubbed bws/wrangler/git
executables on PATH that log every invocation:

  unknown / typo'd / positional / duplicate / trailing args,
  missing or flag-like --deploy-version values, and the
  --dry-run + --deploy-version combination
    -> exit 2, error names the offender, NO stub call at all
  --dry-run                    -> exit 0, only wrangler deploy --dry-run
  --deploy-version <id>        -> exit 0, versions deploy <id>@100%
  no arguments                 -> reaches the upload path (versions
                                  upload invoked; stateful stub)

Run: python3 infra/github-backup/tests/test_deploy_unknown_args.py
"""

import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
DEPLOY = os.path.abspath(os.path.join(HERE, "..", "worker", "deploy.mjs"))

PASS = []


def check(name, ok):
    PASS.append((name, ok))
    print(("PASS " if ok else "FAIL ") + name)


SIMPLE_STUB = """#!/bin/sh
echo "$(basename "$0") $@" >> "$STUB_LOG"
case "$(basename "$0")" in
  bws) echo '[{"key":"cloudflare.api-token-day-to-day","value":"stub-token"}]';;
  wrangler) case "$1 $2" in
              "versions view") echo '{"resources":{"bindings":[{"name":"DRAIN_BEARER","type":"secret_text"},{"name":"GITHUB_WEBHOOK_SECRET","type":"secret_text"}]}}';;
              *) echo '[]';;
            esac;;
  git) echo '0123456789abcdef0123456789abcdef01234567';;
esac
exit 0
"""

BWS_STUB = """#!/bin/sh
echo "bws $@" >> "$STUB_LOG"
echo '[{"key":"cloudflare.api-token-day-to-day","value":"stub-token"},\
{"key":"github-app.backup-worker-bearer","value":"stub-bearer-value"},\
{"key":"github-app.backup-webhook-secret","value":"stub-webhook-value"}]'
exit 0
"""

WRANGLER_STATEFUL_STUB = """#!/bin/sh
echo "wrangler $@" >> "$STUB_LOG"
case "$1 $2" in
  "versions list")
    echo '[{"id":"uploaded-v1","annotations":{"workers/tag":"git-0123456789ab"},"metadata":{"created_on":"2026-01-01T00:00:00Z"}}]';;
  "versions deploy")
    echo "$3" | cut -d@ -f1 > "$STUB_STATE/live";;
  "versions upload")
    echo uploaded;;
  "deployments list")
    live=$(cat "$STUB_STATE/live" 2>/dev/null)
    if [ -n "$live" ]; then
      echo "[{\\"created_on\\":\\"2026-01-02T00:00:00Z\\",\\"versions\\":[{\\"version_id\\":\\"$live\\",\\"percentage\\":100}]}]"
    else
      echo '[]'
    fi;;
  "secret put")
    cat > /dev/null
    echo "configured-final" > "$STUB_STATE/live";;
  "versions view")
    echo '{"resources":{"bindings":[{"name":"DRAIN_BEARER","type":"secret_text"},{"name":"GITHUB_WEBHOOK_SECRET","type":"secret_text"}]}}';;
esac
exit 0
"""

GIT_STUB = """#!/bin/sh
echo "git $@" >> "$STUB_LOG"
echo '0123456789abcdef0123456789abcdef01234567'
exit 0
"""


def run_simple(args):
    root = tempfile.mkdtemp(prefix="ghb-deploy-unknown-test-")
    stub_bin = os.path.join(root, "bin")
    os.makedirs(stub_bin)
    for name in ("bws", "wrangler", "git"):
        path = os.path.join(stub_bin, name)
        with open(path, "w") as f:
            f.write(SIMPLE_STUB)
        os.chmod(path, 0o755)
    log = os.path.join(root, "stub.log")
    env = {
        **os.environ,
        "PATH": stub_bin + os.pathsep + os.environ["PATH"],
        "CLOUDFLARE_ACCOUNT_ID": "stub-account",
        "STUB_LOG": log,
    }
    proc = subprocess.run(
        ["node", DEPLOY, *args], env=env, capture_output=True, text=True, timeout=60
    )
    calls = ""
    if os.path.exists(log):
        with open(log) as f:
            calls = f.read()
    return proc, calls


def run_stateful(args):
    root = tempfile.mkdtemp(prefix="ghb-deploy-unknown-stateful-")
    stub_bin = os.path.join(root, "bin")
    state = os.path.join(root, "state")
    os.makedirs(stub_bin)
    os.makedirs(state)
    for name, body in (
        ("bws", BWS_STUB),
        ("wrangler", WRANGLER_STATEFUL_STUB),
        ("git", GIT_STUB),
    ):
        path = os.path.join(stub_bin, name)
        with open(path, "w") as f:
            f.write(body)
        os.chmod(path, 0o755)
    log = os.path.join(root, "stub.log")
    env = {
        **os.environ,
        "PATH": stub_bin + os.pathsep + os.environ["PATH"],
        "CLOUDFLARE_ACCOUNT_ID": "stub-account",
        "STUB_LOG": log,
        "STUB_STATE": state,
    }
    proc = subprocess.run(
        ["node", DEPLOY, *args], env=env, capture_output=True, text=True, timeout=60
    )
    calls = ""
    if os.path.exists(log):
        with open(log) as f:
            calls = f.read()
    return proc, calls


# 1. typo'd / unknown flags are usage errors before any side effect.
for typo in ("--dry-rnu", "--bogus-flag", "--dryrun", "--deploy-versoin"):
    proc, calls = run_simple([typo])
    check(f"{typo}: exit 2", proc.returncode == 2)
    check(f"{typo}: error names the argument", typo in proc.stderr)
    check(f"{typo}: usage printed", "usage:" in proc.stderr)
    check(f"{typo}: no stub call at all", calls == "")

# 2. stray positionals and trailing args are rejected the same way.
for args in (["some-positional"], ["--dry-run", "extra"], ["--deploy-version", "v1", "extra"]):
    proc, calls = run_simple(args)
    label = "/".join(args)
    check(f"{label}: exit 2", proc.returncode == 2)
    check(f"{label}: error names the offender", args[-1] in proc.stderr)
    check(f"{label}: no stub call at all", calls == "")

# 3. missing, empty, or flag-like --deploy-version values are rejected.
proc, calls = run_simple(["--deploy-version"])
check("missing value: exit 2", proc.returncode == 2)
check("missing value: error names --deploy-version", "--deploy-version" in proc.stderr)
check("missing value: no stub call at all", calls == "")

proc, calls = run_simple(["--deploy-version", ""])
check("empty value: exit 2", proc.returncode == 2)
check("empty value: error names --deploy-version", "--deploy-version" in proc.stderr)
check("empty value: no stub call at all", calls == "")

for flaglike in ("--dry-run", "--bogus-flag"):
    proc, calls = run_simple(["--deploy-version", flaglike])
    check(f"flag-like {flaglike}: exit 2", proc.returncode == 2)
    check(f"flag-like {flaglike}: error names the value", flaglike in proc.stderr)
    check(f"flag-like {flaglike}: no stub call at all", calls == "")

# 4. duplicates and the dry-run/deploy-version combination are rejected.
for args in (["--dry-run", "--dry-run"], ["--deploy-version", "v1", "--deploy-version", "v2"]):
    proc, calls = run_simple(args)
    check(f"{'/'.join(args)}: non-zero exit", proc.returncode != 0)
    check(f"{'/'.join(args)}: no stub call at all", calls == "")

proc, calls = run_simple(["--dry-run", "--deploy-version", "v1"])
check("combo: non-zero exit", proc.returncode != 0)
check(
    "combo: error names both flags",
    "--dry-run" in proc.stderr and "--deploy-version" in proc.stderr,
)
check("combo: no wrangler invocation", "wrangler" not in calls)
check("combo: no bws invocation", "bws" not in calls)

# 5. valid invocations are unchanged.
proc, calls = run_simple(["--dry-run"])
check("dry-run: exit 0", proc.returncode == 0)
check("dry-run: wrangler deploy --dry-run invoked", "wrangler deploy --dry-run" in calls)
check("dry-run: no versions deploy", "versions deploy" not in calls)

proc, calls = run_simple(["--deploy-version", "version-stub"])
check("rollback: exit 0", proc.returncode == 0)
check(
    "rollback: versions deploy at 100%",
    "wrangler versions deploy version-stub@100%" in calls,
)

proc, calls = run_stateful([])
check("bare: exit 0", proc.returncode == 0)
check("bare: versions upload invoked", "wrangler versions upload" in calls)
check("bare: no usage error", "unrecognized argument" not in proc.stderr)

print(f"\n{sum(1 for _, ok in PASS if ok)}/{len(PASS)} checks passed")
sys.exit(0 if all(ok for _, ok in PASS) else 1)
