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

The registry is JSON baked into the package at build time (the Nix
expression renders the attrset definitions, replacing each
derivation with its store path). `--registry PATH` (or $DEPLOY_REGISTRY)
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
the optional healthTimeoutSeconds),
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
# present must carry exactly TIMER_FIELDS.
OPTIONAL_FIELDS = {"healthTimeoutSeconds", "timer"}
TIMER_FIELDS = {"onCalendar", "persistent", "randomizedDelaySec"}
DEFAULT_HEALTH_TIMEOUT_SECONDS = 120
MAX_HEALTH_TIMEOUT_SECONDS = 3600
HEALTH_PROBE_INTERVAL_SECONDS = 0.5
NAME_RE = re.compile(r"^[a-z0-9][a-z0-9-]*$")

# Design §4.1: each systemd target owns a per-service nix profile under
# this root (system scope); the rendered unit points at the profile's
# `current` link.
DEFAULT_PROFILE_BASE = "/nix/var/nix/profiles/per-service"
ACTIVATION_CLASSES = {"systemd", "launchd", "sandbox"}


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
        "",
        "[Service]",
        f"ExecStart={argv}",
        f"User={defn['user']}",
        f"Restart={defn['restart']}",
    ]
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
    # only defined when the folded history has one.
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


def install_generation(name, artifact, generation, base=None):
    """Point the profile link for `name` at a new generation dir
    whose `current` symlink names `artifact`. Returns the link path."""
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
    scope = flag_values.get("--scope") or os.environ.get("DEPLOY_SCOPE") or "system"
    if scope not in ("system", "user"):
        print(f"deploy: --scope must be system or user, got {scope!r}", file=sys.stderr)
        return 2
    dry_run = "--dry-run" in args
    args = [a for a in args if a != "--dry-run"]
    usage = (
        "usage: deploy [--registry PATH] [--state-dir PATH] "
        "[--profile-base PATH] [--unit-dir PATH] [--scope system|user] "
        "list | restore | <target> [rollback|status] [--dry-run]"
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
    name = args[0]
    if name not in registry:
        print(f"deploy: unknown target {name!r}", file=sys.stderr)
        return 1
    defn = registry[name]
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
        health = None
        if defn["healthCheck"] is not None:
            health = probe_health(defn["healthCheck"])
        print(format_status(name, defn, entries, health))
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
            if defn["class"] == "sandbox":
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
        if defn["class"] == "sandbox":
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
