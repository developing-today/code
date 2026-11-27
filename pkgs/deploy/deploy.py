#!/usr/bin/env python3
"""deploy — the deployment-architecture CLI (issue #5766, slices 1-3;
issue #5770, slice 4).

Slice 1 ships the registry foundation:  enumerates the
 registry (name, class, targets) after validating every
definition against the design §3 option shape
(doc/2026-11-04_deployment-architecture-services-layer.md).

Slice 2 added the command logic that does not itself activate anything:

- systemd unit rendering (and a launchd plist stub) from a definition:
  the unit's ExecStart points at the service's per-service nix profile
  `current` link (design §4.1), never at a bare store path, so a
  generation flip is a profile repoint, not a unit rewrite.
- `deploy <target> --dry-run`: the ordered activation plan (build,
  install the next generation into the profile, write the unit,
  daemon-reload, restart only that unit, health check, flip back on
  failure) with no side effects.
- The generation log: one append-only JSONL file per target under the
  state dir (design §4.6: generation id, source rev, artifact,
  timestamp, actor).  folds the log into
  current/previous generations and reports health;  selects
  the previous distinct generation — repointing, never rebuilding.
- Slice 3 adds real activation for the systemd class: installing a
  generation into the per-service profile (a deploy-managed profile
  link whose generation directories hold a `current` symlink at the
  artifact; store-path artifacts are additionally registered with
  `nix-env --set` on a parallel nix profile), writing the unit,
  daemon-reload, restarting only that unit, running the health
  check, and flipping the profile back to the previous generation
  (and restarting on it) on any failure. Rollback is the same
  machinery pointed at the previous distinct generation. The
  launchd real path still refuses explicitly: it cannot be exercised
  from the Linux fleet and stays a rendering/planning stub.

Slice 4 (issue #5770, migration M3) adds the sandbox class of design
§4.4 — the first sandbox target is quota-recorder:

- A definition may carry a second optional field beyond the §3
  set: `timer` ({onCalendar, persistent, randomizedDelaySec}), the
  only way to name a timer-driven oneshot service. It is optional
  for every class; a definition without it validates exactly as
  before.
- Sandbox activation mirrors §4.1 with the sandbox's twists: the
  per-service profile lives in the persistent home
  (~/.local/share/deploy/profiles by default — /etc does not
  survive a sandbox recycle, home does), the rendered unit(s) are
  stored inside the generation directory, and the target is
  recorded in the sandbox manifest (state dir: sandbox-manifest.json),
  the list of currently-activated sandbox targets.
- Activation finishes with the restore step: link the current
  generation's rendered units into the unit dir (/etc/systemd/system),
  daemon-reload, enable --now the timer (or the service, for a
  daemon-shaped target), verify it is active (plus the healthCheck
  URL probe when one is defined). Any failure flips back exactly
  like the systemd class. Rollback is the same repoint + re-link.
- `deploy restore` re-runs that restore step for every target in
  the manifest, with no registry needed: it is the generic unit
  restorer the sandbox bootstrap invokes after a recycle (design
  §4.4). pkgs/deploy/restore.py is the same contract as a
  standalone stdlib script, for boots where no built deploy CLI
  is at hand yet.

The release slice (issue #13306, implementing the release-workflow
design record doc/2026-11-23T13-09-25Z_design_release-workflow-per-app-tags,
#11177) adds the estate's release actor to the same tool:

- `deploy release <app> [--version X.Y.Z] [--commit <sha>]
  [--bump minor|patch]`: verifies the commit is reachable from
  origin/main, judges the attestation gate for that exact commit
  (the main-gate watched set, judged by the same rules
  .github/scripts/release-gate.sh implements, plus the app's own
  evidence workflow when the watched set does not cover it), checks
  the version against the ratified #5790 discipline (short app-name
  prefix, first tag 0.1.0, 0.x only, next minor or next patch only),
  cuts the annotated app tag carrying the release-attestation:v1
  block and the annotated repo tag for the release point, pushes
  both, and appends the release record to the release ledger in the
  state dir (releases.jsonl, beside the generation logs).
- The deploy-time refusal (design §6): a deploy whose artifact
  self-reports a clean X.Y.Z version claims release status and is
  refused unless an attested <app>/vX.Y.Z tag exists at the
  artifact's source commit with a verifiable attestation block.
  Prerelease (-dev) artifacts are unaffected.
- `deploy status` reads the release ledger beside the per-target
  report; where the ledger and the tags disagree, the tag wins and
  the disagreement is reported.
- A definition may carry one more optional field, `evidenceWorkflow`:
  the name of the app's own test workflow when it sits outside the
  main-gate watched set (design §3 part 2). Absent means the watched
  set is the app's whole attestation.

The registry is JSON baked into the package at build time (the Nix
expression renders the attrset definitions, replacing each
derivation with its store path, without depending on it: building the
CLI realizes no target package; a deploy builds its target's package
from the derivation $DEPLOY_TARGET_DRVS records, code#12738). `--registry PATH` (or $DEPLOY_REGISTRY)
points the CLI at another registry file, which is how the flake checks
exercise the malformed-definition path. `--state-dir PATH` (or
$DEPLOY_STATE_DIR) relocates the generation logs, `--profile-base PATH`
(or $DEPLOY_PROFILE_BASE) the per-service profile root; both exist so
tests and dry runs never touch the real profile tree. Slice 3 adds
`--unit-dir PATH` (or $DEPLOY_UNIT_DIR) for the rendered unit's home
and `--scope system|user` (or $DEPLOY_SCOPE, default system) selecting
the systemctl scope; user scope exists so the laptop E2E acceptance
can deploy a harmless service without root.

The validation below is the oracle for "well-formed definition": a
definition is valid iff it carries exactly the §3 fields (plus
the optional fields named in OPTIONAL_FIELDS below),
each of the right type and in range. Any violation is reported by field
and makes  exit non-zero — a registry that cannot be validated
must never be enumerated as if it were trustworthy.
"""

import getpass
import json
import os
import re
import shlex
import subprocess
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from urllib.parse import urlsplit
from xml.sax.saxutils import escape as xml_escape

CLASSES = {"systemd", "launchd", "cloudflare", "sandbox", "container"}
RESTART_POLICIES = {
    "no",
    "on-success",
    "on-failure",
    "on-abnormal",
    "on-watchdog",
    "on-abort",
    "always",
}
FIELDS = {
    "package",
    "entryPoint",
    "args",
    "environment",
    "secrets",
    "user",
    "ports",
    "restart",
    "healthCheck",
    "targets",
    "class",
}
# Optional fields: absent means the default below. healthTimeoutSeconds is
# how long activation keeps probing the healthCheck before it calls the
# deploy failed (#11170). timer (slice 4, #5770) names a timer-driven
# oneshot service — the §3 shape had no way to express one — and when
# present must carry exactly TIMER_FIELDS. evidenceWorkflow (release
# slice, #13306) names the app's own test workflow when it sits outside
# the main-gate watched set, per design §3 part 2 of the release
# workflow record.
OPTIONAL_FIELDS = {
    "healthTimeoutSeconds",
    "timer",
    "evidenceWorkflow",
    "restartDelaySec",
    "startLimitIntervalSec",
}
TIMER_FIELDS = {"onCalendar", "persistent", "randomizedDelaySec"}
DEFAULT_HEALTH_TIMEOUT_SECONDS = 120
MAX_HEALTH_TIMEOUT_SECONDS = 3600
# restartDelaySec / startLimitIntervalSec (#15533): the systemd
# RestartSec / StartLimitIntervalSec a rendered unit carries. Absent
# means systemd's defaults (100ms restart delay; start limit of 5
# starts per 10s) — the defaults that let the 2026-11-25 broker outage
# end in start-limit-hit. startLimitIntervalSec = 0 is systemd's
# spelling for "rate limiting disabled": the manager never gives up
# on the unit, so a boot-window failure cascade cannot leave a
# boot-critical daemon down until a manual reset-failed.
MAX_RESTART_DELAY_SECONDS = 3600
MAX_START_LIMIT_INTERVAL_SECONDS = 86400
HEALTH_PROBE_INTERVAL_SECONDS = 0.5
NAME_RE = re.compile(r"^[a-z0-9][a-z0-9-]*$")

# Design §4.1: each systemd target owns a per-service nix profile under
# this root (system scope); the rendered unit points at the profile's
# `current` link.
DEFAULT_PROFILE_BASE = "/nix/var/nix/profiles/per-service"
ACTIVATION_CLASSES = {"systemd", "launchd", "sandbox"}

# Cloudflare lifecycle (issue #5767, slice 4: cf-container-hello through
# the generic deploy CLI). Only the targets in this set have a
# Cloudflare activation path in this slice; every other cloudflare (or
# container) target still reports the different-slice refusal, so the
# slice-2 "refuses other classes" oracle stays green for non-lifecycle
# names. The required bindings are the Worker's durable-object binding
# names from pkgs/cf-container-hello/wrangler.jsonc: a version that
# does not carry them is refused before any mutation (rollback) or
# recorded as failed (deploy).
CLOUDFLARE_LIFECYCLE_TARGETS = frozenset({"cf-container-hello"})
REQUIRED_CF_CONTAINER_HELLO_BINDINGS = (
    "HELLO_CONTAINER",
    "CRAWLER_PROBE",
    "HELLO_LITE",
)


class CloudflareError(Exception):
    """A Cloudflare provider step failed; the caller records it."""


def validate(registry):
    """Return a list of validation errors (empty = registry is valid)."""
    errors = []
    if not isinstance(registry, dict):
        return ["registry: top level must be an object mapping name -> definition"]
    if not registry:
        return ["registry: no service definitions"]
    for name, defn in sorted(registry.items()):
        where = f"service {name!r}"
        if not NAME_RE.fullmatch(name):
            errors.append(f"{where}: name must match {NAME_RE.pattern}")
        if not isinstance(defn, dict):
            errors.append(f"{where}: definition must be an object")
            continue
        missing = FIELDS - defn.keys()
        extra = defn.keys() - FIELDS - OPTIONAL_FIELDS
        if missing:
            errors.append(f"{where}: missing field(s): {', '.join(sorted(missing))}")
        if extra:
            errors.append(f"{where}: unknown field(s): {', '.join(sorted(extra))}")
        # Only type-check fields that exist; missing ones are already reported.
        if "package" in defn and not (
            isinstance(defn["package"], str) and defn["package"]
        ):
            errors.append(f"{where}: package must be a non-empty string")
        if "entryPoint" in defn:
            ep = defn["entryPoint"]
            if (
                not isinstance(ep, str)
                or not ep
                or ep.startswith("/")
                or ".." in ep.split("/")
            ):
                errors.append(
                    f"{where}: entryPoint must be a relative path inside the package"
                )
        if "args" in defn and not (
            isinstance(defn["args"], list)
            and all(isinstance(a, str) for a in defn["args"])
        ):
            errors.append(f"{where}: args must be a list of strings")
        if "environment" in defn and not (
            isinstance(defn["environment"], dict)
            and all(
                isinstance(k, str) and isinstance(v, str)
                for k, v in defn["environment"].items()
            )
        ):
            errors.append(f"{where}: environment must be an object of string -> string")
        if "secrets" in defn and not (
            isinstance(defn["secrets"], list)
            and all(isinstance(s, str) and s for s in defn["secrets"])
        ):
            errors.append(
                f"{where}: secrets must be a list of secret names (names only, never values)"
            )
        elif "secrets" in defn and len(set(defn["secrets"])) != len(defn["secrets"]):
            errors.append(f"{where}: secrets must not repeat a secret name")
        if "user" in defn and not (isinstance(defn["user"], str) and defn["user"]):
            errors.append(f"{where}: user must be a non-empty string")
        if "ports" in defn:
            ports = defn["ports"]
            if not (
                isinstance(ports, list)
                and all(
                    isinstance(p, int) and not isinstance(p, bool) and 1 <= p <= 65535
                    for p in ports
                )
            ):
                errors.append(f"{where}: ports must be a list of integers in 1..65535")
            elif len(set(ports)) != len(ports):
                errors.append(f"{where}: ports must not repeat a port")
        if "restart" in defn and not (
            isinstance(defn["restart"], str) and defn["restart"] in RESTART_POLICIES
        ):
            errors.append(
                f"{where}: restart must be one of {', '.join(sorted(RESTART_POLICIES))}"
            )
        if "healthCheck" in defn and defn["healthCheck"] is not None:
            hc = defn["healthCheck"]
            try:
                parsed = urlsplit(hc) if isinstance(hc, str) else None
                hostname = parsed.hostname if parsed is not None else None
                if parsed is not None:
                    # Accessing .port also rejects malformed/out-of-range ports.
                    parsed.port
                valid_url = (
                    parsed is not None
                    and parsed.scheme in {"http", "https"}
                    and bool(hostname)
                )
            except ValueError:
                valid_url = False
            if not valid_url:
                errors.append(
                    f"{where}: healthCheck must be an http(s) URL with a host or null"
                )
        if "healthTimeoutSeconds" in defn:
            ht = defn["healthTimeoutSeconds"]
            if not (
                isinstance(ht, int)
                and not isinstance(ht, bool)
                and 1 <= ht <= MAX_HEALTH_TIMEOUT_SECONDS
            ):
                errors.append(
                    f"{where}: healthTimeoutSeconds must be an integer in "
                    f"1..{MAX_HEALTH_TIMEOUT_SECONDS}"
                )
        if "evidenceWorkflow" in defn and not (
            isinstance(defn["evidenceWorkflow"], str) and defn["evidenceWorkflow"]
        ):
            errors.append(
                f"{where}: evidenceWorkflow must be a non-empty workflow name"
            )
        if "restartDelaySec" in defn:
            rd = defn["restartDelaySec"]
            if not (
                isinstance(rd, int)
                and not isinstance(rd, bool)
                and 0 <= rd <= MAX_RESTART_DELAY_SECONDS
            ):
                errors.append(
                    f"{where}: restartDelaySec must be an integer in "
                    f"0..{MAX_RESTART_DELAY_SECONDS}"
                )
        if "startLimitIntervalSec" in defn:
            sl = defn["startLimitIntervalSec"]
            if not (
                isinstance(sl, int)
                and not isinstance(sl, bool)
                and 0 <= sl <= MAX_START_LIMIT_INTERVAL_SECONDS
            ):
                errors.append(
                    f"{where}: startLimitIntervalSec must be an integer in "
                    f"0..{MAX_START_LIMIT_INTERVAL_SECONDS}"
                )
        if "timer" in defn:
            # Slice 4 (#5770): the timer is rendered verbatim into a
            # systemd .timer unit, so its shape is as strict as the
            # rest of the definition: exactly TIMER_FIELDS, a
            # single-line calendar spec that cannot smuggle in a
            # second unit directive, a real bool, a non-negative int.
            timer = defn["timer"]
            if not isinstance(timer, dict):
                errors.append(
                    f"{where}: timer must be an object with fields "
                    f"{', '.join(sorted(TIMER_FIELDS))}"
                )
            else:
                tmissing = TIMER_FIELDS - timer.keys()
                textra = timer.keys() - TIMER_FIELDS
                if tmissing:
                    errors.append(
                        f"{where}: timer missing field(s): "
                        f"{', '.join(sorted(tmissing))}"
                    )
                if textra:
                    errors.append(
                        f"{where}: timer unknown field(s): {', '.join(sorted(textra))}"
                    )
                if "onCalendar" in timer:
                    oc = timer["onCalendar"]
                    if not (
                        isinstance(oc, str)
                        and oc
                        and oc == oc.strip()
                        and not any(c in oc for c in "\n\r")
                        and not oc.startswith("-")
                    ):
                        errors.append(
                            f"{where}: timer.onCalendar must be a non-empty "
                            "single-line systemd calendar spec"
                        )
                if "persistent" in timer and not isinstance(timer["persistent"], bool):
                    errors.append(f"{where}: timer.persistent must be a boolean")
                if "randomizedDelaySec" in timer:
                    rds = timer["randomizedDelaySec"]
                    if not (
                        isinstance(rds, int) and not isinstance(rds, bool) and rds >= 0
                    ):
                        errors.append(
                            f"{where}: timer.randomizedDelaySec must be a "
                            "non-negative integer"
                        )
        if "targets" in defn and not (
            isinstance(defn["targets"], list)
            and defn["targets"]
            and all(isinstance(t, str) and t for t in defn["targets"])
        ):
            errors.append(
                f"{where}: targets must be a non-empty list of host/class names"
            )
        elif "targets" in defn and len(set(defn["targets"])) != len(defn["targets"]):
            errors.append(f"{where}: targets must not repeat a target")
        # Targets are printed verbatim by `deploy list`, so restrict them to
        # the same single-line identifier shape as service names.
        if (
            "targets" in defn
            and isinstance(defn["targets"], list)
            and all(isinstance(t, str) and t for t in defn["targets"])
            and any(not NAME_RE.fullmatch(t) for t in defn["targets"])
        ):
            errors.append(
                f"{where}: targets must be host/class names matching {NAME_RE.pattern}"
            )
        if "class" in defn and not (
            isinstance(defn["class"], str) and defn["class"] in CLASSES
        ):
            errors.append(f"{where}: class must be one of {', '.join(sorted(CLASSES))}")
    return errors


