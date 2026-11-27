#!/usr/bin/env python3
"""content-throttle — shared pacing throttle + circuit breaker for
fleet content creation (dezren39/hatch#13).

The remaining phases of the fleet content-creation throttle, on top of
the phase-1 creation ledger (dezren39/hatch#3, tools/content-ledger).
Design: developing-today/code#2013, record PR developing-today/code#2226,
tracking record developing-today/code#2227. Spec values are carried
intact from code#2227:

  account-wide      45 creations/minute, 300/hour
  spacing           >= 1 s between POSTs
  in flight         <= 8 content calls
  per-lane share    50/hour sustained, idle share reclaimable
  breakage reserve  75/hour held for priority-10 work only
  bulk programs     <= 20 issues per 30-minute window

Rollout (code#2227 order): the throttle ships in WARN-ONLY mode — it
computes exactly what it would delay and logs it to the warn log, but
delays nothing. After one week the warn log is reviewed and the mode
is flipped with `content_throttle.py mode enforce`. The mode and the
warn-week start live in the throttle state DB, not in code.

The circuit breaker is NOT part of the warn-only staging: it reacts to
GitHub's own refusals, and GitHub warns that continuing while limited
can ban the integration (code#2013 §4). It is live in both modes:

  403/429 with retry-after -> sleep that long (capped at 900 s), one retry
  403/429 without it       -> sleep 60 s, then exponential x2, 5 attempts
  5 consecutive failures   -> circuit opens: no content creation for
                              15 minutes; the caller gets a resume
                              brief (park the task, resume after the
                              open-until time). Never spin.

Accounting: budgets are computed from the phase-1 ledger (creations =
rows with outcome reserved|created|verified_existing), plus this
module's own slot table for what the ledger cannot see yet — callers
waiting for a grant, calls in flight, and bulk flags. A throttle or
ledger failure raises ThrottleError BEFORE any API call: callers must
fail visibly, never fall back to unthrottled filing (the phase-1 rule,
applied to pacing).

Gap detector (spec item 6): `gap_check()` / the `gap-check` CLI exit 2
and append to the alerts log when queued filing work exists (stuck
ledger reservations, waiting throttle slots, prepared outbox entries)
and no creation has been seen for > 5 minutes. It belongs in the fleet
monitor: the monitor runs `content_throttle.py gap-check` on its cycle
and alerts on exit 2. There is no monitor in this repo to wire it into;
the CLI + exit code + alerts log are the integration surface.

State: ~/.local/state/fleet/content-throttle.sqlite (override with
CONTENT_THROTTLE_PATH), next to the ledger. Warn log and alerts log
sit beside it (content-throttle-warn.jsonl / content-throttle-alerts
.jsonl). SQLite WAL + BEGIN IMMEDIATE, same as the ledger: the wrapper
is hit by parallel producers on one host.

CLI:
  content_throttle.py reserve  --job J --kind K [--repo R]
                               [--priority] [--bulk] [--max-wait S]
                               (prints the slot id)
  content_throttle.py release  --slot N [--outcome created|failed|
                               rate_limited]
  content_throttle.py create-issue   --repo R --title T --body-file F
                               [--label L ...] [--job J] [--priority]
                               [--bulk]
  content_throttle.py create-comment --repo R (--issue N | --pr N)
                               --body-file F [--job J] [--priority]
  content_throttle.py create-pr      --repo R --title T --body-file F
                               --head H [--base B] [--job J]
  content_throttle.py mode     [warn|enforce]   (read or set the mode)
  content_throttle.py status                    (human-readable state)
  content_throttle.py gap-check [--outbox PATH] (exit 2 = alert)

Exit codes: 0 ok; 1 the create failed (see content-ledger);
2 usage; 5 throttle/ledger infra failure (nothing was filed);
6 visibly refused (max wait exceeded); 7 circuit open.

Env: CONTENT_THROTTLE_MODE=warn|enforce overrides the stored mode;
FLEET_ACTOR / FLEET_JOB set the recorded actor/job defaults (via the
ledger module).
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sqlite3
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "content-ledger"))
try:
    import content_ledger
except Exception as _e:  # noqa: BLE001 — fatal at use, like gh-post
    content_ledger = None
    _LEDGER_IMPORT_ERROR = _e

# ------------------------------------------------------------- spec values
PER_MINUTE = 45
PER_HOUR = 300
SPACING_SECONDS = 1.0
MAX_IN_FLIGHT = 8
LANE_PER_HOUR = 50
BREAKAGE_RESERVE_PER_HOUR = 75
BULK_MAX = 20
BULK_WINDOW_SECONDS = 1800
PRIORITY_LEVEL = 10

BREAKER_FAILURES = 5
BREAKER_OPEN_SECONDS = 900
RETRY_BASE_SECONDS = 60
RETRY_AFTER_CAP_SECONDS = 900

GAP_THRESHOLD_SECONDS = 300
STUCK_RESERVATION_SECONDS = 120
IN_FLIGHT_EXPIRY_SECONDS = 180
WAITING_EXPIRY_SECONDS = 1800
DEFAULT_MAX_WAIT_SECONDS = 1800

CREATION_OUTCOMES = ("reserved", "created", "verified_existing")
LIMIT_RE = re.compile(
    r"secondary rate limit|rate limit|submitted too quickly|"
    r"abuse detection|http 403|http 429|\b403\b|\b429\b",
    re.IGNORECASE,
)
RETRY_AFTER_RE = re.compile(r"retry-after[:\s]+(\d+)", re.IGNORECASE)

STATE_DIR = Path.home() / ".local/state/fleet"
DEFAULT_STATE_PATH = STATE_DIR / "content-throttle.sqlite"

SCHEMA = """
CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE IF NOT EXISTS slots(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  job TEXT NOT NULL,
  kind TEXT NOT NULL,
  repo TEXT,
  priority INTEGER NOT NULL DEFAULT 0,
  bulk INTEGER NOT NULL DEFAULT 0,
  state TEXT NOT NULL,
  created_epoch REAL NOT NULL,
  grant_epoch REAL,
  release_epoch REAL,
  detail TEXT
);
CREATE INDEX IF NOT EXISTS slots_state ON slots(state);
"""


def iso(epoch: float) -> str:
    return datetime.fromtimestamp(epoch, timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def parse_iso(text: str) -> float:
    return (
        datetime.strptime(text, "%Y-%m-%dT%H:%M:%SZ")
        .replace(tzinfo=timezone.utc)
        .timestamp()
    )


class ThrottleError(RuntimeError):
    """Throttle/ledger infrastructure failure. Fatal, visible, exit 5."""


class ThrottleRefused(RuntimeError):
    """Enforcing and the wait exceeded max_wait. Visible refusal, exit 6."""

    def __init__(self, constraint: str, wait_seconds: float):
        super().__init__(
            f"throttle: refused — constraint {constraint} would need a "
            f"{wait_seconds:.0f}s wait (over the max wait); nothing was "
            "filed. Retry later or raise the max wait."
        )
        self.constraint = constraint
        self.wait_seconds = wait_seconds


class CircuitOpen(RuntimeError):
    """The breaker is open. Carries the resume brief. Exit 7."""

    def __init__(self, open_until_epoch: float, failures: int):
        brief = (
            f"circuit open after {failures} consecutive rate-limit "
            f"failures; content creation stopped until "
            f"{iso(open_until_epoch)}. Park the calling task with "
            "this brief and resume after that time."
        )
        super().__init__(f"content-throttle: {brief}")
        self.open_until = open_until_epoch
        self.failures = failures
        self.brief = brief


class RateLimited(RuntimeError):
    """One attempt was refused by GitHub (403/429 class)."""

    def __init__(self, detail: str, retry_after: float | None = None):
        super().__init__(detail)
        self.detail = detail
        self.retry_after = retry_after


class RateLimitExhausted(RuntimeError):
    """Retries per the breaker rules are used up (circuit may be shut)."""

    def __init__(self, detail: str, attempts: int):
        super().__init__(f"rate limited after {attempts} attempt(s): {detail}")
        self.detail = detail
        self.attempts = attempts


def parse_retry_after(detail: str) -> float | None:
    m = RETRY_AFTER_RE.search(detail or "")
    return float(m.group(1)) if m else None


class Throttle:
    def __init__(
        self, state_path=None, ledger_path=None, mode=None, now_fn=None, sleep_fn=None
    ):
        if content_ledger is None:
            raise ThrottleError(
                f"content-ledger module unavailable ({_LEDGER_IMPORT_ERROR})"
            )
        self.state_path = Path(
            state_path or os.environ.get("CONTENT_THROTTLE_PATH") or DEFAULT_STATE_PATH
        )
        self._now = now_fn or time.time
        self._sleep = sleep_fn or time.sleep
        try:
            self.ledger = content_ledger.Ledger(ledger_path)
        except content_ledger.LedgerError as e:
            raise ThrottleError(f"ledger unavailable: {e}") from e
        try:
            self.state_path.parent.mkdir(parents=True, exist_ok=True)
            self.db = sqlite3.connect(
                str(self.state_path), timeout=30, isolation_level=None
            )
            self.db.execute("PRAGMA journal_mode=WAL")
            self.db.execute("PRAGMA busy_timeout=30000")
            self.db.executescript(SCHEMA)
        except (sqlite3.Error, OSError) as e:
            raise ThrottleError(
                f"cannot open throttle state {self.state_path}: {e}"
            ) from e
        self._mode_override = mode
        self._init_meta()

    # ------------------------------------------------------------ state
    def _txn(self, fn):
        try:
            self.db.execute("BEGIN IMMEDIATE")
            out = fn()
            self.db.execute("COMMIT")
            return out
        except sqlite3.Error as e:
            try:
                self.db.execute("ROLLBACK")
            except sqlite3.Error:
                pass
            raise ThrottleError(f"throttle state write failed: {e}") from e

    def _meta_get(self, key):
        row = self.db.execute("SELECT value FROM meta WHERE key=?", (key,)).fetchone()
        return row[0] if row else None

    def _meta_set(self, key, value):
        self.db.execute(
            "INSERT INTO meta(key, value) VALUES(?,?) "
            "ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            (key, str(value)),
        )

    def _init_meta(self):
        now = self._now()

        def init():
            if self._meta_get("mode") is None:
                self._meta_set("mode", "warn")
            if self._meta_get("warn_since") is None:
                self._meta_set("warn_since", iso(now))
            for key, val in (
                ("consecutive_failures", "0"),
                ("circuit_open_until", "0"),
                ("last_grant_epoch", "0"),
                ("last_failure_epoch", "0"),
            ):
                if self._meta_get(key) is None:
                    self._meta_set(key, val)

        self._txn(init)

    @property
    def warn_log_path(self) -> Path:
        return self.state_path.parent / "content-throttle-warn.jsonl"

    @property
    def alerts_log_path(self) -> Path:
        return self.state_path.parent / "content-throttle-alerts.jsonl"

    def _append_jsonl(self, path: Path, record: dict) -> None:
        try:
            with open(path, "a") as f:
                f.write(json.dumps(record) + "\n")
        except OSError as e:
            raise ThrottleError(f"cannot write {path}: {e}") from e

    # --------------------------------------------------------------- mode
    def mode(self) -> str:
        env = os.environ.get("CONTENT_THROTTLE_MODE")
        if env in ("warn", "enforce"):
            return env
        if self._mode_override in ("warn", "enforce"):
            return self._mode_override
        return self._meta_get("mode") or "warn"

    def set_mode(self, mode: str) -> None:
        if mode not in ("warn", "enforce"):
            raise ThrottleError(f"unknown mode {mode!r}")
        self._txn(lambda: self._meta_set("mode", mode))

    # ------------------------------------------------------------ circuit
    def circuit_state(self) -> dict:
        now = self._now()
        open_until = float(self._meta_get("circuit_open_until") or 0)
        failures = int(self._meta_get("consecutive_failures") or 0)
        if open_until and now >= open_until:
            # The 15-minute stop has been served; the breaker resets.
            def reset():
                self._meta_set("circuit_open_until", "0")
                self._meta_set("consecutive_failures", "0")

            self._txn(reset)
            open_until, failures = 0.0, 0
        return {
            "open": bool(open_until and now < open_until),
            "open_until": open_until,
            "failures": failures,
        }

    def _raise_if_circuit_open(self) -> None:
        st = self.circuit_state()
        if st["open"]:
            raise CircuitOpen(st["open_until"], st["failures"])

    def record_success(self) -> None:
        def upd():
            if int(self._meta_get("consecutive_failures") or 0):
                self._meta_set("consecutive_failures", "0")

        self._txn(upd)

    def record_failure(self) -> dict:
        """Record one rate-limit-class failure; open the circuit at 5."""
        now = self._now()
        opened = []

        def upd():
            # Read meta directly: circuit_state() may itself transact
            # (expiry reset), and nested BEGIN IMMEDIATE would fail.
            open_until = float(self._meta_get("circuit_open_until") or 0)
            failures = int(self._meta_get("consecutive_failures") or 0)
            if open_until and now >= open_until:
                open_until, failures = 0.0, 0
                self._meta_set("circuit_open_until", "0")
            failures += 1
            self._meta_set("consecutive_failures", str(failures))
            self._meta_set("last_failure_epoch", str(now))
            if failures >= BREAKER_FAILURES and not open_until:
                open_until = now + BREAKER_OPEN_SECONDS
                self._meta_set("circuit_open_until", str(open_until))
                opened.append(open_until)
            return failures

        failures = self._txn(upd)
        if opened:
            self._append_jsonl(
                self.alerts_log_path,
                {
                    "ts": iso(now),
                    "event": "circuit_open",
                    "failures": failures,
                    "open_until": iso(opened[0]),
                    "note": "content creation stopped for 15 minutes "
                    "(hatch#13 circuit breaker)",
                },
            )
            print(
                f"content-throttle: CIRCUIT OPEN after {failures} "
                f"consecutive rate-limit failures; no content "
                f"creation until {iso(opened[0])}",
                file=sys.stderr,
            )
        return self.circuit_state()

    # --------------------------------------------------------- accounting
    def _ledger_creations(self, window_seconds: float, now: float):
        """(epoch, job) for creation rows inside the window."""
        # Widened by 2 s to match the epoch+1 reading below: the SQL
        # filter must not drop a row the conservative reading counts.
        cutoff = iso(now - window_seconds - 2)
        try:
            out = self.ledger.db.execute(
                "SELECT ts_utc, job FROM events WHERE ts_utc>=? AND outcome IN (?,?,?)",
                (cutoff, *CREATION_OUTCOMES),
            ).fetchall()
        except sqlite3.Error as e:
            raise ThrottleError(f"ledger read failed: {e}") from e
        # Ledger timestamps have one-second resolution (truncated), so
        # a row's true creation instant is up to a second AFTER its
        # recorded epoch. Count each row at epoch+1 — the conservative
        # reading: a creation never leaves a pacing window early.
        return [(parse_iso(ts) + 1.0, job) for ts, job in out]

    def _prune_slots(self, now: float) -> None:
        def prune():
            self.db.execute(
                "UPDATE slots SET state='expired' WHERE state='inflight' "
                "AND grant_epoch < ?",
                (now - IN_FLIGHT_EXPIRY_SECONDS,),
            )
            self.db.execute(
                "UPDATE slots SET state='expired' WHERE state='waiting' "
                "AND created_epoch < ?",
                (now - WAITING_EXPIRY_SECONDS,),
            )
            self.db.execute(
                "DELETE FROM slots WHERE state IN ('done','expired') "
                "AND created_epoch < ?",
                (now - 7200,),
            )

        self._txn(prune)

    @staticmethod
    def _window_wait(epochs_sorted, cap, window, now):
        """Seconds until a windowed cap frees a slot.

        With at least `cap` creations in the window, the cap-th newest
        must age out. When waiting slots (not yet creations) are what
        push the count over the cap, their resolution can't be timed
        from the ledger — return a short re-plan wait instead.
        """
        if len(epochs_sorted) >= cap:
            return epochs_sorted[-cap] + window - now
        return 30.0

    def _plan(self, slot_id: int, job: str, priority: bool, bulk: bool, now: float):
        """Return (wait_seconds, binding_constraint) for granting now."""
        waits = {"none": 0.0}
        # Spacing: >= 1 s between POSTs, account-wide.
        last_grant = float(self._meta_get("last_grant_epoch") or 0)
        if last_grant:
            waits["spacing"] = max(0.0, last_grant + SPACING_SECONDS - now)
        # In flight: <= 8 unexpired granted slots (excluding this one).
        inflight = [
            g
            for (g,) in self.db.execute(
                "SELECT grant_epoch FROM slots WHERE state='inflight' AND id != ?",
                (slot_id,),
            ).fetchall()
        ]
        if len(inflight) >= MAX_IN_FLIGHT:
            waits["inflight_cap"] = min(inflight) + IN_FLIGHT_EXPIRY_SECONDS - now
        # Waiting slots ahead of this one consume budget at grant time.
        waiting = self.db.execute(
            "SELECT COUNT(*) FROM slots WHERE state='waiting' AND id != ?", (slot_id,)
        ).fetchone()[0]
        creations_min = self._ledger_creations(60, now)
        creations_hour = self._ledger_creations(3600, now)
        if len(creations_min) + waiting >= PER_MINUTE:
            epochs = sorted(e for e, _ in creations_min)
            waits["account_minute"] = self._window_wait(epochs, PER_MINUTE, 60, now)
        account_cap = PER_HOUR if priority else (PER_HOUR - BREAKAGE_RESERVE_PER_HOUR)
        if len(creations_hour) + waiting >= account_cap:
            epochs = sorted(e for e, _ in creations_hour)
            waits["account_hour" if priority else "account_hour_reserve"] = (
                self._window_wait(epochs, account_cap, 3600, now)
            )
        # Per-lane share with idle-share reclaim: this lane may use its
        # 50 plus every other lane's unused share of their 50; priority
        # work may additionally draw the 75/hour breakage reserve.
        lane_used = sum(1 for _, j in creations_hour if j == job)
        others = {}
        for _, j in creations_hour:
            if j != job:
                others[j] = others.get(j, 0) + 1
        reclaim = sum(max(0, LANE_PER_HOUR - n) for n in others.values())
        lane_cap = (
            LANE_PER_HOUR + reclaim + (BREAKAGE_RESERVE_PER_HOUR if priority else 0)
        )
        if lane_used + waiting >= lane_cap:
            epochs = sorted(e for e, j in creations_hour if j == job)
            waits["lane_hour"] = self._window_wait(epochs, lane_cap, 3600, now)
        if bulk:
            bulk_epochs = [
                g
                for (g,) in self.db.execute(
                    "SELECT grant_epoch FROM slots WHERE bulk=1 "
                    "AND state IN ('inflight','done') AND grant_epoch >= ?",
                    (now - BULK_WINDOW_SECONDS,),
                ).fetchall()
            ]
            if len(bulk_epochs) >= BULK_MAX:
                waits["bulk_window"] = (
                    sorted(bulk_epochs)[-BULK_MAX] + BULK_WINDOW_SECONDS - now
                )
        binding = max(waits, key=lambda k: waits[k])
        return max(0.0, waits[binding]), binding

    # ------------------------------------------------------------- slots
    def acquire(
        self,
        job: str,
        kind: str,
        repo: str = "",
        priority: bool = False,
        bulk: bool = False,
        max_wait: float | None = None,
    ) -> int:
        """Reserve a pacing slot; returns the slot id once granted.

        Warn-only mode: grants immediately and logs what would have
        been delayed. Enforce mode: sleeps until the budgets allow, up
        to max_wait, then refuses visibly. The circuit breaker blocks
        in both modes.
        """
        if kind not in content_ledger.KINDS:
            raise ThrottleError(
                f"unknown kind {kind!r} (want one of {content_ledger.KINDS})"
            )
        now = self._now()
        self._prune_slots(now)
        self._raise_if_circuit_open()

        def insert():
            cur = self.db.execute(
                "INSERT INTO slots(job, kind, repo, priority, bulk, "
                "state, created_epoch) VALUES(?,?,?,?,?,'waiting',?)",
                (
                    job,
                    kind,
                    repo,
                    PRIORITY_LEVEL if priority else 0,
                    1 if bulk else 0,
                    now,
                ),
            )
            return cur.lastrowid

        slot_id = self._txn(insert)
        max_wait = DEFAULT_MAX_WAIT_SECONDS if max_wait is None else max_wait
        started = now
        try:
            while True:
                now = self._now()
                self._raise_if_circuit_open()
                wait, binding = self._plan(slot_id, job, priority, bulk, now)
                if wait <= 0:

                    def grant():
                        self.db.execute(
                            "UPDATE slots SET state='inflight', "
                            "grant_epoch=? WHERE id=?",
                            (now, slot_id),
                        )
                        self._meta_set("last_grant_epoch", str(now))

                    self._txn(grant)
                    return slot_id
                if self.mode() == "warn":
                    self._append_jsonl(
                        self.warn_log_path,
                        {
                            "ts": iso(now),
                            "mode": "warn",
                            "job": job,
                            "kind": kind,
                            "repo": repo,
                            "priority": bool(priority),
                            "bulk": bool(bulk),
                            "would_wait_seconds": round(wait, 3),
                            "binding_constraint": binding,
                        },
                    )

                    def grant_warn():
                        self.db.execute(
                            "UPDATE slots SET state='inflight', "
                            "grant_epoch=?, detail=? WHERE id=?",
                            (
                                now,
                                f"warn-only: would wait {wait:.1f}s ({binding})",
                                slot_id,
                            ),
                        )
                        self._meta_set("last_grant_epoch", str(now))

                    self._txn(grant_warn)
                    return slot_id
                if now - started >= max_wait:
                    raise ThrottleRefused(binding, wait)
                self._sleep(min(wait, 5.0))
        except (ThrottleRefused, CircuitOpen):

            def done():
                self.db.execute(
                    "UPDATE slots SET state='done', release_epoch=?, "
                    "detail=? WHERE id=?",
                    (self._now(), "not granted", slot_id),
                )

            self._txn(done)
            raise

    def release(self, slot_id: int, outcome: str = "created") -> None:
        def upd():
            cur = self.db.execute(
                "UPDATE slots SET state='done', release_epoch=?, "
                "detail=? WHERE id=? AND state='inflight'",
                (self._now(), f"released: {outcome}", slot_id),
            )
            if cur.rowcount != 1:
                raise ThrottleError(f"no in-flight slot {slot_id} to release")

        self._txn(upd)

    # ------------------------------------------------------ breaker calls
    def call_with_breaker(self, fn):
        """Run fn() under the circuit-breaker rules (design §4).

        fn returns a result or raises RateLimited. With retry-after:
        sleep it (capped), one retry. Without: 60 s then exponential
        x2, five attempts. Five consecutive failures open the circuit
        and raise CircuitOpen with the resume brief.
        """
        attempts = 0
        while True:
            self._raise_if_circuit_open()
            try:
                result = fn()
            except RateLimited as e:
                attempts += 1
                state = self.record_failure()
                if state["open"]:
                    raise CircuitOpen(state["open_until"], state["failures"]) from e
                if e.retry_after is not None:
                    if attempts >= 2:
                        raise RateLimitExhausted(e.detail, attempts) from e
                    self._sleep(min(e.retry_after, RETRY_AFTER_CAP_SECONDS))
                else:
                    if attempts >= BREAKER_FAILURES:
                        raise RateLimitExhausted(e.detail, attempts) from e
                    self._sleep(RETRY_BASE_SECONDS * (2 ** (attempts - 1)))
                continue
            self.record_success()
            return result

    # -------------------------------------------------------- create API
    def _attempt(self, create_fn):
        try:
            return create_fn()
        except content_ledger.CreateFailed as e:
            if LIMIT_RE.search(e.detail or ""):
                raise RateLimited(e.detail, parse_retry_after(e.detail)) from e
            raise

    def _create(self, job, kind, repo, priority, bulk, create_fn, max_wait=None):
        slot_id = self.acquire(
            job, kind, repo, priority=priority, bulk=bulk, max_wait=max_wait
        )
        outcome = "failed"
        try:
            result = self.call_with_breaker(lambda: self._attempt(create_fn))
            outcome = "created"
            return result
        except (CircuitOpen, RateLimitExhausted):
            outcome = "rate_limited"
            raise
        finally:
            self.release(slot_id, outcome)

    def create_issue(
        self,
        repo,
        title,
        body_text,
        labels=(),
        job=None,
        priority=False,
        bulk=False,
        max_wait=None,
    ):
        job = job or content_ledger.default_job()
        return self._create(
            job,
            "issue",
            repo,
            priority,
            bulk,
            lambda: content_ledger.create_issue(
                repo, title, body_text, labels=labels, job=job, ledger=self.ledger
            ),
            max_wait=max_wait,
        )

    def create_comment(
        self, repo, number, body_text, pr=False, job=None, priority=False, max_wait=None
    ):
        job = job or content_ledger.default_job()
        return self._create(
            job,
            "pr_comment" if pr else "issue_comment",
            repo,
            priority,
            False,
            lambda: content_ledger.create_comment(
                repo, number, body_text, pr=pr, job=job, ledger=self.ledger
            ),
            max_wait=max_wait,
        )

    def create_pr(
        self,
        repo,
        title,
        body_text,
        head,
        base="main",
        job=None,
        priority=False,
        max_wait=None,
    ):
        job = job or content_ledger.default_job()
        return self._create(
            job,
            "pr",
            repo,
            priority,
            False,
            lambda: content_ledger.create_pr(
                repo, title, body_text, head, base=base, job=job, ledger=self.ledger
            ),
            max_wait=max_wait,
        )

    # -------------------------------------------------------- gap check
    def gap_check(self, outbox_path=None, now=None) -> dict:
        """Spec item 6: queued filing work + no creation for > 5 min.

        Queued work = ledger reservations stuck past
        STUCK_RESERVATION_SECONDS (a create that reserved and never
        finalised), throttle slots still waiting, and prepared entries
        in the gh-post action outbox (when the file is given/exists).
        """
        now = self._now() if now is None else now
        stuck = []
        try:
            rows = self.ledger.db.execute(
                "SELECT id, ts_utc, job, kind FROM events WHERE outcome='reserved'"
            ).fetchall()
        except sqlite3.Error as e:
            raise ThrottleError(f"ledger read failed: {e}") from e
        for row_id, ts, job, kind in rows:
            if now - parse_iso(ts) > STUCK_RESERVATION_SECONDS:
                stuck.append(
                    {
                        "source": "ledger",
                        "id": row_id,
                        "job": job,
                        "kind": kind,
                        "since": ts,
                    }
                )
        waiting = self.db.execute(
            "SELECT COUNT(*) FROM slots WHERE state='waiting'"
        ).fetchone()[0]
        outbox_pending = 0
        if outbox_path and Path(outbox_path).exists():
            text = Path(outbox_path).read_text(errors="replace")
            pending = text.split("## Done")[0]
            outbox_pending = pending.count("- status: prepared")
        last_creation = None
        try:
            row = self.ledger.db.execute(
                "SELECT MAX(ts_utc) FROM events WHERE outcome IN "
                "('created','verified_existing')"
            ).fetchone()
            if row and row[0]:
                last_creation = parse_iso(row[0])
        except sqlite3.Error as e:
            raise ThrottleError(f"ledger read failed: {e}") from e
        queued = len(stuck) + waiting + outbox_pending
        gap = None if last_creation is None else now - last_creation
        alert = bool(queued) and (gap is None or gap > GAP_THRESHOLD_SECONDS)
        report = {
            "ts": iso(now),
            "alert": alert,
            "queued": {
                "stuck_reservations": len(stuck),
                "waiting_slots": waiting,
                "outbox_prepared": outbox_pending,
            },
            "last_creation": (
                iso(last_creation) if last_creation is not None else None
            ),
            "gap_seconds": (None if gap is None else round(gap, 1)),
            "threshold_seconds": GAP_THRESHOLD_SECONDS,
            "stuck": stuck[:10],
        }
        if alert:
            self._append_jsonl(self.alerts_log_path, {"event": "gap", **report})
        return report

    def status(self) -> dict:
        now = self._now()
        circuit = self.circuit_state()
        return {
            "mode": self.mode(),
            "warn_since": self._meta_get("warn_since"),
            "circuit": circuit,
            "creations_last_minute": len(self._ledger_creations(60, now)),
            "creations_last_hour": len(self._ledger_creations(3600, now)),
            "budgets": {
                "per_minute": PER_MINUTE,
                "per_hour": PER_HOUR,
                "lane_per_hour": LANE_PER_HOUR,
                "breakage_reserve": BREAKAGE_RESERVE_PER_HOUR,
                "bulk_max": BULK_MAX,
                "bulk_window_seconds": BULK_WINDOW_SECONDS,
            },
        }


# -------------------------------------------------------------------- CLI
def _body(args) -> str:
    return Path(args.body_file).read_text(errors="replace")


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(prog="content-throttle", description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)

    pr = sub.add_parser("reserve")
    pr.add_argument("--job", required=True)
    pr.add_argument(
        "--kind", required=True, choices=content_ledger.KINDS if content_ledger else []
    )
    pr.add_argument("--repo", default="")
    pr.add_argument("--priority", action="store_true")
    pr.add_argument("--bulk", action="store_true")
    pr.add_argument("--max-wait", type=float)
    pf = sub.add_parser("release")
    pf.add_argument("--slot", type=int, required=True)
    pf.add_argument(
        "--outcome", default="created", choices=("created", "failed", "rate_limited")
    )
    pi = sub.add_parser("create-issue")
    pi.add_argument("--repo", required=True)
    pi.add_argument("--title", required=True)
    pi.add_argument("--body-file", required=True)
    pi.add_argument("--label", action="append", default=[])
    pi.add_argument("--job")
    pi.add_argument("--priority", action="store_true")
    pi.add_argument("--bulk", action="store_true")
    pi.add_argument("--max-wait", type=float)
    pc = sub.add_parser("create-comment")
    pc.add_argument("--repo", required=True)
    pc.add_argument("--issue", type=int)
    pc.add_argument("--pr", type=int)
    pc.add_argument("--body-file", required=True)
    pc.add_argument("--job")
    pc.add_argument("--priority", action="store_true")
    pc.add_argument("--max-wait", type=float)
    pp = sub.add_parser("create-pr")
    pp.add_argument("--repo", required=True)
    pp.add_argument("--title", required=True)
    pp.add_argument("--body-file", required=True)
    pp.add_argument("--head", required=True)
    pp.add_argument("--base", default="main")
    pp.add_argument("--job")
    pp.add_argument("--max-wait", type=float)
    pm = sub.add_parser("mode")
    pm.add_argument("mode", nargs="?", choices=("warn", "enforce"))
    sub.add_parser("status")
    pg = sub.add_parser("gap-check")
    pg.add_argument("--outbox")
    args = ap.parse_args(argv)

    try:
        th = Throttle()
        if args.cmd == "reserve":
            slot = th.acquire(
                args.job,
                args.kind,
                args.repo,
                priority=args.priority,
                bulk=args.bulk,
                max_wait=args.max_wait,
            )
            print(slot)
            return 0
        if args.cmd == "release":
            th.release(args.slot, args.outcome)
            print(f"released slot {args.slot}: {args.outcome}")
            return 0
        if args.cmd == "mode":
            if args.mode:
                th.set_mode(args.mode)
            st = th.status()
            print(f"mode: {st['mode']} (warn-only since {st['warn_since']})")
            return 0
        if args.cmd == "status":
            print(json.dumps(th.status(), indent=2))
            return 0
        if args.cmd == "gap-check":
            outbox = args.outbox or str(
                Path.home() / "workspace/goals/keep-the-four-repos-"
                "healthy/hidden_files/action-outbox.md"
            )
            report = th.gap_check(outbox_path=outbox)
            if report["alert"]:
                print(
                    f"ALERT gap: queued filing work "
                    f"{report['queued']} and no creation for "
                    f"{report['gap_seconds']}s "
                    f"(threshold {report['threshold_seconds']}s; "
                    f"last creation {report['last_creation']})"
                )
                return 2
            print(
                f"ok: no filing gap (queued {report['queued']}, "
                f"last creation {report['last_creation']})"
            )
            return 0
        if args.cmd == "create-comment" and not (args.issue or args.pr):
            ap.error("create-comment needs --issue N or --pr N")
        job = args.job or content_ledger.default_job()
        if args.cmd == "create-issue":
            res = th.create_issue(
                args.repo,
                args.title,
                _body(args),
                labels=args.label,
                job=job,
                priority=args.priority,
                bulk=args.bulk,
                max_wait=args.max_wait,
            )
        elif args.cmd == "create-comment":
            res = th.create_comment(
                args.repo,
                args.issue or args.pr,
                _body(args),
                pr=bool(args.pr),
                job=job,
                priority=args.priority,
                max_wait=args.max_wait,
            )
        else:
            res = th.create_pr(
                args.repo,
                args.title,
                _body(args),
                args.head,
                base=args.base,
                job=job,
                max_wait=args.max_wait,
            )
        print(res.ref)
        return 0
    except CircuitOpen as e:
        print(f"content-throttle: {e}", file=sys.stderr)
        return 7
    except ThrottleRefused as e:
        print(f"content-throttle: {e}", file=sys.stderr)
        return 6
    except RateLimitExhausted as e:
        print(f"content-throttle: {e}", file=sys.stderr)
        return 1
    except ThrottleError as e:
        print(f"content-throttle: THROTTLE FAILURE: {e}", file=sys.stderr)
        return 5
    except content_ledger.LedgerError as e:
        print(f"content-throttle: LEDGER FAILURE: {e}", file=sys.stderr)
        return 5
    except content_ledger.CreateFailed as e:
        print(
            f"content-throttle: CREATE FAILED (ledger row "
            f"{e.reservation_id}): {e.detail}",
            file=sys.stderr,
        )
        return 1


if __name__ == "__main__":
    sys.exit(main())
