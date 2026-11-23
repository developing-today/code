#!/usr/bin/env python3
"""Placement view of the fleet machine registry (#2179, framework v1.1).

The reader half of the task-placement framework's machine dimension
(doc/2026-11-03T15-10-56Z_design_task-placement-framework-v1, §8): the
framework picks an option, then a pool/route, then — v1.1 — a machine.
This script is how a scheduler reads the machine step off the registry
(doc/fleet-machine-registry.yaml, shape per the onboarding standard
doc/2026-11-04T05-15-02Z_design_machine-onboarding-standard-v1 §3).

What it derives, per record (rules M1–M4 of framework §8):

- M1, the declared-state gate (§2/§3/§4): only a record in state
  `active` may receive new work. `declared`/`joining` have not passed
  §2 steps 5–6; `draining` takes no new work (§4); `retired` is
  terminal. The declared state is never edited or second-guessed here.
- M2, the health gate (§1.5/§4): for an active record, the health view
  is the watchdog module's `placement_view` — imported, not
  reimplemented, so the rule has exactly one home. Fresh health reads
  as its earned status; a `last_seen` older than two watchdog cadences
  (or absent) reads as `left`: silence is a state change, never a pass.
- M3, eligibility and preference: tier 1 = active + healthy; tier 2 =
  active + degraded (placeable only behind every tier-1 machine that
  can serve; the failed checks are named in the view so the scheduler
  sees the axis). Everything else is ineligible, with the reason:
  unreachable and left are the departure shapes (§4), and unknown is
  unearned — a gating check that could not run must not mask a failure
  (§1.5: healthy is earned whole). Within a tier this reader does not
  rank; it exposes the facts a scheduler ranks by (declared capacity,
  the sessions facts beside the §1.3 cap — activity is not concurrency,
  so the cap is reported, not gated on).
- M4, capability requirements (§1.4): `--require NAME` (repeatable)
  keeps only machines whose declared capabilities carry NAME as true.
  Declarations are the whole truth — a name no record declares is a
  usage error, not an empty result.

Argument contract (#3352): this script declares its whole command
line — exactly one positional argument, REGISTRY (path to the registry
YAML), plus the optional flags below — and parses strictly against
that declaration before any file is read. Anything unaccounted for is
a usage error (exit 2). The registry is validated with the §3
validator before any view is derived; an invalid registry is refused
(exit 1), never read leniently. Requires PyYAML. No other dependencies.
"""

import argparse
import importlib.util
import json
import os
import sys
from datetime import datetime, timezone

import yaml

HERE = os.path.dirname(os.path.abspath(__file__))


def _load_module(name, filename):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


validator = _load_module(
    "validate_fleet_machine_registry", "validate-fleet-machine-registry.py"
)
watchdog = _load_module("fleet_machine_watchdog", "fleet-machine-watchdog.py")

STATE_REASONS = {
    "declared": "declared only — §2 steps 5-6 (first real work, watchdog coverage) not passed",
    "joining": "still joining — §2 steps 5-6 (first real work, watchdog coverage) not passed",
    "draining": "draining — no new work (§4)",
    "retired": "retired — terminal",
}

VIEW_REASONS = {
    "unreachable": "unreachable — the departure shape (§4); placement stops",
    "left": "health stale or never seen — treated as left the pool (§1.5)",
    "unknown": "health unearned — a gating check could not run (§1.5)",
}


def failed_check_names(health):
    """Gating checks the watchdog recorded as failed, in canonical order."""
    checks = (health or {}).get("checks") or {}
    return [
        name
        for name in watchdog.GATING_CHECKS
        if (checks.get(name) or {}).get("state") == "failed"
    ]


