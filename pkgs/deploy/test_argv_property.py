#!/usr/bin/env python3
"""Property-based coverage for the deploy argv parser (issue #6376).

Invariants (the parser contract in deploy.py main()):
- Each value flag (--registry, --state-dir, --profile-base) occurs at
  most once, in "--flag PATH" or "--flag=PATH" form, before or after
  the subcommand; a second occurrence is rejected, never a silent
  override of the first.
- A value flag with no value (trailing "--flag", or "--flag=") is
  rejected.
- A valid invocation selects exactly the registry its single
  --registry names (or $DEPLOY_REGISTRY when the flag is absent);
  flag placement and spelling never change the selection.

Oracle: a sequential model of the parser contract. The generator
emits argv as flag/subcommand units; the model scans the units in
order, firing the duplicate error at the second occurrence of a flag
and the missing-value error at a valueless flag, and otherwise
predicts exit 0 with the modelled registry's services listed. Any
disagreement between model and main() is a counterexample.

Stdlib-only (no hypothesis in this repo's Python tooling), like
test_dupkeys_property.py; a fixed default seed keeps CI
deterministic, and a failing case prints its seed and argv.

Usage: test_argv_property.py [path-to-deploy.py] [--seed N] [--cases N]
"""

import contextlib
import importlib.util
import io
import json
import os
import random
import sys
import tempfile

FLAGS = ("--registry", "--state-dir", "--profile-base")


def load(path):
    spec = importlib.util.spec_from_file_location("deploy_under_test", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def run(mod, argv, env):
    out, err = io.StringIO(), io.StringIO()
    old = os.environ.pop("DEPLOY_REGISTRY", None)
    if env is not None:
        os.environ["DEPLOY_REGISTRY"] = env
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = mod.main(argv)
    finally:
        os.environ.pop("DEPLOY_REGISTRY", None)
        if old is not None:
            os.environ["DEPLOY_REGISTRY"] = old
    return code, out.getvalue(), err.getvalue()


def definition():
    return {
        "package": "/nix/store/x-pkg",
        "entryPoint": "bin/x",
        "args": [],
        "environment": {},
        "secrets": [],
        "user": "svc",
        "ports": [8080],
        "restart": "always",
        "healthCheck": None,
        "targets": ["laptop"],
        "class": "systemd",
    }


# A unit is ("flag", flag, spelling, value) | ("list",) | ("dry-run",).
# spelling is "space" ("--flag value") or "eq" ("--flag=value").
def render(unit):
    if unit[0] == "flag":
        _, flag, spelling, value = unit
        return [flag, value] if spelling == "space" else [f"{flag}={value}"]
    if unit[0] == "list":
        return ["list"]
    return ["--dry-run"]


def flag_value(rng, flag, regs):
    if flag == "--registry":
        return rng.choice([regs["A"], regs["B"]])
    return f"/tmp/deploy-prop-{rng.randint(0, 99999)}"


def gen_valid_units(rng, regs):
    flags = [f for f in FLAGS if rng.random() < 0.7]
    units = [
        ("flag", f, rng.choice(["space", "eq"]), flag_value(rng, f, regs))
        for f in flags
    ]
    units.append(("list",))
    if rng.random() < 0.3:
        units.append(("dry-run",))
    rng.shuffle(units)
    return units


def model(units, env, regs):
    """Predict (exit_code, stderr_fragment, listed_service)."""
    seen = {}
    for unit in units:
        if unit[0] != "flag":
            continue
        _, flag, _, value = unit
        if flag in seen:
            return 2, f"deploy: {flag} given more than once", None
        seen[flag] = value
    registry = seen.get("--registry", env)
    if registry is None:
        return 2, "no registry configured", None
    return 0, None, "svc-a" if registry == regs["A"] else "svc-b"


def main(argv):
    args = [a for a in argv if not a.startswith("--")]
    seed, cases = 6376, 300
    for i, a in enumerate(argv):
        if a == "--seed":
            seed = int(argv[i + 1])
        if a == "--cases":
            cases = int(argv[i + 1])
    path = args[0] if args else os.path.join(os.path.dirname(os.path.abspath(__file__)), "deploy.py")
    mod = load(path)

    rng = random.Random(seed)
    with tempfile.TemporaryDirectory() as tmp:
        regs = {}
        for name in ("A", "B"):
            regs[name] = os.path.join(tmp, f"reg{name}.json")
            with open(regs[name], "w") as fh:
                json.dump({f"svc-{name.lower()}": definition()}, fh)

        for case in range(cases):
            kind = rng.choice(["valid", "valid", "duplicate", "missing"])
            units = gen_valid_units(rng, regs)
            env = rng.choice([regs["B"], None])
            if kind == "duplicate":
                present = [u[1] for u in units if u[0] == "flag"]
                flag = rng.choice(present) if present else "--registry"
                if flag not in present:
                    units.insert(0, ("flag", flag, "eq", flag_value(rng, flag, regs)))
                units.insert(
                    rng.randint(0, len(units)),
                    ("flag", flag, rng.choice(["space", "eq"]), flag_value(rng, flag, regs)),
                )
            if kind == "missing":
                flag = rng.choice(FLAGS)
                units = [u for u in units if not (u[0] == "flag" and u[1] == flag)]
                form = rng.choice(["trailing", "eq-empty"])
                if form == "trailing":
                    units.append(("missing-space", flag))
                else:
                    units.insert(rng.randint(0, len(units)), ("missing-eq", flag))
                tokens = []
                for u in units:
                    if u[0] == "missing-space":
                        tokens.append(u[1])
                    elif u[0] == "missing-eq":
                        tokens.append(u[1] + "=")
                    else:
                        tokens.extend(render(u))
                got, out, err = run(mod, tokens, env)
                want = f"deploy: {flag} needs a path"
                if got != 2 or want not in err:
                    print(f"counterexample (missing) seed={seed} case={case}: {tokens}\nexit={got} err={err!r}")
                    return 1
                continue
            tokens = [t for u in units for t in render(u)]
            code, frag, svc = model(units, env, regs)
            got, out, err = run(mod, tokens, env)
            if got != code or (frag is not None and frag not in err) or (svc is not None and svc not in out):
                print(
                    f"counterexample ({kind}) seed={seed} case={case}: {tokens}\n"
                    f"model: exit={code} frag={frag!r} svc={svc!r}\ngot: exit={got} out={out!r} err={err!r}"
                )
                return 1
    print(f"property holds: {cases} cases (seed {seed}); duplicates and missing values rejected, valid placements/spellings preserve the modelled registry selection")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
