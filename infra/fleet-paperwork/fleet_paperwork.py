#!/usr/bin/env python3
"""fleet-paperwork — file GitHub issues as the `developing.today` GitHub App.

Why this exists: on 2026-11-01 a batch of ~322 issue creations through a user
token (`gh`) tripped GitHub's *secondary* rate limit for that user, and two
creates returned exit 0 with empty output while creating nothing. A GitHub App
installation token is a different actor with its own rate-limit bucket, and
every write here is verified by readback — a filing is verified, or it is a
failure. See README.md in this directory.

Auth flow (same one the pm-stack services use):
  1. Read the App private key from the sops-encrypted secrets file
     (default ~/secrets/pm-stack.sops.yaml, keys "github_app_id" and
     "github_private_key_base64"). The key value is never printed, never
     written to the repo, and is held on disk only as a 0600 temp file for
     the seconds it takes openssl to sign one JWT.
  2. Sign a short-lived RS256 JWT (openssl) and exchange it for an
     installation token scoped to the installation that owns the target repo
     (org installation for developing-today/*, personal for dezren39/*,
     resolved live from GET /app/installations — never hardcoded).
  3. POST the issue/comment/close with that token, then GET the result back
     and compare. Missing, empty, or mismatched readback = nonzero exit.

Every successful write is appended to a local ledger
(default ~/.local/state/fleet-paperwork/ledger.tsv) and creates are paced:
before each write the script waits out the remainder of --pace seconds since
the previous ledger-recorded write, so batches cannot burst.

The app now also holds contents/pull_requests write, so two generic commands
expose it beyond issue filing, without hand-rolling the token flow each time:

  token  print the installation token for a repo's owner (stdout only), for
         GH_TOKEN=$(... token --repo owner/name) gh ... or curl.
  api    one arbitrary REST call as the app (PR create/edit, reviews,
         comments anywhere). --method is required for every call, so no
         write can happen by default; the caller owns readback verification.

Usage:
  fleet_paperwork.py check [--repo owner/name]
  fleet_paperwork.py create --repo owner/name --title T --body-file F \
      [--label L ...] [--no-dedupe] [--dry-run]
  fleet_paperwork.py comment --repo owner/name --issue N --body-file F [--dry-run]
  fleet_paperwork.py close --repo owner/name --issue N [--dry-run]
  fleet_paperwork.py token --repo owner/name
  fleet_paperwork.py api --repo owner/name --method GET --path /repos/owner/name
  fleet_paperwork.py api --repo owner/name --method POST \
      --path /repos/owner/name/pulls --input pr.json
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

API = "https://api.github.com"
DEFAULT_SOPS = "~/secrets/pm-stack.sops.yaml"
DEFAULT_LEDGER = "~/.local/state/fleet-paperwork/ledger.tsv"
APP_SLUG_HINT = "developing.today (App ID read from the sops file at runtime)"


def die(msg: str) -> "SystemExit":
    print(f"fleet-paperwork: error: {msg}", file=sys.stderr)
    raise SystemExit(1)


def b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")


# --------------------------------------------------------------------------
# Credential + token flow
# --------------------------------------------------------------------------


def sops_extract(sops_path: str, key: str) -> str:
    out = subprocess.run(
        ["sops", "-d", "--extract", f'["{key}"]', sops_path],
        capture_output=True,
        text=True,
    )
    if out.returncode != 0:
        die(f"sops extract of {key!r} from {sops_path} failed: {out.stderr.strip()}")
    value = out.stdout.strip()
    if not value:
        die(f"sops key {key!r} in {sops_path} is empty")
    return value


def app_jwt(app_id: str, key_b64: str) -> str:
    """RS256 JWT for the App, signed with openssl. Key touches disk only as a
    0600 file inside a temp dir that is removed immediately after signing."""
    try:
        pem = base64.b64decode(key_b64)
    except Exception:
        die("github_private_key_base64 did not base64-decode")
    now = int(time.time())
    header = b64url(json.dumps({"alg": "RS256", "typ": "JWT"}).encode())
    payload = b64url(
        json.dumps({"iat": now - 60, "exp": now + 540, "iss": app_id}).encode()
    )
    signing_input = f"{header}.{payload}".encode()
    tmpdir = tempfile.mkdtemp(prefix="fleet-paperwork-")
    keyfile = os.path.join(tmpdir, "key.pem")
    try:
        with open(keyfile, "wb") as fh:
            fh.write(pem)
        os.chmod(keyfile, 0o600)
        sig = subprocess.run(
            ["openssl", "dgst", "-sha256", "-sign", keyfile],
            input=signing_input,
            capture_output=True,
        )
        if sig.returncode != 0:
            die(
                f"openssl JWT signing failed: {sig.stderr.decode(errors='replace').strip()}"
            )
    finally:
        try:
            os.unlink(keyfile)
        except OSError:
            pass
        try:
            os.rmdir(tmpdir)
        except OSError:
            pass
    return f"{header}.{payload}.{b64url(sig.stdout)}"


def http_raw(
    method: str, path: str, token: str, data: bytes | None = None
) -> tuple[int, str]:
    """One HTTP call that reports HTTP errors instead of dying on them.

    Returns (status, raw body text). Transport failures (DNS, TLS, refused)
    still die — there is no status to report for them. Used directly by the
    `api` command; `http` below is the die-on-error wrapper the issue
    commands have always used.
    """
    url = path if path.startswith("http") else API + path
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Authorization", f"Bearer {token}")
    req.add_header("Accept", "application/vnd.github+json")
    req.add_header("X-GitHub-Api-Version", "2022-11-28")
    if data is not None:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req) as resp:
            return resp.status, resp.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode(errors="replace")
    except urllib.error.URLError as e:
        die(f"{method} {url} failed: {e.reason}")
    raise AssertionError("unreachable")


def _shape_name(value: object) -> str:
    """The JSON shape of a parsed body, for diagnostics (#6864)."""
    if isinstance(value, dict):
        return "object"
    if isinstance(value, list):
        return "array"
    if isinstance(value, str):
        return "string"
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, (int, float)):
        return "number"
    if value is None:
        return "null"
    return type(value).__name__


