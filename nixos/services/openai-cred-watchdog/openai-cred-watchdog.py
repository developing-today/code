#!/usr/bin/env python3
"""Watchdog: OpenAI credential invariant for the laptop OpenCode store.

Invariant (recurrence guard for developing-today/code#3216):
  - provider `openai`: NO credential row with method chatgpt-token-sharing
    may exist, and the ACTIVE credential method must be chatgpt-browser
    or chatgpt-headless.
  - provider `openai.alternate`: the ACTIVE credential method must be
    chatgpt-browser or chatgpt-headless.

Read-only. Never prints token values (only credential ids, labels, methods).
Exit: 0 healthy, 1 violation, 2 check error.
Usage: openai-cred-watchdog.py [path-to-opencode.db]
"""
import json
import os
import sqlite3
import sys

DEFAULT_DB = os.path.expanduser("~/.local/share/opencode/opencode.db")
ALLOWED_ACTIVE_METHODS = {"chatgpt-browser", "chatgpt-headless"}
BANNED_METHOD = "chatgpt-token-sharing"
PROVIDERS = ("openai", "openai.alternate")


def method_of(value_json):
    try:
        return json.loads(value_json).get("methodID")
    except Exception:
        return None


def check(db_path):
    violations = []
    con = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    try:
        marks = ",".join("?" for _ in PROVIDERS)
        rows = con.execute(
            "SELECT id, integration_id, label, active, value FROM credential "
            f"WHERE integration_id IN ({marks})",
            PROVIDERS,
        ).fetchall()
    finally:
        con.close()
    by_provider = {}
    for cred_id, integration_id, label, active, value in rows:
        by_provider.setdefault(integration_id, []).append(
            (cred_id, label, bool(active), method_of(value))
        )
    for provider in PROVIDERS:
        creds = by_provider.get(provider, [])
        if not creds:
            violations.append(f"{provider}: no credential rows found")
            continue
        if provider == "openai":
            for cred_id, label, active, method in creds:
                if method == BANNED_METHOD:
                    violations.append(
                        f"{provider}: banned {BANNED_METHOD} credential row exists: "
                        f"id={cred_id} label={label!r} active={active}"
                    )
        actives = [c for c in creds if c[2]]
        if len(actives) != 1:
            violations.append(
                f"{provider}: expected exactly 1 active credential, found {len(actives)}"
            )
        for cred_id, label, _active, method in actives:
            if method not in ALLOWED_ACTIVE_METHODS:
                violations.append(
                    f"{provider}: active credential method is {method!r} "
                    f"(allowed: {sorted(ALLOWED_ACTIVE_METHODS)}): "
                    f"id={cred_id} label={label!r}"
                )
    return violations


def main():
    db_path = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_DB
    if not os.path.exists(db_path):
        print(f"VIOLATION: credential store not found: {db_path}")
        return 2
    try:
        violations = check(db_path)
    except Exception as exc:  # unreadable store must be loud, not silent
        print(f"VIOLATION: credential check failed: {type(exc).__name__}: {exc}")
        return 2
    if violations:
        for v in violations:
            print(f"VIOLATION: {v}")
        return 1
    print("OK: openai credential invariant holds (no token-sharing row; active methods browser/headless)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