def _no_duplicate_keys(pairs):
    # json silently keeps only the last value of a repeated key; a
    # registry is the validation oracle, so discarded text must fail
    # the load instead of vanishing (issue #6384). This hook runs for
    # every object at every nesting level.
    obj = {}
    for key, value in pairs:
        if key in obj:
            raise ValueError(f"duplicate key {key!r} in registry")
        obj[key] = value
    return obj


def load_registry(path):
    with open(path, "r", encoding="utf-8") as fh:
        return json.load(fh, object_pairs_hook=_no_duplicate_keys)


def cmd_list(registry):
    errors = validate(registry)
    if errors:
        for err in errors:
            print(f"deploy: invalid definition: {err}", file=sys.stderr)
        return 1
    width = max(len(name) for name in registry)
    print(f"{'NAME':<{width}}  {'CLASS':<10}  TARGETS")
    for name in sorted(registry):
        defn = registry[name]
        print(f"{name:<{width}}  {defn['class']:<10}  {','.join(defn['targets'])}")
    return 0


# --- Rendering (slice 2) -------------------------------------------------
#
# Oracle: design §4.1/§4.2 and issue #5766. A rendered unit/plist points
# at the per-service profile's `current` link, so activation and
# rollback are profile repoints; the rendered file itself does not
# change between generations of the same definition.


def profile_base(explicit=None):
    return explicit or os.environ.get("DEPLOY_PROFILE_BASE") or DEFAULT_PROFILE_BASE


def profile_link(name, base=None):
    return os.path.join(profile_base(base), name)


def unit_name(name):
    return f"{name}.service"


def _systemd_quote(value):
    # systemd unit syntax is not shell: values with whitespace or
    # specials are double-quoted, with backslash/quote escaped and the
    # specifier/variable introducers (% and $) doubled.
    if value and not any(c in value for c in ' \t"\\%$'):
        return value
    escaped = (
        value.replace("\\", "\\\\")
        .replace('"', '\\"')
        .replace("%", "%%")
        .replace("$", "$$")
    )
    return f'"{escaped}"'


def render_unit(name, defn, base=None):
    """Render the systemd unit for a definition (design §4.1)."""
    link = profile_link(name, base)
    exec_start = os.path.join(link, "current", defn["entryPoint"])
    argv = " ".join(_systemd_quote(a) for a in [exec_start, *defn["args"]])
    lines = [
        "[Unit]",
        f"Description=deploy target {name} (issue #5766)",
        "After=network.target",
    ]
    if "startLimitIntervalSec" in defn:
        # [Unit] section directive (#15533): 0 disables systemd's
        # start rate limiting for this unit — see the field note at
        # OPTIONAL_FIELDS. Absent keeps the manager default.
        lines.append(f"StartLimitIntervalSec={defn['startLimitIntervalSec']}")
    lines += [
        "",
        "[Service]",
        f"ExecStart={argv}",
        f"User={defn['user']}",
        f"Restart={defn['restart']}",
    ]
    if "restartDelaySec" in defn:
        lines.append(f"RestartSec={defn['restartDelaySec']}")
    for key in sorted(defn["environment"]):
        pair = f"{key}={defn['environment'][key]}"
        lines.append(f"Environment={_systemd_quote(pair)}")
    if defn["secrets"]:
        names = ", ".join(defn["secrets"])
        lines.append(f"# secrets resolved by name at activation time: {names}")
    lines += ["", "[Install]", "WantedBy=multi-user.target", ""]
    return "\n".join(lines)


def render_plist(name, defn, base=None):
    """Launchd stub rendering (design §4.2): same definition shape,
    the agent program points at the profile `current` link."""
    link = profile_link(name, base)
    program = os.path.join(link, "current", defn["entryPoint"])
    args = [program, *defn["args"]]
    arg_xml = "\n".join(f"        <string>{xml_escape(a)}</string>" for a in args)
    env_xml = "\n".join(
        f"        <key>{xml_escape(k)}</key>\n        <string>{xml_escape(v)}</string>"
        for k, v in sorted(defn["environment"].items())
    )
    keep_alive = "true" if defn["restart"] != "no" else "false"
    return (
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<plist version="1.0">\n<dict>\n'
        f"    <key>Label</key>\n    <string>com.developing-today.deploy.{xml_escape(name)}</string>\n"
        f"    <key>ProgramArguments</key>\n    <array>\n{arg_xml}\n    </array>\n"
        f"    <key>EnvironmentVariables</key>\n    <dict>\n{env_xml}\n    </dict>\n"
        f"    <key>KeepAlive</key>\n    <{keep_alive}/>\n"
        "    <key>RunAtLoad</key>\n    <true/>\n"
        "</dict>\n</plist>\n"
    )


# --- Sandbox rendering (slice 4, issue #5770) ---------------------------
#
# Oracle: design §4.4. A sandbox target renders a SET of unit files,
# stored inside its generation directory: the service, plus a timer
# when the definition carries one. The service ExecStart points at
# the profile `current` link exactly like §4.1; the sandbox twists
# are that the unit orders itself after network-online (a sandbox
# service's first act is usually a network call, and the incumbent
# hand-copied units order this way) and that a timer-driven service
# is a oneshot with no [Install] section of its own — the timer is
# what gets enabled.


def timer_unit_name(name):
    return f"{name}.timer"


def sandbox_profile_base():
    """Default profile root for sandbox targets: the persistent home
    (design §4.4 — /etc is ephemeral, home survives a recycle)."""
    return os.path.expanduser("~/.local/share/deploy/profiles")


def render_sandbox_service(name, defn, base=None):
    """Render the .service for a sandbox definition (design §4.4)."""
    link = profile_link(name, base)
    exec_start = os.path.join(link, "current", defn["entryPoint"])
    argv = " ".join(_systemd_quote(a) for a in [exec_start, *defn["args"]])
    lines = [
        "[Unit]",
        f"Description=deploy target {name} (issue #5770)",
        "After=network-online.target",
        "Wants=network-online.target",
        "",
        "[Service]",
        f"ExecStart={argv}",
        f"User={defn['user']}",
    ]
    if defn.get("timer") is not None:
        lines.append("Type=oneshot")
    else:
        lines.append(f"Restart={defn['restart']}")
    for key in sorted(defn["environment"]):
        pair = f"{key}={defn['environment'][key]}"
        lines.append(f"Environment={_systemd_quote(pair)}")
    if defn["secrets"]:
        names = ", ".join(defn["secrets"])
        lines.append(f"# secrets resolved by name at activation time: {names}")
    if defn.get("timer") is None:
        lines += ["", "[Install]", "WantedBy=multi-user.target"]
    lines.append("")
    return "\n".join(lines)


def render_timer_unit(name, defn):
    """Render the .timer for a timer-driven definition (design §4.4)."""
    timer = defn["timer"]
    lines = [
        "[Unit]",
        f"Description=deploy target {name} timer (issue #5770)",
        "",
        "[Timer]",
        f"OnCalendar={timer['onCalendar']}",
        f"Persistent={'true' if timer['persistent'] else 'false'}",
    ]
    if timer["randomizedDelaySec"]:
        lines.append(f"RandomizedDelaySec={timer['randomizedDelaySec']}")
    lines += [
        f"Unit={unit_name(name)}",
        "",
        "[Install]",
        "WantedBy=timers.target",
        "",
    ]
    return "\n".join(lines)


def render_sandbox_units(name, defn, base=None):
    """All unit files a sandbox target renders: {filename: content}."""
    units = {unit_name(name): render_sandbox_service(name, defn, base)}
    if defn.get("timer") is not None:
        units[timer_unit_name(name)] = render_timer_unit(name, defn)
    return units


def generation_dir(name, generation, base=None):
    return os.path.join(profile_base(base), ".generations", name, str(generation))


def generation_units_dir(name, generation, base=None):
    return os.path.join(generation_dir(name, generation, base), "units")


def write_sandbox_units(name, defn, generation, base=None):
    """Render the target's units into its generation directory, where
    the restorer links them from. Returns the sorted unit filenames."""
    rendered = render_sandbox_units(name, defn, base)
    dest = generation_units_dir(name, generation, base)
    for filename, content in rendered.items():
        write_text(os.path.join(dest, filename), content)
    return sorted(rendered)


def enable_unit_for(unit_files):
    """The unit a restore enables: the timer when one was rendered,
    otherwise the service itself."""
    for filename in unit_files:
        if filename.endswith(".timer"):
            return filename
    return unit_files[0]


# --- Generation log (slice 2) --------------------------------------------
#
# Oracle: design §4.6. The log is append-only, one JSON object per
# activation event; two generations are always retained because the
# log is never truncated. Current/previous are DERIVED by folding the
# events in order: a deploy event makes its generation current, a
# rollback event makes the generation it repointed to current; the
# previous generation is the last distinct generation before the
# current one in that folded history. Rollback therefore never needs
# a rebuild: it re-selects an already-recorded generation.


def state_dir(explicit=None):
    return (
        explicit
        or os.environ.get("DEPLOY_STATE_DIR")
        or os.path.expanduser("~/.local/state/deploy")
    )


def generations_path(name, explicit_state=None):
    return os.path.join(state_dir(explicit_state), "generations", f"{name}.jsonl")


def read_generations(name, explicit_state=None):
    path = generations_path(name, explicit_state)
    if not os.path.exists(path):
        return []
    entries = []
    with open(path, "r", encoding="utf-8") as fh:
        for lineno, line in enumerate(fh, 1):
            line = line.strip()
            if not line:
                continue
            try:
                entries.append(json.loads(line))
            except ValueError as exc:
                raise ValueError(
                    f"generation log {path} line {lineno} is not valid JSON: {exc}"
                ) from exc
    return entries


def next_generation_id(entries):
    ids = [e["generation"] for e in entries if isinstance(e.get("generation"), int)]
    return (max(ids) + 1) if ids else 1


