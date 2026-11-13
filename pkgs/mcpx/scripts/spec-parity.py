#!/usr/bin/env python3
"""Compare the MCP spec revisions mcpx supports with those the conformance suite knows.

Usage:
  spec-parity.py <mcpx-json-array> <suite-json-array> <now-iso> <run-id/attempt>
                 <success-var-json-or-empty> <failure-var-json-or-empty>
                 <on-main: true|false> <summary-file> <out-dir>
                 [<allow-only-mcpx-json> <allow-only-suite-json>]

The two allow lists name revisions one side is expected to have and the other
not -- mcpx serves 2024-11-05, which the suite has never tested. An allowed
revision is left out of the comparison. An allowed revision that both sides
now have, or that the side it was allowed for no longer has, is an error at
once: the entry is dead and must be removed, or it would hide a real
mismatch later.

Both lists come from the tools themselves, never from this repository:
  mcpx   mcpx protocol --json | jq '.asServer.supported'
  suite  conformance list --spec-version x   (its error names every valid version)

Writes into <out-dir>:
  status          ok | warn | error
  sleep           seconds the job should sleep before finishing
  success.json    new value for the success variable, when it should change
  failure.json    new value for the failure variable, when it should change
and appends a report to <summary-file>. The variables are only to be saved on
main; the files are written regardless, and the workflow decides.

Deadlines run from the last success; with no success on record, from the first
failure. Revisions only one side has: 840 hours. Revisions each side has that
the other lacks: 360 hours, because the two have diverged rather than one
having moved ahead.
"""

import datetime as dt
import json
import os
import sys

ONE_SIDED_HOURS = 840
BOTH_SIDED_HOURS = 360
# The job sleeps one second per hour since the last success, so a mismatch
# left standing gets slower to ignore. Capped so a long-broken run still ends
# well inside the job's own timeout.
SLEEP_CAP = 900


def parse(ts):
    return dt.datetime.strptime(ts, "%Y-%m-%dT%H:%M:%SZ").replace(
        tzinfo=dt.timezone.utc
    )


class BadInput(Exception):
    def __init__(self, field, message):
        super().__init__(message)
        self.field = field
        self.message = message


def load_json(raw, field):
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        raise BadInput(field, "is not valid JSON") from None


def require_string_list(value, field):
    if not isinstance(value, list) or any(not isinstance(item, str) for item in value):
        raise BadInput(field, "must be a JSON array of strings")


def validate_timestamp(record, name, field):
    if field not in record:
        return
    value = record[field]
    if not isinstance(value, str):
        raise BadInput(f"{name}.{field}", "must be a parseable timestamp")
    try:
        parse(value)
    except ValueError:
        raise BadInput(f"{name}.{field}", "must be a parseable timestamp") from None


def validate_record(value, name, *, timestamps=(), strings=(), string_lists=()):
    if value is None:
        return
    if not isinstance(value, dict):
        raise BadInput(f"{name} record", "must be a JSON object or null")
    record = value
    for field in timestamps:
        validate_timestamp(record, name, field)
    for field in strings:
        if field in record and not isinstance(record[field], str):
            raise BadInput(f"{name}.{field}", "must be a string")
    for field in string_lists:
        if field in record:
            require_string_list(record[field], f"{name}.{field}")
    if name == "failure" and "count" in record:
        count = record["count"]
        if isinstance(count, bool) or not isinstance(count, int) or count < 0:
            raise BadInput("failure.count", "must be a non-negative integer")
    if (
        name == "failure"
        and record.get("count", 0) > 0
        and "first_timestamp" not in record
    ):
        raise BadInput(
            "failure.first_timestamp", "is required when failure.count is positive"
        )


def could_not_read(summary, out, field, message):
    with open(summary, "a") as f:
        f.write(f"\n## Spec parity: ❌ could not read {field}\n\n{field}: {message}.\n")
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, "status"), "w") as f:
        f.write("error")
    with open(os.path.join(out, "sleep"), "w") as f:
        f.write("0")
    return 0


def hours(a, b):
    return (b - a).total_seconds() / 3600.0


def fmt_h(h):
    if h is None:
        return "—"
    return f"{h:,.1f} h ({h / 24:,.1f} days)"


def stale_allowances(mcpx, suite, allow_mcpx, allow_suite):
    """Allow-list entries that no longer describe a one-sided revision."""
    out = []
    for v in allow_mcpx:
        if v in mcpx and v in suite:
            out.append((v, "only-mcpx", "both sides have it now"))
        elif v not in mcpx:
            out.append((v, "only-mcpx", "mcpx does not have it"))
    for v in allow_suite:
        if v in mcpx and v in suite:
            out.append((v, "only-suite", "both sides have it now"))
        elif v not in suite:
            out.append((v, "only-suite", "the suite does not have it"))
    return out


