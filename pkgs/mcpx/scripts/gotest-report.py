#!/usr/bin/env python3
"""Summarise `go test -json` output and hold skipped tests to the record on main.

Usage:
  gotest-report.py <go-test-json-file> <stored-record-json-or-empty> <run-id/attempt>
                   <now-iso> <summary-file> <new-record-file>

The record lives in a repository variable, like the conformance records, and
holds the counts and the name of every skipped test. A run that skips more
tests than the record fails; the newly skipped tests are named. A run whose
skipped set contains ANY name the record does not hold also fails, whatever
the count does (#6391): an equal-count swap -- one test un-skipped, a
different one newly skipped -- must not silently become the new record, or
the newly skipped test is ratified without ever failing a run. A genuine
rename is a new name under this rule: it fails once, and the record is
re-baselined only by explicitly updating the repository variable, never by
a passing run. The record only moves down: a run with fewer skips and no
new names becomes the new ceiling, so a skip cannot creep back in. With no
record yet the run is reported and becomes the record when saved on main.

One exemption: a skip whose reason is a testsupport.Quarantine notice
("quarantined until <date> (<issue link>)", #1973) is reported but does NOT
count against the record. A quarantine is a tracked, expiring exemption
with its own guard test; counting it here would make the documented
quarantine path unusable the first time anyone used it.

Exits 1 when any test/package failed, more tests were skipped than the
record, or any skipped test's name is not in the record.
<new-record-file> is written when the record should change; saving it is the
workflow's decision, on main only.
"""

import collections
import datetime
import json
import re
import sys
from urllib.parse import urlsplit


class StoredRecordError(ValueError):
    """The stored skip record is the wrong shape; message names record + field."""


def validate_stored(stored):
    """Validate the stored record before decide() uses it.

    Returns the record unchanged when well-formed. Raises StoredRecordError
    naming the record and the offending field for: a non-object top level
    (an empty list included -- it must not silently reset the ratchet),
    a missing or non-integer skipped_count, a skipped field that is not a
    list of strings, or a non-string run field.
    """
    if not isinstance(stored, dict):
        raise StoredRecordError(
            f"stored record: top level must be a JSON object, got {type(stored).__name__}"
        )
    if "skipped_count" not in stored:
        raise StoredRecordError("stored record: field 'skipped_count' is missing")
    count = stored["skipped_count"]
    if isinstance(count, bool) or not isinstance(count, int):
        raise StoredRecordError(
            f"stored record: field 'skipped_count' must be an int, got {type(count).__name__}"
        )
    if "skipped" in stored:
        skipped = stored["skipped"]
        if not isinstance(skipped, list) or not all(
            isinstance(item, str) for item in skipped
        ):
            raise StoredRecordError(
                "stored record: field 'skipped' must be a list of strings"
            )
    if "run" in stored and not isinstance(stored["run"], str):
        raise StoredRecordError("stored record: field 'run' must be a string")
    return stored


def parse_stored(raw):
    """Parse + validate the stored-record argument; empty means no record yet."""
    if not raw.strip():
        return None
    try:
        stored = json.loads(raw)
    except ValueError as exc:
        raise StoredRecordError(f"stored record: invalid JSON ({exc})") from exc
    return validate_stored(stored)


# Event-line rule: an event is used only when it is a JSON object and every
# field this reporter reads (Package, Test, Action, Output) is a string when
# present. Anything else -- a non-object line, or a non-string field such as
# a null Package or an integer Output -- is skipped as one malformed event
# and counted in counts["malformed"], never coerced and never crashed on.
_EVENT_STRING_FIELDS = ("Package", "Test", "Action", "Output")


QUARANTINE_NOTICE = re.compile(
    r"quarantined until (?P<expiry>\d{4}-\d{2}-\d{2}) "
    r"\((?P<issue>[^()\s]+)\)"
)
GITHUB_ISSUE_PATH = re.compile(r"/[^/\s?#]+/[^/\s?#]+/issues/[1-9]\d*")
# In the real `go test -json` stream, a test's skip message carries the
# Go-generated source location of the t.Skipf call site:
# "widgets_test.go:42: quarantined until ...". That prefix is part of
# the output format, not of the notice, so strip exactly that shape
# before validating the complete notice.
GO_SOURCE_PREFIX = re.compile(r"[^\s/:]+\.go:\d+: ")