def record_generation(
    name,
    *,
    artifact,
    source_rev,
    actor,
    action="deploy",
    generation=None,
    result="activated",
    at=None,
    explicit_state=None,
    cloudflare_version=None,
    cloudflare_deployment=None,
):
    entries = read_generations(name, explicit_state)
    if generation is None:
        generation = next_generation_id(entries)
    entry = {
        "generation": generation,
        "target": name,
        "artifact": artifact,
        "source_rev": source_rev,
        "actor": actor,
        "action": action,
        "result": result,
        "at": at or datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    }
    # Cloudflare lifecycle (issue #5767): the provider version and
    # deployment ids are recorded only after a successful publish, so
    # a generation can be re-activated later without rebuilding.
    if cloudflare_version is not None:
        entry["cloudflare_version"] = cloudflare_version
    if cloudflare_deployment is not None:
        entry["cloudflare_deployment"] = cloudflare_deployment
    path = generations_path(name, explicit_state)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(entry, sort_keys=True) + "\n")
    return entry


def current_previous(entries):
    # Fold successful activation events into a history of generations
    # that were current at some point, most recent last.
    history = []
    by_generation = {}
    for entry in entries:
        gen = entry.get("generation")
        if not isinstance(gen, int):
            continue
        by_generation.setdefault(gen, entry)
        if entry.get("result") in ("activated", "rolled-back") or entry.get(
            "action"
        ) in ("deploy", "rollback"):
            history.append(gen)
    if not history:
        return None, None
    current = history[-1]
    previous = None
    for gen in reversed(history[:-1]):
        if gen != current:
            previous = gen
            break
    return by_generation.get(current), (
        by_generation.get(previous) if previous is not None else None
    )


# --- Plans, status, rollback logic (slice 2) ------------------------------


def health_timeout(defn):
    """Seconds activation keeps probing before it fails the deploy."""
    return defn.get("healthTimeoutSeconds", DEFAULT_HEALTH_TIMEOUT_SECONDS)


def _health_step(defn):
    if defn["healthCheck"]:
        return (
            f"run the health check: {defn['healthCheck']} "
            f"(retried for up to {health_timeout(defn)}s)"
        )
    return "no health check defined for this target; skip the health step"


def _flip_back_step(current):
    if current is not None:
        return (
            "on any failure: flip the profile back to generation "
            f"{current['generation']} and restart on it"
        )
    return "on any failure: stop the unit (no previous generation to flip back to)"


def plan_deploy(name, defn, entries, base=None):
    # The ordered activation plan of design §4.1. Returns None when the
    # target class has no activation renderer in this slice.
    # Cloudflare lifecycle targets (issue #5767 slice 4) plan through
    # the Cloudflare path below; every other non-ACTIVATION_CLASSES
    # target still returns None (the slice-2 refusal oracle).
    if defn.get("class") == "cloudflare" and name in CLOUDFLARE_LIFECYCLE_TARGETS:
        return plan_deploy_cloudflare(name, defn, entries)
    if defn["class"] not in ACTIVATION_CLASSES:
        return None
    link = profile_link(name, base)
    current, _previous = current_previous(entries)
    next_id = next_generation_id(entries)
    steps = [
        f"build the service package: {defn['package']}",
        f"install generation {next_id} into the profile {link} "
        f"(nix-env --profile {link} --set {defn['package']})",
    ]
    if defn["class"] == "systemd":
        steps.append(
            f"write the unit {unit_name(name)} pointing at "
            f"{os.path.join(link, 'current', defn['entryPoint'])}"
        )
        steps.append("daemon-reload (systemctl)")
        steps.append(f"restart only {unit_name(name)}")
    elif defn["class"] == "sandbox":
        rendered = render_sandbox_units(name, defn, base)
        enable = enable_unit_for(sorted(rendered))
        steps.append(
            f"render the unit(s) {', '.join(sorted(rendered))} into "
            f"generation {next_id} (ExecStart via {link}/current)"
        )
        steps.append(
            f"record {name} in the sandbox manifest ({sandbox_manifest_path()})"
        )
        steps.append(
            "link the current generation's units into the unit dir "
            "and daemon-reload (the restore step, design §4.4)"
        )
        steps.append(f"enable --now {enable} and verify it is active")
    else:  # launchd stub
        steps.append(
            f"write the launchd plist for {name} pointing at "
            f"{os.path.join(link, 'current', defn['entryPoint'])}"
        )
        steps.append(f"launchctl bootout/bootstrap only the {name} agent")
    steps.append(_health_step(defn))
    steps.append(_flip_back_step(current))
    steps.append(
        f"append generation {next_id} to the generation log for {name} "
        "(generation id, source rev, artifact, timestamp, actor)"
    )
    return steps


def plan_rollback(name, defn, entries, base=None):
    # Returns (steps, None) or (None, error). Rollback is a repoint at
    # the previous distinct generation (design §4.1/§4.6), so it is
    # only defined when the folded history has one. Cloudflare
    # lifecycle targets plan through the Cloudflare path below.
    if defn.get("class") == "cloudflare" and name in CLOUDFLARE_LIFECYCLE_TARGETS:
        return plan_rollback_cloudflare(name, defn, entries)
    if defn["class"] not in ACTIVATION_CLASSES:
        return None, (
            f"rollback for class {defn['class']!r} is not an activation-class "
            "operation in this slice"
        )
    current, previous = current_previous(entries)
    if current is None:
        return None, f"no generations recorded for {name!r}; nothing to roll back"
    if previous is None:
        return None, (
            f"only one generation recorded for {name!r}; rollback needs a "
            "previous generation (two generations are always retained, §4.6)"
        )
    link = profile_link(name, base)
    steps = [
        f"repoint the profile {link} at generation {previous['generation']} "
        f"({previous['artifact']}) — no rebuild, no re-upload",
    ]
    if defn["class"] == "systemd":
        steps.append(f"restart only {unit_name(name)}")
    elif defn["class"] == "sandbox":
        steps.append(
            "re-link the rolled-back generation's rendered units into "
            "the unit dir and daemon-reload (the restore step, §4.4)"
        )
        enable = timer_unit_name(name) if defn.get("timer") else unit_name(name)
        steps.append(f"enable --now {enable} and verify it is active")
    else:
        steps.append(f"launchctl bootout/bootstrap only the {name} agent")
    steps.append(_health_step(defn))
    steps.append(
        f"on any failure: flip the profile back to generation "
        f"{current['generation']} and restart on it"
    )
    steps.append(
        f"append a rollback event (to generation {previous['generation']}) "
        f"to the generation log for {name}"
    )
    return steps, None


def probe_health(url, timeout=2.0):
    # Returns (ok, detail). A probe is a plain GET; 2xx/3xx is healthy.
    try:
        req = urllib.request.Request(url, method="GET")
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return (200 <= resp.status < 400), f"HTTP {resp.status}"
    except urllib.error.HTTPError as exc:
        return False, f"HTTP {exc.code}"
    except Exception as exc:  # unreachable, DNS, timeout — all just failed
        return False, f"{type(exc).__name__}: {exc}"


def format_status(name, defn, entries, health=None):
    current, previous = current_previous(entries)

    def describe(entry):
        if entry is None:
            return "none recorded"
        return (
            f"generation {entry['generation']} (source rev {entry.get('source_rev')}, "
            f"activated {entry.get('at')} by {entry.get('actor')}, "
            f"artifact {entry.get('artifact')})"
        )

    lines = [
        f"{name}: class={defn['class']} targets={','.join(defn['targets'])}",
        f"current: {describe(current)}",
        f"previous: {describe(previous)}",
    ]
    if defn["healthCheck"] is None:
        lines.append("health: not checked (no healthCheck defined)")
    elif health is None:
        lines.append(f"health: {defn['healthCheck']} (not probed)")
    else:
        ok, detail = health
        verdict = "ok" if ok else "FAILED"
        lines.append(f"health: {defn['healthCheck']} -> {verdict} ({detail})")
    return "\n".join(lines)


# --- Cloudflare lifecycle (issue #5767, slice 4: cf-container-hello) ---
#
# Oracle: the slice criterion. Status reports the version/deployment
# currently serving from Cloudflare (the provider), never an inference
# from the local log. Deploy publishes the flake-built package and
# appends the provider version/deployment ids to the generation log
# only after success. Rollback re-activates the previously recorded
# version without rebuilding/re-uploading and refuses before mutation
# when the version lacks the required bindings. Plans are pure:
# dry-run never contacts Cloudflare and never mutates state. Every
# provider effect goes through the cf_* boundary below, which the
# automated tests replace with fakes; no live deploy happens here.


def cf_required_bindings(name):
    """Required Cloudflare binding names for a lifecycle target."""
    if name == "cf-container-hello":
        return REQUIRED_CF_CONTAINER_HELLO_BINDINGS
    return ()


def cf_deploy_app(defn):
    """The deploy-app binary that publishes a cloudflare target."""
    return os.path.join(defn["package"], defn["entryPoint"])


def cf_get_serving(name, defn=None):
    """The version/deployment currently serving from Cloudflare.

    Returns {"version_id": ..., "deployment_id": ...}. Raises
    CloudflareError when the provider cannot be read. Tests replace
    this with a fake; the default drives the target's deploy app
    status verb (see pkgs/cf-container-hello/deploy.mjs).
    """
    if defn is None:
        raise CloudflareError(f"{name}: no definition to locate the deploy app")
    app = cf_deploy_app(defn)
    rc, out, err = run_cmd([app, "status", "--json"])
    if rc != 0:
        raise CloudflareError(f"{name}: status failed (exit {rc}): {err.strip()}")
    try:
        data = json.loads(out)
    except ValueError as exc:
        raise CloudflareError(f"{name}: status returned invalid JSON: {exc}") from exc
    version_id = data.get("version_id")
    deployment_id = data.get("deployment_id")
    if not version_id or not deployment_id:
        raise CloudflareError(
            f"{name}: status response lacks version/deployment ids: {out.strip()}"
        )
    return {"version_id": version_id, "deployment_id": deployment_id}


def cf_publish(name, defn):
    """Publish the flake-built package; return its version/deployment ids.

    Tests replace this with a fake. The default drives the target's
    deploy app (wrangler deploy from the staged flake package).
    """
    app = cf_deploy_app(defn)
    rc, out, err = run_cmd([app, "--json"])
    if rc != 0:
        raise CloudflareError(f"{name}: publish failed (exit {rc}): {err.strip()}")
    try:
        data = json.loads(out)
    except ValueError as exc:
        raise CloudflareError(f"{name}: publish returned invalid JSON: {exc}") from exc
    version_id = data.get("version_id")
    deployment_id = data.get("deployment_id")
    if not version_id or not deployment_id:
        raise CloudflareError(
            f"{name}: publish response lacks version/deployment ids: {out.strip()}"
        )
    return {"version_id": version_id, "deployment_id": deployment_id}


def cf_version_bindings(name, defn, version_id):
    """Binding names the given Cloudflare version carries.

    Tests replace this with a fake. The default asks the deploy app's
    verify-bindings verb.
    """
    app = cf_deploy_app(defn)
    rc, out, err = run_cmd([app, "verify-bindings", "--version", version_id, "--json"])
    if rc != 0:
        raise CloudflareError(
            f"{name}: verify-bindings failed (exit {rc}): {err.strip()}"
        )
    try:
        data = json.loads(out)
    except ValueError as exc:
        raise CloudflareError(
            f"{name}: verify-bindings returned invalid JSON: {exc}"
        ) from exc
    bindings = data.get("bindings")
    if not isinstance(bindings, list):
        raise CloudflareError(
            f"{name}: verify-bindings response lacks a bindings list: {out.strip()}"
        )
    return bindings


def cf_activate_version(name, defn, version_id):
    """Re-activate a recorded version; return the new deployment id.

    No rebuild, no re-upload (wrangler rollback to the version).
    Tests replace this with a fake.
    """
    app = cf_deploy_app(defn)
    rc, out, err = run_cmd([app, "rollback", "--version", version_id, "--json"])
    if rc != 0:
        raise CloudflareError(f"{name}: rollback failed (exit {rc}): {err.strip()}")
    try:
        data = json.loads(out)
    except ValueError as exc:
        raise CloudflareError(
            f"{name}: rollback returned invalid JSON: {exc}"
        ) from exc
    deployment_id = data.get("deployment_id")
    if not deployment_id:
        raise CloudflareError(
            f"{name}: rollback response lacks a deployment id: {out.strip()}"
        )
    return {"version_id": version_id, "deployment_id": deployment_id}


def cf_verify_bindings(name, defn, version_id):
    """Raise CloudflareError when a version lacks required bindings."""
    required = cf_required_bindings(name)
    if not required:
        return
    present = set(cf_version_bindings(name, defn, version_id))
    missing = [b for b in required if b not in present]
    if missing:
        raise CloudflareError(
            f"{name}: version {version_id} lacks required bindings: "
            f"{', '.join(missing)}"
        )


def plan_deploy_cloudflare(name, defn, entries):
    """Pure deploy plan for a Cloudflare lifecycle target (no I/O)."""
    next_id = next_generation_id(entries)
    current, _previous = current_previous(entries)
    bindings = ", ".join(cf_required_bindings(name)) or "none required"
    steps = [
        f"build the service package: {defn['package']} "
        "(flake-built, lockfile-pinned; no deploy-time npm install)",
        f"publish {name} to Cloudflare via {defn['entryPoint']} "
        "(wrangler deploy from the staged flake package)",
        f"verify the published version carries required bindings: {bindings}",
    ]
    if current is not None:
        steps.append(
            "on any failure: keep serving the current Cloudflare version "
            f"(generation {current['generation']}); record a deploy_failed event"
        )
    else:
        steps.append(
            "on any failure: record a deploy_failed event; no version to flip back to"
        )
    steps.append(
        f"append generation {next_id} to the generation log for {name} "
        "with the Cloudflare version id and deployment id (only after success)"
    )
    return steps


