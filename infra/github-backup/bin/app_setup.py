#!/usr/bin/env python3
"""app_setup.py — GitHub App (developing-today-backup) lifecycle helper.

Subcommands:
  manifest   Print the App Manifest JSON + the exact click-path to create
             the App (GitHub requires a human to submit the manifest form;
             everything after the redirect is automated here).
  convert CODE
             Exchange the manifest-flow temporary code for the App's
             credentials (POST /app-manifests/<code>/conversions) and store
             them in Bitwarden Secrets Manager BY NAME (see config.json
             app.bitwarden_secret_names). Values are never printed or
             written to disk.
  install-report
             Using the App private key from Bitwarden (via bws env), mint a
             JWT and list installations (GET /app/installations) so the
             runbook's install checklist can be verified.
  webhook-secret
             Generate a webhook secret and store it in Bitwarden by name;
             the same value must then be set on the App (GitHub UI) and on
             the Worker (`wrangler secret put GITHUB_WEBHOOK_SECRET`).
             Prints only the Bitwarden secret name, never the value.

Bitwarden access goes through `bws` with BWS_ACCESS_TOKEN from the
environment (source ~/tools/bws-env.sh first). bws project: the one
~/tools/bws-env.sh defaults to.
"""

import json, os, secrets as secrets_mod, subprocess, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ghb_common import load_config

try:
    CFG = load_config()
except (ValueError, OSError) as e:
    # A wrong-shape config is a clean, field-naming diagnostic for
    # every subcommand (#6849) — never a raw KeyError/TypeError
    # traceback from the import-time indexing below.
    print(f"app_setup: {e}", file=sys.stderr)
    sys.exit(2)
NAMES = CFG["app"]["bitwarden_secret_names"]


def app_manifest(worker_url):
    return {
        "name": CFG["app"]["name"],
        "url": "https://github.com/developing-today/code/tree/main/infra/github-backup",
        "hook_attributes": {
            "url": worker_url.rstrip("/") + CFG["worker"]["webhook_path"]
        },
        "redirect_url": "http://localhost:8765/callback",
        "public": False,
        "default_permissions": {"metadata": "read", "contents": "read"},
        "default_events": CFG["app"]["events"],
    }


def _bws_json(argv, what):
    """Run a bws read command and return its parsed JSON.

    The subprocess failure is reported as the bws failure it is
    (return code + stderr excerpt), and unparseable output is a
    named diagnostic — never a raw JSONDecodeError or a misleading
    downstream shape accident (#6849).
    """
    r = subprocess.run(argv, capture_output=True, text=True, env=dict(os.environ))
    if r.returncode != 0:
        raise RuntimeError(f"{what} failed (exit {r.returncode}): {r.stderr[:200]}")
    try:
        return json.loads(r.stdout or "null")
    except json.JSONDecodeError:
        raise RuntimeError(f"{what} returned unparseable output: {r.stdout[:200]!r}")


def _bws_list():
    secrets = _bws_json(["bws", "secret", "list"], "bws secret list")
    if not isinstance(secrets, list) or not all(
        isinstance(s, dict)
        and isinstance(s.get("id"), str)
        and isinstance(s.get("key"), str)
        for s in secrets
    ):
        raise RuntimeError(
            "bws secret list returned an unexpected shape: "
            "expected a list of objects with string 'id' and 'key'"
        )
    return secrets


def _project_id():
    if os.environ.get("GHB_BWS_PROJECT_ID"):
        return os.environ["GHB_BWS_PROJECT_ID"]
    projects = _bws_json(["bws", "project", "list"], "bws project list")
    if not isinstance(projects, list) or not all(
        isinstance(p, dict) and isinstance(p.get("id"), str) for p in projects
    ):
        raise RuntimeError(
            "bws project list returned an unexpected shape: "
            "expected a list of objects with string 'id'"
        )
    if not projects:
        raise RuntimeError("no Bitwarden projects visible to this token")
    return projects[0]["id"]


def bws_set(name, value):
    """Create-or-update a Bitwarden secret by name."""
    sid = next((s["id"] for s in _bws_list() if s.get("key") == name), None)
    if sid:
        r = subprocess.run(
            ["bws", "secret", "edit", sid, "--value", value],
            capture_output=True,
            text=True,
            env=dict(os.environ),
        )
    else:
        r = subprocess.run(
            ["bws", "secret", "create", name, value, _project_id()],
            capture_output=True,
            text=True,
            env=dict(os.environ),
        )
    if r.returncode != 0:
        raise RuntimeError(f"bws store failed for {name}: {r.stderr[:200]}")


def bws_get(name):
    sid = next((s["id"] for s in _bws_list() if s.get("key") == name), None)
    if not sid:
        raise KeyError(name)
    got = _bws_json(["bws", "secret", "get", sid], f"bws secret get {name}")
    if not isinstance(got, dict) or not isinstance(got.get("value"), str):
        raise RuntimeError(
            f"bws secret get {name} returned an unexpected shape: "
            "expected an object with a string 'value'"
        )
    return got["value"]