def decide(
    mcpx, suite, now, run, success, failure, on_main, allow_mcpx=(), allow_suite=()
):
    """Returns (status, sleep, new_success, new_failure, report_lines)."""
    mcpx, suite = sorted(set(mcpx)), sorted(set(suite))
    stale = stale_allowances(mcpx, suite, allow_mcpx, allow_suite)
    if stale:
        lines = [
            "## Spec parity: ❌ ERROR — an allowance is dead code",
            "",
            "> [!CAUTION]",
            "> An entry in the workflow's allow list no longer describes a revision only one "
            "side has. **Remove it** from `.github/workflows/mcpx-conformance.yml`: left in place it "
            "would hide a real mismatch on that revision later.",
            "",
            "| revision | list | why it is dead |",
            "| --- | --- | --- |",
        ]
        lines += [f"| `{v}` | `{lst}` | {why} |" for v, lst, why in stale]
        lines += [
            "",
            "mcpx: "
            + ", ".join(f"`{v}`" for v in mcpx)
            + ". Suite: "
            + ", ".join(f"`{v}`" for v in suite)
            + ".",
        ]
        return "error", 0, None, None, lines
    only_mcpx = [v for v in mcpx if v not in suite and v not in allow_mcpx]
    only_suite = [v for v in suite if v not in mcpx and v not in allow_suite]
    allowed_note = [v for v in mcpx if v not in suite and v in allow_mcpx] + [
        v for v in suite if v not in mcpx and v in allow_suite
    ]
    both = [v for v in mcpx if v in suite]
    nowt = parse(now)
    success = success or {}
    failure = failure or {}
    lines = []

    if not only_mcpx and not only_suite:
        new_success = {"timestamp": now, "run": run, "specs": mcpx}
        new_failure = dict(failure, count=0) if failure else None
        lines += [
            "## Spec parity: ✅ match",
            "",
            "mcpx: "
            + ", ".join(f"`{v}`" for v in mcpx)
            + ". Suite: "
            + ", ".join(f"`{v}`" for v in suite)
            + ".",
        ]
        if allowed_note:
            lines += [
                "",
                "Allowed by the workflow's lists, so not counted: "
                + ", ".join(f"`{v}`" for v in allowed_note)
                + ".",
            ]
        if failure.get("count"):
            lines += [
                "",
                f"This ends a run of **{failure['count']}** mismatched runs on main "
                f"that began {failure.get('first_timestamp')}.",
            ]
        return "ok", 0, new_success, new_failure, lines

    # A mismatch.
    prev_latest_ts = failure.get("latest_timestamp")
    prev_latest_run = failure.get("latest_run")
    if not failure.get("count"):
        new_failure = {
            "count": 1,
            "first_timestamp": now,
            "first_run": run,
            "latest_timestamp": now,
            "latest_run": run,
        }
    else:
        new_failure = dict(
            failure, count=failure["count"] + 1, latest_timestamp=now, latest_run=run
        )
    # What the report describes: what main will have recorded once this run is
    # saved, or, off main, what main has recorded so far.
    shown = (
        new_failure if on_main else (failure if failure.get("count") else new_failure)
    )

    last_ok = success.get("timestamp")
    baseline_ts = last_ok or shown["first_timestamp"]
    baseline_label = (
        "last success" if last_ok else "first failure (no success on record)"
    )
    since = hours(parse(baseline_ts), nowt)
    two_sided = bool(only_mcpx) and bool(only_suite)
    deadline = BOTH_SIDED_HOURS if two_sided else ONE_SIDED_HOURS
    remaining = deadline - since
    status = "error" if since > deadline else "warn"
    sleep = min(int(since), SLEEP_CAP)

    gap_ok_to_first = (
        hours(parse(last_ok), parse(shown["first_timestamp"])) if last_ok else None
    )
    gap_prev_to_now = hours(parse(prev_latest_ts), nowt) if prev_latest_ts else None

    head = (
        "## Spec parity: ❌ ERROR — deadline passed"
        if status == "error"
        else "## Spec parity: ⚠️ mismatch"
    )
    kind = (
        "**both ways**: each side has revisions the other lacks"
        if two_sided
        else "**one way**: "
        + (
            "mcpx has revisions the suite does not"
            if only_mcpx
            else "the suite has revisions mcpx does not"
        )
    )
    callout = "CAUTION" if status == "error" else "WARNING"
    lines += [
        head,
        "",
        f"> [!{callout}]",
        f"> mcpx and the official conformance suite disagree on which MCP revisions exist, {kind}.",
    ]
    if status == "error":
        lines.append(
            f"> It has been {fmt_h(since)} since the {baseline_label}, past the "
            f"{deadline}-hour deadline for a {'two' if two_sided else 'one'}-sided mismatch. "
            "**This job now fails** until the lists match."
        )
    else:
        lines.append(
            f"> **After {deadline} hours** from the {baseline_label} this becomes an error and "
            f"the job fails. That is {fmt_h(remaining)} from now, at "
            f"{(parse(baseline_ts) + dt.timedelta(hours=deadline)).strftime('%Y-%m-%dT%H:%M:%SZ')}."
        )
    lines.append(
        f"> This job will now sleep **{sleep} seconds** (one per hour since the "
        f"{baseline_label}{', capped' if sleep == SLEEP_CAP else ''}) before finishing "
        f"{'red' if status == 'error' else 'green'}."
    )
    lines += ["", "| revision | mcpx | suite |", "| --- | --- | --- |"]
    for v in sorted(set(mcpx) | set(suite)):
        lines.append(
            f"| `{v}` | {'✓' if v in mcpx else '**missing**'} | {'✓' if v in suite else '**missing**'} |"
        )
    lines += [
        "",
        f"Only in mcpx: {', '.join(f'`{v}`' for v in only_mcpx) or 'none'}. "
        f"Only in the suite: {', '.join(f'`{v}`' for v in only_suite) or 'none'}. "
        f"In both: {len(both)}.",
        "",
        "| | when | run |",
        "| --- | --- | --- |",
        f"| now | {now} | {run} |",
        f"| last success on main | {last_ok or 'never'} | {success.get('run', '—')} |",
        f"| first failure on main | {shown['first_timestamp']} | {shown.get('first_run', '—')} |",
        f"| latest failure before this run | {prev_latest_ts or 'none'} | {prev_latest_run or '—'} |",
        "",
        "| | |",
        "| --- | --- |",
        f"| failures on main in this run of them | **{shown['count']}**"
        f"{' (this run included)' if on_main else ' (this run is not on main and is not counted)'} |",
        f"| hours since the {baseline_label} | {fmt_h(since)} |",
        f"| from last success to first failure | {fmt_h(gap_ok_to_first)} |",
        f"| from the latest failure before this run to now | {fmt_h(gap_prev_to_now)} |",
        f"| deadline | {deadline} h ({'two' if two_sided else 'one'}-sided) |",
        f"| until the deadline | {fmt_h(remaining) if remaining > 0 else '**passed** ' + fmt_h(-remaining) + ' ago'} |",
    ]
    if last_ok and success.get("specs"):
        lines += [
            "",
            "At the last success both sides were: "
            + ", ".join(f"`{v}`" for v in success["specs"])
            + ".",
        ]
    return status, sleep, None, new_failure, lines


