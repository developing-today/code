#!/usr/bin/env python3
"""deploy-restore — the generic sandbox unit restorer (issue #5770).

Design: doc/2026-11-04_deployment-architecture-services-layer.md §4.4.
A sandbox recycle wipes /etc/systemd/system but home survives, so the
only bootstrap the platform still owes is this: read the deploy
sandbox manifest (the list of currently-activated sandbox targets,
kept in the persistent state dir by `deploy` activations), link each
target's current generation's rendered units into the unit dir,
daemon-reload, and enable --now each target's enable unit (its timer
when one was rendered, else its service). One restorer replaces the
per-service install.sh pattern; a service with no deploy target has
no units to restore, so it cannot be stranded silently.

This script is the boot-time copy of the contract that deploy.py's
`deploy restore` verb implements. It is deliberately standalone
(stdlib only, no deploy.py import): the sandbox bootstrap installs
it into ~/tools and runs it from init.sh on every boot, at a point
where no built deploy CLI may exist yet. The manifest format is
owned by deploy.py (slice 4): {targets: {name: {"profile": <profile
link path>, "units": [<unit file>, ...]}}} — the profile LINK is
recorded, never a generation directory, so a rollback repoint is
restored as-is.

Command line (the whole contract; anything else is a usage error
with exit 2, decided before any subprocess or file write — #3352):
  deploy-restore.py [--state-dir PATH] [--unit-dir PATH]
                    [--target NAME]
Each flag takes exactly one value and may appear at most once, in
"--flag PATH" or "--flag=PATH" form. Defaults: state dir
~/.local/state/deploy (or $DEPLOY_STATE_DIR), unit dir
/etc/systemd/system (or $DEPLOY_UNIT_DIR). Sandbox units are system
units (there is no user bus), so there is no scope flag.

Exit: 0 when every manifest target is linked and its enable unit is
active (an absent/empty manifest is a successful no-op), 1 when any
target failed (the others are still restored), 2 on usage errors.
"""

import json
import os
import subprocess
import sys

FLAGS = ("--state-dir", "--unit-dir", "--target")
UNIT_SUFFIXES = (".service", ".timer")


def _no_duplicate_keys(pairs):
    # Same discipline as deploy.py's registry load: discarded text
    # must fail the load instead of vanishing.
    obj = {}
    for key, value in pairs:
        if key in obj:
            raise ValueError(f"duplicate key {key!r} in manifest")
        obj[key] = value
    return obj


def parse_args(argv):
    """Strict parse against the declared contract. Returns
    (options, None) or (None, error_message)."""
    options = {}
    it = iter(argv)
    for arg in it:
        flag = next(
            (f for f in FLAGS if arg == f or arg.startswith(f + "=")),
            None,
        )
        if flag is None:
            return None, f"unexpected argument {arg!r}"
        if flag in options:
            return None, f"{flag} given more than once"
        value = next(it, None) if arg == flag else arg[len(flag) + 1 :]
        if not value:
            return None, f"{flag} needs a value"
        options[flag] = value
    return options, None


def run_cmd(argv):
    """Run one external command; return (returncode, stdout, stderr).
    A missing binary is a failed step (127), never a crash."""
    try:
        proc = subprocess.run(argv, capture_output=True, text=True)
    except OSError as exc:
        return 127, "", f"{argv[0]}: {exc}"
    return proc.returncode, proc.stdout, proc.stderr


def enable_unit_for(unit_files):
    for filename in unit_files:
        if filename.endswith(".timer"):
            return filename
    return unit_files[0]