def cmd_manifest():
    worker_url = os.environ.get("GHB_WORKER_URL", "https://<worker-host>")
    m = app_manifest(worker_url)
    print(json.dumps(m, indent=2))
    print(
        """
CLICK PATH (one human step — GitHub requires it):
  1. Set GHB_WORKER_URL to the deployed Worker URL and re-run to bake it in.
  2. Open https://github.com/settings/apps/new as dezren39 and submit this
     manifest via the manifest-flow form (the RUNBOOK §App has the exact
     HTML snippet), OR paste it using GitHub's manifest flow page.
  3. GitHub redirects to the redirect_url with ?code=XXXX.
  4. Run:  python3 app_setup.py convert XXXX
     -> stores app id / client id / client secret / private key in
        Bitwarden under the names in config.json. Nothing is printed.
""",
        file=sys.stderr,
    )


def validate_conversion(conv):
    """The full conversion response, validated BEFORE anything is
    stored (#6849): a response missing a late field must store
    nothing, never a partial credential set. Returns the validated
    (id, client_id, client_secret, pem, slug) tuple."""
    if not isinstance(conv, dict):
        raise RuntimeError("conversion response is not an object — nothing stored")
    problems = []
    if not isinstance(conv.get("id"), int) or isinstance(conv.get("id"), bool):
        problems.append("'id' must be an integer")
    for field in ("client_id", "client_secret", "pem", "slug"):
        if not isinstance(conv.get(field), str) or not conv.get(field):
            problems.append(f"'{field}' must be a non-empty string")
    if problems:
        raise RuntimeError(
            "conversion response failed validation ("
            + "; ".join(problems)
            + ") — nothing stored"
        )
    return (
        conv["id"],
        conv["client_id"],
        conv["client_secret"],
        conv["pem"],
        conv["slug"],
    )


def cmd_convert(code):
    import urllib.request

    req = urllib.request.Request(
        f"https://api.github.com/app-manifests/{code}/conversions",
        data=b"",
        method="POST",
    )
    req.add_header("Accept", "application/vnd.github+json")
    with urllib.request.urlopen(req, timeout=60) as r:
        conv = json.loads(r.read())
    app_id, client_id, client_secret, pem, slug = validate_conversion(conv)
    bws_set(NAMES["app_id"], str(app_id))
    bws_set(NAMES["client_id"], client_id)
    bws_set(NAMES["client_secret"], client_secret)
    bws_set(NAMES["private_key"], pem)
    print(
        f"App created: id={app_id} slug={slug} — credentials stored "
        f"in Bitwarden by name ({', '.join(NAMES.values())})."
    )
    print("Next: app_setup.py webhook-secret, then install the App per RUNBOOK §App.")


def cmd_webhook_secret():
    value = secrets_mod.token_hex(32)
    bws_set(NAMES["webhook_secret"], value)
    print(
        f"Stored under Bitwarden name {NAMES['webhook_secret']}. "
        "Set the same value on the App (GitHub UI > App settings > Webhook secret) "
        "and on the Worker: wrangler secret put GITHUB_WEBHOOK_SECRET."
    )


def validate_installations(installs):
    """The installations payload, validated before any indexing
    (#6849): a list of objects with an integer 'id' and an
    'account' object with a string 'login'."""
    if not isinstance(installs, list) or not all(
        isinstance(i, dict)
        and isinstance(i.get("id"), int)
        and not isinstance(i.get("id"), bool)
        and isinstance(i.get("account"), dict)
        and isinstance(i["account"].get("login"), str)
        for i in installs
    ):
        raise RuntimeError(
            "installations response returned an unexpected shape: "
            "expected a list of objects with integer 'id' and "
            "'account.login' string"
        )
    return installs


def cmd_install_report():
    # JWT minting needs a JOSE lib; use PyJWT if present, else cryptography.
    try:
        import jwt as pyjwt
    except ImportError:
        print(
            "PyJWT not installed (pip install pyjwt cryptography) — cannot mint App JWT here."
        )
        return 1
    import urllib.request

    app_id = bws_get(NAMES["app_id"])
    pem = bws_get(NAMES["private_key"])
    now = int(time.time())
    token = pyjwt.encode(
        {"iat": now - 60, "exp": now + 540, "iss": app_id}, pem, algorithm="RS256"
    )
    req = urllib.request.Request("https://api.github.com/app/installations")
    req.add_header("Authorization", "Bearer " + token)
    req.add_header("Accept", "application/vnd.github+json")
    with urllib.request.urlopen(req, timeout=60) as r:
        installs = json.loads(r.read())
    validate_installations(installs)
    for i in installs:
        acct = i["account"]["login"]
        print(
            f"installed: {acct} (installation {i['id']}, repos: "
            f"{i.get('repository_selection')})"
        )
    print(f"total installations: {len(installs)}")
    return 0


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    if cmd == "manifest":
        cmd_manifest()
    elif cmd == "convert" and len(sys.argv) == 3:
        cmd_convert(sys.argv[2])
    elif cmd == "webhook-secret":
        cmd_webhook_secret()
    elif cmd == "install-report":
        sys.exit(cmd_install_report())
    else:
        print(__doc__)
        sys.exit(2)