def plan_rollback_cloudflare(name, defn, entries):
    """Pure rollback plan; never rebuilds. Returns (steps, None)/(None, err)."""
    current, previous = current_previous(entries)
    if current is None:
        return None, f"no generations recorded for {name!r}; nothing to roll back"
    if previous is None:
        return None, (
            f"only one generation recorded for {name!r}; rollback needs a "
            "previous generation (two generations are always retained, §4.6)"
        )
    version_id = previous.get("cloudflare_version")
    if not version_id:
        return None, (
            f"previous generation {previous['generation']} for {name!r} lacks a "
            "recorded Cloudflare version id; cannot roll back without rebuilding"
        )
    bindings = ", ".join(cf_required_bindings(name)) or "none required"
    steps = [
        f"verify version {version_id} (generation {previous['generation']}) "
        f"carries required bindings: {bindings} — refuse before mutation if absent",
        f"re-activate Cloudflare version {version_id} for {name} "
        "(wrangler rollback to the recorded version) — no rebuild, no re-upload",
        f"append a rollback event (to generation {previous['generation']}) "
        f"to the generation log for {name} with the new deployment id",
    ]
    return steps, None


def format_cloudflare_status(
    name, defn, entries, serving=None, serving_error=None, health=None
):
    """Status with the provider's serving version, not a log inference."""
    current, previous = current_previous(entries)

    def describe(entry):
        if entry is None:
            return "none recorded"
        base = (
            f"generation {entry['generation']} (source rev {entry.get('source_rev')}, "
            f"activated {entry.get('at')} by {entry.get('actor')}, "
            f"artifact {entry.get('artifact')})"
        )
        version_id = entry.get("cloudflare_version")
        deployment_id = entry.get("cloudflare_deployment")
        if version_id or deployment_id:
            base += (
                f" [cloudflare version {version_id or '?'} "
                f"deployment {deployment_id or '?'}]"
            )
        return base

    lines = [
        f"{name}: class={defn['class']} targets={','.join(defn['targets'])}",
        f"current: {describe(current)}",
        f"previous: {describe(previous)}",
    ]
    if serving is not None:
        lines.append(
            f"serving: cloudflare version {serving.get('version_id')} "
            f"deployment {serving.get('deployment_id')}"
        )
    elif serving_error is not None:
        lines.append(f"serving: Cloudflare query failed: {serving_error}")
    else:
        lines.append("serving: unknown (Cloudflare was not queried)")
    if defn.get("healthCheck") is None:
        lines.append("health: not checked (no healthCheck defined)")
    elif health is None:
        lines.append("health: not probed (cloudflare status probes the provider)")
    else:
        ok, detail = health
        verdict = "ok" if ok else "FAILED"
        lines.append(f"health: {defn['healthCheck']} -> {verdict} ({detail})")
    return "\n".join(lines)


def execute_deploy_cloudflare(name, defn, entries, *, state=None, actor=None):
    """Publish the package, verify bindings, record ids. Returns (code, lines)."""
    actor = actor or getpass.getuser()
    generation = next_generation_id(entries)
    artifact = defn["package"]
    refusal = release_claim_refusal(name, defn)
    if refusal is not None:
        record_generation(
            name,
            artifact=artifact,
            source_rev=_source_rev(),
            actor=actor,
            action="deploy_failed",
            generation=generation,
            result=f"refused: {refusal}",
            explicit_state=state,
        )
        return 1, [f"deploy {name}: REFUSED: {refusal}"]
    try:
        published = cf_publish(name, defn)
        cf_verify_bindings(name, defn, published["version_id"])
    except CloudflareError as exc:
        record_generation(
            name,
            artifact=artifact,
            source_rev=_source_rev(),
            actor=actor,
            action="deploy_failed",
            generation=generation,
            result=f"failed: {exc}",
            explicit_state=state,
        )
        return 1, [f"deploy {name}: FAILED: {exc}"]
    entry = record_generation(
        name,
        artifact=artifact,
        source_rev=_source_rev(),
        actor=actor,
        action="deploy",
        generation=generation,
        result="activated",
        explicit_state=state,
        cloudflare_version=published["version_id"],
        cloudflare_deployment=published["deployment_id"],
    )
    return 0, [
        f"deploy {name}: generation {entry['generation']} activated "
        f"(cloudflare version {published['version_id']} "
        f"deployment {published['deployment_id']})"
    ]


def execute_rollback_cloudflare(name, defn, entries, *, state=None, actor=None):
    """Re-activate the previous version after verifying bindings."""
    actor = actor or getpass.getuser()
    current, previous = current_previous(entries)
    if current is None or previous is None:
        _steps, error = plan_rollback_cloudflare(name, defn, entries)
        return 1, [f"deploy: {error}"]
    version_id = previous.get("cloudflare_version")
    if not version_id:
        return 1, [
            f"deploy: previous generation {previous['generation']} for {name!r} "
            "lacks a recorded Cloudflare version id; "
            "cannot roll back without rebuilding"
        ]
    try:
        cf_verify_bindings(name, defn, version_id)
    except CloudflareError as exc:
        record_generation(
            name,
            artifact=previous["artifact"],
            source_rev=_source_rev(),
            actor=actor,
            action="rollback_failed",
            generation=previous["generation"],
            result=f"failed: {exc}",
            explicit_state=state,
        )
        return 1, [f"deploy {name} rollback: REFUSED: {exc}"]
    try:
        activated = cf_activate_version(name, defn, version_id)
    except CloudflareError as exc:
        record_generation(
            name,
            artifact=previous["artifact"],
            source_rev=_source_rev(),
            actor=actor,
            action="rollback_failed",
            generation=previous["generation"],
            result=f"failed: {exc}",
            explicit_state=state,
        )
        return 1, [f"deploy {name} rollback: FAILED: {exc}"]
    record_generation(
        name,
        artifact=previous["artifact"],
        source_rev=_source_rev(),
        actor=actor,
        action="rollback",
        generation=previous["generation"],
        result="rolled-back",
        explicit_state=state,
        cloudflare_version=version_id,
        cloudflare_deployment=activated["deployment_id"],
    )
    return 0, [
        f"deploy {name}: rolled back to generation {previous['generation']} "
        f"(cloudflare version {version_id} "
        f"deployment {activated['deployment_id']}) — no rebuild, no re-upload"
    ]


# --- Sandbox manifest + restore (slice 4, issue #5770) ------------------
#
# Oracle: design §4.4. The manifest is the list of currently-activated
# sandbox targets, a file in the persistent home (the state dir):
# {name: {"profile": <profile link path>, "units": [<unit file>, ...]}}.
# The restorer — `deploy restore` here, restore.py standalone at boot —
# reads it and links each target's current generation's rendered units
# into the unit dir, daemon-reloads, and enables each target's enable
# unit. The manifest records the profile LINK (stable across
# generations), never a generation directory: a rollback repoints the
# link and the same manifest restores the rolled-back generation.


def sandbox_manifest_path(explicit_state=None):
    return os.path.join(state_dir(explicit_state), "sandbox-manifest.json")


def read_sandbox_manifest(explicit_state=None):
    path = sandbox_manifest_path(explicit_state)
    if not os.path.exists(path):
        return {}
    with open(path, "r", encoding="utf-8") as fh:
        data = json.load(fh, object_pairs_hook=_no_duplicate_keys)
    if not isinstance(data, dict) or not isinstance(data.get("targets", {}), dict):
        raise ValueError(f"sandbox manifest {path} is not a targets object")
    return data.get("targets", {})


def record_sandbox_target(name, unit_files, base=None, explicit_state=None):
    """Add/refresh one target in the manifest (atomic replace)."""
    targets = read_sandbox_manifest(explicit_state)
    targets[name] = {
        "profile": profile_link(name, base),
        "units": sorted(unit_files),
    }
    path = sandbox_manifest_path(explicit_state)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = f"{path}.tmp-{os.getpid()}"
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump({"targets": targets}, fh, indent=2, sort_keys=True)
        fh.write("\n")
    os.replace(tmp, path)
    return targets[name]


def _link_generation_units(profile, unit_files, units_dir):
    """Link one target's current generation's rendered units into the
    unit dir. Returns the linked filenames. Raises ActivationError
    when the generation's rendered units are not on disk."""
    gen_dir = os.path.realpath(profile)
    src_dir = os.path.join(gen_dir, "units")
    if not os.path.isdir(src_dir):
        raise ActivationError(
            f"rendered units not found at {src_dir} (profile {profile} "
            "does not resolve to a deployed generation)"
        )
    linked = []
    os.makedirs(units_dir, exist_ok=True)
    for filename in unit_files:
        src = os.path.join(src_dir, filename)
        if not os.path.exists(src):
            raise ActivationError(f"rendered unit {src} is missing")
        dest = os.path.join(units_dir, filename)
        tmp = f"{dest}.tmp-{os.getpid()}"
        if os.path.islink(tmp) or os.path.exists(tmp):
            os.unlink(tmp)
        os.symlink(src, tmp)
        os.replace(tmp, dest)
        linked.append(filename)
    return linked


def _restore_target(name, profile, unit_files, defn=None, units_dir=None):
    """The restore step for one target: link its current generation's
    units, daemon-reload, enable --now its enable unit, verify the
    unit is active (and probe the healthCheck URL when a definition
    is at hand). Sandbox units are system units — there is no user
    bus — so the scope is always system. Raises ActivationError."""
    units_dir = units_dir if units_dir is not None else unit_dir(scope="system")
    linked = _link_generation_units(profile, unit_files, units_dir)
    _run_step(systemctl_argv("system", "daemon-reload"), "daemon-reload")
    enable = enable_unit_for(linked)
    _run_step(
        systemctl_argv("system", "enable", "--now", enable),
        f"enable --now of {enable}",
    )
    rc, _out, err = run_cmd(systemctl_argv("system", "is-active", "--quiet", enable))
    if rc != 0:
        raise ActivationError(
            f"{enable} is not active after enable --now: {err.strip()}"
        )
    if defn is not None and defn["healthCheck"] is not None:
        ok, detail = wait_health(defn["healthCheck"], health_timeout(defn))
        if not ok:
            raise ActivationError(f"health check failed: {detail}")
    return linked


def restore_sandbox(base=None, state=None, units_dir=None):
    """`deploy restore`: re-run the restore step for every target in
    the sandbox manifest. Returns (exit_code, lines). An empty
    manifest is a successful no-op (a sandbox with no sandbox targets
    deployed yet); a target whose generation is missing fails its
    own line without stopping the others."""
    try:
        targets = read_sandbox_manifest(state)
    except (OSError, ValueError) as exc:
        return 1, [f"deploy restore: cannot read the sandbox manifest: {exc}"]
    if not targets:
        return 0, ["deploy restore: no sandbox targets recorded; nothing to do"]
    units_dir = units_dir if units_dir is not None else unit_dir(scope="system")
    lines = []
    code = 0
    for name in sorted(targets):
        entry = targets[name]
        try:
            linked = _restore_target(
                name, entry["profile"], entry["units"], units_dir=units_dir
            )
        except (ActivationError, KeyError) as exc:
            code = 1
            lines.append(f"deploy restore: {name}: FAILED: {exc}")
            continue
        enable = enable_unit_for(linked)
        lines.append(
            f"deploy restore: {name}: linked {', '.join(linked)}; {enable} active"
        )
    return code, lines


# --- Real activation (slice 3) -------------------------------------------
#
# Oracle: design §4.1 plan order, as printed by plan_deploy /
# plan_rollback: install the generation into the profile, write the
# unit, daemon-reload, restart only that unit, run the health check,
# flip back on any failure. The profile is deploy-managed: the
# profile link <base>/<name> points at a generation directory
# <base>/.generations/<name>/<id>/ whose `current` symlink names the
# artifact, so the rendered unit's <link>/current/<entryPoint> path
# resolves and a flip is one atomic symlink repoint — the same shape
# nix-env gives a profile, kept deploy-owned because the unit path
# goes through `current`. Store-path artifacts are additionally
# registered with nix-env --set on a parallel profile
# (<base>/<name>.nix-env-profile) so the generation also exists in
# nix's own profile bookkeeping. Every external effect goes through
# run_cmd / write_text / probe_health, which the slice-3 tests
# replace with fakes: the order asserted there is the plan order.


class ActivationError(Exception):
    """A step of the activation plan failed; the caller flips back."""


def run_cmd(argv):
    """Run one external command; return (returncode, stdout, stderr).
    A missing binary is a failed step (127), never a crash."""
    try:
        proc = subprocess.run(argv, capture_output=True, text=True)
    except OSError as exc:
        return 127, "", f"{argv[0]}: {exc}"
    return proc.returncode, proc.stdout, proc.stderr


def write_text(path, content):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(content)


def systemctl_argv(scope, *args):
    if scope == "user":
        return ["systemctl", "--user", *args]
    return ["systemctl", *args]


def unit_dir(explicit=None, scope="system"):
    if explicit:
        return explicit
    env = os.environ.get("DEPLOY_UNIT_DIR")
    if env:
        return env
    if scope == "user":
        return os.path.expanduser("~/.config/systemd/user")
    return "/etc/systemd/system"


def _run_step(argv, what):
    rc, _out, err = run_cmd(argv)
    if rc != 0:
        raise ActivationError(f"{what} failed (exit {rc}): {err.strip()}")


def _target_drv(name):
    """The .drv path that builds `name`'s package, from the map the
    flake package bakes in as $DEPLOY_TARGET_DRVS (code#12738), or None
    when no map is configured or it does not name `name`."""
    path = os.environ.get("DEPLOY_TARGET_DRVS")
    if not path:
        return None
    try:
        with open(path, encoding="utf-8") as fh:
            drvs = json.load(fh)
    except (OSError, ValueError):
        return None
    drv = drvs.get(name) if isinstance(drvs, dict) else None
    if isinstance(drv, str) and drv.startswith("/nix/store/") and drv.endswith(".drv"):
        return drv
    return None


