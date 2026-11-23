#!/usr/bin/env python3
"""Validate doc/fleet-machine-registry.yaml against the §3 registry shape.

Argument contract (#3352): this script declares its whole command line —
exactly one positional argument, REGISTRY (path to the registry YAML) —
and parses strictly against that declaration before any file is read.
Anything unaccounted for is a usage error with a non-zero exit.

The shape enforced here is the one defined by the machine onboarding
standard (doc/2026-11-04T05-15-02Z_design_machine-onboarding-standard-v1,
§3) for issue #2179:

- top level is a mapping with a non-empty `machines` list;
- every record carries id, hostname, state, roles, links, access,
  capabilities, capacity, secrets, health;
- state is one of declared|joining|active|draining|retired and ids are
  unique (an id is never reused after `retired`, so duplicates are an
  error regardless of state);
- roles are drawn from execution|lab-cluster, plus `both` only as the
  single recorded-decision marker of §0 (a record listing `both`
  alongside other roles is an error);
- an `active` record must declare all four capabilities and positive
  cpu_cores / ram_gb / worktree_disk_gb (declarations, not probes);
- secrets entries are class names only (§1.6): lowercase slugs, never
  value-shaped strings;
- health.status is one of healthy|degraded|unreachable|unknown and
  health.source names who reports;
- health is an EARNED record (§1.5): a status other than `unknown`, or
  any non-null `last_seen`, is valid only with the watchdog's checks
  record attached — `checks.reported_at` plus all five §1.5 checks
  (reachable, load_headroom, disk_headroom, sessions, last_queue_job),
  each a mapping whose state is ok|failed|unknown. A bare hand-written
  `status: healthy` is a fabricated pass and fails here. `last_seen`,
  when present, must be ISO-8601.

Exit 0 and one `ok:` line per record on success; exit 1 with one
`error:` line per violation otherwise. Requires PyYAML.
"""

import argparse
import re
import sys
from datetime import datetime

import yaml

STATES = {"declared", "joining", "active", "draining", "retired"}
ROLES = {"execution", "lab-cluster", "both"}
HEALTH_STATUSES = {"healthy", "degraded", "unreachable", "unknown"}
HEALTH_CHECKS = {
    "reachable",
    "load_headroom",
    "disk_headroom",
    "sessions",
    "last_queue_job",
}
CHECK_STATES = {"ok", "failed", "unknown"}
CAPABILITIES = {"queue_worker", "opencode_sessions", "builds", "gpu"}
SECRET_CLASS = re.compile(r"^[a-z0-9]+(-[a-z0-9]+)*$")
REQUIRED_FIELDS = {
    "id",
    "hostname",
    "state",
    "roles",
    "links",
    "access",
    "capabilities",
    "capacity",
    "secrets",
    "health",
}