def restore_target(name, entry, unit_dir):
    """Link one target's current generation's units into unit_dir.
    Returns (linked_files, enable_unit) or raises ValueError with
    the failure detail."""
    profile = entry["profile"]
    unit_files = entry["units"]
    gen_dir = os.path.realpath(profile)
    src_dir = os.path.join(gen_dir, "units")
    if not os.path.isdir(src_dir):
        raise ValueError(
            f"rendered units not found at {src_dir} (profile {profile} "
            "does not resolve to a deployed generation)"
        )
    linked = []
    os.makedirs(unit_dir, exist_ok=True)
    for filename in unit_files:
        if not filename.endswith(UNIT_SUFFIXES) or "/" in filename:
            raise ValueError(f"manifest unit {filename!r} is not a unit file name")
        src = os.path.join(src_dir, filename)
        if not os.path.exists(src):
            raise ValueError(f"rendered unit {src} is missing")
        dest = os.path.join(unit_dir, filename)
        tmp = f"{dest}.tmp-{os.getpid()}"
        if os.path.islink(tmp) or os.path.exists(tmp):
            os.unlink(tmp)
        os.symlink(src, tmp)
        os.replace(tmp, dest)
        linked.append(filename)
    return linked, enable_unit_for(linked)


def main(argv):
    options, error = parse_args(argv)
    if error is not None:
        print(f"deploy-restore: {error}", file=sys.stderr)
        print(
            "usage: deploy-restore.py [--state-dir PATH] "
            "[--unit-dir PATH] [--target NAME]",
            file=sys.stderr,
        )
        return 2
    state_dir = (
        options.get("--state-dir")
        or os.environ.get("DEPLOY_STATE_DIR")
        or os.path.expanduser("~/.local/state/deploy")
    )
    unit_dir = (
        options.get("--unit-dir")
        or os.environ.get("DEPLOY_UNIT_DIR")
        or "/etc/systemd/system"
    )
    only = options.get("--target")
    manifest_path = os.path.join(state_dir, "sandbox-manifest.json")
    if not os.path.exists(manifest_path):
        print("deploy-restore: no sandbox manifest; nothing to do")
        return 0
    try:
        with open(manifest_path, "r", encoding="utf-8") as fh:
            data = json.load(fh, object_pairs_hook=_no_duplicate_keys)
        targets = data["targets"]
        if not isinstance(targets, dict):
            raise ValueError("manifest targets is not an object")
    except (OSError, ValueError, KeyError) as exc:
        print(
            f"deploy-restore: cannot read the sandbox manifest {manifest_path}: {exc}",
            file=sys.stderr,
        )
        return 1
    if only is not None:
        if only not in targets:
            print(
                f"deploy-restore: target {only!r} is not in the sandbox manifest",
                file=sys.stderr,
            )
            return 1
        targets = {only: targets[only]}
    if not targets:
        print("deploy-restore: no sandbox targets recorded; nothing to do")
        return 0
    # Link everything first, then one daemon-reload, then enables:
    # systemd never sees a half-linked set as a reload boundary.
    pending = []
    failed = False
    for name in sorted(targets):
        try:
            linked, enable = restore_target(name, targets[name], unit_dir)
        except (ValueError, KeyError, OSError) as exc:
            failed = True
            print(f"deploy-restore: {name}: FAILED: {exc}", file=sys.stderr)
            continue
        pending.append((name, linked, enable))
    if pending:
        rc, _out, err = run_cmd(["systemctl", "daemon-reload"])
        if rc != 0:
            print(
                f"deploy-restore: daemon-reload failed (exit {rc}): {err.strip()}",
                file=sys.stderr,
            )
            return 1
    for name, linked, enable in pending:
        rc, _out, err = run_cmd(["systemctl", "enable", "--now", enable])
        if rc != 0:
            failed = True
            print(
                f"deploy-restore: {name}: enable --now {enable} failed "
                f"(exit {rc}): {err.strip()}",
                file=sys.stderr,
            )
            continue
        rc, _out, err = run_cmd(["systemctl", "is-active", "--quiet", enable])
        if rc != 0:
            failed = True
            print(
                f"deploy-restore: {name}: {enable} is not active after "
                f"enable --now: {err.strip()}",
                file=sys.stderr,
            )
            continue
        print(f"deploy-restore: {name}: linked {', '.join(linked)}; {enable} active")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