def ensure_artifact(name, artifact):
    """Build `artifact` from its recorded derivation when it is not in
    the store yet. The registry names store paths without depending on
    them (code#12738), so a target's package is realized only when that
    target is deployed. A path already present, a non-store artifact, or
    a target with no recorded derivation is left as it is."""
    if not artifact.startswith("/nix/store/") or os.path.exists(artifact):
        return
    drv = _target_drv(name)
    if drv is None or not _which("nix-store"):
        return
    _run_step(["nix-store", "--realise", drv], f"build the {name} package")
    if not os.path.exists(artifact):
        raise ActivationError(
            f"build the {name} package: {drv} did not produce {artifact}"
        )


def install_generation(name, artifact, generation, base=None):
    """Point the profile link for `name` at a new generation dir
    whose `current` symlink names `artifact`. Returns the link path."""
    ensure_artifact(name, artifact)
    base_dir = profile_base(base)
    gen_dir = os.path.join(base_dir, ".generations", name, str(generation))
    os.makedirs(gen_dir, exist_ok=True)
    current = os.path.join(gen_dir, "current")
    if os.path.islink(current) or os.path.exists(current):
        os.unlink(current)
    os.symlink(artifact, current)
    if artifact.startswith("/nix/store/") and _which("nix-env"):
        _run_step(
            [
                "nix-env",
                "--profile",
                os.path.join(base_dir, f"{name}.nix-env-profile"),
                "--set",
                artifact,
            ],
            "nix-env profile install",
        )
    link = profile_link(name, base)
    tmp = f"{link}.tmp-{os.getpid()}"
    if os.path.islink(tmp) or os.path.exists(tmp):
        os.unlink(tmp)
    os.symlink(gen_dir, tmp)
    os.replace(tmp, link)
    return link


def _which(tool):
    for entry in os.environ.get("PATH", "").split(os.pathsep):
        if os.path.exists(os.path.join(entry, tool)):
            return True
    return False


def repoint_profile(name, artifact, generation, base=None):
    """Flip the profile link back/forward to an existing artifact
    (rollback and flip-back share this: a repoint, never a rebuild)."""
    return install_generation(name, artifact, generation, base)


# The clock and sleep wait_health uses; tests replace them so a long
# health window costs no real time.
_monotonic = time.monotonic
_sleep = time.sleep


def wait_health(
    url,
    timeout_seconds=DEFAULT_HEALTH_TIMEOUT_SECONDS,
    interval=HEALTH_PROBE_INTERVAL_SECONDS,
):
    """Probe the health check until it passes or the window closes.

    Every failure is retried until the deadline: an endpoint answering
    not-ready (a 503 while a broker's first refresh runs) and an
    endpoint not listening yet are both a service still starting. A
    service that is truly broken fails once the window closes (#11170).
    """
    deadline = _monotonic() + timeout_seconds
    while True:
        ok, detail = probe_health(url)
        if ok:
            return True, detail
        remaining = deadline - _monotonic()
        if remaining <= 0:
            return False, f"{detail} (still failing after {timeout_seconds}s)"
        _sleep(min(interval, remaining))


def _activate(
    name, defn, *, artifact, generation, base, state, scope, units, action, actor
):
    """Shared tail of deploy/rollback: unit write, daemon-reload,
    restart, health check. Raises ActivationError on any failure."""
    if defn["class"] != "systemd":
        raise ActivationError(
            f"real activation for class {defn['class']!r} is not implemented "
            "in this slice (systemd only; launchd remains a rendering stub)"
        )
    write_text(os.path.join(units, unit_name(name)), render_unit(name, defn, base))
    _run_step(systemctl_argv(scope, "daemon-reload"), "daemon-reload")
    _run_step(
        systemctl_argv(scope, "restart", unit_name(name)),
        f"restart of {unit_name(name)}",
    )
    if defn["healthCheck"] is not None:
        ok, detail = wait_health(defn["healthCheck"], health_timeout(defn))
        if not ok:
            raise ActivationError(f"health check failed: {detail}")


def _flip_back(name, defn, *, previous, base, scope, units):
    """Restore the previous generation after a failed activation and
    restart on it; with no previous generation, stop the unit. Never
    raises: a flip-back failure is an outcome to record, not a crash
    that loses the failure entry (found by the slice-3 laptop E2E:
    a broken unit restart-loops into systemd's start limit, so the
    flip-back restart needs a reset-failed first)."""
    if previous is None:
        run_cmd(systemctl_argv(scope, "stop", unit_name(name)))
        link = profile_link(name, base)
        if os.path.islink(link):
            os.unlink(link)
        return "stopped (no previous generation)"
    try:
        repoint_profile(name, previous["artifact"], previous["generation"], base)
        run_cmd(systemctl_argv(scope, "reset-failed", unit_name(name)))
        _run_step(
            systemctl_argv(scope, "restart", unit_name(name)),
            f"restart of {unit_name(name)} on flip-back",
        )
    except ActivationError as exc:
        return f"flip-back to generation {previous['generation']} INCOMPLETE: {exc}"
    return f"flipped back to generation {previous['generation']}"


def execute_deploy(
    name,
    defn,
    entries,
    *,
    base=None,
    state=None,
    scope="system",
    units=None,
    actor=None,
):
    """Real (non-dry-run) deploy, in plan_deploy's order. Returns
    (exit_code, lines). On failure the profile is flipped back to the
    previously current generation and a non-folding failure entry is
    appended to the generation log (action deploy_failed, so
    current_previous never mistakes it for an activation)."""
    actor = actor or getpass.getuser()
    units = units if units is not None else unit_dir(scope=scope)
    current, _prev = current_previous(entries)
    generation = next_generation_id(entries)
    artifact = defn["package"]
    refusal = release_claim_refusal(name, defn)
    if refusal is not None:
        record_generation(
            name,
            artifact=artifact,
            source_rev=_source_rev(),
            actor=actor,
            action="deploy_failed",
            generation=generation,
            result=f"refused: {refusal}",
            explicit_state=state,
        )
        return 1, [f"deploy {name}: REFUSED: {refusal}"]
    lines = []
    try:
        install_generation(name, artifact, generation, base)
        lines.append(
            f"installed generation {generation} into the profile "
            f"{profile_link(name, base)}"
        )
        _activate(
            name,
            defn,
            artifact=artifact,
            generation=generation,
            base=base,
            state=state,
            scope=scope,
            units=units,
            action="deploy",
            actor=actor,
        )
    except ActivationError as exc:
        outcome = _flip_back(
            name, defn, previous=current, base=base, scope=scope, units=units
        )
        record_generation(
            name,
            artifact=artifact,
            source_rev=_source_rev(),
            actor=actor,
            action="deploy_failed",
            generation=generation,
            result=f"failed: {exc}; {outcome}",
            explicit_state=state,
        )
        lines.append(f"deploy {name}: FAILED: {exc}; {outcome}")
        return 1, lines
    entry = record_generation(
        name,
        artifact=artifact,
        source_rev=_source_rev(),
        actor=actor,
        action="deploy",
        generation=generation,
        result="activated",
        explicit_state=state,
    )
    lines.append(
        f"deploy {name}: generation {entry['generation']} activated "
        f"(artifact {artifact})"
    )
    return 0, lines


def execute_rollback(
    name,
    defn,
    entries,
    *,
    base=None,
    state=None,
    scope="system",
    units=None,
    actor=None,
):
    """Real rollback: repoint at the previous distinct generation and
    restart on it (design §4.1/§4.6 — never a rebuild). On health
    failure, flip back to the generation that was current."""
    actor = actor or getpass.getuser()
    units = units if units is not None else unit_dir(scope=scope)
    current, previous = current_previous(entries)
    if current is None or previous is None:
        _steps, error = plan_rollback(name, defn, entries, base)
        return 1, [f"deploy: {error}"]
    lines = []
    try:
        repoint_profile(name, previous["artifact"], previous["generation"], base)
        _activate(
            name,
            defn,
            artifact=previous["artifact"],
            generation=previous["generation"],
            base=base,
            state=state,
            scope=scope,
            units=units,
            action="rollback",
            actor=actor,
        )
    except ActivationError as exc:
        outcome = _flip_back(
            name, defn, previous=current, base=base, scope=scope, units=units
        )
        record_generation(
            name,
            artifact=previous["artifact"],
            source_rev=_source_rev(),
            actor=actor,
            action="rollback_failed",
            generation=previous["generation"],
            result=f"failed: {exc}; {outcome}",
            explicit_state=state,
        )
        lines.append(f"deploy {name} rollback: FAILED: {exc}; {outcome}")
        return 1, lines
    record_generation(
        name,
        artifact=previous["artifact"],
        source_rev=_source_rev(),
        actor=actor,
        action="rollback",
        generation=previous["generation"],
        result="rolled-back",
        explicit_state=state,
    )
    lines.append(
        f"deploy {name}: rolled back to generation "
        f"{previous['generation']} (artifact {previous['artifact']})"
    )
    return 0, lines


# --- Sandbox activation (slice 4, issue #5770) ---------------------------
#
# Oracle: design §4.4 in plan_deploy's order: install the generation
# into the (home) profile, render the units into the generation,
# record the target in the sandbox manifest, then the restore step
# (link, daemon-reload, enable --now, verify active). Flip-back is
# the systemd class's discipline with sandbox mechanics: repoint at
# the previous generation and re-run the restore step on it; with no
# previous generation, disable the enable unit and unlink the units.


def _generation_unit_files(name, defn, generation, base, state):
    """The unit files a generation rendered: read from its own
    generation directory when it still exists (the authoritative
    record of what that generation linked), else the manifest's
    list for the target, else a fresh render of the definition."""
    gen_units = generation_units_dir(name, generation, base)
    if os.path.isdir(gen_units):
        files = sorted(
            f for f in os.listdir(gen_units) if f.endswith((".service", ".timer"))
        )
        if files:
            return files
    manifest = read_sandbox_manifest(state)
    if name in manifest:
        return manifest[name]["units"]
    return sorted(render_sandbox_units(name, defn, base))


def _flip_back_sandbox(name, defn, *, previous, base, state, units_dir):
    """Restore the previous generation after a failed sandbox
    activation; with no previous generation, disable the target's
    units and unlink them. Never raises (same contract as _flip_back)."""
    if previous is None:
        manifest = read_sandbox_manifest(state)
        unit_files = manifest.get(name, {}).get("units", [unit_name(name)])
        run_cmd(
            systemctl_argv("system", "disable", "--now", enable_unit_for(unit_files))
        )
        for filename in unit_files:
            path = os.path.join(units_dir, filename)
            if os.path.islink(path) or os.path.exists(path):
                os.unlink(path)
        run_cmd(systemctl_argv("system", "daemon-reload"))
        link = profile_link(name, base)
        if os.path.islink(link):
            os.unlink(link)
        return "units removed (no previous generation)"
    try:
        repoint_profile(name, previous["artifact"], previous["generation"], base)
        unit_files = _generation_unit_files(
            name, defn, previous["generation"], base, state
        )
        _restore_target(
            name,
            profile_link(name, base),
            unit_files,
            defn,
            units_dir=units_dir,
        )
    except (ActivationError, ValueError) as exc:
        return f"flip-back to generation {previous['generation']} INCOMPLETE: {exc}"
    return f"flipped back to generation {previous['generation']}"


def execute_deploy_sandbox(
    name, defn, entries, *, base=None, state=None, units_dir=None, actor=None
):
    """Real sandbox deploy, in plan_deploy's order. Returns
    (exit_code, lines); failure semantics mirror execute_deploy."""
    actor = actor or getpass.getuser()
    units_dir = units_dir if units_dir is not None else unit_dir(scope="system")
    current, _prev = current_previous(entries)
    generation = next_generation_id(entries)
    artifact = defn["package"]
    refusal = release_claim_refusal(name, defn)
    if refusal is not None:
        record_generation(
            name,
            artifact=artifact,
            source_rev=_source_rev(),
            actor=actor,
            action="deploy_failed",
            generation=generation,
            result=f"refused: {refusal}",
            explicit_state=state,
        )
        return 1, [f"deploy {name}: REFUSED: {refusal}"]
    lines = []
    try:
        link = install_generation(name, artifact, generation, base)
        lines.append(f"installed generation {generation} into the profile {link}")
        unit_files = write_sandbox_units(name, defn, generation, base)
        record_sandbox_target(name, unit_files, base=base, explicit_state=state)
        lines.append(
            f"rendered {', '.join(unit_files)} into generation "
            f"{generation} and recorded {name} in the sandbox manifest"
        )
        _restore_target(name, link, unit_files, defn, units_dir=units_dir)
        lines.append(
            f"linked the units into {units_dir}; {enable_unit_for(unit_files)} active"
        )
    except (ActivationError, ValueError) as exc:
        outcome = _flip_back_sandbox(
            name, defn, previous=current, base=base, state=state, units_dir=units_dir
        )
        record_generation(
            name,
            artifact=artifact,
            source_rev=_source_rev(),
            actor=actor,
            action="deploy_failed",
            generation=generation,
            result=f"failed: {exc}; {outcome}",
            explicit_state=state,
        )
        lines.append(f"deploy {name}: FAILED: {exc}; {outcome}")
        return 1, lines
    entry = record_generation(
        name,
        artifact=artifact,
        source_rev=_source_rev(),
        actor=actor,
        action="deploy",
        generation=generation,
        result="activated",
        explicit_state=state,
    )
    lines.append(
        f"deploy {name}: generation {entry['generation']} activated "
        f"(artifact {artifact})"
    )
    return 0, lines