def validate(doc):
    """Return a list of violation strings; empty means the doc is valid."""
    errors = []
    if not isinstance(doc, dict) or not isinstance(doc.get("machines"), list):
        return ["top level must be a mapping with a `machines` list"]
    machines = doc["machines"]
    if not machines:
        errors.append("`machines` list is empty")
    seen_ids = set()
    for i, rec in enumerate(machines):
        where = f"machines[{i}]"
        if not isinstance(rec, dict):
            errors.append(f"{where}: record is not a mapping")
            continue
        rid = rec.get("id")
        if isinstance(rid, str) and rid:
            where = f"machine {rid!r}"
            if rid in seen_ids:
                errors.append(f"{where}: duplicate id (ids are never reused)")
            seen_ids.add(rid)
        missing = REQUIRED_FIELDS - set(rec)
        if missing:
            errors.append(f"{where}: missing fields {sorted(missing)}")
            continue
        if rec["state"] not in STATES:
            errors.append(f"{where}: state {rec['state']!r} not in {sorted(STATES)}")
        roles = rec["roles"]
        if not isinstance(roles, list) or not roles or not set(roles) <= ROLES:
            errors.append(
                f"{where}: roles {roles!r} must be a non-empty subset of {sorted(ROLES)}"
            )
        elif "both" in roles and len(roles) > 1:
            errors.append(
                f"{where}: `both` is a standalone marker (§0), not combinable"
            )
        access = rec["access"]
        if (
            not isinstance(access, dict)
            or not access.get("ssh")
            or not access.get("opencode_server")
        ):
            errors.append(f"{where}: access must name ssh and opencode_server")
        caps = rec["capabilities"]
        if not isinstance(caps, dict) or set(caps) != CAPABILITIES:
            errors.append(
                f"{where}: capabilities must be exactly {sorted(CAPABILITIES)}"
            )
        elif not all(isinstance(v, bool) for v in caps.values()):
            errors.append(f"{where}: capability values must be booleans")
        capacity = rec["capacity"]
        if not isinstance(capacity, dict):
            errors.append(f"{where}: capacity must be a mapping")
        elif rec["state"] == "active":
            for key in ("cpu_cores", "ram_gb", "worktree_disk_gb"):
                if (
                    not isinstance(capacity.get(key), (int, float))
                    or capacity[key] <= 0
                ):
                    errors.append(
                        f"{where}: active machine must declare positive capacity.{key}"
                    )
            if (
                not isinstance(capacity.get("max_sessions"), int)
                or capacity["max_sessions"] <= 0
            ):
                errors.append(
                    f"{where}: active machine must declare positive capacity.max_sessions"
                )
        secrets = rec["secrets"]
        if not isinstance(secrets, list) or not all(
            isinstance(s, str) and SECRET_CLASS.match(s) for s in secrets
        ):
            errors.append(
                f"{where}: secrets must be secret CLASS names only (§1.6 slugs)"
            )
        health = rec["health"]
        if not isinstance(health, dict) or health.get("status") not in HEALTH_STATUSES:
            errors.append(
                f"{where}: health.status must be one of {sorted(HEALTH_STATUSES)}"
            )
        elif not health.get("source"):
            errors.append(f"{where}: health.source must name who reports (§1.5)")
        if isinstance(health, dict):
            errors.extend(_health_earned_errors(where, health))
    return errors


def _is_iso8601(value):
    if not isinstance(value, str):
        return False
    try:
        datetime.fromisoformat(value.replace("Z", "+00:00"))
        return True
    except ValueError:
        return False


def _health_earned_errors(where, health):
    """The §1.5 earning rules: health is written by the watchdog from
    checks that actually ran, never asserted by hand."""
    errors = []
    last_seen = health.get("last_seen")
    if last_seen is not None and not _is_iso8601(last_seen):
        errors.append(f"{where}: health.last_seen must be ISO-8601 or null")
    checks = health.get("checks")
    if checks is not None:
        if not isinstance(checks, dict):
            errors.append(f"{where}: health.checks must be a mapping")
            checks = None
        else:
            if not _is_iso8601(checks.get("reported_at")):
                errors.append(f"{where}: health.checks.reported_at must be ISO-8601")
            for name, entry in checks.items():
                if name == "reported_at":
                    continue
                if (
                    not isinstance(entry, dict)
                    or entry.get("state") not in CHECK_STATES
                ):
                    errors.append(
                        f"{where}: health.checks.{name} must be a mapping "
                        f"with state in {sorted(CHECK_STATES)}"
                    )
    earned = isinstance(checks, dict) and HEALTH_CHECKS <= set(checks)
    if health.get("status") != "unknown" and not earned:
        errors.append(
            f"{where}: health.status {health.get('status')!r} without the "
            "watchdog checks record is a fabricated pass (§1.5)"
        )
    if last_seen is not None and checks is None:
        errors.append(
            f"{where}: health.last_seen set without the watchdog checks "
            "record (§1.5: last_seen is watchdog-written)"
        )
    return errors


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Validate a fleet machine registry against the #2179 §3 shape."
    )
    parser.add_argument("registry", help="path to the registry YAML file")
    args = parser.parse_args(argv)
    try:
        with open(args.registry, encoding="utf-8") as fh:
            doc = yaml.safe_load(fh)
    except OSError as exc:
        print(f"error: cannot read {args.registry}: {exc}", file=sys.stderr)
        return 1
    except yaml.YAMLError as exc:
        print(f"error: {args.registry} is not valid YAML: {exc}", file=sys.stderr)
        return 1
    errors = validate(doc)
    if errors:
        for err in errors:
            print(f"error: {err}", file=sys.stderr)
        return 1
    for rec in doc["machines"]:
        print(f"ok: {rec['id']} ({rec['state']})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