def http(
    method: str,
    path: str,
    token: str,
    body: dict | None = None,
    expect: type = dict,
) -> tuple[int, dict | list]:
    """Die-on-error wrapper over http_raw with shape validation (#6864).

    A parseable-but-wrong-shaped body — an array, string, or number
    where the caller needs an object (or the reverse) — dies here with
    a diagnostic naming the method, URL, HTTP status, and received
    shape, instead of crashing every consumer downstream with a raw
    AttributeError/KeyError. The error path is held to the same rule:
    a non-object error body surfaces the HTTP status, the shape, and
    the raw excerpt, never a crash inside the error handler itself.
    `expect` is the one legitimate exception carved out by name:
    /app/installations returns a top-level array.
    """
    url = path if path.startswith("http") else API + path
    data = json.dumps(body).encode() if body is not None else None
    status, raw = http_raw(method, path, token, data)
    if status >= 400:
        try:
            detail = json.loads(raw)
        except Exception:
            detail = {"raw": raw}
        if isinstance(detail, dict):
            die(f"{method} {url} -> HTTP {status}: {detail.get('message', raw[:300])}")
        die(
            f"{method} {url} -> HTTP {status}: expected a JSON object error "
            f"body, got {_shape_name(detail)}: {raw[:300]}"
        )
    if not raw:
        payload: object = {}
    else:
        try:
            payload = json.loads(raw)
        except Exception:
            die(f"{method} {url} -> HTTP {status}: response body is not JSON: {raw[:300]}")
    if not isinstance(payload, expect):
        die(
            f"{method} {url} -> HTTP {status}: expected a JSON "
            f"{'object' if expect is dict else 'array'}, got {_shape_name(payload)}"
        )
    return status, payload


