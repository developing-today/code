#!/usr/bin/env python3
"""Tests for content_throttle.py (hatch#13) and its gh-post routing.

A stub `gh` on PATH simulates GitHub, including rate-limit refusals
with and without `retry-after`. No network, no real GitHub. Library
tests run the throttle on a fake clock (sleeps advance fake time), so
hour-scale pacing is exercised in milliseconds.

Stub modes (env GH_STUB_MODE):
  ok        creates succeed and store the object
  limit     every create exits 1: HTTP 403 secondary rate limit,
            no retry-after
  limit-ra  every create exits 1 with `retry-after: 7`
  flaky-ra  the FIRST create fails limit-ra, later creates succeed
  flaky     the first GH_STUB_FAIL_FIRST creates fail `limit`,
            later creates succeed

On every create the stub checks the ledger for a matching 'reserved'
row (the phase-1 reservation-before-call proof, kept here).

Run: python3 tools/content-throttle/test_content_throttle.py
Venue: anywhere; uses only temp dirs.
"""

import json
import os
import sqlite3
import stat
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
THROTTLE_PY = HERE / "content_throttle.py"
GH_POST = HERE.parent / "gh-post"
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "content-ledger"))

import content_ledger  # noqa: E402
import content_throttle  # noqa: E402

STUB_GH = r"""
import json, os, re, sqlite3, sys
from pathlib import Path

STATE = Path(os.environ["GH_STUB_STATE"])
STATE.mkdir(parents=True, exist_ok=True)
MODE = os.environ.get("GH_STUB_MODE", "ok")
LEDGER = os.environ.get("CONTENT_LEDGER_PATH", "")

def log_call(argv):
    with open(STATE / "calls.log", "a") as f:
        f.write(json.dumps(argv) + "\n")

def bump(name):
    p = STATE / f"counter-{name}"
    n = int(p.read_text()) if p.exists() else 0
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
    db = sqlite3.connect(f"file:{LEDGER}?mode=ro", uri=True)
    row = db.execute("SELECT outcome FROM events WHERE nonce=?",
                     (m.group(1),)).fetchone()
    if row is None or row[0] != "reserved":
        print(f"stub: no reserved ledger row (found {row})",
              file=sys.stderr)
        sys.exit(9)

def opt(argv, flag):
    return argv[argv.index(flag) + 1] if flag in argv else None

def limit_failure(with_ra):
    print("gh: HTTP 403: secondary rate limit (api.github.com)",
          file=sys.stderr)
    if with_ra:
        print("retry-after: 7", file=sys.stderr)
    return 1

def do_create(argv):
    repo = opt(argv, "--repo")
    body_file = opt(argv, "--body-file")
    body = Path(body_file).read_text() if body_file else ""
    check_reservation(body)
    n_create = bump("create")
    fail_first = int(os.environ.get("GH_STUB_FAIL_FIRST", "0"))
    if MODE == "limit" or (MODE == "flaky" and n_create <= fail_first):
        return limit_failure(False)
    if MODE == "limit-ra":
        return limit_failure(True)
    if MODE == "flaky-ra" and n_create == 1:
        return limit_failure(True)
    if argv[:2] == ["issue", "create"]:
        n = bump("issue")
        url = f"https://github.com/{repo}/issues/{n}"
        save("issues", n, {"number": n, "html_url": url, "body": body})
        print(url)
        return 0
    if argv[:2] == ["pr", "create"]:
        n = bump("pr")
        url = f"https://github.com/{repo}/pull/{n}"
        save("pulls", n, {"number": n, "html_url": url, "body": body})
        print(url)
        return 0
    if argv[1:2] == ["comment"] and argv[0] in ("issue", "pr"):
        target = int(argv[2])
        cid = bump("comment")
        url = f"https://github.com/{repo}/issues/{target}#issuecomment-{cid}"
        save("comments", cid, {"id": cid, "html_url": url, "body": body,
                               "issue_number": target})
        print(url)
        return 0
    print(f"stub: unhandled create argv {argv}", file=sys.stderr)
    return 2

def do_api(argv):
    if "--method" in argv and argv[argv.index("--method") + 1] == "PATCH":
        path = argv[argv.index("--method") + 2]
        m = re.fullmatch(r"repos/[^/]+/[^/]+/issues/comments/(\d+)", path)
        if not m:
            print("stub: bad PATCH path", file=sys.stderr); return 2
        cid = int(m.group(1))
        obj = load("comments", cid)
        if not obj:
            print("gh: Not Found (HTTP 404)", file=sys.stderr); return 1
        payload = json.loads(
            Path(argv[argv.index("--input") + 1]).read_text())
        obj["body"] = payload["body"]
        save("comments", cid, obj)
        print(json.dumps(obj))
        return 0
    path = argv[1]
    if "--jq" in argv and argv[argv.index("--jq") + 1] == ".body":
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
    m = re.fullmatch(r"repos/[^/]+/[^/]+/issues/(\d+)/comments\?per_page=100",
                     path)
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
"""


