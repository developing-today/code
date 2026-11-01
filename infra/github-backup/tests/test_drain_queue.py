#!/usr/bin/env python3
"""test_drain_queue.py — regression tests for #3219 ack discipline
and the #3326 owner allowlist.

Drives the real drain_queue.main() with a stub Worker and a stub
fetch_mirrors.fetch_repo, against a temp backup root:

  success          -> event acked, coalesce stamp set
  fetch "fail"     -> event NOT acked, fetch.log line written, no stamp
  fetch "missing"  -> event NOT acked, fetch.log line written, no stamp
  unplanned repo   -> event acked + fetch.log note (behavior preserved)
  inside coalesce  -> event NOT acked, no fetch attempted
  chronic failures -> run exits non-zero, DRAIN-CHRONIC logged, still queued
  foreign owner    -> dropped + acked + DRAIN-DROP logged, never fetched,
                      never counted toward the chronic failure streak
  missing owner    -> dropped + acked + DRAIN-DROP logged (malformed)

Run: python3 infra/github-backup/tests/test_drain_queue.py
"""
import json, os, sys, tempfile, time
from datetime import datetime, timedelta, timezone

BIN = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                   "..", "bin"))
sys.path.insert(0, BIN)

import drain_queue
import fetch_mirrors


def iso_age(seconds):
    t = datetime.now(timezone.utc) - timedelta(seconds=seconds)
    return t.strftime("%Y-%m-%dT%H:%M:%S.") + f"{t.microsecond // 1000:03d}Z"


class Harness:
    """One drain run against a stub Worker + stub fetch."""

    def __init__(self, events, planned, fetch_results, initial_state=None):
        self.root = tempfile.mkdtemp(prefix="ghb-drain-test-")
        with open(os.path.join(self.root, "plan.json"), "w") as f:
            json.dump([{"repo": r} for r in planned], f)
        if initial_state is not None:
            with open(os.path.join(self.root, "state.json"), "w") as f:
                json.dump(initial_state, f)
        self.events = events
        self.fetch_results = fetch_results  # repo -> "ok"|"fail"|"missing"
        self.fetch_calls = []
        self.ack_calls = []
        self._saved = (drain_queue.worker_req, drain_queue.load_config,
                       drain_queue.backup_root, fetch_mirrors.fetch_repo)
        drain_queue.worker_req = self._worker_req
        drain_queue.load_config = lambda: {
            "worker": {"queue_poll_path": "/queue"},
            # "Owner" is capitalised on purpose: the allowlist compare is
            # case-insensitive, and every case below uses "owner/...".
            "owners": ["Owner", "other", "a", "b", "c"]}
        drain_queue.backup_root = lambda cfg=None: self.root
        fetch_mirrors.fetch_repo = self._fetch_repo

    def _worker_req(self, path, payload=None):
        if path.endswith("/ack"):
            self.ack_calls.append(payload["ids"])
            return {}
        return {"events": self.events}

    def _fetch_repo(self, root, repo, state):
        self.fetch_calls.append(repo)
        res = self.fetch_results[repo]
        st = state.setdefault(repo, {})
        if res == "ok":
            st["last_fetch"] = time.time()
            st["fail_days"] = []
        elif res == "fail":
            st["last_error"] = "stub fetch boom"
        return res

    def run(self):
        try:
            return drain_queue.main()
        finally:
            (drain_queue.worker_req, drain_queue.load_config,
             drain_queue.backup_root,
             fetch_mirrors.fetch_repo) = self._saved

    def acked_ids(self):
        return [i for call in self.ack_calls for i in call]

    def state(self):
        with open(os.path.join(self.root, "state.json")) as f:
            return json.load(f)

    def fetch_log(self):
        p = os.path.join(self.root, "fetch.log")
        return open(p).read() if os.path.exists(p) else ""


def ev(eid, repo, age=60):
    return {"id": eid, "repo": repo, "event": "push", "ts": iso_age(age)}


PASS = []


def check(name, cond):
    PASS.append((name, bool(cond)))
    print(("PASS " if cond else "FAIL ") + name)
    assert cond, name