def _account_login(inst: dict) -> str:
    """An installation's account login, or "" for any other shape: an
    absent account, an explicit null, or a non-string login (#6864)."""
    account = inst.get("account")
    if isinstance(account, dict):
        login = account.get("login")
        if isinstance(login, str):
            return login
    return ""


class AppClient:
    def __init__(self, sops_path: str):
        self.app_id = sops_extract(sops_path, "github_app_id")
        key_b64 = sops_extract(sops_path, "github_private_key_base64")
        self.jwt = app_jwt(self.app_id, key_b64)
        self._tokens: dict[int, str] = {}
        self._installations: list[dict] | None = None

    def installations(self) -> list[dict]:
        if self._installations is None:
            _, data = http(
                "GET", "/app/installations?per_page=100", self.jwt, expect=list
            )
            for inst in data:
                if not isinstance(inst, dict):
                    die(
                        "installation entry is not a JSON object "
                        f"(got {_shape_name(inst)})"
                    )
            self._installations = data
        return self._installations

    def installation_for(self, owner: str) -> dict:
        for inst in self.installations():
            if _account_login(inst).lower() == owner.lower():
                return inst
        die(
            f"no installation of the app found for owner {owner!r} "
            f"(installations: {[_account_login(i) for i in self.installations()]})"
        )
        raise AssertionError("unreachable")

    def token_for(self, owner: str) -> str:
        inst = self.installation_for(owner)
        iid = inst.get("id")
        if isinstance(iid, bool) or not isinstance(iid, int):
            die(f"installation for owner {owner!r} has no numeric id")
        if iid not in self._tokens:
            _, data = http(
                "POST", f"/app/installations/{iid}/access_tokens", self.jwt, {}
            )
            token = data.get("token")
            if not token:
                die(f"installation token mint for installation {iid} returned no token")
            self._tokens[iid] = token
        return self._tokens[iid]

    def assert_repo_in_scope(self, repo: str) -> None:
        owner = repo.split("/")[0]
        token = self.token_for(owner)
        page, seen = 1, []
        while True:
            _, data = http(
                "GET", f"/installation/repositories?per_page=100&page={page}", token
            )
            repos = data.get("repositories", [])
            if not isinstance(repos, list):
                die(
                    "installation repositories response is not a JSON array "
                    f"(got {_shape_name(repos)})"
                )
            for r in repos:
                if not isinstance(r, dict) or not isinstance(r.get("full_name"), str):
                    die(
                        "installation repository entry has no string "
                        f"full_name (got {_shape_name(r)}): {str(r)[:120]}"
                    )
                seen.append(r["full_name"])
            total = data.get("total_count", 0)
            if isinstance(total, bool) or not isinstance(total, (int, float)):
                die(
                    "installation repositories total_count is not a number "
                    f"(got {_shape_name(total)})"
                )
            if len(seen) >= total or not repos:
                break
            page += 1
        if repo not in seen:
            die(
                f"repo {repo} is not in the app installation scope for {owner} "
                f"(in scope: {sorted(seen)})"
            )


# --------------------------------------------------------------------------
# Ledger + pacing
# --------------------------------------------------------------------------


def ledger_path() -> Path:
    return Path(os.environ.get("FLEET_PAPERWORK_LEDGER", DEFAULT_LEDGER)).expanduser()


def last_write_epoch(path: Path) -> float | None:
    try:
        lines = path.read_text().strip().splitlines()
    except OSError:
        return None
    if not lines:
        return None
    try:
        return datetime.fromisoformat(lines[-1].split("\t")[0]).timestamp()
    except Exception:
        return None


def pace(pace_secs: float) -> None:
    last = last_write_epoch(ledger_path())
    if last is None or pace_secs <= 0:
        return
    wait = pace_secs - (time.time() - last)
    if wait > 0:
        time.sleep(wait)


