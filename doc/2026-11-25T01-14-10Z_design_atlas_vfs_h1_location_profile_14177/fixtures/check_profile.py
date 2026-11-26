#!/usr/bin/env python3
"""Reference projection and golden-vector check for the VFS location-record
profile v1 (#14177). Standard library only; reads vfs-location-v1.json beside
this file and exits non-zero on the first mismatch.

This is the executable oracle the H2/H3 implementations must agree with. It is
deliberately small and literal: it implements the registry in ../note.md §3-§5
and nothing else.
"""

from __future__ import annotations

import hashlib
import json
import re
import sys
from pathlib import Path

PROFILE = "atlas.vfs-location/v1"
CORE_REQUIRED = (
    "content_id",
    "locator",
    "namespace",
    "tenant_scope",
    "observed_at",
    "access_class",
    "signer",
)
CORE_OPTIONAL = ("occurrence_ref", "source_version_ref")
PRESENTATION = ("display_path", "filename", "host", "produced_by", "archive_member")
DECLARED = set(CORE_REQUIRED) | set(CORE_OPTIONAL) | set(PRESENTATION)
ACCESS_CLASSES = ("public", "scope", "stored_unverified")
ACCESS_FALLBACK = "scope"
CONTENT_ID = re.compile(
    r"^content/(raw-blake3-256|atlas-chunk-sha256-utf8)\.[0-9a-f]{64}$"
)
MAX_LABEL = 256
MEMBER_SEP = "!/"


