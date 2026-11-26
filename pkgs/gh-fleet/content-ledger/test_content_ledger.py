#!/usr/bin/env python3
"""Tests for content_ledger.py (hatch#3) and its gh-post routing.

A stub `gh` on PATH simulates the GitHub behaviours that matter,
including the 2026-10-28 silent failure (`gh issue create` exits 0 with
empty output and creates nothing). No network, no real GitHub.

Stub modes (env GH_STUB_MODE):
  ok      create prints the object URL and stores the object
  silent  create exits 0, prints nothing, but the object EXISTS
          (verify-by-read must find it by nonce -> verified_existing)
  empty   create exits 0, prints nothing, creates nothing -> failed
  error   create exits 1 with an HTTP 403 on stderr -> failed
  badurl  create prints a URL but no object exists -> failed

On every create call the stub also checks the ledger for a matching
'reserved' row (same nonce as the body's fleet-nonce marker) and exits
9 if there is none — so every passing create test also proves the
reservation was written BEFORE the API call.

Run: python3 tools/content-ledger/test_content_ledger.py  (exit 0 = pass)
Venue: anywhere; uses only temp dirs.
"""
import json
import os
import sqlite3
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
LEDGER_PY = HERE / "content_ledger.py"
GH_POST = HERE.parent / "gh-post"
sys.path.insert(0, str(HERE))

STUB_GH = r'''
import json, os, re, sqlite3, sys
from pathlib import Path

STATE = Path(os.environ["GH_STUB_STATE"])
STATE.mkdir(parents=True, exist_ok=True)
MODE = os.environ.get("GH_STUB_MODE", "ok")
LEDGER = os.environ.get("CONTENT_LEDGER_PATH", "")

def log_call(argv):
    with open(STATE / "calls.log", "a") as f:
        f.write(json.dumps(argv) + "\n")

def next_num(name):
    p = STATE / f"counter-{name}"
    n = int(p.read_text()) if p.exists() else 100
    n += 1
    p.write_text(str(n))
    return n

def save(kind, num, obj):
    d = STATE / kind
    d.mkdir(exist_ok=True)
    (d / f"{num}.json").write_text(json.dumps(obj))

def load(kind, num):
    p = STATE / kind / f"{num}.json"
    return json.loads(p.read_text()) if p.exists() else None

def all_of(kind):
    d = STATE / kind
    if not d.exists():
        return []
    return [json.loads(p.read_text()) for p in sorted(d.glob("*.json"))]

def check_reservation(body):
    m = re.search(r"fleet-nonce:([0-9a-f]+)", body)
    if not m:
        print("stub: no fleet-nonce in body", file=sys.stderr); sys.exit(9)
    if not LEDGER:
        print("stub: CONTENT_LEDGER_PATH unset", file=sys.stderr); sys.exit(9)
    db = sqlite3.connect(f"file:{LEDGER}?mode=ro", uri=True)
    row = db.execute("SELECT outcome FROM events WHERE nonce=?",
                     (m.group(1),)).fetchone()
    if row is None or row[0] != "reserved":
        print(f"stub: no reserved ledger row for nonce (found {row})",
              file=sys.stderr)
        sys.exit(9)

def opt(argv, flag):
    return argv[argv.index(flag) + 1] if flag in argv else None

def do_create(argv):
    repo = opt(argv, "--repo")
    body_file = opt(argv, "--body-file")
    body = Path(body_file).read_text() if body_file else ""
    check_reservation(body)
    if MODE == "error":
        print("gh: HTTP 403: secondary rate limit (api.github.com)",
              file=sys.stderr)
        return 1
    if MODE == "empty":
        return 0
    if argv[:2] == ["issue", "create"]:
        n = next_num("issue")
        url = f"https://github.com/{repo}/issues/{n}"
        if MODE != "badurl":
            save("issues", n, {"number": n, "html_url": url, "body": body})
        if MODE == "silent":
            return 0
        print(url)
        return 0
    if argv[:2] == ["pr", "create"]:
        n = next_num("pr")
        url = f"https://github.com/{repo}/pull/{n}"
        if MODE != "badurl":
            save("pulls", n, {"number": n, "html_url": url, "body": body})
        if MODE == "silent":
            return 0
        print(url)
        return 0
    if argv[1:2] == ["comment"] and argv[0] in ("issue", "pr"):
        target = int(argv[2])
        cid = next_num("comment")
        url = f"https://github.com/{repo}/issues/{target}#issuecomment-{cid}"
        if MODE != "badurl":
            save("comments", cid, {"id": cid, "html_url": url, "body": body,
                                   "issue_number": target})
        if MODE == "silent":
            return 0
        print(url)
        return 0
    print(f"stub: unhandled create argv {argv}", file=sys.stderr)
    return 2

def do_api(argv):
    path = argv[1]
    if "--jq" in argv and argv[argv.index("--jq") + 1] == ".body":
        # gh-post's readback: print just the object's body field.
        m = (re.fullmatch(r"repos/[^/]+/[^/]+/issues/comments/(\d+)", path)
             or re.fullmatch(r"repos/[^/]+/[^/]+/issues/(\d+)", path))
        obj = None
        if m:
            obj = (load("comments", int(m.group(1)))
                   or load("issues", int(m.group(1))))
        if obj:
            print(obj["body"])
            return 0
        print("gh: Not Found (HTTP 404)", file=sys.stderr)
        return 1
    m = re.fullmatch(r"repos/[^/]+/[^/]+/issues/comments/(\d+)", path)
    if m:
        obj = load("comments", int(m.group(1)))
        if obj: print(json.dumps(obj)); return 0
        print("gh: Not Found (HTTP 404)", file=sys.stderr); return 1
    m = re.fullmatch(r"repos/[^/]+/[^/]+/issues/(\d+)/comments\?per_page=100", path)
    if m:
        n = int(m.group(1))
        print(json.dumps([c for c in all_of("comments")
                          if c["issue_number"] == n]))
        return 0
    m = re.fullmatch(r"repos/[^/]+/[^/]+/issues/(\d+)", path)
    if m:
        obj = load("issues", int(m.group(1))) or load("pulls", int(m.group(1)))
        if obj: print(json.dumps(obj)); return 0
        print("gh: Not Found (HTTP 404)", file=sys.stderr); return 1
    m = re.fullmatch(r"repos/[^/]+/[^/]+/pulls/(\d+)", path)
    if m:
        obj = load("pulls", int(m.group(1)))
        if obj: print(json.dumps(obj)); return 0
        print("gh: Not Found (HTTP 404)", file=sys.stderr); return 1
    if re.fullmatch(r"repos/[^/]+/[^/]+/issues\?state=all&per_page=100", path):
        print(json.dumps(all_of("issues") + all_of("pulls")))
        return 0
    if re.fullmatch(r"repos/[^/]+/[^/]+/pulls\?state=all&per_page=100", path):
        print(json.dumps(all_of("pulls")))
        return 0
    print(f"stub: unhandled api path {path}", file=sys.stderr)
    return 2

def main():
    argv = sys.argv[1:]
    log_call(argv)
    if argv[:1] == ["api"]:
        return do_api(argv)
    return do_create(argv)

sys.exit(main())
'''