def is_valid_quarantine_notice(reason, today):
    match = QUARANTINE_NOTICE.fullmatch(GO_SOURCE_PREFIX.sub("", reason, count=1))
    if not match:
        return False

    expiry_text = match.group("expiry")
    try:
        expiry = datetime.date.fromisoformat(expiry_text)
        issue = urlsplit(match.group("issue"))
    except ValueError:
        return False

    return (
        expiry.isoformat() == expiry_text
        and expiry >= today
        and issue.scheme == "https"
        and issue.netloc == "github.com"
        and GITHUB_ISSUE_PATH.fullmatch(issue.path) is not None
        and not issue.query
        and not issue.fragment
    )


def utc_date(now):
    parsed = datetime.datetime.fromisoformat(now.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=datetime.timezone.utc)
    return parsed.astimezone(datetime.timezone.utc).date()


def read(path, today):
    counts = collections.Counter()
    out = collections.defaultdict(list)
    skipped, failed, package_failed = [], [], []
    failed_packages = set()
    for line in open(path, errors="replace"):
        try:
            e = json.loads(line)
        except ValueError:
            continue
        if not isinstance(e, dict) or any(
            field in e and not isinstance(e[field], str)
            for field in _EVENT_STRING_FIELDS
        ):
            counts["malformed"] += 1
            continue
        test = e.get("Test")
        act = e.get("Action")
        if not test:
            if act == "fail":
                package_failed.append(e.get("Package") or "(unknown package)")
            continue
        key = f"{e.get('Package', '').rsplit('/', 1)[-1]}.{test}"
        if act == "output":
            out[key].append(e.get("Output", ""))
        elif act in ("pass", "fail", "skip"):
            counts[act] += 1
            (skipped if act == "skip" else failed if act == "fail" else []).append(key)
            if act == "fail":
                failed_packages.add(e.get("Package") or "(unknown package)")
    reasons = {}
    for k in skipped:
        lines = [
            o.strip()
            for o in out[k]
            if o.strip() and not o.lstrip().startswith(("=== ", "--- "))
        ]
        reasons[k] = (lines[-1] if lines else "(no reason given)")[:200]
    quarantined = sorted(
        k for k in skipped if is_valid_quarantine_notice(reasons[k], today)
    )
    counted = sorted(k for k in skipped if k not in quarantined)
    # A failing test also emits a package-level fail for its package; only
    # packages without their own test failure are independent package failures
    # (build failures, TestMain failures). This keeps one failing test at one
    # failure instead of double-counting the package event.
    isolated = sorted(set(package_failed) - failed_packages)
    return (
        counts,
        counted,
        sorted(failed),
        reasons,
        quarantined,
        sorted(skipped),
        isolated,
    )


def decide(skipped, passed, failed_n, stored, run, now):
    """Returns (new_record_or_None, rose, new_names, gone_names)."""
    cur = {
        "timestamp": now,
        "run": run,
        "passed_count": passed,
        "failed_count": failed_n,
        "skipped_count": len(skipped),
        "skipped": skipped,
    }
    if stored is not None:
        validate_stored(stored)
    # Never let an incomplete or failing run lower the main-branch ceiling.
    if failed_n:
        return None, False, [], []
    if not stored:
        return cur, False, [], []
    prev = set(stored.get("skipped", []))
    new_names = sorted(set(skipped) - prev)
    gone_names = sorted(prev - set(skipped))
    # The name half of the invariant (#6391): a skipped name the record
    # does not hold fails the run even when the count did not rise -- an
    # equal-count (or lower-count) swap must never re-baseline the record.
    # `rose` therefore means "the run failed on skips"; main() words the
    # consequence from the counts.
    if len(skipped) > stored["skipped_count"] or new_names:
        return None, True, new_names, gone_names
    if len(skipped) < stored["skipped_count"] or skipped != stored.get("skipped"):
        return cur, False, new_names, gone_names
    return None, False, new_names, gone_names