def canonical_bytes(obj: object) -> bytes:
    return json.dumps(
        obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode("utf-8")


def artifact_id(obj: object) -> str:
    return "sha256:" + hashlib.sha256(canonical_bytes(obj)).hexdigest()


def has_control(text: str) -> bool:
    return any(ord(c) < 0x20 or ord(c) == 0x7F for c in text)


def is_label(value: object) -> bool:
    return (
        isinstance(value, str)
        and value.strip() != ""
        and len(value.encode("utf-8")) <= MAX_LABEL
        and not has_control(value)
    )


def is_segment(value: str) -> bool:
    return (
        value != ""
        and "/" not in value
        and not any(c.isspace() for c in value)
        and not has_control(value)
    )


def parse_locator(text: object):
    """Returns (kind, parts) or raises ValueError. Mirrors
    id_core::location_ref::TypedLocator::parse_tagged."""
    if not isinstance(text, str) or "/" not in text:
        raise ValueError("malformed")
    tag, rest = text.split("/", 1)
    if tag == "r2":
        parts = rest.split("/", 2)
        if len(parts) != 3 or not is_segment(parts[0]) or not is_segment(parts[1]):
            raise ValueError("malformed r2")
        if parts[2] == "" or has_control(parts[2]):
            raise ValueError("bad key")
        return "r2", {"account": parts[0], "bucket": parts[1], "key": parts[2]}
    if tag == "laptop":
        if "/" not in rest:
            raise ValueError("malformed laptop")
        host, tail = rest.split("/", 1)
        path = "/" + tail
        if not is_segment(host) or has_control(path):
            raise ValueError("bad laptop")
        return "laptop", {"hostname": host, "path": path}
    if tag == "drive":
        parts = rest.split("/", 2)
        if len(parts) != 3 or not is_segment(parts[0]) or not is_segment(parts[1]):
            raise ValueError("malformed drive")
        url = parts[2]
        if (
            not url.startswith("https://")
            or len(url) == len("https://")
            or any(c.isspace() for c in url)
            or has_control(url)
        ):
            raise ValueError("bad url")
        return "drive", {"account": parts[0], "file_id": parts[1], "url": url}
    raise LookupError(tag)


def derive_host(kind: str, parts: dict) -> str:
    if kind == "laptop":
        return parts["hostname"]
    if kind == "r2":
        return f"r2:{parts['account']}/{parts['bucket']}"
    return f"drive:{parts['account']}"


def default_display_path(kind: str, parts: dict):
    if kind == "laptop":
        return parts["path"]
    if kind == "r2":
        return "/" + parts["key"]
    return None


def derive_filename(display_path):
    if display_path is None:
        return None
    tail = display_path.rsplit(MEMBER_SEP, 1)[-1]
    name = tail.rsplit("/", 1)[-1]
    return name or None


def relocate(
    extras: dict, reports: list, path: str, value: object, reason: str
) -> None:
    key = path
    if key.startswith("$") or (key in extras and extras[key] != value):
        extras.setdefault("$relocated", []).append(
            {"path": path, "value": value, "reason": reason}
        )
    else:
        extras[key] = value
    reports.append({"path": path, "reason": reason})


def project(record: dict) -> dict:
    """Projects one Metadata object under the profile. Returns
    {view, extras, reports, projectable}."""
    if record.get("schema_version") != 1:
        return {
            "view": None,
            "extras": None,
            "reports": [{"path": "schema_version", "reason": "unsupported-version"}],
            "projectable": False,
        }
    fields = dict(record.get("fields", {}))
    extras = json.loads(json.dumps(record.get("extras", {})))
    reports: list = []
    view: dict = {}
    projectable = True

    for key in sorted(fields):
        if key not in DECLARED:
            relocate(extras, reports, key, fields[key], "unknown-field")

    # Core assertion fields.
    cid = fields.get("content_id")
    if cid is None:
        reports.append({"path": "content_id", "reason": "missing-required"})
        projectable = False
    elif not (isinstance(cid, str) and CONTENT_ID.match(cid)):
        relocate(extras, reports, "content_id", cid, "type-mismatch")
        projectable = False
    else:
        view["content_id"] = cid

    kind = parts = None
    loc = fields.get("locator")
    if loc is None:
        reports.append({"path": "locator", "reason": "missing-required"})
        projectable = False
    else:
        try:
            kind, parts = parse_locator(loc)
            view["locator"] = loc
            view["locator_type"] = kind
        except LookupError:
            relocate(extras, reports, "locator", loc, "unknown-enum")
            projectable = False
        except ValueError:
            relocate(extras, reports, "locator", loc, "type-mismatch")
            projectable = False

    for key in ("namespace", "tenant_scope", "signer"):
        value = fields.get(key)
        if value is None:
            reports.append({"path": key, "reason": "missing-required"})
            projectable = False
        elif not is_label(value):
            relocate(extras, reports, key, value, "type-mismatch")
            projectable = False
        else:
            view[key] = value

    obs = fields.get("observed_at")
    if obs is None:
        reports.append({"path": "observed_at", "reason": "missing-required"})
        projectable = False
    elif isinstance(obs, bool) or not isinstance(obs, int) or obs < 0:
        relocate(extras, reports, "observed_at", obs, "type-mismatch")
        projectable = False
    else:
        view["observed_at"] = obs

    ac = fields.get("access_class")
    if ac in ACCESS_CLASSES:
        view["access_class"] = ac
    else:
        if ac is None:
            reports.append({"path": "access_class", "reason": "missing-required"})
        else:
            relocate(
                extras,
                reports,
                "access_class",
                ac,
                "unknown-enum" if isinstance(ac, str) else "type-mismatch",
            )
        view["access_class"] = ACCESS_FALLBACK
        projectable = False

    for key in CORE_OPTIONAL:
        if key in fields:
            if is_label(fields[key]):
                view[key] = fields[key]
            else:
                relocate(extras, reports, key, fields[key], "type-mismatch")

    # Presentation fields.
    if "produced_by" in fields:
        if is_label(fields["produced_by"]):
            view["produced_by"] = fields["produced_by"]
        else:
            relocate(
                extras, reports, "produced_by", fields["produced_by"], "type-mismatch"
            )

    member = fields.get("archive_member")
    if member is not None:
        ok = (
            isinstance(member, dict)
            and set(member) == {"path", "offset", "length"}
            and isinstance(member["path"], str)
            and member["path"] != ""
            and not member["path"].startswith("/")
            and not has_control(member["path"])
            and all(
                isinstance(member[k], int)
                and not isinstance(member[k], bool)
                and member[k] >= 0
                for k in ("offset", "length")
            )
            and kind in ("r2", "laptop")
        )
        if ok:
            view["archive_member"] = member
        else:
            relocate(
                extras,
                reports,
                "archive_member",
                member,
                "unconvertible" if isinstance(member, dict) else "type-mismatch",
            )
            member = None

    dp = fields.get("display_path")
    if dp is not None:
        if (
            isinstance(dp, str)
            and dp.startswith("/")
            and not has_control(dp)
            and MEMBER_SEP not in dp
        ):
            view["display_path"] = dp
        else:
            relocate(extras, reports, "display_path", dp, "type-mismatch")
            dp = None
    if dp is None and kind is not None:
        dp = default_display_path(kind, parts)
        if dp is not None and member is not None:
            dp = dp + MEMBER_SEP + member["path"]
        if dp is not None:
            view["display_path"] = dp

    if kind is not None:
        host = derive_host(kind, parts)
        if "host" in fields and fields["host"] != host:
            relocate(extras, reports, "host", fields["host"], "unconvertible")
        view["host"] = host

    filename = derive_filename(view.get("display_path"))
    if "filename" in fields and fields["filename"] != filename:
        relocate(extras, reports, "filename", fields["filename"], "unconvertible")
    if filename is not None:
        view["filename"] = filename

    return {
        "view": view,
        "extras": extras,
        "reports": reports,
        "projectable": projectable,
    }


def identity(view: dict) -> str:
    return "|".join(
        (view["namespace"], view["tenant_scope"], view["content_id"], view["locator"])
    )


def fold(events: list, records: dict) -> dict:
    rows: dict = {}
    for event in events:
        if event["op"] == "assert":
            projected = project(records[event["record"]]["object"])
            if not projected["projectable"]:
                continue
            view = projected["view"]
            state = (
                "unverified" if view["access_class"] == "stored_unverified" else "fresh"
            )
            rows[identity(view)] = dict(view, state=state, record=event["record"])
        elif event["op"] == "tombstone":
            row = rows.get(event["identity"])
            if row is None:
                raise AssertionError(
                    f"tombstone for unknown identity {event['identity']}"
                )
            row["state"] = {"delete": "deleted", "stale": "stale"}[event["kind"]]
        else:
            raise AssertionError(f"unknown op {event['op']}")
    return rows


def run_queries(rows: dict) -> dict:
    live = {k: v for k, v in rows.items() if v["state"] != "deleted"}

    def pick(pred):
        return sorted(v["record"] for v in live.values() if pred(v))

    return {
        "by_content_id": pick(lambda v: v["content_id"] == QUERY["content_id"]),
        "by_filename": pick(lambda v: v.get("filename") == QUERY["filename"]),
        "by_path_prefix": pick(
            lambda v: v.get("display_path", "").startswith(QUERY["path_prefix"])
        ),
        "by_host": pick(lambda v: v.get("host") == QUERY["host"]),
        "states": {v["record"]: v["state"] for v in rows.values()},
    }


QUERY: dict = {}


def main() -> int:
    data = json.loads(
        (Path(__file__).parent / "vfs-location-v1.json").read_text(encoding="utf-8")
    )
    assert data["profile"] == PROFILE, "profile id"
    failures = 0
    records = data["records"]
    for name, rec in records.items():
        got = artifact_id(rec["object"])
        if got != rec["artifact_id"]:
            print(f"FAIL artifact id {name}: {got} != {rec['artifact_id']}")
            failures += 1
        projected = project(rec["object"])
        for key, want in rec.get("expect", {}).items():
            have = projected["view"] if key == "view" else projected[key]
            if have != want:
                print(
                    f"FAIL {name} {key}:\n  have {json.dumps(have, sort_keys=True)}\n  want {json.dumps(want, sort_keys=True)}"
                )
                failures += 1
    QUERY.update(data["queries"])
    for step in data["steps"]:
        rows = fold(step["events"], records)
        got = run_queries(rows)
        if got != step["expect"]:
            print(
                f"FAIL step {step['name']}:\n  have {json.dumps(got, sort_keys=True)}\n  want {json.dumps(step['expect'], sort_keys=True)}"
            )
            failures += 1
    if failures:
        print(f"{failures} failure(s)")
        return 1
    print(f"ok: {len(records)} records, {len(data['steps'])} steps")
    return 0


if __name__ == "__main__":
    sys.exit(main())