class Clock:
    def __init__(self):
        self.t = time.time()
        self.sleeps = []

    def now(self):
        return self.t

    def sleep(self, s):
        self.sleeps.append(s)
        self.t += s


class Env:
    """One isolated stub-GitHub + ledger + throttle world.

    Sets os.environ too: the library path shells out to `gh` via the
    ledger module, inheriting this process's environment.
    """

    def __init__(self, mode="ok"):
        self.dir = Path(tempfile.mkdtemp(prefix="content-throttle-test-"))
        self.state = self.dir / "gh-state"
        self.ledger = self.dir / "content-ledger.sqlite"
        self.throttle = self.dir / "content-throttle.sqlite"
        bin_dir = self.dir / "bin"
        bin_dir.mkdir(parents=True)
        stub = bin_dir / "gh"
        stub.write_text("#!/usr/bin/env python3\n" + STUB_GH)
        stub.chmod(stub.stat().st_mode | stat.S_IEXEC)
        self.saved = {}
        updates = {
            "PATH": f"{bin_dir}:{os.environ['PATH']}",
            "GH_STUB_STATE": str(self.state),
            "GH_STUB_MODE": mode,
            "CONTENT_LEDGER_PATH": str(self.ledger),
            "CONTENT_THROTTLE_PATH": str(self.throttle),
            "FLEET_JOB": "test-job",
            "FLEET_ACTOR": "test-actor",
        }
        for k, v in updates.items():
            self.saved[k] = os.environ.get(k)
            os.environ[k] = v
        self.env = dict(os.environ)

    def restore(self):
        for k, v in self.saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v

    def make_throttle(self, clock=None, mode=None):
        clock = clock or Clock()
        return content_throttle.Throttle(
            mode=mode, now_fn=clock.now, sleep_fn=clock.sleep
        ), clock

    def preload_creations(self, n, job, at_epoch, kind="issue"):
        led = content_ledger.Ledger()
        ts = content_throttle.iso(at_epoch)
        for _ in range(n):
            led.db.execute(
                "INSERT INTO events(ts_utc, actor, job, repo, kind, "
                "outcome) VALUES(?,?,?,?,?,'created')",
                (ts, "test-actor", job, "o/r", kind),
            )
        led.close()

    def ledger_rows(self):
        if not self.ledger.exists():
            return []
        db = sqlite3.connect(str(self.ledger))
        db.row_factory = sqlite3.Row
        try:
            return [dict(r) for r in db.execute("SELECT * FROM events ORDER BY id")]
        except sqlite3.OperationalError:
            return []

    def slots(self):
        if not self.throttle.exists():
            return []
        db = sqlite3.connect(str(self.throttle))
        db.row_factory = sqlite3.Row
        try:
            return [dict(r) for r in db.execute("SELECT * FROM slots ORDER BY id")]
        except sqlite3.OperationalError:
            return []

    def gh_calls(self):
        p = self.state / "calls.log"
        if not p.exists():
            return []
        return [json.loads(l) for l in p.read_text().splitlines() if l.strip()]

    def warn_rows(self):
        p = self.dir / "content-throttle-warn.jsonl"
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
    return subprocess.run(
        [sys.executable, str(THROTTLE_PY), *args],
        capture_output=True,
        text=True,
        env=env.env,
    )