def ledger_append(action: str, repo: str, number: str, detail: str) -> None:
    path = ledger_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    ts = datetime.now(timezone.utc).isoformat(timespec="milliseconds")
    with path.open("a") as fh:
        fh.write(f"{ts}\t{action}\t{repo}\t{number}\t{detail}\n")


# --------------------------------------------------------------------------
# Dedupe
# --------------------------------------------------------------------------


def find_by_exact_title(client: AppClient, repo: str, title: str) -> dict | None:
    """Search-API lookup, filtered to byte-exact title equality (the search
    itself matches title+body text, so the filter is what makes it a dedupe).

    Issues only, on two layers (#6697): the query carries `type:issue` AND
    the filter skips items with a `pull_request` key, because the search
    index contains PRs and a same-titled PR is not an already-filed issue —
    treating it as one made `create` exit 0 having filed nothing. The scan
    pages through the matches (the Search API serves at most 1000 results)
    so an exact-title issue past the first page is still found."""
    token = client.token_for(repo.split("/")[0])
    q = urllib.parse.quote(f'repo:{repo} type:issue "{title}"')
    for page in range(1, 11):
        _, data = http(
            "GET", f"/search/issues?q={q}&per_page=100&page={page}", token
        )
        items = data.get("items", [])
        if not isinstance(items, list):
            die(f"search response items is not a JSON array (got {_shape_name(items)})")
        for item in items:
            if not isinstance(item, dict):
                continue
            if "pull_request" in item:
                continue
            if item.get("title") == title:
                return item
        if len(items) < 100:
            return None
    return None


# --------------------------------------------------------------------------
# Commands
# --------------------------------------------------------------------------


def read_body(args) -> str:
    if args.body_file:
        return Path(args.body_file).read_text()
    if args.body is not None:
        return args.body
    die("one of --body-file or --body is required")
    raise AssertionError("unreachable")


def cmd_check(args) -> int:
    client = AppClient(args.sops)
    print(f"app id {client.app_id} ({APP_SLUG_HINT})")
    for inst in client.installations():
        print(
            f"installation {inst.get('id')}: {_account_login(inst)} "
            f"permissions={inst.get('permissions')}"
        )
    if args.repo:
        client.assert_repo_in_scope(args.repo)
        print(f"{args.repo}: in installation scope")
    return 0


def cmd_create(args) -> int:
    client = AppClient(args.sops)
    client.assert_repo_in_scope(args.repo)
    title = args.title
    existing = find_by_exact_title(client, args.repo, title)
    if existing is not None:
        if args.no_dedupe:
            print(
                f"note: exact-title issue already exists: #{existing['number']} "
                f"({existing['html_url']}); --no-dedupe set, creating another"
            )
        else:
            print(
                f"already filed: {args.repo}#{existing['number']} "
                f"{existing['html_url']} (dedupe hit, nothing created)"
            )
            return 0
    body = read_body(args)
    payload: dict = {"title": title, "body": body}
    if args.label:
        payload["labels"] = args.label
    if args.dry_run:
        print(
            f"dry-run: would POST /repos/{args.repo}/issues title={title!r} "
            f"labels={args.label or []} body={len(body)} chars"
        )
        return 0
    pace(args.pace)
    token = client.token_for(args.repo.split("/")[0])
    _, created = http("POST", f"/repos/{args.repo}/issues", token, payload)
    number = created.get("number")
    if not number:
        die(
            f"create in {args.repo} returned no issue number (response keys: "
            f"{sorted(created) if isinstance(created, dict) else type(created)})"
        )
    # Verify by readback — an unverified create counts as a failure. The
    # README promises the comparison covers (title, body, state) and the
    # labels that were sent (#6901): a create whose body was dropped or
    # mangled, or whose labels were not applied, must not report success.
    _, got = http("GET", f"/repos/{args.repo}/issues/{number}", token)
    if got.get("title") != title or got.get("number") != number:
        die(f"readback mismatch for {args.repo}#{number}: title={got.get('title')!r}")
    readback_body = got.get("body")
    if readback_body != body:
        sent_chars = len(body) if isinstance(body, str) else "n/a"
        readback_chars = (
            len(readback_body) if isinstance(readback_body, str) else "n/a"
        )
        die(
            f"readback mismatch for {args.repo}#{number}: body sent "
            f"{sent_chars} chars, read back {readback_chars} chars "
            f"(body={_shape_name(readback_body)})"
        )
    if got.get("state") != "open":
        die(
            f"readback mismatch for {args.repo}#{number}: state={got.get('state')!r}, "
            f"expected 'open' for a freshly created issue"
        )
    if args.label:
        readback_labels = got.get("labels", [])
        if not isinstance(readback_labels, list):
            die(
                f"readback mismatch for {args.repo}#{number}: labels have invalid "
                f"shape (expected array, got {_shape_name(readback_labels)})"
            )
        have = set()
        for label in readback_labels:
            name = label if isinstance(label, str) else (
                label.get("name") if isinstance(label, dict) else None
            )
            if not isinstance(name, str):
                die(
                    f"readback mismatch for {args.repo}#{number}: label entry has "
                    f"no string name (got {_shape_name(label)})"
                )
            have.add(name)
        missing = sorted(set(args.label) - have)
        if missing:
            die(
                f"readback mismatch for {args.repo}#{number}: labels not applied: "
                f"{missing} (read back {sorted(have)})"
            )
    ledger_append("create", args.repo, str(number), title)
    print(f"created+verified: {args.repo}#{number} {created.get('html_url')}")
    return 0


