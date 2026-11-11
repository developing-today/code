#!/usr/bin/env python3
"""test_drain_wrapper.py — regression tests for #5222.

systemd/drain-wrapper.sh loads the Worker URL + bearer from Bitwarden
and execs the drain. Its credential-loading branches (rewritten by
PRs #3222/#3226) had no coverage. This drives a COPY of the real
wrapper inside a temp tree with a fake HOME, a stub `bws` on PATH and a
stub drain target (bin/drain_queue.py) — no real Bitwarden, no network:

  1. BWS_ACCESS_TOKEN already set     -> token file NOT read, bws-env.sh
                                         NOT sourced, drain proceeds
  2. token file present, bare bws ok  -> token exported from the file,
                                         bws-env.sh NOT sourced, drain
                                         proceeds
  3. bare bws fails + bws-env.sh at   -> sourced (each candidate path),
     ~/tools or ~/hatch/tools            credentials load, drain proceeds
  4. no working credential path       -> non-zero exit with an error
                                         naming the credential problem,
                                         drain NOT invoked

Run: python3 infra/github-backup/tests/test_drain_wrapper.py
"""

import os, shutil, stat, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
WRAPPER = os.path.abspath(
    os.path.join(HERE, "..", "systemd", "drain-wrapper.sh")
)

PASS = []


def check(name, ok):
    PASS.append((name, ok))
    print(("PASS " if ok else "FAIL ") + name)


# `bws secret list` succeeds only when BWS_ACCESS_TOKEN is non-empty
# (what a real bws needs); `secret get <id>` returns a canned value.
BWS_STUB = """#!/bin/sh
if [ -z "${BWS_ACCESS_TOKEN:-}" ]; then
  echo "stub bws: no access token" >&2
  exit 1
fi
case "$1 $2" in
  "secret list")
    echo '[{"key":"github-app.backup-worker-url","id":"id-url"},{"key":"github-app.backup-worker-bearer","id":"id-bearer"}]'
    ;;
  "secret get")
    case "$3" in
      id-url) echo '{"value":"https://worker.invalid"}' ;;
      id-bearer) echo '{"value":"stub-bearer"}' ;;
    esac
    ;;
esac
"""

# The stub drain target records the credentials it received.
DRAIN_STUB = """import os
with open(os.environ["DRAIN_LOG"], "w") as f:
    f.write("url=%s\\nbearer=%s\\ntoken=%s\\n" % (
        os.environ.get("GHB_WORKER_URL", ""),
        os.environ.get("GHB_WORKER_BEARER", ""),
        os.environ.get("BWS_ACCESS_TOKEN", ""),
    ))
"""


def write_exec(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)


def run_case(
    token_env=None, token_file=None, env_script=None, env_script_dirs=()
):
    """Run the wrapper in a fresh sandbox; return (rc, stderr, drain, markers)."""
    root = tempfile.mkdtemp(prefix="drain-wrapper-test-")
    try:
        home = os.path.join(root, "home")
        os.makedirs(home)
        tree = os.path.join(root, "tree")
        wrapper = os.path.join(tree, "systemd", "drain-wrapper.sh")
        os.makedirs(os.path.dirname(wrapper))
        shutil.copy(WRAPPER, wrapper)
        os.makedirs(os.path.join(tree, "bin"))
        with open(os.path.join(tree, "bin", "drain_queue.py"), "w") as f:
            f.write(DRAIN_STUB)
        stubs = os.path.join(root, "stubs")
        write_exec(os.path.join(stubs, "bws"), BWS_STUB)
        if token_file is not None:
            p = os.path.join(home, ".config", "bws", "access-token")
            os.makedirs(os.path.dirname(p))
            with open(p, "w") as f:
                f.write(token_file)
        marker = os.path.join(root, "sourced")
        for d in env_script_dirs:
            write_exec(
                os.path.join(home, d, "bws-env.sh"),
                env_script.replace("@MARKER@", marker),
            )
        drain_log = os.path.join(root, "drain.log")
        env = {
            "PATH": stubs + os.pathsep + os.environ.get("PATH", ""),
            "HOME": home,
            "DRAIN_LOG": drain_log,
        }
        if token_env is not None:
            env["BWS_ACCESS_TOKEN"] = token_env
        r = subprocess.run(
            ["bash", wrapper],
            env=env,
            capture_output=True,
            text=True,
            timeout=60,
        )
        drain = None
        if os.path.exists(drain_log):
            with open(drain_log) as f:
                drain = dict(l.split("=", 1) for l in f.read().splitlines())
        return r.returncode, r.stderr, drain, os.path.exists(marker)
    finally:
        shutil.rmtree(root, ignore_errors=True)