# 1. success -> acked + stamped
h = Harness([ev("e1", "owner/repo")], ["owner/repo"], {"owner/repo": "ok"})
check("success: exit 0", h.run() == 0)
check("success: event acked", h.acked_ids() == ["e1"])
check("success: coalesce stamp set", "drain_last::owner/repo" in h.state())
check("success: no fetch.log", h.fetch_log() == "")

# 2. fetch fail -> NOT acked, logged with age, no stamp, failure counted
h = Harness([ev("e1", "owner/repo", age=600)], ["owner/repo"],
            {"owner/repo": "fail"})
check("fail: exit 0 (not yet chronic)", h.run() == 0)
check("fail: event NOT acked", h.acked_ids() == [])
log = h.fetch_log()
check("fail: fetch.log line", "owner/repo fail" in log and "stub fetch boom" in log)
check("fail: event age in log", "oldest event age" in log)
check("fail: no coalesce stamp", "drain_last::owner/repo" not in h.state())
check("fail: failure counted",
      h.state()["drain_fail::owner/repo"]["count"] == 1)

# 3. missing mirror -> NOT acked, logged
h = Harness([ev("e1", "owner/repo")], ["owner/repo"], {"owner/repo": "missing"})
check("missing: exit 0", h.run() == 0)
check("missing: event NOT acked", h.acked_ids() == [])
check("missing: fetch.log line", "owner/repo missing" in h.fetch_log())
check("missing: no coalesce stamp", "drain_last::owner/repo" not in h.state())

# 4. unplanned repo -> acked + noted, no fetch attempted
h = Harness([ev("e1", "other/repo")], ["owner/repo"], {})
check("unplanned: exit 0", h.run() == 0)
check("unplanned: event acked", h.acked_ids() == ["e1"])
check("unplanned: fetch.log note", "unplanned repo" in h.fetch_log())
check("unplanned: no fetch attempted", h.fetch_calls == [])

# 5. inside coalesce window -> NOT acked, no fetch; window passed -> fetched
now = time.time()
h = Harness([ev("e1", "owner/repo")], ["owner/repo"], {"owner/repo": "ok"},
            initial_state={"drain_last::owner/repo": now})
check("coalesce: exit 0", h.run() == 0)
check("coalesce: event NOT acked yet", h.acked_ids() == [])
check("coalesce: no fetch attempted", h.fetch_calls == [])
h = Harness([ev("e1", "owner/repo")], ["owner/repo"], {"owner/repo": "ok"},
            initial_state={"drain_last::owner/repo": now - drain_queue.COALESCE_S - 1})
check("coalesce-expired: exit 0", h.run() == 0)
check("coalesce-expired: fetched + acked",
      h.fetch_calls == ["owner/repo"] and h.acked_ids() == ["e1"])

# 6. chronic: threshold-th consecutive failure -> exit 1, loud log, still queued
n = drain_queue.CHRONIC_DRAIN_FAILURES
h = Harness([ev("e1", "owner/repo")], ["owner/repo"], {"owner/repo": "fail"},
            initial_state={"drain_fail::owner/repo": {"count": n - 1, "first": now}})
check("chronic: exit non-zero", h.run() == 1)
check("chronic: event still NOT acked", h.acked_ids() == [])
check("chronic: DRAIN-CHRONIC logged", "DRAIN-CHRONIC" in h.fetch_log())
check("chronic: count advanced", h.state()["drain_fail::owner/repo"]["count"] == n)

# 7. recovery: success clears the failure counter
h = Harness([ev("e1", "owner/repo")], ["owner/repo"], {"owner/repo": "ok"},
            initial_state={"drain_fail::owner/repo": {"count": 3, "first": now}})
check("recovery: exit 0", h.run() == 0)
check("recovery: acked", h.acked_ids() == ["e1"])
check("recovery: failure counter cleared",
      "drain_fail::owner/repo" not in h.state())

# 8. mixed batch: one ok, one fail, one unplanned -> only ok+unplanned acked
h = Harness([ev("e1", "a/ok"), ev("e2", "b/bad"), ev("e3", "c/new")],
            ["a/ok", "b/bad"], {"a/ok": "ok", "b/bad": "fail"})
