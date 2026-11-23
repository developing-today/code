"""Offline CLI oracle: stable cutoff, independent review cursor, and handled-error recovery."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

CLI = Path(sys.argv[1]).resolve()
REPO = "developing-today/atlas-fixture"
BASE = "2026-01-01T00:00:00Z"
FAKE_GH = """#!/usr/bin/env python3
import json, os, sys
route = sys.argv[2].split("?")[0]
def stamp(n):
    return f"2026-01-01T00:00:0{n}Z"
def record(n, key, thread):
    return {"id":n, "user":{"login":"fixture"}, "body":"fixture",
            "created_at":stamp(n), "submitted_at":stamp(n), "updated_at":stamp(n),
            key:f"https://api.github.com/repos/developing-today/atlas-fixture/issues/{thread}"}
if route.endswith("/issues"):
    owner = "external-fixture" if os.environ.get("FIXTURE_TRANSFER") else "developing-today"
    rows = [{"number":n, "repository_url":f"https://api.github.com/repos/{owner}/atlas-fixture",
             "pull_request":{}, "labels":[]} for n in (1, 2)]
elif route.endswith("/issues/comments"):
    rows = [record(5, "issue_url", 1)]
elif route.endswith("/pulls/comments"):
    rows = [record(3, "pull_request_url", 2)]
elif route.endswith("/pulls/1/reviews"):
    rows = [record(4, "pull_request_url", 1)]
elif route.endswith("/pulls/2/reviews"):
    if os.environ.get("FIXTURE_FAILURE"):
        sys.exit(1)
    rows = [record(2, "pull_request_url", 2)]
else:
    raise SystemExit("unexpected fixture route")
for row in rows:
    print(json.dumps(row))
"""


def exercise(directory):
    gh = directory / "gh"
    gh.write_text(FAKE_GH)
    gh.chmod(0o700)
    state = directory / "state.json"
    output = directory / "records.jsonl"
    initial = json.dumps({"repos": {REPO: BASE}})
    state.write_text(initial)
    output.write_text("")
    env = dict(os.environ, PATH=f"{directory}:{os.environ['PATH']}")
    cmd = [str(CLI), "--repo", REPO, "--state", str(state), "--out", str(output),
           "--observed-date", "2026-01-02"]

    def ids():
        return [json.loads(line)["comment_id"] for line in output.read_text().splitlines()]

    # Explicit targets must fail before creating output or using the fake gh.
    scratch = directory / "forbidden-output"
    for target in ("outside-fixture/name", "dezren39/../repos", "dezren39/%2e%2e",
                   "developing-today/name?query=1", "developing-today-forks/.."):
        invalid = subprocess.run([str(CLI), "--repo", target, "--state", str(state),
                                  "--out", str(scratch)], env=env, capture_output=True)
        assert invalid.returncode == 1 and b"permitted" in invalid.stderr
        assert not scratch.exists()
    # A transfer reflected in the canonical repository URL must fail closed.
    transfer = subprocess.run(cmd, env=dict(env, FIXTURE_TRANSFER="1"), capture_output=True)
    assert transfer.returncode == 1 and b"unexpected GitHub repository" in transfer.stderr
    assert state.read_text() == initial and ids() == []

    # Independent oracle: an unsuccessful last stream leaves the entire prior
    # checkpoint and append unchanged. Successful retry emits every eligible ID.
    failure = subprocess.run(cmd, env=dict(env, FIXTURE_FAILURE="1"), capture_output=True)
    assert failure.returncode == 1 and b"rerun resumes" in failure.stderr
    assert state.read_text() == initial and ids() == []
    subprocess.run(cmd + ["--skip-reviews"], env=env, check=True)
    assert ids() == [5, 3]
    assert json.loads(state.read_text())["reviews"][REPO] == BASE
    subprocess.run(cmd, env=env, check=True)
    assert ids() == [5, 3, 4, 2]
    subprocess.run(cmd, env=env, check=True)
    assert ids() == [5, 3, 4, 2]


if __name__ == "__main__":
    with tempfile.TemporaryDirectory() as tmp:
        exercise(Path(tmp))