class Env:
    """One isolated stub-GitHub + ledger world per test case."""

    def __init__(self, mode):
        self.dir = Path(tempfile.mkdtemp(prefix="content-ledger-test-"))
        self.state = self.dir / "gh-state"
        self.ledger = self.dir / "content-ledger.sqlite"
        bin_dir = self.dir / "bin"
        bin_dir.mkdir(parents=True)
        stub = bin_dir / "gh"
        stub.write_text("#!/usr/bin/env python3\n" + STUB_GH)
        stub.chmod(stub.stat().st_mode | stat.S_IEXEC)
        self.env = dict(os.environ)
        self.env.update({
            "PATH": f"{bin_dir}:{os.environ['PATH']}",
            "GH_STUB_STATE": str(self.state),
            "GH_STUB_MODE": mode,
            "CONTENT_LEDGER_PATH": str(self.ledger),
            # gh-post also passes the content-throttle gate (hatch#13);
            # keep its state in the temp world too.
            "CONTENT_THROTTLE_PATH": str(self.dir / "content-throttle.sqlite"),
            "FLEET_JOB": "test-job",
            "FLEET_ACTOR": "test-actor",
        })

    def rows(self):
        if not self.ledger.exists():
            return []
        db = sqlite3.connect(str(self.ledger))
        db.row_factory = sqlite3.Row
        try:
            return [dict(r) for r in
                    db.execute("SELECT * FROM events ORDER BY id")]
        except sqlite3.OperationalError:
            return []

    def gh_calls(self):
        p = self.state / "calls.log"
        if not p.exists():
            return []
        return [json.loads(l) for l in p.read_text().splitlines() if l.strip()]


FAILURES = []


def check(name, cond, extra=""):
    if cond:
        print(f"ok: {name}")
    else:
        FAILURES.append(f"{name} {extra}")
        print(f"FAIL: {name} {extra}")