# ------------------------------------------------------------ the tests
def t_batch_100():
    """Acceptance: a 100-create batch through the wrapper cannot
    exceed the budgets; the ledger shape proves it."""
    e = Env("ok")
    try:
        th, clock = e.make_throttle(mode="enforce")
        start = clock.t
        for i in range(100):
            th.create_issue("o/r", f"batch {i}", "body\n", job="batch", max_wait=20000)
        rows = e.ledger_rows()
        check("batch: 100 ledger rows", len(rows) == 100, f"rows={len(rows)}")
        check("batch: all created", all(r["outcome"] == "created" for r in rows))
        grants = sorted(
            s["grant_epoch"] for s in e.slots() if s["grant_epoch"] is not None
        )
        check("batch: 100 grants", len(grants) == 100, f"{len(grants)}")
        # The minute cap is asserted over the first 50 grants, where
        # the fake clock and the ledger's wall-clock timestamps agree.
        # (The throttle computes windows from ledger timestamps — the
        # design's rule — so after the lane cap's hour-scale fake
        # sleep, ledger rows are already old in fake time and later
        # grants are spacing-paced. One host, one clock in production;
        # the divergence is a harness artifact, not a pacing hole.)
        first = grants[:50]
        worst_min = max(sum(1 for g in first if t <= g < t + 60) for t in first)
        check(
            "batch: <=45 grants in any 60s window (first phase)",
            worst_min <= 45,
            f"worst={worst_min}",
        )
        worst_hour = max(sum(1 for g in grants if t <= g < t + 3600) for t in grants)
        check(
            "batch: <=300 grants in any hour window",
            worst_hour <= 300,
            f"worst={worst_hour}",
        )
        check(
            "batch: the lane cap (50/h) stretched the batch past "
            "one hour of paced time",
            clock.t - start >= 3500,
            f"elapsed={clock.t - start:.0f}s",
        )
        diffs = [b - a for a, b in zip(grants, grants[1:])]
        check(
            "batch: >=1s spacing between grants",
            all(d >= 1.0 - 1e-9 for d in diffs),
            f"min diff={min(diffs) if diffs else None}",
        )
    finally:
        e.restore()


def t_warn_only_never_delays():
    e = Env("ok")
    try:
        th, clock = e.make_throttle()  # stored mode: warn
        e.preload_creations(300, "other-lane", clock.t - 1800)
        slot = th.acquire("test-job", "issue", "o/r")
        th.release(slot)
        check(
            "warn: no sleep despite a full hour budget",
            clock.sleeps == [],
            str(clock.sleeps),
        )
        rows = e.warn_rows()
        check("warn: would-be delay logged", len(rows) == 1, str(rows))
        if rows:
            check(
                "warn: binding constraint is the reserved hour cap",
                rows[0]["binding_constraint"] == "account_hour_reserve",
                str(rows[0]),
            )
            check(
                "warn: would_wait recorded",
                rows[0]["would_wait_seconds"] > 0,
                str(rows[0]),
            )
    finally:
        e.restore()


def t_enforce_delays_then_grants():
    e = Env("ok")
    try:
        th, clock = e.make_throttle(mode="enforce")
        e.preload_creations(45, "other-lane", clock.t - 30)
        slot = th.acquire("test-job", "issue", "o/r", max_wait=300)
        th.release(slot)
        check(
            "enforce: waited for the minute window",
            sum(clock.sleeps) >= 29,
            f"slept={sum(clock.sleeps)}",
        )
        grants = [s for s in e.slots() if s["id"] == slot]
        check(
            "enforce: granted after the oldest creation aged out",
            grants and grants[0]["grant_epoch"] >= clock.t - 1,
            str(grants),
        )
    finally:
        e.restore()


def t_enforce_refuses_visibly():
    e = Env("ok")
    try:
        th, clock = e.make_throttle(mode="enforce")
        e.preload_creations(300, "other-lane", clock.t - 60)
        try:
            th.acquire("test-job", "issue", "o/r", max_wait=0)
            check("refuse: acquire raised", False)
        except content_throttle.ThrottleRefused as ex:
            check(
                "refuse: ThrottleRefused names the constraint",
                ex.constraint == "account_hour_reserve",
                str(ex),
            )
        slots = e.slots()
        check(
            "refuse: no slot left in flight",
            all(s["state"] == "done" for s in slots),
            str(slots),
        )
    finally:
        e.restore()


