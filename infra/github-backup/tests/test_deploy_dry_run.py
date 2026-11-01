#!/usr/bin/env python3
"""test_deploy_dry_run.py — regression tests for #3474.

deploy.mjs checked --deploy-version before --dry-run, and the
deploy-version branch calls wrangler versions deploy WITHOUT --dry-run,
so `deploy.mjs --dry-run --deploy-version <id>` — the apparent rollback
preview — performed a real 100% deployment. The fix rejects the
combination before any bws/wrangler invocation.

Drives the real deploy.mjs (node) with stubbed bws/wrangler/git
executables on PATH that log every invocation:

  --dry-run --deploy-version -> non-zero exit, error names the
                                combination, NO wrangler (and no bws)
                                invocation at all
  --deploy-version alone     -> exit 0, wrangler versions deploy
                                <id>@100% invoked (rollback path intact)
  --dry-run alone            -> exit 0, wrangler deploy --dry-run
                                invoked, no versions deploy

Run: python3 infra/github-backup/tests/test_deploy_dry_run.py
"""
import os, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
DEPLOY = os.path.abspath(os.path.join(HERE, "..", "worker", "deploy.mjs"))

PASS = []
def check(name, ok):
    PASS.append((name, ok))
    print(("PASS " if ok else "FAIL ") + name)

STUB = """#!/bin/sh
echo "$(basename "$0") $@" >> "$STUB_LOG"
case "$(basename "$0")" in
  bws) echo '[{"key":"cloudflare.api-token-day-to-day","value":"stub-token"}]';;
  wrangler) echo '[]';;
  git) echo '0123456789abcdef0123456789abcdef01234567';;
esac
exit 0
"""

def run_deploy(args):
    root = tempfile.mkdtemp(prefix="ghb-deploy-test-")
    stub_bin = os.path.join(root, "bin")
    os.makedirs(stub_bin)
    for name in ("bws", "wrangler", "git"):
        path = os.path.join(stub_bin, name)
        with open(path, "w") as f:
            f.write(STUB)
        os.chmod(path, 0o755)
    log = os.path.join(root, "stub.log")
    env = {**os.environ,
           "PATH": stub_bin + os.pathsep + os.environ["PATH"],
           "CLOUDFLARE_ACCOUNT_ID": "stub-account",
           "STUB_LOG": log}
    proc = subprocess.run(["node", DEPLOY, *args], env=env,
                          capture_output=True, text=True, timeout=60)
    calls = ""
    if os.path.exists(log):
        with open(log) as f:
            calls = f.read()
    return proc, calls

# 1. the dangerous combination is rejected before anything runs
proc, calls = run_deploy(["--dry-run", "--deploy-version", "version-stub"])
check("combo: non-zero exit", proc.returncode != 0)
check("combo: error names --dry-run and --deploy-version",
      "--dry-run" in proc.stderr and "--deploy-version" in proc.stderr)
check("combo: no wrangler invocation", "wrangler" not in calls)
check("combo: no bws invocation", "bws" not in calls)

# 2. argument order does not matter
proc, calls = run_deploy(["--deploy-version", "version-stub", "--dry-run"])
check("combo-reversed: non-zero exit", proc.returncode != 0)
check("combo-reversed: no wrangler invocation", "wrangler" not in calls)

# 3. the ordinary rollback path is unchanged
proc, calls = run_deploy(["--deploy-version", "version-stub"])
check("rollback: exit 0", proc.returncode == 0)
check("rollback: versions deploy at 100%",
      "wrangler versions deploy version-stub@100%" in calls)

# 4. plain --dry-run still validates without deploying
proc, calls = run_deploy(["--dry-run"])
check("dry-run: exit 0", proc.returncode == 0)
check("dry-run: wrangler deploy --dry-run invoked",
      "wrangler deploy --dry-run" in calls)
check("dry-run: no versions deploy", "versions deploy" not in calls)

print(f"\n{sum(1 for _, ok in PASS if ok)}/{len(PASS)} checks passed")
sys.exit(0 if all(ok for _, ok in PASS) else 1)
