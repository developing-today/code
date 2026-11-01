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
the final configured version; and on the --deploy-version path,
re-provision the secrets against the just-deployed version (secret put
derives from the deployed code, preserving the rollback target's code)
and report the configured live version.

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
    for name, body in (("bws", BWS_STUB), ("wrangler", WRANGLER_STUB), ("git", GIT_STUB)):
        path = os.path.join(stub_bin, name)
        with open(path, "w") as f:
            f.write(body)
        os.chmod(path, 0o755)
    log = os.path.join(root, "stub.log")
    env = {**os.environ,
           "PATH": stub_bin + os.pathsep + os.environ["PATH"],
           "CLOUDFLARE_ACCOUNT_ID": "stub-account",
           "STUB_LOG": log,
           "STUB_STATE": state}
    proc = subprocess.run(["node", DEPLOY, *args], env=env,
                          capture_output=True, text=True, timeout=60)
    calls = ""
    if os.path.exists(log):
        with open(log) as f:
            calls = f.read()
    return proc, calls

# 1. normal deploy advertises the final configured version
proc, calls = run_deploy([])
check("normal: exit 0", proc.returncode == 0)
check("normal: versions deploy of the upload happened",
      "wrangler versions deploy uploaded-v1@100%" in calls)
check("normal: both Worker secrets provisioned",
      "wrangler secret put DRAIN_BEARER" in calls and
      "wrangler secret put GITHUB_WEBHOOK_SECRET" in calls)
check("normal: advertised live version is the configured version",
      "at version configured-final" in proc.stdout)
live_lines = [ln for ln in proc.stdout.splitlines() if ln.startswith("live:")]
check("normal: pre-secrets upload is NOT advertised as live",
      live_lines and all("uploaded-v1" not in ln for ln in live_lines))
check("normal: no mock secret value in output",
      "stub-bearer-value" not in proc.stdout + proc.stderr and
      "stub-webhook-value" not in proc.stdout + proc.stderr)

# 2. rollback re-provisions runtime configuration, in order
proc, calls = run_deploy(["--deploy-version", "old-v9"])
check("rollback: exit 0", proc.returncode == 0)
check("rollback: versions deploy at 100%",
      "wrangler versions deploy old-v9@100%" in calls)
deploy_at = calls.find("wrangler versions deploy old-v9@100%")
secret_at = calls.find("wrangler secret put")
check("rollback: secrets provisioned after the deployment",
      deploy_at >= 0 and secret_at > deploy_at)
check("rollback: both Worker secrets provisioned",
      "wrangler secret put DRAIN_BEARER" in calls and
      "wrangler secret put GITHUB_WEBHOOK_SECRET" in calls)
check("rollback: advertised live version is the configured version",
      "at version configured-final" in proc.stdout)
check("rollback: no mock secret value in output",
      "stub-bearer-value" not in proc.stdout + proc.stderr and
      "stub-webhook-value" not in proc.stdout + proc.stderr)

print(f"\n{sum(1 for _, ok in PASS if ok)}/{len(PASS)} checks passed")
sys.exit(0 if all(ok for _, ok in PASS) else 1)
