#!/usr/bin/env python3
"""deploy — the deployment-architecture CLI (issue #5766, slice 1).

Slice 1 ships the registry foundation only: `deploy list` enumerates the
`deployTargets` registry (name, class, targets) after validating every
definition against the design §3 option shape
(doc/2026-11-04_deployment-architecture-services-layer.md). Activation,
status, and rollback are later slices of issue #5766 and are NOT
implemented here — those subcommands say so and exit non-zero.

The registry is JSON baked into the package at build time (the Nix
expression renders the attrset definitions, replacing each `package`
derivation with its store path). `--registry PATH` (or $DEPLOY_REGISTRY)
points the CLI at another registry file, which is how the flake checks
exercise the malformed-definition path.

The validation below is the oracle for "well-formed definition": a
definition is valid iff it carries exactly the §3 fields (plus `class`),
each of the right type and in range. Any violation is reported by field
and makes `list` exit non-zero — a registry that cannot be validated
must never be enumerated as if it were trustworthy.
"""

import json
import os
import re
import sys

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


def validate(registry):
    """Return a list of validation errors (empty = registry is valid)."""
    errors = []
    if not isinstance(registry, dict):
        return ["registry: top level must be an object mapping name -> definition"]
    if not registry:
        return ["registry: no service definitions"]
    for name, defn in sorted(registry.items()):
        where = f"service {name!r}"
        if not NAME_RE.match(name):
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
        if "restart" in defn and defn["restart"] not in RESTART_POLICIES:
            errors.append(
                f"{where}: restart must be one of {', '.join(sorted(RESTART_POLICIES))}"
            )
        if "healthCheck" in defn and defn["healthCheck"] is not None:
            hc = defn["healthCheck"]
            if not (
                isinstance(hc, str)
                and (hc.startswith("http://") or hc.startswith("https://"))
            ):
                errors.append(f"{where}: healthCheck must be an http(s) URL or null")
        if "targets" in defn and not (
            isinstance(defn["targets"], list)
            and defn["targets"]
            and all(isinstance(t, str) and t for t in defn["targets"])
        ):
            errors.append(
                f"{where}: targets must be a non-empty list of host/class names"
            )
        if "class" in defn and defn["class"] not in CLASSES:
            errors.append(f"{where}: class must be one of {', '.join(sorted(CLASSES))}")
    return errors


def load_registry(path):
    with open(path, "r", encoding="utf-8") as fh:
        return json.load(fh)


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


def main(argv):
    args = list(argv)
    registry_path = os.environ.get("DEPLOY_REGISTRY")
    # --registry may appear before or after the subcommand.
    while "--registry" in args:
        idx = args.index("--registry")
        if idx + 1 >= len(args):
            print("deploy: --registry needs a path", file=sys.stderr)
            return 2
        registry_path = args[idx + 1]
        del args[idx : idx + 2]
    if not args:
        print(__doc__.splitlines()[0], file=sys.stderr)
        print(
            "usage: deploy [--registry PATH] list | <target> [rollback|status] [--dry-run]",
            file=sys.stderr,
        )
        return 2
    if args[0] != "list":
        print(
            f"deploy: {args[0]!r} is not implemented in slice 1 of issue #5766 "
            "(registry + list only; activation/status/rollback are later slices)",
            file=sys.stderr,
        )
        return 2
    if registry_path is None:
        print(
            "deploy: no registry configured (build via the flake package, or pass --registry)",
            file=sys.stderr,
        )
        return 2
    try:
        registry = load_registry(registry_path)
    except (OSError, json.JSONDecodeError) as exc:
        print(f"deploy: cannot load registry {registry_path}: {exc}", file=sys.stderr)
        return 1
    return cmd_list(registry)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