def cmd_comment(args) -> int:
    client = AppClient(args.sops)
    client.assert_repo_in_scope(args.repo)
    body = read_body(args)
    if args.dry_run:
        print(
            f"dry-run: would POST comment on {args.repo}#{args.issue} "
            f"({len(body)} chars)"
        )
        return 0
    pace(args.pace)
    token = client.token_for(args.repo.split("/")[0])
    _, comment = http(
        "POST",
        f"/repos/{args.repo}/issues/{args.issue}/comments",
        token,
        {"body": body},
    )
    cid = comment.get("id")
    if not cid:
        die(f"comment on {args.repo}#{args.issue} returned no comment id")
    _, got = http("GET", f"/repos/{args.repo}/issues/comments/{cid}", token)
    if got.get("id") != cid or got.get("body") != body:
        die(f"comment readback mismatch on {args.repo}#{args.issue} (comment {cid})")
    ledger_append("comment", args.repo, str(args.issue), f"comment {cid}")
    print(
        f"commented+verified: {args.repo}#{args.issue} comment {cid} "
        f"{comment.get('html_url')}"
    )
    return 0


def cmd_token(args) -> int:
    """Print the installation token for the repo's owner — the token and
    nothing else on stdout (all diagnostics go to stderr), so callers can do
    GH_TOKEN=$(fleet_paperwork.py token --repo owner/name) gh .... The repo
    must be in the installation's scope, same check as every other command,
    so a typo'd repo fails here instead of as a confusing 404 downstream."""
    client = AppClient(args.sops)
    client.assert_repo_in_scope(args.repo)
    print(client.token_for(args.repo.split("/")[0]))
    return 0


def cmd_api(args) -> int:
    """One arbitrary REST call as the app, for everything the issue commands
    do not cover (PR create/edit, reviews, comments anywhere). The HTTP
    status goes to stderr and the response body to stdout; HTTP >= 400
    prints both and exits nonzero. Unlike the issue commands there is no
    readback verification — the caller owns verifying what it did."""
    if not (args.path.startswith("/") or args.path.startswith("http")):
        die(f"--path must be a REST path starting with '/': {args.path!r}")
    client = AppClient(args.sops)
    client.assert_repo_in_scope(args.repo)
    token = client.token_for(args.repo.split("/")[0])
    data = None
    if args.input:
        try:
            payload = json.loads(Path(args.input).read_text())
        except (OSError, json.JSONDecodeError) as e:
            die(f"--input {args.input}: not readable JSON: {e}")
        data = json.dumps(payload).encode()
    status, raw = http_raw(args.method, args.path, token, data)
    print(f"{args.method} {args.path} -> HTTP {status}", file=sys.stderr)
    if raw:
        try:
            print(json.dumps(json.loads(raw), indent=2))
        except Exception:
            print(raw)
    return 0 if status < 400 else 1


