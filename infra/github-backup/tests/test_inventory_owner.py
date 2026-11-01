#!/usr/bin/env python3
"""test_inventory_owner.py — regression tests for #3229.

ghb_common.inventory_owner() must never read a failed inventory as an
empty owner. Drives the real ghb_common with ghb_common.run stubbed to
return canned GraphQL payloads:

  visible owner            -> repos returned, no error
  null repositoryOwner     -> owner-level error (was: silent [])
  null repositories        -> owner-level error (was: silent [])
  top-level GraphQL errors -> owner-level error
  gh CLI failure           -> owner-level error (control, pre-existing)
  empty-but-visible owner  -> zero repos, NO error (legit case)
  pagination               -> both pages combined, no error

Run: python3 infra/github-backup/tests/test_inventory_owner.py
"""
import json, os, sys

BIN = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                   "..", "bin"))
sys.path.insert(0, BIN)

import ghb_common


class R:
    def __init__(self, returncode=0, stdout="", stderr=""):
        self.returncode, self.stdout, self.stderr = returncode, stdout, stderr


def node(name):
    return {"nameWithOwner": name, "isFork": False, "isArchived": False,
            "diskUsage": 42, "pushedAt": "2026-10-31T00:00:00Z",
            "defaultBranchRef": {"name": "main", "target": {"oid": "abc123"}},
            "parent": None}


def payload(nodes, has_next=False, cursor=None):
    return json.dumps({"data": {"repositoryOwner": {"repositories": {
        "pageInfo": {"hasNextPage": has_next, "endCursor": cursor},
        "nodes": nodes}}}})


CASES = {
    "visible": payload([node("some-owner/repo1")]),
    "null-owner": json.dumps({"data": {"repositoryOwner": None}}),
    "null-repositories": json.dumps(
        {"data": {"repositoryOwner": {"repositories": None}}}),
    "gql-errors": json.dumps(
        {"data": {"repositoryOwner": None},
         "errors": [{"message": "Something went wrong"}]}),
    "empty-visible": payload([]),
}


def stub_run_factory(case):
    def stub(cmd, **kw):
        if case == "gh-failure":
            return R(1, "", "boom")
        if case == "pagination":
            if any(a.startswith("after=") for a in cmd):
                return R(0, payload([node("some-owner/repo2")]))
            return R(0, payload([node("some-owner/repo1")],
                                has_next=True, cursor="cur1"))
        return R(0, CASES[case])
    return stub


PASS = []


def check(name, cond):
    PASS.append((name, bool(cond)))
    print(("PASS " if cond else "FAIL ") + name)
    assert cond, name


saved_run = ghb_common.run
try:
    for case in ["visible", "null-owner", "null-repositories", "gql-errors",
                 "gh-failure", "empty-visible", "pagination"]:
        ghb_common.run = stub_run_factory(case)
        repos, errors = ghb_common.inventory(["some-owner"])
        if case == "visible":
            check("visible: repos returned",
                  [r["repo"] for r in repos.values()] == ["some-owner/repo1"])
            check("visible: no error", errors == {})
        elif case == "empty-visible":
            check("empty-visible: zero repos", repos == {})
            check("empty-visible: NO error (legit empty owner)", errors == {})
        elif case == "pagination":
            check("pagination: both pages combined",
                  sorted(repos) == ["some-owner/repo1", "some-owner/repo2"])
            check("pagination: no error", errors == {})
        else:
            check(f"{case}: no repos", repos == {})
            check(f"{case}: owner-level error recorded",
                  "some-owner" in errors and bool(errors["some-owner"]))
            print(f"       error: {errors.get('some-owner', '')[:110]}")
        # inventory_owner itself must raise for the failure shapes
        if case in ("null-owner", "null-repositories", "gql-errors",
                    "gh-failure"):
            try:
                ghb_common.inventory_owner("some-owner")
                raised = False
            except RuntimeError:
                raised = True
            check(f"{case}: inventory_owner raises", raised)
finally:
    ghb_common.run = saved_run

print(f"\n{sum(1 for _, ok in PASS if ok)}/{len(PASS)} checks passed")