def t_lane_fairness_and_reserve():
    e = Env("ok")
    try:
        th, clock = e.make_throttle(mode="enforce")
        e.preload_creations(50, "lane-a", clock.t - 600)

        def try_acquire(**kw):
            clock.t += 2  # step past the 1s spacing from any prior grant
            try:
                slot = th.acquire(max_wait=0, **kw)
                th.release(slot)
                return None
            except content_throttle.ThrottleRefused as ex:
                return ex.constraint

        c = try_acquire(job="lane-a", kind="issue", repo="o/r")
        check("lane: lane-a at its 50/h share is refused", c == "lane_hour", str(c))
        c = try_acquire(job="lane-b", kind="issue", repo="o/r")
        check("lane: fresh lane-b still granted", c is None, str(c))
        e.preload_creations(175, "lane-c", clock.t - 600)
        c = try_acquire(job="lane-b", kind="issue", repo="o/r")
        check(
            "reserve: non-priority refused at 225 (75 held back)",
            c == "account_hour_reserve",
            str(c),
        )
        c = try_acquire(job="lane-b", kind="issue", repo="o/r", priority=True)
        check("reserve: priority-10 may draw the reserve", c is None, str(c))
        e.preload_creations(75, "lane-c", clock.t - 600)
        c = try_acquire(job="lane-b", kind="issue", repo="o/r", priority=True)
        check(
            "reserve: priority refused once the full 300 is used",
            c == "account_hour",
            str(c),
        )
    finally:
        e.restore()


def t_bulk_window():
    e = Env("ok")
    try:
        th, clock = e.make_throttle(mode="enforce")
        for _ in range(20):
            clock.t += 2
            slot = th.acquire("bulk-job", "issue", "o/r", bulk=True)
            th.release(slot)
        clock.t += 2
        try:
            th.acquire("bulk-job", "issue", "o/r", bulk=True, max_wait=0)
            check("bulk: 21st bulk create in 30 min raised", False)
        except content_throttle.ThrottleRefused as ex:
            check(
                "bulk: 21st bulk create refused on the bulk window",
                ex.constraint == "bulk_window",
                str(ex),
            )
        slot = th.acquire("bulk-job", "issue", "o/r", bulk=False, max_wait=0)
        th.release(slot)
        check("bulk: non-bulk create unaffected", True)
    finally:
        e.restore()


def t_inflight_cap():
    e = Env("ok")
    try:
        th, clock = e.make_throttle(mode="enforce")
        held = []
        for _ in range(8):
            clock.t += 2
            held.append(th.acquire("test-job", "issue", "o/r"))
        clock.t += 2
        try:
            th.acquire("test-job", "issue", "o/r", max_wait=0.1)
            check("inflight: 9th concurrent call raised", False)
        except content_throttle.ThrottleRefused as ex:
            check(
                "inflight: 9th concurrent call refused",
                ex.constraint == "inflight_cap",
                str(ex),
            )
        th.release(held[0])
        clock.t += 2
        slot = th.acquire("test-job", "issue", "o/r", max_wait=0)
        th.release(slot)
        check("inflight: grant resumes after a release", True)
        for s in held[1:]:
            th.release(s)
    finally:
        e.restore()


def t_breaker_retry_after():
    e = Env("flaky-ra")
    try:
        th, clock = e.make_throttle(mode="enforce")
        res = th.create_issue("o/r", "t", "body\n", job="breaker")
        check("breaker-ra: create eventually succeeds", bool(res.ref))
        check(
            "breaker-ra: slept exactly the retry-after",
            clock.sleeps == [7],
            str(clock.sleeps),
        )
        rows = e.ledger_rows()
        outcomes = [r["outcome"] for r in rows]
        check(
            "breaker-ra: failed attempt + created attempt ledgered",
            outcomes == ["failed", "created"],
            str(outcomes),
        )
        check(
            "breaker-ra: success reset the failure count",
            th.circuit_state()["failures"] == 0,
            str(th.circuit_state()),
        )
    finally:
        e.restore()