def cmd_close(args) -> int:
    client = AppClient(args.sops)
    client.assert_repo_in_scope(args.repo)
    token = client.token_for(args.repo.split("/")[0])
    _, current = http("GET", f"/repos/{args.repo}/issues/{args.issue}", token)
    if current.get("state") == "closed":
        print(f"already closed: {args.repo}#{args.issue}")
        return 0
    if args.dry_run:
        print(f"dry-run: would close {args.repo}#{args.issue}")
        return 0
    pace(args.pace)
    http("PATCH", f"/repos/{args.repo}/issues/{args.issue}", token, {"state": "closed"})
    _, got = http("GET", f"/repos/{args.repo}/issues/{args.issue}", token)
    if got.get("state") != "closed":
        die(
            f"close readback mismatch for {args.repo}#{args.issue}: "
            f"state={got.get('state')!r}"
        )
    ledger_append("close", args.repo, str(args.issue), current.get("title", ""))
    print(f"closed+verified: {args.repo}#{args.issue}")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        prog="fleet-paperwork",
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument(
        "--sops",
        default=os.environ.get("FLEET_PAPERWORK_SOPS", DEFAULT_SOPS),
        help=f"sops file holding the app key (default: {DEFAULT_SOPS})",
    )
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("check", help="read-only: app, installations, repo scope")
    p.add_argument("--repo")
    p.set_defaults(fn=cmd_check)

    p = sub.add_parser("create", help="create an issue (deduped, verified)")
    p.add_argument("--repo", required=True)
    p.add_argument("--title", required=True)
    p.add_argument("--body-file")
    p.add_argument("--body")
    p.add_argument("--label", action="append", default=[])
    p.add_argument("--no-dedupe", action="store_true")
    p.add_argument("--pace", type=float, default=3.0)
    p.add_argument("--dry-run", action="store_true")
    p.set_defaults(fn=cmd_create)

    p = sub.add_parser("comment", help="comment on an issue (verified)")
    p.add_argument("--repo", required=True)
    p.add_argument("--issue", required=True, type=int)
    p.add_argument("--body-file")
    p.add_argument("--body")
    p.add_argument("--pace", type=float, default=3.0)
    p.add_argument("--dry-run", action="store_true")
    p.set_defaults(fn=cmd_comment)

    p = sub.add_parser(
        "token", help="print the installation token for a repo's owner (stdout only)"
    )
    p.add_argument("--repo", required=True)
    p.set_defaults(fn=cmd_token)

    p = sub.add_parser("api", help="one arbitrary REST call as the app")
    p.add_argument("--repo", required=True)
    p.add_argument(
        "--method",
        required=True,
        type=str.upper,
        help="HTTP method — required for every call, so a write "
        "can never happen by default",
    )
    p.add_argument(
        "--path", required=True, help="REST path, e.g. /repos/owner/name/pulls"
    )
    p.add_argument("--input", help="file holding the JSON request body")
    p.set_defaults(fn=cmd_api)

    p = sub.add_parser("close", help="close an issue (verified)")
    p.add_argument("--repo", required=True)
    p.add_argument("--issue", required=True, type=int)
    p.add_argument("--pace", type=float, default=3.0)
    p.add_argument("--dry-run", action="store_true")
    p.set_defaults(fn=cmd_close)

    args = ap.parse_args(argv)
    args.sops = str(Path(args.sops).expanduser())
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
