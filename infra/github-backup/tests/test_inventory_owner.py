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
  malformed cursor         -> bounded failure with owner-level error

Run: python3 infra/github-backup/tests/test_inventory_owner.py
"""

import json, os, sys

BIN = os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "bin")
)
sys.path.insert(0, BIN)

import ghb_common


class R:
    def __init__(self, returncode=0, stdout="", stderr=""):
        self.returncode, self.stdout, self.stderr = returncode, stdout, stderr


def node(name):
    return {
        "nameWithOwner": name,
        "isFork": False,
        "isArchived": False,
        "diskUsage": 42,
        "pushedAt": "2026-10-31T00:00:00Z",
        "defaultBranchRef": {"name": "main", "target": {"oid": "abc123"}},
        "parent": None,
    }


def payload(nodes, has_next=False, cursor=None):
    return json.dumps(
        {
            "data": {
                "repositoryOwner": {
                    "repositories": {
                        "pageInfo": {"hasNextPage": has_next, "endCursor": cursor},
                        "nodes": nodes,
                    }
                }
            }
        }
    )


CASES = {
    "visible": payload([node("some-owner/repo1")]),
    "null-owner": json.dumps({"data": {"repositoryOwner": None}}),
    "null-repositories": json.dumps(
        {"data": {"repositoryOwner": {"repositories": None}}}
    ),
    "gql-errors": json.dumps(
        {
            "data": {"repositoryOwner": None},
            "errors": [{"message": "Something went wrong"}],
        }
    ),
    "empty-visible": payload([]),
}


def stub_run_factory(case):
    def stub(cmd, **kw):
        if case == "gh-failure":
            return R(1, "", "boom")
        if case == "pagination":
            if any(a.startswith("after=") for a in cmd):
                return R(0, payload([node("some-owner/repo2")]))
            return R(
                0, payload([node("some-owner/repo1")], has_next=True, cursor="cur1")
            )
        return R(0, CASES[case])

    return stub


def malformed_cursor_stub_factory(case):
    calls = []
    expected_calls = 2 if case == "repeated-cursor" else 1

    def stub(cmd, **kw):
        calls.append(cmd)
        if len(calls) > expected_calls:
            return R(1, "", "transport call bound exceeded")
        if case == "null-cursor":
            return R(0, payload([node("some-owner/repo1")], True, None))
        if case == "empty-cursor":
            return R(0, payload([node("some-owner/repo1")], True, ""))
        if case == "repeated-cursor":
            return R(0, payload([node("some-owner/repo1")], True, "cur1"))
        raise AssertionError(f"unknown malformed cursor case: {case}")

    return stub, calls, expected_calls


PASS = []


def check(name, cond):
    PASS.append((name, bool(cond)))
    print(("PASS " if cond else "FAIL ") + name)
    assert cond, name


saved_run = ghb_common.run
try:
    for case in [
        "visible",
        "null-owner",
        "null-repositories",
        "gql-errors",
        "gh-failure",
        "empty-visible",
        "pagination",
    ]:
        ghb_common.run = stub_run_factory(case)
        repos, errors = ghb_common.inventory(["some-owner"])
        if case == "visible":
            check(
                "visible: repos returned",
                [r["repo"] for r in repos.values()] == ["some-owner/repo1"],
            )
            check("visible: no error", errors == {})
        elif case == "empty-visible":
            check("empty-visible: zero repos", repos == {})
            check("empty-visible: NO error (legit empty owner)", errors == {})
        elif case == "pagination":
            check(
                "pagination: both pages combined",
                sorted(repos) == ["some-owner/repo1", "some-owner/repo2"],
            )
            check("pagination: no error", errors == {})
        else:
            check(f"{case}: no repos", repos == {})
            check(
                f"{case}: owner-level error recorded",
                "some-owner" in errors and bool(errors["some-owner"]),
            )
            print(f"       error: {errors.get('some-owner', '')[:110]}")
        # inventory_owner itself must raise for the failure shapes
        if case in ("null-owner", "null-repositories", "gql-errors", "gh-failure"):
            try:
                ghb_common.inventory_owner("some-owner")
                raised = False
            except RuntimeError:
                raised = True
            check(f"{case}: inventory_owner raises", raised)
finally:
    ghb_common.run = saved_run

for case, malformed_cursor in [
    ("null-cursor", "None"),
    ("empty-cursor", "''"),
    ("repeated-cursor", "'cur1'"),
]:
    stub, calls, expected_calls = malformed_cursor_stub_factory(case)
    ghb_common.run = stub
    repos, errors = ghb_common.inventory(["some-owner"])
    check(f"{case}: no repos returned for failed owner", repos == {})
    check(
        f"{case}: owner-level cursor error recorded",
        "some-owner" in errors
        and "some-owner" in errors["some-owner"]
        and malformed_cursor in errors["some-owner"],
    )
    check(
        f"{case}: inventory transport calls bounded",
        len(calls) == expected_calls,
    )

    calls.clear()
    try:
        ghb_common.inventory_owner("some-owner")
        raised = False
        message = ""
    except RuntimeError as e:
        raised = True
        message = str(e)
    check(f"{case}: inventory_owner raises RuntimeError", raised)
    check(
        f"{case}: exception names owner and malformed cursor",
        "some-owner" in message and malformed_cursor in message,
    )
    check(
        f"{case}: direct inventory transport calls bounded",
        len(calls) == expected_calls,
    )

ghb_common.run = saved_run


# ------------------------------------------------------------ #6840 shapes
# Oracle: ghb_common.run stubbed with canned GraphQL pages (the same
# oracle as every case above); the expected outcomes are the #6840
# acceptance criteria — a malformed node is named and skipped, never a
# raw TypeError/KeyError that fails the owner's whole inventory, and a
# server inventing a fresh advancing cursor forever hits the hard
# page bound (MAX_INVENTORY_PAGES, shrunk to 3 here) as a named
# owner-level RuntimeError.
def shape_stub_factory(case):
    def stub(cmd, **kw):
        good = node("some-owner/repo1")
        if case == "null-node":
            return R(0, payload([None, good]))
        if case == "missing-field-node":
            bad = node("some-owner/bad")
            del bad["nameWithOwner"]
            return R(0, payload([bad, good]))
        if case == "nonstring-field-node":
            bad = node("some-owner/bad")
            bad["nameWithOwner"] = 42
            return R(0, payload([bad, good]))
        raise AssertionError(f"unknown node shape case: {case}")

    return stub


for case in ["null-node", "missing-field-node", "nonstring-field-node"]:
    ghb_common.run = shape_stub_factory(case)
    repos, errors = ghb_common.inventory(["some-owner"])
    check(
        f"{case}: good repo still inventoried",
        [r["repo"] for r in repos.values()] == ["some-owner/repo1"],
    )
    check(f"{case}: no owner error (node skipped, not fatal)", errors == {})
    ghb_common.run = shape_stub_factory(case)
    direct = ghb_common.inventory_owner("some-owner")
    check(
        f"{case}: inventory_owner returns only the good repo",
        [r["repo"] for r in direct] == ["some-owner/repo1"],
    )

page_calls = []


def advancing_cursor_stub(cmd, **kw):
    page_calls.append(cmd)
    return R(
        0,
        payload(
            [node("some-owner/repo1")],
            has_next=True,
            cursor=f"cur{len(page_calls)}",
        ),
    )


saved_max_pages = ghb_common.MAX_INVENTORY_PAGES
ghb_common.MAX_INVENTORY_PAGES = 3
try:
    ghb_common.run = advancing_cursor_stub
    try:
        ghb_common.inventory_owner("some-owner")
        bound_raised = False
        bound_message = ""
    except RuntimeError as e:
        bound_raised = True
        bound_message = str(e)
    check("page-bound: inventory_owner raises RuntimeError", bound_raised)
    check(
        "page-bound: exception names owner and the bound",
        "some-owner" in bound_message and "3" in bound_message,
    )
    check("page-bound: transport calls stop at the bound", len(page_calls) == 3)
    page_calls.clear()
    repos, errors = ghb_common.inventory(["some-owner"])
    check("page-bound: no repos returned for failed owner", repos == {})
    check(
        "page-bound: owner-level error recorded",
        "some-owner" in errors and bool(errors["some-owner"]),
    )
    check("page-bound: inventory transport calls bounded", len(page_calls) == 3)
finally:
    ghb_common.MAX_INVENTORY_PAGES = saved_max_pages
    ghb_common.run = saved_run

print(f"\n{sum(1 for _, ok in PASS if ok)}/{len(PASS)} checks passed")