def main(argv):
    (mcpx_raw, suite_raw, now, run, succ_raw, fail_raw, on_main, summary, out) = argv[
        1:10
    ]
    try:
        allow_mcpx = (
            load_json(argv[10], "MCPX allow list")
            if len(argv) > 10 and argv[10].strip()
            else []
        )
        allow_suite = (
            load_json(argv[11], "suite allow list")
            if len(argv) > 11 and argv[11].strip()
            else []
        )
        mcpx = load_json(mcpx_raw, "MCPX revisions")
        suite = load_json(suite_raw, "suite revisions")
        require_string_list(mcpx, "MCPX revisions")
        require_string_list(suite, "suite revisions")
        require_string_list(allow_mcpx, "MCPX allow list")
        require_string_list(allow_suite, "suite allow list")
        success = load_json(succ_raw, "success record") if succ_raw.strip() else None
        failure = load_json(fail_raw, "failure record") if fail_raw.strip() else None
        validate_record(
            success,
            "success",
            timestamps=("timestamp",),
            strings=("run",),
            string_lists=("specs",),
        )
        validate_record(
            failure,
            "failure",
            timestamps=("first_timestamp", "latest_timestamp"),
            strings=("first_run", "latest_run"),
        )
        parse(now)
    except BadInput as exc:
        return could_not_read(summary, out, exc.field, exc.message)
    except ValueError:
        return could_not_read(summary, out, "now", "must be a parseable timestamp")
    if not mcpx or not suite:
        with open(summary, "a") as f:
            f.write(
                "\n## Spec parity: ❌ could not read the lists\n\n"
                f"mcpx: `{mcpx_raw}`; suite: `{suite_raw}`. An empty list means the command that "
                "produces it changed; fix the extraction rather than the comparison.\n"
            )
        os.makedirs(out, exist_ok=True)
        open(os.path.join(out, "status"), "w").write("error")
        open(os.path.join(out, "sleep"), "w").write("0")
        return 0
    status, sleep, new_s, new_f, lines = decide(
        mcpx,
        suite,
        now,
        run,
        success,
        failure,
        on_main == "true",
        allow_mcpx,
        allow_suite,
    )
    os.makedirs(out, exist_ok=True)
    open(os.path.join(out, "status"), "w").write(status)
    open(os.path.join(out, "sleep"), "w").write(str(sleep))
    if new_s is not None and new_s != success:
        json.dump(
            new_s, open(os.path.join(out, "success.json"), "w"), separators=(",", ":")
        )
    if new_f is not None and new_f != failure:
        json.dump(
            new_f, open(os.path.join(out, "failure.json"), "w"), separators=(",", ":")
        )
    with open(summary, "a") as f:
        f.write("\n" + "\n".join(lines) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