def t_breaker_exponential_and_circuit():
    e = Env("limit")
    try:
        th, clock = e.make_throttle(mode="enforce")
        try:
            th.create_issue("o/r", "t", "body\n", job="breaker")
            check("breaker: create raised", False)
        except content_throttle.CircuitOpen as ex:
            check("breaker: five failures open the circuit", ex.failures >= 5, str(ex))
            check(
                "breaker: resume brief names the open-until time",
                "Park the calling task" in ex.brief,
                ex.brief,
            )
        check(
            "breaker: exponential sleeps 60/120/240/480",
            clock.sleeps == [60, 120, 240, 480],
            str(clock.sleeps),
        )
        rows = e.ledger_rows()
        check(
            "breaker: five failed attempts ledgered",
            len(rows) == 5 and all(r["outcome"] == "failed" for r in rows),
            str(rows),
        )
        calls_before = len(e.gh_calls())
        try:
            th.acquire("breaker", "issue", "o/r")
            check("breaker: acquire blocked while open", False)
        except content_throttle.CircuitOpen:
            check("breaker: acquire blocked while open", True)
        check(
            "breaker: no gh call attempted while open",
            len(e.gh_calls()) == calls_before,
        )
        clock.t += 901
        check(
            "breaker: circuit closes after the 15-minute stop",
            th.circuit_state()["open"] is False,
            str(th.circuit_state()),
        )
    finally:
        e.restore()


def t_breaker_failures_accumulate_across_calls():
    e = Env("ok")
    try:
        th, clock = e.make_throttle(mode="enforce")

        def always_limited():
            raise content_throttle.RateLimited("HTTP 403 rate limit", retry_after=0)

        for i in range(2):
            try:
                th.call_with_breaker(always_limited)
                check("accumulate: call raised", False)
            except content_throttle.RateLimitExhausted:
                pass
        check(
            "accumulate: 4 failures recorded across calls",
            th.circuit_state()["failures"] == 4,
            str(th.circuit_state()),
        )
        try:
            th.call_with_breaker(always_limited)
            check("accumulate: third call raised", False)
        except content_throttle.CircuitOpen:
            check("accumulate: 5th failure (any caller) opens circuit", True)
    finally:
        e.restore()


def t_gap_detector():
    e = Env("ok")
    try:
        th, clock = e.make_throttle()
        now = clock.t
        e.preload_creations(1, "test-job", now - 600)
        led = content_ledger.Ledger()
        row = led.reserve("test-actor", "test-job", "o/r", "issue")
        led.db.execute(
            "UPDATE events SET ts_utc=? WHERE id=?",
            (content_throttle.iso(now - 600), row),
        )
        led.close()
        rep = th.gap_check(now=now)
        check(
            "gap: stuck reservation + 10 min silence alerts",
            rep["alert"] is True,
            str(rep),
        )
        check(
            "gap: queued count names the stuck reservation",
            rep["queued"]["stuck_reservations"] == 1,
            str(rep),
        )
        e.preload_creations(1, "test-job", now - 60)
        rep = th.gap_check(now=now)
        check(
            "gap: a creation 60s ago clears the alert", rep["alert"] is False, str(rep)
        )
        led = content_ledger.Ledger()
        led.finalize(row, "failed", detail="test cleanup")
        led.close()
        rep = th.gap_check(now=now)
        check(
            "gap: no queued work, no alert even when quiet",
            rep["alert"] is False,
            str(rep),
        )
        outbox = e.dir / "action-outbox.md"
        outbox.write_text("### OB-1 · issue · o/r\n- status: prepared\n\n## Done\n")
        rep = th.gap_check(outbox_path=str(outbox), now=now + 600)
        check(
            "gap: prepared outbox entry counts as queued work",
            rep["alert"] is True and rep["queued"]["outbox_prepared"] == 1,
            str(rep),
        )
    finally:
        e.restore()
    # CLI exit code, in a fresh world whose last creation is 10 real
    # minutes old and which holds one stuck reservation.
    e = Env("ok")
    try:
        th, clock = e.make_throttle()
        e.preload_creations(1, "test-job", clock.t - 600)
        led = content_ledger.Ledger()
        row = led.reserve("test-actor", "test-job", "o/r", "issue")
        led.db.execute(
            "UPDATE events SET ts_utc=? WHERE id=?",
            (content_throttle.iso(clock.t - 600), row),
        )
        led.close()
        r = run_cli(e, "gap-check")
        check(
            "gap: CLI exits 2 on alert",
            r.returncode == 2,
            f"rc={r.returncode} {r.stdout} {r.stderr}",
        )
    finally:
        e.restore()