ENV_SCRIPT = 'echo sourced > "@MARKER@"\nexport BWS_ACCESS_TOKEN=fallback-token\n'
BOTH_DIRS = ("tools", os.path.join("hatch", "tools"))

# 1. token already in the environment: no file read, no fallback.
rc, err, drain, sourced = run_case(
    token_env="env-token",
    token_file="file-token",
    env_script=ENV_SCRIPT,
    env_script_dirs=BOTH_DIRS,
)
check("1: pre-set BWS_ACCESS_TOKEN -> exit 0", rc == 0)
check("1: drain proceeds with credentials", drain is not None
      and drain["url"] == "https://worker.invalid"
      and drain["bearer"] == "stub-bearer")
check("1: environment token kept (token file not exported)",
      drain is not None and drain["token"] == "env-token")
check("1: bws-env.sh not sourced", not sourced)

# 2. token file present: exported, bare bws succeeds, no fallback.
rc, err, drain, sourced = run_case(
    token_file="file-token\n",
    env_script=ENV_SCRIPT,
    env_script_dirs=BOTH_DIRS,
)
check("2: token file -> exit 0", rc == 0)
check("2: drain proceeds with credentials", drain is not None
      and drain["url"] == "https://worker.invalid"
      and drain["bearer"] == "stub-bearer")
check("2: token exported from ~/.config/bws/access-token",
      drain is not None and drain["token"] == "file-token")
check("2: bws-env.sh not sourced", not sourced)

# 3. bare bws fails; bws-env.sh at each candidate path is sourced.
for d in BOTH_DIRS:
    rc, err, drain, sourced = run_case(
        env_script=ENV_SCRIPT, env_script_dirs=(d,)
    )
    check("3: fallback at ~/%s/bws-env.sh -> exit 0" % d, rc == 0)
    check("3: fallback at ~/%s/bws-env.sh sourced" % d, sourced)
    check("3: fallback at ~/%s/bws-env.sh -> drain proceeds" % d,
          drain is not None
          and drain["url"] == "https://worker.invalid"
          and drain["bearer"] == "stub-bearer"
          and drain["token"] == "fallback-token")

# 3b. both candidate paths present: both are sourced, drain proceeds.
rc, err, drain, sourced = run_case(
    env_script=ENV_SCRIPT, env_script_dirs=BOTH_DIRS
)
check("3b: both candidates present -> exit 0 and drain proceeds",
      rc == 0 and drain is not None and drain["token"] == "fallback-token")

# 4. no working credential path: fail before the drain, with a message.
rc, err, drain, sourced = run_case()
check("4: no credential path -> non-zero exit", rc != 0)
check("4: drain target not invoked", drain is None)
check("4: error is comprehensible (names the credential problem)",
      "bws" in err.lower() or "bitwarden" in err.lower()
      or "credential" in err.lower())
check("4: no raw Python traceback", "Traceback" not in err and "KeyError" not in err)

# 4b. a bws-env.sh that exists but yields no token is not a working path.
rc, err, drain, sourced = run_case(
    env_script='echo sourced > "@MARKER@"\n', env_script_dirs=("tools",)
)
check("4b: fallback sourced but no token -> non-zero exit", sourced and rc != 0)
check("4b: drain target not invoked", drain is None)

failed = [n for n, ok in PASS if not ok]
print("\n%d checks, %d failed" % (len(PASS), len(failed)))
sys.exit(1 if failed else 0)