def main(path, stored_raw, run, now, summary, new_file):
    counts, skipped, failed, reasons, quarantined, all_skipped, package_failed = read(
        path, utc_date(now)
    )
    try:
        stored = parse_stored(stored_raw)
    except StoredRecordError as exc:
        print(f"::error::{exc}")
        return 2
    failed_n = counts["fail"] + len(package_failed)
    new, rose, new_names, gone = decide(
        skipped, counts["pass"], failed_n, stored, run, now
    )
    ceiling = stored["skipped_count"] if stored else None

    with open(summary, "a") as f:
        f.write(
            "## Go tests\n\n| passed | failed | skipped | record on main |\n| --- | --- | --- | --- |\n"
        )
        f.write(
            f"| {counts['pass']} | {failed_n} | **{len(skipped)}** | "
            f"{ceiling if ceiling is not None else 'none yet'}"
            f"{' (run ' + stored.get('run', '(unknown run)') + ')' if stored else ''} |\n\n"
        )
        if failed:
            f.write(
                "> [!CAUTION]\n> # Failed tests\n"
                + "".join(f"> - {k}\n" for k in failed)
                + "\n"
            )
        if package_failed:
            f.write(
                "> [!CAUTION]\n> # Failed packages\n"
                + "".join(f"> - `{package}`\n" for package in package_failed)
                + "\n"
            )
        if rose:
            if len(skipped) > ceiling:
                f.write(
                    f"> [!CAUTION]\n> # More tests skipped than the record: {len(skipped)} against {ceiling}\n"
                    "> A skipped test passes nothing. Newly skipped:\n"
                )
            else:
                f.write(
                    f"> [!CAUTION]\n> # Skipped tests not in the record: {len(new_names)} new name(s) at {len(skipped)} skipped against the record's {ceiling}\n"
                    "> Same-or-fewer count does not ratify a swap (#6391): this run FAILED and the record is held unchanged. "
                    "If a name here is a genuine rename, re-baseline by updating the record variable explicitly. Newly skipped:\n"
                )
            for k in new_names:
                f.write(f"> - `{k}` -- {reasons.get(k, '')}\n")
            f.write("\n")
        elif stored and len(skipped) < ceiling:
            f.write(
                f"Skipped tests **down** from {ceiling} to {len(skipped)}"
                f"{' (no longer skipped: ' + ', '.join(f'`{k}`' for k in gone) + ')' if gone else ''}. "
                "On main this becomes the new record.\n\n"
            )
        if quarantined:
            f.write(
                f"{len(quarantined)} quarantined under #1973 (issue + expiry, exempt from the record): "
                + ", ".join(f"`{k}`" for k in quarantined)
                + ".\n\n"
            )
        if all_skipped:
            f.write(
                "<details><summary>Every skipped test, with its reason</summary>\n\n"
                "| test | reason |\n| --- | --- |\n"
            )
            for k in all_skipped:
                f.write(f"| `{k}` | {reasons[k].replace('|', chr(92) + '|')} |\n")
            f.write("\n</details>\n")

    if counts["malformed"]:
        print(f"skipped {counts['malformed']} malformed event line(s)")
    print(
        f"go tests: {counts['pass']} passed, {failed_n} failed, {len(skipped)} skipped "
        f"(record: {ceiling if ceiling is not None else 'none yet'})"
    )
    for package in package_failed:
        print(f"  package fail  {package}")
    for k in all_skipped:
        print(f"  skip  {k}  -- {reasons[k]}")
    if rose:
        if len(skipped) > ceiling:
            print(
                f"::error::{len(skipped)} Go tests skipped, more than the record of {ceiling}; newly skipped: "
                + ", ".join(new_names)
            )
        else:
            print(
                f"::error::skipped tests not in the record of {ceiling}; run failed, record held unchanged; newly skipped: "
                + ", ".join(new_names)
            )
    if new is not None:
        with open(new_file, "w") as f:
            json.dump(new, f, separators=(",", ":"))
    return 1 if (failed or package_failed or rose) else 0


if __name__ == "__main__":
    sys.exit(main(*sys.argv[1:7]))
