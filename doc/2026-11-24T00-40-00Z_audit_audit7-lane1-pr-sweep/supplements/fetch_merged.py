#!/usr/bin/env python3
"""Companion to fetch_full.py: pullRequests(states:[OPEN,CLOSED]) excludes MERGED
PRs in GitHub GraphQL (state is its own enum value), so the merged census PRs are
refetched here with states:[OPEN,CLOSED,MERGED] and unioned into the same JSON."""

import json, subprocess, os

env = dict(os.environ)
env["PATH"] = (
    os.path.expanduser("~/.local/share/hatch-recovery/bin") + ":" + env["PATH"]
)
HERE = os.path.dirname(os.path.abspath(__file__))
Q = os.path.join(HERE, "q3-full.graphql")
q = open(Q).read().replace("states:[OPEN,CLOSED]", "states:[OPEN,CLOSED,MERGED]")
open(os.path.join(HERE, "q3-full-all-states.graphql"), "w").write(q)


def gql(owner, name, cursor=None):
    args = [
        "fleet",
        "gh",
        "api",
        "graphql",
        "-F",
        f"query=@{os.path.join(HERE, 'q3-full-all-states.graphql')}",
        "-f",
        f"owner={owner}",
        "-f",
        f"name={name}",
    ]
    if cursor:
        args += ["-f", f"cursor={cursor}"]
    for attempt in range(4):
        r = subprocess.run(args, capture_output=True, text=True, env=env)
        if r.returncode == 0 and r.stdout.strip():
            return json.loads(r.stdout)["data"]["repository"]["pullRequests"]
    raise SystemExit(f"FAIL {owner}/{name}: {r.stderr[:200]}")


path = os.path.join(HERE, "pr-full-records-refetch.json")
out = json.load(open(path))
for repo, owner, name, cap in [
    ("code", "developing-today", "code", 500),
    ("opencode", "developing-today-forks", "opencode", 250),
    ("openchamber", "developing-today-forks", "openchamber", 150),
    ("fleet", "dezren39", "fleet", 50),
]:
    cur = None
    alln = []
    while True:
        d = gql(owner, name, cur)
        alln += d["nodes"]
        if not d["pageInfo"]["hasNextPage"] or len(alln) >= cap:
            break
        cur = d["pageInfo"]["endCursor"]
    by = {p["number"]: p for p in out[repo]}
    for p in alln:
        by[p["number"]] = p
    out[repo] = sorted(by.values(), key=lambda p: p["number"], reverse=True)
    print(
        repo,
        "union",
        len(out[repo]),
        "merged in window",
        sum(1 for p in alln if p["merged"]),
        flush=True,
    )
json.dump(out, open(path, "w"), indent=1)
print("saved")
