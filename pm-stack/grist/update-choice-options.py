#!/usr/bin/env python3
"""Extend a Grist Choice column's options via the supported REST API.

Verified against Grist OSS v1.7.21 (the digest pinned in
pm-stack/grist/compose.yml) — see CHOICE-OPTIONS.md in this directory
for the probe evidence. The short version:

  * ``POST /api/docs/{doc}/apply`` with ``UpdateColumn`` is blocked by
    access rules that grant data access without the schema bit
    (``'UpdateColumn' actions are not controlled``).
  * ``PATCH /api/docs/{doc}/tables/{table}/columns`` (the collection
    route, body ``{"columns": [{"id": ..., "fields": {...}}]}``) with a
    JSON-string ``widgetOptions`` applies the change under the same
    rules, authenticated with the normal API key. No SQLite writes.

The merge is idempotent and preserves every other widgetOptions key
(e.g. ``choiceOptions`` colors): only ``choices`` is extended, with
new options appended in the order given, duplicates dropped.

The API key is read from the GRIST_API_KEY environment variable —
never from argv or a file — so it cannot leak into shell history or
process listings.

Usage:
    GRIST_API_KEY=... python3 update-choice-options.py \
        --doc DOC_ID --table Roadmap --column Area \
        --add id --add infra [--base http://127.0.0.1:8484] [--dry-run]

Exit codes: 0 ok (including "nothing to add"), 1 API/verification
failure, 2 usage error. A stored widgetOptions value with an
unexpected shape (not a JSON object, or a ``choices`` that is not a
list of strings) is an exit-1 failure reported on stderr -- it is
never coerced, and no PATCH is sent (#6588).
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request

API_TIMEOUT_SECONDS = 30


class WidgetOptionsError(ValueError):
    """The stored widgetOptions value has an unexpected shape (#6588).

    A data problem, not a usage problem: run() reports it on stderr
    and exits 1, like an API/verification failure. It is raised before
    any PATCH is built, so a shape failure never writes anything.
    """


def check_widget_options(widget_options: object) -> dict:
    """Return widget_options if its shape is the one the merge needs.

    The stored value is written by other tools and Grist versions as
    well as this one, so its shape is not guaranteed: widgetOptions
    must be a JSON object, and its ``choices`` (when present and not
    null) must be a list of strings. Anything else is refused by
    name -- a string ``choices`` used to be shredded into
    one-character choices by list(), and a non-object widgetOptions
    died as a raw ValueError traceback out of dict() (#6588).
    """
    if not isinstance(widget_options, dict):
        raise WidgetOptionsError(
            f"widgetOptions is not a JSON object (got {type(widget_options).__name__}); "
            "refusing to merge choices into it"
        )
    choices = widget_options.get("choices")
    if choices is not None:
        if not isinstance(choices, list):
            raise WidgetOptionsError(
                f"widgetOptions 'choices' is not a list (got {type(choices).__name__}); "
                "refusing to merge -- a string would be shredded into one-character choices"
            )
        for i, choice in enumerate(choices):
            if not isinstance(choice, str):
                raise WidgetOptionsError(
                    f"widgetOptions 'choices' entry {i} is not a string "
                    f"(got {type(choice).__name__}); refusing to merge"
                )
    return widget_options


def merge_choices(widget_options: dict, additions: list[str]) -> tuple[dict, list[str]]:
    """Return (new_widget_options, actually_added).

    Pure function (unit-tested in test_update_choice_options.py):
    appends each addition that is not already a choice, preserves all
    other keys and the existing choice order. Raises
    WidgetOptionsError when the stored shape is not a JSON object
    with a list-of-strings ``choices`` (#6588).
    """
    merged = dict(check_widget_options(widget_options))
    choices = list(merged.get("choices") or [])
    added: list[str] = []
    for choice in additions:
        if choice not in choices:
            choices.append(choice)
            added.append(choice)
    merged["choices"] = choices
    return merged, added


def api(
    base: str, key: str, method: str, path: str, body: dict | None = None
) -> object:
    req = urllib.request.Request(
        base.rstrip("/") + path,
        method=method,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(
            req, timeout=API_TIMEOUT_SECONDS
        ) as resp:
            raw = resp.read().decode()
    except urllib.error.URLError as exc:
        if isinstance(exc.reason, TimeoutError):
            raise TimeoutError(
                f"API request timed out after {API_TIMEOUT_SECONDS}s ({method} {path})"
            ) from exc
        raise
    except TimeoutError as exc:
        raise TimeoutError(
            f"API request timed out after {API_TIMEOUT_SECONDS}s ({method} {path})"
        ) from exc
    return json.loads(raw) if raw and raw != "null" else None


def get_widget_options(base: str, key: str, doc: str, table: str, column: str) -> dict:
    data = api(base, key, "GET", f"/api/docs/{doc}/tables/{table}/columns")
    for col in data["columns"]:  # type: ignore[index]
        if col["id"] == column:
            raw = col["fields"].get("widgetOptions") or ""
            if not raw:
                return {}
            try:
                parsed = json.loads(raw)
            except json.JSONDecodeError as exc:
                raise WidgetOptionsError(
                    f"stored widgetOptions for column {column!r} is not valid JSON: {exc}"
                ) from exc
            return check_widget_options(parsed)
    raise SystemExit(f"column {column!r} not found in table {table!r}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", default="http://127.0.0.1:8484")
    parser.add_argument("--doc", required=True)
    parser.add_argument("--table", required=True)
    parser.add_argument("--column", required=True)
    parser.add_argument(
        "--add",
        action="append",
        required=True,
        dest="additions",
        help="choice to add; repeat for multiple",
    )
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(argv)

    key = os.environ.get("GRIST_API_KEY")
    if not key:
        print("GRIST_API_KEY is not set", file=sys.stderr)
        return 2

    current = get_widget_options(args.base, key, args.doc, args.table, args.column)
    merged, added = merge_choices(current, args.additions)
    if not added:
        print(
            f"nothing to add: {args.table}.{args.column} already has {merged['choices']}"
        )
        return 0
    print(
        f"adding {added} to {args.table}.{args.column}: {current.get('choices')} -> {merged['choices']}"
    )
    if args.dry_run:
        print("dry run: no PATCH sent")
        return 0

    api(
        args.base,
        key,
        "PATCH",
        f"/api/docs/{args.doc}/tables/{args.table}/columns",
        {
            "columns": [
                {"id": args.column, "fields": {"widgetOptions": json.dumps(merged)}}
            ]
        },
    )

    readback = get_widget_options(args.base, key, args.doc, args.table, args.column)
    if readback.get("choices") != merged["choices"]:
        print(
            f"verification failed: read-back choices {readback.get('choices')} != {merged['choices']}",
            file=sys.stderr,
        )
        return 1
    for k, v in merged.items():
        if k != "choices" and readback.get(k) != v:
            print(
                f"verification failed: widgetOptions[{k!r}] not preserved",
                file=sys.stderr,
            )
            return 1
    print(f"verified by read-back: {readback['choices']}")
    return 0


def run(argv: list[str] | None = None) -> int:
    try:
        return main(argv)
    except TimeoutError as exc:
        print(str(exc), file=sys.stderr)
        return 1
    except WidgetOptionsError as exc:
        print(str(exc), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(run())