def execute_rollback_sandbox(
    name, defn, entries, *, base=None, state=None, units_dir=None, actor=None
):
    """Real sandbox rollback: repoint at the previous distinct
    generation and re-run the restore step on it (never a rebuild)."""
    actor = actor or getpass.getuser()
    units_dir = units_dir if units_dir is not None else unit_dir(scope="system")
    current, previous = current_previous(entries)
    if current is None or previous is None:
        _steps, error = plan_rollback(name, defn, entries, base)
        return 1, [f"deploy: {error}"]
    lines = []
    try:
        repoint_profile(name, previous["artifact"], previous["generation"], base)
        unit_files = _generation_unit_files(
            name, defn, previous["generation"], base, state
        )
        _restore_target(
            name,
            profile_link(name, base),
            unit_files,
            defn,
            units_dir=units_dir,
        )
    except (ActivationError, ValueError) as exc:
        outcome = _flip_back_sandbox(
            name, defn, previous=current, base=base, state=state, units_dir=units_dir
        )
        record_generation(
            name,
            artifact=previous["artifact"],
            source_rev=_source_rev(),
            actor=actor,
            action="rollback_failed",
            generation=previous["generation"],
            result=f"failed: {exc}; {outcome}",
            explicit_state=state,
        )
        lines.append(f"deploy {name} rollback: FAILED: {exc}; {outcome}")
        return 1, lines
    record_generation(
        name,
        artifact=previous["artifact"],
        source_rev=_source_rev(),
        actor=actor,
        action="rollback",
        generation=previous["generation"],
        result="rolled-back",
        explicit_state=state,
    )
    lines.append(
        f"deploy {name}: rolled back to generation "
        f"{previous['generation']} (artifact {previous['artifact']})"
    )
    return 0, lines


def _source_rev():
    return os.environ.get("DEPLOY_SOURCE_REV") or "working-tree"


# --- Releases (issue #13306) -------------------------------------------
#
# Oracle: doc/2026-11-23T13-09-25Z_design_release-workflow-per-app-tags
# (the release-workflow design record, #11177), implementing the
# ratified #5790 tag scheme. The deploy tool is the estate's only
# release actor: `deploy release` cuts the tags, the annotated tag
# message is the attestation of record, the release ledger in the
# state dir is its cache, and the deploy path refuses a release claim
# with no attested tag behind it (design §6). Every external effect
# goes through run_cmd (git and gh alike), so tests drive the whole
# slice with fakes and fixture repositories.


class ReleaseError(Exception):
    """A release precondition failed; the release is refused."""


# The deploy tool's own version, as packaged (pkgs/deploy/default.nix
# names the package deploy-<version>); carried in the attestation
# block's `tool:` line.
DEPLOY_TOOL_VERSION = "0.1.0"

ATTESTATION_HEADER = "release-attestation:v1"

# The main-gate watched set and its trigger classes, mirroring
# .github/scripts/release-gate.sh (which mirrors main-broken.yml's
# workflow_run.workflows list). Renaming a watched workflow means
# updating all three places together — the same instruction the gate
# script carries. Since 2026-11-16 every watched workflow is non-push
# (nightly main CI), so the non-push rule below is the live one; the
# push rules stay for any workflow that regains a push trigger.
WATCHED_WORKFLOWS = [
    "nix ci",
    "id",
    "mcpx",
    "mcpx conformance",
    "mcpx nix build",
    "go modules build",
    "identity build",
    "map build",
    "id bench",
]
NON_PUSH_WORKFLOWS = {
    "nix ci",
    "id",
    "mcpx",
    "mcpx conformance",
    "mcpx nix build",
    "go modules build",
    "identity build",
    "map build",
    "id bench",
}
PATH_FILTERED_WORKFLOWS = set()

VERSION_RE = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
REPO_TAG_RE = re.compile(r"^repo/v([0-9]{8})\.([0-9]+)\.([0-9]+)$")
FULL_SHA_RE = re.compile(r"^[0-9a-f]{40}$")


def parse_version(text):
    """'X.Y.Z' -> (X, Y, Z), or None when not exactly that shape."""
    match = VERSION_RE.fullmatch(text or "")
    if match is None:
        return None
    return tuple(int(part) for part in match.groups())


def format_version(version):
    return ".".join(str(part) for part in version)


def _git(repo, *args):
    return run_cmd(["git", "-C", repo, *args])


def _git_checked(repo, *args, what):
    rc, out, err = _git(repo, *args)
    if rc != 0:
        raise ReleaseError(f"{what} failed (exit {rc}): {err.strip()}")
    return out


def is_ancestor_or_equal(repo, maybe_ancestor, sha):
    """True when maybe_ancestor is sha or provably an ancestor of it
    in the local checkout. Unprovable (object missing) is False:
    fail closed, exactly like the gate script's is_ancestor_or_equal."""
    if maybe_ancestor == sha:
        return True
    rc, _o, _e = _git(repo, "cat-file", "-e", f"{maybe_ancestor}^{{commit}}")
    if rc != 0:
        return False
    rc, _o, _e = _git(repo, "cat-file", "-e", f"{sha}^{{commit}}")
    if rc != 0:
        return False
    rc, _o, _e = _git(repo, "merge-base", "--is-ancestor", maybe_ancestor, sha)
    return rc == 0


def gh_repo_slug(repo):
    """The owner/repo the gate is judged against: $DEPLOY_GH_REPO or
    $GH_REPO when set, else the checkout's origin remote."""
    for var in ("DEPLOY_GH_REPO", "GH_REPO"):
        slug = os.environ.get(var)
        if slug:
            return slug
    rc, out, _err = _git(repo, "remote", "get-url", "origin")
    if rc != 0:
        return None
    match = re.search(r"github\.com[:/]([^/]+)/([^/]+?)(?:\.git)?$", out.strip())
    if match is None:
        return None
    return f"{match.group(1)}/{match.group(2)}"


def _gh_api(slug, endpoint):
    rc, out, err = run_cmd(["gh", "api", f"repos/{slug}/{endpoint}"])
    if rc != 0:
        raise ReleaseError(
            f"gh api repos/{slug}/{endpoint} failed (exit {rc}): {err.strip()}"
        )
    try:
        return json.loads(out)
    except ValueError as exc:
        raise ReleaseError(
            f"gh api repos/{slug}/{endpoint} returned invalid JSON: {exc}"
        ) from exc


def remote_tags(repo):
    """{tag name: commit sha} from the remote's own advertisement —
    the authority on which tags exist and where they point (annotated
    tags peel to their commit)."""
    out = _git_checked(repo, "ls-remote", "--tags", "origin", what="git ls-remote")
    tags = {}
    peeled = {}
    for line in out.splitlines():
        sha, _, ref = line.partition("\t")
        if not ref.startswith("refs/tags/"):
            continue
        name = ref[len("refs/tags/") :]
        if name.endswith("^{}"):
            peeled[name[:-3]] = sha
        else:
            tags[name] = sha
    tags.update(peeled)
    return tags


def plan_release_version(existing, requested, bump):
    """The version a release may cut. `existing` is the app's tagged
    versions; `requested` a parsed (X, Y, Z) or None; `bump` is
    'minor' or 'patch' and decides when no version is requested.
    Returns the version tuple; raises ReleaseError on any discipline
    violation (design §2 precondition 3, #5790: first tag 0.1.0, 0.x
    only, next minor or next patch only)."""
    if requested is not None and requested[0] != 0:
        raise ReleaseError(
            f"version {format_version(requested)} leaves 0.x; major bumps "
            "need a separate 1.0 decision (#5790)"
        )
    if not existing:
        if requested is not None and requested != (0, 1, 0):
            raise ReleaseError(
                f"the first tag for an app is 0.1.0, not {format_version(requested)}"
            )
        return (0, 1, 0)
    latest = max(existing)
    next_minor = (0, latest[1] + 1, 0)
    next_patch = (0, latest[1], latest[2] + 1)
    if requested is None:
        return next_minor if bump == "minor" else next_patch
    if requested not in (next_minor, next_patch):
        raise ReleaseError(
            f"version {format_version(requested)} is not the next minor "
            f"({format_version(next_minor)}) or next patch "
            f"({format_version(next_patch)}) after {format_version(latest)}"
        )
    return requested


def next_repo_tag(existing_names, today, feature):
    """The repo tag for a release point (design §5, #5790 rule 4):
    repo/v<YYYYMMDD>.<minor>.<patch>. The (minor, patch) counter
    continues from the latest earlier point; on a new date it resets
    to a 0.0 base and the point's own increment applies (a point with
    any app feature increments minor and resets patch; a fix-only
    point increments patch)."""
    points = []
    for name in existing_names:
        match = REPO_TAG_RE.fullmatch(name)
        if match is not None:
            points.append((match.group(1), int(match.group(2)), int(match.group(3))))
    minor = patch = 0
    if points:
        last_date, last_minor, last_patch = max(points)
        if last_date >= today:
            minor, patch = last_minor, last_patch
    if feature:
        minor, patch = minor + 1, 0
    else:
        patch += 1
    return f"repo/v{today}.{minor}.{patch}"


def _runs_by_recency(runs):
    return sorted(runs, key=lambda r: r.get("created_at") or "", reverse=True)


def _judge_runs(runs, workflow, commit, repo):
    """Pick the run that judges `commit` for a non-push workflow and
    return its attestation record. Design §3 part 1 (the gate
    script's non-push rule): the latest completed run on main whose
    head SHA is the release commit or an ancestor of it; it must
    have concluded success."""
    completed = [r for r in runs if r.get("status") == "completed"]
    for run in _runs_by_recency(completed):
        head = run.get("head_sha") or ""
        if head and is_ancestor_or_equal(repo, head, commit):
            if run.get("conclusion") != "success":
                raise ReleaseError(
                    f"{workflow}: latest completed main run at or before "
                    f"the release commit concluded {run.get('conclusion')!r} "
                    f"(run {run.get('id')}, head {head})"
                )
            return {
                "workflow": workflow,
                "run_id": run.get("id"),
                "head_sha": head,
                "conclusion": "success",
            }
    raise ReleaseError(
        f"{workflow}: no completed run on main at or before the release "
        "commit (missing main-gate signal)"
    )


def _judge_push_runs(runs, workflow, commit):
    """The gate script's push rules, for a watched workflow that has
    regained a push trigger: an exact-SHA run judges; a path-filtered
    workflow with no run on the SHA falls back to its latest push run
    on main; any other workflow without an exact-SHA run refuses."""
    exact = [r for r in runs if r.get("head_sha") == commit]
    if exact:
        run = _runs_by_recency(exact)[0]
    elif workflow in PATH_FILTERED_WORKFLOWS and runs:
        run = _runs_by_recency(runs)[0]
    else:
        raise ReleaseError(f"{workflow}: no push run on the release commit")
    if run.get("status") != "completed" or run.get("conclusion") != "success":
        raise ReleaseError(
            f"{workflow}: push run {run.get('id')} on {run.get('head_sha')} "
            f"is status={run.get('status')!r} conclusion={run.get('conclusion')!r}"
        )
    return {
        "workflow": workflow,
        "run_id": run.get("id"),
        "head_sha": run.get("head_sha"),
        "conclusion": "success",
    }


def judge_gate(slug, repo, commit, evidence_workflow):
    """Judge the attestation gate for `commit` (design §3): every
    watched workflow, plus the app's evidence workflow when named and
    not already watched (judged by the same ancestor rule). Returns
    the judged run records in watched order. Raises ReleaseError
    listing every failure — fail closed: a workflow that cannot be
    judged is a workflow that is not green."""
    workflows = _gh_api(slug, "actions/workflows?per_page=100").get("workflows", [])
    ids = {wf.get("name"): wf.get("id") for wf in workflows}
    judged = []
    failures = []

    def judge(name, push):
        wid = ids.get(name)
        if wid is None:
            failures.append(f"{name}: no workflow with that name in {slug}")
            return
        try:
            if push:
                runs = _gh_api(
                    slug,
                    f"actions/workflows/{wid}/runs?branch=main&event=push&per_page=100",
                ).get("workflow_runs", [])
                judged.append(_judge_push_runs(runs, name, commit))
            else:
                runs = _gh_api(
                    slug,
                    f"actions/workflows/{wid}/runs?branch=main&per_page=100",
                ).get("workflow_runs", [])
                judged.append(_judge_runs(runs, name, commit, repo))
        except ReleaseError as exc:
            failures.append(str(exc))

    for name in WATCHED_WORKFLOWS:
        judge(name, push=name not in NON_PUSH_WORKFLOWS)
    if evidence_workflow and evidence_workflow not in WATCHED_WORKFLOWS:
        judge(evidence_workflow, push=False)
    if failures:
        raise ReleaseError("attestation gate is not green: " + "; ".join(failures))
    return judged


def compose_attestation(app, version, commit, evidence_workflow, runs, actor, at):
    """The release-attestation:v1 block (design §4): the record of
    authority, carried in the annotated tag message."""
    gate = (
        f"main-gate + {evidence_workflow}" if evidence_workflow else "main-gate + none"
    )
    lines = [
        ATTESTATION_HEADER,
        f"app: {app}",
        f"version: {version}",
        f"commit: {commit}",
        f"gate: {gate}",
    ]
    for run in runs:
        lines.append(
            f"run: {run['workflow']} {run['run_id']} success {run['head_sha']}"
        )
    lines += [
        f"actor: {actor}",
        f"tool: deploy.py {DEPLOY_TOOL_VERSION}",
        f"at: {at}",
    ]
    return "\n".join(lines)