def run_cli(env, *args):
    return subprocess.run([sys.executable, str(LEDGER_PY), *args],
                          capture_output=True, text=True, env=env.env)


def t_created():
    e = Env("ok")
    body = e.dir / "body.md"
    body.write_text("a real issue body\n")
    r = run_cli(e, "create-issue", "--repo", "o/r", "--title", "t",
                "--body-file", str(body))
    check("created: cli exit 0", r.returncode == 0, r.stderr)
    rows = e.rows()
    check("created: one ledger row", len(rows) == 1, str(rows))
    if rows:
        row = rows[0]
        check("created: outcome", row["outcome"] == "created", str(row))
        check("created: kind/repo/job/actor",
              (row["kind"], row["repo"], row["job"], row["actor"])
              == ("issue", "o/r", "test-job", "test-actor"), str(row))
        check("created: ref is the issue URL",
              row["ref"] == "https://github.com/o/r/issues/101", str(row))
        check("created: nonce recorded", bool(row["nonce"]), str(row))


def t_verified_existing():
    e = Env("silent")
    body = e.dir / "body.md"
    body.write_text("created but gh printed nothing\n")
    r = run_cli(e, "create-issue", "--repo", "o/r", "--title", "t",
                "--body-file", str(body))
    check("verified_existing: cli exit 0", r.returncode == 0, r.stderr)
    rows = e.rows()
    check("verified_existing: outcome",
          len(rows) == 1 and rows[0]["outcome"] == "verified_existing",
          str(rows))
    if rows:
        check("verified_existing: ref found by nonce search",
              rows[0]["ref"] == "https://github.com/o/r/issues/101", str(rows[0]))


def t_empty_output_is_failure():
    e = Env("empty")
    body = e.dir / "body.md"
    body.write_text("the 2026-10-28 silent failure shape\n")
    r = run_cli(e, "create-issue", "--repo", "o/r", "--title", "t",
                "--body-file", str(body))
    check("empty: cli exit 1 (never success)", r.returncode == 1,
          f"rc={r.returncode} out={r.stdout}")
    rows = e.rows()
    check("empty: outcome failed",
          len(rows) == 1 and rows[0]["outcome"] == "failed", str(rows))
    if rows:
        check("empty: detail names the empty output",
              "empty" in (rows[0]["detail"] or ""), str(rows[0]))


def t_api_error_is_failure():
    e = Env("error")
    body = e.dir / "body.md"
    body.write_text("will 403\n")
    r = run_cli(e, "create-issue", "--repo", "o/r", "--title", "t",
                "--body-file", str(body))
    check("error: cli exit 1", r.returncode == 1, f"rc={r.returncode}")
    rows = e.rows()
    check("error: outcome failed",
          len(rows) == 1 and rows[0]["outcome"] == "failed", str(rows))
    if rows:
        check("error: underlying error preserved in detail",
              "403" in (rows[0]["detail"] or ""), str(rows[0]))
    check("error: caller sees the underlying error",
          "403" in r.stderr, r.stderr)


def t_badurl_is_failure():
    e = Env("badurl")
    body = e.dir / "body.md"
    body.write_text("url printed but nothing exists\n")
    r = run_cli(e, "create-issue", "--repo", "o/r", "--title", "t",
                "--body-file", str(body))
    check("badurl: cli exit 1", r.returncode == 1, f"rc={r.returncode}")
    rows = e.rows()
    check("badurl: outcome failed (readback is the oracle)",
          len(rows) == 1 and rows[0]["outcome"] == "failed", str(rows))


def t_comment_and_pr():
    e = Env("ok")
    body = e.dir / "body.md"
    body.write_text("a comment body\n")
    r = run_cli(e, "create-comment", "--repo", "o/r", "--issue", "42",
                "--body-file", str(body))
    check("comment: cli exit 0", r.returncode == 0, r.stderr)
    r = run_cli(e, "create-pr", "--repo", "o/r", "--title", "p",
                "--body-file", str(body), "--head", "feat/x")
    check("pr: cli exit 0", r.returncode == 0, r.stderr)
    rows = e.rows()
    kinds = [(row["kind"], row["outcome"]) for row in rows]
    check("comment+pr: kinds and outcomes",
          kinds == [("issue_comment", "created"), ("pr", "created")],
          str(kinds))
    if len(rows) == 2:
        check("comment: ref is the comment URL",
              rows[0]["ref"] == "https://github.com/o/r/issues/42#issuecomment-101",
              str(rows[0]))