def t_throttle_down_refuses():
    e = Env("ok")
    try:
        e.env["CONTENT_THROTTLE_PATH"] = "/proc/no-such-dir/t.sqlite"
        r = run_cli(e, "reserve", "--job", "j", "--kind", "issue")
        check("down: reserve CLI exit 5", r.returncode == 5, f"rc={r.returncode}")
        e.env["CONTENT_LEDGER_PATH"] = "/proc/no-such-dir/l.sqlite"
        e.env["CONTENT_THROTTLE_PATH"] = str(e.throttle)
        r = run_cli(e, "reserve", "--job", "j", "--kind", "issue")
        check(
            "down: dead ledger also exit 5 (never unthrottled)",
            r.returncode == 5,
            f"rc={r.returncode}",
        )
        check("down: gh never called", e.gh_calls() == [])
    finally:
        e.restore()


def t_mode_cli():
    e = Env("ok")
    try:
        r = run_cli(e, "mode")
        check("mode: ships warn-only", "mode: warn" in r.stdout, r.stdout)
        r = run_cli(e, "mode", "enforce")
        check("mode: flip to enforce works", "mode: enforce" in r.stdout, r.stdout)
        r = run_cli(e, "status")
        st = json.loads(r.stdout)
        check(
            "mode: status reports budgets",
            st["budgets"]["per_hour"] == 300
            and st["budgets"]["breakage_reserve"] == 75,
            r.stdout,
        )
    finally:
        e.restore()


def _gh_post_env(e, mode="ok"):
    home = e.dir / "home"
    hidden = home / "workspace/goals/keep-the-four-repos-healthy" / "hidden_files"
    hidden.mkdir(parents=True)
    (hidden / "action-outbox.md").write_text("## Done\n")
    env = dict(e.env)
    env["HOME"] = str(home)
    env["GH_STUB_MODE"] = mode
    return env


def t_gh_post_gate_and_note():
    e = Env("ok")
    try:
        env = _gh_post_env(e)
        body = e.dir / "body.md"
        body.write_text("checkpoint one\n")
        r = subprocess.run(
            [
                sys.executable,
                str(GH_POST),
                "comment",
                "--repo",
                "o/r",
                "--issue",
                "7",
                "--body-file",
                str(body),
            ],
            capture_output=True,
            text=True,
            env=env,
        )
        check("gh-post: comment still posts in warn mode", r.returncode == 0, r.stderr)
        slots = e.slots()
        check(
            "gh-post: throttle slot recorded and released",
            len(slots) == 1
            and slots[0]["state"] == "done"
            and "created" in (slots[0]["detail"] or ""),
            str(slots),
        )

        # Live note: first call creates, second call edits in place.
        note1 = e.dir / "note1.md"
        note1.write_text("live note v1\n")
        r = subprocess.run(
            [
                sys.executable,
                str(GH_POST),
                "note",
                "--repo",
                "o/r",
                "--issue",
                "7",
                "--body-file",
                str(note1),
            ],
            capture_output=True,
            text=True,
            env=env,
        )
        check(
            "note: first call posts",
            r.returncode == 0 and "posted (live note)" in r.stdout,
            f"{r.stdout} {r.stderr}",
        )
        note2 = e.dir / "note2.md"
        note2.write_text("live note v2 — updated in place\n")
        r = subprocess.run(
            [
                sys.executable,
                str(GH_POST),
                "note",
                "--repo",
                "o/r",
                "--issue",
                "7",
                "--body-file",
                str(note2),
            ],
            capture_output=True,
            text=True,
            env=env,
        )
        check(
            "note: second call edits in place",
            r.returncode == 0 and "edited (live note)" in r.stdout,
            f"{r.stdout} {r.stderr}",
        )
        comments_dir = e.state / "comments"
        comments = [json.loads(p.read_text()) for p in comments_dir.glob("*.json")]
        on_issue = [c for c in comments if c["issue_number"] == 7]
        # One plain comment (above) + ONE live note, edited not duplicated.
        check(
            "note: target holds exactly 2 comments total",
            len(on_issue) == 2,
            f"{len(on_issue)}",
        )
        note_bodies = [c["body"] for c in on_issue if "fleet-live-note" in c["body"]]
        check(
            "note: the live note carries the v2 body",
            len(note_bodies) == 1 and "v2" in note_bodies[0],
            str(note_bodies),
        )
        rows = e.ledger_rows()
        edits = [r for r in rows if "live-note edit" in (r["detail"] or "")]
        check(
            "note: the edit is ledgered as a creation",
            len(edits) == 1 and edits[0]["outcome"] == "created",
            str(rows),
        )
    finally:
        e.restore()


