#!/usr/bin/env python3
"""content-ledger — shared creation ledger + verify-by-read filing.

Phase 1 of the fleet content-creation throttle (dezren39/hatch#3).
Design: developing-today/code#2013, record PR developing-today/code#2226,
tracking record developing-today/code#2227. The remaining phases —
throttle wrapper (warn-only then enforce), circuit breaker, comment
reduction, gap detector — are dezren39/hatch#13, NOT this tool: nothing
here delays or refuses a create for pacing reasons.

Every fleet content-creating call (issue, PR, comment) goes through
this helper:

1. RESERVE — a row is written to the shared ledger BEFORE the API call
   (outcome 'reserved', with a unique nonce that is embedded in the
   posted body as <!-- fleet-nonce:… -->).
2. CREATE — the underlying `gh` call runs.
3. VERIFY-BY-READ — success is only what a follow-up read confirms:
   - the create returned a URL/number AND a read of that object shows
     the nonce -> outcome 'created';
   - the create printed nothing usable, but a nonce search finds the
     object -> outcome 'verified_existing';
   - anything else (HTTP/API error, empty output with no object found,
     readback mismatch) -> outcome 'failed', the underlying error is
     preserved in the row's detail and raised to the caller.

Empty output is never success. That is the 2026-10-28 failure mode:
`gh issue create` exited 0 with empty output while creating nothing,
and no caller noticed (code#2013).

Ledger: ~/.local/state/fleet/content-ledger.sqlite (override with
CONTENT_LEDGER_PATH), table events(ts_utc, actor, job, repo, kind, ref,
outcome, nonce, detail). SQLite WAL + BEGIN IMMEDIATE gives the
cross-process atomicity the design requires on one host. A ledger
failure raises LedgerError before any API call is made — callers must
fail visibly, never fall back to unledgered filing.

CLI:
  content_ledger.py create-issue   --repo R --title T --body-file F
                                   [--label L ...] [--job J] [--actor A]
  content_ledger.py create-comment --repo R (--issue N | --pr N)
                                   --body-file F [--job J] [--actor A]
  content_ledger.py create-pr      --repo R --title T --body-file F
                                   --head H [--base B] [--job J] [--actor A]
  content_ledger.py reserve        --repo R --kind K [--job J] [--actor A]
                                   (prints the reservation id)
  content_ledger.py finalize       --id N --outcome created|verified_existing|failed
                                   [--ref URL] [--detail S] [--actor A]
  content_ledger.py counts         [--window SECONDS]   (default 3600)
  content_ledger.py tail           [--limit N]          (default 20)

Exit codes: 0 ok; 1 the create failed (visible, error preserved);
2 usage; 5 ledger failure (nothing was filed).

Env: FLEET_ACTOR / FLEET_JOB set the recorded actor/job defaults.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sqlite3
import subprocess
import sys
import tempfile
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path

DEFAULT_PATH = Path.home() / ".local/state/fleet/content-ledger.sqlite"
KINDS = ("issue", "issue_comment", "pr", "pr_comment", "review", "label_write", "other")
FINAL_OUTCOMES = ("created", "verified_existing", "failed")
URL_RE = re.compile(r"https://github\.com/\S+")
NUM_RE = re.compile(r"/(issues|pull)/(\d+)")
COMMENT_ID_RE = re.compile(r"issuecomment-(\d+)")

SCHEMA = """
CREATE TABLE IF NOT EXISTS events(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts_utc TEXT NOT NULL,
  actor TEXT NOT NULL,
  job TEXT NOT NULL,
  repo TEXT NOT NULL,
  kind TEXT NOT NULL,
  ref TEXT,
  outcome TEXT NOT NULL,
  nonce TEXT,
  detail TEXT
);
CREATE INDEX IF NOT EXISTS events_ts ON events(ts_utc);
CREATE INDEX IF NOT EXISTS events_nonce ON events(nonce);
"""


class LedgerError(RuntimeError):
    """The ledger itself failed. Always fatal, always visible."""


class CreateFailed(RuntimeError):
    """The create call failed verification; detail preserves the cause."""

    def __init__(self, detail: str, reservation_id: int):
        super().__init__(detail)
        self.detail = detail
        self.reservation_id = reservation_id


def now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def new_nonce() -> str:
    return uuid.uuid4().hex


def nonce_marker(nonce: str) -> str:
    return f"<!-- fleet-nonce:{nonce} -->"


def default_actor() -> str:
    return os.environ.get("FLEET_ACTOR") or "dezren39"


def default_job() -> str:
    return os.environ.get("FLEET_JOB") or "cli"


class Ledger:
    def __init__(self, path=None):
        self.path = Path(path or os.environ.get("CONTENT_LEDGER_PATH") or DEFAULT_PATH)
        try:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            self.db = sqlite3.connect(str(self.path), timeout=30, isolation_level=None)
            self.db.execute("PRAGMA journal_mode=WAL")
            self.db.execute("PRAGMA busy_timeout=30000")
            self.db.executescript(SCHEMA)
        except (sqlite3.Error, OSError) as e:
            raise LedgerError(f"cannot open ledger {self.path}: {e}") from e

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
            raise LedgerError(f"ledger write failed: {e}") from e

    def reserve(
        self, actor: str, job: str, repo: str, kind: str, nonce: str | None = None
    ) -> int:
        if kind not in KINDS:
            raise LedgerError(f"unknown kind {kind!r} (want one of {KINDS})")

        def ins():
            cur = self.db.execute(
                "INSERT INTO events(ts_utc, actor, job, repo, kind, outcome,"
                " nonce) VALUES(?,?,?,?,?,'reserved',?)",
                (now_iso(), actor, job, repo, kind, nonce),
            )
            return cur.lastrowid

        return self._txn(ins)

    def finalize(
        self,
        row_id: int,
        outcome: str,
        ref: str | None = None,
        detail: str | None = None,
        actor: str | None = None,
    ) -> None:
        if outcome not in FINAL_OUTCOMES:
            raise LedgerError(
                f"unknown outcome {outcome!r} (want one of {FINAL_OUTCOMES})"
            )

        def upd():
            cur = self.db.execute(
                "UPDATE events SET outcome=?, ref=COALESCE(?, ref),"
                " detail=COALESCE(?, detail), actor=COALESCE(?, actor)"
                " WHERE id=? AND outcome='reserved'",
                (outcome, ref, detail, actor, row_id),
            )
            if cur.rowcount != 1:
                raise LedgerError(
                    f"no reserved row {row_id} to finalize "
                    "(already finalized or never reserved)"
                )

        self._txn(upd)

    def counts(self, window_seconds: int):
        cutoff = (
            datetime.now(timezone.utc) - timedelta(seconds=window_seconds)
        ).strftime("%Y-%m-%dT%H:%M:%SZ")
        try:
            rows = self.db.execute(
                "SELECT outcome, COUNT(*) FROM events WHERE ts_utc>=?"
                " GROUP BY outcome ORDER BY outcome",
                (cutoff,),
            ).fetchall()
        except sqlite3.Error as e:
            raise LedgerError(f"ledger read failed: {e}") from e
        return rows, sum(n for _, n in rows)

    def tail(self, limit: int):
        try:
            cur = self.db.execute(
                "SELECT id, ts_utc, actor, job, repo, kind, ref, outcome"
                " FROM events ORDER BY id DESC LIMIT ?",
                (limit,),
            )
            cols = [c[0] for c in cur.description]
            return [dict(zip(cols, r)) for r in cur.fetchall()]
        except sqlite3.Error as e:
            raise LedgerError(f"ledger read failed: {e}") from e

    def close(self):
        self.db.close()


# ------------------------------------------------------------ gh plumbing
def _run_gh(args: list[str]) -> tuple[int, str, str]:
    r = subprocess.run(["gh", *args], capture_output=True, text=True)
    return r.returncode, r.stdout or "", r.stderr or ""


def _api_json(path: str):
    rc, out, _err = _run_gh(["api", path])
    if rc != 0 or not out.strip():
        return None
    try:
        return json.loads(out)
    except json.JSONDecodeError:
        return None


def _body_of(obj) -> str:
    return (obj or {}).get("body") or ""


def read_object(repo: str, kind: str, number: int):
    """Read back an issue/PR by number; None if it cannot be read."""
    if kind == "pr":
        return _api_json(f"repos/{repo}/pulls/{number}")
    return _api_json(f"repos/{repo}/issues/{number}")


def read_comment(repo: str, comment_id: int):
    return _api_json(f"repos/{repo}/issues/comments/{comment_id}")


def search_nonce(
    repo: str, kind: str, nonce: str, target_number: int | None = None
) -> str | None:
    """Find an object carrying the nonce; return its URL or None."""
    marker = nonce_marker(nonce)
    if kind in ("issue_comment", "pr_comment"):
        if target_number is None:
            return None
        items = _api_json(f"repos/{repo}/issues/{target_number}/comments?per_page=100")
        if not isinstance(items, list):
            return None
        for it in items:
            if marker in _body_of(it):
                return it.get("html_url")
        return None
    if kind == "pr":
        items = _api_json(f"repos/{repo}/pulls?state=all&per_page=100")
    else:
        items = _api_json(f"repos/{repo}/issues?state=all&per_page=100")
    if not isinstance(items, list):
        return None
    for it in items:
        if kind == "issue" and "pull_request" in it:
            continue
        if marker in _body_of(it):
            return it.get("html_url")
    return None


def confirm_created(
    repo: str, kind: str, url: str, nonce: str, target_number: int | None = None
) -> str | None:
    """Verify-by-read for a create that returned a URL.

    Returns 'created' when a read of the object at that URL shows the
    nonce, 'verified_existing' when the direct read fails but the nonce
    search finds the object, and None when no read confirms it.
    """
    obj = None
    if kind in ("issue_comment", "pr_comment"):
        m = COMMENT_ID_RE.search(url)
        if m:
            obj = read_comment(repo, int(m.group(1)))
    else:
        m = NUM_RE.search(url)
        if m:
            obj = read_object(repo, kind, int(m.group(2)))
    if obj is not None and nonce_marker(nonce) in _body_of(obj):
        return "created"
    found = search_nonce(repo, kind, nonce, target_number)
    if found:
        return "verified_existing"
    return None


# ---------------------------------------------------------------- creates
class Created:
    def __init__(self, outcome: str, ref: str, reservation_id: int):
        self.outcome = outcome
        self.ref = ref
        self.reservation_id = reservation_id

    def __repr__(self):
        return f"Created({self.outcome}, {self.ref}, row {self.reservation_id})"


def _create(
    kind: str,
    repo: str,
    body_text: str,
    create_args,
    job: str | None,
    actor: str | None,
    target_number: int | None,
    ledger: Ledger | None,
) -> Created:
    led = ledger or Ledger()
    job = job or default_job()
    actor = actor or default_actor()
    nonce = new_nonce()
    # The reservation lands BEFORE the API call; a LedgerError here
    # propagates and no gh call is made (never file unledgered).
    row = led.reserve(actor, job, repo, kind, nonce)
    effective = body_text.rstrip("\n") + f"\n\n{nonce_marker(nonce)}\n"
    tmp = None
    try:
        fd, tmp = tempfile.mkstemp(prefix="content-ledger-body-", suffix=".md")
        with os.fdopen(fd, "w") as f:
            f.write(effective)
        rc, out, err = _run_gh(create_args(tmp))
    except Exception as e:  # transport-level failure of the gh call itself
        detail = f"gh invocation failed: {e}"
        led.finalize(row, "failed", detail=detail)
        raise CreateFailed(detail, row) from e
    finally:
        if tmp:
            try:
                os.unlink(tmp)
            except OSError:
                pass

    url = None
    m = URL_RE.search(out)
    if rc == 0 and m:
        url = m.group(0).rstrip(").,")
    if url:
        outcome = confirm_created(repo, kind, url, nonce, target_number)
        if outcome:
            led.finalize(row, outcome, ref=url)
            return Created(outcome, url, row)
        detail = (
            f"create returned {url} but no read confirms the object "
            f"(readback mismatch); stderr: {err.strip()[-300:]}"
        )
        led.finalize(row, "failed", detail=detail)
        raise CreateFailed(detail, row)

    # No usable URL: the object may still exist (gh printed nothing).
    found = search_nonce(repo, kind, nonce, target_number)
    if found:
        led.finalize(row, "verified_existing", ref=found)
        return Created("verified_existing", found, row)
    if rc == 0:
        detail = (
            "gh exited 0 with empty/unparsable output and no object "
            "was found by nonce search — the silent-create failure "
            "mode (code#2013)"
        )
        if err.strip():
            detail += f"; stderr: {err.strip()[-300:]}"
    else:
        detail = (err.strip() or out.strip() or f"gh exited {rc}")[-2000:]
    led.finalize(row, "failed", detail=detail)
    raise CreateFailed(detail, row)


def create_issue(
    repo: str, title: str, body_text: str, labels=(), job=None, actor=None, ledger=None
) -> Created:
    def args(body_file):
        a = [
            "issue",
            "create",
            "--repo",
            repo,
            "--title",
            title,
            "--body-file",
            body_file,
        ]
        for lab in labels:
            a += ["--label", lab]
        return a

    return _create("issue", repo, body_text, args, job, actor, None, ledger)


def create_comment(
    repo: str,
    number: int,
    body_text: str,
    pr: bool = False,
    job=None,
    actor=None,
    ledger=None,
) -> Created:
    def args(body_file):
        return [
            "pr" if pr else "issue",
            "comment",
            str(number),
            "--repo",
            repo,
            "--body-file",
            body_file,
        ]

    return _create(
        "pr_comment" if pr else "issue_comment",
        repo,
        body_text,
        args,
        job,
        actor,
        number,
        ledger,
    )


def create_pr(
    repo: str,
    title: str,
    body_text: str,
    head: str,
    base: str = "main",
    job=None,
    actor=None,
    ledger=None,
) -> Created:
    def args(body_file):
        return [
            "pr",
            "create",
            "--repo",
            repo,
            "--title",
            title,
            "--body-file",
            body_file,
            "--head",
            head,
            "--base",
            base,
        ]

    return _create("pr", repo, body_text, args, job, actor, None, ledger)


# -------------------------------------------------------------------- CLI
def _body(args) -> str:
    return Path(args.body_file).read_text(errors="replace")


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(prog="content-ledger", description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)

    def common(p):
        p.add_argument("--repo", required=True)
        p.add_argument("--job")
        p.add_argument("--actor")

    pi = sub.add_parser("create-issue")
    common(pi)
    pi.add_argument("--title", required=True)
    pi.add_argument("--body-file", required=True)
    pi.add_argument("--label", action="append", default=[])
    pc = sub.add_parser("create-comment")
    common(pc)
    pc.add_argument("--issue", type=int)
    pc.add_argument("--pr", type=int)
    pc.add_argument("--body-file", required=True)
    pp = sub.add_parser("create-pr")
    common(pp)
    pp.add_argument("--title", required=True)
    pp.add_argument("--body-file", required=True)
    pp.add_argument("--head", required=True)
    pp.add_argument("--base", default="main")
    pr = sub.add_parser("reserve")
    common(pr)
    pr.add_argument("--kind", required=True, choices=KINDS)
    pf = sub.add_parser("finalize")
    pf.add_argument("--id", type=int, required=True)
    pf.add_argument("--outcome", required=True, choices=FINAL_OUTCOMES)
    pf.add_argument("--ref")
    pf.add_argument("--detail")
    pf.add_argument("--actor")
    pk = sub.add_parser("counts")
    pk.add_argument("--window", type=int, default=3600)
    pt = sub.add_parser("tail")
    pt.add_argument("--limit", type=int, default=20)
    args = ap.parse_args(argv)

    try:
        if args.cmd == "reserve":
            led = Ledger()
            row = led.reserve(
                args.actor or default_actor(),
                args.job or default_job(),
                args.repo,
                args.kind,
                new_nonce(),
            )
            print(row)
            return 0
        if args.cmd == "finalize":
            led = Ledger()
            led.finalize(
                args.id,
                args.outcome,
                ref=args.ref,
                detail=args.detail,
                actor=args.actor,
            )
            print(f"finalized row {args.id}: {args.outcome}")
            return 0
        if args.cmd == "counts":
            led = Ledger()
            rows, total = led.counts(args.window)
            for outcome, n in rows:
                print(f"{outcome}: {n}")
            print(f"total: {total} (window {args.window}s)")
            return 0
        if args.cmd == "tail":
            led = Ledger()
            for r in led.tail(args.limit):
                print(
                    f"{r['id']}\t{r['ts_utc']}\t{r['actor']}\t{r['job']}\t"
                    f"{r['repo']}\t{r['kind']}\t{r['outcome']}\t"
                    f"{r['ref'] or ''}"
                )
            return 0
        if args.cmd == "create-comment" and not (args.issue or args.pr):
            ap.error("create-comment needs --issue N or --pr N")
        led = Ledger()
        if args.cmd == "create-issue":
            res = create_issue(
                args.repo,
                args.title,
                _body(args),
                labels=args.label,
                job=args.job,
                actor=args.actor,
                ledger=led,
            )
        elif args.cmd == "create-comment":
            res = create_comment(
                args.repo,
                args.issue or args.pr,
                _body(args),
                pr=bool(args.pr),
                job=args.job,
                actor=args.actor,
                ledger=led,
            )
        else:
            res = create_pr(
                args.repo,
                args.title,
                _body(args),
                args.head,
                base=args.base,
                job=args.job,
                actor=args.actor,
                ledger=led,
            )
        print(res.ref)
        return 0
    except LedgerError as e:
        print(f"content-ledger: LEDGER FAILURE: {e}", file=sys.stderr)
        return 5
    except CreateFailed as e:
        print(
            f"content-ledger: CREATE FAILED (ledger row "
            f"{e.reservation_id}): {e.detail}",
            file=sys.stderr,
        )
        return 1


if __name__ == "__main__":
    sys.exit(main())