check("mixed: exit 0", h.run() == 0)
check("mixed: acked = ok + unplanned only",
      sorted(h.acked_ids()) == ["e1", "e3"])

# 9. foreign owner -> dropped + acked + DRAIN-DROP logged, no fetch,
#    even though the repo IS in plan.json (filter precedes the plan check)
import contextlib, io
h = Harness([ev("e9", "stranger/pwned")], ["stranger/pwned"],
            {"stranger/pwned": "ok"})
buf = io.StringIO()
with contextlib.redirect_stdout(buf):
    rc = h.run()
check("foreign: exit 0", rc == 0)
check("foreign: event acked (handled-by-policy)", h.acked_ids() == ["e9"])
check("foreign: no fetch attempted", h.fetch_calls == [])
check("foreign: DRAIN-DROP logged",
      "DRAIN-DROP foreign owner=stranger repo=stranger/pwned delivery=e9"
      in h.fetch_log())
check("foreign: no failure state",
      not any(k.startswith("drain_fail::") for k in h.state()))
summary = json.loads(buf.getvalue().strip().splitlines()[-1])
check("foreign: drop counted in summary", summary.get("dropped_foreign") == 1)

# 10. malformed event, no determinable owner -> dropped + acked + logged
h = Harness([{"id": "e10", "event": "push", "ts": iso_age(60)}],
            ["owner/repo"], {})
check("no-owner: exit 0", h.run() == 0)
check("no-owner: event acked", h.acked_ids() == ["e10"])
check("no-owner: DRAIN-DROP logged",
      "DRAIN-DROP foreign owner=unknown repo=unknown delivery=e10"
      in h.fetch_log())
check("no-owner: no fetch attempted", h.fetch_calls == [])

# 11. repository.owner.login takes precedence over the installation
#     account: an allowed installation does NOT rescue a foreign repo
evf = {"id": "e11", "event": "push", "ts": iso_age(60),
       "repository": {"full_name": "dezren39/real",
                      "owner": {"login": "mallory"}},
       "installation": {"account": {"login": "Owner"}}}
h = Harness([evf], [], {})
check("precedence: exit 0", h.run() == 0)
check("precedence: dropped despite allowed installation",
      "DRAIN-DROP foreign owner=mallory repo=dezren39/real delivery=e11"
      in h.fetch_log())
check("precedence: acked", h.acked_ids() == ["e11"])
check("precedence: no fetch attempted", h.fetch_calls == [])

# 12. installation account is the fallback when the payload carries no
#     repository owner; repo normalises from repository.full_name
evi = {"id": "e12", "event": "push", "ts": iso_age(60),
       "repository": {"full_name": "owner/repo"},
       "installation": {"account": {"login": "owner"}}}
h = Harness([evi], ["owner/repo"], {"owner/repo": "ok"})
check("install-fallback: exit 0", h.run() == 0)
check("install-fallback: fetched via repository full_name",
      h.fetch_calls == ["owner/repo"])
check("install-fallback: acked", h.acked_ids() == ["e12"])
check("install-fallback: not dropped", "DRAIN-DROP" not in h.fetch_log())

# 13. a foreign drop never feeds the chronic-failure streak: a repo one
#     failure short of DRAIN-CHRONIC is dropped quietly, streak untouched
h = Harness([ev("e13", "stranger/pwned")], ["stranger/pwned"],
            {"stranger/pwned": "fail"},
            initial_state={"drain_fail::stranger/pwned":
                           {"count": drain_queue.CHRONIC_DRAIN_FAILURES - 1,
                            "first": now}})
check("foreign-chronic: exit 0 (drop is not a failure)", h.run() == 0)
check("foreign-chronic: acked", h.acked_ids() == ["e13"])
check("foreign-chronic: streak untouched",
      h.state()["drain_fail::stranger/pwned"]["count"]
      == drain_queue.CHRONIC_DRAIN_FAILURES - 1)
check("foreign-chronic: no DRAIN-CHRONIC", "DRAIN-CHRONIC" not in h.fetch_log())

print(f"\n{sum(1 for _, ok in PASS if ok)}/{len(PASS)} checks passed")
