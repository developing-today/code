#!/usr/bin/env python3
"""deploy — the deployment-architecture CLI (issue #5766, slices 1-3).

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
definition is valid iff it carries exactly the §3 fields (plus ),
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
NAME_RE = re.compile(r"^[a-z0-9][a-z0-9-]*$")

# Design §4.1: each systemd target owns a per-service nix profile under
# this root (system scope); the rendered unit points at the profile's
# `current` link.
DEFAULT_PROFILE_BASE = "/nix/var/nix/profiles/per-service"
ACTIVATION_CLASSES = {"systemd", "launchd"}


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
        extra = defn.keys() - FIELDS
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


def _health_step(defn):
    if defn["healthCheck"]:
        return f"run the health check: {defn['healthCheck']}"
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


def wait_health(url, attempts=10, interval=0.5):
    """Probe the health check until it passes or attempts run out."""
    detail = "not probed"
    for attempt in range(attempts):
        ok, detail = probe_health(url)
        if ok:
            return True, detail
        if attempt + 1 < attempts:
            time.sleep(interval)
    return False, detail


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
        ok, detail = wait_health(defn["healthCheck"])
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


def _source_rev():
    return os.environ.get("DEPLOY_SOURCE_REV") or "working-tree"


# --- Command dispatch ------------------------------------------------------

NOT_IMPLEMENTED = (
    "real activation for this class is not implemented in this slice; "
    "slice 3 implements real systemd activation (profile install + unit "
    "write + daemon-reload + restart + health-check flip-back) — "
    "no changes were made"
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
        "list | <target> [rollback|status] [--dry-run]"
    )
    if not args:
        print(__doc__.splitlines()[0], file=sys.stderr)
        print(usage, file=sys.stderr)
        return 2
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
                "slice covers the systemd/launchd classes",
                file=sys.stderr,
            )
            return 2
        if not dry_run:
            if defn["class"] != "systemd":
                print(f"deploy: {NOT_IMPLEMENTED}", file=sys.stderr)
                return 2
            code, lines = execute_deploy(
                name,
                defn,
                entries,
                base=explicit_base,
                state=explicit_state,
                scope=scope,
                units=unit_dir(explicit_units, scope),
            )
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
        if defn["class"] != "systemd":
            print(f"deploy: {NOT_IMPLEMENTED}", file=sys.stderr)
            return 2
        code, lines = execute_rollback(
            name,
            defn,
            entries,
            base=explicit_base,
            state=explicit_state,
            scope=scope,
            units=unit_dir(explicit_units, scope),
        )
        for line in lines:
            print(line, file=sys.stderr if code else sys.stdout)
        return code
    print(f"deploy {name} rollback --dry-run: no changes made")
    for i, step in enumerate(steps, 1):
        print(f"  {i}. {step}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