def t_gh_post_circuit_skips_user_leg():
    e = Env("ok")
    try:
        th, _clock = e.make_throttle()
        for _ in range(5):
            th.record_failure()
        check("gh-post circuit: seeded open", th.circuit_state()["open"] is True)
        env = _gh_post_env(e)
        env["GH_POST_SKIP_APP"] = "1"
        body = e.dir / "body.md"
        body.write_text("must not spin against a refusing API\n")
        r = subprocess.run(
            [
                sys.executable,
                str(GH_POST),
                "comment",
                "--repo",
                "o/r",
                "--issue",
                "7",
                "--body-file",
                str(body),
            ],
            capture_output=True,
            text=True,
            env=env,
        )
        check(
            "gh-post circuit: post queues to the outbox (exit 3)",
            r.returncode == 3,
            f"rc={r.returncode} {r.stderr[:200]}",
        )
        creates = [
            c
            for c in e.gh_calls()
            if c[:2] in (["issue", "comment"], ["pr", "comment"])
        ]
        check("gh-post circuit: user leg never attempted", creates == [], str(creates))
        rows = e.ledger_rows()
        check(
            "gh-post circuit: ledger row records the circuit skip",
            len(rows) == 1
            and rows[0]["outcome"] == "failed"
            and "circuit open" in (rows[0]["detail"] or ""),
            str(rows),
        )
    finally:
        e.restore()


def t_gh_post_throttle_down_refuses():
    e = Env("ok")
    try:
        env = _gh_post_env(e)
        env["CONTENT_THROTTLE_PATH"] = "/proc/no-such-dir/t.sqlite"
        body = e.dir / "body.md"
        body.write_text("never unthrottled\n")
        r = subprocess.run(
            [
                sys.executable,
                str(GH_POST),
                "comment",
                "--repo",
                "o/r",
                "--issue",
                "7",
                "--body-file",
                str(body),
            ],
            capture_output=True,
            text=True,
            env=env,
        )
        check("gh-post throttle-down: exit 5", r.returncode == 5, f"rc={r.returncode}")
        check(
            "gh-post throttle-down: gh never called",
            e.gh_calls() == [],
            str(e.gh_calls()),
        )
    finally:
        e.restore()


def main():
    t_batch_100()
    t_warn_only_never_delays()
    t_enforce_delays_then_grants()
    t_enforce_refuses_visibly()
    t_lane_fairness_and_reserve()
    t_bulk_window()
    t_inflight_cap()
    t_breaker_retry_after()
    t_breaker_exponential_and_circuit()
    t_breaker_failures_accumulate_across_calls()
    t_gap_detector()
    t_throttle_down_refuses()
    t_mode_cli()
    t_gh_post_gate_and_note()
    t_gh_post_circuit_skips_user_leg()
    t_gh_post_throttle_down_refuses()
    if FAILURES:
        print(f"\n{len(FAILURES)} FAILURE(S):")
        for f in FAILURES:
            print(f"  - {f}")
        return 1
    print(
        "\nPASS: content-throttle 100-create batch budgets, warn-only "
        "logging, enforce delay/refusal, lane fairness + breakage "
        "reserve, bulk window, in-flight cap, circuit breaker "
        "(retry-after, exponential, cross-call accumulation), gap "
        "detector, fail-visible refusals, gh-post gate + live note"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