def parse_attestation(message):
    """Parse a release-attestation:v1 block out of a tag message.
    Returns {app, version, commit, runs: [(workflow, run_id,
    conclusion, head_sha)]} or None when no well-formed block is
    present. Fail closed: a block that does not parse is no
    attestation (design §6)."""
    if not message:
        return None
    lines = message.splitlines()
    try:
        start = lines.index(ATTESTATION_HEADER)
    except ValueError:
        return None
    fields = {}
    runs = []
    for line in lines[start + 1 :]:
        if line.startswith("run: "):
            # "<workflow> <run-id> <conclusion> <head-sha>"; workflow
            # names contain spaces, so split from the right.
            parts = line[len("run: ") :].rsplit(" ", 3)
            if len(parts) != 4:
                return None
            runs.append(tuple(parts))
        elif ": " in line:
            key, value = line.split(": ", 1)
            fields[key.strip()] = value.strip()
        elif line.strip() == "":
            continue
        else:
            break
    app = fields.get("app")
    version = fields.get("version")
    commit = fields.get("commit")
    if not app or not version or not commit or not runs:
        return None
    if parse_version(version) is None or not FULL_SHA_RE.fullmatch(commit):
        return None
    if any(conclusion != "success" for _w, _i, conclusion, _s in runs):
        return None
    return {"app": app, "version": version, "commit": commit, "runs": runs}


def verify_attestation(message, app, version, commit):
    """The parsed attestation when it names exactly this app,
    version and commit; otherwise None."""
    parsed = parse_attestation(message)
    if parsed is None:
        return None
    if (
        parsed["app"] != app
        or parsed["version"] != version
        or parsed["commit"] != commit
    ):
        return None
    return parsed


def app_evidence_workflow(registry, app):
    """The app's recorded evidence workflow (design §3 part 2), or
    None when the registry records none. An app resolves to the
    definition named after it, or to the definitions whose entry
    point is the app's binary (bin/<app>) — the fleet app, for
    instance, is deployed as the queue-broker and secrets-broker
    services, both running bin/fleet. Definitions resolving to one
    app must agree on the field; disagreement is registry data in
    conflict and refuses. An app no definition resolves to is not
    this tool's to release."""
    candidates = []
    if app in registry:
        candidates.append(registry[app])
    else:
        for defn in registry.values():
            if os.path.basename(defn.get("entryPoint", "")) == app:
                candidates.append(defn)
    if not candidates:
        raise ReleaseError(
            f"app {app!r} is not in the deploy registry (no definition "
            "names it or runs its binary)"
        )
    workflows = {d["evidenceWorkflow"] for d in candidates if d.get("evidenceWorkflow")}
    if len(workflows) > 1:
        raise ReleaseError(
            f"registry definitions for app {app!r} disagree on "
            f"evidenceWorkflow: {', '.join(sorted(workflows))}"
        )
    return workflows.pop() if workflows else None


def releases_path(explicit_state=None):
    return os.path.join(state_dir(explicit_state), "releases.jsonl")


def read_releases(explicit_state=None):
    path = releases_path(explicit_state)
    if not os.path.exists(path):
        return []
    entries = []
    with open(path, "r", encoding="utf-8") as fh:
        for lineno, line in enumerate(fh, 1):
            line = line.strip()
            if not line:
                continue
            try:
                entries.append(json.loads(line))
            except ValueError as exc:
                raise ValueError(
                    f"release ledger {path} line {lineno} is not valid JSON: {exc}"
                ) from exc
    return entries


def record_release(entry, explicit_state=None):
    path = releases_path(explicit_state)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(entry, sort_keys=True) + "\n")
    return entry


def local_tag_attestations(repo):
    """{tag name: parsed attestation} for the attested tags in the
    local checkout, read from the tag messages (the record of
    authority)."""
    rc, out, _err = _git(repo, "tag", "--list")
    if rc != 0:
        return {}
    found = {}
    for name in out.split():
        rc, message, _err = _git(
            repo, "for-each-ref", f"refs/tags/{name}", "--format=%(contents)"
        )
        if rc != 0:
            continue
        parsed = parse_attestation(message)
        if parsed is not None:
            found[name] = parsed
    return found


def format_releases(entries, tag_attestations):
    """The `deploy status` release report (design §4): releases and
    their gate evidence. With tag_attestations (the tags read from a
    checkout), the tag is the authority: a ledger record the tags do
    not confirm is a disagreement, never a release. With None (no
    checkout at hand), the ledger is listed unverified."""
    lines = ["releases:"]
    if tag_attestations is None:
        if not entries:
            lines.append("  none recorded")
        for entry in entries:
            lines.append(
                f"  {entry.get('tag')}: app {entry.get('app')} version "
                f"{entry.get('version')} commit {entry.get('commit')} "
                "(ledger only; tags not read)"
            )
        return lines
    by_tag = {entry.get("tag"): entry for entry in entries}
    names = sorted(set(by_tag) | set(tag_attestations))
    if not names:
        lines.append("  none recorded")
        return lines
    for name in names:
        parsed = tag_attestations.get(name)
        entry = by_tag.get(name)
        if parsed is not None:
            runs = ", ".join(
                f"{wf} run {run_id}" for wf, run_id, _c, _s in parsed["runs"]
            )
            lines.append(
                f"  {name}: app {parsed['app']} version {parsed['version']} "
                f"commit {parsed['commit']} (attested; gate: {runs})"
            )
            if entry is not None and (
                entry.get("commit") != parsed["commit"]
                or entry.get("version") != parsed["version"]
            ):
                lines.append(
                    f"  DISAGREEMENT: the ledger records {name} at commit "
                    f"{entry.get('commit')} version {entry.get('version')}; "
                    "the tag wins"
                )
        elif entry is not None:
            lines.append(
                f"  DISAGREEMENT: the ledger records {name} (app "
                f"{entry.get('app')} version {entry.get('version')} commit "
                f"{entry.get('commit')}) but no attested tag confirms it; "
                "the tag wins — not a release"
            )
    return lines


def execute_release(
    app,
    registry,
    *,
    repo,
    state=None,
    requested_version=None,
    commit_arg=None,
    bump="minor",
    dry_run=False,
    actor=None,
):
    """`deploy release <app>`: cut the app's next release tag and the
    release point's repo tag, per the design record §2's order:
    verify preconditions, compose the attestation, create the
    annotated tags, push, append the ledger record, print the tags
    and the gate verdicts. Returns (exit_code, lines); every refusal
    is reported with nothing pushed and nothing recorded. A failure
    after a tag is created but before the push leaves only a local
    tag, which the next invocation's precondition pass deletes."""
    actor = actor or getpass.getuser()
    lines = []
    created = []
    try:
        evidence = app_evidence_workflow(registry, app)
        if not dry_run:
            _git_checked(repo, "fetch", "origin", "main", what="git fetch origin main")
        rc, origin_main, err = _git(repo, "rev-parse", "origin/main^{commit}")
        if rc != 0:
            raise ReleaseError(f"cannot resolve origin/main in {repo}: {err.strip()}")
        origin_main = origin_main.strip()
        commit = origin_main
        if commit_arg:
            rc, resolved, err = _git(repo, "rev-parse", f"{commit_arg}^{{commit}}")
            if rc != 0:
                raise ReleaseError(
                    f"cannot resolve commit {commit_arg!r}: {err.strip()}"
                )
            commit = resolved.strip()
        if not is_ancestor_or_equal(repo, commit, origin_main):
            raise ReleaseError(
                f"commit {commit} is not reachable from origin/main "
                f"({origin_main}); releases are cut from main (design §2)"
            )
        tags = remote_tags(repo)
        existing = []
        for name in tags:
            if name.startswith(f"{app}/v"):
                version = parse_version(name[len(app) + 2 :])
                if version is not None:
                    existing.append(version)
        requested = None
        if requested_version is not None:
            requested = parse_version(requested_version)
            if requested is None:
                raise ReleaseError(
                    f"--version {requested_version!r} is not an X.Y.Z version"
                )
            # Precondition 4 (design §2) is checked before the
            # discipline: a requested tag that already exists is a
            # duplicate or a collision, and says so.
            requested_tag = f"{app}/v{format_version(requested)}"
            if requested_tag in tags:
                if tags[requested_tag] == commit:
                    raise ReleaseError(
                        f"{requested_tag} already exists at {commit}: already released"
                    )
                raise ReleaseError(
                    f"{requested_tag} already exists at "
                    f"{tags[requested_tag]}, not at {commit}"
                )
        version = plan_release_version(existing, requested, bump)
        version_text = format_version(version)
        app_tag = f"{app}/v{version_text}"
        if app_tag in tags:
            if tags[app_tag] == commit:
                raise ReleaseError(
                    f"{app_tag} already exists at {commit}: already released"
                )
            raise ReleaseError(
                f"{app_tag} already exists at {tags[app_tag]}, not at {commit}"
            )
        today = datetime.now(timezone.utc).strftime("%Y%m%d")
        feature = not existing or version[2] == 0
        repo_tag = next_repo_tag(list(tags), today, feature)
        if repo_tag in tags:
            raise ReleaseError(f"{repo_tag} already exists on the remote")
        # Unpublished local leftovers from a failed earlier run are
        # deleted on this pass (design §2); a tag the remote also has
        # was already refused above.
        for name in (app_tag, repo_tag):
            rc, local_commit, _e = _git(
                repo, "rev-parse", f"refs/tags/{name}^{{commit}}"
            )
            if rc == 0 and name not in tags:
                _git(repo, "tag", "-d", name)
                lines.append(
                    f"removed unpublished local tag {name} "
                    f"(leftover at {local_commit.strip()})"
                )
        slug = gh_repo_slug(repo)
        if slug is None:
            raise ReleaseError(
                "cannot determine the GitHub repo (no DEPLOY_GH_REPO/GH_REPO "
                "and no github.com origin remote); the gate cannot be judged"
            )
        runs = judge_gate(slug, repo, commit, evidence)
        at = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
        block = compose_attestation(
            app, version_text, commit, evidence, runs, actor, at
        )
        app_message = f"{app} v{version_text}\n\n{block}\n"
        repo_version = repo_tag[len("repo/v") :]
        repo_block = compose_attestation(
            "repo", repo_version, commit, evidence, runs, actor, at
        )
        repo_message = (
            f"repo release point {repo_version}\n\napp-tag: {app_tag}\n\n{repo_block}\n"
        )
        verdicts = "; ".join(
            f"{run['workflow']} run {run['run_id']} success at {run['head_sha']}"
            for run in runs
        )
        if dry_run:
            lines.append(
                "deploy release --dry-run: no tags cut, nothing recorded "
                "(origin/main as last fetched)"
            )
            lines.append(f"  would cut {app_tag} at {commit}")
            lines.append(f"  would cut {repo_tag} at {commit}")
            lines.append(f"  gate: {verdicts}")
            return 0, lines
        _git_checked(
            repo,
            "tag",
            "-a",
            app_tag,
            "-m",
            app_message,
            commit,
            what=f"create tag {app_tag}",
        )
        created.append(app_tag)
        _git_checked(
            repo,
            "tag",
            "-a",
            repo_tag,
            "-m",
            repo_message,
            commit,
            what=f"create tag {repo_tag}",
        )
        created.append(repo_tag)
        _git_checked(
            repo,
            "push",
            "origin",
            f"refs/tags/{app_tag}",
            f"refs/tags/{repo_tag}",
            what="push the release tags",
        )
        record_release(
            {
                "app": app,
                "version": version_text,
                "commit": commit,
                "tag": app_tag,
                "repo_tag": repo_tag,
                "gate": runs,
                "actor": actor,
                "at": at,
            },
            explicit_state=state,
        )
        lines.append(f"released {app_tag} at {commit}")
        lines.append(f"released {repo_tag} at {commit}")
        lines.append(f"gate: {verdicts}")
        return 0, lines
    except ReleaseError as exc:
        lines.append(f"deploy release {app}: REFUSED: {exc}")
        if created:
            lines.append(
                f"note: local tag(s) {', '.join(created)} were created but "
                "not pushed; the next invocation's precondition pass "
                "removes them"
            )
        return 1, lines


def probe_artifact_version(defn):
    """The version the artifact self-reports, read from its own
    `Derived version:` line (design §4 part 3; the fleet README's
    release procedure step 3) by running its entry point with the
    `version` argument. Returns the version token, or None when the
    artifact reports none (not executable, no such line, any error).
    A read-only probe; tests replace it like probe_health."""
    path = os.path.join(defn["package"], defn["entryPoint"])
    if not (os.path.isfile(path) and os.access(path, os.X_OK)):
        return None
    try:
        proc = subprocess.run(
            [path, "version"], capture_output=True, text=True, timeout=15
        )
    except (OSError, subprocess.SubprocessError):
        return None
    for line in proc.stdout.splitlines():
        if line.startswith("Derived version:"):
            return line.split(":", 1)[1].strip() or None
    return None


def _verify_release_claim(slug, app, version, source_rev):
    """None when an attested <app>/v<version> tag exists at
    source_rev with a verifiable attestation block; otherwise the
    reason it does not verify."""
    tag = f"{app}/v{version}"
    try:
        ref = _gh_api(slug, f"git/ref/tags/{tag}")
        obj = ref.get("object", {})
        if obj.get("type") != "tag":
            return f"{tag} is not an annotated tag and carries no attestation"
        tag_object = _gh_api(slug, f"git/tags/{obj.get('sha')}")
    except ReleaseError as exc:
        return f"the attested tag cannot be read: {exc}"
    points_at = tag_object.get("object", {}).get("sha")
    if points_at != source_rev:
        return (
            f"{tag} points at {points_at}, not the artifact's "
            f"source commit {source_rev}"
        )
    if verify_attestation(tag_object.get("message"), app, version, source_rev) is None:
        return (
            f"{tag} carries no verifiable release-attestation:v1 "
            "block naming this commit"
        )
    return None


