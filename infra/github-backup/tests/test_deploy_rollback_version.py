#!/usr/bin/env python3
"""test_deploy_rollback_version.py — regression tests for #3475.

deploy.mjs's normal flow deployed the uploaded version, then set the
Worker secrets — and `wrangler secret put` creates and deploys a NEW
version carrying the bindings, so the version the script advertised
("live: ... at version <uploaded id>") was the PRE-secrets version:
rolling back to it would restore a Worker without the runtime
configuration its authenticated endpoints need. The --deploy-version
(rollback) path never provisioned secrets at all.

The fix: after provisioning, re-read the live deployment and advertise
the final configured version. On the --deploy-version path the
configuration cannot be added after the fact (ordinary `secret put`
hard-errors when the latest version is not deployed — the rollback
state by definition — and `versions secret put` patches the latest
version, not the selected one), so the path verifies BEFORE deploying:
a target missing the required secret bindings is refused with the
roll-forward remedy named; a configured target deploys as-is.

Drives the real deploy.mjs (node) with stubbed bws/wrangler/git
executables on PATH. The wrangler stub is stateful: `versions deploy`
records the live version, each `secret put` replaces it with
"configured-final" (modelling the new version secret put creates),
and `deployments list --json` reports the recorded live version.
Mock secret values are stubs; a check asserts they never reach stdout
or stderr.

Run: python3 infra/github-backup/tests/test_deploy_rollback_version.py
"""

import os, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
DEPLOY = os.path.abspath(os.path.join(HERE, "..", "worker", "deploy.mjs"))

PASS = []


def check(name, ok):
    PASS.append((name, ok))
    print(("PASS " if ok else "FAIL ") + name)


BWS_STUB = """#!/bin/sh
echo "bws $@" >> "$STUB_LOG"
echo '[{"key":"cloudflare.api-token-day-to-day","value":"stub-token"},\
{"key":"github-app.backup-worker-bearer","value":"stub-bearer-value"},\
{"key":"github-app.backup-webhook-secret","value":"stub-webhook-value"}]'
exit 0
"""

WRANGLER_STUB = """#!/bin/sh
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
    if [ "$3" = "old-v9" ]; then
      echo '{"resources":{"bindings":[{"name":"DRAIN_BEARER","type":"secret_text"},{"name":"GITHUB_WEBHOOK_SECRET","type":"secret_text"}]}}'
    else
      echo '{"resources":{"bindings":[]}}'
    fi;;
esac
exit 0
"""

GIT_STUB = """#!/bin/sh
echo "git $@" >> "$STUB_LOG"
echo '0123456789abcdef0123456789abcdef01234567'
exit 0
"""


def run_deploy(args):
    root = tempfile.mkdtemp(prefix="ghb-deploy-rollback-test-")
    stub_bin = os.path.join(root, "bin")
    state = os.path.join(root, "state")
    os.makedirs(stub_bin)
    os.makedirs(state)
    for name, body in (
        ("bws", BWS_STUB),
        ("wrangler", WRANGLER_STUB),
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


# 1. normal deploy advertises the final configured version
proc, calls = run_deploy([])
check("normal: exit 0", proc.returncode == 0)
check(
    "normal: versions deploy of the upload happened",
    "wrangler versions deploy uploaded-v1@100%" in calls,
)
check(
    "normal: both Worker secrets provisioned",
    "wrangler secret put DRAIN_BEARER" in calls
    and "wrangler secret put GITHUB_WEBHOOK_SECRET" in calls,
)
check(
    "normal: advertised live version is the configured version",
    "at version configured-final" in proc.stdout,
)
live_lines = [ln for ln in proc.stdout.splitlines() if ln.startswith("live:")]
check(
    "normal: pre-secrets upload is NOT advertised as live",
    live_lines and all("uploaded-v1" not in ln for ln in live_lines),
)
check(
    "normal: no mock secret value in output",
    "stub-bearer-value" not in proc.stdout + proc.stderr
    and "stub-webhook-value" not in proc.stdout + proc.stderr,
)

# 2. rollback of a CONFIGURED version deploys as-is, verified first
proc, calls = run_deploy(["--deploy-version", "old-v9"])
check("rollback: exit 0", proc.returncode == 0)
check(
    "rollback: bindings verified before deploying",
    "wrangler versions view old-v9" in calls
    and calls.find("wrangler versions view old-v9")
    < calls.find("wrangler versions deploy"),
)
check(
    "rollback: versions deploy at 100%", "wrangler versions deploy old-v9@100%" in calls
)
check(
    "rollback: no secret put (a configured version needs none, and "
    "secret put would hard-error in the rollback state)",
    "wrangler secret put" not in calls,
)
check(
    "rollback: advertised live version is the target, verified",
    "at version old-v9" in proc.stdout and "verified" in proc.stdout,
)
check(
    "rollback: no mock secret value in output",
    "stub-bearer-value" not in proc.stdout + proc.stderr
    and "stub-webhook-value" not in proc.stdout + proc.stderr,
)

# 3. rollback of an UNCONFIGURED version is refused before any deploy
proc, calls = run_deploy(["--deploy-version", "bare-v1"])
check("unconfigured: non-zero exit", proc.returncode != 0)
check(
    "unconfigured: error names the missing bindings and the refusal",
    "DRAIN_BEARER" in proc.stderr and "refusing to roll back" in proc.stderr,
)
check("unconfigured: no versions deploy happened", "versions deploy" not in calls)
check("unconfigured: no secret put attempted", "wrangler secret put" not in calls)

print(f"\n{sum(1 for _, ok in PASS if ok)}/{len(PASS)} checks passed")
sys.exit(0 if all(ok for _, ok in PASS) else 1)