def machine_view(record, now, require=()):
    """The placement view of one registry record (framework §8, M1–M4)."""
    capabilities = record.get("capabilities") or {}
    capacity = record.get("capacity") or {}
    health = record.get("health") or {}
    sessions = (health.get("checks") or {}).get("sessions") or {}
    view = {
        "id": record["id"],
        "state": record["state"],
        "health_status": health.get("status"),
        "health_last_seen": health.get("last_seen"),
        "view": None,
        "tier": None,
        "eligible": False,
        "reason": None,
        "capabilities": capabilities,
        "capacity": capacity,
        "sessions": {
            "listed": sessions.get("listed"),
            "active_within_cadence": sessions.get("active_within_cadence"),
            "max_sessions": capacity.get("max_sessions"),
        },
    }

    missing = [name for name in require if capabilities.get(name) is not True]
    if missing:
        view["reason"] = (
            "missing declared capability: " + ", ".join(missing) + " (§1.4)"
        )
        return view

    if record["state"] != "active":
        view["reason"] = STATE_REASONS.get(
            record["state"], f"state {record['state']} may not receive work"
        )
        return view

    health_view = watchdog.placement_view(health, now)
    view["view"] = health_view
    if health_view == "healthy":
        view["tier"] = 1
        view["eligible"] = True
    elif health_view == "degraded":
        view["tier"] = 2
        view["eligible"] = True
        failed = failed_check_names(health)
        view["reason"] = (
            "degraded — failed checks: " + (", ".join(failed) if failed else "none named")
        )
    else:
        view["reason"] = VIEW_REASONS.get(
            health_view, f"health view {health_view} may not receive work"
        )
    return view


def order_views(views):
    """Eligible machines first (tier, then id); ineligible after, by id."""
    eligible = sorted(
        (v for v in views if v["eligible"]), key=lambda v: (v["tier"], v["id"])
    )
    ineligible = sorted((v for v in views if not v["eligible"]), key=lambda v: v["id"])
    return eligible + ineligible


def render_text(view):
    caps = ",".join(name for name, on in view["capabilities"].items() if on is True)
    sess = view["sessions"]
    sess_text = (
        f"sessions listed={sess['listed']} active={sess['active_within_cadence']} "
        f"cap={sess['max_sessions']}"
    )
    if view["eligible"]:
        head = f"tier {view['tier']}"
    else:
        head = "ineligible"
    line = (
        f"{view['id']}: {head} (state={view['state']}, view={view['view']}, "
        f"health={view['health_status']}, last_seen={view['health_last_seen']}) "
        f"caps={caps} {sess_text}"
    )
    if view["reason"]:
        line += f" — {view['reason']}"
    return line


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Derive the placement view of the fleet machine registry "
        "(#2179; task-placement framework v1.1, §8)."
    )
    parser.add_argument("registry", help="path to the fleet machine registry YAML")
    parser.add_argument("--machine", help="only this machine id (default: all records)")
    parser.add_argument(
        "--require",
        action="append",
        default=[],
        metavar="CAPABILITY",
        help="require a declared capability (repeatable; §1.4)",
    )
    parser.add_argument(
        "--now",
        default=None,
        help="evaluation time, ISO-8601 UTC (default: now); "
        "exists so runs are reproducible and testable",
    )
    parser.add_argument(
        "--json", action="store_true", help="emit the views as JSON instead of text"
    )
    args = parser.parse_args(argv)

    now = watchdog.parse_iso(args.now) if args.now else datetime.now(timezone.utc)
    if now is None:
        parser.error(f"--now {args.now!r} is not ISO-8601 UTC (YYYY-MM-DDTHH:MM:SSZ)")

    try:
        with open(args.registry, encoding="utf-8") as fh:
            doc = yaml.safe_load(fh)
    except (OSError, yaml.YAMLError) as exc:
        print(f"error: cannot read registry: {exc}", file=sys.stderr)
        return 1
    errors = validator.validate(doc)
    if errors:
        for err in errors:
            print(f"error: registry invalid: {err}", file=sys.stderr)
        return 1

    declared_caps = set()
    for record in doc["machines"]:
        declared_caps.update((record.get("capabilities") or {}).keys())
    unknown_caps = [name for name in args.require if name not in declared_caps]
    if unknown_caps:
        parser.error(
            "--require names no record declares: " + ", ".join(unknown_caps)
        )

    targets = [
        r for r in doc["machines"] if args.machine is None or r["id"] == args.machine
    ]
    if not targets:
        print(f"error: machine {args.machine!r} not in registry", file=sys.stderr)
        return 1

    views = order_views([machine_view(r, now, args.require) for r in targets])
    placeable = [v["id"] for v in views if v["eligible"]]
    if args.json:
        print(
            json.dumps(
                {
                    "now": watchdog.iso(now),
                    "require": args.require,
                    "machines": views,
                    "placeable": placeable,
                },
                indent=2,
            )
        )
    else:
        for view in views:
            print(render_text(view))
        print("placeable: " + (", ".join(placeable) if placeable else "(none)"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