def release_claim_refusal(name, defn):
    """Design §6 point 1: a deploy whose artifact self-reports a
    clean X.Y.Z claims release status and is refused unless an
    attested <app>/vX.Y.Z tag exists at the artifact's source commit
    with a verifiable attestation block. The app a definition speaks
    for is its own name or the app whose binary it runs (bin/<app>)
    — the fleet app is deployed as the queue-broker and
    secrets-broker services — so a claim verifies under either name.
    Returns the refusal reason, or None when the deploy may proceed
    (no claim, or a verified attested tag). Fail closed: a claim
    that cannot be verified is a claim refused."""
    reported = probe_artifact_version(defn)
    if reported is None or parse_version(reported) is None:
        # No version reported, or a -dev / prerelease string: no
        # release claim, and prerelease deploys are untouched by the
        # refusal (design §6).
        return None
    source_rev = _source_rev()
    if not FULL_SHA_RE.fullmatch(source_rev):
        return (
            f"artifact claims release {reported} but its source commit is "
            f"not recorded (DEPLOY_SOURCE_REV={source_rev!r}); the attested "
            "tag cannot be checked — deploy refused (design §6)"
        )
    slug = gh_repo_slug(os.getcwd())
    if slug is None:
        return (
            f"artifact claims release {reported} but the GitHub repo is "
            "unknown (no DEPLOY_GH_REPO/GH_REPO and no github.com origin "
            "in the working directory); the attested tag cannot be "
            "checked — deploy refused (design §6)"
        )
    apps = [name]
    binary_app = os.path.basename(defn.get("entryPoint", ""))
    if binary_app and binary_app != name:
        apps.append(binary_app)
    reasons = []
    for app in apps:
        reason = _verify_release_claim(slug, app, reported, source_rev)
        if reason is None:
            return None
        reasons.append(reason)
    return (
        f"artifact claims release {reported} but no attested tag verifies: "
        + "; ".join(reasons)
        + " — deploy refused (design §6)"
    )


def _release_args(rest):
    """Parse the arguments after `release`: one positional app plus
    --version/--commit/--bump in '--flag value' or '--flag=value'
    form. Returns (app, options) or (None, error)."""
    app = None
    options = {}
    it = iter(rest)
    for arg in it:
        flag = next(
            (
                f
                for f in ("--version", "--commit", "--bump")
                if arg == f or arg.startswith(f + "=")
            ),
            None,
        )
        if flag is None:
            if app is not None:
                return None, f"unexpected argument {arg!r} after the app name"
            app = arg
            continue
        if flag in options:
            return None, f"{flag} given more than once"
        value = next(it, None) if arg == flag else arg[len(flag) + 1 :]
        if not value:
            return None, f"{flag} needs a value"
        options[flag] = value
    if app is None:
        return None, "release needs an app name"
    bump = options.get("--bump", "minor")
    if bump not in ("minor", "patch"):
        return None, f"--bump must be minor or patch, got {bump!r}"
    return app, {
        "requested_version": options.get("--version"),
        "commit_arg": options.get("--commit"),
        "bump": bump,
    }


def cmd_status_all(registry, state, repo):
    """Bare `deploy status`: every target's report (health unprobed),
    then the release report (design §4) — the tags are read when a
    git checkout is at hand, the ledger listed unverified otherwise."""
    for name in sorted(registry):
        try:
            entries = read_generations(name, state)
        except ValueError as exc:
            print(f"deploy: {exc}", file=sys.stderr)
            return 1
        defn = registry[name]
        if defn.get("class") == "cloudflare" and name in CLOUDFLARE_LIFECYCLE_TARGETS:
            print(format_cloudflare_status(name, defn, entries))
        else:
            print(format_status(name, registry[name], entries))
    try:
        entries = read_releases(state)
    except ValueError as exc:
        print(f"deploy: {exc}", file=sys.stderr)
        return 1
    attestations = None
    rc, _o, _e = _git(repo, "rev-parse", "--git-dir")
    if rc == 0:
        attestations = local_tag_attestations(repo)
    for line in format_releases(entries, attestations):
        print(line)
    return 0


# --- Command dispatch ------------------------------------------------------

NOT_IMPLEMENTED = (
    "real activation for this class is not implemented in this slice; "
    "slice 3 implements real systemd activation (profile install + unit "
    "write + daemon-reload + restart + health-check flip-back) and "
    "slice 4 real sandbox activation — no changes were made"
)


def main(argv):
    registry_path = os.environ.get("DEPLOY_REGISTRY")
    explicit_state = None
    explicit_base = None
    # Value flags may appear before or after the subcommand, in either
    # "--flag PATH" or "--flag=PATH" form, each at most once: a second
    # occurrence is the typo-flag class of issue #6376, never a silent
    # override of the first.
    flag_values = {}
    args = []
    it = iter(argv)
    for arg in it:
        flag = next(
            (
                f
                for f in (
                    "--registry",
                    "--state-dir",
                    "--profile-base",
                    "--unit-dir",
                    "--scope",
                    "--repo",
                )
                if arg == f or arg.startswith(f + "=")
            ),
            None,
        )
        if flag is None:
            args.append(arg)
            continue
        if flag in flag_values:
            print(f"deploy: {flag} given more than once", file=sys.stderr)
            return 2
        value = next(it, None) if arg == flag else arg[len(flag) + 1 :]
        if not value:
            print(f"deploy: {flag} needs a path", file=sys.stderr)
            return 2
        flag_values[flag] = value
    if "--registry" in flag_values:
        registry_path = flag_values["--registry"]
    explicit_state = flag_values.get("--state-dir")
    explicit_base = flag_values.get("--profile-base")
    explicit_units = flag_values.get("--unit-dir")
    explicit_repo = (
        flag_values.get("--repo") or os.environ.get("DEPLOY_REPO") or os.getcwd()
    )
    scope = flag_values.get("--scope") or os.environ.get("DEPLOY_SCOPE") or "system"
    if scope not in ("system", "user"):
        print(f"deploy: --scope must be system or user, got {scope!r}", file=sys.stderr)
        return 2
    dry_run = "--dry-run" in args
    args = [a for a in args if a != "--dry-run"]
    usage = (
        "usage: deploy [--registry PATH] [--state-dir PATH] "
        "[--profile-base PATH] [--unit-dir PATH] [--scope system|user] "
        "[--repo PATH] list | restore | status | "
        "release <app> [--version X.Y.Z] [--commit SHA] [--bump minor|patch] | "
        "<target> [rollback|status] [--dry-run]"
    )
    if not args:
        print(__doc__.splitlines()[0], file=sys.stderr)
        print(usage, file=sys.stderr)
        return 2
    if args[0] == "restore":
        # The generic sandbox restorer (design §4.4): manifest-driven,
        # so it needs no registry and works before any target's
        # package is built on a freshly recycled sandbox.
        if len(args) > 1:
            print(
                f"deploy: unexpected argument {args[1]!r} after 'restore'",
                file=sys.stderr,
            )
            return 2
        if scope != "system":
            print(
                "deploy: restore manages system units (sandbox targets "
                "have no user bus); --scope must be system",
                file=sys.stderr,
            )
            return 2
        code, lines = restore_sandbox(
            base=explicit_base,
            state=explicit_state,
            units_dir=unit_dir(explicit_units, "system"),
        )
        for line in lines:
            print(line, file=sys.stderr if code else sys.stdout)
        return code
    if registry_path is None:
        print(
            "deploy: no registry configured (build via the flake package, or pass --registry)",
            file=sys.stderr,
        )
        return 2
    try:
        registry = load_registry(registry_path)
    except (OSError, ValueError) as exc:
        print(f"deploy: cannot load registry {registry_path}: {exc}", file=sys.stderr)
        return 1
    if args[0] == "list":
        if len(args) > 1:
            print(
                f"deploy: unexpected argument {args[1]!r} after 'list'",
                file=sys.stderr,
            )
            return 2
        return cmd_list(registry)
    errors = validate(registry)
    if errors:
        for err in errors:
            print(f"deploy: invalid definition: {err}", file=sys.stderr)
        return 1
    if args[0] == "release":
        app, parsed = _release_args(args[1:])
        if app is None:
            print(f"deploy: {parsed}", file=sys.stderr)
            print(usage, file=sys.stderr)
            return 2
        code, lines = execute_release(
            app,
            registry,
            repo=explicit_repo,
            state=explicit_state,
            dry_run=dry_run,
            **parsed,
        )
        for line in lines:
            print(line, file=sys.stderr if code else sys.stdout)
        return code
    if args[0] == "status" and len(args) == 1:
        return cmd_status_all(registry, explicit_state, explicit_repo)
    name = args[0]
    if name not in registry:
        print(f"deploy: unknown target {name!r}", file=sys.stderr)
        return 1
    defn = registry[name]
    is_cloudflare_lifecycle = (
        defn.get("class") == "cloudflare" and name in CLOUDFLARE_LIFECYCLE_TARGETS
    )
    if defn["class"] == "sandbox":
        # Sandbox targets keep their generations in the persistent
        # home profile (design §4.4), not the system profile root,
        # unless a base was named explicitly or by the environment.
        if explicit_base is None and "DEPLOY_PROFILE_BASE" not in os.environ:
            explicit_base = sandbox_profile_base()
        if scope != "system":
            print(
                "deploy: sandbox targets manage system units (there is "
                "no user bus); --scope must be system",
                file=sys.stderr,
            )
            return 2
    action = args[1] if len(args) > 1 else "deploy"
    if len(args) > 2 or action not in ("deploy", "rollback", "status"):
        print(usage, file=sys.stderr)
        return 2
    try:
        entries = read_generations(name, explicit_state)
    except ValueError as exc:
        print(f"deploy: {exc}", file=sys.stderr)
        return 1
    if action == "status":
        if is_cloudflare_lifecycle:
            # Cloudflare status is the provider's serving version, never
            # a log inference. A provider failure is reported, not a
            # crash: status stays the audit surface (exit 0).
            serving = None
            serving_error = None
            if dry_run:
                serving_error = "not queried (--dry-run)"
            else:
                try:
                    serving = cf_get_serving(name, defn)
                except CloudflareError as exc:
                    serving_error = str(exc)
            print(
                format_cloudflare_status(
                    name,
                    defn,
                    entries,
                    serving=serving,
                    serving_error=serving_error,
                )
            )
            try:
                app_releases = [
                    e for e in read_releases(explicit_state) if e.get("app") == name
                ]
            except ValueError as exc:
                print(f"deploy: {exc}", file=sys.stderr)
                return 1
            for entry in app_releases:
                print(
                    f"release: {entry.get('tag')} at {entry.get('commit')} "
                    f"(ledger record {entry.get('at')} by {entry.get('actor')}; "
                    "the tag's attestation block is the record of authority)"
                )
            return 0
        health = None
        if defn["healthCheck"] is not None:
            health = probe_health(defn["healthCheck"])
        print(format_status(name, defn, entries, health))
        try:
            app_releases = [
                e for e in read_releases(explicit_state) if e.get("app") == name
            ]
        except ValueError as exc:
            print(f"deploy: {exc}", file=sys.stderr)
            return 1
        for entry in app_releases:
            print(
                f"release: {entry.get('tag')} at {entry.get('commit')} "
                f"(ledger record {entry.get('at')} by {entry.get('actor')}; "
                "the tag's attestation block is the record of authority)"
            )
        return 0
    if action == "deploy":
        steps = plan_deploy(name, defn, entries, explicit_base)
        if steps is None:
            print(
                f"deploy: activation for class {defn['class']!r} is a different "
                "slice of the deployment architecture (see issue #5767); this "
                "slice covers the systemd/launchd/sandbox classes",
                file=sys.stderr,
            )
            return 2
        if not dry_run:
            if is_cloudflare_lifecycle:
                code, lines = execute_deploy_cloudflare(
                    name,
                    defn,
                    entries,
                    state=explicit_state,
                )
            elif defn["class"] == "sandbox":
                code, lines = execute_deploy_sandbox(
                    name,
                    defn,
                    entries,
                    base=explicit_base,
                    state=explicit_state,
                    units_dir=unit_dir(explicit_units, "system"),
                )
            elif defn["class"] == "systemd":
                code, lines = execute_deploy(
                    name,
                    defn,
                    entries,
                    base=explicit_base,
                    state=explicit_state,
                    scope=scope,
                    units=unit_dir(explicit_units, scope),
                )
            else:
                print(f"deploy: {NOT_IMPLEMENTED}", file=sys.stderr)
                return 2
            for line in lines:
                print(line, file=sys.stderr if code else sys.stdout)
            return code
        print(f"deploy {name} --dry-run: no changes made")
        for i, step in enumerate(steps, 1):
            print(f"  {i}. {step}")
        return 0
    # action == rollback
    steps, error = plan_rollback(name, defn, entries, explicit_base)
    if error is not None:
        print(f"deploy: {error}", file=sys.stderr)
        return 1
    if not dry_run:
        if is_cloudflare_lifecycle:
            code, lines = execute_rollback_cloudflare(
                name,
                defn,
                entries,
                state=explicit_state,
            )
        elif defn["class"] == "sandbox":
            code, lines = execute_rollback_sandbox(
                name,
                defn,
                entries,
                base=explicit_base,
                state=explicit_state,
                units_dir=unit_dir(explicit_units, "system"),
            )
        elif defn["class"] == "systemd":
            code, lines = execute_rollback(
                name,
                defn,
                entries,
                base=explicit_base,
                state=explicit_state,
                scope=scope,
                units=unit_dir(explicit_units, scope),
            )
        else:
            print(f"deploy: {NOT_IMPLEMENTED}", file=sys.stderr)
            return 2
        for line in lines:
            print(line, file=sys.stderr if code else sys.stdout)
        return code
    print(f"deploy {name} rollback --dry-run: no changes made")
    for i, step in enumerate(steps, 1):
        print(f"  {i}. {step}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