def t_concurrent_reserve():
    e = Env("ok")
    procs, per_proc = 8, 25
    code = (
        "import sys; sys.path.insert(0, sys.argv[1]);"
        "import content_ledger;"
        "led = content_ledger.Ledger();"
        "ids = [led.reserve('a', 'conc', 'o/r', 'issue') "
        "       for _ in range(int(sys.argv[2]))];"
        "print(len(set(ids)))"
    )
    runs = [subprocess.Popen([sys.executable, "-c", code, str(HERE),
                              str(per_proc)],
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                             text=True, env=e.env)
            for _ in range(procs)]
    ok = True
    for p in runs:
        out, err = p.communicate()
        if p.returncode != 0 or out.strip() != str(per_proc):
            ok = False
            print(f"concurrency worker failed: rc={p.returncode} {err[:300]}")
    check("concurrency: 8 processes x 25 reserves, no errors, unique ids", ok)
    rows = e.rows()
    check("concurrency: all 200 reservations landed",
          len(rows) == procs * per_proc, f"rows={len(rows)}")


def t_ledger_failure_is_visible_and_blocks():
    e = Env("ok")
    e.env["CONTENT_LEDGER_PATH"] = "/proc/no-such-dir/content-ledger.sqlite"
    body = e.dir / "body.md"
    body.write_text("must never be filed unledgered\n")
    r = run_cli(e, "create-issue", "--repo", "o/r", "--title", "t",
                "--body-file", str(body))
    check("ledger-down: cli exit 5", r.returncode == 5, f"rc={r.returncode}")
    check("ledger-down: gh was never called", e.gh_calls() == [],
          str(e.gh_calls()))


def t_counts_cli():
    e = Env("ok")
    body = e.dir / "body.md"
    body.write_text("counted\n")
    run_cli(e, "create-issue", "--repo", "o/r", "--title", "t",
            "--body-file", str(body))
    r = run_cli(e, "counts", "--window", "3600")
    check("counts: exit 0", r.returncode == 0, r.stderr)
    check("counts: created total visible",
          "created" in r.stdout and "total: 1" in r.stdout, r.stdout)


def t_gh_post_routing():
    # gh-post with the user leg served by the stub: posts, ledgers created.
    e = Env("ok")
    home = e.dir / "home"
    (home / "workspace/goals/keep-the-four-repos-healthy/hidden_files").mkdir(
        parents=True)
    (home / "workspace/goals/keep-the-four-repos-healthy/hidden_files"
     / "action-outbox.md").write_text("## Done\n")
    e.env["HOME"] = str(home)
    body = e.dir / "body.md"
    body.write_text("posted through gh-post\n")
    r = subprocess.run([sys.executable, str(GH_POST), "comment",
                        "--repo", "o/r", "--issue", "7",
                        "--body-file", str(body)],
                       capture_output=True, text=True, env=e.env)
    check("gh-post: exit 0", r.returncode == 0, r.stderr)
    rows = e.rows()
    check("gh-post: ledger row created via its readback",
          len(rows) == 1 and rows[0]["outcome"] == "created"
          and rows[0]["kind"] == "issue_comment", str(rows))

    # gh-post with a dead ledger: refuses BEFORE any gh call (exit 5).
    e2 = Env("ok")
    e2.env["HOME"] = str(home)
    e2.env["CONTENT_LEDGER_PATH"] = "/proc/no-such-dir/content-ledger.sqlite"
    r = subprocess.run([sys.executable, str(GH_POST), "comment",
                        "--repo", "o/r", "--issue", "7",
                        "--body-file", str(body)],
                       capture_output=True, text=True, env=e2.env)
    check("gh-post ledger-down: exit 5", r.returncode == 5,
          f"rc={r.returncode} {r.stderr[:200]}")
    check("gh-post ledger-down: gh never called", e2.gh_calls() == [],
          str(e2.gh_calls()))


def main():
    t_created()
    t_verified_existing()
    t_empty_output_is_failure()
    t_api_error_is_failure()
    t_badurl_is_failure()
    t_comment_and_pr()
    t_concurrent_reserve()
    t_ledger_failure_is_visible_and_blocks()
    t_counts_cli()
    t_gh_post_routing()
    if FAILURES:
        print(f"\n{len(FAILURES)} FAILURE(S):")
        for f in FAILURES:
            print(f"  - {f}")
        return 1
    print("\nPASS: content-ledger created / verified_existing / empty / "
          "error / badurl outcomes, comment+pr kinds, concurrent reserve, "
          "ledger-failure visibility, counts, gh-post routing")
    return 0


if __name__ == "__main__":
    sys.exit(main())
